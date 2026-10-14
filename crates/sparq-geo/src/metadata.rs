//! GeoSPARQL geometry-METADATA properties (GeoSPARQL 1.0 §8.4 / 1.1 §8.5 — the
//! `geo:Geometry` data properties). sq-mzmh — closes OGC R9.
//!
//! The GeoSPARQL ontology attaches a handful of *metadata* properties to every
//! `geo:Geometry` node, describing the geometry itself rather than its topology
//! against another geometry:
//!
//! | property | type | meaning |
//! |---|---|---|
//! | `geo:dimension` | `xsd:integer` | the TOPOLOGICAL dimension: 0 for points, 1 for curves, 2 for surfaces |
//! | `geo:coordinateDimension` | `xsd:integer` | the number of measurements/axes in each coordinate (2 for this SF profile) |
//! | `geo:spatialDimension` | `xsd:integer` | the number of measurements that are SPATIAL (i.e. excluding the M measure); 2 here |
//! | `geo:isEmpty` | `xsd:boolean` | whether the geometry is the empty set |
//! | `geo:isSimple` | `xsd:boolean` | whether the geometry has no anomalous geometric points (no self-intersection / self-tangency) |
//! | `geo:hasSerialization` | `geo:wktLiteral` | a serialization of the geometry (the WKT lexical form) |
//!
//! This module MATERIALISES those property values from a parsed
//! [`GeoGeometry`]: [`GeoGeometry::metadata`] returns a [`GeometryMetadata`]
//! carrying every value, and the [`lex`] sub-module mirrors each at the lexical
//! level (a `geo:wktLiteral`/`geo:gmlLiteral` string in, the property value out)
//! — the shape a SPARQL engine or a triple-materialising loader consumes.
//!
//! ## Coordinate / spatial dimension
//!
//! sparq-geo's geometry model is the 2-D GML/WKT Simple-Features profile (the
//! GML parser rejects 3-D coordinates with `srsDimension="3"`, and the WKT path
//! is `geo_types::Geometry<f64>`, which is 2-D). So `coordinateDimension` and
//! `spatialDimension` are both **2** for every non-empty geometry — there is no
//! Z (elevation) nor M (measure) ordinate in this profile. The values are
//! reported honestly as 2 rather than guessed per geometry.
//!
//! ## Topological dimension of a collection
//!
//! For a heterogeneous `GEOMETRYCOLLECTION` the topological dimension is the
//! MAXIMUM dimension of its members (OGC SFA: a collection of a point and a
//! polygon has dimension 2). An empty geometry has no topological dimension
//! (`geo:dimension` is left unbound), exactly as the empty set has none.

use crate::literal::GeoGeometry;
use geo::HasDimensions;
use geo_types::{Coord, Geometry, LineString};

/// The materialised values of the GeoSPARQL geometry-metadata properties for one
/// geometry (`geo:dimension`/`coordinateDimension`/`spatialDimension`/`isEmpty`/
/// `isSimple`/`hasSerialization`). sq-mzmh
#[derive(Debug, Clone, PartialEq)]
pub struct GeometryMetadata {
    /// `geo:dimension` — the topological dimension (0 = points, 1 = curves,
    /// 2 = surfaces; the maximum over a collection's members). `None` for the
    /// empty geometry, which has no topological dimension.
    pub dimension: Option<u8>,
    /// `geo:coordinateDimension` — measurements per coordinate. Always 2 in this
    /// 2-D Simple-Features profile (`None` only for the empty geometry).
    pub coordinate_dimension: Option<u8>,
    /// `geo:spatialDimension` — spatial measurements per coordinate (excludes the
    /// M measure). Always 2 here (`None` only for the empty geometry).
    pub spatial_dimension: Option<u8>,
    /// `geo:isEmpty` — whether the geometry is the empty set.
    pub is_empty: bool,
    /// `geo:isSimple` — whether the geometry has no anomalous geometric points
    /// (no self-intersection / self-tangency); see [`is_simple`].
    pub is_simple: bool,
    /// `geo:hasSerialization` — a serialization of the geometry, as the
    /// `geo:wktLiteral` lexical form ([`GeoGeometry::to_wkt_literal`]).
    pub has_serialization: String,
}

