// AUTHORED-BY Claude Fable 5
//! P1.6 (beyond-50k — `research/lws-design-records.md` §7; RSS
//! `docs/design/beyond-50k-throughput.md` §4) — the **embedded-sparq benchmark config that
//! exercises the backend round-trip counters** on the READ path. The seam-level counters
//! (`research/lws-design-records.md` §7, `tests/read_path_counters.rs`) are re-proven here against
//! the REAL in-process `EmbeddedSparqClient` engine — so the deterministic per-operation backend
//! round-trip counts are the observability the NEXT phase (round-trip reduction) targets, and a
//! regression that adds a round-trip on the embedded path fails.
//!
//! Gated on the opt-in `embedded-sparq` feature (the default build carries no sparq dependency); a
//! no-op test binary when the feature is off.
//!
//! ## Two measurements, two truths (both deterministic)
//! 1. **App-view seam counts** (`counted_router`): the full assembled router (auth → WAC → LDP →
//!    store) over `CountingSparqClient<EmbeddedSparqClient>`. This counts how many times the APP layer
//!    invokes each `SparqClient`/`BlobStore` trait method — `read_plan` is ONE seam call regardless of
//!    backend — so the per-op counts match `read_path_counters.rs`. It proves the counters + the
//!    app-layer round-trip model hold over the real engine (not just the in-memory double), and guards
//!    against a regression that adds a store call on the embedded read path.
//! 2. **True engine round-trips** — the round-trip-reduction WIN, now landed (76ea):
//!    - `embedded_read_plan_override_is_one_engine_round_trip` measures the REAL engine round-trips
//!      of `EmbeddedSparqClient::read_plan` via its own `engine_round_trips()` counter (one increment
//!      per blocking engine dispatch): the combined `VALUES ?g { … } GRAPH ?g { … }` SELECT answers
//!      the whole plan in **ONE** round-trip (down from `1 + N`), and returns the byte-identical
//!      `ReadPlan` the sequential loop would — a round-trip reduction, not a behaviour change.
//!    - `embedded_read_plan_default_loop_would_fan_out_to_one_plus_n` keeps the BASELINE pinned: the
//!      trait DEFAULT (which a client WITHOUT the override inherits) still decomposes a read plan into
//!      `1 (target) + N (ACL candidates)` sequential `get_meta` round-trips — the cost the override
//!      collapses. A `RawSeamCounter` (a seam counter WITHOUT the read_plan override, so the default
//!      fan-out is visible) pins that `1 + N`.
//!
//!    The app-view seam counts in (1) stay FLAT across the win, because `CountingSparqClient::read_plan`
//!    forwards to the wrapped client's override and counts ONE seam query regardless of backend.

#![cfg(feature = "embedded-sparq")]

mod common;

use std::collections::HashSet;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use axum::body::{to_bytes, Body, Bytes};
use axum::http::{header, Request, StatusCode};
use common::{jwks_provider, mint_access_token, mint_dpop_proof, KeyKit, BASE_URL};
use solid_oidc_verifier::config::VerifierConfig;
use solid_oidc_verifier::replay::InMemoryReplayStore;
use solid_oidc_verifier::verifier::Verifier;
use sparq_lws_core::app::{build_router, AppState};
use sparq_lws_core::auth::AuthContext;
use sparq_lws_core::ldp::handler::LdpState;
use sparq_lws_core::store::embedded::EmbeddedSparqClient;
use sparq_lws_core::store::{
    BackendCounters, CompositeStore, CounterSnapshot, CountingBlobStore, CountingSparqClient,
    DeleteOutcome, InMemoryBlobStore, ReadPlan, ResourceMeta, SparqClient, SparqError, Store,
};
use tower::ServiceExt;

const TURTLE: &str =
    "<https://pod.example/alice/c/doc#it> <http://xmlns.com/foaf/0.1/name> \"Doc\" .";

type CountedEmbeddedStore =
    CompositeStore<CountingSparqClient<EmbeddedSparqClient>, CountingBlobStore<InMemoryBlobStore>>;

// ---------------------------------------------------------------------------------------------------
// (1) App-view seam counts — full router over the EMBEDDED engine.
// ---------------------------------------------------------------------------------------------------

