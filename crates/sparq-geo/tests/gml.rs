//! `geo:gmlLiteral` (GML Simple-Features) parsing + equivalence to its WKT twin.
//! sq-zy0

use geo_types::{
    Coord, Geometry, LineString, MultiLineString, MultiPoint, MultiPolygon, Point, Polygon,
};
use sparq_geo::{
    is_geometry_datatype, parse_geometry_literal, parse_gml_literal, parse_wkt_literal, vocab, Crs,
    GeoError,
};

// ---- Each GML-SF geometry type -> hand-computed geo geometry ----------------

#[test]
fn parses_point_pos_default_crs() {
    let g = parse_gml_literal(
        r#"<gml:Point><gml:pos>-83.38 33.95</gml:pos></gml:Point>"#,
    )
    .unwrap();
    assert_eq!(g.crs, Crs::Crs84);
    assert_eq!(g.geometry, Geometry::Point(Point::new(-83.38, 33.95)));
}

#[test]
fn parses_point_gml2_coordinates() {
    // GML 2 legacy spelling: <gml:coordinates>x,y</gml:coordinates>.
    let g = parse_gml_literal(
        r#"<gml:Point><gml:coordinates>1.5,-2.25</gml:coordinates></gml:Point>"#,
    )
    .unwrap();
    assert_eq!(g.geometry, Geometry::Point(Point::new(1.5, -2.25)));
}

#[test]
fn parses_linestring_poslist() {
    let g = parse_gml_literal(
        r#"<gml:LineString><gml:posList>0 0 1 1 2 0</gml:posList></gml:LineString>"#,
    )
    .unwrap();
    let expected = LineString::new(vec![
        Coord { x: 0.0, y: 0.0 },
        Coord { x: 1.0, y: 1.0 },
        Coord { x: 2.0, y: 0.0 },
    ]);
    assert_eq!(g.geometry, Geometry::LineString(expected));
}

#[test]
fn parses_linestring_repeated_pos() {
    let g = parse_gml_literal(
        r#"<gml:LineString>
             <gml:pos>0 0</gml:pos>
             <gml:pos>1 1</gml:pos>
           </gml:LineString>"#,
    )
    .unwrap();
    let expected =
        LineString::new(vec![Coord { x: 0.0, y: 0.0 }, Coord { x: 1.0, y: 1.0 }]);
    assert_eq!(g.geometry, Geometry::LineString(expected));
}

#[test]
fn parses_polygon_exterior_only() {
    let g = parse_gml_literal(
        r#"<gml:Polygon>
             <gml:exterior>
               <gml:LinearRing>
                 <gml:posList>0 0 4 0 4 4 0 4 0 0</gml:posList>
               </gml:LinearRing>
             </gml:exterior>
           </gml:Polygon>"#,
    )
    .unwrap();
    let exterior = LineString::new(vec![
        Coord { x: 0.0, y: 0.0 },
        Coord { x: 4.0, y: 0.0 },
        Coord { x: 4.0, y: 4.0 },
        Coord { x: 0.0, y: 4.0 },
        Coord { x: 0.0, y: 0.0 },
    ]);
    assert_eq!(g.geometry, Geometry::Polygon(Polygon::new(exterior, vec![])));
}

#[test]
fn parses_polygon_with_interior_ring() {
    let g = parse_gml_literal(
        r#"<gml:Polygon>
             <gml:exterior><gml:LinearRing>
               <gml:posList>0 0 4 0 4 4 0 4 0 0</gml:posList>
             </gml:LinearRing></gml:exterior>
             <gml:interior><gml:LinearRing>
               <gml:posList>1 1 2 1 2 2 1 2 1 1</gml:posList>
             </gml:LinearRing></gml:interior>
           </gml:Polygon>"#,
    )
    .unwrap();
    let exterior = LineString::new(vec![
        Coord { x: 0.0, y: 0.0 },
        Coord { x: 4.0, y: 0.0 },
        Coord { x: 4.0, y: 4.0 },
        Coord { x: 0.0, y: 4.0 },
        Coord { x: 0.0, y: 0.0 },
    ]);
    let interior = LineString::new(vec![
        Coord { x: 1.0, y: 1.0 },
        Coord { x: 2.0, y: 1.0 },
        Coord { x: 2.0, y: 2.0 },
        Coord { x: 1.0, y: 2.0 },
        Coord { x: 1.0, y: 1.0 },
    ]);
    assert_eq!(
        g.geometry,
        Geometry::Polygon(Polygon::new(exterior, vec![interior]))
    );
}

#[test]
fn parses_polygon_gml2_boundary_spellings() {
    // GML 2 used outerBoundaryIs / innerBoundaryIs instead of exterior/interior.
    let g = parse_gml_literal(
        r#"<gml:Polygon>
             <gml:outerBoundaryIs><gml:LinearRing>
               <gml:coordinates>0,0 4,0 4,4 0,4 0,0</gml:coordinates>
             </gml:LinearRing></gml:outerBoundaryIs>
           </gml:Polygon>"#,
    )
    .unwrap();
    let exterior = LineString::new(vec![
        Coord { x: 0.0, y: 0.0 },
        Coord { x: 4.0, y: 0.0 },
        Coord { x: 4.0, y: 4.0 },
        Coord { x: 0.0, y: 4.0 },
        Coord { x: 0.0, y: 0.0 },
    ]);
    assert_eq!(g.geometry, Geometry::Polygon(Polygon::new(exterior, vec![])));
}

