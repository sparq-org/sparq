//! Differential and candidate-shrinkage tests for the opt-in exact topology index.

#![cfg(feature = "topology_index")]

use oxrdf::Term;
use sparq_core::Graph;
use sparq_geo::{geof, parse_geometry_literal, parse_wkt_literal, GeoGeometry, GeoIndex};
use std::collections::HashSet;

// sq-jrdds: every fixture geometry's AABB intersects the diamond's
// AABB, but only the centre and near-axis points are topologically within it.
const FIXTURE: &str = r#"
    @prefix geo: <http://www.opengis.net/ont/geosparql#> .
    @prefix ex:  <http://ex/> .
    ex:centre-a geo:hasGeometry ex:centre-geometry .
    ex:centre-b geo:hasGeometry ex:centre-geometry .
    ex:centre-geometry geo:asWKT "POINT(0 0)"^^geo:wktLiteral .
    ex:east   geo:asWKT "POINT(1.5 0)"^^geo:wktLiteral .
    ex:north  geo:asWKT "POINT(0 1.5)"^^geo:wktLiteral .
    ex:ne     geo:asWKT "POINT(1.5 1.5)"^^geo:wktLiteral .
    ex:nw     geo:asWKT "POINT(-1.5 1.5)"^^geo:wktLiteral .
    ex:se     geo:asWKT "POINT(1.5 -1.5)"^^geo:wktLiteral .
    ex:sw     geo:asWKT "POINT(-1.5 -1.5)"^^geo:wktLiteral .
    ex:far    geo:asWKT "POINT(8 8)"^^geo:wktLiteral .
"#;

fn index() -> GeoIndex {
    GeoIndex::build(&Graph::load_str(FIXTURE, "turtle").expect("valid topology fixture"))
}

fn geometry(term: &Term) -> GeoGeometry {
    let Term::Literal(literal) = term else {
        panic!("GeoIndex candidates must be geometry literals")
    };
    parse_geometry_literal(literal.value(), literal.datatype().as_str())
        .expect("fixture geometry parses")
}

fn set(terms: Vec<Term>) -> HashSet<Term> {
    terms.into_iter().collect()
}

fn assert_exact_for_region(index: &GeoIndex, region: &GeoGeometry) {
    let bbox = index.bbox_candidate_literals(region);
    let expected_within = set(bbox
        .iter()
        .filter(|literal| geof::sf_within(&geometry(literal), region).unwrap())
        .cloned()
        .collect());
    let expected_contains = set(bbox
        .iter()
        .filter(|literal| geof::sf_contains(region, &geometry(literal)).unwrap())
        .cloned()
        .collect());

    assert_eq!(set(index.within_region_literals(region)), expected_within);
    assert_eq!(
        set(index.contains_region_literals(region)),
        expected_contains
    );
    assert_eq!(expected_within, expected_contains);
    assert!(
        !expected_within.is_empty(),
        "fixture must witness true matches"
    );
}

#[test]
fn exact_refinement_matches_geof_for_polygon_and_box_regions() {
    let index = index();
    let diamond = parse_wkt_literal("POLYGON((0 2, 2 0, 0 -2, -2 0, 0 2))").unwrap();
    let axis_aligned_box =
        parse_wkt_literal("POLYGON((-1.75 -1.75, 1.75 -1.75, 1.75 1.75, -1.75 1.75, -1.75 -1.75))")
            .unwrap();

    assert_exact_for_region(&index, &diamond);
    assert_exact_for_region(&index, &axis_aligned_box);
}

#[test]
fn diamond_exact_refinement_strictly_shrinks_the_bbox_superset() {
    let index = index();
    let diamond = parse_wkt_literal("POLYGON((0 2, 2 0, 0 -2, -2 0, 0 2))").unwrap();
    let bbox = index.bbox_candidate_literals(&diamond);
    let exact = index.within_region_literals(&diamond);

    assert_eq!(
        exact.len(),
        3,
        "centre and the two near-axis points are within"
    );
    assert!(
        exact.len() < bbox.len(),
        "corner points must be removed by DE-9IM refinement"
    );
}

#[test]
fn unsupported_constant_regions_decline_with_an_empty_set() {
    let index = index();
    let projected = parse_wkt_literal(
        "<http://www.opengis.net/def/crs/EPSG/0/3857> POLYGON((0 0, 2 0, 2 2, 0 2, 0 0))",
    )
    .unwrap();
    let empty = parse_wkt_literal("POLYGON EMPTY").unwrap();

    assert!(index.within_region_literals(&projected).is_empty());
    assert!(index.contains_region_literals(&projected).is_empty());
    assert!(index.within_region_literals(&empty).is_empty());
    assert!(index.contains_region_literals(&empty).is_empty());
}
