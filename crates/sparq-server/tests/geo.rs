//! The opt-in `geo` cargo feature: GeoSPARQL `geof:` functions over real HTTP.
//!
//! Runs only with `cargo test -p sparq-server --features geo`. Each test boots the
//! actual axum server in-process and drives `/sparql` with queries/updates whose
//! FILTER/BIND expressions call `geof:` — proving the server installs sparq-geo's
//! registry on the query path (spawn_blocking workers) and the update path (the
//! sequenced writer's `ServerApplier`, both the per-batch fork and the in-place
//! application).
#![cfg(feature = "geo")]

use sparq_core::Graph;
use sparq_server::{router, AppState};
use tokio::net::TcpListener;

const DATA: &str = r#"
    @prefix geo: <http://www.opengis.net/ont/geosparql#> .
    @prefix ex:  <http://ex/> .
    ex:london ex:loc "POINT(-0.1278 51.5074)"^^geo:wktLiteral .
    ex:paris  ex:loc "POINT(2.3522 48.8566)"^^geo:wktLiteral .
    ex:lyon   ex:loc "POINT(4.8357 45.7640)"^^geo:wktLiteral .
"#;

/// [SONNET-4.6] sq-6ep — a GeoSPARQL-VOCABULARY fixture (`geo:Feature` /
/// `geo:hasGeometry` / `geo:asWKT`), as opposed to [`DATA`], which hangs WKT
/// literals off a plain `ex:loc` predicate. Used by the protocol-level probe the
/// OGC R1 row cites.
const GEO_VOCAB_DATA: &str = r#"
    @prefix geo: <http://www.opengis.net/ont/geosparql#> .
    @prefix ex:  <http://ex/> .
    ex:london a geo:Feature ; geo:hasGeometry ex:londonGeom .
    ex:londonGeom a geo:Geometry ;
        geo:asWKT "POINT(-0.1278 51.5074)"^^geo:wktLiteral .
"#;

const PREFIXES: &str = "PREFIX geof: <http://www.opengis.net/def/function/geosparql/> \
                        PREFIX uom:  <http://www.opengis.net/def/uom/OGC/1.0/> \
                        PREFIX ex:   <http://ex/> ";

async fn spawn() -> String {
    spawn_with(DATA).await
}

async fn spawn_with(data: &str) -> String {
    let graph = Graph::load_str(data, "turtle").unwrap();
    let app = router(AppState::new(graph));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("http://{addr}")
}

#[tokio::test]
async fn select_with_geof_distance_filter() {
    let base = spawn().await;
    // Cities within 400 km of London: Paris (≈343.6 km) only.
    let resp = reqwest::Client::new()
        .get(format!("{base}/sparql"))
        .query(&[(
            "query",
            format!(
                "{PREFIXES} SELECT ?city WHERE {{ \
                   ex:london ex:loc ?here . ?city ex:loc ?there . \
                   FILTER(?city != ex:london && geof:distance(?here, ?there, uom:kilometre) < 400) \
                 }}"
            ),
        )])
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body = resp.text().await.unwrap();
    assert!(body.contains("http://ex/paris"), "got: {body}");
    assert!(!body.contains("http://ex/lyon"), "got: {body}");
}

#[tokio::test]
async fn ask_with_geof_relation() {
    let base = spawn().await;
    let resp = reqwest::Client::new()
        .get(format!("{base}/sparql"))
        .query(&[(
            "query",
            format!(
                "{PREFIXES} ASK {{ ex:paris ex:loc ?pt . \
                   FILTER(geof:sfWithin(?pt, \"POLYGON((-1 42.5, 7 42.5, 7 51, -1 51, -1 42.5))\"^^<http://www.opengis.net/ont/geosparql#wktLiteral>)) }}"
            ),
        )])
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body = resp.text().await.unwrap();
    assert!(body.contains("\"boolean\":true") || body.contains("\"boolean\": true"), "got: {body}");
}

