//! sq-vczh2 (epic sq-2m6zm, design research/llm-ergonomic-sparql-surface.md §4):
//! e2e integration tests for the OPT-IN verifiable terse-transpiler endpoint
//! (`POST /terse/transpile`).
//!
//! These tests run ONLY with the `terse` cargo feature (the whole file is gated). They spin the
//! real axum server and assert, over an HTTP request, that the endpoint exercises the REAL
//! transpiler path (not a mock) and upholds the load-bearing invariants:
//!   * the OPT-IN posture — `/terse/transpile` is `404` unless the config flag is set;
//!   * the verifiable contract — the `K:<name>` keyword layer expands to CANONICAL SPARQL and the
//!     expansion is echoed in `keywords` + `legendVersion` (so the network contract is the
//!     auditable expansion, not an opaque answer);
//!   * identity pass-through — canonical SPARQL with no terse token round-trips unchanged;
//!   * loud-fail discipline — an unknown `K:<name>` keyword is a `400` (never a silent guess),
//!     and a `V("phrase")` construct is a `400` in this lean (no-`vectors`) server build (concept
//!     resolution is a future extension; the V() ambiguity caveat is tracked by sq-26fdp);
//!   * the wrong HTTP method is a `405`;
//!   * read-auth gating (`--auth-token-read`) — `401` without the bearer token, `200` with it.
#![cfg(feature = "terse")]

use sparq_core::Graph;
use sparq_server::{router, AppState, ServerConfig};
use tokio::net::TcpListener;

/// A minimal store — the transpiler never reads it (transpiling is purely lexical), but the
/// server needs a graph to boot.
const DATA: &str = r#"
    @prefix ex: <http://example.org/> .
    ex:alice a ex:Person .
"#;

/// Boots a server with the terse flag set as requested, returns its base URL.
async fn spawn_with(terse_on: bool, config: ServerConfig) -> String {
    let graph = Graph::load_str(DATA, "turtle").unwrap();
    let config = ServerConfig {
        terse: terse_on,
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

async fn spawn(terse_on: bool) -> String {
    spawn_with(terse_on, ServerConfig::default()).await
}

fn client() -> reqwest::Client {
    reqwest::Client::new()
}

// ---------------------------------------------------------------------------
// OPT-IN posture.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn terse_404_when_flag_off() {
    let base = spawn(false).await;
    let resp = client()
        .post(format!("{base}/terse/transpile"))
        .body("SELECT ?s WHERE { ?s ?p ?o }")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 404);
}

// ---------------------------------------------------------------------------
// The verifiable contract — keyword expansion echoed against canonical SPARQL.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn keyword_expands_to_canonical_sparql_and_is_echoed() {
    let base = spawn(true).await;
    let resp = client()
        .post(format!("{base}/terse/transpile"))
        .body("SELECT ?s WHERE { ?s K:derivedFrom ?o }")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    assert_eq!(
        resp.headers()["content-type"],
        "application/json; charset=utf-8"
    );
    let body = resp.text().await.unwrap();
    // The K:derivedFrom keyword expanded to the canonical PROV-O IRI — the actual transpiler ran.
    assert!(
        body.contains("<http://www.w3.org/ns/prov#wasDerivedFrom>"),
        "canonical SPARQL should carry the expanded IRI: {body}"
    );
    // The expansion is echoed (the network contract is the auditable expansion).
    assert!(body.contains("\"keyword\":\"derivedFrom\""), "{body}");
    assert!(
        body.contains("\"iri\":\"http://www.w3.org/ns/prov#wasDerivedFrom\""),
        "{body}"
    );
    // The frozen legend version is published with the response.
    assert!(body.contains("\"legendVersion\":\"pkg-keywords/v1\""), "{body}");
    // Lean server build: no V() resolution path, so resolutions is always empty.
    assert!(body.contains("\"resolutions\":[]"), "{body}");
}

#[tokio::test]
async fn identity_passthrough_round_trips_unchanged() {
    let base = spawn(true).await;
    let q = "SELECT ?s WHERE { ?s ?p ?o }";
    let resp = client()
        .post(format!("{base}/terse/transpile"))
        .body(q)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body = resp.text().await.unwrap();
    // A query with no terse token is emitted byte-for-byte unchanged, with empty keywords.
    assert!(body.contains(q), "{body}");
    assert!(body.contains("\"keywords\":[]"), "{body}");
}

// ---------------------------------------------------------------------------
// Loud-fail discipline — never a silent guess.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn unknown_keyword_is_400() {
    let base = spawn(true).await;
    let resp = client()
        .post(format!("{base}/terse/transpile"))
        .body("SELECT ?s WHERE { ?s K:bogus ?o }")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 400);
    let body = resp.text().await.unwrap();
    assert!(body.contains("unknown keyword"), "{body}");
}

#[tokio::test]
async fn v_construct_is_400_in_lean_build() {
    let base = spawn(true).await;
    let resp = client()
        .post(format!("{base}/terse/transpile"))
        .body("SELECT ?s WHERE { ?s ?p V(\"some concept\") }")
        .send()
        .await
        .unwrap();
    // The lean (no-`vectors`) server build cannot resolve V(): it loud-fails rather than guessing
    // (the V() ambiguity caveat is tracked by sq-26fdp).
    assert_eq!(resp.status(), 400);
    let body = resp.text().await.unwrap();
    assert!(body.contains("vectors"), "{body}");
}

#[tokio::test]
async fn non_parsing_input_is_400_via_canary() {
    let base = spawn(true).await;
    let resp = client()
        .post(format!("{base}/terse/transpile"))
        .body("this is not sparql at all")
        .send()
        .await
        .unwrap();
    // The silent-rewrite canary refuses to emit non-conformant SPARQL.
    assert_eq!(resp.status(), 400);
}

// ---------------------------------------------------------------------------
// Method + auth gating.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn wrong_method_is_405() {
    let base = spawn(true).await;
    let resp = client()
        .get(format!("{base}/terse/transpile"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 405);
}

#[tokio::test]
async fn read_auth_gates_transpile() {
    let config = ServerConfig {
        auth_token: Some("s3cret".to_string()),
        auth_token_read: true,
        ..ServerConfig::default()
    };
    let base = spawn_with(true, config).await;

    // No token → 401.
    let resp = client()
        .post(format!("{base}/terse/transpile"))
        .body("SELECT ?s WHERE { ?s ?p ?o }")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 401);

    // With the token → 200.
    let resp = client()
        .post(format!("{base}/terse/transpile"))
        .header("Authorization", "Bearer s3cret")
        .body("SELECT ?s WHERE { ?s ?p ?o }")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
}