/// The counting harness: the assembled router the LDP e2e tests drive, with the counting decorators
/// wrapped around the REAL embedded SPARQ engine + an in-memory blob store, and the shared
/// [`BackendCounters`] exposed.
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
        let embedded = EmbeddedSparqClient::in_memory().expect("empty in-memory graph");
        let store = CompositeStore::new(
            CountingSparqClient::new(embedded, Arc::clone(&counters)),
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

    /// Measure ONE request's backend-counter deltas over an OPERATION-SCOPED window.
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

/// Seed the ROOT `<base>/.acl` owner grant (Read/Write/Control + `acl:default` for descendants) —
/// the same fixture `read_path_counters.rs` uses, so the doc's governing ACL sits at k = 3.
async fn seed_root_owner_acl(store: &CountedEmbeddedStore, base_url: &str, owner_webid: &str) {
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

/// Container `/alice/c/` + doc `/alice/c/doc` inheriting the root ACL (k = 3 for the doc), then one
/// un-measured GET so the parsed-ACL cache is WARM for every measured op.
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
    let warm = h
        .request("GET", "/alice/c/doc", None, &[], Body::empty())
        .await;
    assert_eq!(warm.status(), StatusCode::OK);
}

/// App-view seam counts, warm doc GET (k = 3), over the embedded engine. The app issues ONE
/// `store.read_plan` (seam count 1) + one live found-ACL re-confirm = **2 SPARQL queries**, + **0
/// blob gets** (read-4: the unchanged body is a `(blob_key, etag)` cache hit — was 1) — identical
/// to the in-memory pins (`read_path_counters.rs`), because these are app-layer store-method
/// invocation counts, backend-agnostic. (The embedded engine's TRUE per-read round-trips are
/// higher — see `embedded_read_plan_fans_out`.)
#[tokio::test]
async fn embedded_get_doc_warm_k3_seam_counts_match_in_memory() {
    let h = Harness::new().await;
    fixture(&h).await;

    let (resp, d) = h.measured("GET", "/alice/c/doc", &[]).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    assert_eq!(&body[..], TURTLE.as_bytes());

    assert_eq!(
        d.sparql_queries, 2,
        "warm doc GET seam count = read_plan(1) + found-ACL re-confirm(1) over the embedded engine: {d:?}"
    );
    assert_eq!(
        d.blob_gets, 0,
        "warm doc GET serves the unchanged body from the read-4 cache (was 1): {d:?}"
    );
    assert_eq!(d.sparql_updates, 0);
    assert_eq!(d.blob_puts, 0);
    assert_eq!(d.max_in_flight, 1, "strictly sequential");
}

/// App-view seam counts, warm HEAD (k = 3): same as GET — read_plan(1) + re-confirm(1) = 2 queries,
/// 0 blob gets (read-4 body-cache hit — was 1).
#[tokio::test]
async fn embedded_head_doc_warm_k3_seam_counts() {
    let h = Harness::new().await;
    fixture(&h).await;

    let (resp, d) = h.measured("HEAD", "/alice/c/doc", &[]).await;
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(
        d.sparql_queries, 2,
        "warm HEAD seam count = read_plan(1) + re-confirm(1): {d:?}"
    );
    assert_eq!(
        d.blob_gets, 0,
        "warm HEAD serves the unchanged body from the read-4 cache (was 1): {d:?}"
    );
    assert_eq!(d.max_in_flight, 1);
}

/// App-view seam counts, warm container GET (k = 2): read_plan(1) + found-ACL re-confirm(1) + ONE
/// membership listing(1) = **3 queries**, **0 blob gets** (read-4 body-cache hit — was 1) —
/// matching `read_path_counters.rs`.
#[tokio::test]
async fn embedded_get_container_warm_seam_counts() {
    let h = Harness::new().await;
    fixture(&h).await;

    // First-ever read of the container itself (the fixture warmed only the doc): COLD stored body.
    let (resp, cold) = h.measured("GET", "/alice/c/", &[]).await;
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(
        cold.sparql_queries, 3,
        "container GET seam count = read_plan(1) + re-confirm(1) + ONE listing(1): {cold:?}"
    );
    assert_eq!(
        cold.blob_gets, 1,
        "the container's FIRST read pays its stored-body fetch: {cold:?}"
    );
    assert_eq!(cold.max_in_flight, 1);

    // WARM: the stored body is a read-4 cache hit (0 blob gets — was 1); the listing still renders
    // from LIVE membership (query counted above), so no stale member list can be served.
    let (resp, warm) = h.measured("GET", "/alice/c/", &[]).await;
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(warm.sparql_queries, 3, "warm container queries: {warm:?}");
    assert_eq!(
        warm.blob_gets, 0,
        "warm container stored-body is a read-4 cache hit (was 1): {warm:?}"
    );
    assert_eq!(warm.max_in_flight, 1);
}

/// The no-N+1 pin over the embedded engine: a container LISTING is ONE membership query independent
/// of child count — adding children must not change the per-read seam count.
#[tokio::test]
async fn embedded_container_listing_seam_count_independent_of_child_count() {
    let h = Harness::new().await;
    fixture(&h).await;

    // Warm the container's stored-body cache (un-measured) so BOTH measured reads are the same
    // WARM shape — the equality then isolates child-count independence from cache warm-up.
    let warm = h
        .request("GET", "/alice/c/", None, &[], Body::empty())
        .await;
    assert_eq!(warm.status(), StatusCode::OK);

    let (resp, one_child) = h.measured("GET", "/alice/c/", &[]).await;
    assert_eq!(resp.status(), StatusCode::OK);

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
        "listing seam queries must be independent of child count: {one_child:?} vs {five_children:?}"
    );
    assert_eq!(five_children.blob_gets, one_child.blob_gets);
}

