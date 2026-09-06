// AUTHORED-BY Claude Opus 4.8
//! End-to-end HTTP tests for the WebSocketChannel2023 notification surface through the assembled
//! router: discovery (the `/.well-known/solid` storage description + the LDP read `Link` rels), the
//! auth- and WAC-gated subscribe handshake (anonymous refused, an authenticated non-reader refused,
//! an authorized subscriber returns `receiveFrom`), and a
//! default-`#[ignore]`d live WebSocket handshake that exercises subscribe → connect → mutate →
//! receive against a really-bound server.
//!
//! Each authenticated request carries a fresh DPoP-bound token + a per-request proof (a new jti) so
//! the verifier's single-use replay protection does not reject back-to-back requests.

mod common;

use std::sync::Arc;

use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use common::{
    jwks_provider, mint_access_token, mint_access_token_for, mint_dpop_proof, KeyKit, BASE_URL,
};
use solid_oidc_verifier::config::VerifierConfig;
use solid_oidc_verifier::replay::InMemoryReplayStore;
use solid_oidc_verifier::verifier::Verifier;
use sparq_lws_core::app::{build_router, AppState};
use sparq_lws_core::auth::AuthContext;
use sparq_lws_core::ldp::handler::LdpState;
use sparq_lws_core::notifications::activity::ActivityType;
use sparq_lws_core::notifications::ws::NotificationMetrics;
use sparq_lws_core::notifications::NotificationHub;
use sparq_lws_core::store::{CompositeStore, InMemoryBlobStore, InMemorySparqClient};
use tower::ServiceExt;

const WS_TYPE: &str = "http://www.w3.org/ns/solid/notifications#WebSocketChannel2023";

/// A oneshot-driven app + the keys to mint authenticated requests. (For the live WS test a separately
/// bound server is used — see `live_websocket_*`.)
struct Harness {
    app: axum::Router,
    issuer_key: KeyKit,
    client_key: KeyKit,
}

impl Harness {
    async fn new() -> Self {
        use sparq_lws_core::store::Store;
        let issuer_key = KeyKit::generate();
        let client_key = KeyKit::generate();
        let config = VerifierConfig::new(vec![common::ISSUER.to_string()], BASE_URL);
        let replay = InMemoryReplayStore::with_window(config.replay_ttl());
        let verifier = Verifier::new(config, jwks_provider(&issuer_key), replay).unwrap();
        let ctx = AuthContext::new(verifier, BASE_URL);
        let store = CompositeStore::new(InMemorySparqClient::new(), InMemoryBlobStore::new());
        // WAC is enforced, so the authenticated test caller (Alice, `common::WEBID`) needs an effective
        // ACL. Seed a ROOT `.acl` granting Alice Read/Write/Control on the base root + `acl:default`
        // (every descendant inherits owner control) so PUT/GET on `/alice/data` are authorized.
        let acl_iri = format!("{BASE_URL}/.acl");
        let acl_body = format!(
            r#"@prefix acl: <http://www.w3.org/ns/auth/acl#>.
<#owner> a acl:Authorization;
         acl:agent <{}>;
         acl:accessTo <{BASE_URL}/>;
         acl:default <{BASE_URL}/>;
         acl:mode acl:Read, acl:Write, acl:Control."#,
            common::WEBID
        );
        store
            .write(&acl_iri, axum::body::Bytes::from(acl_body), "text/turtle")
            .await
            .expect("seed root owner acl");
        let ldp = LdpState::new(store, BASE_URL);
        let app = build_router(AppState {
            auth: Arc::new(ctx),
            ldp: Arc::new(ldp),
            identity: None,
        });
        Self {
            app,
            issuer_key,
            client_key,
        }
    }

    fn auth_headers(&self, method: &str, path: &str) -> (String, String) {
        let access = mint_access_token(&self.issuer_key, &self.client_key.thumbprint);
        let htu = format!("{BASE_URL}{path}");
        let proof = mint_dpop_proof(&self.client_key, method, &htu, &access);
        (format!("DPoP {access}"), proof)
    }

    /// An authenticated request.
    async fn auth_request(
        &self,
        method: &str,
        path: &str,
        content_type: Option<&str>,
        body: Body,
    ) -> axum::http::Response<Body> {
        let (authz, dpop) = self.auth_headers(method, path);
        let mut builder = Request::builder()
            .method(method)
            .uri(path)
            .header("authorization", authz)
            .header("dpop", dpop);
        if let Some(ct) = content_type {
            builder = builder.header("content-type", ct);
        }
        self.app
            .clone()
            .oneshot(builder.body(body).unwrap())
            .await
            .unwrap()
    }

