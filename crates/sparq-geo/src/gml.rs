//! `geo:gmlLiteral` lexical forms <-> CRS-tagged geometries. sq-zy0
//!
//! GeoSPARQL (1.0 §8.5.2 / 1.1 §8.8) defines a second geometry serialization
//! datatype, `geo:gmlLiteral`, whose lexical form is a **GML geometry element**
//! (an XML fragment) in the OGC *GML Simple Features* profile. GeoSPARQL only
//! ever uses the geometry subset of GML (points, curves, surfaces and their
//! aggregates), so this module is a focused GML-SF *geometry* parser — NOT a
//! general GML processor.
//!
//! ```text
//! "<gml:Point srsName=\"http://www.opengis.net/def/crs/OGC/1.3/CRS84\">\
//!    <gml:pos>-83.4 34.3</gml:pos></gml:Point>"^^geo:gmlLiteral
//! ```
//!
//! Output is the SAME [`GeoGeometry`] the WKT path produces
//! (a [`geo_types::Geometry<f64>`] tagged with a [`Crs`]), so every
//! downstream `geof:` function, the [`GeoIndex`](crate::GeoIndex), and the CRS /
//! axis-order handling work UNCHANGED. As with WKT, coordinates are normalised
//! to x = longitude / y = latitude: an EPSG:4326 `srsName` (LAT/LONG axis order)
//! is axis-swapped on parse exactly as the WKT path swaps EPSG:4326 literals.
//!
//! ## Supported GML-SF geometry elements
//!
//! * `gml:Point` — coordinates via `gml:pos` (GML 3) or `gml:coordinates`
//!   (GML 2 legacy, still emitted by many tools);
//! * `gml:LineString` — `gml:posList`, repeated `gml:pos`, or `gml:coordinates`;
//! * `gml:Polygon` — one `gml:exterior` ring and zero or more `gml:interior`
//!   rings, each a `gml:LinearRing` (GML 3) or the GML 2
//!   `gml:outerBoundaryIs` / `gml:innerBoundaryIs` spellings;
//! * `gml:MultiPoint` — `gml:pointMember` / `gml:pointMembers`;
//! * `gml:MultiCurve` — `gml:curveMember(s)` of `gml:LineString` (the SF
//!   curve type) -> a [`geo_types::MultiLineString`];
//! * `gml:MultiSurface` — `gml:surfaceMember(s)` of `gml:Polygon` -> a
//!   [`geo_types::MultiPolygon`];
//! * the GML 2 aggregates `gml:MultiLineString` (`gml:lineStringMember`) and
//!   `gml:MultiPolygon` (`gml:polygonMember`), accepted as synonyms.
//!
//! Namespace prefixes are NOT assumed to be `gml:`: elements are matched on
//! their LOCAL name (the part after the last `:`), so a document binding the GML
//! namespace to a different prefix — or to the default namespace — parses too.
//! `srsName` is read from the OUTERMOST geometry element (GML allows it on inner
//! elements; the SF profile keeps it on the root, which is what we honour).
//!
//! ## Beyond GML-SF — real GML that is NOT in the gmlLiteral profile sq-47vu
//!
//! The GeoSPARQL `geo:gmlLiteral` profile is GML Simple-Features, but data in the
//! wild carries a few non-SF forms. These parse ADDITIVELY on the SAME path (they
//! never change how an SF literal parses) and are projected into the crate's
//! existing 2-D [`geo_types`] model:
//!
//! * `gml:Envelope` — `gml:lowerCorner` / `gml:upperCorner` -> the 5-point bbox
//!   rectangle [`Polygon`] (`minx miny -> maxx miny -> maxx maxy -> minx maxy ->
//!   minx miny`), so a bounding box can flow into the same topology machinery.
//! * `gml:Curve` / `gml:Surface` with arc segments — a `gml:Curve` is a sequence
//!   of `gml:segments`; we handle `gml:LineStringSegment` (linear),
//!   `gml:Arc` / `gml:ArcString` (a poly-arc through point triples) and
//!   `gml:CircularArcByCenterPoint` (centre + radius + start/end angle). Arc
//!   segments are **densified** to a polyline at a fixed angular step of
//!   [`ARC_STEP_DEG`] degrees per chord; the result is an APPROXIMATION (each arc
//!   is replaced by line segments), so downstream topology/area are approximate to
//!   that resolution. A `gml:Surface` is `gml:patches` of `gml:PolygonPatch`,
//!   whose rings may themselves contain arc segments and are densified the same way.
//! * `gml:srsDimension="3"` — 3-D coordinates parse as XYZ; the crate's geometry
//!   model is 2-D ([`geo_types::Coord<f64>`] has no Z), so **Z is parsed then
//!   projected out** (dropped). XY-only literals are unaffected.
//!
//! Deferred still (clean `GeoError::Unsupported`, tracked as beads):
//! `gml:Triangle` / `gml:PolygonPatch` tessellated `gml:TriangulatedSurface` /
//! `gml:TIN`, and elliptic / non-circular-arc interpolations.

use crate::literal::{swap_xy, Crs, GeoGeometry};
use crate::GeoError;
use geo_types::{
    Coord, Geometry, GeometryCollection, LineString, MultiLineString, MultiPoint, MultiPolygon,
    Point, Polygon,
};
use quick_xml::events::Event;
use quick_xml::reader::Reader;
use std::fmt::Write;

