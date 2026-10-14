//! `geof:` functions through REAL SPARQL: every test parses a query with
//! `geof:` IRIs in FILTER / BIND / SELECT expressions and runs it on
//! sparq-engine via `query_with_functions(…, &geof_registry())` — the wiring a
//! GeoSPARQL user actually exercises (engine `Function::Custom` dispatch ->
//! registry -> `geof::lex`).
#![cfg(feature = "engine")]

use sparq_core::Graph;
use sparq_engine::query_with_functions;
use sparq_geo::geof_registry;

const PREFIXES: &str = "PREFIX geof: <http://www.opengis.net/def/function/geosparql/> \
                        PREFIX uom:  <http://www.opengis.net/def/uom/OGC/1.0/> \
                        PREFIX ex:   <http://ex/> ";

/// Three cities (points) and two areas (polygons): the UK box contains London,
/// the France box contains Paris and Lyon.
fn cities() -> Graph {
    Graph::load_str(
        r#"
        @prefix geo: <http://www.opengis.net/ont/geosparql#> .
        @prefix ex:  <http://ex/> .
        ex:london ex:loc  "POINT(-0.1278 51.5074)"^^geo:wktLiteral .
        ex:paris  ex:loc  "POINT(2.3522 48.8566)"^^geo:wktLiteral .
        ex:lyon   ex:loc  "POINT(4.8357 45.7640)"^^geo:wktLiteral .
        ex:uk     ex:area "POLYGON((-6 50, 2 50, 2 59, -6 59, -6 50))"^^geo:wktLiteral .
        ex:france ex:area "POLYGON((-1 42.5, 7 42.5, 7 51, -1 51, -1 42.5))"^^geo:wktLiteral .
        "#,
        "turtle",
    )
    .unwrap()
}

fn names(r: &sparq_engine::QueryResult, col: usize) -> Vec<String> {
    r.rows.iter().map(|row| row[col].as_ref().unwrap().to_string()).collect()
}

#[test]
fn filter_distance() {
    // Cities within 400 km of London: Paris (≈343.6 km), not Lyon (≈750 km).
    let r = query_with_functions(
        &cities(),
        &format!(
            "{PREFIXES} SELECT ?city WHERE {{ \
               ex:london ex:loc ?here . ?city ex:loc ?there . \
               FILTER(?city != ex:london && geof:distance(?here, ?there, uom:kilometre) < 400) \
             }}"
        ),
        &geof_registry(),
    )
    .unwrap();
    assert_eq!(names(&r, 0), vec!["<http://ex/paris>"]);
}

#[test]
fn bind_distance_value() {
    let r = query_with_functions(
        &cities(),
        &format!(
            "{PREFIXES} SELECT ?km WHERE {{ \
               ex:london ex:loc ?a . ex:paris ex:loc ?b . \
               BIND(geof:distance(?a, ?b, uom:kilometre) AS ?km) \
             }}"
        ),
        &geof_registry(),
    )
    .unwrap();
    assert_eq!(r.len(), 1);
    let term = r.rows[0][0].as_ref().expect("?km must be bound").to_string();
    let km: f64 = term
        .strip_prefix('"')
        .and_then(|s| s.split('"').next())
        .unwrap()
        .parse()
        .unwrap();
    assert!((km - 343.6).abs() < 1.0, "London–Paris ≈ 343.6 km, got {km}");
    assert!(term.ends_with("^^<http://www.w3.org/2001/XMLSchema#double>"), "got {term}");
}

