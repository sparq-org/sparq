//! Access-controlled queries driven through the production LWS HTTP router.
#![cfg(all(feature = "sparql-endpoint", not(target_arch = "wasm32")))]

use axum::body::Bytes;
use axum::{
    body::{to_bytes, Body},
    http::{HeaderMap, Request, StatusCode},
    Router,
};
use serde_json::{json, Value};
use sparq_lws_core::{
    lws::{self, tokens, LwsConfig},
    store::{CompositeStore, InMemoryBlobStore, InMemorySparqClient, Store},
};
use std::collections::BTreeSet;
use tower::ServiceExt;

const ROOT: &str = "https://pod.example/";
const OWNER: &str = "https://owner.example/";
const A: &str = "https://a.example/";
const B: &str = "https://b.example/";
const CLIENT: &str = "https://client.example/";
const ENDPOINT: &str = "/.lws/sparql";
const UNION: &str = "http://www.w3.org/ns/solid/sparql#union-default-graph";
const GRAPH_NAMES: &str = "SELECT ?g { GRAPH ?g {} }";

struct Fixture {
    app: Router,
    cfg: LwsConfig,
    owner: String,
    a: String,
    b: String,
}
impl Fixture {
    async fn new(configure: impl FnOnce(&mut LwsConfig)) -> Self {
        let mut cfg = LwsConfig::new(ROOT);
        cfg.owner = Some(OWNER.into());
        configure(&mut cfg);
        let token = |who| format!("Bearer {}", tokens::mint(&cfg, who, CLIENT, ROOT));
        let (owner, a, b) = (token(OWNER), token(A), token(B));
        let store = CompositeStore::new(InMemorySparqClient::new(), InMemoryBlobStore::new());
        store
            .write(ROOT, Bytes::new(), lws::LWS_JSON)
            .await
            .unwrap();
        // Backend enumeration can contain internal records. The query must filter them itself.
        let internal = format!("{ROOT}.lws/");
        store
            .create_in_container(ROOT, &internal, Bytes::new(), lws::LWS_JSON)
            .await
            .unwrap();
        store
            .create_in_container(
                &internal,
                &format!("{internal}record"),
                Bytes::from_static(b"<urn:private> <urn:p> 1 ."),
                "text/turtle",
            )
            .await
            .unwrap();
        store
            .create_in_container(
                ROOT,
                &format!("{ROOT}ghost.meta"),
                Bytes::from_static(b"<urn:metadata> <urn:p> 1 ."),
                "text/turtle",
            )
            .await
            .unwrap();
        let app = lws::router(store, cfg.clone()).await.unwrap();
        Self {
            app,
            cfg,
            owner,
            a,
            b,
        }
    }
    async fn create(&self, parent: &str, slug: &str, media: &str, body: &str) -> String {
        let (status, headers, response) = call(
            &self.app,
            "POST",
            parent,
            &self.owner,
            &[("slug", slug), ("content-type", media)],
            body,
        )
        .await;
        assert_eq!(
            status,
            StatusCode::CREATED,
            "{}",
            String::from_utf8_lossy(&response)
        );
        headers["location"].to_str().unwrap().into()
    }
    async fn grant(&self, agent: &str, targets: &[&str], constraints: Value) -> String {
        let body = json!({"@context": lws::LWS_CONTEXT, "type": "AccessGrant", "storage": ROOT,
            "access": [{"type": "AccessPolicy", "action": "read", "assignee": agent,
            "target": {"type":"StorageResource", "value":targets}, "constraint": constraints}]});
        let (status, headers, response) = call(
            &self.app,
            "POST",
            lws::GRANTS_PATH,
            &self.owner,
            &[("content-type", lws::LWS_JSON)],
            &body.to_string(),
        )
        .await;
        assert_eq!(
            status,
            StatusCode::CREATED,
            "{}",
            String::from_utf8_lossy(&response)
        );
        headers["location"].to_str().unwrap().into()
    }
    async fn query(&self, token: &str, query: &str) -> Value {
        let (status, _, body) = call(
            &self.app,
            "POST",
            ENDPOINT,
            token,
            &[("content-type", "application/sparql-query")],
            query,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
        serde_json::from_slice(&body).unwrap()
    }
    async fn names(&self, token: &str) -> BTreeSet<String> {
        self.query(token, GRAPH_NAMES).await["results"]["bindings"]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| row["g"]["value"].as_str().unwrap().into())
            .collect()
    }
}
async fn call(
    app: &Router,
    method: &str,
    path: &str,
    token: &str,
    headers: &[(&str, &str)],
    body: &str,
) -> (StatusCode, HeaderMap, Vec<u8>) {
    let mut req = Request::builder().method(method).uri(path);
    if !token.is_empty() {
        req = req.header("authorization", token);
    }
    for (key, value) in headers {
        req = req.header(*key, *value);
    }
    let response = app
        .clone()
        .oneshot(req.body(Body::from(body.to_owned())).unwrap())
        .await
        .unwrap();
    let (parts, body) = response.into_parts();
    (
        parts.status,
        parts.headers,
        to_bytes(body, 1024 * 1024).await.unwrap().to_vec(),
    )
}
fn names(expected: &[&str]) -> BTreeSet<String> {
    expected.iter().map(|v| (*v).into()).collect()
}
fn rows(value: &Value) -> usize {
    value["results"]["bindings"].as_array().unwrap().len()
}
fn assert_problem(status: StatusCode, headers: HeaderMap, body: Vec<u8>, expected: StatusCode) {
    assert_eq!(status, expected, "{}", String::from_utf8_lossy(&body));
    assert_eq!(headers["content-type"], "application/problem+json");
    let value: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(value["status"], expected.as_u16());
    assert!(!String::from_utf8_lossy(&body).contains(ROOT));
}

