//! (sq-p5oy, epic sq-3183) End-to-end tests for **cyclic** join sub-BGPs in
//! the transitive filtered-ANN BGP→IdMask derivation of the `vec:` magic predicate
//! (follow-up to sq-3tjd/#287, which added the join-connected-component mask).
//!
//! sq-3tjd evaluates the connected component of the BGP join-graph that contains the
//! neighbour variable and projects it back onto that variable. This file proves that
//! component derivation behaves correctly when the sub-BGP contains a **cycle** — a join
//! path that loops back to the neighbour variable (`?node :owns ?x . ?x :ownedBy ?node`),
//! or a cycle among the intermediate variables. The two concerns:
//!
//! 1. **Termination.** `connected_component` is a fixpoint worklist over a per-pattern
//!    `taken` flag, so a cycle in the join-graph cannot loop forever (each pass either
//!    consumes a not-yet-taken pattern or stops). These tests would HANG (and time the
//!    suite out) on a regression that re-walked a cycle without a fixpoint guard.
//! 2. **Correctness.** The cyclic sub-BGP is evaluated through the standard engine, whose
//!    BGP semantics already handle cycles; the derived mask must therefore equal
//!    post-filtering the unfiltered top-k by that same cyclic constraint.
//!
//! Gated on BOTH `vec-predicate` (the rewrite) and `filtered-ann` (the `IdMask` API).
#![cfg(all(feature = "vec-predicate", feature = "filtered-ann"))]

use oxrdf::{NamedNode, Term};
use sparq_core::Graph;
use sparq_vectors::{query_vec, QueryResult, VectorStore};

fn tmp_path(name: &str) -> std::path::PathBuf {
    let mut p = std::env::temp_dir();
    p.push(format!(
        "sparq_vec_fbgpc_{}_{}.spqv",
        std::process::id(),
        name
    ));
    p
}

/// Six owners a..f on the unit circle (same geometry as filtered_bgp_transitive.rs so a
/// +x query has a known best-first order). Each `:owns` a thing, and the thing points
/// BACK at its owner with `:ownedBy` — so `?node :owns ?x . ?x :ownedBy ?node` is a
/// 2-pattern **cycle** (a back-edge to `?node`). The reciprocal `:ownedBy` edge is present
/// for a..c but DELIBERATELY ABSENT for d, e, f — so the cycle only closes for {a, b, c},
/// exactly the Vehicle-owner set of the transitive fixture, letting us reuse its assertions.
///
///   a (+x)    owns ta ; ta ownedBy a   <- cycle closes
///   c (~+x)   owns tc ; tc ownedBy c   <- cycle closes
///   b (+y)    owns tb ; tb ownedBy b   <- cycle closes
///   d (-x)    owns td ; (no ownedBy)   <- cycle does NOT close
///   e (~+y)   owns te ; (no ownedBy)   <- cycle does NOT close
///   f (~+x)   owns tf ; (no ownedBy)   <- closest non-a to +x, but cycle does NOT close
fn cyclic_fixture(name: &str) -> (Graph, VectorStore) {
    let g = Graph::load_str(
        r#"
        <http://ex/a> <http://ex/owns> <http://ex/ta> .
        <http://ex/b> <http://ex/owns> <http://ex/tb> .
        <http://ex/c> <http://ex/owns> <http://ex/tc> .
        <http://ex/d> <http://ex/owns> <http://ex/td> .
        <http://ex/e> <http://ex/owns> <http://ex/te> .
        <http://ex/f> <http://ex/owns> <http://ex/tf> .
        <http://ex/ta> <http://ex/ownedBy> <http://ex/a> .
        <http://ex/tb> <http://ex/ownedBy> <http://ex/b> .
        <http://ex/tc> <http://ex/ownedBy> <http://ex/c> .
        "#,
        "ntriples",
    )
    .unwrap();
    let id = |s: &str| {
        g.id_of(&Term::NamedNode(NamedNode::new(s).unwrap()))
            .unwrap()
    };
    let mut store = VectorStore::create(tmp_path(name), 2).unwrap();
    store.put(id("http://ex/a"), &[1.0, 0.0]).unwrap(); // +x   cycle closes
    store.put(id("http://ex/b"), &[0.0, 1.0]).unwrap(); // +y   cycle closes
    store.put(id("http://ex/c"), &[0.9, 0.1]).unwrap(); // ~+x  cycle closes
    store.put(id("http://ex/d"), &[-1.0, 0.0]).unwrap(); // -x  no cycle
    store.put(id("http://ex/e"), &[0.2, 0.98]).unwrap(); // ~+y no cycle
    store.put(id("http://ex/f"), &[0.95, 0.05]).unwrap(); // ~+x no cycle (closest to +x after a)
    (g, store)
}

