//! LWS query behavior through the authenticated HTTP router.
#![cfg(feature = "sparql-endpoint")]

use axum::body::{to_bytes, Body, Bytes};
use axum::http::{header, Request, StatusCode};
use axum::response::Response;
use serde_json::{json, Value};
use sparq_lws_core::lws::{self, tokens, LwsConfig, LWS_CONTEXT};
use sparq_lws_core::store::{
    BlobEntry, BlobError, BlobStore, BodyCache, CompositeStore, InMemoryBlobStore,
    InMemorySparqClient, Store,
};
use std::collections::BTreeSet;
use tower::ServiceExt;

const BASE: &str = "https://lws.example";
const OWNER: &str = "https://id.example/owner";
const A: &str = "https://id.example/a";
const B: &str = "https://id.example/b";
const CLIENT: &str = "https://app.example/one";
const UNION: &str = "http://www.w3.org/ns/solid/sparql#union-default-graph";
const PATH: &str = "/.lws/sparql";

struct Harness {
    app: axum::Router,
    cfg: LwsConfig,
}

impl Harness {
    async fn new(configure: impl FnOnce(&mut LwsConfig)) -> Self {
        Self::with_blobs(configure, InMemoryBlobStore::new()).await
    }

    async fn with_blobs(
        blobs_configure: impl FnOnce(&mut LwsConfig),
        blobs: impl BlobStore + 'static,
    ) -> Self {
        let mut cfg = LwsConfig::new(BASE);
        cfg.owner = Some(OWNER.into());
        blobs_configure(&mut cfg);
        let store =
            CompositeStore::with_body_cache(InMemorySparqClient::new(), blobs, BodyCache::new(0));
        // A malformed RDF representation can exist in storage independently of HTTP validation.
        store
            .write(&format!("{BASE}/"), Bytes::new(), "application/lws+json")
            .await
            .unwrap();
        store
            .create_in_container(
                &format!("{BASE}/"),
                &format!("{BASE}/broken"),
                Bytes::from_static(b"malformed Turtle"),
                "text/turtle",
            )
            .await
            .unwrap();
        // Stored service records must never be enumerated, even by the owner.
        store
            .create_in_container(
                &format!("{BASE}/"),
                &format!("{BASE}/.lws/"),
                Bytes::new(),
                "application/lws+json",
            )
            .await
            .unwrap();
        store
            .create_in_container(
                &format!("{BASE}/.lws/"),
                &format!("{BASE}/.lws/record"),
                Bytes::from_static(b"<urn:service> <urn:p> <urn:record> ."),
                "text/turtle",
            )
            .await
            .unwrap();
        let app = lws::router(store, cfg.clone()).await.unwrap();
        Self { app, cfg }
    }

    async fn call(
        &self,
        who: &str,
        client: &str,
        method: &str,
        path: &str,
        headers: &[(&str, &str)],
        body: &str,
    ) -> Response {
        let token = tokens::mint(&self.cfg, who, client, &self.cfg.realm());
        let mut req = Request::builder()
            .method(method)
            .uri(path)
            .header(header::AUTHORIZATION, format!("Bearer {token}"));
        for (name, value) in headers {
            req = req.header(*name, *value);
        }
        self.app
            .clone()
            .oneshot(req.body(Body::from(body.to_owned())).unwrap())
            .await
            .unwrap()
    }

    async fn query_as(&self, who: &str, client: &str, text: &str) -> Response {
        self.call(
            who,
            client,
            "POST",
            PATH,
            &[("content-type", "application/sparql-query")],
            text,
        )
        .await
    }