#[test]
fn parses_multipoint() {
    let g = parse_gml_literal(
        r#"<gml:MultiPoint>
             <gml:pointMember><gml:Point><gml:pos>0 0</gml:pos></gml:Point></gml:pointMember>
             <gml:pointMember><gml:Point><gml:pos>1 1</gml:pos></gml:Point></gml:pointMember>
           </gml:MultiPoint>"#,
    )
    .unwrap();
    let expected = MultiPoint::new(vec![Point::new(0.0, 0.0), Point::new(1.0, 1.0)]);
    assert_eq!(g.geometry, Geometry::MultiPoint(expected));
}

#[test]
fn parses_multicurve_of_linestrings() {
    let g = parse_gml_literal(
        r#"<gml:MultiCurve>
             <gml:curveMember><gml:LineString>
               <gml:posList>0 0 1 1</gml:posList>
             </gml:LineString></gml:curveMember>
             <gml:curveMember><gml:LineString>
               <gml:posList>2 2 3 3</gml:posList>
             </gml:LineString></gml:curveMember>
           </gml:MultiCurve>"#,
    )
    .unwrap();
    let expected = MultiLineString::new(vec![
        LineString::new(vec![Coord { x: 0.0, y: 0.0 }, Coord { x: 1.0, y: 1.0 }]),
        LineString::new(vec![Coord { x: 2.0, y: 2.0 }, Coord { x: 3.0, y: 3.0 }]),
    ]);
    assert_eq!(g.geometry, Geometry::MultiLineString(expected));
}

#[test]
fn parses_multisurface_of_polygons() {
    let g = parse_gml_literal(
        r#"<gml:MultiSurface>
             <gml:surfaceMember><gml:Polygon><gml:exterior><gml:LinearRing>
               <gml:posList>0 0 1 0 1 1 0 0</gml:posList>
             </gml:LinearRing></gml:exterior></gml:Polygon></gml:surfaceMember>
             <gml:surfaceMember><gml:Polygon><gml:exterior><gml:LinearRing>
               <gml:posList>5 5 6 5 6 6 5 5</gml:posList>
             </gml:LinearRing></gml:exterior></gml:Polygon></gml:surfaceMember>
           </gml:MultiSurface>"#,
    )
    .unwrap();
    let p1 = Polygon::new(
        LineString::new(vec![
            Coord { x: 0.0, y: 0.0 },
            Coord { x: 1.0, y: 0.0 },
            Coord { x: 1.0, y: 1.0 },
            Coord { x: 0.0, y: 0.0 },
        ]),
        vec![],
    );
    let p2 = Polygon::new(
        LineString::new(vec![
            Coord { x: 5.0, y: 5.0 },
            Coord { x: 6.0, y: 5.0 },
            Coord { x: 6.0, y: 6.0 },
            Coord { x: 5.0, y: 5.0 },
        ]),
        vec![],
    );
    assert_eq!(g.geometry, Geometry::MultiPolygon(MultiPolygon::new(vec![p1, p2])));
}

#[test]
fn namespace_prefix_is_irrelevant() {
    // A document binding the GML namespace to a different prefix, or the default
    // namespace, parses just the same (we match on the local name).
    for lex in [
        r#"<Point xmlns="http://www.opengis.net/gml/3.2"><pos>1 2</pos></Point>"#,
        r#"<g:Point xmlns:g="http://www.opengis.net/gml/3.2"><g:pos>1 2</g:pos></g:Point>"#,
    ] {
        let g = parse_gml_literal(lex).unwrap_or_else(|e| panic!("{lex}: {e}"));
        assert_eq!(g.geometry, Geometry::Point(Point::new(1.0, 2.0)));
    }
}

// ---- CRS / srsName handling consistent with the WKT path --------------------

