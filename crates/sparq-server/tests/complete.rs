//! sq-lsp7k.9.3: end-to-end tests for the double-opt-in `GET /complete` endpoint.
//!
//! The suite drives the real axum route over a generation-cached `CompletionIndex`. It witnesses
//! runtime-off 404, exact IRI/local-name/label JSON, a post-update stale-cache rebuild, and the
//! required `q` parameter.
#![cfg(feature = "complete")]

use serde_json::{json, Value};
use sparq_core::Graph;
use sparq_server::{router, AppState, ServerConfig};
use tokio::net::TcpListener;

const RDFS_LABEL: &str = "http://www.w3.org/2000/01/rdf-schema#label";

const DATA: &str = r#"
    @prefix ex: <http://ex/> .
    @prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
    ex:alice rdfs:label "Alice" ; ex:knows ex:bob .
    ex:bob rdfs:label "Robert" .
"#;

async fn spawn_with(complete_on: bool, config: ServerConfig) -> String {
    let graph = Graph::load_str(DATA, "turtle").unwrap();
    let config = ServerConfig {
        complete: complete_on,
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

async fn spawn(complete_on: bool) -> String {
    spawn_with(complete_on, ServerConfig::default()).await
}

async fn candidates(base: &str, prefix: &str) -> reqwest::Response {
    reqwest::Client::new()
        .get(format!("{base}/complete"))
        .query(&[("q", prefix), ("limit", "100")])
        .send()
        .await
        .unwrap()
}

#[tokio::test]
async fn complete_404_when_runtime_flag_is_off() {
    let base = spawn(false).await;
    let response = candidates(&base, "al").await;
    assert_eq!(response.status(), 404);
}

#[tokio::test]
async fn prefix_returns_exact_local_name_and_label_candidates() {
    let base = spawn(true).await;
    let response = candidates(&base, "al").await;
    assert_eq!(response.status(), 200);
    assert_eq!(
        response.headers()["content-type"],
        "application/json; charset=utf-8"
    );
    let actual: Value = response.json().await.unwrap();
    assert_eq!(
        actual,
        json!([
            {
                "iri": "http://ex/alice",
                "key": "alice",
                "kind": "localName",
                "score": 0.0
            },
            {
                "iri": "http://ex/alice",
                "key": "alice",
                "kind": "label",
                "score": 0.0
            }
        ])
    );
}

#[tokio::test]
async fn update_rebuilds_stale_completion_index() {
    let base = spawn(true).await;
    let client = reqwest::Client::new();

    // Populate the generation-zero cache before the update. Without generation reconciliation,
    // the second lookup would incorrectly keep returning this empty result.
    let before: Value = candidates(&base, "zel").await.json().await.unwrap();
    assert_eq!(before, json!([]));

    let update = format!("INSERT DATA {{ <http://ex/new-entity> <{RDFS_LABEL}> \"Zelda\" }}");
    let response = client
        .post(format!("{base}/sparql"))
        .header("content-type", "application/sparql-update")
        .body(update)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 204);

    let after = candidates(&base, "zel").await;
    assert_eq!(after.status(), 200);
    let actual: Value = after.json().await.unwrap();
    assert_eq!(
        actual,
        json!([{
            "iri": "http://ex/new-entity",
            "key": "zelda",
            "kind": "label",
            "score": 0.0
        }])
    );
}

#[tokio::test]
async fn missing_q_is_400() {
    let base = spawn(true).await;
    let response = reqwest::Client::new()
        .get(format!("{base}/complete"))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 400);
    assert_eq!(
        response.text().await.unwrap(),
        r#"{"error":"missing 'q' parameter"}"#
    );
}
