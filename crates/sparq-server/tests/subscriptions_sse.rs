//! sq-bxog: integration tests for the SSE (Server-Sent Events) transport for
//! SPARQL update subscriptions — `GET /subscriptions/sse`.
//!
//! These boot the real axum server on an ephemeral port and speak raw HTTP/SSE with
//! `reqwest` (reading the `text/event-stream` body chunk by chunk), asserting the same
//! logical events the WebSocket `/subscriptions` path delivers — only the wire framing
//! differs. The SSE transport reuses the SAME subscription engine (register + initial
//! result via `subscribe_init`, re-evaluate + diff via `Subscription::reevaluate_step`,
//! global/per-connection slot accounting via `ConnSlots`).
//!
//! Coverage: (1) register → `subscribed` + initial-result frames; (2) a committed UPDATE
//! triggers a `notification` diff frame; (3) a non-matching UPDATE produces no frame;
//! (4) dropping the stream releases the global slot (no leak); (5) bad-query refusals
//! surface as real HTTP error statuses BEFORE the stream opens; (6) sq-bxog
//! (Copilot #120) an initial result over the row cap is a 413 (matching `/sparql`), NOT a 503.
//!
//! (sq-1b390) Gate the whole suite on the `server` feature. It spins the real axum
//! server and uses the `server`-gated `sparq_server::router` / `AppState` API, so under
//! `--no-default-features --all-targets` (the pure-serialiser-library build) this file must
//! compile OUT — otherwise `clippy --no-default-features --all-targets` breaks on the
//! unresolved axum / serde_json / router imports. 🤖 SPARQ agent.
#![cfg(feature = "server")]

use std::time::Duration;

use reqwest::Response;
use serde_json::Value;
use sparq_core::Graph;
use sparq_server::{router, AppState, ServerConfig};
use tokio::net::TcpListener;

const DATA: &str = r#"
    @prefix ex: <http://ex/> .
    ex:alice ex:age 30 .
    ex:bob   ex:age 25 .
"#;

const AGES: &str = "SELECT ?s ?age WHERE { ?s <http://ex/age> ?age }";

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

/// Opens the SSE stream for `query` (and optional `alias`); asserts the `text/event-stream`
/// content type and returns the streaming response.
async fn open_sse(addr: &str, query: &str, alias: Option<&str>) -> Response {
    let mut url = reqwest::Url::parse(&format!("http://{addr}/subscriptions/sse")).unwrap();
    url.query_pairs_mut().append_pair("query", query);
    if let Some(a) = alias {
        url.query_pairs_mut().append_pair("alias", a);
    }
    let resp = reqwest::Client::new().get(url).send().await.unwrap();
    assert_eq!(resp.status(), 200, "SSE stream should open with 200");
    let ct = resp
        .headers()
        .get("content-type")
        .unwrap()
        .to_str()
        .unwrap();
    assert!(
        ct.starts_with("text/event-stream"),
        "unexpected content-type: {ct}"
    );
    resp
}

/// An accumulating SSE frame reader over a streaming reqwest body.
struct SseReader {
    resp: Response,
    buf: String,
}

/// One parsed SSE event: its `event:` name, concatenated `data:` payload, and `id:`.
#[derive(Debug, Default, Clone)]
struct SseEvent {
    event: Option<String>,
    data: String,
    id: Option<String>,
    /// Raw comment lines (e.g. the keep-alive `: ping`).
    comment: Option<String>,
}

impl SseEvent {
    /// Parses the (single) `data:` line as JSON.
    fn json(&self) -> Value {
        serde_json::from_str(&self.data)
            .unwrap_or_else(|e| panic!("data not JSON ({e}): {:?}", self.data))
    }
}

impl SseReader {
    fn new(resp: Response) -> Self {
        Self {
            resp,
            buf: String::new(),
        }
    }

    /// Reads the next complete SSE event (a record terminated by a blank line). Keep-alive
    /// `: ping` comments are returned as their own events (with `comment` set) so callers can
    /// skip them. Times out after 5 s so a missing event fails fast.
    async fn next_event(&mut self) -> SseEvent {
        loop {
            if let Some(idx) = self.buf.find("\n\n") {
                let raw: String = self.buf.drain(..idx + 2).collect();
                return parse_event(&raw);
            }
            let chunk = tokio::time::timeout(Duration::from_secs(5), self.resp.chunk())
                .await
                .expect("timed out waiting for an SSE frame")
                .expect("stream error")
                .expect("stream closed");
            self.buf.push_str(std::str::from_utf8(&chunk).unwrap());
        }
    }

    /// Reads the next non-keep-alive event (skips `: ping` comment frames).
    async fn next_data_event(&mut self) -> SseEvent {
        loop {
            let ev = self.next_event().await;
            if ev.comment.is_some() && ev.data.is_empty() && ev.event.is_none() {
                continue; // keep-alive ping — skip
            }
            return ev;
        }
    }