// quick-xml 0.40 removed `BytesText::unescape()` and deprecated
// `Attribute::unescape_value()`. Both used to decode + resolve the XML
// predefined entities without attribute-value whitespace folding. We replicate
// that exact behaviour with `quick_xml::escape::unescape`
// (= `unescape_with(resolve_predefined_entity)`, the resolver the old methods
// used), rather than 0.40's `normalized_value`/`xml_content` which would add
// AVN whitespace folding / EOL normalization the old GML path never applied.
fn unescape_attr_value(raw: &[u8]) -> Result<String, GeoError> {
    let s =
        std::str::from_utf8(raw).map_err(|e| GeoError::Parse(format!("GML attr error: {e}")))?;
    quick_xml::escape::unescape(s)
        .map(|c| c.into_owned())
        .map_err(|e| GeoError::Parse(format!("GML attr error: {e}")))
}

/// Parses a `geo:gmlLiteral` LEXICAL FORM (the literal's string value, without
/// the `^^geo:gmlLiteral` datatype): a GML-SF geometry element.
///
/// The CRS comes from the root element's `srsName` attribute (GeoSPARQL Req 11);
/// absent, it defaults to CRS84 — the same default the WKT path uses.
///
/// An EMPTY lexical form (an empty or whitespace-only string) is the EMPTY
/// geometry (GeoSPARQL Req 16 — the GML counterpart of the empty `wktLiteral`),
/// returned as an empty `GEOMETRYCOLLECTION` in CRS84 exactly as the WKT path
/// represents emptiness. A well-formed GML aggregate element that carries NO
/// members (e.g. `<gml:MultiPoint/>`, `<gml:MultiGeometry/>`) is likewise the
/// empty geometry rather than a parse error. sq-mzmh
pub fn parse_gml_literal(lex: &str) -> Result<GeoGeometry, GeoError> {
    // R16: an empty/whitespace-only lexical form is the empty geometry.
    if lex.trim().is_empty() {
        return Ok(GeoGeometry {
            crs: Crs::Crs84,
            geometry: Geometry::GeometryCollection(GeometryCollection(Vec::new())),
        });
    }
    let node = parse_root(lex)?;
    let crs = node.crs.clone();
    let geometry = build_geometry(&node)?;
    // EPSG:4326 is LAT/LONG; normalise to internal long/lat exactly as WKT does.
    let geometry = if crs == Crs::Epsg4326 { swap_xy(geometry) } else { geometry };
    Ok(GeoGeometry { crs, geometry })
}

// ---- GeoGeometry -> GML 3 Simple Features ---------------------------------

/// Serialises a CRS-tagged geometry to the GML 3 Simple Features profile.
/// sq-i3wb5
pub(crate) fn serialize_gml_literal(value: &GeoGeometry) -> String {
    // Internal geographic coordinates are always longitude/latitude. GML uses
    // the declared CRS axis order, so reverse the parser's EPSG:4326 swap.
    let geometry = if value.crs == Crs::Epsg4326 {
        swap_xy(value.geometry.clone())
    } else {
        value.geometry.clone()
    };

    let mut output = String::new();
    write_geometry(&mut output, &geometry, Some(&value.crs));
    output
}

fn write_geometry(output: &mut String, geometry: &Geometry<f64>, root_crs: Option<&Crs>) {
    match geometry {
        Geometry::Point(point) => write_point(output, point, root_crs),
        Geometry::Line(line) => {
            write_linestring(output, &[line.start, line.end], root_crs);
        }
        Geometry::LineString(line) => write_linestring(output, &line.0, root_crs),
        Geometry::Polygon(polygon) => write_polygon(output, polygon, root_crs),
        Geometry::MultiPoint(points) => {
            write_start(output, "MultiPoint", root_crs);
            for point in points {
                output.push_str("<gml:pointMember>");
                write_point(output, point, None);
                output.push_str("</gml:pointMember>");
            }
            output.push_str("</gml:MultiPoint>");
        }
        Geometry::MultiLineString(lines) => {
            write_start(output, "MultiCurve", root_crs);
            for line in lines {
                output.push_str("<gml:curveMember>");
                write_linestring(output, &line.0, None);
                output.push_str("</gml:curveMember>");
            }
            output.push_str("</gml:MultiCurve>");
        }
        Geometry::MultiPolygon(polygons) => {
            write_start(output, "MultiSurface", root_crs);
            for polygon in polygons {
                output.push_str("<gml:surfaceMember>");
                write_polygon(output, polygon, None);
                output.push_str("</gml:surfaceMember>");
            }
            output.push_str("</gml:MultiSurface>");
        }
        Geometry::GeometryCollection(collection) => {
            write_start(output, "MultiGeometry", root_crs);
            for child in collection {
                output.push_str("<gml:geometryMember>");
                write_geometry(output, child, None);
                output.push_str("</gml:geometryMember>");
            }
            output.push_str("</gml:MultiGeometry>");
        }
        Geometry::Rect(rect) => write_polygon(output, &rect.to_polygon(), root_crs),
        Geometry::Triangle(triangle) => {
            write_polygon(output, &triangle.to_polygon(), root_crs);
        }
    }
}

fn write_start(output: &mut String, name: &str, root_crs: Option<&Crs>) {
    write!(output, "<gml:{name}").expect("writing to a String cannot fail");
    if let Some(crs) = root_crs {
        output.push_str(" xmlns:gml=\"http://www.opengis.net/gml/3.2\" srsName=\"");
        output.push_str(&quick_xml::escape::escape(crs.iri()));
        output.push('"');
    }
    output.push('>');
}

fn write_point(output: &mut String, point: &Point<f64>, root_crs: Option<&Crs>) {
    write_start(output, "Point", root_crs);
    output.push_str("<gml:pos>");
    write_coord(output, point.0);
    output.push_str("</gml:pos></gml:Point>");
}

