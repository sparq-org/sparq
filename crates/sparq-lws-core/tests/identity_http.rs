// AUTHORED-BY Claude Fable 5
//! End-to-end tests of the identity host (provider WebIDs OUTSIDE the pod —
//! `research/lws-design-records.md` §4) through the assembled router:
//!
//! - the id-doc is served on GET/HEAD (Turtle/JSON-LD conneg, ETag/304, public cache, `ACAO: *`)
//!   with **no auth and no WAC** — no `WWW-Authenticate`, no `.acl` Link, no `WAC-Allow`;
//! - every non-read method on the id host is a `405`;
//! - every other path shape on the id host is a fail-closed `404`;
//! - the LDP surface REFUSES the reserved `/.identity/**` namespace — 404 for every method,
//!   anonymous AND credentialed (the refusal runs BEFORE auth, so an anonymous write gets 404,
//!   never a 401 challenge), %-encoded forms too, and REGARDLESS of whether the identity feature
//!   is enabled (flag-independent).

mod common;

use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use common::{jwks_provider, mint_access_token, mint_dpop_proof, KeyKit, BASE_URL};
use oxrdf::NamedNode;
use solid_oidc_verifier::config::VerifierConfig;
use solid_oidc_verifier::replay::InMemoryReplayStore;
use solid_oidc_verifier::verifier::Verifier;
use sparq_lws_core::app::{build_router, AppState};
use sparq_lws_core::auth::AuthContext;
use sparq_lws_core::identity::IdentityConfig;
use sparq_lws_core::ldp::content::{parse_to_triples, RdfFormat};
use sparq_lws_core::ldp::handler::LdpState;
use sparq_lws_core::seed::{seed_conformance, seed_conformance_with_identity};
use sparq_lws_core::store::{CompositeStore, InMemoryBlobStore, InMemorySparqClient};
use tower::ServiceExt;

/// The identity host derived from `BASE_URL` (`https://pod.example`).
const ID_HOST: &str = "id.pod.example";

/// One assembled app in IDENTITY mode (id-host serving ON, the identity conformance seed run),
/// plus the keys to mint credentialed requests.
struct Harness {
    app: axum::Router,
    issuer_key: KeyKit,
    client_key: KeyKit,
}

impl Harness {
    async fn new(identity: bool) -> Self {
        let issuer_key = KeyKit::generate();
        let client_key = KeyKit::generate();
        let config = VerifierConfig::new(vec![common::ISSUER.to_string()], BASE_URL);
        let replay = InMemoryReplayStore::with_window(config.replay_ttl());
        let verifier = Verifier::new(config, jwks_provider(&issuer_key), replay).unwrap();
        let ctx = AuthContext::new(verifier, BASE_URL);
        let store = CompositeStore::new(InMemorySparqClient::new(), InMemoryBlobStore::new());

        let id_config = IdentityConfig::new(BASE_URL, None).unwrap();
        assert_eq!(id_config.host(), ID_HOST);
        if identity {
            seed_conformance_with_identity(&store, BASE_URL, common::ISSUER, Some(&id_config))
                .await
                .unwrap();
        } else {
            seed_conformance(&store, BASE_URL, common::ISSUER)
                .await
                .unwrap();
        }

        let ldp = LdpState::new(store, BASE_URL);
        let mut state = AppState::new(ctx, ldp);
        if identity {
            state = state.with_identity(id_config);
        }
        Self {
            app: build_router(state),
            issuer_key,
            client_key,
        }
    }

    /// A fresh `(Authorization, DPoP)` pair for one credentialed request (new proof jti each call).
    fn auth_headers(&self, method: &str, path: &str) -> (String, String) {
        let access = mint_access_token(&self.issuer_key, &self.client_key.thumbprint);
        let htu = format!("{BASE_URL}{path}");
        let proof = mint_dpop_proof(&self.client_key, method, &htu, &access);
        (format!("DPoP {access}"), proof)
    }

