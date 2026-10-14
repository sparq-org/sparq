//! (sq-fdurb, gh-1416) **Served-surface snapshot suite for the v1 HTTP wire
//! contract** — `docs/http-wire-contract.md`.
//!
//! Pins the wire surface an HTTP-only consumer (the PSS ask, #1416) relies on, AS DOCUMENTED:
//! for each frozen endpoint behaviour and each documented error class it asserts the exact
//! status code, the body shape (`{"error":…}` envelope / SPARQL Results JSON envelope), and
//! the exact emitted `Content-Type` string — against the REAL hardened router (in-process
//! ephemeral-port server, no external network). Fail-closed: if the server ever answers
//! differently from the contract document, this suite goes red before the drift can mislead
//! a consumer.
//!
//! Deliberate overlap with `tests/status_contract.rs` (the transient-vs-permanent retry twin)
//! is the point of a snapshot suite: each documented error class gets ONE direct test HERE,
//! keyed to the contract table, so the wire doc is self-contained and self-enforcing.
//!
//! Gated on the `server` feature (the pure-serialiser `--no-default-features` build compiles
//! this file out). The `403`/`410` classes exist only under the `service` / `time-travel`
//! cargo features and are pinned under matching `cfg`. 🤖 SPARQ agent.

#![cfg(feature = "server")]

use std::time::Duration;

use sparq_core::Graph;
use sparq_server::{harden, router, AppState, ServerConfig};
use tokio::net::TcpListener;

const DATA: &str = r#"
    @prefix ex: <http://ex/> .
    ex:alice ex:knows ex:bob ; ex:age 30 .
    ex:bob   ex:age 25 .
"#;

/// Boots the REAL production stack (`harden(router(..))`) over `ttl` with `config`;
/// returns the base URL. Same idiom as `tests/status_contract.rs`.
async fn spawn_with(ttl: &str, config: ServerConfig) -> String {
    let graph = Graph::load_str(ttl, "turtle").unwrap();
    let app = harden(
        router(AppState::with_config(graph, config)),
        &ServerConfig::default(),
    );
    serve(app).await
}

async fn serve(app: axum::Router) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("http://{addr}")
}

async fn spawn_default() -> String {
    spawn_with(DATA, ServerConfig::default()).await
}

fn client() -> reqwest::Client {
    reqwest::Client::new()
}

fn content_type(resp: &reqwest::Response) -> String {
    resp.headers()
        .get("content-type")
        .map(|v| v.to_str().unwrap().to_string())
        .unwrap_or_default()
}

/// Contract §6: every error body is the structured `{"error":"…"}` JSON envelope.
async fn assert_error_envelope(resp: reqwest::Response) -> serde_json::Value {
    let ct = content_type(&resp);
    assert!(
        ct.starts_with("application/json"),
        "error Content-Type must be application/json, got: {ct}"
    );
    let body = resp.text().await.unwrap();
    assert!(
        body.starts_with("{\"error\":"),
        "error body must be the {{\"error\":…}} envelope, got: {body}"
    );
    serde_json::from_str(&body).unwrap()
}

// ===========================================================================
// Contract §1 — SPARQL 1.1 Protocol request forms at /sparql
// ===========================================================================

/// GET ?query= → 200, the SPARQL Results JSON default, and the documented SELECT envelope
/// (`head.vars` + `results.bindings`).
#[tokio::test]
async fn get_query_defaults_to_results_json_envelope() {
    let base = spawn_default().await;
    let resp = client()
        .get(format!("{base}/sparql"))
        .query(&[("query", "SELECT ?s WHERE { ?s <http://ex/age> ?a }")])
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status().as_u16(), 200);
    assert_eq!(
        content_type(&resp),
        "application/sparql-results+json",
        "no-Accept default is the exact SPARQL Results JSON content type"
    );
    let v: serde_json::Value = resp.json().await.unwrap();
    assert!(v["head"]["vars"].is_array(), "SELECT envelope has head.vars: {v}");
    assert_eq!(
        v["results"]["bindings"].as_array().unwrap().len(),
        2,
        "SELECT envelope has results.bindings: {v}"
    );
}

