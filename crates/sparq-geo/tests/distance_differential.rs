//! sq-lk3aw.3: Differential tests for `geof::distance_meters` with extended
//! (non-point) geometries, verifying the vertex-HaversineClosestPoint approach against
//! two complementary oracles:
//!
//! 1. **Equirectangular regression test** — two parallel vertical linestrings spanning
//!    10°N–50°N, where the equirectangular approximation (single mean-latitude cos factor)
//!    overestimates the minimum distance by ~35 % compared to the haversine oracle at the
//!    northernmost (cosine-minimising) latitude. The new approach finds the correct answer.
//!
//! 2. **Continent-spanning Geodesic oracle** — two short linestrings near London/Heathrow
//!    and New York/JFK, cross-checked against `geo::Geodesic.distance` (WGS84 Vincenty/
//!    Karney exact ellipsoid) on the nearest vertex pair. Haversine-sphere vs WGS84
//!    differs by ≤ 0.3 %; the vertex spacing adds a further ≤ ~0.3 % for these ~15-km
//!    segments over a ~5 540-km baseline. Total documented bound: ≤ 1 %.
//!
//! These tests exercise the REAL `distance_meters` code path (not a mock) and assert the
//! load-bearing invariant (result-equivalence vs the documented oracle within the stated
//! bound). Point-to-point Haversine stays unchanged — both tests only involve extended
//! (non-point) geometry arguments.

use geo::{Distance, Geodesic, Haversine};
use geo_types::{Coord, Geometry, LineString, Point};
use sparq_geo::geof;

/// Build a `Geometry::LineString` from `(longitude, latitude)` pairs.
fn linestring(coords: &[(f64, f64)]) -> Geometry<f64> {
    Geometry::LineString(LineString(
        coords.iter().map(|&(x, y)| Coord { x, y }).collect(),
    ))
}

/// Haversine point-to-point distance (metres) between two long/lat positions.
fn haversine_m(lon1: f64, lat1: f64, lon2: f64, lat2: f64) -> f64 {
    Haversine.distance(Point::new(lon1, lat1), Point::new(lon2, lat2))
}

/// Geodesic (WGS84 Vincenty/Karney) point-to-point distance (metres).
fn geodesic_m(lon1: f64, lat1: f64, lon2: f64, lat2: f64) -> f64 {
    Geodesic.distance(Point::new(lon1, lat1), Point::new(lon2, lat2))
}

// ── Test 1: equirectangular regression ──────────────────────────────────────────────

/// Two parallel vertical linestrings at 0°E and 5°E spanning 10°N–50°N.
///
/// True minimum haversine distance: (0°E, 50°N) ↔ (5°E, 50°N) — the northernmost row
/// minimises the zonal haversine (cos factor decreasing away from the equator).
///
/// The OLD equirectangular approach used the mean of the two bounding-box centres
/// (both at 30°N), giving cos(30°) ≈ 0.866 instead of cos(50°) ≈ 0.643 — an
/// overestimate of ~35 %. The new vertex-HaversineClosestPoint approach correctly
/// finds the northernmost vertex pair and returns a result within 0.5 % of the
/// haversine oracle.
#[test]
fn extended_distance_resolves_equirectangular_distortion() {
    // Line A: vertical at 0°E, 10°N to 50°N.
    let a = linestring(&[(0.0, 10.0), (0.0, 50.0)]);
    // Line B: vertical at 5°E, 10°N to 50°N (same latitude range, different longitude).
    let b = linestring(&[(5.0, 10.0), (5.0, 50.0)]);

    let computed = geof::distance_meters(&a, &b)
        .expect("distance_meters must succeed for non-empty linestrings");

    // Oracle: haversine between the northernmost endpoints (0°E, 50°N) and (5°E, 50°N)
    // — the minimum for this symmetric configuration.
    let oracle = haversine_m(0.0, 50.0, 5.0, 50.0);

    // Vertex-HaversineClosestPoint must match the haversine oracle to within 0.5 %
    // (spherical-sphere vs spherical is exact here; only FP rounding applies).
    let rel_err = (computed - oracle).abs() / oracle;
    assert!(
        rel_err < 0.005,
        "vertex-haversine result {:.0} m vs oracle {:.0} m: {:.3} % error (expected < 0.5 %)",
        computed,
        oracle,
        rel_err * 100.0,
    );

    // Cross-check: the haversine at the SOUTHERNMOST latitude is substantially larger
    // (cos(10°) ≈ 0.985 → wider zones) so the true minimum is NOT at 10°N.
    let southern_pair = haversine_m(0.0, 10.0, 5.0, 10.0);
    assert!(
        oracle < southern_pair,
        "northernmost pair ({:.0} m) should be shorter than southernmost pair ({:.0} m)",
        oracle,
        southern_pair,
    );
}

// ── Test 2: continent-spanning Geodesic oracle ──────────────────────────────────────

