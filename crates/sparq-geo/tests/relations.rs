//! geof: spatial functions: a truth table for the eight simple-features
//! relations on hand-built geometries, plus distance / envelope / boundary /
//! convexHull checks.

use sparq_geo::geof::{self, lex, Unit};
use sparq_geo::{parse_wkt_literal, GeoError};

/// (a, b, [equals, disjoint, intersects, touches, crosses, within, contains, overlaps])
const TRUTH_TABLE: &[(&str, &str, [bool; 8])] = &[
    // Identical polygons.
    (
        "POLYGON((0 0, 4 0, 4 4, 0 4, 0 0))",
        "POLYGON((0 0, 4 0, 4 4, 0 4, 0 0))",
        [true, false, true, false, false, true, true, false],
    ),
    // Same point set, different vertex order: still topologically equal.
    (
        "POLYGON((0 0, 4 0, 4 4, 0 4, 0 0))",
        "POLYGON((4 0, 4 4, 0 4, 0 0, 4 0))",
        [true, false, true, false, false, true, true, false],
    ),
    // Small polygon strictly inside a big one.
    (
        "POLYGON((1 1, 2 1, 2 2, 1 2, 1 1))",
        "POLYGON((0 0, 4 0, 4 4, 0 4, 0 0))",
        [false, false, true, false, false, true, false, false],
    ),
    // …and the reverse: contains.
    (
        "POLYGON((0 0, 4 0, 4 4, 0 4, 0 0))",
        "POLYGON((1 1, 2 1, 2 2, 1 2, 1 1))",
        [false, false, true, false, false, false, true, false],
    ),
    // Far-apart polygons: disjoint.
    (
        "POLYGON((0 0, 1 0, 1 1, 0 1, 0 0))",
        "POLYGON((5 5, 6 5, 6 6, 5 6, 5 5))",
        [false, true, false, false, false, false, false, false],
    ),
    // Edge-adjacent polygons: touches (boundaries meet, interiors don't).
    (
        "POLYGON((0 0, 1 0, 1 1, 0 1, 0 0))",
        "POLYGON((1 0, 2 0, 2 1, 1 1, 1 0))",
        [false, false, true, true, false, false, false, false],
    ),
    // Corner-touching polygons: touches.
    (
        "POLYGON((0 0, 1 0, 1 1, 0 1, 0 0))",
        "POLYGON((1 1, 2 1, 2 2, 1 2, 1 1))",
        [false, false, true, true, false, false, false, false],
    ),
    // Partially overlapping polygons: overlaps.
    (
        "POLYGON((0 0, 2 0, 2 2, 0 2, 0 0))",
        "POLYGON((1 1, 3 1, 3 3, 1 3, 1 1))",
        [false, false, true, false, false, false, false, true],
    ),
    // Line crossing a polygon interior: crosses (dim 1 vs dim 2).
    (
        "LINESTRING(-1 1, 3 1)",
        "POLYGON((0 0, 2 0, 2 2, 0 2, 0 0))",
        [false, false, true, false, true, false, false, false],
    ),
    // Two lines crossing at a point: crosses.
    (
        "LINESTRING(0 0, 2 2)",
        "LINESTRING(0 2, 2 0)",
        [false, false, true, false, true, false, false, false],
    ),
    // Two lines sharing an endpoint: touches.
    (
        "LINESTRING(0 0, 1 1)",
        "LINESTRING(1 1, 2 0)",
        [false, false, true, true, false, false, false, false],
    ),
    // Collinear overlapping lines: overlaps (same dimension, partial sharing).
    (
        "LINESTRING(0 0, 2 0)",
        "LINESTRING(1 0, 3 0)",
        [false, false, true, false, false, false, false, true],
    ),
    // Point inside a polygon: within (and the polygon would contain it).
    (
        "POINT(1 1)",
        "POLYGON((0 0, 2 0, 2 2, 0 2, 0 0))",
        [false, false, true, false, false, true, false, false],
    ),
    // Point on a polygon's boundary: touches, NOT within (interior rule).
    (
        "POINT(0 1)",
        "POLYGON((0 0, 2 0, 2 2, 0 2, 0 0))",
        [false, false, true, true, false, false, false, false],
    ),
    // Point vs same point: equals.
    ("POINT(3 4)", "POINT(3 4)", [true, false, true, false, false, true, true, false]),
    // Point vs different point: disjoint.
    ("POINT(3 4)", "POINT(3 5)", [false, true, false, false, false, false, false, false]),
    // Line within a polygon.
    (
        "LINESTRING(0.5 0.5, 1.5 1.5)",
        "POLYGON((0 0, 2 0, 2 2, 0 2, 0 0))",
        [false, false, true, false, false, true, false, false],
    ),
];

#[test]
fn simple_features_truth_table() {
    type Rel = fn(&str, &str) -> Result<bool, GeoError>;
    let relations: [(&str, Rel); 8] = [
        ("sfEquals", lex::sf_equals),
        ("sfDisjoint", lex::sf_disjoint),
        ("sfIntersects", lex::sf_intersects),
        ("sfTouches", lex::sf_touches),
        ("sfCrosses", lex::sf_crosses),
        ("sfWithin", lex::sf_within),
        ("sfContains", lex::sf_contains),
        ("sfOverlaps", lex::sf_overlaps),
    ];
    for (a, b, expected) in TRUTH_TABLE {
        for ((name, f), want) in relations.iter().zip(expected) {
            let got = f(a, b).unwrap_or_else(|e| panic!("{name}({a}, {b}): {e}"));
            assert_eq!(got, *want, "{name}({a}, {b})");
        }
    }
}

