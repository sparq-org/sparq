//! sq-r868 (from-pss gh-162 follow-up (c)): e2e integration tests for the
//! OPT-IN HTTP SHACL validate endpoint (`POST /shacl/validate`).
//!
//! These tests run ONLY with the `shacl` cargo feature (the whole file is gated). They spin
//! the real axum server and assert, over an HTTP request:
//!   * the OPT-IN posture — `/shacl/validate` is `404` unless the config flag is set;
//!   * the server's loaded DATA graph is validated against the POSTed SHACL SHAPES graph;
//!   * a conforming store yields `{ "conforms": true, "results": [] }`;
//!   * a violating store yields `conforms:false` with the PSS-consumed JSON report shape
//!     (`focusNode` / `path` / `value` / `sourceShape` / `sourceConstraintComponent` /
//!     `severity` / `message`);
//!   * content negotiation — JSON by default, the W3C report Turtle on `Accept: text/turtle`;
//!   * a malformed shapes body is a 400, an unsupported media type a 415;
//!   * the wrong HTTP method is a 405;
//!   * read-auth gating (`--auth-token-read`) — 401 without the bearer token, 200 with it.
#![cfg(feature = "shacl")]

use sparq_core::Graph;
use sparq_server::{router, AppState, ServerConfig, ShaclShapes};
use tokio::net::TcpListener;

/// A store with one `ex:Person` whose `ex:age` is a string (violates `xsd:integer`).
const VIOLATING_DATA: &str = r#"
    @prefix ex: <http://example.org/> .
    ex:alice a ex:Person ; ex:age "thirty" .
"#;

/// A store with one `ex:Person` whose `ex:age` is a valid integer.
const CONFORMING_DATA: &str = r#"
    @prefix ex: <http://example.org/> .
    ex:bob a ex:Person ; ex:age 42 .
"#;

const SHAPES: &str = r#"
    @prefix sh:  <http://www.w3.org/ns/shacl#> .
    @prefix ex:  <http://example.org/> .
    @prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
    ex:PersonShape a sh:NodeShape ;
      sh:targetClass ex:Person ;
      sh:property [ sh:path ex:age ; sh:datatype xsd:integer ; sh:minCount 1 ] .
"#;

/// Boots a server holding `data` with the SHACL flag set, returns its base URL.
async fn spawn_with(data: &str, shacl_on: bool, config: ServerConfig) -> String {
    let graph = Graph::load_str(data, "turtle").unwrap();
    let config = ServerConfig {
        shacl: shacl_on,
        ..config
    };
    let app = router(AppState::with_config(graph, config));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("http://{addr}")
}

async fn spawn(data: &str, shacl_on: bool) -> String {
    spawn_with(data, shacl_on, ServerConfig::default()).await
}

/// sq-lsp7k.2.4: boots guard mode with pre-loaded shapes.
async fn spawn_guard(data: &str, guard_on: bool) -> String {
    spawn_with(data, false, ServerConfig {
        shacl_guard: guard_on,
        shacl_shapes: Some(ShaclShapes::new(Graph::load_str(SHAPES, "turtle").unwrap())),
        ..ServerConfig::default()
    }).await
}

fn client() -> reqwest::Client {
    reqwest::Client::new()
}

// ---------------------------------------------------------------------------
// OPT-IN posture.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn shacl_404_when_flag_off() {
    let base = spawn(VIOLATING_DATA, false).await;
    let resp = client()
        .post(format!("{base}/shacl/validate"))
        .header("Content-Type", "text/turtle")
        .body(SHAPES)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 404);
}

// ---------------------------------------------------------------------------
// Validation verdicts (JSON report — the PSS-consumed shape).
// ---------------------------------------------------------------------------

#[tokio::test]
async fn violating_store_reports_violation_json() {
    let base = spawn(VIOLATING_DATA, true).await;
    let resp = client()
        .post(format!("{base}/shacl/validate"))
        .header("Content-Type", "text/turtle")
        .body(SHAPES)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    assert_eq!(
        resp.headers()["content-type"],
        "application/json; charset=utf-8"
    );
    let body = resp.text().await.unwrap();
    assert!(body.contains("\"conforms\":false"), "{body}");
    // The PSS-consumed JSON projection fields.
    assert!(body.contains("\"focusNode\":"), "{body}");
    assert!(body.contains("\"sourceConstraintComponent\":"), "{body}");
    assert!(
        body.contains("DatatypeConstraintComponent"),
        "expected the datatype violation: {body}"
    );
    assert!(body.contains("http://example.org/alice"), "{body}");
}