/// Short (≈ 15-km) E-W linestrings near London/Heathrow and New York/JFK.
///
/// The oracle is computed robustly as the geodesic (WGS84 Karney) minimum over **all
/// four** vertex pairs; an explicit precondition assertion verifies which pair is the
/// nearest for this fixture, so the test fails loudly if the geometry is ever changed.
///
/// For this specific fixture the nearest vertex pair is **west London → east NYC**:
/// the west London endpoint (−0.50°) has a smaller longitude gap to east NYC (−73.75°)
/// of 73.25°, versus east London (−0.30°) → east NYC at 73.45°. Great-circle distance
/// grows with longitude gap at these latitudes, so the smaller-gap pair wins.
///
/// Documented bound ≤ 1 %:
/// - Haversine-sphere vs WGS84-ellipsoid: ≤ 0.3 %
/// - Nearest vertex vs true-minimum interior point: ≤ segment-arc / baseline
///   ≈ 15 km / 5 540 km ≈ 0.3 %
#[test]
fn extended_distance_continent_spanning_geodesic_oracle() {
    // Short E-W linestring near London Heathrow (≈ 15 km, long/lat).
    let london = linestring(&[(-0.50, 51.48), (-0.30, 51.48)]);
    // Short E-W linestring near New York JFK (≈ 21 km, long/lat).
    let nyc = linestring(&[(-74.00, 40.64), (-73.75, 40.64)]);

    let computed = geof::distance_meters(&london, &nyc)
        .expect("distance_meters must succeed for non-empty linestrings");

    // Robust oracle: geodesic (WGS84 Karney) minimum over all 4 vertex–vertex pairs.
    // Computing the minimum over all combinations avoids a fragile hard-coded pair
    // assumption — if the fixture geometry ever shifts so a different pair becomes
    // nearest, the oracle adapts automatically.
    let (lon_w, lat_lon) = (-0.50_f64, 51.48_f64);
    let (lon_e, _) = (-0.30_f64, 51.48_f64);
    let (nyc_w, lat_nyc) = (-74.00_f64, 40.64_f64);
    let (nyc_e, _) = (-73.75_f64, 40.64_f64);
    let pair_dists = [
        geodesic_m(lon_w, lat_lon, nyc_w, lat_nyc),
        geodesic_m(lon_w, lat_lon, nyc_e, lat_nyc),
        geodesic_m(lon_e, lat_lon, nyc_w, lat_nyc),
        geodesic_m(lon_e, lat_lon, nyc_e, lat_nyc),
    ];
    let oracle = pair_dists.iter().copied().fold(f64::INFINITY, f64::min);

    // Precondition: verify that west London → east NYC (pair index 1) is the nearest
    // vertex pair for this fixture. If this assertion fires, the fixture has changed
    // and the pair index and comment in this function need updating.
    assert!(
        pair_dists[1] <= pair_dists[0]
            && pair_dists[1] <= pair_dists[2]
            && pair_dists[1] <= pair_dists[3],
        "fixture precondition: west-London→east-NYC ({:.0} m) should be the nearest vertex \
         pair (others: {:.0}, {:.0}, {:.0} m)",
        pair_dists[1],
        pair_dists[0],
        pair_dists[2],
        pair_dists[3],
    );

    // Bound: ≤ 1 % (Haversine-sphere + vertex-spacing effects, see doc above).
    let rel_err = (computed - oracle).abs() / oracle;
    assert!(
        rel_err < 0.01,
        "continent-spanning: computed {:.0} m vs geodesic oracle {:.0} m: {:.3} % (bound ≤ 1 %)",
        computed,
        oracle,
        rel_err * 100.0,
    );

    // The result should be in a plausible range (roughly 5 000 – 6 000 km).
    assert!(
        computed > 5_000_000.0 && computed < 6_100_000.0,
        "London-NYC computed distance {:.0} m is outside the expected 5 000–6 100 km range",
        computed,
    );
}

// ── Test 3: Haversine point-to-point path unchanged ─────────────────────────────────

/// Verify that the point↔point path through `geof::distance_meters` is UNCHANGED:
/// a `Geometry::Point` triggers `point_to_geometry_meters` which uses the exact
/// Haversine path, not the extended-extended iteration.
#[test]
fn point_to_point_haversine_path_unchanged() {
    // London ↔ Paris (well-known ~343.6 km pair).
    let london = Geometry::Point(Point::new(-0.1278, 51.5074));
    let paris = Geometry::Point(Point::new(2.3522, 48.8566));

    let d = geof::distance_meters(&london, &paris)
        .expect("point-to-point haversine must succeed");

    // Haversine oracle for the same pair.
    let oracle = haversine_m(-0.1278, 51.5074, 2.3522, 48.8566);

    assert!(
        (d - oracle).abs() < 1.0,
        "point-to-point: {:.0} m vs haversine oracle {:.0} m (must be identical)",
        d,
        oracle,
    );

    // Sanity: ~343 km.
    assert!(
        d > 340_000.0 && d < 350_000.0,
        "London-Paris distance {:.0} m outside 340–350 km range",
        d,
    );
}