fn write_linestring(output: &mut String, coords: &[Coord<f64>], root_crs: Option<&Crs>) {
    write_start(output, "LineString", root_crs);
    output.push_str("<gml:posList>");
    write_coords(output, coords);
    output.push_str("</gml:posList></gml:LineString>");
}

fn write_polygon(output: &mut String, polygon: &Polygon<f64>, root_crs: Option<&Crs>) {
    write_start(output, "Polygon", root_crs);
    output.push_str("<gml:exterior>");
    write_ring(output, polygon.exterior());
    output.push_str("</gml:exterior>");
    for ring in polygon.interiors() {
        output.push_str("<gml:interior>");
        write_ring(output, ring);
        output.push_str("</gml:interior>");
    }
    output.push_str("</gml:Polygon>");
}

fn write_ring(output: &mut String, ring: &LineString<f64>) {
    output.push_str("<gml:LinearRing><gml:posList>");
    write_coords(output, &ring.0);
    output.push_str("</gml:posList></gml:LinearRing>");
}

fn write_coords(output: &mut String, coords: &[Coord<f64>]) {
    for (index, coord) in coords.iter().enumerate() {
        if index != 0 {
            output.push(' ');
        }
        write_coord(output, *coord);
    }
}

fn write_coord(output: &mut String, coord: Coord<f64>) {
    write!(output, "{} {}", coord.x, coord.y).expect("writing to a String cannot fail");
}

// ---- Stage 1: XML -> a small geometry DOM ----------------------------------

/// A parsed GML element: its local name, its `srsName`-derived CRS, the
/// coordinate dimension in force, the whitespace-collapsed text it directly
/// contained, and its child elements.
#[derive(Debug)]
struct Node {
    /// Local name (namespace prefix stripped), lower-cased for matching.
    name: String,
    /// CRS resolved from the nearest `srsName` (root element's, in practice).
    crs: Crs,
    /// Coordinate dimension from the nearest `srsDimension` (`2` if none seen),
    /// inherited downward like `crs`. `3` means XYZ — Z is dropped on parse
    /// (the geometry model is 2-D). sq-47vu
    dim: usize,
    /// Concatenated character data directly inside this element.
    text: String,
    children: Vec<Node>,
}

impl Node {
    /// The first child whose local name equals `name` (case-insensitive).
    fn child(&self, name: &str) -> Option<&Node> {
        self.children.iter().find(|c| c.name == name)
    }
    /// All children whose local name equals `name`.
    fn children_named<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Node> {
        self.children.iter().filter(move |c| c.name == name)
    }
}

/// Local name (after the last `:`), lower-cased.
fn local_name(qname: &[u8]) -> String {
    let s = String::from_utf8_lossy(qname);
    let local = s.rsplit(':').next().unwrap_or(&s);
    local.to_ascii_lowercase()
}

/// Resolves a `srsName` attribute value to a [`Crs`]. EPSG codes appear in many
/// equivalent IRI/URN spellings; the common ones are normalised so axis-order
/// handling matches the WKT path.
fn crs_from_srs_name(srs: &str) -> Crs {
    let s = srs.trim();
    // urn:ogc:def:crs:EPSG::4326 / EPSG:4326 / .../EPSG/0/4326 all mean 4326.
    let is_4326 = s.ends_with(":4326")
        || s.ends_with("/4326")
        || s == crate::vocab::EPSG_4326;
    let is_crs84 = s == crate::vocab::CRS84
        || s.ends_with("CRS84")
        || s.ends_with(":CRS84");
    if is_crs84 {
        Crs::Crs84
    } else if is_4326 {
        Crs::Epsg4326
    } else {
        Crs::from_iri(s)
    }
}

/// Scans a start/empty element's attributes for `srsName` (-> [`Crs`]) and
/// `srsDimension` (-> coordinate dimension), updating the inherited values in
/// place so they propagate to descendants. Both attributes are matched on their
/// LOCAL name, exactly as element names are. sq-47vu
fn scan_element_attrs(
    e: &quick_xml::events::BytesStart<'_>,
    inherited_crs: &mut Crs,
    inherited_dim: &mut usize,
) -> Result<(Crs, usize), GeoError> {
    let mut crs = inherited_crs.clone();
    let mut dim = *inherited_dim;
    for attr in e.attributes().flatten() {
        match local_name(attr.key.as_ref()).as_str() {
            "srsname" => {
                let val = unescape_attr_value(&attr.value)?;
                crs = crs_from_srs_name(&val);
                *inherited_crs = crs.clone();
            }
            "srsdimension" => {
                let val = unescape_attr_value(&attr.value)?;
                // A malformed srsDimension is ignored (defaults stay), never a
                // hard error — the coordinate count is the real arbiter.
                if let Ok(d) = val.trim().parse::<usize>() {
                    if d >= 2 {
                        dim = d;
                        *inherited_dim = d;
                    }
                }
            }
            _ => {}
        }
    }
    Ok((crs, dim))
}

