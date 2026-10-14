//! (sq-3tjd, epic sq-3183) End-to-end tests for the **transitive /
//! multi-variable** filtered-ANN BGP→IdMask derivation of the `vec:` magic predicate
//! (follow-up to sq-bvmd/#281). The single-variable wiring restricted the candidate
//! id-set to patterns that DIRECTLY mention the neighbour variable; sq-3tjd extends the
//! mask to the projection of the whole **join-connected sub-BGP** — patterns that
//! constrain the neighbour variable transitively through intermediate variables.
//!
//! Invariants proven here (preserved + extended from sq-bvmd):
//! - a 2-hop constraint (`?node :owns ?x . ?x a :Vehicle`) restricts `?node` to subjects
//!   join-connected to a `:Vehicle`, and matches post-filtering the unfiltered top-k by
//!   that same transitive constraint;
//! - a **disconnected** pattern (no shared variable path to `?node`) does NOT affect the
//!   mask;
//! - the **single-variable** case is unchanged (regression guard vs the old rule);
//! - **multiple** `vec:` requests each get their own connected-component mask;
//! - filtered ⊆ unfiltered holds transitively.
//!
//! Gated on BOTH `vec-predicate` (the rewrite) and `filtered-ann` (the `IdMask` API).
#![cfg(all(feature = "vec-predicate", feature = "filtered-ann"))]

use oxrdf::{NamedNode, Term};
use sparq_core::Graph;
use sparq_vectors::{query_vec, QueryResult, VectorStore};

fn tmp_path(name: &str) -> std::path::PathBuf {
    let mut p = std::env::temp_dir();
    p.push(format!(
        "sparq_vec_fbgpt_{}_{}.spqv",
        std::process::id(),
        name
    ));
    p
}

/// Six owner entities a..f on the unit circle. Each `:owns` a thing; only some of those
/// things are a `:Vehicle`. The neighbour variable is the OWNER `?node`; the `:Vehicle`
/// type sits one hop away on the owned thing — so the constraint only bites through a
/// transitive (2-hop) join.
///
/// Owners + vectors (same geometry as filtered_bgp.rs so a +x query has a known order):
///   a (+x)        owns ta — ta a :Vehicle
///   c (~+x)       owns tc — tc a :Vehicle
///   b (+y)        owns tb — tb a :Vehicle
///   d (-x)        owns td — td a :Gadget   (NOT a Vehicle)
///   e (~+y)       owns te — te a :Gadget   (NOT a Vehicle)
///   f (~+x)       owns tf — tf a :Gadget   (NOT a Vehicle)  <- closest non-a to +x, but no Vehicle
///
/// So the Vehicle-owners are {a,b,c}; the +x query's closest non-a (f) is a Gadget-owner
/// that the transitive constraint must exclude — exactly the single-variable fixture's
/// shape, lifted one join hop.
fn fixture(name: &str) -> (Graph, VectorStore) {
    let g = Graph::load_str(
        r#"
        <http://ex/a> <http://ex/owns> <http://ex/ta> .
        <http://ex/b> <http://ex/owns> <http://ex/tb> .
        <http://ex/c> <http://ex/owns> <http://ex/tc> .
        <http://ex/d> <http://ex/owns> <http://ex/td> .
        <http://ex/e> <http://ex/owns> <http://ex/te> .
        <http://ex/f> <http://ex/owns> <http://ex/tf> .
        <http://ex/ta> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://ex/Vehicle> .
        <http://ex/tb> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://ex/Vehicle> .
        <http://ex/tc> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://ex/Vehicle> .
        <http://ex/td> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://ex/Gadget> .
        <http://ex/te> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://ex/Gadget> .
        <http://ex/tf> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://ex/Gadget> .
        "#,
        "ntriples",
    )
    .unwrap();
    let id = |s: &str| {
        g.id_of(&Term::NamedNode(NamedNode::new(s).unwrap()))
            .unwrap()
    };
    let mut store = VectorStore::create(tmp_path(name), 2).unwrap();
    store.put(id("http://ex/a"), &[1.0, 0.0]).unwrap(); // +x   Vehicle-owner
    store.put(id("http://ex/b"), &[0.0, 1.0]).unwrap(); // +y   Vehicle-owner
    store.put(id("http://ex/c"), &[0.9, 0.1]).unwrap(); // ~+x  Vehicle-owner
    store.put(id("http://ex/d"), &[-1.0, 0.0]).unwrap(); // -x  Gadget-owner
    store.put(id("http://ex/e"), &[0.2, 0.98]).unwrap(); // ~+y Gadget-owner
    store.put(id("http://ex/f"), &[0.95, 0.05]).unwrap(); // ~+x Gadget-owner (closest to +x after a)
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

/// 2-hop transitive constraint: `?node :owns ?x . ?x a :Vehicle` restricts `?node` to
/// owners of a Vehicle even though `?node` never appears in the `:Vehicle` pattern. The
/// closer Gadget-owner `f` must be excluded; the next Vehicle-owner `b` returns instead.
#[test]
fn transitive_two_hop_restricts_correctly() {
    let (g, store) = fixture("transitive");
    let r = query_vec(
        &g,
        &format!(
            "{PFX} SELECT ?node WHERE {{
               ?node vec:nearest ( \"1,0\" 3 ) .
               ?node <http://ex/owns> ?x .
               ?x rdf:type <http://ex/Vehicle> .
             }}"
        ),
        &store,
    )
    .unwrap();
    let mut got = iris(&r, 0);
    got.sort();
    got.dedup();
    // +x query restricted to Vehicle-owners {a,b,c}: top-3 are a, c, b.
    assert_eq!(
        got,
        vec![
            "http://ex/a".to_string(),
            "http://ex/b".to_string(),
            "http://ex/c".to_string()
        ],
        "transitive constraint must admit only Vehicle-owners: {r:?}"
    );
    assert!(
        !got.contains(&"http://ex/f".to_string()),
        "the closer Gadget-owner `f` must be filtered out by the 2-hop :Vehicle constraint: {r:?}"
    );
}