#[test]
fn metric_measurements_and_centroid_through_sparql() {
    // sq-lsp7k.18: exercise custom-function dispatch, numeric RDF
    // typing, and geometry-result re-entry through the real SPARQL evaluator.
    let r = query_with_functions(
        &cities(),
        &format!(
            "{PREFIXES} SELECT ?area ?length ?perimeter ?centroid WHERE {{ \
               ex:france ex:area ?g . \
               BIND(geof:metricArea(?g) AS ?area) \
               BIND(geof:metricLength(?g) AS ?length) \
               BIND(geof:metricPerimeter(?g) AS ?perimeter) \
               BIND(geof:centroid(?g) AS ?centroid) \
             }}"
        ),
        &geof_registry(),
    )
    .unwrap();
    assert_eq!(r.len(), 1);

    let values = names(&r, 0)
        .into_iter()
        .chain(names(&r, 1))
        .chain(names(&r, 2))
        .collect::<Vec<_>>();
    let numeric = values
        .iter()
        .map(|term| {
            assert!(term.ends_with("^^<http://www.w3.org/2001/XMLSchema#double>"));
            term.strip_prefix('"')
                .and_then(|value| value.split('"').next())
                .unwrap()
                .parse::<f64>()
                .unwrap()
        })
        .collect::<Vec<_>>();
    assert!(numeric.iter().all(|value| *value > 0.0));
    assert_eq!(numeric[1], numeric[2]);

    let centroid = names(&r, 3).pop().unwrap();
    assert!(centroid.contains("POINT"), "got {centroid}");
    assert!(centroid.ends_with(
        "^^<http://www.opengis.net/ont/geosparql#wktLiteral>"
    ));
}

#[test]
fn spatial_join_sf_within() {
    // Which city lies within which area — a real spatial join (FILTER over the
    // cross product of 3 points x 2 polygons).
    let r = query_with_functions(
        &cities(),
        &format!(
            "{PREFIXES} SELECT ?city ?region WHERE {{ \
               ?city ex:loc ?pt . ?region ex:area ?poly . \
               FILTER(geof:sfWithin(?pt, ?poly)) \
             }} ORDER BY ?city"
        ),
        &geof_registry(),
    )
    .unwrap();
    assert_eq!(
        r.rows
            .iter()
            .map(|row| (row[0].as_ref().unwrap().to_string(), row[1].as_ref().unwrap().to_string()))
            .collect::<Vec<_>>(),
        vec![
            ("<http://ex/london>".to_string(), "<http://ex/uk>".to_string()),
            ("<http://ex/lyon>".to_string(), "<http://ex/france>".to_string()),
            ("<http://ex/paris>".to_string(), "<http://ex/france>".to_string()),
        ]
    );
}

#[test]
fn unary_geometry_function_roundtrips_through_relations() {
    // geof:envelope returns a geo:wktLiteral that feeds straight back into
    // another geof: function within the same expression.
    let r = query_with_functions(
        &cities(),
        &format!(
            "{PREFIXES} SELECT ?region WHERE {{ \
               ?region ex:area ?poly . ex:paris ex:loc ?pt . \
               FILTER(geof:sfContains(geof:envelope(?poly), ?pt)) \
             }}"
        ),
        &geof_registry(),
    )
    .unwrap();
    assert_eq!(names(&r, 0), vec!["<http://ex/france>"]);
}

#[test]
fn simplify_through_sparql_drops_the_midpoint() {
    // sq-lsp7k.23: witness the real custom-function dispatch, numeric
    // tolerance parsing, and geometry-literal result type.
    let graph = Graph::load_str(
        r#"@prefix geo: <http://www.opengis.net/ont/geosparql#> .
           <http://ex/line> <http://ex/loc> "LINESTRING(0 0,1 0.1,2 0)"^^geo:wktLiteral ."#,
        "turtle",
    )
    .unwrap();
    let result = query_with_functions(
        &graph,
        &format!(
            "{PREFIXES} SELECT ?simplified WHERE {{ \
               ex:line ex:loc ?line . \
               BIND(geof:simplify(?line, 0.2) AS ?simplified) \
             }}"
        ),
        &geof_registry(),
    )
    .unwrap();

    assert_eq!(
        names(&result, 0),
        vec![
            "\"LINESTRING(0 0,2 0)\"^^<http://www.opengis.net/ont/geosparql#wktLiteral>"
                .to_string()
        ]
    );
}

#[test]
fn select_expression_boolean() {
    let r = query_with_functions(
        &cities(),
        &format!(
            "{PREFIXES} SELECT (geof:sfDisjoint(?a, ?b) AS ?d) WHERE {{ \
               ex:uk ex:area ?a . ex:france ex:area ?b . \
             }}"
        ),
        &geof_registry(),
    )
    .unwrap();
    assert_eq!(
        names(&r, 0),
        vec!["\"false\"^^<http://www.w3.org/2001/XMLSchema#boolean>"] // the boxes overlap near Calais
    );
}