#[test]
fn intersects_is_negation_of_disjoint_and_relations_are_consistent() {
    for (a, b, _) in TRUTH_TABLE {
        let inter = lex::sf_intersects(a, b).unwrap();
        assert_eq!(inter, !lex::sf_disjoint(a, b).unwrap(), "{a} / {b}");
        // within(a,b) == contains(b,a)
        assert_eq!(
            lex::sf_within(a, b).unwrap(),
            lex::sf_contains(b, a).unwrap(),
            "{a} / {b}"
        );
    }
}

const UOM_METRE: &str = "http://www.opengis.net/def/uom/OGC/1.0/metre";
const UOM_KM: &str = "http://www.opengis.net/def/uom/OGC/1.0/kilometre";
const UOM_DEGREE: &str = "http://www.opengis.net/def/uom/OGC/1.0/degree";
const UOM_RADIAN: &str = "http://www.opengis.net/def/uom/OGC/1.0/radian";

#[test]
fn distance_in_meters_london_to_paris() {
    // Charing Cross -> Notre-Dame, great-circle ≈ 341 km.
    let london = "POINT(-0.1276 51.5074)";
    let paris = "POINT(2.3496 48.8530)";
    let m = lex::distance(london, paris, UOM_METRE).unwrap();
    assert!((m - 341_000.0).abs() < 4_000.0, "got {m}");
    let km = lex::distance(london, paris, UOM_KM).unwrap();
    assert!((km - m / 1000.0).abs() < 1e-9);
}

#[test]
fn distance_in_degrees_and_radians() {
    let d = lex::distance("POINT(0 0)", "POINT(3 4)", UOM_DEGREE).unwrap();
    assert!((d - 5.0).abs() < 1e-12, "got {d}");
    let r = lex::distance("POINT(0 0)", "POINT(3 4)", UOM_RADIAN).unwrap();
    assert!((r - 5.0_f64.to_radians()).abs() < 1e-12, "got {r}");
}

#[test]
fn distance_point_to_polygon_is_to_closest_point() {
    // Point 1 degree of longitude east of the polygon's right edge, on the equator.
    let d = lex::distance("POINT(2 0)", "POLYGON((0 -1, 1 -1, 1 1, 0 1, 0 -1))", UOM_METRE)
        .unwrap();
    // One degree of longitude at the equator on the GRS80 mean sphere ≈ 111.195 km.
    assert!((d - 111_195.0).abs() < 100.0, "got {d}");
    // Point inside the polygon: distance 0.
    let z = lex::distance("POINT(0.5 0)", "POLYGON((0 -1, 1 -1, 1 1, 0 1, 0 -1))", UOM_METRE)
        .unwrap();
    assert_eq!(z, 0.0);
}

#[test]
fn distance_between_extended_geometries_approximates_locally() {
    // Two short parallel vertical lines 0.01° of longitude apart on the equator:
    // the equirectangular approximation should be within a metre of the truth.
    let d = lex::distance(
        "LINESTRING(0 0, 0 0.01)",
        "LINESTRING(0.01 0, 0.01 0.01)",
        UOM_METRE,
    )
    .unwrap();
    assert!((d - 1_111.95).abs() < 1.0, "got {d}");
}

#[test]
fn distance_epsg4326_mixes_with_crs84() {
    // Same two points, one written in EPSG:4326 lat/long order.
    let a = "POINT(-0.1276 51.5074)";
    let b = "<http://www.opengis.net/def/crs/EPSG/0/4326> POINT(48.8530 2.3496)";
    let m = lex::distance(a, b, UOM_METRE).unwrap();
    assert!((m - 341_000.0).abs() < 4_000.0, "got {m}");
}

#[test]
fn distance_errors() {
    // Unknown unit IRI.
    assert!(matches!(
        lex::distance("POINT(0 0)", "POINT(1 1)", "http://example.org/uom/furlong"),
        Err(GeoError::UnknownUnit(_))
    ));
    // Metres in a projected CRS: refused (no projection support).
    let bng = "<http://www.opengis.net/def/crs/EPSG/0/27700> POINT(530000 180000)";
    let bng2 = "<http://www.opengis.net/def/crs/EPSG/0/27700> POINT(531000 180000)";
    assert!(matches!(
        lex::distance(bng, bng2, UOM_METRE),
        Err(GeoError::NonGeographicCrs(_))
    ));
    // …but coordinate-space distance still works within that CRS.
    let d = lex::distance(bng, bng2, UOM_DEGREE).unwrap();
    assert!((d - 1000.0).abs() < 1e-9);
    // CRS mismatch between a projected and a geographic literal.
    assert!(matches!(
        lex::distance(bng, "POINT(0 0)", UOM_DEGREE),
        Err(GeoError::CrsMismatch(_, _))
    ));
}