#[test]
fn srs_name_crs84_default_equivalent() {
    let with = parse_gml_literal(
        &format!(r#"<gml:Point srsName="{}"><gml:pos>-83.38 33.95</gml:pos></gml:Point>"#, vocab::CRS84),
    )
    .unwrap();
    assert_eq!(with.crs, Crs::Crs84);
    assert_eq!(with.geometry, Geometry::Point(Point::new(-83.38, 33.95)));
}

#[test]
fn epsg_4326_srs_name_swaps_axis_order() {
    // EPSG:4326 is LAT/LONG, like the WKT path: the lexical form writes latitude
    // first but internal x must be longitude. Multiple srsName spellings count.
    for srs in [
        "http://www.opengis.net/def/crs/EPSG/0/4326",
        "urn:ogc:def:crs:EPSG::4326",
        "EPSG:4326",
    ] {
        let g = parse_gml_literal(&format!(
            r#"<gml:Point srsName="{srs}"><gml:pos>33.95 -83.38</gml:pos></gml:Point>"#
        ))
        .unwrap();
        assert_eq!(g.crs, Crs::Epsg4326, "{srs}");
        assert_eq!(
            g.geometry,
            Geometry::Point(Point::new(-83.38, 33.95)),
            "{srs}"
        );
    }
}

#[test]
fn other_crs_kept_verbatim() {
    let iri = "http://www.opengis.net/def/crs/EPSG/0/27700"; // British National Grid
    let g = parse_gml_literal(&format!(
        r#"<gml:Point srsName="{iri}"><gml:pos>530000 180000</gml:pos></gml:Point>"#
    ))
    .unwrap();
    assert_eq!(g.crs, Crs::Other(iri.to_string()));
    assert!(!g.crs.is_geographic());
    assert_eq!(g.geometry, Geometry::Point(Point::new(530000.0, 180000.0)));
}

// ---- Malformed GML -> clean GeoError, never a panic -------------------------

#[test]
fn malformed_gml_is_a_clean_error() {
    let cases = [
        // Not XML at all.
        "POINT(1 2)",
        // Unknown / non-geometry root element.
        r#"<gml:Feature><gml:name>x</gml:name></gml:Feature>"#,
        // Point with no coordinates.
        r#"<gml:Point></gml:Point>"#,
        // Odd ordinate count (only 2-D supported).
        r#"<gml:Point><gml:pos>1 2 3</gml:pos></gml:Point>"#,
        // Non-numeric ordinate.
        r#"<gml:Point><gml:pos>foo bar</gml:pos></gml:Point>"#,
        // Polygon missing its exterior ring.
        r#"<gml:Polygon><gml:interior><gml:LinearRing><gml:posList>0 0 1 0 1 1 0 0</gml:posList></gml:LinearRing></gml:interior></gml:Polygon>"#,
        // LinearRing too short to close.
        r#"<gml:Polygon><gml:exterior><gml:LinearRing><gml:posList>0 0 1 1</gml:posList></gml:LinearRing></gml:exterior></gml:Polygon>"#,
        // Truncated / unbalanced XML.
        r#"<gml:Point><gml:pos>1 2</gml:pos>"#,
        // NB: an empty lexical form is NOT an error — it is the empty geometry
        // (GeoSPARQL Req 16); see `empty_gml_lexical_form_is_empty_geometry`.
    ];
    for lex in cases {
        match parse_gml_literal(lex) {
            Err(GeoError::Parse(_)) | Err(GeoError::Unsupported(_)) => {}
            other => panic!("expected a clean error for {lex:?}, got {other:?}"),
        }
    }
}

#[test]
fn still_deferred_types_are_clean_unsupported() {
    // sq-47vu: gml:Envelope, arc-segment Curve/Surface, and 3-D coords
    // are now SUPPORTED (see the dedicated tests below). What remains deferred —
    // tessellated TIN/TriangulatedSurface patches and non-PolygonPatch surface
    // patches — must still be a clean Unsupported, never a panic or wrong answer.
    let cases = [
        // A gml:Surface whose patch is a gml:Triangle (a tessellation patch), not
        // a gml:PolygonPatch: deferred.
        r#"<gml:Surface><gml:patches>
             <gml:Triangle><gml:exterior><gml:LinearRing>
               <gml:posList>0 0 1 0 0 1 0 0</gml:posList>
             </gml:LinearRing></gml:exterior></gml:Triangle>
           </gml:patches></gml:Surface>"#,
        // A gml:Curve segment kind we do not interpolate (e.g. a clothoid).
        r#"<gml:Curve><gml:segments>
             <gml:Clothoid/>
           </gml:segments></gml:Curve>"#,
    ];
    for lex in cases {
        assert!(
            matches!(parse_gml_literal(lex), Err(GeoError::Unsupported(_))),
            "expected Unsupported for {lex:?}, got {:?}",
            parse_gml_literal(lex)
        );
    }
}

// ---- Beyond GML-SF: Envelope, arc Curve/Surface, 3-D coords (sq-47vu) -------
//

#[test]
fn envelope_becomes_bbox_polygon() {
    // gml:Envelope(lowerCorner=0 0, upperCorner=4 4) -> the closed 5-point
    // rectangle, identical to the WKT bbox polygon (roundtrip equivalence).
    let g = parse_gml_literal(
        r#"<gml:Envelope srsName="http://www.opengis.net/def/crs/OGC/1.3/CRS84">
             <gml:lowerCorner>0 0</gml:lowerCorner>
             <gml:upperCorner>4 4</gml:upperCorner>
           </gml:Envelope>"#,
    )
    .unwrap();
    assert_eq!(g.crs, Crs::Crs84);
    let expected = Polygon::new(
        LineString::new(vec![
            Coord { x: 0.0, y: 0.0 },
            Coord { x: 4.0, y: 0.0 },
            Coord { x: 4.0, y: 4.0 },
            Coord { x: 0.0, y: 4.0 },
            Coord { x: 0.0, y: 0.0 },
        ]),
        vec![],
    );
    assert_eq!(g.geometry, Geometry::Polygon(expected.clone()));
    // Roundtrip: the same rectangle as the equivalent WKT polygon.
    let wkt = parse_wkt_literal("POLYGON((0 0, 4 0, 4 4, 0 4, 0 0))").unwrap();
    assert_eq!(g, wkt);
}

#[test]
fn envelope_epsg4326_axis_swapped() {
    // EPSG:4326 corners are lat/long; like every geometry, they axis-swap to the
    // internal long/lat, so the bbox covers x in [-1,1], y in [50,52].
    let g = parse_gml_literal(
        r#"<gml:Envelope srsName="urn:ogc:def:crs:EPSG::4326">
             <gml:lowerCorner>50 -1</gml:lowerCorner>
             <gml:upperCorner>52 1</gml:upperCorner>
           </gml:Envelope>"#,
    )
    .unwrap();
    assert_eq!(g.crs, Crs::Epsg4326);
    if let Geometry::Polygon(p) = &g.geometry {
        let xs: Vec<f64> = p.exterior().coords().map(|c| c.x).collect();
        let ys: Vec<f64> = p.exterior().coords().map(|c| c.y).collect();
        assert_eq!(xs.iter().cloned().fold(f64::INFINITY, f64::min), -1.0);
        assert_eq!(xs.iter().cloned().fold(f64::NEG_INFINITY, f64::max), 1.0);
        assert_eq!(ys.iter().cloned().fold(f64::INFINITY, f64::min), 50.0);
        assert_eq!(ys.iter().cloned().fold(f64::NEG_INFINITY, f64::max), 52.0);
    } else {
        panic!("expected a Polygon, got {:?}", g.geometry);
    }
}

/// The max distance of any densified vertex from the true circle of radius `r`
/// centred at `c` — should be ~0 (the vertices lie ON the circle; densification
/// only approximates the arc BETWEEN them by chords).
fn max_radial_error(coords: &[Coord<f64>], c: Coord<f64>, r: f64) -> f64 {
    coords
        .iter()
        .map(|p| (((p.x - c.x).powi(2) + (p.y - c.y).powi(2)).sqrt() - r).abs())
        .fold(0.0, f64::max)
}

#[test]
fn arc_string_densifies_to_linestring_within_tolerance() {
    // A gml:Curve with one gml:ArcString through three points of the unit circle:
    // (1,0) -> (0,1) -> (-1,0), i.e. the upper semicircle centred at the origin.
    let g = parse_gml_literal(
        r#"<gml:Curve><gml:segments>
             <gml:ArcString><gml:posList>1 0  0 1  -1 0</gml:posList></gml:ArcString>
           </gml:segments></gml:Curve>"#,
    )
    .unwrap();
    let ls = match &g.geometry {
        Geometry::LineString(ls) => ls,
        other => panic!("expected a LineString, got {other:?}"),
    };
    let coords: Vec<Coord<f64>> = ls.coords().cloned().collect();
    // The endpoints are exact, and every vertex lies on the unit circle.
    assert!((coords.first().unwrap().x - 1.0).abs() < 1e-9);
    assert!((coords.last().unwrap().x + 1.0).abs() < 1e-9);
    assert!(coords.len() > 10, "semicircle densified to {} pts", coords.len());
    // Vertices sit on the circle to f64 precision; the polyline approximation is
    // the chords BETWEEN them, bounded by the 5-degree-per-chord step.
    assert!(max_radial_error(&coords, Coord { x: 0.0, y: 0.0 }, 1.0) < 1e-9);
    // Sagitta (chord-to-arc) bound for the fixed 5-degree step on a unit circle:
    // r*(1 - cos(2.5 deg)) ~= 9.5e-4. Allow generous headroom.
    let max_step_deg = coords
        .windows(2)
        .map(|w| {
            let a0 = w[0].y.atan2(w[0].x);
            let a1 = w[1].y.atan2(w[1].x);
            (a1 - a0).abs().to_degrees()
        })
        .fold(0.0, f64::max);
    assert!(max_step_deg <= 5.0 + 1e-6, "max angular step {max_step_deg} deg");
}

#[test]
fn circular_arc_by_center_point_densifies() {
    // A quarter circle, radius 2 centred at (10, 10), 0 deg -> 90 deg.
    let g = parse_gml_literal(
        r#"<gml:Curve><gml:segments>
             <gml:CircularArcByCenterPoint>
               <gml:pos>10 10</gml:pos>
               <gml:radius>2</gml:radius>
               <gml:startAngle>0</gml:startAngle>
               <gml:endAngle>90</gml:endAngle>
             </gml:CircularArcByCenterPoint>
           </gml:segments></gml:Curve>"#,
    )
    .unwrap();
    let ls = match &g.geometry {
        Geometry::LineString(ls) => ls,
        other => panic!("expected a LineString, got {other:?}"),
    };
    let coords: Vec<Coord<f64>> = ls.coords().cloned().collect();
    let center = Coord { x: 10.0, y: 10.0 };
    // Starts at angle 0 (12, 10), ends at angle 90 (10, 12).
    assert!((coords.first().unwrap().x - 12.0).abs() < 1e-9);
    assert!((coords.last().unwrap().y - 12.0).abs() < 1e-9);
    // Every vertex is exactly radius 2 from the centre.
    assert!(max_radial_error(&coords, center, 2.0) < 1e-9);
    // 90 deg / 5 deg = 18 chords -> 19 vertices.
    assert_eq!(coords.len(), 19);
}

#[test]
fn arc_by_center_full_circle_when_no_end_angle() {
    // No endAngle -> a full 360-degree circle (closed back to the start).
    let g = parse_gml_literal(
        r#"<gml:Curve><gml:segments>
             <gml:CircularArcByCenterPoint>
               <gml:pos>0 0</gml:pos><gml:radius>1</gml:radius><gml:startAngle>0</gml:startAngle>
             </gml:CircularArcByCenterPoint>
           </gml:segments></gml:Curve>"#,
    )
    .unwrap();
    let ls = match &g.geometry {
        Geometry::LineString(ls) => ls,
        other => panic!("expected a LineString, got {other:?}"),
    };
    let coords: Vec<Coord<f64>> = ls.coords().cloned().collect();
    assert!(max_radial_error(&coords, Coord { x: 0.0, y: 0.0 }, 1.0) < 1e-9);
    // Closes (first ~= last).
    let (f, l) = (coords.first().unwrap(), coords.last().unwrap());
    assert!((f.x - l.x).abs() < 1e-9 && (f.y - l.y).abs() < 1e-9);
}

#[test]
fn curve_with_linestring_segment_is_linear() {
    // A gml:Curve whose only segment is a linear gml:LineStringSegment is just
    // the polyline through its control points (no densification).
    let g = parse_gml_literal(
        r#"<gml:Curve><gml:segments>
             <gml:LineStringSegment><gml:posList>0 0 1 1 2 0</gml:posList></gml:LineStringSegment>
           </gml:segments></gml:Curve>"#,
    )
    .unwrap();
    let expected = LineString::new(vec![
        Coord { x: 0.0, y: 0.0 },
        Coord { x: 1.0, y: 1.0 },
        Coord { x: 2.0, y: 0.0 },
    ]);
    assert_eq!(g.geometry, Geometry::LineString(expected));
}

#[test]
fn curve_joins_mixed_segments() {
    // A linear segment followed by an arc segment sharing the join point (1,0):
    // the duplicate join vertex is dropped, producing one continuous polyline.
    let g = parse_gml_literal(
        r#"<gml:Curve><gml:segments>
             <gml:LineStringSegment><gml:posList>-1 0 1 0</gml:posList></gml:LineStringSegment>
             <gml:ArcString><gml:posList>1 0 0 1 -1 0</gml:posList></gml:ArcString>
           </gml:segments></gml:Curve>"#,
    )
    .unwrap();
    let ls = match &g.geometry {
        Geometry::LineString(ls) => ls,
        other => panic!("expected a LineString, got {other:?}"),
    };
    let coords: Vec<Coord<f64>> = ls.coords().cloned().collect();
    // Starts at (-1,0) (line), the join (1,0) appears exactly once.
    assert_eq!(coords.first().unwrap(), &Coord { x: -1.0, y: 0.0 });
    let join_count = coords
        .iter()
        .filter(|c| (c.x - 1.0).abs() < 1e-9 && c.y.abs() < 1e-9)
        .count();
    assert_eq!(join_count, 1, "join point (1,0) should appear once");
}

#[test]
fn surface_with_arc_ring_densifies_to_polygon() {
    // A gml:Surface whose single PolygonPatch exterior is a gml:Ring built from a
    // gml:Curve of two semicircle arcs forming a full circle of radius 1.
    let g = parse_gml_literal(
        r#"<gml:Surface><gml:patches><gml:PolygonPatch>
             <gml:exterior><gml:Ring>
               <gml:curveMember><gml:Curve><gml:segments>
                 <gml:ArcString><gml:posList>1 0 0 1 -1 0</gml:posList></gml:ArcString>
                 <gml:ArcString><gml:posList>-1 0 0 -1 1 0</gml:posList></gml:ArcString>
               </gml:segments></gml:Curve></gml:curveMember>
             </gml:Ring></gml:exterior>
           </gml:PolygonPatch></gml:patches></gml:Surface>"#,
    )
    .unwrap();
    let p = match &g.geometry {
        Geometry::Polygon(p) => p,
        other => panic!("expected a Polygon, got {other:?}"),
    };
    let ring: Vec<Coord<f64>> = p.exterior().coords().cloned().collect();
    // Vertices lie on the unit circle; the ring is closed.
    assert!(max_radial_error(&ring, Coord { x: 0.0, y: 0.0 }, 1.0) < 1e-9);
    assert_eq!(ring.first().unwrap(), ring.last().unwrap(), "ring must close");
}

#[test]
fn three_d_srs_dimension_drops_z() {
    // srsDimension="3": XYZ parses (previously rejected as an odd-ordinate error),
    // and Z is projected out so the result equals its 2-D twin.
    let g = parse_gml_literal(
        r#"<gml:Point srsName="http://www.opengis.net/def/crs/OGC/1.3/CRS84" srsDimension="3">
             <gml:pos>1 2 99</gml:pos></gml:Point>"#,
    )
    .unwrap();
    assert_eq!(g.geometry, Geometry::Point(Point::new(1.0, 2.0)));
    assert_eq!(g, parse_wkt_literal("POINT(1 2)").unwrap());
}

#[test]
fn three_d_linestring_drops_z() {
    // A 3-D posList: 3 XYZ tuples -> 3 XY vertices (Z dropped).
    let g = parse_gml_literal(
        r#"<gml:LineString srsDimension="3">
             <gml:posList>0 0 5 1 1 6 2 0 7</gml:posList></gml:LineString>"#,
    )
    .unwrap();
    let expected = LineString::new(vec![
        Coord { x: 0.0, y: 0.0 },
        Coord { x: 1.0, y: 1.0 },
        Coord { x: 2.0, y: 0.0 },
    ]);
    assert_eq!(g.geometry, Geometry::LineString(expected));
}

#[test]
fn three_d_srs_dimension_on_pos_element() {
    // srsDimension may sit on the gml:pos itself, not just the root.
    let g = parse_gml_literal(
        r#"<gml:Point><gml:pos srsDimension="3">7 8 9</gml:pos></gml:Point>"#,
    )
    .unwrap();
    assert_eq!(g.geometry, Geometry::Point(Point::new(7.0, 8.0)));
}

#[test]
fn two_d_unchanged_when_no_srs_dimension() {
    // Regression guard: with NO srsDimension, a 6-number posList is still 3 XY
    // vertices (the historical 2-D behaviour), never 2 XYZ vertices.
    let g = parse_gml_literal(
        r#"<gml:LineString><gml:posList>0 0 1 1 2 0</gml:posList></gml:LineString>"#,
    )
    .unwrap();
    let expected = LineString::new(vec![
        Coord { x: 0.0, y: 0.0 },
        Coord { x: 1.0, y: 1.0 },
        Coord { x: 2.0, y: 0.0 },
    ]);
    assert_eq!(g.geometry, Geometry::LineString(expected));
}

// ---- Equivalence with the WKT twin ------------------------------------------

/// Every GML geometry parses to the SAME `GeoGeometry` as the equivalent WKT
/// literal — proving GML rides the identical downstream pipeline.
#[test]
fn gml_equals_its_wkt_twin() {
    let pairs: &[(&str, &str)] = &[
        (
            r#"<gml:Point><gml:pos>1 2</gml:pos></gml:Point>"#,
            "POINT(1 2)",
        ),
        (
            r#"<gml:LineString><gml:posList>0 0 1 1 2 0</gml:posList></gml:LineString>"#,
            "LINESTRING(0 0, 1 1, 2 0)",
        ),
        (
            r#"<gml:Polygon><gml:exterior><gml:LinearRing><gml:posList>0 0 4 0 4 4 0 4 0 0</gml:posList></gml:LinearRing></gml:exterior><gml:interior><gml:LinearRing><gml:posList>1 1 2 1 2 2 1 2 1 1</gml:posList></gml:LinearRing></gml:interior></gml:Polygon>"#,
            "POLYGON((0 0, 4 0, 4 4, 0 4, 0 0), (1 1, 2 1, 2 2, 1 2, 1 1))",
        ),
        (
            r#"<gml:MultiPoint><gml:pointMember><gml:Point><gml:pos>0 0</gml:pos></gml:Point></gml:pointMember><gml:pointMember><gml:Point><gml:pos>1 1</gml:pos></gml:Point></gml:pointMember></gml:MultiPoint>"#,
            "MULTIPOINT(0 0, 1 1)",
        ),
        (
            r#"<gml:MultiCurve><gml:curveMember><gml:LineString><gml:posList>0 0 1 1</gml:posList></gml:LineString></gml:curveMember><gml:curveMember><gml:LineString><gml:posList>2 2 3 3</gml:posList></gml:LineString></gml:curveMember></gml:MultiCurve>"#,
            "MULTILINESTRING((0 0, 1 1), (2 2, 3 3))",
        ),
        (
            r#"<gml:MultiSurface><gml:surfaceMember><gml:Polygon><gml:exterior><gml:LinearRing><gml:posList>0 0 1 0 1 1 0 0</gml:posList></gml:LinearRing></gml:exterior></gml:Polygon></gml:surfaceMember></gml:MultiSurface>"#,
            "MULTIPOLYGON(((0 0, 1 0, 1 1, 0 0)))",
        ),
    ];
    for (gml, wkt) in pairs {
        let from_gml = parse_gml_literal(gml).unwrap_or_else(|e| panic!("{gml}: {e}"));
        let from_wkt = parse_wkt_literal(wkt).unwrap_or_else(|e| panic!("{wkt}: {e}"));
        assert_eq!(from_gml, from_wkt, "GML {gml} != WKT {wkt}");
    }
}

#[test]
fn parse_geometry_literal_dispatches_by_datatype() {
    assert!(is_geometry_datatype(vocab::WKT_LITERAL));
    assert!(is_geometry_datatype(vocab::GML_LITERAL));
    assert!(!is_geometry_datatype("http://www.w3.org/2001/XMLSchema#string"));

    let from_dispatch = parse_geometry_literal(
        r#"<gml:Point><gml:pos>1 2</gml:pos></gml:Point>"#,
        vocab::GML_LITERAL,
    )
    .unwrap();
    assert_eq!(from_dispatch, parse_wkt_literal("POINT(1 2)").unwrap());

    let wkt_dispatch = parse_geometry_literal("POINT(1 2)", vocab::WKT_LITERAL).unwrap();
    assert_eq!(wkt_dispatch.geometry, Geometry::Point(Point::new(1.0, 2.0)));

    // An unrecognised geometry datatype is a clean Unsupported.
    assert!(matches!(
        parse_geometry_literal("POINT(1 2)", "http://example.org/notGeometry"),
        Err(GeoError::Unsupported(_))
    ));
}

// ---- R16: an empty gmlLiteral is the empty geometry -------------------------
// sq-mzmh — GeoSPARQL Req 16, the GML counterpart of the empty
// wktLiteral (Req 13).

/// An empty / whitespace-only lexical form is the empty geometry (CRS84).
#[test]
fn empty_gml_lexical_form_is_empty_geometry() {
    for lex in ["", "   ", "\n\t "] {
        let g = parse_gml_literal(lex).expect("empty gmlLiteral parses");
        assert_eq!(g.crs, Crs::Crs84);
        assert!(g.metadata().is_empty, "lexical form {lex:?} must be empty");
        assert_eq!(g.metadata().dimension, None);
    }
}

/// A member-less GML aggregate element is the empty geometry, not a parse error.
#[test]
fn member_less_gml_aggregates_are_empty() {
    const NS: &str = "xmlns:gml=\"http://www.opengis.net/gml\"";
    for lex in [
        format!("<gml:MultiPoint {NS}/>"),
        format!("<gml:MultiPoint {NS}></gml:MultiPoint>"),
        format!("<gml:MultiCurve {NS}/>"),
        format!("<gml:MultiSurface {NS}/>"),
        format!("<gml:MultiGeometry {NS}/>"),
    ] {
        let g = parse_gml_literal(&lex).unwrap_or_else(|e| panic!("{lex} -> {e}"));
        assert!(g.metadata().is_empty, "{lex} must be the empty geometry");
    }
}

/// A NON-empty GML aggregate (a MultiGeometry with members) still parses to its
/// constituents — the empty-handling change does not weaken populated parsing.
#[test]
fn gml_multigeometry_with_members_parses() {
    let lex = "<gml:MultiGeometry xmlns:gml=\"http://www.opengis.net/gml\">\
               <gml:geometryMember><gml:Point><gml:pos>1 2</gml:pos></gml:Point></gml:geometryMember>\
               <gml:geometryMember><gml:Point><gml:pos>3 4</gml:pos></gml:Point></gml:geometryMember>\
               </gml:MultiGeometry>";
    let g = parse_gml_literal(lex).unwrap();
    assert!(!g.metadata().is_empty);
    match g.geometry {
        Geometry::GeometryCollection(gc) => assert_eq!(gc.0.len(), 2),
        other => panic!("expected a GEOMETRYCOLLECTION, got {other:?}"),
    }
}

// ---- A gmlLiteral round-trips through SPARQL geof: like its WKT twin ---------

#[cfg(feature = "engine")]
mod sparql {
    use sparq_core::Graph;
    use sparq_engine::query_with_functions;
    use sparq_geo::{geof_registry, GeoIndex};

    const PREFIXES: &str = "PREFIX geof: <http://www.opengis.net/def/function/geosparql/> \
                            PREFIX uom:  <http://www.opengis.net/def/uom/OGC/1.0/> \
                            PREFIX ex:   <http://ex/> ";

    /// London/Paris/Lyon as POINTS, but each city's location is given BOTH as a
    /// wktLiteral (ex:wloc) and as the equivalent gmlLiteral (ex:gloc); the UK
    /// box is a wktLiteral polygon.
    fn cities() -> Graph {
        Graph::load_str(
            r#"
            @prefix geo: <http://www.opengis.net/ont/geosparql#> .
            @prefix ex:  <http://ex/> .
            ex:london ex:wloc "POINT(-0.1278 51.5074)"^^geo:wktLiteral .
            ex:london ex:gloc "<gml:Point><gml:pos>-0.1278 51.5074</gml:pos></gml:Point>"^^geo:gmlLiteral .
            ex:paris  ex:wloc "POINT(2.3522 48.8566)"^^geo:wktLiteral .
            ex:paris  ex:gloc "<gml:Point><gml:pos>2.3522 48.8566</gml:pos></gml:Point>"^^geo:gmlLiteral .
            ex:lyon   ex:wloc "POINT(4.8357 45.7640)"^^geo:wktLiteral .
            ex:lyon   ex:gloc "<gml:Point><gml:pos>4.8357 45.7640</gml:pos></gml:Point>"^^geo:gmlLiteral .
            ex:uk     ex:area "POLYGON((-6 50, 2 50, 2 59, -6 59, -6 50))"^^geo:wktLiteral .
            "#,
            "turtle",
        )
        .unwrap()
    }

    fn names(r: &sparq_engine::QueryResult, col: usize) -> Vec<String> {
        r.rows.iter().map(|row| row[col].as_ref().unwrap().to_string()).collect()
    }

    #[test]
    fn distance_gml_equals_wkt() {
        // Cities within 400 km of London computed over the GML locations:
        // same answer as over WKT — Paris in, Lyon out.
        let r = query_with_functions(
            &cities(),
            &format!(
                "{PREFIXES} SELECT ?city WHERE {{ \
                   ex:london ex:gloc ?here . ?city ex:gloc ?there . \
                   FILTER(?city != ex:london && geof:distance(?here, ?there, uom:kilometre) < 400) \
                 }}"
            ),
            &geof_registry(),
        )
        .unwrap();
        assert_eq!(names(&r, 0), vec!["<http://ex/paris>"]);
    }

    #[test]
    fn mixed_gml_and_wkt_arguments_interoperate() {
        // One argument a gmlLiteral, the other a wktLiteral: geof: treats both
        // serializations identically (GeoSPARQL §8.5), so this still finds Paris.
        let r = query_with_functions(
            &cities(),
            &format!(
                "{PREFIXES} SELECT ?city WHERE {{ \
                   ex:london ex:wloc ?here . ?city ex:gloc ?there . \
                   FILTER(?city != ex:london && geof:distance(?here, ?there, uom:kilometre) < 400) \
                 }}"
            ),
            &geof_registry(),
        )
        .unwrap();
        assert_eq!(names(&r, 0), vec!["<http://ex/paris>"]);
    }

    #[test]
    fn sf_within_gml_point_in_wkt_polygon() {
        // The London GML point is within the UK wktLiteral polygon.
        let r = query_with_functions(
            &cities(),
            &format!(
                "{PREFIXES} SELECT ?city WHERE {{ \
                   ?city ex:gloc ?pt . ex:uk ex:area ?poly . \
                   FILTER(geof:sfWithin(?pt, ?poly)) \
                 }}"
            ),
            &geof_registry(),
        )
        .unwrap();
        assert_eq!(names(&r, 0), vec!["<http://ex/london>"]);
    }

    #[test]
    fn geof_buffer_of_gml_feeds_sf_within() {
        // A geometry-producing geof: over a GML arg returns a wktLiteral that
        // feeds straight back into another geof: call — Paris is within a 400 km
        // buffer of the London GML point.
        let r = query_with_functions(
            &cities(),
            &format!(
                "{PREFIXES} SELECT ?city WHERE {{ \
                   ex:london ex:gloc ?here . ?city ex:gloc ?there . \
                   FILTER(?city != ex:london && geof:sfWithin(?there, geof:buffer(?here, 400, uom:kilometre))) \
                 }}"
            ),
            &geof_registry(),
        )
        .unwrap();
        assert_eq!(names(&r, 0), vec!["<http://ex/paris>"]);
    }

    /// The GeoIndex extracts geometries from BOTH geo:asWKT and geo:asGML, and a
    /// GML-sourced point answers the same spatial query as a WKT-sourced one.
    #[test]
    fn geo_index_extracts_as_gml() {
        let g = Graph::load_str(
            r#"
            @prefix geo: <http://www.opengis.net/ont/geosparql#> .
            @prefix ex:  <http://ex/> .
            ex:london geo:asWKT "POINT(-0.1278 51.5074)"^^geo:wktLiteral .
            ex:paris  geo:asGML "<gml:Point><gml:pos>2.3522 48.8566</gml:pos></gml:Point>"^^geo:gmlLiteral .
            "#,
            "turtle",
        )
        .unwrap();
        let index = GeoIndex::build(&g);
        assert_eq!(index.len(), 2, "both asWKT and asGML geometries indexed");
        assert_eq!(index.skipped(), 0);

        use geo_types::Point;
        // Within 400 km of London: London itself + Paris (the GML-sourced one).
        let near = index.within_distance(Point::new(-0.1278, 51.5074), 400_000.0, None);
        let mut hit: Vec<String> = near.iter().map(|(t, _)| t.to_string()).collect();
        hit.sort();
        assert_eq!(hit, vec!["<http://ex/london>", "<http://ex/paris>"]);
    }
}