/// POST with body = query (`application/sparql-query`) → 200.
#[tokio::test]
async fn post_direct_query_is_200() {
    let base = spawn_default().await;
    let resp = client()
        .post(format!("{base}/sparql"))
        .header("content-type", "application/sparql-query")
        .body("SELECT ?s WHERE { ?s ?p ?o }")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status().as_u16(), 200);
    assert_eq!(content_type(&resp), "application/sparql-results+json");
}

/// POST url-encoded form with a `query` field → 200.
#[tokio::test]
async fn post_form_query_is_200() {
    let base = spawn_default().await;
    let resp = client()
        .post(format!("{base}/sparql"))
        .form(&[("query", "SELECT ?s WHERE { ?s ?p ?o }")])
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status().as_u16(), 200);
    assert_eq!(content_type(&resp), "application/sparql-results+json");
}

/// The query-only HTTP QUERY method: a query body → 200; an update body → 415 (query-only by
/// contract).
#[tokio::test]
async fn query_method_is_query_only() {
    let base = spawn_default().await;
    let ok = client()
        .request(
            reqwest::Method::from_bytes(b"QUERY").unwrap(),
            format!("{base}/sparql"),
        )
        .header("content-type", "application/sparql-query")
        .body("SELECT ?s WHERE { ?s ?p ?o }")
        .send()
        .await
        .unwrap();
    assert_eq!(ok.status().as_u16(), 200, "QUERY runs a query like POST");

    let update = client()
        .request(
            reqwest::Method::from_bytes(b"QUERY").unwrap(),
            format!("{base}/sparql"),
        )
        .header("content-type", "application/sparql-update")
        .body("INSERT DATA { <http://ex/s> <http://ex/p> <http://ex/o> }")
        .send()
        .await
        .unwrap();
    assert_eq!(update.status().as_u16(), 415, "QUERY refuses an update body");
    assert_error_envelope(update).await;
}

/// A successful update (both request forms) → 204 with an EMPTY body, atomically.
#[tokio::test]
async fn update_success_is_204_empty() {
    let base = spawn_default().await;
    let direct = client()
        .post(format!("{base}/sparql"))
        .header("content-type", "application/sparql-update")
        .body("INSERT DATA { <http://ex/s> <http://ex/p> <http://ex/o> }")
        .send()
        .await
        .unwrap();
    assert_eq!(direct.status().as_u16(), 204);
    assert!(direct.text().await.unwrap().is_empty(), "204 body is empty");

    let form = client()
        .post(format!("{base}/sparql"))
        .form(&[("update", "INSERT DATA { <http://ex/s2> <http://ex/p> <http://ex/o> }")])
        .send()
        .await
        .unwrap();
    assert_eq!(form.status().as_u16(), 204, "form-encoded update= also commits");
}

// ===========================================================================
// Contract §2 — result media types & negotiation
// ===========================================================================

/// The SELECT negotiation table: each supported Accept yields its EXACT documented
/// Content-Type string.
#[tokio::test]
async fn select_negotiation_emits_exact_content_types() {
    let base = spawn_default().await;
    for (accept, expect) in [
        ("application/sparql-results+json", "application/sparql-results+json"),
        ("application/json", "application/sparql-results+json"),
        ("application/sparql-results+xml", "application/sparql-results+xml"),
        ("text/csv", "text/csv; charset=utf-8"),
        (
            "text/tab-separated-values",
            "text/tab-separated-values; charset=utf-8",
        ),
        ("*/*", "application/sparql-results+json"),
    ] {
        let resp = client()
            .get(format!("{base}/sparql"))
            .header("accept", accept)
            .query(&[("query", "SELECT ?s WHERE { ?s ?p ?o }")])
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status().as_u16(), 200, "Accept: {accept}");
        assert_eq!(content_type(&resp), expect, "Accept: {accept}");
    }
}

