//! `geo:wktLiteral` lexical forms <-> CRS-tagged geometries.
//!
//! GeoSPARQL (1.0 §8.5.1 / 1.1 §8.7) defines the wktLiteral lexical form as an
//! OPTIONAL leading CRS IRI in angle brackets followed by a WKT string:
//!
//! ```text
//! "<http://www.opengis.net/def/crs/OGC/1.3/CRS84> POINT(-83.4 34.3)"^^geo:wktLiteral
//! "POINT(-83.4 34.3)"^^geo:wktLiteral            # CRS defaults to CRS84
//! ```
//!
//! The WKT body is parsed with the `wkt` crate and converted into a
//! [`geo_types::Geometry<f64>`] (POINT, LINESTRING, POLYGON, the MULTI*
//! variants, and GEOMETRYCOLLECTION). Coordinates are normalised internally to
//! x = longitude / y = latitude: EPSG:4326 literals (LAT/LONG axis order per
//! the EPSG registry, which GeoSPARQL honours) are axis-swapped on parse and
//! swapped back on serialisation, so the rest of the crate computes in a
//! single coordinate convention.

use crate::{vocab, GeoError};
use geo::MapCoords;
use geo_types::{Coord, Geometry};
use wkt::{ToWkt, TryFromWkt};

/// The coordinate reference system of a wktLiteral.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Crs {
    /// `<http://www.opengis.net/def/crs/OGC/1.3/CRS84>` — WGS84 long/lat, the
    /// GeoSPARQL default when the literal carries no CRS IRI.
    Crs84,
    /// `<http://www.opengis.net/def/crs/EPSG/0/4326>` — WGS84 LAT/LONG axis order.
    Epsg4326,
    /// Any other CRS IRI: coordinates are kept exactly as written (no axis
    /// metadata, no transformation). Topological relations still work within
    /// the same CRS; metric distance does not (no projection support in v1).
    Other(String),
}

impl Crs {
    /// Classifies a CRS IRI (the text between `<` and `>`).
    pub fn from_iri(iri: &str) -> Crs {
        match iri {
            vocab::CRS84 => Crs::Crs84,
            vocab::EPSG_4326 => Crs::Epsg4326,
            other => Crs::Other(other.to_string()),
        }
    }

    /// The CRS IRI.
    pub fn iri(&self) -> &str {
        match self {
            Crs::Crs84 => vocab::CRS84,
            Crs::Epsg4326 => vocab::EPSG_4326,
            Crs::Other(iri) => iri,
        }
    }

    /// Whether coordinates are WGS84 degrees (so haversine metres are meaningful).
    pub fn is_geographic(&self) -> bool {
        matches!(self, Crs::Crs84 | Crs::Epsg4326)
    }
}

/// A parsed `geo:wktLiteral`: a geometry plus the CRS it was written in.
///
/// `geometry` coordinates are ALWAYS x = longitude / y = latitude for
/// geographic CRSs (EPSG:4326 input is axis-swapped on parse); for
/// [`Crs::Other`] they are whatever the source wrote.
#[derive(Debug, Clone, PartialEq)]
pub struct GeoGeometry {
    pub crs: Crs,
    pub geometry: Geometry<f64>,
}

/// Swaps x and y in every coordinate (EPSG:4326 lat/long <-> internal long/lat).
/// `pub(crate)` so the GML path can normalise EPSG:4326 axis order identically.
pub(crate) fn swap_xy(g: Geometry<f64>) -> Geometry<f64> {
    g.map_coords(|c| Coord { x: c.y, y: c.x })
}

/// Parses a `geo:wktLiteral` LEXICAL FORM (the literal's string value, without
/// the `^^geo:wktLiteral` datatype): an optional `<CRS-IRI>` prefix, then WKT.
///
/// Empty geometries parse to geo-types' empty representations (`POINT EMPTY`
/// becomes an empty `MultiPoint` — geo-types has no empty point); they have no
/// bounding box and are skipped by the [`GeoIndex`](crate::GeoIndex).
pub fn parse_wkt_literal(lex: &str) -> Result<GeoGeometry, GeoError> {
    let s = lex.trim_start();
    let (crs, body) = if let Some(rest) = s.strip_prefix('<') {
        let end = rest
            .find('>')
            .ok_or_else(|| GeoError::Parse("unterminated CRS IRI (missing '>')".to_string()))?;
        (Crs::from_iri(&rest[..end]), &rest[end + 1..])
    } else {
        (Crs::Crs84, s)
    };
    let geom = Geometry::<f64>::try_from_wkt_str(body.trim())
        .map_err(|e| GeoError::Parse(e.to_string()))?;
    let geometry = if crs == Crs::Epsg4326 { swap_xy(geom) } else { geom };
    Ok(GeoGeometry { crs, geometry })
}

/// Parses a geometry literal by its DATATYPE: `geo:wktLiteral` lexical forms via
/// [`parse_wkt_literal`], `geo:gmlLiteral` via
/// [`parse_gml_literal`](crate::gml::parse_gml_literal). Both yield the same
/// [`GeoGeometry`] (CRS-tagged, long/lat-normalised), so downstream `geof:`
/// functions and the [`GeoIndex`](crate::GeoIndex) treat the two serializations
/// identically (GeoSPARQL §8.5). Any other datatype is `GeoError::Unsupported`.
pub fn parse_geometry_literal(value: &str, datatype: &str) -> Result<GeoGeometry, GeoError> {
    match datatype {
        vocab::WKT_LITERAL => parse_wkt_literal(value),
        vocab::GML_LITERAL => crate::gml::parse_gml_literal(value),
        other => Err(GeoError::Unsupported(format!(
            "geometry datatype <{other}> is not a geo:wktLiteral or geo:gmlLiteral"
        ))),
    }
}

/// Whether a datatype IRI is one of the GeoSPARQL geometry serializations this
/// crate parses (`geo:wktLiteral` or `geo:gmlLiteral`).
pub fn is_geometry_datatype(datatype: &str) -> bool {
    datatype == vocab::WKT_LITERAL || datatype == vocab::GML_LITERAL
}

impl GeoGeometry {
    /// A geometry in the default CRS84.
    pub fn new(geometry: Geometry<f64>) -> GeoGeometry {
        GeoGeometry { crs: Crs::Crs84, geometry }
    }

    /// Serialises back to the wktLiteral lexical form. CRS84 (the default) is
    /// emitted WITHOUT a CRS prefix; EPSG:4326 is axis-swapped back to its
    /// lat/long order; any other CRS keeps its IRI prefix verbatim.
    pub fn to_wkt_literal(&self) -> String {
        let body = match self.crs {
            Crs::Epsg4326 => swap_xy(self.geometry.clone()).wkt_string(),
            _ => self.geometry.wkt_string(),
        };
        match &self.crs {
            Crs::Crs84 => body,
            crs => format!("<{}> {}", crs.iri(), body),
        }
    }

    /// Serialises to a `geo:gmlLiteral` lexical form in the GML 3 Simple
    /// Features profile.
    ///
    /// The root geometry carries its CRS in `srsName`. EPSG:4326 coordinates
    /// are axis-swapped back to latitude/longitude, mirroring
    /// [`parse_gml_literal`](crate::parse_gml_literal); all other CRS coordinate
    /// orders are preserved. sq-i3wb5
    pub fn to_gml_literal(&self) -> String {
        crate::gml::serialize_gml_literal(self)
    }
}
