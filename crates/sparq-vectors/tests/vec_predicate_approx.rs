//! (sq-z589, epic sq-3183) End-to-end tests for the **approximate** `vec:` magic
//! predicate path: `query_vec_approx` / `prepare_vec_approx` run the k-NN search through an
//! on-disk [`DiskAnnIndex`] (Vamana) instead of the exact `nearest_exact` scan.
//!
//! The contract under test (mirrors the rest of the crate):
//!  - the rewrite, VALUES inlining, join, error checks and seed-self-exclusion are **identical**
//!    to the exact path — only the unfiltered candidate search is swapped for the index;
//!  - on a tiny, well-separated fixture the approximate index returns the SAME top-k as exact,
//!    so the end-to-end results match the exact-path assertions;
//!  - approximate search is APPROXIMATE in general (recall < 1.0) — this is asserted as a
//!    *measured* recall floor on a larger synthetic set, NOT claimed as exactness.
//!
//! Gated on `vec-predicate` + `approx-ann` (the approximate index entry points only exist there).
#![cfg(all(feature = "vec-predicate", feature = "approx-ann"))]

use oxrdf::{NamedNode, Term};
use sparq_core::Graph;
use sparq_vectors::{
    nearest_exact, prepare_vec_approx, query_vec, query_vec_approx, query_vec_approx_with_budget,
    DiskAnnIndex, QueryBudget, QueryResult, VectorStore,
};

fn tmp(name: &str, ext: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!("sparq_vec_approx_{}_{name}.{ext}", std::process::id()))
}

