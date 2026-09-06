//! Opt-in CRS reprojection (`reproject` cargo feature): transform a
//! [`GeoGeometry`] written in a PROJECTED or non-default GEOGRAPHIC CRS into
//! CRS84 (WGS84 long/lat), so it can join the geographic machinery — metric
//! `geof:distance`, the [`GeoIndex`](crate::GeoIndex), buffer in metres.
//!
//! Transformation is pure Rust via [`proj4rs`] (a Rust port of proj.4 — NOT
//! the C `proj` library, keeping the workspace pure-Rust and wasm-friendly).
//! proj4rs evaluates proj-strings but ships no EPSG database, so this module
//! carries a SMALL CURATED TABLE of proj4 definitions ([`proj4_definition`])
//! for the CRSs that actually show up in published RDF.
//!
//! Projected CRSs (metre-valued coordinates):
//!
//! - EPSG:27700 — British National Grid (OSGB36, 7-param Helmert; verified
//!   against the Ordnance Survey worked example to ~1e-6°),
//! - EPSG:3857 — Web Mercator,
//! - EPSG:2154 — RGF93 / Lambert-93 (France),
//! - EPSG:25832 / 25833 — ETRS89 / UTM 32N, 33N (German/Nordic open data),
//! - EPSG:32601–32660 and 32701–32760 — WGS84 / UTM north + south zones.
//!
//! Geographic CRSs (degree-valued coordinates, sq-ove): the EPSG registry
//! defines these with LAT/LONG axis order, which GeoSPARQL honours — the
//! EPSG:4326 convention, but parsing only axis-normalises 4326 itself (other
//! codes are [`Crs::Other`], kept verbatim). The curated axis-order map
//! ([`geographic_axis_order`]) lets [`to_crs84`] normalise them too:
//!
//! - EPSG:4258 — ETRS89, EPSG:4269 — NAD83, EPSG:4283 — GDA94,
//!   EPSG:4171 — RGF93, EPSG:4490 — CGCS2000 (all lat/long).
//!
//! These datums are treated as COINCIDENT with WGS84 (`+towgs84=0,0,0`) — the
//! standard proj convention, accurate to a metre or two (plate motion since
//! each datum's epoch). EPSG:4267 (NAD27) is deliberately NOT curated: a
//! correct NAD27 transform needs NADCON grid shifts (tens of metres), which
//! this crate does not ship — it stays an explicit [`GeoError::Unsupported`].
//!
//! CRS84 / EPSG:4326 inputs pass through unchanged (parsing already
//! normalised them to long/lat). Anything else is a
//! [`GeoError::Unsupported`] naming the missing EPSG code — extend the table
//! as needed; correctness of a new entry is one worked example away.
//!
//! # Full EPSG registry (`epsg_full` feature, sq-248)
//!
//! The additional opt-in `epsg_full` feature (implies `reproject`) embeds the
//! full EPSG registry — the CC0-licensed `crs_definitions` crate's proj4
//! strings, ~6k codes — as a FALLBACK consulted only when the curated table
//! has no entry (the curated table always wins: its definitions are verified
//! against worked examples and never drift with a data-crate bump). The
//! fallback stays honest rather than maximal — a registry definition is
//! refused (the code stays [`GeoError::Unsupported`]) when its transform
//! would be unevaluable or silently wrong with the machinery this crate
//! ships:
//!
//! - grid-shift datums (`+datum=NAD27`) and real `+nadgrids=` files — proj4rs
//!   ships no shift grids, so these cannot be evaluated correctly
//!   (`+nadgrids=@null` is fine: it *disables* the shift, Web-Mercator
//!   style);
//! - definitions with NO datum information (`+towgs84`/`+datum`/`+nadgrids`)
//!   on a non-WGS84-coincident ellipsoid — proj4 semantics would silently
//!   skip the datum shift, off by up to hundreds of metres. GRS80/WGS84
//!   ellipsoids without datum info are accepted (coincident at metre level,
//!   same convention as the curated geographic entries);
//! - PROJECTED definitions whose official coordinate order the registry
//!   cannot PROVE to agree with what proj4rs will read. A PROJ.4 string does
//!   carry ONE piece of axis metadata — the optional `+axis=` parameter
//!   (default `enu`), which proj4rs parses and applies as the directions of
//!   the coordinates it is HANDED — but nothing that fixes the EPSG axis
//!   ORDER, and the EPSG registry defines many projected CRSs northing-first
//!   (Poland CS92, the ETRS89 "(N-E)" UTM variants, the Gauss-Krüger
//!   families), which verbatim coordinates would silently transpose. The
//!   registry's parallel WKT definitions DO carry the order: a projected
//!   entry is accepted only when its first two `AXIS[…,DIRECTION]` nodes
//!   equal the directions its own `+axis=` parameter declares, in that order
//!   (issue #3752). So:
//!
//!   - `+axis` absent (`enu`) + `AXIS[…,EAST],AXIS[…,NORTH]` — the common
//!     case, accepted; a no-AXIS projected entry stays refused. The registry
//!     data is PostGIS `spatial_ref_sys` (`srtext`/`proj4text`), whose WKT1
//!     export omits its AXIS nodes almost exactly when the official order is
//!     not the GIS-traditional easting/northing — VERIFIED against the EPSG
//!     registry's coordinate-system axis table (EPSG v11.022): of the 1249
//!     projected no-AXIS entries, 1241 are officially northing/easting with
//!     zero exceptions, and the other 8 are the entries whose `wkt` string is
//!     EMPTY (so they carry no axis evidence at all) and are officially
//!     easting/northing. Refusing the whole bucket is therefore correct, and
//!     "no AXIS means northing-first" is NOT a safe blanket rule — those 8
//!     are refused by this axis rule alone. See
//!     `research/epsg-no-axis-projected-axis-order.md` (issue #4284);
//!   - `+axis=wsu` + `AXIS[…,WEST],AXIS[…,SOUTH]` — the 28 southern-African
//!     Gauss-conformal "Lo" grids (Hartebeesthoek94, Cape, Schwarzeck), and
//!     `+axis=swu` + `AXIS[…,SOUTH],AXIS[…,WEST]` — EPSG:8352 (S-JTSK
//!     \[JTSK03\] / Krovak). proj4rs negates each declared axis before the
//!     inverse projection, so these need NO coordinate rewriting here: the
//!     wktLiteral's (westing, southing) / (southing, westing) pair is exactly
//!     what proj4rs consumes;
//!   - anything else refused — including a WKT that declares south/west order
//!     while the proj4 string forgot the matching `+axis=` (EPSG:2065, 5513,
//!     8044, 8045: the export's `proj4text` column is the east/north
//!     Krovak/Cassini variant while its `srtext` column is the official
//!     south/west one, so the two disagree and neither can be trusted), and
//!     the two transposed entries EPSG:8433 (Macao Grid) and EPSG:8441
//!     (Tananarive / Laborde Grid), whose WKT declares `NORTH,EAST` against
//!     an `enu` proj4 string. A transposed pair COULD be normalised by
//!     swapping (x, y) before the transform, but neither of those two is
//!     evaluable here for an unrelated reason — 8433's definition carries no
//!     datum information on an International-1924 ellipsoid (refused by the
//!     datum rule above) and 8441 needs `+proj=labrd`, which proj4rs does not
//!     implement — so there is no entry a swap could correctly serve and no
//!     worked example that could regression-test one. The swap is therefore
//!     deliberately NOT implemented rather than shipped untested.
//!
//! Geographic (`+proj=longlat`) registry definitions follow the EPSG
//! lat/long axis-order convention for geographic 2D CRSs (the same rule as
//! every curated geographic entry) — unless the registry WKT explicitly
//! declares longitude first (the rare EPSG "(lon-lat)" variants), which
//! [`geographic_axis_order`] honours as [`AxisOrder::LongLat`].

