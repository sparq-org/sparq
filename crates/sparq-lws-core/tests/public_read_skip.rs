// AUTHORED-BY Claude Opus 4.8
//! Skip-crypto opt 3 (PUBLIC-read skip) — adversarial end-to-end tests through the assembled router.
//!
//! These execute the REAL router (CORS → public-read skip → auth → handler) over the in-memory
//! store and pin the security invariants of `research/lws-design-records.md` §5 (RSS
//! `decisions/0002`). The HARD SCOPE LIMIT the WAC-Allow conformance suite enforces is
//! load-bearing: the skip fires ONLY for a GET/HEAD that carries NEITHER an `Authorization` NOR a
//! `DPoP` header; a CREDENTIALED read is never short-circuited (so an authenticated owner of a
//! public resource sees their full `WAC-Allow user=` modes, and a forged token is rejected, not
//! served). A malformed `DPoP` header (even without `Authorization`) keeps the auth path's
//! canonical 400.
//!
//! - **INV-1 anonymous-equivalence:** the skip path's FULL response (status + EVERY header + body) is
//!   byte-identical to the FORCED full-anonymous path (no `Authorization` + a present `DPoP` header,
//!   which falls through to the auth middleware → public token → the same `serve_read`), for a public
//!   GET, a public HEAD, AND a private-401 denial.
//! - **INV-2 identity-independence + scope limit:** an authenticated OWNER of a public resource sees
//!   their FULL `user` modes (the full verify ran — NOT served as anonymous); a FORGED token is
//!   rejected (401), never served as anonymous.
//! - **INV-3 no-oracle:** `WAC-Allow` advertises `user == public` on the anonymous skip; a missing
//!   public resource is the same 404 anonymous gets (no existence leak).
//! - **INV-6 origin-fail-closed:** an `acl:origin`-scoped public grant is skipped only for a matching
//!   Origin; a no-Origin / wrong-Origin anonymous caller fails closed (401).
//! - **private-resource-still-crypto:** a private resource still runs the FULL verifier — a valid
//!   owner token gets its access; anonymous gets 401; a forged token gets 401.

mod common;

use axum::body::{to_bytes, Body};
use axum::http::{Request, Response, StatusCode};
use common::{jwks_provider, mint_access_token, mint_dpop_proof, KeyKit, BASE_URL, ISSUER, WEBID};
use solid_oidc_verifier::config::VerifierConfig;
use solid_oidc_verifier::replay::InMemoryReplayStore;
use solid_oidc_verifier::verifier::Verifier;
use sparq_lws_core::app::{build_router, AppState};
use sparq_lws_core::auth::AuthContext;
use sparq_lws_core::ldp::handler::LdpState;
use sparq_lws_core::store::{CompositeStore, InMemoryBlobStore, InMemorySparqClient, Store};
use tower::ServiceExt;

const TURTLE: &str =
    "<https://pod.example/alice/data#me> <http://xmlns.com/foaf/0.1/name> \"Alice\" .";

/// The application + the keys to mint requests. The CLIENT key is what a valid token is cnf-bound to;
/// the ISSUER key signs the token. A separate ROGUE issuer key mints "forged" (untrusted) tokens.
struct Harness {
    app: axum::Router,
    issuer_key: KeyKit,
    client_key: KeyKit,
    rogue_key: KeyKit,
}

