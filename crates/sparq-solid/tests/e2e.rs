//! End-to-end: the SAME SPARQL query, run as different sessions, returns different
//! results — enforced by the engine's zero-copy dataset view (the default path),
//! with the v1 FROM-NAMED rewrite path kept as a differential oracle: both paths
//! must return byte-identical JSON for every fixture session.
//!
//! sq-gq28y (issue #1546): the DEFAULT read path is now spec-conformant
//! (empty standing default graph). Tests that probe pod content with a BARE default-graph
//! pattern and assert the old UNION-ALWAYS counts are gated
//! `#[cfg(feature = "legacy-union-default-graph")]` (the opt-in escape hatch that restores
//! that behaviour). The spec default (a bare pattern is empty; `FROM <…#union-default-graph>`
//! opts into the union) is covered by `tests/union_default_graph.rs`, and mirrored here by
//! the ungated `bare_default_graph_pattern_is_empty_by_default` test. Tests that use an
//! explicit `GRAPH` pattern are unaffected by the flip and stay ungated.

use sparq_core::Graph;
use sparq_solid::fixture::{wac_fixture, ALICE, APP, BOB, CAROL};
use sparq_solid::{rewrite_for, Mode, PodStore, Session};

fn store() -> PodStore {
    let g = Graph::load_dataset(&wac_fixture(), "nquads").expect("fixture loads");
    let mut s = PodStore::new(g);
    s.materialize_wac().expect("materialize");
    s
}

/// Hand-computed readable-document counts for the WAC fixture (4 docs per leaf
/// container, 144 per depth-1 subtree; see tests/wac.rs for the per-subtree logic):
///   alice  = priv0 144 + pub1 144 + friends3 48* + mixed4 (144-24-1) + origin5 144 = 599
///           (*friends3's own ACL gives alice acl:accessTo ONLY — no acl:default — so
///            she reads just the two restated subtrees c0/c1; nearest-ACL fail-closed)
///   carol  = pub1 144 + team2 144 + mixed4 (144-24-24-1 = 95)** + mixed4/c0 24     = 407
///           (**minus c0 (carol reads it via the DEEP acl instead), minus c2 — its
///             own ACL is owner-only, so AuthenticatedAgent loses it — minus the
///             bob-only doc override)
///   anon   = pub1 144
const TITLES: &str = "SELECT ?title WHERE { ?s <https://ex.dev/ns#title> ?title }";

/// sq-gq28y (issue #1546): the SPEC default. A bare default-graph pattern is
/// evaluated against the standing EMPTY default graph, so it matches nothing for every
/// session (the union counts are reachable only via the opt-in — see
/// `tests/union_default_graph.rs`). This is the mirror of the legacy-gated union test below.
#[cfg(not(feature = "legacy-union-default-graph"))]
#[test]
fn bare_default_graph_pattern_is_empty_by_default() {
    let s = store();
    let alice = s.query_as(&Session { agent: Some(ALICE), client: None, issuer: None, now: None }, Mode::Read, TITLES).unwrap();
    let carol = s.query_as(&Session { agent: Some(CAROL), client: None, issuer: None, now: None }, Mode::Read, TITLES).unwrap();
    let anon = s.query_as(&Session::default(), Mode::Read, TITLES).unwrap();
    assert_eq!(alice.rows.len(), 0, "empty standing default graph");
    assert_eq!(carol.rows.len(), 0, "empty standing default graph");
    assert_eq!(anon.rows.len(), 0, "empty standing default graph");
}

// sq-gq28y: bare-pattern union-always counts — only under the legacy escape hatch.
#[cfg(feature = "legacy-union-default-graph")]
#[test]
fn same_query_different_agents_different_results() {
    let s = store();
    let alice = s.query_as(&Session { agent: Some(ALICE), client: None, issuer: None, now: None }, Mode::Read, TITLES).unwrap();
    let carol = s.query_as(&Session { agent: Some(CAROL), client: None, issuer: None, now: None }, Mode::Read, TITLES).unwrap();
    let anon = s.query_as(&Session::default(), Mode::Read, TITLES).unwrap();
    assert_eq!(alice.rows.len(), 599, "alice sees her documents");
    assert_eq!(carol.rows.len(), 407, "carol sees group + public + deep-override docs");
    assert_eq!(anon.rows.len(), 144, "anonymous sees only the public subtree");
    // the kept v1 portability path (FROM NAMED rewrite) returns the same counts
    let v1 = s.query_as_rewrite(&Session { agent: Some(ALICE), client: None, issuer: None, now: None }, Mode::Read, TITLES).unwrap();
    assert_eq!(v1.rows.len(), 599);
}

