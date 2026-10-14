//! (sq-bvmd, epic sq-3183) End-to-end tests for the engine-side
//! **filtered-ANN BGP→IdMask wiring** of the `vec:` magic predicate (follow-up to
//! sq-1wc1/#242). When a `vec:` neighbour variable is also constrained by ordinary
//! patterns in the same BGP, those patterns carve out a candidate id-set (the
//! `IdMask`) and the k-NN search is restricted to it.
//!
//! Correctness invariant proven here:
//! - the filtered top-k is **identical** to post-filtering the *unfiltered* top-k by
//!   the same BGP constraint (filtered-search ≡ post-filter, the whole point — same
//!   answer, derived without scanning the rest of the store);
//! - the filtered result is therefore a **subset** of the unfiltered result;
//! - an **unconstrained** neighbour variable falls back to the exact unfiltered path
//!   (the pre-sq-bvmd behaviour, preserved).
//!
//! The path is gated on BOTH `vec-predicate` (the rewrite) and `filtered-ann` (the
//! `IdMask` API), so the whole file only compiles with both on.
#![cfg(all(feature = "vec-predicate", feature = "filtered-ann"))]

use oxrdf::{NamedNode, Term};
use sparq_core::Graph;
use sparq_vectors::{query_vec, QueryResult, VectorStore};

fn tmp_path(name: &str) -> std::path::PathBuf {
    let mut p = std::env::temp_dir();
    p.push(format!(
        "sparq_vec_fbgp_{}_{}.spqv",
        std::process::id(),
        name
    ));
    p
}