    /// An authenticated request from a DIFFERENT WebID than [`common::WEBID`] — the pod's root
    /// `.acl` grants ONLY Alice, so this agent is authenticated but holds no mode anywhere.
    async fn auth_request_as(
        &self,
        web_id: &str,
        method: &str,
        path: &str,
        content_type: Option<&str>,
        body: Body,
    ) -> axum::http::Response<Body> {
        let access =
            mint_access_token_for(&self.issuer_key, &self.client_key.thumbprint, web_id);
        let htu = format!("{BASE_URL}{path}");
        let proof = mint_dpop_proof(&self.client_key, method, &htu, &access);
        let mut builder = Request::builder()
            .method(method)
            .uri(path)
            .header("authorization", format!("DPoP {access}"))
            .header("dpop", proof);
        if let Some(ct) = content_type {
            builder = builder.header("content-type", ct);
        }
        self.app
            .clone()
            .oneshot(builder.body(body).unwrap())
            .await
            .unwrap()
    }

    /// An UNAUTHENTICATED request (no Authorization / DPoP).
    async fn anon_request(
        &self,
        method: &str,
        path: &str,
        content_type: Option<&str>,
        body: Body,
    ) -> axum::http::Response<Body> {
        let mut builder = Request::builder().method(method).uri(path);
        if let Some(ct) = content_type {
            builder = builder.header("content-type", ct);
        }
        self.app
            .clone()
            .oneshot(builder.body(body).unwrap())
            .await
            .unwrap()
    }
}

async fn body_string(resp: axum::http::Response<Body>) -> String {
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    String::from_utf8(bytes.to_vec()).unwrap()
}

#[tokio::test]
async fn well_known_solid_advertises_the_subscription_service() {
    let h = Harness::new().await;
    // Discovery is PUBLIC — no auth needed.
    let resp = h
        .anon_request("GET", "/.well-known/solid", None, Body::empty())
        .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_string(resp).await;
    assert!(body.contains("notificationChannel"), "{body}");
    assert!(body.contains(WS_TYPE), "{body}");
    assert!(
        body.contains("https://pod.example/.notifications/WebSocketChannel2023/"),
        "{body}"
    );
}

#[tokio::test]
async fn ldp_read_advertises_discovery_link_headers() {
    let h = Harness::new().await;
    // Create a resource, then GET it and inspect the Link headers.
    let put = h
        .auth_request(
            "PUT",
            "/alice/data",
            Some("text/turtle"),
            Body::from("<#me> <http://xmlns.com/foaf/0.1/name> \"Alice\" ."),
        )
        .await;
    assert_eq!(put.status(), StatusCode::CREATED);

    let get = h
        .auth_request("GET", "/alice/data", None, Body::empty())
        .await;
    assert_eq!(get.status(), StatusCode::OK);
    let links: Vec<String> = get
        .headers()
        .get_all(axum::http::header::LINK)
        .iter()
        .map(|v| v.to_str().unwrap().to_string())
        .collect();
    let all = links.join(" | ");
    assert!(all.contains("rel=\"describedby\""), "{all}");
    assert!(all.contains("storageDescription"), "{all}");
    assert!(all.contains("/.well-known/solid"), "{all}");
}

