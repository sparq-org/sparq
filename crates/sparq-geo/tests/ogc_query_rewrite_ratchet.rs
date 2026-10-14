#![cfg(feature = "geosparql_rewrite")]
//! sq-wf9qg — OGC GeoSPARQL **query-rewrite extension** conformance
//! ratchet (the `/conf/query-rewrite-extension` conformance class).
//!
//! The sibling ratchet [`ogc_compliance_ratchet`](../ogc_compliance_ratchet.rs)
//! (sq-9h1r) scores the `geof:` topology FILTER FUNCTIONS at the lexical level.
//! This ratchet scores the OTHER half of the topology surface the spec defines —
//! the topology RELATION PROPERTIES (`?f geo:sfWithin ?g` as a graph-pattern
//! predicate) — driven END-TO-END through the OPT-IN query-rewrite extension
//! ([`sparq_geo::geosparql_rewrite`], gated behind the `geosparql_rewrite`
//! feature, sq-wf9qg): parse → rewrite the property pattern into a
//! default-geometry resolution + matching `geof:` FILTER → execute on the REAL
//! engine → compare the bound feature set.
//!
//! Each case asserts THREE invariants, so a pass is load-bearing, not cosmetic:
//!
//! 1. **result-equivalence** — the rewritten property form binds EXACTLY the
//!    features the authoritative lexical relation (`geof::lex::*`, the same
//!    DE-9IM evaluation the FILTER functions use) says it should. The expected
//!    set is DERIVED from that oracle, never hand-typed, so the ratchet can never
//!    drift from the relation semantics.
//! 2. **answer-safety / no asserted triple** — the data contains NO asserted
//!    `geo:sf*/eh*/rcc8*` triple; every answer comes purely from geometry
//!    evaluation via the rewrite.
//! 3. **default-behaviour-unchanged (fail-closed)** — the SAME query through the
//!    STANDARD `sparq_engine` entry point (no rewrite) binds ZERO rows, proving
//!    the rewrite is a strict opt-in superset and default SPARQL semantics
//!    (a `geo:sfWithin` triple matches only asserted triples) are untouched.
//!
//! Like the topology ratchet it enforces a pass-count FLOOR
//! ([`OGC_QUERY_REWRITE_FLOOR`]) that may only RISE, plus zero regressions.
//!
//! ## OGC sub-conformance EXERCISED here
//!
//! The query-rewrite extension over all THREE topology-relation families: Simple
//! Features (`geo:sf*`), Egenhofer (`geo:eh*`), and RCC8 (`geo:rcc8*`), in both
//! the subject-variable (`?f geo:R ex:fixed`) and object-variable
//! (`ex:fixed geo:R ?f`) orientations, plus the two-variable full-relation form.
//!
//! ## NOT exercised (tracked, not asserted)
//!
//! * The official OGC CITE / TEAM-Engine suite (live-endpoint HTTP harness) — not
//!   vendored, as the topology ratchet header records.
//! * Property forms over geometries resolved through serializations OTHER than
//!   `geo:asWKT` (e.g. a `geo:asGML`-only default geometry) — the resolution path
//!   the rewrite emits is `(geo:hasDefaultGeometry|geo:hasGeometry)/geo:asWKT`;
//!   GML-only-default features are out of this corpus (bead `sq-cbe4t`).

use sparq_core::Graph;
use sparq_engine::{query, query_prepared, with_functions, PreparedQuery};
use sparq_geo::geof::lex;
use sparq_geo::{geof_registry, geosparql_rewrite, GeoError};

/// sq-wf9qg — pass-count FLOOR for the OGC GeoSPARQL query-rewrite
/// conformance cases. A ratchet: it may only RISE. This is the ACTUAL measured
/// pass count of [`CASES`] expanded across the query orientations the harness
/// drives; raising it is a deliberate act when cases are added. The remainder of
/// the `/conf/query-rewrite-extension` space (CITE suite, non-WKT default
/// serializations) is TRACKED in the module header, NOT asserted — an honest
/// floor, no skip-laundering.
///
/// MEASURED: the 24 cases in [`CASES`] (19 original + 5 added in sq-lk3aw.2:
/// `sfCrosses`, `ehEquals`, `ehCovers`, `rcc8eq`, `rcc8tppi`), each driven in
/// BOTH the subject-variable and object-variable orientation, all pass = 48.
/// Set to that actual count. sq-lk3aw.2
/// sq-z1xv8 — the VALUE now lives once in the zero-dependency
/// `sparq-conformance-floors` crate, which `sparq-conformance`'s central
/// `scoreboard::SUITES` reads too, so the enforced floor and the reported floor are
/// ONE `const` and cannot drift (replacing the old textual re-read of this file).
/// Raise it THERE; the measurement narrative stays here.
const OGC_QUERY_REWRITE_FLOOR: usize =
    sparq_conformance_floors::geo::OGC_QUERY_REWRITE_FLOOR;

