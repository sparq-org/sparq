use super::*;
use test_store::{call, response_bytes};

#[tokio::test]
async fn quota_refusals_are_indistinguishable() {
    let (state, _) = test_store::state(1000).await;
    let x = "https://x.example/";
    {
        let mut requests = state.access.requests.write().unwrap();
        for i in 0..16 {
            let id = format!("r{i:04}");
            requests.insert(
                id.clone(),
                Record {
                    id,
                    document: Value::Null,
                    policies: Vec::new(),
                    author: Some(if i < 16 {
                        x.to_string()
                    } else {
                        format!("https://p{i}.example/")
                    }),
                    etag: new_etag(),
                    quarantined: false,
                },
            );
        }
    }
    let body = access_doc("AccessRequest", "https://a/", None);
    let own = response_bytes(call(&state, "POST", REQUESTS_PATH, Some(x), &body).await).await;
    for i in 16..MAX_REQUESTS {
        let id = format!("r{i:04}");
        state.access.requests.write().unwrap().insert(
            id.clone(),
            Record {
                id,
                document: Value::Null,
                policies: Vec::new(),
                author: Some(format!("https://p{i}.example/")),
                etag: new_etag(),
                quarantined: false,
            },
        );
    }
    let full = response_bytes(
        call(
            &state,
            "POST",
            REQUESTS_PATH,
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
        .contains("this agent may not hold more open access requests now"));
}

#[tokio::test]
async fn quarantined_requests_take_no_place_from_others() {
    let (state, store) = test_store::state(1000).await;
    let container = state.cfg.absolute(REQUESTS_PATH);
    for i in 0..3 {
        store
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
        .access
        .replace(AccessStore::load(&store, &state.cfg).await.unwrap());
    let body = access_doc("AccessRequest", "https://a/", None);
    for i in 0..MAX_REQUESTS {
        let who = format!("https://p{i}.example/");
        assert_eq!(
            call(&state, "POST", REQUESTS_PATH, Some(&who), &body)
                .await
                .status(),
            StatusCode::CREATED,
            "request {i}"
        );
    }
    assert_eq!(
        call(
            &state,
            "POST",
            REQUESTS_PATH,
            Some("https://last.example/"),
            &body
        )
        .await
        .status(),
        StatusCode::TOO_MANY_REQUESTS
    );
}

#[tokio::test]
async fn quarantined_requests_take_no_place_from_the_anonymous_principal() {
    let (state, _) = test_store::state(100).await;
    for i in 0..MAX_REQUESTS_PER_AUTHOR {
        let id = format!("bad{i}");
        state.access.requests.write().unwrap().insert(
            id.clone(),
            Record {
                id,
                document: Value::Null,
                policies: Vec::new(),
                author: None,
                etag: new_etag(),
                quarantined: true,
            },
        );
    }
    let body = access_doc("AccessRequest", "https://a/", None);
    for _ in 0..MAX_REQUESTS_PER_AUTHOR {
        assert_eq!(
            call(&state, "POST", REQUESTS_PATH, None, &body)
                .await
                .status(),
            StatusCode::CREATED
        );
    }
    assert_eq!(
        call(&state, "POST", REQUESTS_PATH, None, &body)
            .await
            .status(),
        StatusCode::TOO_MANY_REQUESTS
    );
}

#[tokio::test]
async fn one_principal_cannot_take_every_settlement_place() {
    use crate::lws::records::MAX_SETTLING_PER_PRINCIPAL;
    use std::sync::{atomic::Ordering, Arc};
    let (state, store) = test_store::closed_state(100).await;
    let body = access_doc("AccessGrant", "https://a/", None);
    let gate = Arc::new(tokio::sync::Semaphore::new(0));
    *store.unsettle_creates.lock().unwrap() = Some(gate.clone());
    store.fail_fence.store(true, Ordering::SeqCst);
    let x = test_store::OWNER;
    for _ in 0..MAX_SETTLING_PER_PRINCIPAL {
        assert_eq!(
            call(&state, "POST", GRANTS_PATH, Some(x), &body)
                .await
                .status(),
            StatusCode::INTERNAL_SERVER_ERROR
        );
    }
    let principal_full =
        response_bytes(call(&state, "POST", GRANTS_PATH, Some(x), &body).await).await;
    assert_eq!(principal_full.0, StatusCode::SERVICE_UNAVAILABLE);
    *store.unsettle_creates.lock().unwrap() = None;
    assert_eq!(
        call(
            &state,
            "POST",
            REQUESTS_PATH,
            Some("https://y.example/"),
            &body.replace("AccessGrant", "AccessRequest")
        )
        .await
        .status(),
        StatusCode::CREATED
    );
    // Exercise the shared limit using the same real route and compare the full refusal.
    let mut places = Vec::new();
    for i in 0..(crate::lws::records::MAX_SETTLING - MAX_SETTLING_PER_PRINCIPAL) {
        let principal = format!("https://other{i}.example/");
        places.push(state.settlements.reserve(Some(&principal)).unwrap());
    }
    let shared_full = response_bytes(
        call(
            &state,
            "POST",
            REQUESTS_PATH,
            Some("https://z.example/"),
            &body.replace("AccessGrant", "AccessRequest"),
        )
        .await,
    )
    .await;
    assert_eq!(principal_full, shared_full);
    drop(places);
    store.fail_fence.store(false, Ordering::SeqCst);
    eventually("all principal places returned", || {
        state.settlements.pending() == 0
    })
    .await;
    assert_eq!(
        call(&state, "POST", GRANTS_PATH, Some(x), &body)
            .await
            .status(),
        StatusCode::CREATED
    );
    gate.add_permits(MAX_SETTLING_PER_PRINCIPAL);
}

#[tokio::test]
async fn one_principal_cannot_take_every_settlement_place_with_removals() {
    use crate::lws::records::MAX_SETTLING_PER_PRINCIPAL;
    use std::sync::{atomic::Ordering, Arc};
    let (state, store) = test_store::closed_state(100).await;
    let body = access_doc("AccessGrant", "https://a/", None);
    let x = test_store::OWNER;
    let mut iris = Vec::new();
    for _ in 0..MAX_SETTLING_PER_PRINCIPAL {
        let response = call(&state, "POST", GRANTS_PATH, Some(x), &body).await;
        assert_eq!(response.status(), StatusCode::CREATED);
        iris.push(
            response.headers()[header::LOCATION]
                .to_str()
                .unwrap()
                .to_string(),
        );
    }
    store.fail_fence.store(true, Ordering::SeqCst);
    for iri in iris {
        *store.unsettle_next_delete_of.lock().unwrap() =
            Some((iri.clone(), Arc::new(tokio::sync::Semaphore::new(0))));
        let path = iri.strip_prefix(&state.cfg.base_url).unwrap();
        assert_eq!(
            call(&state, "DELETE", path, Some(x), "").await.status(),
            StatusCode::INTERNAL_SERVER_ERROR
        );
    }
    assert_eq!(
        call(&state, "POST", GRANTS_PATH, Some(x), &body)
            .await
            .status(),
        StatusCode::SERVICE_UNAVAILABLE
    );
    assert_eq!(
        call(
            &state,
            "POST",
            REQUESTS_PATH,
            Some("https://y.example/"),
            &body.replace("AccessGrant", "AccessRequest")
        )
        .await
        .status(),
        StatusCode::CREATED
    );
    store.fail_fence.store(false, Ordering::SeqCst);
}

#[tokio::test]
async fn ordinary_creates_do_not_spend_principal_settlement_places() {
    use std::sync::{atomic::Ordering, Arc};
    for open in [true, false] {
        let (state, store) = if open {
            test_store::state(100).await
        } else {
            test_store::closed_state(100).await
        };
        let gate = Arc::new(tokio::sync::Semaphore::new(0));
        *store.hold_creates.lock().unwrap() = Some(gate.clone());
        let mut tasks = tokio::task::JoinSet::new();
        for _ in 0..64 {
            let state = state.clone();
            tasks.spawn(async move {
                call(
                    &state,
                    "POST",
                    GRANTS_PATH,
                    (!open).then_some(test_store::OWNER),
                    &access_doc("AccessGrant", "https://a/", None),
                )
                .await
                .status()
            });
        }
        // These are slow, successful calls, not unsettled answers. All can enter the store.
        tokio::time::timeout(crate::lws::records::LOCK_WAIT, async {
            while store.creates_waiting.load(Ordering::SeqCst) != 64 {
                assert!(
                    tasks.try_join_next().is_none(),
                    "ordinary create was refused at the principal settlement ceiling"
                );
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("ordinary creates were capped by principal settlement places");
        gate.add_permits(64);
        while let Some(result) = tasks.join_next().await {
            assert_eq!(result.unwrap(), StatusCode::CREATED);
        }
        assert_eq!(state.settlements.pending(), 0);
    }
}

#[tokio::test]
async fn open_mode_skips_the_principal_settlement_ceiling() {
    use std::sync::{atomic::Ordering, Arc};
    let (state, store) = test_store::state(100).await;
    let gate = Arc::new(tokio::sync::Semaphore::new(0));
    *store.unsettle_creates.lock().unwrap() = Some(gate.clone());
    store.fail_fence.store(true, Ordering::SeqCst);
    // Grants have no record quota; anonymous subscriptions retain their independent quota.
    for _ in 0..64 {
        assert_eq!(
            call(
                &state,
                "POST",
                GRANTS_PATH,
                None,
                &access_doc("AccessGrant", "https://a/", None)
            )
            .await
            .status(),
            StatusCode::INTERNAL_SERVER_ERROR
        );
    }
    assert_eq!(state.settlements.pending(), 64);
    *store.unsettle_creates.lock().unwrap() = None;
    store.fail_fence.store(false, Ordering::SeqCst);
    gate.add_permits(64);
}

#[tokio::test]
async fn only_the_pool_signal_becomes_an_access_request_quota_refusal() {
    let (state, store) = test_store::state(100).await;
    let body = access_doc("AccessRequest", "https://a/", None);
    *store.refuse_creates.lock().unwrap() = Some(false);
    assert_eq!(
        call(
            &state,
            "POST",
            REQUESTS_PATH,
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
            REQUESTS_PATH,
            Some("https://x.example/"),
            &body,
        )
        .await,
    )
    .await;
    // Grants have no service quota answer and retain their storage refusal.
    assert_eq!(
        call(
            &state,
            "POST",
            GRANTS_PATH,
            None,
            &access_doc("AccessGrant", "https://a/", None)
        )
        .await
        .status(),
        StatusCode::INSUFFICIENT_STORAGE
    );
    *store.refuse_creates.lock().unwrap() = None;
    for _ in 0..MAX_REQUESTS_PER_AUTHOR {
        assert_eq!(
            call(
                &state,
                "POST",
                REQUESTS_PATH,
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
            REQUESTS_PATH,
            Some("https://x.example/"),
            &body,
        )
        .await,
    )
    .await;
    assert_eq!(pool, quota);
}

#[tokio::test]
async fn concurrent_unknown_answers_wait_for_principal_settlement_places() {
    use crate::lws::records::{LOCK_WAIT, MAX_SETTLING_PER_PRINCIPAL};
    use std::sync::{atomic::Ordering, Arc};
    let (state, store) = test_store::closed_state(100).await;
    let ordinary = Arc::new(tokio::sync::Semaphore::new(0));
    *store.hold_creates.lock().unwrap() = Some(ordinary.clone());
    let mut tasks = tokio::task::JoinSet::new();
    for _ in 0..64 {
        let state = state.clone();
        tasks.spawn(async move {
            call(
                &state,
                "POST",
                GRANTS_PATH,
                Some(test_store::OWNER),
                &access_doc("AccessGrant", "https://a/", None),
            )
            .await
            .status()
        });
    }
    tokio::time::timeout(LOCK_WAIT, async {
        while store.creates_waiting.load(Ordering::SeqCst) != 64 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let unknown = Arc::new(tokio::sync::Semaphore::new(0));
    *store.unsettle_creates.lock().unwrap() = Some(unknown.clone());
    store.fail_fence.store(true, Ordering::SeqCst);
    ordinary.add_permits(64);
    for _ in 0..MAX_SETTLING_PER_PRINCIPAL {
        assert_eq!(
            tokio::time::timeout(LOCK_WAIT, tasks.join_next())
                .await
                .unwrap()
                .unwrap()
                .unwrap(),
            StatusCode::INTERNAL_SERVER_ERROR
        );
    }
    eventually("all already-running writes answered unknown", || {
        store.creates_waiting.load(Ordering::SeqCst) == 0
    })
    .await;
    assert_eq!(state.settlements.pending(), MAX_SETTLING_PER_PRINCIPAL);
    assert!(
        tasks.try_join_next().is_none(),
        "too many unknown outcomes left their admitted write tasks"
    );
    assert_eq!(
        call(
            &state,
            "POST",
            GRANTS_PATH,
            Some(test_store::OWNER),
            &access_doc("AccessGrant", "https://a/", None)
        )
        .await
        .status(),
        StatusCode::SERVICE_UNAVAILABLE
    );
    *store.unsettle_creates.lock().unwrap() = None;
    store.fail_fence.store(false, Ordering::SeqCst);
    unknown.add_permits(64);
    tokio::time::timeout(LOCK_WAIT * 8, async {
        while let Some(result) = tasks.join_next().await {
            assert!(matches!(
                result.unwrap(),
                StatusCode::CREATED | StatusCode::INTERNAL_SERVER_ERROR
            ));
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn confirmed_creates_waiting_to_register_release_principal_places() {
    use crate::lws::records::{LOCK_WAIT, MAX_SETTLING_PER_PRINCIPAL};
    use std::sync::{atomic::Ordering, Arc};
    let (state, store) = test_store::closed_state(100).await;
    let gate = Arc::new(tokio::sync::Semaphore::new(0));
    *store.unsettle_creates.lock().unwrap() = Some(gate.clone());
    store.fail_fence.store(true, Ordering::SeqCst);
    let body = json!({"type": crate::lws::notify::WEBHOOK, "topic": [state.cfg.storage()],
        "inbox": "https://inbox.example/"})
    .to_string();
    for _ in 0..MAX_SETTLING_PER_PRINCIPAL {
        assert_eq!(
            call(
                &state,
                "POST",
                crate::lws::SUBSCRIPTIONS_PATH,
                Some(test_store::OWNER),
                &body
            )
            .await
            .status(),
            StatusCode::INTERNAL_SERVER_ERROR
        );
    }
    // Subscription settlement releases this listing until registration. Hold registration
    // while the store confirms the outcomes, then use an unrelated real grant-create route.
    let container = state.cfg.absolute(crate::lws::SUBSCRIPTIONS_PATH);
    let held = state.locks.lock(&container).await.unwrap();
    *store.unsettle_creates.lock().unwrap() = None;
    gate.add_permits(MAX_SETTLING_PER_PRINCIPAL);
    let children = tokio::time::timeout(LOCK_WAIT * 4, async {
        loop {
            let children = store.list_children(&container).await.unwrap();
            if children.len() == MAX_SETTLING_PER_PRINCIPAL {
                break children;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    *store.trace.lock().unwrap() = Some(Vec::new());
    store.fail_fence.store(false, Ordering::SeqCst);
    tokio::time::timeout(LOCK_WAIT * 8, async {
        loop {
            let read = {
                let trace = store.trace.lock().unwrap();
                let trace = trace.as_ref().unwrap();
                children
                    .iter()
                    .all(|child| trace.contains(&format!("read {}", child.as_str())))
            };
            if read {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(
        call(
            &state,
            "POST",
            GRANTS_PATH,
            Some(test_store::OWNER),
            &access_doc("AccessGrant", "https://a/", None)
        )
        .await
        .status(),
        StatusCode::CREATED,
        "confirmed outcomes still charged principal places while waiting to register"
    );
    drop(held);
}