#[tokio::test]
async fn subscribe_rejects_anonymous_caller() {
    let h = Harness::new().await;
    let body = serde_json::json!({
        "@context": "https://www.w3.org/ns/solid/notifications-context/v1",
        "type": WS_TYPE,
        "topic": "https://pod.example/alice/data",
    })
    .to_string();
    let resp = h
        .anon_request(
            "POST",
            "/.notifications/WebSocketChannel2023/",
            Some("application/ld+json"),
            Body::from(body),
        )
        .await;
    // Fail-closed: no anonymous subscriptions.
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn subscribe_authenticated_returns_receive_from() {
    let h = Harness::new().await;
    let body = serde_json::json!({
        "@context": "https://www.w3.org/ns/solid/notifications-context/v1",
        "type": WS_TYPE,
        "topic": "https://pod.example/alice/data",
    })
    .to_string();
    let resp = h
        .auth_request(
            "POST",
            "/.notifications/WebSocketChannel2023/",
            Some("application/ld+json"),
            Body::from(body),
        )
        .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let out = body_string(resp).await;
    assert!(out.contains("\"receiveFrom\""), "{out}");
    assert!(
        out.contains("wss://pod.example/.notifications/WebSocketChannel2023/receive"),
        "{out}"
    );
    assert!(out.contains(WS_TYPE), "{out}");
}

/// Per-resource WAC through the ASSEMBLED router: a WebID that authenticates successfully but holds
/// no `acl:Read` on Alice's resource is refused the subscription — so it never obtains a
/// `receiveFrom` URL and never receives that resource's change notifications.
#[tokio::test]
async fn subscribe_denies_authenticated_non_reader() {
    const MALLORY: &str = "https://mallory.example/profile/card#me";
    let h = Harness::new().await;
    // Alice creates the resource she owns.
    let put = h
        .auth_request(
            "PUT",
            "/alice/data",
            Some("text/turtle"),
            Body::from("<#me> <http://xmlns.com/foaf/0.1/name> \"Alice\" ."),
        )
        .await;
    assert_eq!(put.status(), StatusCode::CREATED);

    let body = serde_json::json!({
        "@context": "https://www.w3.org/ns/solid/notifications-context/v1",
        "type": WS_TYPE,
        "topic": "https://pod.example/alice/data",
    })
    .to_string();
    let resp = h
        .auth_request_as(
            MALLORY,
            "POST",
            "/.notifications/WebSocketChannel2023/",
            Some("application/ld+json"),
            Body::from(body),
        )
        .await;
    assert_eq!(
        resp.status(),
        StatusCode::FORBIDDEN,
        "an authenticated non-reader must not be able to subscribe to Alice's resource"
    );
    let out = body_string(resp).await;
    assert!(
        !out.contains("receiveFrom"),
        "a denied subscribe must not hand back a receiveFrom URL: {out}"
    );
}

// --- Receive endpoint token-gate (the HIGH-finding fix; non-ignored, via oneshot) ---------------

/// Build a GET request that LOOKS like a WS upgrade for the receive endpoint with the given query.
fn ws_upgrade_request(path_and_query: &str) -> Request<Body> {
    Request::builder()
        .method("GET")
        .uri(path_and_query)
        .header("connection", "upgrade")
        .header("upgrade", "websocket")
        .header("sec-websocket-version", "13")
        .header("sec-websocket-key", "dGhlIHNhbXBsZSBub25jZQ==")
        .body(Body::empty())
        .unwrap()
}

/// MUTATION-CHECK for the fix: the receive endpoint with NO token must be REJECTED (401). The pre-fix
/// topic-only receive would have upgraded (101) here — so this test fails against the vulnerable code.
#[tokio::test]
async fn receive_without_token_is_rejected() {
    let h = Harness::new().await;
    let req = ws_upgrade_request(
        "/.notifications/WebSocketChannel2023/receive?topic=https://pod.example/alice/data",
    );
    let resp = h.app.clone().oneshot(req).await.unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::UNAUTHORIZED,
        "an unauthenticated/tokenless receive must be refused (no open-receive bypass)"
    );
}

/// An INVALID (never-minted) token is rejected.
#[tokio::test]
async fn receive_with_invalid_token_is_rejected() {
    let h = Harness::new().await;
    let req = ws_upgrade_request(
        "/.notifications/WebSocketChannel2023/receive?topic=https://pod.example/alice/data&token=bogus-token",
    );
    let resp = h.app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

/// A VALID token (from an authenticated subscribe) bound to topic A must NOT authorize receive on a
/// DIFFERENT topic B — the topic-binding is enforced end-to-end through the router.
#[tokio::test]
async fn receive_with_valid_token_for_wrong_topic_is_rejected() {
    let h = Harness::new().await;
    // Subscribe (authenticated) to topic A to mint a real token.
    let topic_a = "https://pod.example/alice/data";
    let sub_body = serde_json::json!({ "type": WS_TYPE, "topic": topic_a }).to_string();
    let sub = h
        .auth_request(
            "POST",
            "/.notifications/WebSocketChannel2023/",
            Some("application/ld+json"),
            Body::from(sub_body),
        )
        .await;
    assert_eq!(sub.status(), StatusCode::OK);
    let channel: serde_json::Value = serde_json::from_str(&body_string(sub).await).unwrap();
    let receive_from = channel["receiveFrom"].as_str().unwrap().to_string();
    // Extract the minted token from the receiveFrom URL's `&token=` param.
    let token = receive_from
        .split("token=")
        .nth(1)
        .expect("receiveFrom carries a token")
        .to_string();

    // Present that valid token but for a DIFFERENT topic B → rejected.
    let req = ws_upgrade_request(&format!(
        "/.notifications/WebSocketChannel2023/receive?topic=https://pod.example/alice/OTHER&token={token}"
    ));
    let resp = h.app.clone().oneshot(req).await.unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::UNAUTHORIZED,
        "a token bound to topic A must not authorize receive on topic B"
    );
}