#[test]
fn geo_errors_are_expression_errors_not_query_errors() {
    let g = Graph::load_str(
        r#"
        @prefix geo: <http://www.opengis.net/ont/geosparql#> .
        @prefix ex:  <http://ex/> .
        ex:good ex:loc "POINT(0 0)"^^geo:wktLiteral .
        ex:bad  ex:loc "POINT(not wkt)"^^geo:wktLiteral .
        ex:str  ex:loc "POINT(1 1)" .
        "#,
        "turtle",
    )
    .unwrap();
    // A malformed wktLiteral / plain-string geometry errors PER ROW: the BIND
    // leaves ?e unbound, the query succeeds, the good row still computes.
    let r = query_with_functions(
        &g,
        &format!("{PREFIXES} SELECT ?s ?e WHERE {{ ?s ex:loc ?g . BIND(geof:envelope(?g) AS ?e) }} ORDER BY ?s"),
        &geof_registry(),
    )
    .unwrap();
    assert_eq!(r.len(), 3);
    let bound: Vec<bool> = r.rows.iter().map(|row| row[1].is_some()).collect();
    assert_eq!(bound, vec![false, true, false]); // ex:bad, ex:good, ex:str
    // In a FILTER the same error just drops the row.
    let r = query_with_functions(
        &g,
        &format!(
            "{PREFIXES} SELECT ?s WHERE {{ ?s ex:loc ?g . \
               FILTER(geof:sfIntersects(?g, ?g)) }}"
        ),
        &geof_registry(),
    )
    .unwrap();
    assert_eq!(names(&r, 0), vec!["<http://ex/good>"]);
}

/// Evaluates one binary `geof:` relation through REAL SPARQL on a two-triple
/// graph: `SELECT (geof:<name>(?ga, ?gb) AS ?r)`.
fn eval_relation(name: &str, a: &str, b: &str) -> bool {
    let g = Graph::load_str(
        &format!(
            r#"<http://x/a> <http://x/g> "{a}"^^<http://www.opengis.net/ont/geosparql#wktLiteral> .
               <http://x/b> <http://x/g> "{b}"^^<http://www.opengis.net/ont/geosparql#wktLiteral> ."#
        ),
        "ntriples",
    )
    .unwrap();
    let r = query_with_functions(
        &g,
        &format!(
            "{PREFIXES} SELECT (geof:{name}(?ga, ?gb) AS ?r) WHERE {{ \
               <http://x/a> <http://x/g> ?ga . <http://x/b> <http://x/g> ?gb }}"
        ),
        &geof_registry(),
    )
    .unwrap();
    assert_eq!(r.len(), 1);
    match r.rows[0][0].as_ref().expect("?r must be bound").to_string().as_str() {
        "\"true\"^^<http://www.w3.org/2001/XMLSchema#boolean>" => true,
        "\"false\"^^<http://www.w3.org/2001/XMLSchema#boolean>" => false,
        other => panic!("geof:{name} returned {other}"),
    }
}

#[test]
fn egenhofer_family_through_sparql() {
    const SMALL: &str = "POLYGON((1 1, 2 1, 2 2, 1 2, 1 1))"; // strictly inside BIG
    const TANGENT: &str = "POLYGON((0 0, 1 0, 1 1, 0 1, 0 0))"; // inside BIG, shares boundary
    const BIG: &str = "POLYGON((0 0, 4 0, 4 4, 0 4, 0 0))";
    const ADJACENT: &str = "POLYGON((4 0, 6 0, 6 2, 4 2, 4 0))"; // shares an edge with BIG
    const FAR: &str = "POLYGON((9 9, 10 9, 10 10, 9 10, 9 9))";
    const OVERLAP: &str = "POLYGON((3 3, 6 3, 6 6, 3 6, 3 3))"; // overlaps BIG

    // (name, a, b, expected)
    for (name, a, b, want) in [
        ("ehEquals", BIG, BIG, true),
        ("ehEquals", BIG, SMALL, false),
        ("ehDisjoint", BIG, FAR, true),
        ("ehDisjoint", BIG, OVERLAP, false),
        ("ehMeet", BIG, ADJACENT, true),
        ("ehMeet", BIG, OVERLAP, false),
        ("ehOverlap", BIG, OVERLAP, true),
        ("ehOverlap", BIG, ADJACENT, false),
        ("ehInside", SMALL, BIG, true),
        ("ehInside", TANGENT, BIG, false), // tangential: coveredBy, not inside
        ("ehContains", BIG, SMALL, true),
        ("ehContains", BIG, TANGENT, false),
        ("ehCoveredBy", TANGENT, BIG, true),
        ("ehCoveredBy", SMALL, BIG, false), // strict containment is inside, not coveredBy
        ("ehCovers", BIG, TANGENT, true),
        ("ehCovers", BIG, SMALL, false),
    ] {
        assert_eq!(eval_relation(name, a, b), want, "geof:{name}({a}, {b})");
    }
}

