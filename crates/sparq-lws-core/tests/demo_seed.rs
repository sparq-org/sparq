// AUTHORED-BY Claude Fable 5
//! End-to-end HTTP tests for the opt-in demo playground seed (`SOLID_SERVER_SEED_DEMO` — the §3.2
//! public-demo posture of `research/lws-demo-architecture.md`, sq-5ougp).
//!
//! The seeded posture under test: a shared root-level `/playground/` container any AUTHENTICATED
//! agent can Read/Write/Append (via `acl:agentClass acl:AuthenticatedAgent` + `acl:default`
//! inheritance), publicly readable, with NO `acl:Control` granted to anyone; plus a public-read
//! `/README` Turtle document carrying the ephemeral-demo banner. Flag-off (unseeded) boots stay
//! byte-identical fail-closed: no `/playground/`, no ACLs, everything denied.
//!
//! Each request carries a fresh, well-formed DPoP-bound token + a per-request proof (a new jti) so
//! the verifier's single-use replay protection never rejects a follow-up request.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use common::{
    jwks_provider, mint_access_token_for, mint_dpop_proof, KeyKit, BASE_URL, WEBID,
};

/// A SECOND, DIFFERENT authenticated visitor — the shared playground's disclosed
/// cross-visitor overwrite/delete is only demonstrable with two distinct WebIDs.
const OTHER_WEBID: &str = "https://pod.example/mallory/profile/card#me";
use solid_oidc_verifier::config::VerifierConfig;
use solid_oidc_verifier::replay::InMemoryReplayStore;
use solid_oidc_verifier::verifier::Verifier;
use sparq_lws_core::app::{build_router, AppState};
use sparq_lws_core::auth::AuthContext;
use sparq_lws_core::ldp::handler::LdpState;
use sparq_lws_core::store::{CompositeStore, InMemoryBlobStore};
use tower::ServiceExt;

const TURTLE: &str =
    "<https://pod.example/playground/note#it> <http://xmlns.com/foaf/0.1/name> \"Note\" .";

/// The same backend split as `ldp_http.rs`: the embedded in-process engine on the default build,
/// the in-memory double on `--no-default-features`.
#[cfg(feature = "embedded-sparq")]
type BackendSparqClient = sparq_lws_core::store::EmbeddedSparqClient;
#[cfg(not(feature = "embedded-sparq"))]
type BackendSparqClient = sparq_lws_core::store::InMemorySparqClient;

fn backend_sparq_client() -> BackendSparqClient {
    #[cfg(feature = "embedded-sparq")]
    {
        sparq_lws_core::store::EmbeddedSparqClient::in_memory()
            .expect("fresh in-memory embedded graph")
    }
    #[cfg(not(feature = "embedded-sparq"))]
    {
        sparq_lws_core::store::InMemorySparqClient::new()
    }
}

/// One assembled app over a fresh store, either demo-seeded (flag ON) or untouched (flag OFF).
struct Harness {
    app: axum::Router,
    issuer_key: KeyKit,
    client_key: KeyKit,
}

impl Harness {
    async fn new(demo_seeded: bool) -> Self {
        let store = CompositeStore::new(backend_sparq_client(), InMemoryBlobStore::new());
        if demo_seeded {
            sparq_lws_core::seed::seed_demo(&store, BASE_URL)
                .await
                .expect("demo seed");
        }
        let issuer_key = KeyKit::generate();
        let client_key = KeyKit::generate();
        let config = VerifierConfig::new(vec![common::ISSUER.to_string()], BASE_URL);
        let replay = InMemoryReplayStore::with_window(config.replay_ttl());
        let verifier = Verifier::new(config, jwks_provider(&issuer_key), replay).unwrap();
        let ctx = AuthContext::new(verifier, BASE_URL);
        let ldp = LdpState::new(store, BASE_URL);
        let app = build_router(AppState::new(ctx, ldp));
        Self {
            app,
            issuer_key,
            client_key,
        }
    }

    /// An AUTHENTICATED request (fresh DPoP-bound token + proof; the WebID is `common::WEBID` — an
    /// arbitrary authenticated agent, deliberately NOT named by any seeded ACL).
    async fn request(
        &self,
        method: &str,
        path: &str,
        content_type: Option<&str>,
        body: Body,
    ) -> axum::http::Response<Body> {
        self.request_as(WEBID, method, path, content_type, body).await
    }