    /// An ANONYMOUS request with an explicit `Host` header (the id-host / main-host selector).
    async fn anon(
        &self,
        method: &str,
        host: &str,
        path: &str,
        extra: &[(&str, &str)],
    ) -> axum::http::Response<Body> {
        let mut req = Request::builder()
            .method(method)
            .uri(path)
            .header("host", host);
        for (k, v) in extra {
            req = req.header(*k, *v);
        }
        self.app
            .clone()
            .oneshot(req.body(Body::empty()).unwrap())
            .await
            .unwrap()
    }

    /// A CREDENTIALED (DPoP-bound) request against the MAIN host.
    async fn authed(&self, method: &str, path: &str) -> axum::http::Response<Body> {
        let (authz, proof) = self.auth_headers(method, path);
        let req = Request::builder()
            .method(method)
            .uri(path)
            .header("host", "pod.example")
            .header("authorization", authz)
            .header("dpop", proof)
            .header("content-type", "text/turtle")
            .body(Body::from(
                "<https://pod.example/x> <https://pod.example/p> \"v\" .",
            ))
            .unwrap();
        self.app.clone().oneshot(req).await.unwrap()
    }
}

async fn body_string(resp: axum::http::Response<Body>) -> String {
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    String::from_utf8(bytes.to_vec()).unwrap()
}

// --- The id-host serving contract -------------------------------------------------------------

#[tokio::test]
async fn id_doc_is_served_anonymously_on_get_with_no_wac_and_no_auth_surface() {
    let h = Harness::new(true).await;
    let resp = h.anon("GET", ID_HOST, "/alice", &[]).await;
    assert_eq!(resp.status(), StatusCode::OK);

    let headers = resp.headers().clone();
    // The positive contract: Turtle, ETag, public cache, ACAO *, Vary: Accept.
    assert_eq!(headers.get("content-type").unwrap(), "text/turtle");
    assert!(headers.get("etag").is_some(), "id-doc must carry an ETag");
    assert_eq!(headers.get("cache-control").unwrap(), "public, max-age=300");
    assert_eq!(headers.get("access-control-allow-origin").unwrap(), "*");
    assert_eq!(headers.get("vary").unwrap(), "accept");
    // The NEGATIVE contract (the no-WAC path): no auth challenge, no .acl Link, no WAC-Allow.
    assert!(headers.get("www-authenticate").is_none());
    assert!(headers.get("link").is_none());
    assert!(headers.get("wac-allow").is_none());

    // The LOCKED statements, with subjects on the IDENTITY origin.
    let body = body_string(resp).await;
    assert!(body.contains("https://id.pod.example/alice#me"));
    assert!(body.contains("solid/terms#oidcIssuer"));
    assert!(body.contains(common::ISSUER));
    assert!(body.contains("pim/space#storage"));
    assert!(body.contains("https://pod.example/alice/"));
    assert!(body.contains("solid/terms#owner"));
}

#[tokio::test]
async fn id_doc_head_serves_headers_with_empty_body() {
    let h = Harness::new(true).await;
    let resp = h.anon("HEAD", ID_HOST, "/alice", &[]).await;
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(resp.headers().get("content-type").unwrap(), "text/turtle");
    assert!(resp.headers().get("etag").is_some());
    assert!(resp.headers().get("www-authenticate").is_none());
    let body = body_string(resp).await;
    assert!(body.is_empty(), "HEAD must carry no body");
}

#[tokio::test]
async fn id_doc_negotiates_jsonld() {
    let h = Harness::new(true).await;
    let resp = h
        .anon(
            "GET",
            ID_HOST,
            "/alice",
            &[("accept", "application/ld+json")],
        )
        .await;
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(
        resp.headers().get("content-type").unwrap(),
        "application/ld+json"
    );
    let body = body_string(resp).await;
    assert!(body.contains("https://id.pod.example/alice#me"));
    assert!(body.contains("oidcIssuer"));
}

#[tokio::test]
async fn id_doc_unknown_accept_falls_back_to_turtle() {
    // An Accept naming no producible type degrades to the Solid default (text/turtle) — the
    // id-doc read never 406s over an exotic Accept.
    let h = Harness::new(true).await;
    let resp = h
        .anon("GET", ID_HOST, "/alice", &[("accept", "text/html")])
        .await;
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(resp.headers().get("content-type").unwrap(), "text/turtle");
}

