// [GPT-5.6] sq-r1ei8: adversarial WAC-scoped dataset tests.
#![cfg(feature = "sparql-endpoint")]

mod common;

use axum::body::{to_bytes, Body, Bytes};
use axum::http::{Request, Response, StatusCode};
use common::{jwks_provider, mint_access_token, mint_dpop_proof, KeyKit, BASE_URL, ISSUER, WEBID};
use serde_json::Value;
use solid_oidc_verifier::config::VerifierConfig;
use solid_oidc_verifier::replay::InMemoryReplayStore;
use solid_oidc_verifier::verifier::Verifier;
use sparq_lws_core::app::{build_router, AppState};
use sparq_lws_core::auth::AuthContext;
use sparq_lws_core::ldp::handler::LdpState;
use sparq_lws_core::store::{
    BlobEntry, BlobError, BlobStore, BodyCache, CompositeStore, DeleteOutcome, InMemoryBlobStore,
    InMemorySparqClient, ResourceMeta, SparqClient, SparqError, Store,
};
use tower::ServiceExt;

const ROOT: &str = "https://pod.example/";
const BOB: &str = "https://id.example/bob#me";
const UNION: &str = "http://www.w3.org/ns/solid/sparql#union-default-graph";

type MemoryStore = CompositeStore<InMemorySparqClient, InMemoryBlobStore>;

struct Harness {
    app: axum::Router,
    issuer_key: KeyKit,
    client_key: KeyKit,
}

impl Harness {
    async fn new(with_secret: bool, with_broken_acl: bool) -> Self {
        let store = MemoryStore::new(InMemorySparqClient::new(), InMemoryBlobStore::new());
        store
            .write(ROOT, Bytes::new(), "text/turtle")
            .await
            .expect("seed root container");
        seed_acl(
            &store,
            "https://pod.example/.acl",
            &format!(
                r#"@prefix acl: <http://www.w3.org/ns/auth/acl#>.
<#owner> a acl:Authorization; acl:agent <{WEBID}>;
  acl:accessTo <{ROOT}>; acl:default <{ROOT}>;
  acl:mode acl:Read, acl:Write, acl:Control."#
            ),
        )
        .await;
        store
            .create_in_container(
                ROOT,
                "https://pod.example/a",
                Bytes::from_static(b"<urn:a> <urn:p> \"visible\" ."),
                "text/turtle",
            )
            .await
            .expect("seed readable resource");

        if with_secret {
            store
                .create_in_container(
                    ROOT,
                    "https://pod.example/b",
                    Bytes::from_static(b"<urn:b> <urn:p> \"secret\" ."),
                    "text/turtle",
                )
                .await
                .expect("seed unreadable resource");
            seed_acl(
                &store,
                "https://pod.example/b.acl",
                &format!(
                    r#"@prefix acl: <http://www.w3.org/ns/auth/acl#>.
<#bob> a acl:Authorization; acl:agent <{BOB}>;
  acl:accessTo <https://pod.example/b>; acl:mode acl:Read."#
                ),
            )
            .await;
        }

        if with_broken_acl {
            store
                .create_in_container(
                    ROOT,
                    "https://pod.example/broken",
                    Bytes::from_static(b"<urn:broken> <urn:p> \"must-not-appear\" ."),
                    "text/turtle",
                )
                .await
                .expect("seed resource governed by broken ACL");
            store
                .write(
                    "https://pod.example/broken.acl",
                    Bytes::from_static(b"this is not Turtle"),
                    "text/turtle",
                )
                .await
                .expect("store deliberately malformed ACL fixture");
        }

        let issuer_key = KeyKit::generate();
        let client_key = KeyKit::generate();
        let config = VerifierConfig::new(vec![ISSUER.to_owned()], BASE_URL);
        let replay = InMemoryReplayStore::with_window(config.replay_ttl());
        let verifier = Verifier::new(config, jwks_provider(&issuer_key), replay)
            .expect("valid verifier fixture");
        let auth = AuthContext::new(verifier, BASE_URL);
        let ldp = LdpState::new(store, BASE_URL);
        Self {
            app: build_router(AppState::new(auth, ldp)),
            issuer_key,
            client_key,
        }
    }