impl GeoGeometry {
    /// Materialises this geometry's GeoSPARQL metadata-property values
    /// (`geo:dimension`/`coordinateDimension`/`spatialDimension`/`isEmpty`/
    /// `isSimple`/`hasSerialization`). sq-mzmh — OGC R9.
    pub fn metadata(&self) -> GeometryMetadata {
        let empty = self.geometry.is_empty();
        GeometryMetadata {
            dimension: topological_dimension(&self.geometry),
            coordinate_dimension: if empty { None } else { Some(2) },
            spatial_dimension: if empty { None } else { Some(2) },
            is_empty: empty,
            is_simple: is_simple(&self.geometry),
            has_serialization: self.to_wkt_literal(),
        }
    }
}

/// The topological dimension of a geometry per OGC SFA: 0 for points, 1 for
/// curves, 2 for surfaces, and the MAXIMUM over a collection's members. `None`
/// for the empty geometry (it has no topological dimension).
pub fn topological_dimension(g: &Geometry<f64>) -> Option<u8> {
    match g {
        Geometry::Point(_) => Some(0),
        Geometry::Line(_) | Geometry::LineString(_) => Some(1),
        Geometry::Polygon(_) | Geometry::Rect(_) | Geometry::Triangle(_) => Some(2),
        // The geo-types empty representations have an empty coordinate set; an
        // empty MULTI* contributes no dimension.
        Geometry::MultiPoint(mp) => (!mp.0.is_empty()).then_some(0),
        Geometry::MultiLineString(mls) => (!mls.0.iter().all(|l| l.0.is_empty())).then_some(1),
        Geometry::MultiPolygon(mp) => (!mp.0.is_empty()).then_some(2),
        Geometry::GeometryCollection(gc) => gc.0.iter().filter_map(topological_dimension).max(),
    }
}

/// `geo:coordinateDimension` as a standalone pure fn: measurements per
/// coordinate — always 2 in this 2-D Simple-Features profile, `None` for the
/// empty geometry (same rule as [`GeoGeometry::metadata`]'s field; the
/// `geof_accessors` differential tests pin the correspondence).
/// sq-lsp7k
#[cfg(feature = "geof_accessors")]
pub fn coordinate_dimension(g: &Geometry<f64>) -> Option<u8> {
    (!g.is_empty()).then_some(2)
}

/// `geo:spatialDimension` as a standalone pure fn: SPATIAL measurements per
/// coordinate (excludes the M measure) — always 2 here, `None` for the empty
/// geometry (same rule as [`GeoGeometry::metadata`]'s field; the
/// `geof_accessors` differential tests pin the correspondence).
/// sq-lsp7k
#[cfg(feature = "geof_accessors")]
pub fn spatial_dimension(g: &Geometry<f64>) -> Option<u8> {
    (!g.is_empty()).then_some(2)
}

/// The OGC Simple Features geometry-class IRI (`sf:Point`, `sf:LineString`, …)
/// of a parsed geometry — the value the opt-in `geof:geometryType` accessor
/// returns. sq-lsp7k
///
/// `None` — mapped by the registry to an honest `GeoError::Unsupported`
/// expression error, never a fabricated IRI — for:
///
/// * an **EMPTY** geometry: geo-types canonicalises empties (`POINT EMPTY`
///   parses to an empty `MultiPoint` — there is no empty point), so the
///   AUTHORED class of an empty literal is unrecoverable from the parsed
///   representation and any answer would be a guess;
/// * a **`GEOMETRYCOLLECTION`**: conservatively unmapped in this profile
///   (the accessor targets the instantiable simple-features classes).
///
/// The geo-types-only variants map to their exact simple-features class: a
/// `Line` IS a two-point `sf:LineString`; `Rect` / `Triangle` ARE
/// `sf:Polygon`s (none of the three is producible by this crate's WKT/GML
/// parsers, so the registry path never sees them).
#[cfg(feature = "geof_accessors")]
pub fn sf_geometry_type(g: &Geometry<f64>) -> Option<&'static str> {
    if g.is_empty() {
        return None;
    }
    match g {
        Geometry::Point(_) => Some("http://www.opengis.net/ont/sf#Point"),
        Geometry::Line(_) | Geometry::LineString(_) => {
            Some("http://www.opengis.net/ont/sf#LineString")
        }
        Geometry::Polygon(_) | Geometry::Rect(_) | Geometry::Triangle(_) => {
            Some("http://www.opengis.net/ont/sf#Polygon")
        }
        Geometry::MultiPoint(_) => Some("http://www.opengis.net/ont/sf#MultiPoint"),
        Geometry::MultiLineString(_) => Some("http://www.opengis.net/ont/sf#MultiLineString"),
        Geometry::MultiPolygon(_) => Some("http://www.opengis.net/ont/sf#MultiPolygon"),
        Geometry::GeometryCollection(_) => None,
    }
}