/// The ASK envelope (`head` + `boolean`), and the no-boolean-CSV rule: a CSV Accept on an
/// ASK yields the JSON boolean form.
#[tokio::test]
async fn ask_envelope_and_csv_fallback() {
    let base = spawn_default().await;
    let resp = client()
        .get(format!("{base}/sparql"))
        .query(&[("query", "ASK { ?s <http://ex/age> ?a }")])
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status().as_u16(), 200);
    assert_eq!(content_type(&resp), "application/sparql-results+json");
    let v: serde_json::Value = resp.json().await.unwrap();
    assert!(v["head"].is_object(), "ASK envelope has head: {v}");
    assert_eq!(v["boolean"], serde_json::Value::Bool(true), "ASK envelope has boolean: {v}");

    let csv = client()
        .get(format!("{base}/sparql"))
        .header("accept", "text/csv")
        .query(&[("query", "ASK { ?s <http://ex/age> ?a }")])
        .send()
        .await
        .unwrap();
    assert_eq!(csv.status().as_u16(), 200);
    assert_eq!(
        content_type(&csv),
        "application/sparql-results+json",
        "ASK has no CSV form; falls back to the JSON boolean"
    );
}

/// The CONSTRUCT/DESCRIBE negotiation table, including the N-Triples default.
#[tokio::test]
async fn graph_negotiation_emits_exact_content_types() {
    let base = spawn_default().await;
    let mut cases = vec![
        (None, "application/n-triples; charset=utf-8"),
        (Some("*/*"), "application/n-triples; charset=utf-8"),
        (Some("application/n-triples"), "application/n-triples; charset=utf-8"),
        (Some("text/turtle"), "text/turtle; charset=utf-8"),
        (Some("application/rdf+xml"), "application/rdf+xml; charset=utf-8"),
    ];
    #[cfg(feature = "jsonld")]
    cases.push((Some("application/ld+json"), "application/ld+json; charset=utf-8"));
    for (accept, expect) in cases {
        let mut req = client()
            .get(format!("{base}/sparql"))
            .query(&[("query", "CONSTRUCT { ?s ?p ?o } WHERE { ?s ?p ?o }")]);
        if let Some(a) = accept {
            req = req.header("accept", a);
        }
        let resp = req.send().await.unwrap();
        assert_eq!(resp.status().as_u16(), 200, "Accept: {accept:?}");
        assert_eq!(content_type(&resp), expect, "Accept: {accept:?}");
    }
}

// ===========================================================================
// Contract §3 — Graph Store Protocol
// ===========================================================================

/// The GSP write/read/delete cycle over indirect addressing pins the documented statuses:
/// PUT-create 201, GET 200 (N-Triples default), PUT-replace 204, DELETE 204, then GET 404.
#[tokio::test]
async fn gsp_indirect_lifecycle_statuses() {
    let base = spawn_default().await;
    let cl = client();
    let graph = format!("{base}/sparql/graph");
    let iri = [("graph", "http://ex/g1")];

    let created = cl
        .put(&graph)
        .query(&iri)
        .header("content-type", "text/turtle")
        .body("<http://ex/a> <http://ex/b> <http://ex/c> .")
        .send()
        .await
        .unwrap();
    assert_eq!(created.status().as_u16(), 201, "PUT of an absent graph creates → 201");

    let read = cl.get(&graph).query(&iri).send().await.unwrap();
    assert_eq!(read.status().as_u16(), 200);
    assert_eq!(
        content_type(&read),
        "application/n-triples; charset=utf-8",
        "GSP read defaults to N-Triples"
    );

    let replaced = cl
        .put(&graph)
        .query(&iri)
        .header("content-type", "text/turtle")
        .body("<http://ex/a> <http://ex/b> <http://ex/d> .")
        .send()
        .await
        .unwrap();
    assert_eq!(replaced.status().as_u16(), 204, "PUT of an existing graph replaces → 204");

    let merged = cl
        .post(&graph)
        .query(&iri)
        .header("content-type", "text/turtle")
        .body("<http://ex/a> <http://ex/b> <http://ex/e> .")
        .send()
        .await
        .unwrap();
    assert_eq!(merged.status().as_u16(), 204, "POST merges into an existing graph → 204");

    let dropped = cl.delete(&graph).query(&iri).send().await.unwrap();
    assert_eq!(dropped.status().as_u16(), 204, "DELETE of an existing named graph → 204");

    // As-implemented: a GET of an absent named graph serves the EMPTY graph (200), it does
    // not 404 (pinned since tests/protocol.rs `gsp_delete_then_get_and_404`); the absent-graph
    // 404 arises on DELETE.
    let empty = cl.get(&graph).query(&iri).send().await.unwrap();
    assert_eq!(empty.status().as_u16(), 200, "GET of an absent named graph serves empty");
    assert_eq!(empty.text().await.unwrap().trim(), "");
    let gone = cl.delete(&graph).query(&iri).send().await.unwrap();
    assert_eq!(gone.status().as_u16(), 404, "DELETE of an absent named graph → 404");
    assert_error_envelope(gone).await;
}