    async fn query(&self, who: &str, text: &str) -> Value {
        let response = self.query_as(who, CLIENT, text).await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.headers()[header::CONTENT_TYPE],
            "application/sparql-results+json"
        );
        body_json(response).await
    }

    async fn graphs(&self, who: &str) -> BTreeSet<String> {
        values(
            &self.query(who, "SELECT ?g WHERE { GRAPH ?g {} }").await,
            "g",
        )
    }

    async fn create(&self, slug: &str, content_type: &str, body: &str) -> String {
        let response = self
            .call(
                OWNER,
                CLIENT,
                "POST",
                "/",
                &[
                    ("slug", slug.trim_end_matches('/')),
                    ("content-type", content_type),
                    (
                        "link",
                        if slug.ends_with('/') {
                            "<https://www.w3.org/ns/lws#Container>; rel=\"type\""
                        } else {
                            ""
                        },
                    ),
                ],
                body,
            )
            .await;
        assert_eq!(
            response.status(),
            StatusCode::CREATED,
            "{}",
            String::from_utf8_lossy(&to_bytes(response.into_body(), usize::MAX).await.unwrap())
        );
        response.headers()[header::LOCATION]
            .to_str()
            .unwrap()
            .into()
    }

    async fn grant(&self, who: &str, paths: &[&str], constraints: Value) -> String {
        let doc = json!({
            "@context": LWS_CONTEXT, "type": "AccessGrant", "storage": format!("{BASE}/"),
            "access": [{ "type": "AccessPolicy", "assignee": who, "action": ["read"],
                "target": {"type": "StorageResource", "value": paths.iter().map(|p| format!("{BASE}{p}")).collect::<Vec<_>>()},
                "constraint": constraints }]
        });
        let response = self
            .call(
                OWNER,
                CLIENT,
                "POST",
                "/.lws/grants/",
                &[("content-type", "application/ld+json")],
                &doc.to_string(),
            )
            .await;
        assert_eq!(
            response.status(),
            StatusCode::CREATED,
            "{}",
            String::from_utf8_lossy(&to_bytes(response.into_body(), usize::MAX).await.unwrap())
        );
        response.headers()[header::LOCATION]
            .to_str()
            .unwrap()
            .into()
    }

    async fn seed(&self) {
        self.create("r1", "text/turtle", "<urn:r1> <urn:p> \"one\" .")
            .await;
        self.create(
            "r2",
            "application/ld+json",
            r#"{"@id":"urn:r2","urn:p":{"@value":"two"}}"#,
        )
        .await;
        self.create("r3", "image/png", "PNG").await;
        self.create("c/", "application/lws+json", "").await;
        self.grant(A, &["/r1", "/r2"], json!([])).await;
        self.grant(B, &["/r2", "/r3"], json!([])).await;
    }
}

async fn body_json(response: Response) -> Value {
    serde_json::from_slice(&to_bytes(response.into_body(), usize::MAX).await.unwrap()).unwrap()
}