#[tokio::test]
async fn id_doc_explicit_q_zero_refusal_is_406() {
    // The only remaining 406: the client explicitly refused (q=0) every producible type.
    let h = Harness::new(true).await;
    let resp = h
        .anon(
            "GET",
            ID_HOST,
            "/alice",
            &[("accept", "text/turtle;q=0, application/ld+json;q=0")],
        )
        .await;
    assert_eq!(resp.status(), StatusCode::NOT_ACCEPTABLE);
}

#[tokio::test]
async fn id_doc_if_none_match_answers_304() {
    let h = Harness::new(true).await;
    let first = h.anon("GET", ID_HOST, "/alice", &[]).await;
    let etag = first
        .headers()
        .get("etag")
        .unwrap()
        .to_str()
        .unwrap()
        .to_owned();
    let resp = h
        .anon("GET", ID_HOST, "/alice", &[("if-none-match", &etag)])
        .await;
    assert_eq!(resp.status(), StatusCode::NOT_MODIFIED);
    assert_eq!(resp.headers().get("etag").unwrap().to_str().unwrap(), etag);
}

#[tokio::test]
async fn id_doc_if_none_match_is_representation_specific_no_cross_repr_304() {
    // Finding 1 (Medium): a client that cached the TURTLE representation and then requests JSON-LD
    // carrying the Turtle ETag must get a fresh 200 — NEVER a 304 for a representation it never
    // received. The variant tags are distinct, and Vary: Accept keeps caches from conflating them.
    let h = Harness::new(true).await;

    // The Turtle representation's strong tag.
    let turtle = h.anon("GET", ID_HOST, "/alice", &[]).await;
    assert_eq!(turtle.status(), StatusCode::OK);
    let turtle_etag = turtle
        .headers()
        .get("etag")
        .unwrap()
        .to_str()
        .unwrap()
        .to_owned();

    // The JSON-LD representation's tag is DISTINCT from the Turtle one.
    let jsonld = h
        .anon(
            "GET",
            ID_HOST,
            "/alice",
            &[("accept", "application/ld+json")],
        )
        .await;
    assert_eq!(jsonld.status(), StatusCode::OK);
    let jsonld_etag = jsonld
        .headers()
        .get("etag")
        .unwrap()
        .to_str()
        .unwrap()
        .to_owned();
    assert_ne!(
        turtle_etag, jsonld_etag,
        "the JSON-LD representation must carry a DISTINCT ETag from Turtle"
    );

    // The CROSS-representation conditional: JSON-LD requested with the TURTLE tag ⇒ fresh 200.
    let resp = h
        .anon(
            "GET",
            ID_HOST,
            "/alice",
            &[
                ("accept", "application/ld+json"),
                ("if-none-match", &turtle_etag),
            ],
        )
        .await;
    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "a Turtle ETag must NOT 304 a JSON-LD request (no cross-representation 304)"
    );

    // The SAME-representation conditional still 304s (JSON-LD tag + JSON-LD request).
    let resp = h
        .anon(
            "GET",
            ID_HOST,
            "/alice",
            &[
                ("accept", "application/ld+json"),
                ("if-none-match", &jsonld_etag),
            ],
        )
        .await;
    assert_eq!(
        resp.status(),
        StatusCode::NOT_MODIFIED,
        "the matching JSON-LD ETag must still 304"
    );
    assert_eq!(
        resp.headers().get("etag").unwrap().to_str().unwrap(),
        jsonld_etag
    );
}