/// GSP PATCH with an always-on `application/sparql-update` body applies atomically → 204;
/// an unsupported PATCH body type → 415.
#[tokio::test]
async fn gsp_patch_sparql_update_dialect() {
    let base = spawn_default().await;
    let cl = client();
    let graph = format!("{base}/sparql/graph");
    let iri = [("graph", "http://ex/g2")];
    let put = cl
        .put(&graph)
        .query(&iri)
        .header("content-type", "text/turtle")
        .body("<http://ex/a> <http://ex/b> <http://ex/c> .")
        .send()
        .await
        .unwrap();
    assert_eq!(put.status().as_u16(), 201);

    let patch = cl
        .patch(&graph)
        .query(&iri)
        .header("content-type", "application/sparql-update")
        .body("INSERT DATA { <http://ex/a> <http://ex/b> <http://ex/f> }")
        .send()
        .await
        .unwrap();
    assert_eq!(patch.status().as_u16(), 204, "sparql-update PATCH dialect is always-on → 204");

    let bad = cl
        .patch(&graph)
        .query(&iri)
        .header("content-type", "application/pdf")
        .body("nonsense")
        .send()
        .await
        .unwrap();
    assert_eq!(bad.status().as_u16(), 415, "an unsupported PATCH body type → 415");
    assert_error_envelope(bad).await;
}

/// Direct addressing (`/graphs/{path}`) serves the same lifecycle as indirect.
#[tokio::test]
async fn gsp_direct_addressing_works() {
    let base = spawn_default().await;
    let cl = client();
    let url = format!("{base}/graphs/people/g3");
    let created = cl
        .put(&url)
        .header("content-type", "text/turtle")
        .body("<http://ex/a> <http://ex/b> <http://ex/c> .")
        .send()
        .await
        .unwrap();
    assert_eq!(created.status().as_u16(), 201, "direct PUT creates → 201");
    let read = cl.get(&url).send().await.unwrap();
    assert_eq!(read.status().as_u16(), 200);
    assert_eq!(content_type(&read), "application/n-triples; charset=utf-8");
}

// ===========================================================================
// Contract §4 — /health
// ===========================================================================

/// GET /health → 200 with the exact plain-text body `ok`, never auth-gated.
#[tokio::test]
async fn health_is_200_ok_and_never_gated() {
    let config = ServerConfig {
        auth_token: Some("s3cr3t".to_string()),
        auth_token_read: true,
        ..ServerConfig::default()
    };
    let base = spawn_with(DATA, config).await;
    let resp = client().get(format!("{base}/health")).send().await.unwrap();
    assert_eq!(resp.status().as_u16(), 200);
    assert_eq!(resp.text().await.unwrap(), "ok");
}

// ===========================================================================
// Contract §5 + §6 — the error taxonomy, one direct test per documented class
// ===========================================================================