use crate::literal::{Crs, GeoGeometry};
use crate::GeoError;
use geo::MapCoords;
use geo_types::Coord;
use proj4rs::Proj;

/// The coordinate axis order a GEOGRAPHIC CRS's authority definition uses for
/// its lexical forms (GeoSPARQL honours the authority order in wktLiterals).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AxisOrder {
    /// Latitude first (the EPSG registry convention for geographic 2D CRSs).
    LatLong,
    /// Longitude first (the OGC CRS84 / internal convention).
    LongLat,
}

/// The wktLiteral axis order of a curated GEOGRAPHIC (degree-valued) EPSG CRS.
///
/// `Some(order)` marks the code as geographic — [`to_crs84`] converts its
/// coordinates from `order` degrees into long/lat radians before the proj4
/// transform. `None` means "not a curated geographic CRS" (projected codes
/// like 27700 land here: their metre coordinates are fed to proj4 verbatim).
///
/// Hand-maintained alongside [`proj4_definition`]; every geographic entry
/// there must appear here (that pairing is unit-tested). Under the `epsg_full`
/// feature, a non-curated code whose (accepted) registry definition is
/// `+proj=longlat` is also geographic — `LatLong` per the EPSG convention for
/// geographic 2D CRSs, unless the registry WKT explicitly declares longitude
/// first (the rare EPSG "(lon-lat)" variants; see the module docs).
pub fn geographic_axis_order(epsg: u32) -> Option<AxisOrder> {
    match epsg {
        // WGS84 itself — axis-normalised on parse, listed for introspection.
        4326 => Some(AxisOrder::LatLong),
        // ETRS89, NAD83, GDA94, RGF93, CGCS2000 — EPSG geographic 2D,
        // lat/long per the registry.
        4258 | 4269 | 4283 | 4171 | 4490 => Some(AxisOrder::LatLong),
        #[cfg(feature = "epsg_full")]
        _ => full_epsg_definition(epsg).and_then(|d| {
            if !d.proj4.starts_with("+proj=longlat") {
                return None;
            }
            Some(match wkt_axis_directions(d.wkt) {
                Some(("EAST", "NORTH")) => AxisOrder::LongLat,
                _ => AxisOrder::LatLong,
            })
        }),
        #[cfg(not(feature = "epsg_full"))]
        _ => None,
    }
}