#[tokio::test]
async fn id_doc_demoted_card_is_an_honest_extension_of_the_id_host_webid() {
    // Finding 2 (Low): the demoted in-pod card must extend the id-host WebID (owl:sameAs), not
    // assert a separate legacy person — so the id-doc's rdfs:seeAlso link is honest. Asserted as
    // EXACT parsed triples (not substrings), so a regression that points `foaf:primaryTopic` back
    // at the in-pod `<card#me>` — re-asserting a competing profile — fails this test.
    let h = Harness::new(true).await;
    let resp = h
        .anon("GET", "pod.example", "/alice/profile/card", &[])
        .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_string(resp).await;

    const CARD: &str = "https://pod.example/alice/profile/card";
    const CARD_ME: &str = "https://pod.example/alice/profile/card#me";
    const ID_WEBID: &str = "https://id.pod.example/alice#me";
    const FOAF_PRIMARY_TOPIC: &str = "http://xmlns.com/foaf/0.1/primaryTopic";
    const OWL_SAME_AS: &str = "http://www.w3.org/2002/07/owl#sameAs";

    let triples = parse_to_triples(RdfFormat::Turtle, body.as_bytes(), CARD)
        .expect("the demoted card must parse as Turtle");
    let nn = |s: &str| NamedNode::new(s).unwrap();
    let has = |s: &str, p: &str, o: &str| {
        triples
            .iter()
            .any(|t| t.subject == nn(s).into() && t.predicate == nn(p) && t.object == nn(o).into())
    };
    assert!(
        has(CARD, FOAF_PRIMARY_TOPIC, ID_WEBID),
        "the demoted card's foaf:primaryTopic must be the id-host WebID: {body}"
    );
    assert!(
        has(CARD_ME, OWL_SAME_AS, ID_WEBID),
        "the demoted card must tie the legacy IRI to the id-host WebID via owl:sameAs: {body}"
    );
    // The regression guard: no foaf:primaryTopic triple may target the demoted in-pod `<card#me>`
    // (from ANY subject) — that would restore the card as a competing profile document.
    assert!(
        !triples
            .iter()
            .any(|t| t.predicate == nn(FOAF_PRIMARY_TOPIC) && t.object == nn(CARD_ME).into()),
        "foaf:primaryTopic must NOT target the demoted in-pod <card#me>: {body}"
    );
}

#[tokio::test]
async fn id_host_write_methods_are_405_with_allow_get_head() {
    let h = Harness::new(true).await;
    for method in ["PUT", "POST", "DELETE", "PATCH", "OPTIONS"] {
        let resp = h.anon(method, ID_HOST, "/alice", &[]).await;
        assert_eq!(
            resp.status(),
            StatusCode::METHOD_NOT_ALLOWED,
            "{method} on the id host must be 405 (GET/HEAD-only surface)"
        );
        assert_eq!(resp.headers().get("allow").unwrap(), "GET, HEAD");
        // Still no auth surface — a write is refused, never challenged.
        assert!(resp.headers().get("www-authenticate").is_none());
    }
}

#[tokio::test]
async fn id_host_non_handle_paths_are_fail_closed_404() {
    let h = Harness::new(true).await;
    for path in [
        "/",                  // the root is not a handle
        "/alice/",            // trailing slash — not a handle
        "/alice/profile",     // nested — not a handle
        "/Alice",             // uppercase — outside the grammar
        "/alice.card",        // dot — outside the grammar (the namespace can never be shadowed)
        "/%61lice",           // percent form — only the canonical spelling addresses an id-doc
        "/identity",          // reserved handle
        "/nobody",            // valid shape, no such user
        "/.well-known/solid", // the LDP discovery doc is NOT served on the id host
    ] {
        let resp = h.anon("GET", ID_HOST, path, &[]).await;
        assert_eq!(
            resp.status(),
            StatusCode::NOT_FOUND,
            "GET {path} on the id host must be a fail-closed 404"
        );
    }
}

#[tokio::test]
async fn health_probes_stay_reachable_outside_the_identity_gate() {
    // /livez and /readyz are mounted OUTSIDE the gate (deliberately overload- and gate-exempt), so
    // they answer on ANY Host — which is why "livez"/"readyz" are reserved handle names.
    let h = Harness::new(true).await;
    let resp = h.anon("GET", ID_HOST, "/livez", &[]).await;
    assert_eq!(resp.status(), StatusCode::OK);
}

