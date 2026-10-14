//! sq-cxk5 — integration tests for the Bearer **read**-token gate on the
//! subscription READ surfaces: the `/subscriptions` WebSocket AND the `/subscriptions/sse`
//! Server-Sent-Events stream.
//!
//! `sq-zcby` gated the HTTP write/read surface; the subscription transports were left open,
//! a read-auth bypass. These tests boot the real axum server on an ephemeral port and assert:
//!
//!   * **SSE** (`GET /subscriptions/sse`, a plain GET): with `--auth-token-read` set, no token
//!     and a WRONG token are `401`; the CORRECT `Authorization: Bearer` token opens the stream.
//!   * **WebSocket** (`/subscriptions` upgrade): with `--auth-token-read` set, no token is
//!     rejected; the `Authorization: Bearer` header is accepted (non-browser clients); the
//!     `Sec-WebSocket-Protocol: bearer.<token>` subprotocol is accepted (the browser channel)
//!     and the matched subprotocol is echoed back per RFC 6455.
//!   * **back-compat**: with NO read token configured, both transports still open.
//!
//! Reuses the WS harness shape from `subscriptions.rs` (tokio-tungstenite) and the SSE shape
//! from `subscriptions_sse.rs` (reqwest reads the `text/event-stream` body).
//!
//! (sq-1b390) Gate the whole suite on the `server` feature. It spins the real axum
//! server and uses the `server`-gated `sparq_server::router` / `AppState` API, so under
//! `--no-default-features --all-targets` (the pure-serialiser-library build) this file must
//! compile OUT — otherwise `clippy --no-default-features --all-targets` breaks on the
//! unresolved axum / serde_json / router imports. 🤖 SPARQ agent.
#![cfg(feature = "server")]

use sparq_core::Graph;
use sparq_server::{router, AppState, ServerConfig};
use tokio::net::TcpListener;
use tokio_tungstenite::tungstenite::client::ClientRequestBuilder;
use tokio_tungstenite::tungstenite::http::Uri;

const DATA: &str = r#"
    @prefix ex: <http://ex/> .
    ex:alice ex:age 30 .
    ex:bob   ex:age 25 .
"#;

const AGES: &str = "SELECT ?s ?age WHERE { ?s <http://ex/age> ?age }";
const TOKEN: &str = "s3cr3t-read-token";

/// Boots the server with `config`; returns its `host:port` (no scheme — callers add `http`/`ws`).
async fn spawn_with(config: ServerConfig) -> String {
    let graph = Graph::load_str(DATA, "turtle").unwrap();
    let app = router(AppState::with_config(graph, config));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    addr.to_string()
}

/// A config whose read surface is gated by `TOKEN` (`--auth-token` + `--auth-token-read`).
fn read_gated_config() -> ServerConfig {
    ServerConfig { auth_token: Some(TOKEN.to_string()), auth_token_read: true, ..ServerConfig::default() }
}

// ---------------------------------------------------------------------------
// SSE (plain GET): Authorization: Bearer header only
// ---------------------------------------------------------------------------

/// Sends `GET /subscriptions/sse?query=<AGES>`, optionally with a Bearer token, and returns the
/// HTTP status (without consuming the stream body).
async fn sse_status(addr: &str, bearer: Option<&str>) -> reqwest::StatusCode {
    let mut url = reqwest::Url::parse(&format!("http://{addr}/subscriptions/sse")).unwrap();
    url.query_pairs_mut().append_pair("query", AGES);
    let mut req = reqwest::Client::new().get(url);
    if let Some(tok) = bearer {
        req = req.header("authorization", format!("Bearer {tok}"));
    }
    req.send().await.unwrap().status()
}