/// Reads the whole lexical form into a single root [`Node`] tree, propagating
/// the root `srsName` / `srsDimension` down to children (so inner geometry
/// builders see the CRS and coordinate dimension).
fn parse_root(lex: &str) -> Result<Node, GeoError> {
    let mut reader = Reader::from_str(lex.trim());
    reader.config_mut().trim_text(true);
    let mut stack: Vec<Node> = Vec::new();
    let mut root: Option<Node> = None;
    // The CRS / dimension in force: set by the first srsName / srsDimension seen,
    // inherited downward (dimension defaults to 2-D when none is declared).
    let mut inherited_crs = Crs::Crs84;
    let mut inherited_dim = 2usize;

    loop {
        match reader.read_event() {
            Err(e) => return Err(GeoError::Parse(format!("GML XML error: {e}"))),
            Ok(Event::Eof) => break,
            Ok(Event::Start(e)) => {
                let name = local_name(e.name().as_ref());
                let (crs, dim) =
                    scan_element_attrs(&e, &mut inherited_crs, &mut inherited_dim)?;
                stack.push(Node { name, crs, dim, text: String::new(), children: Vec::new() });
            }
            Ok(Event::Text(t)) => {
                if let Some(top) = stack.last_mut() {
                    // 0.40: decode (no EOL normalization, matching the old
                    // `unescape`) then resolve predefined entities.
                    let decoded = t
                        .decode()
                        .map_err(|err| GeoError::Parse(format!("GML text error: {err}")))?;
                    let txt = quick_xml::escape::unescape(&decoded)
                        .map_err(|err| GeoError::Parse(format!("GML text error: {err}")))?;
                    if !top.text.is_empty() {
                        top.text.push(' ');
                    }
                    top.text.push_str(txt.trim());
                }
            }
            Ok(Event::End(_)) => {
                let node = stack
                    .pop()
                    .ok_or_else(|| GeoError::Parse("GML: unbalanced end tag".to_string()))?;
                match stack.last_mut() {
                    Some(parent) => parent.children.push(node),
                    None => root = Some(node),
                }
            }
            // Empty self-closing element, e.g. <gml:pos/> (degenerate) — keep it.
            Ok(Event::Empty(e)) => {
                let name = local_name(e.name().as_ref());
                let (crs, dim) =
                    scan_element_attrs(&e, &mut inherited_crs, &mut inherited_dim)?;
                let node = Node { name, crs, dim, text: String::new(), children: Vec::new() };
                match stack.last_mut() {
                    Some(parent) => parent.children.push(node),
                    None => root = Some(node),
                }
            }
            _ => {}
        }
    }
    if !stack.is_empty() {
        return Err(GeoError::Parse("GML: unclosed element(s)".to_string()));
    }
    root.ok_or_else(|| GeoError::Parse("GML: no geometry element found".to_string()))
}

// ---- Stage 2: the geometry DOM -> geo_types ---------------------------------

/// Dispatches the root element by local name to the matching geometry builder.
fn build_geometry(node: &Node) -> Result<Geometry<f64>, GeoError> {
    match node.name.as_str() {
        "point" => Ok(Geometry::Point(build_point(node)?)),
        "linestring" => Ok(Geometry::LineString(build_linestring(node)?)),
        "polygon" => Ok(Geometry::Polygon(build_polygon(node)?)),
        "multipoint" => Ok(Geometry::MultiPoint(build_multipoint(node)?)),
        // GML-SF curves are LineStrings; MultiCurve aggregates them.
        "multicurve" | "multilinestring" => {
            Ok(Geometry::MultiLineString(build_multicurve(node)?))
        }
        // GML-SF surfaces are Polygons; MultiSurface aggregates them.
        "multisurface" | "multipolygon" => {
            Ok(Geometry::MultiPolygon(build_multisurface(node)?))
        }
        // A heterogeneous aggregate: the GML 3 `gml:MultiGeometry` and the GML 2
        // `gml:GeometryCollection`. An empty one is the empty geometry (R16).
        "multigeometry" | "geometrycollection" => {
            Ok(Geometry::GeometryCollection(build_geometrycollection(node)?))
        }
        // sq-47vu: real-GML-but-not-SF forms, parsed additively (see
        // the module header). gml:Envelope -> a bbox Polygon; gml:Curve /
        // gml:Surface with arc segments -> arc densification.
        "envelope" => Ok(Geometry::Polygon(build_envelope(node)?)),
        "curve" => Ok(Geometry::LineString(build_curve(node)?)),
        "surface" => Ok(Geometry::Polygon(build_surface(node)?)),
        other => Err(GeoError::Parse(format!(
            "unrecognised GML geometry element <{other}>"
        ))),
    }
}

/// Parses a whitespace/comma-separated coordinate run into `(x, y)` pairs at the
/// given coordinate `dim`ension.
///
/// GML `gml:posList` / `gml:pos` use whitespace separators; the GML 2
/// `gml:coordinates` element uses `,` between the ordinates of a tuple and
/// (by default) a space between tuples. We accept both by treating `,` as a
/// separator too — adequate for the SF profile (default cs/ts).
///
/// `dim` is the per-tuple ordinate count from the nearest `srsDimension`
/// (`2` when none was declared, preserving the historical 2-D behaviour). For
/// `dim == 3` the third ordinate (Z) is parsed then **dropped**: the crate's
/// geometry model is 2-D, so 3-D coordinates are projected onto the XY plane
/// (documented in the module header). sq-47vu
fn parse_coords(text: &str, dim: usize) -> Result<Vec<Coord<f64>>, GeoError> {
    let nums: Vec<f64> = text
        .split([' ', '\t', '\n', '\r', ','])
        .filter(|t| !t.is_empty())
        .map(|t| {
            t.parse::<f64>()
                .map_err(|_| GeoError::Parse(format!("GML: non-numeric ordinate {t:?}")))
        })
        .collect::<Result<_, _>>()?;
    if nums.is_empty() {
        return Err(GeoError::Parse("GML: empty coordinate list".to_string()));
    }
    if dim < 2 || !nums.len().is_multiple_of(dim) {
        return Err(GeoError::Parse(format!(
            "GML: ordinate count {} is not a multiple of srsDimension {}",
            nums.len(),
            dim
        )));
    }
    // Take the first two ordinates (X, Y) of each `dim`-wide tuple; any Z (and
    // beyond) is projected out.
    Ok(nums.chunks_exact(dim).map(|p| Coord { x: p[0], y: p[1] }).collect())
}