impl Harness {
    async fn new() -> Self {
        let issuer_key = KeyKit::generate();
        let client_key = KeyKit::generate();
        let rogue_key = KeyKit::generate();
        let config = VerifierConfig::new(vec![ISSUER.to_string()], BASE_URL);
        let replay = InMemoryReplayStore::with_window(config.replay_ttl());
        let verifier = Verifier::new(config, jwks_provider(&issuer_key), replay).unwrap();
        let ctx = AuthContext::new(verifier, BASE_URL);
        let store = CompositeStore::new(InMemorySparqClient::new(), InMemoryBlobStore::new());

        // --- ACL fixtures ---------------------------------------------------------------------
        // Root: owner (alice == WEBID) full control on the root + all descendants (so alice can seed
        // and read everything), but NO public default — descendants are private unless a closer ACL
        // grants the public.
        seed_acl(
            &store,
            "https://pod.example/.acl",
            &format!(
                r#"@prefix acl: <http://www.w3.org/ns/auth/acl#>.
<#owner> a acl:Authorization; acl:agent <{WEBID}>;
  acl:accessTo <https://pod.example/>; acl:default <https://pod.example/>;
  acl:mode acl:Read, acl:Write, acl:Control."#
            ),
        )
        .await;

        // A PUBLIC document: foaf:Agent acl:Read (+ owner control) → the skip SHOULD fire.
        store
            .write(
                "https://pod.example/pub",
                axum::body::Bytes::from(TURTLE),
                "text/turtle",
            )
            .await
            .unwrap();
        seed_acl(
            &store,
            "https://pod.example/pub.acl",
            &format!(
                r#"@prefix acl: <http://www.w3.org/ns/auth/acl#>.
@prefix foaf: <http://xmlns.com/foaf/0.1/>.
<#owner> a acl:Authorization; acl:agent <{WEBID}>; acl:accessTo <https://pod.example/pub>; acl:mode acl:Read, acl:Write, acl:Control.
<#pub> a acl:Authorization; acl:agentClass foaf:Agent; acl:accessTo <https://pod.example/pub>; acl:mode acl:Read."#
            ),
        )
        .await;

        // A PRIVATE document: only alice (owner) — no public grant → the skip must NOT fire.
        store
            .write(
                "https://pod.example/secret",
                axum::body::Bytes::from(TURTLE),
                "text/turtle",
            )
            .await
            .unwrap();
        seed_acl(
            &store,
            "https://pod.example/secret.acl",
            &format!(
                r#"@prefix acl: <http://www.w3.org/ns/auth/acl#>.
<#owner> a acl:Authorization; acl:agent <{WEBID}>; acl:accessTo <https://pod.example/secret>; acl:mode acl:Read, acl:Write, acl:Control."#
            ),
        )
        .await;

        // A PUBLIC container with `acl:default` foaf:Agent Read — so a MISSING child under it is
        // publicly readable (the skip fires) and serves a 404 identical to anonymous (INV-3,
        // no existence leak). The container itself is created so its `.acl` resolves.
        store
            .write(
                "https://pod.example/pubc/.dummy",
                axum::body::Bytes::from(TURTLE),
                "text/turtle",
            )
            .await
            .unwrap();
        seed_acl(
            &store,
            "https://pod.example/pubc/.acl",
            &format!(
                r#"@prefix acl: <http://www.w3.org/ns/auth/acl#>.
@prefix foaf: <http://xmlns.com/foaf/0.1/>.
<#owner> a acl:Authorization; acl:agent <{WEBID}>; acl:accessTo <https://pod.example/pubc/>; acl:default <https://pod.example/pubc/>; acl:mode acl:Read, acl:Write, acl:Control.
<#pub> a acl:Authorization; acl:agentClass foaf:Agent; acl:default <https://pod.example/pubc/>; acl:mode acl:Read."#
            ),
        )
        .await;

        // An ORIGIN-SCOPED public document: foaf:Agent acl:Read but ONLY from Origin https://app.example.
        store
            .write(
                "https://pod.example/origin-pub",
                axum::body::Bytes::from(TURTLE),
                "text/turtle",
            )
            .await
            .unwrap();
        seed_acl(
            &store,
            "https://pod.example/origin-pub.acl",
            &format!(
                r#"@prefix acl: <http://www.w3.org/ns/auth/acl#>.
@prefix foaf: <http://xmlns.com/foaf/0.1/>.
<#owner> a acl:Authorization; acl:agent <{WEBID}>; acl:accessTo <https://pod.example/origin-pub>; acl:mode acl:Read, acl:Write, acl:Control.
<#app> a acl:Authorization; acl:agentClass foaf:Agent; acl:origin <https://app.example>; acl:accessTo <https://pod.example/origin-pub>; acl:mode acl:Read."#
            ),
        )
        .await;

        let ldp = LdpState::new(store, BASE_URL);
        let app = build_router(AppState::new(ctx, ldp));
        Self {
            app,
            issuer_key,
            client_key,
            rogue_key,
        }
    }

    /// A VALID `(Authorization, DPoP)` pair for one request: token signed by the trusted issuer, bound
    /// to the client key, with a fresh proof jti.
    fn valid_headers(&self, method: &str, path: &str) -> (String, String) {
        let access = mint_access_token(&self.issuer_key, &self.client_key.thumbprint);
        let htu = format!("{BASE_URL}{path}");
        let proof = mint_dpop_proof(&self.client_key, method, &htu, &access);
        (format!("DPoP {access}"), proof)
    }

