//! (sq-lk3aw.4) End-to-end EXACT-candidate spatial pushdown: the real
//! `GeoIndexProvider::candidates_exact` (topology_index) certifying a constant-region
//! `geof:sfWithin` / `geof:sfContains` to the engine, which skips the residual DE-9IM
//! FILTER for certified rows. The `topology_index` feature enables the engine's
//! `spatial-exact-pushdown` seam (a weak feature edge in Cargo.toml), so these tests
//! ARE the feature-ON leg; the same queries without a provider installed take the
//! per-row path the feature-OFF build always takes — the differential oracle.
//!
//! Mirrors `tests/pushdown.rs`'s correctness-proof structure: identical results to
//! post-hoc, PLUS the counter evidence that the residual exact check ran ZERO times
//! on an all-indexed corpus (the refinement is eliminated, not reordered).
#![cfg(all(feature = "engine", feature = "topology_index"))]

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use sparq_core::Graph;
use sparq_engine::{
    query, with_functions, with_spatial_index, FunctionRegistry, QueryResult, SpatialProvider,
};
use sparq_geo::{geof_registry, GeoIndex, GeoIndexProvider};

const PREFIXES: &str = "PREFIX geo:  <http://www.opengis.net/ont/geosparql#> \
                        PREFIX geof: <http://www.opengis.net/def/function/geosparql/> \
                        PREFIX ex:   <http://ex/> ";
const GEOF: &str = "http://www.opengis.net/def/function/geosparql/";

/// A DIAMOND region: AABB `[0,4]²`, so grid points like `(0,0)`/`(4,4)` are inside
/// the AABB (superset candidates) but OUTSIDE the region (exact-excluded).
const DIAMOND: &str = "POLYGON((2 0, 4 2, 2 4, 0 2, 2 0))";

/// Wraps the real `geof_registry` counting `sfWithin`/`sfContains` invocations —
/// how many residual DE-9IM exact checks the engine performs.
fn counting_registry(counter: Arc<AtomicUsize>) -> FunctionRegistry {
    let real = geof_registry();
    let mut reg = FunctionRegistry::new();
    for name in ["sfWithin", "sfContains"] {
        let iri = format!("{GEOF}{name}");
        let inner = real.get(&iri).expect("registered").clone();
        let c = counter.clone();
        reg.register(iri, move |args: &[oxrdf::Term]| {
            c.fetch_add(1, Ordering::Relaxed);
            inner(args)
        });
    }
    reg
}

/// An n×n unit grid of INDEXED features (the `geo:hasGeometry`/`geo:asWKT` shape
/// `GeoIndex` extracts), plus — when `mixed` — two bindings bound via `ex:wkt`
/// with literals the index NEVER saw: `ex:u_in` inside the diamond (the residual
/// FILTER must KEEP it) and `ex:u_out` outside it (the residual FILTER must DROP
/// it — the row a skip-for-all-rows mutant would wrongly admit).
fn corpus(n: i32, mixed: bool) -> Graph {
    let mut ttl = String::from(
        "@prefix geo: <http://www.opengis.net/ont/geosparql#> .\n@prefix ex: <http://ex/> .\n",
    );
    for x in 0..n {
        for y in 0..n {
            ttl.push_str(&format!(
                "ex:f{x}_{y} geo:hasGeometry ex:g{x}_{y} . ex:g{x}_{y} geo:asWKT \"POINT({x} {y})\"^^geo:wktLiteral .\n"
            ));
        }
    }
    if mixed {
        // Fractional coordinates: term-identical to NO indexed literal.
        ttl.push_str(
            "ex:u_in ex:wkt \"POINT(2.25 2.25)\"^^geo:wktLiteral .\n\
             ex:u_out ex:wkt \"POINT(0.25 0.25)\"^^geo:wktLiteral .\n",
        );
    }
    Graph::load_str(&ttl, "turtle").unwrap()
}

fn rows_sorted(r: &QueryResult) -> Vec<Vec<String>> {
    let mut out: Vec<Vec<String>> = r
        .rows
        .iter()
        .map(|row| row.iter().map(|c| c.as_ref().map(|t| t.to_string()).unwrap_or_default()).collect())
        .collect();
    out.sort();
    out
}

/// Runs `sparql` post-hoc and with the real provider installed, asserting identical
/// results, returning `(rows, residual_checks_posthoc, residual_checks_pushed)`.
fn differential(graph: &Graph, sparql: &str) -> (Vec<Vec<String>>, usize, usize) {
    let provider: Arc<dyn SpatialProvider> = Arc::new(GeoIndexProvider::new(GeoIndex::build(graph)));
    let counter = Arc::new(AtomicUsize::new(0));
    let reg = counting_registry(counter.clone());

    let posthoc = with_functions(&reg, || query(graph, sparql)).unwrap();
    let checks_posthoc = counter.swap(0, Ordering::Relaxed);

    let pushed = with_spatial_index(provider, || with_functions(&reg, || query(graph, sparql))).unwrap();
    let checks_pushed = counter.load(Ordering::Relaxed);

    let a = rows_sorted(&posthoc);
    let b = rows_sorted(&pushed);
    assert_eq!(a, b, "exact pushdown changed the RESULT (must equal post-hoc)\nquery: {sparql}");
    (a, checks_posthoc, checks_pushed)
}