// --- Live WebSocket end-to-end (default-ignored; binds a real server) ---------------------------

/// A really-bound server whose base URL matches the bound address, so DPoP htu + the WS upgrade work
/// against `127.0.0.1:<port>`. Returns the base URL + the keys.
async fn bind_live_server() -> (String, KeyKit, KeyKit, NotificationHub) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let base_url = format!("http://{addr}");

    let issuer_key = KeyKit::generate();
    let client_key = KeyKit::generate();
    // The audience must match the minted token's `aud` (the `BASE_URL` const), but the AuthContext's
    // base_url must be the BOUND address so the reconstructed DPoP `htu` matches the real request URL.
    let config = VerifierConfig::new(vec![common::ISSUER.to_string()], BASE_URL);
    let replay = InMemoryReplayStore::with_window(config.replay_ttl());
    let verifier = Verifier::new(config, jwks_provider(&issuer_key), replay).unwrap();
    let ctx = AuthContext::new(verifier, base_url.clone());
    let store = CompositeStore::new(InMemorySparqClient::new(), InMemoryBlobStore::new());
    // WAC is enforced: seed a root owner ACL so the authenticated PUT/GET in the live test are allowed
    // (the WebID is `common::WEBID`; the base is the bound address).
    {
        use sparq_lws_core::store::Store;
        let acl_iri = format!("{base_url}/.acl");
        let acl_body = format!(
            r#"@prefix acl: <http://www.w3.org/ns/auth/acl#>.
<#owner> a acl:Authorization;
         acl:agent <{}>;
         acl:accessTo <{base_url}/>;
         acl:default <{base_url}/>;
         acl:mode acl:Read, acl:Write, acl:Control."#,
            common::WEBID
        );
        store
            .write(&acl_iri, axum::body::Bytes::from(acl_body), "text/turtle")
            .await
            .expect("seed live root owner acl");
    }
    let ldp = LdpState::new(store, base_url.clone());
    let notification_hub = ldp.notifications.clone();
    let app = build_router(AppState {
        auth: Arc::new(ctx),
        ldp: Arc::new(ldp),
        identity: None,
    });

    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    (base_url, issuer_key, client_key, notification_hub)
}

// [GPT-5.6] A current-thread runtime makes the burst deterministic: the receive task cannot drain
// the broadcast channel until this task yields after filling it beyond its fixed capacity.
#[tokio::test(flavor = "current_thread")]
async fn live_websocket_overflow_closes_and_increments_metrics() {
    use futures_util::StreamExt;
    use tokio_tungstenite::tungstenite::Message as WsMessage;

    let before = NotificationMetrics::snapshot();
    let (base_url, issuer_key, client_key, hub) = bind_live_server().await;
    let topic = format!("{base_url}/alice/overflow");
    let access = mint_access_token(&issuer_key, &client_key.thumbprint);
    let htu = format!("{base_url}/.notifications/WebSocketChannel2023/");
    let proof = mint_dpop_proof(&client_key, "POST", &htu, &access);
    let sub = reqwest_like_client()
        .post(htu)
        .header("authorization", format!("DPoP {access}"))
        .header("dpop", proof)
        .header("content-type", "application/ld+json")
        .body(
            serde_json::json!({
                "type": WS_TYPE,
                "topic": topic,
            })
            .to_string(),
        )
        .send()
        .await
        .unwrap();
    assert_eq!(sub.status(), 200);
    let channel: serde_json::Value = sub.json().await.unwrap();
    let receive_from = channel["receiveFrom"].as_str().unwrap();
    let (mut ws, _) = tokio_tungstenite::connect_async(receive_from)
        .await
        .expect("ws connects");

    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while hub.subscriber_count(&topic).await != 1 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("subscriber registered");

    for _ in 0..65 {
        hub.notify(&topic, ActivityType::Update, None).await;
    }

    let frame = tokio::time::timeout(std::time::Duration::from_secs(5), ws.next())
        .await
        .expect("overflow close arrived")
        .expect("websocket stream yielded a frame")
        .expect("close frame is valid");
    match frame {
        WsMessage::Close(Some(close)) => {
            assert_eq!(
                close.code,
                tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode::Error
            );
            assert_eq!(
                close.reason,
                "notification backlog overflow; reconnect and reconcile"
            );
        }
        other => panic!("expected overflow close frame, got {other:?}"),
    }

    let after = NotificationMetrics::snapshot();
    assert_eq!(
        after.dropped_subscribers_total,
        before.dropped_subscribers_total + 1
    );
    assert_eq!(after.lagged_events_total, before.lagged_events_total + 1);
}