/// Filtered top-k via the transitive constraint ≡ post-filtering the unfiltered top-k by
/// the SAME (transitive) constraint, AND filtered ⊆ unfiltered.
#[test]
fn transitive_filtered_equals_postfilter_and_is_subset() {
    let (g, store) = fixture("transitive_postfilter");

    // Unfiltered: all 6 neighbours of a +x query, best-first.
    let unfiltered = query_vec(
        &g,
        &format!("{PFX} SELECT ?node WHERE {{ ?node vec:nearest ( \"1,0\" 6 ) }}"),
        &store,
    )
    .unwrap();
    let unfiltered_order = iris(&unfiltered, 0);

    // The transitively-admitted set (Vehicle-owners), via plain SPARQL (no vec:).
    let owners: std::collections::HashSet<String> = query_vec(
        &g,
        &format!(
            "{PFX} SELECT ?node WHERE {{
               ?node <http://ex/owns> ?x . ?x rdf:type <http://ex/Vehicle> }}"
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

    // Post-filter: unfiltered order, keep only Vehicle-owners, take top-2.
    let k = 2;
    let postfilter: Vec<String> = unfiltered_order
        .iter()
        .filter(|n| owners.contains(*n))
        .take(k)
        .cloned()
        .collect();

    // Filtered search: same query + the transitive constraint, k=2.
    let filtered = query_vec(
        &g,
        &format!(
            "{PFX} SELECT ?node WHERE {{
               ( ?node ?score ) vec:search ( \"1,0\" {k} ) .
               ?node <http://ex/owns> ?x .
               ?x rdf:type <http://ex/Vehicle> .
             }} ORDER BY DESC(?score)"
        ),
        &store,
    )
    .unwrap();
    let filtered_order = iris(&filtered, 0);

    assert_eq!(
        filtered_order, postfilter,
        "transitive filtered top-k must equal post-filtering the unfiltered top-k\nfiltered={filtered_order:?}\npostfilter={postfilter:?}"
    );
    let unfiltered_set: std::collections::HashSet<&String> = unfiltered_order.iter().collect();
    for n in &filtered_order {
        assert!(
            unfiltered_set.contains(n),
            "filtered neighbour {n} not in the unfiltered result {unfiltered_order:?}"
        );
    }
}

/// A pattern with no shared-variable path to `?node` (`?y rdf:type :Vehicle`, where `?y`
/// is bound by nothing the neighbour touches) is DISCONNECTED and must not narrow the
/// mask. Without it the +x top-2 over all owners is {a, f}; the disconnected pattern must
/// leave that unchanged (Gadget-owner `f` survives).
#[test]
fn disconnected_pattern_does_not_affect_mask() {
    let (g, store) = fixture("disconnected");
    let r = query_vec(
        &g,
        // `?node :owns ?x` connects ?node↔?x. `?y rdf:type :Vehicle` shares NO variable
        // with that component (?y is fresh), so it is disconnected from ?node and must not
        // constrain it. The mask is derived from `?node :owns ?x` alone (all owners), so
        // the Gadget-owner f is admitted — exactly the unfiltered-over-owners top-2.
        &format!(
            "{PFX} SELECT ?node WHERE {{
               ?node vec:nearest ( \"1,0\" 2 ) .
               ?node <http://ex/owns> ?x .
               ?y rdf:type <http://ex/Vehicle> .
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
        "a disconnected `?y a :Vehicle` must NOT narrow ?node's mask (f survives): {r:?}"
    );
}

/// Regression guard: the single-variable case (constraint mentions `?node` directly, no
/// intermediate join) must yield exactly the old mask. Same shape as the sq-bvmd
/// `mask_restricts_to_bgp_satisfying_candidates`, but here the direct constraint sits on
/// the OWNED-thing identity to prove the 1-hop component equals the old direct-mention set.
#[test]
fn single_variable_case_unchanged() {
    // A fixture where `?node` is directly typed (no transitive hop) — the component is the
    // single directly-mentioning pattern, identical to the pre-sq-3tjd rule.
    let g = Graph::load_str(
        r#"
        <http://ex/a> <http://ex/kind> <http://ex/Car> .
        <http://ex/b> <http://ex/kind> <http://ex/Car> .
        <http://ex/c> <http://ex/kind> <http://ex/Car> .
        <http://ex/f> <http://ex/kind> <http://ex/Boat> .
        "#,
        "ntriples",
    )
    .unwrap();
    let id = |s: &str| {
        g.id_of(&Term::NamedNode(NamedNode::new(s).unwrap()))
            .unwrap()
    };
    let mut store = VectorStore::create(tmp_path("single_var"), 2).unwrap();
    store.put(id("http://ex/a"), &[1.0, 0.0]).unwrap();
    store.put(id("http://ex/b"), &[0.0, 1.0]).unwrap();
    store.put(id("http://ex/c"), &[0.9, 0.1]).unwrap();
    store.put(id("http://ex/f"), &[0.95, 0.05]).unwrap();

    let r = query_vec(
        &g,
        &format!(
            "{PFX} SELECT ?node WHERE {{
               ?node vec:nearest ( \"1,0\" 3 ) .
               ?node <http://ex/kind> <http://ex/Car> .
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
        "single-variable direct constraint must behave exactly as before (no Boat f): {r:?}"
    );
}

/// Multiple `vec:` requests in one BGP, each with its OWN connecting constraint, get their
/// own connected-component mask. `?n1` is constrained to Vehicle-owners (transitive);
/// `?n2` is left unconstrained — proving the per-request component derivation is
/// independent (one request's constraint does not leak into the other's mask).
#[test]
fn multiple_requests_each_own_component_mask() {
    let (g, store) = fixture("multi_request");
    let r = query_vec(
        &g,
        &format!(
            "{PFX} SELECT ?n1 ?n2 WHERE {{
               ?n1 vec:nearest ( \"1,0\" 2 ) .
               ?n1 <http://ex/owns> ?x .
               ?x rdf:type <http://ex/Vehicle> .
               ?n2 vec:nearest ( \"1,0\" 2 ) .
             }}"
        ),
        &store,
    )
    .unwrap();

    // ?n1: transitively constrained to Vehicle-owners → +x top-2 = {a, c} (f excluded).
    let mut n1: Vec<String> = iris(&r, 0);
    n1.sort();
    n1.dedup();
    assert_eq!(
        n1,
        vec!["http://ex/a".to_string(), "http://ex/c".to_string()],
        "?n1's own component (Vehicle-owners) must exclude f: {r:?}"
    );

    // ?n2: unconstrained → plain unfiltered +x top-2 = {a, f} (the Gadget-owner returns,
    // proving ?n1's mask did NOT leak into ?n2's search).
    let mut n2: Vec<String> = iris(&r, 1);
    n2.sort();
    n2.dedup();
    assert_eq!(
        n2,
        vec!["http://ex/a".to_string(), "http://ex/f".to_string()],
        "?n2 is unconstrained → unfiltered (f returns), independent of ?n1's mask: {r:?}"
    );
}