#[test]
fn relations_require_compatible_crs() {
    let bng = "<http://www.opengis.net/def/crs/EPSG/0/27700> POINT(530000 180000)";
    assert!(matches!(lex::sf_intersects(bng, "POINT(0 0)"), Err(GeoError::CrsMismatch(_, _))));
}

#[test]
fn envelope_boundary_convex_hull() {
    // Envelope of a tilted line: its bounding box as a polygon.
    let env = lex::envelope("LINESTRING(0 0, 2 1, 1 3)").unwrap();
    let env = parse_wkt_literal(&env).unwrap();
    let corners = parse_wkt_literal("POLYGON((0 0, 2 0, 2 3, 0 3, 0 0))").unwrap();
    assert!(geof::sf_equals(&env, &corners).unwrap());

    // Boundary of a polygon with a hole: both rings.
    let b = lex::boundary("POLYGON((0 0, 4 0, 4 4, 0 4, 0 0), (1 1, 2 1, 2 2, 1 2, 1 1))")
        .unwrap();
    assert!(b.starts_with("MULTILINESTRING"), "got {b}");
    let b = parse_wkt_literal(&b).unwrap();
    let rings = parse_wkt_literal(
        "MULTILINESTRING((0 0, 4 0, 4 4, 0 4, 0 0), (1 1, 2 1, 2 2, 1 2, 1 1))",
    )
    .unwrap();
    assert!(geof::sf_equals(&b, &rings).unwrap());

    // Boundary of an open linestring: its two endpoints; of a closed one: empty.
    let b = lex::boundary("LINESTRING(0 0, 1 1, 2 0)").unwrap();
    let b = parse_wkt_literal(&b).unwrap();
    assert!(geof::sf_equals(&b, &parse_wkt_literal("MULTIPOINT(0 0, 2 0)").unwrap()).unwrap());
    let closed = lex::boundary("LINESTRING(0 0, 1 1, 2 0, 0 0)").unwrap();
    assert!(closed.contains("EMPTY"), "got {closed}");

    // Boundary of a point: empty.
    assert!(lex::boundary("POINT(1 2)").unwrap().contains("EMPTY"));

    // Convex hull of a concave polygon: the dent disappears.
    let hull = lex::convex_hull("POLYGON((0 0, 4 0, 4 4, 2 1, 0 4, 0 0))").unwrap();
    let hull = parse_wkt_literal(&hull).unwrap();
    let square = parse_wkt_literal("POLYGON((0 0, 4 0, 4 4, 0 4, 0 0))").unwrap();
    assert!(geof::sf_equals(&hull, &square).unwrap(), "got {}", hull.to_wkt_literal());
}

// ---- geof:relate + Egenhofer / RCC8 ------------------------------------------------

#[test]
fn relate_generic_de9im_patterns() {
    let poly = "POLYGON((0 0, 4 0, 4 4, 0 4, 0 0))";
    // sfWithin's pattern, evaluated generically.
    assert!(lex::relate("POINT(1 1)", poly, "T*F**F***").unwrap());
    assert!(!lex::relate("POINT(9 9)", poly, "T*F**F***").unwrap());
    // Dimension digits: point-in-polygon interior intersection has dimension 0.
    assert!(lex::relate("POINT(1 1)", poly, "0*F**F***").unwrap());
    assert!(!lex::relate("POINT(1 1)", poly, "2*F**F***").unwrap());
    // Malformed patterns are reported as parse errors.
    assert!(matches!(lex::relate("POINT(1 1)", poly, "TTT"), Err(GeoError::Parse(_))));
    assert!(matches!(lex::relate("POINT(1 1)", poly, "XXXXXXXXX"), Err(GeoError::Parse(_))));
    // CRS compatibility is enforced like every other relation.
    let bng = "<http://www.opengis.net/def/crs/EPSG/0/27700> POINT(0 0)";
    assert!(matches!(lex::relate(bng, poly, "T*F**F***"), Err(GeoError::CrsMismatch(_, _))));
}

