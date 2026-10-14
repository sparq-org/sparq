//! (sq-abhuq) Mutation witnesses for opt-in response compression.

#![cfg(all(feature = "server", feature = "response-compression"))]

use std::io::Read;

use flate2::read::GzDecoder;
use sparq_core::Graph;
use sparq_server::{router, AppState};
use tokio::net::TcpListener;

async fn spawn() -> String {
    let graph =
        Graph::load_str("<http://ex/alice> <http://ex/name> \"Alice\" .", "ntriples").unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, router(AppState::new(graph)))
            .await
            .unwrap();
    });
    format!("http://{addr}")
}

fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .no_gzip()
        .no_zstd()
        .build()
        .unwrap()
}

#[tokio::test]
async fn gzip_select_decodes_to_the_unencoded_result() {
    let base = spawn().await;
    let query = "SELECT ?s ?name WHERE { ?s <http://ex/name> ?name }";
    let plain = client()
        .get(format!("{base}/sparql"))
        .query(&[("query", query)])
        .send()
        .await
        .unwrap()
        .bytes()
        .await
        .unwrap();
    let compressed = client()
        .get(format!("{base}/sparql"))
        .query(&[("query", query)])
        .header("accept-encoding", "gzip")
        .send()
        .await
        .unwrap();
    assert_eq!(compressed.headers()["content-encoding"], "gzip");
    let mut decoded = Vec::new();
    GzDecoder::new(compressed.bytes().await.unwrap().as_ref())
        .read_to_end(&mut decoded)
        .unwrap();
    assert_eq!(decoded, plain.as_ref());
}

#[tokio::test]
async fn sse_is_not_compressed_when_gzip_is_accepted() {
    let base = spawn().await;
    let response = client()
        .get(format!("{base}/subscriptions/sse"))
        .query(&[("query", "SELECT ?s WHERE { ?s ?p ?o }")])
        .header("accept-encoding", "gzip")
        .send()
        .await
        .unwrap();
    assert!(response.status().is_success());
    assert!(response.headers().get("content-encoding").is_none());
    assert!(response.headers()["content-type"]
        .to_str()
        .unwrap()
        .starts_with("text/event-stream"));
}