/// The coordinate run of a positional element, from any of the SF spellings:
/// `gml:posList`, one-or-more `gml:pos`, or the GML 2 `gml:coordinates`. The
/// effective coordinate dimension is the element's own (a `gml:pos` /
/// `gml:posList` may carry its own `srsDimension`) else the inherited one.
fn coords_of(node: &Node) -> Result<Vec<Coord<f64>>, GeoError> {
    if let Some(pl) = node.child("poslist") {
        return parse_coords(&pl.text, pl.dim);
    }
    if let Some(c) = node.child("coordinates") {
        // The GML 2 gml:coordinates element predates srsDimension; it is 2-D.
        return parse_coords(&c.text, 2);
    }
    let pos: Vec<&Node> = node.children_named("pos").collect();
    if !pos.is_empty() {
        let mut out = Vec::new();
        for p in pos {
            out.extend(parse_coords(&p.text, p.dim)?);
        }
        return Ok(out);
    }
    // A bare element carrying inline text (some encoders put it directly).
    if !node.text.is_empty() {
        return parse_coords(&node.text, node.dim);
    }
    Err(GeoError::Parse(format!(
        "GML <{}>: no gml:pos / gml:posList / gml:coordinates",
        node.name
    )))
}

fn build_point(node: &Node) -> Result<Point<f64>, GeoError> {
    let coords = coords_of(node)?;
    if coords.len() != 1 {
        return Err(GeoError::Parse(format!(
            "GML gml:Point must have exactly one coordinate, got {}",
            coords.len()
        )));
    }
    Ok(Point::from(coords[0]))
}

fn build_linestring(node: &Node) -> Result<LineString<f64>, GeoError> {
    let coords = coords_of(node)?;
    if coords.len() < 2 {
        return Err(GeoError::Parse(
            "GML gml:LineString needs at least two coordinates".to_string(),
        ));
    }
    Ok(LineString::new(coords))
}

/// A `gml:LinearRing` (or the bare positional content of a boundary wrapper).
fn build_ring(node: &Node) -> Result<LineString<f64>, GeoError> {
    // The ring may be the node itself (a LinearRing) or wrapped in
    // exterior/interior/outerBoundaryIs/innerBoundaryIs holding a LinearRing.
    let ring_node = node.child("linearring").unwrap_or(node);
    let coords = coords_of(ring_node)?;
    if coords.len() < 4 {
        return Err(GeoError::Parse(
            "GML gml:LinearRing needs at least four coordinates (closed ring)".to_string(),
        ));
    }
    Ok(LineString::new(coords))
}

fn build_polygon(node: &Node) -> Result<Polygon<f64>, GeoError> {
    // Exterior: gml:exterior (GML 3) or gml:outerBoundaryIs (GML 2).
    let ext_wrap = node
        .child("exterior")
        .or_else(|| node.child("outerboundaryis"))
        .ok_or_else(|| {
            GeoError::Parse("GML gml:Polygon missing gml:exterior ring".to_string())
        })?;
    let exterior = build_ring(ext_wrap)?;

    // Interiors: gml:interior (GML 3) or gml:innerBoundaryIs (GML 2).
    let mut interiors = Vec::new();
    for wrap in node
        .children_named("interior")
        .chain(node.children_named("innerboundaryis"))
    {
        interiors.push(build_ring(wrap)?);
    }
    Ok(Polygon::new(exterior, interiors))
}

fn build_multipoint(node: &Node) -> Result<MultiPoint<f64>, GeoError> {
    let mut points = Vec::new();
    // gml:pointMember (one Point each) and gml:pointMembers (many Points).
    for member in node.children_named("pointmember") {
        let p = member.child("point").ok_or_else(|| {
            GeoError::Parse("GML gml:pointMember missing gml:Point".to_string())
        })?;
        points.push(build_point(p)?);
    }
    for members in node.children_named("pointmembers") {
        for p in members.children_named("point") {
            points.push(build_point(p)?);
        }
    }
    // An empty gml:MultiPoint is the empty geometry (R16), not an error.
    Ok(MultiPoint::new(points))
}

fn build_multicurve(node: &Node) -> Result<MultiLineString<f64>, GeoError> {
    let mut lines = Vec::new();
    // gml:curveMember / gml:curveMembers (MultiCurve) and the GML 2
    // gml:lineStringMember (MultiLineString) each wrap a gml:LineString.
    for member in node
        .children_named("curvemember")
        .chain(node.children_named("linestringmember"))
    {
        let ls = member.child("linestring").ok_or_else(|| {
            GeoError::Unsupported(
                "GML curve member is not a gml:LineString (only SF curves supported)".to_string(),
            )
        })?;
        lines.push(build_linestring(ls)?);
    }
    for members in node.children_named("curvemembers") {
        for ls in members.children_named("linestring") {
            lines.push(build_linestring(ls)?);
        }
    }
    // An empty gml:MultiCurve is the empty geometry (R16), not an error.
    Ok(MultiLineString::new(lines))
}

fn build_multisurface(node: &Node) -> Result<MultiPolygon<f64>, GeoError> {
    let mut polys = Vec::new();
    // gml:surfaceMember / gml:surfaceMembers (MultiSurface) and the GML 2
    // gml:polygonMember (MultiPolygon) each wrap a gml:Polygon.
    for member in node
        .children_named("surfacemember")
        .chain(node.children_named("polygonmember"))
    {
        let poly = member.child("polygon").ok_or_else(|| {
            GeoError::Unsupported(
                "GML surface member is not a gml:Polygon (only SF surfaces supported)".to_string(),
            )
        })?;
        polys.push(build_polygon(poly)?);
    }
    for members in node.children_named("surfacemembers") {
        for poly in members.children_named("polygon") {
            polys.push(build_polygon(poly)?);
        }
    }
    // An empty gml:MultiSurface is the empty geometry (R16), not an error.
    Ok(MultiPolygon::new(polys))
}

