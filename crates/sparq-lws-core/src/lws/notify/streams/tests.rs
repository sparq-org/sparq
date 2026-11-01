use super::*;
use crate::lws::meta_key;
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

async fn assert_kept(h: &Harness, doc: &Value) {
    let id = subscription_id(doc);
    assert!(h.state.notify.subs.read().unwrap().contains_key(id));
    assert!(!h.state.notify.withdrawn.read().unwrap().contains(id));
    assert!(h
        .state
        .notify
        .streams
        .slots
        .lock()
        .unwrap()
        .contains_key(id));
    assert!(h
        .state
        .store
        .exists(doc["id"].as_str().unwrap())
        .await
        .unwrap());
}

type ClientSocket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

async fn open_socket(
    h: &Harness,
    doc: &Value,
) -> (ClientSocket, String, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = h.app.clone();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let path = url::Url::parse(doc["receiveFrom"].as_str().unwrap())
        .unwrap()
        .path()
        .to_string();
    let url = format!("ws://{addr}{path}");
    let (socket, response) = tokio_tungstenite::connect_async(&url).await.unwrap();
    assert_eq!(response.status(), StatusCode::SWITCHING_PROTOCOLS);
    (socket, url, server)
}

async fn close_code(socket: &mut ClientSocket) -> u16 {
    use futures_util::StreamExt;
    use tokio_tungstenite::tungstenite::Message as ClientMessage;
    let message = tokio::time::timeout(Duration::from_secs(2), socket.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let ClientMessage::Close(Some(close)) = message else {
        panic!("expected close: {message:?}")
    };
    u16::from(close.code)
}

#[tokio::test(start_paused = true)]
async fn a_set_aside_topic_does_not_withdraw_a_stream_subscription() {
    let h = Harness::new().await;
    let topic = h.create("aside").await;
    h.grant(&[&topic]).await;
    let doc = h.subscribe(SSE, READER, &[&topic]).await;
    let topics = vec![topic];
    h.state.locks.hide(&topics, HashMap::new());
    let response = h.stream(&doc, None).await;
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert!(response.headers().contains_key(header::RETRY_AFTER));
    assert_eq!(
        axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap(),
        "authorization could not be evaluated"
    );
    assert_kept(&h, &doc).await;
    h.state.locks.show(&topics);
    assert_eq!(h.stream(&doc, None).await.status(), StatusCode::OK);
}

#[tokio::test(start_paused = true)]
async fn a_store_fault_or_timeout_on_connect_keeps_the_capability() {
    let h = Harness::new().await;
    let topic = h.create("connect").await;
    h.grant(&[&topic]).await;
    let doc = h.subscribe(SSE, READER, &[&topic]).await;
    *h.state.store.fail_read_of.lock().unwrap() = Some(meta_key(&topic));
    assert_eq!(
        h.stream(&doc, None).await.status(),
        StatusCode::SERVICE_UNAVAILABLE
    );
    assert_kept(&h, &doc).await;
    *h.state.store.fail_read_of.lock().unwrap() = None;
    let guard = h.state.locks.lock(&topic).await.unwrap();
    let response = h.stream(&doc, None).await;
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert!(response.headers().contains_key(header::RETRY_AFTER));
    assert_kept(&h, &doc).await;
    drop(guard);
    assert_eq!(h.stream(&doc, None).await.status(), StatusCode::OK);
}

#[tokio::test(start_paused = true)]
async fn a_store_fault_during_the_periodic_check_closes_without_withdrawing() {
    let h = Harness::new().await;
    let topic = h.create("periodic").await;
    h.grant(&[&topic]).await;
    let doc = h.subscribe(SSE, READER, &[&topic]).await;
    let mut body = h.stream(&doc, None).await.into_body();
    settle().await;
    *h.state.store.fail_read_of.lock().unwrap() = Some(meta_key(&topic));
    tokio::time::advance(RECHECK).await;
    settle().await;
    assert!(
        frame(&mut body).await.is_none(),
        "retry must not send revoked"
    );
    assert_kept(&h, &doc).await;
    *h.state.store.fail_read_of.lock().unwrap() = None;
    assert!(h
        .state
        .store
        .exists(doc["id"].as_str().unwrap())
        .await
        .unwrap());
    let mut body = h.stream(&doc, None).await.into_body();
    h.update(&topic).await;
    assert!(frame(&mut body)
        .await
        .unwrap()
        .contains("event: notification"));
}

#[tokio::test]
async fn a_store_fault_during_the_periodic_check_closes_without_withdrawing_websocket() {
    let h = Harness::new().await;
    let topic = h.create("periodic-ws").await;
    h.grant(&[&topic]).await;
    let doc = h.subscribe(WEBSOCKET, READER, &[&topic]).await;
    let (mut socket, url, server) = open_socket(&h, &doc).await;
    tokio::time::pause();
    settle().await;
    *h.state.store.fail_read_of.lock().unwrap() = Some(meta_key(&topic));
    tokio::time::advance(RECHECK).await;
    settle().await;
    tokio::time::resume();
    assert_eq!(close_code(&mut socket).await, 1013);
    settle().await;
    assert_kept(&h, &doc).await;
    *h.state.store.fail_read_of.lock().unwrap() = None;
    let (_reconnected, response) = tokio_tungstenite::connect_async(&url).await.unwrap();
    assert_eq!(response.status(), StatusCode::SWITCHING_PROTOCOLS);
    server.abort();
}

#[tokio::test]
async fn a_websocket_retry_close_keeps_the_capability() {
    let h = Harness::new().await;
    let root = h.state.cfg.storage();
    let doc = h.subscribe(WEBSOCKET, OWNER, &[&root]).await;
    let (mut socket, url, server) = open_socket(&h, &doc).await;
    let slot = h.state.notify.streams.slot(subscription_id(&doc));
    let sender = slot.channel.lock().unwrap().sender.clone().unwrap();
    sender.close(End::Undecided);
    assert_eq!(close_code(&mut socket).await, 1013);
    settle().await;
    assert_kept(&h, &doc).await;
    let (_reconnected, response) = tokio_tungstenite::connect_async(&url).await.unwrap();
    assert_eq!(response.status(), StatusCode::SWITCHING_PROTOCOLS);
    server.abort();
}

#[tokio::test]
async fn a_slow_websocket_consumer_is_withdrawn() {
    let h = Harness::new().await;
    let root = h.state.cfg.storage();
    let doc = h.subscribe(WEBSOCKET, OWNER, &[&root]).await;
    let (mut socket, url, server) = open_socket(&h, &doc).await;
    let slot = h.state.notify.streams.slot(subscription_id(&doc));
    let sender = slot.channel.lock().unwrap().sender.clone().unwrap();
    // Fill the live queue before yielding to the socket writer.
    for _ in 0..=QUEUE {
        sender.push(Frame::Heartbeat);
    }
    assert_eq!(close_code(&mut socket).await, 1013);
    settle().await;
    assert!(!h
        .state
        .notify
        .subs
        .read()
        .unwrap()
        .contains_key(subscription_id(&doc)));
    assert!(!h
        .state
        .store
        .exists(doc["id"].as_str().unwrap())
        .await
        .unwrap());
    let error = tokio_tungstenite::connect_async(&url).await.unwrap_err();
    let tokio_tungstenite::tungstenite::Error::Http(response) = error else {
        panic!("expected an HTTP response: {error:?}")
    };
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    server.abort();
}

#[tokio::test(start_paused = true)]
async fn a_retry_end_during_a_pending_socket_write_keeps_the_capability() {
    struct PendingSocket {
        started: Option<tokio::sync::oneshot::Sender<()>>,
        finish: tokio::sync::oneshot::Receiver<Result<(), ()>>,
    }

    impl StreamSocket for PendingSocket {
        type Error = ();

        async fn receive(&mut self) -> Option<Result<Message, ()>> {
            std::future::pending().await
        }

        async fn transmit(&mut self, _: Message) -> Result<(), ()> {
            self.started.take().unwrap().send(()).unwrap();
            (&mut self.finish).await.unwrap()
        }
    }

    for end in [End::Undecided, End::Slow, End::Revoked] {
        for fails in [false, true] {
            let (started, writing) = tokio::sync::oneshot::channel();
            let (finish, pending) = tokio::sync::oneshot::channel();
            let (data_tx, data) = mpsc::channel(QUEUE);
            let (end_tx, end_rx) = mpsc::channel(1);
            let permit = Arc::new(Semaphore::new(1)).acquire_owned().await.unwrap();
            let sender = Arc::new(Sender {
                data: data_tx,
                end: end_tx,
                permit: Mutex::new(Some(permit)),
            });
            let connection = Connection {
                data: Frames {
                    replay: None,
                    live: data,
                },
                end: end_rx,
                sender: sender.clone(),
                done: false,
            };
            let task = tokio::spawn(socket_loop(
                PendingSocket {
                    started: Some(started),
                    finish: pending,
                },
                connection,
            ));
            sender.push(Frame::Heartbeat);
            writing.await.unwrap();
            sender.close(end);
            if fails {
                finish.send(Err(())).unwrap();
            } else {
                tokio::time::advance(WRITE_TIMEOUT).await;
            }
            assert_eq!(
                task.await.unwrap(),
                matches!(end, End::Undecided),
                "a pending write must preserve the terminal decision"
            );
        }
    }
}

#[tokio::test(start_paused = true)]
async fn a_periodic_check_timeout_closes_without_withdrawing() {
    let h = Harness::new().await;
    let topic = h.create("periodic-timeout").await;
    let doc = h.subscribe(SSE, OWNER, &[&topic]).await;
    let mut body = h.stream(&doc, None).await.into_body();
    settle().await;
    let guard = h.state.locks.lock(&topic).await.unwrap();
    tokio::time::advance(RECHECK).await;
    settle().await;
    tokio::time::advance(RECHECK).await;
    settle().await;
    assert!(frame(&mut body).await.is_none());
    assert_kept(&h, &doc).await;
    drop(guard);
    assert_eq!(h.stream(&doc, None).await.status(), StatusCode::OK);
}

#[tokio::test]
async fn a_revoked_reader_is_still_withdrawn() {
    let h = Harness::new().await;
    let topic = h.create("revoked").await;
    let grant = h.grant(&[&topic]).await;
    let sse = h.subscribe(SSE, READER, &[&topic]).await;
    let ws = h.subscribe(WEBSOCKET, READER, &[&topic]).await;
    let pending = h.subscribe(SSE, READER, &[&topic]).await;
    let mut body = h.stream(&sse, None).await.into_body();
    let (mut socket, _, server) = open_socket(&h, &ws).await;
    tokio::time::pause();
    settle().await;
    h.delete(&grant, OWNER).await;
    assert_eq!(
        h.stream(&pending, None).await.status(),
        StatusCode::FORBIDDEN
    );
    tokio::time::advance(RECHECK).await;
    settle().await;
    assert_eq!(
        frame(&mut body).await.as_deref(),
        Some("event: revoked\ndata:\n\n")
    );
    assert!(frame(&mut body).await.is_none());
    tokio::time::resume();
    assert_eq!(close_code(&mut socket).await, 4403);
    for doc in [&sse, &ws, &pending] {
        assert!(!h
            .state
            .notify
            .subs
            .read()
            .unwrap()
            .contains_key(subscription_id(doc)));
        assert!(!h
            .state
            .store
            .exists(doc["id"].as_str().unwrap())
            .await
            .unwrap());
    }
    server.abort();
}

#[tokio::test(start_paused = true)]
async fn an_unreadable_absent_topic_is_denied_not_undecided() {
    let h = Harness::new().await;
    let present = h.create("present").await;
    let removed = h.create("removed").await;
    let grant = h.grant(&[&present, &removed]).await;
    let existing = h.subscribe(SSE, READER, &[&present]).await;
    let absent = h.subscribe(SSE, READER, &[&removed]).await;
    h.delete(&grant, OWNER).await;
    h.delete(&removed, OWNER).await;
    for doc in [&existing, &absent] {
        let sub = h
            .state
            .notify
            .subs
            .read()
            .unwrap()
            .get(subscription_id(doc))
            .unwrap()
            .clone();
        assert_eq!(readable(&h.state, &sub).await, ReadAccess::Denied);
        let response = h.stream(doc, None).await;
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        assert!(!response.headers().contains_key(header::RETRY_AFTER));
        assert!(!h
            .state
            .notify
            .subs
            .read()
            .unwrap()
            .contains_key(subscription_id(doc)));
    }
}

#[tokio::test(start_paused = true)]
async fn readable_topics_take_priority_over_undecided_topics() {
    let h = Harness::new().await;
    let a = h.create("unknown").await;
    let b = h.create("readable").await;
    h.grant(&[&a, &b]).await;
    let doc = h.subscribe(SSE, READER, &[&a, &b]).await;
    *h.state.store.fail_read_of.lock().unwrap() = Some(meta_key(&a));
    assert_eq!(h.stream(&doc, None).await.status(), StatusCode::OK);
    *h.state.store.fail_read_of.lock().unwrap() = None;
    h.state.locks.hide(&[a], HashMap::new());
    assert_eq!(h.stream(&doc, None).await.status(), StatusCode::OK);
}

#[tokio::test(start_paused = true)]
async fn a_readable_topic_wins_while_another_topic_waits() {
    for busy_lock in [false, true] {
        let h = Harness::new().await;
        let a = h.create("pending").await;
        let b = h.create("readable").await;
        h.grant(&[&a, &b]).await;
        let doc = h.subscribe(SSE, READER, &[&a, &b]).await;
        let gate = Arc::new(Semaphore::new(0));
        let guard = if busy_lock {
            Some(h.state.locks.lock(&a).await.unwrap())
        } else {
            *h.state.store.hold_next_read_reply_of.lock().unwrap() = Some((meta_key(&a), gate));
            None
        };
        let response = h.stream(&doc, None).await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_kept(&h, &doc).await;
        drop(guard);
    }
}

#[tokio::test(start_paused = true)]
async fn revoked_hidden_and_deleted_topics_both_withdraw() {
    let h = Harness::new().await;
    let a = h.create("hidden").await;
    let b = h.create("deleted").await;
    let grant = h.grant(&[&a, &b]).await;
    let hidden = h.subscribe(SSE, READER, &[&a]).await;
    let deleted = h.subscribe(SSE, READER, &[&b]).await;
    let open = h.subscribe(SSE, READER, &[&a]).await;
    let mut body = h.stream(&open, None).await.into_body();
    settle().await;
    h.delete(&grant, OWNER).await;
    h.delete(&b, OWNER).await;
    let prior = super::super::super::Prior {
        existed: Some(true),
        meta: Some(h.state.resource_meta(&a).await.unwrap()),
        ..Default::default()
    };
    // The in-progress metadata would grant Read; the prior metadata and current grants decide.
    let mut changed = h.state.resource_meta(&a).await.unwrap();
    changed.creator = Some(READER.into());
    h.state.put_resource_meta(&a, &changed).await.unwrap();
    h.state.locks.hide(
        std::slice::from_ref(&a),
        HashMap::from([(a.clone(), prior)]),
    );
    for doc in [&hidden, &deleted] {
        assert_eq!(h.stream(doc, None).await.status(), StatusCode::FORBIDDEN);
        assert!(!h
            .state
            .notify
            .subs
            .read()
            .unwrap()
            .contains_key(subscription_id(doc)));
        assert!(!h
            .state
            .store
            .exists(doc["id"].as_str().unwrap())
            .await
            .unwrap());
    }
    tokio::time::advance(RECHECK).await;
    settle().await;
    assert_eq!(
        frame(&mut body).await.as_deref(),
        Some("event: revoked\ndata:\n\n")
    );
    assert!(frame(&mut body).await.is_none());
    assert!(!h
        .state
        .notify
        .subs
        .read()
        .unwrap()
        .contains_key(subscription_id(&open)));
    assert!(!h
        .state
        .store
        .exists(open["id"].as_str().unwrap())
        .await
        .unwrap());
    h.state.locks.show(&[a]);
}

#[tokio::test(start_paused = true)]
async fn a_revoked_reader_does_not_wait_on_a_busy_topic() {
    let h = Harness::new().await;
    let topic = h.create("busy").await;
    let grant = h.grant(&[&topic]).await;
    let doc = h.subscribe(SSE, READER, &[&topic]).await;
    h.delete(&grant, OWNER).await;
    let _guard = h.state.locks.lock(&topic).await.unwrap();
    let response = tokio::time::timeout(Duration::from_secs(1), h.stream(&doc, None))
        .await
        .expect("a denied topic must not wait for its lock");
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test(start_paused = true)]
async fn a_changing_topic_uses_prior_metadata_before_waiting() {
    let h = Harness::new().await;
    let topic = h.create("changing").await;
    let grant = h.grant(&[&topic]).await;
    let doc = h.subscribe(SSE, READER, &[&topic]).await;
    h.delete(&grant, OWNER).await;
    let prior = super::super::super::Prior {
        existed: Some(true),
        meta: Some(h.state.resource_meta(&topic).await.unwrap()),
        ..Default::default()
    };
    let mut current = h.state.resource_meta(&topic).await.unwrap();
    current.creator = Some(READER.into());
    h.state.put_resource_meta(&topic, &current).await.unwrap();
    let guard = h.state.locks.lock(&topic).await.unwrap();
    h.state
        .locks
        .changing_from(HashMap::from([(topic.clone(), prior)]));
    let response = tokio::time::timeout(Duration::from_secs(1), h.stream(&doc, None))
        .await
        .expect("prior denial must not wait for a changing topic");
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert!(!h
        .state
        .notify
        .subs
        .read()
        .unwrap()
        .contains_key(subscription_id(&doc)));
    assert!(!h
        .state
        .store
        .exists(doc["id"].as_str().unwrap())
        .await
        .unwrap());
    h.state.locks.landed(&[topic]);
    drop(guard);
}

#[tokio::test(start_paused = true)]
async fn a_hidden_topic_that_was_missing_keeps_the_capability() {
    let h = Harness::new().await;
    let topic = h.create("prior-missing").await;
    h.grant(&[&topic]).await;
    let doc = h.subscribe(SSE, READER, &[&topic]).await;
    h.state.locks.hide(
        std::slice::from_ref(&topic),
        HashMap::from([(
            topic.clone(),
            super::super::super::Prior {
                existed: Some(false),
                ..Default::default()
            },
        )]),
    );
    let response = h.stream(&doc, None).await;
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert!(response.headers().contains_key(header::RETRY_AFTER));
    assert_kept(&h, &doc).await;
    h.state.locks.show(&[topic]);
    assert_eq!(h.stream(&doc, None).await.status(), StatusCode::OK);
}

#[tokio::test]
async fn a_halted_service_closes_streams_for_retry_and_keeps_capabilities() {
    let h = Harness::new().await;
    h.state.notify.sweeper_off.store(true, Ordering::SeqCst);
    let root = h.state.cfg.storage();
    let sse = h.subscribe(SSE, OWNER, &[&root]).await;
    let ws = h.subscribe(WEBSOCKET, OWNER, &[&root]).await;
    let mut body = h.stream(&sse, None).await.into_body();
    let (mut socket, url, server) = open_socket(&h, &ws).await;
    tokio::time::pause();
    settle().await;
    h.state
        .store
        .fail_marker_write
        .store(true, Ordering::SeqCst);
    assert!(
        !h.state
            .notify
            .raise(&*h.state.store, h.state.notify.high_water() + 1)
            .await
    );
    assert!(h.state.notify.halted());
    let response = h.stream(&sse, None).await;
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert!(response.headers().contains_key(header::RETRY_AFTER));
    tokio::time::advance(RECHECK).await;
    settle().await;
    assert!(frame(&mut body).await.is_none());
    tokio::time::resume();
    assert_eq!(close_code(&mut socket).await, 1013);
    settle().await;
    for doc in [&sse, &ws] {
        assert_kept(&h, doc).await;
    }
    let error = tokio_tungstenite::connect_async(&url).await.unwrap_err();
    let tokio_tungstenite::tungstenite::Error::Http(response) = error else {
        panic!("expected an HTTP response: {error:?}")
    };
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert!(response.headers().contains_key(header::RETRY_AFTER));
    h.state
        .store
        .fail_marker_write
        .store(false, Ordering::SeqCst);
    h.state.notify.resume(&*h.state.store).await;
    assert!(!h.state.notify.halted());
    assert_eq!(h.stream(&sse, None).await.status(), StatusCode::OK);
    let (_reconnected, response) = tokio_tungstenite::connect_async(&url).await.unwrap();
    assert_eq!(response.status(), StatusCode::SWITCHING_PROTOCOLS);
    server.abort();
}

#[tokio::test(start_paused = true)]
async fn a_service_halted_while_connect_waits_on_the_gate_is_retryable() {
    let h = Harness::new().await;
    h.state.notify.sweeper_off.store(true, Ordering::SeqCst);
    let root = h.state.cfg.storage();
    let doc = h.subscribe(SSE, OWNER, &[&root]).await;
    let slot = h.state.notify.streams.slot(subscription_id(&doc));
    let gate = slot.gate.lock().await;
    let req = Request::builder()
        .uri(doc["source"].as_str().unwrap())
        .header(header::ACCEPT, "text/event-stream")
        .body(Body::empty())
        .unwrap();
    let pending = tokio::spawn(h.app.clone().oneshot(req));
    settle().await;
    assert!(!pending.is_finished());
    h.state
        .store
        .fail_marker_write
        .store(true, Ordering::SeqCst);
    assert!(
        !h.state
            .notify
            .raise(&*h.state.store, h.state.notify.high_water() + 1)
            .await
    );
    drop(gate);
    let response = pending.await.unwrap().unwrap();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert!(response.headers().contains_key(header::RETRY_AFTER));
    assert_kept(&h, &doc).await;
}

#[tokio::test(start_paused = true)]
async fn a_service_halted_during_topic_checks_keeps_capabilities() {
    for periodic in [false, true] {
        for allowed in [false, true] {
            let h = Harness::new().await;
            h.state.notify.sweeper_off.store(true, Ordering::SeqCst);
            let topic = h.create("halt-during-check").await;
            let grant = h.grant(&[&topic]).await;
            let doc = h.subscribe(SSE, READER, &[&topic]).await;
            let mut body = if periodic {
                let response = h.stream(&doc, None).await;
                assert_eq!(response.status(), StatusCode::OK);
                settle().await;
                Some(response.into_body())
            } else {
                None
            };
            if !allowed {
                h.delete(&grant, OWNER).await;
            }
            let gate = Arc::new(Semaphore::new(0));
            *h.state.store.hold_next_read_reply_of.lock().unwrap() =
                Some((meta_key(&topic), gate.clone()));
            let pending = if periodic {
                tokio::time::advance(RECHECK).await;
                None
            } else {
                let req = Request::builder()
                    .uri(doc["source"].as_str().unwrap())
                    .header(header::ACCEPT, "text/event-stream")
                    .body(Body::empty())
                    .unwrap();
                Some(tokio::spawn(h.app.clone().oneshot(req)))
            };
            wait_until(|| {
                h.state
                    .store
                    .hold_next_read_reply_of
                    .lock()
                    .unwrap()
                    .is_none()
            })
            .await;
            h.state
                .store
                .fail_marker_write
                .store(true, Ordering::SeqCst);
            assert!(
                !h.state
                    .notify
                    .raise(&*h.state.store, h.state.notify.high_water() + 1)
                    .await
            );
            assert!(h.state.notify.halted());
            gate.add_permits(1);
            if let Some(pending) = pending {
                let response = pending.await.unwrap().unwrap();
                assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
                assert!(response.headers().contains_key(header::RETRY_AFTER));
            } else {
                assert!(frame(body.as_mut().unwrap()).await.is_none());
            }
            assert_kept(&h, &doc).await;
            h.state
                .store
                .fail_marker_write
                .store(false, Ordering::SeqCst);
            h.state.notify.resume(&*h.state.store).await;
            assert!(!h.state.notify.halted());
            if !allowed {
                h.grant(&[&topic]).await;
            }
            assert_eq!(h.stream(&doc, None).await.status(), StatusCode::OK);
        }
    }
}

#[tokio::test(start_paused = true)]
async fn hidden_metadata_errors_keep_parse_failures_separate_from_store_failures() {
    for malformed in [false, true] {
        let h = Harness::new().await;
        let topic = h.create("hidden-metadata").await;
        h.grant(&[&topic]).await;
        let doc = h.subscribe(SSE, READER, &[&topic]).await;
        if malformed {
            h.state
                .store
                .write(
                    &meta_key(&topic),
                    Bytes::from_static(b"{invalid"),
                    "application/json",
                )
                .await
                .unwrap();
            let error = h.state.resource_meta(&topic).await.unwrap_err();
            assert!(matches!(
                error,
                crate::error::ServerError::InvalidMetadata(_)
            ));
            assert_eq!(error.status(), StatusCode::INTERNAL_SERVER_ERROR);
        } else {
            *h.state.store.fail_read_of.lock().unwrap() = Some(meta_key(&topic));
        }
        h.state
            .locks
            .hide(std::slice::from_ref(&topic), HashMap::new());
        let response = h.stream(&doc, None).await;
        if malformed {
            assert_eq!(response.status(), StatusCode::FORBIDDEN);
            assert!(!h
                .state
                .notify
                .subs
                .read()
                .unwrap()
                .contains_key(subscription_id(&doc)));
            assert!(!h
                .state
                .store
                .exists(doc["id"].as_str().unwrap())
                .await
                .unwrap());
        } else {
            assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
            assert!(response.headers().contains_key(header::RETRY_AFTER));
            assert_kept(&h, &doc).await;
        }
        h.state.locks.show(&[topic]);
    }
}

#[tokio::test(start_paused = true)]
async fn malformed_metadata_denies_connect_and_revokes_an_open_stream() {
    let h = Harness::new().await;
    let topic = h.create("malformed").await;
    h.grant(&[&topic]).await;
    let pending = h.subscribe(SSE, READER, &[&topic]).await;
    let open = h.subscribe(SSE, READER, &[&topic]).await;
    let mut body = h.stream(&open, None).await.into_body();
    settle().await;
    h.state
        .store
        .write(
            &meta_key(&topic),
            Bytes::from_static(b"{invalid"),
            "application/json",
        )
        .await
        .unwrap();
    assert_eq!(
        h.stream(&pending, None).await.status(),
        StatusCode::FORBIDDEN
    );
    tokio::time::advance(RECHECK).await;
    settle().await;
    assert_eq!(
        frame(&mut body).await.as_deref(),
        Some("event: revoked\ndata:\n\n")
    );
    assert!(frame(&mut body).await.is_none());
    for doc in [&pending, &open] {
        assert!(!h
            .state
            .notify
            .subs
            .read()
            .unwrap()
            .contains_key(subscription_id(doc)));
        assert!(!h
            .state
            .store
            .exists(doc["id"].as_str().unwrap())
            .await
            .unwrap());
    }
}

#[tokio::test(start_paused = true)]
async fn multi_topic_checks_keep_denied_and_transient_decisions_separate() {
    let h = Harness::new().await;
    let a = h.create("denied").await;
    let b = h.create("transient").await;
    h.grant(&[&a, &b]).await;
    let doc = h.subscribe(SSE, READER, &[&a, &b]).await;
    h.state
        .store
        .write(
            &meta_key(&a),
            Bytes::from_static(b"{invalid"),
            "application/json",
        )
        .await
        .unwrap();
    *h.state.store.fail_read_of.lock().unwrap() = Some(meta_key(&b));
    let response = h.stream(&doc, None).await;
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert!(response.headers().contains_key(header::RETRY_AFTER));
    assert_kept(&h, &doc).await;
    *h.state.store.fail_read_of.lock().unwrap() = None;
    // Any confirmed readable topic wins even when another topic's metadata is malformed.
    assert_eq!(h.stream(&doc, None).await.status(), StatusCode::OK);
}

#[tokio::test(start_paused = true)]
async fn capability_index_tracks_register_remove_and_restart() {
    fn agrees(notify: &Notifier) {
        let subs = notify.subs.read().unwrap();
        assert_eq!(subs.ordered_ids, subs.by_id.keys().cloned().collect());
        let expected: HashMap<_, _> = subs
            .values()
            .filter_map(|s| s.capability_hash.map(|hash| (hash, s.id.clone())))
            .collect();
        assert_eq!(subs.capability_index, expected);
    }
    let h = Harness::new().await;
    let root = h.state.cfg.storage();
    let sse = h.subscribe(SSE, OWNER, &[&root]).await;
    let ws = h.subscribe(WEBSOCKET, OWNER, &[&root]).await;
    h.subscribe(WEBHOOK, OWNER, &[&root]).await;
    agrees(&h.state.notify);
    assert_eq!(
        h.state.notify.subs.read().unwrap().capability_index.len(),
        2
    );
    h.delete(sse["id"].as_str().unwrap(), OWNER).await;
    agrees(&h.state.notify);
    assert_eq!(
        h.state.notify.subs.read().unwrap().capability_index.len(),
        1
    );
    let store = (*h.state.store).clone();
    let cfg = h.state.cfg.clone();
    drop(h);
    settle().await;
    let h = Harness::with_state(LwsState::new(store, cfg).await.unwrap());
    agrees(&h.state.notify);
    assert_eq!(
        h.state.notify.subs.read().unwrap().capability_index.len(),
        1
    );
    let path = url::Url::parse(ws["receiveFrom"].as_str().unwrap())
        .unwrap()
        .path()
        .to_string();
    assert_eq!(
        h.call(Method::GET, &path, None, &[], "").await.status(),
        StatusCode::BAD_REQUEST
    );

    // Loading excludes held-back and quarantined records from every in-force index.
    let notify = Notifier::new(&h.state.cfg).unwrap();
    notify
        .load_with(&*h.state.store, &h.state.cfg, |uri| {
            uri != ws["id"].as_str().unwrap()
        })
        .await
        .unwrap();
    agrees(&notify);
    assert!(notify.subs.read().unwrap().capability_index.is_empty());
    assert_eq!(notify.held_back.load(Ordering::SeqCst), 1);
    h.state
        .store
        .write(
            ws["id"].as_str().unwrap(),
            Bytes::from_static(b"{}"),
            LWS_JSON,
        )
        .await
        .unwrap();
    notify
        .load_with(&*h.state.store, &h.state.cfg, |_| true)
        .await
        .unwrap();
    agrees(&notify);
    assert!(notify.subs.read().unwrap().capability_index.is_empty());
    assert!(notify
        .quarantined
        .read()
        .unwrap()
        .contains(subscription_id(&ws)));
}

#[tokio::test(start_paused = true)]
async fn replacing_a_subscription_updates_its_capability_index() {
    let h = Harness::new().await;
    let root = h.state.cfg.storage();
    let doc = h.subscribe(SSE, OWNER, &[&root]).await;
    let id = subscription_id(&doc);
    let mut replacement = h.state.notify.subs.read().unwrap().get(id).unwrap().clone();
    let old_hash = replacement.capability_hash.unwrap();
    let (_, new_hash) = capability();
    replacement.capability_hash = Some(new_hash);
    h.state
        .notify
        .subs
        .write()
        .unwrap()
        .insert(id.into(), replacement);
    let subs = h.state.notify.subs.read().unwrap();
    assert!(!subs.capability_index.contains_key(&old_hash));
    assert_eq!(subs.by_capability(&new_hash).unwrap().id, id);
    assert_eq!(subs.capability_index.len(), 1);
}

#[tokio::test(start_paused = true)]
async fn duplicate_stored_capabilities_quarantine_one_record_and_preserve_the_other() {
    let h = Harness::new().await;
    let root = h.state.cfg.storage();
    let doc = h.subscribe(SSE, OWNER, &[&root]).await;
    let container = h.state.cfg.absolute(SUBSCRIPTIONS_PATH);
    let mut duplicate = h
        .state
        .notify
        .subs
        .read()
        .unwrap()
        .get(subscription_id(&doc))
        .unwrap()
        .clone();
    let hash = duplicate.capability_hash.unwrap();
    duplicate.id = "duplicate".into();
    let uri = format!("{container}duplicate");
    h.state
        .store
        .create_in_container(
            &container,
            &uri,
            Bytes::from(serde_json::to_vec(&duplicate).unwrap()),
            LWS_JSON,
        )
        .await
        .unwrap();
    h.state
        .notify
        .load_with(&*h.state.store, &h.state.cfg, |_| true)
        .await
        .unwrap();
    let notify = &h.state.notify;
    let (kept, quarantined) = {
        let subs = notify.subs.read().unwrap();
        assert_eq!(subs.len(), 1);
        assert_eq!(subs.capability_index.len(), 1);
        let quarantined = notify.quarantined.read().unwrap();
        assert_eq!(quarantined.len(), 1);
        (
            subs.by_capability(&hash).unwrap().id.clone(),
            quarantined.first().unwrap().clone(),
        )
    };
    assert_ne!(kept, quarantined);
    assert!(h
        .state
        .store
        .exists(&format!("{container}{quarantined}"))
        .await
        .unwrap());
    // Quarantined records belong to the owner's service listing and quota totals.
    let listing = test_store::body_json(
        h.call(Method::GET, SUBSCRIPTIONS_PATH, Some(OWNER), &[], "")
            .await,
    )
    .await;
    let listed: std::collections::BTreeSet<_> = listing["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["id"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(
        listed,
        [
            format!("{container}{kept}"),
            format!("{container}{quarantined}")
        ]
        .into()
    );
    {
        let mut subs = h.state.notify.subs.write().unwrap();
        for n in 0..MAX_OWNER_SUBSCRIPTIONS - 2 {
            let mut other = subs.get(&kept).unwrap().clone();
            other.id = format!("quota-{n}");
            other.kind = SubscriptionKind::Webhook;
            other.capability_hash = None;
            other.expires_at = None;
            subs.insert(other.id.clone(), other);
        }
    }
    assert_eq!(
        h.subscribe_as(SSE, OWNER, &[&root], None).await.status(),
        StatusCode::INSUFFICIENT_STORAGE
    );
    h.delete(&format!("{container}{quarantined}"), OWNER).await;
    assert_eq!(
        h.state
            .notify
            .subs
            .read()
            .unwrap()
            .by_capability(&hash)
            .unwrap()
            .id,
        kept
    );
    assert_eq!(
        h.subscribe_as(SSE, OWNER, &[&root], None).await.status(),
        StatusCode::CREATED
    );
    assert_eq!(h.stream(&doc, None).await.status(), StatusCode::OK);
}

#[tokio::test(start_paused = true)]
async fn duplicate_capability_insert_is_refused_and_removal_keeps_the_index_owner() {
    let h = Harness::new().await;
    let root = h.state.cfg.storage();
    let doc = h.subscribe(SSE, OWNER, &[&root]).await;
    let id = subscription_id(&doc);
    let mut subs = h.state.notify.subs.write().unwrap();
    let mut duplicate = subs.get(id).unwrap().clone();
    let hash = duplicate.capability_hash.unwrap();
    duplicate.id = "duplicate".into();
    assert!(!subs.insert(duplicate.id.clone(), duplicate.clone()));
    assert_eq!(subs.len(), 1);
    assert_eq!(subs.by_capability(&hash).unwrap().id, id);
    let mut replacement = duplicate.clone();
    replacement.id = "replacement".into();
    let other_hash = capability().1;
    replacement.capability_hash = Some(other_hash);
    assert!(subs.insert(replacement.id.clone(), replacement.clone()));
    replacement.capability_hash = Some(hash);
    assert!(!subs.insert(replacement.id.clone(), replacement.clone()));
    assert_eq!(subs.len(), 2);
    assert_eq!(subs.by_capability(&other_hash).unwrap().id, replacement.id);
    assert_eq!(subs.by_capability(&hash).unwrap().id, id);
    assert!(subs.remove(&replacement.id).is_some());
    // Also remove a second id from an older in-memory snapshot that shared this digest.
    // Its removal must preserve the mapping belonging to the retained id.
    subs.by_id.insert(duplicate.id.clone(), duplicate.clone());
    subs.ordered_ids.insert(duplicate.id.clone());
    subs.by_subscriber
        .entry(duplicate.subscriber.clone())
        .or_default()
        .insert(duplicate.id.clone());
    assert!(subs.remove(&duplicate.id).is_some());
    assert_eq!(subs.by_capability(&hash).unwrap().id, id);
    assert!(subs.remove(id).is_some());
    assert!(subs.by_capability(&hash).is_none());
}

#[tokio::test(start_paused = true)]
async fn connect_work_does_not_grow_with_subscriptions() {
    let h = Harness::new().await;
    let unknown = format!("{STREAMS}{}", "A".repeat(43));
    for count in [0, 500] {
        for n in 0..count {
            let (_, hash) = capability();
            let mut sub: Subscription = serde_json::from_value(json!({
                "subscriber": format!("https://other-{n}.example/"), "topics": [h.state.cfg.storage()],
                "kind": "Sse", "capability_hash": hash, "expires_at": jose::now_secs() + 3600
            })).unwrap();
            sub.id = format!("other-{n}");
            h.state
                .notify
                .subs
                .write()
                .unwrap()
                .insert(sub.id.clone(), sub);
        }
        {
            let subs = h.state.notify.subs.read().unwrap();
            assert_eq!(subs.len(), count);
            subs.capability_probes.store(0, Ordering::SeqCst);
            subs.record_probes.store(0, Ordering::SeqCst);
        }
        assert_eq!(
            h.call(Method::GET, &unknown, None, &[], "").await.status(),
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            h.state
                .notify
                .subs
                .read()
                .unwrap()
                .capability_probes
                .load(Ordering::SeqCst),
            1
        );
        assert!(
            h.state
                .notify
                .subs
                .read()
                .unwrap()
                .record_probes
                .load(Ordering::SeqCst)
                <= 1
        );
    }
}

#[tokio::test(start_paused = true)]
async fn a_known_capability_uses_one_record_hash_probe() {
    let h = Harness::new().await;
    let root = h.state.cfg.storage();
    let doc = h.subscribe(SSE, OWNER, &[&root]).await;
    let template = h
        .state
        .notify
        .subs
        .read()
        .unwrap()
        .get(subscription_id(&doc))
        .unwrap()
        .clone();
    let mut subs = h.state.notify.subs.write().unwrap();
    for count in [0, 500] {
        for n in 0..count {
            let mut other = template.clone();
            other.id = format!("other-{n}");
            other.subscriber = Some(format!("https://other-{n}.example/"));
            other.capability_hash = Some(capability().1);
            subs.insert(other.id.clone(), other);
        }
        subs.record_probes.store(0, Ordering::SeqCst);
        subs.capability_probes.store(0, Ordering::SeqCst);
        assert_eq!(
            subs.by_capability(&template.capability_hash.unwrap())
                .unwrap()
                .id,
            template.id
        );
        assert_eq!(subs.capability_probes.load(Ordering::SeqCst), 1);
        assert_eq!(subs.record_probes.load(Ordering::SeqCst), 1);
    }
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
    drop(h);
    settle().await;
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
