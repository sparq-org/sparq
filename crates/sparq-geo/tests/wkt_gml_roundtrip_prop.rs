//! WKT-to-GML cross-format property coverage for every GML-SF class.

use geo_types::{
    Coord, Geometry, LineString, MultiLineString, MultiPoint, MultiPolygon, Point, Polygon,
};
use proptest::prelude::*;
use sparq_geo::{parse_gml_literal, parse_wkt_literal, Crs, GeoGeometry};

fn coordinate() -> impl Strategy<Value = Coord<f64>> {
    (-10_000i32..=10_000, -10_000i32..=10_000).prop_map(|(x, y)| Coord {
        x: f64::from(x),
        y: f64::from(y),
    })
}

fn linestring() -> impl Strategy<Value = LineString<f64>> {
    prop::collection::vec(coordinate(), 2..=8).prop_map(LineString::new)
}

fn polygon() -> impl Strategy<Value = Polygon<f64>> {
    (
        -10_000i32..=10_000,
        -10_000i32..=10_000,
        4i32..=1_000,
        4i32..=1_000,
    )
        .prop_map(|(x, y, width, height)| {
            let (x, y, width, height) = (
                f64::from(x),
                f64::from(y),
                f64::from(width),
                f64::from(height),
            );
            Polygon::new(
                LineString::from(vec![
                    (x, y),
                    (x + width, y),
                    (x + width, y + height),
                    (x, y + height),
                    (x, y),
                ]),
                vec![],
            )
        })
}

fn crs() -> impl Strategy<Value = Crs> {
    prop_oneof![
        Just(Crs::Crs84),
        Just(Crs::Epsg4326),
        Just(Crs::Other(
            "http://www.opengis.net/def/crs/EPSG/0/27700".to_owned()
        )),
        Just(Crs::Other(
            "http://example.org/crs/local?x=1&y=2".to_owned()
        )),
    ]
}

fn tagged(geometry: impl Strategy<Value = Geometry<f64>>) -> impl Strategy<Value = GeoGeometry> {
    (crs(), geometry).prop_map(|(crs, geometry)| GeoGeometry { crs, geometry })
}

fn assert_cross_format_roundtrip(original: GeoGeometry) -> Result<(), TestCaseError> {
    let wkt = original.to_wkt_literal();
    let from_wkt =
        parse_wkt_literal(&wkt).expect("to_wkt_literal must produce an accepted WKT lexical form");
    let gml = from_wkt.to_gml_literal();
    let reparsed =
        parse_gml_literal(&gml).expect("to_gml_literal must produce an accepted GML lexical form");

    prop_assert_eq!(&reparsed, &from_wkt, "WKT {}; GML {}", wkt, gml);

    // Mutation witness: equality observes coordinates, not merely the geometry
    // class and CRS. Perturbing one expected coordinate must make it unequal.
    let mut perturbed = from_wkt;
    perturb_first_coordinate(&mut perturbed.geometry);
    prop_assert_ne!(reparsed, perturbed);
    Ok(())
}

fn perturb_first_coordinate(geometry: &mut Geometry<f64>) {
    match geometry {
        Geometry::Point(point) => point.0.x += 1.0,
        Geometry::LineString(line) => {
            line.0.first_mut().expect("generated line is non-empty").x += 1.0;
        }
        Geometry::Polygon(polygon) => polygon.exterior_mut(|ring| {
            ring.0
                .first_mut()
                .expect("generated polygon is non-empty")
                .x += 1.0;
        }),
        Geometry::MultiPoint(points) => {
            points.0.first_mut().expect("generated multi-point").0.x += 1.0;
        }
        Geometry::MultiLineString(lines) => {
            lines
                .0
                .first_mut()
                .and_then(|line| line.0.first_mut())
                .expect("generated multi-line string")
                .x += 1.0;
        }
        Geometry::MultiPolygon(polygons) => polygons
            .0
            .first_mut()
            .expect("generated multi-polygon")
            .exterior_mut(|ring| {
                ring.0
                    .first_mut()
                    .expect("generated polygon is non-empty")
                    .x += 1.0;
            }),
        other => panic!("unexpected generated geometry: {other:?}"),
    }
}

proptest! {
    #[test]
    fn point_roundtrip(original in tagged(coordinate().prop_map(|coord| Geometry::Point(Point(coord))))) {
        assert_cross_format_roundtrip(original)?;
    }

    #[test]
    fn linestring_roundtrip(original in tagged(linestring().prop_map(Geometry::LineString))) {
        assert_cross_format_roundtrip(original)?;
    }

    #[test]
    fn polygon_roundtrip(original in tagged(polygon().prop_map(Geometry::Polygon))) {
        assert_cross_format_roundtrip(original)?;
    }

    #[test]
    fn multipoint_roundtrip(original in tagged(
        prop::collection::vec(coordinate(), 1..=8)
            .prop_map(|coords| Geometry::MultiPoint(MultiPoint::from(coords)))
    )) {
        assert_cross_format_roundtrip(original)?;
    }

    #[test]
    fn multilinestring_roundtrip(original in tagged(
        prop::collection::vec(linestring(), 1..=4)
            .prop_map(|lines| Geometry::MultiLineString(MultiLineString::new(lines)))
    )) {
        assert_cross_format_roundtrip(original)?;
    }

    #[test]
    fn multipolygon_roundtrip(original in tagged(
        prop::collection::vec(polygon(), 1..=4)
            .prop_map(|polygons| Geometry::MultiPolygon(MultiPolygon::new(polygons)))
    )) {
        assert_cross_format_roundtrip(original)?;
    }
}

#[test]
fn polygon_holes_and_xml_escaped_crs_roundtrip() {
    let original = parse_wkt_literal(
        "<http://example.org/crs/local?x=1&y=2> POLYGON((0 0, 8 0, 8 8, 0 8, 0 0), \
         (2 2, 4 2, 4 4, 2 4, 2 2))",
    )
    .unwrap();

    let gml = original.to_gml_literal();
    assert!(gml.contains("srsName=\"http://example.org/crs/local?x=1&amp;y=2\""));
    assert!(gml.contains("<gml:interior>"));
    assert_eq!(parse_gml_literal(&gml).unwrap(), original);
}