    async fn request(
        &self,
        method: &str,
        uri: &str,
        content_type: Option<&str>,
        accept: Option<&str>,
        body: Body,
    ) -> Response<Body> {
        let access = mint_access_token(&self.issuer_key, &self.client_key.thumbprint);
        // The native verifier intentionally binds ordinary DPoP to the request
        // path (query excluded), matching the production auth middleware.
        let path = uri.split('?').next().expect("URI always has a path");
        let proof = mint_dpop_proof(
            &self.client_key,
            method,
            &format!("{BASE_URL}{path}"),
            &access,
        );
        let mut request = Request::builder()
            .method(method)
            .uri(uri)
            .header("authorization", format!("DPoP {access}"))
            .header("dpop", proof);
        if let Some(content_type) = content_type {
            request = request.header("content-type", content_type);
        }
        if let Some(accept) = accept {
            request = request.header("accept", accept);
        }
        self.app
            .clone()
            .oneshot(request.body(body).expect("valid request"))
            .await
            .expect("router is infallible")
    }

    async fn direct_query(&self, query: &str) -> Response<Body> {
        self.request(
            "POST",
            "/sparql",
            Some("application/sparql-query"),
            None,
            Body::from(query.to_owned()),
        )
        .await
    }
}

async fn seed_acl(store: &MemoryStore, iri: &str, body: &str) {
    store
        .write(iri, Bytes::from(body.to_owned()), "text/turtle")
        .await
        .expect("seed ACL");
}

async fn json(response: Response<Body>) -> Value {
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("buffer response");
    serde_json::from_slice(&bytes).expect("SPARQL results JSON")
}

fn binding_values<'a>(document: &'a Value, variable: &str) -> Vec<&'a str> {
    document["results"]["bindings"]
        .as_array()
        .expect("bindings array")
        .iter()
        .map(|row| row[variable]["value"].as_str().expect("bound string value"))
        .collect()
}

#[tokio::test]
async fn direct_no_leak_matches_ldp_get_authorization() {
    let harness = Harness::new(true, true).await;

    let readable = harness
        .request("GET", "/a", None, None, Body::empty())
        .await;
    let secret = harness
        .request("GET", "/b", None, None, Body::empty())
        .await;
    let broken = harness
        .request("GET", "/broken", None, None, Body::empty())
        .await;
    assert_eq!(readable.status(), StatusCode::OK);
    assert_eq!(secret.status(), StatusCode::FORBIDDEN);
    assert_eq!(broken.status(), StatusCode::FORBIDDEN);

    let rows = json(
        harness
            .direct_query("SELECT ?g WHERE { GRAPH ?g { ?s <urn:p> ?o } } ORDER BY ?g")
            .await,
    )
    .await;
    assert_eq!(binding_values(&rows, "g"), ["https://pod.example/a"]);

    let named_secret = json(
        harness
            .direct_query("SELECT ?s WHERE { GRAPH <https://pod.example/b> { ?s ?p ?o } }")
            .await,
    )
    .await;
    assert!(binding_values(&named_secret, "s").is_empty());

    let broken_acl = json(
        harness
            .direct_query("ASK { GRAPH <https://pod.example/broken> { <urn:broken> <urn:p> ?o } }")
            .await,
    )
    .await;
    assert_eq!(
        broken_acl["boolean"], false,
        "malformed ACL must exclude data"
    );
}

// Dataset assembly must not erase the request's VERSION contract.
#[tokio::test]
async fn endpoint_preserves_version_ebv_and_rejects_unknown_labels() {
    let harness = Harness::new(false, false).await;
    for (version, expected) in [("1.1", true), ("1.2", false)] {
        let query = format!("VERSION '{version}' ASK {{ FILTER(!\"z\"^^<http://www.w3.org/2001/XMLSchema#boolean>) }}");
        let response = harness.direct_query(&query).await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(json(response).await["boolean"], expected);
    }
    for prologue in ["VERSION 'bogus'", "VERSION '1.1' VERSION '1.2'"] {
        assert_eq!(harness.direct_query(&format!("{prologue} ASK {{}}")).await.status(), StatusCode::BAD_REQUEST);
    }
}

#[tokio::test]
async fn negation_cannot_distinguish_an_unreadable_resource_from_absence() {
    let with_secret = Harness::new(true, false).await;
    let without_secret = Harness::new(false, false).await;
    let query = "ASK { FILTER NOT EXISTS { GRAPH ?g { <urn:b> ?p ?o } } }";

    let present = json(with_secret.direct_query(query).await).await;
    let absent = json(without_secret.direct_query(query).await).await;
    assert_eq!(present["boolean"], true);
    assert_eq!(
        present, absent,
        "unreadable and absent must be observationally equal"
    );

    // Pin the design record's bare-default probe too: the standing default is
    // empty regardless of what named resources exist.
    let bare = json(
        with_secret
            .direct_query("ASK { FILTER NOT EXISTS { <urn:b> ?p ?o } }")
            .await,
    )
    .await;
    assert_eq!(bare["boolean"], true);
}

