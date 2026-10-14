//! Direct unit tests covering previously-uncovered code paths in sparq-geo.
//! sq-qcnn.17: raise sparq-geo line coverage floor 87->90.

use geo_types::{
    Coord, Geometry, GeometryCollection, Line as GeoLine, LineString, MultiLineString, MultiPoint,
    MultiPolygon, Point, Polygon, Rect, Triangle,
}; //
use sparq_core::Graph;
use sparq_geo::{
    geof, is_geometry_datatype, metadata, parse_geometry_literal, parse_wkt_literal, Crs, GeoError,
    GeoGeometry, GeoIndex,
};
use sparq_geo::vocab::WKT_LITERAL;

// ---- literal.rs: GeoGeometry::new, Crs::from_iri Other, parse_geometry_literal unsupported ---

/// GeoGeometry::new() wraps a geometry in the default CRS84.
#[test]
fn geo_geometry_new_uses_crs84() {
    let pt = Geometry::Point(Point::new(1.0, 2.0)); //
    let g = GeoGeometry::new(pt.clone()); //
    assert_eq!(g.crs, Crs::Crs84);
    assert_eq!(g.geometry, pt);
}

/// Crs::from_iri with an unrecognised IRI returns Crs::Other.
#[test]
fn crs_from_iri_other_branch() {
    let iri = "http://example.org/my-projection"; //
    let crs = Crs::from_iri(iri);
    assert_eq!(crs, Crs::Other(iri.to_string()));
    assert!(!crs.is_geographic());
    assert_eq!(crs.iri(), iri);
}

/// parse_geometry_literal with an unsupported datatype returns GeoError::Unsupported.
#[test]
fn parse_geometry_literal_unsupported_datatype_errors() {
    let result = parse_geometry_literal( //
        "POINT(1 2)",
        "http://example.org/not-a-geo-literal",
    );
    assert!(
        matches!(result, Err(GeoError::Unsupported(ref m)) if m.contains("not-a-geo-literal")),
        "expected Unsupported, got {:?}",
        result
    );
}

/// is_geometry_datatype distinguishes geo serializations from other datatypes.
#[test]
fn is_geometry_datatype_false_for_xsd_string() {
    assert!(!is_geometry_datatype("http://www.w3.org/2001/XMLSchema#string")); //
    assert!(is_geometry_datatype(WKT_LITERAL));
}

// ---- geof.rs: euclidean_distance dispatch for all geometry types as first arg ----

/// euclidean_distance hits the MultiPolygon, MultiPoint, Rect, Triangle, and
/// GeometryCollection match arms when they are the FIRST argument.
#[test]
fn euclidean_distance_all_first_arg_types() {
    let target = Geometry::Point(Point::new(0.0, 0.0)); //

    // MultiPolygon as a: closest vertex is (1,0) → distance = 1.
    let mp_geom = Geometry::MultiPolygon(MultiPolygon(vec![Polygon::new( //
        LineString(vec![
            Coord { x: 1.0, y: 0.0 },
            Coord { x: 2.0, y: 0.0 },
            Coord { x: 1.5, y: 1.0 },
            Coord { x: 1.0, y: 0.0 },
        ]),
        vec![],
    )]));
    let d_mp = geof::euclidean_distance(&mp_geom, &target);
    assert!((d_mp - 1.0).abs() < 1e-6, "MultiPolygon distance was {}", d_mp);

    // MultiPoint as a: closest is (1,0) → distance = 1.
    let mpt_geom = Geometry::MultiPoint(MultiPoint(vec![ //
        Point::new(3.0, 4.0),
        Point::new(1.0, 0.0),
    ]));
    let d_mpt = geof::euclidean_distance(&mpt_geom, &target);
    assert!((d_mpt - 1.0).abs() < 1e-6, "MultiPoint distance was {}", d_mpt);

    // Rect as a: closest corner is (1,1) → distance = sqrt(2).
    let rect_geom = Geometry::Rect(Rect::new( //
        Coord { x: 1.0, y: 1.0 },
        Coord { x: 2.0, y: 2.0 },
    ));
    let d_rect = geof::euclidean_distance(&rect_geom, &target);
    let expected_rect = 2_f64.sqrt();
    assert!(
        (d_rect - expected_rect).abs() < 1e-6,
        "Rect distance was {}, expected {}",
        d_rect,
        expected_rect
    );

    // Triangle as a: vertex (1,0) is closest → distance = 1.
    let tri_geom = Geometry::Triangle(Triangle( //
        Coord { x: 1.0, y: 0.0 },
        Coord { x: 2.0, y: 0.0 },
        Coord { x: 1.5, y: 1.0 },
    ));
    let d_tri = geof::euclidean_distance(&tri_geom, &target);
    assert!((d_tri - 1.0).abs() < 1e-6, "Triangle distance was {}", d_tri);

    // GeometryCollection as a: closest member gives distance = 1.
    let gc_geom = Geometry::GeometryCollection(GeometryCollection(vec![ //
        Geometry::Point(Point::new(5.0, 0.0)),
        Geometry::Point(Point::new(1.0, 0.0)),
    ]));
    let d_gc = geof::euclidean_distance(&gc_geom, &target);
    assert!((d_gc - 1.0).abs() < 1e-6, "GeometryCollection distance was {}", d_gc);
}

