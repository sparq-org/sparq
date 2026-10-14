//! sq-gq28y (issue #1546): end-to-end enforcement of the **spec-conformant
//! empty-default + union-default-graph opt-in** through the REAL `PodStore` read path, per
//! the *Access-Controlled SPARQL Query over a Solid Pod* Editor's Draft (`jeswr/solid-sparql-query`).
//!
//! This is the DEFAULT behaviour (issue #1546: "spec compliant - empty by default"), so this
//! suite runs on the default build. It is gated OFF only under the `legacy-union-default-graph`
//! escape hatch, whose mirror-image union-always behaviour is asserted by the legacy-gated
//! tests in `tests/e2e.rs` and `tests/hardening.rs`.
#![cfg(not(feature = "legacy-union-default-graph"))]

use sparq_core::Graph;
use sparq_solid::fixture::{wac_fixture, ALICE, CAROL};
use sparq_solid::{Mode, PodStore, Session, UNION_DEFAULT_GRAPH_IRI};

fn store() -> PodStore {
    let g = Graph::load_dataset(&wac_fixture(), "nquads").expect("fixture loads");
    let mut s = PodStore::new(g);
    s.materialize_wac().expect("materialize");
    s
}

fn alice() -> Session<'static> {
    Session { agent: Some(ALICE), client: None, issuer: None, now: None }
}
fn carol() -> Session<'static> {
    Session { agent: Some(CAROL), client: None, issuer: None, now: None }
}

// The bare default-graph title probe; the hand-computed union counts (matching tests/e2e.rs)
// are alice=599, carol=407, anon=144.
const BARE: &str = "SELECT ?title WHERE { ?s <https://ex.dev/ns#title> ?title }";

/// Opt into the union default graph: `FROM <reserved>` on an otherwise-bare query.
fn opt_in(body: &str) -> String {
    format!("SELECT ?title FROM <{}> WHERE {{ {} }}", UNION_DEFAULT_GRAPH_IRI, body)
}

/// Draft §4: "default graph: empty, always, unless the query explicitly requests otherwise".
/// A bare default-graph pattern with NO opt-in matches nothing — for every session.
#[test]
fn bare_default_graph_pattern_is_empty_without_opt_in() {
    let s = store();
    assert_eq!(s.query_as(&alice(), Mode::Read, BARE).unwrap().rows.len(), 0, "alice");
    assert_eq!(s.query_as(&carol(), Mode::Read, BARE).unwrap().rows.len(), 0, "carol");
    assert_eq!(s.query_as(&Session::default(), Mode::Read, BARE).unwrap().rows.len(), 0, "anon");
}

/// Draft §4: with the reserved IRI in a `FROM` clause, the default graph FOR THAT QUERY is
/// the union of the session's authorized named graphs — reproducing today's union counts.
#[test]
fn union_default_opt_in_reproduces_authorized_union() {
    let s = store();
    let q = opt_in("?s <https://ex.dev/ns#title> ?title");
    assert_eq!(s.query_as(&alice(), Mode::Read, &q).unwrap().rows.len(), 599, "alice union");
    assert_eq!(s.query_as(&carol(), Mode::Read, &q).unwrap().rows.len(), 407, "carol union");
    assert_eq!(
        s.query_as(&Session::default(), Mode::Read, &q).unwrap().rows.len(),
        144,
        "anon union"
    );
}

/// The opt-in is PER-REQUEST: it is a pure function of the query's dataset clause and leaves
/// no state on the store/session. Interleaving opt-in and bare queries on the SAME store and
/// session never leaks the union view into a subsequent bare request.
#[test]
fn opt_in_is_per_request_with_no_state_leak() {
    let s = store();
    let q = opt_in("?s <https://ex.dev/ns#title> ?title");
    assert_eq!(s.query_as(&alice(), Mode::Read, &q).unwrap().rows.len(), 599, "opt-in 1");
    // A following BARE request must be empty — the previous opt-in did not stick.
    assert_eq!(s.query_as(&alice(), Mode::Read, BARE).unwrap().rows.len(), 0, "bare after opt-in");
    // And opting back in still works (no state was consumed/poisoned either).
    assert_eq!(s.query_as(&alice(), Mode::Read, &q).unwrap().rows.len(), 599, "opt-in 2");
}

