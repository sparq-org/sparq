//! (sq-lk3aw.4) EXACT-candidate spatial pushdown (`spatial-exact-pushdown`
//! feature): the soundness proof that skipping the residual `geof:` FILTER for
//! provider-CERTIFIED rows never changes a query answer.
//!
//! The engine is geometry-free, so these tests use a MOCK provider/registry pair that
//! agree by construction on ONE fixed region — the DIAMOND `|x-5| + |y-5| <= 5`
//! (whose axis-aligned bounding box is `[0,10]²`, so "in the AABB but outside the
//! region" rows exist) — exactly as sparq-geo's real `GeoIndexProvider` + `geof`
//! registry agree on real geometry (pinned by its `topology_index` tests).
//!
//! DIFFERENTIAL CONTRACT: the provider-installed run must return the IDENTICAL
//! result multiset as the post-hoc run (no provider — the same evaluation the
//! feature-OFF build performs: with the feature off, or with no exact certification,
//! every row goes through the residual FILTER). The corpus deliberately mixes
//! (a) indexed bindings inside the region, (b) indexed bindings in the region's AABB
//! but OUTSIDE it, (c) NOT-indexed bindings both inside and outside the region.
//!
//! NON-VACUITY (mutation oracle): `ex:u_out` is a NOT-indexed binding that does NOT
//! satisfy the predicate but survives the pushdown (the index has no opinion on it).
//! A mutant that skips the residual FILTER for ALL rows (`Partial` treated as
//! `AllCertified`) admits `ex:u_out` into the answer and turns the differential red —
//! verified by hand against `exec.rs` before landing.

#![cfg(feature = "spatial-exact-pushdown")]

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use oxrdf::{Literal, NamedNode, Term};
use rustc_hash::FxHashSet;
use sparq_core::dict::Id;
use sparq_core::Graph;
use sparq_engine::{
    query, with_functions, with_spatial_index, FunctionRegistry, QueryResult, SpatialExactQuery,
    SpatialProvider, SpatialQuery,
};

const WKT: &str = "http://www.opengis.net/ont/geosparql#wktLiteral";
const SF_WITHIN: &str = "http://www.opengis.net/def/function/geosparql/sfWithin";
const SF_CONTAINS: &str = "http://www.opengis.net/def/function/geosparql/sfContains";
/// The one constant region both mock halves implement: a lexical form the ENGINE
/// treats as opaque (it only checks the wktLiteral datatype), decoded by the mock
/// provider and the mock registry as the diamond `|x-5| + |y-5| <= 5`.
const REGION: &str = "DIAMOND(5 5 5)";

fn parse_point(s: &str) -> Option<(f64, f64)> {
    let (x, y) = s.strip_prefix("POINT(")?.strip_suffix(')')?.split_once(' ')?;
    Some((x.parse().ok()?, y.parse().ok()?))
}

fn in_diamond(x: f64, y: f64) -> bool {
    (x - 5.0).abs() + (y - 5.0).abs() <= 5.0
}

/// The diamond's axis-aligned bounding box — the mock index's window superset.
fn in_aabb(x: f64, y: f64) -> bool {
    (0.0..=10.0).contains(&x) && (0.0..=10.0).contains(&y)
}

fn wkt_term(v: &str) -> Term {
    Term::Literal(Literal::new_typed_literal(v, NamedNode::new_unchecked(WKT)))
}

/// The residual-side mock: `geof:sfWithin(?g, region)` / `geof:sfContains(region, ?g)`
/// judged by the SAME diamond, counting every invocation — the deterministic oracle
/// for "how many residual exact checks ran".
fn diamond_registry(counter: Arc<AtomicUsize>) -> FunctionRegistry {
    fn judge(geom: &Term) -> Result<Term, String> {
        let Term::Literal(l) = geom else { return Err("geometry must be a literal".into()) };
        let (x, y) = parse_point(l.value()).ok_or_else(|| "unparsable point".to_string())?;
        Ok(Literal::from(in_diamond(x, y)).into())
    }
    let mut reg = FunctionRegistry::new();
    let c = counter.clone();
    // sfWithin(?g, region): the geometry is the FIRST argument.
    reg.register(SF_WITHIN, move |args: &[Term]| {
        c.fetch_add(1, Ordering::Relaxed);
        judge(&args[0])
    });
    // sfContains(region, ?g): the geometry is the SECOND argument.
    reg.register(SF_CONTAINS, move |args: &[Term]| {
        counter.fetch_add(1, Ordering::Relaxed);
        judge(&args[1])
    });
    reg
}