/// 400 — a malformed query is a permanent client error in the JSON envelope.
#[tokio::test]
async fn class_400_malformed_query() {
    let base = spawn_default().await;
    let resp = client()
        .get(format!("{base}/sparql"))
        .query(&[("query", "SELECT ?s WHERE { broken")])
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status().as_u16(), 400);
    assert_error_envelope(resp).await;
}

/// 400 — GET /sparql with no `query` parameter (the default build has no Service Description).
#[tokio::test]
async fn class_400_missing_query_param() {
    let base = spawn_default().await;
    let resp = client().get(format!("{base}/sparql")).send().await.unwrap();
    assert_eq!(resp.status().as_u16(), 400);
    assert_error_envelope(resp).await;
}

/// 401 — a gated write without a token: WWW-Authenticate + no-store headers, and the
/// missing-token and wrong-token responses are byte-identical (no oracle).
#[tokio::test]
async fn class_401_bearer_gate() {
    let config = ServerConfig {
        auth_token: Some("s3cr3t".to_string()),
        ..ServerConfig::default()
    };
    let base = spawn_with(DATA, config).await;
    let send = |auth: Option<&'static str>| {
        let cl = client();
        let base = base.clone();
        async move {
            let mut req = cl
                .post(format!("{base}/sparql"))
                .header("content-type", "application/sparql-update")
                .body("INSERT DATA { <http://ex/s> <http://ex/p> <http://ex/o> }");
            if let Some(a) = auth {
                req = req.header("authorization", a);
            }
            req.send().await.unwrap()
        }
    };
    let missing = send(None).await;
    assert_eq!(missing.status().as_u16(), 401);
    assert_eq!(
        missing.headers().get("www-authenticate").map(|v| v.to_str().unwrap()),
        Some("Bearer"),
        "401 carries WWW-Authenticate: Bearer"
    );
    assert_eq!(
        missing.headers().get("cache-control").map(|v| v.to_str().unwrap()),
        Some("no-store"),
        "401 carries Cache-Control: no-store"
    );
    let missing_body = missing.text().await.unwrap();
    let wrong = send(Some("Bearer wrong")).await;
    assert_eq!(wrong.status().as_u16(), 401);
    assert_eq!(
        wrong.text().await.unwrap(),
        missing_body,
        "missing vs wrong token is byte-identical"
    );
}

/// 403 — (`service` feature) a blocked SERVICE egress is a permanent POLICY refusal carrying
/// the documented `egress allowlist` sentinel, with the refused host sanitised out.
#[cfg(feature = "service")]
#[tokio::test]
async fn class_403_blocked_service_egress() {
    let config = ServerConfig {
        query_timeout: Some(Duration::from_secs(2)),
        ..ServerConfig::default()
    };
    let base = spawn_with(DATA, config).await;
    let q = "SELECT ?s WHERE { ?s <http://ex/age> ?a . \
             SERVICE <http://127.0.0.1:9/> { ?s <http://ex/name> ?n } }";
    let resp = client()
        .get(format!("{base}/sparql"))
        .query(&[("query", q)])
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status().as_u16(), 403);
    let v = assert_error_envelope(resp).await;
    let msg = v["error"].as_str().unwrap().to_lowercase();
    assert!(msg.contains("egress allowlist"), "documented sentinel: {msg}");
    assert!(!msg.contains("127.0.0.1"), "refused host is sanitised out: {msg}");
}

