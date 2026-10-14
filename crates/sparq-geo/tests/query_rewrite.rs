#![cfg(feature = "geosparql_rewrite")] // sq-wf9qg: rewrite is now opt-in
//! sq-9g58 — the GeoSPARQL query-rewrite extension end-to-end.
//!
//! A topology relation PROPERTY pattern (`?f geo:sfWithin ?region`, etc.) is
//! answered by resolving each feature's default geometry and applying the matching
//! `geof:` FILTER function — WITHOUT any asserted `geo:sfWithin` triple in the
//! data. This is the property-form rewrite that `tests/entailment.rs` previously
//! pinned as not-yet-supported; the engine support now exists on the dedicated
//! [`sparq_geo::geosparql_rewrite`] entry point.

use sparq_core::Graph;
use sparq_engine::{query_prepared, with_functions};
use sparq_geo::{geof_registry, geosparql_rewrite, rewrite_query};

const TTL: &str = r#"
@prefix geo: <http://www.opengis.net/ont/geosparql#> .
@prefix ex:  <http://example.org/> .

# A region polygon (inner London box) and two points: one inside, one outside.
ex:region     geo:hasDefaultGeometry ex:regionGeom .
ex:regionGeom geo:asWKT "POLYGON((-0.25 51.45, 0.05 51.45, 0.05 51.57, -0.25 51.57, -0.25 51.45))"^^geo:wktLiteral .

ex:london     geo:hasDefaultGeometry ex:londonGeom .
ex:londonGeom geo:asWKT "POINT(-0.13 51.51)"^^geo:wktLiteral .

ex:paris      geo:hasDefaultGeometry ex:parisGeom .
ex:parisGeom  geo:asWKT "POINT(2.3522 48.8566)"^^geo:wktLiteral .

# A feature resolved through geo:hasGeometry (the common dataset shape; the
# rewrite accepts hasGeometry as the resolution edge alongside hasDefaultGeometry).
ex:bigben     geo:hasGeometry ex:bigbenGeom .
ex:bigbenGeom geo:asWKT "POINT(-0.1246 51.5007)"^^geo:wktLiteral .

# Crucially: NO asserted ex:* geo:sfWithin ex:region triple anywhere.
"#;

fn run(graph: &Graph, sparql: &str) -> Vec<String> {
    let prepared = geosparql_rewrite(sparql).expect("rewrite + parse");
    let reg = geof_registry();
    let r = with_functions(&reg, || query_prepared(graph, &prepared)).expect("execute");
    // Project the single (first) projected column to comparable strings.
    let mut out: Vec<String> = r
        .rows
        .iter()
        .map(|row| {
            row.first()
                .and_then(|t| t.as_ref())
                .map(|t| t.to_string())
                .unwrap_or_default()
        })
        .collect();
    out.sort();
    out
}

#[test]
fn sf_within_property_form_resolves_default_geometry() {
    let graph = Graph::load_str(TTL, "turtle").unwrap();
    // ?f geo:sfWithin ex:region — London and Big Ben (both inside) match, and the
    // region itself (a polygon is sfWithin itself under DE-9IM — reflexive); Paris
    // (far outside) does not. No asserted topology triple anywhere.
    let got = run(
        &graph,
        "PREFIX geo: <http://www.opengis.net/ont/geosparql#>
         PREFIX ex:  <http://example.org/>
         SELECT ?f WHERE { ?f geo:sfWithin ex:region }",
    );
    assert_eq!(
        got,
        vec![
            "<http://example.org/bigben>".to_string(),
            "<http://example.org/london>".to_string(),
            "<http://example.org/region>".to_string(),
        ],
        "sfWithin property form must select the features inside (or equal to) the region"
    );
}