#[test]
fn rcc8_family_through_sparql() {
    const SMALL: &str = "POLYGON((1 1, 2 1, 2 2, 1 2, 1 1))";
    const TANGENT: &str = "POLYGON((0 0, 1 0, 1 1, 0 1, 0 0))";
    const BIG: &str = "POLYGON((0 0, 4 0, 4 4, 0 4, 0 0))";
    const ADJACENT: &str = "POLYGON((4 0, 6 0, 6 2, 4 2, 4 0))";
    const FAR: &str = "POLYGON((9 9, 10 9, 10 10, 9 10, 9 9))";
    const OVERLAP: &str = "POLYGON((3 3, 6 3, 6 6, 3 6, 3 3))";

    for (name, a, b, want) in [
        ("rcc8eq", BIG, BIG, true),
        ("rcc8eq", BIG, SMALL, false),
        ("rcc8dc", BIG, FAR, true),
        ("rcc8dc", BIG, ADJACENT, false), // touching is ec, not dc
        ("rcc8ec", BIG, ADJACENT, true),
        ("rcc8ec", BIG, OVERLAP, false),
        ("rcc8po", BIG, OVERLAP, true),
        ("rcc8po", BIG, SMALL, false),
        ("rcc8ntpp", SMALL, BIG, true),
        ("rcc8ntpp", TANGENT, BIG, false),
        ("rcc8ntppi", BIG, SMALL, true),
        ("rcc8ntppi", BIG, TANGENT, false),
        ("rcc8tpp", TANGENT, BIG, true),
        ("rcc8tpp", SMALL, BIG, false),
        ("rcc8tppi", BIG, TANGENT, true),
        ("rcc8tppi", BIG, SMALL, false),
    ] {
        assert_eq!(eval_relation(name, a, b), want, "geof:{name}({a}, {b})");
    }
}

#[test]
fn relate_generic_de9im_through_sparql() {
    // Generic DE-9IM: the "within" pattern for a point in a polygon.
    let r = query_with_functions(
        &cities(),
        &format!(
            "{PREFIXES} SELECT ?city WHERE {{ \
               ?city ex:loc ?pt . ex:uk ex:area ?poly . \
               FILTER(geof:relate(?pt, ?poly, \"T*F**F***\")) \
             }}"
        ),
        &geof_registry(),
    )
    .unwrap();
    assert_eq!(names(&r, 0), vec!["<http://ex/london>"]);
    // A malformed pattern is an expression error (row dropped), not a hard error.
    let r = query_with_functions(
        &cities(),
        &format!(
            "{PREFIXES} SELECT ?city WHERE {{ \
               ?city ex:loc ?pt . ex:uk ex:area ?poly . \
               FILTER(geof:relate(?pt, ?poly, \"BOGUS\")) \
             }}"
        ),
        &geof_registry(),
    )
    .unwrap();
    assert_eq!(r.len(), 0);
}

#[test]
fn get_srid_through_sparql() {
    let g = Graph::load_str(
        r#"
        @prefix geo: <http://www.opengis.net/ont/geosparql#> .
        @prefix ex:  <http://ex/> .
        ex:default ex:loc "POINT(0 0)"^^geo:wktLiteral .
        ex:bng ex:loc "<http://www.opengis.net/def/crs/EPSG/0/27700> POINT(530000 180000)"^^geo:wktLiteral .
        "#,
        "turtle",
    )
    .unwrap();
    // Select entities whose geometry is in the DEFAULT CRS (CRS84).
    let r = query_with_functions(
        &g,
        &format!(
            "{PREFIXES} SELECT ?s ?srid WHERE {{ \
               ?s ex:loc ?g . BIND(geof:getSRID(?g) AS ?srid) \
             }} ORDER BY ?s"
        ),
        &geof_registry(),
    )
    .unwrap();
    assert_eq!(
        names(&r, 1),
        vec![
            "\"http://www.opengis.net/def/crs/EPSG/0/27700\"^^<http://www.w3.org/2001/XMLSchema#anyURI>",
            "\"http://www.opengis.net/def/crs/OGC/1.3/CRS84\"^^<http://www.w3.org/2001/XMLSchema#anyURI>",
        ]
    );
}