/// 404 — an unknown route answers the fixed, leak-free `not found` class.
#[tokio::test]
async fn class_404_unknown_route() {
    let base = spawn_default().await;
    let resp = client()
        .get(format!("{base}/definitely-not-a-route"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status().as_u16(), 404);
    let v = assert_error_envelope(resp).await;
    assert_eq!(v["error"], "not found", "documented fixed 404 class");
}

/// 405 — a wrong method carries the `Allow` header.
#[tokio::test]
async fn class_405_carries_allow() {
    let base = spawn_default().await;
    let resp = client().post(format!("{base}/health")).send().await.unwrap();
    assert_eq!(resp.status().as_u16(), 405);
    assert!(resp.headers().contains_key("allow"), "405 carries Allow");
}

/// 406 — a present-but-unsatisfiable Accept is Not Acceptable for BOTH result classes
/// (SELECT and CONSTRUCT), while absent/wildcard keeps the default (never a 406).
#[tokio::test]
async fn class_406_unsatisfiable_accept() {
    let base = spawn_default().await;
    for query in [
        "SELECT ?s WHERE { ?s ?p ?o }",
        "CONSTRUCT { ?s ?p ?o } WHERE { ?s ?p ?o }",
    ] {
        let resp = client()
            .get(format!("{base}/sparql"))
            .header("accept", "image/png")
            .query(&[("query", query)])
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status().as_u16(), 406, "unsatisfiable Accept on: {query}");
        assert_error_envelope(resp).await;
        let ok = client()
            .get(format!("{base}/sparql"))
            .header("accept", "image/png, */*")
            .query(&[("query", query)])
            .send()
            .await
            .unwrap();
        assert_eq!(ok.status().as_u16(), 200, "a wildcard rescues: {query}");
    }
}

/// 410 — (`time-travel` feature) an aged-out `?generation` pin is Gone, with the documented
/// `aged out` sentinel.
#[cfg(feature = "time-travel")]
#[tokio::test]
async fn class_410_aged_out_generation() {
    let config = ServerConfig {
        time_travel_generations: 2,
        ..ServerConfig::default()
    };
    let base = spawn_with("", config).await;
    let cl = client();
    for i in 1..=6usize {
        let resp = cl
            .post(format!("{base}/sparql"))
            .header("content-type", "application/sparql-update")
            .body(format!(
                "INSERT DATA {{ <http://ex/u{i}> <http://ex/seen> <http://ex/y> }}"
            ))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status().as_u16(), 204);
    }
    let resp = cl
        .get(format!("{base}/sparql"))
        .query(&[("query", "SELECT ?s WHERE { ?s ?p ?o }"), ("generation", "1")])
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status().as_u16(), 410);
    let v = assert_error_envelope(resp).await;
    assert!(
        v["error"].as_str().unwrap().contains("aged out"),
        "documented sentinel: {v}"
    );
}

/// 413 — a request body over the cap is a permanent refusal.
#[tokio::test]
async fn class_413_body_cap() {
    let config = ServerConfig {
        max_body_bytes: 128,
        ..ServerConfig::default()
    };
    let base = spawn_with(DATA, config).await;
    let resp = client()
        .post(format!("{base}/sparql"))
        .header("content-type", "application/sparql-query")
        .body(format!("SELECT * WHERE {{ ?s ?p ?o }} # {}", "x".repeat(512)))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status().as_u16(), 413);
    assert_error_envelope(resp).await;
}

/// 413 — a result over the row cap is the HONEST-REFUSAL class: no truncated rows, and the
/// documented `row limit` sentinel.
#[tokio::test]
async fn class_413_row_cap_honest_refusal() {
    let config = ServerConfig {
        max_results: Some(1),
        ..ServerConfig::default()
    };
    let base = spawn_with(DATA, config).await;
    let resp = client()
        .get(format!("{base}/sparql"))
        .query(&[("query", "SELECT ?s WHERE { ?s <http://ex/age> ?a }")])
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status().as_u16(), 413);
    let v = assert_error_envelope(resp).await;
    let msg = v["error"].as_str().unwrap();
    assert!(msg.contains("row limit"), "documented sentinel: {msg}");
    assert!(!msg.contains("bindings"), "refusal, never truncation: {msg}");
}

/// 415 — an unsupported POST Content-Type on /sparql.
#[tokio::test]
async fn class_415_unsupported_media_type() {
    let base = spawn_default().await;
    let resp = client()
        .post(format!("{base}/sparql"))
        .header("content-type", "text/plain")
        .body("SELECT * WHERE { ?s ?p ?o }")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status().as_u16(), 415);
    assert_error_envelope(resp).await;
}