#[test]
fn sf_contains_is_the_inverse_direction() {
    let graph = Graph::load_str(TTL, "turtle").unwrap();
    // ex:region geo:sfContains ?f — the region contains London, Big Ben, and
    // itself (reflexive); not Paris.
    let got = run(
        &graph,
        "PREFIX geo: <http://www.opengis.net/ont/geosparql#>
         PREFIX ex:  <http://example.org/>
         SELECT ?f WHERE { ex:region geo:sfContains ?f }",
    );
    assert_eq!(
        got,
        vec![
            "<http://example.org/bigben>".to_string(),
            "<http://example.org/london>".to_string(),
            "<http://example.org/region>".to_string(),
        ]
    );
}

#[test]
fn two_variable_topology_pattern_is_a_full_relation() {
    let graph = Graph::load_str(TTL, "turtle").unwrap();
    // ?a geo:sfWithin ?b with both ends variable: every (a,b) whose geometry of a
    // is within geometry of b. Each geometry is within the region and within
    // itself (sfWithin is reflexive for equal geometries under DE-9IM), so this is
    // a 2-variable join — assert the known pair london-in-region appears.
    let prepared = geosparql_rewrite(
        "PREFIX geo: <http://www.opengis.net/ont/geosparql#>
         SELECT ?a ?b WHERE { ?a geo:sfWithin ?b }",
    )
    .unwrap();
    let reg = geof_registry();
    let r = with_functions(&reg, || query_prepared(&graph, &prepared)).unwrap();
    let pairs: Vec<(String, String)> = r
        .rows
        .iter()
        .map(|row| {
            let cell = |i: usize| {
                row.get(i)
                    .and_then(|t| t.as_ref())
                    .map(|t| t.to_string())
                    .unwrap_or_default()
            };
            (cell(0), cell(1))
        })
        .collect();
    assert!(
        pairs.contains(&(
            "<http://example.org/london>".to_string(),
            "<http://example.org/region>".to_string(),
        )),
        "london within region missing from {pairs:?}"
    );
    // Paris is within NO other feature's geometry except itself.
    assert!(
        pairs.contains(&(
            "<http://example.org/paris>".to_string(),
            "<http://example.org/paris>".to_string(),
        )),
        "paris within itself missing from {pairs:?}"
    );
    assert!(
        !pairs.contains(&(
            "<http://example.org/paris>".to_string(),
            "<http://example.org/region>".to_string(),
        )),
        "paris must NOT be within the london region"
    );
}

#[test]
fn topology_pattern_composes_with_ordinary_patterns() {
    let ttl = r#"
@prefix geo: <http://www.opengis.net/ont/geosparql#> .
@prefix ex:  <http://example.org/> .
ex:region     geo:hasDefaultGeometry ex:regionGeom .
ex:regionGeom geo:asWKT "POLYGON((-0.25 51.45, 0.05 51.45, 0.05 51.57, -0.25 51.57, -0.25 51.45))"^^geo:wktLiteral .
ex:london     a ex:City ; geo:hasDefaultGeometry ex:lg .
ex:lg         geo:asWKT "POINT(-0.13 51.51)"^^geo:wktLiteral .
ex:depot      a ex:Warehouse ; geo:hasDefaultGeometry ex:dg .
ex:dg         geo:asWKT "POINT(-0.12 51.50)"^^geo:wktLiteral .
"#;
    let graph = Graph::load_str(ttl, "turtle").unwrap();
    // Combine an ordinary BGP triple (?f a ex:City) with a topology triple in the
    // SAME basic graph pattern: only the City inside the region survives.
    let got = run(
        &graph,
        "PREFIX geo: <http://www.opengis.net/ont/geosparql#>
         PREFIX ex:  <http://example.org/>
         SELECT ?f WHERE { ?f a ex:City . ?f geo:sfWithin ex:region }",
    );
    assert_eq!(got, vec!["<http://example.org/london>".to_string()]);
}