/// A `gml:MultiGeometry` / `gml:GeometryCollection`: every `gml:geometryMember`
/// (or `gml:geometryMembers`) recursively built. An empty one is the empty
/// geometry (R16). sq-mzmh
fn build_geometrycollection(node: &Node) -> Result<GeometryCollection<f64>, GeoError> {
    let mut geoms = Vec::new();
    for member in node.children_named("geometrymember") {
        for child in &member.children {
            geoms.push(build_geometry(child)?);
        }
    }
    for members in node.children_named("geometrymembers") {
        for child in &members.children {
            geoms.push(build_geometry(child)?);
        }
    }
    Ok(GeometryCollection(geoms))
}

// ---- Beyond GML-SF: Envelope, arc-segment Curve/Surface (sq-47vu) -

/// The fixed angular resolution at which circular arcs are densified to a
/// polyline: at most this many DEGREES of arc per chord. A smaller value means a
/// closer polyline approximation and more vertices; `5°` keeps the worst-case
/// chord-to-arc sagitta error under ~0.1% of the radius while staying cheap. The
/// densified geometry is an APPROXIMATION — there is no true circular-arc type in
/// the 2-D `geo_types` model — so any downstream topology/length/area is accurate
/// only to this resolution. This is deliberately NOT tunable in v1 (the gmlLiteral
/// profile is GML-SF, which has no arcs at all; this is a best-effort ingest of
/// real-world non-SF GML).
pub const ARC_STEP_DEG: f64 = 5.0;

/// `gml:Envelope` -> the bbox rectangle [`Polygon`]. The corners come from
/// `gml:lowerCorner` (min X, min Y) and `gml:upperCorner` (max X, max Y); some
/// encoders instead give two `gml:pos`, which we accept as a fallback. The result
/// is the closed 5-point ring `(minx miny) (maxx miny) (maxx maxy) (minx maxy)
/// (minx miny)`. Axis-order normalisation (EPSG:4326 swap) is applied uniformly
/// by the caller, exactly as for every other geometry.
fn build_envelope(node: &Node) -> Result<Polygon<f64>, GeoError> {
    let corner = |which: &str| -> Result<Option<Coord<f64>>, GeoError> {
        match node.child(which) {
            Some(c) => {
                let pts = parse_coords(&c.text, c.dim)?;
                let p = pts.first().ok_or_else(|| {
                    GeoError::Parse(format!("GML gml:Envelope <{}> has no coordinate", which))
                })?;
                Ok(Some(*p))
            }
            None => Ok(None),
        }
    };
    let (lower, upper) = match (corner("lowercorner")?, corner("uppercorner")?) {
        (Some(l), Some(u)) => (l, u),
        _ => {
            // Fallback: two gml:pos children (lower then upper).
            let pos: Vec<&Node> = node.children_named("pos").collect();
            if pos.len() == 2 {
                let l = *parse_coords(&pos[0].text, pos[0].dim)?.first().ok_or_else(|| {
                    GeoError::Parse("GML gml:Envelope: empty gml:pos".to_string())
                })?;
                let u = *parse_coords(&pos[1].text, pos[1].dim)?.first().ok_or_else(|| {
                    GeoError::Parse("GML gml:Envelope: empty gml:pos".to_string())
                })?;
                (l, u)
            } else {
                return Err(GeoError::Parse(
                    "GML gml:Envelope needs gml:lowerCorner + gml:upperCorner (or two gml:pos)"
                        .to_string(),
                ));
            }
        }
    };
    let (minx, miny) = (lower.x, lower.y);
    let (maxx, maxy) = (upper.x, upper.y);
    let ring = LineString::new(vec![
        Coord { x: minx, y: miny },
        Coord { x: maxx, y: miny },
        Coord { x: maxx, y: maxy },
        Coord { x: minx, y: maxy },
        Coord { x: minx, y: miny },
    ]);
    Ok(Polygon::new(ring, vec![]))
}

/// `gml:Curve` -> a single densified [`LineString`]. A curve is a sequence of
/// `gml:segments` whose members are joined end-to-end. Each segment is either a
/// `gml:LineStringSegment` (linear, taken verbatim) or an arc form
/// (`gml:Arc` / `gml:ArcString` / `gml:CircularArcByCenterPoint`) densified to a
/// polyline at [`ARC_STEP_DEG`].
fn build_curve(node: &Node) -> Result<LineString<f64>, GeoError> {
    let segments = node.child("segments").ok_or_else(|| {
        GeoError::Parse("GML gml:Curve missing gml:segments".to_string())
    })?;
    let coords = densify_segments(&segments.children)?;
    if coords.len() < 2 {
        return Err(GeoError::Parse(
            "GML gml:Curve produced fewer than two coordinates".to_string(),
        ));
    }
    Ok(LineString::new(coords))
}

