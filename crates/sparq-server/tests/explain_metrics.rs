//! Integration tests for T22: the `/sparql` EXPLAIN surface (query parameter and
//! `Accept: text/x-sparq-explain`) and the `/metrics` Prometheus endpoint.
//!
//! (sq-1b390) Gate the whole suite on the `server` feature. It spins the real axum
//! server and uses the `server`-gated `sparq_server::router` / `AppState` API, so under
//! `--no-default-features --all-targets` (the pure-serialiser-library build) this file must
//! compile OUT — otherwise `clippy --no-default-features --all-targets` breaks on the
//! unresolved axum / serde_json / router imports. 🤖 SPARQ agent.
#![cfg(feature = "server")]

use sparq_core::Graph;
use sparq_server::{router, AppState};
use tokio::net::TcpListener;

const DATA: &str = r#"
    @prefix ex: <http://ex/> .
    ex:alice ex:knows ex:bob ; ex:age 30 ; ex:name "Alice" .
    ex:bob   ex:knows ex:carol ; ex:age 25 ; ex:name "Bob" .
    ex:carol ex:age 35 ; ex:name "Carol" .
"#;

/// Boots the server on a random local port and returns its base URL.
async fn spawn() -> String {
    let graph = Graph::load_str(DATA, "turtle").unwrap();
    let app = router(AppState::new(graph));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("http://{addr}")
}

fn client() -> reqwest::Client {
    reqwest::Client::new()
}

const JOIN_QUERY: &str = "PREFIX ex: <http://ex/> SELECT ?a ?b ?age WHERE { ?a ex:knows ?b . ?b ex:age ?age }";

// ---------------------------------------------------------------------------
// EXPLAIN
// ---------------------------------------------------------------------------

#[tokio::test]
async fn get_explain_param_returns_plan_text() {
    let base = spawn().await;
    let resp = client()
        .get(format!("{base}/sparql"))
        .query(&[("query", JOIN_QUERY), ("explain", "true")])
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    assert!(resp.headers()["content-type"].to_str().unwrap().starts_with("text/plain"));
    let body = resp.text().await.unwrap();
    assert!(body.contains("EXPLAIN (SELECT)"), "{body}");
    assert!(body.contains("planning-only"), "{body}");
    // The plan names the join order and strategy — not a result set.
    assert!(body.contains("merge join on ?b"), "{body}");
    assert!(!body.contains("\"bindings\""), "{body}");
}

#[tokio::test]
async fn accept_header_requests_explain() {
    let base = spawn().await;
    let resp = client()
        .post(format!("{base}/sparql"))
        .header("content-type", "application/sparql-query")
        .header("accept", "text/x-sparq-explain")
        .body(JOIN_QUERY)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body = resp.text().await.unwrap();
    assert!(body.contains("EXPLAIN (SELECT)"), "{body}");
    assert!(body.contains("BGP [binary join plan: greedy GOO ordering]"), "{body}");
}

#[tokio::test]
async fn post_form_explain_analyze_executes_and_traces() {
    let base = spawn().await;
    let resp = client()
        .post(format!("{base}/sparql"))
        .header("content-type", "application/x-www-form-urlencoded")
        .body(format!("query={}&explain=analyze", urlencoding(JOIN_QUERY)))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body = resp.text().await.unwrap();
    assert!(body.contains("EXPLAIN ANALYZE (SELECT)"), "{body}");
    assert!(body.contains("Execution trace"), "{body}");
    assert!(body.contains("rows=2"), "{body}");
    assert!(body.contains("Total: 2 result row(s)"), "{body}");
}

#[tokio::test]
async fn explain_of_malformed_query_is_400_and_explain_false_executes() {
    let base = spawn().await;
    let resp = client()
        .get(format!("{base}/sparql"))
        .query(&[("query", "SELECT WHERE {"), ("explain", "true")])
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 400);

    // explain=false answers with normal results.
    let resp = client()
        .get(format!("{base}/sparql"))
        .query(&[("query", JOIN_QUERY), ("explain", "false")])
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    assert!(resp.text().await.unwrap().contains("\"bindings\""));
}

// ---------------------------------------------------------------------------
// STRUCTURED explain (`Accept: application/x-sparq-explain+json`) — sq-ixc3.19.
// The camelCase typed plan tree (the sq-jbqh4 schema contract the GUI plan
// explorer renders). Feature-gated: the arm exists only under `explain-json`
// (default-on for the server binary); a lean build answers 406 (tested below).
// 🤖 SPARQ agent.
// ---------------------------------------------------------------------------

const EXPLAIN_JSON_CT: &str = "application/x-sparq-explain+json";

#[cfg(feature = "explain-json")]
#[tokio::test]
async fn accept_json_ct_returns_structured_plan_dry_run() {
    let base = spawn().await;
    // The JSON Accept ALONE requests a (plan-only) explain, mirroring the text CT.
    let resp = client()
        .get(format!("{base}/sparql"))
        .query(&[("query", JOIN_QUERY)])
        .header("accept", EXPLAIN_JSON_CT)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let ct = resp.headers()["content-type"].to_str().unwrap().to_string();
    assert!(ct.starts_with(EXPLAIN_JSON_CT), "{ct}");
    let body = resp.text().await.unwrap();
    // The schema contract: camelCase keys + children nesting; a dry run executes
    // nothing, so actual/nanos/qError are null and it is NOT a result set.
    assert!(body.contains("\"operator\":"), "{body}");
    assert!(body.contains("\"children\":"), "{body}");
    assert!(body.contains("\"actual\":null"), "{body}");
    assert!(body.contains("\"qError\":null"), "{body}");
    assert!(!body.contains("\"bindings\""), "{body}");
}