/// App-view 304 path (warm, k = 3): read_plan(1) + re-confirm(1) = 2 queries; the body is still
/// materialised but the unchanged bytes are a read-4 cache hit (0 blob gets — was 1).
#[tokio::test]
async fn embedded_get_304_warm_seam_counts() {
    let h = Harness::new().await;
    fixture(&h).await;

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
        "304 path seam count = read_plan(1) + re-confirm(1): {d:?}"
    );
    assert_eq!(
        d.blob_gets, 0,
        "304 path serves the unchanged bytes from the read-4 cache (was 1): {d:?}"
    );
    assert_eq!(d.max_in_flight, 1);
}

// ---------------------------------------------------------------------------------------------------
// (2) True engine round-trips — the combined read_plan WIN (1) vs the trait-default FAN-OUT (1 + N).
// ---------------------------------------------------------------------------------------------------

/// A [`SparqClient`] seam decorator that counts `get_meta` round-trips (via its own plain atomic —
/// independent of the `BackendCounters` seam used above) and — crucially — does NOT override
/// `read_plan`. So a `read_plan` call runs the trait DEFAULT (which loops `self.get_meta` over the
/// target + every ACL candidate), and each of those `get_meta` calls IS counted here. This makes the
/// embedded backend's true per-candidate read-plan fan-out visible at the seam (unlike
/// `CountingSparqClient`, whose `read_plan` override reports 1 and forwards the fan-out uncounted
/// into the inner engine). Every other method forwards transparently.
struct RawSeamCounter<S: SparqClient> {
    inner: S,
    get_meta_calls: Arc<AtomicU64>,
}

impl<S: SparqClient> RawSeamCounter<S> {
    fn new(inner: S) -> (Self, Arc<AtomicU64>) {
        let get_meta_calls = Arc::new(AtomicU64::new(0));
        (
            Self {
                inner,
                get_meta_calls: Arc::clone(&get_meta_calls),
            },
            get_meta_calls,
        )
    }
}