/// The index-side mock over an explicit indexed universe of points.
struct DiamondProvider {
    /// The indexed universe: (literal term, x, y). `is_indexed` is membership here.
    points: Vec<(Term, f64, f64)>,
    /// `false` -> `candidates_exact` declines (`None`), forcing the superset path.
    serve_exact: bool,
    exact_calls: Arc<AtomicUsize>,
    superset_calls: Arc<AtomicUsize>,
    /// `Some` -> serve the id-level universe (engine's FAST retain branch) for the
    /// captured dict; `None` -> per-row `is_indexed` fallback (SLOW branch).
    id_universe: Option<(usize, Arc<FxHashSet<Id>>)>,
}

impl DiamondProvider {
    fn new(graph: &Graph, indexed: &[(&str, f64, f64)], serve_exact: bool, id_level: bool) -> Self {
        let points: Vec<(Term, f64, f64)> =
            indexed.iter().map(|(v, x, y)| (wkt_term(v), *x, *y)).collect();
        let id_universe = id_level.then(|| {
            let ids: FxHashSet<Id> = points.iter().filter_map(|(t, _, _)| graph.id_of(t)).collect();
            (std::ptr::from_ref(graph.dict()) as usize, Arc::new(ids))
        });
        Self {
            points,
            serve_exact,
            exact_calls: Arc::new(AtomicUsize::new(0)),
            superset_calls: Arc::new(AtomicUsize::new(0)),
            id_universe,
        }
    }
}

impl SpatialProvider for DiamondProvider {
    fn candidates(&self, q: &SpatialQuery) -> Option<Vec<Term>> {
        match q {
            SpatialQuery::BboxIntersects { arg_wkt } => {
                assert_eq!(*arg_wkt, REGION, "the engine forwards the constant's lexical form");
                self.superset_calls.fetch_add(1, Ordering::Relaxed);
                // The window SUPERSET: everything indexed inside the diamond's AABB.
                Some(
                    self.points
                        .iter()
                        .filter(|(_, x, y)| in_aabb(*x, *y))
                        .map(|(t, _, _)| t.clone())
                        .collect(),
                )
            }
            SpatialQuery::DistanceWithin { .. } => None,
        }
    }

    fn is_indexed(&self, term: &Term) -> bool {
        self.points.iter().any(|(t, _, _)| t == term)
    }

    fn indexed_ids(&self, dict_ptr: usize) -> Option<Arc<FxHashSet<Id>>> {
        match &self.id_universe {
            Some((p, ids)) if *p == dict_ptr => Some(Arc::clone(ids)),
            _ => None,
        }
    }

    fn candidates_exact(&self, q: &SpatialExactQuery) -> Option<Vec<Term>> {
        if !self.serve_exact {
            return None;
        }
        let SpatialExactQuery::WithinRegion { region_wkt } = q;
        assert_eq!(*region_wkt, REGION, "the engine forwards the constant's lexical form");
        self.exact_calls.fetch_add(1, Ordering::Relaxed);
        // EXACT certification: precisely the indexed points inside the diamond.
        Some(
            self.points
                .iter()
                .filter(|(_, x, y)| in_diamond(*x, *y))
                .map(|(t, _, _)| t.clone())
                .collect(),
        )
    }
}

/// The indexed universe used by the fixtures: two points in the diamond, one in the
/// AABB but OUTSIDE the diamond (the superset false positive the exact set removes),
/// one outside the AABB entirely.
const INDEXED: &[(&str, f64, f64)] = &[
    ("POINT(5 5)", 5.0, 5.0),
    ("POINT(3 5)", 3.0, 5.0),
    ("POINT(9 9)", 9.0, 9.0),
    ("POINT(50 50)", 50.0, 50.0),
];

/// Mixed corpus: the four INDEXED bindings plus two the index NEVER saw —
/// `ex:u_in` satisfies the predicate (must be kept via the residual FILTER),
/// `ex:u_out` does not (must be DROPPED by the residual FILTER — the row a
/// skip-for-all-rows mutant would wrongly admit).
fn mixed_graph() -> Graph {
    Graph::load_str(
        r#"@prefix ex: <http://ex/> .
           @prefix geo: <http://www.opengis.net/ont/geosparql#> .
           ex:i0 ex:geom "POINT(5 5)"^^geo:wktLiteral .
           ex:i1 ex:geom "POINT(3 5)"^^geo:wktLiteral .
           ex:i2 ex:geom "POINT(9 9)"^^geo:wktLiteral .
           ex:i3 ex:geom "POINT(50 50)"^^geo:wktLiteral .
           ex:u_in ex:geom "POINT(6 5)"^^geo:wktLiteral .
           ex:u_out ex:geom "POINT(0 0)"^^geo:wktLiteral ."#,
        "turtle",
    )
    .unwrap()
}

