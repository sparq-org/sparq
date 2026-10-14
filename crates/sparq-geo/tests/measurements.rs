//! Deterministic, mutation-witnessing oracles for GeoSPARQL metric measurements.

use geo::{Distance, Haversine};
use geo_types::Point;
use sparq_geo::{geof, parse_wkt_literal, Crs, GeoError, GeoGeometry};

fn relative_error(actual: f64, expected: f64) -> f64 {
    (actual - expected).abs() / expected
}

#[test]
fn equatorial_square_area_and_degenerate_ring() {
    // sq-lsp7k.18: the analytic one-degree equatorial square is about
    // 12,300 km². A 1% bound is deliberately wider than the local-frame model's
    // expected error while tight enough to kill degree-space or length returns.
    let square = parse_wkt_literal("POLYGON((0 0, 1 0, 1 1, 0 1, 0 0))").unwrap();
    let expected_square_metres = 12_300_000_000.0;
    let area = geof::metric_area(&square).unwrap();
    assert!(
        relative_error(area, expected_square_metres) < 0.01,
        "area {area} m² differs from analytic oracle"
    );

    let degenerate = parse_wkt_literal("POLYGON((0 0, 1 0, 2 0, 0 0))").unwrap();
    assert_eq!(geof::metric_area(&degenerate).unwrap(), 0.0);

    let point = parse_wkt_literal("POINT(0 0)").unwrap();
    assert!(matches!(
        geof::metric_area(&point),
        Err(GeoError::Unsupported(_))
    ));
}

#[test]
fn line_length_matches_haversine_oracle() {
    let line = parse_wkt_literal("LINESTRING(0 0, 1 0)").unwrap();
    let expected = Haversine.distance(Point::new(0.0, 0.0), Point::new(1.0, 0.0));
    let length = geof::metric_length(&line).unwrap();
    assert!(
        relative_error(length, expected) < 1e-12,
        "length {length}, expected {expected}"
    );

    let point = parse_wkt_literal("POINT(0 0)").unwrap();
    assert!(matches!(
        geof::metric_length(&point),
        Err(GeoError::Unsupported(_))
    ));
}

#[test]
fn square_centroid_and_perimeter_are_orientation_invariant() {
    let counter_clockwise = parse_wkt_literal("POLYGON((0 0, 1 0, 1 1, 0 1, 0 0))").unwrap();
    // Same ring, reversed and cyclically shifted.
    let reordered = parse_wkt_literal("POLYGON((1 1, 1 0, 0 0, 0 1, 1 1))").unwrap();

    let area = geof::metric_area(&counter_clockwise).unwrap();
    let length = geof::metric_length(&counter_clockwise).unwrap();
    let perimeter = geof::metric_perimeter(&counter_clockwise).unwrap();
    assert_eq!(area, geof::metric_area(&reordered).unwrap());
    assert_eq!(length, geof::metric_length(&reordered).unwrap());
    assert_eq!(perimeter, geof::metric_perimeter(&reordered).unwrap());
    assert_eq!(length, perimeter);

    for geometry in [&counter_clockwise, &reordered] {
        let result = geof::centroid(geometry).unwrap();
        let geo_types::Geometry::Point(point) = result.geometry else {
            panic!("centroid must be a point")
        };
        assert!((point.x() - 0.5).abs() < 1e-12);
        assert!((point.y() - 0.5).abs() < 1e-12);
    }
}

#[test]
fn metric_functions_reject_unknown_coordinate_units_but_centroid_preserves_crs() {
    let projected = GeoGeometry {
        crs: Crs::Other("http://example.com/crs/local".to_string()),
        geometry: parse_wkt_literal("LINESTRING(0 0, 2 0)").unwrap().geometry,
    };
    assert!(matches!(
        geof::metric_length(&projected),
        Err(GeoError::NonGeographicCrs(_))
    ));

    let centroid = geof::centroid(&projected).unwrap();
    assert_eq!(centroid.crs, projected.crs);
    assert_eq!(
        centroid.geometry,
        geo_types::Geometry::Point(Point::new(1.0, 0.0))
    );
}