    /// A FORGED `(Authorization, DPoP)` pair: the token is signed by an UNTRUSTED (rogue) issuer but
    /// claims `webid=alice`. The full verifier MUST reject it (untrusted issuer → 401); the skip path
    /// never reaches the verifier, so a public read is unaffected by it (INV-2).
    fn forged_headers(&self, method: &str, path: &str) -> (String, String) {
        let access = mint_access_token(&self.rogue_key, &self.client_key.thumbprint);
        let htu = format!("{BASE_URL}{path}");
        let proof = mint_dpop_proof(&self.client_key, method, &htu, &access);
        (format!("DPoP {access}"), proof)
    }

    async fn send(&self, req: Request<Body>) -> Response<Body> {
        self.app.clone().oneshot(req).await.unwrap()
    }
}

async fn seed_acl(
    store: &CompositeStore<InMemorySparqClient, InMemoryBlobStore>,
    iri: &str,
    body: &str,
) {
    store
        .write(
            iri,
            axum::body::Bytes::from(body.to_string()),
            "text/turtle",
        )
        .await
        .expect("seed acl");
}

fn get(path: &str) -> Request<Body> {
    Request::builder()
        .method("GET")
        .uri(path)
        .body(Body::empty())
        .unwrap()
}

/// Look up a single header's bytes in a sorted snapshot header list (panics if absent).
fn header_value<'a>(headers: &'a [(String, Vec<u8>)], name: &str) -> &'a [u8] {
    headers
        .iter()
        .find(|(k, _)| k == name)
        .map(|(_, v)| v.as_slice())
        .unwrap_or_else(|| panic!("header {name} not present"))
}

/// The full response as a comparable tuple: status + ALL headers (sorted) + body bytes. This is the
/// byte-equivalence comparison the invariants are pinned against.
async fn snapshot(resp: Response<Body>) -> (StatusCode, Vec<(String, Vec<u8>)>, Vec<u8>) {
    let status = resp.status();
    let mut headers: Vec<(String, Vec<u8>)> = resp
        .headers()
        .iter()
        .map(|(k, v)| (k.as_str().to_string(), v.as_bytes().to_vec()))
        .collect();
    headers.sort();
    let body = to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap()
        .to_vec();
    (status, headers, body)
}

// --------------------------------------------------------------------------------------------------
// INV-1 — anonymous-equivalence: the skip fires ONLY for a NO-Authorization request, and that
// response is byte-identical to the genuinely anonymous one (it IS an anonymous request).
// --------------------------------------------------------------------------------------------------

/// A request that takes the SKIP path: no `Authorization`, no `DPoP` → the pre-crypto middleware
/// handles it (delegating to `serve_read` with a public token).
fn skip_path(method: &str, path: &str) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(path)
        .body(Body::empty())
        .unwrap()
}

/// A request that is FORCED through the FULL auth path while remaining anonymous: no `Authorization`,
/// but a valid-UTF8 `DPoP` header present → the skip middleware falls through (any `DPoP` header is a
/// fall-through trigger), the auth middleware sees no `Authorization` → a public token → the SAME
/// `serve_read`. The presence of a meaningless `DPoP` header changes nothing the response can observe
/// (it is only consulted alongside an `Authorization`), so this is a genuine full-path ANONYMOUS read
/// — the perfect baseline to byte-compare the skip path against.
fn full_path_anonymous(method: &str, path: &str) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(path)
        .header("dpop", "ignored-but-present-valid-utf8")
        .body(Body::empty())
        .unwrap()
}

#[tokio::test]
async fn inv1_skip_path_is_byte_identical_to_full_anonymous_path_get() {
    let h = Harness::new().await;
    // FULL byte-equivalence: the SKIP path (no creds) vs the FULL anonymous path (no Authorization +
    // a present DPoP header forcing fall-through). Compare the ENTIRE response tuple — status, EVERY
    // header (ETag, Content-Length, Content-Type, Accept-Ranges, Link, Allow, WAC-Allow, CORS …) and
    // body — so a divergence in ANY header is caught (INV-1).
    let skip = snapshot(h.send(skip_path("GET", "/pub")).await).await;
    let full = snapshot(h.send(full_path_anonymous("GET", "/pub")).await).await;
    assert_eq!(skip.0, StatusCode::OK, "skip-path public GET is 200");
    assert_eq!(
        skip.2,
        TURTLE.as_bytes(),
        "skip-path public GET serves the body"
    );
    assert_eq!(
        skip, full,
        "the skip path must be byte-identical (status + ALL headers + body) to the full anonymous \
         path for the same public GET (INV-1)"
    );
    // And the public WAC-Allow is exactly the public set.
    assert_eq!(
        header_value(&skip.1, "wac-allow"),
        b"user=\"read\",public=\"read\""
    );
}