// ---- geof.rs: distance_meters empty-geometry error path -----------------------

/// distance_meters with a non-point empty geometry and a non-empty partner hits
/// the bounding-rect None branch and returns GeoError::Unsupported.
#[test]
fn distance_meters_empty_geometry_error() {
    let empty_mls = Geometry::MultiLineString(MultiLineString(vec![])); //
    // A distant non-empty LineString so the two don't intersect.
    let far_ls = Geometry::LineString(LineString(vec![ //
        Coord { x: 100.0, y: 0.0 },
        Coord { x: 101.0, y: 0.0 },
    ]));
    let result = geof::distance_meters(&empty_mls, &far_ls);
    assert!(
        matches!(result, Err(GeoError::Unsupported(ref m)) if m.contains("empty")),
        "expected Unsupported(empty geometries), got {:?}",
        result
    );
}

// ---- geof.rs: point_to_geometry_meters Indeterminate --------------------------

/// An empty GeometryCollection returns Closest::Indeterminate from haversine_closest_point,
/// causing point_to_geometry_meters to return GeoError::Unsupported.
#[test]
fn point_to_geometry_meters_indeterminate_for_empty_gc() {
    let empty_gc = Geometry::GeometryCollection(GeometryCollection(vec![])); //
    let p = Point::new(0.0, 0.0);
    let result = geof::point_to_geometry_meters(p, &empty_gc);
    assert!(
        matches!(result, Err(GeoError::Unsupported(ref m)) if m.contains("indeterminate")),
        "expected Indeterminate error, got {:?}",
        result
    );
}

// ---- geof.rs: boundary on Line, MultiLineString, MultiPolygon, Rect, Triangle ---

/// boundary of a bare geo_types::Line gives MULTIPOINT of its two endpoints.
#[test]
fn boundary_of_bare_line() {
    let g = GeoGeometry::new(Geometry::Line(GeoLine { //
        start: Coord { x: 0.0, y: 0.0 },
        end: Coord { x: 3.0, y: 4.0 },
    }));
    let b = geof::boundary(&g).unwrap();
    let wkt = b.to_wkt_literal();
    // The boundary of a bare Line is a MULTIPOINT of its two endpoints.
    assert!(wkt.contains("MULTIPOINT"), "expected MULTIPOINT, got {}", wkt);
}

/// boundary of a MultiLineString applies the mod-2 endpoint rule.
#[test]
fn boundary_of_multilinestring() {
    // Two disjoint open curves: each contributes two boundary endpoints.
    let g = parse_wkt_literal("MULTILINESTRING((0 0, 1 0),(2 0, 3 0))").unwrap(); //
    let b = geof::boundary(&g).unwrap();
    let wkt = b.to_wkt_literal();
    assert!(wkt.contains("MULTIPOINT"), "expected MULTIPOINT, got {}", wkt);
}

/// boundary of a MultiPolygon gives a MULTILINESTRING of all rings.
#[test]
fn boundary_of_multipolygon() {
    let g = parse_wkt_literal( //
        "MULTIPOLYGON(((0 0,1 0,1 1,0 1,0 0)),((2 2,3 2,3 3,2 3,2 2)))",
    )
    .unwrap();
    let b = geof::boundary(&g).unwrap();
    let wkt = b.to_wkt_literal();
    assert!(
        wkt.contains("MULTILINESTRING"),
        "expected MULTILINESTRING, got {}",
        wkt
    );
}

/// boundary of a Rect delegates to its polygon form (MULTILINESTRING ring).
#[test]
fn boundary_of_rect() {
    let g = GeoGeometry::new(Geometry::Rect(Rect::new( //
        Coord { x: 0.0, y: 0.0 },
        Coord { x: 1.0, y: 1.0 },
    )));
    let b = geof::boundary(&g).unwrap();
    let wkt = b.to_wkt_literal();
    assert!(
        wkt.contains("LINESTRING"),
        "expected LINESTRING in Rect boundary, got {}",
        wkt
    );
}

