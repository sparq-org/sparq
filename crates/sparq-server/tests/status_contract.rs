//! (sq-r5bv, gh-50) **Contract test for the stable HTTP error/status contract.**
//!
//! Asserts the consumer-facing transient-vs-permanent classification documented in
//! `src/status_contract.rs` against the REAL hardened router: for each condition a client
//! cares about (auth failure, malformed query, malformed UPDATE, body cap, result/row cap,
//! timeout, unsupported media type, not-found, method-not-allowed), it asserts both the
//! exact status code AND the transient/permanent class a retry classifier should encode.
//!
//! The point is to nail down the bit a `5xx`-only classifier gets wrong against sparq: a
//! `413` row-cap is a PERMANENT honest refusal (narrow the query), while `429`/`503` are the
//! only TRANSIENT classes. If the server's status mapping ever drifts, this test fails before
//! the documented contract can mislead a consumer.

#![cfg(feature = "server")]

use std::time::Duration;

use sparq_core::Graph;
use sparq_server::{harden, router, AppState, ServerConfig};
use tokio::net::TcpListener;

const DATA: &str = r#"
    @prefix ex: <http://ex/> .
    ex:alice ex:knows ex:bob ; ex:age 30 ; ex:name "Alice" .
    ex:bob   ex:age 25 ; ex:name "Bob"@en .
    ex:carol ex:age 35 .
"#;

/// A dense graph whose 3-way disconnected pattern is far too large to ever finish — the
/// deliberately slow query that trips the timeout `503`.
fn dense_graph_ttl() -> String {
    let n = 80; // 80*80 edges; the 3-way cross-product never finishes within the budget.
    let mut ttl = String::from("@prefix ex: <http://ex/> .\n");
    for i in 0..n {
        for j in 0..n {
            ttl.push_str(&format!("ex:n{i} ex:e ex:n{j} .\n"));
        }
    }
    ttl
}

const SLOW_QUERY: &str =
    "PREFIX ex: <http://ex/> SELECT * WHERE { ?a ex:e ?b . ?c ex:e ?d . ?e ex:e ?f }";

async fn serve(app: axum::Router) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("http://{addr}")
}

async fn spawn_with(ttl: &str, config: ServerConfig) -> String {
    let graph = Graph::load_str(ttl, "turtle").unwrap();
    // `harden()` is the production middleware stack (panic->500, concurrency->429,
    // body-limit->413, JSON error bodies) — exercise the REAL contract, not the bare router.
    serve(harden(
        router(AppState::with_config(graph, config)),
        &ServerConfig::default(),
    ))
    .await
}

fn client() -> reqwest::Client {
    reqwest::Client::new()
}

/// The transient/permanent split the contract names: a retry classifier should treat ONLY
/// these statuses as transient (a retry of the identical request may succeed).
fn is_transient(status: u16) -> bool {
    matches!(status, 429 | 503)
}

/// Every error body is the structured `{"error":"..."}` envelope and carries no echoed input
/// (the sanitisation invariant the contract leans on — classify on status, not body text).
fn assert_structured_error(body: &str) {
    assert!(
        body.starts_with("{\"error\":"),
        "expected structured JSON error envelope, got: {body}"
    );
}

// ---------------------------------------------------------------------------
// PERMANENT classes — retrying the identical request cannot help
// ---------------------------------------------------------------------------

#[tokio::test]
async fn malformed_query_is_permanent_400() {
    let base = spawn_with(DATA, ServerConfig::default()).await;
    let resp = client()
        .get(format!("{base}/sparql"))
        .query(&[("query", "SELECT ?s WHERE { this is not sparql")])
        .send()
        .await
        .unwrap();
    let status = resp.status().as_u16();
    assert_eq!(status, 400, "malformed query is a client 400");
    assert!(!is_transient(status), "400 must classify as PERMANENT");
    assert_structured_error(&resp.text().await.unwrap());
}

#[tokio::test]
async fn missing_query_param_is_permanent_400() {
    let base = spawn_with(DATA, ServerConfig::default()).await;
    let resp = client().get(format!("{base}/sparql")).send().await.unwrap();
    let status = resp.status().as_u16();
    assert_eq!(
        status, 400,
        "GET /sparql with no ?query is the historical 400"
    );
    assert!(!is_transient(status));
}

#[tokio::test]
async fn malformed_update_is_permanent_400() {
    let base = spawn_with(DATA, ServerConfig::default()).await;
    let resp = client()
        .post(format!("{base}/sparql"))
        .header("content-type", "application/sparql-update")
        .body("INSERT DATA { this is not a valid update")
        .send()
        .await
        .unwrap();
    let status = resp.status().as_u16();
    assert_eq!(
        status, 400,
        "a malformed UPDATE is a client 400, atomically (no partial effect)"
    );
    assert!(!is_transient(status));
    assert_structured_error(&resp.text().await.unwrap());
}

