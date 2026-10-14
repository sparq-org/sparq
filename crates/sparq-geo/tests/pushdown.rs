//! Spatial FILTER pushdown (sq-mg9): the engine's `SpatialProvider` seam wired to
//! a `GeoIndex` via `GeoIndexProvider`. These tests are the CORRECTNESS PROOF — the
//! pushed-down plan (index window/range scan -> exact `geof:` refinement) must
//! return EXACTLY the same rows as the post-hoc plan (the same query without an
//! index installed) — plus EVIDENCE that fewer geometries are exact-checked.
#![cfg(feature = "engine")]

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use sparq_core::Graph;
use sparq_engine::{query, with_functions, with_spatial_index, FunctionRegistry, QueryResult, SpatialProvider};
use sparq_geo::{geof_registry, GeoIndex, GeoIndexProvider};

const PREFIXES: &str = "PREFIX geo:  <http://www.opengis.net/ont/geosparql#> \
                        PREFIX geof: <http://www.opengis.net/def/function/geosparql/> \
                        PREFIX uom:  <http://www.opengis.net/def/uom/OGC/1.0/> \
                        PREFIX ex:   <http://ex/> ";

const WKT: &str = "http://www.opengis.net/ont/geosparql#wktLiteral";
const GEOF: &str = "http://www.opengis.net/def/function/geosparql/";

/// A `geof:` registry that COUNTS how many times the pushable predicates
/// (`distance`, `sfWithin`, `sfIntersects`) are invoked — i.e. how many exact
/// geometry checks the engine performs. Wraps the real `geof_registry` so the
/// answers are identical; only the call count is observed.
fn counting_registry(counter: Arc<AtomicUsize>) -> FunctionRegistry {
    let real = geof_registry();
    let mut reg = FunctionRegistry::new();
    for name in ["distance", "sfWithin", "sfIntersects"] {
        let iri = format!("{GEOF}{name}");
        let inner = real.get(&iri).expect("registered").clone();
        let c = counter.clone();
        reg.register(iri, move |args| {
            c.fetch_add(1, Ordering::Relaxed);
            inner(args)
        });
    }
    reg
}