    /// Asserts no DATA event arrives within `window` (keep-alive pings are tolerated).
    async fn assert_silent(&mut self, window: Duration) {
        match tokio::time::timeout(window, self.next_data_event()).await {
            Err(_elapsed) => {}
            Ok(ev) => panic!("expected silence, got: {ev:?}"),
        }
    }
}

/// Parses one SSE record (the text up to and including its blank-line terminator).
fn parse_event(raw: &str) -> SseEvent {
    let mut ev = SseEvent::default();
    for line in raw.lines() {
        if line.is_empty() {
            continue;
        }
        if let Some(c) = line.strip_prefix(':') {
            ev.comment = Some(c.trim().to_string());
        } else if let Some(v) = line.strip_prefix("event:") {
            ev.event = Some(v.trim().to_string());
        } else if let Some(v) = line.strip_prefix("data:") {
            if !ev.data.is_empty() {
                ev.data.push('\n');
            }
            // SSE strips a single leading space after the colon.
            ev.data.push_str(v.strip_prefix(' ').unwrap_or(v));
        } else if let Some(v) = line.strip_prefix("id:") {
            ev.id = Some(v.trim().to_string());
        }
    }
    ev
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

/// Reads and asserts the opening `subscribed` + sequence-0 `notification` frames, returning
/// `(id, initial_notification_event)`.
async fn read_open(reader: &mut SseReader) -> (u64, SseEvent) {
    let subscribed = reader.next_data_event().await;
    assert_eq!(
        subscribed.event.as_deref(),
        Some("subscribed"),
        "first frame: {subscribed:?}"
    );
    let id = subscribed.json()["subscribed"]["id"]
        .as_u64()
        .expect("subscribed.id");

    let initial = reader.next_data_event().await;
    assert_eq!(
        initial.event.as_deref(),
        Some("notification"),
        "second frame: {initial:?}"
    );
    let body = initial.json();
    assert_eq!(body["notification"]["id"].as_u64(), Some(id));
    assert_eq!(body["notification"]["sequence"], 0);
    // The sequence-0 notification is carried with `id: 0` (SSE event id).
    assert_eq!(initial.id.as_deref(), Some("0"));
    (id, initial)
}

// ---------------------------------------------------------------------------
// (1) register → initial result frames
// ---------------------------------------------------------------------------

#[tokio::test]
async fn sse_register_streams_subscribed_and_initial_result() {
    let addr = spawn().await;
    let resp = open_sse(&addr, AGES, None).await;
    let mut reader = SseReader::new(resp);
    let (_id, initial) = read_open(&mut reader).await;
    let body = initial.json();
    let added = bindings(&body, "addedResults");
    assert_eq!(added.len(), 2, "initial result = both ages");
    let removed = bindings(&body, "removedResults");
    assert!(removed.is_empty());
}

// ---------------------------------------------------------------------------
// (2) UPDATE triggers a notification diff frame on the SSE stream
// ---------------------------------------------------------------------------

#[tokio::test]
async fn sse_update_delivers_added_diff_frame() {
    let addr = spawn().await;
    let resp = open_sse(&addr, AGES, Some("watch")).await;
    let mut reader = SseReader::new(resp);
    let (id, _initial) = read_open(&mut reader).await;

    // Insert a new matching triple — the subscription must observe it as an `addedResults`.
    update(
        &addr,
        "INSERT DATA { <http://ex/carol> <http://ex/age> 40 }",
    )
    .await;

    let note = reader.next_data_event().await;
    assert_eq!(note.event.as_deref(), Some("notification"));
    let body = note.json();
    assert_eq!(body["notification"]["id"].as_u64(), Some(id));
    assert_eq!(body["notification"]["sequence"], 1);
    assert_eq!(
        body["notification"]["alias"], "watch",
        "alias echoed (SEPA)"
    );
    // SSE event id mirrors the per-subscription sequence.
    assert_eq!(note.id.as_deref(), Some("1"));

    let added = bindings(&body, "addedResults");
    assert_eq!(added.len(), 1, "exactly the new row");
    assert_eq!(added[0]["s"]["value"], "http://ex/carol");
    assert_eq!(added[0]["age"]["value"], "40");
    assert!(bindings(&body, "removedResults").is_empty());
}

// ---------------------------------------------------------------------------
// (3) a non-matching UPDATE produces no frame
// ---------------------------------------------------------------------------

#[tokio::test]
async fn sse_non_matching_update_is_silent() {
    let addr = spawn().await;
    // Subscribe to alice's age only — an unrelated insert must not notify.
    let resp = open_sse(
        &addr,
        "SELECT ?age WHERE { <http://ex/alice> <http://ex/age> ?age }",
        None,
    )
    .await;
    let mut reader = SseReader::new(resp);
    read_open(&mut reader).await;

    update(&addr, "INSERT DATA { <http://ex/dave> <http://ex/age> 99 }").await;
    reader.assert_silent(Duration::from_millis(500)).await;
}

// ---------------------------------------------------------------------------
// (4) dropping the stream releases the global slot (no leak)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn sse_dropped_stream_releases_its_global_slot() {
    let addr = spawn_with(ServerConfig {
        max_subscriptions: 1,
        ..ServerConfig::default()
    })
    .await;

    {
        let resp = open_sse(&addr, AGES, None).await;
        let mut reader = SseReader::new(resp);
        read_open(&mut reader).await;
        // `reader` (and the underlying response/stream) drops here — a vanished client.
    }

    // The server notices the closed stream and releases the slot; poll a fresh stream.
    let mut ok = false;
    for _ in 0..50 {
        let mut url = reqwest::Url::parse(&format!("http://{addr}/subscriptions/sse")).unwrap();
        url.query_pairs_mut().append_pair("query", AGES);
        let resp = reqwest::Client::new().get(url).send().await.unwrap();
        if resp.status() == 200 {
            // Confirm it really registered (read the opening frames).
            let mut reader = SseReader::new(resp);
            read_open(&mut reader).await;
            ok = true;
            break;
        }
        assert_eq!(
            resp.status(),
            503,
            "expected a capacity refusal while the slot is held"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(
        ok,
        "global slot was never released after the SSE stream dropped"
    );
}

// ---------------------------------------------------------------------------
// (5) registration refusals surface as real HTTP statuses before the stream opens
// ---------------------------------------------------------------------------

#[tokio::test]
async fn sse_missing_query_is_400() {
    let addr = spawn().await;
    let resp = reqwest::Client::new()
        .get(format!("http://{addr}/subscriptions/sse"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 400);
    let body: Value = resp.json().await.unwrap();
    assert!(body["error"].as_str().unwrap().contains("query"), "{body}");
}

#[tokio::test]
async fn sse_non_select_query_is_400() {
    let addr = spawn().await;
    let resp = reqwest::Client::new()
        .get(format!("http://{addr}/subscriptions/sse"))
        .query(&[("query", "ASK { ?s ?p ?o }")])
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 400);
    let body: Value = resp.json().await.unwrap();
    assert!(
        body["error"].as_str().unwrap().contains("only SELECT"),
        "{body}"
    );
}

#[tokio::test]
async fn sse_malformed_query_is_400() {
    let addr = spawn().await;
    let resp = reqwest::Client::new()
        .get(format!("http://{addr}/subscriptions/sse"))
        .query(&[("query", "SELECT ?s WHERE { this is not sparql")])
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 400);
    let body: Value = resp.json().await.unwrap();
    assert!(
        body["error"].as_str().unwrap().contains("malformed"),
        "{body}"
    );
}

// ---------------------------------------------------------------------------
// (6) sq-bxog (Copilot #120): an initial result over the max-results / row cap is
//     refused with 413 PAYLOAD_TOO_LARGE — the SAME status `/sparql` gives a row-cap overflow
//     (`engine_error_response`), NOT the blanket 503 the SSE path used before. 503 is reserved
//     for genuine capacity/overload (subscription-slot exhaustion — see test (4)).
// ---------------------------------------------------------------------------

#[tokio::test]
async fn sse_initial_result_over_max_results_is_413() {
    // `AGES` selects BOTH ages (2 rows) from `DATA`; cap the result at 1 row so the INITIAL
    // evaluation overflows the budget and the registration is refused before the stream opens.
    let addr = spawn_with(ServerConfig {
        max_results: Some(1),
        ..ServerConfig::default()
    })
    .await;

    let resp = reqwest::Client::new()
        .get(format!("http://{addr}/subscriptions/sse"))
        .query(&[("query", AGES)])
        .send()
        .await
        .unwrap();

    // 413 PAYLOAD_TOO_LARGE — consistent with the `/sparql` endpoint's row-cap refusal, not 503.
    assert_eq!(
        resp.status(),
        413,
        "an initial result over the row cap must be a 413, not a 503"
    );
    let ct = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    assert!(
        ct.starts_with("application/json"),
        "refusal is a JSON error, not an event-stream: {ct}"
    );
    let body: Value = resp.json().await.unwrap();
    let msg = body["error"].as_str().unwrap_or("");
    assert!(
        msg.contains("max-results") && msg.contains("initial evaluation"),
        "413 body names the cap that was exceeded: {body}"
    );
}

#[tokio::test]
async fn sse_initial_result_within_max_results_still_opens() {
    // Sanity counterpart to the 413 test: a cap the initial result does NOT exceed (2 rows,
    // cap 2) must still open the stream normally — the 413 is for a genuine overflow only.
    let addr = spawn_with(ServerConfig {
        max_results: Some(2),
        ..ServerConfig::default()
    })
    .await;
    let resp = open_sse(&addr, AGES, None).await;
    let mut reader = SseReader::new(resp);
    let (_id, initial) = read_open(&mut reader).await;
    assert_eq!(bindings(&initial.json(), "addedResults").len(), 2);
}
