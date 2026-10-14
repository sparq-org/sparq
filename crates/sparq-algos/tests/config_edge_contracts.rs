//! sq-bif.42: exact contracts for boundary-valued algorithm configuration.

use sparq_algos::{label_propagation, pagerank, top_k, LabelPropConfig, NodeGraph, PageRankConfig};
use sparq_core::Graph;

fn node_graph(ntriples: &str) -> NodeGraph {
    let graph = Graph::load_str(ntriples, "nt").expect("valid N-Triples fixture");
    NodeGraph::build(&graph)
}

#[test]
fn pagerank_with_zero_damping_is_uniform() {
    let graph = node_graph(
        r#"
<http://example.com/a> <http://example.com/edge> <http://example.com/b> .
<http://example.com/b> <http://example.com/edge> <http://example.com/c> .
"#,
    );

    let ranks = pagerank(
        &graph,
        PageRankConfig {
            damping: 0.0,
            ..PageRankConfig::default()
        },
    );

    assert_eq!(ranks.len(), 3);
    for rank in ranks {
        assert!((rank - 1.0 / 3.0).abs() < 1e-12, "rank was {rank}");
    }
}

#[test]
fn pagerank_with_full_damping_is_uniform_on_directed_cycle() {
    let graph = node_graph(
        r#"
<http://example.com/a> <http://example.com/edge> <http://example.com/b> .
<http://example.com/b> <http://example.com/edge> <http://example.com/c> .
<http://example.com/c> <http://example.com/edge> <http://example.com/a> .
"#,
    );

    let ranks = pagerank(
        &graph,
        PageRankConfig {
            damping: 1.0,
            ..PageRankConfig::default()
        },
    );

    assert_eq!(ranks.len(), 3);
    for rank in ranks {
        assert!((rank - 1.0 / 3.0).abs() < 1e-9, "rank was {rank}");
    }
}

#[test]
fn top_k_with_zero_k_is_empty() {
    assert_eq!(top_k(&[5, 13, 8], 0), Vec::<(usize, usize)>::new());
}

#[test]
fn label_propagation_with_zero_sweeps_preserves_singletons() {
    let graph =
        node_graph("<http://example.com/a> <http://example.com/edge> <http://example.com/b> .\n");

    let labels = label_propagation(&graph, LabelPropConfig { max_iterations: 0 });

    assert_eq!(labels, vec![0, 1]);
}