#[tokio::test]
async fn auth_failure_is_permanent_401_byte_identical() {
    let config = ServerConfig {
        auth_token: Some("s3cr3t".to_string()),
        ..ServerConfig::default()
    };
    let base = spawn_with(DATA, config).await;

    // A write with NO token -> 401.
    let missing = client()
        .post(format!("{base}/sparql"))
        .header("content-type", "application/sparql-update")
        .body("INSERT DATA { <http://ex/s> <http://ex/p> <http://ex/o> }")
        .send()
        .await
        .unwrap();
    assert_eq!(missing.status().as_u16(), 401);
    assert_eq!(
        missing
            .headers()
            .get("www-authenticate")
            .map(|v| v.to_str().unwrap().to_string()),
        Some("Bearer".to_string()),
        "401 carries WWW-Authenticate: Bearer"
    );
    let missing_body = missing.text().await.unwrap();

    // A write with the WRONG token -> the SAME 401 (no oracle for missing-vs-wrong).
    let wrong = client()
        .post(format!("{base}/sparql"))
        .header("content-type", "application/sparql-update")
        .header("authorization", "Bearer wrong-token")
        .body("INSERT DATA { <http://ex/s> <http://ex/p> <http://ex/o> }")
        .send()
        .await
        .unwrap();
    assert_eq!(wrong.status().as_u16(), 401);
    let wrong_body = wrong.text().await.unwrap();
    assert_eq!(
        missing_body, wrong_body,
        "missing vs wrong token must be byte-identical"
    );
    assert!(!is_transient(401), "401 must classify as PERMANENT");

    // The correct token succeeds, so the 401 was genuinely the auth gate.
    let ok = client()
        .post(format!("{base}/sparql"))
        .header("content-type", "application/sparql-update")
        .header("authorization", "Bearer s3cr3t")
        .body("INSERT DATA { <http://ex/s> <http://ex/p> <http://ex/o> }")
        .send()
        .await
        .unwrap();
    assert_eq!(
        ok.status().as_u16(),
        204,
        "the gated write succeeds with the right token"
    );
}

#[tokio::test]
async fn unsupported_post_content_type_is_permanent_415() {
    let base = spawn_with(DATA, ServerConfig::default()).await;
    let resp = client()
        .post(format!("{base}/sparql"))
        .header("content-type", "text/plain")
        .body("SELECT * WHERE { ?s ?p ?o }")
        .send()
        .await
        .unwrap();
    let status = resp.status().as_u16();
    assert_eq!(
        status, 415,
        "POST query with an unsupported Content-Type is a 415"
    );
    assert!(!is_transient(status));
}

#[tokio::test]
async fn unknown_route_is_permanent_404() {
    let base = spawn_with(DATA, ServerConfig::default()).await;
    let resp = client()
        .get(format!("{base}/no-such-route"))
        .send()
        .await
        .unwrap();
    let status = resp.status().as_u16();
    assert_eq!(status, 404);
    assert!(!is_transient(status));
    // (sq-pj6u) The unmatched-route 404 carries the SAME structured envelope as
    // every other error, now with the CATEGORISED `not found` message (was the bare empty
    // `{"error":""}`). It must NOT echo the requested path — leak-free.
    let body = resp.text().await.unwrap();
    assert_structured_error(&body);
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(v["error"], "not found", "categorised, generic 404 message");
    assert!(
        !body.contains("no-such-route"),
        "404 must not echo the requested path: {body}"
    );
}

#[tokio::test]
async fn wrong_method_is_permanent_405_with_allow() {
    let base = spawn_with(DATA, ServerConfig::default()).await;
    // /health is GET-only; a POST is a 405 with an Allow header (a permanent class).
    let resp = client()
        .post(format!("{base}/health"))
        .send()
        .await
        .unwrap();
    let status = resp.status().as_u16();
    assert_eq!(status, 405);
    assert!(resp.headers().contains_key("allow"), "405 carries Allow");
    assert!(!is_transient(status));
}

#[tokio::test]
async fn oversized_body_is_permanent_413() {
    let config = ServerConfig {
        max_body_bytes: 256,
        ..ServerConfig::default()
    };
    let base = spawn_with(DATA, config).await;
    let big = format!("SELECT * WHERE {{ ?s ?p ?o }} # {}", "x".repeat(1024));
    let resp = client()
        .post(format!("{base}/sparql"))
        .header("content-type", "application/sparql-query")
        .body(big)
        .send()
        .await
        .unwrap();
    let status = resp.status().as_u16();
    assert_eq!(status, 413, "a body over --max-body-bytes is a 413");
    assert!(
        !is_transient(status),
        "413 must classify as PERMANENT (shrink the body)"
    );
    assert_structured_error(&resp.text().await.unwrap());
}