#[tokio::test]
async fn conforming_store_reports_conformance_json() {
    let base = spawn(CONFORMING_DATA, true).await;
    let resp = client()
        .post(format!("{base}/shacl/validate"))
        .header("Content-Type", "text/turtle")
        .body(SHAPES)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body = resp.text().await.unwrap();
    assert!(body.contains("\"conforms\":true"), "{body}");
    assert!(body.contains("\"results\":[]"), "{body}");
}

#[tokio::test]
async fn guard_rejects_violating_update_without_publishing_then_accepts_correction() {
    let base = spawn_guard(CONFORMING_DATA, true).await;
    let rejected = client().post(format!("{base}/sparql"))
        .header("Content-Type", "application/sparql-update")
        .body("DELETE { <http://example.org/bob> <http://example.org/age> ?age } INSERT { <http://example.org/bob> <http://example.org/age> \"forty-two\" } WHERE { <http://example.org/bob> <http://example.org/age> ?age }")
        .send().await.unwrap();
    assert_eq!(rejected.status(), 422);
    assert_eq!(rejected.headers()["content-type"], "application/json; charset=utf-8");
    let report = rejected.text().await.unwrap();
    assert!(report.contains("\"conforms\":false"), "{report}");
    assert!(report.contains("DatatypeConstraintComponent"), "{report}");

    let unchanged = client().get(format!("{base}/sparql"))
        .query(&[("query", "ASK { <http://example.org/bob> <http://example.org/age> 42 }")])
        .send().await.unwrap().text().await.unwrap();
    assert!(unchanged.contains("\"boolean\":true"));

    let corrected = client().post(format!("{base}/sparql"))
        .header("Content-Type", "application/sparql-update")
        .body("DELETE DATA { <http://example.org/bob> <http://example.org/age> 42 } ; INSERT DATA { <http://example.org/bob> <http://example.org/age> 43 }")
        .send().await.unwrap();
    assert_eq!(corrected.status(), 204);
}

#[tokio::test]
async fn guard_covers_gsp_and_runtime_off_preserves_write_path() {
    const BAD: &str = "<http://example.org/alice> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://example.org/Person> .\n<http://example.org/alice> <http://example.org/age> <http://example.org/not-an-integer> .";
    let guarded = spawn_guard(CONFORMING_DATA, true).await;
    let rejected = client().post(format!("{guarded}/sparql/graph?default"))
        .header("Content-Type", "text/turtle").body(BAD).send().await.unwrap();
    let status = rejected.status();
    let body = rejected.text().await.unwrap();
    assert_eq!(status, 422, "{body}");
    assert!(body.contains("\"conforms\":false"));

    let unchanged = client().get(format!("{guarded}/sparql"))
        .query(&[("query", "ASK { <http://example.org/bob> <http://example.org/age> 42 }")])
        .send().await.unwrap().text().await.unwrap();
    assert!(unchanged.contains("\"boolean\":true"));

    let unguarded = spawn_guard(CONFORMING_DATA, false).await;
    let accepted = client().post(format!("{unguarded}/sparql/graph?default"))
        .header("Content-Type", "text/turtle").body(BAD).send().await.unwrap();
    assert!(accepted.status().is_success(), "{}", accepted.status());
}

/// (PR #1999 security review) A guard shapes graph whose `sh:pattern` regex does
/// not compile: well-formed at parse time (the value IS a string literal), but the constraint
/// is unevaluable — the lenient validator SKIPS it and reports `conforms:true`.
const UNEVALUABLE_PATTERN_SHAPES: &str = r#"
    @prefix sh: <http://www.w3.org/ns/shacl#> .
    @prefix ex: <http://example.org/> .
    ex:PersonShape a sh:NodeShape ;
      sh:targetClass ex:Person ;
      sh:property [ sh:path ex:age ; sh:pattern "(unclosed" ] .
"#;

/// A guard shapes graph with a parse-time ill-formed construct (a non-integer
/// `sh:minCount` — a violated SHACL syntax rule the lenient validator silently skips).
const ILL_FORMED_SHAPES: &str = r#"
    @prefix sh: <http://www.w3.org/ns/shacl#> .
    @prefix ex: <http://example.org/> .
    ex:PersonShape a sh:NodeShape ;
      sh:targetClass ex:Person ;
      sh:property [ sh:path ex:age ; sh:minCount "one" ] .
"#;