#[test]
fn non_topology_predicate_is_left_as_an_asserted_triple() {
    let ttl = r#"
@prefix ex: <http://example.org/> .
ex:a ex:knows ex:b .
"#;
    let graph = Graph::load_str(ttl, "turtle").unwrap();
    // A non-topology predicate is NOT rewritten: it matches the asserted triple.
    let got = run(
        &graph,
        "PREFIX ex: <http://example.org/>
         SELECT ?f WHERE { ex:a ex:knows ?f }",
    );
    assert_eq!(got, vec!["<http://example.org/b>".to_string()]);
}

// ---------------------------------------------------------------------------
// sq-5ts8 — query-rewrite conformance for the OTHER two OGC
// topology-relation families.
//
// The OGC `/conf/query-rewrite-extension` conformance class spans all THREE
// topology-relation families — Simple Features (`geo:sf*`), Egenhofer
// (`geo:eh*`) and RCC8 (`geo:rcc8*`). The fixtures above cover only `sf*`; the
// rewrite (`src/rewrite.rs`) recognises all three (see `TOPOLOGY_RELATIONS`)
// but, until now, only `sf*` was proven end-to-end. These fixtures close that
// gap: an `eh*` and an `rcc8*` property form, answered with NO asserted
// topology triple, over polygons whose expected relation is the authoritative
// ground truth from
// `tests/relations.rs::egenhofer_and_rcc8_partition_region_pairs`
// (SMALL ⊏ BIG ⇒ ehInside / rcc8ntpp; BIG ∥ FAR ⇒ ehDisjoint / rcc8dc).
// ---------------------------------------------------------------------------

/// Polygons whose pairwise Egenhofer/RCC8 relations are pinned by the lexical
/// truth table in `tests/relations.rs`. SMALL lies strictly inside BIG; FAR is
/// disjoint from both. Each feature resolves through `geo:hasDefaultGeometry`.
const REGIONS_TTL: &str = r#"
@prefix geo: <http://www.opengis.net/ont/geosparql#> .
@prefix ex:  <http://example.org/> .

ex:small  geo:hasDefaultGeometry ex:smallGeom .
ex:smallGeom geo:asWKT "POLYGON((1 1, 2 1, 2 2, 1 2, 1 1))"^^geo:wktLiteral .

ex:big    geo:hasDefaultGeometry ex:bigGeom .
ex:bigGeom geo:asWKT "POLYGON((0 0, 4 0, 4 4, 0 4, 0 0))"^^geo:wktLiteral .

ex:far    geo:hasDefaultGeometry ex:farGeom .
ex:farGeom geo:asWKT "POLYGON((9 9, 10 9, 10 10, 9 10, 9 9))"^^geo:wktLiteral .

# No asserted geo:eh*/geo:rcc8* triple anywhere.
"#;

#[test]
fn egenhofer_property_form_resolves_default_geometry() {
    let graph = Graph::load_str(REGIONS_TTL, "turtle").unwrap();
    // ?f geo:ehInside ex:big — SMALL is the one region strictly inside BIG.
    // (BIG is ehEquals, not ehInside, of itself; FAR is ehDisjoint.)
    let got = run(
        &graph,
        "PREFIX geo: <http://www.opengis.net/ont/geosparql#>
         PREFIX ex:  <http://example.org/>
         SELECT ?f WHERE { ?f geo:ehInside ex:big }",
    );
    assert_eq!(
        got,
        vec!["<http://example.org/small>".to_string()],
        "ehInside property form must select exactly the region strictly inside BIG"
    );
}

#[test]
fn egenhofer_disjoint_property_form() {
    let graph = Graph::load_str(REGIONS_TTL, "turtle").unwrap();
    // ?f geo:ehDisjoint ex:far — BIG and SMALL are both disjoint from FAR; FAR
    // is ehEquals (not ehDisjoint) of itself.
    let got = run(
        &graph,
        "PREFIX geo: <http://www.opengis.net/ont/geosparql#>
         PREFIX ex:  <http://example.org/>
         SELECT ?f WHERE { ?f geo:ehDisjoint ex:far }",
    );
    assert_eq!(
        got,
        vec![
            "<http://example.org/big>".to_string(),
            "<http://example.org/small>".to_string(),
        ]
    );
}

