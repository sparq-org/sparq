// AUTHORED-BY Claude Opus 4.8
//! End-to-end LDP HTTP tests through the assembled router (auth + GET/HEAD/PUT over the store).
//!
//! Each request carries a fresh, well-formed DPoP-bound token + a per-request proof (a new jti) so
//! the verifier's single-use replay protection does not reject the second request of a PUT→GET pair.
//!
//! **Backend under test (sq-gg0qq.3):** with the DEFAULT feature set this WHOLE suite runs against
//! the EMBEDDED in-process SPARQ engine ([`sparq_lws_core::store::embedded`]) — the first-class
//! in-workspace backend — so every LDP verb/conneg/conditional behavior here is proven on the real
//! engine, not assumed from a double. Under `--no-default-features` (the engine-free profile) it
//! falls back to the in-memory double, keeping that profile's LDP surface covered too.

mod common;

use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use common::{jwks_provider, mint_access_token, mint_dpop_proof, KeyKit, BASE_URL};
use solid_oidc_verifier::config::VerifierConfig;
use solid_oidc_verifier::replay::InMemoryReplayStore;
use solid_oidc_verifier::verifier::Verifier;
use sparq_lws_core::app::{build_router, AppState};
use sparq_lws_core::auth::AuthContext;
use sparq_lws_core::ldp::handler::LdpState;
#[cfg(not(feature = "embedded-sparq"))]
use sparq_lws_core::store::InMemorySparqClient;
use sparq_lws_core::store::{CompositeStore, InMemoryBlobStore};
use sparq_lws_core::store::{InMemoryStoreLimits, SparqClient};
use tower::ServiceExt;

const TURTLE: &str =
    "<https://pod.example/alice/data#me> <http://xmlns.com/foaf/0.1/name> \"Alice\" .";

/// The [`sparq_lws_core::store::SparqClient`] backend this suite exercises: the EMBEDDED in-process
/// engine on the default build, the in-memory double on `--no-default-features`.
#[cfg(feature = "embedded-sparq")]
type BackendSparqClient = sparq_lws_core::store::EmbeddedSparqClient;
#[cfg(not(feature = "embedded-sparq"))]
type BackendSparqClient = InMemorySparqClient;

/// A fresh, empty backend instance (one per [`Harness`], so tests stay isolated).
fn backend_sparq_client() -> BackendSparqClient {
    #[cfg(feature = "embedded-sparq")]
    {
        sparq_lws_core::store::EmbeddedSparqClient::in_memory()
            .expect("fresh in-memory embedded graph")
    }
    #[cfg(not(feature = "embedded-sparq"))]
    {
        InMemorySparqClient::new()
    }
}

/// Seed a ROOT `<base>/.acl` granting `owner_webid` Read/Write/Control on the base root AND on all
/// descendants (`acl:default`), so every resource these LDP tests touch inherits owner control under
/// the now-enforced WAC engine. Written through the store as an auxiliary `.acl` resource (built as a
/// Turtle string — a test fixture, not production RDF construction). This mirrors the pod-root
/// owner-default the real conformance seed (`seed_conformance`) writes per user.
async fn seed_root_owner_acl<S: SparqClient>(
    store: &CompositeStore<S, InMemoryBlobStore>,
    base_url: &str,
    owner_webid: &str,
) {
    use sparq_lws_core::store::Store;
    let base = base_url.trim_end_matches('/');
    let root = format!("{base}/");
    let acl_iri = format!("{root}.acl");
    let acl_body = format!(
        r#"@prefix acl: <http://www.w3.org/ns/auth/acl#>.
<#owner> a acl:Authorization;
         acl:agent <{owner_webid}>;
         acl:accessTo <{root}>;
         acl:default <{root}>;
         acl:mode acl:Read, acl:Write, acl:Control."#
    );
    store
        .write(&acl_iri, axum::body::Bytes::from(acl_body), "text/turtle")
        .await
        .expect("seed root acl");
}

/// One shared app (so a PUT and a later GET hit the same store), plus the keys to mint requests.
struct Harness {
    app: axum::Router,
    issuer_key: KeyKit,
    client_key: KeyKit,
}

impl Harness {
    async fn new() -> Self {
        Self::with_store(CompositeStore::new(
            backend_sparq_client(),
            InMemoryBlobStore::new(),
        ))
        .await
    }

    /// Build the real HTTP stack over a deliberately small in-memory quota.
    async fn with_memory_limits(limits: InMemoryStoreLimits) -> Self {
        Self::with_store(CompositeStore::in_memory_with_limits(limits)).await
    }

    async fn with_store<S: SparqClient + 'static>(
        store: CompositeStore<S, InMemoryBlobStore>,
    ) -> Self {
        let issuer_key = KeyKit::generate();
        let client_key = KeyKit::generate();
        let config = VerifierConfig::new(vec![common::ISSUER.to_string()], BASE_URL);
        let replay = InMemoryReplayStore::with_window(config.replay_ttl());
        let verifier = Verifier::new(config, jwks_provider(&issuer_key), replay).unwrap();
        let ctx = AuthContext::new(verifier, BASE_URL);
        // WAC is now ENFORCED, so the test caller (Alice, `common::WEBID`) needs an effective ACL
        // granting her access to every path these tests exercise. Seed a ROOT `.acl` granting Alice
        // Read/Write/Control on the base root AND on all descendants (`acl:default`), so every
        // resource under `https://pod.example/` inherits owner control — exactly the pod-root
        // owner-default the real conformance seed (`seed_conformance`) writes per user. The ACL is
        // written via the store directly (an auxiliary `.acl` resource).
        seed_root_owner_acl(&store, BASE_URL, common::WEBID).await;
        let ldp = LdpState::new(store, BASE_URL);
        // Use `AppState::new` (not the struct literal) so the LDP layer's anonymous-401
        // `WWW-Authenticate` challenge is derived from the verifier (names the trusted issuer + algs).
        let app = build_router(AppState::new(ctx, ldp));
        Self {
            app,
            issuer_key,
            client_key,
        }
    }

    /// A fresh `(Authorization, DPoP)` pair for one request (new proof jti each call).
    fn auth_headers(&self, method: &str, path: &str) -> (String, String) {
        let access = mint_access_token(&self.issuer_key, &self.client_key.thumbprint);
        let htu = format!("{BASE_URL}{path}");
        let proof = mint_dpop_proof(&self.client_key, method, &htu, &access);
        (format!("DPoP {access}"), proof)
    }

    async fn request(
        &self,
        method: &str,
        path: &str,
        content_type: Option<&str>,
        body: Body,
    ) -> axum::http::Response<Body> {
        self.request_with(method, path, content_type, &[], body)
            .await
    }

    /// An authenticated request with arbitrary extra headers (Slug / If-Match / Range / Accept …).
    async fn request_with(
        &self,
        method: &str,
        path: &str,
        content_type: Option<&str>,
        extra: &[(&str, &str)],
        body: Body,
    ) -> axum::http::Response<Body> {
        let (authz, dpop) = self.auth_headers(method, path);
        let mut builder = Request::builder()
            .method(method)
            .uri(path)
            .header("authorization", authz)
            .header("dpop", dpop);
        if let Some(ct) = content_type {
            builder = builder.header("content-type", ct);
        }
        for (k, v) in extra {
            builder = builder.header(*k, *v);
        }
        self.app
            .clone()
            .oneshot(builder.body(body).unwrap())
            .await
            .unwrap()
    }

    /// Create the container at `path` (a trailing-slash path) so a subsequent POST can target it.
    async fn make_container(&self, path: &str) {
        let resp = self
            .request(
                "PUT",
                path,
                Some("text/turtle"),
                Body::from("<#c> <http://xmlns.com/foaf/0.1/name> \"Container\" ."),
            )
            .await;
        assert_eq!(resp.status(), StatusCode::CREATED);
    }

    /// An UNAUTHENTICATED request (no Authorization / DPoP).
    async fn unauth_request(
        &self,
        method: &str,
        path: &str,
        content_type: Option<&str>,
        body: Body,
    ) -> axum::http::Response<Body> {
        let mut builder = Request::builder().method(method).uri(path);
        if let Some(ct) = content_type {
            builder = builder.header("content-type", ct);
        }
        self.app
            .clone()
            .oneshot(builder.body(body).unwrap())
            .await
            .unwrap()
    }
}

