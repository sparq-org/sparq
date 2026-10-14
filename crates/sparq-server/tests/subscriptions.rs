//! Integration tests for the SEPA-style subscription protocol (T23, `/subscriptions`).
//!
//! Each test boots the real axum server on an ephemeral port, opens a real WebSocket with
//! `tokio-tungstenite` (the same crate axum's `ws` feature is built on) and drives the
//! JSON protocol end to end: subscribe → initial notification, committed SPARQL Update
//! via POST /sparql → added/removed diff, non-matching update → silence, unsubscribe,
//! the per-connection / global limits, and slot cleanup after a socket drop.
//!
//! (sq-1b390) Gate the whole suite on the `server` feature. It spins the real axum
//! server and uses the `server`-gated `sparq_server::router` / `AppState` API, so under
//! `--no-default-features --all-targets` (the pure-serialiser-library build) this file must
//! compile OUT — otherwise `clippy --no-default-features --all-targets` breaks on the
//! unresolved axum / serde_json / router imports. 🤖 SPARQ agent.
#![cfg(feature = "server")]

use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use sparq_core::Graph;
use sparq_server::{router, AppState, ServerConfig};
use tokio::net::TcpListener;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

type Ws = WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;

const DATA: &str = r#"
    @prefix ex: <http://ex/> .
    ex:alice ex:age 30 .
    ex:bob   ex:age 25 .
"#;

/// Boots the server and returns its `host:port`.
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

async fn spawn() -> String {
    spawn_with(ServerConfig::default()).await
}

async fn connect(addr: &str) -> Ws {
    let (ws, _) = tokio_tungstenite::connect_async(format!("ws://{addr}/subscriptions"))
        .await
        .expect("websocket connect");
    ws
}

async fn send_json(ws: &mut Ws, msg: Value) {
    // (sq-1qkm) tungstenite 0.26+ — `Message::Text` holds a `Utf8Bytes`, not a
    // `String`; `Utf8Bytes: From<String>` so `.into()` bridges it.
    ws.send(Message::Text(msg.to_string().into())).await.unwrap();
}

/// Receives the next text frame as JSON (5 s guard so a missing message fails fast).
async fn recv_json(ws: &mut Ws) -> Value {
    loop {
        let frame = tokio::time::timeout(Duration::from_secs(5), ws.next())
            .await
            .expect("timed out waiting for a server message")
            .expect("socket closed")
            .expect("socket error");
        match frame {
            // (sq-1qkm) `t` is `Utf8Bytes` (tungstenite 0.26+); `.as_str()` yields &str.
            Message::Text(t) => return serde_json::from_str(t.as_str()).unwrap(),
            Message::Ping(_) | Message::Pong(_) => continue,
            other => panic!("unexpected frame: {other:?}"),
        }
    }
}

/// Asserts no server message arrives within `window` (frame-level silence).
async fn assert_silent(ws: &mut Ws, window: Duration) {
    match tokio::time::timeout(window, ws.next()).await {
        Err(_elapsed) => {}
        Ok(msg) => panic!("expected silence, got: {msg:?}"),
    }
}

/// Subscribes and consumes the `subscribed` + initial notification pair; returns
/// `(id, initial_notification)`.
async fn subscribe(ws: &mut Ws, query: &str) -> (u64, Value) {
    send_json(ws, json!({ "subscribe": { "query": query } })).await;
    let subscribed = recv_json(ws).await;
    let id = subscribed["subscribed"]["id"].as_u64().expect("subscribed.id");
    let initial = recv_json(ws).await;
    assert_eq!(initial["notification"]["id"].as_u64(), Some(id));
    assert_eq!(initial["notification"]["sequence"], 0);
    (id, initial)
}

/// Commits a SPARQL Update through the HTTP endpoint (the protocol's only write path).
async fn update(addr: &str, sparql: &str) {
    let resp = reqwest::Client::new()
        .post(format!("http://{addr}/sparql"))
        .header("content-type", "application/sparql-update")
        .body(sparql.to_string())
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 204, "update failed");
}

fn bindings<'a>(notification: &'a Value, which: &str) -> &'a Vec<Value> {
    notification["notification"][which]["results"]["bindings"]
        .as_array()
        .unwrap_or_else(|| panic!("missing {which} bindings"))
}

const AGES: &str = "SELECT ?s ?age WHERE { ?s <http://ex/age> ?age }";

// ---------------------------------------------------------------------------
// Subscribe / initial result
// ---------------------------------------------------------------------------