/// Default-ignored: a full subscribe → connect WebSocket → PUT → receive an AS2.0 Update over the
/// socket. Ignored because it depends on transport timing + a really-bound listener; run explicitly
/// with `cargo test --test notifications_http -- --ignored`.
#[tokio::test]
#[ignore = "live WebSocket handshake; run with --ignored"]
async fn live_websocket_delivers_update_on_put() {
    use futures_util::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::Message as WsMessage;

    let (base_url, issuer_key, client_key, _hub) = bind_live_server().await;
    let topic = format!("{base_url}/alice/data");

    let auth = |method: &str, path: &str| {
        let access = mint_access_token(&issuer_key, &client_key.thumbprint);
        let htu = format!("{base_url}{path}");
        let proof = mint_dpop_proof(&client_key, method, &htu, &access);
        (format!("DPoP {access}"), proof)
    };

    let http = reqwest_like_client();

    // 1. Subscribe (authenticated) → get the receiveFrom URL.
    let (authz, dpop) = auth("POST", "/.notifications/WebSocketChannel2023/");
    let sub_body = serde_json::json!({
        "type": WS_TYPE,
        "topic": topic,
    })
    .to_string();
    let sub = http
        .post(format!("{base_url}/.notifications/WebSocketChannel2023/"))
        .header("authorization", authz)
        .header("dpop", dpop)
        .header("content-type", "application/ld+json")
        .body(sub_body)
        .send()
        .await
        .unwrap();
    assert_eq!(sub.status(), 200);
    let channel: serde_json::Value = sub.json().await.unwrap();
    let receive_from = channel["receiveFrom"].as_str().unwrap().to_string();
    // The receiveFrom URL now carries the minted receive token (the WS receive endpoint is
    // token-gated). Without it the connect below would be refused.
    assert!(
        receive_from.contains("&token="),
        "receiveFrom must carry a receive token: {receive_from}"
    );

    // 1b. A connect WITHOUT the token is refused (the open-receive bypass is closed). Strip the
    // `&token=…` query param and confirm the handshake fails.
    let no_token_url = receive_from
        .split("&token=")
        .next()
        .expect("split keeps the pre-token portion")
        .to_string();
    assert!(
        tokio_tungstenite::connect_async(&no_token_url)
            .await
            .is_err(),
        "a tokenless receive connect must be refused"
    );

    // 2. Connect the WebSocket to receiveFrom (with the valid token).
    let (mut ws, _) = tokio_tungstenite::connect_async(&receive_from)
        .await
        .expect("ws connects");

    // Give the receive task a moment to register the subscriber before we mutate.
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    // 3. PUT the topic resource (authenticated).
    let (authz, dpop) = auth("PUT", "/alice/data");
    let put = http
        .put(topic.clone())
        .header("authorization", authz)
        .header("dpop", dpop)
        .header("content-type", "text/turtle")
        .body("<#me> <http://xmlns.com/foaf/0.1/name> \"Alice\" .")
        .send()
        .await
        .unwrap();
    assert!(put.status().is_success(), "PUT status {}", put.status());

    // 4. Receive the AS2.0 notification over the socket (with a timeout so a miss fails, not hangs).
    let frame = tokio::time::timeout(std::time::Duration::from_secs(5), ws.next())
        .await
        .expect("a notification arrived within the timeout")
        .expect("the stream yielded a frame")
        .expect("the frame is Ok");
    let text = match frame {
        WsMessage::Text(t) => t.to_string(),
        other => panic!("expected a text notification, got {other:?}"),
    };
    // A PUT-create ⇒ Create on the resource.
    assert!(text.contains("\"type\":\"Create\""), "{text}");
    assert!(text.contains(&format!("\"object\":\"{topic}\"")), "{text}");

    let _ = ws.send(WsMessage::Close(None)).await;
}

/// A tiny HTTP client over reqwest (already in the resolved tree via the verifier's network feature).
fn reqwest_like_client() -> reqwest::Client {
    reqwest::Client::builder().build().unwrap()
}
