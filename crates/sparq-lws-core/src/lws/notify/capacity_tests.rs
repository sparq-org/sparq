use super::*;
use test_store::{call, response_bytes};

fn subscription_body<S: Store>(state: &LwsState<S>, topic: &str) -> String {
    let _ = state;
    json!({"type": WEBHOOK, "topic": [topic], "inbox": "https://inbox.example/"}).to_string()
}

#[tokio::test]
async fn quota_refusals_are_indistinguishable() {
    let (state, _) = test_store::state(1000).await;
    let body = subscription_body(&state, &state.cfg.storage());
    let x = "https://x.example/";
    for i in 0..16 {
        let mut sub = sub_until(i64::MAX);
        sub.id = format!("s{i:04}");
        sub.subscriber = Some(if i < 16 {
            x.to_string()
        } else {
            format!("https://p{i}.example/")
        });
        state
            .notify
            .subs
            .write()
            .unwrap()
            .insert(sub.id.clone(), sub);
    }
    let own = response_bytes(call(&state, "POST", SUBSCRIPTIONS_PATH, Some(x), &body).await).await;
    for i in 16..MAX_SUBSCRIPTIONS {
        let mut sub = sub_until(i64::MAX);
        sub.id = format!("s{i:04}");
        sub.subscriber = Some(format!("https://p{i}.example/"));
        state
            .notify
            .subs
            .write()
            .unwrap()
            .insert(sub.id.clone(), sub);
    }
    let full = response_bytes(
        call(
            &state,
            "POST",
            SUBSCRIPTIONS_PATH,
            Some("https://new.example/"),
            &body,
        )
        .await,
    )
    .await;
    assert_eq!(own.0, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(own, full);
    assert!(String::from_utf8(own.2.to_vec())
        .unwrap()
        .contains("this agent may not hold more subscriptions now"));
}

#[tokio::test]
async fn quarantined_subscriptions_take_no_place_from_others() {
    let (state, _) = test_store::state(1000).await;
    // Reload the startup listing, including malformed records and one held back by recovery.
    let container = state.cfg.absolute(SUBSCRIPTIONS_PATH);
    for i in 0..3 {
        state
            .store
            .create_in_container(
                &container,
                &format!("{container}bad{i}"),
                Bytes::from_static(b"bad"),
                LWS_JSON,
            )
            .await
            .unwrap();
    }
    state
        .store
        .create_in_container(
            &container,
            &format!("{container}held"),
            Bytes::from_static(b"bad"),
            LWS_JSON,
        )
        .await
        .unwrap();
    state
        .notify
        .load_with(&state.store, &state.cfg, |iri| !iri.ends_with("/held"))
        .await
        .unwrap();
    assert_eq!(state.notify.quarantined.read().unwrap().len(), 3);
    assert_eq!(state.notify.held_back.load(Ordering::SeqCst), 1);
    let body = subscription_body(&state, &state.cfg.storage());
    for i in 0..MAX_SUBSCRIPTIONS {
        let who = format!("https://p{i}.example/");
        assert_eq!(
            call(&state, "POST", SUBSCRIPTIONS_PATH, Some(&who), &body)
                .await
                .status(),
            StatusCode::CREATED,
            "subscription {i}"
        );
    }
    assert_eq!(
        call(
            &state,
            "POST",
            SUBSCRIPTIONS_PATH,
            Some("https://last.example/"),
            &body
        )
        .await
        .status(),
        StatusCode::TOO_MANY_REQUESTS
    );
}

#[tokio::test]
async fn a_create_is_not_delayed_by_another_principal_s_settling_removal() {
    let (state, store) = test_store::closed_state(100).await;
    let topic = test_store::data(&state, "r").await;
    let x = "https://x.example/";
    let y = "https://y.example/";
    test_store::grant_read(&state, x, &[&topic]).await;
    test_store::grant_read(&state, y, &[&topic]).await;
    let body = subscription_body(&state, &topic);
    let created = call(&state, "POST", SUBSCRIPTIONS_PATH, Some(x), &body).await;
    assert_eq!(created.status(), StatusCode::CREATED);
    let iri = created.headers()[header::LOCATION]
        .to_str()
        .unwrap()
        .to_string();
    *store.unsettle_next_delete_of.lock().unwrap() =
        Some((iri.clone(), Arc::new(tokio::sync::Semaphore::new(0))));
    store.hang_fence.store(true, Ordering::SeqCst);
    let path = iri.strip_prefix(&state.cfg.base_url).unwrap();
    assert_eq!(
        call(&state, "DELETE", path, Some(x), "").await.status(),
        StatusCode::INTERNAL_SERVER_ERROR
    );
    let response = tokio::time::timeout(
        crate::lws::records::LOCK_WAIT,
        call(&state, "POST", SUBSCRIPTIONS_PATH, Some(y), &body),
    )
    .await
    .expect("another principal's settlement delayed the create");
    assert_eq!(response.status(), StatusCode::CREATED);
    store.hang_fence.store(false, Ordering::SeqCst);
}

async fn stream(state: &LwsState<test_store::FlakyStore>, doc: &Value) -> axum::body::Body {
    use tower::ServiceExt;
    let request = axum::http::Request::builder()
        .method("GET")
        .uri(doc["source"].as_str().unwrap())
        .header("accept", "text/event-stream")
        .body(axum::body::Body::empty())
        .unwrap();
    let response = axum::Router::new()
        .fallback(crate::lws::dispatch::<test_store::FlakyStore>)
        .with_state(state.clone())
        .oneshot(request)
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    response.into_body()
}

async fn next_notification(body: &mut axum::body::Body) -> String {
    use http_body_util::BodyExt;
    let frame = tokio::time::timeout(crate::lws::records::LOCK_WAIT, body.frame())
        .await
        .expect("notification did not arrive")
        .unwrap()
        .unwrap();
    String::from_utf8(frame.into_data().unwrap().to_vec()).unwrap()
}

#[tokio::test]
async fn a_write_s_response_does_not_wait_on_subscriber_selection() {
    use futures_util::FutureExt;
    use http_body_util::BodyExt;
    let (state, _) = test_store::state(100).await;
    let topic = test_store::data(&state, "r").await;
    let body = json!({"type": streams::SSE, "topic": [&topic]}).to_string();
    let response = call(
        &state,
        "POST",
        SUBSCRIPTIONS_PATH,
        Some("https://reader.example/"),
        &body,
    )
    .await;
    assert_eq!(response.status(), StatusCode::CREATED);
    let doc = test_store::body_json(response).await;
    let mut received = stream(&state, &doc).await;
    run_ready().await;
    let gate = Arc::new(tokio::sync::Semaphore::new(0));
    *state.notify.selection_gate.lock().unwrap() = Some(gate.clone());
    let response = tokio::time::timeout(
        crate::lws::records::LOCK_WAIT,
        call(
            &state,
            "PUT",
            "/r",
            Some("https://writer.example/"),
            "changed",
        ),
    )
    .await
    .expect("PUT waited on selection");
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    assert!(received.frame().now_or_never().is_none());
    gate.add_permits(1);
    let delivered = next_notification(&mut received).await;
    assert!(delivered.contains("event: notification\n"));
    assert!(delivered.contains("Update"));
    assert!(delivered.contains(&topic));
}

#[tokio::test]
async fn commit_work_does_not_grow_with_subscriptions() {
    async fn work(
        n: usize,
        denied: bool,
    ) -> (crate::store::counting::CounterSnapshot, Vec<String>) {
        let (state, store) = test_store::closed_state(100).await;
        let topic = test_store::data(&state, "r").await;
        // Also exercise a nonempty shared policy array.
        test_store::grant_read(&state, "https://reader.example/", &[&topic]).await;
        run_ready().await;
        for i in 0..n {
            let mut sub = sub_until(i64::MAX);
            sub.id = format!("s{i:04}");
            sub.subscriber = Some(if denied {
                format!("https://p{i}.example/")
            } else {
                "https://reader.example/".into()
            });
            sub.topics = vec![topic.clone()];
            state
                .notify
                .subs
                .write()
                .unwrap()
                .insert(sub.id.clone(), sub);
        }
        *state.notify.selection_gate.lock().unwrap() =
            Some(Arc::new(tokio::sync::Semaphore::new(0)));
        *store.trace.lock().unwrap() = Some(Vec::new());
        let scope = store.counters.measure();
        assert_eq!(
            call(&state, "PUT", "/r", Some(test_store::OWNER), "changed")
                .await
                .status(),
            StatusCode::NO_CONTENT
        );
        let counts = scope.delta();
        let trace = store
            .trace
            .lock()
            .unwrap()
            .take()
            .unwrap()
            .into_iter()
            .map(|line| {
                if let Some((prefix, _)) = line.split_once("/.lws/intents/") {
                    format!("{prefix}/.lws/intents/{{id}}")
                } else {
                    line
                }
            })
            .collect();
        (counts, trace)
    }
    let empty = work(0, false).await;
    assert_eq!(empty, work(500, false).await);
    assert_eq!(empty, work(500, true).await);
}

#[tokio::test]
async fn a_subscription_registered_after_the_commit_hears_nothing_of_it() {
    use futures_util::FutureExt;
    use http_body_util::BodyExt;
    let (state, _) = test_store::state(100).await;
    let topic = test_store::data(&state, "r").await;
    run_ready().await;
    let before = state.notify.selected.load(Ordering::SeqCst);
    let gate = Arc::new(tokio::sync::Semaphore::new(0));
    *state.notify.selection_gate.lock().unwrap() = Some(gate.clone());
    assert_eq!(
        call(&state, "PUT", "/r", None, "changed").await.status(),
        StatusCode::NO_CONTENT
    );
    let body = json!({"type": streams::SSE, "topic": [&topic]}).to_string();
    let response = call(
        &state,
        "POST",
        SUBSCRIPTIONS_PATH,
        Some("https://reader.example/"),
        &body,
    )
    .await;
    assert_eq!(response.status(), StatusCode::CREATED);
    let doc = test_store::body_json(response).await;
    let mut received = stream(&state, &doc).await;
    // A later write witnesses that selection has finished and delivery works.
    assert_eq!(
        call(&state, "PUT", "/r", None, "changed again")
            .await
            .status(),
        StatusCode::NO_CONTENT
    );
    gate.add_permits(16);
    tokio::time::timeout(crate::lws::records::LOCK_WAIT, async {
        while state.notify.selected.load(Ordering::SeqCst) < before + 3 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(next_notification(&mut received).await.contains("Update"));
    assert!(
        received.frame().now_or_never().is_none(),
        "heard a change committed before registration"
    );
}

fn grant_body<S: Store>(state: &LwsState<S>) -> String {
    json!({"@context": [crate::lws::LWS_CONTEXT], "type": ["AccessGrant"], "storage": state.cfg.storage(),
        "access": [{"type": ["AccessPolicy"], "action": ["read"], "assignee": "https://a.example/"}]}).to_string()
}

#[tokio::test]
async fn automatic_cancellations_use_the_subscriber_s_settlement_places() {
    let (state, store) = test_store::closed_state(100).await;
    let topic = test_store::data(&state, "r").await;
    let x = test_store::OWNER;
    let body = subscription_body(&state, &topic);
    for _ in 0..crate::lws::records::MAX_SETTLING_PER_PRINCIPAL {
        assert_eq!(
            call(&state, "POST", SUBSCRIPTIONS_PATH, Some(x), &body)
                .await
                .status(),
            StatusCode::CREATED
        );
    }
    run_ready().await;
    let gate = Arc::new(tokio::sync::Semaphore::new(0));
    *state.notify.selection_gate.lock().unwrap() = Some(gate.clone());
    // Only the transport result is injected; the HTTP write selects and queues every delivery,
    // and the real completion path cancels the subscriptions and reserves their places.
    *state.notify.delivery_status.lock().unwrap() = Some(StatusCode::GONE);
    assert_eq!(
        call(&state, "PUT", "/r", Some(test_store::OWNER), "changed")
            .await
            .status(),
        StatusCode::NO_CONTENT
    );
    store.fail_delete.store(true, Ordering::SeqCst);
    store.fail_exists.store(true, Ordering::SeqCst);
    store.fail_list.store(true, Ordering::SeqCst);
    gate.add_permits(1);
    tokio::time::timeout(crate::lws::records::LOCK_WAIT * 4, async {
        while state.settlements.pending() != crate::lws::records::MAX_SETTLING_PER_PRINCIPAL {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("automatic cancellations did not enter settlement");
    // Another subject reaches the failing store; the subscriber is at its own place bound.
    let grant = grant_body(&state);
    assert_eq!(
        call(
            &state,
            "POST",
            crate::lws::REQUESTS_PATH,
            Some("https://y.example/"),
            &grant.replace("AccessGrant", "AccessRequest")
        )
        .await
        .status(),
        StatusCode::INTERNAL_SERVER_ERROR
    );
    assert_eq!(
        call(&state, "POST", crate::lws::GRANTS_PATH, Some(x), &grant)
            .await
            .status(),
        StatusCode::SERVICE_UNAVAILABLE
    );
    store.fail_delete.store(false, Ordering::SeqCst);
    store.fail_exists.store(false, Ordering::SeqCst);
    store.fail_list.store(false, Ordering::SeqCst);
}

#[tokio::test]
async fn quarantined_deletions_use_the_requesting_principal_s_places() {
    let (state, store) = test_store::closed_state(100).await;
    let container = state.cfg.absolute(SUBSCRIPTIONS_PATH);
    store
        .create_in_container(
            &container,
            &format!("{container}bad"),
            Bytes::from_static(b"bad"),
            LWS_JSON,
        )
        .await
        .unwrap();
    state
        .notify
        .load_with(&state.store, &state.cfg, |_| true)
        .await
        .unwrap();
    let y = test_store::OWNER;
    let gate = Arc::new(tokio::sync::Semaphore::new(0));
    *store.unsettle_creates.lock().unwrap() = Some(gate.clone());
    store.fail_fence.store(true, Ordering::SeqCst);
    let grant = grant_body(&state);
    for _ in 0..crate::lws::records::MAX_SETTLING_PER_PRINCIPAL {
        assert_eq!(
            call(&state, "POST", crate::lws::GRANTS_PATH, Some(y), &grant)
                .await
                .status(),
            StatusCode::INTERNAL_SERVER_ERROR
        );
    }
    let path = format!("{SUBSCRIPTIONS_PATH}bad");
    assert_eq!(
        call(&state, "DELETE", &path, Some(y), "").await.status(),
        StatusCode::SERVICE_UNAVAILABLE
    );
    *store.unsettle_creates.lock().unwrap() = None;
    store.fail_fence.store(false, Ordering::SeqCst);
    gate.add_permits(crate::lws::records::MAX_SETTLING_PER_PRINCIPAL);
    tokio::time::timeout(crate::lws::records::LOCK_WAIT * 4, async {
        while state.settlements.pending() != 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(
        call(&state, "DELETE", &path, Some(y), "").await.status(),
        StatusCode::NO_CONTENT
    );
}

#[tokio::test]
async fn content_only_patch_reuses_authorization_metadata() {
    async fn work(
        n: usize,
        denied: bool,
        mode: &str,
    ) -> (crate::store::counting::CounterSnapshot, Vec<String>) {
        use tower::ServiceExt;
        let store = test_store::FlakyStore::new();
        let mut cfg = crate::lws::LwsConfig::new("http://localhost:3000");
        cfg.owner = Some(test_store::OWNER.into());
        cfg.allow_insecure_fetch = true;
        cfg.open = mode == "open";
        let state = LwsState::new(store.clone(), cfg).await.unwrap();
        let topic = test_store::data(&state, "r").await;
        let writer = if mode.starts_with("owner") {
            test_store::OWNER
        } else {
            "https://writer.example/"
        };
        state
            .store
            .write(
                &topic,
                Bytes::from_static(br#"{"value":0}"#),
                "application/json",
            )
            .await
            .unwrap();
        state
            .put_resource_meta(
                &topic,
                &crate::lws::ResourceMeta {
                    creator: Some(writer.into()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        test_store::grant_read(&state, "https://reader.example/", &[&topic]).await;
        run_ready().await;
        for i in 0..n {
            let mut sub = sub_until(i64::MAX);
            sub.id = format!("s{i:04}");
            sub.subscriber = Some(if mode == "owner_only" {
                test_store::OWNER.into()
            } else if denied {
                format!("https://p{i}.example/")
            } else {
                "https://reader.example/".into()
            });
            sub.topics = vec![topic.clone()];
            state
                .notify
                .subs
                .write()
                .unwrap()
                .insert(sub.id.clone(), sub);
        }
        if mode == "owner_removed" {
            let mut subs = state.notify.subs.write().unwrap();
            let ids: Vec<_> = subs.keys().cloned().collect();
            for id in ids {
                subs.remove(&id);
            }
        }
        *state.notify.selection_gate.lock().unwrap() =
            Some(Arc::new(tokio::sync::Semaphore::new(0)));
        let token = crate::lws::tokens::mint(
            &state.cfg,
            writer,
            "https://app.example/",
            &state.cfg.realm(),
        );
        let request = axum::http::Request::builder()
            .method("PATCH")
            .uri("/r")
            .header("authorization", format!("Bearer {token}"))
            .header("content-type", crate::lws::JSON_PATCH)
            .body(axum::body::Body::from(
                r#"[{"op":"replace","path":"/value","value":1}]"#,
            ))
            .unwrap();
        *store.trace.lock().unwrap() = Some(Vec::new());
        let scope = store.counters.measure();
        let response = axum::Router::new()
            .fallback(crate::lws::dispatch::<test_store::FlakyStore>)
            .with_state(state.clone())
            .oneshot(request)
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NO_CONTENT);
        let counts = scope.delta();
        let trace = store.trace.lock().unwrap().take().unwrap();
        let meta_reads = trace
            .iter()
            .filter(|line| **line == format!("read {topic}.meta"))
            .count();
        assert_eq!(
            meta_reads,
            match mode {
                "open" => 0,
                "owner" => usize::from(n != 0),
                "owner_only" | "owner_removed" => 0,
                _ => 4,
            },
            "PATCH must not duplicate its authorization metadata read: {trace:?}"
        );
        assert_eq!(
            store.read(&topic).await.unwrap().body,
            Bytes::from_static(br#"{"value":1}"#)
        );
        (counts, trace)
    }
    for mode in ["creator", "owner", "owner_only", "owner_removed", "open"] {
        let empty = work(0, false, mode).await;
        let populated = work(500, false, mode).await;
        let unreadable = work(500, true, mode).await;
        assert_eq!(populated, unreadable);
        if mode != "owner" {
            assert_eq!(empty, populated);
        } else {
            // Owner authorization reads no metadata. The O(1) non-owner check adds exactly
            // the one snapshot read only when another reader could hear this capture.
            assert_eq!(
                populated
                    .1
                    .iter()
                    .filter(|line| line.starts_with("read "))
                    .count(),
                empty
                    .1
                    .iter()
                    .filter(|line| line.starts_with("read "))
                    .count()
                    + 1
            );
        }
    }
}

#[tokio::test]
async fn selection_keeps_the_request_s_admission_slot() {
    use tower::ServiceExt;
    const BOUND: usize = 4;
    for method in ["PUT", "POST", "DELETE", "GRANT", "SUBSCRIBE", "REVOKE"] {
        let (state, _) = test_store::state(100).await;
        let topic = test_store::data(&state, "r").await;
        for i in 0..BOUND {
            test_store::data(&state, &format!("d{i}")).await;
        }
        let mut revocations = Vec::new();
        if method == "REVOKE" {
            for _ in 0..BOUND {
                let response = call(
                    &state,
                    "POST",
                    crate::lws::GRANTS_PATH,
                    None,
                    &grant_body(&state),
                )
                .await;
                assert_eq!(response.status(), StatusCode::CREATED);
                revocations.push(
                    response.headers()[header::LOCATION]
                        .to_str()
                        .unwrap()
                        .strip_prefix(&state.cfg.base_url)
                        .unwrap()
                        .to_string(),
                );
            }
        }
        for i in 0..1024 {
            let mut sub = sub_until(i64::MAX);
            sub.id = format!("s{i:04}");
            sub.topics = vec![topic.clone()];
            sub.subscriber = Some(format!("https://p{i}.example/"));
            state
                .notify
                .subs
                .write()
                .unwrap()
                .insert(sub.id.clone(), sub);
        }
        // Inject only the transport result; the full selection and queueing still run.
        *state.notify.delivery_status.lock().unwrap() = Some(StatusCode::NO_CONTENT);
        // Subscription creates need room in the existing quota while exercising selection.
        if method == "SUBSCRIBE" {
            let mut subs = state.notify.subs.write().unwrap();
            let ids: Vec<_> = subs.keys().cloned().collect();
            for id in ids {
                subs.remove(&id);
            }
        }
        run_ready().await;
        let before = state.notify.selected.load(Ordering::SeqCst);
        let gate = Arc::new(tokio::sync::Semaphore::new(0));
        *state.notify.selection_gate.lock().unwrap() = Some(gate.clone());
        let app = crate::app::with_overload_layers(
            axum::Router::new()
                .fallback(crate::lws::dispatch::<test_store::FlakyStore>)
                .with_state(state.clone()),
            crate::app::OverloadConfig::new(BOUND, None),
        );
        for i in 0..BOUND * 2 {
            let (verb, path, body, expected) = match method {
                "POST" => ("POST", "/".to_string(), "new".into(), StatusCode::CREATED),
                "DELETE" => (
                    "DELETE",
                    format!("/d{}", i % BOUND),
                    String::new(),
                    StatusCode::NO_CONTENT,
                ),
                "REVOKE" => (
                    "DELETE",
                    revocations[i % BOUND].clone(),
                    String::new(),
                    StatusCode::NO_CONTENT,
                ),
                "GRANT" => (
                    "POST",
                    crate::lws::GRANTS_PATH.into(),
                    grant_body(&state),
                    StatusCode::CREATED,
                ),
                "SUBSCRIBE" => (
                    "POST",
                    SUBSCRIPTIONS_PATH.into(),
                    subscription_body(&state, &topic),
                    StatusCode::CREATED,
                ),
                _ => ("PUT", "/r".into(), "changed".into(), StatusCode::NO_CONTENT),
            };
            let request = axum::http::Request::builder()
                .method(verb)
                .uri(path)
                .header("content-type", LWS_JSON)
                .header("slug", format!("c{i}"))
                .body(axum::body::Body::from(body))
                .unwrap();
            let response =
                tokio::time::timeout(crate::lws::records::LOCK_WAIT, app.clone().oneshot(request))
                    .await
                    .expect("response waited on selection")
                    .unwrap();
            if i < BOUND {
                assert_eq!(response.status(), expected, "{method} write {i}");
            } else {
                assert_eq!(
                    response.status(),
                    StatusCode::SERVICE_UNAVAILABLE,
                    "{method}: selection escaped admission"
                );
                assert!(response.headers().contains_key(header::RETRY_AFTER));
            }
        }
        assert_eq!(state.notify.selected.load(Ordering::SeqCst), before);
        gate.add_permits(BOUND);
        tokio::time::timeout(crate::lws::records::LOCK_WAIT, async {
            while state.notify.selected.load(Ordering::SeqCst) < before + BOUND {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        let response = app
            .oneshot(
                axum::http::Request::builder()
                    .uri("/r")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            StatusCode::OK,
            "{method}: admission was not released"
        );
    }
}

#[tokio::test]
async fn owner_patch_keeps_a_snapshot_for_pending_subscriptions() {
    use tower::ServiceExt;
    for unknown in [false, true] {
        let (state, store) = test_store::closed_state(100).await;
        let topic = test_store::data(&state, "r").await;
        store
            .write(
                &topic,
                Bytes::from_static(br#"{"value":0}"#),
                "application/json",
            )
            .await
            .unwrap();
        let reader = "https://reader.example/";
        test_store::grant_read(&state, reader, &[&topic]).await;
        run_ready().await;
        let selection = Arc::new(tokio::sync::Semaphore::new(0));
        *state.notify.selection_gate.lock().unwrap() = Some(selection.clone());
        let registration = Arc::new(tokio::sync::Semaphore::new(0));
        if unknown {
            *store.unsettle_creates.lock().unwrap() = Some(registration.clone());
            store.fail_fence.store(true, Ordering::SeqCst);
        } else {
            *store.hold_next_create.lock().unwrap() = Some(registration.clone());
        }
        let mut subscribing = Some(tokio::spawn({
            let (state, topic) = (state.clone(), topic.clone());
            async move {
                call(
                    &state,
                    "POST",
                    SUBSCRIPTIONS_PATH,
                    Some(reader),
                    &json!({"type": streams::SSE, "topic": [topic]}).to_string(),
                )
                .await
            }
        }));
        if unknown {
            assert_eq!(
                subscribing.take().unwrap().await.unwrap().status(),
                StatusCode::INTERNAL_SERVER_ERROR
            );
        } else {
            tokio::time::timeout(crate::lws::records::LOCK_WAIT, async {
                while store.hold_next_create.lock().unwrap().is_some() {
                    tokio::task::yield_now().await;
                }
            })
            .await
            .expect("subscription did not reach the store");
        }
        assert!(state.notify.subs.read().unwrap().is_empty());
        let writing = Arc::new(tokio::sync::Semaphore::new(0));
        *store.hold_next_write_of.lock().unwrap() = Some((topic.clone(), writing.clone()));
        *store.trace.lock().unwrap() = Some(Vec::new());
        let patching = tokio::spawn({
            let state = state.clone();
            async move {
                let token = crate::lws::tokens::mint(
                    &state.cfg,
                    test_store::OWNER,
                    "https://app.example/",
                    &state.cfg.realm(),
                );
                let request = axum::http::Request::builder()
                    .method("PATCH")
                    .uri("/r")
                    .header("authorization", format!("Bearer {token}"))
                    .header("content-type", crate::lws::JSON_PATCH)
                    .body(axum::body::Body::from(
                        r#"[{"op":"replace","path":"/value","value":1}]"#,
                    ))
                    .unwrap();
                axum::Router::new()
                    .fallback(crate::lws::dispatch::<test_store::FlakyStore>)
                    .with_state(state)
                    .oneshot(request)
                    .await
                    .unwrap()
            }
        });
        tokio::time::timeout(crate::lws::records::LOCK_WAIT, async {
            while store.hold_next_write_of.lock().unwrap().is_some() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("PATCH did not reach the content write");
        assert!(
            store
                .trace
                .lock()
                .unwrap()
                .as_ref()
                .unwrap()
                .contains(&format!("read {topic}.meta")),
            "pending registration lost its commit snapshot"
        );
        *store.unsettle_creates.lock().unwrap() = None;
        registration.add_permits(1);
        if unknown {
            let container = state.cfg.absolute(SUBSCRIPTIONS_PATH);
            tokio::time::timeout(crate::lws::records::LOCK_WAIT, async {
                while store.list_children(&container).await.unwrap().is_empty() {
                    tokio::task::yield_now().await;
                }
            })
            .await
            .expect("subscription did not land before its fence");
        }
        store.fail_fence.store(false, Ordering::SeqCst);
        let doc = if let Some(subscribing) = subscribing {
            let response = subscribing.await.unwrap();
            assert_eq!(response.status(), StatusCode::CREATED);
            Some(test_store::body_json(response).await)
        } else {
            tokio::time::timeout(crate::lws::records::LOCK_WAIT * 4, async {
                while state.notify.subs.read().unwrap().is_empty() {
                    tokio::task::yield_now().await;
                }
            })
            .await
            .expect("subscription was not confirmed");
            None
        };
        // Registration precedes the commit, although PATCH initially saw no live subscriptions.
        assert_eq!(
            state.notify.registering_non_owners.load(Ordering::SeqCst),
            0
        );
        writing.add_permits(1);
        assert_eq!(patching.await.unwrap().status(), StatusCode::NO_CONTENT);
        if let Some(doc) = doc {
            let mut received = stream(&state, &doc).await;
            selection.add_permits(1);
            assert!(next_notification(&mut received).await.contains("Update"));
        } else {
            selection.add_permits(1);
        }
    }
}

#[tokio::test]
async fn delayed_record_selection_keeps_a_shared_place() {
    use crate::lws::records::{MAX_SETTLING, MAX_SETTLING_PER_PRINCIPAL};
    let count = MAX_SETTLING_PER_PRINCIPAL;
    for removal in [false, true] {
        let (state, store) = test_store::closed_state(100).await;
        let body = grant_body(&state);
        let mut records = Vec::new();
        if removal {
            for _ in 0..count {
                let response = call(
                    &state,
                    "POST",
                    crate::lws::GRANTS_PATH,
                    Some(test_store::OWNER),
                    &body,
                )
                .await;
                assert_eq!(response.status(), StatusCode::CREATED);
                records.push(
                    response.headers()[header::LOCATION]
                        .to_str()
                        .unwrap()
                        .to_string(),
                );
            }
        } else {
            records.resize(count, String::new());
        }
        run_ready().await;
        let mut occupied = Vec::new();
        for i in 0..MAX_SETTLING - count {
            occupied.push(
                state
                    .settlements
                    .reserve(Some(&format!("https://p{i}.example/")))
                    .unwrap(),
            );
        }
        let before = state.notify.selected.load(Ordering::SeqCst);
        let selection = Arc::new(tokio::sync::Semaphore::new(0));
        *state.notify.selection_gate.lock().unwrap() = Some(selection.clone());
        let store_gate = Arc::new(tokio::sync::Semaphore::new(0));
        store.fail_fence.store(true, Ordering::SeqCst);
        if !removal {
            *store.unsettle_creates.lock().unwrap() = Some(store_gate.clone());
        }
        for iri in &records {
            let response = if removal {
                *store.unsettle_next_delete_of.lock().unwrap() =
                    Some((iri.clone(), store_gate.clone()));
                call(
                    &state,
                    "DELETE",
                    iri.strip_prefix(&state.cfg.base_url).unwrap(),
                    Some(test_store::OWNER),
                    "",
                )
                .await
            } else {
                call(
                    &state,
                    "POST",
                    crate::lws::GRANTS_PATH,
                    Some(test_store::OWNER),
                    &body,
                )
                .await
            };
            assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        }
        assert_eq!(state.settlements.pending(), count);
        *store.unsettle_creates.lock().unwrap() = None;
        store_gate.add_permits(count);
        // Let every sent operation land before fencing it; a fence may otherwise cancel
        // the operation, leaving no committed change to announce.
        let container = state.cfg.absolute(crate::lws::GRANTS_PATH);
        tokio::time::timeout(crate::lws::records::LOCK_WAIT * 4, async {
            loop {
                let held = store.list_children(&container).await.unwrap().len();
                if held == if removal { 0 } else { count } {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("sent operations did not land");
        store.fail_fence.store(false, Ordering::SeqCst);
        tokio::time::timeout(crate::lws::records::LOCK_WAIT * 4, async {
            while state.settlements.pending() != 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("record outcomes were not settled");
        assert_eq!(state.notify.selected.load(Ordering::SeqCst), before);
        assert_eq!(
            call(
                &state,
                "POST",
                crate::lws::GRANTS_PATH,
                Some(test_store::OWNER),
                &body
            )
            .await
            .status(),
            StatusCode::SERVICE_UNAVAILABLE,
            "delayed selection released its shared work bound"
        );
        drop(occupied.pop());
        assert_eq!(
            call(
                &state,
                "POST",
                crate::lws::GRANTS_PATH,
                Some(test_store::OWNER),
                &body
            )
            .await
            .status(),
            StatusCode::CREATED,
            "known outcomes kept principal settlement places during selection"
        );
        selection.add_permits(count + 1);
        tokio::time::timeout(crate::lws::records::LOCK_WAIT, async {
            while state.notify.selected.load(Ordering::SeqCst) < before + count + 1 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert_eq!(
            call(
                &state,
                "POST",
                crate::lws::GRANTS_PATH,
                Some(test_store::OWNER),
                &body
            )
            .await
            .status(),
            StatusCode::CREATED
        );
    }
}

#[tokio::test(start_paused = true)]
async fn maintenance_selection_keeps_its_maintenance_place() {
    use crate::lws::records::MAINTENANCE_PLACES;
    let (state, _) = test_store::closed_state(100).await;
    let expires = format_rfc3339(state.notify.clock() + 60);
    let body = json!({"type": WEBHOOK, "topic": [state.cfg.storage()],
        "inbox": "https://inbox.example/", "expires": expires})
    .to_string();
    for _ in 0..MAINTENANCE_PLACES {
        assert_eq!(
            call(
                &state,
                "POST",
                SUBSCRIPTIONS_PATH,
                Some(test_store::OWNER),
                &body
            )
            .await
            .status(),
            StatusCode::CREATED
        );
    }
    run_ready().await;
    let before = state.notify.selected.load(Ordering::SeqCst);
    let selection = Arc::new(tokio::sync::Semaphore::new(0));
    *state.notify.selection_gate.lock().unwrap() = Some(selection.clone());
    state.notify.clock_offset.store(120, Ordering::SeqCst);
    tokio::time::sleep(EXPIRY_SWEEP_EVERY).await;
    run_ready().await;
    assert!(
        state.notify.subs.read().unwrap().is_empty(),
        "expiry did not remove the records"
    );
    assert_eq!(
        state.settlements.maintenance_free(),
        0,
        "maintenance selection released its work bound"
    );
    // Maintenance places are separate: a client create still has its shared place.
    assert_eq!(
        call(
            &state,
            "POST",
            SUBSCRIPTIONS_PATH,
            Some(test_store::OWNER),
            &subscription_body(&state, &state.cfg.storage())
        )
        .await
        .status(),
        StatusCode::CREATED
    );
    selection.add_permits(1);
    run_ready().await;
    assert_eq!(
        state.notify.selected.load(Ordering::SeqCst),
        before + MAINTENANCE_PLACES + 1
    );
    assert_eq!(state.settlements.maintenance_free(), MAINTENANCE_PLACES);
}

#[tokio::test]
async fn withdrawn_state_is_captured_at_commit() {
    use futures_util::FutureExt;
    use http_body_util::BodyExt;
    for withdrawn_at_commit in [true, false] {
        let (state, _) = test_store::state(100).await;
        let topic = test_store::data(&state, "r").await;
        let response = call(
            &state,
            "POST",
            SUBSCRIPTIONS_PATH,
            Some("https://reader.example/"),
            &json!({"type": streams::SSE, "topic": [&topic]}).to_string(),
        )
        .await;
        assert_eq!(response.status(), StatusCode::CREATED);
        let doc = test_store::body_json(response).await;
        let id = doc["id"]
            .as_str()
            .unwrap()
            .rsplit('/')
            .next()
            .unwrap()
            .to_string();
        let mut received = stream(&state, &doc).await;
        run_ready().await;
        let selected = state.notify.selected.load(Ordering::SeqCst);
        let prepared = state.notify.prepared_for.load(Ordering::SeqCst);
        let gate = Arc::new(tokio::sync::Semaphore::new(0));
        *state.notify.selection_gate.lock().unwrap() = Some(gate.clone());
        if withdrawn_at_commit {
            Arc::make_mut(&mut state.notify.withdrawn.write().unwrap()).insert(id.clone());
        }
        assert_eq!(
            call(&state, "PUT", "/r", None, "changed").await.status(),
            StatusCode::NO_CONTENT
        );
        if withdrawn_at_commit {
            Arc::make_mut(&mut state.notify.withdrawn.write().unwrap()).remove(&id);
        } else {
            Arc::make_mut(&mut state.notify.withdrawn.write().unwrap()).insert(id.clone());
        }
        gate.add_permits(1);
        tokio::time::timeout(crate::lws::records::LOCK_WAIT, async {
            while state.notify.selected.load(Ordering::SeqCst) == selected {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert_eq!(
            state.notify.prepared_for.load(Ordering::SeqCst) - prepared,
            usize::from(!withdrawn_at_commit),
            "selection used withdrawal state after capture"
        );
        // Delivery still checks current withdrawal, even when initial selection included it.
        assert!(received.frame().now_or_never().is_none());
    }
}

#[tokio::test]
async fn only_the_pool_signal_becomes_a_subscription_quota_refusal() {
    let (state, store) = test_store::state(100).await;
    let body = subscription_body(&state, &state.cfg.storage());
    *store.refuse_creates.lock().unwrap() = Some(false);
    assert_eq!(
        call(
            &state,
            "POST",
            SUBSCRIPTIONS_PATH,
            Some("https://x.example/"),
            &body
        )
        .await
        .status(),
        StatusCode::INSUFFICIENT_STORAGE
    );
    *store.refuse_creates.lock().unwrap() = Some(true);
    let pool = response_bytes(
        call(
            &state,
            "POST",
            SUBSCRIPTIONS_PATH,
            Some("https://x.example/"),
            &body,
        )
        .await,
    )
    .await;
    *store.refuse_creates.lock().unwrap() = None;
    for _ in 0..MAX_SUBSCRIPTIONS_PER_SUBSCRIBER {
        assert_eq!(
            call(
                &state,
                "POST",
                SUBSCRIPTIONS_PATH,
                Some("https://x.example/"),
                &body
            )
            .await
            .status(),
            StatusCode::CREATED
        );
    }
    let quota = response_bytes(
        call(
            &state,
            "POST",
            SUBSCRIPTIONS_PATH,
            Some("https://x.example/"),
            &body,
        )
        .await,
    )
    .await;
    assert_eq!(pool, quota);
    // A real in-memory pool refusal carries precisely the same bytes.
    let mut cfg = LwsConfig::new("http://localhost:3000");
    cfg.open = true;
    cfg.allow_insecure_fetch = true;
    let limits = crate::store::InMemoryStoreLimits::new(16 << 20, 64).with_pool(1, 16 << 20);
    let limited = LwsState::new(test_store::FlakyStore::with_limits(limits), cfg.clone())
        .await
        .unwrap();
    let real = response_bytes(
        call(
            &limited,
            "POST",
            SUBSCRIPTIONS_PATH,
            Some("https://x.example/"),
            &body,
        )
        .await,
    )
    .await;
    assert_eq!(real, quota);
    // Keep the blob pool roomy so the index's own pool signal is exercised independently.
    let indexed = LwsState::new(
        crate::store::CompositeStore::new(
            crate::store::InMemorySparqClient::with_limits(limits),
            crate::store::InMemoryBlobStore::new(),
        ),
        cfg,
    )
    .await
    .unwrap();
    let index_full = response_bytes(
        call(
            &indexed,
            "POST",
            SUBSCRIPTIONS_PATH,
            Some("https://x.example/"),
            &body,
        )
        .await,
    )
    .await;
    assert_eq!(index_full, quota);
}

#[tokio::test]
async fn anonymous_subscription_creates_keep_their_existing_quota_in_open_mode() {
    let (state, _) = test_store::state(100).await;
    let body = subscription_body(&state, &state.cfg.storage());
    let mut tasks = tokio::task::JoinSet::new();
    for _ in 0..64 {
        let (state, body) = (state.clone(), body.clone());
        tasks.spawn(async move {
            call(&state, "POST", SUBSCRIPTIONS_PATH, None, &body)
                .await
                .status()
        });
    }
    let mut created = 0;
    let mut quota_full = 0;
    while let Some(result) = tasks.join_next().await {
        match result.unwrap() {
            StatusCode::CREATED => created += 1,
            StatusCode::TOO_MANY_REQUESTS => quota_full += 1,
            status => panic!("unexpected anonymous create refusal: {status}"),
        }
    }
    assert_eq!(created, MAX_SUBSCRIPTIONS_PER_SUBSCRIBER);
    assert_eq!(quota_full, 64 - MAX_SUBSCRIPTIONS_PER_SUBSCRIBER);
}

async fn waiting_unknown_subscription() -> (
    LwsState<test_store::FlakyStore>,
    test_store::FlakyStore,
    Arc<tokio::sync::Semaphore>,
    tokio::task::JoinHandle<StatusCode>,
) {
    use crate::lws::records::{LOCK_WAIT, MAX_SETTLING_PER_PRINCIPAL};
    let (state, store) = test_store::closed_state(100).await;
    let a = "https://a.example/";
    let topic = state.cfg.storage();
    test_store::grant_read(&state, a, &[&topic]).await;
    run_ready().await;
    let unknown = Arc::new(tokio::sync::Semaphore::new(0));
    *store.unsettle_creates.lock().unwrap() = Some(unknown.clone());
    store.fail_fence.store(true, Ordering::SeqCst);
    let request = grant_body(&state).replace("AccessGrant", "AccessRequest");
    for _ in 0..MAX_SETTLING_PER_PRINCIPAL - 1 {
        assert_eq!(
            call(&state, "POST", crate::lws::REQUESTS_PATH, Some(a), &request)
                .await
                .status(),
            StatusCode::INTERNAL_SERVER_ERROR
        );
    }
    // Both changes reserve their shared places before either unknown answer takes A's last
    // principal permit. Only the subscription holds the global high-water writing gate.
    let sending = Arc::new(tokio::sync::Semaphore::new(0));
    *store.hold_creates.lock().unwrap() = Some(sending.clone());
    let first = tokio::spawn({
        let (state, body) = (state.clone(), request.clone());
        async move {
            call(&state, "POST", crate::lws::REQUESTS_PATH, Some(a), &body)
                .await
                .status()
        }
    });
    tokio::time::timeout(LOCK_WAIT, async {
        while store.creates_waiting.load(Ordering::SeqCst) != 1 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let second = tokio::spawn({
        let state = state.clone();
        let body = subscription_body(&state, &topic);
        async move {
            call(&state, "POST", SUBSCRIPTIONS_PATH, Some(a), &body)
                .await
                .status()
        }
    });
    tokio::time::timeout(LOCK_WAIT, async {
        while store.creates_waiting.load(Ordering::SeqCst) != 2 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    sending.add_permits(1);
    assert_eq!(first.await.unwrap(), StatusCode::INTERNAL_SERVER_ERROR);
    sending.add_permits(1);
    tokio::time::timeout(LOCK_WAIT, async {
        while state.settlements.principal_waits() == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(!second.is_finished());
    (state, store, unknown, second)
}

async fn conditional_subscription(
    state: &LwsState<test_store::FlakyStore>,
    subject: &str,
    body: &str,
) -> Response {
    use tower::ServiceExt;
    let listing = call(state, "GET", SUBSCRIPTIONS_PATH, Some(subject), "").await;
    assert_eq!(listing.status(), StatusCode::OK);
    let token = crate::lws::tokens::mint(
        &state.cfg,
        subject,
        "https://app.example/",
        &state.cfg.realm(),
    );
    let request = axum::http::Request::builder()
        .method("POST")
        .uri(SUBSCRIPTIONS_PATH)
        .header("authorization", format!("Bearer {token}"))
        .header("content-type", LWS_JSON)
        .header("if-match", &listing.headers()[header::ETAG])
        .body(axum::body::Body::from(body.to_string()))
        .unwrap();
    axum::Router::new()
        .fallback(crate::lws::dispatch::<test_store::FlakyStore>)
        .with_state(state.clone())
        .oneshot(request)
        .await
        .unwrap()
}

#[tokio::test]
async fn a_principal_wait_releases_shared_gates_for_another_creator() {
    use crate::lws::records::{LOCK_WAIT, MAX_SETTLING_PER_PRINCIPAL};
    let (state, store, unknown, second) = waiting_unknown_subscription().await;
    let a = "https://a.example/";
    let topic = state.cfg.storage();
    let container = state.cfg.absolute(SUBSCRIPTIONS_PATH);
    assert!(
        state.locks.try_lock(&container).is_some(),
        "principal wait kept the container listing lock"
    );
    assert!(
        state
            .locks
            .try_lock(&listing_lock(&container, Some(a)))
            .is_some(),
        "principal wait kept the subscriber listing lock"
    );
    *store.unsettle_creates.lock().unwrap() = None;
    *store.hold_creates.lock().unwrap() = None;
    let before = store.creates_started.load(Ordering::SeqCst);
    let body = subscription_body(&state, &topic);
    let response = call(
        &state,
        "POST",
        SUBSCRIPTIONS_PATH,
        Some(test_store::OWNER),
        &body,
    )
    .await;
    assert_eq!(
        store.creates_started.load(Ordering::SeqCst),
        before + 1,
        "another principal's create never reached the store"
    );
    assert_eq!(response.status(), StatusCode::CREATED);
    // Only A's listing is claimed: the owner's whole-listing conditional create proceeds.
    let before = store.creates_started.load(Ordering::SeqCst);
    let response = conditional_subscription(&state, test_store::OWNER, &body).await;
    assert_eq!(response.status(), StatusCode::CREATED);
    assert_eq!(store.creates_started.load(Ordering::SeqCst), before + 1);
    // A's own comparison is still deferred by its claim. Pin the detail so the principal's
    // separate settlement refusal cannot mask a missing claim check.
    let before = store.creates_started.load(Ordering::SeqCst);
    let response = response_bytes(conditional_subscription(&state, a, &body).await).await;
    assert_eq!(response.0, StatusCode::SERVICE_UNAVAILABLE);
    assert!(String::from_utf8(response.2.to_vec())
        .unwrap()
        .contains("the resource is busy"));
    assert_eq!(store.creates_started.load(Ordering::SeqCst), before);
    store.fail_fence.store(false, Ordering::SeqCst);
    unknown.add_permits(MAX_SETTLING_PER_PRINCIPAL + 1);
    let _ = tokio::time::timeout(LOCK_WAIT * 8, second)
        .await
        .unwrap()
        .unwrap();
    tokio::time::timeout(LOCK_WAIT * 8, async {
        while state.settlements.pending() != 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let before = store.creates_started.load(Ordering::SeqCst);
    let response = conditional_subscription(&state, a, &body).await;
    assert_eq!(
        response.status(),
        StatusCode::CREATED,
        "a confirmed outcome retained a listing claim"
    );
    assert_eq!(store.creates_started.load(Ordering::SeqCst), before + 1);
}

#[tokio::test]
async fn subscriber_confirmation_preserves_an_owner_s_conditional_create_window() {
    use crate::lws::records::{LOCK_WAIT, MAX_SETTLING_PER_PRINCIPAL};
    let (state, store, unknown, second) = waiting_unknown_subscription().await;
    *store.unsettle_creates.lock().unwrap() = None;
    let owner_gate = Arc::new(tokio::sync::Semaphore::new(0));
    *store.hold_creates.lock().unwrap() = Some(owner_gate.clone());
    let creating = tokio::spawn({
        let state = state.clone();
        let body = subscription_body(&state, &state.cfg.storage());
        async move {
            conditional_subscription(&state, test_store::OWNER, &body)
                .await
                .status()
        }
    });
    tokio::time::timeout(LOCK_WAIT, async {
        while store.creates_waiting.load(Ordering::SeqCst) != 1 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    *store.hold_creates.lock().unwrap() = None;
    // Let A's backend write land while its principal wait is still stalled. Hold its
    // confirmation read at the reply, so the next poll must reach publication.
    unknown.add_permits(MAX_SETTLING_PER_PRINCIPAL + 1);
    let container = state.cfg.absolute(SUBSCRIPTIONS_PATH);
    let iri = tokio::time::timeout(LOCK_WAIT, async {
        loop {
            if let Some(child) = store.list_children(&container).await.unwrap().first() {
                break child.as_str().to_string();
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let read_reply = Arc::new(tokio::sync::Semaphore::new(0));
    *store.hold_next_read_reply_of.lock().unwrap() = Some((iri, read_reply.clone()));
    store.fail_fence.store(false, Ordering::SeqCst);
    tokio::time::timeout(LOCK_WAIT * 8, async {
        while store.hold_next_read_reply_of.lock().unwrap().is_some() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    read_reply.add_permits(1);
    run_ready().await;
    assert_eq!(store.creates_waiting.load(Ordering::SeqCst), 1);
    assert!(
        !creating.is_finished(),
        "the owner's conditional window must still be open"
    );
    assert!(
        !second.is_finished(),
        "another subscriber registered inside the owner's conditional window"
    );
    owner_gate.add_permits(1);
    assert_eq!(creating.await.unwrap(), StatusCode::CREATED);
    assert_eq!(
        tokio::time::timeout(LOCK_WAIT, second)
            .await
            .unwrap()
            .unwrap(),
        StatusCode::CREATED
    );
}

async fn owner_patch_with_a_container_subscription_during_the_write(parent: bool) {
    use tower::ServiceExt;
    let (state, store) = test_store::closed_state(100).await;
    let ancestor = test_store::data(&state, "a/").await;
    let container = test_store::data(&state, "a/p/").await;
    let topic = test_store::data(&state, "a/p/r").await;
    store
        .write(
            &topic,
            Bytes::from_static(br#"{"value":0}"#),
            "application/json",
        )
        .await
        .unwrap();
    let reader = "https://reader.example/";
    test_store::grant_read(&state, reader, &[&ancestor, &container, &topic]).await;
    run_ready().await;
    *store.trace.lock().unwrap() = Some(Vec::new());
    let writing = Arc::new(tokio::sync::Semaphore::new(0));
    *store.hold_next_write_of.lock().unwrap() = Some((topic.clone(), writing.clone()));
    let patching = tokio::spawn({
        let state = state.clone();
        async move {
            let token = crate::lws::tokens::mint(
                &state.cfg,
                test_store::OWNER,
                "https://app.example/",
                &state.cfg.realm(),
            );
            let request = axum::http::Request::builder()
                .method("PATCH")
                .uri("/a/p/r")
                .header("authorization", format!("Bearer {token}"))
                .header("content-type", crate::lws::JSON_PATCH)
                .body(axum::body::Body::from(
                    r#"[{"op":"replace","path":"/value","value":1}]"#,
                ))
                .unwrap();
            axum::Router::new()
                .fallback(crate::lws::dispatch::<test_store::FlakyStore>)
                .with_state(state)
                .oneshot(request)
                .await
                .unwrap()
                .status()
        }
    });
    tokio::time::timeout(crate::lws::records::LOCK_WAIT, async {
        while store.hold_next_write_of.lock().unwrap().is_some() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(
        !store
            .trace
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .contains(&format!("read {topic}.meta")),
        "an idle owner PATCH must start without a metadata read"
    );
    let response = call(
        &state,
        "POST",
        SUBSCRIPTIONS_PATH,
        Some(reader),
        &json!({"type": streams::SSE, "topic": [if parent { container } else { ancestor }]})
            .to_string(),
    )
    .await;
    assert_eq!(response.status(), StatusCode::CREATED);
    let doc = test_store::body_json(response).await;
    let mut received = stream(&state, &doc).await;
    assert_eq!(
        state.notify.registering_non_owners.load(Ordering::SeqCst),
        0
    );
    writing.add_permits(1);
    assert_eq!(patching.await.unwrap(), StatusCode::NO_CONTENT);
    let notification = next_notification(&mut received).await;
    assert!(notification.contains("Update"));
    assert!(notification.contains(&topic));
}

#[tokio::test]
async fn owner_patch_includes_an_ancestor_subscription_registered_during_the_write() {
    owner_patch_with_a_container_subscription_during_the_write(false).await;
}

#[tokio::test]
async fn owner_patch_includes_a_parent_subscription_registered_during_the_write() {
    owner_patch_with_a_container_subscription_during_the_write(true).await;
}

#[tokio::test]
async fn a_stalled_stream_publication_releases_the_write_s_admission_slot() {
    use futures_util::FutureExt;
    use http_body_util::BodyExt;
    use tower::ServiceExt;
    let (state, _) = test_store::state(100).await;
    let topic = test_store::data(&state, "r").await;
    let response = call(
        &state,
        "POST",
        SUBSCRIPTIONS_PATH,
        Some("https://reader.example/"),
        &json!({"type": streams::SSE, "topic": [topic]}).to_string(),
    )
    .await;
    assert_eq!(response.status(), StatusCode::CREATED);
    let doc = test_store::body_json(response).await;
    let mut received = stream(&state, &doc).await;
    let id = doc["id"].as_str().unwrap().rsplit('/').next().unwrap();
    let publication = state.notify.streams.hold_publication(id).await;
    run_ready().await;
    let before = state.notify.selected.load(Ordering::SeqCst);
    let selection = Arc::new(tokio::sync::Semaphore::new(0));
    *state.notify.selection_gate.lock().unwrap() = Some(selection.clone());
    let app = crate::app::with_overload_layers(
        axum::Router::new()
            .fallback(crate::lws::dispatch::<test_store::FlakyStore>)
            .with_state(state.clone()),
        crate::app::OverloadConfig::new(1, None),
    );
    let request = axum::http::Request::builder()
        .method("PUT")
        .uri("/r")
        .header("content-type", LWS_JSON)
        .body(axum::body::Body::from("changed"))
        .unwrap();
    assert_eq!(
        app.clone().oneshot(request).await.unwrap().status(),
        StatusCode::NO_CONTENT
    );
    let get = || {
        axum::http::Request::builder()
            .uri("/r")
            .body(axum::body::Body::empty())
            .unwrap()
    };
    assert_eq!(
        app.clone().oneshot(get()).await.unwrap().status(),
        StatusCode::SERVICE_UNAVAILABLE
    );
    selection.add_permits(1);
    tokio::time::timeout(crate::lws::records::LOCK_WAIT, async {
        while state.notify.selected.load(Ordering::SeqCst) == before {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(
        app.oneshot(get()).await.unwrap().status(),
        StatusCode::OK,
        "stream publication retained the write's admission slot"
    );
    assert!(received.frame().now_or_never().is_none());
    drop(publication);
    assert!(next_notification(&mut received).await.contains("Update"));
}

#[tokio::test]
async fn a_stalled_stream_publication_releases_a_delayed_announcement_s_place() {
    use crate::lws::records::{LOCK_WAIT, MAX_SETTLING};
    let (state, store) = test_store::state(100).await;
    let container = state.cfg.absolute(crate::lws::GRANTS_PATH);
    let response = call(
        &state,
        "POST",
        SUBSCRIPTIONS_PATH,
        Some("https://reader.example/"),
        &json!({"type": streams::SSE, "topic": [container]}).to_string(),
    )
    .await;
    assert_eq!(response.status(), StatusCode::CREATED);
    let doc = test_store::body_json(response).await;
    let mut received = stream(&state, &doc).await;
    let id = doc["id"].as_str().unwrap().rsplit('/').next().unwrap();
    let publication = state.notify.streams.hold_publication(id).await;
    run_ready().await;
    let before = state.notify.selected.load(Ordering::SeqCst);
    let selection = Arc::new(tokio::sync::Semaphore::new(0));
    *state.notify.selection_gate.lock().unwrap() = Some(selection.clone());
    let writing = Arc::new(tokio::sync::Semaphore::new(0));
    *store.unsettle_creates.lock().unwrap() = Some(writing.clone());
    store.fail_fence.store(true, Ordering::SeqCst);
    let body = grant_body(&state);
    assert_eq!(
        call(&state, "POST", crate::lws::GRANTS_PATH, None, &body)
            .await
            .status(),
        StatusCode::INTERNAL_SERVER_ERROR
    );
    *store.unsettle_creates.lock().unwrap() = None;
    writing.add_permits(1);
    tokio::time::timeout(LOCK_WAIT, async {
        while store.list_children(&container).await.unwrap().is_empty() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    // Keep every other shared place occupied; only the delayed announcement can free room.
    let mut places = Vec::new();
    for _ in 0..MAX_SETTLING - 1 {
        places.push(state.settlements.reserve(None).unwrap());
    }
    store.fail_fence.store(false, Ordering::SeqCst);
    assert_eq!(
        call(&state, "POST", crate::lws::GRANTS_PATH, None, &body)
            .await
            .status(),
        StatusCode::SERVICE_UNAVAILABLE
    );
    selection.add_permits(1);
    tokio::time::timeout(LOCK_WAIT * 8, async {
        while state.notify.selected.load(Ordering::SeqCst) == before {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(
        call(&state, "POST", crate::lws::GRANTS_PATH, None, &body)
            .await
            .status(),
        StatusCode::CREATED,
        "stream publication retained the delayed announcement's shared place"
    );
    drop(publication);
    assert!(next_notification(&mut received).await.contains("Create"));
    drop(places);
    selection.add_permits(1);
}

#[tokio::test]
async fn stalled_stream_publications_have_a_storage_wide_bound() {
    use futures_util::FutureExt;
    use http_body_util::BodyExt;
    let mut cfg = LwsConfig::new("http://localhost:3000");
    cfg.open = true;
    cfg.allow_insecure_fetch = true;
    cfg.delivery.queue = 2;
    let state = LwsState::new(test_store::FlakyStore::new(), cfg)
        .await
        .unwrap();
    let mut received = Vec::new();
    let mut publications = Vec::new();
    for name in ["r", "s"] {
        let topic = test_store::data(&state, name).await;
        let response = call(
            &state,
            "POST",
            SUBSCRIPTIONS_PATH,
            Some("https://reader.example/"),
            &json!({"type": streams::SSE, "topic": [topic]}).to_string(),
        )
        .await;
        assert_eq!(response.status(), StatusCode::CREATED);
        let doc = test_store::body_json(response).await;
        received.push(stream(&state, &doc).await);
        let id = doc["id"].as_str().unwrap().rsplit('/').next().unwrap();
        publications.push(state.notify.streams.hold_publication(id).await);
    }
    run_ready().await;
    // A full webhook queue must leave the independent stream capacity available.
    let lookups = Arc::new(tokio::sync::Semaphore::new(0));
    for _ in 0..2 {
        let lookups = lookups.clone();
        state.notify.deliver_found(&state, json!({}), async move {
            lookups.acquire().await.unwrap().forget();
            None
        });
    }
    state.notify.deliver_found(&state, json!({}), async {
        panic!("the webhook queue admitted a lookup past its bound")
    });
    let dropped_before = state.notify.dropped();
    assert_eq!(dropped_before, 1);
    let before = state.notify.selected.load(Ordering::SeqCst);
    for (i, path) in ["/r", "/s", "/r", "/s"].into_iter().enumerate() {
        assert_eq!(
            call(&state, "PUT", path, None, "changed").await.status(),
            StatusCode::NO_CONTENT
        );
        tokio::time::timeout(crate::lws::records::LOCK_WAIT, async {
            while state.notify.selected.load(Ordering::SeqCst) < before + i + 1 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
    }
    assert_eq!(
        state.notify.dropped() - dropped_before,
        2,
        "publications past the storage-wide queue bound must be dropped"
    );
    drop(publications);
    for body in &mut received {
        assert!(next_notification(body).await.contains("Update"));
        assert!(body.frame().now_or_never().is_none());
    }
    // Finished publications give their own places back, independently of request admission.
    assert_eq!(
        call(&state, "PUT", "/r", None, "again").await.status(),
        StatusCode::NO_CONTENT
    );
    assert!(next_notification(&mut received[0]).await.contains("Update"));
    assert_eq!(state.notify.dropped() - dropped_before, 2);
    lookups.add_permits(2);
}

#[cfg(feature = "sparql-endpoint")]
#[tokio::test]
async fn a_principal_settlement_wait_allows_sparql_barriers() {
    use crate::lws::records::{LOCK_WAIT, MAX_SETTLING_PER_PRINCIPAL};
    async fn query_under_barrier(state: &LwsState<test_store::FlakyStore>) {
        // Force the real HTTP query through its fair barrier, rather than an optimistic read.
        let entered = Arc::new(tokio::sync::Semaphore::new(0));
        let release = Arc::new(tokio::sync::Semaphore::new(0));
        let changing = tokio::spawn({
            let (state, entered, release) = (state.clone(), entered.clone(), release.clone());
            async move {
                state
                    .store
                    .change(async {
                        entered.add_permits(1);
                        release.acquire().await.unwrap().forget();
                    })
                    .await
                    .unwrap();
            }
        });
        entered.acquire().await.unwrap().forget();
        let reading = tokio::spawn({
            let state = state.clone();
            async move {
                call(
                    &state,
                    "GET",
                    "/.lws/sparql?query=ASK%7B%7D",
                    Some(test_store::OWNER),
                    "",
                )
                .await
            }
        });
        tokio::time::timeout(LOCK_WAIT, async {
            while !state.store.barrier_pending() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        release.add_permits(1);
        changing.await.unwrap();
        let response = tokio::time::timeout(LOCK_WAIT, reading)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            response.status(),
            StatusCode::OK,
            "a settlement wait retained a query change lease"
        );
        assert_eq!(test_store::body_json(response).await["boolean"], true);
    }

    let (state, store) = test_store::closed_state(100).await;
    let a = "https://a.example/";
    let topic = state.cfg.storage();
    test_store::grant_read(&state, a, &[&topic]).await;
    let unknown = Arc::new(tokio::sync::Semaphore::new(0));
    *store.unsettle_creates.lock().unwrap() = Some(unknown.clone());
    store.fail_fence.store(true, Ordering::SeqCst);
    let body = grant_body(&state).replace("AccessGrant", "AccessRequest");
    for _ in 0..MAX_SETTLING_PER_PRINCIPAL - 1 {
        assert_eq!(
            call(&state, "POST", crate::lws::REQUESTS_PATH, Some(a), &body)
                .await
                .status(),
            StatusCode::INTERNAL_SERVER_ERROR
        );
    }
    let sending = Arc::new(tokio::sync::Semaphore::new(0));
    *store.hold_creates.lock().unwrap() = Some(sending.clone());
    let first = tokio::spawn({
        let (state, body) = (state.clone(), body.clone());
        async move {
            call(&state, "POST", crate::lws::REQUESTS_PATH, Some(a), &body)
                .await
                .status()
        }
    });
    tokio::time::timeout(LOCK_WAIT, async {
        while store.creates_waiting.load(Ordering::SeqCst) != 1 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let second = tokio::spawn({
        let state = state.clone();
        let body = subscription_body(&state, &topic);
        async move {
            call(&state, "POST", SUBSCRIPTIONS_PATH, Some(a), &body)
                .await
                .status()
        }
    });
    tokio::time::timeout(LOCK_WAIT, async {
        while store.creates_waiting.load(Ordering::SeqCst) != 2 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    sending.add_permits(1);
    assert_eq!(first.await.unwrap(), StatusCode::INTERNAL_SERVER_ERROR);
    sending.add_permits(1);
    tokio::time::timeout(LOCK_WAIT, async {
        while state.settlements.principal_waits() == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    *store.hold_creates.lock().unwrap() = None;
    *store.unsettle_creates.lock().unwrap() = None;
    assert!(!second.is_finished());
    query_under_barrier(&state).await;
    assert!(
        !second.is_finished(),
        "the principal wait must still be stalled"
    );

    // The subscriber claim excludes its conditional comparisons through publication. Its old
    // physical listing guard must not be reacquired, even while that guard is held elsewhere.
    let container = state.cfg.absolute(SUBSCRIPTIONS_PATH);
    let listing = state
        .locks
        .lock(&listing_lock(&container, Some(a)))
        .await
        .unwrap();
    unknown.add_permits(MAX_SETTLING_PER_PRINCIPAL + 1);
    tokio::time::timeout(LOCK_WAIT * 4, async {
        while store.list_children(&container).await.unwrap().is_empty() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    store.fail_fence.store(false, Ordering::SeqCst);
    assert_eq!(
        tokio::time::timeout(LOCK_WAIT * 8, second)
            .await
            .unwrap()
            .unwrap(),
        StatusCode::CREATED,
        "confirmation waited to reacquire the claimed subscriber listing"
    );
    query_under_barrier(&state).await;
    drop(listing);
}
