// AUTHORED-BY Claude Fable 5
//! read-1 (`research/lws-design-records.md` §7; RSS `docs/design/backend-read-path.md` §7 — bare
//! `§N` references below are sections of that RSS document, which is not in this tree): PINNED
//! deterministic backend round-trip counts per read operation, measured end-to-end through the
//! assembled router (auth → WAC → LDP → store) with the counting decorators at the
//! `SparqClient`/`BlobStore` seams.
//!
//! These are the §1.1 RTT-model counts made executable: **exact integer pins** (the repo's
//! perf-gate discipline — deterministic metrics hard-gated, wall-clock advisory, never asserted).
//! `max_in_flight == 1` in every scenario is the await-depth witness: every backend call strictly
//! awaited the previous one, so the op's sequential RTT depth EQUALS the pinned call totals.
//!
//! Terminology (matching §1.1): a resource whose governing ACL sits at ancestor index *k*
//! (0 = its own `.acl`). BEFORE read-2 (pinned at the read-1 commit, `git log` this file) a read
//! cost `k+1` sequential ACL probes + 1 target meta = **k+2 SPARQL queries** warm (k+3 cold), +1
//! blob get (+1 cold), +1 query for a container listing. AFTER read-2 (the §3.1 combined read-plan
//! query) the O(depth) ACL WALK collapses into ONE combined query; AFTER read-4 (the §3.4
//! `(blob_key, etag)`-keyed blob-BODY cache) a warm same-version read pays ZERO blob gets. The pins
//! below are the AFTER table, each test recording its before→after delta (the deterministic
//! evidence of both wins):
//!
//!   op                         queries before → after   blob gets before → after (read-4)
//!   doc GET  warm (any k)              k+2 → 2              1 → 0   (body-cache hit)
//!   doc GET  cold ACL                  k+3 → 2              2 → 1   (fresh ACL bytes only; warm target body hits)
//!   HEAD     warm                      k+2 → 2              1 → 0
//!   GET 304  warm                      k+2 → 2              1 → 0
//!   container GET warm                 k+3 → 3              1 → 0   (plan + found-ACL re-confirm + listing)
//!   doc GET  first-ever (cold body)          2              1       (the fetch that populates the cache)
//!   doc GET  after a REWRITE (new etag)      2              1       (new (blob_key, etag) ⇒ MISS — never stale)
//!
//! Depth-independence is the point: the per-read query count no longer scales with k — it is a flat
//! `plan(1) + found-ACL existence re-confirm(1) [+ container listing(1)]`. The found-ACL re-confirm
//! is a LIVE index probe (`WacAuthorizer::read_acl_confirmed`), REQUIRED for fail-closed-on-delete:
//! the combined query's plan-time etag is not a safe cache gate (a delete-after-plan would grant
//! from a stale cache), so the ONE governing ACL is re-confirmed live while the k absent-candidate
//! probes stay collapsed into the plan. The walk-collapse win is real and depth-independent; the
//! honest warm count is 2, not 1 (an earlier pin of 1 trusted the plan-time etag, the roborev
//! Medium). read-3 removed the cold duplicate `get_meta`; read-4 (this table's blob column) serves
//! a hot unchanged body from the `(blob_key, etag)` LRU — a hit is provably current because the
//! lookup key comes from THIS request's authoritative read-plan round and blob keys are minted
//! unique per write (a rewrite ⇒ new key ⇒ miss; see `src/store/body_cache.rs`), and it can never
//! bypass WAC (authorization runs BEFORE the body fetch, hit or miss — pinned by
//! `warm_body_cache_never_serves_an_unauthorized_request` below).

mod common;

use std::sync::Arc;