/// boundary of a Triangle delegates to its polygon form (MULTILINESTRING ring).
#[test]
fn boundary_of_triangle() {
    let g = GeoGeometry::new(Geometry::Triangle(Triangle( //
        Coord { x: 0.0, y: 0.0 },
        Coord { x: 1.0, y: 0.0 },
        Coord { x: 0.5, y: 1.0 },
    )));
    let b = geof::boundary(&g).unwrap();
    let wkt = b.to_wkt_literal();
    assert!(
        wkt.contains("LINESTRING"),
        "expected LINESTRING in Triangle boundary, got {}",
        wkt
    );
}

// ---- geof.rs: polygonal() Rect/Triangle arms ----------------------------------

/// intersection of a Rect with a Triangle exercises polygonal() for both types.
#[test]
fn intersection_rect_with_triangle_covers_polygonal() {
    let rect = GeoGeometry::new(Geometry::Rect(Rect::new( //
        Coord { x: 0.0, y: 0.0 },
        Coord { x: 2.0, y: 2.0 },
    )));
    let tri = GeoGeometry::new(Geometry::Triangle(Triangle( //
        Coord { x: 0.0, y: 0.0 },
        Coord { x: 3.0, y: 0.0 },
        Coord { x: 1.5, y: 3.0 },
    )));
    // Both are polygonal: the BooleanOps overlay runs.
    let result = geof::intersection(&rect, &tri).unwrap();
    let wkt = result.to_wkt_literal();
    assert!(
        wkt.to_uppercase().contains("POLYGON"),
        "expected a polygon intersection, got {}",
        wkt
    );
}

// ---- geof.rs: sym_difference None/None and union with empty operand -----------

/// sym_difference with two empty geometries hits the (None, None) arm → empty_of_dimension(0).
#[test]
fn sym_difference_both_empty_is_empty() {
    let empty_a = GeoGeometry::new(Geometry::MultiPoint(MultiPoint(vec![]))); //
    let empty_b = GeoGeometry::new(Geometry::MultiPoint(MultiPoint(vec![]))); //
    let result = geof::sym_difference(&empty_a, &empty_b).unwrap();
    // (None, None) => empty_of_dimension(0) => empty MULTIPOINT.
    let wkt = result.to_wkt_literal();
    assert!(
        wkt.to_uppercase().contains("MULTIPOINT"),
        "expected empty MULTIPOINT, got {}",
        wkt
    );
}

/// union with an empty left operand hits the (None, _) arm → returns right.
#[test]
fn union_with_empty_left_returns_right() {
    let empty = GeoGeometry::new(Geometry::MultiPoint(MultiPoint(vec![]))); //
    let pt = GeoGeometry::new(Geometry::Point(Point::new(1.0, 2.0))); //
    let result = geof::union(&empty, &pt).unwrap();
    assert_eq!(result.geometry, pt.geometry);
}

/// union with an empty right operand hits the (_, None) arm → returns left.
#[test]
fn union_with_empty_right_returns_left() {
    let pt = GeoGeometry::new(Geometry::Point(Point::new(1.0, 2.0))); //
    let empty = GeoGeometry::new(Geometry::MultiPoint(MultiPoint(vec![]))); //
    let result = geof::union(&pt, &empty).unwrap();
    assert_eq!(result.geometry, pt.geometry);
}

/// union of mixed dimension (point ∪ line) returns a GEOMETRYCOLLECTION.
#[test]
fn union_mixed_dimension_is_geometry_collection() {
    let pt = GeoGeometry::new(Geometry::Point(Point::new(0.0, 0.0))); //
    let ls = GeoGeometry::new(Geometry::LineString(LineString(vec![ //
        Coord { x: 1.0, y: 0.0 },
        Coord { x: 2.0, y: 0.0 },
    ])));
    let result = geof::union(&pt, &ls).unwrap();
    let wkt = result.to_wkt_literal();
    assert!(
        wkt.to_uppercase().contains("GEOMETRYCOLLECTION"),
        "expected GEOMETRYCOLLECTION, got {}",
        wkt
    );
}

// ---- geof.rs: buffer at the pole ----------------------------------------------