/// Full-registry fallback (`epsg_full`): the embedded definition for `epsg`,
/// or `None` when the registry has no entry OR its transform would be
/// unevaluable / silently wrong here — grid-shift datums (NAD27) and real
/// `+nadgrids=` files (proj4rs ships no shift grids), datum-less definitions
/// on a non-WGS84-coincident ellipsoid (proj4 would silently skip the datum
/// shift), and projected definitions whose WKT coordinate order does not
/// match the axis directions their own `+axis=` parameter declares to proj4rs
/// (verbatim coordinates would silently transpose northing-first CRSs). See
/// the module docs for the full policy.
#[cfg(feature = "epsg_full")]
fn full_epsg_definition(epsg: u32) -> Option<crs_definitions::Def> {
    let def = crs_definitions::from_code(u16::try_from(epsg).ok()?)?;
    let needs_grids = def.proj4.contains("+datum=NAD27")
        || def
            .proj4
            .split_ascii_whitespace()
            .any(|t| t.strip_prefix("+nadgrids=").is_some_and(|g| g != "@null"));
    if needs_grids {
        return None;
    }
    let has_datum_info =
        ["+towgs84=", "+datum=", "+nadgrids="].iter().any(|k| def.proj4.contains(k));
    let wgs84_coincident = def.proj4.contains("+ellps=GRS80") || def.proj4.contains("+ellps=WGS84");
    if !has_datum_info && !wgs84_coincident {
        return None;
    }
    // Projected axis-order gate (see the module docs): proj4rs reads the two
    // horizontal coordinates in the directions the proj-string's `+axis=`
    // parameter declares (default `enu` = easting/northing), so a registry
    // entry is safe to feed VERBATIM exactly when its WKT declares that same
    // pair of directions in that same order. Everything else — a no-AXIS
    // export, a transposed pair, or a WKT/proj4 disagreement — is refused.
    // Geographic definitions are axis-normalised by the caller via
    // `geographic_axis_order` instead.
    if !def.proj4.starts_with("+proj=longlat")
        && wkt_axis_directions(def.wkt) != proj4_axis_directions(def.proj4)
    {
        return None;
    }
    Some(def)
}