#[tokio::test]
async fn inv1_skip_path_is_byte_identical_to_full_anonymous_path_head() {
    let h = Harness::new().await;
    // Same full-tuple byte-equivalence for HEAD (no body; the header set — incl. Content-Length —
    // must still match exactly).
    let skip = snapshot(h.send(skip_path("HEAD", "/pub")).await).await;
    let full = snapshot(h.send(full_path_anonymous("HEAD", "/pub")).await).await;
    assert_eq!(skip.0, StatusCode::OK, "skip-path public HEAD is 200");
    assert_eq!(
        skip, full,
        "the skip path must be byte-identical (status + ALL headers) to the full anonymous path for \
         the same public HEAD (INV-1)"
    );
}

#[tokio::test]
async fn inv1_skip_path_private_401_is_byte_identical_to_full_anonymous_path() {
    let h = Harness::new().await;
    // The denial path too: an anonymous read of a PRIVATE resource via the skip path must be
    // byte-identical to the full anonymous path's 401 (same status + WWW-Authenticate + every header).
    let skip = snapshot(h.send(skip_path("GET", "/secret")).await).await;
    let full = snapshot(h.send(full_path_anonymous("GET", "/secret")).await).await;
    assert_eq!(
        skip.0,
        StatusCode::UNAUTHORIZED,
        "anonymous private read is 401"
    );
    assert_eq!(
        skip, full,
        "an anonymous private read via the skip path must be byte-identical to the full anonymous \
         path's 401 (incl. WWW-Authenticate) — no oracle, no divergence (INV-1)"
    );
}

// --------------------------------------------------------------------------------------------------
// INV-2 — identity-independence + the HARD SCOPE LIMIT: a CREDENTIALED public read is NEVER
// short-circuited. The authenticated OWNER of a public resource must see their FULL user modes (the
// full verify ran), and a FORGED token must be REJECTED (not served as anonymous). This is the
// conformance-critical correctness the WAC-Allow suite enforces.
// --------------------------------------------------------------------------------------------------

