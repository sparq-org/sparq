//! Skolem stability: the materialized `<urn:sparq:auth>` view must be invariant under a
//! permutation of `graph.named`.
//!
//! The loader skolemizes a control document's blank nodes so the single-graph merge keeps
//! per-document scoping (`loader.rs`). Those skolem IRIs are NOT internal: a blank-node
//! `acp:noneOf` matcher — the idiomatic Turtle shape — surfaces in the auth view as the
//! object of `auth:exceptMatcher` and as the subject of the copied `solidx:accepts*P`
//! matcher facts (`materialize.rs`). So the skolem key is part of the view's observable
//! triple set.
//!
//! Keying it on the document's POSITION in `graph.named` makes the view position-dependent,
//! and `put_acl`/`delete_acl` permute positions (`take_named_slot` uses `Vec::swap_remove`,
//! the new content is `push`ed). A write to document X therefore renumbers document Y's
//! skolems, `AuthIndex::matcher_*` changes, `matchers_eq` reports a difference, and
//! `reindex_with` falls back to `ReindexScope::Full` — a full session-cache clear on every
//! ACL write, defeating the per-origin diff invalidation `sq-b7k7u` shipped.
//!
//! These tests pin the invariant directly: the auth view is a function of the dataset's
//! CONTENT, not of the order its named graphs happen to sit in.
//!
//! Research record: `research/solid-pod-scoped-materializer-design.md` §3.1 (issue #5579).
//! [OPUS-5]

use oxrdf::Term;
use sparq_core::Graph;
use sparq_solid::{AuthIndex, Mode, PodStore, Session};

const ACL: &str = "http://www.w3.org/ns/auth/acl#";
const ACP: &str = "http://www.w3.org/ns/solid/acp#";
const CAROL: &str = "https://carol.ex/card#me";
const BOB: &str = "https://bob.ex/card#me";

/// One pod's ACR, as N-Quads: a public `acp:allow Read` policy applying to the pod root and
/// (via `memberAccessControl`) everything under it, carved out by a **blank-node**
/// `acp:noneOf` exception matcher excluding `excluded`.
///
/// `label` is the blank-node label used in the N-Quads source. N-Quads blank labels are
/// document-scoped, so two pods sharing a label share the node in the *source* dataset —
/// the loader's per-document skolemization is exactly what must keep them apart.
fn acp_pod_quads(pod: &str, label: &str, excluded: &str) -> String {
    let acr = format!("{pod}/.acr");
    format!(
        "<{pod}/n1#it> <https://ex.dev/ns#k> \"v\" <{pod}/n1> .\n\
         <{acr}> <{ACP}accessControl> <{acr}#ac> <{acr}> .\n\
         <{acr}> <{ACP}memberAccessControl> <{acr}#ac> <{acr}> .\n\
         <{acr}#ac> <{ACP}apply> <{acr}#pol> <{acr}> .\n\
         <{acr}#pol> <{ACP}allow> <{ACL}Read> <{acr}> .\n\
         <{acr}#pol> <{ACP}anyOf> <{acr}#mpub> <{acr}> .\n\
         <{acr}#mpub> <{ACP}agent> <{ACP}PublicAgent> <{acr}> .\n\
         <{acr}#pol> <{ACP}noneOf> _:{label} <{acr}> .\n\
         _:{label} <{ACP}agent> <{excluded}> <{acr}> .\n"
    )
}

/// The same ACR body as N-Triples (no graph column) — what `put_acl_acp` takes. Rewriting a
/// document with this is a content no-op.
fn acr_body(pod: &str, label: &str, excluded: &str) -> String {
    let acr = format!("{pod}/.acr");
    format!(
        "<{acr}> <{ACP}accessControl> <{acr}#ac> .\n\
         <{acr}> <{ACP}memberAccessControl> <{acr}#ac> .\n\
         <{acr}#ac> <{ACP}apply> <{acr}#pol> .\n\
         <{acr}#pol> <{ACP}allow> <{ACL}Read> .\n\
         <{acr}#pol> <{ACP}anyOf> <{acr}#mpub> .\n\
         <{acr}#mpub> <{ACP}agent> <{ACP}PublicAgent> .\n\
         <{acr}#pol> <{ACP}noneOf> _:{label} .\n\
         _:{label} <{ACP}agent> <{excluded}> .\n"
    )
}

/// Two pods, each with a blank-node `noneOf` matcher carving `CAROL` out of an otherwise
/// public read. Distinct blank labels, so the two matchers are distinct in the source too.
fn two_pod_dataset() -> String {
    format!(
        "{}{}",
        acp_pod_quads("https://a.ex", "exa", CAROL),
        acp_pod_quads("https://b.ex", "exb", CAROL)
    )
}

/// The installed `<urn:sparq:auth>` view as a sorted, deduplicated triple list.
fn auth_view(store: &PodStore) -> Vec<String> {
    let (_, auth) = store
        .graph
        .named
        .iter()
        .find(|(n, _)| matches!(n, Term::NamedNode(n) if n.as_str() == sparq_solid::AUTH_GRAPH))
        .expect("an auth view is installed");
    let pat: sparq_core::store::Pattern = [None, None, None];
    let scan = auth.store.scan(&pat);
    let mut out: Vec<String> = scan
        .rows
        .iter()
        .map(|r| {
            let t = scan.to_spo(r);
            format!("{} {} {}", auth.dict.term(t[0]), auth.dict.term(t[1]), auth.dict.term(t[2]))
        })
        .collect();
    out.sort();
    out.dedup();
    out
}