/// buffer with metric units at a latitude beyond the pole (cos > 90° is negative → kx ≤ 0)
/// returns GeoError::Unsupported. sq-qcnn.17 note: cos(90°) in f64 is
/// ~6e-17 (not exactly 0 due to FP), so exactly 90° slips through the kx <= 0 guard;
/// a latitude just past 90 gives a negative cosine and reliably triggers the error path.
#[test]
fn buffer_at_pole_returns_unsupported() {
    // lat 91°: cos(91° in rad) ≈ -0.01745 → kx < 0 → pole guard fires.
    let beyond_pole = GeoGeometry::new(Geometry::Point(Point::new(0.0, 91.0))); //
    let result = geof::buffer(&beyond_pole, 1000.0, geof::Unit::Metre);
    assert!(
        matches!(result, Err(GeoError::Unsupported(ref m)) if m.contains("pole")),
        "expected Unsupported(pole), got {:?}",
        result
    );
}

// ---- geof.rs: empty_of_dimension via set ops ----------------------------------

/// intersection with an empty MultiPoint on the left hits the (None, _) arm →
/// empty_of_dimension(0) → empty MULTIPOINT.
#[test]
fn intersection_with_empty_operand_returns_empty_multipoint() {
    let empty = GeoGeometry::new(Geometry::MultiPoint(MultiPoint(vec![]))); //
    let ls = GeoGeometry::new(Geometry::LineString(LineString(vec![ //
        Coord { x: 0.0, y: 0.0 },
        Coord { x: 1.0, y: 0.0 },
    ])));
    let result = geof::intersection(&empty, &ls).unwrap();
    let wkt = result.to_wkt_literal();
    assert!(
        wkt.to_uppercase().contains("MULTIPOINT"),
        "expected empty MULTIPOINT, got {}",
        wkt
    );
}

/// difference with a line minus itself hits collect_lines([]) → empty_of_dimension(1) →
/// empty MULTILINESTRING.
#[test]
fn difference_line_minus_itself_is_empty_multilinestring() {
    let ls = GeoGeometry::new(Geometry::LineString(LineString(vec![ //
        Coord { x: 0.0, y: 0.0 },
        Coord { x: 1.0, y: 0.0 },
    ])));
    let result = geof::difference(&ls, &ls).unwrap();
    // Line minus itself = empty portions → collect_lines([]) → empty_of_dimension(1).
    let wkt = result.to_wkt_literal();
    assert!(
        wkt.to_uppercase().contains("MULTILINESTRING"),
        "expected empty MULTILINESTRING, got {}",
        wkt
    );
}

// ---- index.rs: within_distance_literals and bbox_candidate_literals -----------

/// Build a small GeoIndex: three points — two nearby, one far away.
fn build_small_index() -> (Graph, GeoIndex) {
    let nt = concat!( //
        "<http://ex.org/e1> <http://www.opengis.net/ont/geosparql#asWKT> ",
        "\"POINT(0 0)\"^^<http://www.opengis.net/ont/geosparql#wktLiteral> .\n",
        "<http://ex.org/e2> <http://www.opengis.net/ont/geosparql#asWKT> ",
        "\"POINT(1 0)\"^^<http://www.opengis.net/ont/geosparql#wktLiteral> .\n",
        "<http://ex.org/e3> <http://www.opengis.net/ont/geosparql#asWKT> ",
        "\"POINT(50 50)\"^^<http://www.opengis.net/ont/geosparql#wktLiteral> .\n"
    );
    let graph = Graph::load_str(nt, "ntriples").unwrap();
    let index = GeoIndex::build(&graph);
    (graph, index)
}

/// within_distance_literals returns the correct literal terms within radius.
#[test]
fn within_distance_literals_returns_nearby_literals() {
    let (_g, index) = build_small_index(); //
    // Center at origin; 200 km radius captures e1 (0 km) and e2 (~111 km) but not e3.
    let center = Point::new(0.0, 0.0);
    let literals = index.within_distance_literals(center, 200_000.0);
    assert_eq!(literals.len(), 2, "expected 2 nearby literals, got {:?}", literals);
    // The candidate set is a superset of the exact within_distance result.
    let hits = index.within_distance(center, 200_000.0, None);
    assert!(
        literals.len() >= hits.len(),
        "literals {} must be >= exact hits {}",
        literals.len(),
        hits.len()
    );
}

/// within_distance_literals with zero radius returns only the exact-match point.
#[test]
fn within_distance_literals_zero_radius() {
    let (_g, index) = build_small_index(); //
    let center = Point::new(0.0, 0.0);
    let literals = index.within_distance_literals(center, 0.0);
    assert_eq!(literals.len(), 1, "expected exactly e1 at distance 0");
}