/// THE load-bearing assertion for gh-50: a result-set row cap is a PERMANENT honest refusal
/// (`413`), NOT a transient signal and NOT a truncation. A `5xx`-only classifier misreads it.
#[tokio::test]
async fn result_row_cap_is_permanent_413_honest_refusal() {
    let config = ServerConfig {
        max_results: Some(2),
        ..ServerConfig::default()
    };
    let base = spawn_with(DATA, config).await;
    // 3 matching rows > cap of 2.
    let resp = client()
        .get(format!("{base}/sparql"))
        .query(&[("query", "SELECT ?s WHERE { ?s <http://ex/age> ?a }")])
        .send()
        .await
        .unwrap();
    let status = resp.status().as_u16();
    assert_eq!(status, 413, "exceeding the result cap is a 413, not a 5xx");
    assert!(
        !is_transient(status),
        "413 row-cap must classify as PERMANENT"
    );
    let body = resp.text().await.unwrap();
    assert_structured_error(&body);
    // Honest refusal, not truncation: the rows are NOT in the body.
    assert!(
        !body.contains("bindings"),
        "must refuse, not truncate: {body}"
    );
    // Documented stable sentinel: the generic message names the row limit (not echoed input).
    assert!(
        body.contains("row limit"),
        "row-cap 413 body carries the documented generic 'row limit' sentinel: {body}"
    );
}

/// (sq-s5is) The byte-accounted working-set cap (`--max-query-bytes`) is the
/// same PERMANENT honest `413` class as the row cap — a 413 (not a 5xx / not a truncation),
/// and its body carries the documented `byte limit` sentinel naming the byte knob.
#[tokio::test]
async fn result_byte_cap_is_permanent_413_honest_refusal() {
    let config = ServerConfig {
        // A byte budget far below any multi-row, multi-column working set, but with NO row
        // cap — so a trip can ONLY come from the byte accounting (width), proving it is wired.
        max_query_bytes: Some(4),
        ..ServerConfig::default()
    };
    let base = spawn_with(DATA, config).await;
    let resp = client()
        .get(format!("{base}/sparql"))
        .query(&[("query", "SELECT * WHERE { ?s ?p ?o }")])
        .send()
        .await
        .unwrap();
    let status = resp.status().as_u16();
    assert_eq!(status, 413, "exceeding the byte cap is a 413, not a 5xx");
    assert!(!is_transient(status), "413 byte-cap must classify as PERMANENT");
    let body = resp.text().await.unwrap();
    assert_structured_error(&body);
    assert!(!body.contains("bindings"), "must refuse, not truncate: {body}");
    assert!(
        body.contains("byte limit"),
        "byte-cap 413 body carries the documented generic 'byte limit' sentinel: {body}"
    );
}

/// (sq-s5is) Counterpart: a generous byte cap that the working set does NOT
/// exceed must answer the query normally (200) — the cap refuses only genuine overflow.
#[tokio::test]
async fn result_under_byte_cap_succeeds() {
    let config = ServerConfig {
        max_query_bytes: Some(1 << 20),
        ..ServerConfig::default()
    };
    let base = spawn_with(DATA, config).await;
    let resp = client()
        .get(format!("{base}/sparql"))
        .query(&[("query", "SELECT ?s WHERE { ?s <http://ex/age> ?a }")])
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status().as_u16(), 200, "a query under the byte cap must succeed");
    let body = resp.text().await.unwrap();
    assert!(body.contains("bindings"), "a 200 result returns its bindings: {body}");
}

// ---------------------------------------------------------------------------
// TRANSIENT classes — a retry of the identical request may succeed
// ---------------------------------------------------------------------------

/// A query timeout is a TRANSIENT `503` (with a wall-clock guarantee), and the body carries
/// the documented `timed out` sentinel.
#[tokio::test]
async fn query_timeout_is_transient_503() {
    let config = ServerConfig {
        query_timeout: Some(Duration::from_secs(1)),
        ..ServerConfig::default()
    };
    let base = spawn_with(&dense_graph_ttl(), config).await;

    let started = std::time::Instant::now();
    let resp = client()
        .get(format!("{base}/sparql"))
        .query(&[("query", SLOW_QUERY)])
        .send()
        .await
        .unwrap();
    let status = resp.status().as_u16();
    assert_eq!(status, 503, "a query timeout is a 503");
    assert!(is_transient(status), "503 must classify as TRANSIENT");
    // Wall-clock guarantee: 1s budget + hard grace; well under 5s even mid-stretch.
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "503 took {:?}",
        started.elapsed()
    );
    let body = resp.text().await.unwrap();
    assert_structured_error(&body);
    assert!(
        body.contains("timed out"),
        "503 timeout body carries the 'timed out' sentinel: {body}"
    );
}

