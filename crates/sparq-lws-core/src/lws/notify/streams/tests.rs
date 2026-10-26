use super::*;
use crate::lws::test_store::{self, FlakyStore, OWNER};
use axum::Router;
use http_body_util::BodyExt;
use tower::ServiceExt;

const READER: &str = "https://reader.example/#me";

struct Harness {
    state: LwsState<FlakyStore>,
    app: Router,
}

impl Harness {
    async fn new() -> Self {
        let (state, _) = test_store::closed_state(2000).await;
        Self::with_state(state)
    }

    fn with_state(state: LwsState<FlakyStore>) -> Self {
        let app = Router::new()
            .fallback(crate::lws::dispatch::<FlakyStore>)
            .with_state(state.clone())
            .layer(axum::middleware::from_fn(crate::ldp::cors::cors_middleware));
        Self { state, app }
    }

    async fn call(
        &self,
        method: Method,
        uri: &str,
        subject: Option<&str>,
        headers: &[(&str, &str)],
        body: &str,
    ) -> Response {
        let mut req = Request::builder().method(method).uri(uri);
        if let Some(subject) = subject {
            let token = crate::lws::tokens::mint(
                &self.state.cfg,
                subject,
                "https://client.example/id",
                &self.state.cfg.realm(),
            );
            req = req.header(header::AUTHORIZATION, format!("Bearer {token}"));
        }
        for (k, v) in headers {
            req = req.header(*k, *v);
        }
        self.app
            .clone()
            .oneshot(req.body(Body::from(body.to_string())).unwrap())
            .await
            .unwrap()
    }

    async fn subscribe_as(
        &self,
        kind: &str,
        subject: &str,
        topics: &[&str],
        expires: Option<i64>,
    ) -> Response {
        let mut doc = json!({"type": kind, "topic": topics});
        if kind == WEBHOOK {
            doc["inbox"] = json!("https://inbox.example/");
        }
        if let Some(until) = expires {
            doc["expires"] = json!(format_rfc3339(until));
        }
        self.call(
            Method::POST,
            SUBSCRIPTIONS_PATH,
            Some(subject),
            &[("content-type", LWS_JSON)],
            &doc.to_string(),
        )
        .await
    }

    async fn subscribe(&self, kind: &str, subject: &str, topics: &[&str]) -> Value {
        let response = self.subscribe_as(kind, subject, topics, None).await;
        assert_eq!(response.status(), StatusCode::CREATED);
        test_store::body_json(response).await
    }

    async fn stream(&self, doc: &Value, last: Option<&str>) -> Response {
        let mut headers = vec![
            ("accept", "text/event-stream"),
            ("origin", "https://suite.jeswr.org"),
        ];
        if let Some(last) = last {
            headers.push(("last-event-id", last));
        }
        self.call(
            Method::GET,
            doc["source"].as_str().unwrap(),
            None,
            &headers,
            "",
        )
        .await
    }

    async fn create(&self, name: &str) -> String {
        let response = self
            .call(
                Method::POST,
                "/",
                Some(OWNER),
                &[("content-type", "text/plain"), ("slug", name)],
                "contents",
            )
            .await;
        assert_eq!(response.status(), StatusCode::CREATED);
        response.headers()[header::LOCATION]
            .to_str()
            .unwrap()
            .into()
    }

    async fn update(&self, uri: &str) {
        let response = self
            .call(
                Method::PUT,
                uri,
                Some(OWNER),
                &[("content-type", "text/plain")],
                "changed",
            )
            .await;
        assert!(response.status().is_success(), "{}", response.status());
        settle().await;
    }

    async fn delete(&self, uri: &str, who: &str) {
        let response = self.call(Method::DELETE, uri, Some(who), &[], "").await;
        assert_eq!(response.status(), StatusCode::NO_CONTENT);
        settle().await;
    }

    async fn grant(&self, topics: &[&str]) -> String {
        test_store::grant_read(&self.state, READER, topics).await
    }
}

async fn settle() {
    for _ in 0..40 {
        tokio::task::yield_now().await;
    }
}

async fn frame(body: &mut Body) -> Option<String> {
    tokio::time::timeout(Duration::from_secs(2), body.frame())
        .await
        .expect("stream did not respond")
        .map(|f| String::from_utf8(f.unwrap().into_data().unwrap().to_vec()).unwrap())
}

fn notification(frame: &str) -> (&str, Value) {
    assert!(frame.contains("event: notification\n"), "{frame}");
    let id = frame.lines().find_map(|s| s.strip_prefix("id: ")).unwrap();
    let body = frame
        .lines()
        .find_map(|s| s.strip_prefix("data: "))
        .unwrap();
    (id, serde_json::from_str(body).unwrap())
}

fn subscription_id(doc: &Value) -> &str {
    doc["id"].as_str().unwrap().rsplit('/').next().unwrap()
}