#[tokio::test]
async fn protocol_get_form_post_construct_and_union_default_are_live() {
    let harness = Harness::new(false, false).await;
    let encoded = form_urlencoded::Serializer::new(String::new())
        .append_pair(
            "query",
            "SELECT ?s WHERE { GRAPH <https://pod.example/a> { ?s <urn:p> ?o } }",
        )
        .finish();
    let get = harness
        .request(
            "GET",
            &format!("/sparql?{encoded}"),
            None,
            Some("application/sparql-results+json"),
            Body::empty(),
        )
        .await;
    assert_eq!(binding_values(&json(get).await, "s"), ["urn:a"]);

    let form = form_urlencoded::Serializer::new(String::new())
        .append_pair(
            "query",
            &format!("ASK FROM <{UNION}> {{ <urn:a> <urn:p> \"visible\" }}"),
        )
        .finish();
    let ask = harness
        .request(
            "POST",
            "/sparql",
            Some("application/x-www-form-urlencoded"),
            None,
            Body::from(form),
        )
        .await;
    assert_eq!(json(ask).await["boolean"], true);

    let default_only = form_urlencoded::Serializer::new(String::new())
        .append_pair("query", "ASK { GRAPH ?g { <urn:a> <urn:p> \"visible\" } }")
        .append_pair("default-graph-uri", "https://pod.example/a")
        .finish();
    let default_only = harness
        .request(
            "POST",
            "/sparql",
            Some("application/x-www-form-urlencoded"),
            None,
            Body::from(default_only),
        )
        .await;
    assert_eq!(
        json(default_only).await["boolean"],
        false,
        "a protocol default-graph override must leave the named set empty"
    );

    let construct = harness
        .request(
            "POST",
            "/sparql",
            Some("application/sparql-query"),
            Some("application/n-triples"),
            Body::from(
                "CONSTRUCT { ?s <urn:q> ?o } WHERE { GRAPH <https://pod.example/a> { ?s <urn:p> ?o } }",
            ),
        )
        .await;
    assert_eq!(construct.status(), StatusCode::OK);
    assert_eq!(construct.headers()["content-type"], "application/n-triples");
    let body = to_bytes(construct.into_body(), usize::MAX)
        .await
        .expect("buffer CONSTRUCT result");
    assert_eq!(body.as_ref(), b"<urn:a> <urn:q> \"visible\" .\n");

    let unsupported_graph_format = harness
        .request(
            "POST",
            "/sparql",
            Some("application/sparql-query"),
            Some("text/turtle"),
            Body::from("CONSTRUCT { <urn:a> <urn:p> ?o } WHERE { GRAPH <https://pod.example/a> { <urn:a> <urn:p> ?o } }"),
        )
        .await;
    assert_eq!(
        unsupported_graph_format.status(),
        StatusCode::NOT_ACCEPTABLE
    );

    let hidden_union_graph = json(
        harness
            .direct_query(&format!("ASK {{ GRAPH <{UNION}> {{ ?s ?p ?o }} }}"))
            .await,
    )
    .await;
    assert_eq!(
        hidden_union_graph["boolean"], false,
        "the union opt-in must not leak into GRAPH ?g enumeration"
    );
}

/// A blob store that records every key `get` is asked for.
struct RecordingBlob {
    inner: InMemoryBlobStore,
    gets: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
}

#[async_trait::async_trait]
impl BlobStore for RecordingBlob {
    async fn get(&self, key: &str) -> Result<Bytes, BlobError> {
        self.gets.lock().unwrap().push(key.to_owned());
        self.inner.get(key).await
    }
    async fn put(&self, key: &str, body: Bytes) -> Result<(), BlobError> {
        self.inner.put(key, body).await
    }
    async fn exists(&self, key: &str) -> Result<bool, BlobError> {
        self.inner.exists(key).await
    }
    async fn delete(&self, key: &str) -> Result<(), BlobError> {
        self.inner.delete(key).await
    }
    async fn list(&self) -> Result<Vec<BlobEntry>, BlobError> {
        self.inner.list().await
    }
    async fn delete_if_unchanged(
        &self,
        key: &str,
        expected_generation: u64,
    ) -> Result<bool, BlobError> {
        self.inner
            .delete_if_unchanged(key, expected_generation)
            .await
    }
}