#[test]
fn set_operations_through_sparql() {
    // intersection / union / difference / symDifference of the UK and France
    // boxes, each fed back into geof: relations within the same query.
    let q = |expr: &str| {
        query_with_functions(
            &cities(),
            &format!(
                "{PREFIXES} SELECT (geof:sfContains({expr}, ?pt) AS ?r) WHERE {{ \
                   ex:uk ex:area ?a . ex:france ex:area ?b . ex:london ex:loc ?pt }}"
            ),
            &geof_registry(),
        )
        .unwrap()
    };
    // London is in uk minus france, in the union, not in the overlap strip, and
    // in the symmetric difference.
    assert_eq!(names(&q("geof:difference(?a, ?b)"), 0)[0], "\"true\"^^<http://www.w3.org/2001/XMLSchema#boolean>");
    assert_eq!(names(&q("geof:union(?a, ?b)"), 0)[0], "\"true\"^^<http://www.w3.org/2001/XMLSchema#boolean>");
    assert_eq!(names(&q("geof:intersection(?a, ?b)"), 0)[0], "\"false\"^^<http://www.w3.org/2001/XMLSchema#boolean>");
    assert_eq!(names(&q("geof:symDifference(?a, ?b)"), 0)[0], "\"true\"^^<http://www.w3.org/2001/XMLSchema#boolean>");

    // Mixed-dimension union (point ∪ polygon) now yields a GEOMETRYCOLLECTION
    // that CONTAINS the point — so every city's loc is contained in its union
    // with the UK box.
    let r = query_with_functions(
        &cities(),
        &format!(
            "{PREFIXES} SELECT ?s WHERE {{ ?s ex:loc ?pt . ex:uk ex:area ?a . \
               FILTER(geof:sfIntersects(geof:union(?pt, ?a), ?pt)) }}"
        ),
        &geof_registry(),
    )
    .unwrap();
    assert_eq!(r.len(), 3);

    // …but a GENUINELY-unsupported combo (1-D set subtraction) is still a
    // per-row expression error, not a hard query failure: the FILTER errors on
    // every row, so the result is empty (zero rows), query still Ok.
    let r = query_with_functions(
        &cities(),
        &format!(
            "{PREFIXES} SELECT ?s WHERE {{ ?s ex:loc ?pt . \
               BIND(\"LINESTRING(0 0, 2 0)\"^^<http://www.opengis.net/ont/geosparql#wktLiteral> AS ?l1) \
               BIND(\"LINESTRING(1 0, 3 0)\"^^<http://www.opengis.net/ont/geosparql#wktLiteral> AS ?l2) \
               FILTER(geof:sfIntersects(geof:difference(?l1, ?l2), ?pt)) }}"
        ),
        &geof_registry(),
    )
    .unwrap();
    assert_eq!(r.len(), 0);
}

#[test]
fn buffer_through_sparql() {
    // Cities within a 400 km buffer around London: Paris (≈343.6 km), not Lyon.
    let r = query_with_functions(
        &cities(),
        &format!(
            "{PREFIXES} SELECT ?city WHERE {{ \
               ex:london ex:loc ?here . ?city ex:loc ?there . \
               FILTER(?city != ex:london && \
                      geof:sfWithin(?there, geof:buffer(?here, 400, uom:kilometre))) \
             }}"
        ),
        &geof_registry(),
    )
    .unwrap();
    assert_eq!(names(&r, 0), vec!["<http://ex/paris>"]);
}

#[test]
fn unregistered_geof_iri_stays_a_hard_error() {
    // geof:gmlToWkt does not exist (gmlLiteral is not supported): it is NOT in
    // the registry, so the engine raises the same hard error as for any unknown
    // function IRI.
    let err = query_with_functions(
        &cities(),
        &format!("{PREFIXES} SELECT ?s WHERE {{ ?s ex:loc ?g . FILTER(geof:gmlToWkt(?g) = ?g) }}"),
        &geof_registry(),
    )
    .map(|r| r.len())
    .unwrap_err();
    assert!(err.contains("unsupported SPARQL function"), "got: {err}");
}