/// Explicit `GRAPH` patterns are UNAFFECTED by the feature: they range over the authorized
/// named graphs with or without the opt-in (only the empty/union DEFAULT graph is gated).
#[test]
fn explicit_graph_pattern_unaffected_by_feature() {
    let s = store();
    let q = "SELECT ?title WHERE { GRAPH ?g { ?s <https://ex.dev/ns#title> ?title } }";
    assert_eq!(s.query_as(&alice(), Mode::Read, q).unwrap().rows.len(), 599, "alice GRAPH");
    assert_eq!(s.query_as(&Session::default(), Mode::Read, q).unwrap().rows.len(), 144, "anon GRAPH");
}

/// Draft §4: the reserved IRI in `FROM NAMED` position "names nothing (treated as absent)".
/// It must be STRIPPED — NOT intersected — so `GRAPH ?g` still ranges over the full
/// authorized set (leaving it would wrongly collapse the named-graph set to empty).
#[test]
fn reserved_iri_in_from_named_is_absent_graph_stays_usable() {
    let s = store();
    let with_reserved = format!(
        "SELECT ?title FROM NAMED <{}> WHERE {{ GRAPH ?g {{ ?s <https://ex.dev/ns#title> ?title }} }}",
        UNION_DEFAULT_GRAPH_IRI
    );
    let plain = "SELECT ?title WHERE { GRAPH ?g { ?s <https://ex.dev/ns#title> ?title } }";
    // identical to the query WITHOUT the reserved IRI: it contributed nothing and did not
    // restrict the authorized named-graph set.
    assert_eq!(
        s.query_as(&alice(), Mode::Read, &with_reserved).unwrap().rows.len(),
        s.query_as(&alice(), Mode::Read, plain).unwrap().rows.len(),
        "reserved FROM NAMED must be a no-op, not a restriction",
    );
    assert_eq!(s.query_as(&alice(), Mode::Read, &with_reserved).unwrap().rows.len(), 599);
}

/// The reserved IRI is a signal, never a graph name: even under the opt-in, `GRAPH ?g` never
/// binds it, and it never appears in results.
#[test]
fn reserved_iri_never_binds_graph_variable() {
    let s = store();
    // Opt into the union default graph AND enumerate graph names with `GRAPH ?g`.
    let q = format!(
        "SELECT ?g FROM <{}> WHERE {{ GRAPH ?g {{ ?s <https://ex.dev/ns#title> ?o }} }}",
        UNION_DEFAULT_GRAPH_IRI
    );
    let res = s.query_as(&alice(), Mode::Read, &q).unwrap();
    let bound_reserved = res
        .rows
        .iter()
        .flatten()
        .filter_map(|t| t.as_ref())
        .any(|t| t.to_string().contains(UNION_DEFAULT_GRAPH_IRI));
    assert!(!bound_reserved, "reserved IRI must never bind ?g or surface in results");
    assert!(!res.rows.is_empty(), "alice still sees her authorized graphs");
}

/// Fail-closed exact-match: a near-miss IRI is a normal (absent) dataset reference, NOT the
/// opt-in — it must NOT silently enable the union default graph.
#[test]
fn near_miss_iri_does_not_enable_union_fail_closed() {
    let s = store();
    let q = "SELECT ?title FROM <http://www.w3.org/ns/solid/sparql#union-default-graphX> \
             WHERE { ?s <https://ex.dev/ns#title> ?title }";
    assert_eq!(
        s.query_as(&alice(), Mode::Read, q).unwrap().rows.len(),
        0,
        "a near-miss IRI must not opt into the union default graph",
    );
}

/// The opt-in is honoured on ALL read entry points, and the empty-default is a genuine
/// non-disclosure property at the aggregate level too (draft §5): a COUNT over a bare
/// pattern is 0 without the opt-in.
#[test]
fn ask_json_and_aggregate_honour_opt_in() {
    let s = store();
    // ASK: false on the bare pattern (empty default), true on the opt-in.
    assert!(!s.ask_as(&alice(), Mode::Read, "ASK { ?s <https://ex.dev/ns#title> ?t }").unwrap());
    let ask_opt = format!("ASK FROM <{}> {{ ?s <https://ex.dev/ns#title> ?t }}", UNION_DEFAULT_GRAPH_IRI);
    assert!(s.ask_as(&alice(), Mode::Read, &ask_opt).unwrap());
    // Aggregate: COUNT over the bare pattern discloses nothing without the opt-in.
    let json_bare = s
        .query_json_as(&alice(), Mode::Read, "SELECT (COUNT(?s) AS ?n) WHERE { ?s <https://ex.dev/ns#title> ?t }")
        .unwrap();
    assert!(json_bare.contains("\"value\":\"0\""), "COUNT must be 0 without opt-in: {json_bare}");
}