use axum::body::{to_bytes, Body};
use axum::http::{header, Request, StatusCode};
use common::{jwks_provider, mint_access_token, mint_dpop_proof, KeyKit, BASE_URL};
use solid_oidc_verifier::config::VerifierConfig;
use solid_oidc_verifier::replay::InMemoryReplayStore;
use solid_oidc_verifier::verifier::Verifier;
use sparq_lws_core::app::{build_router, AppState};
use sparq_lws_core::auth::AuthContext;
use sparq_lws_core::ldp::handler::LdpState;
use sparq_lws_core::store::{
    BackendCounters, CompositeStore, CounterSnapshot, CountingBlobStore, CountingSparqClient,
    InMemoryBlobStore, InMemorySparqClient, Store,
};
use tower::ServiceExt;

const TURTLE: &str =
    "<https://pod.example/alice/c/doc#it> <http://xmlns.com/foaf/0.1/name> \"Doc\" .";

type CountedStore =
    CompositeStore<CountingSparqClient<InMemorySparqClient>, CountingBlobStore<InMemoryBlobStore>>;

/// The counting harness: the same assembled router the LDP e2e tests drive, with the counting
/// decorators wrapped around the in-memory backends and the shared [`BackendCounters`] exposed.
struct Harness {
    app: axum::Router,
    issuer_key: KeyKit,
    client_key: KeyKit,
    counters: Arc<BackendCounters>,
}

impl Harness {
    async fn new() -> Self {
        let issuer_key = KeyKit::generate();
        let client_key = KeyKit::generate();
        let config = VerifierConfig::new(vec![common::ISSUER.to_string()], BASE_URL);
        let replay = InMemoryReplayStore::with_window(config.replay_ttl());
        let verifier = Verifier::new(config, jwks_provider(&issuer_key), replay).unwrap();
        let ctx = AuthContext::new(verifier, BASE_URL);

        let counters = BackendCounters::new();
        let store = CompositeStore::new(
            CountingSparqClient::new(InMemorySparqClient::new(), Arc::clone(&counters)),
            CountingBlobStore::new(InMemoryBlobStore::new(), Arc::clone(&counters)),
        );
        seed_root_owner_acl(&store, BASE_URL, common::WEBID).await;
        let ldp = LdpState::new(store, BASE_URL);
        let app = build_router(AppState::new(ctx, ldp));
        Self {
            app,
            issuer_key,
            client_key,
            counters,
        }
    }

    async fn request(
        &self,
        method: &str,
        path: &str,
        content_type: Option<&str>,
        extra: &[(&str, &str)],
        body: Body,
    ) -> axum::http::Response<Body> {
        let access = mint_access_token(&self.issuer_key, &self.client_key.thumbprint);
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
        for (k, v) in extra {
            builder = builder.header(*k, *v);
        }
        self.app
            .clone()
            .oneshot(builder.body(body).unwrap())
            .await
            .unwrap()
    }

    /// Measure ONE request's backend-counter deltas over an OPERATION-SCOPED window (`measure()`),
    /// so `max_in_flight` is this request's peak concurrency — NOT a global high-water that a prior
    /// (e.g. the fixture's PUTs) overlapping op could contaminate.
    async fn measured(
        &self,
        method: &str,
        path: &str,
        extra: &[(&str, &str)],
    ) -> (axum::http::Response<Body>, CounterSnapshot) {
        let scope = self.counters.measure();
        let resp = self.request(method, path, None, extra, Body::empty()).await;
        (resp, scope.delta())
    }
}