/// Region pairs and which Egenhofer/RCC8 relation holds for each — both
/// families partition region-region space, so exactly ONE relation per family
/// must be true for each pair.
#[test]
fn egenhofer_and_rcc8_partition_region_pairs() {
    const SMALL: &str = "POLYGON((1 1, 2 1, 2 2, 1 2, 1 1))";
    const TANGENT: &str = "POLYGON((0 0, 1 0, 1 1, 0 1, 0 0))";
    const BIG: &str = "POLYGON((0 0, 4 0, 4 4, 0 4, 0 0))";
    const ADJACENT: &str = "POLYGON((4 0, 6 0, 6 2, 4 2, 4 0))";
    const FAR: &str = "POLYGON((9 9, 10 9, 10 10, 9 10, 9 9))";
    const OVERLAP: &str = "POLYGON((3 3, 6 3, 6 6, 3 6, 3 3))";

    type Rel = fn(&str, &str) -> Result<bool, GeoError>;
    let egenhofer: [(&str, Rel); 8] = [
        ("ehEquals", lex::eh_equals),
        ("ehDisjoint", lex::eh_disjoint),
        ("ehMeet", lex::eh_meet),
        ("ehOverlap", lex::eh_overlap),
        ("ehCovers", lex::eh_covers),
        ("ehCoveredBy", lex::eh_covered_by),
        ("ehInside", lex::eh_inside),
        ("ehContains", lex::eh_contains),
    ];
    let rcc8: [(&str, Rel); 8] = [
        ("rcc8eq", lex::rcc8_eq),
        ("rcc8dc", lex::rcc8_dc),
        ("rcc8ec", lex::rcc8_ec),
        ("rcc8po", lex::rcc8_po),
        ("rcc8tppi", lex::rcc8_tppi),
        ("rcc8tpp", lex::rcc8_tpp),
        ("rcc8ntpp", lex::rcc8_ntpp),
        ("rcc8ntppi", lex::rcc8_ntppi),
    ];

    // (a, b, the-one-true-egenhofer, the-one-true-rcc8)
    for (a, b, eh_want, rcc8_want) in [
        (BIG, BIG, "ehEquals", "rcc8eq"),
        (BIG, FAR, "ehDisjoint", "rcc8dc"),
        (BIG, ADJACENT, "ehMeet", "rcc8ec"),
        (BIG, OVERLAP, "ehOverlap", "rcc8po"),
        (SMALL, BIG, "ehInside", "rcc8ntpp"),
        (BIG, SMALL, "ehContains", "rcc8ntppi"),
        (TANGENT, BIG, "ehCoveredBy", "rcc8tpp"),
        (BIG, TANGENT, "ehCovers", "rcc8tppi"),
    ] {
        for (name, f) in &egenhofer {
            assert_eq!(f(a, b).unwrap(), name == &eh_want, "{name}({a}, {b})");
        }
        for (name, f) in &rcc8 {
            assert_eq!(f(a, b).unwrap(), name == &rcc8_want, "{name}({a}, {b})");
        }
    }
}

#[test]
fn egenhofer_and_rcc8_inverse_pairs_are_consistent() {
    const PAIRS: &[(&str, &str)] = &[
        ("POLYGON((1 1, 2 1, 2 2, 1 2, 1 1))", "POLYGON((0 0, 4 0, 4 4, 0 4, 0 0))"),
        ("POLYGON((0 0, 1 0, 1 1, 0 1, 0 0))", "POLYGON((0 0, 4 0, 4 4, 0 4, 0 0))"),
        ("POLYGON((0 0, 2 0, 2 2, 0 2, 0 0))", "POLYGON((1 1, 3 1, 3 3, 1 3, 1 1))"),
    ];
    for (a, b) in PAIRS {
        assert_eq!(lex::eh_inside(a, b).unwrap(), lex::eh_contains(b, a).unwrap());
        assert_eq!(lex::eh_covered_by(a, b).unwrap(), lex::eh_covers(b, a).unwrap());
        assert_eq!(lex::rcc8_tpp(a, b).unwrap(), lex::rcc8_tppi(b, a).unwrap());
        assert_eq!(lex::rcc8_ntpp(a, b).unwrap(), lex::rcc8_ntppi(b, a).unwrap());
    }
}

// ---- getSRID ----------------------------------------------------------------------

#[test]
fn get_srid_returns_the_crs_iri() {
    assert_eq!(lex::get_srid("POINT(0 0)").unwrap(), "http://www.opengis.net/def/crs/OGC/1.3/CRS84");
    assert_eq!(
        lex::get_srid("<http://www.opengis.net/def/crs/EPSG/0/4326> POINT(51.5 -0.13)").unwrap(),
        "http://www.opengis.net/def/crs/EPSG/0/4326"
    );
    assert_eq!(
        lex::get_srid("<http://www.opengis.net/def/crs/EPSG/0/27700> POINT(530000 180000)").unwrap(),
        "http://www.opengis.net/def/crs/EPSG/0/27700"
    );
    assert!(lex::get_srid("PONT(0 0)").is_err());
}

// ---- Set operations (polygonal subset) ---------------------------------------------

#[test]
fn set_operations_on_polygons() {
    let a = "POLYGON((0 0, 2 0, 2 2, 0 2, 0 0))";
    let b = "POLYGON((1 1, 3 1, 3 3, 1 3, 1 1))";

    // intersection: the shared unit square.
    let i = parse_wkt_literal(&lex::intersection(a, b).unwrap()).unwrap();
    let unit = parse_wkt_literal("POLYGON((1 1, 2 1, 2 2, 1 2, 1 1))").unwrap();
    assert!(geof::sf_equals(&i, &unit).unwrap(), "got {}", i.to_wkt_literal());

    // union of edge-adjacent squares: the 2x1 rectangle.
    let u = parse_wkt_literal(
        &lex::union("POLYGON((0 0, 1 0, 1 1, 0 1, 0 0))", "POLYGON((1 0, 2 0, 2 1, 1 1, 1 0))")
            .unwrap(),
    )
    .unwrap();
    let rect = parse_wkt_literal("POLYGON((0 0, 2 0, 2 1, 0 1, 0 0))").unwrap();
    assert!(geof::sf_equals(&u, &rect).unwrap(), "got {}", u.to_wkt_literal());

    // difference: the L-shape.
    let d = parse_wkt_literal(&lex::difference(a, b).unwrap()).unwrap();
    let ell = parse_wkt_literal("POLYGON((0 0, 2 0, 2 1, 1 1, 1 2, 0 2, 0 0))").unwrap();
    assert!(geof::sf_equals(&d, &ell).unwrap(), "got {}", d.to_wkt_literal());

    // symDifference of identical operands: empty.
    assert!(lex::sym_difference(a, a).unwrap().contains("EMPTY"));

    // symDifference == union minus intersection on this pair.
    let sym = parse_wkt_literal(&lex::sym_difference(a, b).unwrap()).unwrap();
    let umi = parse_wkt_literal(
        &lex::difference(&lex::union(a, b).unwrap(), &lex::intersection(a, b).unwrap()).unwrap(),
    )
    .unwrap();
    assert!(geof::sf_equals(&sym, &umi).unwrap());
}