#[tokio::test]
async fn sq_4b_live_visibility_and_empty_graphs() {
    let f = Fixture::new(|_| {}).await;
    let r1 = f
        .create("/", "r1", "text/turtle", "<urn:a> <urn:p> 1 .")
        .await;
    let r2 = f
        .create(
            "/",
            "r2",
            "application/ld+json",
            r#"{"@id":"urn:b","urn:p":{"@value":2}}"#,
        )
        .await;
    let r3 = f.create("/", "r3", "image/png", "PNG").await;
    let container = f
        .create("/", "folder", lws::LWS_JSON, r#"{"type":"Container"}"#)
        .await;
    let malformed = f.create("/", "broken", "text/plain", "broken RDF").await;
    // SQ-4b's overlapping grants include both RDF formats and a non-RDF empty graph.
    let grant = f.grant(A, &[&r1, &r2], json!([])).await;
    f.grant(B, &[&r2, &r3], json!([])).await;
    assert_eq!(f.names(&f.a).await, names(&[&r1, &r2]));
    assert_eq!(f.names(&f.b).await, names(&[&r2, &r3]));
    assert_eq!(
        f.names(&f.owner).await,
        names(&[ROOT, &r1, &r2, &r3, &container, &malformed])
    );
    assert!(f.names("").await.is_empty());
    // Containers are visible by their own grant, without granting the root.
    f.grant(A, &[&container], json!([])).await;
    assert_eq!(f.names(&f.a).await, names(&[&r1, &r2, &container]));
    assert_eq!(rows(&f.query(&f.a, "SELECT * { ?s ?p ?o }").await), 0);
    let union = format!("SELECT ?s FROM <{UNION}> {{ ?s ?p ?o }}");
    let values = f.query(&f.a, &union).await;
    assert_eq!(rows(&values), 2);
    assert_eq!(
        values["results"]["bindings"]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| row["s"]["value"].as_str().unwrap())
            .collect::<BTreeSet<_>>(),
        BTreeSet::from(["urn:a", "urn:b"])
    );
    let (status, _, _) = call(&f.app, "DELETE", &grant, &f.owner, &[], "").await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert_eq!(f.names(&f.a).await, names(&[&container]));
    assert_eq!(
        f.query(&f.a, &format!("ASK FROM <{UNION}> {{ ?s ?p ?o }}"))
            .await["boolean"],
        false
    );
}