/// Builds the joined-and-densified coordinate run for a list of curve SEGMENT
/// elements (the children of `gml:segments`, or of a `gml:Ring`'s curve member).
/// Consecutive segments share a boundary point, so the duplicate start of each
/// following segment is dropped.
fn densify_segments(segs: &[Node]) -> Result<Vec<Coord<f64>>, GeoError> {
    let mut out: Vec<Coord<f64>> = Vec::new();
    let mut any = false;
    for seg in segs {
        let seg_coords = match seg.name.as_str() {
            // Linear / geodesic segment: its control points ARE the polyline
            // vertices (a 2-D geodesic between two points is the chord).
            "linestringsegment" | "geodesicstring" => coords_of(seg)?,
            // Arc forms through control points (point triples).
            "arc" | "arcstring" => densify_arc_string(&coords_of(seg)?)?,
            // Centre + radius + start/end angle.
            "circulararcbycenterpoint" | "arcbycenterpoint" => {
                densify_arc_by_center(seg)?
            }
            other => {
                return Err(GeoError::Unsupported(format!(
                    "GML curve segment <{other}> not supported (linear / Arc / ArcString / \
                     CircularArcByCenterPoint only)"
                )))
            }
        };
        append_joined(&mut out, seg_coords);
        any = true;
    }
    if !any {
        return Err(GeoError::Parse("GML gml:segments has no segment".to_string()));
    }
    Ok(out)
}

/// Appends `next` onto `out`, dropping a leading point that coincides with the
/// current tail (segments share their boundary vertex).
fn append_joined(out: &mut Vec<Coord<f64>>, next: Vec<Coord<f64>>) {
    let mut iter = next.into_iter();
    if let (Some(&last), Some(first)) = (out.last(), iter.clone().next()) {
        if (last.x - first.x).abs() < f64::EPSILON && (last.y - first.y).abs() < f64::EPSILON {
            iter.next(); // skip the duplicate join point
        }
    }
    out.extend(iter);
}

/// Densifies a `gml:Arc` / `gml:ArcString`: a poly-arc through control points,
/// each consecutive (start, mid, end) TRIPLE (stepping by two) defining one
/// circular arc. `gml:Arc` is the degenerate case of exactly three points.
fn densify_arc_string(pts: &[Coord<f64>]) -> Result<Vec<Coord<f64>>, GeoError> {
    // An Arc/ArcString is one or more 3-point arcs sharing endpoints, so the
    // control-point count is odd and at least 3 (3, 5, 7, ...).
    if pts.len() < 3 || pts.len().is_multiple_of(2) {
        return Err(GeoError::Parse(format!(
            "GML Arc/ArcString needs an odd number (>=3) of control points, got {}",
            pts.len()
        )));
    }
    let mut out: Vec<Coord<f64>> = vec![pts[0]];
    let mut i = 0;
    while i + 2 < pts.len() {
        let arc = densify_three_point_arc(pts[i], pts[i + 1], pts[i + 2]);
        // The first vertex of `arc` is `pts[i]`, already the current tail.
        append_joined(&mut out, arc);
        i += 2;
    }
    Ok(out)
}

/// The polyline through three points `(a, b, c)` along the circular arc that
/// passes through all three, sampled at most [`ARC_STEP_DEG`] degrees apart. If
/// the three points are collinear (no finite circumcentre) the arc degenerates to
/// the two chords `a-b-c`.
fn densify_three_point_arc(a: Coord<f64>, b: Coord<f64>, c: Coord<f64>) -> Vec<Coord<f64>> {
    // Circumcentre of triangle abc (intersection of perpendicular bisectors).
    // d = 2 * (ax(by-cy) + bx(cy-ay) + cx(ay-by)); collinear when d ~ 0.
    let d = 2.0 * (a.x * (b.y - c.y) + b.x * (c.y - a.y) + c.x * (a.y - b.y));
    if d.abs() < 1e-12 {
        return vec![a, b, c]; // collinear: just the chords
    }
    let a2 = a.x * a.x + a.y * a.y;
    let b2 = b.x * b.x + b.y * b.y;
    let c2 = c.x * c.x + c.y * c.y;
    let ux = (a2 * (b.y - c.y) + b2 * (c.y - a.y) + c2 * (a.y - b.y)) / d;
    let uy = (a2 * (c.x - b.x) + b2 * (a.x - c.x) + c2 * (b.x - a.x)) / d;
    let center = Coord { x: ux, y: uy };
    let r = ((a.x - ux).powi(2) + (a.y - uy).powi(2)).sqrt();

    let ang = |p: Coord<f64>| (p.y - center.y).atan2(p.x - center.x);
    let a_ang = ang(a);
    let b_ang = ang(b);
    let c_ang = ang(c);

    // Direction: does going a->b->c sweep counter-clockwise (positive cross)?
    let cross = (b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x);
    let ccw = cross > 0.0;

    // Total sweep a->c in the chosen direction, validated to pass through b.
    let sweep = directed_sweep(a_ang, c_ang, ccw);
    let to_b = directed_sweep(a_ang, b_ang, ccw);
    // If b is not between a and c in this direction, the geometry is ill-posed;
    // fall back to the chords rather than producing a wrong-way arc.
    if to_b > sweep + 1e-9 {
        return vec![a, b, c];
    }

    let step = ARC_STEP_DEG.to_radians();
    let n = (sweep / step).ceil().max(1.0) as usize;
    let mut out = Vec::with_capacity(n + 1);
    for k in 0..=n {
        let t = sweep * (k as f64) / (n as f64);
        let theta = if ccw { a_ang + t } else { a_ang - t };
        out.push(Coord { x: center.x + r * theta.cos(), y: center.y + r * theta.sin() });
    }
    out
}

/// The positive sweep angle (in `[0, 2π)`) from `from` to `to` going CCW, or its
/// complement going CW.
fn directed_sweep(from: f64, to: f64, ccw: bool) -> f64 {
    let two_pi = std::f64::consts::TAU;
    let mut delta = if ccw { to - from } else { from - to };
    while delta < 0.0 {
        delta += two_pi;
    }
    while delta >= two_pi {
        delta -= two_pi;
    }
    delta
}