// sq-gq28y: exercises bare-pattern cross-document joins (union-always) — only
// under the legacy escape hatch; the spec-default GRAPH/join coverage is in
// tests/union_default_graph.rs.
#[cfg(feature = "legacy-union-default-graph")]
#[test]
fn graph_patterns_and_cross_document_joins_stay_inside_the_sandbox() {
    let s = store();
    // explicit GRAPH ?g: ranges over authorized graphs only (FROM NAMED injection)
    let q = "SELECT ?g WHERE { GRAPH ?g { ?s <https://ex.dev/ns#title> ?title } }";
    let anon = s.query_as(&Session::default(), Mode::Read, q).unwrap();
    assert_eq!(anon.rows.len(), 144);
    // a cross-document join: doc graph (inSubtree) ⋈ container graph (ldp:contains);
    // carol can read team2 documents AND the team2 container itself (acl:accessTo) —
    // 144 docs × 6 children listed in the container graph
    let join = "SELECT ?s ?child WHERE { \
                  ?s <https://ex.dev/ns#inSubtree> <https://pod.ex/team2/> . \
                  <https://pod.ex/team2/> <http://www.w3.org/ns/ldp#contains> ?child }";
    let carol = s.query_as(&Session { agent: Some(CAROL), client: None, issuer: None, now: None }, Mode::Read, join).unwrap();
    assert_eq!(carol.rows.len(), 144 * 6, "join across document + container graphs");
    let bob_no_container =
        s.query_as(&Session { agent: Some(BOB), client: None, issuer: None, now: None }, Mode::Read, join).unwrap();
    // bob reads team2 docs too (group member) and the container via… also group
    assert_eq!(bob_no_container.rows.len(), 144 * 6);
    let alice = s.query_as(&Session { agent: Some(ALICE), client: None, issuer: None, now: None }, Mode::Read, join).unwrap();
    assert_eq!(alice.rows.len(), 0, "alice is shadowed out of team2 entirely");
}

#[test]
fn explicit_named_graph_query_cannot_escape() {
    let s = store();
    // anonymous explicitly asks for a private graph: absent from FROM NAMED ⇒ empty
    let q = "SELECT ?o WHERE { GRAPH <https://pod.ex/priv0/c4/g0/d0.ttl> { ?s ?p ?o } }";
    let anon = s.query_as(&Session::default(), Mode::Read, q).unwrap();
    assert_eq!(anon.rows.len(), 0, "unauthorized graph behaves as absent");
    let alice = s.query_as(&Session { agent: Some(ALICE), client: None, issuer: None, now: None }, Mode::Read, q).unwrap();
    assert!(!alice.rows.is_empty());
    // …and a pre-existing FROM NAMED is intersected, never widened
    let widened = "SELECT ?o FROM NAMED <https://pod.ex/priv0/c4/g0/d0.ttl> \
                   WHERE { GRAPH ?g { ?s ?p ?o } }";
    let anon2 = s.query_as(&Session::default(), Mode::Read, widened).unwrap();
    assert_eq!(anon2.rows.len(), 0);
}