#[tokio::test]
async fn grant_constraints_are_evaluated_for_each_request() {
    let f = Fixture::new(|_| {}).await;
    let resource = f
        .create("/", "r", "text/turtle", "<urn:s> <urn:p> 1 .")
        .await;
    f.grant(
        A,
        &[&resource],
        json!([{"leftOperand":"dateTime","operator":"lt","rightOperand":"2000-01-01T00:00:00Z"}]),
    )
    .await;
    assert!(f.names(&f.a).await.is_empty());
    f.grant(A, &[&resource], json!([{"leftOperand":"client","operator":"eq","rightOperand": CLIENT}, {"leftOperand":"format","operator":"eq","rightOperand":"text/turtle"}, {"leftOperand":"type","operator":"eq","rightOperand":"DataResource"}])).await;
    assert_eq!(f.names(&f.a).await, names(&[&resource]));
    let other = format!(
        "Bearer {}",
        tokens::mint(&f.cfg, A, "https://other-client.example/", ROOT)
    );
    assert!(f.names(&other).await.is_empty());
    f.grant(
        B,
        &[&resource],
        json!([{"leftOperand":"purpose","operator":"eq","rightOperand":"research"}]),
    )
    .await;
    assert!(f.names(&f.b).await.is_empty());
}

#[tokio::test]
async fn query_bindings_dataset_selection_and_negotiation() {
    let f = Fixture::new(|_| {}).await;
    let resource = f
        .create("/", "r", "text/turtle", "<urn:s> <urn:p> 1 .")
        .await;
    f.grant(A, &[&resource], json!([])).await;
    let params = form_urlencoded::Serializer::new(String::new())
        .append_pair("query", "SELECT * { ?s ?p ?o }")
        .append_pair("default-graph-uri", &resource)
        .finish();
    for method in ["GET", "POST"] {
        let path = if method == "GET" {
            format!("{ENDPOINT}?{params}")
        } else {
            ENDPOINT.into()
        };
        let (status, _, body) = call(
            &f.app,
            method,
            &path,
            &f.a,
            &[("content-type", "application/x-www-form-urlencoded")],
            if method == "POST" { &params } else { "" },
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(rows(&serde_json::from_slice(&body).unwrap()), 1);
    }
    let hidden = f
        .query(
            &f.a,
            "SELECT ?g FROM NAMED <https://pod.example/hidden> { GRAPH ?g { ?s ?p ?o } }",
        )
        .await;
    assert_eq!(rows(&hidden), 0);
    let params = form_urlencoded::Serializer::new(String::new())
        .append_pair("query", "SELECT ?g { GRAPH ?g {} }")
        .append_pair("named-graph-uri", &resource)
        .finish();
    let (status, _, body) = call(
        &f.app,
        "POST",
        ENDPOINT,
        &f.a,
        &[("content-type", "application/x-www-form-urlencoded")],
        &params,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(rows(&serde_json::from_slice(&body).unwrap()), 1);
    let query = format!("CONSTRUCT {{ ?s ?p ?o }} FROM <{UNION}> {{ ?s ?p ?o }}");
    let (status, headers, body) = call(
        &f.app,
        "POST",
        ENDPOINT,
        &f.a,
        &[
            ("content-type", "application/sparql-query"),
            ("accept", "application/n-triples"),
        ],
        &query,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers["content-type"], "application/n-triples");
    assert!(String::from_utf8(body).unwrap().contains("<urn:s> <urn:p>"));
    let (status, headers, body) = call(
        &f.app,
        "POST",
        ENDPOINT,
        &f.a,
        &[
            ("content-type", "application/sparql-query"),
            ("accept", "text/html"),
        ],
        "ASK {}",
    )
    .await;
    assert_problem(status, headers, body, StatusCode::NOT_ACCEPTABLE);
}

#[tokio::test]
async fn caps_refuse_whole_queries_and_ignore_unreadable_bytes() {
    let f = Fixture::new(|cfg| cfg.sparql_max_resources = 1).await;
    let r1 = f
        .create("/", "r1", "text/turtle", "<urn:s> <urn:p> 1 .")
        .await;
    let r2 = f
        .create("/", "r2", "text/turtle", "<urn:s> <urn:p> 2 .")
        .await;
    f.grant(A, &[&r1], json!([])).await;
    assert_eq!(f.names(&f.a).await, names(&[&r1]));
    f.grant(A, &[&r2], json!([])).await;
    let (status, headers, body) = call(
        &f.app,
        "POST",
        ENDPOINT,
        &f.a,
        &[("content-type", "application/sparql-query")],
        GRAPH_NAMES,
    )
    .await;
    assert_problem(status, headers, body, StatusCode::INSUFFICIENT_STORAGE);
    let f = Fixture::new(|cfg| cfg.sparql_max_bytes = 22).await;
    let r1 = f
        .create("/", "r1", "text/turtle", "<urn:s> <urn:p> 1 .")
        .await;
    let r2 = f
        .create(
            "/",
            "r2",
            "text/turtle",
            "<urn:secret> <urn:p> \"this is deliberately too large\" .",
        )
        .await;
    f.grant(A, &[&r1], json!([])).await;
    assert_eq!(f.names(&f.a).await, names(&[&r1]));
    f.grant(A, &[&r2], json!([])).await;
    let (status, headers, body) = call(
        &f.app,
        "POST",
        ENDPOINT,
        &f.a,
        &[("content-type", "application/sparql-query")],
        GRAPH_NAMES,
    )
    .await;
    assert_problem(status, headers, body, StatusCode::PAYLOAD_TOO_LARGE);
}

#[tokio::test]
async fn updates_are_refused_and_catalog_matches_mount() {
    let f = Fixture::new(|_| {}).await;
    for (method, path, media, body) in [
        (
            "POST",
            ENDPOINT,
            "application/sparql-update",
            "INSERT DATA { <urn:s> <urn:p> 1 }",
        ),
        (
            "POST",
            ENDPOINT,
            "application/sparql-query",
            "INSERT DATA { <urn:s> <urn:p> 1 }",
        ),
        (
            "POST",
            ENDPOINT,
            "application/x-www-form-urlencoded",
            "update=CLEAR+ALL",
        ),
        ("GET", "/.lws/sparql?update=CLEAR+ALL", "", ""),
        ("PUT", ENDPOINT, "application/sparql-query", "ASK {}"),
    ] {
        let (status, headers, body) = call(
            &f.app,
            method,
            path,
            &f.owner,
            &[("content-type", media)],
            body,
        )
        .await;
        assert_eq!(headers["allow"], "GET, POST");
        assert_problem(status, headers, body, StatusCode::METHOD_NOT_ALLOWED);
    }
    let (status, _, body) = call(&f.app, "GET", "/", "", &[("accept", lws::LWS_CID)], "").await;
    assert_eq!(status, StatusCode::OK);
    let doc: Value = serde_json::from_slice(&body).unwrap();
    assert!(doc["service"].as_array().unwrap().iter().any(|s| s["type"]
        == "https://jeswr.org/ns/solid-sd#SparqlQueryService"
        && s["serviceEndpoint"] == format!("{ROOT}.lws/sparql")));
    // /sparql remains an ordinary client-allocated resource in LWS mode.
    let ordinary = f.create("/", "sparql", "text/plain", "ordinary").await;
    assert_eq!(ordinary, format!("{ROOT}sparql"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn concurrent_writer_queries_never_combine_document_generations() {
    let f = Fixture::new(|_| {}).await;
    let r = f
        .create("/", "r", "text/turtle", "<urn:s> <urn:x> 0 ; <urn:y> 0 .")
        .await;
    f.grant(A, &[&r], json!([])).await;
    let app = f.app.clone();
    let token = f.owner.clone();
    let writer = tokio::spawn(async move {
        for n in 1..=20 {
            let doc = format!("<urn:s> <urn:x> {n} ; <urn:y> {n} .");
            let (status, _, body) = call(
                &app,
                "PUT",
                "/r",
                &token,
                &[("content-type", "text/turtle")],
                &doc,
            )
            .await;
            assert!(status.is_success(), "{}", String::from_utf8_lossy(&body));
        }
    });
    for _ in 0..20 {
        let value = f
            .query(
                &f.a,
                "SELECT ?x ?y { GRAPH ?g { ?s <urn:x> ?x ; <urn:y> ?y } }",
            )
            .await;
        let rows = value["results"]["bindings"].as_array().unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["x"]["value"], rows[0]["y"]["value"]);
    }
    writer.await.unwrap();
}