/// Six entities on the unit circle, each tagged with a `:kind` so a BGP can carve out
/// a candidate subset. The +x cluster (a, c) and the +y cluster (b, e) let a query
/// pick a winner that is *excluded* by the constraint, exercising the filter.
///
/// kinds: a,b,c → :Car ; d,e,f → :Boat. Vectors:
///   a (+x)        :Car
///   c (near +x)   :Car
///   b (+y)        :Car
///   d (-x)        :Boat
///   e (near +y)   :Boat
///   f (near +x)   :Boat   <- closest non-a to a +x query, but a Boat
fn fixture(name: &str) -> (Graph, VectorStore) {
    let g = Graph::load_str(
        r#"
        <http://ex/a> <http://ex/kind> <http://ex/Car> .
        <http://ex/b> <http://ex/kind> <http://ex/Car> .
        <http://ex/c> <http://ex/kind> <http://ex/Car> .
        <http://ex/d> <http://ex/kind> <http://ex/Boat> .
        <http://ex/e> <http://ex/kind> <http://ex/Boat> .
        <http://ex/f> <http://ex/kind> <http://ex/Boat> .
        "#,
        "ntriples",
    )
    .unwrap();
    let id = |s: &str| {
        g.id_of(&Term::NamedNode(NamedNode::new(s).unwrap()))
            .unwrap()
    };
    let mut store = VectorStore::create(tmp_path(name), 2).unwrap();
    store.put(id("http://ex/a"), &[1.0, 0.0]).unwrap(); // +x   Car
    store.put(id("http://ex/b"), &[0.0, 1.0]).unwrap(); // +y   Car
    store.put(id("http://ex/c"), &[0.9, 0.1]).unwrap(); // ~+x  Car
    store.put(id("http://ex/d"), &[-1.0, 0.0]).unwrap(); // -x   Boat
    store.put(id("http://ex/e"), &[0.2, 0.98]).unwrap(); // ~+y  Boat
    store.put(id("http://ex/f"), &[0.95, 0.05]).unwrap(); // ~+x  Boat (closest to +x after a)
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

const PFX: &str = "PREFIX vec: <http://sparq.dev/vec#> ";

/// The candidate id-set (mask) derived from `?node a/:kind :Car` admits ONLY Cars, so a
/// +x query that would otherwise pull the Boat `f` instead returns the next-best Car.
/// Proves: the result contains only candidates that satisfy the rest of the BGP.
#[test]
fn mask_restricts_to_bgp_satisfying_candidates() {
    let (g, store) = fixture("mask_restrict");
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
    // +x query, restricted to Cars {a,b,c}: top-3 are a, c, b (b is the only other Car).
    // The Boat `f` (cosine ~0.999, beats b) must NOT appear — it fails the BGP.
    assert_eq!(
        got,
        vec![
            "http://ex/a".to_string(),
            "http://ex/b".to_string(),
            "http://ex/c".to_string()
        ],
        "filtered result must be only BGP-satisfying (Car) candidates: {r:?}"
    );
    assert!(
        !got.contains(&"http://ex/f".to_string()),
        "the closer Boat `f` must be filtered out by the :Car constraint: {r:?}"
    );
}

/// Filtered top-k ≡ post-filtering the unfiltered top-k by the same constraint, AND the
/// filtered result ⊆ unfiltered result. We compute both inside one process: the
/// unfiltered neighbours (large k) intersected with the BGP-admitted set must equal the
/// filtered neighbours.
#[test]
fn filtered_equals_postfilter_and_is_subset() {
    let (g, store) = fixture("postfilter");

    // Unfiltered: all 6 neighbours of a +x query, best-first.
    let unfiltered = query_vec(
        &g,
        &format!("{PFX} SELECT ?node WHERE {{ ?node vec:nearest ( \"1,0\" 6 ) }}"),
        &store,
    )
    .unwrap();
    let unfiltered_order = iris(&unfiltered, 0);

    // The BGP-admitted set (Cars), via a plain SPARQL query (no vec:).
    let cars: std::collections::HashSet<String> = query_vec(
        &g,
        "SELECT ?node WHERE { ?node <http://ex/kind> <http://ex/Car> }",
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

    // Post-filter: unfiltered order, keep only Cars, take top-2.
    let k = 2;
    let postfilter: Vec<String> = unfiltered_order
        .iter()
        .filter(|n| cars.contains(*n))
        .take(k)
        .cloned()
        .collect();

    // Filtered search: same query + the :Car constraint, k=2 (use vec:search +
    // ORDER BY to get a deterministic best-first order to compare against).
    let filtered = query_vec(
        &g,
        &format!(
            "{PFX} SELECT ?node WHERE {{
               ( ?node ?score ) vec:search ( \"1,0\" {k} ) .
               ?node <http://ex/kind> <http://ex/Car> .
             }} ORDER BY DESC(?score)"
        ),
        &store,
    )
    .unwrap();
    let filtered_order = iris(&filtered, 0);

    assert_eq!(
        filtered_order, postfilter,
        "filtered top-k must equal post-filtering the unfiltered top-k\nfiltered={filtered_order:?}\npostfilter={postfilter:?}"
    );
    // ⊆ unfiltered.
    let unfiltered_set: std::collections::HashSet<&String> = unfiltered_order.iter().collect();
    for n in &filtered_order {
        assert!(
            unfiltered_set.contains(n),
            "filtered neighbour {n} not in the unfiltered result {unfiltered_order:?}"
        );
    }
}

/// No constraining pattern on the neighbour variable → fall back to the exact unfiltered
/// search (pre-sq-bvmd behaviour). The closest Boat `f` is now allowed back in.
#[test]
fn unconstrained_falls_back_to_unfiltered() {
    let (g, store) = fixture("fallback");
    let r = query_vec(
        &g,
        // `?other` is constrained, but the NEIGHBOUR variable `?node` is not — so no
        // mask is derived and the search is the plain unfiltered top-2.
        &format!(
            "{PFX} SELECT ?node WHERE {{
               ?node vec:nearest ( \"1,0\" 2 ) .
               ?other <http://ex/kind> <http://ex/Car> .
             }}"
        ),
        &store,
    )
    .unwrap();
    // Cross-product join keeps every (?node, ?other) pair, so each neighbour repeats per
    // Car; dedup the neighbour column.
    let mut got: Vec<String> = iris(&r, 0);
    got.sort();
    got.dedup();
    // Unfiltered +x top-2 over ALL kinds: a (1.0) then f (~0.999) — a Boat that the
    // constrained-path test excluded. Its presence proves the fallback path.
    assert_eq!(
        got,
        vec!["http://ex/a".to_string(), "http://ex/f".to_string()],
        "unconstrained ?node must use the unfiltered search (Boat f returns): {r:?}"
    );
}

/// A constraint that admits NOTHING (no entity is a :Plane) yields no neighbours — the
/// honest answer, distinct from "unfiltered".
#[test]
fn empty_constraint_yields_no_rows() {
    let (g, store) = fixture("empty_mask");
    let r = query_vec(
        &g,
        &format!(
            "{PFX} SELECT ?node WHERE {{
               ?node vec:nearest ( \"1,0\" 3 ) .
               ?node <http://ex/kind> <http://ex/Plane> .
             }}"
        ),
        &store,
    )
    .unwrap();
    assert!(
        r.is_empty(),
        "an unsatisfiable constraint admits no candidates: {r:?}"
    );
}

