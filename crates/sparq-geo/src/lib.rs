// sq-wo5jw: the README front page embeds a RUNNABLE quickstart driving
// `geof_registry` through `sparq_engine::query_with_functions` — an `engine`-feature
// surface. Attached unconditionally, that doctest failed to COMPILE under
// `--no-default-features` (its imports don't exist there). The README — and its
// still-executed doctest — is therefore attached only when `engine` is on (the
// default, and what docs.rs renders: all-features); the engine-less build gets the
// short pure-geometry crate doc below instead of a broken example. The quickstart
// stays a real, executed doctest in the default state (never demoted to `ignore`).
#![cfg_attr(feature = "engine", doc = include_str!("../README.md"))]
#![cfg_attr(
    not(feature = "engine"),
    doc = "Opt-in GeoSPARQL 1.0/1.1 core for the sparq RDF engine, built WITHOUT the",
    doc = "default `engine` feature: the pure geometry library — `geo:wktLiteral` /",
    doc = "`geo:gmlLiteral` parsing, the `geof` module's functions as plain Rust, and",
    doc = "the R-tree `GeoIndex` — with no SPARQL engine in the dependency graph.",
    doc = "",
    doc = "The `geof_registry` SPARQL `FILTER`/`BIND` packaging requires the `engine`",
    doc = "feature; the crate README quickstart exercises it, so the README front page",
    doc = "(a runnable doctest) is attached only when `engine` is enabled. docs.rs",
    doc = "builds all features and always renders the full README."
)]
#![forbid(unsafe_code)] // sq-emay: crate has zero `unsafe`

pub mod geof;
pub mod gml; // sq-zy0: GML-SF geometry parser (geo:gmlLiteral)
pub mod index;
pub mod literal;
pub mod metadata; // sq-mzmh: geo:dimension/isEmpty/isSimple/… metadata (OGC R9)
#[cfg(feature = "engine")]
pub mod provider;
#[cfg(feature = "engine")]
pub mod registry;
#[cfg(feature = "reproject")]
pub mod reproject;
// sq-9g58: GeoSPARQL query-rewrite extension (topology property forms).
// sq-wf9qg: behind the OPT-IN `geosparql_rewrite` feature (OFF by default),
// NOT the default-on `engine` feature — a default build never exposes the rewrite entry
// point, so the STANDARD SPARQL behaviour stays untouched. The feature implies `engine`,
// so the rewrite's `sparq_engine` dependency is always satisfied here.
#[cfg(feature = "geosparql_rewrite")]
pub mod rewrite;

pub use geof::Unit;
pub use gml::parse_gml_literal; //
pub use index::GeoIndex;
pub use literal::{
    is_geometry_datatype, parse_geometry_literal, parse_wkt_literal, Crs, GeoGeometry,
}; //
pub use metadata::GeometryMetadata; // sq-mzmh
#[cfg(feature = "engine")]
pub use provider::GeoIndexProvider;
#[cfg(feature = "engine")]
pub use registry::geof_registry;
// sq-wf9qg: the rewrite surface is exported only under the opt-in
// `geosparql_rewrite` feature, so a default (engine-only) build neither compiles nor
// re-exports it — the rewrite is a strict opt-in superset over standard SPARQL.
#[cfg(feature = "geosparql_rewrite")]
pub use rewrite::{geosparql_rewrite, is_topology_property, rewrite_query}; // sq-9g58