    /// An AUTHENTICATED request as an ARBITRARY `webid`. The playground grant is
    /// `acl:agentClass acl:AuthenticatedAgent`, so distinguishing two visitors requires two
    /// genuinely different WebIDs — which is what makes the cross-visitor
    /// overwrite/delete assertion meaningful rather than a self-overwrite.
    async fn request_as(
        &self,
        webid: &str,
        method: &str,
        path: &str,
        content_type: Option<&str>,
        body: Body,
    ) -> axum::http::Response<Body> {
        let access =
            mint_access_token_for(&self.issuer_key, &self.client_key.thumbprint, webid);
        let htu = format!("{BASE_URL}{path}");
        let proof = mint_dpop_proof(&self.client_key, method, &htu, &access);
        let mut builder = Request::builder()
            .method(method)
            .uri(path)
            .header("authorization", format!("DPoP {access}"))
            .header("dpop", proof);
        if let Some(ct) = content_type {
            builder = builder.header("content-type", ct);
        }
        self.app
            .clone()
            .oneshot(builder.body(body).unwrap())
            .await
            .unwrap()
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

/// Any authenticated agent (a throwaway demo identity) can create a resource in the playground —
/// the `acl:AuthenticatedAgent` Write grant flows to the new child via `acl:default`.
#[tokio::test]
async fn demo_seed_authenticated_put_under_playground_creates() {
    let h = Harness::new(true).await;
    let put = h
        .request("PUT", "/playground/note", Some("text/turtle"), Body::from(TURTLE))
        .await;
    assert_eq!(put.status(), StatusCode::CREATED);

    // And any authenticated agent (incl. the same one) reads it back.
    let get = h.request("GET", "/playground/note", None, Body::empty()).await;
    assert_eq!(get.status(), StatusCode::OK);
}

/// Anonymous writes stay rejected (the §3.1 "anonymous writes stay rejected" claim): the public
/// grant is Read-only, so the only write friction the public demo relies on — registration — holds.
#[tokio::test]
async fn demo_seed_anonymous_put_under_playground_is_unauthorized() {
    let h = Harness::new(true).await;
    let put = h
        .unauth_request("PUT", "/playground/anon", Some("text/turtle"), Body::from(TURTLE))
        .await;
    assert_eq!(put.status(), StatusCode::UNAUTHORIZED);
}

/// The `/README` banner document is anonymously dereferenceable (public Read).
#[tokio::test]
async fn demo_seed_anonymous_get_readme_is_public() {
    let h = Harness::new(true).await;
    let get = h.unauth_request("GET", "/README", None, Body::empty()).await;
    assert_eq!(get.status(), StatusCode::OK);
    // The playground container itself is anonymously readable too (all data public-readable).
    let list = h
        .unauth_request("GET", "/playground/", None, Body::empty())
        .await;
    assert_eq!(list.status(), StatusCode::OK);
}

/// NOBODY holds `acl:Control`, so even an authenticated visitor cannot rewrite the playground ACL —
/// the sandbox can never be widened, locked, or hijacked over HTTP.
#[tokio::test]
async fn demo_seed_authenticated_put_of_playground_acl_is_denied() {
    let h = Harness::new(true).await;
    // A well-formed ACL body, so the denial is authorization, not parsing.
    let widened = r#"@prefix acl: <http://www.w3.org/ns/auth/acl#>.
<#public> a acl:Authorization;
          acl:agentClass <http://xmlns.com/foaf/0.1/Agent>;
          acl:accessTo </playground/>;
          acl:mode acl:Read, acl:Write, acl:Control."#;
    let put = h
        .request(
            "PUT",
            "/playground/.acl",
            Some("text/turtle"),
            Body::from(widened),
        )
        .await;
    assert_eq!(put.status(), StatusCode::FORBIDDEN);
    // Anonymous is likewise shut out (401, fail-closed before any Control question).
    let anon = h
        .unauth_request(
            "PUT",
            "/playground/.acl",
            Some("text/turtle"),
            Body::from(widened),
        )
        .await;
    assert_eq!(anon.status(), StatusCode::UNAUTHORIZED);
}

// =====================================================================================
// sq-5ougp review round 3 (gpt-5.6-sol findings 3 + 4): FLAG-ON SCOPE NEGATIVES.
//
// Every flag-ON test above is a POSITIVE: it asserts what the seed grants. Nothing
// asserted what it must NOT grant, so "a widening that adds a root `/.acl` would be
// caught by no test", and deleting the public `acl:default` triple from
// `demo_playground_acl_turtle` failed no test either. The two tests below close both.
// =====================================================================================

/// The demo grants reach EXACTLY `/playground/` (+ descendants) and `/README`. Everything
/// else on the origin stays fail-closed WHILE THE FLAG IS ON — in particular:
///
/// - `/` — the root container is created by the seed (as `/playground/`'s parent) but is
///   given NO ACL, so a widening that added a root `/.acl` would redden here.
/// - `/other` — an unrelated sibling.
/// - `/playgroundX` — the PREFIX SIBLING. `/playground/`'s grant must not leak to it:
///   the resolver walks slash-delimited parents, not string prefixes, so `/playgroundX`'s
///   only ancestor is `/`. A prefix-match regression in `ancestors_nearest_first` would
///   redden here.
/// - `/playground/.acl` and `/README.acl` — reading an ACL needs `acl:Control`, which the
///   seed grants to nobody.
#[tokio::test]
async fn demo_seed_flag_on_grants_nothing_outside_the_playground_and_readme() {
    let h = Harness::new(true).await;
    // Authenticated: no grant ⇒ 403 (authorization precedes the existence check, so this
    // is deliberately not a 404 — no existence oracle).
    for path in [
        "/",
        "/other",
        "/other/child",
        "/playgroundX",
        "/playgroundX/",
        "/playgroundX/note",
        "/playground/.acl",
        "/README.acl",
    ] {
        let get = h.request("GET", path, None, Body::empty()).await;
        assert_eq!(
            get.status(),
            StatusCode::FORBIDDEN,
            "authenticated GET {path} must be denied: the demo seed grants ONLY /playground/ \
             (+ descendants) and /README"
        );
        let put = h
            .request("PUT", path, Some("text/turtle"), Body::from(TURTLE))
            .await;
        assert_eq!(
            put.status(),
            StatusCode::FORBIDDEN,
            "authenticated PUT {path} must be denied: the demo seed grants ONLY /playground/ \
             (+ descendants) and /README"
        );
    }
    // Anonymous: no grant ⇒ 401.
    for path in ["/", "/other", "/playgroundX", "/playground/.acl", "/README.acl"] {
        let get = h.unauth_request("GET", path, None, Body::empty()).await;
        assert_eq!(
            get.status(),
            StatusCode::UNAUTHORIZED,
            "anonymous GET {path} must be denied: the demo seed grants ONLY /playground/ \
             (+ descendants) and /README"
        );
    }
    // Sanity anchor: the two IN-scope resources really are reachable in this same harness,
    // so the negatives above cannot be passing because everything is broken.
    assert_eq!(
        h.unauth_request("GET", "/README", None, Body::empty())
            .await
            .status(),
        StatusCode::OK
    );
    assert_eq!(
        h.unauth_request("GET", "/playground/", None, Body::empty())
            .await
            .status(),
        StatusCode::OK
    );
}

/// The public `acl:default` triple on the playground ACL is what makes a playground CHILD
/// publicly readable — the `/README` banner's "All data is public-readable" claim. Before
/// this test, deleting `Triple::new(public_auth, acl:default, container)` from
/// `demo_playground_acl_turtle` failed NO test: the public `acl:accessTo` grant kept
/// `GET /playground/` at 200 and no test ever anonymously read a child.
///
/// A child has no `.acl` of its own, so the resolver falls back to `/playground/.acl` in
/// `AclScope::Default` — which only applies through `acl:default`. Removing that triple
/// therefore turns this 200 into a 401 (fail-safe, but it silently falsifies the banner).
#[tokio::test]
async fn demo_seed_anonymous_get_of_a_playground_child_inherits_public_read() {
    let h = Harness::new(true).await;
    // An authenticated visitor creates the child (public Write is deliberately absent).
    let put = h
        .request("PUT", "/playground/note", Some("text/turtle"), Body::from(TURTLE))
        .await;
    assert_eq!(put.status(), StatusCode::CREATED);

    // ANONYMOUS read of that child must succeed via the public `acl:default` inheritance.
    let anon_get = h
        .unauth_request("GET", "/playground/note", None, Body::empty())
        .await;
    assert_eq!(
        anon_get.status(),
        StatusCode::OK,
        "an anonymous GET of a playground CHILD must succeed — this is the public \
         `acl:default` inheritance the /README banner's \"All data is public-readable\" \
         claim rests on; deleting that triple makes this a 401"
    );

    // ...while the anonymous WRITE of that same existing child stays rejected: the public
    // grant is Read-only, so `acl:default` inheritance did not widen writes.
    let anon_put = h
        .unauth_request(
            "PUT",
            "/playground/note",
            Some("text/turtle"),
            Body::from(TURTLE),
        )
        .await;
    assert_eq!(
        anon_put.status(),
        StatusCode::UNAUTHORIZED,
        "the public `acl:default` grant must remain Read-ONLY on descendants"
    );

    // And the DISCLOSED cross-visitor consequence (banner: visitors are not isolated): a
    // DIFFERENT authenticated visitor can overwrite and DELETE the first one's resource.
    // This is the RATIFIED §3.2 posture (proceed-and-document #2329), so the point of this
    // assertion is to keep the banner honest, NOT to change the posture.
    assert_ne!(OTHER_WEBID, WEBID, "the two visitors must really differ");
    let overwrite = h
        .request_as(
            OTHER_WEBID,
            "PUT",
            "/playground/note",
            Some("text/turtle"),
            Body::from(TURTLE),
        )
        .await;
    assert!(
        overwrite.status().is_success(),
        "a DIFFERENT authenticated visitor can OVERWRITE another's playground resource — \
         this is the ratified shared-Write posture and is why DEMO_README_BANNER must \
         disclose it; got {}",
        overwrite.status()
    );
    let delete = h
        .request_as(OTHER_WEBID, "DELETE", "/playground/note", None, Body::empty())
        .await;
    assert!(
        delete.status().is_success(),
        "a DIFFERENT authenticated visitor can DELETE another's playground resource — \
         this is the ratified shared-Write posture and is why DEMO_README_BANNER must \
         disclose it; got {}",
        delete.status()
    );
}

/// The `/README` banner is the demo's only disclosure surface, so it must actually say the
/// things the posture makes true. Pins the four disclosures the review required — including
/// the cross-visitor OVERWRITE/DELETE consequence of the shared `acl:Write` grant, which the
/// original banner ("not isolated from each other") did not state plainly.
#[tokio::test]
async fn demo_seed_readme_banner_discloses_the_shared_write_consequences() {
    let banner = sparq_lws_core::seed::DEMO_README_BANNER;
    for claim in [
        "EPHEMERAL",
        "public-readable",
        "throwaway",
        "overwrite",
        "delete",
        "not isolated",
    ] {
        assert!(
            banner.to_ascii_lowercase().contains(&claim.to_ascii_lowercase()),
            "DEMO_README_BANNER must disclose {claim:?} — the shared-Write posture is \
             ratified (research/lws-demo-architecture.md §3.2, proceed-and-document #2329), \
             so it must be DISCLOSED, not silently carried: {banner}"
        );
    }
    // ...and the banner really is what a visitor gets when they dereference /README.
    let h = Harness::new(true).await;
    let get = h.unauth_request("GET", "/README", None, Body::empty()).await;
    assert_eq!(get.status(), StatusCode::OK);
    let body = axum::body::to_bytes(get.into_body(), 64 * 1024)
        .await
        .expect("README body");
    let body = String::from_utf8_lossy(&body);
    assert!(
        body.contains("overwrite"),
        "the served /README must carry the overwrite/delete disclosure: {body}"
    );
}

/// Flag OFF ⇒ byte-identical fail-closed boot: no `/playground/`, no README, no ACLs — every
/// request is denied exactly as on an unseeded server (the feature-off-by-default invariant).
///
/// NOTE (review round 3, finding 1): this test alone does NOT establish off-by-default — it
/// never touches `env_flag`, Guard #2, or the boot call site, so it would pass unchanged if
/// the binary seeded unconditionally. The invariant is now pinned by
/// `demo_seed_boot_call_site_writes_nothing_unless_flag_is_on` and
/// `guard2_seed_requested_sees_the_demo_flag_and_refuses_nonmemory_boot` in `src/main.rs`.
/// This one stays as the HTTP-surface complement: what an UNSEEDED store answers.
#[tokio::test]
async fn demo_seed_flag_off_boot_has_no_playground() {
    let h = Harness::new(false).await;
    // Authenticated GET: WAC fail-closed (no ACL anywhere) → 403, never a 200 listing.
    let get = h.request("GET", "/playground/", None, Body::empty()).await;
    assert_eq!(get.status(), StatusCode::FORBIDDEN);
    // Authenticated PUT into the (nonexistent) playground: denied, nothing is auto-granted.
    let put = h
        .request("PUT", "/playground/note", Some("text/turtle"), Body::from(TURTLE))
        .await;
    assert_eq!(put.status(), StatusCode::FORBIDDEN);
    // The README does not exist / is not readable either.
    let readme = h.unauth_request("GET", "/README", None, Body::empty()).await;
    assert_eq!(readme.status(), StatusCode::UNAUTHORIZED);
}