#[test]
fn set_operations_crs_mismatch_still_errors() {
    let poly = "POLYGON((0 0, 2 0, 2 2, 0 2, 0 0))";
    let bng = "<http://www.opengis.net/def/crs/EPSG/0/27700> POLYGON((0 0, 1 0, 1 1, 0 1, 0 0))";
    assert!(matches!(lex::difference(poly, bng), Err(GeoError::CrsMismatch(_, _))));
    // MULTIPOLYGON operands work; result CRS follows the operands.
    let mp = "<http://www.opengis.net/def/crs/EPSG/0/27700> MULTIPOLYGON(((0 0, 2 0, 2 2, 0 2, 0 0)))";
    let r = lex::intersection(mp, bng).unwrap();
    assert!(r.starts_with("<http://www.opengis.net/def/crs/EPSG/0/27700>"), "got {r}");
}

// ---- Set operations: line / point operands (sq-gn3) --------------------------------

#[test]
fn intersection_point_in_and_out_of_polygon() {
    let poly = "POLYGON((0 0, 4 0, 4 4, 0 4, 0 0))";
    // Inside: the point survives (as a MULTIPOINT containing it).
    let inside = parse_wkt_literal(&lex::intersection("POINT(2 2)", poly).unwrap()).unwrap();
    assert!(geof::sf_equals(&inside, &parse_wkt_literal("POINT(2 2)").unwrap()).unwrap());
    // On the boundary: still kept.
    let onb = lex::intersection("POINT(4 2)", poly).unwrap();
    assert!(onb.contains("MULTIPOINT") && onb.contains('4'), "got {onb}");
    // Outside: empty result.
    let outside = lex::intersection("POINT(9 9)", poly).unwrap();
    assert!(outside.contains("EMPTY") || outside.contains("MULTIPOINT()"), "got {outside}");
    // Operand order is symmetric.
    let flipped = lex::intersection(poly, "POINT(2 2)").unwrap();
    assert!(flipped.contains('2'), "got {flipped}");
}

#[test]
fn intersection_polygon_clips_line() {
    // A horizontal line from x=-1 to x=5 at y=2 across the unit-4 square: the
    // clipped portion is the segment from (0,2) to (4,2).
    let poly = "POLYGON((0 0, 4 0, 4 4, 0 4, 0 0))";
    let line = "LINESTRING(-1 2, 5 2)";
    let clipped = parse_wkt_literal(&lex::intersection(line, poly).unwrap()).unwrap();
    let expected = parse_wkt_literal("LINESTRING(0 2, 4 2)").unwrap();
    assert!(geof::sf_equals(&clipped, &expected).unwrap(), "got {}", clipped.to_wkt_literal());
    // A line entirely inside is returned whole.
    let inside = parse_wkt_literal(&lex::intersection("LINESTRING(1 1, 3 3)", poly).unwrap()).unwrap();
    assert!(geof::sf_equals(&inside, &parse_wkt_literal("LINESTRING(1 1, 3 3)").unwrap()).unwrap());
    // A line entirely outside clips to empty.
    let outside = lex::intersection("LINESTRING(5 5, 6 6)", poly).unwrap();
    assert!(outside.contains("EMPTY") || outside.contains("()"), "got {outside}");
}

#[test]
fn intersection_line_grazing_polygon_vertex_is_a_multipoint() {
    // A line that only TOUCHES the polygon at an isolated corner vertex (with no
    // in-polygon span): the intersection is that single point as a MULTIPOINT.
    // `i_overlay`'s string-line clip drops zero-length pieces, so this exercises
    // the isolated-touch-point fallback preserved through the migration.
    let poly = "POLYGON((0 0, 4 0, 4 4, 0 4, 0 0))";
    // A line whose ENDPOINT lands on the (0,0) corner, otherwise heading
    // below-left (outside): no in-polygon span, just the isolated touch vertex.
    let grazing = lex::intersection("LINESTRING(0 0, -1 -1)", poly).unwrap();
    assert!(grazing.contains("MULTIPOINT") && grazing.contains('0'), "got {grazing}");
    let pt = parse_wkt_literal(&grazing).unwrap();
    assert!(
        geof::sf_equals(&pt, &parse_wkt_literal("MULTIPOINT(0 0)").unwrap()).unwrap(),
        "got {grazing}"
    );
}