#[cfg(feature = "explain-json")]
#[tokio::test]
async fn explain_analyze_with_json_ct_fills_actuals() {
    let base = spawn().await;
    let resp = client()
        .get(format!("{base}/sparql"))
        .query(&[("query", JOIN_QUERY), ("explain", "analyze")])
        .header("accept", EXPLAIN_JSON_CT)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body = resp.text().await.unwrap();
    // ANALYZE executed: the join yields exactly 2 rows (alice→bob, bob→carol), so
    // the root operator's actual output-row count is a number, with wall nanos.
    assert!(body.contains("\"actual\":2"), "{body}");
    assert!(!body.contains("\"nanos\":null"), "{body}");
}

#[cfg(feature = "explain-json")]
#[tokio::test]
async fn json_analyze_of_construct_is_400() {
    let base = spawn().await;
    let resp = client()
        .get(format!("{base}/sparql"))
        .query(&[
            ("query", "CONSTRUCT { ?s ?p ?o } WHERE { ?s ?p ?o }"),
            ("explain", "analyze"),
        ])
        .header("accept", EXPLAIN_JSON_CT)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 400);
}

/// A build WITHOUT `explain-json` refuses the structured request up front (406,
/// never a silent text fallback the caller would mis-parse), while the TEXT
/// explain surface stays fully available.
#[cfg(not(feature = "explain-json"))]
#[tokio::test]
async fn lean_build_answers_structured_explain_406() {
    let base = spawn().await;
    let resp = client()
        .get(format!("{base}/sparql"))
        .query(&[("query", JOIN_QUERY)])
        .header("accept", EXPLAIN_JSON_CT)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 406);
    let body = resp.text().await.unwrap();
    assert!(body.contains("explain-json"), "{body}");
    // The text explain still works on the same build.
    let resp = client()
        .get(format!("{base}/sparql"))
        .query(&[("query", JOIN_QUERY), ("explain", "true")])
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    assert!(resp.text().await.unwrap().contains("EXPLAIN (SELECT)"));
}

/// Minimal percent-encoding for the form body (space + reserved chars used here).
fn urlencoding(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

// ---------------------------------------------------------------------------
// /metrics
// ---------------------------------------------------------------------------

#[tokio::test]
async fn metrics_exposes_requests_histogram_and_gauges() {
    let base = spawn().await;
    // One successful query, one malformed (400).
    let r = client().get(format!("{base}/sparql")).query(&[("query", JOIN_QUERY)]).send().await.unwrap();
    assert_eq!(r.status(), 200);
    let r = client().get(format!("{base}/sparql")).query(&[("query", "SELECT WHERE {")]).send().await.unwrap();
    assert_eq!(r.status(), 400);
    let r = client().get(format!("{base}/health")).send().await.unwrap();
    assert_eq!(r.status(), 200);

    let resp = client().get(format!("{base}/metrics")).send().await.unwrap();
    assert_eq!(resp.status(), 200);
    assert!(resp.headers()["content-type"].to_str().unwrap().starts_with("text/plain"));
    let body = resp.text().await.unwrap();
    assert!(body.contains("sparq_http_requests_total{endpoint=\"/sparql\",status=\"200\"} 1"), "{body}");
    assert!(body.contains("sparq_http_requests_total{endpoint=\"/sparql\",status=\"400\"} 1"), "{body}");
    assert!(body.contains("sparq_http_requests_total{endpoint=\"/health\",status=\"200\"} 1"), "{body}");
    // Histogram: both /sparql requests observed; +Inf equals the count.
    assert!(body.contains("sparq_query_duration_seconds_bucket{le=\"+Inf\"} 2"), "{body}");
    assert!(body.contains("sparq_query_duration_seconds_count 2"), "{body}");
    assert!(body.contains("sparq_query_duration_seconds_sum "), "{body}");
    // Gauges read live state: 8 triples in DATA, no subscriptions open.
    assert!(body.contains("sparq_graph_triples 8"), "{body}");
    assert!(body.contains("sparq_active_subscriptions 0"), "{body}");
    assert!(body.contains("sparq_updates_total 0"), "{body}");
}

#[tokio::test]
async fn metrics_counts_updates_and_triple_gauge_follows() {
    let base = spawn().await;
    let resp = client()
        .post(format!("{base}/sparql"))
        .header("content-type", "application/sparql-update")
        .body("INSERT DATA { <http://ex/new> <http://ex/p> <http://ex/o> }")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 204);

    let body = client().get(format!("{base}/metrics")).send().await.unwrap().text().await.unwrap();
    assert!(body.contains("sparq_updates_total 1"), "{body}");
    assert!(body.contains("sparq_graph_triples 9"), "{body}");
    // The update went through /sparql, so the histogram observed it too.
    assert!(body.contains("sparq_query_duration_seconds_count 1"), "{body}");
}