/// (PR #1999 security review) The guard FAILS CLOSED on an unevaluable shapes
/// graph — it must never silently ADMIT a write just because the operator's shapes graph
/// could not be (fully) evaluated. Two layers:
///   (a) runtime: an uncompilable `sh:pattern` (NOT parse-time detectable) rejects the
///       write with a sanitized 500 and publishes nothing — the lenient validator would
///       have skipped the constraint and admitted the write;
///   (b) startup: a parse-time ill-formed construct is a hard `try_with_config` error, so a
///       mis-authored guard never boots at all.
#[tokio::test]
async fn guard_fails_closed_on_unevaluable_shapes() {
    // (a) Runtime fail-closed. Boots cleanly (the shapes graph is well-FORMED)…
    let base = spawn_with(CONFORMING_DATA, false, ServerConfig {
        shacl_guard: true,
        shacl_shapes: Some(ShaclShapes::new(Graph::load_str(UNEVALUABLE_PATTERN_SHAPES, "turtle").unwrap())),
        ..ServerConfig::default()
    }).await;
    let resp = client().post(format!("{base}/sparql"))
        .header("Content-Type", "application/sparql-update")
        .body("INSERT DATA { <http://example.org/mallory> a <http://example.org/Person> ; <http://example.org/age> \"not-an-integer\" }")
        .send().await.unwrap();
    // …but the guarded write is REJECTED (5xx: the operator's misconfiguration, not the
    // client's data), never admitted past the unevaluable guard.
    assert_eq!(resp.status(), 500, "an unevaluable guard constraint must fail CLOSED, not silently admit the write");
    let body = resp.text().await.unwrap();
    assert!(!body.contains("(unclosed"), "shapes-graph fragment must not leak into the response body: {body}");
    // Nothing was published.
    let unchanged = client().get(format!("{base}/sparql"))
        .query(&[("query", "ASK { <http://example.org/mallory> ?p ?o }")])
        .send().await.unwrap().text().await.unwrap();
    assert!(unchanged.contains("\"boolean\":false"), "{unchanged}");

    // (b) Startup fail-closed: a parse-time ill-formed guard shape never boots.
    let graph = Graph::load_str(CONFORMING_DATA, "turtle").unwrap();
    let err = AppState::try_with_config(graph, ServerConfig {
        shacl_guard: true,
        shacl_shapes: Some(ShaclShapes::new(Graph::load_str(ILL_FORMED_SHAPES, "turtle").unwrap())),
        ..ServerConfig::default()
    }).err().expect("an ill-formed guard shapes graph must be a hard startup error");
    assert!(err.contains("not fully evaluable"), "{err}");
}

#[test]
fn guard_requires_loaded_shapes_graph() {
    let graph = Graph::load_str(CONFORMING_DATA, "turtle").unwrap();
    let error = AppState::try_with_config(graph, ServerConfig {
        shacl_guard: true, ..ServerConfig::default()
    }).err().expect("guard without shapes must fail closed");
    assert!(error.contains("requires a loaded shapes graph"), "{error}");
}

// ---------------------------------------------------------------------------
// Content negotiation — W3C report Turtle.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn turtle_report_on_accept_turtle() {
    let base = spawn(VIOLATING_DATA, true).await;
    let resp = client()
        .post(format!("{base}/shacl/validate"))
        .header("Content-Type", "text/turtle")
        .header("Accept", "text/turtle")
        .body(SHAPES)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    assert_eq!(resp.headers()["content-type"], "text/turtle; charset=utf-8");
    let body = resp.text().await.unwrap();
    // The W3C SHACL report vocabulary.
    assert!(body.contains("ValidationReport"), "{body}");
    assert!(body.contains("conforms"), "{body}");
    assert!(body.contains("ValidationResult"), "{body}");
}

// ---------------------------------------------------------------------------
// Error handling.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn malformed_shapes_body_is_400() {
    let base = spawn(VIOLATING_DATA, true).await;
    let resp = client()
        .post(format!("{base}/shacl/validate"))
        .header("Content-Type", "text/turtle")
        .body("this is not valid turtle @@@")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 400);
}

#[tokio::test]
async fn unsupported_media_type_is_415() {
    let base = spawn(VIOLATING_DATA, true).await;
    let resp = client()
        .post(format!("{base}/shacl/validate"))
        .header("Content-Type", "application/pdf")
        .body(SHAPES)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 415);
}

#[tokio::test]
async fn get_is_405() {
    let base = spawn(VIOLATING_DATA, true).await;
    let resp = client()
        .get(format!("{base}/shacl/validate"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 405);
}

// ---------------------------------------------------------------------------
// Read-auth gating — validation is a READ over the store, gated like any GET.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn read_auth_gates_the_endpoint() {
    let config = ServerConfig {
        auth_token: Some("s3cret".to_string()),
        auth_token_read: true,
        ..ServerConfig::default()
    };
    let base = spawn_with(VIOLATING_DATA, true, config).await;

    // No token → 401.
    let resp = client()
        .post(format!("{base}/shacl/validate"))
        .header("Content-Type", "text/turtle")
        .body(SHAPES)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 401);

    // Correct token → 200.
    let resp = client()
        .post(format!("{base}/shacl/validate"))
        .header("Content-Type", "text/turtle")
        .header("Authorization", "Bearer s3cret")
        .body(SHAPES)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
}