/// `geo:isSimple` (OGC SFA "Simple"): whether the geometry has no anomalous
/// geometric points such as self-intersection or self-tangency.
///
/// Per the Simple-Features definitions:
///
/// * a **point** is always simple; a **MultiPoint** is simple iff no two of its
///   points are equal;
/// * a **curve** (Line / LineString) is simple iff it does not pass through the
///   same point twice — with the exception that a *closed* curve (first vertex
///   = last vertex) is simple if that is its ONLY repeat; a **MultiCurve** is
///   simple iff every member is simple and members meet only at points that are
///   on the boundary of both;
/// * **surfaces** (Polygon / MultiPolygon / Rect / Triangle) are simple by
///   definition in the SF model (self-intersection of a surface is a *validity*
///   concern, exposed separately) — reported `true` here;
/// * an **empty** geometry is simple;
/// * a **GeometryCollection** is simple iff all of its members are simple.
pub fn is_simple(g: &Geometry<f64>) -> bool {
    match g {
        Geometry::Point(_) => true,
        Geometry::MultiPoint(mp) => {
            // No two points coincide.
            let coords: Vec<Coord<f64>> = mp.0.iter().map(|p| p.0).collect();
            !has_duplicate_coord(&coords)
        }
        Geometry::Line(l) => l.start != l.end,
        Geometry::LineString(ls) => linestring_is_simple(ls),
        Geometry::MultiLineString(mls) => mls.0.iter().all(linestring_is_simple),
        Geometry::Polygon(_)
        | Geometry::MultiPolygon(_)
        | Geometry::Rect(_)
        | Geometry::Triangle(_) => true,
        Geometry::GeometryCollection(gc) => gc.0.iter().all(is_simple),
    }
}

/// Whether two coordinates coincide (within a tiny epsilon, matching the rest of
/// the crate's robust coordinate comparison).
fn coord_eq(a: Coord<f64>, b: Coord<f64>) -> bool {
    const EPS: f64 = 1e-12;
    (a.x - b.x).abs() <= EPS && (a.y - b.y).abs() <= EPS
}

/// Whether `coords` contains two coincident coordinates.
fn has_duplicate_coord(coords: &[Coord<f64>]) -> bool {
    for (i, a) in coords.iter().enumerate() {
        for b in &coords[i + 1..] {
            if coord_eq(*a, *b) {
                return true;
            }
        }
    }
    false
}

/// A single curve is simple iff it does not pass through any point more than
/// once — except that a CLOSED curve (first vertex = last vertex) may repeat
/// that one shared endpoint. Implemented by checking every distinct pair of
/// segments for an intersection that is not a permitted shared endpoint of
/// adjacent segments / the ring closure.
fn linestring_is_simple(ls: &LineString<f64>) -> bool {
    use geo::line_intersection::line_intersection;
    use geo_types::Line;

    let pts = &ls.0;
    // Degenerate / trivial curves: fewer than two distinct points is simple.
    if pts.len() < 2 {
        return true;
    }
    let closed = pts.len() >= 2 && coord_eq(pts[0], *pts.last().unwrap());

    // A repeated VERTEX (other than the permitted closing one) is a self-touch.
    let interior_end = if closed { pts.len() - 1 } else { pts.len() };
    if has_duplicate_coord(&pts[..interior_end]) {
        return false;
    }

    let segs: Vec<Line<f64>> = ls.lines().filter(|s| s.start != s.end).collect();
    let n = segs.len();
    for i in 0..n {
        for j in (i + 1)..n {
            let adjacent = j == i + 1;
            // For a closed ring, the first and last segments share the closure
            // vertex and are allowed to meet there.
            let ring_ends = closed && i == 0 && j == n - 1;
            match line_intersection(segs[i], segs[j]) {
                None => {}
                Some(geo::LineIntersection::Collinear { .. }) => return false,
                Some(geo::LineIntersection::SinglePoint { intersection, .. }) => {
                    // A single shared point is permitted ONLY at the joint of two
                    // consecutive segments, or at the closure of a ring.
                    let permitted = (adjacent
                        && coord_eq(intersection, segs[i].end)
                        && coord_eq(intersection, segs[j].start))
                        || (ring_ends
                            && coord_eq(intersection, segs[0].start)
                            && coord_eq(intersection, segs[n - 1].end));
                    if !permitted {
                        return false;
                    }
                }
            }
        }
    }
    true
}