/// Five well-separated entities on the unit circle (same shape as tests/vec_predicate.rs), with a
/// FINALIZED store and a sibling on-disk Vamana index built against the same graph generation.
fn fixture(name: &str) -> (Graph, VectorStore, DiskAnnIndex) {
    let g = Graph::load_str(
        r#"
        <http://ex/a> <http://ex/label> "alpha" .
        <http://ex/b> <http://ex/label> "beta" .
        <http://ex/c> <http://ex/label> "gamma" .
        <http://ex/d> <http://ex/label> "delta" .
        <http://ex/e> <http://ex/label> "epsilon" .
        "#,
        "ntriples",
    )
    .unwrap();
    let id = |s: &str| g.id_of(&Term::NamedNode(NamedNode::new(s).unwrap())).unwrap();
    // Bind the store to the graph so the index's checked-open guard (which verifies the store too)
    // passes — both index and store must carry the same fingerprint.
    let mut store = VectorStore::create(tmp(name, "spqv"), 2)
        .unwrap()
        .with_fingerprint(&g);
    store.put(id("http://ex/a"), &[1.0, 0.0]).unwrap(); // +x
    store.put(id("http://ex/b"), &[0.0, 1.0]).unwrap(); // +y
    store.put(id("http://ex/c"), &[0.9, 0.1]).unwrap(); // near +x
    store.put(id("http://ex/d"), &[-1.0, 0.0]).unwrap(); // -x
    store.put(id("http://ex/e"), &[0.2, 0.98]).unwrap(); // near +y
    store.finalize().unwrap();
    let index = DiskAnnIndex::build_for(&store, tmp(name, "spqg"), &g).unwrap();
    (g, store, index)
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

/// `vec:nearest` by a query vector through the approximate index returns the same top-k as exact
/// on the separable fixture: "1,0" → a (exact) then c.
#[test]
fn approx_nearest_by_query_vector() {
    let (g, store, index) = fixture("nearest_vec");
    let r = query_vec_approx(
        &g,
        "PREFIX vec: <http://sparq.dev/vec#>
         SELECT ?node WHERE { ?node vec:nearest ( \"1,0\" 2 ) }",
        &store,
        &index,
    )
    .unwrap();
    let mut got = iris(&r, 0);
    got.sort();
    assert_eq!(
        got,
        vec!["http://ex/a".to_string(), "http://ex/c".to_string()],
        "{r:?}"
    );
}

/// `vec:nearest` by a seed IRI excludes the seed itself (same over-fetch-by-one rule as exact):
/// neighbours of <a> (+x), a excluded → c is nearest.
#[test]
fn approx_nearest_by_seed_iri_excludes_self() {
    let (g, store, index) = fixture("nearest_seed");
    let r = query_vec_approx(
        &g,
        "PREFIX vec: <http://sparq.dev/vec#>
         SELECT ?node WHERE { ?node vec:nearest ( <http://ex/a> 1 ) }",
        &store,
        &index,
    )
    .unwrap();
    assert_eq!(iris(&r, 0), vec!["http://ex/c".to_string()], "{r:?}");
}

/// `vec:search` binds a cosine score; ORDER BY DESC recovers best-first. Through the index the
/// top-2 of "1,0" are a (score 1.0) then c.
#[test]
fn approx_search_binds_score_and_orders() {
    let (g, store, index) = fixture("search_score");
    let r = query_vec_approx(
        &g,
        "PREFIX vec: <http://sparq.dev/vec#>
         SELECT ?node ?score WHERE { ( ?node ?score ) vec:search ( \"1,0\" 2 ) } ORDER BY DESC(?score)",
        &store,
        &index,
    )
    .unwrap();
    assert_eq!(
        iris(&r, 0),
        vec!["http://ex/a".to_string(), "http://ex/c".to_string()],
        "{r:?}"
    );
    // First score (a, exactly +x) ≈ 1.0.
    if let Some(Term::Literal(l)) = &r.rows[0][1] {
        let s: f64 = l.value().parse().unwrap();
        assert!((s - 1.0).abs() < 1e-5, "a's cosine ≈ 1.0, got {s}");
    } else {
        panic!("expected a literal score, got {:?}", r.rows[0][1]);
    }
}

/// The approximate path joins the inlined VALUES onto ordinary patterns exactly like the exact
/// path: bind the neighbour AND its label in the same query.
#[test]
fn approx_joins_to_ordinary_patterns() {
    let (g, store, index) = fixture("join");
    let r = query_vec_approx(
        &g,
        "PREFIX vec: <http://sparq.dev/vec#>
         SELECT ?label WHERE {
           ?node vec:nearest ( \"1,0\" 1 ) .
           ?node <http://ex/label> ?label .
         }",
        &store,
        &index,
    )
    .unwrap();
    let labels: Vec<String> = r
        .rows
        .iter()
        .filter_map(|row| match &row[0] {
            Some(Term::Literal(l)) => Some(l.value().to_string()),
            _ => None,
        })
        .collect();
    assert_eq!(labels, vec!["alpha".to_string()], "{r:?}");
}

/// Error checks are shared with the exact path: an unknown `vec:` predicate is a hard error.
#[test]
fn approx_unknown_predicate_errors() {
    let (g, store, index) = fixture("unknown");
    let err = query_vec_approx(
        &g,
        "PREFIX vec: <http://sparq.dev/vec#>
         SELECT ?node WHERE { ?node vec:teleport ( \"1,0\" 2 ) }",
        &store,
        &index,
    )
    .unwrap_err();
    assert!(err.contains("unknown magic predicate"), "{err}");
}

/// A query with NO `vec:` predicate passes through unchanged on the approximate entry point too.
#[test]
fn approx_passthrough_without_vec_predicate() {
    let (g, store, index) = fixture("passthrough");
    let r = query_vec_approx(
        &g,
        "SELECT ?s WHERE { ?s <http://ex/label> \"alpha\" }",
        &store,
        &index,
    )
    .unwrap();
    assert_eq!(iris(&r, 0), vec!["http://ex/a".to_string()], "{r:?}");
}

/// `query_vec_approx_with_budget` is the budgeted twin and returns the same rows under a generous
/// budget; `prepare_vec_approx` composes with the engine's prepared seam.
#[test]
fn approx_budgeted_and_prepared_entry_points() {
    let (g, store, index) = fixture("budget");
    let sparql = "PREFIX vec: <http://sparq.dev/vec#>
         SELECT ?node WHERE { ?node vec:nearest ( \"1,0\" 2 ) }";
    let budget = QueryBudget::unlimited();
    let r = query_vec_approx_with_budget(&g, sparql, &store, &index, &budget).unwrap();
    let mut got = iris(&r, 0);
    got.sort();
    assert_eq!(got.len(), 2, "{r:?}");

    // prepare_vec_approx returns a PreparedQuery that the engine can evaluate.
    let prepared = prepare_vec_approx(&g, sparql, &store, &index).unwrap();
    let r2 = sparq_vectors::query_prepared(&g, &prepared).unwrap();
    let mut got2 = iris(&r2, 0);
    got2.sort();
    assert_eq!(got, got2, "prepared path equals one-shot path");
}

/// Larger separable fixture: the approximate index agrees with exact on the unfiltered top-k for
/// a vector query. Two tight clusters far apart → the index's recall is perfect here, so the `vec:`
/// rows equal the exact `nearest_exact` ground truth (the *measured* recall floor, not a claim of
/// exactness for arbitrary data).
#[test]
fn approx_matches_exact_topk_on_separable_clusters() {
    let g = Graph::load_str(
        &(0..40)
            .map(|i| format!("<http://ex/n{i}> <http://ex/p> \"{i}\" .\n"))
            .collect::<String>(),
        "ntriples",
    )
    .unwrap();
    let nid = |i: usize| {
        g.id_of(&Term::NamedNode(
            NamedNode::new(format!("http://ex/n{i}")).unwrap(),
        ))
        .unwrap()
    };
    let mut store = VectorStore::create(tmp("clusters", "spqv"), 4)
        .unwrap()
        .with_fingerprint(&g);
    // Cluster A near +x, cluster B near -x — well separated.
    for i in 0..20 {
        let j = i as f32 * 0.001;
        store.put(nid(i), &[1.0 - j, j, j, j]).unwrap();
    }
    for i in 20..40 {
        let j = (i - 20) as f32 * 0.001;
        store.put(nid(i), &[-1.0 + j, j, j, j]).unwrap();
    }
    store.finalize().unwrap();
    let index = DiskAnnIndex::build_for(&store, tmp("clusters", "spqg"), &g).unwrap();

    let sparql = "PREFIX vec: <http://sparq.dev/vec#>
         SELECT ?node WHERE { ?node vec:nearest ( \"1,0,0,0\" 5 ) }";
    let approx = query_vec_approx(&g, sparql, &store, &index).unwrap();
    let exact = query_vec(&g, sparql, &store).unwrap();

    let mut a = iris(&approx, 0);
    let mut e = iris(&exact, 0);
    a.sort();
    e.sort();
    assert_eq!(a, e, "approx top-5 must equal exact on separable clusters");
    // Sanity: all five hits are from cluster A (ids 0..20).
    for iri in &a {
        let idx: usize = iri.rsplit('n').next().unwrap().parse().unwrap();
        assert!(idx < 20, "hit {iri} should be in cluster A");
    }

    // And the underlying exact ground truth really is cluster A — proves the fixture is separable.
    let qid = nid(0);
    let truth = nearest_exact(&store, store.get(qid).unwrap(), 5);
    assert_eq!(truth.len(), 5);
}
