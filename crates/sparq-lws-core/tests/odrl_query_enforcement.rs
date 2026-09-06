// AUTHORED-BY Claude Fable 5
//! sq-hfnyz — the **fail-closed policy-differential contract, end-to-end through the REAL embedded
//! Solid server** (`EmbeddedSparqClient::in_memory()`, the same harness shape as
//! `tests/embedded_read_counters.rs`): the SAME read driven by two different requesters provably
//! yields DIFFERENT result sets — the granted party sees the rows, the non-granted party sees none —
//! and a malformed or un-admissible policy DENIES everyone, never silently allows.
//!
//! ## Scope note (honest, per the bead's fallback)
//! The bead's target is the ODRL-gated query path. `sparq-lws-core` today has NO reach to the ODRL
//! bridge: the bridge (`sparq_solid::PodStore::materialize_odrl_policy` + `sparq-policy`) is wired
//! into `sparq-server`'s `/authz/*` lane (PR #1941), while this crate's shipped decide surface is
//! the WAC authorizer (`src/authz/wac.rs`, `mode.rs`). Gating a request here on an ODRL verdict
//! would need a src seam (or test-authored ODRL→WAC translation glue, which would test the GLUE,
//! not a shipped bridge). So — exactly as the bead directs — this file proves the
//! **graph-granularity enforcement contract PR #1941 established**, on THIS crate's real embedded
//! server: in the lws data model every resource IS a named graph whose graph IRI equals the
//! resource IRI (see `store/sparql.rs`), each read is answered by SPARQL over that graph through
//! the in-process engine, and the policy decision gates the WHOLE graph. The ODRL-bridge reach is
//! filed as a follow-up bead (see the PR body).
//!
//! ## The three rows of the contract (the bead's invariant)
//! 1. **Differential grant**: a policy that permits requester A (and only A) on the doc's graph ⇒
//!    A's read returns the graph's rows byte-for-byte, B's IDENTICAL read is denied with NO row
//!    leaked — same query, two requesters, provably different result sets, through the full router
//!    (auth → WAC → LDP → embedded engine).
//! 2. **Malformed policy ⇒ deny**: a syntactically-unparseable governing policy denies EVERY
//!    requester — including the pod owner, whose root-default grant it must NOT fall through to —
//!    while the rows provably still exist in the engine underneath.
//! 3. **Un-admissible policy ⇒ deny**: a policy that parses but grants nothing recognisable (an
//!    unknown mode IRI; a target-less rule) flips the doc from readable to denied for everyone —
//!    never a silent allow.
//!
//! Gated on the opt-in `embedded-sparq` feature: the default build carries no sparq dependency and
//! this test binary is empty (default-features state byte-identical).

#![cfg(feature = "embedded-sparq")]

mod common;

use axum::body::{to_bytes, Body, Bytes};
use axum::http::{header, Request, StatusCode};
use common::{jwks_provider, mint_dpop_proof, KeyKit, BASE_URL, CLIENT_ID, ISSUER};
use serde_json::json;
use solid_oidc_verifier::config::VerifierConfig;
use solid_oidc_verifier::replay::InMemoryReplayStore;
use solid_oidc_verifier::verifier::Verifier;
use sparq_lws_core::app::{build_router, AppState};
use sparq_lws_core::auth::AuthContext;
use sparq_lws_core::ldp::handler::LdpState;
use sparq_lws_core::store::embedded::EmbeddedSparqClient;
use sparq_lws_core::store::{CompositeStore, InMemoryBlobStore, SparqClient, Store};
use tower::ServiceExt;

/// The pod OWNER (root-default Read/Write/Control) — `common::WEBID`.
const OWNER: &str = common::WEBID;
/// Requester A — the party the policy PERMITS on the doc's graph.
const REQ_A: &str = "https://agents.example/req-a#me";
/// Requester B — a distinct authenticated party the policy does NOT permit.
const REQ_B: &str = "https://agents.example/req-b#me";