/// The directions proj4rs will read a projected definition's first two
/// coordinates in, from its `+axis=` parameter (absent means `enu`, i.e.
/// easting then northing) — e.g. `+axis=wsu` yields `Some(("WEST",
/// "SOUTH"))`, spelled with the WKT1 direction keywords so the two can be
/// compared directly.
///
/// `None` for an axis string proj4rs itself would reject (not exactly three
/// characters, or a character outside `ewnsud`), which keeps the gate
/// fail-closed: an unparseable axis matches no WKT and the entry is refused.
#[cfg(feature = "epsg_full")]
fn proj4_axis_directions(proj4: &str) -> Option<(&'static str, &'static str)> {
    let axis =
        proj4.split_ascii_whitespace().find_map(|t| t.strip_prefix("+axis=")).unwrap_or("enu");
    let direction = |b: u8| match b {
        b'e' => Some("EAST"),
        b'w' => Some("WEST"),
        b'n' => Some("NORTH"),
        b's' => Some("SOUTH"),
        b'u' => Some("UP"),
        b'd' => Some("DOWN"),
        _ => None,
    };
    let bytes = axis.as_bytes();
    if bytes.len() != 3 {
        return None;
    }
    Some((direction(bytes[0])?, direction(bytes[1])?))
}

/// The directions of the first two `AXIS["Name",DIRECTION]` nodes in a WKT1
/// definition (e.g. `Some(("EAST", "NORTH"))`), or `None` when the WKT
/// declares fewer than two axes.
#[cfg(feature = "epsg_full")]
fn wkt_axis_directions(wkt: &str) -> Option<(&str, &str)> {
    let mut dirs = wkt.match_indices("AXIS[").filter_map(|(start, _)| {
        let node = wkt[start..].split_once(']')?.0;
        Some(node.rsplit_once(',')?.1.trim())
    });
    Some((dirs.next()?, dirs.next()?))
}

/// The proj4 definition string for a supported EPSG code: the curated table
/// first, then (under the `epsg_full` feature only) the embedded full EPSG
/// registry, filtered per the module-docs policy.
pub fn proj4_definition(epsg: u32) -> Option<String> {
    match epsg {
        // British National Grid (OSGB36 -> WGS84 7-param Helmert).
        27700 => Some(
            "+proj=tmerc +lat_0=49 +lon_0=-2 +k=0.9996012717 +x_0=400000 +y_0=-100000 \
             +ellps=airy +towgs84=446.448,-125.157,542.06,0.15,0.247,0.842,-20.489 \
             +units=m +no_defs"
                .to_string(),
        ),
        // Web Mercator (spherical, the tile-map CRS).
        3857 => Some(
            "+proj=merc +a=6378137 +b=6378137 +lat_ts=0 +lon_0=0 +x_0=0 +y_0=0 +k=1 \
             +units=m +nadgrids=@null +no_defs"
                .to_string(),
        ),
        // RGF93 / Lambert-93 (ETRS89-based; null Helmert to WGS84).
        2154 => Some(
            "+proj=lcc +lat_0=46.5 +lon_0=3 +lat_1=49 +lat_2=44 +x_0=700000 +y_0=6600000 \
             +ellps=GRS80 +towgs84=0,0,0,0,0,0,0 +units=m +no_defs"
                .to_string(),
        ),
        // ETRS89 / UTM zones 32N, 33N.
        25832 | 25833 => Some(format!(
            "+proj=utm +zone={} +ellps=GRS80 +towgs84=0,0,0,0,0,0,0 +units=m +no_defs",
            epsg - 25800
        )),
        // WGS84 / UTM north (326xx) and south (327xx) zones.
        32601..=32660 => {
            Some(format!("+proj=utm +zone={} +datum=WGS84 +units=m +no_defs", epsg - 32600))
        }
        32701..=32760 => Some(format!(
            "+proj=utm +zone={} +south +datum=WGS84 +units=m +no_defs",
            epsg - 32700
        )),
        // Geographic CRSs (see geographic_axis_order). WGS84 itself:
        4326 => Some("+proj=longlat +datum=WGS84 +no_defs".to_string()),
        // ETRS89 / NAD83 / GDA94 / RGF93 / CGCS2000: GRS80-family ellipsoids
        // (CGCS2000's ellipsoid shares GRS80's a and 1/f to the published
        // precision), null Helmert — coincident with WGS84 at metre level.
        4258 | 4269 | 4283 | 4171 | 4490 => {
            Some("+proj=longlat +ellps=GRS80 +towgs84=0,0,0,0,0,0,0 +no_defs".to_string())
        }
        #[cfg(feature = "epsg_full")]
        _ => full_epsg_definition(epsg).map(|d| d.proj4.to_string()),
        #[cfg(not(feature = "epsg_full"))]
        _ => None,
    }
}