#[tokio::test]
async fn subscribe_sends_initial_full_result_as_added() {
    let addr = spawn().await;
    let mut ws = connect(&addr).await;
    let (_, initial) = subscribe(&mut ws, AGES).await;
    let added = bindings(&initial, "addedResults");
    assert_eq!(added.len(), 2);
    assert!(bindings(&initial, "removedResults").is_empty());
    assert_eq!(
        initial["notification"]["addedResults"]["head"]["vars"],
        json!(["s", "age"])
    );
    // SPARQL JSON term encoding in the bindings.
    assert!(added.iter().any(|b| b["s"]["value"] == "http://ex/alice" && b["age"]["value"] == "30"));
}

#[tokio::test]
async fn alias_is_echoed_in_subscribed_and_notifications() {
    let addr = spawn().await;
    let mut ws = connect(&addr).await;
    send_json(&mut ws, json!({ "subscribe": { "query": AGES, "alias": "ages" } })).await;
    let subscribed = recv_json(&mut ws).await;
    assert_eq!(subscribed["subscribed"]["alias"], "ages");
    let initial = recv_json(&mut ws).await;
    assert_eq!(initial["notification"]["alias"], "ages");
}

// ---------------------------------------------------------------------------
// Update → diff notifications
// ---------------------------------------------------------------------------

#[tokio::test]
async fn committed_insert_pushes_added_diff() {
    let addr = spawn().await;
    let mut ws = connect(&addr).await;
    let (id, _) = subscribe(&mut ws, AGES).await;

    update(&addr, "INSERT DATA { <http://ex/carol> <http://ex/age> 35 }").await;

    let n = recv_json(&mut ws).await;
    assert_eq!(n["notification"]["id"].as_u64(), Some(id));
    assert_eq!(n["notification"]["sequence"], 1);
    let added = bindings(&n, "addedResults");
    assert_eq!(added.len(), 1);
    assert_eq!(added[0]["s"]["value"], "http://ex/carol");
    assert_eq!(added[0]["age"]["value"], "35");
    assert!(bindings(&n, "removedResults").is_empty());
}

#[tokio::test]
async fn committed_delete_pushes_removed_diff() {
    let addr = spawn().await;
    let mut ws = connect(&addr).await;
    subscribe(&mut ws, AGES).await;

    update(&addr, "DELETE DATA { <http://ex/bob> <http://ex/age> 25 }").await;

    let n = recv_json(&mut ws).await;
    let removed = bindings(&n, "removedResults");
    assert_eq!(removed.len(), 1);
    assert_eq!(removed[0]["s"]["value"], "http://ex/bob");
    assert!(bindings(&n, "addedResults").is_empty());
}

#[tokio::test]
async fn non_matching_update_produces_no_notification() {
    let addr = spawn().await;
    let mut ws = connect(&addr).await;
    subscribe(&mut ws, AGES).await;

    // Different predicate — re-evaluation runs but the diff is empty.
    update(&addr, "INSERT DATA { <http://ex/alice> <http://ex/name> \"Alice\" }").await;
    assert_silent(&mut ws, Duration::from_millis(300)).await;

    // Liveness proof: the NEXT message is the diff of a matching update (nothing was
    // queued for the non-matching one; a coalesced evaluation would also be tolerated).
    update(&addr, "INSERT DATA { <http://ex/dave> <http://ex/age> 40 }").await;
    let n = recv_json(&mut ws).await;
    let added = bindings(&n, "addedResults");
    assert_eq!(added.len(), 1);
    assert_eq!(added[0]["s"]["value"], "http://ex/dave");
}

#[tokio::test]
async fn multiple_subscriptions_on_one_socket_are_independent() {
    let addr = spawn().await;
    let mut ws = connect(&addr).await;
    let (ages_id, _) = subscribe(&mut ws, AGES).await;
    let (names_id, _) = subscribe(&mut ws, "SELECT ?n WHERE { ?s <http://ex/name> ?n }").await;
    assert_ne!(ages_id, names_id);

    // Touches only the names subscription.
    update(&addr, "INSERT DATA { <http://ex/alice> <http://ex/name> \"Alice\" }").await;
    let n = recv_json(&mut ws).await;
    assert_eq!(n["notification"]["id"].as_u64(), Some(names_id));
    assert_eq!(bindings(&n, "addedResults")[0]["n"]["value"], "Alice");
    // And nothing for the ages subscription.
    assert_silent(&mut ws, Duration::from_millis(300)).await;
}

// ---------------------------------------------------------------------------
// Unsubscribe
// ---------------------------------------------------------------------------

#[tokio::test]
async fn unsubscribe_stops_notifications() {
    let addr = spawn().await;
    let mut ws = connect(&addr).await;
    let (id, _) = subscribe(&mut ws, AGES).await;

    send_json(&mut ws, json!({ "unsubscribe": { "id": id } })).await;
    let resp = recv_json(&mut ws).await;
    assert_eq!(resp["unsubscribed"]["id"].as_u64(), Some(id));

    update(&addr, "INSERT DATA { <http://ex/carol> <http://ex/age> 35 }").await;
    assert_silent(&mut ws, Duration::from_millis(300)).await;
}