#[test]
fn intersection_line_crosses_line() {
    // Two crossing diagonals of the unit square meet at the centre (0.5, 0.5).
    let r = parse_wkt_literal(
        &lex::intersection("LINESTRING(0 0, 1 1)", "LINESTRING(0 1, 1 0)").unwrap(),
    )
    .unwrap();
    let centre = parse_wkt_literal("POINT(0.5 0.5)").unwrap();
    assert!(geof::sf_equals(&r, &centre).unwrap(), "got {}", r.to_wkt_literal());
    // Parallel non-touching lines: empty intersection.
    let none = lex::intersection("LINESTRING(0 0, 1 0)", "LINESTRING(0 1, 1 1)").unwrap();
    assert!(none.contains("EMPTY") || none.contains("()"), "got {none}");
    // Collinear overlap: the shared sub-segment is a LINESTRING.
    let overlap = parse_wkt_literal(
        &lex::intersection("LINESTRING(0 0, 3 0)", "LINESTRING(1 0, 5 0)").unwrap(),
    )
    .unwrap();
    let shared = parse_wkt_literal("LINESTRING(1 0, 3 0)").unwrap();
    assert!(geof::sf_equals(&overlap, &shared).unwrap(), "got {}", overlap.to_wkt_literal());
}

#[test]
fn union_same_dimension_operands() {
    // point ∪ point -> MULTIPOINT.
    let pts = lex::union("POINT(0 0)", "POINT(1 1)").unwrap();
    assert!(pts.contains("MULTIPOINT"), "got {pts}");
    let parsed = parse_wkt_literal(&pts).unwrap();
    assert!(geof::sf_contains(&parsed, &parse_wkt_literal("POINT(0 0)").unwrap()).unwrap());
    assert!(geof::sf_contains(&parsed, &parse_wkt_literal("POINT(1 1)").unwrap()).unwrap());
    // Coincident points collapse.
    let dup = parse_wkt_literal(&lex::union("POINT(2 2)", "POINT(2 2)").unwrap()).unwrap();
    assert!(geof::sf_equals(&dup, &parse_wkt_literal("POINT(2 2)").unwrap()).unwrap());

    // line ∪ line -> MULTILINESTRING.
    let lines = lex::union("LINESTRING(0 0, 1 1)", "LINESTRING(2 2, 3 3)").unwrap();
    assert!(lines.contains("MULTILINESTRING"), "got {lines}");
    let lparsed = parse_wkt_literal(&lines).unwrap();
    assert!(geof::sf_intersects(&lparsed, &parse_wkt_literal("LINESTRING(2 2, 3 3)").unwrap()).unwrap());
}

#[test]
fn union_mixed_dimension_is_geometry_collection() {
    let gc = lex::union("POINT(5 5)", "LINESTRING(0 0, 1 1)").unwrap();
    assert!(gc.contains("GEOMETRYCOLLECTION"), "got {gc}");
    // It round-trips through the parser and contains both operands.
    let parsed = parse_wkt_literal(&gc).unwrap();
    assert!(geof::sf_intersects(&parsed, &parse_wkt_literal("POINT(5 5)").unwrap()).unwrap());
    assert!(geof::sf_intersects(&parsed, &parse_wkt_literal("LINESTRING(0 0, 1 1)").unwrap()).unwrap());
}

#[test]
fn difference_and_symdifference_point_operands() {
    // point − polygon: keep only the points NOT on the polygon.
    let poly = "POLYGON((0 0, 4 0, 4 4, 0 4, 0 0))";
    let mp = "MULTIPOINT(2 2, 9 9)"; // (2,2) inside, (9,9) outside
    let d = parse_wkt_literal(&lex::difference(mp, poly).unwrap()).unwrap();
    assert!(geof::sf_equals(&d, &parse_wkt_literal("POINT(9 9)").unwrap()).unwrap(), "got {}", d.to_wkt_literal());

    // point ∆ point: symmetric difference of the coordinate sets.
    let sym = parse_wkt_literal(
        &lex::sym_difference("MULTIPOINT(0 0, 1 1)", "MULTIPOINT(1 1, 2 2)").unwrap(),
    )
    .unwrap();
    let expected = parse_wkt_literal("MULTIPOINT(0 0, 2 2)").unwrap();
    assert!(geof::sf_equals(&sym, &expected).unwrap(), "got {}", sym.to_wkt_literal());
}

// ---- 1-D set subtraction: line−line / line−polygon / symDifference (sq-fxv3) -------
//
// Now SUPPORTED via i_overlay's string-line clip (line−polygon) plus a
// linear-referencing collinear-overlap subtraction (line−line). Hand-computed
// expected geometries below; compared with geof::sf_equals (topological
// equality, so vertex order / multi-wrapping don't matter).

/// Assert a wktLiteral result is topologically equal to `expected`.
fn assert_geo_eq(got_lit: &str, expected: &str) {
    let got = parse_wkt_literal(got_lit).unwrap();
    let exp = parse_wkt_literal(expected).unwrap();
    assert!(
        geof::sf_equals(&got, &exp).unwrap(),
        "got {}, expected {expected}",
        got.to_wkt_literal()
    );
}