fn iris(r: &QueryResult, col: usize) -> Vec<String> {
    r.rows
        .iter()
        .filter_map(|row| match &row[col] {
            Some(Term::NamedNode(n)) => Some(n.as_str().to_string()),
            _ => None,
        })
        .collect()
}

const PFX: &str = "PREFIX vec: <http://sparq.dev/vec#> PREFIX rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> ";

/// 2-pattern back-edge cycle: `?node :owns ?x . ?x :ownedBy ?node`. The component contains
/// both patterns (they share `?node` AND `?x`, forming a cycle), and the engine evaluates
/// the cycle to the owners for which the reciprocal edge closes — {a, b, c}. The closer
/// `f` (no `:ownedBy` back-edge) must be excluded. This test would HANG on a non-terminating
/// component walk and FAIL on an incorrect candidate set.
#[test]
fn cyclic_back_edge_restricts_correctly_and_terminates() {
    let (g, store) = cyclic_fixture("back_edge");
    let r = query_vec(
        &g,
        &format!(
            "{PFX} SELECT ?node WHERE {{
               ?node vec:nearest ( \"1,0\" 3 ) .
               ?node <http://ex/owns> ?x .
               ?x <http://ex/ownedBy> ?node .
             }}"
        ),
        &store,
    )
    .unwrap();
    let mut got = iris(&r, 0);
    got.sort();
    got.dedup();
    assert_eq!(
        got,
        vec![
            "http://ex/a".to_string(),
            "http://ex/b".to_string(),
            "http://ex/c".to_string()
        ],
        "the back-edge cycle must admit only owners whose reciprocal edge closes: {r:?}"
    );
    assert!(
        !got.contains(&"http://ex/f".to_string()),
        "the closer `f` (cycle does not close) must be filtered out: {r:?}"
    );
}

/// Filtered top-k via the cyclic constraint ≡ post-filtering the unfiltered top-k by the
/// SAME cyclic constraint, AND filtered ⊆ unfiltered. Proves the cyclic mask is exactly the
/// post-filter set (no over- or under-narrowing).
#[test]
fn cyclic_filtered_equals_postfilter_and_is_subset() {
    let (g, store) = cyclic_fixture("postfilter");

    // Unfiltered: all 6 neighbours of a +x query, best-first.
    let unfiltered = query_vec(
        &g,
        &format!("{PFX} SELECT ?node WHERE {{ ?node vec:nearest ( \"1,0\" 6 ) }}"),
        &store,
    )
    .unwrap();
    let unfiltered_order = iris(&unfiltered, 0);

    // The cyclically-admitted set, via plain SPARQL (no vec:): owners whose cycle closes.
    let owners: std::collections::HashSet<String> = query_vec(
        &g,
        &format!(
            "{PFX} SELECT ?node WHERE {{
               ?node <http://ex/owns> ?x . ?x <http://ex/ownedBy> ?node }}"
        ),
        &store,
    )
    .unwrap()
    .rows
    .iter()
    .filter_map(|row| match &row[0] {
        Some(Term::NamedNode(n)) => Some(n.as_str().to_string()),
        _ => None,
    })
    .collect();

    // Post-filter: unfiltered order, keep only cycle-closing owners, take top-2.
    let k = 2;
    let postfilter: Vec<String> = unfiltered_order
        .iter()
        .filter(|n| owners.contains(*n))
        .take(k)
        .cloned()
        .collect();

    // Filtered search: same query + the cyclic constraint, k=2.
    let filtered = query_vec(
        &g,
        &format!(
            "{PFX} SELECT ?node WHERE {{
               ( ?node ?score ) vec:search ( \"1,0\" {k} ) .
               ?node <http://ex/owns> ?x .
               ?x <http://ex/ownedBy> ?node .
             }} ORDER BY DESC(?score)"
        ),
        &store,
    )
    .unwrap();
    let filtered_order = iris(&filtered, 0);

    assert_eq!(
        filtered_order, postfilter,
        "cyclic filtered top-k must equal post-filtering the unfiltered top-k\nfiltered={filtered_order:?}\npostfilter={postfilter:?}"
    );
    let unfiltered_set: std::collections::HashSet<&String> = unfiltered_order.iter().collect();
    for n in &filtered_order {
        assert!(
            unfiltered_set.contains(n),
            "filtered neighbour {n} not in the unfiltered result {unfiltered_order:?}"
        );
    }
}