/// Every metadata property at the LEXICAL level: a `geo:wktLiteral` /
/// `geo:gmlLiteral` lexical form in, the property value out — the shape a SPARQL
/// engine builtin or a triple-materialising loader consumes. Each parses the
/// geometry via [`crate::parse_geometry_literal`] and reads off
/// [`GeoGeometry::metadata`]. sq-mzmh
pub mod lex {
    use super::GeometryMetadata;
    use crate::{parse_geometry_literal, GeoError};

    /// All metadata values for a geometry literal, dispatched by its datatype
    /// (`geo:wktLiteral` or `geo:gmlLiteral`).
    pub fn metadata(value: &str, datatype: &str) -> Result<GeometryMetadata, GeoError> {
        Ok(parse_geometry_literal(value, datatype)?.metadata())
    }

    /// `geo:dimension` — the topological dimension (`None`/unbound for the empty
    /// geometry).
    pub fn dimension(value: &str, datatype: &str) -> Result<Option<u8>, GeoError> {
        Ok(parse_geometry_literal(value, datatype)?
            .metadata()
            .dimension)
    }

    /// `geo:coordinateDimension` — measurements per coordinate (2 in this 2-D
    /// SF profile; `None` for the empty geometry).
    pub fn coordinate_dimension(value: &str, datatype: &str) -> Result<Option<u8>, GeoError> {
        Ok(parse_geometry_literal(value, datatype)?
            .metadata()
            .coordinate_dimension)
    }

    /// `geo:spatialDimension` — spatial measurements per coordinate (2 here;
    /// `None` for the empty geometry).
    pub fn spatial_dimension(value: &str, datatype: &str) -> Result<Option<u8>, GeoError> {
        Ok(parse_geometry_literal(value, datatype)?
            .metadata()
            .spatial_dimension)
    }

    /// `geo:isEmpty` — whether the geometry is the empty set.
    pub fn is_empty(value: &str, datatype: &str) -> Result<bool, GeoError> {
        Ok(parse_geometry_literal(value, datatype)?.metadata().is_empty)
    }

    /// `geo:isSimple` — whether the geometry has no anomalous self-intersection.
    pub fn is_simple(value: &str, datatype: &str) -> Result<bool, GeoError> {
        Ok(parse_geometry_literal(value, datatype)?
            .metadata()
            .is_simple)
    }