/// Densifies a `gml:CircularArcByCenterPoint` / `gml:ArcByCenterPoint`: a centre
/// (`gml:pos`), a `gml:radius`, and `gml:startAngle` / `gml:endAngle` in degrees.
/// GML measures these angles counter-clockwise from the +X axis. A full circle
/// (start == end) sweeps the whole 360°.
fn densify_arc_by_center(seg: &Node) -> Result<Vec<Coord<f64>>, GeoError> {
    let center_node = seg
        .child("pos")
        .ok_or_else(|| GeoError::Parse("GML ArcByCenterPoint missing gml:pos centre".to_string()))?;
    let center = *parse_coords(&center_node.text, center_node.dim)?
        .first()
        .ok_or_else(|| GeoError::Parse("GML ArcByCenterPoint: empty centre".to_string()))?;
    let num = |name: &str| -> Result<f64, GeoError> {
        seg.child(name)
            .map(|n| n.text.trim().parse::<f64>())
            .transpose()
            .map_err(|_| {
                GeoError::Parse(format!("GML ArcByCenterPoint: non-numeric gml:{}", name))
            })?
            .ok_or_else(|| GeoError::Parse(format!("GML ArcByCenterPoint missing gml:{}", name)))
    };
    let radius = num("radius")?;
    if !radius.is_finite() || radius <= 0.0 {
        return Err(GeoError::Parse(
            "GML ArcByCenterPoint: radius must be a positive number".to_string(),
        ));
    }
    let start = num("startangle")?.to_radians();
    // A missing endAngle, or endAngle == startAngle, is a full circle.
    let end = match seg.child("endangle") {
        Some(n) => n
            .text
            .trim()
            .parse::<f64>()
            .map_err(|_| GeoError::Parse("GML ArcByCenterPoint: non-numeric gml:endAngle".to_string()))?
            .to_radians(),
        None => start + std::f64::consts::TAU,
    };
    let mut sweep = end - start;
    if sweep.abs() < 1e-12 {
        sweep = std::f64::consts::TAU; // start == end -> full circle
    }
    let step = ARC_STEP_DEG.to_radians();
    let n = (sweep.abs() / step).ceil().max(1.0) as usize;
    let mut out = Vec::with_capacity(n + 1);
    for k in 0..=n {
        let theta = start + sweep * (k as f64) / (n as f64);
        out.push(Coord { x: center.x + radius * theta.cos(), y: center.y + radius * theta.sin() });
    }
    Ok(out)
}

/// `gml:Surface` -> a single [`Polygon`]. A surface is `gml:patches` of (in the
/// near-SF case) ONE `gml:PolygonPatch`, whose `gml:exterior` / `gml:interior`
/// boundaries are rings that may themselves contain arc segments (a `gml:Ring`
/// wrapping curve members) and are densified the same way. Only the first patch
/// is taken (a multi-patch surface is beyond a single `geo_types::Polygon`).
fn build_surface(node: &Node) -> Result<Polygon<f64>, GeoError> {
    let patches = node.child("patches").ok_or_else(|| {
        GeoError::Parse("GML gml:Surface missing gml:patches".to_string())
    })?;
    let patch = patches
        .child("polygonpatch")
        .or_else(|| patches.child("rectangle"))
        .ok_or_else(|| {
            GeoError::Unsupported(
                "GML gml:Surface: only gml:PolygonPatch patches supported".to_string(),
            )
        })?;
    let ext_wrap = patch
        .child("exterior")
        .or_else(|| patch.child("outerboundaryis"))
        .ok_or_else(|| {
            GeoError::Parse("GML gml:PolygonPatch missing gml:exterior".to_string())
        })?;
    let exterior = build_surface_ring(ext_wrap)?;
    let mut interiors = Vec::new();
    for wrap in patch
        .children_named("interior")
        .chain(patch.children_named("innerboundaryis"))
    {
        interiors.push(build_surface_ring(wrap)?);
    }
    Ok(Polygon::new(exterior, interiors))
}

/// A patch boundary ring: either a plain `gml:LinearRing` (handled by the SF
/// [`build_ring`]) or a `gml:Ring` whose `gml:curveMember`s carry arc segments,
/// which are densified and joined into one closed ring.
fn build_surface_ring(wrap: &Node) -> Result<LineString<f64>, GeoError> {
    if wrap.child("linearring").is_some() {
        return build_ring(wrap);
    }
    if let Some(ring) = wrap.child("ring") {
        let mut coords: Vec<Coord<f64>> = Vec::new();
        for member in ring.children_named("curvemember") {
            let curve = member.child("curve").ok_or_else(|| {
                GeoError::Unsupported(
                    "GML gml:Ring member is not a gml:Curve".to_string(),
                )
            })?;
            let segments = curve.child("segments").ok_or_else(|| {
                GeoError::Parse("GML gml:Curve in ring missing gml:segments".to_string())
            })?;
            append_joined(&mut coords, densify_segments(&segments.children)?);
        }
        if coords.len() < 4 {
            return Err(GeoError::Parse(
                "GML gml:Ring produced fewer than four coordinates".to_string(),
            ));
        }
        // Close the ring if the densified arcs did not return exactly to start.
        if let (Some(&first), Some(&last)) = (coords.first(), coords.last()) {
            if (first.x - last.x).abs() > 1e-9 || (first.y - last.y).abs() > 1e-9 {
                coords.push(first);
            }
        }
        return Ok(LineString::new(coords));
    }
    // A bare LinearRing-less boundary: try the SF ring builder (handles inline pos).
    build_ring(wrap)
}