/// Seed a ROOT `<base>/.acl` granting the test WebID Read/Write/Control on the root and (via
/// `acl:default`) all descendants — the same pod-root owner-default the LDP e2e harness seeds.
async fn seed_root_owner_acl(store: &CountedStore, base_url: &str, owner_webid: &str) {
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

/// Build the standard fixture: container `/alice/c/` + document `/alice/c/doc`, all inheriting the
/// root owner ACL (so the doc's governing ACL sits at ancestor index k = 3:
/// candidates = doc.acl → /alice/c/.acl → /alice/.acl → /.acl ✓present).
async fn fixture(h: &Harness) {
    let mk = h
        .request(
            "PUT",
            "/alice/c/",
            Some("text/turtle"),
            &[],
            Body::from("<#c> <http://xmlns.com/foaf/0.1/name> \"C\" ."),
        )
        .await;
    assert_eq!(mk.status(), StatusCode::CREATED);
    let put = h
        .request(
            "PUT",
            "/alice/c/doc",
            Some("text/turtle"),
            &[],
            Body::from(TURTLE),
        )
        .await;
    assert_eq!(put.status(), StatusCode::CREATED);
    // Warm the parsed-ACL cache (the PUTs above already resolved + cached the root ACL, but be
    // explicit: one un-measured GET so every measured request below is the WARM path).
    let warm = h
        .request("GET", "/alice/c/doc", None, &[], Body::empty())
        .await;
    assert_eq!(warm.status(), StatusCode::OK);
}

/// §3.7 row 1 — **doc GET, warm (k = 3)**: the O(depth) ACL walk collapses into ONE combined
/// read-plan query; the ONE governing ACL is then re-confirmed live (fail-closed on delete) — so
/// **2 SPARQL queries** (plan + found-ACL re-confirm), + **0 blob gets** (read-4: the unchanged
/// body is a `(blob_key, etag)` cache HIT — was 1 before the body cache, k+2 = 5 queries before
/// read-2). The pin is DEPTH-INDEPENDENT (flat 2 at any k). `max_in_flight == 1` ⇒ RTT depth = 2.
#[tokio::test]
async fn get_doc_warm_k3_pins_plan_plus_confirm_query_0_blob_gets() {
    let h = Harness::new().await;
    fixture(&h).await;

    let (resp, d) = h.measured("GET", "/alice/c/doc", &[]).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    assert_eq!(&body[..], TURTLE.as_bytes());

    assert_eq!(
        d.sparql_queries, 2,
        "warm doc GET = plan + found-ACL live re-confirm, depth-independent (was k+2 = 5): {d:?}"
    );
    assert_eq!(
        d.blob_gets, 0,
        "warm doc GET serves the unchanged body from the read-4 cache (was 1): {d:?}"
    );
    assert_eq!(d.sparql_updates, 0);
    assert_eq!(d.blob_puts, 0);
    assert_eq!(d.max_in_flight, 1, "strictly sequential ⇒ RTT depth = 2");
}

/// §3.7 rows 1+3 — **doc GET with an OWN `.acl` (k = 0), cold then warm**: cold = 1 combined
/// read-plan query + 1 live found-ACL re-confirm (its parse-cache miss fetches the bytes via
/// `read_at` — no duplicate `get_meta`) = **2 queries** + **1 blob get** (the FRESH ACL's bytes;
/// the target body — unchanged since the fixture's warm GET — is a read-4 body-cache HIT; before
/// the body cache this was 2 blob gets); warm = **2 queries** (plan + the cache-hit re-confirm) +
/// **0 blob gets** (ACL parse cached, target body cached — was 1). The found-ACL re-confirm is the
/// fail-closed-on-delete probe (was, insecurely, elided to warm = 1).
#[tokio::test]
async fn get_doc_own_acl_cold_then_warm_pins_k0_counts() {
    let h = Harness::new().await;
    fixture(&h).await;

    // Give the doc its OWN ACL (owner full access) — the governing ACL moves to k = 0, and its
    // parse is NOT yet cached (the PUT writes bytes; only a resolve parses).
    let own_acl = format!(
        r#"@prefix acl: <http://www.w3.org/ns/auth/acl#>.
<#o> a acl:Authorization; acl:agent <{owner}>;
     acl:accessTo <https://pod.example/alice/c/doc>;
     acl:mode acl:Read, acl:Write, acl:Control."#,
        owner = common::WEBID
    );
    let put = h
        .request(
            "PUT",
            "/alice/c/doc.acl",
            Some("text/turtle"),
            &[],
            Body::from(own_acl),
        )
        .await;
    assert!(put.status().is_success(), "PUT own acl: {}", put.status());

    // COLD: the fresh own-ACL's first resolve reads + parses it.
    let (resp, cold) = h.measured("GET", "/alice/c/doc", &[]).await;
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(
        cold.sparql_queries, 2,
        "cold doc GET = plan + found-ACL live re-confirm (parse miss reads via read_at): {cold:?}"
    );
    assert_eq!(
        cold.blob_gets, 1,
        "cold pays only the FRESH ACL's byte-fetch — the unchanged target body is a read-4 \
         body-cache hit (was 2 before the body cache): {cold:?}"
    );
    assert_eq!(cold.max_in_flight, 1);

    // WARM: the ACL parse is cached under its etag AND the target body under its (blob_key, etag).
    let (resp, warm) = h.measured("GET", "/alice/c/doc", &[]).await;
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(
        warm.sparql_queries, 2,
        "warm doc GET = plan + found-ACL cache-hit re-confirm (was k+2 = 2): {warm:?}"
    );
    assert_eq!(
        warm.blob_gets, 0,
        "warm serves the unchanged target body from the read-4 cache (was 1): {warm:?}"
    );
    assert_eq!(warm.max_in_flight, 1);
}

/// **HEAD, warm (k = 3)** — same backend cost as GET (the read path materialises the bytes for
/// HEAD too): **2 queries** (plan + found-ACL re-confirm, was k+2 = 5), **0 blob gets** (read-4:
/// the unchanged body is a cache hit — was 1).
#[tokio::test]
async fn head_doc_warm_k3_pins_same_as_get() {
    let h = Harness::new().await;
    fixture(&h).await;

    let (resp, d) = h.measured("HEAD", "/alice/c/doc", &[]).await;
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(
        d.sparql_queries, 2,
        "warm HEAD = plan + found-ACL re-confirm (was k+2 = 5): {d:?}"
    );
    assert_eq!(
        d.blob_gets, 0,
        "warm HEAD serves the unchanged body from the read-4 cache (was 1): {d:?}"
    );
    assert_eq!(d.max_in_flight, 1);
}

/// **304 path, warm (k = 3)** — a matching `If-None-Match` returns 304; the metadata cost is
/// **2 queries** (plan + found-ACL re-confirm, was k+2 = 5). The body is still materialised (the
/// precondition is evaluated after the read) but the unchanged bytes are a read-4 body-cache HIT —
/// **0 blob gets** (was 1).
#[tokio::test]
async fn get_304_warm_k3_pins_plan_plus_confirm_query() {
    let h = Harness::new().await;
    fixture(&h).await;

    // Learn the current ETag.
    let (resp, _) = h.measured("GET", "/alice/c/doc", &[]).await;
    let etag = resp
        .headers()
        .get(header::ETAG)
        .expect("etag")
        .to_str()
        .unwrap()
        .to_string();

    let (resp, d) = h
        .measured("GET", "/alice/c/doc", &[("if-none-match", etag.as_str())])
        .await;
    assert_eq!(resp.status(), StatusCode::NOT_MODIFIED);
    assert_eq!(
        d.sparql_queries, 2,
        "304 path = plan + found-ACL re-confirm (was k+2 = 5): {d:?}"
    );
    assert_eq!(
        d.blob_gets, 0,
        "304 path serves the unchanged bytes from the read-4 cache (was 1): {d:?}"
    );
    assert_eq!(d.max_in_flight, 1);
}

/// §3.7 row 4 — **container GET, warm (k = 2)**: 1 combined read-plan query + 1 found-ACL live
/// re-confirm + 1 membership listing = **3 queries** (was k+3 = 5; the §3.1 membership-fold that
/// would drop the listing is read-5, measure-first), **1 blob get**. The listing stays ONE query at
/// any child count (no-N+1).
#[tokio::test]
async fn get_container_warm_k2_pins_plan_confirm_listing_queries() {
    let h = Harness::new().await;
    fixture(&h).await;

    // First-ever read of the container itself (the fixture warmed only the doc): COLD body — the
    // 1 blob get that populates the read-4 cache.
    let (resp, cold) = h.measured("GET", "/alice/c/", &[]).await;
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(
        cold.sparql_queries, 3,
        "container GET = plan + found-ACL re-confirm + ONE membership listing (was k+3 = 5): {cold:?}"
    );
    assert_eq!(
        cold.blob_gets, 1,
        "the container's FIRST read pays its stored-body fetch: {cold:?}"
    );
    assert_eq!(cold.max_in_flight, 1);

    // WARM: same queries, and the container's stored body is a read-4 cache hit (0 blob gets —
    // was 1). The LISTING is still rendered from LIVE membership (its query is counted above), so
    // the cached stored bytes can never serve a stale member list.
    let (resp, warm) = h.measured("GET", "/alice/c/", &[]).await;
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(warm.sparql_queries, 3, "warm container queries: {warm:?}");
    assert_eq!(
        warm.blob_gets, 0,
        "warm container stored-body is a read-4 cache hit (was 1): {warm:?}"
    );
    assert_eq!(warm.max_in_flight, 1);
}

/// The no-N+1 pin (§7): a container LISTING is ONE membership query **independent of child
/// count** — adding children must not change the per-read query count.
#[tokio::test]
async fn container_listing_query_count_is_independent_of_child_count() {
    let h = Harness::new().await;
    fixture(&h).await;

    // Warm the container's stored-body cache (un-measured) so BOTH measured reads below are the
    // same WARM shape — the equality then isolates child-count independence from cache warm-up.
    let warm = h
        .request("GET", "/alice/c/", None, &[], Body::empty())
        .await;
    assert_eq!(warm.status(), StatusCode::OK);

    // Baseline: 1 child (the fixture doc).
    let (resp, one_child) = h.measured("GET", "/alice/c/", &[]).await;
    assert_eq!(resp.status(), StatusCode::OK);

    // Add 4 more children.
    for i in 0..4 {
        let put = h
            .request(
                "PUT",
                &format!("/alice/c/doc{i}"),
                Some("text/turtle"),
                &[],
                Body::from(format!(
                    "<https://pod.example/alice/c/doc{i}#it> <http://xmlns.com/foaf/0.1/name> \"D{i}\" ."
                )),
            )
            .await;
        assert_eq!(put.status(), StatusCode::CREATED);
    }

    let (resp, five_children) = h.measured("GET", "/alice/c/", &[]).await;
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(
        five_children.sparql_queries, one_child.sparql_queries,
        "listing queries must be independent of child count: {one_child:?} vs {five_children:?}"
    );
    assert_eq!(five_children.blob_gets, one_child.blob_gets);
}

// ---------------------------------------------------------------------------------------------------
// read-4 (§3.4) — the blob-body cache: the cold/warm/rewrite measurement + the two critical
// adversarial properties (no stale serve, no authz bypass).
// ---------------------------------------------------------------------------------------------------

/// **The read-4 measurement, pinned end-to-end: blob-gets/op = 1 cold → 0 warm → 1 after the etag
/// changes — and the post-change body is the NEW bytes, never the old.** A fresh resource's first
/// read pays the blob fetch (populating the cache); an unchanged repeat read is a `(blob_key, etag)`
/// HIT (0 blob gets, byte-identical body); a REWRITE commits a new blob key + etag into the index,
/// so the next read's authoritative metadata can only MISS (1 blob get) and serves the rewritten
/// bytes — the stale-serve the etag/unique-key keying makes impossible by construction.
#[tokio::test]
async fn body_cache_cold_warm_then_rewrite_pins_1_0_1_and_never_serves_stale() {
    let h = Harness::new().await;
    fixture(&h).await;

    const V1: &str =
        "<https://pod.example/alice/c/fresh#it> <http://xmlns.com/foaf/0.1/name> \"version-one\" .";
    const V2: &str = "<https://pod.example/alice/c/fresh#it> <http://xmlns.com/foaf/0.1/name> \"VERSION-TWO-different-bytes\" .";

    // A NEVER-READ resource (writes do not populate the cache — only a read's miss does).
    let put = h
        .request(
            "PUT",
            "/alice/c/fresh",
            Some("text/turtle"),
            &[],
            Body::from(V1),
        )
        .await;
    assert_eq!(put.status(), StatusCode::CREATED);

    // COLD: 1 blob get — the fetch that populates the cache.
    let (resp, cold) = h.measured("GET", "/alice/c/fresh", &[]).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    assert_eq!(&body[..], V1.as_bytes());
    assert_eq!(cold.blob_gets, 1, "cold read pays the blob fetch: {cold:?}");

    // WARM (same etag): 0 blob gets, byte-identical body.
    let (resp, warm) = h.measured("GET", "/alice/c/fresh", &[]).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    assert_eq!(
        &body[..],
        V1.as_bytes(),
        "a hit is byte-identical to the fetch"
    );
    assert_eq!(
        warm.blob_gets, 0,
        "warm same-etag read is a body-cache hit (the read-4 win, 1 → 0): {warm:?}"
    );

    // REWRITE: new bytes ⇒ the index now holds a NEW (blob_key, etag) pair.
    let rewrite = h
        .request(
            "PUT",
            "/alice/c/fresh",
            Some("text/turtle"),
            &[],
            Body::from(V2),
        )
        .await;
    assert!(
        rewrite.status().is_success(),
        "rewrite PUT: {}",
        rewrite.status()
    );

    // POST-CHANGE: the authoritative metadata names the new key ⇒ a MISS (1 blob get) — and the
    // served body MUST be the new bytes. Serving V1 here would be the stale-serve bug this cache's
    // keying exists to make impossible.
    let (resp, after) = h.measured("GET", "/alice/c/fresh", &[]).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    assert_eq!(
        &body[..],
        V2.as_bytes(),
        "a changed resource must NEVER serve the old cached body"
    );
    assert_eq!(
        after.blob_gets, 1,
        "the new (blob_key, etag) is a guaranteed miss — 1 fresh fetch: {after:?}"
    );

    // And the NEW version is now itself cached: warm again = 0.
    let (resp, warm2) = h.measured("GET", "/alice/c/fresh", &[]).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    assert_eq!(&body[..], V2.as_bytes());
    assert_eq!(
        warm2.blob_gets, 0,
        "the rewritten body caches too: {warm2:?}"
    );
}

/// **A Range request over a CACHED body slices correctly (0 blob gets).** The handler slices the
/// rendered full body (`range::evaluate`) — a cache hit hands it the same full `Bytes` a blob fetch
/// would, so the 206 slice is byte-identical and the Content-Range math unchanged.
#[tokio::test]
async fn range_request_over_a_cached_body_slices_correctly_with_0_blob_gets() {
    let h = Harness::new().await;
    fixture(&h).await;

    // The fixture's warm GET already populated the cache for /alice/c/doc.
    let (resp, d) = h
        .measured("GET", "/alice/c/doc", &[("range", "bytes=0-9")])
        .await;
    assert_eq!(resp.status(), StatusCode::PARTIAL_CONTENT);
    let content_range = resp
        .headers()
        .get(header::CONTENT_RANGE)
        .expect("content-range")
        .to_str()
        .unwrap()
        .to_string();
    assert_eq!(
        content_range,
        format!("bytes 0-9/{}", TURTLE.len()),
        "Content-Range math over the cached full body is unchanged"
    );
    let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    assert_eq!(
        &body[..],
        &TURTLE.as_bytes()[0..10],
        "the 206 slice over a cached body is byte-identical to a fetched one"
    );
    assert_eq!(
        d.blob_gets, 0,
        "the Range read served the slice from the cached body: {d:?}"
    );
}

/// **A warm body cache can never serve an UNAUTHORIZED request (the authz-bypass adversarial
/// check).** The cache sits inside the store, BELOW the WAC gate: `serve_read` authorizes BEFORE
/// any body lookup, hit or miss. So after the owner has warmed the cache for a private resource, an
/// ANONYMOUS request for the same resource is still 401 — and (design invariant 5: no speculative
/// byte work for a denied request) it triggers ZERO blob gets, cached or not.
#[tokio::test]
async fn warm_body_cache_never_serves_an_unauthorized_request() {
    let h = Harness::new().await;
    fixture(&h).await; // the owner's warm GET has populated the cache for /alice/c/doc

    // Sanity: the owner's warm read IS a cache hit (the cache is live for this resource).
    let (resp, owner) = h.measured("GET", "/alice/c/doc", &[]).await;
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(
        owner.blob_gets, 0,
        "owner warm read hits the cache: {owner:?}"
    );

    // The ADVERSARIAL probe: the SAME resource, ANONYMOUS (no Authorization/DPoP). The root ACL
    // grants only the owner, so WAC denies with 401 — the warm cache must change NOTHING about
    // that, and no body may be touched for the denied request.
    let scope = h.counters.measure();
    let resp = h
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/alice/c/doc")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let anon = scope.delta();
    assert_eq!(
        resp.status(),
        StatusCode::UNAUTHORIZED,
        "an anonymous read of a private resource stays 401 with a warm body cache"
    );
    assert_eq!(
        anon.blob_gets, 0,
        "a denied request performs no blob fetch (no speculative byte work): {anon:?}"
    );
}

/// **A DELETE is never resurrected from the body cache, and a RECREATE serves the new bytes.**
/// Existence is decided by the authoritative index BEFORE any body lookup, so a deleted resource
/// 404s regardless of what the cache still holds; a recreate mints a fresh blob key + etag, so its
/// first read is a miss that fetches the NEW bytes (never the tombstoned entry).
#[tokio::test]
async fn deleted_resource_404s_and_a_recreate_never_serves_the_old_cached_body() {
    let h = Harness::new().await;
    fixture(&h).await; // /alice/c/doc read + cached (TURTLE)

    let del = h
        .request("DELETE", "/alice/c/doc", None, &[], Body::empty())
        .await;
    assert_eq!(del.status(), StatusCode::NO_CONTENT);

    // The cache still physically holds the old entry — but the index says GONE, so 404 (and no
    // blob work: the 404 decision precedes any byte fetch).
    let (resp, d) = h.measured("GET", "/alice/c/doc", &[]).await;
    assert_eq!(
        resp.status(),
        StatusCode::NOT_FOUND,
        "a deleted resource must 404 — the body cache can never resurrect it"
    );
    assert_eq!(
        d.blob_gets, 0,
        "no byte fetch for an absent resource: {d:?}"
    );

    // RECREATE with different bytes: a fresh (blob_key, etag) ⇒ first read is a MISS serving the
    // NEW body, never the old cached one.
    const RECREATED: &str =
        "<https://pod.example/alice/c/doc#it> <http://xmlns.com/foaf/0.1/name> \"recreated\" .";
    let put = h
        .request(
            "PUT",
            "/alice/c/doc",
            Some("text/turtle"),
            &[],
            Body::from(RECREATED),
        )
        .await;
    assert_eq!(put.status(), StatusCode::CREATED);
    let (resp, d) = h.measured("GET", "/alice/c/doc", &[]).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    assert_eq!(
        &body[..],
        RECREATED.as_bytes(),
        "a recreate must serve its own bytes, never the pre-delete cached body"
    );
    assert_eq!(d.blob_gets, 1, "the recreate's first read is a miss: {d:?}");
}