/// A LONGER cycle among intermediate variables that loops back to `?node`:
/// `?node :owns ?x . ?x :part ?y . ?y :owner ?node` — a 3-pattern cycle
/// (`?node → ?x → ?y → ?node`). Termination must hold for the longer loop, and the mask
/// must be the set of owners for which the whole 3-edge cycle closes.
#[test]
fn longer_cycle_three_patterns_terminates_and_restricts() {
    // a, b close the 3-cycle; c, d do not (broken at the last hop). Geometry: a +x, b +y,
    // c ~+x (closest non-a), d -x — so the unconstrained +x top-2 would be {a, c}, and the
    // cyclic constraint must drop c (its cycle does not close) leaving {a, b}.
    let g = Graph::load_str(
        r#"
        <http://ex/a> <http://ex/owns> <http://ex/xa> .
        <http://ex/xa> <http://ex/part> <http://ex/ya> .
        <http://ex/ya> <http://ex/owner> <http://ex/a> .
        <http://ex/b> <http://ex/owns> <http://ex/xb> .
        <http://ex/xb> <http://ex/part> <http://ex/yb> .
        <http://ex/yb> <http://ex/owner> <http://ex/b> .
        <http://ex/c> <http://ex/owns> <http://ex/xc> .
        <http://ex/xc> <http://ex/part> <http://ex/yc> .
        <http://ex/yc> <http://ex/owner> <http://ex/other> .
        <http://ex/d> <http://ex/owns> <http://ex/xd> .
        "#,
        "ntriples",
    )
    .unwrap();
    let id = |s: &str| {
        g.id_of(&Term::NamedNode(NamedNode::new(s).unwrap()))
            .unwrap()
    };
    let mut store = VectorStore::create(tmp_path("longer_cycle"), 2).unwrap();
    store.put(id("http://ex/a"), &[1.0, 0.0]).unwrap(); // +x   cycle closes
    store.put(id("http://ex/b"), &[0.0, 1.0]).unwrap(); // +y   cycle closes
    store.put(id("http://ex/c"), &[0.95, 0.05]).unwrap(); // ~+x cycle does NOT close
    store.put(id("http://ex/d"), &[-1.0, 0.0]).unwrap(); // -x  cycle does NOT close

    let r = query_vec(
        &g,
        &format!(
            "{PFX} SELECT ?node WHERE {{
               ?node vec:nearest ( \"1,0\" 2 ) .
               ?node <http://ex/owns> ?x .
               ?x <http://ex/part> ?y .
               ?y <http://ex/owner> ?node .
             }}"
        ),
        &store,
    )
    .unwrap();
    let mut got = iris(&r, 0);
    got.sort();
    got.dedup();
    assert_eq!(
        got,
        vec!["http://ex/a".to_string(), "http://ex/b".to_string()],
        "the 3-pattern cycle must admit only owners whose full cycle closes (c dropped): {r:?}"
    );
    assert!(
        !got.contains(&"http://ex/c".to_string()),
        "the closer `c` (3-cycle does not close at the last hop) must be excluded: {r:?}"
    );
}

/// No-regression: an ACYCLIC transitive constraint on the SAME reciprocal fixture still
/// behaves correctly. `?node :owns ?x` alone (no back-edge) is acyclic and unconstraining
/// beyond "is an owner" — all six owners qualify, so the +x top-2 is the plain {a, f}. This
/// guards against the cyclic handling accidentally changing the acyclic path.
#[test]
fn acyclic_path_unaffected() {
    let (g, store) = cyclic_fixture("acyclic");
    let r = query_vec(
        &g,
        &format!(
            "{PFX} SELECT ?node WHERE {{
               ?node vec:nearest ( \"1,0\" 2 ) .
               ?node <http://ex/owns> ?x .
             }}"
        ),
        &store,
    )
    .unwrap();
    let mut got = iris(&r, 0);
    got.sort();
    got.dedup();
    assert_eq!(
        got,
        vec!["http://ex/a".to_string(), "http://ex/f".to_string()],
        "an acyclic `?node :owns ?x` admits all owners (f survives): {r:?}"
    );
}