/// The EPSG code of an OGC CRS IRI (`…/def/crs/EPSG/0/<code>`), if it is one.
pub fn epsg_code(crs: &Crs) -> Option<u32> {
    crs.iri()
        .strip_prefix("http://www.opengis.net/def/crs/EPSG/0/")
        .and_then(|c| c.parse().ok())
}

/// Reprojects a geometry into CRS84 (WGS84 long/lat degrees).
///
/// CRS84 / EPSG:4326 inputs are returned as-is (re-tagged CRS84 —
/// coordinates were already normalised to long/lat on parse). Anything else
/// must be an EPSG CRS the [`proj4_definition`] lookup knows (the curated
/// table, plus the embedded full EPSG registry under the `epsg_full` feature)
/// — projected codes are transformed from their metre coordinates; GEOGRAPHIC
/// codes are first axis-normalised from the registry's lat/long order
/// ([`geographic_axis_order`]) and then datum-shifted. Anything else is
/// [`GeoError::Unsupported`].
pub fn to_crs84(g: &GeoGeometry) -> Result<GeoGeometry, GeoError> {
    if g.crs.is_geographic() {
        return Ok(GeoGeometry { crs: Crs::Crs84, geometry: g.geometry.clone() });
    }
    let epsg = epsg_code(&g.crs).ok_or_else(|| {
        GeoError::Unsupported(format!(
            "reprojection needs an EPSG CRS IRI ({}/<code>), got <{}>",
            "http://www.opengis.net/def/crs/EPSG/0",
            g.crs.iri()
        ))
    })?;
    let def = proj4_definition(epsg).ok_or_else(|| {
        #[cfg(feature = "epsg_full")]
        let hint = "not in the embedded EPSG registry, or its transform would be silently \
                    wrong here (datum-shift grids this crate does not ship, or a projected \
                    coordinate order the registry's WKT and PROJ.4 definitions do not agree \
                    on)";
        #[cfg(not(feature = "epsg_full"))]
        let hint = "supported: 27700, 3857, 2154, 25832, 25833, 326xx/327xx UTM; geographic \
                    4258, 4269, 4283, 4171, 4490; the `epsg_full` feature adds the full \
                    EPSG registry";
        GeoError::Unsupported(format!("no built-in proj4 definition for EPSG:{epsg} ({hint})"))
    })?;
    let src = Proj::from_proj_string(&def)
        .map_err(|e| GeoError::Parse(format!("proj4 definition for EPSG:{epsg}: {e}")))?;
    let dst = Proj::from_proj_string("+proj=longlat +datum=WGS84 +no_defs")
        .map_err(|e| GeoError::Parse(format!("proj4 WGS84 definition: {e}")))?;
    let src_axis = geographic_axis_order(epsg);

    // proj4rs transforms in place; failures (e.g. coordinates outside the
    // projection's domain) abort the whole geometry.
    let geometry = g.geometry.try_map_coords(|c: Coord<f64>| -> Result<Coord<f64>, GeoError> {
        // Normalise the input into proj's convention: geographic sources are
        // written in the authority axis order in DEGREES but proj4rs consumes
        // long/lat RADIANS; projected sources are metres, fed verbatim.
        let (x, y) = match src_axis {
            Some(AxisOrder::LatLong) => (c.y.to_radians(), c.x.to_radians()),
            Some(AxisOrder::LongLat) => (c.x.to_radians(), c.y.to_radians()),
            None => (c.x, c.y),
        };
        let mut p = (x, y, 0.0);
        proj4rs::transform::transform(&src, &dst, &mut p).map_err(|e| {
            GeoError::Unsupported(format!("EPSG:{epsg} -> CRS84 transform failed: {e}"))
        })?;
        // Geographic outputs are radians in proj convention.
        Ok(Coord { x: p.0.to_degrees(), y: p.1.to_degrees() })
    })?;
    Ok(GeoGeometry { crs: Crs::Crs84, geometry })
}

/// Lexical-level mirror of [`to_crs84`]: a wktLiteral lexical form in, the
/// reprojected CRS84 lexical form out — the shape an engine builtin or a
/// pre-indexing normalisation pass consumes.
pub fn to_crs84_lex(lex: &str) -> Result<String, GeoError> {
    Ok(to_crs84(&crate::parse_wkt_literal(lex)?)?.to_wkt_literal())
}