#[test]
fn constant_polygon_filter_cold_and_warm_runs_agree() {
    // sq-lkrgi: the Geographica large-constant selection shape — one
    // constant POLYGON literal in the FILTER, evaluated against every row. The
    // registry now serves the constant from a per-thread parsed-geometry cache
    // after the first row; a cold run and a warm re-run (same thread, cache
    // already populated) must return identical rows, and both must match the
    // hand-derived answer.
    let g = cities();
    let q = format!(
        "{PREFIXES} SELECT ?city WHERE {{ \
           ?city ex:loc ?there . \
           FILTER(geof:sfWithin(?there, \
             \"POLYGON((-1 42.5, 7 42.5, 7 51, -1 51, -1 42.5))\"^^<http://www.opengis.net/ont/geosparql#wktLiteral>)) \
         }} ORDER BY ?city"
    );
    let reg = geof_registry();
    let cold = query_with_functions(&g, &q, &reg).unwrap();
    let warm = query_with_functions(&g, &q, &reg).unwrap();
    let expected = vec!["<http://ex/lyon>".to_string(), "<http://ex/paris>".to_string()];
    assert_eq!(names(&cold, 0), expected);
    assert_eq!(names(&warm, 0), expected, "warm (cached) run must equal the cold run");
}

#[test]
#[cfg(feature = "geof_accessors")]
fn geof_accessor_functions_through_sparql() {
    // sq-lsp7k: the GeoSPARQL 1.1 non-topological accessors
    // (opt-in `geof_accessors` feature) dispatch through real SPARQL BIND —
    // integer/boolean/anyURI result typing included.
    let r = query_with_functions(
        &cities(),
        &format!(
            "{PREFIXES} SELECT ?d ?cd ?sd ?simple ?ty WHERE {{ \
               ex:uk ex:area ?g . \
               BIND(geof:dimension(?g) AS ?d) \
               BIND(geof:coordinateDimension(?g) AS ?cd) \
               BIND(geof:spatialDimension(?g) AS ?sd) \
               BIND(geof:isSimple(?g) AS ?simple) \
               BIND(geof:geometryType(?g) AS ?ty) \
             }}"
        ),
        &geof_registry(),
    )
    .unwrap();
    assert_eq!(r.len(), 1);
    let int = |v: &str| vec![format!("\"{v}\"^^<http://www.w3.org/2001/XMLSchema#integer>")];
    assert_eq!(names(&r, 0), int("2"), "geof:dimension(polygon)");
    assert_eq!(names(&r, 1), int("2"), "geof:coordinateDimension");
    assert_eq!(names(&r, 2), int("2"), "geof:spatialDimension");
    assert_eq!(
        names(&r, 3),
        vec!["\"true\"^^<http://www.w3.org/2001/XMLSchema#boolean>".to_string()]
    );
    assert_eq!(
        names(&r, 4),
        vec![
            "\"http://www.opengis.net/ont/sf#Polygon\"^^<http://www.w3.org/2001/XMLSchema#anyURI>"
                .to_string()
        ]
    );
}

#[test]
#[cfg(feature = "geof_accessors")]
fn geof_accessor_empty_geometry_is_a_per_row_expression_error() {
    // sq-lsp7k: an accessor with NO defined value (the empty
    // geometry's dimension) is a per-row expression error — the row is
    // FILTERed out / left unbound, never a fabricated integer and never a
    // hard query error.
    let graph = Graph::load_str(
        r#"@prefix geo: <http://www.opengis.net/ont/geosparql#> .
           <http://ex/somewhere> <http://ex/loc> "POINT(1 2)"^^geo:wktLiteral .
           <http://ex/nowhere>   <http://ex/loc> "POINT EMPTY"^^geo:wktLiteral ."#,
        "turtle",
    )
    .unwrap();
    let r = query_with_functions(
        &graph,
        &format!(
            "{PREFIXES} SELECT ?s WHERE {{ \
               ?s ex:loc ?g . FILTER(geof:dimension(?g) >= 0) \
             }}"
        ),
        &geof_registry(),
    )
    .unwrap();
    assert_eq!(names(&r, 0), vec!["<http://ex/somewhere>"]);
}