/// A named WKT geometry body assigned to a feature in the test graph.
struct Feature {
    /// Feature local name (becomes `ex:<name>`).
    name: &'static str,
    /// The feature's default-geometry WKT body.
    wkt: &'static str,
}

/// The authoritative lexical relation for a `geo:`/`geof:` local name — the SAME
/// DE-9IM evaluation the FILTER functions use, used as the result-equivalence
/// oracle so expected feature sets are derived, never hand-typed.
type Rel = fn(&str, &str) -> Result<bool, GeoError>;

fn relation_for(local: &str) -> Rel {
    match local {
        "sfEquals" => lex::sf_equals as Rel,
        "sfDisjoint" => lex::sf_disjoint,
        "sfIntersects" => lex::sf_intersects,
        "sfTouches" => lex::sf_touches,
        "sfCrosses" => lex::sf_crosses,
        "sfWithin" => lex::sf_within,
        "sfContains" => lex::sf_contains,
        "sfOverlaps" => lex::sf_overlaps,
        "ehEquals" => lex::eh_equals,
        "ehDisjoint" => lex::eh_disjoint,
        "ehMeet" => lex::eh_meet,
        "ehOverlap" => lex::eh_overlap,
        "ehCovers" => lex::eh_covers,
        "ehCoveredBy" => lex::eh_covered_by,
        "ehInside" => lex::eh_inside,
        "ehContains" => lex::eh_contains,
        "rcc8eq" => lex::rcc8_eq,
        "rcc8dc" => lex::rcc8_dc,
        "rcc8ec" => lex::rcc8_ec,
        "rcc8po" => lex::rcc8_po,
        "rcc8tpp" => lex::rcc8_tpp,
        "rcc8tppi" => lex::rcc8_tppi,
        "rcc8ntpp" => lex::rcc8_ntpp,
        "rcc8ntppi" => lex::rcc8_ntppi,
        other => panic!("query-rewrite ratchet references an unknown relation: {other:?}"),
    }
}

/// One query-rewrite conformance case: over the `features`, the property form
/// `?f geo:<relation> ex:<fixed>` (and its inverse) is answered by the rewrite.
struct Case {
    /// The topology relation local name (shared between `geo:` and `geof:`).
    relation: &'static str,
    /// The feature corpus (default geometries).
    features: &'static [Feature],
    /// The fixed operand the variable side is related to.
    fixed: &'static str,
}

// Reusable polygon / line / point bodies, named so the corpus reads like the OGC
// matrices (mirrors the lexical ratchet's named geometries).
const BIG: &str = "POLYGON((0 0, 4 0, 4 4, 0 4, 0 0))";
const SMALL_INSIDE: &str = "POLYGON((1 1, 2 1, 2 2, 1 2, 1 1))";
const CORNER_TANGENT: &str = "POLYGON((0 0, 1 0, 1 1, 0 1, 0 0))";
const EDGE_ADJACENT: &str = "POLYGON((4 0, 6 0, 6 2, 4 2, 4 0))";
const FAR: &str = "POLYGON((9 9, 10 9, 10 10, 9 10, 9 9))";
const OVERLAP: &str = "POLYGON((3 3, 6 3, 6 6, 3 6, 3 3))";

/// A standing region corpus: a big region, a small region strictly inside it, a
/// corner-tangent region, an edge-adjacent region, a far region, and an
/// overlapping region. Pairwise relations cover the sf/eh/rcc8 truth tables.
const REGIONS: &[Feature] = &[
    Feature { name: "big", wkt: BIG },
    Feature { name: "small", wkt: SMALL_INSIDE },
    Feature { name: "tangent", wkt: CORNER_TANGENT },
    Feature { name: "adjacent", wkt: EDGE_ADJACENT },
    Feature { name: "far", wkt: FAR },
    Feature { name: "overlap", wkt: OVERLAP },
];

