//! The `geof:` function namespace (GeoSPARQL 1.0 §8.7 / 1.1 §10).
//!
//! Implemented over [`GeoGeometry`] values:
//!
//! - [`distance`] — `geof:distance(geom1, geom2, units)`. Units are selected
//!   by IRI ([`Unit::from_iri`]). Metric units on geographic-CRS geometries
//!   use the haversine great-circle distance (exact for point/point and
//!   point/geometry via the spherical closest point; for two EXTENDED geometries
//!   uses vertex-`HaversineClosestPoint` iteration — see [`distance_meters`]).
//!   sq-lk3aw.3
//! - the eight simple-features relations (`geof:sfEquals`, `sfDisjoint`,
//!   `sfIntersects`, `sfTouches`, `sfCrosses`, `sfWithin`, `sfContains`,
//!   `sfOverlaps`) — DE-9IM intersection matrices via `geo`'s `Relate` — plus
//!   the generic [`relate`] (`geof:relate`, arbitrary DE-9IM pattern) and the
//!   Egenhofer (`geof:eh*`) and RCC8 (`geof:rcc8*`) families (the GeoSPARQL
//!   1.0 Req 25/26 matrix patterns over the same machinery).
//! - [`envelope`] / [`boundary`] / [`convex_hull`] — `geof:envelope`,
//!   `geof:boundary`, `geof:convexHull` — [`simplify`] (`geof:simplify`,
//!   Douglas–Peucker), and [`buffer`] (`geof:buffer`, geo 0.33's `Buffer`;
//!   metric radii via a local equirectangular frame).
//! - [`max_x`] / [`min_x`] / [`max_y`] / [`min_y`] — the bounding-box
//!   coordinates in the geometry's stored CRS.
//! - [`is_empty`] — `geof:isEmpty`, a unit-free geometry predicate.
//! - [`metric_area`] / [`metric_length`] / [`metric_perimeter`] and
//!   [`centroid`] — metric measurements and the mathematical centroid, using
//!   the same local equirectangular frame for geographic geometries.
//! - the set operations [`intersection`] / [`union`] / [`difference`] /
//!   [`sym_difference`] — point-set operations over `geo`'s `BooleanOps`
//!   (polygon overlay) plus directly-implemented line/point cases: point-in/on
//!   tests, line∩line via `geo`'s `line_intersection`, and the 1-D
//!   set-subtraction cases (line−line / line−polygon and their symDifference)
//!   via `i_overlay`'s string-line clip + linear referencing — the gap `geo`'s
//!   polygon-only overlay leaves. Operands the dimension dispatch cannot
//!   classify return an honest [`GeoError::Unsupported`]; see each function's
//!   docs and the README for the supported matrix.
//!
//! The [`lex`] sub-module provides lexical-level WKT-string helpers for direct
//! use. `crate::registry::geof_registry` parses geometry literals and invokes
//! the typed functions in this module, so they run inside SPARQL FILTER/BIND via
//! `sparq_engine::query_with_functions` (default-on `engine` cargo feature).
//! (Code span, not an intra-doc link: `registry` is `engine`-gated, and this
//! module doc is compiled in the engine-less build too — a link would break
//! `--no-default-features` rustdoc. sq-wo5jw)

use crate::literal::{Crs, GeoGeometry};
use crate::GeoError;
use geo::relate::IntersectionMatrix;
use geo::{
    Area, BooleanOps, BoundingRect, Buffer, Centroid, Closest, ConvexHull, CoordsIter, Distance,
    Euclidean, HasDimensions, Haversine, HaversineClosestPoint, Intersects, Length,
    LineIntersection, MapCoords, Relate, Simplify,
};
use geo_types::{
    Coord, Geometry, LineString, MultiLineString, MultiPoint, MultiPolygon, Point, Polygon,
};

/// Mean-Earth-radius metres per degree of arc (GRS80 mean radius 6 371 008.8 m,
/// the same sphere `geo`'s `Haversine` measures on): π·R/180.
const METERS_PER_DEGREE: f64 = std::f64::consts::PI * 6_371_008.8 / 180.0;

/// The local equirectangular metre frame shared by metric measurement and
/// buffering. Its latitude is the geometry bounding box's centre, matching
/// the crate's established `geof:buffer` convention. sq-lsp7k.18
#[derive(Debug, Clone, Copy)]
struct LocalMetricFrame {
    x_scale: f64,
}

impl LocalMetricFrame {
    fn for_geometry(g: &GeoGeometry, operation: &str) -> Result<Self, GeoError> {
        if !g.crs.is_geographic() {
            return Err(GeoError::NonGeographicCrs(g.crs.iri().to_string()));
        }
        let rect = g.geometry.bounding_rect().ok_or_else(|| {
            GeoError::Unsupported(format!("geof:{operation} of an empty geometry"))
        })?;
        let x_scale = METERS_PER_DEGREE * rect.center().y.to_radians().cos();
        if x_scale <= 0.0 {
            return Err(GeoError::Unsupported(format!(
                "geof:{operation} with metric units at the poles"
            )));
        }
        Ok(Self { x_scale })
    }

    fn project(self, geometry: &Geometry<f64>) -> Geometry<f64> {
        geometry.map_coords(|c| Coord {
            x: c.x * self.x_scale,
            y: c.y * METERS_PER_DEGREE,
        })
    }

    fn unproject_point(self, point: Point<f64>) -> Point<f64> {
        Point::new(point.x() / self.x_scale, point.y() / METERS_PER_DEGREE)
    }
}

/// A unit of measure accepted by `geof:distance`, selected by IRI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unit {
    Metre,
    Kilometre,
    Mile,
    Degree,
    Radian,
}

impl Unit {
    /// Resolves a unit IRI: the OGC `uom:` registry entries plus the common
    /// QUDT aliases.
    pub fn from_iri(iri: &str) -> Result<Unit, GeoError> {
        match iri {
            "http://www.opengis.net/def/uom/OGC/1.0/metre" | "http://qudt.org/vocab/unit/M" => {
                Ok(Unit::Metre)
            }
            "http://www.opengis.net/def/uom/OGC/1.0/kilometre"
            | "http://qudt.org/vocab/unit/KiloM" => Ok(Unit::Kilometre),
            "http://qudt.org/vocab/unit/MI" => Ok(Unit::Mile),
            "http://www.opengis.net/def/uom/OGC/1.0/degree"
            | "http://qudt.org/vocab/unit/DEG" => Ok(Unit::Degree),
            "http://www.opengis.net/def/uom/OGC/1.0/radian"
            | "http://qudt.org/vocab/unit/RAD" => Ok(Unit::Radian),
            other => Err(GeoError::UnknownUnit(other.to_string())),
        }
    }

    /// Metres per 1.0 of this unit (metric units only); `None` for the angular
    /// units (degree/radian), whose distance is euclidean coordinate-space — a
    /// different metric the metre-keyed `GeoIndex` cannot bound (so the spatial
    /// pushdown declines them, see `provider`).
    pub(crate) fn meters_scale(self) -> Option<f64> {
        match self {
            Unit::Metre => Some(1.0),
            Unit::Kilometre => Some(1000.0),
            Unit::Mile => Some(1609.344),
            Unit::Degree | Unit::Radian => None,
        }
    }
}

/// Both arguments must be in the same coordinate space: the two geographic
/// CRSs (CRS84 / EPSG:4326) are mutually compatible because parsing
/// normalised them to long/lat; any other CRS must match exactly.
/// `pub(crate)` so the engine registry's prepared-relate path (sq-hq8t5)
/// applies the SAME compatibility rule before relating.
pub(crate) fn ensure_compatible(a: &GeoGeometry, b: &GeoGeometry) -> Result<(), GeoError> {
    let compatible = match (&a.crs, &b.crs) {
        (x, y) if x.is_geographic() && y.is_geographic() => true,
        (Crs::Other(x), Crs::Other(y)) => x == y,
        _ => false,
    };
    if compatible {
        Ok(())
    } else {
        Err(GeoError::CrsMismatch(a.crs.iri().to_string(), b.crs.iri().to_string()))
    }
}

/// Euclidean coordinate-space distance between two geometries (degrees for
/// geographic CRSs). Dispatches one side so `geo`'s `Euclidean` pairwise
/// impls (every concrete type vs `&Geometry`) apply.
pub fn euclidean_distance(a: &Geometry<f64>, b: &Geometry<f64>) -> f64 {
    match a {
        Geometry::Point(g) => Euclidean.distance(g, b),
        Geometry::Line(g) => Euclidean.distance(g, b),
        Geometry::LineString(g) => Euclidean.distance(g, b),
        Geometry::Polygon(g) => Euclidean.distance(g, b),
        Geometry::MultiPoint(g) => Euclidean.distance(g, b),
        Geometry::MultiLineString(g) => Euclidean.distance(g, b),
        Geometry::MultiPolygon(g) => Euclidean.distance(g, b),
        Geometry::GeometryCollection(g) => Euclidean.distance(g, b),
        Geometry::Rect(g) => Euclidean.distance(g, b),
        Geometry::Triangle(g) => Euclidean.distance(g, b),
    }
}