/// The doc's rows — the result set the policy differential is observed on. The literal is
/// distinctive so a leak into a denial body would be caught by substring assertion.
const GRANTED_ROWS: &str = "<https://pod.example/alice/c/doc#it> \
<http://xmlns.com/foaf/0.1/name> \"granted-row-9d41\" .";
const SECRET_MARKER: &str = "granted-row-9d41";

/// One authenticated requester: their own DPoP client key + WebID (each gets a distinct token).
struct Requester {
    key: KeyKit,
    webid: &'static str,
}

impl Requester {
    fn new(webid: &'static str) -> Self {
        Self {
            key: KeyKit::generate(),
            webid,
        }
    }
}

/// Mint a well-formed RFC-9068 access token for an ARBITRARY WebID (the `common` helper hard-codes
/// the owner's). Same shape as `common::mint_access_token`, parameterised on `webid`.
fn mint_access_token_for(issuer_key: &KeyKit, cnf_jkt: &str, webid: &str) -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static TOKEN_COUNTER: AtomicU64 = AtomicU64::new(0);
    let header = json!({ "alg": "ES256", "typ": "at+jwt" });
    let iat = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    let claims = json!({
        "iss": ISSUER,
        "sub": webid,
        "jti": format!("at-odrl-{}", TOKEN_COUNTER.fetch_add(1, Ordering::Relaxed)),
        "client_id": CLIENT_ID,
        "aud": BASE_URL,
        "webid": webid,
        "cnf": { "jkt": cnf_jkt },
        "iat": iat,
        "exp": iat + 300,
    });
    issuer_key.sign(&header, &claims)
}

/// The harness: the FULL assembled router (auth → WAC → LDP) over the REAL in-process embedded
/// engine + an in-memory blob store — the same stack `embedded_read_counters.rs` drives. A clone of
/// the [`EmbeddedSparqClient`] (same logical backend — the same graph-owning actor thread) is kept
/// OUTSIDE the router so a test can prove data EXISTS in the engine while every HTTP requester is
/// denied.
struct Harness {
    app: axum::Router,
    issuer_key: KeyKit,
    /// Same logical backend as the router's store (clones share the graph).
    engine: EmbeddedSparqClient,
}

impl Harness {
    /// Build the server; `seed` runs against the store BEFORE it moves into the router (the same
    /// pre-router seeding seam `embedded_read_counters::seed_root_owner_acl` uses).
    async fn new<F, Fut>(seed: F) -> Self
    where
        F: FnOnce(CompositeStore<EmbeddedSparqClient, InMemoryBlobStore>) -> Fut,
        Fut: std::future::Future<Output = CompositeStore<EmbeddedSparqClient, InMemoryBlobStore>>,
    {
        let issuer_key = KeyKit::generate();
        let config = VerifierConfig::new(vec![ISSUER.to_string()], BASE_URL);
        let replay = InMemoryReplayStore::with_window(config.replay_ttl());
        let verifier = Verifier::new(config, jwks_provider(&issuer_key), replay).unwrap();
        let ctx = AuthContext::new(verifier, BASE_URL);

        let engine = EmbeddedSparqClient::in_memory().expect("empty in-memory graph");
        let mut store = CompositeStore::new(engine.clone(), InMemoryBlobStore::default());
        seed_root_owner_acl(&store, BASE_URL, OWNER).await;
        store = seed(store).await;
        let ldp = LdpState::new(store, BASE_URL);
        let app = build_router(AppState::new(ctx, ldp));
        Self {
            app,
            issuer_key,
            engine,
        }
    }