/// A mixed point/line/polygon corpus for the Simple-Features cases.
const MIXED: &[Feature] = &[
    Feature { name: "ptIn", wkt: "POINT(2 2)" },
    Feature { name: "ptBoundary", wkt: "POINT(0 2)" },
    Feature { name: "ptOut", wkt: "POINT(9 9)" },
    Feature { name: "lineCross", wkt: "LINESTRING(-1 2, 5 2)" },
    Feature { name: "lineInside", wkt: "LINESTRING(1 1, 3 3)" },
    Feature { name: "region", wkt: BIG },
];

/// The query-rewrite conformance corpus. Each case is driven in both
/// orientations; expected feature sets are derived from [`relation_for`].
const CASES: &[Case] = &[
    // ---- Simple Features over the mixed point/line/polygon corpus -----------
    Case { relation: "sfWithin", features: MIXED, fixed: "region" },
    Case { relation: "sfContains", features: MIXED, fixed: "region" },
    Case { relation: "sfIntersects", features: MIXED, fixed: "region" },
    Case { relation: "sfDisjoint", features: MIXED, fixed: "region" },
    Case { relation: "sfTouches", features: MIXED, fixed: "region" },
    // ---- Simple Features over the region corpus -----------------------------
    Case { relation: "sfOverlaps", features: REGIONS, fixed: "big" },
    Case { relation: "sfEquals", features: REGIONS, fixed: "big" },
    // sq-lk3aw.2 — sfCrosses: a line that enters and exits the
    // big polygon crosses it; the MIXED corpus contains `lineCross` which does
    // exactly that. sfCrosses is undefined for point-polygon (always false in
    // DE-9IM), so only `lineCross` matches in the subject orientation.
    Case { relation: "sfCrosses", features: MIXED, fixed: "region" },
    // ---- Egenhofer over the region corpus -----------------------------------
    Case { relation: "ehInside", features: REGIONS, fixed: "big" },
    Case { relation: "ehContains", features: REGIONS, fixed: "big" },
    Case { relation: "ehCoveredBy", features: REGIONS, fixed: "big" },
    Case { relation: "ehDisjoint", features: REGIONS, fixed: "far" },
    Case { relation: "ehMeet", features: REGIONS, fixed: "big" },
    Case { relation: "ehOverlap", features: REGIONS, fixed: "big" },
    // sq-lk3aw.2 — ehEquals: reflexive on `big`; the only pair
    // in REGIONS with the same WKT is (big, big). ehCovers(A, B): A covers B
    // (every point of B is in the closure of A's interior); `big` covers all
    // contained regions (`small`, `tangent`). Both fixed on "big".
    Case { relation: "ehEquals", features: REGIONS, fixed: "big" },
    Case { relation: "ehCovers", features: REGIONS, fixed: "big" },
    // ---- RCC8 over the region corpus ----------------------------------------
    Case { relation: "rcc8ntpp", features: REGIONS, fixed: "big" },
    Case { relation: "rcc8ntppi", features: REGIONS, fixed: "big" },
    Case { relation: "rcc8tpp", features: REGIONS, fixed: "big" },
    Case { relation: "rcc8ec", features: REGIONS, fixed: "big" },
    Case { relation: "rcc8dc", features: REGIONS, fixed: "far" },
    Case { relation: "rcc8po", features: REGIONS, fixed: "big" },
    // sq-lk3aw.2 — rcc8eq: reflexive on `big`. rcc8tppi(A, B)
    // means B is a Tangential Proper Part of A; with fixed="tangent" the
    // subject orientation finds `big` (which has `tangent` as a TPP sharing
    // the corner-vertex boundary).
    Case { relation: "rcc8eq", features: REGIONS, fixed: "big" },
    Case { relation: "rcc8tppi", features: REGIONS, fixed: "tangent" },
];