fn values(result: &Value, name: &str) -> BTreeSet<String> {
    result["results"]["bindings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row[name]["value"].as_str().unwrap().to_string())
        .collect()
}

fn iris(paths: &[&str]) -> BTreeSet<String> {
    paths.iter().map(|p| format!("{BASE}{p}")).collect()
}

#[tokio::test]
async fn each_agent_gets_exactly_its_readable_named_graphs_including_empty_graphs() {
    let h = Harness::new(|_| {}).await;
    h.seed().await;
    assert_eq!(h.graphs(A).await, iris(&["/r1", "/r2"]));
    assert_eq!(h.graphs(B).await, iris(&["/r2", "/r3"]));
    assert_eq!(
        h.graphs(OWNER).await,
        iris(&["/", "/r1", "/r2", "/r3", "/c/", "/broken"])
    );
    // Malformed RDF remains an empty named graph when it is contained and readable.
    h.grant(A, &["/c/", "/broken"], json!([])).await;
    assert_eq!(h.graphs(A).await, iris(&["/r1", "/r2", "/c/", "/broken"]));
}

#[tokio::test]
async fn default_union_protocol_datasets_and_serialization_use_the_visible_view() {
    let h = Harness::new(|_| {}).await;
    h.seed().await;
    h.create("other", "text/turtle", "<urn:other> <urn:p> \"other\" .")
        .await;
    assert_eq!(h.query(A, "ASK { ?s ?p ?o }").await["boolean"], false);
    let union = format!("SELECT ?o FROM <{UNION}> WHERE {{ ?s <urn:p> ?o }}");
    assert_eq!(
        values(&h.query(A, &union).await, "o"),
        BTreeSet::from(["one".into(), "two".into()])
    );
    assert_eq!(
        h.query(A, "SELECT (COUNT(*) AS ?n) WHERE { GRAPH ?g { ?s ?p ?o } }")
            .await["results"]["bindings"][0]["n"]["value"],
        "2"
    );
    let form = form_urlencoded::Serializer::new(String::new())
        .append_pair("query", "SELECT ?g { GRAPH ?g {} }")
        .append_pair("named-graph-uri", &format!("{BASE}/r2"))
        .append_pair("named-graph-uri", &format!("{BASE}/other"))
        .finish();
    let response = h
        .call(
            A,
            CLIENT,
            "POST",
            PATH,
            &[("content-type", "application/x-www-form-urlencoded")],
            &form,
        )
        .await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(values(&body_json(response).await, "g"), iris(&["/r2"]));
    let params = form_urlencoded::Serializer::new(String::new())
        .append_pair("query", "ASK { ?s ?p ?o }")
        .append_pair("default-graph-uri", &format!("{BASE}/r2"))
        .finish();
    let response = h
        .call(A, CLIENT, "GET", &format!("{PATH}?{params}"), &[], "")
        .await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(body_json(response).await["boolean"], true);
    for text in [
        format!("CONSTRUCT {{ ?s ?p ?o }} FROM <{UNION}> WHERE {{ ?s ?p ?o }}"),
        format!("DESCRIBE <urn:r1> FROM <{UNION}>"),
    ] {
        let response = h.query_as(A, CLIENT, &text).await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.headers()[header::CONTENT_TYPE],
            "application/n-triples"
        );
        let body = String::from_utf8(
            to_bytes(response.into_body(), usize::MAX)
                .await
                .unwrap()
                .to_vec(),
        )
        .unwrap();
        assert!(body.contains("\"one\""));
        assert!(!body.contains("other"));
    }
    let response = h
        .call(
            A,
            CLIENT,
            "POST",
            PATH,
            &[
                ("content-type", "application/sparql-query"),
                ("accept", "text/html"),
            ],
            "ASK {}",
        )
        .await;
    assert_eq!(response.status(), StatusCode::NOT_ACCEPTABLE);
}

#[tokio::test]
async fn document_blank_nodes_stay_distinct_including_nested_triple_terms() {
    let h = Harness::new(|_| {}).await;
    for name in ["left", "right"] {
        h.create(
            name,
            "text/turtle",
            &format!("_:same <urn:p> \"{name}\"; <urn:term> <<( _:same <urn:q> _:same )>> ."),
        )
        .await;
    }
    let result = h
        .query(
            OWNER,
            &format!(
                "SELECT ?left ?right {{
                    GRAPH <{BASE}/left> {{ ?left <urn:p> \"left\" }}
                    GRAPH <{BASE}/right> {{ ?right <urn:p> \"right\" }}
                }}"
            ),
        )
        .await;
    let rows = result["results"]["bindings"].as_array().unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["left"]["type"], "bnode");
    assert_eq!(rows[0]["right"]["type"], "bnode");
    assert_ne!(rows[0]["left"]["value"], rows[0]["right"]["value"]);
    assert_eq!(
        h.query(
            OWNER,
            &format!(
                "ASK {{
                    GRAPH <{BASE}/left> {{ ?s <urn:term> ?term }}
                    GRAPH <{BASE}/right> {{ ?s2 <urn:term> ?term }}
                }}"
            )
        )
        .await["boolean"],
        false
    );
}