/// bbox_candidate_literals returns at least every entity whose bbox intersects the query.
#[test]
fn bbox_candidate_literals_superset_of_intersects() {
    let (_g, index) = build_small_index(); //
    // A polygon bbox covering (0,0) and (1,0) but not (50,50).
    let query_geom = parse_wkt_literal("POLYGON((-1 -1, 2 -1, 2 1, -1 1, -1 -1))").unwrap();
    let candidates = index.bbox_candidate_literals(&query_geom);
    assert!(
        candidates.len() >= 2,
        "expected >= 2 candidates (e1 and e2), got {}",
        candidates.len()
    );
    // Exact intersects must be a subset of the candidates.
    let exact = index.intersects(&query_geom);
    assert!(
        candidates.len() >= exact.len(),
        "candidate set ({}) must be a superset of exact intersects ({})",
        candidates.len(),
        exact.len()
    );
}

/// bbox_candidate_literals returns empty for a non-geographic CRS.
#[test]
fn bbox_candidate_literals_empty_for_non_geographic_crs() {
    let (_g, index) = build_small_index(); //
    let other_crs = parse_wkt_literal("<http://example.org/my-crs> POINT(0 0)").unwrap();
    let candidates = index.bbox_candidate_literals(&other_crs);
    assert_eq!(candidates.len(), 0, "non-geographic CRS must yield no candidates");
}

// ---- index.rs: ball_windows pole-covering branch ------------------------------

/// within_distance with a center close to the north pole and a large radius triggers the
/// pole-covering window (lat_max >= 90.0 - 1e-12) inside ball_windows.
#[test]
fn within_distance_pole_covering_window() {
    let nt = concat!( //
        "<http://ex.org/a> <http://www.opengis.net/ont/geosparql#asWKT> ",
        "\"POINT(0 89)\"^^<http://www.opengis.net/ont/geosparql#wktLiteral> .\n"
    );
    let graph = Graph::load_str(nt, "ntriples").unwrap();
    let index = GeoIndex::build(&graph);
    // Center at (0°, 89.5°); 1 000 km radius wraps past the north pole.
    // This forces lat_max > 90.0 - 1e-12 inside ball_windows → single full-longitude window.
    let center = Point::new(0.0, 89.5); //
    let hits = index.within_distance(center, 1_000_000.0, None);
    // The point at (0, 89) is ~55 km from (0, 89.5); well within 1 000 km.
    assert!(!hits.is_empty(), "expected the high-lat point to be found near the pole");
}

// ---- metadata.rs: uncovered paths --------------------------------------------

/// topological_dimension for an empty MultiLineString returns None.
#[test]
fn topological_dimension_empty_multilinestring_is_none() {
    let g = GeoGeometry::new(Geometry::MultiLineString(MultiLineString(vec![]))); //
    let m = g.metadata();
    assert_eq!(m.dimension, None);
    assert!(m.is_empty);
}

/// topological_dimension for a non-empty MultiPolygon returns Some(2).
#[test]
fn topological_dimension_nonempty_multipolygon_is_two() {
    let g = parse_wkt_literal("MULTIPOLYGON(((0 0,1 0,1 1,0 1,0 0)))").unwrap(); //
    let m = g.metadata();
    assert_eq!(m.dimension, Some(2));
    assert!(!m.is_empty);
    assert_eq!(m.coordinate_dimension, Some(2));
    assert_eq!(m.spatial_dimension, Some(2));
}

/// is_simple for a MultiLineString (multi-curve): two disjoint simple curves → simple;
/// a self-crossing member curve → not simple.
#[test]
fn is_simple_multilinestring() {
    // Two disjoint straight curves: simple.
    let simple_mls = parse_wkt_literal("MULTILINESTRING((0 0, 1 1),(2 2, 3 3))").unwrap();
    assert!(simple_mls.metadata().is_simple, "two disjoint straight curves must be simple");

    // A MultiLineString with a self-crossing member curve: not simple.
    let non_simple_mls = parse_wkt_literal("MULTILINESTRING((0 0, 2 2, 0 2, 2 0))").unwrap(); //
    assert!(!non_simple_mls.metadata().is_simple, "self-crossing curve must not be simple");
}

/// lex::spatial_dimension returns Some(2) for a non-empty point.
#[test]
fn lex_spatial_dimension_returns_two_for_point() {
    let result = metadata::lex::spatial_dimension("POINT(1 2)", WKT_LITERAL).unwrap(); //
    assert_eq!(result, Some(2));
}

/// lex::spatial_dimension returns None for an empty geometry.
#[test]
fn lex_spatial_dimension_returns_none_for_empty() {
    let result = //
        metadata::lex::spatial_dimension("GEOMETRYCOLLECTION EMPTY", WKT_LITERAL).unwrap();
    assert_eq!(result, None);
}