#[tokio::test]
async fn query_never_fetches_readable_non_rdf_bodies() {
    // RDF eligibility is decided from the authorized METADATA before any byte fetch: a readable
    // image must not be pulled from the blob store just to be discarded on every query.
    let gets = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let store = CompositeStore::with_body_cache(
        InMemorySparqClient::new(),
        RecordingBlob {
            inner: InMemoryBlobStore::new(),
            gets: std::sync::Arc::clone(&gets),
        },
        BodyCache::disabled(),
    );
    store
        .write(ROOT, Bytes::new(), "text/turtle")
        .await
        .expect("seed root container");
    store
        .write(
            "https://pod.example/.acl",
            Bytes::from(format!(
                r#"@prefix acl: <http://www.w3.org/ns/auth/acl#>.
<#owner> a acl:Authorization; acl:agent <{WEBID}>;
  acl:accessTo <{ROOT}>; acl:default <{ROOT}>;
  acl:mode acl:Read, acl:Write, acl:Control."#
            )),
            "text/turtle",
        )
        .await
        .expect("seed ACL");
    store
        .create_in_container(
            ROOT,
            "https://pod.example/a",
            Bytes::from_static(b"<urn:a> <urn:p> \"visible\" ."),
            "text/turtle",
        )
        .await
        .expect("seed RDF resource");
    let image = store
        .create_in_container(
            ROOT,
            "https://pod.example/photo.png",
            Bytes::from(vec![0x89u8; 4096]),
            "image/png",
        )
        .await
        .expect("seed non-RDF resource");
    let rdf = store.meta("https://pod.example/a").await.unwrap().unwrap();

    let issuer_key = KeyKit::generate();
    let client_key = KeyKit::generate();
    let config = VerifierConfig::new(vec![ISSUER.to_owned()], BASE_URL);
    let replay = InMemoryReplayStore::with_window(config.replay_ttl());
    let verifier = Verifier::new(config, jwks_provider(&issuer_key), replay).unwrap();
    let app = build_router(AppState::new(
        AuthContext::new(verifier, BASE_URL),
        LdpState::new(store, BASE_URL),
    ));

    gets.lock().unwrap().clear();
    let access = mint_access_token(&issuer_key, &client_key.thumbprint);
    let proof = mint_dpop_proof(&client_key, "POST", &format!("{BASE_URL}/sparql"), &access);
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/sparql")
                .header("authorization", format!("DPoP {access}"))
                .header("dpop", proof)
                .header("content-type", "application/sparql-query")
                .body(Body::from(
                    "SELECT ?g WHERE { GRAPH ?g { ?s <urn:p> ?o } }".to_owned(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    let rows = json(response).await;
    assert_eq!(binding_values(&rows, "g"), ["https://pod.example/a"]);

    let gets = gets.lock().unwrap();
    assert!(
        gets.contains(&rdf.blob_key),
        "the readable RDF resource is fetched and loaded"
    );
    assert!(
        !gets.contains(&image.blob_key),
        "a readable non-RDF body must not be fetched by a SPARQL query (gets: {gets:?})"
    );
}

/// A [`SparqClient`] that, once `churn` is set, answers every `get_meta` of `churn_iri` with a
/// brand-new version whose blob does not exist — exactly what a reader observes while that resource
/// is rewritten (and its previous bytes reclaimed) during every one of its byte fetches.
struct ChurningSparq {
    inner: InMemorySparqClient,
    churn_iri: &'static str,
    churn: std::sync::Arc<std::sync::atomic::AtomicBool>,
    version: std::sync::atomic::AtomicU64,
}

#[async_trait::async_trait]
impl SparqClient for ChurningSparq {
    async fn get_meta(&self, iri: &str) -> Result<ResourceMeta, SparqError> {
        let mut meta = self.inner.get_meta(iri).await?;
        if iri == self.churn_iri && self.churn.load(std::sync::atomic::Ordering::SeqCst) {
            let n = self
                .version
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            meta.blob_key = format!("churned-{n}");
            meta.etag = format!("\"churned-{n}\"");
        }
        Ok(meta)
    }
    async fn put_meta(&self, iri: &str, meta: ResourceMeta) -> Result<(), SparqError> {
        self.inner.put_meta(iri, meta).await
    }
    async fn replace_meta(
        &self,
        iri: &str,
        meta: ResourceMeta,
    ) -> Result<Option<ResourceMeta>, SparqError> {
        self.inner.replace_meta(iri, meta).await
    }
    async fn exists(&self, iri: &str) -> Result<bool, SparqError> {
        self.inner.exists(iri).await
    }
    async fn delete_meta(&self, iri: &str) -> Result<(), SparqError> {
        self.inner.delete_meta(iri).await
    }
    async fn delete_meta_if_empty(
        &self,
        iri: &str,
        parent: Option<&str>,
    ) -> Result<DeleteOutcome, SparqError> {
        self.inner.delete_meta_if_empty(iri, parent).await
    }
    async fn create_child(
        &self,
        container: &str,
        child: &str,
        meta: ResourceMeta,
    ) -> Result<(), SparqError> {
        self.inner.create_child(container, child, meta).await
    }
    async fn remove_child(&self, container: &str, child: &str) -> Result<(), SparqError> {
        self.inner.remove_child(container, child).await
    }
    async fn list_children(&self, container: &str) -> Result<Vec<String>, SparqError> {
        self.inner.list_children(container).await
    }
    async fn referenced_blob_keys(&self) -> Result<std::collections::HashSet<String>, SparqError> {
        self.inner.referenced_blob_keys().await
    }
}

#[tokio::test]
async fn a_governing_acl_that_keeps_changing_fails_the_query_instead_of_dropping_graphs() {
    // If the governing ACL is rewritten under every authorization attempt, the walk surfaces
    // `ResourceChanged`. That must never be read as a denial (a readable graph silently missing from
    // a 200 answer — wrong counts, false ASK): the bounded restarts run out and the query is a 503.
    const ROOT_ACL: &str = "https://pod.example/.acl";
    let churn = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let store = CompositeStore::with_body_cache(
        ChurningSparq {
            inner: InMemorySparqClient::new(),
            churn_iri: ROOT_ACL,
            churn: std::sync::Arc::clone(&churn),
            version: std::sync::atomic::AtomicU64::new(0),
        },
        InMemoryBlobStore::new(),
        BodyCache::disabled(),
    );
    store
        .write(ROOT, Bytes::new(), "text/turtle")
        .await
        .expect("seed root container");
    store
        .write(
            ROOT_ACL,
            Bytes::from(format!(
                r#"@prefix acl: <http://www.w3.org/ns/auth/acl#>.
<#owner> a acl:Authorization; acl:agent <{WEBID}>;
  acl:accessTo <{ROOT}>; acl:default <{ROOT}>;
  acl:mode acl:Read, acl:Write, acl:Control."#
            )),
            "text/turtle",
        )
        .await
        .expect("seed ACL");
    store
        .create_in_container(
            ROOT,
            "https://pod.example/a",
            Bytes::from_static(b"<urn:a> <urn:p> \"visible\" ."),
            "text/turtle",
        )
        .await
        .expect("seed RDF resource");

    let issuer_key = KeyKit::generate();
    let client_key = KeyKit::generate();
    let config = VerifierConfig::new(vec![ISSUER.to_owned()], BASE_URL);
    let replay = InMemoryReplayStore::with_window(config.replay_ttl());
    let verifier = Verifier::new(config, jwks_provider(&issuer_key), replay).unwrap();
    let app = build_router(AppState::new(
        AuthContext::new(verifier, BASE_URL),
        LdpState::new(store, BASE_URL),
    ));
    let query = |app: axum::Router| {
        let access = mint_access_token(&issuer_key, &client_key.thumbprint);
        let proof = mint_dpop_proof(&client_key, "POST", &format!("{BASE_URL}/sparql"), &access);
        async move {
            app.oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/sparql")
                    .header("authorization", format!("DPoP {access}"))
                    .header("dpop", proof)
                    .header("content-type", "application/sparql-query")
                    .body(Body::from(
                        "SELECT ?g WHERE { GRAPH ?g { ?s <urn:p> ?o } }".to_owned(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap()
        }
    };

    // Stable ACL: the owner sees the graph.
    let rows = json(query(app.clone()).await).await;
    assert_eq!(binding_values(&rows, "g"), ["https://pod.example/a"]);

    // ACL churning under every attempt: a retryable 503, never a 200 missing the graph.
    churn.store(true, std::sync::atomic::Ordering::SeqCst);
    let response = query(app.clone()).await;
    assert_eq!(
        response.status(),
        StatusCode::SERVICE_UNAVAILABLE,
        "an ACL race must fail the query, not silently exclude a readable graph"
    );
    assert!(response.headers().contains_key("retry-after"));

    // LDP GET under the same churn: the error propagates out of authorization (never a 401/403
    // denial), `serve_read`'s bounded restart runs out, and the read is the same retryable 503.
    let access = mint_access_token(&issuer_key, &client_key.thumbprint);
    let proof = mint_dpop_proof(&client_key, "GET", &format!("{BASE_URL}/a"), &access);
    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/a")
                .header("authorization", format!("DPoP {access}"))
                .header("dpop", proof)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
}