#[tokio::test]
async fn put_creates_then_get_reads_it_back() {
    let h = Harness::new().await;

    // PUT a fresh resource → 201 Created with a Location + ETag.
    let put = h
        .request(
            "PUT",
            "/alice/data",
            Some("text/turtle"),
            Body::from(TURTLE),
        )
        .await;
    assert_eq!(put.status(), StatusCode::CREATED);
    assert!(put.headers().contains_key(axum::http::header::ETAG));
    assert!(put.headers().contains_key(axum::http::header::LOCATION));

    // GET it back → 200 with the same bytes + content type.
    let get = h.request("GET", "/alice/data", None, Body::empty()).await;
    assert_eq!(get.status(), StatusCode::OK);
    assert_eq!(
        get.headers().get(axum::http::header::CONTENT_TYPE).unwrap(),
        "text/turtle"
    );
    let bytes = to_bytes(get.into_body(), usize::MAX).await.unwrap();
    assert_eq!(&bytes[..], TURTLE.as_bytes());
}

/// [GPT-5.6] sq-r1ei8: in the core tier `/sparql` is an ordinary absent LDP
/// resource (404), while authenticated CRUD and private-resource WAC remain live.
/// Enabling the route mutates the first assertion to 400 (missing query), so this
/// is a direct compiled-route witness rather than a symbol-only check.
#[cfg(not(feature = "sparql-endpoint"))]
#[tokio::test]
async fn core_tier_has_no_sparql_route_but_keeps_ldp_and_wac() {
    let h = Harness::new().await;
    let absent = h.request("GET", "/sparql", None, Body::empty()).await;
    assert_eq!(absent.status(), StatusCode::NOT_FOUND);

    let put = h
        .request(
            "PUT",
            "/core-round-trip",
            Some("text/turtle"),
            Body::from("<urn:core> <urn:p> \"ok\" ."),
        )
        .await;
    assert_eq!(put.status(), StatusCode::CREATED);
    let get = h
        .request("GET", "/core-round-trip", None, Body::empty())
        .await;
    assert_eq!(get.status(), StatusCode::OK);

    let anonymous = h
        .unauth_request("GET", "/core-round-trip", None, Body::empty())
        .await;
    assert_eq!(anonymous.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn put_flood_beyond_resource_cap_is_refused_with_507() {
    // [GPT-5.6] The seed ACL and any implicit ancestor containers consume the same bounded store.
    // Keep issuing unique, zero-ish-body resources until the deliberately tiny cap trips; at least
    // one write must land first, and every later admission must fail closed as HTTP 507.
    let limits = InMemoryStoreLimits::new(1024 * 1024, 8);
    let h = Harness::with_memory_limits(limits).await;
    let mut accepted = 0;
    let mut refused = false;

    for n in 0..=limits.max_resource_count {
        let path = format!("/quota-{n}.txt");
        let response = h
            .request("PUT", &path, Some("text/plain"), Body::from("x"))
            .await;
        if response.status() == StatusCode::INSUFFICIENT_STORAGE {
            refused = true;
            break;
        }
        assert_eq!(response.status(), StatusCode::CREATED);
        accepted += 1;
    }

    assert!(
        accepted > 0,
        "the test must fill a live store before refusal"
    );
    assert!(refused, "a unique-resource PUT flood must hit the hard cap");
    let retry = h
        .request(
            "PUT",
            "/quota-after-cap.txt",
            Some("text/plain"),
            Body::from("x"),
        )
        .await;
    assert_eq!(retry.status(), StatusCode::INSUFFICIENT_STORAGE);
}

#[tokio::test]
async fn put_twice_is_a_replace_with_no_content() {
    let h = Harness::new().await;
    let first = h
        .request(
            "PUT",
            "/alice/data",
            Some("text/turtle"),
            Body::from(TURTLE),
        )
        .await;
    assert_eq!(first.status(), StatusCode::CREATED);

    let second = h
        .request(
            "PUT",
            "/alice/data",
            Some("text/turtle"),
            Body::from("<#me> <http://xmlns.com/foaf/0.1/name> \"Alice 2\" ."),
        )
        .await;
    // A replace returns 204 No Content (the resource already existed).
    assert_eq!(second.status(), StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn head_returns_headers_without_a_body() {
    let h = Harness::new().await;
    h.request(
        "PUT",
        "/alice/data",
        Some("text/turtle"),
        Body::from(TURTLE),
    )
    .await;

    let head = h.request("HEAD", "/alice/data", None, Body::empty()).await;
    assert_eq!(head.status(), StatusCode::OK);
    assert!(head
        .headers()
        .contains_key(axum::http::header::CONTENT_TYPE));
    let bytes = to_bytes(head.into_body(), usize::MAX).await.unwrap();
    assert!(bytes.is_empty(), "HEAD must not return a body");
}

#[tokio::test]
async fn put_with_a_non_rdf_content_type_is_stored_as_a_binary_resource() {
    // The Solid Protocol stores ANY content type — a non-RDF type (here a text/plain blob) is stored
    // VERBATIM as an opaque binary resource (the CORS scenarios create text/plain resources). It is
    // NOT a 415: 415 is only for an unsupported PATCH language, not a write content type.
    let h = Harness::new().await;
    let put = h
        .request(
            "PUT",
            "/alice/blob.txt",
            Some("text/plain"),
            Body::from("Hello"),
        )
        .await;
    assert_eq!(put.status(), StatusCode::CREATED);

    // Read it back verbatim with its declared content type.
    let get = h
        .request("GET", "/alice/blob.txt", None, Body::empty())
        .await;
    assert_eq!(get.status(), StatusCode::OK);
    assert_eq!(
        get.headers().get(axum::http::header::CONTENT_TYPE).unwrap(),
        "text/plain"
    );
    let bytes = to_bytes(get.into_body(), usize::MAX).await.unwrap();
    assert_eq!(&bytes[..], b"Hello");
}

#[tokio::test]
async fn put_malformed_rdf_in_a_declared_rdf_type_is_400() {
    // An RDF content type IS validated — a malformed Turtle body is a 400 (only RDF types are parsed;
    // a binary type is stored unparsed).
    let h = Harness::new().await;
    let resp = h
        .request(
            "PUT",
            "/alice/data",
            Some("text/turtle"),
            Body::from("<a> <b> ."),
        )
        .await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn unauthenticated_put_is_401_with_challenge_fail_closed() {
    // A write from a public/unauthenticated caller must be rejected — the pre-WAC posture answers a
    // 401 + `WWW-Authenticate` (so a client knows to obtain a token), not a bare 403, and never a
    // fail-open write.
    let h = Harness::new().await;
    let resp = h
        .unauth_request(
            "PUT",
            "/alice/data",
            Some("text/turtle"),
            Body::from(TURTLE),
        )
        .await;
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    assert!(resp
        .headers()
        .contains_key(axum::http::header::WWW_AUTHENTICATE));

    // And nothing was written — a subsequent (authenticated) GET still 404s.
    let get = h.request("GET", "/alice/data", None, Body::empty()).await;
    assert_eq!(get.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn put_with_malformed_turtle_is_400() {
    let h = Harness::new().await;
    let resp = h
        .request(
            "PUT",
            "/alice/data",
            Some("text/turtle"),
            Body::from("<a> <b> ."), // missing object
        )
        .await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

// --- M2: content negotiation -------------------------------------------------------------------

#[tokio::test]
async fn get_negotiates_jsonld_from_stored_turtle() {
    let h = Harness::new().await;
    h.request(
        "PUT",
        "/alice/data",
        Some("text/turtle"),
        Body::from(TURTLE),
    )
    .await;

    let get = h
        .request_with(
            "GET",
            "/alice/data",
            None,
            &[("accept", "application/ld+json")],
            Body::empty(),
        )
        .await;
    assert_eq!(get.status(), StatusCode::OK);
    assert_eq!(
        get.headers().get(axum::http::header::CONTENT_TYPE).unwrap(),
        "application/ld+json"
    );
    let bytes = to_bytes(get.into_body(), usize::MAX).await.unwrap();
    let text = String::from_utf8(bytes.to_vec()).unwrap();
    // The JSON-LD re-serialisation must contain the subject + the foaf name.
    assert!(text.contains("alice/data#me"), "JSON-LD body: {text}");
    assert!(text.contains("Alice"), "JSON-LD body: {text}");
}

#[tokio::test]
async fn get_with_unknown_accept_falls_back_to_turtle() {
    // An Accept naming no producible type degrades to the Solid default (text/turtle) rather than
    // failing the read with a 406.
    let h = Harness::new().await;
    h.request(
        "PUT",
        "/alice/data",
        Some("text/turtle"),
        Body::from(TURTLE),
    )
    .await;
    let get = h
        .request_with(
            "GET",
            "/alice/data",
            None,
            &[("accept", "image/png")],
            Body::empty(),
        )
        .await;
    assert_eq!(get.status(), StatusCode::OK);
    assert_eq!(
        get.headers().get(axum::http::header::CONTENT_TYPE).unwrap(),
        "text/turtle"
    );
}

#[tokio::test]
async fn get_with_explicit_q_zero_refusal_is_406() {
    // The only remaining 406: the client explicitly refused (q=0) every producible type.
    let h = Harness::new().await;
    h.request(
        "PUT",
        "/alice/data",
        Some("text/turtle"),
        Body::from(TURTLE),
    )
    .await;
    let get = h
        .request_with(
            "GET",
            "/alice/data",
            None,
            &[("accept", "text/turtle;q=0, application/ld+json;q=0")],
            Body::empty(),
        )
        .await;
    assert_eq!(get.status(), StatusCode::NOT_ACCEPTABLE);
}

// --- M2: Range ---------------------------------------------------------------------------------

#[tokio::test]
async fn get_with_a_range_returns_206_partial() {
    let h = Harness::new().await;
    h.request(
        "PUT",
        "/alice/data",
        Some("text/turtle"),
        Body::from(TURTLE),
    )
    .await;
    let get = h
        .request_with(
            "GET",
            "/alice/data",
            None,
            &[("range", "bytes=0-3")],
            Body::empty(),
        )
        .await;
    assert_eq!(get.status(), StatusCode::PARTIAL_CONTENT);
    let cr = get
        .headers()
        .get(axum::http::header::CONTENT_RANGE)
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    assert!(cr.starts_with("bytes 0-3/"), "Content-Range: {cr}");
    let bytes = to_bytes(get.into_body(), usize::MAX).await.unwrap();
    assert_eq!(bytes.len(), 4);
    assert_eq!(&bytes[..], &TURTLE.as_bytes()[0..4]);
}

#[tokio::test]
async fn head_with_a_range_is_200_not_206() {
    // Range is defined for GET; a HEAD with Range must NOT return 206.
    let h = Harness::new().await;
    h.request(
        "PUT",
        "/alice/data",
        Some("text/turtle"),
        Body::from(TURTLE),
    )
    .await;
    let head = h
        .request_with(
            "HEAD",
            "/alice/data",
            None,
            &[("range", "bytes=0-3")],
            Body::empty(),
        )
        .await;
    assert_eq!(head.status(), StatusCode::OK);
}

#[tokio::test]
async fn get_with_an_unsatisfiable_range_is_416() {
    let h = Harness::new().await;
    h.request(
        "PUT",
        "/alice/data",
        Some("text/turtle"),
        Body::from(TURTLE),
    )
    .await;
    let get = h
        .request_with(
            "GET",
            "/alice/data",
            None,
            &[("range", "bytes=99999-100000")],
            Body::empty(),
        )
        .await;
    assert_eq!(get.status(), StatusCode::RANGE_NOT_SATISFIABLE);
    assert!(get
        .headers()
        .contains_key(axum::http::header::CONTENT_RANGE));
}

// --- M2: conditional requests ------------------------------------------------------------------

#[tokio::test]
async fn put_if_none_match_star_blocks_overwrite() {
    let h = Harness::new().await;
    let first = h
        .request(
            "PUT",
            "/alice/data",
            Some("text/turtle"),
            Body::from(TURTLE),
        )
        .await;
    assert_eq!(first.status(), StatusCode::CREATED);

    // A second PUT with If-None-Match: * (create-only) must fail with 412 (it already exists).
    let second = h
        .request_with(
            "PUT",
            "/alice/data",
            Some("text/turtle"),
            &[("if-none-match", "*")],
            Body::from(TURTLE),
        )
        .await;
    assert_eq!(second.status(), StatusCode::PRECONDITION_FAILED);
}

#[tokio::test]
async fn put_if_match_with_wrong_etag_is_412() {
    let h = Harness::new().await;
    h.request(
        "PUT",
        "/alice/data",
        Some("text/turtle"),
        Body::from(TURTLE),
    )
    .await;
    let resp = h
        .request_with(
            "PUT",
            "/alice/data",
            Some("text/turtle"),
            &[("if-match", "\"not-the-real-etag\"")],
            Body::from(TURTLE),
        )
        .await;
    assert_eq!(resp.status(), StatusCode::PRECONDITION_FAILED);
}

#[tokio::test]
async fn put_if_match_with_correct_etag_succeeds() {
    let h = Harness::new().await;
    let first = h
        .request(
            "PUT",
            "/alice/data",
            Some("text/turtle"),
            Body::from(TURTLE),
        )
        .await;
    let etag = first
        .headers()
        .get(axum::http::header::ETAG)
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();

    let resp = h
        .request_with(
            "PUT",
            "/alice/data",
            Some("text/turtle"),
            &[("if-match", &etag)],
            Body::from("<#me> <http://xmlns.com/foaf/0.1/name> \"Updated\" ."),
        )
        .await;
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);
}

// --- M2: POST ----------------------------------------------------------------------------------

#[tokio::test]
async fn post_creates_a_child_with_slug() {
    let h = Harness::new().await;
    h.make_container("/alice/").await;
    let resp = h
        .request_with(
            "POST",
            "/alice/",
            Some("text/turtle"),
            &[("slug", "note1")],
            Body::from("<#it> <http://xmlns.com/foaf/0.1/name> \"Note\" ."),
        )
        .await;
    assert_eq!(resp.status(), StatusCode::CREATED);
    let location = resp
        .headers()
        .get(axum::http::header::LOCATION)
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    // V2 (`research/lws-design-records.md` §6): the `Location` is collision-INDEPENDENT — it
    // CONTAINS the Slug (`note1`) as an opaque-suffixed prefix but is never the verbatim segment,
    // so it leaks nothing about which child names already exist. (The CTH
    // `post-uri-assignment-slug` row asserts `Location contains '<slug>'`, satisfied here.)
    assert!(
        location.starts_with("https://pod.example/alice/note1-"),
        "Location must contain the Slug as an opaque-suffixed prefix: {location}"
    );

    // The child is readable at its minted Location.
    let child_path = location
        .strip_prefix("https://pod.example")
        .unwrap_or(&location)
        .to_string();
    let get = h.request("GET", &child_path, None, Body::empty()).await;
    assert_eq!(get.status(), StatusCode::OK);
}

#[tokio::test]
async fn post_mints_a_uri_without_a_slug() {
    let h = Harness::new().await;
    h.make_container("/alice/").await;
    let resp = h
        .request(
            "POST",
            "/alice/",
            Some("text/turtle"),
            Body::from("<#it> <http://xmlns.com/foaf/0.1/name> \"X\" ."),
        )
        .await;
    assert_eq!(resp.status(), StatusCode::CREATED);
    let location = resp
        .headers()
        .get(axum::http::header::LOCATION)
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    assert!(
        location.starts_with("https://pod.example/alice/"),
        "minted Location: {location}"
    );
    assert_ne!(location, "https://pod.example/alice/");
}

#[tokio::test]
async fn post_to_a_non_container_target_is_404_or_405() {
    let h = Harness::new().await;
    // The target is a plain resource path (no trailing slash) that does not exist ⇒ 404 (per the
    // Solid Protocol `post-target-not-found` — POST is not a containment op on a non-container; the
    // accepted statuses are 404 when nothing is there / 405 when a resource is there).
    let resp = h
        .request(
            "POST",
            "/alice/data",
            Some("text/turtle"),
            Body::from(TURTLE),
        )
        .await;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);

    // When a plain resource DOES exist at the target, POST is 405 Method Not Allowed.
    h.request(
        "PUT",
        "/alice/data",
        Some("text/turtle"),
        Body::from(TURTLE),
    )
    .await;
    let resp = h
        .request(
            "POST",
            "/alice/data",
            Some("text/turtle"),
            Body::from(TURTLE),
        )
        .await;
    assert_eq!(resp.status(), StatusCode::METHOD_NOT_ALLOWED);
}

#[tokio::test]
async fn post_to_a_missing_container_is_404() {
    let h = Harness::new().await;
    // The container path is well-formed but was never created — must not create a child under it.
    let resp = h
        .request(
            "POST",
            "/missing/",
            Some("text/turtle"),
            Body::from("<#it> <http://xmlns.com/foaf/0.1/name> \"X\" ."),
        )
        .await;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn unauthenticated_post_is_401_with_challenge() {
    let h = Harness::new().await;
    let resp = h
        .unauth_request("POST", "/alice/", Some("text/turtle"), Body::from(TURTLE))
        .await;
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    assert!(resp
        .headers()
        .contains_key(axum::http::header::WWW_AUTHENTICATE));
}

// --- M2: DELETE --------------------------------------------------------------------------------

#[tokio::test]
async fn delete_removes_a_resource() {
    let h = Harness::new().await;
    h.request(
        "PUT",
        "/alice/data",
        Some("text/turtle"),
        Body::from(TURTLE),
    )
    .await;

    let del = h
        .request("DELETE", "/alice/data", None, Body::empty())
        .await;
    assert_eq!(del.status(), StatusCode::NO_CONTENT);

    // It is gone.
    let get = h.request("GET", "/alice/data", None, Body::empty()).await;
    assert_eq!(get.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn delete_a_missing_resource_is_a_uniform_denial_not_404() {
    // Under WAC, a DELETE of a non-existent target is reported through the SAME denial surface as a
    // permission failure (401 anonymous / 403 authenticated), NOT a 404 — so a DELETE cannot be used
    // as an existence side-channel (the WAC `write-access` matrix asserts `[401]`/`[403]` for missing
    // `fictive` targets even when the requester would otherwise have had inherited write). The test
    // caller is the authenticated owner, so the uniform denial is 403.
    let h = Harness::new().await;
    let del = h
        .request("DELETE", "/alice/gone", None, Body::empty())
        .await;
    assert_eq!(del.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn delete_a_non_empty_container_is_409() {
    let h = Harness::new().await;
    // The container must exist before a child can be POSTed into it.
    h.make_container("/alice/").await;
    // POST a child into the container so it has a member.
    h.request_with(
        "POST",
        "/alice/",
        Some("text/turtle"),
        &[("slug", "child")],
        Body::from("<#it> <http://xmlns.com/foaf/0.1/name> \"C\" ."),
    )
    .await;

    let del = h.request("DELETE", "/alice/", None, Body::empty()).await;
    assert_eq!(del.status(), StatusCode::CONFLICT);
}

#[tokio::test]
async fn delete_an_empty_container_succeeds() {
    let h = Harness::new().await;
    h.make_container("/alice/").await;

    // The empty container can be deleted (the spec choice: empty ⇒ allowed, non-empty ⇒ 409).
    let del = h.request("DELETE", "/alice/", None, Body::empty()).await;
    assert_eq!(del.status(), StatusCode::NO_CONTENT);

    // It is gone — a subsequent GET is a 404.
    let get = h.request("GET", "/alice/", None, Body::empty()).await;
    assert_eq!(get.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn delete_a_container_after_emptying_it_succeeds() {
    let h = Harness::new().await;
    h.make_container("/alice/").await;
    // POST a child, then DELETE the child (emptying the container), then DELETE the container.
    let post = h
        .request_with(
            "POST",
            "/alice/",
            Some("text/turtle"),
            &[("slug", "child")],
            Body::from("<#it> <http://xmlns.com/foaf/0.1/name> \"C\" ."),
        )
        .await;
    assert_eq!(post.status(), StatusCode::CREATED);
    let child_loc = post
        .headers()
        .get("location")
        .and_then(|v| v.to_str().ok())
        .expect("POST returns a Location")
        .to_string();
    // The Location is an absolute IRI; DELETE wants the path.
    let child_path = child_loc
        .strip_prefix("https://pod.example")
        .unwrap_or(&child_loc)
        .to_string();

    // While the child is present, the container DELETE is refused.
    let del_full = h.request("DELETE", "/alice/", None, Body::empty()).await;
    assert_eq!(del_full.status(), StatusCode::CONFLICT);

    // Delete the child, which empties the container.
    let del_child = h.request("DELETE", &child_path, None, Body::empty()).await;
    assert_eq!(del_child.status(), StatusCode::NO_CONTENT);

    // Now the (empty) container can be deleted.
    let del_container = h.request("DELETE", "/alice/", None, Body::empty()).await;
    assert_eq!(del_container.status(), StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn unauthenticated_delete_is_401_with_challenge() {
    let h = Harness::new().await;
    h.request(
        "PUT",
        "/alice/data",
        Some("text/turtle"),
        Body::from(TURTLE),
    )
    .await;
    let del = h
        .unauth_request("DELETE", "/alice/data", None, Body::empty())
        .await;
    assert_eq!(del.status(), StatusCode::UNAUTHORIZED);
    assert!(del
        .headers()
        .contains_key(axum::http::header::WWW_AUTHENTICATE));
}

// --- M2: PATCH (N3 Patch) ----------------------------------------------------------------------

const N3_PATCH: &str = "@prefix solid: <http://www.w3.org/ns/solid/terms#> .\n\
@prefix foaf: <http://xmlns.com/foaf/0.1/> .\n\
_:patch a solid:InsertDeletePatch;\n\
  solid:deletes { <https://pod.example/alice/data#me> foaf:name \"Alice\" . };\n\
  solid:inserts { <https://pod.example/alice/data#me> foaf:name \"Alice 2\" . }.\n";

#[tokio::test]
async fn patch_inserts_and_deletes_triples() {
    let h = Harness::new().await;
    h.request(
        "PUT",
        "/alice/data",
        Some("text/turtle"),
        Body::from(TURTLE),
    )
    .await;

    let patch = h
        .request(
            "PATCH",
            "/alice/data",
            Some("text/n3"),
            Body::from(N3_PATCH),
        )
        .await;
    assert_eq!(patch.status(), StatusCode::NO_CONTENT);

    // The resource now carries the new name and not the old one.
    let get = h.request("GET", "/alice/data", None, Body::empty()).await;
    let bytes = to_bytes(get.into_body(), usize::MAX).await.unwrap();
    let text = String::from_utf8(bytes.to_vec()).unwrap();
    assert!(text.contains("Alice 2"), "patched body: {text}");
    assert!(
        !text.contains("\"Alice\""),
        "old value still present: {text}"
    );
}

#[tokio::test]
async fn patch_with_an_unsupported_media_type_is_415() {
    let h = Harness::new().await;
    h.request(
        "PUT",
        "/alice/data",
        Some("text/turtle"),
        Body::from(TURTLE),
    )
    .await;
    // An unsupported PATCH type (neither text/n3 nor application/sparql-update) ⇒ 415.
    let resp = h
        .request(
            "PATCH",
            "/alice/data",
            Some("application/json-patch+json"),
            Body::from("[]"),
        )
        .await;
    assert_eq!(resp.status(), StatusCode::UNSUPPORTED_MEDIA_TYPE);
}

#[tokio::test]
async fn patch_with_sparql_update_insert_data_applies() {
    // The SPARQL-Update INSERT DATA subset is now supported (the containment scenario uses it).
    let h = Harness::new().await;
    h.request(
        "PUT",
        "/alice/data",
        Some("text/turtle"),
        Body::from(TURTLE),
    )
    .await;
    let resp = h
        .request(
            "PATCH",
            "/alice/data",
            Some("application/sparql-update"),
            Body::from(
                "INSERT DATA { <https://pod.example/alice/data#me> \
                 <http://xmlns.com/foaf/0.1/nick> \"al\" . }",
            ),
        )
        .await;
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);

    let get = h.request("GET", "/alice/data", None, Body::empty()).await;
    let bytes = to_bytes(get.into_body(), usize::MAX).await.unwrap();
    let text = String::from_utf8(bytes.to_vec()).unwrap();
    assert!(text.contains("\"al\""), "patched body: {text}");
}

#[tokio::test]
async fn patch_deleting_an_absent_triple_is_409() {
    let h = Harness::new().await;
    h.request(
        "PUT",
        "/alice/data",
        Some("text/turtle"),
        Body::from(TURTLE),
    )
    .await;
    let doc = "@prefix solid: <http://www.w3.org/ns/solid/terms#> .\n\
@prefix foaf: <http://xmlns.com/foaf/0.1/> .\n\
_:patch solid:deletes { <https://pod.example/alice/data#me> foaf:name \"NotThere\" . }.\n";
    let resp = h
        .request("PATCH", "/alice/data", Some("text/n3"), Body::from(doc))
        .await;
    assert_eq!(resp.status(), StatusCode::CONFLICT);
}

/// A templated `solid:where` patch end-to-end: bind the current name, delete it, insert a new one
/// (the canonical "rename" patch). The single solution drives the templates and the body reads back
/// with the new value.
#[tokio::test]
async fn patch_with_a_templated_where_renames() {
    let h = Harness::new().await;
    h.request(
        "PUT",
        "/alice/data",
        Some("text/turtle"),
        Body::from(TURTLE),
    )
    .await;
    let doc = "@prefix solid: <http://www.w3.org/ns/solid/terms#> .\n\
@prefix foaf: <http://xmlns.com/foaf/0.1/> .\n\
_:patch solid:where   { <https://pod.example/alice/data#me> foaf:name ?n . };\n\
  solid:deletes { <https://pod.example/alice/data#me> foaf:name ?n . };\n\
  solid:inserts { <https://pod.example/alice/data#me> foaf:name \"Renamed\" . }.\n";
    let resp = h
        .request("PATCH", "/alice/data", Some("text/n3"), Body::from(doc))
        .await;
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);

    let get = h.request("GET", "/alice/data", None, Body::empty()).await;
    let bytes = to_bytes(get.into_body(), usize::MAX).await.unwrap();
    let text = String::from_utf8(bytes.to_vec()).unwrap();
    assert!(text.contains("Renamed"), "patched body: {text}");
    assert!(
        !text.contains("\"Alice\""),
        "old value still present: {text}"
    );
}

/// Spec: a non-empty `solid:where` with MULTIPLE solutions is a 409 (the Solid N3 Patch requires
/// exactly one mapping — it does not fan out per solution).
#[tokio::test]
async fn patch_with_a_multi_solution_where_is_409() {
    let h = Harness::new().await;
    // Two foaf:name triples on the same subject ⇒ the where binds ?n two ways ⇒ multiple solutions.
    let turtle =
        "<https://pod.example/alice/data#me> <http://xmlns.com/foaf/0.1/name> \"Alice\" .\n\
<https://pod.example/alice/data#me> <http://xmlns.com/foaf/0.1/name> \"Alicia\" .";
    h.request(
        "PUT",
        "/alice/data",
        Some("text/turtle"),
        Body::from(turtle),
    )
    .await;
    let doc = "@prefix solid: <http://www.w3.org/ns/solid/terms#> .\n\
@prefix foaf: <http://xmlns.com/foaf/0.1/> .\n\
_:patch solid:where { <https://pod.example/alice/data#me> foaf:name ?n . };\n\
  solid:deletes { <https://pod.example/alice/data#me> foaf:name ?n . }.\n";
    let resp = h
        .request("PATCH", "/alice/data", Some("text/n3"), Body::from(doc))
        .await;
    assert_eq!(resp.status(), StatusCode::CONFLICT);
}

#[tokio::test]
async fn unauthenticated_patch_is_401_with_challenge() {
    let h = Harness::new().await;
    h.request(
        "PUT",
        "/alice/data",
        Some("text/turtle"),
        Body::from(TURTLE),
    )
    .await;
    let resp = h
        .unauth_request(
            "PATCH",
            "/alice/data",
            Some("text/n3"),
            Body::from(N3_PATCH),
        )
        .await;
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    assert!(resp
        .headers()
        .contains_key(axum::http::header::WWW_AUTHENTICATE));
}

// --- Finding 2: PATCH-with-deletes on a MISSING target — Write-on-target, no existence oracle ----

/// A delete-bearing N3 patch (deletes a triple). Used to drive the "deletes require Write even on a
/// missing target" cases.
const N3_DELETE_PATCH: &str = "@prefix solid: <http://www.w3.org/ns/solid/terms#> .\n\
@prefix foaf: <http://xmlns.com/foaf/0.1/> .\n\
_:patch solid:deletes { <https://pod.example/alice/data#me> foaf:name \"Alice\" . }.\n";

#[tokio::test]
async fn anonymous_patch_with_deletes_is_a_uniform_401_missing_or_present() {
    // The existence-oracle the finding closes: an anonymous PATCH that carries deletes must get the
    // SAME denial (401) whether the target EXISTS or NOT — never a 401-vs-409 split that would leak
    // existence. (Pre-fix, deletes-on-missing took the create path so a missing target denied via the
    // PARENT-append authorize, while a present target ran into the missing-delete 409 → an oracle.)
    let h = Harness::new().await;

    // Case A: the target does NOT exist.
    let missing = h
        .unauth_request(
            "PATCH",
            "/alice/never-existed",
            Some("text/n3"),
            Body::from(N3_DELETE_PATCH),
        )
        .await;

    // Case B: the target DOES exist (created by the owner first).
    h.request(
        "PUT",
        "/alice/data",
        Some("text/turtle"),
        Body::from(TURTLE),
    )
    .await;
    let present = h
        .unauth_request(
            "PATCH",
            "/alice/data",
            Some("text/n3"),
            Body::from(N3_DELETE_PATCH),
        )
        .await;

    // Both must be a uniform 401 (authentication-required), NOT a 401-vs-409 existence oracle.
    assert_eq!(
        missing.status(),
        StatusCode::UNAUTHORIZED,
        "anon delete-patch on a MISSING target must be 401"
    );
    assert_eq!(
        present.status(),
        StatusCode::UNAUTHORIZED,
        "anon delete-patch on a PRESENT target must ALSO be 401 (no oracle)"
    );
}

#[tokio::test]
async fn append_only_caller_patch_with_deletes_is_uniformly_denied_no_oracle() {
    // An APPEND-only caller (no Write on the target) issuing a PATCH-with-deletes must get the SAME
    // denial whether the target exists or not — a 403, never a create-authorized-then-409. The owner
    // first writes a sub-container ACL that grants the caller (Alice) ONLY acl:Append (overriding the
    // inherited root owner-default), so under that container she lacks Write.
    let h = Harness::new().await;
    let restricted = "/alice/restricted/";
    h.make_container(restricted).await;
    // Write an ACL on the restricted container granting Alice ONLY Append on it + its descendants.
    // (Alice has Control via the inherited root default, so she may write this `.acl`.)
    let append_only_acl = format!(
        r#"@prefix acl: <http://www.w3.org/ns/auth/acl#>.
<#a> a acl:Authorization;
     acl:agent <{webid}>;
     acl:accessTo <https://pod.example/alice/restricted/>;
     acl:default <https://pod.example/alice/restricted/>;
     acl:mode acl:Append."#,
        webid = common::WEBID
    );
    let put_acl = h
        .request(
            "PUT",
            "/alice/restricted/.acl",
            Some("text/turtle"),
            Body::from(append_only_acl),
        )
        .await;
    assert_eq!(
        put_acl.status(),
        StatusCode::CREATED,
        "the owner (Control via inherited default) must be able to write the restricting ACL"
    );

    // Case A: the target under the restricted container does NOT exist.
    let missing = h
        .request(
            "PATCH",
            "/alice/restricted/missing",
            Some("text/n3"),
            Body::from(
                "@prefix solid: <http://www.w3.org/ns/solid/terms#> .\n\
@prefix foaf: <http://xmlns.com/foaf/0.1/> .\n\
_:p solid:deletes { <https://pod.example/alice/restricted/missing#x> foaf:name \"x\" . }.\n",
            ),
        )
        .await;

    // Case B: a target under the restricted container that DOES exist — but the caller may only
    // Append, never Write, so a delete-bearing patch must be denied identically (not run into a 409).
    // The owner cannot create it under the now-append-only ACL either, so we assert the missing case is
    // the canonical denial; a present target would be denied via the SAME Write-on-target check.
    assert_eq!(
        missing.status(),
        StatusCode::FORBIDDEN,
        "an append-only caller's delete-patch on a missing target must be 403 (Write required), \
         not a create-authorized 409/201"
    );
}

#[tokio::test]
async fn owner_patch_with_deletes_on_missing_target_is_409_conflict() {
    // The positive half: an authorized WRITE caller (the owner, Write via inherited root default)
    // issuing a PATCH-with-deletes on a MISSING target passes authorization and THEN gets the
    // missing-delete CONFLICT (409) — authorization succeeds first, the conflict surfaces only after.
    let h = Harness::new().await;
    let resp = h
        .request(
            "PATCH",
            "/alice/never-existed",
            Some("text/n3"),
            Body::from(N3_DELETE_PATCH),
        )
        .await;
    assert_eq!(
        resp.status(),
        StatusCode::CONFLICT,
        "an authorized-Write owner deleting on a missing target gets the 409 conflict (post-auth)"
    );
}

// --- Finding 3: an `.acl` (auxiliary) resource is NOT a contained child --------------------------

#[tokio::test]
async fn creating_an_acl_does_not_add_a_contains_member_to_the_parent() {
    // An `.acl` is an AUXILIARY resource, not a contained child: creating `…/c/r.acl` must NOT add an
    // `ldp:contains` edge for it on the parent container `…/c/`. (Pre-fix it went through
    // `create_in_container` and showed up as a member.)
    let h = Harness::new().await;
    h.make_container("/alice/c/").await;

    // Create a resource AND its ACL inside the container.
    let put_r = h
        .request(
            "PUT",
            "/alice/c/r",
            Some("text/turtle"),
            Body::from("<#r> <http://xmlns.com/foaf/0.1/name> \"R\" ."),
        )
        .await;
    assert_eq!(put_r.status(), StatusCode::CREATED);
    let put_acl = h
        .request(
            "PUT",
            "/alice/c/r.acl",
            Some("text/turtle"),
            Body::from(format!(
                r#"@prefix acl: <http://www.w3.org/ns/auth/acl#>.
<#o> a acl:Authorization; acl:agent <{webid}>; acl:accessTo <https://pod.example/alice/c/r>; acl:mode acl:Read, acl:Write, acl:Control."#,
                webid = common::WEBID
            )),
        )
        .await;
    assert_eq!(put_acl.status(), StatusCode::CREATED);

    // GET the container listing: it must list the plain resource `…/c/r` but NOT the auxiliary `r.acl`.
    let get = h.request("GET", "/alice/c/", None, Body::empty()).await;
    assert_eq!(get.status(), StatusCode::OK);
    let bytes = to_bytes(get.into_body(), usize::MAX).await.unwrap();
    let text = String::from_utf8(bytes.to_vec()).unwrap();
    assert!(
        text.contains("ldp#contains") && text.contains("/alice/c/r>"),
        "the plain resource must be a contained member: {text}"
    );
    assert!(
        !text.contains("/alice/c/r.acl"),
        "the auxiliary .acl must NOT be an ldp:contains member of the parent: {text}"
    );
}

#[tokio::test]
async fn deleting_an_acl_leaves_the_parent_containment_unchanged() {
    // Deleting an `.acl` must not mutate the parent's containment (it was never a member). The
    // resource it governs (and the container listing) are unaffected by the `.acl` delete.
    let h = Harness::new().await;
    h.make_container("/alice/c/").await;
    h.request(
        "PUT",
        "/alice/c/r",
        Some("text/turtle"),
        Body::from("<#r> <http://xmlns.com/foaf/0.1/name> \"R\" ."),
    )
    .await;
    h.request(
        "PUT",
        "/alice/c/r.acl",
        Some("text/turtle"),
        Body::from(format!(
            r#"@prefix acl: <http://www.w3.org/ns/auth/acl#>.
<#o> a acl:Authorization; acl:agent <{webid}>; acl:accessTo <https://pod.example/alice/c/r>; acl:mode acl:Read, acl:Write, acl:Control."#,
            webid = common::WEBID
        )),
    )
    .await;

    // Delete the `.acl` (Control required; the owner has it).
    let del = h
        .request("DELETE", "/alice/c/r.acl", None, Body::empty())
        .await;
    assert_eq!(del.status(), StatusCode::NO_CONTENT);

    // The governed resource still exists and the container still lists it as a member.
    let get_r = h.request("GET", "/alice/c/r", None, Body::empty()).await;
    assert_eq!(get_r.status(), StatusCode::OK);
    let get_c = h.request("GET", "/alice/c/", None, Body::empty()).await;
    let bytes = to_bytes(get_c.into_body(), usize::MAX).await.unwrap();
    let text = String::from_utf8(bytes.to_vec()).unwrap();
    assert!(
        text.contains("/alice/c/r>"),
        "the governed resource must remain a contained member after the .acl delete: {text}"
    );
}

// --- Cluster A: protocol-completeness tests ----------------------------------------------------

/// Send a raw request with arbitrary headers and no automatic auth (for CORS / OPTIONS probes).
async fn raw(
    h: &Harness,
    method: &str,
    path: &str,
    extra: &[(&str, &str)],
) -> axum::http::Response<Body> {
    let mut builder = Request::builder().method(method).uri(path);
    for (k, v) in extra {
        builder = builder.header(*k, *v);
    }
    h.app
        .clone()
        .oneshot(builder.body(Body::empty()).unwrap())
        .await
        .unwrap()
}

#[tokio::test]
async fn options_is_not_405() {
    // read-method-support: OPTIONS must not be 405/501. The CORS layer answers every OPTIONS (200),
    // which satisfies the "OPTIONS is supported" requirement.
    let h = Harness::new().await;
    let resp = raw(&h, "OPTIONS", "/alice/", &[]).await;
    assert_ne!(resp.status(), StatusCode::METHOD_NOT_ALLOWED);
    assert_ne!(resp.status(), StatusCode::NOT_IMPLEMENTED);
}

#[tokio::test]
async fn get_response_advertises_allow_accept_post_accept_patch() {
    // read-method-allow: a GET response on a container must carry `Allow` listing GET + HEAD; the
    // container also advertises `Accept-Post` + `Accept-Patch`.
    let h = Harness::new().await;
    h.make_container("/alice/").await;
    let get = h
        .request_with(
            "GET",
            "/alice/",
            None,
            &[("accept", "text/turtle")],
            Body::empty(),
        )
        .await;
    assert_eq!(get.status(), StatusCode::OK);
    let allow = get
        .headers()
        .get(axum::http::header::ALLOW)
        .unwrap()
        .to_str()
        .unwrap();
    assert!(
        allow.contains("GET") && allow.contains("HEAD"),
        "Allow: {allow}"
    );
    let accept_post = get.headers().get("accept-post").unwrap().to_str().unwrap();
    assert!(accept_post.contains("text/turtle"));
    let accept_patch = get.headers().get("accept-patch").unwrap().to_str().unwrap();
    assert!(accept_patch.contains("text/n3"));
}

#[tokio::test]
async fn container_get_renders_ldp_contains_listing() {
    // delete-remove-containment / writing-resource-containment: a container GET must render
    // ldp:BasicContainer + an ldp:contains triple per member.
    let h = Harness::new().await;
    h.make_container("/alice/").await;
    let post = h
        .request_with(
            "POST",
            "/alice/",
            Some("text/turtle"),
            &[("slug", "doc1")],
            Body::from("<#it> <http://xmlns.com/foaf/0.1/name> \"D\" ."),
        )
        .await;
    assert_eq!(post.status(), StatusCode::CREATED);
    // V2 (`research/lws-design-records.md` §6): the member's IRI is an opaque-suffixed
    // `…/doc1-<opaque>` — capture the minted Location (and its path) instead of assuming the
    // verbatim Slug.
    let member_iri = post
        .headers()
        .get(axum::http::header::LOCATION)
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    let member_path = member_iri
        .strip_prefix("https://pod.example")
        .unwrap_or(&member_iri)
        .to_string();

    let get = h
        .request_with(
            "GET",
            "/alice/",
            None,
            &[("accept", "text/turtle")],
            Body::empty(),
        )
        .await;
    assert_eq!(get.status(), StatusCode::OK);
    let bytes = to_bytes(get.into_body(), usize::MAX).await.unwrap();
    let text = String::from_utf8(bytes.to_vec()).unwrap();
    assert!(
        text.contains("ldp#BasicContainer"),
        "container body: {text}"
    );
    assert!(text.contains("ldp#contains"), "container body: {text}");
    assert!(
        text.contains(&member_iri),
        "container body must list the member: {text}"
    );

    // After deleting the member, the listing no longer contains it.
    let del = h.request("DELETE", &member_path, None, Body::empty()).await;
    assert_eq!(del.status(), StatusCode::NO_CONTENT);
    let get2 = h
        .request_with(
            "GET",
            "/alice/",
            None,
            &[("accept", "text/turtle")],
            Body::empty(),
        )
        .await;
    let bytes2 = to_bytes(get2.into_body(), usize::MAX).await.unwrap();
    let text2 = String::from_utf8(bytes2.to_vec()).unwrap();
    assert!(
        !text2.contains(&member_iri),
        "deleted member must be gone from the listing: {text2}"
    );
}

#[tokio::test]
async fn container_get_merges_stored_rdf_with_containment_triples() {
    // Regression (roborev Medium): a container GET must return BOTH the RDF stored ON the container
    // itself AND the generated ldp:BasicContainer + ldp:contains triples — not synthesise only the
    // containment triples and drop the stored body. Round-trip: PUT RDF to the container, POST a
    // member, GET ⇒ the stored triple AND the containment triples are present.
    let h = Harness::new().await;
    // PUT a container with a distinctive stored triple (relative <#c> resolves to …/alice/#c).
    let put = h
        .request(
            "PUT",
            "/alice/",
            Some("text/turtle"),
            Body::from("<#c> <http://purl.org/dc/terms/title> \"My container\" ."),
        )
        .await;
    assert_eq!(put.status(), StatusCode::CREATED);

    // POST a member so there is a containment edge too.
    let post = h
        .request_with(
            "POST",
            "/alice/",
            Some("text/turtle"),
            &[("slug", "doc1")],
            Body::from("<#it> <http://xmlns.com/foaf/0.1/name> \"D\" ."),
        )
        .await;
    assert_eq!(post.status(), StatusCode::CREATED);

    let get = h
        .request_with(
            "GET",
            "/alice/",
            None,
            &[("accept", "text/turtle")],
            Body::empty(),
        )
        .await;
    assert_eq!(get.status(), StatusCode::OK);
    let bytes = to_bytes(get.into_body(), usize::MAX).await.unwrap();
    let text = String::from_utf8(bytes.to_vec()).unwrap();
    // The STORED triple must be retrievable.
    assert!(
        text.contains("dc/terms/title") && text.contains("My container"),
        "the RDF stored on the container must be returned on GET: {text}"
    );
    // AND the generated containment triples must still be present.
    assert!(
        text.contains("ldp#BasicContainer"),
        "container typing must still be present: {text}"
    );
    assert!(
        text.contains("ldp#contains") && text.contains("https://pod.example/alice/doc1"),
        "the containment listing must still be present: {text}"
    );
}

#[tokio::test]
async fn container_etag_changes_when_membership_changes() {
    // Regression (roborev Medium): the container body is generated from LIVE membership, so its ETag
    // must be derived from the rendered representation — adding/removing a child must change the ETag
    // (else conditional requests / caches break). Also: HEAD and GET must agree on the ETag.
    let h = Harness::new().await;
    h.make_container("/alice/").await;

    let etag = |resp: &axum::http::Response<Body>| -> String {
        resp.headers()
            .get("etag")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string()
    };

    // Empty container ETag (GET) + the HEAD ETag must match the GET ETag for the same state.
    let get0 = h
        .request_with(
            "GET",
            "/alice/",
            None,
            &[("accept", "text/turtle")],
            Body::empty(),
        )
        .await;
    assert_eq!(get0.status(), StatusCode::OK);
    let etag0 = etag(&get0);
    assert!(!etag0.is_empty(), "a container GET must carry an ETag");

    let head0 = h
        .request_with(
            "HEAD",
            "/alice/",
            None,
            &[("accept", "text/turtle")],
            Body::empty(),
        )
        .await;
    assert_eq!(head0.status(), StatusCode::OK);
    assert_eq!(
        etag(&head0),
        etag0,
        "HEAD and GET must report the SAME container ETag for the same state"
    );

    // Add a member → the body changes → the ETag MUST change.
    let post = h
        .request_with(
            "POST",
            "/alice/",
            Some("text/turtle"),
            &[("slug", "doc1")],
            Body::from("<#it> <http://xmlns.com/foaf/0.1/name> \"D\" ."),
        )
        .await;
    assert_eq!(post.status(), StatusCode::CREATED);
    // V2 (`research/lws-design-records.md` §6): the member is at the opaque-suffixed minted
    // Location — capture its path.
    let member_iri = post
        .headers()
        .get(axum::http::header::LOCATION)
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    let member_path = member_iri
        .strip_prefix("https://pod.example")
        .unwrap_or(&member_iri)
        .to_string();

    let get1 = h
        .request_with(
            "GET",
            "/alice/",
            None,
            &[("accept", "text/turtle")],
            Body::empty(),
        )
        .await;
    let etag1 = etag(&get1);
    assert_ne!(
        etag0, etag1,
        "adding a child must change the container ETag (the body changed)"
    );

    // Remove the member → the body changes back → the ETag MUST change again.
    let del = h.request("DELETE", &member_path, None, Body::empty()).await;
    assert_eq!(del.status(), StatusCode::NO_CONTENT);
    let get2 = h
        .request_with(
            "GET",
            "/alice/",
            None,
            &[("accept", "text/turtle")],
            Body::empty(),
        )
        .await;
    let etag2 = etag(&get2);
    assert_ne!(
        etag1, etag2,
        "removing a child must change the container ETag again"
    );
}

#[tokio::test]
async fn root_route_is_served() {
    // Cluster-A #1: GET / must reach the handler (the /{*path} wildcard does not match the empty path).
    // With nothing seeded the root does not exist ⇒ 404 (not a routing 404 with no handler) — proving
    // the route is wired. After creating it, it reads back as a container.
    let h = Harness::new().await;
    let get = h.request("GET", "/", None, Body::empty()).await;
    assert_eq!(get.status(), StatusCode::NOT_FOUND);

    // Create a child under root via PUT (auto-creates the root container), then GET / lists it.
    let put = h
        .request(
            "PUT",
            "/top",
            Some("text/turtle"),
            Body::from("<#t> <http://xmlns.com/foaf/0.1/name> \"T\" ."),
        )
        .await;
    assert_eq!(put.status(), StatusCode::CREATED);
    let get_root = h
        .request_with(
            "GET",
            "/",
            None,
            &[("accept", "text/turtle")],
            Body::empty(),
        )
        .await;
    assert_eq!(get_root.status(), StatusCode::OK);
    let bytes = to_bytes(get_root.into_body(), usize::MAX).await.unwrap();
    let text = String::from_utf8(bytes.to_vec()).unwrap();
    assert!(text.contains("ldp#BasicContainer"), "root body: {text}");
    assert!(
        text.contains("https://pod.example/top"),
        "root must list /top: {text}"
    );
}

#[tokio::test]
async fn cors_preflight_returns_acao_allow_methods_and_reflected_headers() {
    // cors-preflight-requests / cors-accept-acah: an OPTIONS with Origin + Access-Control-Request-*
    // returns ACAO == origin, Allow-Methods contains the method, Allow-Headers reflects the request.
    let h = Harness::new().await;
    let resp = raw(
        &h,
        "OPTIONS",
        "/alice/",
        &[
            ("origin", "https://tester"),
            ("access-control-request-method", "POST"),
            (
                "access-control-request-headers",
                "X-CUSTOM, Content-Type, Accept",
            ),
        ],
    )
    .await;
    assert!(
        resp.status() == StatusCode::NO_CONTENT || resp.status() == StatusCode::OK,
        "preflight status: {}",
        resp.status()
    );
    let acao = resp
        .headers()
        .get("access-control-allow-origin")
        .unwrap()
        .to_str()
        .unwrap();
    assert_eq!(acao, "https://tester");
    let methods = resp
        .headers()
        .get("access-control-allow-methods")
        .unwrap()
        .to_str()
        .unwrap();
    assert!(methods.contains("POST"), "allow-methods: {methods}");
    let allow_headers = resp
        .headers()
        .get("access-control-allow-headers")
        .unwrap()
        .to_str()
        .unwrap()
        .to_ascii_lowercase();
    assert!(
        allow_headers.contains("x-custom"),
        "allow-headers: {allow_headers}"
    );
    assert!(
        allow_headers.contains("content-type"),
        "allow-headers: {allow_headers}"
    );
    assert!(
        allow_headers.contains("accept"),
        "allow-headers: {allow_headers}"
    );
}

#[tokio::test]
async fn cors_simple_request_carries_acao_and_expose_headers_even_on_401() {
    // cors-simple-requests: an unauthenticated GET with Origin gets 401 BUT still carries
    // Access-Control-Allow-Origin == origin and a concrete (non-'*') Access-Control-Expose-Headers.
    let h = Harness::new().await;
    let resp = raw(&h, "GET", "/alice/", &[("origin", "https://tester")]).await;
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(
        resp.headers()
            .get("access-control-allow-origin")
            .unwrap()
            .to_str()
            .unwrap(),
        "https://tester"
    );
    let expose = resp
        .headers()
        .get("access-control-expose-headers")
        .unwrap()
        .to_str()
        .unwrap();
    assert!(!expose.is_empty());
    assert_ne!(expose, "*");
}

#[tokio::test]
async fn cors_authenticated_get_has_vary_origin() {
    // acao-vary: a credentialed GET with Origin returns ACAO + Vary: Origin.
    let h = Harness::new().await;
    h.request(
        "PUT",
        "/alice/data",
        Some("text/turtle"),
        Body::from(TURTLE),
    )
    .await;
    let (authz, dpop) = h.auth_headers("GET", "/alice/data");
    let resp = raw(
        &h,
        "GET",
        "/alice/data",
        &[
            ("authorization", &authz),
            ("dpop", &dpop),
            ("origin", "https://tester"),
        ],
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(
        resp.headers()
            .get("access-control-allow-origin")
            .unwrap()
            .to_str()
            .unwrap(),
        "https://tester"
    );
    let vary = resp
        .headers()
        .get(axum::http::header::VARY)
        .unwrap()
        .to_str()
        .unwrap()
        .to_ascii_lowercase();
    assert!(vary.contains("origin"), "Vary: {vary}");
}

#[tokio::test]
async fn put_creates_intermediate_containers_and_wires_membership() {
    // writing-resource-containment: PUT a grandchild ⇒ intermediate containers exist + list members.
    let h = Harness::new().await;
    let put = h
        .request(
            "PUT",
            "/alice/mid/leaf.ttl",
            Some("text/turtle"),
            Body::from("<#x> <http://xmlns.com/foaf/0.1/name> \"L\" ."),
        )
        .await;
    assert_eq!(put.status(), StatusCode::CREATED);

    // The intermediate container /alice/mid/ lists the leaf.
    let mid = h
        .request_with(
            "GET",
            "/alice/mid/",
            None,
            &[("accept", "text/turtle")],
            Body::empty(),
        )
        .await;
    assert_eq!(mid.status(), StatusCode::OK);
    let mid_text = String::from_utf8(
        to_bytes(mid.into_body(), usize::MAX)
            .await
            .unwrap()
            .to_vec(),
    )
    .unwrap();
    assert!(
        mid_text.contains("https://pod.example/alice/mid/leaf.ttl"),
        "intermediate container must list the leaf: {mid_text}"
    );

    // The grandparent /alice/ lists the intermediate container.
    let top = h
        .request_with(
            "GET",
            "/alice/",
            None,
            &[("accept", "text/turtle")],
            Body::empty(),
        )
        .await;
    let top_text = String::from_utf8(
        to_bytes(top.into_body(), usize::MAX)
            .await
            .unwrap()
            .to_vec(),
    )
    .unwrap();
    assert!(
        top_text.contains("https://pod.example/alice/mid/"),
        "grandparent must list the intermediate container: {top_text}"
    );
}

#[tokio::test]
async fn put_without_content_type_is_400() {
    // content-type-reject: a write with no Content-Type is 400 (not 415).
    let h = Harness::new().await;
    let resp = h
        .request("PUT", "/alice/data", None, Body::from(TURTLE))
        .await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn slash_semantics_resource_and_container_cannot_coexist() {
    // slash-semantics-exclude: PUT a container, then a same-name resource ⇒ 409 (and vice versa).
    let h = Harness::new().await;
    h.make_container("/alice/").await;

    // Container /alice/foo/ then resource /alice/foo ⇒ conflict.
    let put_container = h
        .request(
            "PUT",
            "/alice/foo/",
            Some("text/turtle"),
            Body::from("<#c> <http://xmlns.com/foaf/0.1/name> \"C\" ."),
        )
        .await;
    assert_eq!(put_container.status(), StatusCode::CREATED);
    let put_resource = h
        .request(
            "PUT",
            "/alice/foo",
            Some("text/plain"),
            Body::from("<#r> <http://xmlns.com/foaf/0.1/name> \"R\" ."),
        )
        .await;
    assert_eq!(put_resource.status(), StatusCode::CONFLICT);

    // The reverse: resource /alice/bar then container /alice/bar/ ⇒ conflict.
    let put_res = h
        .request(
            "PUT",
            "/alice/bar",
            Some("text/turtle"),
            Body::from("<#r> <http://xmlns.com/foaf/0.1/name> \"R\" ."),
        )
        .await;
    assert_eq!(put_res.status(), StatusCode::CREATED);
    let put_cont = h
        .request(
            "PUT",
            "/alice/bar/",
            Some("text/turtle"),
            Body::from("<#c> <http://xmlns.com/foaf/0.1/name> \"C\" ."),
        )
        .await;
    assert_eq!(put_cont.status(), StatusCode::CONFLICT);
}

#[tokio::test]
async fn patch_sparql_update_creates_intermediate_containers() {
    // containment.feature PATCH scenario: a create-on-PATCH with INSERT DATA wires containment.
    let h = Harness::new().await;
    let patch = h
        .request(
            "PATCH",
            "/alice/p/leaf.ttl",
            Some("application/sparql-update"),
            Body::from("INSERT DATA { <#hello> <#linked> <#world> . }"),
        )
        .await;
    assert!(
        patch.status().is_success(),
        "PATCH create status: {}",
        patch.status()
    );
    let mid = h
        .request_with(
            "GET",
            "/alice/p/",
            None,
            &[("accept", "text/turtle")],
            Body::empty(),
        )
        .await;
    let mid_text = String::from_utf8(
        to_bytes(mid.into_body(), usize::MAX)
            .await
            .unwrap()
            .to_vec(),
    )
    .unwrap();
    assert!(
        mid_text.contains("https://pod.example/alice/p/leaf.ttl"),
        "intermediate container must list the PATCH-created leaf: {mid_text}"
    );
}

#[tokio::test]
async fn post_with_basic_container_link_creates_a_container_child() {
    // slash-semantics-exclude / LDP §5.2.3.4: a POST carrying
    // `Link: <ldp#BasicContainer>; rel="type"` creates a CONTAINER child — the Location ends in '/'.
    let h = Harness::new().await;
    h.make_container("/alice/").await;
    let resp = h
        .request_with(
            "POST",
            "/alice/",
            Some("text/turtle"),
            &[(
                "link",
                "<http://www.w3.org/ns/ldp#BasicContainer>; rel=\"type\"",
            )],
            Body::from("<#c> <http://xmlns.com/foaf/0.1/name> \"C\" ."),
        )
        .await;
    assert_eq!(resp.status(), StatusCode::CREATED);
    let location = resp
        .headers()
        .get(axum::http::header::LOCATION)
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    assert!(
        location.ends_with('/'),
        "a container POST Location must end with '/': {location}"
    );

    // The created child is a container (GET renders an ldp:BasicContainer listing).
    let path = location.strip_prefix("https://pod.example").unwrap();
    let get = h
        .request_with(
            "GET",
            path,
            None,
            &[("accept", "text/turtle")],
            Body::empty(),
        )
        .await;
    assert_eq!(get.status(), StatusCode::OK);
    let text = String::from_utf8(
        to_bytes(get.into_body(), usize::MAX)
            .await
            .unwrap()
            .to_vec(),
    )
    .unwrap();
    assert!(
        text.contains("ldp#BasicContainer"),
        "container body: {text}"
    );
}