/// Build the Turtle for a feature corpus: each feature resolves through
/// `geo:hasDefaultGeometry`/`geo:asWKT`. CRUCIALLY there is NO asserted topology
/// triple anywhere, so every answer must come from the rewrite.
fn corpus_ttl(features: &[Feature]) -> String {
    let mut ttl = String::from(
        "@prefix geo: <http://www.opengis.net/ont/geosparql#> .\n\
         @prefix ex:  <http://example.org/> .\n",
    );
    for f in features {
        // Positional format args (CodeQL rust/unused-variable false-positive guard).
        ttl.push_str(&format!(
            "ex:{0} geo:hasDefaultGeometry ex:{0}Geom .\nex:{0}Geom geo:asWKT \"{1}\"^^geo:wktLiteral .\n",
            f.name, f.wkt
        ));
    }
    ttl
}

/// The feature names the lexical oracle says satisfy `relation(var, fixed)` when
/// the variable is the SUBJECT (`?f geo:R ex:fixed`).
fn expected_subject_side(case: &Case) -> Vec<String> {
    let rel = relation_for(case.relation);
    let fixed_wkt = case
        .features
        .iter()
        .find(|f| f.name == case.fixed)
        .expect("fixed operand is in the corpus")
        .wkt;
    let mut out: Vec<String> = case
        .features
        .iter()
        .filter(|f| rel(f.wkt, fixed_wkt).expect("oracle relation evaluates"))
        .map(|f| format!("<http://example.org/{}>", f.name))
        .collect();
    out.sort();
    out
}

/// The feature names the lexical oracle says satisfy `relation(fixed, var)` when
/// the variable is the OBJECT (`ex:fixed geo:R ?f`).
fn expected_object_side(case: &Case) -> Vec<String> {
    let rel = relation_for(case.relation);
    let fixed_wkt = case
        .features
        .iter()
        .find(|f| f.name == case.fixed)
        .expect("fixed operand is in the corpus")
        .wkt;
    let mut out: Vec<String> = case
        .features
        .iter()
        .filter(|f| rel(fixed_wkt, f.wkt).expect("oracle relation evaluates"))
        .map(|f| format!("<http://example.org/{}>", f.name))
        .collect();
    out.sort();
    out
}

/// Execute `sparql` through the REWRITE path and return the sorted first-column
/// feature IRIs.
fn run_rewrite(graph: &Graph, sparql: &str) -> Vec<String> {
    let prepared = geosparql_rewrite(sparql).expect("rewrite + parse");
    let reg = geof_registry();
    let r = with_functions(&reg, || query_prepared(graph, &prepared)).expect("execute");
    let mut out: Vec<String> = r
        .rows
        .iter()
        .filter_map(|row| row.first().and_then(|t| t.as_ref()).map(|t| t.to_string()))
        .collect();
    out.sort();
    out
}

/// Execute `sparql` through the STANDARD entry point (NO rewrite) and return the
/// row count. For a property form over data with no asserted topology triple this
/// MUST be 0 — the answer-safety invariant.
fn run_standard_count(graph: &Graph, sparql: &str) -> usize {
    let reg = geof_registry();
    with_functions(&reg, || query(graph, sparql))
        .expect("standard execute")
        .len()
}

const PREFIXES: &str = "PREFIX geo: <http://www.opengis.net/ont/geosparql#>\n\
                        PREFIX ex:  <http://example.org/>\n";

/// Run every case in both orientations; return (pass, fail, failures).
fn run_cases() -> (usize, usize, Vec<String>) {
    let mut pass = 0usize;
    let mut fail = 0usize;
    let mut failures = Vec::new();

    let mut check = |label: String, graph: &Graph, sparql: &str, expected: &[String]| {
        // Invariant 3: the STANDARD entry point binds zero rows (no asserted triple).
        let std_count = run_standard_count(graph, sparql);
        if std_count != 0 {
            fail += 1;
            failures.push(format!(
                "{label}: STANDARD entry point bound {std_count} rows for a property form \
                 over data with no asserted topology triple (default behaviour changed!)"
            ));
            return;
        }
        // Invariants 1 + 2: the rewrite binds EXACTLY the oracle-derived set.
        let got = run_rewrite(graph, sparql);
        if got == expected {
            pass += 1;
        } else {
            fail += 1;
            failures.push(format!(
                "{label}: rewrite bound {got:?}, oracle expected {expected:?}"
            ));
        }
    };

    for case in CASES {
        let graph = Graph::load_str(&corpus_ttl(case.features), "turtle").unwrap();

        // Subject-variable orientation: ?f geo:R ex:fixed.
        let q_subj = format!(
            "{PREFIXES} SELECT ?f WHERE {{ ?f geo:{} ex:{} }}",
            case.relation, case.fixed
        );
        check(
            format!("subject ?f geo:{} ex:{}", case.relation, case.fixed),
            &graph,
            &q_subj,
            &expected_subject_side(case),
        );

        // Object-variable orientation: ex:fixed geo:R ?f.
        let q_obj = format!(
            "{PREFIXES} SELECT ?f WHERE {{ ex:{} geo:{} ?f }}",
            case.fixed, case.relation
        );
        check(
            format!("object ex:{} geo:{} ?f", case.fixed, case.relation),
            &graph,
            &q_obj,
            &expected_object_side(case),
        );
    }

    (pass, fail, failures)
}

