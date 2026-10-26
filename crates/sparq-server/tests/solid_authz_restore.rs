//! Stateful authorization caches must follow in-memory restore lineage changes.
#![cfg(all(feature = "solid-authz", feature = "backup"))]

use sparq_core::Graph;
use sparq_server::{router, AppState, ServerConfig};
use tokio::net::TcpListener;

const ALICE: &str = "https://alice.ex/card#me";
const BOB: &str = "https://bob.ex/card#me";
const RESOURCE: &str = "https://pod.ex/n1";

fn wac(agent: &str) -> String {
    format!(
        "<{RESOURCE}> <urn:p> <urn:o> <{RESOURCE}> .\n\
         <https://pod.ex/.acl#r> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://www.w3.org/ns/auth/acl#Authorization> <https://pod.ex/.acl> .\n\
         <https://pod.ex/.acl#r> <http://www.w3.org/ns/auth/acl#default> <https://pod.ex/> <https://pod.ex/.acl> .\n\
         <https://pod.ex/.acl#r> <http://www.w3.org/ns/auth/acl#agent> <{agent}> <https://pod.ex/.acl> .\n\
         <https://pod.ex/.acl#r> <http://www.w3.org/ns/auth/acl#mode> <http://www.w3.org/ns/auth/acl#Read> <https://pod.ex/.acl> .\n"
    )
}

async fn spawn(dataset: &str) -> String {
    let state = AppState::with_config(
        Graph::load_dataset(dataset, "nquads").unwrap(),
        ServerConfig {
            solid_authz: true,
            ..ServerConfig::default()
        },
    );
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, router(state)).await.unwrap() });
    format!("http://{addr}")
}

async fn decide(client: &reqwest::Client, base: &str, agent: &str) -> (u16, serde_json::Value) {
    let response = client
        .post(format!("{base}/authz/decide"))
        .json(&serde_json::json!({
            "source": "server", "session": { "agent": agent },
            "resource": RESOURCE, "mode": "read"
        }))
        .send()
        .await
        .unwrap();
    (response.status().as_u16(), response.json().await.unwrap())
}

async fn restore(client: &reqwest::Client, destination: &str, source: &str) {
    let artifact = client
        .post(format!("{source}/admin/backup"))
        .send()
        .await
        .unwrap();
    assert_eq!(artifact.status().as_u16(), 200);
    let response = client
        .post(format!("{destination}/admin/restore"))
        .body(artifact.bytes().await.unwrap())
        .send()
        .await
        .unwrap();
    assert_eq!(response.status().as_u16(), 200);
}

#[tokio::test]
async fn restored_wac_revokes_a_warm_generation_zero_grant() {
    let client = reqwest::Client::new();
    let destination = spawn(&wac(ALICE)).await;
    let source = spawn(&wac(BOB)).await;
    assert_eq!(decide(&client, &destination, ALICE).await.0, 200);
    restore(&client, &destination, &source).await;
    let actual = decide(&client, &destination, ALICE).await;
    assert_eq!(actual.0, 403);
    assert_eq!(actual.1["allow"], false);
    assert_eq!(actual, decide(&client, &source, ALICE).await);
}

#[cfg(feature = "odrl-authz")]
#[tokio::test]
async fn restored_odrl_refusal_matches_a_fresh_server_after_a_warm_grant() {
    let client = reqwest::Client::new();
    let destination = spawn(&wac(ALICE)).await;
    let source = spawn(&format!(
        "{}<urn:policy> <http://www.w3.org/ns/odrl/2/prohibition> <urn:rule> <urn:policies> .\n",
        wac(ALICE)
    ))
    .await;
    assert_eq!(decide(&client, &destination, ALICE).await.0, 200);
    let expected = decide(&client, &source, ALICE).await;
    assert_eq!(expected.0, 400);
    restore(&client, &destination, &source).await;
    assert_eq!(decide(&client, &destination, ALICE).await, expected);
}

#[tokio::test]
async fn restored_acp_selection_matches_a_fresh_server_after_a_warm_wac_grant() {
    let client = reqwest::Client::new();
    let destination = spawn(&wac(ALICE)).await;
    let acp = "http://www.w3.org/ns/solid/acp#";
    let doc = "https://pod.ex/.acr";
    let dataset = format!(
        "<{RESOURCE}> <urn:p> <urn:o> <{RESOURCE}> .\n\
         <{doc}> <{acp}memberAccessControl> <{doc}#c> <{doc}> .\n\
         <{doc}#c> <{acp}apply> <{doc}#p> <{doc}> .\n\
         <{doc}#p> <{acp}allow> <http://www.w3.org/ns/auth/acl#Read> <{doc}> .\n\
         <{doc}#p> <{acp}anyOf> <{doc}#m> <{doc}> .\n\
         <{doc}#m> <{acp}agent> <{BOB}> <{doc}> .\n"
    );
    let source = spawn(&dataset).await;
    assert_eq!(decide(&client, &destination, ALICE).await.0, 200);
    assert_eq!(decide(&client, &source, BOB).await.0, 200);
    restore(&client, &destination, &source).await;
    for agent in [ALICE, BOB] {
        assert_eq!(
            decide(&client, &destination, agent).await,
            decide(&client, &source, agent).await
        );
    }
}