#[tokio::test]
async fn sse_without_token_is_401_when_read_gated() {
    let addr = spawn_with(read_gated_config()).await;
    assert_eq!(sse_status(&addr, None).await, reqwest::StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn sse_with_wrong_token_is_401_when_read_gated() {
    let addr = spawn_with(read_gated_config()).await;
    assert_eq!(sse_status(&addr, Some("not-the-token")).await, reqwest::StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn sse_with_correct_token_opens_stream() {
    let addr = spawn_with(read_gated_config()).await;
    let mut url = reqwest::Url::parse(&format!("http://{addr}/subscriptions/sse")).unwrap();
    url.query_pairs_mut().append_pair("query", AGES);
    let resp = reqwest::Client::new().get(url).header("authorization", format!("Bearer {TOKEN}")).send().await.unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::OK, "correct token must open the stream");
    let ct = resp.headers().get("content-type").unwrap().to_str().unwrap();
    assert!(ct.starts_with("text/event-stream"), "unexpected content-type: {ct}");
}

#[tokio::test]
async fn sse_open_when_no_read_token_configured() {
    // Back-compat: no token at all → the stream opens with no auth header.
    let addr = spawn_with(ServerConfig::default()).await;
    assert_eq!(sse_status(&addr, None).await, reqwest::StatusCode::OK);
}

#[tokio::test]
async fn sse_open_when_only_write_gated() {
    // A write-only token (no --auth-token-read) leaves the read surface open (back-compat).
    let cfg = ServerConfig { auth_token: Some(TOKEN.to_string()), auth_token_read: false, ..ServerConfig::default() };
    let addr = spawn_with(cfg).await;
    assert_eq!(sse_status(&addr, None).await, reqwest::StatusCode::OK);
}

// ---------------------------------------------------------------------------
// WebSocket upgrade: Authorization header OR Sec-WebSocket-Protocol subprotocol
// ---------------------------------------------------------------------------

fn ws_uri(addr: &str) -> Uri {
    format!("ws://{addr}/subscriptions").parse().unwrap()
}

/// Attempts the WS upgrade with the given builder; returns `Ok(selected_subprotocol)` on a
/// successful handshake (the echoed `Sec-WebSocket-Protocol`, if any), or `Err(status)` with
/// the HTTP status the server refused the upgrade with.
async fn try_ws_upgrade(builder: ClientRequestBuilder) -> Result<Option<String>, u16> {
    match tokio_tungstenite::connect_async(builder).await {
        Ok((_stream, resp)) => {
            let proto = resp.headers().get("sec-websocket-protocol").and_then(|v| v.to_str().ok()).map(str::to_string);
            Ok(proto)
        }
        Err(tokio_tungstenite::tungstenite::Error::Http(resp)) => Err(resp.status().as_u16()),
        Err(other) => panic!("unexpected WS connect error: {other:?}"),
    }
}

#[tokio::test]
async fn ws_upgrade_without_token_is_rejected_when_read_gated() {
    let addr = spawn_with(read_gated_config()).await;
    let status = try_ws_upgrade(ClientRequestBuilder::new(ws_uri(&addr))).await.expect_err("upgrade must be refused without a token");
    assert_eq!(status, 401, "unauthenticated WS upgrade must be 401");
}

#[tokio::test]
async fn ws_upgrade_with_wrong_token_is_rejected_when_read_gated() {
    let addr = spawn_with(read_gated_config()).await;
    let builder = ClientRequestBuilder::new(ws_uri(&addr)).with_header("Authorization", "Bearer nope");
    let status = try_ws_upgrade(builder).await.expect_err("wrong token must be refused");
    assert_eq!(status, 401);
}

#[tokio::test]
async fn ws_upgrade_with_authorization_header_is_accepted() {
    let addr = spawn_with(read_gated_config()).await;
    let builder = ClientRequestBuilder::new(ws_uri(&addr)).with_header("Authorization", format!("Bearer {TOKEN}"));
    let proto = try_ws_upgrade(builder).await.expect("Bearer header must be accepted");
    // No subprotocol was offered, so none is echoed back.
    assert_eq!(proto, None);
}

#[tokio::test]
async fn ws_upgrade_with_bearer_subprotocol_is_accepted_and_echoed() {
    let addr = spawn_with(read_gated_config()).await;
    let sub = format!("bearer.{TOKEN}");
    let builder = ClientRequestBuilder::new(ws_uri(&addr)).with_sub_protocol(sub.clone());
    let proto = try_ws_upgrade(builder).await.expect("bearer subprotocol must be accepted");
    // RFC 6455: the server confirms the matched subprotocol so the browser handshake completes.
    assert_eq!(proto.as_deref(), Some(sub.as_str()), "server must echo the accepted bearer subprotocol");
}

#[tokio::test]
async fn ws_upgrade_with_wrong_bearer_subprotocol_is_rejected() {
    let addr = spawn_with(read_gated_config()).await;
    let builder = ClientRequestBuilder::new(ws_uri(&addr)).with_sub_protocol("bearer.nope");
    let status = try_ws_upgrade(builder).await.expect_err("a wrong subprotocol token must be refused");
    assert_eq!(status, 401, "the subprotocol token is VALIDATED, not merely echoed");
}

#[tokio::test]
async fn ws_upgrade_open_when_no_read_token_configured() {
    // Back-compat: no token at all → the upgrade succeeds with no credentials.
    let addr = spawn_with(ServerConfig::default()).await;
    let proto = try_ws_upgrade(ClientRequestBuilder::new(ws_uri(&addr))).await.expect("upgrade must succeed with no token configured");
    assert_eq!(proto, None);
}

#[tokio::test]
async fn ws_upgrade_open_when_only_write_gated() {
    // A write-only token leaves the WS read surface open (back-compat).
    let cfg = ServerConfig { auth_token: Some(TOKEN.to_string()), auth_token_read: false, ..ServerConfig::default() };
    let addr = spawn_with(cfg).await;
    try_ws_upgrade(ClientRequestBuilder::new(ws_uri(&addr))).await.expect("write-only gate must leave the WS upgrade open");
}
