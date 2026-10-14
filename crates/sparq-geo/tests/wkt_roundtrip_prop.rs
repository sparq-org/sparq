//! Property coverage for the accepted WKT literal domain.

use geo_types::{Coord, Geometry, LineString, Point, Polygon};
use proptest::prelude::*;
use sparq_geo::{parse_wkt_literal, Crs, GeoGeometry};

fn coordinate() -> impl Strategy<Value = Coord<f64>> {
    (-1_000_000.0f64..1_000_000.0, -1_000_000.0f64..1_000_000.0).prop_map(|(x, y)| Coord { x, y })
}

fn geometry() -> impl Strategy<Value = Geometry<f64>> {
    let point = coordinate().prop_map(|c| Geometry::Point(Point(c)));
    let line = prop::collection::vec(coordinate(), 2..=8)
        .prop_map(|coords| Geometry::LineString(LineString::new(coords)));
    let polygon = (
        -1_000_000.0f64..1_000_000.0,
        -1_000_000.0f64..1_000_000.0,
        0.000_001f64..10_000.0,
        0.000_001f64..10_000.0,
    )
        .prop_map(|(x, y, width, height)| {
            Geometry::Polygon(Polygon::new(
                LineString::from(vec![
                    (x, y),
                    (x + width, y),
                    (x + width, y + height),
                    (x, y + height),
                    (x, y),
                ]),
                vec![],
            ))
        });

    prop_oneof![point, line, polygon]
}

fn crs() -> impl Strategy<Value = Crs> {
    prop_oneof![
        Just(Crs::Crs84),
        Just(Crs::Epsg4326),
        Just(Crs::Other(
            "http://www.opengis.net/def/crs/EPSG/0/27700".to_owned()
        )),
        Just(Crs::Other("http://example.org/crs/local".to_owned())),
    ]
}

fn wkt_geometry() -> impl Strategy<Value = GeoGeometry> {
    (crs(), geometry()).prop_map(|(crs, geometry)| GeoGeometry { crs, geometry })
}

proptest! {
    #[test]
    fn wkt_parse_serialize_roundtrip(original in wkt_geometry()) {
        let lexical = original.to_wkt_literal();
        let reparsed = parse_wkt_literal(&lexical)
            .expect("to_wkt_literal must produce an accepted WKT lexical form");

        prop_assert_eq!(reparsed, original);
    }
}