#[tokio::test]
async fn inv2_authenticated_owner_of_public_resource_sees_full_user_modes() {
    let h = Harness::new().await;
    // alice (WEBID) OWNS /pub (read/write/control via the root owner ACL) AND /pub is public-read. A
    // valid alice token must take the FULL path — NOT be served as anonymous — so WAC-Allow `user`
    // reflects her elevated owner modes, while `public` stays the public {read}. (The pre-crypto skip
    // MUST NOT downgrade her to user=="read"; that was the bug the WAC-Allow conformance suite caught.)
    let (authz, dpop) = h.valid_headers("GET", "/pub");
    let resp = h
        .send(
            Request::builder()
                .method("GET")
                .uri("/pub")
                .header("authorization", authz)
                .header("dpop", dpop)
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let wac = resp
        .headers()
        .get("wac-allow")
        .and_then(|v| v.to_str().ok())
        .unwrap()
        .to_string();
    assert_eq!(
        wac, "user=\"read write control\",public=\"read\"",
        "an authenticated OWNER of a public resource must see their FULL user modes — the skip must \
         NOT serve a credentialed read as anonymous (the WAC-Allow conformance contract)"
    );
}

#[tokio::test]
async fn inv2_forged_token_on_public_resource_is_rejected_not_served_as_anonymous() {
    let h = Harness::new().await;
    // A FORGED (untrusted-issuer) token on a PUBLIC resource must be REJECTED by the full verifier
    // (401), NOT short-circuited and served as anonymous. Because a credentialed request is never
    // skipped, a forged WebID can never influence a served response — it is rejected, exactly like on
    // any other resource. (If the skip served credentialed public reads as anonymous, a forged proof
    // would be indistinguishable from an owner's — the reason opt-3-for-credentialed-reads is unsafe.)
    let (authz, dpop) = h.forged_headers("GET", "/pub");
    let resp = h
        .send(
            Request::builder()
                .method("GET")
                .uri("/pub")
                .header("authorization", authz)
                .header("dpop", dpop)
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(
        resp.status(),
        StatusCode::UNAUTHORIZED,
        "a forged token on a public resource must be rejected by the full verifier (401), never \
         served as anonymous — a credentialed request is never short-circuited (INV-2)"
    );
}

// --------------------------------------------------------------------------------------------------
// INV-3 — no-oracle: WAC-Allow user==public on the (anonymous) skip; missing public resource is the
// same 404.
// --------------------------------------------------------------------------------------------------

#[tokio::test]
async fn inv3_anonymous_public_read_wac_allow_user_equals_public() {
    let h = Harness::new().await;
    let resp = h.send(get("/pub")).await;
    let wac = resp
        .headers()
        .get("wac-allow")
        .and_then(|v| v.to_str().ok())
        .unwrap()
        .to_string();
    // The public document grants `foaf:Agent acl:Read`; the anonymous caller IS the public, so both
    // audiences are exactly {read} — no would-be-authenticated mode set leaks.
    assert_eq!(
        wac, "user=\"read\",public=\"read\"",
        "anonymous skip-path WAC-Allow advertises user==public (the public set)"
    );
}

#[tokio::test]
async fn inv3_missing_public_resource_is_same_404_as_anonymous() {
    let h = Harness::new().await;
    // A MISSING child under a publicly-readable container: the public ACL grants Read, so the skip
    // fires for both anonymous and token-bearing reads and serves the SAME 404 — WAC runs BEFORE the
    // existence check, so the skip does not leak existence any differently than anonymous (INV-3).
    let anon = snapshot(h.send(get("/pubc/missing")).await).await;
    let (authz, dpop) = h.valid_headers("GET", "/pubc/missing");
    let tok = snapshot(
        h.send(
            Request::builder()
                .method("GET")
                .uri("/pubc/missing")
                .header("authorization", authz)
                .header("dpop", dpop)
                .body(Body::empty())
                .unwrap(),
        )
        .await,
    )
    .await;
    assert_eq!(anon.0, StatusCode::NOT_FOUND, "missing public child is 404");
    assert_eq!(
        tok, anon,
        "a missing publicly-readable child must be the same 404 token-vs-anonymous — no existence oracle (INV-3)"
    );
}

// --------------------------------------------------------------------------------------------------
// INV-6 — origin-fail-closed: acl:origin-scoped public grant skipped only for a matching Origin.
// --------------------------------------------------------------------------------------------------

#[tokio::test]
async fn inv6_origin_scoped_public_skips_only_for_matching_origin() {
    let h = Harness::new().await;

    // MATCHING Origin (anonymous) → the origin-scoped public grant applies → the skip fires → 200 +
    // the body. The skip's anonymous predicate runs `authorize_read(web_id=None, origin)`, which
    // honours `acl:origin`.
    let anon_match = h
        .send(
            Request::builder()
                .method("GET")
                .uri("/origin-pub")
                .header("origin", "https://app.example")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(
        anon_match.status(),
        StatusCode::OK,
        "matching-origin anonymous read of an origin-scoped public resource is 200 (the skip fires)"
    );

    // NO Origin → the origin-scoped grant FAILS CLOSED → anonymous is denied 401 (the skip falls
    // through; anonymous still can't read it). This is the load-bearing fail-closed assertion (INV-6):
    // a no-Origin caller never satisfies an `acl:origin`-restricted public rule.
    let anon_no_origin = h.send(get("/origin-pub")).await.status();
    assert_eq!(
        anon_no_origin,
        StatusCode::UNAUTHORIZED,
        "no-Origin anonymous read of an origin-scoped public resource must fail closed (401)"
    );

    // WRONG Origin → likewise fails closed for anonymous.
    let anon_wrong = h
        .send(
            Request::builder()
                .method("GET")
                .uri("/origin-pub")
                .header("origin", "https://evil.example")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .status();
    assert_eq!(
        anon_wrong,
        StatusCode::UNAUTHORIZED,
        "wrong-Origin anonymous read of an origin-scoped public resource must fail closed (401)"
    );
}

// --------------------------------------------------------------------------------------------------
// private-resource-still-crypto: the skip must NOT change a private resource's behaviour.
// --------------------------------------------------------------------------------------------------

#[tokio::test]
async fn private_resource_still_runs_full_crypto() {
    let h = Harness::new().await;

    // Anonymous → 401 (the skip's anonymous predicate denies; it falls through; the handler 401s).
    let anon = h.send(get("/secret")).await;
    assert_eq!(
        anon.status(),
        StatusCode::UNAUTHORIZED,
        "anonymous read of a private resource is 401"
    );
    assert!(
        anon.headers().contains_key("www-authenticate"),
        "the 401 must carry a WWW-Authenticate challenge"
    );

    // A VALID owner token → 200 + the body (the full verifier ran; alice is granted).
    let (authz, dpop) = h.valid_headers("GET", "/secret");
    let owner = h
        .send(
            Request::builder()
                .method("GET")
                .uri("/secret")
                .header("authorization", authz)
                .header("dpop", dpop)
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(
        owner.status(),
        StatusCode::OK,
        "a valid owner token must still get 200 on a private resource (no behaviour change)"
    );
    let body = to_bytes(owner.into_body(), usize::MAX).await.unwrap();
    assert_eq!(body.as_ref(), TURTLE.as_bytes());

    // A FORGED (untrusted-issuer) token → 401 (the verifier rejects it; the skip never short-circuits
    // a private read). This is the 401-vs-403-oracle-free behaviour: a private resource ALWAYS runs
    // the crypto, so a forged token is rejected by the verifier exactly as before.
    let (authz_f, dpop_f) = h.forged_headers("GET", "/secret");
    let forged = h
        .send(
            Request::builder()
                .method("GET")
                .uri("/secret")
                .header("authorization", authz_f)
                .header("dpop", dpop_f)
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(
        forged.status(),
        StatusCode::UNAUTHORIZED,
        "a forged token on a private resource is rejected by the full verifier (401)"
    );
}

#[tokio::test]
async fn mutation_never_skips_crypto_anonymous_put_is_401() {
    let h = Harness::new().await;
    // A PUT (mutation) must NEVER be short-circuited — it goes straight to the auth path. An anonymous
    // PUT to the public document needs Write on the parent, which the public lacks → 401 (the full
    // path), NOT a skipped read. (The skip only handles GET/HEAD.)
    let req = Request::builder()
        .method("PUT")
        .uri("/pub")
        .header("content-type", "text/turtle")
        .body(Body::from(TURTLE))
        .unwrap();
    let resp = h.send(req).await;
    assert_eq!(
        resp.status(),
        StatusCode::UNAUTHORIZED,
        "an anonymous PUT must hit the full auth path (the skip is GET/HEAD only), 401"
    );
}

#[tokio::test]
async fn malformed_dpop_without_authorization_falls_through_to_400() {
    let h = Harness::new().await;
    // A GET with NO Authorization but a PRESENT-but-malformed (non-UTF-8) `DPoP` header. The full auth
    // path rejects this with 400 ("malformed DPoP header") even without Authorization. The skip MUST
    // fall through (it triggers on the presence of a `DPoP` header) so this stays a 400 — not served as
    // anonymous 200 (the divergence roborev flagged). A public resource makes the bug observable: if
    // the skip fired it would 200 the public doc; the correct behaviour is 400.
    let req = Request::builder()
        .method("GET")
        .uri("/pub")
        .header(
            "dpop",
            axum::http::HeaderValue::from_bytes(&[0xff, 0xfe]).unwrap(),
        )
        .body(Body::empty())
        .unwrap();
    let resp = h.send(req).await;
    assert_eq!(
        resp.status(),
        StatusCode::BAD_REQUEST,
        "a malformed DPoP header (even without Authorization) must be a 400 — the skip falls through \
         to the auth path, never serving it as anonymous"
    );
}

#[tokio::test]
async fn dpop_header_without_authorization_falls_through() {
    let h = Harness::new().await;
    // A GET with NO Authorization but a well-formed `DPoP` header present. The skip falls through (any
    // DPoP header → full path). The full path sees no Authorization → public token → serves the public
    // resource → 200. (The point is the skip does NOT short-circuit; the auth path owns the decision.)
    let req = Request::builder()
        .method("GET")
        .uri("/pub")
        .header("dpop", "not-a-real-proof-but-valid-utf8")
        .body(Body::empty())
        .unwrap();
    let resp = h.send(req).await;
    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "a no-Authorization GET with a valid-UTF8 DPoP header falls through; the public resource is 200"
    );
}
