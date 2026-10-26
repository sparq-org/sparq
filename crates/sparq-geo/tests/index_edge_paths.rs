//! Direct GeoIndex convenience/error-path coverage. sq-bif.19

use sparq_core::Graph;
use sparq_geo::{GeoError, GeoIndex};

fn point_graph() -> Graph {
    let nt = r##"<http://e.org/near> <http://www.opengis.net/ont/geosparql#asWKT> "POINT(0.01 50)"^^<http://www.opengis.net/ont/geosparql#wktLiteral> .
<http://e.org/far> <http://www.opengis.net/ont/geosparql#asWKT> "POINT(1 50)"^^<http://www.opengis.net/ont/geosparql#wktLiteral> .
"##;
    Graph::load_str(nt, "ntriples").expect("valid point fixture")
}

#[test]
fn within_distance_wkt_returns_known_point_in_radius() {
    let graph = point_graph();
    let index = GeoIndex::build(&graph);

    let hits = index
        .within_distance_wkt("POINT(0 50)", 2_000.0, None)
        .expect("valid point center");

    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].0.to_string(), "<http://e.org/near>");
    assert!(
        (700.0..720.0).contains(&hits[0].1),
        "unexpected distance: {}",
        hits[0].1
    );
}

#[test]
fn within_distance_wkt_rejects_malformed_wkt() {
    let graph = point_graph();
    let index = GeoIndex::build(&graph);

    assert!(matches!(
        index.within_distance_wkt("not valid wkt", 2_000.0, None),
        Err(GeoError::Parse(_))
    ));
}

#[test]
fn within_distance_wkt_requires_a_point_center() {
    let graph = point_graph();
    let index = GeoIndex::build(&graph);

    let error = index
        .within_distance_wkt("POLYGON((0 49, 1 49, 1 51, 0 51, 0 49))", 2_000.0, None)
        .expect_err("polygon center must be rejected");
    assert!(
        matches!(&error, GeoError::Unsupported(message) if message.contains("Point")),
        "unexpected error: {error}"
    );
}

#[test]
fn intersects_wkt_rejects_malformed_wkt() {
    let graph = point_graph();
    let index = GeoIndex::build(&graph);

    assert!(matches!(
        index.intersects_wkt("not valid wkt"),
        Err(GeoError::Parse(_))
    ));
}

#[test]
fn apply_delta_with_a_different_graph_invalidates_indexed_ids() {
    let graph_a = point_graph();
    let graph_b = Graph::load_str(
        r##"<http://e.org/other> <http://www.opengis.net/ont/geosparql#asWKT> "POINT(2 50)"^^<http://www.opengis.net/ont/geosparql#wktLiteral> ."##,
        "ntriples",
    )
    .expect("valid second graph fixture");
    let graph_a_dict_ptr = std::ptr::from_ref(graph_a.dict()) as usize;
    let graph_b_dict_ptr = std::ptr::from_ref(graph_b.dict()) as usize;
    let mut index = GeoIndex::build(&graph_a);

    assert!(index.indexed_ids_for(graph_a_dict_ptr).is_some());
    index.apply_delta(&graph_b, &[], &[]);

    assert!(index.indexed_ids_for(graph_a_dict_ptr).is_none());
    assert!(index.indexed_ids_for(graph_b_dict_ptr).is_none());
}