#[test]
fn difference_line_minus_line() {
    // Collinear partial overlap: a=[0,2] minus b covering [1,2] -> [0,1].
    assert_geo_eq(
        &lex::difference("LINESTRING(0 0, 2 0)", "LINESTRING(1 0, 3 0)").unwrap(),
        "LINESTRING(0 0, 1 0)",
    );
    // b strictly interior to a: a splits into two pieces.
    assert_geo_eq(
        &lex::difference("LINESTRING(0 0, 4 0)", "LINESTRING(1 0, 3 0)").unwrap(),
        "MULTILINESTRING((0 0, 1 0), (3 0, 4 0))",
    );
    // b covers a entirely: empty result.
    let cover = lex::difference("LINESTRING(1 0, 2 0)", "LINESTRING(0 0, 3 0)").unwrap();
    assert!(cover.contains("EMPTY") || cover.contains("()"), "got {cover}");
    // b merely CROSSES a (a point, measure-zero): a unchanged.
    assert_geo_eq(
        &lex::difference("LINESTRING(0 0, 2 0)", "LINESTRING(1 -1, 1 1)").unwrap(),
        "LINESTRING(0 0, 2 0)",
    );
    // Disjoint b: a unchanged.
    assert_geo_eq(
        &lex::difference("LINESTRING(0 0, 2 0)", "LINESTRING(5 5, 6 6)").unwrap(),
        "LINESTRING(0 0, 2 0)",
    );
}

#[test]
fn difference_line_minus_polygon() {
    let sq = "POLYGON((0 0, 4 0, 4 4, 0 4, 0 0))";
    // A horizontal line crossing the square: only the two outside spans survive.
    assert_geo_eq(
        &lex::difference("LINESTRING(-1 2, 5 2)", sq).unwrap(),
        "MULTILINESTRING((-1 2, 0 2), (4 2, 5 2))",
    );
    // Line entirely inside the polygon: empty.
    let inside = lex::difference("LINESTRING(1 1, 3 3)", sq).unwrap();
    assert!(inside.contains("EMPTY") || inside.contains("()"), "got {inside}");
    // Line entirely outside: whole line back.
    assert_geo_eq(&lex::difference("LINESTRING(5 5, 6 6)", sq).unwrap(), "LINESTRING(5 5, 6 6)");
    // Polygon WITH A HOLE: the hole interior counts as OUTSIDE the polygon, so
    // a line crossing both the outer ring and the hole keeps 3 spans.
    let holed = "POLYGON((0 0, 6 0, 6 6, 0 6, 0 0), (2 2, 4 2, 4 4, 2 4, 2 2))";
    assert_geo_eq(
        &lex::difference("LINESTRING(-1 3, 7 3)", holed).unwrap(),
        "MULTILINESTRING((-1 3, 0 3), (2 3, 4 3), (6 3, 7 3))",
    );
    // Difference and intersection PARTITION the original line (complementary).
    let inter = lex::intersection("LINESTRING(-1 3, 7 3)", holed).unwrap();
    assert_geo_eq(&inter, "MULTILINESTRING((0 3, 2 3), (4 3, 6 3))");
}

#[test]
fn difference_line_along_polygon_boundary_excludes_the_boundary_span() {
    // A line running exactly ALONG the polygon's top edge (y=4). The polygon is
    // a CLOSED set, so the boundary span belongs to the polygon: it is REMOVED
    // from line−polygon (only the two outside overhangs survive) and KEPT in
    // line∩polygon — the two operations remain a clean partition.
    let sq = "POLYGON((0 0, 4 0, 4 4, 0 4, 0 0))";
    let along = "LINESTRING(-1 4, 5 4)";
    assert_geo_eq(
        &lex::difference(along, sq).unwrap(),
        "MULTILINESTRING((-1 4, 0 4), (4 4, 5 4))",
    );
    assert_geo_eq(&lex::intersection(along, sq).unwrap(), "LINESTRING(0 4, 4 4)");
}

#[test]
fn difference_polygon_minus_line_or_point_is_unchanged() {
    // Subtracting a measure-zero 1-D / 0-D set from a 2-D surface leaves it intact.
    let sq = "POLYGON((0 0, 4 0, 4 4, 0 4, 0 0))";
    assert_geo_eq(&lex::difference(sq, "LINESTRING(-1 2, 5 2)").unwrap(), sq);
    assert_geo_eq(&lex::difference(sq, "POINT(2 2)").unwrap(), sq);
    // …and line − point.
    assert_geo_eq(
        &lex::difference("LINESTRING(0 0, 2 0)", "POINT(1 0)").unwrap(),
        "LINESTRING(0 0, 2 0)",
    );
}

#[test]
fn sym_difference_line_cases() {
    // line ∆ line = (a−b) ∪ (b−a): the non-shared portions of both curves.
    assert_geo_eq(
        &lex::sym_difference("LINESTRING(0 0, 2 0)", "LINESTRING(1 0, 3 0)").unwrap(),
        "MULTILINESTRING((0 0, 1 0), (2 0, 3 0))",
    );
    // Identical lines: empty symmetric difference.
    let same = lex::sym_difference("LINESTRING(0 0, 2 0)", "LINESTRING(0 0, 2 0)").unwrap();
    assert!(same.contains("EMPTY") || same.contains("()"), "got {same}");
    // line ∆ polygon = (line outside polygon) ∪ polygon -> a GEOMETRYCOLLECTION
    // (polygon − line is the polygon unchanged).
    let sq = "POLYGON((0 0, 4 0, 4 4, 0 4, 0 0))";
    let sym = lex::sym_difference("LINESTRING(-1 2, 5 2)", sq).unwrap();
    assert!(sym.contains("GEOMETRYCOLLECTION"), "got {sym}");
    let parsed = parse_wkt_literal(&sym).unwrap();
    // Contains the polygon and the outside line span.
    assert!(geof::sf_intersects(&parsed, &parse_wkt_literal(sq).unwrap()).unwrap());
    assert!(geof::sf_intersects(&parsed, &parse_wkt_literal("LINESTRING(-1 2, -0.5 2)").unwrap())
        .unwrap());
    // Operand order is symmetric (polygon ∆ line == line ∆ polygon).
    let flipped = lex::sym_difference(sq, "LINESTRING(-1 2, 5 2)").unwrap();
    assert!(flipped.contains("GEOMETRYCOLLECTION"), "got {flipped}");
}