/// Great-circle metres from a point to the nearest point of a geometry
/// (haversine on the GRS80 mean sphere; exact for points, spherical
/// closest-point for extended geometries).
pub fn point_to_geometry_meters(p: Point<f64>, g: &Geometry<f64>) -> Result<f64, GeoError> {
    match g.haversine_closest_point(&p) {
        Closest::Intersection(_) => Ok(0.0),
        Closest::SinglePoint(c) => Ok(Haversine.distance(p, c)),
        Closest::Indeterminate => Err(GeoError::Unsupported(
            "closest point is indeterminate (degenerate geometry)".to_string(),
        )),
    }
}

/// Great-circle metres between two geometries in a GEOGRAPHIC CRS (long/lat
/// degrees). Exact (haversine) when either side is a point; between two
/// extended geometries uses **vertex-`HaversineClosestPoint` iteration**
/// (sq-lk3aw.3): for each vertex of each geometry the haversine distance to
/// the nearest point on the other geometry is computed, and the minimum taken.
/// This resolves the prior equirectangular projection distortion for
/// continent-spanning extended↔extended pairs. Remaining approximation:
/// interior-of-segment↔interior-of-segment closest pairs (uncommon; bounded
/// by the vertex arc spacing on each side).
pub fn distance_meters(a: &Geometry<f64>, b: &Geometry<f64>) -> Result<f64, GeoError> {
    if a.intersects(b) {
        return Ok(0.0);
    }
    match (a, b) {
        (Geometry::Point(p), _) => point_to_geometry_meters(*p, b),
        (_, Geometry::Point(p)) => point_to_geometry_meters(*p, a),
        _ => {
            // sq-lk3aw.3: vertex-HaversineClosestPoint iteration,
            // replacing the local equirectangular approximation. For each vertex
            // of a, find the haversine distance to b via HaversineClosestPoint
            // (exact point-to-nearest-on-b); likewise for each vertex of b
            // against a. The minimum is exact when the closest pair involves at
            // least one vertex; interior-to-interior segment pairs are bounded
            // by vertex arc spacing (typically small for GeoSPARQL geometries).
            let mut min_dist = f64::INFINITY;
            for coord in a.coords_iter() {
                let p = Point::from(coord);
                match b.haversine_closest_point(&p) {
                    Closest::Intersection(_) => return Ok(0.0),
                    Closest::SinglePoint(c) => {
                        let d = Haversine.distance(p, c);
                        if d < min_dist {
                            min_dist = d;
                        }
                    }
                    Closest::Indeterminate => {}
                }
            }
            for coord in b.coords_iter() {
                let p = Point::from(coord);
                match a.haversine_closest_point(&p) {
                    Closest::Intersection(_) => return Ok(0.0),
                    Closest::SinglePoint(c) => {
                        let d = Haversine.distance(p, c);
                        if d < min_dist {
                            min_dist = d;
                        }
                    }
                    Closest::Indeterminate => {}
                }
            }
            if min_dist.is_infinite() {
                Err(GeoError::Unsupported("distance between empty geometries".to_string()))
            } else {
                Ok(min_dist)
            }
        }
    }
}

/// `geof:distance(geom1, geom2, units)`.
///
/// - Metric units ([`Unit::Metre`] / `Kilometre` / `Mile`) require a
///   geographic CRS and measure great-circle distance (see
///   [`distance_meters`]).
/// - [`Unit::Degree`] / [`Unit::Radian`] measure euclidean coordinate-space
///   distance (degrees of arc for geographic CRSs; raw coordinate units for
///   [`Crs::Other`]).
pub fn distance(a: &GeoGeometry, b: &GeoGeometry, unit: Unit) -> Result<f64, GeoError> {
    ensure_compatible(a, b)?;
    match unit.meters_scale() {
        Some(scale) => {
            if !a.crs.is_geographic() {
                return Err(GeoError::NonGeographicCrs(a.crs.iri().to_string()));
            }
            Ok(distance_meters(&a.geometry, &b.geometry)? / scale)
        }
        None => {
            let d = euclidean_distance(&a.geometry, &b.geometry);
            Ok(match unit {
                Unit::Radian => d.to_radians(),
                _ => d,
            })
        }
    }
}

// ---- Metric measurements + centroid ----------------------------------------------

/// `geof:metricArea(geom)` — planar area in square metres after projecting a
/// geographic geometry into the same local equirectangular frame as
/// [`buffer`]. Polygon holes are subtracted and multipolygon areas are summed.
/// Degenerate polygon rings return zero. Non-areal and heterogeneous geometry
/// types return [`GeoError::Unsupported`] rather than a dimensionally-wrong
/// number.
///
/// This is a local planar approximation. Distortion grows with geographic
/// extent and towards the poles. The deterministic acceptance oracle pins a
/// one-degree square near the equator to the analytic area within one percent.
pub fn metric_area(g: &GeoGeometry) -> Result<f64, GeoError> {
    match &g.geometry {
        Geometry::Polygon(_)
        | Geometry::MultiPolygon(_)
        | Geometry::Rect(_)
        | Geometry::Triangle(_) => {
            let frame = LocalMetricFrame::for_geometry(g, "metricArea")?;
            Ok(frame.project(&g.geometry).unsigned_area())
        }
        other => Err(GeoError::Unsupported(format!(
            "geof:metricArea is undefined for {}",
            wkt_type_name(other)
        ))),
    }
}

/// Returns the local-equirectangular length of the geometry's one-dimensional
/// components. Polygonal inputs include exterior and interior rings.
fn metric_length_for(g: &GeoGeometry, operation: &str) -> Result<f64, GeoError> {
    let frame = LocalMetricFrame::for_geometry(g, operation)?;
    let projected = frame.project(&g.geometry);
    let ring_length = |polygon: &Polygon<f64>| {
        Euclidean.length(polygon.exterior())
            + polygon.interiors().iter().map(|ring| Euclidean.length(ring)).sum::<f64>()
    };
    match &projected {
        Geometry::Line(line) => Ok(Euclidean.length(line)),
        Geometry::LineString(line) => Ok(Euclidean.length(line)),
        Geometry::MultiLineString(lines) => Ok(Euclidean.length(lines)),
        Geometry::Polygon(polygon) => Ok(ring_length(polygon)),
        Geometry::MultiPolygon(polygons) => Ok(polygons.0.iter().map(ring_length).sum()),
        Geometry::Rect(rect) => Ok(ring_length(&rect.to_polygon())),
        Geometry::Triangle(triangle) => Ok(ring_length(&triangle.to_polygon())),
        other => Err(GeoError::Unsupported(format!(
            "geof:{operation} is undefined for {}",
            wkt_type_name(other)
        ))),
    }
}

/// `geof:metricLength(geom)` — length in metres of a curve, or the boundary
/// length of an areal geometry, in the local equirectangular frame. A
/// `LineString` is measured as the sum of its segment lengths; the equatorial
/// two-point acceptance oracle agrees with the haversine distance to floating-
/// point tolerance.
pub fn metric_length(g: &GeoGeometry) -> Result<f64, GeoError> {
    metric_length_for(g, "metricLength")
}

/// `geof:metricPerimeter(geom)` — perimeter in metres. For areal geometries
/// this is the sum of the exterior and interior ring lengths; for curve
/// geometries it is equivalent to [`metric_length`].
pub fn metric_perimeter(g: &GeoGeometry) -> Result<f64, GeoError> {
    metric_length_for(g, "metricPerimeter")
}

/// `geof:centroid(geom)` — the mathematical centroid as a point in the input
/// geometry's CRS. Geographic geometries are projected to the local metric
/// frame for the calculation and then unprojected; other CRSs are evaluated in
/// their coordinate space. Empty geometries return [`GeoError::Unsupported`].
pub fn centroid(g: &GeoGeometry) -> Result<GeoGeometry, GeoError> {
    let point = if g.crs.is_geographic() {
        let frame = LocalMetricFrame::for_geometry(g, "centroid")?;
        let projected = frame.project(&g.geometry);
        frame.unproject_point(projected.centroid().ok_or_else(|| {
            GeoError::Unsupported("geof:centroid of an empty geometry".to_string())
        })?)
    } else {
        g.geometry.centroid().ok_or_else(|| {
            GeoError::Unsupported("geof:centroid of an empty geometry".to_string())
        })?
    };
    Ok(GeoGeometry { crs: g.crs.clone(), geometry: Geometry::Point(point) })
}

// ---- Simple-features relations (DE-9IM via geo's Relate) -------------------------