#[tokio::test]
async fn update_where_with_geof_filter() {
    let base = spawn().await;
    let client = reqwest::Client::new();
    // The update path (writer fork + in-place application): classify cities near London.
    let update = format!(
        "{PREFIXES} INSERT {{ ?city ex:nearLondon true }} WHERE {{ \
           ex:london ex:loc ?here . ?city ex:loc ?there . \
           FILTER(?city != ex:london && geof:distance(?here, ?there, uom:kilometre) < 400) \
         }}"
    );
    // Twice: each batch exercises both ServerApplier::fork and ServerApplier::apply.
    for _ in 0..2 {
        let resp = client
            .post(format!("{base}/sparql"))
            .header("content-type", "application/sparql-update")
            .body(update.clone())
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 204);
    }
    let resp = client
        .get(format!("{base}/sparql"))
        .query(&[("query", format!("{PREFIXES} SELECT ?c WHERE {{ ?c ex:nearLondon true }}"))])
        .send()
        .await
        .unwrap();
    let body = resp.text().await.unwrap();
    assert!(body.contains("http://ex/paris"), "got: {body}");
    assert!(!body.contains("http://ex/lyon"), "got: {body}");
}

/// [SONNET-4.6] sq-6ep — the SPARQL-PROTOCOL half of OGC GeoSPARQL R1 ("support
/// SPARQL Query + Protocol + the `geo:` ontology vocabulary"): a `geo:Feature` /
/// `geo:hasGeometry` / `geo:asWKT` graph pattern answered over real HTTP by the
/// `/sparql` endpoint.
///
/// The other tests in this file drive `geof:` FUNCTIONS over a plain `ex:loc`
/// predicate, so they exercise none of the `geo:` vocabulary; this one is what
/// lets sparq-geo's requirements scoreboard
/// (`crates/sparq-geo/tests/ogc_geosparql_requirements.rs`, R1) cite a
/// protocol-level probe over that vocabulary instead of resting the Protocol
/// conjunct on an in-process evaluator, which is not an endpoint.
#[tokio::test]
async fn geo_vocabulary_graph_pattern_over_http() {
    let base = spawn_with(GEO_VOCAB_DATA).await;
    let resp = reqwest::Client::new()
        .get(format!("{base}/sparql"))
        .query(&[(
            "query",
            "PREFIX geo: <http://www.opengis.net/ont/geosparql#> \
             SELECT ?wkt WHERE { ?f a geo:Feature ; geo:hasGeometry ?g . \
                                 ?g geo:asWKT ?wkt }"
                .to_string(),
        )])
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body = resp.text().await.unwrap();
    assert!(body.contains("POINT(-0.1278 51.5074)"), "got: {body}");
}

#[tokio::test]
async fn unregistered_geof_iri_is_still_an_engine_error() {
    let base = spawn().await;
    // geof:gmlToWkt is NOT in the registry — geo registers the KNOWN geof:
    // functions, not the whole geof: namespace, so an unregistered geof: IRI is
    // still an unknown extension function. The engine's hard unknown-function
    // error surfaces as the server's 500 engine-error response, not a 200.
    //
    // (geof:buffer would NOT work here: it IS registered, so a wrong-arity call
    // is a per-row expression error → 200, never a hard query error. This must
    // be a genuinely unregistered IRI — see sparq-geo's
    // `unregistered_geof_iri_stays_a_hard_error`.)
    let resp = reqwest::Client::new()
        .get(format!("{base}/sparql"))
        .query(&[(
            "query",
            format!("{PREFIXES} SELECT ?s WHERE {{ ?s ex:loc ?g . FILTER(geof:gmlToWkt(?g) = ?g) }}"),
        )])
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 500);
    // The server SANITIZES engine error strings (they can embed loaded-graph term
    // text) down to a stable generic class message; the detailed
    // "unsupported SPARQL function: …" cause goes only to the server log. So the
    // 500 status is the load-bearing assertion that the unknown geof: IRI is a
    // hard engine error, and the body is the generic sanitized class.
    let body = resp.text().await.unwrap();
    assert!(body.contains("query execution error"), "got: {body}");
}