#[async_trait]
impl<S: SparqClient> SparqClient for RawSeamCounter<S> {
    async fn get_meta(&self, iri: &str) -> Result<ResourceMeta, SparqError> {
        self.get_meta_calls.fetch_add(1, Ordering::Relaxed);
        self.inner.get_meta(iri).await
    }
    async fn put_meta(&self, iri: &str, meta: ResourceMeta) -> Result<(), SparqError> {
        self.inner.put_meta(iri, meta).await
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
    async fn referenced_blob_keys(&self) -> Result<HashSet<String>, SparqError> {
        self.inner.referenced_blob_keys().await
    }
    // NB: no `read_plan` override — the trait default fans out counted `get_meta` calls.
}

fn meta(bk: &str) -> ResourceMeta {
    ResourceMeta {
        content_type: "text/turtle".into(),
        blob_key: bk.into(),
        etag: "\"e1\"".into(),
        last_modified: None,
    }
}

/// **The BASELINE the combined override collapses.** A `SparqClient` WITHOUT a combined `read_plan`
/// (here surfaced by `RawSeamCounter`, which does not override it) inherits the trait default: a plan
/// over a target + `N` ACL candidates costs exactly `1 + N` sequential `get_meta` round-trips. Here
/// `N = 4` (a doc at depth k = 3: `doc.acl`, `c/.acl`, `alice/.acl`, `/.acl`), so **5 queries**,
/// strictly sequential. This pins the cost `EmbeddedSparqClient::read_plan`'s one combined SELECT now
/// avoids (see `embedded_read_plan_override_is_one_engine_round_trip`).
#[tokio::test]
async fn embedded_read_plan_default_loop_would_fan_out_to_one_plus_n() {
    let (raw, get_meta_calls) =
        RawSeamCounter::new(EmbeddedSparqClient::in_memory().expect("empty in-memory graph"));

    // Seed the target doc + the governing root ACL (both un-measured writes: put_meta, not counted).
    let target = "https://pod.example/alice/c/doc";
    let root_acl = "https://pod.example/.acl";
    raw.put_meta(target, meta("doc-blob")).await.unwrap();
    raw.put_meta(root_acl, meta("acl-blob")).await.unwrap();
    assert_eq!(
        get_meta_calls.load(Ordering::Relaxed),
        0,
        "seeding uses put_meta, not get_meta — the counter starts clean"
    );

    // The child→root ACL candidate list for the doc at k = 3 (the WAC walk's candidate set).
    let candidates: Vec<String> = [
        "https://pod.example/alice/c/doc.acl",
        "https://pod.example/alice/c/.acl",
        "https://pod.example/alice/.acl",
        "https://pod.example/.acl",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();

    let plan: ReadPlan = raw.read_plan(target, &candidates).await.unwrap();

    // The plan is correct (target present; only the root ACL present among candidates) …
    assert!(plan.target.is_some(), "target metadata present in the plan");
    let present: Vec<&String> = plan
        .acls
        .iter()
        .filter(|(_, etag)| etag.is_some())
        .map(|(iri, _)| iri)
        .collect();
    assert_eq!(
        present,
        vec![root_acl],
        "only the root ACL is present among the candidates"
    );

    // … and it cost 1 (target) + N (candidates) = 5 sequential get_meta round-trips. The default
    // read_plan awaits each get_meta before the next (a sequential `for` loop), so the round-trip
    // count IS the sequential RTT depth. `EmbeddedSparqClient::read_plan` now collapses this to ONE
    // combined round-trip (`embedded_read_plan_override_is_one_engine_round_trip`); this test keeps
    // the `1 + N` cost of the trait default pinned as the baseline that win is measured against.
    assert_eq!(
        get_meta_calls.load(Ordering::Relaxed),
        1 + candidates.len() as u64,
        "trait-default read_plan = 1 target + N candidate get_meta round-trips (N={})",
        candidates.len()
    );
}

/// **The round-trip-reduction WIN (76ea), measured on the REAL engine.** `EmbeddedSparqClient` now
/// overrides `read_plan` with ONE combined `VALUES ?g { … } GRAPH ?g { … }` SELECT. Its
/// `engine_round_trips()` counter (one increment per blocking engine dispatch — the embedded analogue
/// of a network round-trip) proves the whole plan over a target + `N = 4` ACL candidates costs
/// exactly **1** round-trip, down from the `1 + N = 5` the trait default (pinned above) would spend.
/// And the returned `ReadPlan` is byte-identical to what the sequential loop produces — a round-trip
/// reduction, NOT a behaviour change (the ACL candidate set the override resolves is the SAME set the
/// WAC walk would see, so no authz decision can differ).
#[tokio::test]
async fn embedded_read_plan_override_is_one_engine_round_trip() {
    let client = EmbeddedSparqClient::in_memory().expect("empty in-memory graph");

    let target = "https://pod.example/alice/c/doc";
    let root_acl = "https://pod.example/.acl";
    client.put_meta(target, meta("doc-blob")).await.unwrap();
    client.put_meta(root_acl, meta("acl-blob")).await.unwrap();

    let candidates: Vec<String> = [
        "https://pod.example/alice/c/doc.acl",
        "https://pod.example/alice/c/.acl",
        "https://pod.example/alice/.acl",
        "https://pod.example/.acl",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();

    // The OVERRIDE: exactly ONE engine round-trip for the whole plan.
    let before = client.engine_round_trips();
    let plan: ReadPlan = client.read_plan(target, &candidates).await.unwrap();
    assert_eq!(
        client.engine_round_trips() - before,
        1,
        "combined read_plan is ONE engine round-trip (was 1 + N = {} via the default loop)",
        1 + candidates.len()
    );

    // The plan is correct: target present; only the root ACL present among the candidates.
    assert!(plan.target.is_some(), "target metadata present");
    let present: Vec<&String> = plan
        .acls
        .iter()
        .filter(|(_, etag)| etag.is_some())
        .map(|(iri, _)| iri)
        .collect();
    assert_eq!(present, vec![root_acl], "only the root ACL is present");

    // Semantic identity: the combined plan equals the sequential-loop plan (same metas + ACL set).
    let mut loop_acls = Vec::with_capacity(candidates.len());
    for cand in &candidates {
        let etag = match client.get_meta(cand).await {
            Ok(m) => Some(m.etag),
            Err(SparqError::NotFound) => None,
            Err(e) => panic!("unexpected error: {e}"),
        };
        loop_acls.push((cand.clone(), etag));
    }
    let loop_target = client.get_meta(target).await.ok();
    assert_eq!(
        plan,
        ReadPlan {
            target: loop_target,
            acls: loop_acls,
        },
        "combined read_plan is semantically identical to the 1 + N loop"
    );
}
