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
    BlobEntry, BlobError, BlobStore, BodyCache, CompositeStore, InMemoryBlobStore,
    InMemorySparqClient, Store,
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