#[tokio::test]
async fn grants_are_live_and_apply_client_date_format_type_and_purpose_constraints() {
    let h = Harness::new(|_| {}).await;
    h.create("r1", "text/turtle", "<> a <urn:Document> .").await;
    let constraint =
        |left, right| json!({"leftOperand":left, "operator":"eq", "rightOperand":right});
    h.grant(
        A,
        &["/r1"],
        json!([{"leftOperand":"dateTime","operator":"lt","rightOperand":"2000-01-01T00:00:00Z"}]),
    )
    .await;
    h.grant(A, &["/r1"], json!([constraint("purpose", "urn:research")]))
        .await;
    assert!(h.graphs(A).await.is_empty());
    let grant = h
        .grant(
            A,
            &["/r1"],
            json!([
                constraint("client", CLIENT),
                constraint("format", "text/turtle"),
                constraint("type", "urn:Document")
            ]),
        )
        .await;
    assert_eq!(h.graphs(A).await, iris(&["/r1"]));
    let other = h
        .query_as(A, "https://app.example/two", "SELECT ?g { GRAPH ?g {} }")
        .await;
    assert!(values(&body_json(other).await, "g").is_empty());
    let revoked = h
        .call(
            OWNER,
            CLIENT,
            "DELETE",
            grant.strip_prefix(BASE).unwrap(),
            &[],
            "",
        )
        .await;
    assert!(revoked.status().is_success());
    assert!(h.graphs(A).await.is_empty());
}

#[tokio::test]
async fn caps_are_problem_responses_and_count_only_the_visible_view() {
    let h = Harness::new(|c| c.sparql_max_resources = 1).await;
    h.create("r1", "text/turtle", "<urn:r1> <urn:p> \"one\" .")
        .await;
    h.grant(A, &["/r1"], json!([])).await;
    h.create(
        "unreadable",
        "text/turtle",
        "<urn:other> <urn:p> \"other\" .",
    )
    .await;
    assert_eq!(h.graphs(A).await, iris(&["/r1"]));
    let response = h.query_as(OWNER, CLIENT, "SELECT ?g { GRAPH ?g {} }").await;
    assert_eq!(response.status(), StatusCode::INSUFFICIENT_STORAGE);
    assert_eq!(
        response.headers()[header::CONTENT_TYPE],
        "application/problem+json"
    );
    assert_eq!(body_json(response).await["status"], 507);
    let h = Harness::new(|c| c.sparql_max_bytes = 1).await;
    h.create("r1", "text/turtle", "<urn:r1> <urn:p> \"one\" .")
        .await;
    assert!(h.graphs(A).await.is_empty());
    let response = h.query_as(OWNER, CLIENT, "ASK {}").await;
    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(
        response.headers()[header::CONTENT_TYPE],
        "application/problem+json"
    );
    assert_eq!(body_json(response).await["status"], 413);
}