/// All-indexed corpus, both predicate orientations: identical answers AND the
/// residual DE-9IM check runs ZERO times — the certification covered every row.
#[test]
fn all_indexed_within_and_contains_run_zero_residual_checks() {
    let g = corpus(5, false); // 25 indexed points
    for filter in [
        format!("geof:sfWithin(?wkt, \"{DIAMOND}\"^^geo:wktLiteral)"),
        format!("geof:sfContains(\"{DIAMOND}\"^^geo:wktLiteral, ?wkt)"),
    ] {
        let q = format!(
            "{PREFIXES} SELECT ?f WHERE {{ \
               ?f geo:hasGeometry ?n . ?n geo:asWKT ?wkt . FILTER({filter}) }}"
        );
        let (rows, checks_posthoc, checks_pushed) = differential(&g, &q);
        // DE-9IM within = the region's INTERIOR: boundary lattice points
        // (|x-2|+|y-2| == 2) are NOT within, leaving the 5 with |x-2|+|y-2| <= 1.
        assert_eq!(rows.len(), 5, "query: {q}");
        assert_eq!(checks_posthoc, 25, "post-hoc DE-9IM-checks every binding");
        assert_eq!(
            checks_pushed, 0,
            "every binding is indexed and certified: the residual DE-9IM check must \
             run ZERO times\nquery: {q}"
        );
    }
}

/// Mixed corpus (indexed grid + two literals the index never saw): identical
/// answers; the not-indexed in-region binding is KEPT (judged by the residual
/// FILTER), the not-indexed out-of-region one DROPPED; the residual input shrank.
#[test]
fn mixed_corpus_not_indexed_bindings_are_judged_by_the_residual_filter() {
    let g = corpus(5, true);
    for filter in [
        format!("geof:sfWithin(?wkt, \"{DIAMOND}\"^^geo:wktLiteral)"),
        format!("geof:sfContains(\"{DIAMOND}\"^^geo:wktLiteral, ?wkt)"),
    ] {
        // One variable bound from BOTH shapes: indexed asWKT literals and the
        // never-indexed ex:wkt literals flow through the SAME FILTER.
        let q = format!(
            "{PREFIXES} SELECT ?f WHERE {{ \
               ?f ex:wkt ?wkt . FILTER({filter}) }}"
        );
        let (rows, checks_posthoc, checks_pushed) = differential(&g, &q);
        assert_eq!(
            rows,
            vec![vec!["<http://ex/u_in>".to_string()]],
            "u_in (in-region, not indexed) kept via the residual FILTER; u_out dropped\nquery: {q}"
        );
        assert_eq!(checks_posthoc, 2);
        assert_eq!(
            checks_pushed, 2,
            "not-indexed bindings MUST still be DE-9IM-judged (never certified)\nquery: {q}"
        );
    }
}

/// Mixed bindings on ONE variable (indexed + not-indexed via UNION-free property
/// mixing): the certified rows skip nothing they shouldn't — result identical,
/// residual runs only on the survivors.
#[test]
fn shared_variable_mixed_bindings_shrink_but_keep_the_residual() {
    let mut ttl = String::from(
        "@prefix geo: <http://www.opengis.net/ont/geosparql#> .\n@prefix ex: <http://ex/> .\n",
    );
    for x in 0..5 {
        for y in 0..5 {
            ttl.push_str(&format!(
                "ex:f{x}_{y} geo:hasGeometry ex:g{x}_{y} . ex:g{x}_{y} geo:asWKT \"POINT({x} {y})\"^^geo:wktLiteral .\n\
                 ex:f{x}_{y} ex:wkt \"POINT({x} {y})\"^^geo:wktLiteral .\n"
            ));
        }
    }
    // Two extra features whose ex:wkt literals the index never saw.
    ttl.push_str(
        "ex:u_in ex:wkt \"POINT(2.25 2.25)\"^^geo:wktLiteral .\n\
         ex:u_out ex:wkt \"POINT(0.25 0.25)\"^^geo:wktLiteral .\n",
    );
    let g = Graph::load_str(&ttl, "turtle").unwrap();
    let q = format!(
        "{PREFIXES} SELECT ?f WHERE {{ \
           ?f ex:wkt ?wkt . FILTER(geof:sfWithin(?wkt, \"{DIAMOND}\"^^geo:wktLiteral)) }}"
    );
    let (rows, checks_posthoc, checks_pushed) = differential(&g, &q);
    // 5 strictly-inside grid features + u_in.
    assert_eq!(rows.len(), 6);
    assert_eq!(checks_posthoc, 27, "post-hoc checks all 25 grid + 2 unindexed bindings");
    // Not-indexed rows survive, so the residual FILTER still runs — over the 7
    // survivors of the exact restriction (5 certified rows, on which it is an
    // identity, + the 2 not-indexed it must judge), not the original 27.
    assert_eq!(checks_pushed, 7, "residual input shrank to the exact-restriction survivors");
}