macro_rules! sf_relation {
    ($(#[$doc:meta])* $name:ident, $pred:ident) => {
        $(#[$doc])*
        pub fn $name(a: &GeoGeometry, b: &GeoGeometry) -> Result<bool, GeoError> {
            ensure_compatible(a, b)?;
            Ok(a.geometry.relate(&b.geometry).$pred())
        }
    };
}

sf_relation!(
    /// `geof:sfEquals` — topologically equal (same point set).
    sf_equals, is_equal_topo
);
sf_relation!(
    /// `geof:sfDisjoint` — no points in common.
    sf_disjoint, is_disjoint
);
sf_relation!(
    /// `geof:sfIntersects` — at least one point in common.
    sf_intersects, is_intersects
);
sf_relation!(
    /// `geof:sfTouches` — boundaries touch, interiors do not intersect.
    sf_touches, is_touches
);
sf_relation!(
    /// `geof:sfCrosses` — interiors intersect with lower-dimensional result.
    sf_crosses, is_crosses
);
sf_relation!(
    /// `geof:sfWithin` — `a` lies in the interior+boundary of `b`.
    sf_within, is_within
);
sf_relation!(
    /// `geof:sfContains` — `b` lies in the interior+boundary of `a` (and
    /// their interiors intersect).
    sf_contains, is_contains
);
sf_relation!(
    /// `geof:sfOverlaps` — same dimension, interiors intersect, neither
    /// contains the other.
    sf_overlaps, is_overlaps
);

// ---- Generic DE-9IM + Egenhofer / RCC8 relation families ---------------------------

/// `geof:relate` — generic DE-9IM pattern test (GeoSPARQL 1.0 §9 / Simple
/// Features `Relate`): `true` iff the intersection matrix of `a` against `b`
/// matches `pattern` (nine of `T` / `F` / `*` / `0` / `1` / `2`).
pub fn relate(a: &GeoGeometry, b: &GeoGeometry, pattern: &str) -> Result<bool, GeoError> {
    ensure_compatible(a, b)?;
    a.geometry
        .relate(&b.geometry)
        .matches(pattern)
        .map_err(|e| GeoError::Parse(format!("invalid DE-9IM pattern {pattern:?}: {e}")))
}

/// `true` iff `matrix` matches ANY of `patterns` — the spec defines some
/// relations as a disjunction of matrices. `pub(crate)` so the engine
/// registry's prepared-relate path (sq-hq8t5) evaluates the SAME disjunction
/// over a matrix it computed from a cached prepared side.
pub(crate) fn matrix_matches_any(matrix: &IntersectionMatrix, patterns: &[&str]) -> bool {
    // Patterns are compile-time constants below — a failure is a crate bug.
    patterns.iter().any(|p| matrix.matches(p).expect("valid built-in DE-9IM pattern"))
}

/// `true` iff the DE-9IM matrix of `a` vs `b` matches ANY of `patterns`
/// (the spec defines some relations as a disjunction of matrices).
fn relate_any(a: &GeoGeometry, b: &GeoGeometry, patterns: &[&str]) -> Result<bool, GeoError> {
    ensure_compatible(a, b)?;
    Ok(matrix_matches_any(&a.geometry.relate(&b.geometry), patterns))
}

macro_rules! de9im_relation {
    ($(#[$doc:meta])* $name:ident, $patterns:ident, [$($pattern:literal),+]) => {
        /// The GeoSPARQL DE-9IM matrix pattern disjunction defining the
        /// relation of the same name. `pub(crate)` so the engine registry's
        /// prepared-relate path (sq-hq8t5) tests the SAME spec matrices —
        /// this const and the public function below are the single source
        /// of truth.
        pub(crate) const $patterns: &[&str] = &[$($pattern),+];
        $(#[$doc])*
        pub fn $name(a: &GeoGeometry, b: &GeoGeometry) -> Result<bool, GeoError> {
            relate_any(a, b, $patterns)
        }
    };
}

// The Egenhofer relation family (GeoSPARQL 1.0 Req 25 / 1.1 §9 — the standard
// DE-9IM matrix patterns for each relation).
de9im_relation!(
    /// `geof:ehEquals` — Egenhofer equal.
    eh_equals, EH_EQUALS_PATTERNS, ["TFFFTFFFT"]
);
de9im_relation!(
    /// `geof:ehDisjoint` — Egenhofer disjoint.
    eh_disjoint, EH_DISJOINT_PATTERNS, ["FF*FF****"]
);
de9im_relation!(
    /// `geof:ehMeet` — Egenhofer meet (boundaries in contact, interiors not).
    eh_meet, EH_MEET_PATTERNS, ["FT*******", "F**T*****", "F***T****"]
);
de9im_relation!(
    /// `geof:ehOverlap` — Egenhofer overlap.
    eh_overlap, EH_OVERLAP_PATTERNS, ["T*T***T**"]
);
de9im_relation!(
    /// `geof:ehCovers` — Egenhofer covers.
    eh_covers, EH_COVERS_PATTERNS, ["T*TFT*FF*"]
);
de9im_relation!(
    /// `geof:ehCoveredBy` — Egenhofer coveredBy.
    eh_covered_by, EH_COVERED_BY_PATTERNS, ["TFF*TFT**"]
);
de9im_relation!(
    /// `geof:ehInside` — Egenhofer inside.
    eh_inside, EH_INSIDE_PATTERNS, ["TFF*FFT**"]
);
de9im_relation!(
    /// `geof:ehContains` — Egenhofer contains.
    eh_contains, EH_CONTAINS_PATTERNS, ["T*TFF*FF*"]
);

// The RCC8 relation family (GeoSPARQL 1.0 Req 26 / 1.1 §9). RCC8 is defined
// over REGIONS (non-empty interiors); the matrices below are the spec's.
de9im_relation!(
    /// `geof:rcc8eq` — equal.
    rcc8_eq, RCC8_EQ_PATTERNS, ["TFFFTFFFT"]
);
de9im_relation!(
    /// `geof:rcc8dc` — disconnected.
    rcc8_dc, RCC8_DC_PATTERNS, ["FFTFFTTTT"]
);
de9im_relation!(
    /// `geof:rcc8ec` — externally connected (boundaries touch).
    rcc8_ec, RCC8_EC_PATTERNS, ["FFTFTTTTT"]
);
de9im_relation!(
    /// `geof:rcc8po` — partially overlapping.
    rcc8_po, RCC8_PO_PATTERNS, ["TTTTTTTTT"]
);
de9im_relation!(
    /// `geof:rcc8tppi` — tangential proper part inverse.
    rcc8_tppi, RCC8_TPPI_PATTERNS, ["TTTFTTFFT"]
);
de9im_relation!(
    /// `geof:rcc8tpp` — tangential proper part.
    rcc8_tpp, RCC8_TPP_PATTERNS, ["TFFTTFTTT"]
);
de9im_relation!(
    /// `geof:rcc8ntpp` — non-tangential proper part.
    rcc8_ntpp, RCC8_NTPP_PATTERNS, ["TFFTFFTTT"]
);
de9im_relation!(
    /// `geof:rcc8ntppi` — non-tangential proper part inverse.
    rcc8_ntppi, RCC8_NTPPI_PATTERNS, ["TTTFFTFFT"]
);

// ---- Envelope and bounding-coordinate functions -----------------------------------

/// `geof:isEmpty` — whether the geometry is the empty set.
///
/// This predicate is pure geometry introspection and does not depend on the
/// geometry's CRS or any unit convention. sq-lc2io
pub fn is_empty(g: &GeoGeometry) -> Result<bool, GeoError> {
    Ok(g.geometry.is_empty())
}

fn bounding_box(g: &GeoGeometry) -> Result<geo_types::Rect<f64>, GeoError> {
    g.geometry
        .bounding_rect()
        .ok_or_else(|| GeoError::Unsupported("empty geometry has no bounding box".to_string()))
}

/// `geof:maxX` — the maximum X (easting or longitude) of the geometry's
/// envelope in its stored CRS.
///
/// Empty geometries return [`GeoError::Unsupported`]. No reprojection is
/// performed.
pub fn max_x(g: &GeoGeometry) -> Result<f64, GeoError> {
    Ok(bounding_box(g)?.max().x)
}

/// `geof:minX` — the minimum X (easting or longitude) of the geometry's
/// envelope in its stored CRS.
///
/// Empty geometries return [`GeoError::Unsupported`]. No reprojection is
/// performed.
pub fn min_x(g: &GeoGeometry) -> Result<f64, GeoError> {
    Ok(bounding_box(g)?.min().x)
}

/// `geof:maxY` — the maximum Y (northing or latitude) of the geometry's
/// envelope in its stored CRS.
///
/// Empty geometries return [`GeoError::Unsupported`]. No reprojection is
/// performed.
pub fn max_y(g: &GeoGeometry) -> Result<f64, GeoError> {
    Ok(bounding_box(g)?.max().y)
}

/// `geof:minY` — the minimum Y (northing or latitude) of the geometry's
/// envelope in its stored CRS.
///
/// Empty geometries return [`GeoError::Unsupported`]. No reprojection is
/// performed.
pub fn min_y(g: &GeoGeometry) -> Result<f64, GeoError> {
    Ok(bounding_box(g)?.min().y)
}

// ---- Geometry-producing functions -------------------------------------------------

/// `geof:envelope` — the minimum bounding rectangle, as a polygon (degenerate
/// for points / vertical / horizontal inputs, matching the spec's "envelope").
pub fn envelope(g: &GeoGeometry) -> Result<GeoGeometry, GeoError> {
    let rect = g
        .geometry
        .bounding_rect()
        .ok_or_else(|| GeoError::Unsupported("empty geometry has no envelope".to_string()))?;
    Ok(GeoGeometry { crs: g.crs.clone(), geometry: Geometry::Polygon(rect.to_polygon()) })
}

/// `geof:convexHull` — the convex hull of the geometry's coordinates.
pub fn convex_hull(g: &GeoGeometry) -> Result<GeoGeometry, GeoError> {
    if g.geometry.coords_count() == 0 {
        return Err(GeoError::Unsupported("empty geometry has no convex hull".to_string()));
    }
    let points: MultiPoint<f64> = g.geometry.coords_iter().map(Point::from).collect();
    Ok(GeoGeometry { crs: g.crs.clone(), geometry: Geometry::Polygon(points.convex_hull()) })
}

/// `geof:simplify(geom, tolerance)` — Ramer–Douglas–Peucker simplification in
/// the input CRS's coordinate space. The result preserves the CRS and retains
/// only vertices from the input. As with `geo`'s [`Simplify`] implementation,
/// polygon simplification is not topology-preserving and can produce an invalid
/// polygon.
///
/// A non-positive tolerance returns `g` unchanged. Points, multipoints, and a
/// two-vertex [`geo_types::Line`] are also unchanged because they have no
/// removable vertices. `LineString`, `MultiLineString`, `Polygon`, and
/// `MultiPolygon` delegate directly to `geo`'s Douglas–Peucker implementation.
/// Rectangles, triangles, and geometry collections return
/// [`GeoError::Unsupported`] because `geo` does not implement [`Simplify`] for
/// those types. A positive tolerance must be finite. sq-lsp7k.23
pub fn simplify(g: &GeoGeometry, tolerance: f64) -> Result<GeoGeometry, GeoError> {
    if tolerance <= 0.0 {
        return Ok(g.clone());
    }
    if !tolerance.is_finite() {
        return Err(GeoError::Unsupported(
            "geof:simplify requires a finite tolerance".to_string(),
        ));
    }

    let geometry = match &g.geometry {
        Geometry::Point(_) | Geometry::MultiPoint(_) | Geometry::Line(_) => g.geometry.clone(),
        Geometry::LineString(line) => Geometry::LineString(line.simplify(tolerance)),
        Geometry::MultiLineString(lines) => Geometry::MultiLineString(lines.simplify(tolerance)),
        Geometry::Polygon(polygon) => Geometry::Polygon(polygon.simplify(tolerance)),
        Geometry::MultiPolygon(polygons) => Geometry::MultiPolygon(polygons.simplify(tolerance)),
        other => {
            return Err(GeoError::Unsupported(format!(
                "geof:simplify is undefined for {}",
                wkt_type_name(other)
            )))
        }
    };
    Ok(GeoGeometry {
        crs: g.crs.clone(),
        geometry,
    })
}

/// `geof:boundary` — the simple-features boundary:
///
/// - points / multipoints: empty (an empty MULTIPOINT),
/// - curves: the end points appearing an ODD number of times (mod-2 rule;
///   closed curves have an empty boundary),
/// - surfaces: the exterior + interior rings as a MULTILINESTRING,
/// - geometry collections: unsupported in v1 (heterogeneous boundary).
pub fn boundary(g: &GeoGeometry) -> Result<GeoGeometry, GeoError> {
    let geometry = boundary_geometry(&g.geometry)?;
    Ok(GeoGeometry { crs: g.crs.clone(), geometry })
}

fn boundary_geometry(g: &Geometry<f64>) -> Result<Geometry<f64>, GeoError> {
    /// Mod-2 endpoint rule over a set of curves.
    fn curve_boundary<'a>(curves: impl Iterator<Item = &'a LineString<f64>>) -> Geometry<f64> {
        let mut counts: Vec<(Coord<f64>, usize)> = Vec::new();
        let mut bump = |c: Coord<f64>| match counts.iter_mut().find(|(x, _)| *x == c) {
            Some((_, n)) => *n += 1,
            None => counts.push((c, 1)),
        };
        for ls in curves {
            if ls.0.len() < 2 || ls.is_closed() {
                continue;
            }
            bump(ls.0[0]);
            bump(*ls.0.last().unwrap());
        }
        let pts: Vec<Point<f64>> =
            counts.into_iter().filter(|(_, n)| n % 2 == 1).map(|(c, _)| Point::from(c)).collect();
        Geometry::MultiPoint(MultiPoint(pts))
    }

    fn rings(p: &Polygon<f64>) -> impl Iterator<Item = LineString<f64>> + '_ {
        std::iter::once(p.exterior().clone()).chain(p.interiors().iter().cloned())
    }

    Ok(match g {
        Geometry::Point(_) | Geometry::MultiPoint(_) => Geometry::MultiPoint(MultiPoint(vec![])),
        Geometry::Line(l) => {
            Geometry::MultiPoint(MultiPoint(vec![l.start_point(), l.end_point()]))
        }
        Geometry::LineString(ls) => curve_boundary(std::iter::once(ls)),
        Geometry::MultiLineString(mls) => curve_boundary(mls.0.iter()),
        Geometry::Polygon(p) => Geometry::MultiLineString(MultiLineString(rings(p).collect())),
        Geometry::MultiPolygon(mp) => {
            Geometry::MultiLineString(MultiLineString(mp.0.iter().flat_map(rings).collect()))
        }
        Geometry::Rect(r) => boundary_geometry(&Geometry::Polygon(r.to_polygon()))?,
        Geometry::Triangle(t) => boundary_geometry(&Geometry::Polygon(t.to_polygon()))?,
        Geometry::GeometryCollection(_) => {
            return Err(GeoError::Unsupported(
                "geof:boundary of a GEOMETRYCOLLECTION".to_string(),
            ))
        }
    })
}

// ---- Set operations (GeoSPARQL §10.x point-set ops) -------------------------------
//
// GeoSPARQL defines `geof:intersection` / `union` / `difference` /
// `symDifference` as the set-theoretic operations on the geometries' point
// sets, returning the result geometry. `geo`'s `BooleanOps` realises this for
// the POLYGON×POLYGON case (a polygon-overlay algorithm) but does NOT touch
// 1-D operands — it only nodes/overlays polygons. We fill the 1-D gap with two
// roll-your-own pieces (see AGENTS.md "Upstream blockers"):
//
//   * line ∩ polygon / line − polygon — a robustly-noded polyline clip via
//     `i_overlay`'s string-line overlay (`FloatClip::clip_by`). This is the
//     SAME overlay engine `geo 0.33` itself uses for polygon overlay, exposed
//     for the open-path (string) case `geo` does not re-export. `invert=false`
//     keeps the in-polygon portions (intersection); `invert=true` keeps the
//     out-of-polygon portions (difference).
//   * line − line / line ∆ line — `i_overlay`'s string overlay clips lines
//     only against CLOSED shapes, so a genuine line-on-line subtraction is
//     done here directly: subtract the collinear-overlap parameter intervals of
//     `b` from each segment of `a` (linear referencing). Crossing points are
//     measure-zero and so do not change a 1-D point set.
//
// line ∩ line stays on `geo`'s `line_intersection` (unchanged; already
// correct). Genuinely-intractable mixes still return an honest
// [`GeoError::Unsupported`]. See the README operand matrix.

use geo::coordinate_position::CoordPos;
use geo::line_intersection::line_intersection;
use geo::CoordinatePosition;
use geo_types::Line;

use i_overlay::core::fill_rule::FillRule;
use i_overlay::float::clip::FloatClip;
use i_overlay::string::clip::ClipRule;

/// The topological dimension of a geometry: 0 = point(s), 1 = curve(s),
/// 2 = surface(s). `None` for an empty geometry or a heterogeneous
/// GEOMETRYCOLLECTION (which the dimension-keyed set ops do not handle).
fn dimension(g: &Geometry<f64>) -> Option<u8> {
    match g {
        Geometry::Point(_) => Some(0),
        Geometry::MultiPoint(mp) if !mp.0.is_empty() => Some(0),
        Geometry::Line(_) | Geometry::LineString(_) => Some(1),
        Geometry::MultiLineString(mls) if !mls.0.is_empty() => Some(1),
        Geometry::Polygon(_)
        | Geometry::MultiPolygon(_)
        | Geometry::Rect(_)
        | Geometry::Triangle(_) => Some(2),
        // Empty MULTIPOINT / MULTILINESTRING (geo-types' empty representations).
        Geometry::MultiPoint(_) | Geometry::MultiLineString(_) => None,
        Geometry::GeometryCollection(_) => None,
    }
}

/// The geometry as a multipolygon, if it is polygonal (POLYGON, MULTIPOLYGON,
/// or the rect/triangle shorthands). `None` for points, curves, collections.
fn polygonal(g: &Geometry<f64>) -> Option<MultiPolygon<f64>> {
    match g {
        Geometry::Polygon(p) => Some(MultiPolygon(vec![p.clone()])),
        Geometry::MultiPolygon(mp) => Some(mp.clone()),
        Geometry::Rect(r) => Some(MultiPolygon(vec![r.to_polygon()])),
        Geometry::Triangle(t) => Some(MultiPolygon(vec![t.to_polygon()])),
        _ => None,
    }
}

/// The WKT keyword for a geometry variant (for error messages).
fn wkt_type_name(g: &Geometry<f64>) -> &'static str {
    match g {
        Geometry::Point(_) => "POINT",
        Geometry::Line(_) | Geometry::LineString(_) => "LINESTRING",
        Geometry::Polygon(_) | Geometry::Rect(_) | Geometry::Triangle(_) => "POLYGON",
        Geometry::MultiPoint(_) => "MULTIPOINT",
        Geometry::MultiLineString(_) => "MULTILINESTRING",
        Geometry::MultiPolygon(_) => "MULTIPOLYGON",
        Geometry::GeometryCollection(_) => "GEOMETRYCOLLECTION",
    }
}

/// An empty geometry of the given dimension (geo-types has no empty Point /
/// LineString / Polygon, so 0/1 use the empty MULTI* and 2 the empty
/// MULTIPOLYGON — matching how the rest of the crate models emptiness).
fn empty_of_dimension(dim: u8) -> Geometry<f64> {
    match dim {
        0 => Geometry::MultiPoint(MultiPoint(vec![])),
        1 => Geometry::MultiLineString(MultiLineString(vec![])),
        _ => Geometry::MultiPolygon(MultiPolygon(vec![])),
    }
}

/// Every constituent [`Coord`] of a 0-dimensional operand (POINT / MULTIPOINT).
fn point_coords(g: &Geometry<f64>) -> Vec<Coord<f64>> {
    match g {
        Geometry::Point(p) => vec![p.0],
        Geometry::MultiPoint(mp) => mp.0.iter().map(|p| p.0).collect(),
        _ => Vec::new(),
    }
}

/// Every constituent [`LineString`] of a 1-dimensional operand. A bare `Line`
/// is promoted to a two-vertex `LineString`.
fn line_strings(g: &Geometry<f64>) -> Vec<LineString<f64>> {
    match g {
        Geometry::Line(l) => vec![LineString(vec![l.start, l.end])],
        Geometry::LineString(ls) => vec![ls.clone()],
        Geometry::MultiLineString(mls) => mls.0.clone(),
        _ => Vec::new(),
    }
}

/// Two coords equal within a tiny epsilon (intersection arithmetic rarely lands
/// on an exact bit pattern; we de-duplicate result points robustly).
fn coord_eq(a: Coord<f64>, b: Coord<f64>) -> bool {
    const EPS: f64 = 1e-12;
    (a.x - b.x).abs() <= EPS && (a.y - b.y).abs() <= EPS
}

/// Collect a MULTIPOINT result, de-duplicating coincident coords.
fn collect_points(coords: Vec<Coord<f64>>) -> Geometry<f64> {
    let mut out: Vec<Point<f64>> = Vec::new();
    for c in coords {
        if !out.iter().any(|p| coord_eq(p.0, c)) {
            out.push(Point::from(c));
        }
    }
    Geometry::MultiPoint(MultiPoint(out))
}

/// Collect line pieces as the narrowest single 1-D geometry: a lone piece as a
/// LINESTRING, several as a MULTILINESTRING, an empty set as an empty
/// MULTILINESTRING.
fn collect_lines(lines: Vec<LineString<f64>>) -> Geometry<f64> {
    match lines.len() {
        0 => empty_of_dimension(1),
        1 => Geometry::LineString(lines.into_iter().next().unwrap()),
        _ => Geometry::MultiLineString(MultiLineString(lines)),
    }
}

// ---- intersection -----------------------------------------------------------------

/// `geof:intersection` — the geometry of the points common to `a` and `b`.
///
/// Supported operand combinations (either argument order):
///
/// - polygon ∩ polygon → MULTIPOLYGON (`geo`'s `BooleanOps`),
/// - point ∩ anything → the point if it lies on the other geometry, else an
///   empty MULTIPOINT (point-in/on test via `CoordinatePosition`),
/// - line ∩ polygon → the portion(s) of the line inside-or-on the polygon,
///   as a (MULTI)LINESTRING (line clipping),
/// - line ∩ line → the crossing points and any overlapping (collinear)
///   segments, per `geo`'s `line_intersection`.
///
/// The result is the lowest-dimensional geometry that captures the
/// intersection (e.g. a line just *touching* a polygon at one point yields a
/// MULTIPOINT). Result CRS follows the operands.
pub fn intersection(a: &GeoGeometry, b: &GeoGeometry) -> Result<GeoGeometry, GeoError> {
    ensure_compatible(a, b)?;
    let geometry = intersection_geometry(&a.geometry, &b.geometry)?;
    Ok(GeoGeometry { crs: a.crs.clone(), geometry })
}

fn intersection_geometry(a: &Geometry<f64>, b: &Geometry<f64>) -> Result<Geometry<f64>, GeoError> {
    // Polygon × polygon: the BooleanOps overlay.
    if let (Some(pa), Some(pb)) = (polygonal(a), polygonal(b)) {
        return Ok(Geometry::MultiPolygon(pa.intersection(&pb)));
    }
    match (dimension(a), dimension(b)) {
        // Either operand empty (or a heterogeneous collection): no intersection.
        (None, _) | (_, None) => Ok(empty_of_dimension(0)),
        // Point on the LOWER-dimension side: keep the points that lie on the other.
        (Some(0), _) => Ok(points_on(point_coords(a), b)),
        (_, Some(0)) => Ok(points_on(point_coords(b), a)),
        // Line ∩ line.
        (Some(1), Some(1)) => Ok(line_line_intersection(a, b)),
        // Line ∩ polygon (clip the line to the polygon).
        (Some(1), Some(2)) => Ok(intersect_line_with_polygon(a, &polygonal(b).unwrap())),
        (Some(2), Some(1)) => Ok(intersect_line_with_polygon(b, &polygonal(a).unwrap())),
        _ => Err(GeoError::Unsupported(format!(
            "geof:intersection does not support {} ∩ {}",
            wkt_type_name(a),
            wkt_type_name(b)
        ))),
    }
}

/// The subset of `coords` lying inside-or-on `g` (a MULTIPOINT).
fn points_on(coords: Vec<Coord<f64>>, g: &Geometry<f64>) -> Geometry<f64> {
    let kept = coords
        .into_iter()
        .filter(|c| g.coordinate_position(c) != CoordPos::Outside)
        .collect();
    collect_points(kept)
}

/// Line ∩ line over every constituent segment pair: crossing points and any
/// overlapping collinear segments. Collinear overlaps are emitted as
/// LINESTRINGs; isolated crossings as a MULTIPOINT; a mix as a
/// GEOMETRYCOLLECTION.
fn line_line_intersection(a: &Geometry<f64>, b: &Geometry<f64>) -> Geometry<f64> {
    let mut points: Vec<Coord<f64>> = Vec::new();
    let mut segments: Vec<LineString<f64>> = Vec::new();
    for la in line_strings(a) {
        for sa in la.lines() {
            for lb in line_strings(b) {
                for sb in lb.lines() {
                    match line_intersection(sa, sb) {
                        Some(LineIntersection::SinglePoint { intersection, .. }) => {
                            points.push(intersection)
                        }
                        Some(LineIntersection::Collinear { intersection }) => {
                            segments.push(LineString(vec![intersection.start, intersection.end]))
                        }
                        None => {}
                    }
                }
            }
        }
    }
    // Drop crossing points that already lie on an overlapping segment.
    points.retain(|p| !segments.iter().any(|s| point_on_linestring(*p, s)));
    match (segments.is_empty(), points.is_empty()) {
        (true, true) => empty_of_dimension(0),
        (false, true) => collect_lines(segments),
        (true, false) => collect_points(points),
        (false, false) => {
            // Mixed dimensions (crossings + overlaps): a GEOMETRYCOLLECTION.
            let geoms = vec![collect_lines(segments), collect_points(points)];
            Geometry::GeometryCollection(geo_types::GeometryCollection(geoms))
        }
    }
}

/// Whether `c` lies on the linestring `ls` (any segment), within epsilon.
fn point_on_linestring(c: Coord<f64>, ls: &LineString<f64>) -> bool {
    ls.lines().any(|seg| seg.coordinate_position(&c) != CoordPos::Outside)
}

/// Clip a 1-D operand to a polygon (line ∩ polygon): the portions of the line
/// lying inside-or-on the polygon. Routed through the same `i_overlay`
/// string-line clip as line − polygon (`clip_line_by_polygon`) for a single
/// shared, robustly-noded clipper across both halves of the partition;
/// `invert == false` keeps the in-polygon portions and `boundary_included ==
/// true` keeps a span running ALONG the boundary (the polygon is a CLOSED set),
/// the exact complement of the `(invert: true, boundary_included: true)` rule
/// used for line − polygon. Returns a (MULTI)LINESTRING, or — when the line
/// only touches the polygon at isolated points (which `i_overlay` drops as
/// zero-length pieces) — a MULTIPOINT of those touch points.
fn intersect_line_with_polygon(line: &Geometry<f64>, poly: &MultiPolygon<f64>) -> Geometry<f64> {
    let pieces = clip_line_by_polygon(line, poly, false, true);
    if !pieces.is_empty() {
        return collect_lines(pieces);
    }
    // No 1-D overlap: the line may still graze the polygon at isolated vertices.
    // `i_overlay` drops these zero-length pieces, so recover them directly as a
    // MULTIPOINT (matching the prior hand-rolled fallback).
    let mut touch_points: Vec<Coord<f64>> = Vec::new();
    for ls in line_strings(line) {
        for c in ls.0 {
            if poly.coordinate_position(&c) != CoordPos::Outside {
                touch_points.push(c);
            }
        }
    }
    collect_points(touch_points)
}

/// The parameter t∈[0,1] of `c` along `seg` (projection onto the longer axis;
/// `seg` is assumed non-degenerate here).
fn param_of(seg: Line<f64>, c: Coord<f64>) -> f64 {
    let dx = seg.end.x - seg.start.x;
    let dy = seg.end.y - seg.start.y;
    if dx.abs() >= dy.abs() {
        if dx == 0.0 {
            0.0
        } else {
            (c.x - seg.start.x) / dx
        }
    } else {
        (c.y - seg.start.y) / dy
    }
}

/// The coord at parameter t along `seg`.
fn at_param(seg: Line<f64>, t: f64) -> Coord<f64> {
    Coord {
        x: seg.start.x + (seg.end.x - seg.start.x) * t,
        y: seg.start.y + (seg.end.y - seg.start.y) * t,
    }
}

/// Stitch consecutive clip pieces that share an endpoint back into longer
/// linestrings (so a straight clipped span returns one LINESTRING, not many
/// two-vertex hops). A simple greedy chain; good enough for the clip output.
fn merge_pieces(pieces: &mut Vec<LineString<f64>>) {
    let mut merged: Vec<LineString<f64>> = Vec::new();
    for piece in pieces.drain(..) {
        let mut coords = piece.0;
        if coords.len() < 2 {
            continue;
        }
        match merged.last_mut() {
            Some(prev) if coord_eq(*prev.0.last().unwrap(), coords[0]) => {
                prev.0.extend_from_slice(&coords[1..]);
            }
            _ => merged.push(LineString(std::mem::take(&mut coords))),
        }
    }
    *pieces = merged;
}

// ---- 1-D overlay via i_overlay (string-line clip) + linear-referencing -------------
//
// `geo`'s `BooleanOps` does not touch open paths; these helpers fill the gap.
//

/// A geo-types ring as an `i_overlay` contour ([`[f64; 2]`] is
/// `FloatPointCompatible`). The closing duplicate vertex is dropped —
/// `i_overlay` auto-closes contours.
fn ring_to_contour(ring: &LineString<f64>) -> Vec<[f64; 2]> {
    let mut coords: &[Coord<f64>] = &ring.0;
    if let (Some(first), Some(last)) = (coords.first(), coords.last()) {
        if coords.len() >= 2 && coord_eq(*first, *last) {
            coords = &coords[..coords.len() - 1];
        }
    }
    coords.iter().map(|c| [c.x, c.y]).collect()
}

/// A multipolygon as `i_overlay` "shapes" (each shape: outer contour + hole
/// contours).
fn polygon_to_shapes(poly: &MultiPolygon<f64>) -> Vec<Vec<Vec<[f64; 2]>>> {
    poly.0
        .iter()
        .map(|p| {
            std::iter::once(ring_to_contour(p.exterior()))
                .chain(p.interiors().iter().map(ring_to_contour))
                .collect()
        })
        .collect()
}

/// Clip the 1-D operand `line` by `poly` with `i_overlay`'s robustly-noded
/// string-line overlay. `invert == false` keeps the in-polygon portions
/// (intersection); `invert == true` keeps the out-of-polygon portions
/// (difference). `boundary_included` decides whether a sub-segment running
/// ALONG the polygon boundary is treated as inside (kept for intersection,
/// dropped for difference). Returns the clipped pieces as geo-types
/// linestrings (the caller wraps them into the narrowest 1-D geometry).
fn clip_line_by_polygon(
    line: &Geometry<f64>,
    poly: &MultiPolygon<f64>,
    invert: bool,
    boundary_included: bool,
) -> Vec<LineString<f64>> {
    let shapes = polygon_to_shapes(poly);
    // Even-odd fill is winding-direction-insensitive and resolves holes
    // correctly regardless of how the WKT oriented the rings (verified by
    // spike); the positive/negative rules are NOT — do not use them.
    let rule = ClipRule { invert, boundary_included };
    let mut out: Vec<LineString<f64>> = Vec::new();
    for ls in line_strings(line) {
        if ls.0.len() < 2 {
            continue;
        }
        let path: Vec<[f64; 2]> = ls.0.iter().map(|c| [c.x, c.y]).collect();
        for piece in path.clip_by(&shapes, FillRule::EvenOdd, rule) {
            if piece.len() < 2 {
                continue;
            }
            out.push(LineString(piece.into_iter().map(|p| Coord { x: p[0], y: p[1] }).collect()));
        }
    }
    out
}

/// `a − b` for two 1-D operands (line − line): the portions of `a` not
/// collinearly covered by `b`. Crossing points are measure-zero in a 1-D point
/// set, so only the COLLINEAR overlaps of `b` are removed (a curve minus a set
/// of isolated points is the same curve). Implemented by linear referencing:
/// for each segment of `a`, subtract the parameter intervals where a segment of
/// `b` overlaps it.
fn line_minus_lines(a: &Geometry<f64>, b: &Geometry<f64>) -> Vec<LineString<f64>> {
    let mut pieces: Vec<LineString<f64>> = Vec::new();
    let b_segs: Vec<Line<f64>> =
        line_strings(b).iter().flat_map(|ls| ls.lines().collect::<Vec<_>>()).collect();
    for la in line_strings(a) {
        for seg in la.lines() {
            if coord_eq(seg.start, seg.end) {
                continue;
            }
            // Covered parameter intervals [lo, hi] ⊆ [0,1] where `b` overlaps.
            let mut covered: Vec<(f64, f64)> = Vec::new();
            for bseg in &b_segs {
                if let Some(LineIntersection::Collinear { intersection }) =
                    line_intersection(seg, *bseg)
                {
                    let mut t0 = param_of(seg, intersection.start);
                    let mut t1 = param_of(seg, intersection.end);
                    if t0 > t1 {
                        std::mem::swap(&mut t0, &mut t1);
                    }
                    let t0 = t0.clamp(0.0, 1.0);
                    let t1 = t1.clamp(0.0, 1.0);
                    if t1 - t0 > 1e-12 {
                        covered.push((t0, t1));
                    }
                }
            }
            // Emit the GAPS between covered intervals as surviving sub-segments.
            covered.sort_by(|x, y| x.0.partial_cmp(&y.0).unwrap());
            let mut cursor = 0.0_f64;
            for (lo, hi) in covered {
                if lo - cursor > 1e-12 {
                    pieces.push(LineString(vec![at_param(seg, cursor), at_param(seg, lo)]));
                }
                cursor = cursor.max(hi);
            }
            if 1.0 - cursor > 1e-12 {
                pieces.push(LineString(vec![at_param(seg, cursor), at_param(seg, 1.0)]));
            }
        }
    }
    merge_pieces(&mut pieces);
    pieces
}

// ---- union ------------------------------------------------------------------------

/// `geof:union` — the geometry of the points in `a` OR `b`.
///
/// - polygon ∪ polygon → MULTIPOLYGON (`geo`'s `BooleanOps`),
/// - point ∪ point → MULTIPOINT (the union of the coordinate sets),
/// - line ∪ line → MULTILINESTRING (the constituent curves, concatenated; this
///   is a valid OGC union — overlapping curves are NOT dissolved/noded, which
///   `geo` cannot do for 1-D geometry),
/// - mixed-dimension union → GEOMETRYCOLLECTION of the two operands.
pub fn union(a: &GeoGeometry, b: &GeoGeometry) -> Result<GeoGeometry, GeoError> {
    ensure_compatible(a, b)?;
    let geometry = union_geometry(&a.geometry, &b.geometry)?;
    Ok(GeoGeometry { crs: a.crs.clone(), geometry })
}

fn union_geometry(a: &Geometry<f64>, b: &Geometry<f64>) -> Result<Geometry<f64>, GeoError> {
    if let (Some(pa), Some(pb)) = (polygonal(a), polygonal(b)) {
        return Ok(Geometry::MultiPolygon(pa.union(&pb)));
    }
    match (dimension(a), dimension(b)) {
        // Empty operand: the union is the other operand (cloned).
        (None, _) => Ok(b.clone()),
        (_, None) => Ok(a.clone()),
        (Some(0), Some(0)) => {
            let mut coords = point_coords(a);
            coords.extend(point_coords(b));
            Ok(collect_points(coords))
        }
        (Some(1), Some(1)) => {
            let mut lines = line_strings(a);
            lines.extend(line_strings(b));
            Ok(collect_lines(lines))
        }
        // Mixed dimension: a heterogeneous union is exactly a GEOMETRYCOLLECTION.
        _ => Ok(Geometry::GeometryCollection(geo_types::GeometryCollection(vec![
            a.clone(),
            b.clone(),
        ]))),
    }
}

// ---- difference / symDifference ---------------------------------------------------

/// `geof:difference` — the geometry of the points in `a` but NOT in `b`.
///
/// - polygon − polygon → MULTIPOLYGON (`geo`'s `BooleanOps`),
/// - point − anything → the points of `a` that do NOT lie on `b` (MULTIPOINT),
/// - point − point (special-cased by the above): exact set subtraction,
/// - line − polygon → the portions of the line OUTSIDE the polygon, as a
///   (MULTI)LINESTRING (`i_overlay` string-line clip),
/// - line − line → the portions of `a` not collinearly overlapped by `b`
///   (linear-referencing subtraction; crossing points are measure-zero),
/// - polygon − line / polygon − point → the polygon unchanged (subtracting a
///   lower-dimensional, measure-zero set from a surface leaves it intact).
pub fn difference(a: &GeoGeometry, b: &GeoGeometry) -> Result<GeoGeometry, GeoError> {
    ensure_compatible(a, b)?;
    let geometry = difference_geometry(&a.geometry, &b.geometry)?;
    Ok(GeoGeometry { crs: a.crs.clone(), geometry })
}

fn difference_geometry(a: &Geometry<f64>, b: &Geometry<f64>) -> Result<Geometry<f64>, GeoError> {
    if let (Some(pa), Some(pb)) = (polygonal(a), polygonal(b)) {
        return Ok(Geometry::MultiPolygon(pa.difference(&pb)));
    }
    match (dimension(a), dimension(b)) {
        // Empty `a`: nothing to keep. Empty `b`: `a` unchanged.
        (None, _) => Ok(empty_of_dimension(0)),
        (Some(_), None) => Ok(a.clone()),
        // Point − anything: keep the coords of `a` not lying on `b`.
        (Some(0), _) => {
            let kept = point_coords(a)
                .into_iter()
                .filter(|c| b.coordinate_position(c) == CoordPos::Outside)
                .collect();
            Ok(collect_points(kept))
        }
        // Line − point: a point is measure-zero in a curve, so the line is
        // unchanged.
        (Some(1), Some(0)) => Ok(a.clone()),
        // Line − line: subtract `b`'s collinear overlaps from `a`.
        (Some(1), Some(1)) => Ok(collect_lines(line_minus_lines(a, b))),
        // Line − polygon: keep the line OUTSIDE the polygon. The polygon is a
        // CLOSED set, so a span lying ALONG its boundary belongs to the polygon
        // and is removed — `boundary_included: true` under `invert` excludes the
        // boundary from the "outside" result, making line−polygon the exact
        // complement of line∩polygon (verified).
        (Some(1), Some(2)) => {
            let poly = polygonal(b).unwrap();
            Ok(collect_lines(clip_line_by_polygon(a, &poly, true, true)))
        }
        // Polygon − (line | point): removing a measure-zero set leaves the
        // surface unchanged.
        (Some(2), Some(0)) | (Some(2), Some(1)) => Ok(a.clone()),
        // No remaining combinations: only mixes involving a heterogeneous
        // collection (dimension None) reach here, already handled above.
        _ => Err(GeoError::Unsupported(format!(
            "geof:difference does not support {} − {}",
            wkt_type_name(a),
            wkt_type_name(b)
        ))),
    }
}

/// `geof:symDifference` — the geometry of the points in exactly one of `a`, `b`
/// (i.e. (a−b) ∪ (b−a)).
///
/// - polygon ∆ polygon → MULTIPOLYGON (`geo`'s `BooleanOps`),
/// - point ∆ point → MULTIPOINT (the symmetric difference of the coord sets),
/// - point ∆ line/polygon (either order) → the points of the 0-D operand not on
///   the other, unioned with the other operand (a GEOMETRYCOLLECTION when mixed
///   dimension),
/// - line ∆ line → (a−b) ∪ (b−a): the non-shared portions of both curves, as a
///   (MULTI)LINESTRING (linear-referencing subtraction),
/// - line ∆ polygon (either order) → (line outside polygon) ∪ polygon, a
///   GEOMETRYCOLLECTION (polygon − line is the polygon unchanged).
pub fn sym_difference(a: &GeoGeometry, b: &GeoGeometry) -> Result<GeoGeometry, GeoError> {
    ensure_compatible(a, b)?;
    let geometry = sym_difference_geometry(&a.geometry, &b.geometry)?;
    Ok(GeoGeometry { crs: a.crs.clone(), geometry })
}

fn sym_difference_geometry(
    a: &Geometry<f64>,
    b: &Geometry<f64>,
) -> Result<Geometry<f64>, GeoError> {
    if let (Some(pa), Some(pb)) = (polygonal(a), polygonal(b)) {
        return Ok(Geometry::MultiPolygon(pa.xor(&pb)));
    }
    match (dimension(a), dimension(b)) {
        (None, None) => Ok(empty_of_dimension(0)),
        (None, Some(_)) => Ok(b.clone()),
        (Some(_), None) => Ok(a.clone()),
        // Same-dimension 0-D: exact coordinate-set symmetric difference.
        (Some(0), Some(0)) => {
            let ca = point_coords(a);
            let cb = point_coords(b);
            let mut out: Vec<Coord<f64>> =
                ca.iter().filter(|c| !cb.iter().any(|d| coord_eq(**c, *d))).copied().collect();
            out.extend(cb.iter().filter(|c| !ca.iter().any(|d| coord_eq(**c, *d))).copied());
            Ok(collect_points(out))
        }
        // Every remaining combination is the generic symmetric difference
        // (a−b) ∪ (b−a). With `difference_geometry` now covering the 1-D cases,
        // this serves point∆line/polygon, line∆line, and line∆polygon alike;
        // `union_geometry` collapses same-dimension results and emits a
        // GEOMETRYCOLLECTION for mixed dimensions.
        _ => {
            let left = difference_geometry(a, b)?;
            let right = difference_geometry(b, a)?;
            union_geometry(&left, &right)
        }
    }
}

// ---- geof:buffer -------------------------------------------------------------------

/// `geof:buffer(geom, radius, units)` — all points within `radius` of the
/// geometry, as a MULTIPOLYGON (geo 0.33's `Buffer`, rounded joins/caps).
///
/// - Metric units require a geographic CRS: the geometry is projected into a
///   LOCAL EQUIRECTANGULAR metre frame about its mean latitude, buffered
///   there, and unprojected — accurate at local scale and increasingly
///   distorted for continent-scale geometries or near the poles.
/// - [`Unit::Degree`] / [`Unit::Radian`] buffer in euclidean coordinate space
///   (degrees for geographic CRSs, raw units for [`Crs::Other`]).
pub fn buffer(g: &GeoGeometry, radius: f64, unit: Unit) -> Result<GeoGeometry, GeoError> {
    let buffered = match unit.meters_scale() {
        Some(scale) => {
            let frame = LocalMetricFrame::for_geometry(g, "buffer")?;
            let meters = radius * scale;
            frame.project(&g.geometry).buffer(meters).map_coords(|c| Coord {
                x: c.x / frame.x_scale,
                y: c.y / METERS_PER_DEGREE,
            })
        }
        None => {
            let d = match unit {
                Unit::Radian => radius.to_degrees(),
                _ => radius,
            };
            g.geometry.buffer(d)
        }
    };
    Ok(GeoGeometry { crs: g.crs.clone(), geometry: Geometry::MultiPolygon(buffered) })
}

// ---- Lexical-level mirrors (the engine-builtin shape) ------------------------------

/// Lexical-level `geof:` helpers: arguments and geometry results are
/// `geo:wktLiteral` lexical forms (plus plain `f64` / `bool`).
pub mod lex {
    use super::Unit;
    use crate::literal::parse_wkt_literal;
    use crate::GeoError;

    /// `geof:distance(?a, ?b, ?unitIri)`.
    pub fn distance(a: &str, b: &str, unit_iri: &str) -> Result<f64, GeoError> {
        super::distance(&parse_wkt_literal(a)?, &parse_wkt_literal(b)?, Unit::from_iri(unit_iri)?)
    }

    /// `geof:metricArea(?a)` -> square metres.
    pub fn metric_area(a: &str) -> Result<f64, GeoError> {
        super::metric_area(&parse_wkt_literal(a)?)
    }

    /// `geof:metricLength(?a)` -> metres.
    pub fn metric_length(a: &str) -> Result<f64, GeoError> {
        super::metric_length(&parse_wkt_literal(a)?)
    }

    /// `geof:metricPerimeter(?a)` -> metres.
    pub fn metric_perimeter(a: &str) -> Result<f64, GeoError> {
        super::metric_perimeter(&parse_wkt_literal(a)?)
    }

    macro_rules! lex_relation {
        ($(#[$doc:meta])* $name:ident) => {
            $(#[$doc])*
            pub fn $name(a: &str, b: &str) -> Result<bool, GeoError> {
                super::$name(&parse_wkt_literal(a)?, &parse_wkt_literal(b)?)
            }
        };
    }

    lex_relation!(
        /// `geof:sfEquals(?a, ?b)`.
        sf_equals
    );
    lex_relation!(
        /// `geof:sfDisjoint(?a, ?b)`.
        sf_disjoint
    );
    lex_relation!(
        /// `geof:sfIntersects(?a, ?b)`.
        sf_intersects
    );
    lex_relation!(
        /// `geof:sfTouches(?a, ?b)`.
        sf_touches
    );
    lex_relation!(
        /// `geof:sfCrosses(?a, ?b)`.
        sf_crosses
    );
    lex_relation!(
        /// `geof:sfWithin(?a, ?b)`.
        sf_within
    );
    lex_relation!(
        /// `geof:sfContains(?a, ?b)`.
        sf_contains
    );
    lex_relation!(
        /// `geof:sfOverlaps(?a, ?b)`.
        sf_overlaps
    );

    macro_rules! lex_geometry_fn {
        ($(#[$doc:meta])* $name:ident) => {
            $(#[$doc])*
            pub fn $name(a: &str) -> Result<String, GeoError> {
                Ok(super::$name(&parse_wkt_literal(a)?)?.to_wkt_literal())
            }
        };
    }

    lex_geometry_fn!(
        /// `geof:envelope(?a)` -> wktLiteral lexical form.
        envelope
    );
    lex_geometry_fn!(
        /// `geof:convexHull(?a)` -> wktLiteral lexical form.
        convex_hull
    );
    lex_geometry_fn!(
        /// `geof:boundary(?a)` -> wktLiteral lexical form.
        boundary
    );
    lex_geometry_fn!(
        /// `geof:centroid(?a)` -> wktLiteral point lexical form.
        centroid
    );

    /// `geof:simplify(?a, ?tolerance)` -> wktLiteral lexical form.
    pub fn simplify(a: &str, tolerance: f64) -> Result<String, GeoError> {
        Ok(super::simplify(&parse_wkt_literal(a)?, tolerance)?.to_wkt_literal())
    }

    /// `geof:getSRID(?a)` -> the geometry's CRS IRI (an `xsd:anyURI` value).
    pub fn get_srid(a: &str) -> Result<String, GeoError> {
        Ok(parse_wkt_literal(a)?.crs.iri().to_string())
    }

    /// `geof:relate(?a, ?b, ?de9imPattern)`.
    pub fn relate(a: &str, b: &str, pattern: &str) -> Result<bool, GeoError> {
        super::relate(&parse_wkt_literal(a)?, &parse_wkt_literal(b)?, pattern)
    }

    lex_relation!(
        /// `geof:ehEquals(?a, ?b)`.
        eh_equals
    );
    lex_relation!(
        /// `geof:ehDisjoint(?a, ?b)`.
        eh_disjoint
    );
    lex_relation!(
        /// `geof:ehMeet(?a, ?b)`.
        eh_meet
    );
    lex_relation!(
        /// `geof:ehOverlap(?a, ?b)`.
        eh_overlap
    );
    lex_relation!(
        /// `geof:ehCovers(?a, ?b)`.
        eh_covers
    );
    lex_relation!(
        /// `geof:ehCoveredBy(?a, ?b)`.
        eh_covered_by
    );
    lex_relation!(
        /// `geof:ehInside(?a, ?b)`.
        eh_inside
    );
    lex_relation!(
        /// `geof:ehContains(?a, ?b)`.
        eh_contains
    );
    lex_relation!(
        /// `geof:rcc8eq(?a, ?b)`.
        rcc8_eq
    );
    lex_relation!(
        /// `geof:rcc8dc(?a, ?b)`.
        rcc8_dc
    );
    lex_relation!(
        /// `geof:rcc8ec(?a, ?b)`.
        rcc8_ec
    );
    lex_relation!(
        /// `geof:rcc8po(?a, ?b)`.
        rcc8_po
    );
    lex_relation!(
        /// `geof:rcc8tppi(?a, ?b)`.
        rcc8_tppi
    );
    lex_relation!(
        /// `geof:rcc8tpp(?a, ?b)`.
        rcc8_tpp
    );
    lex_relation!(
        /// `geof:rcc8ntpp(?a, ?b)`.
        rcc8_ntpp
    );
    lex_relation!(
        /// `geof:rcc8ntppi(?a, ?b)`.
        rcc8_ntppi
    );

    macro_rules! lex_set_operation {
        ($(#[$doc:meta])* $name:ident) => {
            $(#[$doc])*
            pub fn $name(a: &str, b: &str) -> Result<String, GeoError> {
                Ok(super::$name(&parse_wkt_literal(a)?, &parse_wkt_literal(b)?)?.to_wkt_literal())
            }
        };
    }

    lex_set_operation!(
        /// `geof:intersection(?a, ?b)` -> wktLiteral lexical form (polygon/line/point
        /// operands; see [`super::intersection`] for the supported matrix).
        intersection
    );
    lex_set_operation!(
        /// `geof:union(?a, ?b)` -> wktLiteral lexical form (polygon/line/point
        /// operands; see [`super::union`]).
        union
    );
    lex_set_operation!(
        /// `geof:difference(?a, ?b)` -> wktLiteral lexical form (polygon, point,
        /// AND line operands incl. 1-D subtraction; see [`super::difference`]).
        difference
    );
    lex_set_operation!(
        /// `geof:symDifference(?a, ?b)` -> wktLiteral lexical form (see
        /// [`super::sym_difference`]).
        sym_difference
    );

    /// `geof:buffer(?a, ?radius, ?unitIri)` -> wktLiteral lexical form.
    pub fn buffer(a: &str, radius: f64, unit_iri: &str) -> Result<String, GeoError> {
        Ok(super::buffer(&parse_wkt_literal(a)?, radius, Unit::from_iri(unit_iri)?)?
            .to_wkt_literal())
    }
}

#[cfg(test)]
mod tests {
    use super::{is_empty, max_x, max_y, min_x, min_y};
    use crate::{parse_wkt_literal, GeoError, GeoGeometry};

    type CoordinateAccessor = fn(&GeoGeometry) -> Result<f64, GeoError>;

    fn assert_close(actual: Result<f64, GeoError>, expected: f64) {
        let actual = actual.expect("bounding coordinate");
        assert!(
            (actual - expected).abs() < 1e-12,
            "expected {expected}, got {actual}"
        );
    }

    #[test]
    fn bounding_coordinates_distinguish_polygon_extrema() {
        let polygon = parse_wkt_literal("POLYGON((0 0, 4 0, 4 3, 0 3, 0 0))")
            .expect("polygon WKT");

        assert_close(max_x(&polygon), 4.0);
        assert_close(min_x(&polygon), 0.0);
        assert_close(max_y(&polygon), 3.0);
        assert_close(min_y(&polygon), 0.0);
    }

    #[test]
    fn point_has_identical_minimum_and_maximum_coordinates() {
        let point = parse_wkt_literal("POINT(2.5 -1.5)").expect("point WKT");

        assert_close(max_x(&point), 2.5);
        assert_close(min_x(&point), 2.5);
        assert_close(max_y(&point), -1.5);
        assert_close(min_y(&point), -1.5);
    }

    #[test]
    fn empty_geometry_has_no_bounding_coordinates() {
        let empty = parse_wkt_literal("GEOMETRYCOLLECTION EMPTY").expect("empty WKT");
        let accessors: [CoordinateAccessor; 4] = [max_x, min_x, max_y, min_y];

        for accessor in accessors {
            assert!(matches!(
                accessor(&empty),
                Err(GeoError::Unsupported(_))
            ));
        }
    }

    #[test]
    fn empty_predicate_distinguishes_empty_and_non_empty_geometries() {
        for (wkt, expected) in [
            ("POINT(1 2)", false),
            ("POINT EMPTY", true),
            ("LINESTRING EMPTY", true),
            ("POLYGON((0 0,1 0,1 1,0 1,0 0))", false),
        ] {
            let geometry = parse_wkt_literal(wkt).expect("geometry WKT");
            assert_eq!(is_empty(&geometry), Ok(expected), "WKT: {wkt}");
        }
    }
}