/// (sq-36ol) ADVERSARIAL end-to-end cache-invalidation through the PUBLIC
/// `query_vec` seam: the derived-mask cache must never serve a stale mask after the graph
/// changes. We run the same constrained `vec:` query against graph g1 (Cars {a,b,c}), then
/// against a MUTATED graph g2 that demotes `c` to a Boat. With a sound cache the second query
/// must reflect the change — `c` is no longer an admissible Car neighbour — NOT replay the
/// cached g1 result. (Caching is transparent: the only observable is the correct answer.)
#[test]
fn mutation_changes_filtered_result_through_query_vec() {
    // g1: a,b,c are Cars (matches the standard fixture's kinds for a,b,c).
    let (g1, store1) = fixture("cache_mut_g1");
    let q = format!(
        "{PFX} SELECT ?node WHERE {{
           ?node vec:nearest ( \"1,0\" 3 ) .
           ?node <http://ex/kind> <http://ex/Car> .
         }}"
    );
    let mut got1 = iris(&query_vec(&g1, &q, &store1).unwrap(), 0);
    got1.sort();
    assert_eq!(
        got1,
        vec![
            "http://ex/a".to_string(),
            "http://ex/b".to_string(),
            "http://ex/c".to_string()
        ],
        "g1: the three Cars are admissible"
    );

    // g2: MUTATION — `c` is now a Boat (its :kind triple changed). The store is rebuilt against
    // g2 so the fingerprint binding holds; the constraining sub-BGP is byte-identical to g1's, so
    // a sub-BGP-only cache would WRONGLY replay the g1 mask (still admitting `c`). The graph
    // fingerprint differs (a changed triple), so the real cache MISSES and recomputes.
    let g2 = Graph::load_str(
        r#"
        <http://ex/a> <http://ex/kind> <http://ex/Car> .
        <http://ex/b> <http://ex/kind> <http://ex/Car> .
        <http://ex/c> <http://ex/kind> <http://ex/Boat> .
        <http://ex/d> <http://ex/kind> <http://ex/Boat> .
        <http://ex/e> <http://ex/kind> <http://ex/Boat> .
        <http://ex/f> <http://ex/kind> <http://ex/Boat> .
        "#,
        "ntriples",
    )
    .unwrap();
    let id2 = |s: &str| {
        g2.id_of(&Term::NamedNode(NamedNode::new(s).unwrap()))
            .unwrap()
    };
    let mut store2 = VectorStore::create(tmp_path("cache_mut_g2"), 2).unwrap();
    store2.put(id2("http://ex/a"), &[1.0, 0.0]).unwrap();
    store2.put(id2("http://ex/b"), &[0.0, 1.0]).unwrap();
    store2.put(id2("http://ex/c"), &[0.9, 0.1]).unwrap();
    store2.put(id2("http://ex/d"), &[-1.0, 0.0]).unwrap();
    store2.put(id2("http://ex/e"), &[0.2, 0.98]).unwrap();
    store2.put(id2("http://ex/f"), &[0.95, 0.05]).unwrap();

    let mut got2 = iris(&query_vec(&g2, &q, &store2).unwrap(), 0);
    got2.sort();
    // `c` is no longer a Car: it MUST be gone. The stale g1 result (which still contained c) must
    // NOT be served. Remaining admissible Cars are {a, b}.
    assert!(
        !got2.contains(&"http://ex/c".to_string()),
        "after demoting c to a Boat, the cache must NOT replay the stale result still admitting c: {got2:?}"
    );
    assert_eq!(
        got2,
        vec!["http://ex/a".to_string(), "http://ex/b".to_string()],
        "g2: only a and b remain Cars"
    );
}

/// The seed (node-IRI) form, filtered: neighbours of `a` restricted to Cars exclude the
/// seed itself AND the closer Boat `f`, leaving Car `c`.
#[test]
fn seed_form_filtered_excludes_self_and_non_candidates() {
    let (g, store) = fixture("seed_filtered");
    let r = query_vec(
        &g,
        &format!(
            "{PFX} SELECT ?node WHERE {{
               ?node vec:nearest ( <http://ex/a> 1 ) .
               ?node <http://ex/kind> <http://ex/Car> .
             }}"
        ),
        &store,
    )
    .unwrap();
    assert_eq!(
        iris(&r, 0),
        vec!["http://ex/c".to_string()],
        "neighbours of a, restricted to Cars, exclude self (a) and the Boat f → c: {r:?}"
    );
}
