//! Metamorphic laws for the pure Simple Features operations.

use geo_types::{Coord, Geometry, LineString, Point, Polygon};
use proptest::prelude::*;
use sparq_geo::geof::{
    distance, envelope, sf_contains, sf_disjoint, sf_equals, sf_intersects, sf_overlaps, sf_within,
    Unit,
};
use sparq_geo::{Crs, GeoGeometry};

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

fn geo_geometry() -> impl Strategy<Value = GeoGeometry> {
    geometry().prop_map(|geometry| GeoGeometry {
        crs: Crs::Crs84,
        geometry,
    })
}

proptest! {
    #[test]
    fn symmetric_relations_and_negation(
        a in geo_geometry(),
        b in geo_geometry(),
    ) {
        let intersects_ab = sf_intersects(&a, &b)?;
        let intersects_ba = sf_intersects(&b, &a)?;
        let disjoint_ab = sf_disjoint(&a, &b)?;

        prop_assert_eq!(intersects_ab, intersects_ba);
        prop_assert_eq!(disjoint_ab, sf_disjoint(&b, &a)?);
        prop_assert_eq!(sf_equals(&a, &b)?, sf_equals(&b, &a)?);
        prop_assert_eq!(sf_overlaps(&a, &b)?, sf_overlaps(&b, &a)?);
        prop_assert_eq!(disjoint_ab, !intersects_ab);
    }

    #[test]
    fn within_and_contains_are_dual(a in geo_geometry(), b in geo_geometry()) {
        prop_assert_eq!(sf_within(&a, &b)?, sf_contains(&b, &a)?);
        prop_assert_eq!(sf_contains(&a, &b)?, sf_within(&b, &a)?);
    }

    #[test]
    fn equality_and_disjointness_are_reflexive_laws(a in geo_geometry()) {
        prop_assert!(sf_equals(&a, &a)?);
        prop_assert!(!sf_disjoint(&a, &a)?);
    }

    #[test]
    fn distance_is_symmetric_and_non_negative(
        a in geo_geometry(),
        b in geo_geometry(),
    ) {
        let ab = distance(&a, &b, Unit::Degree)?;
        let ba = distance(&b, &a, Unit::Degree)?;

        prop_assert_eq!(ab, ba);
        prop_assert!(ab >= 0.0);
    }

    #[test]
    fn envelope_is_idempotent(g in geo_geometry()) {
        let once = envelope(&g)?;
        prop_assume!(sf_equals(&once, &once)?);
        let twice = envelope(&once)?;

        prop_assert!(sf_equals(&once, &twice)?);
    }
}
