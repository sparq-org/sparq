// AUTHORED-BY Claude Fable 5
//! write-1 (`research/lws-design-records.md` §7; the RSS `docs/design/backend-read-path.md` §7
//! discipline, applied to the WRITE verbs): PINNED deterministic backend round-trip counts per
//! WRITE operation, measured end-to-end through the assembled router (auth → WAC → LDP → store)
//! with the counting decorators at the `SparqClient`/`BlobStore` seams — the write-path sibling of
//! `read_path_counters.rs`.
//!
//! These pins are the MEASURE-FIRST baseline for the write-path walk-collapse round: the read path
//! (read-2) already folds its O(depth) ACL walk into ONE combined `Store::read_plan` query, but the
//! WRITE verbs (PUT/POST/DELETE/PATCH) still authorize via the SEQUENTIAL
//! `WacAuthorizer::authorize` walk — k+1 per-candidate `meta` probes for a resource whose governing
//! ACL sits at ancestor index k — plus, on the CREATE paths, a second sequential walk from the
//! parent and per-ancestor existence probes.
//!
//! Fixture depth matches the read tests: the governing ACL is the root `<base>/.acl`, so a doc at
//! `/alice/c/doc` has k = 3 (candidates doc.acl → /alice/c/.acl → /alice/.acl → /.acl ✓).
//!
//! Pinned counts AFTER write-2 (the planned-walk collapse: each sequential walk of k+1 probes
//! becomes a flat `plan(1) + found-ACL live re-confirm(1)` — MEASURED before → after, both sides
//! observed by running this file at the write-1 baseline commit and at write-2, `git log` this
//! file):
//!
//!   op                                   queries before → after   walks collapsed
//!   PUT overwrite (target exists, k=3)            6 → 4           1 (target: 4→2)
//!   PUT create    (parent walk)                  12 → 11          1 (parent: 3→2)
//!   POST create   (container target)              6 → 5           1 (container: 3→2)
//!   DELETE doc    (k=3)                          10 → 7           2 (target 4→2, parent 3→2)
//!   PATCH insert  (existing doc, k=3)             5 → 3           1 (target: 4→2)
//!
//! The win is DEPTH-INDEPENDENT: the sequential side grows with k (a deeper resource pays a longer
//! walk), the planned side is flat 2 at any k. The creation paths' remaining per-ancestor
//! EXISTENCE probes (`nearest_existing_container` / `ensure_ancestor_containers`) are a separate
//! follow-up lever (write-3), deliberately not folded here (one concern per commit).
//!
//! `max_in_flight == 1` in every scenario is the await-depth witness (strictly sequential).

mod common;

use std::sync::Arc;

use axum::body::{Body, Bytes};
use axum::http::{Request, StatusCode};
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

/// The counting harness — identical assembly to `read_path_counters.rs`.
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
        content_type: Option<&str>,
        body: Body,
    ) -> (axum::http::Response<Body>, CounterSnapshot) {
        let scope = self.counters.measure();
        let resp = self.request(method, path, content_type, body).await;
        (resp, scope.delta())
    }
}

/// Seed the ROOT owner ACL — same fixture as `read_path_counters.rs`.
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
        .write(&acl_iri, Bytes::from(acl_body), "text/turtle")
        .await
        .expect("seed root acl");
}

/// Standard fixture: container `/alice/c/` + doc `/alice/c/doc` inheriting the root ACL (k = 3 for
/// the doc), then one un-measured GET so the parsed-ACL cache is WARM for every measured op.
async fn fixture(h: &Harness) {
    let mk = h
        .request(
            "PUT",
            "/alice/c/",
            Some("text/turtle"),
            Body::from("<#c> <http://xmlns.com/foaf/0.1/name> \"C\" ."),
        )
        .await;
    assert_eq!(mk.status(), StatusCode::CREATED);
    let put = h
        .request(
            "PUT",
            "/alice/c/doc",
            Some("text/turtle"),
            Body::from(TURTLE),
        )
        .await;
    assert_eq!(put.status(), StatusCode::CREATED);
    let warm = h.request("GET", "/alice/c/doc", None, Body::empty()).await;
    assert_eq!(warm.status(), StatusCode::OK);
}

/// **PUT overwrite (k = 3, warm ACL cache).** The write-authz walk is now PLANNED: 1 combined
/// read-plan query + 1 found-ACL live re-confirm, depth-independent (was the sequential k+1 = 4
/// per-candidate probes). Remaining queries: the handler's own existence `meta` + the
/// slash-semantics conflict probe. MEASURED before → after: 6 → 4 queries.
#[tokio::test]
async fn put_overwrite_k3_counts() {
    let h = Harness::new().await;
    fixture(&h).await;

    let (resp, d) = h
        .measured(
            "PUT",
            "/alice/c/doc",
            Some("text/turtle"),
            Body::from(TURTLE),
        )
        .await;
    assert_eq!(resp.status(), StatusCode::NO_CONTENT, "overwrite 204");
    assert_eq!(
        d.sparql_queries, 4,
        "PUT overwrite = existence meta + planned walk (plan + re-confirm) + slash probe, depth-independent (was 6): {d:?}"
    );
    assert_eq!(d.blob_puts, 1, "one body write: {d:?}");
    assert_eq!(d.sparql_updates, 1, "one meta upsert: {d:?}");
    assert_eq!(d.max_in_flight, 1, "strictly sequential");
}