#[test]
fn ogc_query_rewrite_compliance_ratchet() {
    let (pass, fail, failures) = run_cases();

    println!("\nOGC GeoSPARQL query-rewrite compliance scoreboard");
    for why in &failures {
        println!("  FAIL {why}");
    }
    println!("pass {pass} / fail {fail} (floor {OGC_QUERY_REWRITE_FLOOR})");

    assert_eq!(
        fail, 0,
        "OGC query-rewrite compliance regressions:\n{}",
        failures.join("\n")
    );
    assert!(
        pass >= OGC_QUERY_REWRITE_FLOOR,
        "OGC query-rewrite pass count regressed: {pass} < floor {OGC_QUERY_REWRITE_FLOOR} — \
         if cases were removed deliberately, lower the floor in the same commit"
    );
}

/// The recorded floor must never exceed what the corpus can actually deliver —
/// catches a stale floor left above the case count after a deletion.
#[test]
fn floor_is_consistent_with_corpus() {
    let (pass, fail, _) = run_cases();
    assert_eq!(fail, 0);
    assert!(
        OGC_QUERY_REWRITE_FLOOR <= pass,
        "OGC_QUERY_REWRITE_FLOOR ({OGC_QUERY_REWRITE_FLOOR}) exceeds achievable passes ({pass}) — \
         the floor is unreachable; lower it or add cases"
    );
}

/// A composed case proving the rewrite interleaves with ordinary BGP triples in
/// the SAME graph pattern (the property form is not isolated): only the City
/// inside the region survives `?f a ex:City . ?f geo:sfWithin ex:region`.
#[test]
fn topology_property_composes_with_ordinary_bgp() {
    let ttl = r#"
@prefix geo: <http://www.opengis.net/ont/geosparql#> .
@prefix ex:  <http://example.org/> .
ex:region     geo:hasDefaultGeometry ex:rg .
ex:rg         geo:asWKT "POLYGON((0 0, 4 0, 4 4, 0 4, 0 0))"^^geo:wktLiteral .
ex:london     a ex:City ; geo:hasDefaultGeometry ex:lg .
ex:lg         geo:asWKT "POINT(2 2)"^^geo:wktLiteral .
ex:depot      a ex:Warehouse ; geo:hasDefaultGeometry ex:dg .
ex:dg         geo:asWKT "POINT(3 1)"^^geo:wktLiteral .
ex:paris      a ex:City ; geo:hasDefaultGeometry ex:pg .
ex:pg         geo:asWKT "POINT(9 9)"^^geo:wktLiteral .
"#;
    let graph = Graph::load_str(ttl, "turtle").unwrap();
    let got = run_rewrite(
        &graph,
        &format!("{PREFIXES} SELECT ?f WHERE {{ ?f a ex:City . ?f geo:sfWithin ex:region }}"),
    );
    assert_eq!(got, vec!["<http://example.org/london>".to_string()]);
}

/// The algebra entry point is a structural no-op for a topology-free query, so a
/// non-geo query is byte-for-byte unchanged by the rewrite — the proof that the
/// rewrite never perturbs ordinary SPARQL.
#[test]
fn rewrite_is_structural_noop_for_non_topology_query() {
    let q = "PREFIX ex: <http://example.org/> SELECT ?s ?o WHERE { ?s ex:p ?o FILTER(?o > 3) }";
    let before = PreparedQuery::parse(q).unwrap().into_query();
    let after = sparq_geo::rewrite_query(before.clone());
    assert_eq!(before.to_string(), after.to_string());
}