#[test]
fn rcc8_ntpp_property_form_resolves_default_geometry() {
    let graph = Graph::load_str(REGIONS_TTL, "turtle").unwrap();
    // ?f geo:rcc8ntpp ex:big — SMALL is a non-tangential proper part of BIG.
    // rcc8ntpp is irreflexive, so BIG is NOT rcc8ntpp of itself; FAR is rcc8dc.
    let got = run(
        &graph,
        "PREFIX geo: <http://www.opengis.net/ont/geosparql#>
         PREFIX ex:  <http://example.org/>
         SELECT ?f WHERE { ?f geo:rcc8ntpp ex:big }",
    );
    assert_eq!(
        got,
        vec!["<http://example.org/small>".to_string()],
        "rcc8ntpp property form must select exactly the non-tangential proper part of BIG"
    );
}

#[test]
fn rcc8_inverse_direction_property_form() {
    let graph = Graph::load_str(REGIONS_TTL, "turtle").unwrap();
    // ex:big geo:rcc8ntppi ?f — the inverse: BIG has SMALL as a non-tangential
    // proper part (rcc8ntppi is rcc8ntpp with the arguments swapped).
    let got = run(
        &graph,
        "PREFIX geo: <http://www.opengis.net/ont/geosparql#>
         PREFIX ex:  <http://example.org/>
         SELECT ?f WHERE { ex:big geo:rcc8ntppi ?f }",
    );
    assert_eq!(got, vec!["<http://example.org/small>".to_string()]);
}

#[test]
fn egenhofer_standard_entry_point_does_not_rewrite() {
    // HONESTY anchor mirroring `standard_entry_point_does_not_rewrite` for the
    // Egenhofer family: WITHOUT geosparql_rewrite, a geo:ehInside predicate binds
    // only asserted triples (there are none), so zero results.
    let graph = Graph::load_str(REGIONS_TTL, "turtle").unwrap();
    let reg = geof_registry();
    let r = with_functions(&reg, || {
        sparq_engine::query(
            &graph,
            "PREFIX geo: <http://www.opengis.net/ont/geosparql#>
             PREFIX ex:  <http://example.org/>
             SELECT ?f WHERE { ?f geo:ehInside ex:big }",
        )
    })
    .unwrap();
    assert_eq!(
        r.len(),
        0,
        "the STANDARD entry point must NOT auto-expand the eh* property form"
    );
}

#[test]
fn rewrite_is_a_no_op_for_non_topology_queries() {
    // The algebra-level entry point leaves a topology-free query structurally
    // identical (so the standard evaluator behaviour is untouched).
    let q = "PREFIX ex: <http://example.org/>
             SELECT ?s ?o WHERE { ?s ex:p ?o . FILTER(?o > 3) }";
    let before = sparq_engine::PreparedQuery::parse(q).unwrap().into_query();
    let after = rewrite_query(before.clone());
    assert_eq!(before.to_string(), after.to_string());
}

#[test]
fn standard_entry_point_does_not_rewrite() {
    // HONESTY anchor: WITHOUT geosparql_rewrite, the property form binds only
    // asserted triples — here there are none, so zero results. This is the
    // contrast that the rewrite entry point fixes.
    let graph = Graph::load_str(TTL, "turtle").unwrap();
    let reg = geof_registry();
    let r = with_functions(&reg, || {
        sparq_engine::query(
            &graph,
            "PREFIX geo: <http://www.opengis.net/ont/geosparql#>
             PREFIX ex:  <http://example.org/>
             SELECT ?f WHERE { ?f geo:sfWithin ex:region }",
        )
    })
    .unwrap();
    assert_eq!(
        r.len(),
        0,
        "the STANDARD entry point must NOT auto-expand the property form"
    );
}