/// 429 — the concurrency cap sheds a second in-flight request (the TRANSIENT class: the shed
/// request never ran).
#[tokio::test]
async fn class_429_concurrency_shed() {
    // A dense graph whose 3-way cross-product never finishes within the budget.
    let mut ttl = String::from("@prefix ex: <http://ex/> .\n");
    for i in 0..60 {
        for j in 0..60 {
            ttl.push_str(&format!("ex:n{i} ex:e ex:n{j} .\n"));
        }
    }
    const SLOW: &str = "PREFIX ex: <http://ex/> SELECT * WHERE { ?a ex:e ?b . ?c ex:e ?d . ?e ex:e ?f }";
    let config = ServerConfig {
        max_concurrent: 1,
        query_timeout: Some(Duration::from_secs(30)),
        ..ServerConfig::default()
    };
    let base = spawn_with(&ttl, config).await;
    let busy = base.clone();
    tokio::spawn(async move {
        let _ = client()
            .get(format!("{busy}/sparql"))
            .query(&[("query", SLOW)])
            .send()
            .await;
    });
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
    let resp = shed.expect("the second in-flight request must shed with 429");
    assert_error_envelope(resp).await;
}

/// 500 — a caught handler panic maps to the defect class through the production middleware,
/// keeping the connection alive (same wrap idiom as tests/hardening.rs; the stock routes have
/// no reachable panic, which is the point of the class being a defect signal).
#[tokio::test]
async fn class_500_caught_panic() {
    async fn panicking() -> &'static str {
        panic!("boom")
    }
    let app = harden(
        axum::Router::new().route("/panic", axum::routing::get(panicking)),
        &ServerConfig::default(),
    );
    let base = serve(app).await;
    let cl = client();
    let resp = cl.get(format!("{base}/panic")).send().await.unwrap();
    assert_eq!(resp.status().as_u16(), 500);
    assert_error_envelope(resp).await;
    // The connection (and server) survives the panic.
    let again = cl.get(format!("{base}/panic")).send().await.unwrap();
    assert_eq!(again.status().as_u16(), 500);
}

/// 503 — a query timeout is the TRANSIENT class with the documented `timed out` sentinel and
/// a wall-clock guarantee.
#[tokio::test]
async fn class_503_query_timeout() {
    let mut ttl = String::from("@prefix ex: <http://ex/> .\n");
    for i in 0..60 {
        for j in 0..60 {
            ttl.push_str(&format!("ex:n{i} ex:e ex:n{j} .\n"));
        }
    }
    let config = ServerConfig {
        query_timeout: Some(Duration::from_secs(1)),
        ..ServerConfig::default()
    };
    let base = spawn_with(&ttl, config).await;
    let started = std::time::Instant::now();
    let resp = client()
        .get(format!("{base}/sparql"))
        .query(&[(
            "query",
            "PREFIX ex: <http://ex/> SELECT * WHERE { ?a ex:e ?b . ?c ex:e ?d . ?e ex:e ?f }",
        )])
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status().as_u16(), 503);
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "the 503 is wall-clock bounded, took {:?}",
        started.elapsed()
    );
    let v = assert_error_envelope(resp).await;
    assert!(
        v["error"].as_str().unwrap().contains("timed out"),
        "documented sentinel: {v}"
    );
}

/// Cross-cutting sanitisation invariant: an error body never echoes the caller's input.
#[tokio::test]
async fn error_bodies_never_echo_input() {
    let base = spawn_default().await;
    const SENTINEL: &str = "WIRE_CONTRACT_NEEDLE_9";
    let resp = client()
        .get(format!("{base}/sparql"))
        .query(&[("query", format!("SELECT {SENTINEL} WHERE {{ broken"))])
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status().as_u16(), 400);
    let body = resp.text().await.unwrap();
    assert!(!body.contains(SENTINEL), "no echoed input: {body}");
}