/// The concurrency cap sheds excess in-flight requests with a TRANSIENT `429` — the shed
/// request never ran, so a back-off retry is exactly right.
#[tokio::test]
async fn concurrency_shed_is_transient_429() {
    let config = ServerConfig {
        max_concurrent: 1,
        query_timeout: Some(Duration::from_secs(30)),
        ..ServerConfig::default()
    };
    let base = spawn_with(&dense_graph_ttl(), config).await;

    // Occupy the single slot with a never-finishing query, then fire a second request that
    // must be shed with 429 (the slot is busy). The slow one is left running in the background.
    let busy_base = base.clone();
    tokio::spawn(async move {
        let _ = client()
            .get(format!("{busy_base}/sparql"))
            .query(&[("query", SLOW_QUERY)])
            .send()
            .await;
    });
    // Give the occupying request time to claim the slot.
    let mut shed = None;
    for _ in 0..50 {
        tokio::time::sleep(Duration::from_millis(100)).await;
        let resp = client()
            .get(format!("{base}/sparql"))
            .query(&[("query", "SELECT ?s WHERE { ?s <http://ex/e> ?o } LIMIT 1")])
            .send()
            .await
            .unwrap();
        if resp.status().as_u16() == 429 {
            shed = Some(resp);
            break;
        }
    }
    let resp = shed.expect("a second concurrent request should be shed with 429");
    let status = resp.status().as_u16();
    assert_eq!(status, 429);
    assert!(is_transient(status), "429 must classify as TRANSIENT");
    assert_structured_error(&resp.text().await.unwrap());
}

// ---------------------------------------------------------------------------
// Cross-cutting: the envelope + sanitisation invariant the contract relies on
// ---------------------------------------------------------------------------

/// Every error class above is the SAME `{"error":...}` JSON envelope with a JSON content type,
/// and none echoes the sentinel input token — so a consumer can rely on status + a generic
/// class, never on parsing echoed input.
#[tokio::test]
async fn all_error_bodies_are_sanitized_json() {
    let base = spawn_with(DATA, ServerConfig::default()).await;
    const SENTINEL: &str = "SENTINEL_NEEDLE_42";
    // A malformed query carrying a sentinel token must not echo it back.
    let resp = client()
        .get(format!("{base}/sparql"))
        .query(&[("query", format!("SELECT {SENTINEL} WHERE {{ broken"))])
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status().as_u16(), 400);
    let ct = resp
        .headers()
        .get("content-type")
        .map(|v| v.to_str().unwrap().to_string())
        .unwrap_or_default();
    assert!(
        ct.starts_with("application/json"),
        "error content-type is JSON: {ct}"
    );
    let body = resp.text().await.unwrap();
    assert_structured_error(&body);
    assert!(
        !body.contains(SENTINEL),
        "error body must not echo the caller's input: {body}"
    );
}

// ---------------------------------------------------------------------------
// (sq-iu0c) PERMANENT-but-distinct class: a blocked SERVICE egress is a 403
// POLICY refusal, NOT the 500 a server-fault execution error gives. A retry classifier must
// see a permanent 403 (ask the operator to allowlist the host), never a transient 5xx.
// ---------------------------------------------------------------------------

#[cfg(feature = "service")]
#[tokio::test]
async fn blocked_service_egress_is_permanent_403() {
    // Default config => empty egress allowlist + default-deny SSRF: a non-SILENT SERVICE to a
    // loopback endpoint is refused before any network call. A short query timeout bounds any
    // regression that would dial. (127.0.0.1 is a private address, so the default-deny SSRF
    // branch refuses it even without the strict allowlist-only `serve()` wiring.)
    let config = ServerConfig {
        query_timeout: Some(Duration::from_secs(2)),
        ..ServerConfig::default()
    };
    let base = spawn_with(DATA, config).await;
    let q = "PREFIX ex: <http://ex/> SELECT ?s WHERE { ?s ex:age ?a . \
             SERVICE <http://127.0.0.1:9/> { ?s ex:name ?name } }";
    let resp = client()
        .get(format!("{base}/sparql"))
        .query(&[("query", q)])
        .send()
        .await
        .unwrap();
    let status = resp.status().as_u16();
    assert_eq!(status, 403, "a blocked SERVICE egress is a 403 policy refusal");
    assert!(!is_transient(status), "403 must classify as PERMANENT (allowlist the host)");
    let body = resp.text().await.unwrap();
    assert_structured_error(&body);
    // The refused host detail is sanitized out of the body (classify on status, not text).
    assert!(
        !body.contains("127.0.0.1"),
        "the refused host must not reach the client body: {body}"
    );
    // The stable generic class names the egress allowlist so a consumer can act.
    assert!(
        body.to_lowercase().contains("egress allowlist"),
        "403 body carries the documented generic egress-refusal sentinel: {body}"
    );
}