/// **PUT create (k = 3 from the parent, warm).** The existence-non-disclosure V1 closure
/// (`research/lws-design-records.md` §6) makes a PUT-create authorize the TARGET's own effective
/// ACL (`acl:Write`, inherited via `acl:default`) — a PLANNED walk (plan + re-confirm = 2) — BEFORE
/// probing existence, so create and overwrite are indistinguishable to an under-authorized
/// requester. It THEN also authorizes container-modification `acl:Append` on the nearest EXISTING
/// ancestor container (`/alice/c/`), a SECOND planned walk (found by upward existence probes) — two
/// DISTINCT ACL resolutions (the target's inherited grant vs the container's own `acl:accessTo`),
/// the same two-walk shape DELETE has always had. MEASURED before → after the planned-walk
/// optimization was 12 → 11 for main's create (which authorized ONLY the parent); the integrated V1
/// path adds the target-`acl:Write` planned walk (+2), for 13 total.
#[tokio::test]
async fn put_create_k3_counts() {
    let h = Harness::new().await;
    fixture(&h).await;

    let (resp, d) = h
        .measured(
            "PUT",
            "/alice/c/new",
            Some("text/turtle"),
            Body::from(TURTLE),
        )
        .await;
    assert_eq!(resp.status(), StatusCode::CREATED, "create 201");
    assert_eq!(
        d.sparql_queries, 13,
        "PUT create = planned TARGET-Write walk (V1 closure: 2) + existence meta + nearest-ancestor probe + planned container-modification walk (parent Append: 2) + slash probe + ensure-ancestors + create: {d:?}"
    );
    assert_eq!(d.blob_puts, 1, "one body write: {d:?}");
    assert_eq!(d.sparql_updates, 1, "the create transaction: {d:?}");
    assert_eq!(d.max_in_flight, 1, "strictly sequential");
}

/// **POST create into `/alice/c/` (warm).** The container's write-authz walk is PLANNED
/// (plan + re-confirm — was the sequential 3 probes: c/.acl, alice/.acl, /.acl ✓). MEASURED
/// before → after: 6 → 5.
#[tokio::test]
async fn post_create_counts() {
    let h = Harness::new().await;
    fixture(&h).await;

    let (resp, d) = h
        .measured("POST", "/alice/c/", Some("text/turtle"), Body::from(TURTLE))
        .await;
    assert_eq!(resp.status(), StatusCode::CREATED, "post 201");
    assert_eq!(
        d.sparql_queries, 5,
        "POST = planned container walk (2, was sequential 3) + container-shape/existence + slug probes + create (was 6): {d:?}"
    );
    assert_eq!(d.blob_puts, 1, "one body write: {d:?}");
    assert_eq!(d.max_in_flight, 1, "strictly sequential");
}

/// **DELETE doc (k = 3, warm).** DELETE runs TWO authorizations — Write on the TARGET (walk 4→2)
/// and Write on the PARENT container (walk 3→2) — BOTH now PLANNED. MEASURED before → after:
/// 10 → 7 (the largest single-verb win: two walks collapsed).
#[tokio::test]
async fn delete_doc_k3_counts() {
    let h = Harness::new().await;
    fixture(&h).await;

    let (resp, d) = h
        .measured("DELETE", "/alice/c/doc", None, Body::empty())
        .await;
    assert_eq!(resp.status(), StatusCode::NO_CONTENT, "delete 204");
    assert_eq!(
        d.sparql_queries, 7,
        "DELETE = planned target walk (2) + planned parent walk (2) + existence/aux probes, depth-independent (was 10): {d:?}"
    );
    assert_eq!(d.blob_puts, 0, "no body write: {d:?}");
    assert_eq!(d.max_in_flight, 1, "strictly sequential");
}

/// **PATCH insert-only on the existing doc (k = 3, warm).** PATCH's content-derived required mode
/// (Append for insert-only) authorizes via the SAME planned walk (plan + re-confirm — was the
/// sequential k+1 = 4 probes). MEASURED before → after: 5 → 3.
#[tokio::test]
async fn patch_insert_existing_k3_counts() {
    let h = Harness::new().await;
    fixture(&h).await;

    let patch = "@prefix solid: <http://www.w3.org/ns/solid/terms#>.\n\
@prefix foaf: <http://xmlns.com/foaf/0.1/>.\n\
_:patch a solid:InsertDeletePatch;\n\
  solid:inserts { <https://pod.example/alice/c/doc#it> foaf:nick \"D\" . }.\n";
    let (resp, d) = h
        .measured("PATCH", "/alice/c/doc", Some("text/n3"), Body::from(patch))
        .await;
    assert_eq!(resp.status(), StatusCode::NO_CONTENT, "patch 204");
    assert_eq!(
        d.sparql_queries, 3,
        "PATCH = planned walk (2) + target read meta, depth-independent (was 5): {d:?}"
    );
    assert_eq!(d.blob_puts, 1, "one rewritten body: {d:?}");
    assert_eq!(d.max_in_flight, 1, "strictly sequential");
}