#[tokio::test]
async fn discovery_update_refusal_and_the_root_sparql_resource_are_independent() {
    let h = Harness::new(|_| {}).await;
    h.create(
        "sparql",
        "text/turtle",
        "<urn:normal> <urn:p> \"resource\" .",
    )
    .await;
    let response = h.call(OWNER, CLIENT, "GET", "/sparql", &[], "").await;
    assert_eq!(response.status(), StatusCode::OK);
    let response = h
        .call(
            A,
            CLIENT,
            "GET",
            "/",
            &[("accept", "application/lws+cid")],
            "",
        )
        .await;
    assert_eq!(response.status(), StatusCode::OK);
    let doc = body_json(response).await;
    let services = doc["service"].as_array().unwrap();
    assert!(services.iter().any(|s| s["type"]
        == "https://jeswr.org/ns/solid-sd#SparqlQueryService"
        && s["serviceEndpoint"] == format!("{BASE}{PATH}")));
    for (method, path, ct, body) in [
        ("PUT", PATH.to_owned(), "text/plain", ""),
        (
            "POST",
            PATH.to_owned(),
            "application/sparql-update",
            "INSERT DATA {}",
        ),
        (
            "POST",
            PATH.to_owned(),
            "application/sparql-query",
            "INSERT DATA {}",
        ),
        (
            "POST",
            PATH.to_owned(),
            "application/x-www-form-urlencoded",
            "update=INSERT+DATA+%7B%7D",
        ),
        (
            "GET",
            format!("{PATH}?update=INSERT+DATA+%7B%7D"),
            "text/plain",
            "",
        ),
    ] {
        let response = h
            .call(A, CLIENT, method, &path, &[("content-type", ct)], body)
            .await;
        assert_eq!(response.status(), StatusCode::METHOD_NOT_ALLOWED);
        assert_eq!(response.headers()[header::ALLOW], "GET, POST");
        assert_eq!(body_json(response).await["status"], 405);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn concurrent_writes_never_mix_resource_generations_or_drop_the_graph() {
    let h = std::sync::Arc::new(Harness::new(|_| {}).await);
    h.create(
        "r1",
        "text/turtle",
        "<> a <urn:Even>; <urn:x> 0; <urn:y> 0 .",
    )
    .await;
    let writer = {
        let h = h.clone();
        tokio::spawn(async move {
            for n in 1..40 {
                let body = format!("<> a <urn:Type{n}>; <urn:x> {n}; <urn:y> {n} .");
                let response = h
                    .call(
                        OWNER,
                        CLIENT,
                        "PUT",
                        "/r1",
                        &[("content-type", "text/turtle")],
                        &body,
                    )
                    .await;
                assert!(response.status().is_success());
                tokio::task::yield_now().await;
            }
        })
    };
    for _ in 0..40 {
        let result = h.query(OWNER, "SELECT ?t ?x ?y { GRAPH <https://lws.example/r1> { ?s a ?t; <urn:x> ?x; <urn:y> ?y } }").await;
        let rows = result["results"]["bindings"].as_array().unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["x"]["value"], rows[0]["y"]["value"]);
        let n = rows[0]["x"]["value"].as_str().unwrap();
        assert_eq!(
            rows[0]["t"]["value"],
            if n == "0" {
                "urn:Even".into()
            } else {
                format!("urn:Type{n}")
            }
        );
    }
    writer.await.unwrap();
}

#[tokio::test]
async fn query_deadline_applies_to_assembly_and_returns_a_problem() {
    let h = Harness::new(|c| c.sparql_timeout = Some(std::time::Duration::ZERO)).await;
    let response = h.query_as(OWNER, CLIENT, "ASK {}").await;
    assert_eq!(response.status(), StatusCode::GATEWAY_TIMEOUT);
    assert_eq!(body_json(response).await["status"], 504);
}

#[derive(Clone, Default)]
struct ControlledBlobs {
    inner: std::sync::Arc<InMemoryBlobStore>,
    fail_reads: std::sync::Arc<std::sync::atomic::AtomicBool>,
    pause_next_read: std::sync::Arc<std::sync::atomic::AtomicBool>,
    entered: std::sync::Arc<tokio::sync::Notify>,
    release: std::sync::Arc<tokio::sync::Notify>,
}

#[async_trait::async_trait]
impl BlobStore for ControlledBlobs {
    async fn get(&self, key: &str) -> Result<Bytes, BlobError> {
        let body = self.inner.get(key).await?;
        if body.starts_with(b"<urn:read-paused>")
            && self
                .pause_next_read
                .swap(false, std::sync::atomic::Ordering::SeqCst)
        {
            self.entered.notify_one();
            self.release.notified().await;
        }
        if self.fail_reads.load(std::sync::atomic::Ordering::SeqCst)
            && body.starts_with(b"<urn:fault>")
        {
            return Err(BlobError::Backend("opaque read diagnostic".into()));
        }
        Ok(body)
    }
    async fn put(&self, key: &str, body: Bytes) -> Result<(), BlobError> {
        if body.starts_with(b"<urn:paused>") {
            self.entered.notify_one();
            self.release.notified().await;
        }
        self.inner.put(key, body).await
    }
    async fn exists(&self, key: &str) -> Result<bool, BlobError> {
        self.inner.exists(key).await
    }
    async fn delete(&self, key: &str) -> Result<(), BlobError> {
        self.inner.delete(key).await
    }
    async fn delete_if_unchanged(&self, key: &str, generation: u64) -> Result<bool, BlobError> {
        self.inner.delete_if_unchanged(key, generation).await
    }
    async fn list(&self) -> Result<Vec<BlobEntry>, BlobError> {
        self.inner.list().await
    }
}

#[tokio::test]
async fn a_failed_authorized_body_read_omits_the_resource_without_diagnostics() {
    let blobs = ControlledBlobs::default();
    let h = Harness::with_blobs(|_| {}, blobs.clone()).await;
    h.create("r1", "text/turtle", "<urn:fault> <urn:p> 1 .")
        .await;
    h.grant(A, &["/r1"], json!([])).await;
    assert_eq!(h.graphs(A).await, iris(&["/r1"]));
    blobs
        .fail_reads
        .store(true, std::sync::atomic::Ordering::SeqCst);
    let result = h.query(A, "SELECT ?g { GRAPH ?g {} }").await;
    assert!(values(&result, "g").is_empty());
    assert!(!result.to_string().contains("diagnostic"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn snapshot_waiting_obeys_the_query_deadline_and_recovers_after_the_writer() {
    let blobs = ControlledBlobs::default();
    let h = std::sync::Arc::new(
        Harness::with_blobs(
            |c| c.sparql_timeout = Some(std::time::Duration::from_millis(50)),
            blobs.clone(),
        )
        .await,
    );
    h.create("r1", "text/turtle", "<> a <urn:Old>; <urn:p> 1 .")
        .await;
    let writer = {
        let h = h.clone();
        tokio::spawn(async move {
            h.call(
                OWNER,
                CLIENT,
                "PUT",
                "/r1",
                &[("content-type", "text/turtle")],
                "<urn:paused> a <urn:New>; <urn:p> 2 .",
            )
            .await
        })
    };
    tokio::time::timeout(std::time::Duration::from_secs(5), blobs.entered.notified())
        .await
        .unwrap();
    let response = h.query_as(OWNER, CLIENT, "SELECT ?g { GRAPH ?g {} }").await;
    assert_eq!(response.status(), StatusCode::GATEWAY_TIMEOUT);
    assert_eq!(body_json(response).await["status"], 504);
    blobs.release.notify_one();
    assert!(writer.await.unwrap().status().is_success());
    assert_eq!(
        values(
            &h.query(
                OWNER,
                "SELECT ?t { GRAPH <https://lws.example/r1> { ?s a ?t } }"
            )
            .await,
            "t"
        ),
        BTreeSet::from(["urn:New".to_owned()])
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_change_during_assembly_discards_the_attempt_and_rebuilds_the_view() {
    let blobs = ControlledBlobs::default();
    let h = std::sync::Arc::new(Harness::with_blobs(|_| {}, blobs.clone()).await);
    h.create("r1", "text/turtle", "<urn:read-paused> <urn:p> 1 .")
        .await;
    blobs
        .pause_next_read
        .store(true, std::sync::atomic::Ordering::SeqCst);
    let query = {
        let h = h.clone();
        tokio::spawn(async move {
            h.query(
                OWNER,
                "SELECT ?o { GRAPH <https://lws.example/r1> { ?s <urn:p> ?o } }",
            )
            .await
        })
    };
    tokio::time::timeout(std::time::Duration::from_secs(5), blobs.entered.notified())
        .await
        .unwrap();
    // The query holds the old body. A mutation to that resource completes while its
    // generation guard is held, then the query resumes and must discard that old attempt.
    let written = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        h.call(
            OWNER,
            CLIENT,
            "PUT",
            "/r1",
            &[("content-type", "text/turtle")],
            "<urn:new> <urn:p> 2 .",
        ),
    )
    .await
    .unwrap();
    assert!(written.status().is_success());
    blobs.release.notify_one();
    let result = query.await.unwrap();
    assert_eq!(values(&result, "o"), BTreeSet::from(["2".to_owned()]));
}

#[tokio::test]
async fn malformed_rdf_has_an_empty_graph_and_anonymous_queries_use_their_view() {
    let h = Harness::new(|_| {}).await;
    assert_eq!(h.graphs(OWNER).await, iris(&["/", "/broken"]));
    let result = h.query(OWNER, "SELECT ?g { GRAPH ?g { ?s ?p ?o } }").await;
    assert!(values(&result, "g").is_empty());
    let response = h
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(PATH)
                .header("content-type", "application/sparql-query")
                .body(Body::from("SELECT ?g { GRAPH ?g {} }"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert!(values(&body_json(response).await, "g").is_empty());
}
