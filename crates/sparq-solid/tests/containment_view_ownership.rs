//! sq-3jtd.4 — `ldp:contains` / containment-view OWNERSHIP decision guard.
//!
//! DECISION (research/solid-access-control-design.md §2.1 / §7.7,
//! research/sparq-solid-scope.md area 2): a container's `ldp:contains` listing is
//! **PSS-written explicit content**, NOT a sparq-derived/materialized view. sparq-solid
//! derives *ancestry* structurally from IRI slash-semantics (`solidx:parent`/
//! `solidx:ancestor`, §3.2) purely to drive ACL inheritance, and **never**:
//!   - derives `ldp:contains` from structure,
//!   - mutates or re-derives `ldp:contains` on a write, or
//!   - reads `ldp:contains` from pod content into the reasoner.
//!
//! These tests pin that boundary so the codebase cannot silently drift toward
//! sparq-owned containment. If PSS ever asks sparq to own containment derivation it is a
//! separate, explicitly-scoped structural-only spike (the bead body) — and these tests
//! would be the thing that has to change deliberately.

use sparq_core::Graph;
use sparq_engine::query;
use sparq_solid::{wac_fixture, PodStore, AUTH_GRAPH};

const LDP_CONTAINS: &str = "http://www.w3.org/ns/ldp#contains";

fn rows(g: &Graph, q: &str) -> usize {
    query(g, q).expect("query ok").rows.len()
}

/// All `ldp:contains` triples, regardless of named graph, as `(s, o, graph)` strings.
fn all_contains(g: &Graph) -> Vec<(String, String, String)> {
    let q = format!(
        "SELECT ?s ?o ?g WHERE {{ GRAPH ?g {{ ?s <{LDP_CONTAINS}> ?o }} }} ORDER BY ?s ?o ?g"
    );
    query(g, &q)
        .expect("query ok")
        .rows
        .iter()
        .map(|r| {
            let cell = |i: usize| r[i].as_ref().map(|t| t.to_string()).unwrap_or_default();
            (cell(0), cell(1), cell(2))
        })
        .collect()
}

/// Materializing the auth view leaves every PSS-written `ldp:contains` triple
/// byte-identical: the materializer neither derives new containment nor mutates or
/// drops the explicit ones. (The fixture is the stand-in for a PSS-written pod.)
#[test]
fn materialization_preserves_pss_written_contains_verbatim() {
    let g = Graph::load_dataset(&wac_fixture(), "nquads").expect("fixture loads");
    let before = all_contains(&g);
    assert!(
        !before.is_empty(),
        "fixture must carry PSS-written ldp:contains so the guard is meaningful"
    );

    let mut store = PodStore::new(g);
    store.materialize_wac().expect("wac materializes");

    let after = all_contains(store.graph());
    assert_eq!(
        before, after,
        "ldp:contains is PSS-written content; materialization must not add, drop, or move any"
    );
}

/// sparq does NOT derive a containment view: no `ldp:contains` triple is ever emitted
/// into the reserved auth graph. Containment ancestry lives only as the internal
/// `solidx:` ancestry facts that drive inheritance — never re-surfaced as `ldp:contains`.
#[test]
fn no_contains_is_derived_into_the_auth_view() {
    let g = Graph::load_dataset(&wac_fixture(), "nquads").expect("fixture loads");
    let mut store = PodStore::new(g);
    store.materialize_wac().expect("wac materializes");

    let derived = rows(
        store.graph(),
        &format!("SELECT ?s ?o WHERE {{ GRAPH <{AUTH_GRAPH}> {{ ?s <{LDP_CONTAINS}> ?o }} }}"),
    );
    assert_eq!(
        derived, 0,
        "the auth view must not own/derive ldp:contains (containment is PSS-written content)"
    );
}

/// `ldp:contains` is opaque content to the reasoner: every `ldp:contains` triple that
/// exists after materialization is exactly one that was supplied in a *content* graph
/// (the container's own graph), never synthesized into the reserved `urn:sparq:` space.
#[test]
fn contains_lives_only_in_pss_content_graphs() {
    let g = Graph::load_dataset(&wac_fixture(), "nquads").expect("fixture loads");
    let mut store = PodStore::new(g);
    store.materialize_wac().expect("wac materializes");

    for (s, _o, graph) in all_contains(store.graph()) {
        assert!(
            !graph.contains("urn:sparq:"),
            "ldp:contains for {s} leaked into a reserved sparq graph ({graph}); \
             containment must stay PSS-written content"
        );
    }
}