async fn wait_until(mut ready: impl FnMut() -> bool) {
    tokio::time::timeout(Duration::from_secs(1), async {
        while !ready() {
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .expect("notification did not progress while replay was unread or gated");
}

// A persisted fixture sorts after every generated stream id, so dispatch tests always exercise
// a webhook that the original inline publication loop would visit after the blocked stream.
async fn webhook_after_streams(h: &Harness, topic: &str, inbox: &str) {
    let id = "~webhook";
    let mut sub: Subscription = serde_json::from_value(json!({
        "subscriber": OWNER, "topics": [topic], "inbox": inbox
    }))
    .unwrap();
    let container = h.state.cfg.absolute(SUBSCRIPTIONS_PATH);
    h.state
        .store
        .create_in_container(
            &container,
            &format!("{container}{id}"),
            Bytes::from(serde_json::to_vec(&sub).unwrap()),
            LWS_JSON,
        )
        .await
        .unwrap();
    sub.id = id.into();
    h.state.notify.subs.write().unwrap().insert(id.into(), sub);
}

#[tokio::test]
async fn unread_replay_does_not_gate_live_publication_and_drains_in_order() {
    for round in 0..3 {
        let h = Harness::new().await;
        let root = h.state.cfg.storage();
        let a = h.create("replayed").await;
        let b = h.create("live").await;
        let doc = h.subscribe(SSE, OWNER, &[&root]).await;
        let mut initial = h.stream(&doc, None).await.into_body();
        h.update(&a).await;
        let f = frame(&mut initial).await.unwrap();
        let anchor = notification(&f).0.to_string();
        let (epoch, seq) = anchor.rsplit_once('-').unwrap();
        let seq: u64 = seq.parse().unwrap();
        drop(initial);
        let slot = h.state.notify.streams.slot(subscription_id(&doc));
        // A literal count, independent of the live-queue constant, exceeds that queue's capacity.
        for _ in 0..80 {
            h.update(&a).await;
        }
        wait_until(|| slot.channel.lock().unwrap().seq == seq + 80).await;

        let observer = h.subscribe(SSE, OWNER, &[&root]).await;
        let mut observed = h.stream(&observer, None).await.into_body();
        let (sent, mut received) = mpsc::channel::<Value>(1);
        let inbox = Router::new().fallback(axum::routing::post(
            move |axum::Json(body): axum::Json<Value>| {
                let sent = sent.clone();
                async move {
                    sent.send(body).await.unwrap();
                    StatusCode::NO_CONTENT
                }
            },
        ));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move { axum::serve(listener, inbox).await.unwrap() });
        webhook_after_streams(&h, &root, &format!("http://{address}/inbox")).await;

        // Also pause a replay Watch in one round. Registration reads the root, while replay
        // rechecks this child; neither its access check nor client pacing may hold the gate.
        let guard = if round == 1 {
            Some(h.state.locks.lock(&a).await.unwrap())
        } else {
            None
        };
        let response = h.stream(&doc, Some(&anchor)).await;
        assert_eq!(response.status(), StatusCode::OK);
        let mut unread = response.into_body();
        settle().await;
        h.update(&b).await;
        wait_until(|| slot.channel.lock().unwrap().seq == seq + 81).await;
        let f = frame(&mut observed).await.unwrap();
        assert_eq!(notification(&f).1["activity"]["object"]["id"], b);
        let webhook = tokio::time::timeout(Duration::from_secs(2), received.recv())
            .await
            .expect("webhook waited for the unread stream")
            .unwrap();
        assert_eq!(webhook["activity"]["object"]["id"], b);
        drop(guard);
        for offset in 1..=81 {
            let f = frame(&mut unread).await.unwrap();
            let (id, body) = notification(&f);
            assert_eq!(id, format!("{epoch}-{}", seq + offset));
            let expected = if offset == 81 { &b } else { &a };
            assert_eq!(body["activity"]["object"]["id"], *expected);
        }
        assert!(
            tokio::time::timeout(Duration::from_millis(20), unread.frame())
                .await
                .is_err(),
            "duplicate or unexpected frame after replay and live delivery"
        );
        server.abort();
    }
}

#[tokio::test(start_paused = true)]
async fn webhook_is_queued_while_stream_publication_waits_on_its_gate() {
    let h = Harness::new().await;
    let a = h.create("a").await;
    let b = h.create("b").await;
    let root = h.state.cfg.storage();
    let doc = h.subscribe(SSE, OWNER, &[&root]).await;
    webhook_after_streams(&h, &root, "https://inbox.example/").await;
    // Keep deliveries queued rather than doing network I/O; admission is the observed behavior.
    let _workers = h
        .state
        .notify
        .workers
        .acquire_many(h.state.notify.limits.workers as u32)
        .await
        .unwrap();
    let slot = h.state.notify.streams.slot(subscription_id(&doc));
    let gate = slot.gate.lock().await;
    h.update(&a).await;
    h.update(&b).await;
    wait_until(|| h.state.notify.admitted.load(Ordering::SeqCst) == 2).await;
    assert_eq!(slot.channel.lock().unwrap().seq, 0);
    drop(gate);
    wait_until(|| slot.channel.lock().unwrap().seq == 2).await;
    let anchor = {
        let channel = slot.channel.lock().unwrap();
        let resources: Vec<_> = channel
            .ring
            .iter()
            .map(|e| {
                serde_json::from_str::<Value>(&e.body).unwrap()["activity"]["object"]["id"].clone()
            })
            .collect();
        assert_eq!(resources, vec![json!(a), json!(b)]);
        channel.ring[0].id.clone()
    };
    let mut body = h.stream(&doc, Some(&anchor)).await.into_body();
    let f = frame(&mut body).await.unwrap();
    assert_eq!(notification(&f).1["activity"]["object"]["id"], b);
    h.update(&a).await;
    let f = frame(&mut body).await.unwrap();
    assert!(notification(&f).0.ends_with("-3"));
}

#[tokio::test(start_paused = true)]
async fn retained_body_bytes_evict_oldest_events_and_reset_the_cursor() {
    let h = Harness::new().await;
    // Slugs are truncated by the HTTP create route; nested containers give the changed
    // resource a genuinely long URI while still exercising the normal HTTP write path.
    let mut a = h.state.cfg.storage();
    let slug = "a".repeat(120);
    for depth in 0..9 {
        let mut headers = vec![("content-type", "text/plain"), ("slug", slug.as_str())];
        if depth < 8 {
            headers.push(("link", "<https://www.w3.org/ns/lws#Container>; rel=type"));
        }
        let response = h
            .call(Method::POST, &a, Some(OWNER), &headers, "contents")
            .await;
        assert_eq!(response.status(), StatusCode::CREATED);
        a = response.headers()[header::LOCATION]
            .to_str()
            .unwrap()
            .to_string();
    }
    let doc = h.subscribe(SSE, OWNER, &[&a]).await;
    let mut body = h.stream(&doc, None).await.into_body();
    h.update(&a).await;
    let f = frame(&mut body).await.unwrap();
    let oldest = notification(&f).0.to_string();
    let epoch = oldest.rsplit_once('-').unwrap().0;
    drop(body);
    // Below count retention, but beyond the independently specified serialized-body byte cap.
    for _ in 0..180 {
        h.update(&a).await;
    }
    let slot = h.state.notify.streams.slot(subscription_id(&doc));
    wait_until(|| slot.channel.lock().unwrap().seq == 181).await;
    let (cursor, first, last) = {
        let channel = slot.channel.lock().unwrap();
        let bytes: usize = channel.ring.iter().map(|e| e.body.len()).sum();
        assert_eq!(channel.ring_bytes, bytes);
        assert!(bytes <= 128 * 1024, "serialized bodies exceed the byte cap");
        assert!(bytes + channel.ring.back().unwrap().body.len() > 128 * 1024);
        assert!(channel.ring.len() < 181);
        assert!(!channel.ring.iter().any(|e| e.id == oldest));
        let cursor = channel.ring.front().unwrap().id.clone();
        let first: u64 = cursor.rsplit_once('-').unwrap().1.parse().unwrap();
        (cursor, first, channel.seq)
    };
    let mut replay = h.stream(&doc, Some(&cursor)).await.into_body();
    for seq in first + 1..=last {
        let f = frame(&mut replay).await.unwrap();
        let (id, body) = notification(&f);
        assert_eq!(id, format!("{epoch}-{seq}"));
        assert_eq!(body["activity"]["object"]["id"], a);
    }
    let mut reset = h.stream(&doc, Some(&oldest)).await.into_body();
    assert_eq!(
        frame(&mut reset).await.as_deref(),
        Some("event: reset\ndata:\n\n")
    );
    h.update(&a).await;
    let f = frame(&mut reset).await.unwrap();
    assert_eq!(notification(&f).0, format!("{epoch}-182"));
    tokio::time::advance(RETENTION + Duration::from_secs(1)).await;
    h.update(&a).await;
    let channel = slot.channel.lock().unwrap();
    assert_eq!(channel.ring.len(), 1);
    assert_eq!(channel.ring_bytes, channel.ring[0].body.len());
}

#[tokio::test(start_paused = true)]
async fn sse_sq4a_container_create_and_revocation() {
    let h = Harness::new().await;
    let root = h.state.cfg.storage();
    // An untargeted grant covers the container and future children.
    let grant = json!({"@context": [LWS_CONTEXT], "type": ["AccessGrant"], "storage": root,
        "access": [{"type": ["AccessPolicy"], "action": ["read"], "assignee": READER}]});
    let response = h
        .call(
            Method::POST,
            crate::lws::GRANTS_PATH,
            Some(OWNER),
            &[("content-type", LWS_JSON)],
            &grant.to_string(),
        )
        .await;
    assert_eq!(response.status(), StatusCode::CREATED);
    let grant = response.headers()[header::LOCATION]
        .to_str()
        .unwrap()
        .to_string();
    let doc = h.subscribe(SSE, READER, &[&root]).await;
    let response = h.stream(&doc, None).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers()["access-control-allow-origin"],
        "https://suite.jeswr.org"
    );
    let mut body = response.into_body();
    settle().await;
    let child = h.create("child").await;
    let f = frame(&mut body).await.unwrap();
    let (id, envelope) = notification(&f);
    assert!(id.rsplit_once('-').unwrap().1.parse::<u64>().is_ok());
    assert_eq!(envelope["type"], "Notification");
    assert_eq!(envelope["storage"], root);
    assert_eq!(envelope["activity"]["object"]["id"], child);
    assert_eq!(envelope["activity"]["type"], json!(["Create"]));
    assert!(envelope["activity"]["object"].get("content").is_none());
    h.delete(&grant, OWNER).await;
    h.create("after-revoke").await;
    tokio::time::advance(RECHECK).await;
    settle().await;
    assert_eq!(
        frame(&mut body).await.as_deref(),
        Some("event: revoked\ndata:\n\n")
    );
    assert!(frame(&mut body).await.is_none());
    assert_eq!(h.stream(&doc, None).await.status(), StatusCode::NOT_FOUND);
}

#[tokio::test(start_paused = true)]
async fn sse_replay_reset_retention_and_single_connection() {
    let h = Harness::new().await;
    let root = h.state.cfg.storage();
    let doc = h.subscribe(SSE, OWNER, &[&root]).await;
    let mut first = h.stream(&doc, None).await.into_body();
    h.create("a").await;
    let f = frame(&mut first).await.unwrap();
    let id = notification(&f).0.to_string();
    let b = h.create("b").await;
    settle().await;
    let mut second = h.stream(&doc, Some(&id)).await.into_body();
    assert!(
        frame(&mut first).await.is_none(),
        "the first connection was not replaced"
    );
    let f = frame(&mut second).await.unwrap();
    assert_eq!(notification(&f).1["activity"]["object"]["id"], b);
    let mut other = h.stream(&doc, Some("other-epoch-1")).await.into_body();
    assert_eq!(
        frame(&mut other).await.as_deref(),
        Some("event: reset\ndata:\n\n")
    );
    tokio::time::advance(RETENTION + Duration::from_secs(1)).await;
    settle().await;
    let mut aged = h.stream(&doc, Some(&id)).await.into_body();
    assert_eq!(
        frame(&mut aged).await.as_deref(),
        Some("event: reset\ndata:\n\n")
    );
    let live = h.create("live").await;
    let f = frame(&mut aged).await.unwrap();
    assert_eq!(notification(&f).1["activity"]["object"]["id"], live);
}

#[tokio::test(start_paused = true)]
async fn sse_replay_reauthorizes_each_resource_and_filters_partial_revocation() {
    let h = Harness::new().await;
    let root = h.state.cfg.storage();
    let a = h.create("a").await;
    let b = h.create("b").await;
    h.grant(&[&root]).await;
    let ga = h.grant(&[&a]).await;
    h.grant(&[&b]).await;
    let doc = h.subscribe(SSE, READER, &[&root]).await;
    let mut body = h.stream(&doc, None).await.into_body();
    h.update(&b).await;
    let f = frame(&mut body).await.unwrap();
    let id = notification(&f).0.to_string();
    drop(body);
    h.update(&a).await;
    h.update(&b).await;
    h.delete(&ga, OWNER).await;
    let mut replay = h.stream(&doc, Some(&id)).await.into_body();
    let f = frame(&mut replay).await.unwrap();
    assert_eq!(notification(&f).1["activity"]["object"]["id"], b);
    h.update(&a).await;
    h.update(&b).await;
    let f = frame(&mut replay).await.unwrap();
    assert_eq!(notification(&f).1["activity"]["object"]["id"], b);
    tokio::time::advance(RECHECK).await;
    settle().await;
    h.update(&b).await;
    let f = frame(&mut replay).await.unwrap();
    assert_eq!(notification(&f).1["activity"]["object"]["id"], b);
}

#[tokio::test(start_paused = true)]
async fn sse_never_retains_events_from_before_read_access() {
    let h = Harness::new().await;
    let root = h.state.cfg.storage();
    h.grant(&[&root]).await;
    let a = h.create("a").await;
    let b = h.create("b").await;
    h.grant(&[&b]).await;
    let doc = h.subscribe(SSE, READER, &[&root]).await;
    let mut body = h.stream(&doc, None).await.into_body();
    h.update(&b).await;
    let f = frame(&mut body).await.unwrap();
    let id = notification(&f).0.to_string();
    drop(body);
    // Stop the background delivery at the registration gate, then grant Read after the write.
    let sub_id = doc["id"].as_str().unwrap().rsplit('/').next().unwrap();
    let slot = h.state.notify.streams.slot(sub_id);
    let guard = slot.gate.lock().await;
    h.update(&a).await;
    h.grant(&[&a]).await;
    drop(guard);
    settle().await;
    h.update(&b).await;
    let mut replay = h.stream(&doc, Some(&id)).await.into_body();
    let f = frame(&mut replay).await.unwrap();
    assert_eq!(notification(&f).1["activity"]["object"]["id"], b);
}

#[tokio::test(start_paused = true)]
async fn capabilities_are_one_time_disclosures_and_bound_to_their_transport_and_topics() {
    let h = Harness::new().await;
    let a = h.create("a").await;
    let b = h.create("b").await;
    let doc = h.subscribe(SSE, OWNER, &[&a]).await;
    let ws = h.subscribe(WEBSOCKET, OWNER, &[&b]).await;
    let source = doc["source"].as_str().unwrap();
    assert_eq!(doc["receiveFrom"], doc["source"]);
    let token = source.rsplit('/').next().unwrap();
    assert_eq!(URL_SAFE_NO_PAD.decode(token).unwrap().len(), 32);
    for uri in [doc["id"].as_str().unwrap(), SUBSCRIPTIONS_PATH] {
        let response = h.call(Method::GET, uri, Some(OWNER), &[], "").await;
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        assert!(!String::from_utf8_lossy(&bytes).contains(token));
    }
    for child in h
        .state
        .store
        .list_children(&h.state.cfg.absolute(SUBSCRIPTIONS_PATH))
        .await
        .unwrap()
    {
        let bytes = h.state.store.read(child.as_str()).await.unwrap().body;
        assert!(
            !String::from_utf8_lossy(&bytes).contains(token),
            "capability was stored in clear"
        );
    }
    for uri in [
        format!("{STREAMS}{}", "A".repeat(43)),
        format!("{SOCKETS}{token}"),
        format!(
            "{STREAMS}{}",
            ws["receiveFrom"]
                .as_str()
                .unwrap()
                .rsplit('/')
                .next()
                .unwrap()
        ),
    ] {
        assert_eq!(
            h.call(
                Method::GET,
                &uri,
                None,
                &[("accept", "text/event-stream")],
                ""
            )
            .await
            .status(),
            StatusCode::NOT_FOUND
        );
    }
    let mut body = h.stream(&doc, None).await.into_body();
    h.update(&b).await;
    h.update(&a).await;
    let f = frame(&mut body).await.unwrap();
    assert_eq!(notification(&f).1["activity"]["object"]["id"], a);
    h.delete(doc["id"].as_str().unwrap(), OWNER).await;
    assert!(frame(&mut body).await.is_none());
    assert_eq!(h.stream(&doc, None).await.status(), StatusCode::NOT_FOUND);
}

#[test]
fn legacy_webhook_record_round_trips_byte_identically() {
    let golden = br#"{"subscriber":"https://a/","client":"https://c/","topics":["https://pod/"],"inbox":"https://i/","expires":"2027-01-01T00:00:00Z","expires_at":1798761600}"#;
    let sub: Subscription = serde_json::from_slice(golden).unwrap();
    assert_eq!(sub.kind, SubscriptionKind::Webhook);
    assert_eq!(serde_json::to_vec(&sub).unwrap(), golden);
    for minimal in [
        br#"{"topics":["https://pod/"],"inbox":"https://i/"}"#.as_slice(),
        br#"{"topics":[],"inbox":""}"#.as_slice(),
    ] {
        let sub: Subscription = serde_json::from_slice(minimal).unwrap();
        assert_eq!(serde_json::to_vec(&sub).unwrap(), minimal);
    }
}

#[tokio::test(start_paused = true)]
async fn streams_share_the_existing_subscription_quota() {
    let h = Harness::new().await;
    let root = h.state.cfg.storage();
    h.grant(&[&root]).await;
    for n in 0..MAX_SUBSCRIPTIONS_PER_SUBSCRIBER {
        h.subscribe([SSE, WEBSOCKET, WEBHOOK][n % 3], READER, &[&root])
            .await;
    }
    for kind in [SSE, WEBSOCKET, WEBHOOK] {
        assert_eq!(
            h.subscribe_as(kind, READER, &[&root], None).await.status(),
            StatusCode::TOO_MANY_REQUESTS
        );
    }
}

#[tokio::test(start_paused = true)]
async fn sweeper_expires_streams_and_capabilities() {
    let h = Harness::new().await;
    let root = h.state.cfg.storage();
    let response = h
        .subscribe_as(SSE, OWNER, &[&root], Some(jose::now_secs() + 5))
        .await;
    assert_eq!(response.status(), StatusCode::CREATED);
    let doc = test_store::body_json(response).await;
    let mut body = h.stream(&doc, None).await.into_body();
    settle().await;
    h.state.notify.clock_offset.store(10, Ordering::SeqCst);
    tokio::time::advance(Duration::from_secs(3)).await;
    settle().await;
    assert!(frame(&mut body).await.is_none());
    assert_eq!(h.stream(&doc, None).await.status(), StatusCode::NOT_FOUND);
    assert!(!h
        .state
        .store
        .exists(doc["id"].as_str().unwrap())
        .await
        .unwrap());
}

#[tokio::test(start_paused = true)]
async fn slow_consumer_closes_and_retains_events_for_reconnect() {
    let h = Harness::new().await;
    let a = h.create("a").await;
    let doc = h.subscribe(SSE, OWNER, &[&a]).await;
    let mut body = h.stream(&doc, None).await.into_body();
    h.update(&a).await;
    let f = frame(&mut body).await.unwrap();
    let id = notification(&f).0.to_string();
    for _ in 0..=QUEUE {
        h.update(&a).await;
    }
    assert!(frame(&mut body).await.is_none());
    let mut replay = h.stream(&doc, Some(&id)).await.into_body();
    for _ in 0..=QUEUE {
        let f = frame(&mut replay).await.unwrap();
        assert_eq!(notification(&f).1["activity"]["object"]["id"], a);
    }
}

#[tokio::test(start_paused = true)]
async fn a_hundred_open_streams_do_not_hold_admission_permits_or_timeout_their_bodies() {
    let mut h = Harness::new().await;
    let admission = crate::overload::AdmissionControl::new(4);
    let overload = crate::app::OverloadConfig {
        admission: admission.clone(),
        ..crate::app::OverloadConfig::new(4, Some(Duration::from_secs(1)))
    };
    h.app = crate::app::with_overload_layers(h.app, overload);
    let root = h.state.cfg.storage();
    let mut bodies = Vec::new();
    for _ in 0..100 {
        let doc = h.subscribe(SSE, OWNER, &[&root]).await;
        let response = h.stream(&doc, None).await;
        assert_eq!(response.status(), StatusCode::OK);
        bodies.push(response.into_body());
    }
    assert_eq!(admission.metrics().in_flight(), 0);
    assert_eq!(
        h.call(Method::GET, "/", Some(OWNER), &[], "")
            .await
            .status(),
        StatusCode::OK
    );
    settle().await;
    tokio::time::advance(HEARTBEAT).await;
    settle().await;
    for body in &mut bodies {
        assert_eq!(frame(body).await.as_deref(), Some(": heartbeat\n\n"));
    }
}

#[tokio::test(start_paused = true)]
async fn stream_creation_requires_all_topics_and_secure_capability_base() {
    let h = Harness::new().await;
    let a = h.create("a").await;
    let b = h.create("b").await;
    h.grant(&[&a]).await;
    for kind in [SSE, WEBSOCKET] {
        assert_eq!(
            h.subscribe_as(kind, READER, &[&a, &b], None).await.status(),
            StatusCode::FORBIDDEN
        );
    }
    let store = FlakyStore::new();
    let mut cfg = LwsConfig::new("http://pod.example");
    cfg.owner = Some(OWNER.into());
    let h = Harness::with_state(LwsState::new(store, cfg).await.unwrap());
    let root = h.state.cfg.storage();
    for kind in [SSE, WEBSOCKET] {
        assert_eq!(
            h.subscribe_as(kind, OWNER, &[&root], None).await.status(),
            StatusCode::BAD_REQUEST
        );
    }
}

#[tokio::test(start_paused = true)]
async fn subscription_documents_validate_against_vendored_shapes() {
    use crate::ldp::content::{parse_to_triples, RdfFormat};
    // The WebSocket shape requires wss; exercise the deployed TLS-base configuration.
    let mut cfg = LwsConfig::new("https://pod.example");
    cfg.owner = Some(OWNER.into());
    let h = Harness::with_state(LwsState::new(FlakyStore::new(), cfg).await.unwrap());
    let shapes = sparq_shacl::load_turtle_with_base(
        include_str!("../../../../tests/fixtures/jlws-subscription.ttl"),
        "https://pod.example/",
    )
    .unwrap();
    let root = h.state.cfg.storage();
    for kind in [SSE, WEBSOCKET] {
        let mut doc = h.subscribe(kind, OWNER, &[&root]).await;
        // All binding terms are defined in the response's inline context. No remote context
        // loading is involved in this structural conformance check.
        doc["@context"] = doc["@context"][1].clone();
        let triples =
            parse_to_triples(RdfFormat::JsonLd, doc.to_string().as_bytes(), &root).unwrap();
        let graph = sparq_shacl::graph_from_triples(triples);
        let report = sparq_shacl::validate(&graph, &shapes);
        assert!(report.conforms, "{report:?}");
        // A malformed connection member must actually violate the same shapes.
        if kind == WEBSOCKET {
            doc["receiveFrom"] = json!("http://wrong.example/socket");
        } else {
            doc["topic"] = json!([]);
        }
        let triples =
            parse_to_triples(RdfFormat::JsonLd, doc.to_string().as_bytes(), &root).unwrap();
        assert!(
            !sparq_shacl::validate(&sparq_shacl::graph_from_triples(triples), &shapes).conforms
        );
    }
}

#[tokio::test(start_paused = true)]
async fn the_global_connection_cap_refuses_the_1025th_stream() {
    let (state, _) = test_store::state(2000).await;
    let h = Harness::with_state(state);
    let root = h.state.cfg.storage();
    let mut docs = Vec::new();
    // Boot accepts old stores already over their quotas. Seed that case so this test reaches
    // the independent connection ceiling rather than the earlier subscription quota.
    for n in 0..=MAX_CONNECTIONS {
        let (token, hash) = capability();
        let id = format!("seed-{n}");
        let sub: Subscription = serde_json::from_value(json!({
            "subscriber": format!("https://reader-{n}.example/"), "topics": [root],
            "kind": "Sse", "capability_hash": hash,
            "expires": format_rfc3339(jose::now_secs() + 3600), "expires_at": jose::now_secs() + 3600
        })).unwrap();
        let mut sub = sub;
        sub.id = id.clone();
        h.state.notify.subs.write().unwrap().insert(id.clone(), sub);
        h.state.notify.streams.register(&id);
        docs.push(json!({"source": h.state.cfg.absolute(&format!("{STREAMS}{token}"))}));
    }
    let mut bodies = Vec::new();
    for doc in docs.iter().take(MAX_CONNECTIONS) {
        let response = h.stream(doc, None).await;
        assert_eq!(response.status(), StatusCode::OK);
        bodies.push(response.into_body());
    }
    assert_eq!(
        h.stream(docs.last().unwrap(), None).await.status(),
        StatusCode::TOO_MANY_REQUESTS
    );
    bodies.pop();
    assert_eq!(
        h.stream(docs.last().unwrap(), None).await.status(),
        StatusCode::OK
    );
}

#[tokio::test]
async fn websocket_upgrade_text_frames_and_revocation_4403() {
    use futures_util::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::Message as ClientMessage;
    let h = Harness::new().await;
    let root = h.state.cfg.storage();
    let a = h.create("a").await;
    let b = h.create("b").await;
    let grant = h.grant(&[&root, &a, &b]).await;
    let doc = h.subscribe(WEBSOCKET, READER, &[&root]).await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = h.app.clone();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let path = url::Url::parse(doc["receiveFrom"].as_str().unwrap())
        .unwrap()
        .path()
        .to_string();
    let (mut socket, response) = tokio_tungstenite::connect_async(format!("ws://{addr}{path}"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SWITCHING_PROTOCOLS);
    socket
        .send(ClientMessage::Text("ignored".into()))
        .await
        .unwrap();
    h.update(&a).await;
    h.update(&b).await;
    for expected in [a, b] {
        let frame = tokio::time::timeout(Duration::from_secs(2), socket.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        let ClientMessage::Text(text) = frame else {
            panic!("expected a text frame: {frame:?}")
        };
        let envelope: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(envelope["activity"]["object"]["id"], expected);
        assert_eq!(envelope["activity"]["type"], json!(["Update"]));
    }
    tokio::time::pause();
    settle().await;
    h.delete(&grant, OWNER).await;
    tokio::time::advance(RECHECK).await;
    settle().await;
    let frame = tokio::time::timeout(Duration::from_secs(5), socket.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let ClientMessage::Close(Some(close)) = frame else {
        panic!("expected close: {frame:?}")
    };
    assert_eq!(u16::from(close.code), 4403);
    server.abort();
}

#[tokio::test(start_paused = true)]
async fn ring_insertion_rechecks_the_watch_after_preparation() {
    let h = Harness::new().await;
    let root = h.state.cfg.storage();
    h.grant(&[&root]).await;
    let a = h.create("a").await;
    let b = h.create("b").await;
    let grant = h.grant(&[&a]).await;
    h.grant(&[&b]).await;
    let doc = h.subscribe(SSE, READER, &[&root]).await;
    let mut body = h.stream(&doc, None).await.into_body();
    h.update(&b).await;
    let f = frame(&mut body).await.unwrap();
    let id = notification(&f).0.to_string();
    drop(body);
    let sub_id = doc["id"].as_str().unwrap().rsplit('/').next().unwrap();
    let slot = h.state.notify.streams.slot(sub_id);
    let guard = slot.gate.lock().await;
    h.update(&a).await;
    h.delete(&grant, OWNER).await;
    drop(guard);
    settle().await;
    h.grant(&[&a]).await;
    h.update(&b).await;
    let mut body = h.stream(&doc, Some(&id)).await.into_body();
    let f = frame(&mut body).await.unwrap();
    assert_eq!(notification(&f).1["activity"]["object"]["id"], b);
}

#[tokio::test(start_paused = true)]
async fn delete_deliveries_use_the_pre_delete_snapshot() {
    let h = Harness::new().await;
    let a = h.create("a").await;
    h.grant(&[&a]).await;
    let doc = h.subscribe(SSE, READER, &[&a]).await;
    let mut body = h.stream(&doc, None).await.into_body();
    h.delete(&a, OWNER).await;
    let f = frame(&mut body).await.unwrap();
    let (_, envelope) = notification(&f);
    assert_eq!(envelope["activity"]["type"], json!(["Delete"]));
    assert_eq!(envelope["activity"]["object"]["id"], a);
}

#[tokio::test(start_paused = true)]
async fn nonreader_wildcard_deletes_announce_only_present_targets() {
    let h = Harness::new().await;
    let root = h.state.cfg.storage();
    h.state.access.put_grant_for_test(
        "public-delete",
        crate::lws::access::policies(
            &json!([{"type": ["AccessPolicy"], "action": ["delete"],
                "assignee": crate::lws::FOAF_AGENT}]),
            &root,
        )
        .unwrap(),
    );
    let doc = h.subscribe(SSE, OWNER, &[&root]).await;
    let mut body = h.stream(&doc, None).await.into_body();
    let slot = h.state.notify.streams.slot(subscription_id(&doc));
    for caller in [None, Some(READER)] {
        for headers in [vec![], vec![("if-match", "*")]] {
            let target = h.create("deleted").await;
            let created = frame(&mut body).await.unwrap();
            assert_eq!(notification(&created).1["activity"]["type"], json!(["Create"]));
            let response = h.call(Method::DELETE, &target, caller, &headers, "").await;
            assert_eq!(response.status(), StatusCode::NO_CONTENT);
            let deleted = frame(&mut body).await.unwrap();
            let (_, event) = notification(&deleted);
            assert_eq!(event["activity"]["type"], json!(["Delete"]));
            assert_eq!(event["activity"]["object"]["id"], target);
            let seq = slot.channel.lock().unwrap().seq;
            let response = h.call(Method::DELETE, &target, caller, &headers, "").await;
            assert_eq!(response.status(), StatusCode::NO_CONTENT);
            // A following create must be the very next event, with no absent-delete event.
            let sentinel = h.create("sentinel").await;
            let created = frame(&mut body).await.unwrap();
            let (_, event) = notification(&created);
            assert_eq!(event["activity"]["type"], json!(["Create"]));
            assert_eq!(event["activity"]["object"]["id"], sentinel);
            assert_eq!(slot.channel.lock().unwrap().seq, seq + 1);
        }
    }
}

#[tokio::test(start_paused = true)]
async fn count_retention_and_restart_reset_the_replay_cursor() {
    let h = Harness::new().await;
    let a = h.create("a").await;
    let doc = h.subscribe(SSE, OWNER, &[&a]).await;
    let mut body = h.stream(&doc, None).await.into_body();
    h.update(&a).await;
    let f = frame(&mut body).await.unwrap();
    let old_id = notification(&f).0.to_string();
    drop(body);
    for _ in 0..=RETAINED {
        h.update(&a).await;
    }
    let mut body = h.stream(&doc, Some(&old_id)).await.into_body();
    assert_eq!(
        frame(&mut body).await.as_deref(),
        Some("event: reset\ndata:\n\n")
    );
    h.update(&a).await;
    let f = frame(&mut body).await.unwrap();
    let current_id = notification(&f).0.to_string();
    drop(body);
    let store = (*h.state.store).clone();
    let cfg = h.state.cfg.clone();
    // Detached maintenance can retain the lease after the last service handle is dropped.
    let draining = h.state.detached();
    let drain = tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        drop(draining);
    });
    drop(h);
    test_store::wait_for_stop(&store, "count_retention_and_restart_reset_the_replay_cursor").await;
    assert!(drain.is_finished(), "the detached task released its lease");
    let h = Harness::with_state(LwsState::new(store, cfg).await.unwrap());
    let mut body = h.stream(&doc, Some(&current_id)).await.into_body();
    assert_eq!(
        frame(&mut body).await.as_deref(),
        Some("event: reset\ndata:\n\n")
    );
    h.update(&a).await;
    let f = frame(&mut body).await.unwrap();
    assert_ne!(
        notification(&f).0.rsplit_once('-').unwrap().0,
        current_id.rsplit_once('-').unwrap().0
    );
}

#[tokio::test(start_paused = true)]
async fn stream_dispatch_does_not_buffer_the_incoming_body() {
    let h = Harness::new().await;
    let root = h.state.cfg.storage();
    let doc = h.subscribe(SSE, OWNER, &[&root]).await;
    let req = Request::builder()
        .uri(doc["source"].as_str().unwrap())
        .header(header::ACCEPT, "text/event-stream")
        .body(Body::from_stream(futures_util::stream::pending::<
            Result<Bytes, Infallible>,
        >()))
        .unwrap();
    let response = tokio::time::timeout(Duration::from_secs(1), h.app.oneshot(req))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test(start_paused = true)]
async fn websocket_capabilities_fail_after_expiry_or_cancellation() {
    let h = Harness::new().await;
    let root = h.state.cfg.storage();
    let doc = h.subscribe(WEBSOCKET, OWNER, &[&root]).await;
    let url = doc["receiveFrom"].as_str().unwrap();
    let path = url::Url::parse(url).unwrap().path().to_string();
    // A valid capability reaches upgrade validation; an invalid one cannot reach it.
    assert_eq!(
        h.call(Method::GET, &path, None, &[], "").await.status(),
        StatusCode::BAD_REQUEST
    );
    h.delete(doc["id"].as_str().unwrap(), OWNER).await;
    assert_eq!(
        h.call(Method::GET, &path, None, &[], "").await.status(),
        StatusCode::NOT_FOUND
    );
    let response = h
        .subscribe_as(WEBSOCKET, OWNER, &[&root], Some(jose::now_secs() + 5))
        .await;
    assert_eq!(response.status(), StatusCode::CREATED);
    let doc = test_store::body_json(response).await;
    let path = url::Url::parse(doc["receiveFrom"].as_str().unwrap())
        .unwrap()
        .path()
        .to_string();
    h.state.notify.clock_offset.store(10, Ordering::SeqCst);
    assert_eq!(
        h.call(Method::GET, &path, None, &[], "").await.status(),
        StatusCode::NOT_FOUND
    );
}

#[tokio::test(start_paused = true)]
async fn malformed_records_of_every_kind_remain_quarantined() {
    let store = FlakyStore::new();
    let mut cfg = LwsConfig::new("http://localhost:3000");
    cfg.owner = Some(OWNER.into());
    cfg.allow_insecure_fetch = true;
    let root = cfg.storage();
    let container = cfg.absolute(SUBSCRIPTIONS_PATH);
    store
        .write(&container, Bytes::new(), LWS_JSON)
        .await
        .unwrap();
    let malformed = [
        ("webhook", json!({"topics": [root]})),
        (
            "sse",
            json!({"kind": "Sse", "topics": [root], "expires_at": jose::now_secs() + 3600}),
        ),
        (
            "socket",
            json!({"kind": "WebSocket", "topics": [root], "capability_hash": vec![0u8; 32]}),
        ),
    ];
    for (id, raw) in &malformed {
        store
            .create_in_container(
                &container,
                &format!("{container}{id}"),
                Bytes::from(raw.to_string()),
                LWS_JSON,
            )
            .await
            .unwrap();
    }
    let h = Harness::with_state(LwsState::new(store, cfg).await.unwrap());
    for (id, _) in &malformed {
        let uri = format!("{container}{id}");
        assert_eq!(
            h.call(Method::GET, &uri, Some(OWNER), &[], "")
                .await
                .status(),
            StatusCode::INTERNAL_SERVER_ERROR
        );
        h.delete(&uri, OWNER).await;
        assert_eq!(
            h.call(Method::GET, &uri, Some(OWNER), &[], "")
                .await
                .status(),
            StatusCode::NOT_FOUND
        );
    }
}
