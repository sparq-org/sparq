//! sq-lsp7k.5.2: end-to-end tests for the double-opt-in `POST /facets` endpoint.
//!
//! The suite drives the real axum route and `sparq-introspect` scan path over a pinned server
//! snapshot. It witnesses the runtime-off 404, exact class-filtered facet counts, malformed-JSON
//! 400, and the read-auth gate.
#![cfg(feature = "facets")]

use serde_json::{json, Value};
use sparq_core::Graph;
use sparq_server::{router, AppState, ServerConfig};
use tokio::net::TcpListener;

const RDF_TYPE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";
const XSD_INTEGER: &str = "http://www.w3.org/2001/XMLSchema#integer";

const DATA: &str = r#"
    @prefix ex: <http://ex/> .
    ex:alice a ex:Person ; ex:status ex:active ; ex:tag "red", "blue" ; ex:age 30 .
    ex:bob   a ex:Person ; ex:status ex:active ; ex:tag "red" ; ex:age 40 .
    ex:carol a ex:Person ; ex:status ex:inactive ; ex:tag "blue" .
    ex:robot a ex:Robot  ; ex:status ex:active ; ex:tag "red" .
"#;

async fn spawn_with(facets_on: bool, config: ServerConfig) -> String {
    let graph = Graph::load_str(DATA, "turtle").unwrap();
    let config = ServerConfig {
        facets: facets_on,
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

async fn spawn(facets_on: bool) -> String {
    spawn_with(facets_on, ServerConfig::default()).await
}

fn facet_request() -> Value {
    json!({
        "class": "http://ex/Person",
        "constraints": [["http://ex/status", "<http://ex/active>"]],
        "facet_predicates": ["http://ex/age", "http://ex/tag"],
        "top_k": 10
    })
}

#[tokio::test]
async fn facets_404_when_runtime_flag_is_off() {
    let base = spawn(false).await;
    let response = reqwest::Client::new()
        .post(format!("{base}/facets"))
        .json(&facet_request())
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 404);
}

#[tokio::test]
async fn class_filtered_request_returns_exact_hand_computed_counts() {
    let base = spawn(true).await;
    let response = reqwest::Client::new()
        .post(format!("{base}/facets"))
        .json(&facet_request())
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(
        response.headers()["content-type"],
        "application/json; charset=utf-8"
    );

    let actual: Value = response.json().await.unwrap();
    let expected = json!({
        "candidates": 2,
        "types": [
            {"iri": "http://ex/Person", "count": 2}
        ],
        "predicates": [
            {"iri": "http://ex/tag", "count": 3},
            {"iri": "http://ex/age", "count": 2},
            {"iri": "http://ex/status", "count": 2},
            {"iri": RDF_TYPE, "count": 2}
        ],
        "values": [
            {
                "predicate": "http://ex/age",
                "values": [
                    {"iri": format!("\"30\"^^<{XSD_INTEGER}>"), "count": 1},
                    {"iri": format!("\"40\"^^<{XSD_INTEGER}>"), "count": 1}
                ],
                "elided": 0
            },
            {
                "predicate": "http://ex/tag",
                "values": [
                    {"iri": "\"red\"", "count": 2},
                    {"iri": "\"blue\"", "count": 1}
                ],
                "elided": 0
            }
        ]
    });
    assert_eq!(actual, expected);
}

#[tokio::test]
async fn malformed_json_is_400() {
    let base = spawn(true).await;
    let response = reqwest::Client::new()
        .post(format!("{base}/facets"))
        .header("content-type", "application/json")
        .body(r#"{"class":"facet_secret_sentinel""#)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 400);
    let body = response.text().await.unwrap();
    assert_eq!(body, r#"{"error":"malformed facet request"}"#);
    assert!(!body.contains("facet_secret_sentinel"));
}

#[tokio::test]
async fn facets_are_gated_as_a_read() {
    let config = ServerConfig {
        auth_token: Some("s3cret".to_string()),
        auth_token_read: true,
        ..ServerConfig::default()
    };
    let base = spawn_with(true, config).await;
    let client = reqwest::Client::new();

    let response = client
        .post(format!("{base}/facets"))
        .json(&facet_request())
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 401);

    let response = client
        .post(format!("{base}/facets"))
        .header("authorization", "Bearer s3cret")
        .json(&facet_request())
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
}