#[tokio::test]
async fn unsubscribe_unknown_id_is_an_error() {
    let addr = spawn().await;
    let mut ws = connect(&addr).await;
    send_json(&mut ws, json!({ "unsubscribe": { "id": 999 } })).await;
    let resp = recv_json(&mut ws).await;
    assert_eq!(resp["error"]["id"], 999);
    assert!(resp["error"]["message"].as_str().unwrap().contains("no active subscription"));
}

// ---------------------------------------------------------------------------
// Refusals: bad requests + limits
// ---------------------------------------------------------------------------

#[tokio::test]
async fn non_select_and_malformed_queries_are_refused() {
    let addr = spawn().await;
    let mut ws = connect(&addr).await;

    send_json(&mut ws, json!({ "subscribe": { "query": "ASK { ?s ?p ?o }" } })).await;
    let resp = recv_json(&mut ws).await;
    assert!(resp["error"]["message"].as_str().unwrap().contains("only SELECT"));

    send_json(&mut ws, json!({ "subscribe": { "query": "SELECT WHERE {" } })).await;
    let resp = recv_json(&mut ws).await;
    assert!(resp["error"]["message"].as_str().unwrap().contains("malformed query"));

    send_json(&mut ws, json!({ "not-a-verb": {} })).await;
    let resp = recv_json(&mut ws).await;
    assert!(resp["error"]["message"].as_str().unwrap().contains("unknown message"));
}

#[tokio::test]
async fn per_connection_limit_is_enforced() {
    let addr = spawn_with(ServerConfig { max_subscriptions_per_conn: 2, ..ServerConfig::default() }).await;
    let mut ws = connect(&addr).await;
    subscribe(&mut ws, AGES).await;
    subscribe(&mut ws, AGES).await;

    send_json(&mut ws, json!({ "subscribe": { "query": AGES } })).await;
    let resp = recv_json(&mut ws).await;
    let msg = resp["error"]["message"].as_str().unwrap();
    assert!(msg.contains("limit reached for this connection (2)"), "got: {msg}");
}

#[tokio::test]
async fn global_limit_is_enforced_across_connections() {
    let addr = spawn_with(ServerConfig { max_subscriptions: 1, ..ServerConfig::default() }).await;
    let mut ws1 = connect(&addr).await;
    subscribe(&mut ws1, AGES).await;

    let mut ws2 = connect(&addr).await;
    send_json(&mut ws2, json!({ "subscribe": { "query": AGES } })).await;
    let resp = recv_json(&mut ws2).await;
    let msg = resp["error"]["message"].as_str().unwrap();
    assert!(msg.contains("server-wide subscription limit reached (1)"), "got: {msg}");
}

#[tokio::test]
async fn oversized_initial_result_refuses_the_subscription() {
    // The data has 2 matching rows; cap results at 1 → honest refusal naming the limit.
    let addr = spawn_with(ServerConfig { max_results: Some(1), ..ServerConfig::default() }).await;
    let mut ws = connect(&addr).await;
    send_json(&mut ws, json!({ "subscribe": { "query": AGES } })).await;
    let resp = recv_json(&mut ws).await;
    let msg = resp["error"]["message"].as_str().unwrap();
    assert!(msg.contains("max-results limit (1 rows)"), "got: {msg}");

    // The refused subscription must not consume a slot: with the cap respected, a
    // narrower query still subscribes fine.
    let (_, initial) = subscribe(&mut ws, "SELECT ?age WHERE { <http://ex/alice> <http://ex/age> ?age }").await;
    assert_eq!(bindings(&initial, "addedResults").len(), 1);
}

// ---------------------------------------------------------------------------
// Cleanup on socket drop
// ---------------------------------------------------------------------------

#[tokio::test]
async fn dropped_socket_releases_its_global_slots() {
    let addr = spawn_with(ServerConfig { max_subscriptions: 1, ..ServerConfig::default() }).await;

    {
        let mut ws = connect(&addr).await;
        subscribe(&mut ws, AGES).await;
        // Dropped here without unsubscribe/close handshake — a vanished client.
    }

    // The server notices the closed socket and releases the slot; poll briefly.
    let mut ws2 = connect(&addr).await;
    let mut ok = false;
    for _ in 0..50 {
        send_json(&mut ws2, json!({ "subscribe": { "query": AGES } })).await;
        let resp = recv_json(&mut ws2).await;
        if resp.get("subscribed").is_some() {
            let initial = recv_json(&mut ws2).await;
            assert_eq!(initial["notification"]["sequence"], 0);
            ok = true;
            break;
        }
        assert!(resp.get("error").is_some());
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(ok, "global slot was never released after the socket dropped");
}