// --- The LDP-surface refusal of the reserved namespace ----------------------------------------

#[tokio::test]
async fn ldp_surface_refuses_reserved_namespace_for_every_method_anonymously() {
    let h = Harness::new(true).await;
    // The id-doc EXISTS in the store (seeded) — yet the LDP surface must refuse to address it:
    // 404 for every method, and for an anonymous WRITE the answer is STILL 404, never a 401
    // challenge (the refusal runs BEFORE auth — nothing here is worth authenticating for).
    for method in ["GET", "HEAD", "PUT", "POST", "DELETE", "PATCH", "OPTIONS"] {
        let resp = h.anon(method, "pod.example", "/.identity/alice", &[]).await;
        assert_eq!(
            resp.status(),
            StatusCode::NOT_FOUND,
            "{method} /.identity/alice on the MAIN host must be 404"
        );
        assert!(
            resp.headers().get("www-authenticate").is_none(),
            "{method}: the refusal must not emit an auth challenge"
        );
    }
}

#[tokio::test]
async fn ldp_surface_refuses_reserved_namespace_for_credentialed_requests_too() {
    let h = Harness::new(true).await;
    // A fully-credentialed (DPoP-bound) write into the namespace — incl. the `.acl` that must
    // never exist — is refused identically: 404, not 401/403/201.
    for path in [
        "/.identity/alice",
        "/.identity/alice.acl",
        "/.identity/newuser",
    ] {
        let resp = h.authed("PUT", path).await;
        assert_eq!(
            resp.status(),
            StatusCode::NOT_FOUND,
            "credentialed PUT {path} must be refused with 404"
        );
    }
    let resp = h.authed("GET", "/.identity/alice").await;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn ldp_surface_refuses_percent_encoded_reserved_forms() {
    let h = Harness::new(true).await;
    for path in [
        "/%2Eidentity/alice",
        "/%2e%69dentity/alice",
        "/.identity%2Falice",
        "/.IDENTITY/alice",
    ] {
        let resp = h.anon("GET", "pod.example", path, &[]).await;
        assert_eq!(
            resp.status(),
            StatusCode::NOT_FOUND,
            "GET {path} must be refused (%-decoded / case-insensitive match)"
        );
    }
}

#[tokio::test]
async fn reserved_namespace_refusal_is_flag_independent() {
    // A server WITHOUT identity serving (the default posture) must STILL refuse the namespace —
    // otherwise pre-seeded documents could be reached (and `.acl`-ed) before the flag turns on.
    let h = Harness::new(false).await;
    for method in ["GET", "PUT", "DELETE"] {
        let resp = h.anon(method, "pod.example", "/.identity/alice", &[]).await;
        assert_eq!(
            resp.status(),
            StatusCode::NOT_FOUND,
            "{method} /.identity/alice must be 404 even with identity serving OFF"
        );
    }
    // …while the normal LDP surface is untouched: the (non-demoted) WebID card serves publicly.
    let resp = h
        .anon("GET", "pod.example", "/alice/profile/card", &[])
        .await;
    assert_eq!(resp.status(), StatusCode::OK);
    // And the id host is NOT special without the flag: the request falls through to the normal
    // stack, where `/alice` (no public grant, anonymous) is a 401 challenge — proving the gate
    // served nothing.
    let resp = h.anon("GET", ID_HOST, "/alice", &[]).await;
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

// --- Identity mode end-to-end coherence --------------------------------------------------------

#[tokio::test]
async fn identity_mode_demotes_the_in_pod_card_on_the_ldp_surface() {
    let h = Harness::new(true).await;
    // The demoted card still serves publicly on the MAIN host (it is a normal LDP resource)…
    let resp = h
        .anon("GET", "pod.example", "/alice/profile/card", &[])
        .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_string(resp).await;
    // …but carries NOTHING security-bearing: the issuer + storage statements live ONLY in the
    // locked id-doc on the id host.
    assert!(!body.contains("oidcIssuer"));
    assert!(!body.contains("pim/space#storage"));
}