/// All-indexed corpus: every binding is in the provider's indexed universe, so an
/// exact certification covers every row and the residual FILTER can be skipped.
fn all_indexed_graph() -> Graph {
    Graph::load_str(
        r#"@prefix ex: <http://ex/> .
           @prefix geo: <http://www.opengis.net/ont/geosparql#> .
           ex:i0 ex:geom "POINT(5 5)"^^geo:wktLiteral .
           ex:i1 ex:geom "POINT(3 5)"^^geo:wktLiteral .
           ex:i2 ex:geom "POINT(9 9)"^^geo:wktLiteral .
           ex:i3 ex:geom "POINT(50 50)"^^geo:wktLiteral ."#,
        "turtle",
    )
    .unwrap()
}

fn q_within() -> String {
    format!(
        "PREFIX geof: <http://www.opengis.net/def/function/geosparql/> \
         PREFIX geo: <http://www.opengis.net/ont/geosparql#> \
         SELECT ?s WHERE {{ ?s <http://ex/geom> ?g . \
           FILTER(geof:sfWithin(?g, \"{REGION}\"^^geo:wktLiteral)) }}"
    )
}

fn q_contains() -> String {
    format!(
        "PREFIX geof: <http://www.opengis.net/def/function/geosparql/> \
         PREFIX geo: <http://www.opengis.net/ont/geosparql#> \
         SELECT ?s WHERE {{ ?s <http://ex/geom> ?g . \
           FILTER(geof:sfContains(\"{REGION}\"^^geo:wktLiteral, ?g)) }}"
    )
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

/// Runs `sparql` post-hoc (no provider — the same per-row evaluation the feature-OFF
/// build performs) and with `provider` installed, asserting IDENTICAL sorted rows,
/// and returns `(rows, residual_checks_posthoc, residual_checks_pushed)`.
fn differential(
    graph: &Graph,
    sparql: &str,
    provider: Arc<dyn SpatialProvider>,
) -> (Vec<Vec<String>>, usize, usize) {
    let counter = Arc::new(AtomicUsize::new(0));
    let reg = diamond_registry(counter.clone());

    let posthoc = with_functions(&reg, || query(graph, sparql)).unwrap();
    let checks_posthoc = counter.swap(0, Ordering::Relaxed);

    let pushed = with_spatial_index(provider, || with_functions(&reg, || query(graph, sparql))).unwrap();
    let checks_pushed = counter.load(Ordering::Relaxed);

    let a = rows_sorted(&posthoc);
    let b = rows_sorted(&pushed);
    assert_eq!(
        a, b,
        "exact pushdown changed the RESULT (must be identical to post-hoc)\nquery: {sparql}"
    );
    (a, checks_posthoc, checks_pushed)
}

fn subjects(rows: &[Vec<String>]) -> Vec<String> {
    rows.iter().map(|r| r[0].clone()).collect()
}

/// Acceptance (1)+(4): mixed indexed/non-indexed corpus, BOTH predicate orientations,
/// BOTH engine retain branches (id-level fast path and per-row fallback) — the
/// provider-certified run returns the identical multiset as post-hoc; the AABB
/// false positive is excluded; the not-indexed rows are judged by the residual
/// FILTER (present iff they satisfy it — never dropped unjudged, never admitted).
#[test]
fn mixed_corpus_differential_both_orientations_and_retain_branches() {
    let g = mixed_graph();
    for id_level in [false, true] {
        for q in [q_within(), q_contains()] {
            let p = Arc::new(DiamondProvider::new(&g, INDEXED, true, id_level));
            let exact_calls = p.exact_calls.clone();
            let (rows, checks_posthoc, checks_pushed) = differential(&g, &q, p);
            assert_eq!(
                subjects(&rows),
                vec!["<http://ex/i0>", "<http://ex/i1>", "<http://ex/u_in>"],
                "diamond members only: indexed i0/i1 (certified), not-indexed u_in \
                 (kept by the residual FILTER); i2 is the AABB false positive, u_out \
                 the not-indexed non-match (id_level={id_level})\nquery: {q}"
            );
            assert!(exact_calls.load(Ordering::Relaxed) >= 1, "the exact certification was consulted");
            assert!(
                checks_pushed >= 1,
                "not-indexed bindings survive, so the residual FILTER MUST still run (Partial)"
            );
            assert!(
                checks_pushed < checks_posthoc,
                "the exact restriction must shrink the residual FILTER's input \
                 ({checks_pushed} vs {checks_posthoc})"
            );
        }
    }
}

/// Acceptance (2): on an all-indexed corpus the certification covers every surviving
/// row, so the residual exact check runs ZERO times — the refinement is eliminated,
/// not merely reordered — while the answer stays identical to post-hoc.
#[test]
fn all_indexed_corpus_skips_the_residual_filter_entirely() {
    let g = all_indexed_graph();
    for id_level in [false, true] {
        for q in [q_within(), q_contains()] {
            let p = Arc::new(DiamondProvider::new(&g, INDEXED, true, id_level));
            let exact_calls = p.exact_calls.clone();
            let (rows, checks_posthoc, checks_pushed) = differential(&g, &q, p);
            assert_eq!(subjects(&rows), vec!["<http://ex/i0>", "<http://ex/i1>"]);
            assert_eq!(
                checks_pushed, 0,
                "every surviving row is certified: the residual `geof:` check must run \
                 ZERO times (id_level={id_level})\nquery: {q}"
            );
            assert_eq!(checks_posthoc, 4, "post-hoc exact-checks every binding");
            assert_eq!(exact_calls.load(Ordering::Relaxed), 1);
        }
    }
}

/// Acceptance (3): `geof:sfContains(REGION, ?g)` — previously never recognised — now
/// pushes down: the provider IS consulted for that operand order.
#[test]
fn contains_constant_first_orientation_now_pushes_down() {
    let g = all_indexed_graph();
    let p = Arc::new(DiamondProvider::new(&g, INDEXED, true, false));
    let exact_calls = p.exact_calls.clone();
    let reg = diamond_registry(Arc::new(AtomicUsize::new(0)));
    let r = with_spatial_index(p, || with_functions(&reg, || query(&g, &q_contains()))).unwrap();
    assert_eq!(r.len(), 2);
    assert_eq!(
        exact_calls.load(Ordering::Relaxed),
        1,
        "sfContains(CONST, ?g) must be recognised and consult the index"
    );
}

/// A provider that declines exact certification (`candidates_exact -> None`) falls
/// back to the superset + residual-FILTER path: identical answers, superset consulted,
/// residual FILTER still runs (even on the all-indexed corpus).
#[test]
fn declining_exact_certification_falls_back_to_the_superset_path() {
    let g = all_indexed_graph();
    for q in [q_within(), q_contains()] {
        let p = Arc::new(DiamondProvider::new(&g, INDEXED, false, false));
        let superset_calls = p.superset_calls.clone();
        let (rows, _, checks_pushed) = differential(&g, &q, p);
        assert_eq!(subjects(&rows), vec!["<http://ex/i0>", "<http://ex/i1>"]);
        assert!(superset_calls.load(Ordering::Relaxed) >= 1, "superset path consulted");
        assert!(
            checks_pushed >= 1,
            "without exact certification the residual FILTER must run\nquery: {q}"
        );
    }
}

/// A pre-existing provider that never heard of `candidates_exact` (does NOT override
/// the default) keeps working unchanged: the ADDITIVE default declines, the superset
/// path runs, answers identical — the trait extension is backwards-compatible.
#[test]
fn provider_without_candidates_exact_inherits_the_declining_default() {
    struct SupersetOnly(DiamondProvider);
    impl SpatialProvider for SupersetOnly {
        fn candidates(&self, q: &SpatialQuery) -> Option<Vec<Term>> {
            self.0.candidates(q)
        }
        fn is_indexed(&self, term: &Term) -> bool {
            self.0.is_indexed(term)
        }
        // Deliberately NOT overriding `candidates_exact` -> the default `None`.
    }
    let g = mixed_graph();
    let inner = DiamondProvider::new(&g, INDEXED, true, false);
    let superset_calls = inner.superset_calls.clone();
    let (rows, _, checks_pushed) = differential(&g, &q_within(), Arc::new(SupersetOnly(inner)));
    assert_eq!(
        subjects(&rows),
        vec!["<http://ex/i0>", "<http://ex/i1>", "<http://ex/u_in>"]
    );
    assert!(superset_calls.load(Ordering::Relaxed) >= 1);
    assert!(checks_pushed >= 1, "no certification -> the residual FILTER always runs");
}

/// The certified-exact restriction also EXCLUDES indexed rows outside the certified
/// set before the residual runs: on the mixed corpus the pushed residual judges only
/// the survivors (2 not-indexed rows), never the excluded indexed ones.
#[test]
fn partial_run_residual_input_is_exactly_the_survivors() {
    let g = mixed_graph();
    let p = Arc::new(DiamondProvider::new(&g, INDEXED, true, false));
    let (_, checks_posthoc, checks_pushed) = differential(&g, &q_within(), p);
    assert_eq!(checks_posthoc, 6);
    // Survivors of the exact restriction: i0, i1 (certified) + u_in, u_out
    // (not indexed) = 4 rows through the residual FILTER.
    assert_eq!(checks_pushed, 4);
}