    /// `geo:hasSerialization` — the geometry's `geo:wktLiteral` lexical form.
    pub fn has_serialization(value: &str, datatype: &str) -> Result<String, GeoError> {
        Ok(parse_geometry_literal(value, datatype)?
            .metadata()
            .has_serialization)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse_wkt_literal;

    fn meta(wkt: &str) -> GeometryMetadata {
        parse_wkt_literal(wkt).unwrap().metadata()
    }

    #[test]
    fn dimension_by_geometry_kind() {
        assert_eq!(meta("POINT(1 2)").dimension, Some(0));
        assert_eq!(meta("LINESTRING(0 0, 1 1)").dimension, Some(1));
        assert_eq!(
            meta("POLYGON((0 0, 2 0, 2 2, 0 2, 0 0))").dimension,
            Some(2)
        );
        assert_eq!(meta("MULTIPOINT((0 0),(1 1))").dimension, Some(0));
        // A collection takes the MAX member dimension.
        assert_eq!(
            meta("GEOMETRYCOLLECTION(POINT(0 0), POLYGON((0 0,1 0,1 1,0 1,0 0)))").dimension,
            Some(2)
        );
    }

    #[test]
    fn coordinate_and_spatial_dimension_are_two() {
        let m = meta("POINT(1 2)");
        assert_eq!(m.coordinate_dimension, Some(2));
        assert_eq!(m.spatial_dimension, Some(2));
    }

    #[test]
    fn empty_geometry_has_no_dimension() {
        let m = meta("GEOMETRYCOLLECTION EMPTY");
        assert!(m.is_empty);
        assert_eq!(m.dimension, None);
        assert_eq!(m.coordinate_dimension, None);
        assert_eq!(m.spatial_dimension, None);
    }

    #[test]
    fn non_empty_is_not_empty() {
        assert!(!meta("POINT(1 2)").is_empty);
        assert!(!meta("LINESTRING(0 0, 1 1)").is_empty);
    }

    #[test]
    fn has_serialization_round_trips_to_wkt() {
        let m = meta("POINT(1 2)");
        assert!(m.has_serialization.contains("POINT"));
        // The serialization re-parses to the same geometry.
        assert_eq!(
            parse_wkt_literal(&m.has_serialization).unwrap().geometry,
            parse_wkt_literal("POINT(1 2)").unwrap().geometry
        );
    }

    #[test]
    fn simple_points_and_multipoints() {
        assert!(meta("POINT(1 2)").is_simple);
        assert!(meta("MULTIPOINT((0 0),(1 1))").is_simple);
        // A MultiPoint with a coincident pair is NOT simple.
        assert!(!meta("MULTIPOINT((0 0),(0 0))").is_simple);
    }

    #[test]
    fn simple_linestrings() {
        // A plain open curve is simple.
        assert!(meta("LINESTRING(0 0, 1 0, 1 1)").is_simple);
        // A closed ring is simple (the closure vertex repeat is permitted).
        assert!(meta("LINESTRING(0 0, 1 0, 1 1, 0 1, 0 0)").is_simple);
        // A self-crossing "bowtie" curve is NOT simple.
        assert!(!meta("LINESTRING(0 0, 2 2, 0 2, 2 0)").is_simple);
        // A curve that revisits an interior vertex is NOT simple.
        assert!(!meta("LINESTRING(0 0, 1 1, 2 0, 1 1, 0 2)").is_simple);
    }

    #[test]
    fn surfaces_are_simple() {
        assert!(meta("POLYGON((0 0, 2 0, 2 2, 0 2, 0 0))").is_simple);
        assert!(meta("MULTIPOLYGON(((0 0,1 0,1 1,0 1,0 0)),((2 2,3 2,3 3,2 3,2 2)))").is_simple);
    }

    #[test]
    fn lex_mirror_dispatches_by_datatype() {
        use crate::vocab::WKT_LITERAL;
        assert_eq!(lex::dimension("POINT(1 2)", WKT_LITERAL).unwrap(), Some(0));
        assert!(!lex::is_empty("POINT(1 2)", WKT_LITERAL).unwrap());
        assert!(!lex::is_simple("LINESTRING(0 0, 2 2, 0 2, 2 0)", WKT_LITERAL).unwrap());
        assert_eq!(
            lex::coordinate_dimension("POINT(1 2)", WKT_LITERAL).unwrap(),
            Some(2)
        );
        assert!(lex::has_serialization("POINT(1 2)", WKT_LITERAL)
            .unwrap()
            .contains("POINT"));
        // GML rides the same path.
        let gml = "<gml:Point xmlns:gml=\"http://www.opengis.net/gml\">\
                   <gml:pos>1 2</gml:pos></gml:Point>";
        assert_eq!(
            lex::dimension(gml, crate::vocab::GML_LITERAL).unwrap(),
            Some(0)
        );
    }

    #[test]
    fn lex_metadata_aggregate_returns_all_values() {
        // sq-9g58 coverage: the aggregate `lex::metadata` helper (the
        // single-call form a materialising loader uses) — distinct from the
        // per-property `lex::dimension` etc. that the test above drives.
        use crate::vocab::WKT_LITERAL;
        let m = lex::metadata("POLYGON((0 0, 2 0, 2 2, 0 2, 0 0))", WKT_LITERAL).unwrap();
        assert_eq!(m.dimension, Some(2));
        assert_eq!(m.coordinate_dimension, Some(2));
        assert_eq!(m.spatial_dimension, Some(2));
        assert!(!m.is_empty);
        assert!(m.is_simple);
        assert!(m.has_serialization.contains("POLYGON"));
    }

    #[test]
    fn degenerate_linestring_is_simple() {
        // sq-9g58 coverage: linestring_is_simple's `< 2 points`
        // early-return arm. An EMPTY linestring has no vertices and is simple.
        assert!(meta("LINESTRING EMPTY").is_simple);
    }

    // ---- the standalone `geof_accessors` pure fns (sq-lsp7k) ----

    #[cfg(feature = "geof_accessors")]
    #[test]
    fn standalone_coordinate_dimension_matches_metadata_field() {
        for (lex, expected) in [
            ("POINT(1 2)", Some(2)),
            ("LINESTRING(0 0, 1 1)", Some(2)),
            ("POINT EMPTY", None),
            ("GEOMETRYCOLLECTION EMPTY", None),
        ] {
            let g = parse_wkt_literal(lex).unwrap();
            assert_eq!(coordinate_dimension(&g.geometry), expected, "coordinate_dimension({lex})");
            // Single-source pin: the standalone fn IS the metadata() field.
            assert_eq!(coordinate_dimension(&g.geometry), g.metadata().coordinate_dimension);
        }
    }

    #[cfg(feature = "geof_accessors")]
    #[test]
    fn standalone_spatial_dimension_matches_metadata_field() {
        for (lex, expected) in [
            ("POLYGON((0 0, 2 0, 2 2, 0 2, 0 0))", Some(2)),
            ("MULTIPOINT((0 0),(1 1))", Some(2)),
            ("LINESTRING EMPTY", None),
        ] {
            let g = parse_wkt_literal(lex).unwrap();
            assert_eq!(spatial_dimension(&g.geometry), expected, "spatial_dimension({lex})");
            assert_eq!(spatial_dimension(&g.geometry), g.metadata().spatial_dimension);
        }
    }

    #[cfg(feature = "geof_accessors")]
    #[test]
    fn sf_geometry_type_maps_each_parsed_class_and_declines_undefined() {
        const SF: &str = "http://www.opengis.net/ont/sf#";
        for (lex, expected) in [
            ("POINT(1 2)", Some("Point")),
            ("LINESTRING(0 0, 1 1)", Some("LineString")),
            ("POLYGON((0 0, 2 0, 2 2, 0 2, 0 0))", Some("Polygon")),
            ("MULTIPOINT((0 0),(1 1))", Some("MultiPoint")),
            ("MULTILINESTRING((0 0, 1 1),(2 2, 3 3))", Some("MultiLineString")),
            ("MULTIPOLYGON(((0 0,1 0,1 1,0 1,0 0)))", Some("MultiPolygon")),
            // No fabricated class: collections are unmapped, and an empty
            // literal's authored class is unrecoverable (POINT EMPTY parses
            // to an empty MultiPoint), so both decline with None.
            ("GEOMETRYCOLLECTION(POINT(0 0), POLYGON((0 0,1 0,1 1,0 1,0 0)))", None),
            ("POINT EMPTY", None),
            ("POLYGON EMPTY", None),
            ("GEOMETRYCOLLECTION EMPTY", None),
        ] {
            assert_eq!(
                sf_geometry_type(&parse_wkt_literal(lex).unwrap().geometry),
                expected.map(|c| format!("{SF}{c}")).as_deref(),
                "sf_geometry_type({lex})"
            );
        }
        // The geo-types-only variants (never produced by the WKT/GML parsers)
        // map to their exact simple-features class.
        use geo_types::{coord, Line, Rect, Triangle};
        let line = Geometry::Line(Line::new(coord! {x: 0., y: 0.}, coord! {x: 1., y: 1.}));
        assert_eq!(sf_geometry_type(&line), Some("http://www.opengis.net/ont/sf#LineString"));
        let rect = Geometry::Rect(Rect::new(coord! {x: 0., y: 0.}, coord! {x: 1., y: 1.}));
        assert_eq!(sf_geometry_type(&rect), Some("http://www.opengis.net/ont/sf#Polygon"));
        let tri = Geometry::Triangle(Triangle::new(
            coord! {x: 0., y: 0.},
            coord! {x: 1., y: 0.},
            coord! {x: 0., y: 1.},
        ));
        assert_eq!(sf_geometry_type(&tri), Some("http://www.opengis.net/ont/sf#Polygon"));
    }

    #[test]
    fn collinear_self_overlapping_linestring_is_not_simple() {
        // sq-9g58 coverage: linestring_is_simple's COLLINEAR
        // intersection arm — a curve whose segments overlap along a shared line
        // (it doubles back over itself) is NOT simple.
        // (0 0)->(3 0)->(1 0): the second segment lies ON the first.
        assert!(!meta("LINESTRING(0 0, 3 0, 1 0)").is_simple);
    }
}