/// The GeoSPARQL vocabulary IRIs this crate touches.
pub mod vocab {
    /// `geo:` — the GeoSPARQL ontology namespace.
    pub const GEO_NS: &str = "http://www.opengis.net/ont/geosparql#";
    /// `geo:wktLiteral` — the WKT serialization datatype (GeoSPARQL 8.5.1).
    pub const WKT_LITERAL: &str = "http://www.opengis.net/ont/geosparql#wktLiteral";
    /// `geo:gmlLiteral` — the GML serialization datatype (GeoSPARQL 8.5.2).
    pub const GML_LITERAL: &str = "http://www.opengis.net/ont/geosparql#gmlLiteral";
    /// `gml:` — the GML 3.2 namespace (GML-SF geometry elements).
    pub const GML_NS: &str = "http://www.opengis.net/gml/3.2";
    /// `geo:asWKT` — geometry node -> WKT serialization (GeoSPARQL 8.5.2).
    pub const AS_WKT: &str = "http://www.opengis.net/ont/geosparql#asWKT";
    /// `geo:asGML` — geometry node -> GML serialization (GeoSPARQL 8.5.2).
    pub const AS_GML: &str = "http://www.opengis.net/ont/geosparql#asGML";
    /// `geo:hasGeometry` — feature -> geometry node (GeoSPARQL 8.3).
    pub const HAS_GEOMETRY: &str = "http://www.opengis.net/ont/geosparql#hasGeometry";
    /// `geo:hasDefaultGeometry` — feature -> default geometry node (GeoSPARQL 8.3).
    pub const HAS_DEFAULT_GEOMETRY: &str = "http://www.opengis.net/ont/geosparql#hasDefaultGeometry";
    // ---- geometry-METADATA properties (GeoSPARQL 8.4; OGC R9). sq-mzmh ----
    /// `geo:dimension` — topological dimension of a geometry (`xsd:integer`).
    pub const DIMENSION: &str = "http://www.opengis.net/ont/geosparql#dimension";
    /// `geo:coordinateDimension` — coordinate measurements per point (`xsd:integer`).
    pub const COORDINATE_DIMENSION: &str =
        "http://www.opengis.net/ont/geosparql#coordinateDimension";
    /// `geo:spatialDimension` — spatial measurements per point (`xsd:integer`).
    pub const SPATIAL_DIMENSION: &str = "http://www.opengis.net/ont/geosparql#spatialDimension";
    /// `geo:isEmpty` — whether the geometry is the empty set (`xsd:boolean`).
    pub const IS_EMPTY: &str = "http://www.opengis.net/ont/geosparql#isEmpty";
    /// `geo:isSimple` — whether the geometry has no self-intersection (`xsd:boolean`).
    pub const IS_SIMPLE: &str = "http://www.opengis.net/ont/geosparql#isSimple";
    /// `geo:hasSerialization` — a serialization of the geometry (`geo:wktLiteral`).
    pub const HAS_SERIALIZATION: &str = "http://www.opengis.net/ont/geosparql#hasSerialization";
    /// `geof:` — the GeoSPARQL function namespace.
    pub const GEOF_NS: &str = "http://www.opengis.net/def/function/geosparql/";
    /// `sf:` — the OGC Simple Features geometry-class namespace (`sf:Point`,
    /// `sf:Polygon`, …), the value space of the opt-in `geof:geometryType`
    /// accessor (`geof_accessors` feature). sq-lsp7k
    #[cfg(feature = "geof_accessors")]
    pub const SF_NS: &str = "http://www.opengis.net/ont/sf#";
    /// `uom:` — the OGC units-of-measure namespace used by `geof:distance`.
    pub const UOM_NS: &str = "http://www.opengis.net/def/uom/OGC/1.0/";
    /// CRS84 (WGS84 long/lat) — the GeoSPARQL DEFAULT CRS for wktLiterals.
    pub const CRS84: &str = "http://www.opengis.net/def/crs/OGC/1.3/CRS84";
    /// EPSG:4326 (WGS84 LAT/LONG axis order).
    pub const EPSG_4326: &str = "http://www.opengis.net/def/crs/EPSG/0/4326";
}

/// Errors from WKT/GML parsing, `geof:` evaluation, or index queries.
#[derive(Debug, Clone, PartialEq)]
pub enum GeoError {
    /// A geometry literal lexical form (wktLiteral CRS prefix / WKT body, or a
    /// gmlLiteral GML element) failed to parse.
    Parse(String),
    /// The two arguments are in incompatible CRSs (no transformation in v1).
    CrsMismatch(String, String),
    /// The operation needs a geographic CRS (CRS84 / EPSG:4326), e.g. metric distance.
    NonGeographicCrs(String),
    /// The unit IRI passed to `geof:distance` is not recognised.
    UnknownUnit(String),
    /// The operation is not defined / not implemented for this geometry.
    Unsupported(String),
}

impl std::fmt::Display for GeoError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GeoError::Parse(m) => write!(f, "geometry parse error: {m}"),
            GeoError::CrsMismatch(a, b) => write!(f, "CRS mismatch: <{a}> vs <{b}>"),
            GeoError::NonGeographicCrs(c) => {
                write!(f, "operation requires a geographic CRS (CRS84/EPSG:4326), got <{c}>")
            }
            GeoError::UnknownUnit(u) => write!(f, "unknown unit of measure: <{u}>"),
            GeoError::Unsupported(m) => write!(f, "unsupported: {m}"),
        }
    }
}

impl std::error::Error for GeoError {}

#[cfg(test)]
mod error_display_tests {
    // sq-9g58 coverage: every GeoError Display arm renders its
    // payload. These arms (CrsMismatch / NonGeographicCrs / UnknownUnit) were
    // unexercised — Parse / Unsupported are hit by the parser/eval tests.
    use super::GeoError;

    #[test]
    fn each_variant_renders_its_payload() {
        let parse = GeoError::Parse("bad wkt".to_string()).to_string();
        assert!(parse.contains("bad wkt"), "{parse}");

        let crs = GeoError::CrsMismatch("urn:a".to_string(), "urn:b".to_string()).to_string();
        assert!(crs.contains("urn:a") && crs.contains("urn:b"), "{crs}");

        let non_geo = GeoError::NonGeographicCrs("urn:proj".to_string()).to_string();
        assert!(non_geo.contains("urn:proj") && non_geo.contains("geographic"), "{non_geo}");

        let unit = GeoError::UnknownUnit("urn:furlong".to_string()).to_string();
        assert!(unit.contains("urn:furlong") && unit.contains("unit"), "{unit}");

        let unsup = GeoError::Unsupported("no can do".to_string()).to_string();
        assert!(unsup.contains("no can do"), "{unsup}");

        // The error implements std::error::Error (trait-object usable).
        let _: &dyn std::error::Error = &GeoError::Parse("x".to_string());
    }
}