#[test]
fn line_symdifference_recombines_from_the_two_differences() {
    // By definition a∆b = (a−b) ∪ (b−a); cross-check on collinear lines.
    let a = "LINESTRING(0 0, 3 0)";
    let b = "LINESTRING(1 0, 5 0)";
    let amb = lex::difference(a, b).unwrap(); // (0,0)-(1,0)
    let bma = lex::difference(b, a).unwrap(); // (3,0)-(5,0)
    let sym = lex::sym_difference(a, b).unwrap();
    let recombined = parse_wkt_literal(&lex::union(&amb, &bma).unwrap()).unwrap();
    assert!(
        geof::sf_equals(&recombined, &parse_wkt_literal(&sym).unwrap()).unwrap(),
        "sym {sym} != union of diffs"
    );
}

#[test]
fn intersection_of_unequal_points_is_empty_not_error() {
    // Intersection of two points that are NOT equal: empty, not an error.
    let empty = lex::intersection("POINT(0 0)", "POINT(1 1)").unwrap();
    assert!(empty.contains("EMPTY") || empty.contains("()"), "got {empty}");
}

// ---- geof:buffer -------------------------------------------------------------------

#[test]
fn buffer_metric_approximates_a_great_circle_disc() {
    // 100 km around a point at 51°N: a point 95 km north is inside, 105 km
    // north is outside (1 km of slack for the polygonal circle approximation).
    let buf = lex::buffer("POINT(0 51)", 100.0, UOM_KM).unwrap();
    assert!(buf.contains("MULTIPOLYGON"), "got {buf}");
    let dlat_95 = 95_000.0 / 111_194.93;
    let dlat_105 = 105_000.0 / 111_194.93;
    assert!(lex::sf_within(&format!("POINT(0 {})", 51.0 + dlat_95), &buf).unwrap());
    assert!(!lex::sf_within(&format!("POINT(0 {})", 51.0 + dlat_105), &buf).unwrap());
    // …and the same east-west, where degrees are shorter (cos 51° ≈ 0.6293):
    // the local projection must keep the disc round in METRES, not degrees.
    let dlon_95 = 95_000.0 / (111_194.93 * (51.0f64).to_radians().cos());
    let dlon_105 = 105_000.0 / (111_194.93 * (51.0f64).to_radians().cos());
    assert!(lex::sf_within(&format!("POINT({dlon_95} 51)", ), &buf).unwrap());
    assert!(!lex::sf_within(&format!("POINT({dlon_105} 51)"), &buf).unwrap());
}

#[test]
fn buffer_grows_polygons_and_respects_units() {
    // A point 0.5° outside the polygon falls inside its 1°-buffer.
    let poly = "POLYGON((0 0, 2 0, 2 2, 0 2, 0 0))";
    let buf = lex::buffer(poly, 1.0, UOM_DEGREE).unwrap();
    assert!(lex::sf_within("POINT(2.5 1)", &buf).unwrap());
    assert!(!lex::sf_within("POINT(3.5 1)", &buf).unwrap());
    // Radian radius: same buffer expressed in radians.
    let buf_rad = lex::buffer(poly, 1.0f64.to_radians(), UOM_RADIAN).unwrap();
    assert!(lex::sf_within("POINT(2.5 1)", &buf_rad).unwrap());
    // Degree-unit buffering works in raw coordinate space for projected CRSs…
    let bng = "<http://www.opengis.net/def/crs/EPSG/0/27700> POINT(530000 180000)";
    let buf_bng = lex::buffer(bng, 100.0, UOM_DEGREE).unwrap();
    assert!(buf_bng.starts_with("<http://www.opengis.net/def/crs/EPSG/0/27700>"), "got {buf_bng}");
    // …but METRIC buffering refuses them (no projection support).
    assert!(matches!(lex::buffer(bng, 100.0, UOM_METRE), Err(GeoError::NonGeographicCrs(_))));
    // Empty geometries cannot be buffered metrically (no anchor latitude).
    assert!(matches!(lex::buffer("POINT EMPTY", 1.0, UOM_METRE), Err(GeoError::Unsupported(_))));
}

#[test]
fn unit_iri_aliases() {
    assert_eq!(Unit::from_iri("http://qudt.org/vocab/unit/M").unwrap(), Unit::Metre);
    assert_eq!(Unit::from_iri("http://qudt.org/vocab/unit/KiloM").unwrap(), Unit::Kilometre);
    assert_eq!(Unit::from_iri("http://qudt.org/vocab/unit/DEG").unwrap(), Unit::Degree);
    assert_eq!(Unit::from_iri(UOM_RADIAN).unwrap(), Unit::Radian);
    assert!(Unit::from_iri("metre").is_err());
}