fn store_from(nq: &str) -> PodStore {
    let mut s = PodStore::new(Graph::load_dataset(nq, "nquads").expect("dataset loads"));
    s.materialize_acp().expect("acp materializes");
    s
}

fn reads(store: &mut PodStore, who: Option<&str>, graph: &str) -> bool {
    store
        .accessible(&Session { agent: who, client: None, issuer: None, now: None }, Mode::Read)
        .iter()
        .any(|g| g.as_str() == graph)
}

/// Sanity: the fixture really does put a skolemized blank-node matcher into the auth view,
/// so the permutation assertions below are non-vacuous. Without this they would pass on a
/// fixture that simply has no skolems to renumber.
#[test]
fn blank_node_matcher_reaches_the_auth_view() {
    let mut store = store_from(&two_pod_dataset());
    let view = auth_view(&store);
    assert!(
        view.iter().any(|t| t.contains("urn:skolem:")),
        "fixture must surface a skolemized blank-node matcher in the auth view, got:\n{}",
        view.join("\n")
    );
    // ... and it is load-bearing: carol is carved out of an otherwise-public read.
    assert!(reads(&mut store, Some(BOB), "https://a.ex/n1"), "bob reads (public)");
    assert!(
        !reads(&mut store, Some(CAROL), "https://a.ex/n1"),
        "carol is carved out by the noneOf exception"
    );
}

/// **The invariant.** Permuting `graph.named` and re-materializing must install the
/// identical auth view — the view is a function of the dataset's content, not of the order
/// its named graphs sit in.
#[test]
fn auth_view_is_invariant_under_named_graph_permutation() {
    let mut store = store_from(&two_pod_dataset());
    let before = auth_view(&store);

    // Reverse the named-graph vector. Content is untouched; only positions change.
    store.graph.named.reverse();
    store.materialize_acp().expect("acp re-materializes");
    let after = auth_view(&store);

    assert_eq!(before, after, "auth view changed under a pure permutation of graph.named");
}

/// The consequence that bites in production: `put_acl_acp` on pod B with byte-identical
/// content must not perturb pod A's slice of the view. `take_named_slot` +
/// `Vec::swap_remove` + `push` permutes positions, so a position-keyed skolem renumbers A's
/// matcher — which makes `matchers_eq` false and collapses the per-origin diff invalidation
/// to a full cache clear.
#[test]
fn identical_rewrite_of_one_acr_does_not_perturb_the_view() {
    let mut store = store_from(&two_pod_dataset());
    let before = auth_view(&store);

    store
        .put_acl_acp("https://b.ex/.acr", &acr_body("https://b.ex", "exb", CAROL), "ntriples")
        .expect("re-put identical ACR content");

    let after = auth_view(&store);
    assert_eq!(
        before, after,
        "a content-identical put_acl_acp changed the auth view (position-keyed skolems)"
    );
}

/// And the store stays consistent with a from-scratch rebuild across the permuting write, so
/// the stability cannot have been bought by dropping facts.
#[test]
fn view_stays_equal_to_a_fresh_rebuild_across_a_permuting_write() {
    let mut store = store_from(&two_pod_dataset());
    store
        .put_acl_acp("https://a.ex/.acr", &acr_body("https://a.ex", "exa", CAROL), "ntriples")
        .expect("re-put identical ACR content");

    let fresh = AuthIndex::from_graph(&store.graph);
    for who in [Some(BOB), Some(CAROL), None] {
        let s = Session { agent: who, client: None, issuer: None, now: None };
        for mode in [Mode::Read, Mode::Write, Mode::Append, Mode::Control] {
            let got: Vec<String> =
                store.accessible(&s, mode).iter().map(|n| n.as_str().to_owned()).collect();
            let want: Vec<String> =
                fresh.accessible(&s, mode).iter().map(|n| n.as_str().to_owned()).collect();
            assert_eq!(got, want, "cached view diverged from a fresh rebuild for {who:?}/{mode:?}");
        }
    }
}

/// Per-document scoping still holds: two documents using the SAME blank-node label must get
/// DISTINCT skolems. a.ex excludes carol, b.ex excludes bob; if the skolems collided the
/// merged matcher would accept both and carve both out of both pods.
#[test]
fn same_blank_label_in_two_documents_stays_distinct() {
    let nq = format!(
        "{}{}",
        acp_pod_quads("https://a.ex", "shared", CAROL),
        acp_pod_quads("https://b.ex", "shared", BOB)
    );
    let mut store = store_from(&nq);

    assert!(!reads(&mut store, Some(CAROL), "https://a.ex/n1"), "carol carved out at a.ex");
    assert!(reads(&mut store, Some(BOB), "https://a.ex/n1"), "bob is NOT carved out at a.ex");
    assert!(!reads(&mut store, Some(BOB), "https://b.ex/n1"), "bob carved out at b.ex");
    assert!(
        reads(&mut store, Some(CAROL), "https://b.ex/n1"),
        "b.ex's `_:shared` is a DIFFERENT node — carol must not be carved out there"
    );
}