/// The view path (default, zero-copy) and the v1 rewrite path (FROM NAMED + copy)
/// must be observationally EQUAL: for every e2e fixture session, the same query
/// returns byte-identical SPARQL-JSON through both. Queries carry a total ORDER BY
/// over the projected variables so row order is fully determined by the semantics
/// (both paths produce the same duplicate-row multiset; identical rows serialize
/// identically regardless of their relative order — a full sort makes the whole
/// serialization canonical).
///
/// sq-gq28y: EXPLICIT-`GRAPH` queries agree on both paths regardless of the
/// default-graph flip (the flip changes only what a *bare* default-graph pattern sees), so
/// this differential oracle runs on the default build over the explicit-GRAPH subset. The
/// bare-pattern queries (which diverge: empty on the view vs union on the v1 `rewrite_for`
/// path) are exercised by the legacy-gated oracle below.
#[test]
fn view_path_and_rewrite_path_agree_on_explicit_graph_queries() {
    let queries = [
        // explicit GRAPH ?g enumeration
        "SELECT ?g ?s WHERE { GRAPH ?g { ?s <https://ex.dev/ns#title> ?title } } ORDER BY ?g ?s",
        // explicit named graph (authorized for alice, absent for everyone else)
        "SELECT ?p ?o WHERE { GRAPH <https://pod.ex/priv0/c4/g0/d0.ttl> { ?s ?p ?o } } ORDER BY ?p ?o",
        // attacker-supplied FROM NAMED: intersected (view) / intersected (rewrite)
        "SELECT ?g ?o FROM NAMED <https://pod.ex/priv0/c4/g0/d0.ttl> \
           WHERE { GRAPH ?g { ?s ?p ?o } } ORDER BY ?g ?o",
    ];
    assert_paths_agree(&queries);
}

/// sq-gq28y: the FULL oracle including bare default-graph patterns is only
/// coherent under the legacy escape hatch, where the view path also does union-always (so it
/// matches the v1 `rewrite_for` path). Under the spec default the two intentionally diverge
/// for bare patterns (empty vs union) — see `tests/union_default_graph.rs`.
#[cfg(feature = "legacy-union-default-graph")]
#[test]
fn view_path_and_rewrite_path_return_identical_json() {
    let queries = [
        // plain default-graph pattern (union-default emulation on both paths under legacy)
        "SELECT ?s ?title WHERE { ?s <https://ex.dev/ns#title> ?title } ORDER BY ?title ?s",
        "SELECT ?g ?s WHERE { GRAPH ?g { ?s <https://ex.dev/ns#title> ?title } } ORDER BY ?g ?s",
        "SELECT ?p ?o WHERE { GRAPH <https://pod.ex/priv0/c4/g0/d0.ttl> { ?s ?p ?o } } ORDER BY ?p ?o",
        // cross-document join
        "SELECT ?s ?child WHERE { \
           ?s <https://ex.dev/ns#inSubtree> <https://pod.ex/team2/> . \
           <https://pod.ex/team2/> <http://www.w3.org/ns/ldp#contains> ?child } ORDER BY ?s ?child",
        "SELECT ?g ?o FROM NAMED <https://pod.ex/priv0/c4/g0/d0.ttl> \
           WHERE { GRAPH ?g { ?s ?p ?o } } ORDER BY ?g ?o",
        // aggregate over the visible set
        "SELECT (COUNT(?s) AS ?n) WHERE { ?s <https://ex.dev/ns#title> ?title }",
    ];
    assert_paths_agree(&queries);
}

/// Shared driver: for every fixture session, assert the view path and the v1 `rewrite_for`
/// path return byte-identical SPARQL-JSON for each query in `queries`.
fn assert_paths_agree(queries: &[&str]) {
    let s = store();
    let sessions = [
        ("alice", Session { agent: Some(ALICE), client: None, issuer: None, now: None }),
        ("carol", Session { agent: Some(CAROL), client: None, issuer: None, now: None }),
        ("bob+app (origin pair)", Session { agent: Some(BOB), client: Some(APP), issuer: None, now: None }),
        ("anonymous", Session::default()),
    ];
    for (label, session) in sessions {
        for q in queries {
            let view_json = s.query_json_as(&session, Mode::Read, q).unwrap();
            let allowed = s.accessible(&session, Mode::Read);
            let rewritten = rewrite_for(q, &allowed).unwrap();
            let v1_json = sparq_engine::query_json(s.graph(), &rewritten).unwrap();
            assert_eq!(view_json, v1_json, "paths diverge for {label}: {q}");
        }
    }
}

#[test]
fn rewrite_shape() {
    let allowed = [oxrdf::NamedNode::new("https://pod.ex/pub1/c0/g0/d0.ttl").unwrap()];
    let out = rewrite_for(TITLES, &allowed).unwrap();
    assert!(out.contains("FROM NAMED <https://pod.ex/pub1/c0/g0/d0.ttl>"), "{out}");
    assert!(out.contains("GRAPH"), "default-graph pattern wrapped: {out}");
}