/// Semantics-preservation assertion for the TWO-VARIABLE form: the property-form
/// rewrite of `?f1 geo:sfWithin ?f2` yields EXACTLY the same result set as the
/// manually-written geometry-resolution FILTER form executed through the STANDARD
/// engine. This directly proves the rewrite is a semantics-preserving algebra
/// transformation, not just a syntax convenience. sq-lk3aw.2
#[test]
fn semantics_preserving_rewrite_matches_explicit_filter_form() {
    let graph = Graph::load_str(&corpus_ttl(MIXED), "turtle").unwrap();

    // Property-form query via the REWRITE path.
    let prop_sparql = format!("{PREFIXES} SELECT ?f1 ?f2 WHERE {{ ?f1 geo:sfWithin ?f2 }}");

    // Manually-written equivalent FILTER form — the expansion the rewrite
    // produces, written explicitly so we can run it through the STANDARD path.
    // Plain string literal (no format! — the earlier "positional format args" comment
    // was a copy/paste error and has been removed).
    let filter_sparql = "PREFIX geo:  <http://www.opengis.net/ont/geosparql#>
                         PREFIX geof: <http://www.opengis.net/def/function/geosparql/>
                         PREFIX ex:   <http://example.org/>
                         SELECT ?f1 ?f2 WHERE {
                             ?f1 (geo:hasDefaultGeometry|geo:hasGeometry)/geo:asWKT ?g1 .
                             ?f2 (geo:hasDefaultGeometry|geo:hasGeometry)/geo:asWKT ?g2 .
                             FILTER(geof:sfWithin(?g1, ?g2))
                         }";

    let reg = geof_registry();

    // Execute property form through the rewrite.
    let rewrite_prepared = geosparql_rewrite(&prop_sparql).expect("rewrite parse");
    let rewrite_result =
        with_functions(&reg, || query_prepared(&graph, &rewrite_prepared)).expect("rewrite execute");

    // Execute FILTER form through the STANDARD engine (geof: functions registered).
    let filter_result =
        with_functions(&reg, || query(&graph, filter_sparql)).expect("filter execute");

    // Assert row counts match first so a count divergence is caught before the
    // pair-by-pair comparison (otherwise a shorter result set could coincidentally
    // compare equal after sorting).
    assert_eq!(
        rewrite_result.rows.len(),
        filter_result.rows.len(),
        "rewrite and FILTER paths returned different row counts ({} vs {})",
        rewrite_result.rows.len(),
        filter_result.rows.len(),
    );

    // Collect both result sets using `map` (NOT `filter_map`) so that a row with an
    // unbound ?f1 or ?f2 binding causes the test to FAIL loudly rather than being
    // silently dropped — a silent drop could hide a divergence between the two paths.
    //
    let mut rewrite_pairs: Vec<(String, String)> = rewrite_result
        .rows
        .iter()
        .enumerate()
        .map(|(i, row)| {
            let f1 = row
                .first()
                .and_then(|t| t.as_ref())
                .unwrap_or_else(|| panic!("rewrite row {i}: ?f1 is unbound"))
                .to_string();
            let f2 = row
                .get(1)
                .and_then(|t| t.as_ref())
                .unwrap_or_else(|| panic!("rewrite row {i}: ?f2 is unbound"))
                .to_string();
            (f1, f2)
        })
        .collect();
    rewrite_pairs.sort();

    let mut filter_pairs: Vec<(String, String)> = filter_result
        .rows
        .iter()
        .enumerate()
        .map(|(i, row)| {
            let f1 = row
                .first()
                .and_then(|t| t.as_ref())
                .unwrap_or_else(|| panic!("filter row {i}: ?f1 is unbound"))
                .to_string();
            let f2 = row
                .get(1)
                .and_then(|t| t.as_ref())
                .unwrap_or_else(|| panic!("filter row {i}: ?f2 is unbound"))
                .to_string();
            (f1, f2)
        })
        .collect();
    filter_pairs.sort();

    // The two paths must agree exactly on every (f1, f2) pair.
    assert_eq!(
        rewrite_pairs, filter_pairs,
        "property-form rewrite yielded different results than the explicit FILTER form"
    );
    // Sanity: sfWithin is reflexive for equal geometries, so we always have
    // at least the (region, region) self-pair in MIXED.
    assert!(
        !rewrite_pairs.is_empty(),
        "sfWithin must bind at least one pair in the MIXED corpus"
    );
}