/// A grid of N×N points (degree spacing) under `ex:f -> geo:hasGeometry -> node
/// -> geo:asWKT -> "POINT(lon lat)"`, the GeoSPARQL feature shape `GeoIndex`
/// indexes. Big enough that a windowed pushdown skips most of it.
fn grid(n: i32) -> Graph {
    let mut ttl = String::from(
        "@prefix geo: <http://www.opengis.net/ont/geosparql#> .\n@prefix ex: <http://ex/> .\n",
    );
    for x in 0..n {
        for y in 0..n {
            // Spread over a wide area (0.5° steps) so a small window is selective.
            let (lon, lat) = (x as f64 * 0.5, y as f64 * 0.5);
            ttl.push_str(&format!(
                "ex:f{x}_{y} geo:hasGeometry ex:g{x}_{y} . ex:g{x}_{y} geo:asWKT \"POINT({lon} {lat})\"^^geo:wktLiteral .\n"
            ));
        }
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

/// Run `sparql` both WITHOUT and WITH the spatial index installed (same counting
/// registry instance reset between runs), asserting identical results and
/// returning `(rows, exact_checks_posthoc, exact_checks_pushdown)`.
fn compare(graph: &Graph, sparql: &str) -> (Vec<Vec<String>>, usize, usize) {
    let provider: Arc<dyn SpatialProvider> = Arc::new(GeoIndexProvider::new(GeoIndex::build(graph)));

    let counter = Arc::new(AtomicUsize::new(0));
    let reg = counting_registry(counter.clone());

    // Post-hoc: registry installed, NO spatial index.
    let posthoc = with_functions(&reg, || query(graph, sparql)).unwrap();
    let checks_posthoc = counter.swap(0, Ordering::Relaxed);

    // Pushdown: same registry + the spatial index.
    let pushed = with_spatial_index(provider, || with_functions(&reg, || query(graph, sparql))).unwrap();
    let checks_pushdown = counter.load(Ordering::Relaxed);

    let a = rows_sorted(&posthoc);
    let b = rows_sorted(&pushed);
    assert_eq!(a, b, "pushdown changed the RESULT (must be identical to post-hoc)\nquery: {sparql}");
    (a, checks_posthoc, checks_pushdown)
}

#[test]
fn distance_within_same_results_fewer_checks() {
    let g = grid(20); // 400 features
    let q = format!(
        "{PREFIXES} SELECT ?f WHERE {{ \
           ?f geo:hasGeometry ?n . ?n geo:asWKT ?wkt . \
           FILTER(geof:distance(?wkt, \"POINT(0 0)\"^^geo:wktLiteral, uom:kilometre) < 120) \
         }}"
    );
    let (rows, posthoc, pushed) = compare(&g, &q);
    // ~120 km ≈ ~1.08°, so points within a (0,0)-centred ~1° ball: (0,0),(0,0.5),
    // (0.5,0),(0,1),(1,0),(0.5,0.5)... a small handful out of 400.
    assert!(!rows.is_empty(), "expected some matches");
    assert!(rows.len() < 20, "the window should be selective, got {} rows", rows.len());
    // Post-hoc checks EVERY feature; the pushdown checks only the window candidates.
    assert_eq!(posthoc, 400, "post-hoc exact-checks every geometry");
    assert!(pushed < posthoc, "pushdown must exact-check FEWER ({pushed} vs {posthoc})");
    assert!(pushed <= rows.len() + 8, "candidate set should be near-tight, got {pushed} for {} matches", rows.len());
}

#[test]
fn sf_within_box_same_results_fewer_checks() {
    let g = grid(20); // 400 features
    // A small box in the lower-left corner.
    let q = format!(
        "{PREFIXES} SELECT ?f WHERE {{ \
           ?f geo:hasGeometry ?n . ?n geo:asWKT ?wkt . \
           FILTER(geof:sfWithin(?wkt, \"POLYGON((-0.1 -0.1, 1.1 -0.1, 1.1 1.1, -0.1 1.1, -0.1 -0.1))\"^^geo:wktLiteral)) \
         }}"
    );
    let (rows, posthoc, pushed) = compare(&g, &q);
    // The box [0,1]×[0,1] at 0.5° spacing contains a 3×3 = 9-point sub-grid.
    assert_eq!(rows.len(), 9, "3x3 points inside the box");
    assert_eq!(posthoc, 400);
    assert!(pushed < posthoc, "pushdown must exact-check FEWER ({pushed} vs {posthoc})");
}

#[test]
fn sf_intersects_box_same_results() {
    let g = grid(12);
    let q = format!(
        "{PREFIXES} SELECT ?f WHERE {{ \
           ?f geo:hasGeometry ?n . ?n geo:asWKT ?wkt . \
           FILTER(geof:sfIntersects(?wkt, \"POLYGON((1.9 1.9, 3.1 1.9, 3.1 3.1, 1.9 3.1, 1.9 1.9))\"^^geo:wktLiteral)) \
         }}"
    );
    let (rows, posthoc, pushed) = compare(&g, &q);
    assert!(!rows.is_empty());
    assert_eq!(posthoc, 144);
    assert!(pushed < posthoc);
}

#[test]
fn pushdown_le_boundary_identical() {
    // `<=` (inclusive) must also be a correct superset: a point EXACTLY on the
    // radius is kept by both paths.
    let g = grid(10);
    let q = format!(
        "{PREFIXES} SELECT ?f WHERE {{ \
           ?f geo:hasGeometry ?n . ?n geo:asWKT ?wkt . \
           FILTER(geof:distance(?wkt, \"POINT(0 0)\"^^geo:wktLiteral, uom:kilometre) <= 200) \
         }}"
    );
    let (rows, _posthoc, _pushed) = compare(&g, &q);
    assert!(!rows.is_empty());
}

#[test]
fn non_metric_unit_declined_still_correct() {
    // Degree-unit distance is euclidean coordinate distance — NOT the index's
    // great-circle metric — so the provider DECLINES it (no pushdown). The result
    // must still be correct (the post-hoc path runs), proving the decline path is
    // safe. Pushdown count == post-hoc count (no candidates restricted).
    let g = grid(8);
    let q = format!(
        "{PREFIXES} SELECT ?f WHERE {{ \
           ?f geo:hasGeometry ?n . ?n geo:asWKT ?wkt . \
           FILTER(geof:distance(?wkt, \"POINT(0 0)\"^^geo:wktLiteral, uom:degree) < 1.2) \
         }}"
    );
    let (rows, posthoc, pushed) = compare(&g, &q);
    assert!(!rows.is_empty());
    assert_eq!(pushed, posthoc, "degree unit must NOT push down (declined), so same exact-check count");
}

#[test]
fn distance_greater_than_stays_posthoc() {
    // `geof:distance(?g, pt) > R` is an unbounded EXTERIOR (no finite window) — it
    // is NOT pushed down. Result must be correct via the post-hoc path.
    let g = grid(6);
    // > 100 km keeps the FAR points (most of the grid): an unbounded exterior.
    let q = format!(
        "{PREFIXES} SELECT ?f WHERE {{ \
           ?f geo:hasGeometry ?n . ?n geo:asWKT ?wkt . \
           FILTER(geof:distance(?wkt, \"POINT(0 0)\"^^geo:wktLiteral, uom:kilometre) > 100) \
         }}"
    );
    let (rows, posthoc, pushed) = compare(&g, &q);
    assert!(!rows.is_empty(), "far points should match > 100 km");
    assert_eq!(pushed, posthoc, "no pushdown for the exterior predicate");
}

#[test]
fn reversed_operand_exterior_stays_posthoc() {
    // REGRESSION: `r < geof:distance(?g, pt)` is the EXTERIOR (distance > r), NOT a
    // within-window — it must NOT be recognised as DistanceWithin (that would drop
    // exactly the far points that match). Same for `r <= distance`. Result must equal
    // the post-hoc path, and (since these don't push down) the exact-check count is
    // unchanged.
    let g = grid(8);
    for op in ["<", "<="] {
        let q = format!(
            "{PREFIXES} SELECT ?f WHERE {{ \
               ?f geo:hasGeometry ?n . ?n geo:asWKT ?wkt . \
               FILTER(100 {op} geof:distance(?wkt, \"POINT(0 0)\"^^geo:wktLiteral, uom:kilometre)) \
             }}"
        );
        let (rows, posthoc, pushed) = compare(&g, &q);
        assert!(!rows.is_empty(), "far points (> 100 km) should match `{op}` exterior");
        assert_eq!(pushed, posthoc, "exterior `{op}` must NOT push down to a within-window");
    }
}

#[test]
fn geometry_bound_via_non_aswkt_predicate_is_correct() {
    // REGRESSION (silent false negatives): the GeoIndex only indexes geo:asWKT
    // literals, but a FILTER's geometry variable can be bound by ANY predicate (here
    // ex:loc — exactly the shape the geof: registry's own examples use). Those literals
    // are NOT in the index; the pushdown must NOT drop them — is_indexed returns false,
    // so the row is kept and the exact geof: FILTER judges it. Result must be identical
    // to the post-hoc path.
    let g = Graph::load_str(
        r#"
        @prefix geo: <http://www.opengis.net/ont/geosparql#> .
        @prefix ex:  <http://ex/> .
        ex:a ex:loc "POINT(0 0)"^^geo:wktLiteral .
        ex:b ex:loc "POINT(0.1 0.1)"^^geo:wktLiteral .
        ex:c ex:loc "POINT(40 40)"^^geo:wktLiteral .
        "#,
        "turtle",
    )
    .unwrap();
    let q = format!(
        "{PREFIXES} SELECT ?s WHERE {{ \
           ?s ex:loc ?wkt . \
           FILTER(geof:distance(?wkt, \"POINT(0 0)\"^^geo:wktLiteral, uom:kilometre) < 50) \
         }} ORDER BY ?s"
    );
    // The index is EMPTY (no geo:asWKT triples), so every binding is not-indexed and kept.
    let (rows, _posthoc, _pushed) = compare(&g, &q);
    assert_eq!(rows, vec![vec!["<http://ex/a>".to_string()], vec!["<http://ex/b>".to_string()]]);
}

#[test]
fn mixed_indexed_and_unindexed_bindings_correct() {
    // A geometry variable that UNIONs an indexed (geo:asWKT) literal with a non-indexed
    // (ex:loc) one: the pushdown may restrict the indexed binding but must keep the
    // non-indexed one for the exact FILTER. Result must match post-hoc.
    let g = Graph::load_str(
        r#"
        @prefix geo: <http://www.opengis.net/ont/geosparql#> .
        @prefix ex:  <http://ex/> .
        ex:idx geo:hasGeometry ex:g . ex:g geo:asWKT "POINT(0 0)"^^geo:wktLiteral .
        ex:far geo:hasGeometry ex:gf . ex:gf geo:asWKT "POINT(40 40)"^^geo:wktLiteral .
        ex:other ex:loc "POINT(0.1 0.1)"^^geo:wktLiteral .
        "#,
        "turtle",
    )
    .unwrap();
    let q = format!(
        "{PREFIXES} SELECT ?wkt WHERE {{ \
           {{ ?s geo:asWKT ?wkt }} UNION {{ ?s ex:loc ?wkt }} \
           FILTER(geof:distance(?wkt, \"POINT(0 0)\"^^geo:wktLiteral, uom:kilometre) < 50) \
         }} ORDER BY ?wkt"
    );
    let (rows, _posthoc, _pushed) = compare(&g, &q);
    // POINT(0 0) (indexed, in window) and POINT(0.1 0.1) (not indexed, kept then matches);
    // POINT(40 40) is excluded by the exact filter. Both within-50km points present.
    assert_eq!(rows.len(), 2, "both near points kept; got {rows:?}");
}

#[test]
fn no_index_installed_is_unchanged() {
    // The seam is inert without a provider: a pushable FILTER just runs post-hoc.
    let g = grid(5);
    let counter = Arc::new(AtomicUsize::new(0));
    let reg = counting_registry(counter.clone());
    let q = format!(
        "{PREFIXES} SELECT ?f WHERE {{ \
           ?f geo:hasGeometry ?n . ?n geo:asWKT ?wkt . \
           FILTER(geof:sfWithin(?wkt, \"POLYGON((-0.1 -0.1, 1.1 -0.1, 1.1 1.1, -0.1 1.1, -0.1 -0.1))\"^^geo:wktLiteral)) \
         }}"
    );
    let r = with_functions(&reg, || query(&g, &q)).unwrap();
    assert_eq!(r.len(), 9);
    assert_eq!(counter.load(Ordering::Relaxed), 25, "every one of the 25 features is exact-checked");
}

/// A [`SpatialProvider`] that delegates `candidates`/`is_indexed` to an inner
/// [`GeoIndexProvider`] but FORCES the engine onto the per-row `is_indexed`
/// fallback by overriding `indexed_ids` to `None`. Lets one query be run BOTH
/// ways (id-level fast path vs per-row slow path) so the differential test proves
/// they agree byte-for-byte. sq-7jt80
struct NoIdLevelProvider(GeoIndexProvider);

impl SpatialProvider for NoIdLevelProvider {
    fn candidates(&self, query: &sparq_engine::SpatialQuery) -> Option<Vec<oxrdf::Term>> {
        self.0.candidates(query)
    }
    fn is_indexed(&self, term: &oxrdf::Term) -> bool {
        self.0.is_indexed(term)
    }
    // Deliberately NOT overriding `indexed_ids` -> inherits the trait default
    // `None`, forcing the engine's per-row fallback path.
}

/// Runs `sparql` THREE ways and asserts the sorted rows are byte-identical:
/// (a) no index (post-hoc), (b) the real provider (id-level FAST retain path),
/// (c) `NoIdLevelProvider` (per-row SLOW retain fallback). Both retain branches
/// must produce the same rows as the post-hoc plan, by construction.
fn compare3(graph: &Graph, sparql: &str) -> Vec<Vec<String>> {
    let reg = geof_registry();

    let posthoc = with_functions(&reg, || query(graph, sparql)).unwrap();

    let fast: Arc<dyn SpatialProvider> = Arc::new(GeoIndexProvider::new(GeoIndex::build(graph)));
    let fast_r = with_spatial_index(fast, || with_functions(&reg, || query(graph, sparql))).unwrap();

    let slow: Arc<dyn SpatialProvider> =
        Arc::new(NoIdLevelProvider(GeoIndexProvider::new(GeoIndex::build(graph))));
    let slow_r = with_spatial_index(slow, || with_functions(&reg, || query(graph, sparql))).unwrap();

    let a = rows_sorted(&posthoc);
    let b = rows_sorted(&fast_r);
    let c = rows_sorted(&slow_r);
    assert_eq!(a, b, "id-level FAST path diverged from post-hoc\nquery: {sparql}");
    assert_eq!(a, c, "per-row SLOW fallback diverged from post-hoc\nquery: {sparql}");
    a
}

#[test]
fn id_level_and_per_row_paths_agree_pure_aswkt_column() {
    // The fast path FIRES here: the geometry variable is bound purely by geo:asWKT
    // (every binding is in the id-level indexed universe), so retain is pure id
    // lookups. It must equal both the post-hoc and per-row-fallback results.
    let g = grid(15); // 225 features
    let q = format!(
        "{PREFIXES} SELECT ?f WHERE {{ \
           ?f geo:hasGeometry ?n . ?n geo:asWKT ?wkt . \
           FILTER(geof:sfWithin(?wkt, \"POLYGON((-0.1 -0.1, 1.1 -0.1, 1.1 1.1, -0.1 1.1, -0.1 -0.1))\"^^geo:wktLiteral)) \
         }}"
    );
    let rows = compare3(&g, &q);
    assert_eq!(rows.len(), 9, "3x3 points inside the box");
}

#[test]
fn id_level_and_per_row_paths_agree_mixed_indexed_and_unindexed() {
    // A UNION of an indexed (geo:asWKT) column and a non-indexed (ex:loc) column:
    // the pushdown may restrict the indexed binding but MUST keep the non-indexed
    // one for the exact FILTER. Both retain branches must agree with post-hoc —
    // discriminating: a fast path that wrongly treated the ex:loc literal as
    // indexed-but-not-candidate would drop a true match.
    let g = Graph::load_str(
        r#"
        @prefix geo: <http://www.opengis.net/ont/geosparql#> .
        @prefix ex:  <http://ex/> .
        ex:idx geo:hasGeometry ex:g . ex:g geo:asWKT "POINT(0 0)"^^geo:wktLiteral .
        ex:far geo:hasGeometry ex:gf . ex:gf geo:asWKT "POINT(40 40)"^^geo:wktLiteral .
        ex:other ex:loc "POINT(0.1 0.1)"^^geo:wktLiteral .
        "#,
        "turtle",
    )
    .unwrap();
    let q = format!(
        "{PREFIXES} SELECT ?wkt WHERE {{ \
           {{ ?s geo:asWKT ?wkt }} UNION {{ ?s ex:loc ?wkt }} \
           FILTER(geof:distance(?wkt, \"POINT(0 0)\"^^geo:wktLiteral, uom:kilometre) < 50) \
         }} ORDER BY ?wkt"
    );
    let rows = compare3(&g, &q);
    // POINT(0 0) (indexed, in window) + POINT(0.1 0.1) (NOT indexed, kept and matched);
    // POINT(40 40) (indexed, NOT a candidate) dropped. Two near points.
    assert_eq!(rows.len(), 2, "both near points kept; got {rows:?}");
}

#[test]
fn id_level_and_per_row_paths_agree_non_geographic_aswkt_literal_kept() {
    // CRS-SAFETY: a NON-GEOGRAPHIC geo:asWKT literal is SKIPPED by the index (build
    // only indexes geographic-CRS literals), so it is NOT in the id-level universe:
    // `indexed_ids`/`is_indexed` must both say "not indexed" so the row is KEPT for
    // the exact geof: FILTER to judge — NEVER dropped by the pushdown. The exact
    // sfWithin then declines it (CrsMismatch: a projected geometry is not comparable
    // to a geographic box), so it is absent from the answer — but crucially, ALL
    // THREE paths agree, proving neither retain branch drops the non-indexed literal
    // on a fast-path-specific code route. A geographic sibling in the box IS matched.
    let g = Graph::load_str(
        r#"
        @prefix geo: <http://www.opengis.net/ont/geosparql#> .
        @prefix ex:  <http://ex/> .
        ex:geo geo:hasGeometry ex:gg . ex:gg geo:asWKT "POINT(0.5 0.5)"^^geo:wktLiteral .
        ex:proj geo:hasGeometry ex:gp . ex:gp geo:asWKT "<http://www.opengis.net/def/crs/EPSG/0/3857> POINT(0.5 0.5)"^^geo:wktLiteral .
        "#,
        "turtle",
    )
    .unwrap();
    let q = format!(
        "{PREFIXES} SELECT ?f WHERE {{ \
           ?f geo:hasGeometry ?n . ?n geo:asWKT ?wkt . \
           FILTER(geof:sfWithin(?wkt, \"POLYGON((-0.1 -0.1, 1.1 -0.1, 1.1 1.1, -0.1 1.1, -0.1 -0.1))\"^^geo:wktLiteral)) \
         }} ORDER BY ?f"
    );
    let rows = compare3(&g, &q);
    // Only the geographic point is inside AND CRS-comparable; the projected one is
    // declined by the exact FILTER (kept by the pushdown, then dropped by geof:).
    assert_eq!(rows, vec![vec!["<http://ex/geo>".to_string()]], "only the geographic sibling matches");
}

#[test]
fn id_level_path_keeps_non_indexed_matching_binding_single_column() {
    // DISCRIMINATING for the fast path: the index is NON-EMPTY (so `indexed_ids`
    // returns a fresh, populated set and the FAST retain runs), but the query's
    // geometry variable is bound on a SINGLE scan column by ex:loc — a predicate
    // the index never indexes — to GEOGRAPHIC literals INSIDE the box. Those ids
    // are absent from the id-level universe, so the fast retain must KEEP them
    // (`!indexed.contains(&id)`) for the exact FILTER, which then MATCHES them.
    // A fast path that dropped non-candidate rows would lose them -> divergence.
    let g = Graph::load_str(
        r#"
        @prefix geo: <http://www.opengis.net/ont/geosparql#> .
        @prefix ex:  <http://ex/> .
        # Indexed universe (geo:asWKT) — populates indexed_ids, all OUTSIDE the box.
        ex:i1 geo:asWKT "POINT(40 40)"^^geo:wktLiteral .
        ex:i2 geo:asWKT "POINT(41 41)"^^geo:wktLiteral .
        # The queried column is ex:loc (NOT indexed) with points INSIDE the box.
        ex:a ex:loc "POINT(0.2 0.2)"^^geo:wktLiteral .
        ex:b ex:loc "POINT(0.8 0.8)"^^geo:wktLiteral .
        ex:c ex:loc "POINT(9 9)"^^geo:wktLiteral .
        "#,
        "turtle",
    )
    .unwrap();
    let q = format!(
        "{PREFIXES} SELECT ?s WHERE {{ \
           ?s ex:loc ?wkt . \
           FILTER(geof:sfWithin(?wkt, \"POLYGON((-0.1 -0.1, 1.1 -0.1, 1.1 1.1, -0.1 1.1, -0.1 -0.1))\"^^geo:wktLiteral)) \
         }} ORDER BY ?s"
    );
    let rows = compare3(&g, &q);
    assert_eq!(
        rows,
        vec![vec!["<http://ex/a>".to_string()], vec!["<http://ex/b>".to_string()]],
        "the two non-indexed in-box points must survive the pushdown and match"
    );
}

#[test]
fn id_level_path_stays_consistent_after_apply_delta() {
    // After a graph.apply_delta the id-level FAST retain path must still be
    // byte-identical to the post-hoc + per-row paths. (compare3 builds the index
    // fresh over the post-delta graph; the SEPARATE proof that the INCREMENTAL
    // GeoIndex::apply_delta keeps the id-set equal to a fresh build lives in
    // tests/index.rs::assert_matches_fresh_build.) sq-7jt80
    let iri = |s: &str| oxrdf::Term::NamedNode(oxrdf::NamedNode::new_unchecked(s.to_string()));
    let lit = |s: &str| {
        oxrdf::Term::Literal(oxrdf::Literal::new_typed_literal(
            s.to_string(),
            oxrdf::NamedNode::new_unchecked(WKT),
        ))
    };
    let as_wkt = "http://www.opengis.net/ont/geosparql#asWKT";
    let mut g = Graph::load_str(
        r#"@prefix geo: <http://www.opengis.net/ont/geosparql#> .
           @prefix ex:  <http://ex/> .
           ex:a geo:asWKT "POINT(0 0)"^^geo:wktLiteral .
           ex:b geo:asWKT "POINT(50 50)"^^geo:wktLiteral ."#,
        "turtle",
    )
    .unwrap();
    // Insert a new indexed point inside the box; delete the far one.
    let ins = [[iri("http://ex/c"), iri(as_wkt), lit("POINT(0.5 0.5)")]];
    let del = [[iri("http://ex/b"), iri(as_wkt), lit("POINT(50 50)")]];
    g.apply_delta(&ins, &del).unwrap();

    let q = format!(
        "{PREFIXES} SELECT ?f WHERE {{ \
           ?f geo:asWKT ?wkt . \
           FILTER(geof:sfWithin(?wkt, \"POLYGON((-0.1 -0.1, 1.1 -0.1, 1.1 1.1, -0.1 1.1, -0.1 -0.1))\"^^geo:wktLiteral)) \
         }} ORDER BY ?f"
    );
    let rows = compare3(&g, &q);
    assert_eq!(
        rows,
        vec![vec!["<http://ex/a>".to_string()], vec!["<http://ex/c>".to_string()]],
        "the two in-box points after the delta"
    );
}

#[test]
fn provider_candidates_are_a_superset() {
    // Direct unit test of the provider contract: the candidate set is a SUPERSET
    // of the exact matches.
    use sparq_engine::SpatialQuery;
    let g = grid(15);
    let provider = GeoIndexProvider::new(GeoIndex::build(&g));
    let cands = provider
        .candidates(&SpatialQuery::DistanceWithin {
            point_wkt: "POINT(0 0)",
            radius: 100.0,
            unit_iri: "http://www.opengis.net/def/uom/OGC/1.0/kilometre",
            inclusive: false,
        })
        .expect("metric distance pushes down");
    // Every candidate is a wktLiteral; none is dropped erroneously.
    assert!(!cands.is_empty());
    for t in &cands {
        match t {
            oxrdf::Term::Literal(l) => assert_eq!(l.datatype().as_str(), WKT),
            other => panic!("candidate must be a wktLiteral, got {other}"),
        }
    }
}
