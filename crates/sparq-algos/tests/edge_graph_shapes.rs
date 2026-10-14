// sq-bif.29: pin the default-feature graph-shape seams through the public API.

use sparq_algos::{
    degree_centrality, degree_centrality_normalized, label_propagation, num_communities, pagerank,
    top_k, weakly_connected_components, Direction, LabelPropConfig, NodeGraph, PageRankConfig,
};
use sparq_core::Graph;

fn build(nt: &str) -> NodeGraph {
    let graph = Graph::load_str(nt, "nt").expect("parse N-Triples fixture");
    NodeGraph::build(&graph)
}

#[test]
fn empty_graph_algorithms_return_empty_results() {
    let graph = build("");

    assert_eq!(
        pagerank(&graph, PageRankConfig::default()),
        Vec::<f64>::new()
    );
    assert_eq!(
        degree_centrality(&graph, Direction::Total),
        Vec::<usize>::new()
    );
    assert_eq!(weakly_connected_components(&graph), Vec::<usize>::new());
    assert_eq!(
        label_propagation(&graph, LabelPropConfig::default()),
        Vec::<usize>::new()
    );
    assert_eq!(num_communities(&[]), 0);
}

#[test]
fn single_directed_edge_has_expected_two_node_degrees() {
    let graph = build("<http://e/a> <http://p/k> <http://e/b> .\n");

    assert_eq!(degree_centrality(&graph, Direction::Total), vec![1, 1]);
    assert_eq!(
        degree_centrality_normalized(&graph, Direction::In),
        vec![0.0, 1.0]
    );
}

#[test]
fn top_k_breaks_score_ties_by_ascending_index() {
    assert_eq!(top_k(&[5, 5, 1, 5], 2), vec![(0, 5), (1, 5)]);
}

#[test]
fn disconnected_edges_count_as_two_components() {
    let graph = build(
        "<http://e/a> <http://p/k> <http://e/b> .\n\
         <http://e/c> <http://p/k> <http://e/d> .\n",
    );

    let components = weakly_connected_components(&graph);
    assert_eq!(num_communities(&components), 2);
}