    /// One authenticated request AS `who` (their own token + DPoP proof).
    async fn request_as(
        &self,
        who: &Requester,
        method: &str,
        path: &str,
        content_type: Option<&str>,
        body: Body,
    ) -> axum::http::Response<Body> {
        let access = mint_access_token_for(&self.issuer_key, &who.key.thumbprint, who.webid);
        let htu = format!("{BASE_URL}{path}");
        let proof = mint_dpop_proof(&who.key, method, &htu, &access);
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

    /// One ANONYMOUS request (no credentials at all).
    async fn request_anon(&self, method: &str, path: &str) -> axum::http::Response<Body> {
        let req = Request::builder()
            .method(method)
            .uri(path)
            .body(Body::empty())
            .unwrap();
        self.app.clone().oneshot(req).await.unwrap()
    }
}

/// The same root owner-ACL fixture as `embedded_read_counters.rs`: Read/Write/Control +
/// `acl:default` for the owner at the storage root.
async fn seed_root_owner_acl(
    store: &CompositeStore<EmbeddedSparqClient, InMemoryBlobStore>,
    base_url: &str,
    owner_webid: &str,
) {
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
        .write(&acl_iri, Bytes::from(acl_body), "text/turtle")
        .await
        .expect("seed root acl");
}

/// Read a response body to a UTF-8 string (denial bodies are small error payloads).
async fn body_text(resp: axum::http::Response<Body>) -> String {
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    String::from_utf8_lossy(&bytes).into_owned()
}

/// Owner creates the container + the doc carrying [`GRANTED_ROWS`] through the server (e2e writes).
async fn create_doc(h: &Harness, owner: &Requester) {
    let mk = h
        .request_as(
            owner,
            "PUT",
            "/alice/c/",
            Some("text/turtle"),
            Body::from("<#c> <http://xmlns.com/foaf/0.1/name> \"C\" ."),
        )
        .await;
    assert_eq!(mk.status(), StatusCode::CREATED, "owner creates /alice/c/");
    let put = h
        .request_as(
            owner,
            "PUT",
            "/alice/c/doc",
            Some("text/turtle"),
            Body::from(GRANTED_ROWS),
        )
        .await;
    assert_eq!(put.status(), StatusCode::CREATED, "owner creates the doc");
}

// ---------------------------------------------------------------------------------------------------
// Row 1 — the differential grant: same query, two requesters, provably different result sets.
// ---------------------------------------------------------------------------------------------------

/// A policy permitting requester A (and ONLY A) on the doc's graph makes the SAME read yield
/// DIFFERENT result sets end-to-end through the embedded server: A observes the granted rows
/// byte-for-byte; B (an authenticated, distinct party) is denied with an EMPTY result set — no row
/// leaks into the denial; an anonymous read is a 401 challenge. The policy names the doc's IRI,
/// which in the lws data model IS the named graph the read's SPARQL runs over — graph granularity.
#[tokio::test]
async fn same_query_two_requesters_observe_different_result_sets() {
    let h = Harness::new(|s| async { s }).await;
    let owner = Requester::new(OWNER);
    let req_a = Requester::new(REQ_A);
    let req_b = Requester::new(REQ_B);

    create_doc(&h, &owner).await;

    // The policy: permit A — and only A — to Read the doc's graph. (The owner has Control via the
    // root default, so may install it; once present, this NEAREST policy alone governs the doc.)
    let policy = format!(
        r#"@prefix acl: <http://www.w3.org/ns/auth/acl#>.
<#permit-a> a acl:Authorization;
            acl:agent <{REQ_A}>;
            acl:accessTo <https://pod.example/alice/c/doc>;
            acl:mode acl:Read."#
    );
    let put_acl = h
        .request_as(
            &owner,
            "PUT",
            "/alice/c/doc.acl",
            Some("text/turtle"),
            Body::from(policy),
        )
        .await;
    assert_eq!(
        put_acl.status(),
        StatusCode::CREATED,
        "owner installs the policy"
    );

    // Requester A: the SAME query (GET doc → SPARQL over the doc's named graph via the embedded
    // engine) returns the granted rows byte-for-byte.
    let resp_a = h
        .request_as(&req_a, "GET", "/alice/c/doc", None, Body::empty())
        .await;
    assert_eq!(
        resp_a.status(),
        StatusCode::OK,
        "permitted party reads the graph"
    );
    let body_a = body_text(resp_a).await;
    assert_eq!(body_a, GRANTED_ROWS, "A observes exactly the granted rows");

    // Requester B: the IDENTICAL query is DENIED — an empty result set, with no row leaked into
    // the denial body. Same query, different requester, provably different observation.
    let resp_b = h
        .request_as(&req_b, "GET", "/alice/c/doc", None, Body::empty())
        .await;
    assert_eq!(
        resp_b.status(),
        StatusCode::FORBIDDEN,
        "non-permitted party is denied (403, not a silent allow)"
    );
    let body_b = body_text(resp_b).await;
    assert!(
        !body_b.contains(SECRET_MARKER),
        "the denial must not leak any granted row: {body_b}"
    );

    // Anonymous: the same query is an auth challenge (401), also row-free.
    let resp_anon = h.request_anon("GET", "/alice/c/doc").await;
    assert_eq!(resp_anon.status(), StatusCode::UNAUTHORIZED);
    assert!(resp_anon.headers().contains_key(header::WWW_AUTHENTICATE));
    let body_anon = body_text(resp_anon).await;
    assert!(
        !body_anon.contains(SECRET_MARKER),
        "no row leaks anonymously"
    );

    // And the differential is the POLICY's doing, not the owner's identity: the nearest policy
    // names only A, so even the pod OWNER's identical query is denied on this graph.
    let resp_owner = h
        .request_as(&owner, "GET", "/alice/c/doc", None, Body::empty())
        .await;
    assert_eq!(
        resp_owner.status(),
        StatusCode::FORBIDDEN,
        "the nearest policy (permit A only) governs — even the owner's root default does not reach past it"
    );
}

// ---------------------------------------------------------------------------------------------------
// Row 2 — malformed policy ⇒ deny (never silent-allow, never fall-through).
// ---------------------------------------------------------------------------------------------------

/// A syntactically-MALFORMED governing policy denies EVERY requester — including the pod owner,
/// whose root-default grant it must NOT fall through to — while the rows provably still exist in
/// the embedded engine underneath (so the empty result sets are the POLICY's doing, not missing
/// data). Malformed-policy row of the bead's invariant: deny, never silently allow.
#[tokio::test]
async fn malformed_policy_denies_every_requester_never_allows() {
    const DOC: &str = "https://pod.example/alice/m/doc";
    const DOC_ACL: &str = "https://pod.example/alice/m/doc.acl";

    // Seed the doc + its PRESENT-but-unparseable policy directly at the store seam (the server's
    // own write path rightly refuses to persist garbage Turtle, so a corrupted/rotted policy is
    // modelled as pre-existing state — exactly what the read path must fail closed on).
    let h = Harness::new(|store| async move {
        store
            .write(DOC, Bytes::from(GRANTED_ROWS), "text/turtle")
            .await
            .expect("seed the doc");
        store
            .write(
                DOC_ACL,
                Bytes::from("@@@ this is not turtle — a corrupted policy body @@@"),
                "text/turtle",
            )
            .await
            .expect("seed the malformed policy");
        store
    })
    .await;
    let owner = Requester::new(OWNER);
    let req_a = Requester::new(REQ_A);

    // The rows EXIST in the engine (same shared graph the router queries) …
    assert!(
        h.engine.get_meta(DOC).await.is_ok(),
        "the doc is present in the embedded engine"
    );

    // … yet the malformed governing policy denies the OWNER (no fall-through to the root-default
    // grant that would otherwise allow this exact read) …
    let resp_owner = h
        .request_as(&owner, "GET", "/alice/m/doc", None, Body::empty())
        .await;
    assert_eq!(
        resp_owner.status(),
        StatusCode::FORBIDDEN,
        "a malformed policy DENIES even the owner — never falls through to an inherited grant"
    );
    assert!(!body_text(resp_owner).await.contains(SECRET_MARKER));

    // … and every other requester too: authenticated ⇒ 403, anonymous ⇒ 401. Empty result sets all.
    let resp_a = h
        .request_as(&req_a, "GET", "/alice/m/doc", None, Body::empty())
        .await;
    assert_eq!(resp_a.status(), StatusCode::FORBIDDEN);
    assert!(!body_text(resp_a).await.contains(SECRET_MARKER));

    let resp_anon = h.request_anon("GET", "/alice/m/doc").await;
    assert_eq!(resp_anon.status(), StatusCode::UNAUTHORIZED);
    assert!(!body_text(resp_anon).await.contains(SECRET_MARKER));
}

// ---------------------------------------------------------------------------------------------------
// Row 3 — un-admissible policy ⇒ deny (parses, but grants nothing recognisable).
// ---------------------------------------------------------------------------------------------------

/// An UN-ADMISSIBLE policy — syntactically valid but granting nothing the decide surface can admit
/// (an unrecognised mode IRI; a rule with no target) — flips the doc from readable to denied for
/// EVERYONE, through the server, end-to-end. The un-admissible rules must be inert (deny), never
/// rounded up to a grant.
#[tokio::test]
async fn unadmissible_policy_grants_nothing() {
    let h = Harness::new(|s| async { s }).await;
    let owner = Requester::new(OWNER);
    let req_a = Requester::new(REQ_A);

    create_doc(&h, &owner).await;

    // Control observation: BEFORE the un-admissible policy exists the owner's identical query
    // succeeds (root-default grant) — so the flip below is attributable to the policy alone.
    let before = h
        .request_as(&owner, "GET", "/alice/c/doc", None, Body::empty())
        .await;
    assert_eq!(
        before.status(),
        StatusCode::OK,
        "readable before the policy lands"
    );
    assert_eq!(body_text(before).await, GRANTED_ROWS);

    // The un-admissible policy: rule 1 names A with a mode IRI the model does not recognise; rule 2
    // names A with a real mode but NO target (binds to nothing). Both must be inert.
    let policy = format!(
        r#"@prefix acl: <http://www.w3.org/ns/auth/acl#>.
<#unknown-mode> a acl:Authorization;
                acl:agent <{REQ_A}>;
                acl:accessTo <https://pod.example/alice/c/doc>;
                acl:mode <http://www.w3.org/ns/auth/acl#Teleport>.
<#no-target> a acl:Authorization;
             acl:agent <{REQ_A}>;
             acl:mode acl:Read."#
    );
    let put_acl = h
        .request_as(
            &owner,
            "PUT",
            "/alice/c/doc.acl",
            Some("text/turtle"),
            Body::from(policy),
        )
        .await;
    assert_eq!(put_acl.status(), StatusCode::CREATED);

    // The named party gains NOTHING from the un-admissible rules …
    let resp_a = h
        .request_as(&req_a, "GET", "/alice/c/doc", None, Body::empty())
        .await;
    assert_eq!(
        resp_a.status(),
        StatusCode::FORBIDDEN,
        "an unrecognised mode / target-less rule must be inert — never rounded up to a grant"
    );
    assert!(!body_text(resp_a).await.contains(SECRET_MARKER));

    // … and the doc flipped to denied for the owner too (the nearest, granting-nothing policy
    // governs; no fall-through): 200 before ⇒ 403 after, on the identical query.
    let after = h
        .request_as(&owner, "GET", "/alice/c/doc", None, Body::empty())
        .await;
    assert_eq!(
        after.status(),
        StatusCode::FORBIDDEN,
        "the un-admissible policy denies — never silently allows"
    );
    assert!(!body_text(after).await.contains(SECRET_MARKER));
}
