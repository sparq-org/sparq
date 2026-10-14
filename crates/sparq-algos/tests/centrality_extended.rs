//! sq-lsp7k.15: exact and mutation-witness oracles for extended centrality.

#![cfg(feature = "centrality-extended")]

use std::collections::VecDeque;

use oxrdf::{NamedNode, Term};
use proptest::prelude::*;
use sparq_algos::{betweenness_centrality, closeness_centrality, NodeGraph};
use sparq_core::Graph;

fn build(nt: &str) -> (Graph, NodeGraph) {
    let graph = Graph::load_str(nt, "nt").expect("parse N-Triples");
    let nodes = NodeGraph::build(&graph);
    (graph, nodes)
}

fn index(graph: &Graph, nodes: &NodeGraph, label: &str) -> usize {
    let term = Term::NamedNode(NamedNode::new(format!("http://e/{label}")).unwrap());
    nodes.index_of(graph.id_of(&term).unwrap()).unwrap()
}

fn assert_close(actual: f64, expected: f64) {
    assert!(
        (actual - expected).abs() < 1e-12,
        "expected {expected}, got {actual}"
    );
}

#[test]
fn three_node_path_has_exact_centrality_oracles() {
    // One RDF direction per edge deliberately witnesses the weak/undirected traversal.
    let (graph, nodes) = build(
        r#"
<http://e/a> <http://p/k> <http://e/b> .
<http://e/b> <http://p/k> <http://e/c> .
"#,
    );
    let (a, b, c) = (
        index(&graph, &nodes, "a"),
        index(&graph, &nodes, "b"),
        index(&graph, &nodes, "c"),
    );

    let betweenness = betweenness_centrality(&nodes);
    assert_close(betweenness[a], 0.0);
    assert_close(betweenness[b], 1.0);
    assert_close(betweenness[c], 0.0);

    let closeness = closeness_centrality(&nodes);
    assert_close(closeness[a], 0.75); // mean of distances^-1: (1 + 1/2) / 2
    assert_close(closeness[b], 1.0);
    assert_close(closeness[c], 0.75);
}

#[test]
fn star_center_counts_leaf_pairs_and_has_exact_harmonic_scores() {
    let (graph, nodes) = build(
        r#"
<http://e/h> <http://p/k> <http://e/a> .
<http://e/h> <http://p/k> <http://e/b> .
<http://e/h> <http://p/k> <http://e/c> .
<http://e/h> <http://p/k> <http://e/d> .
"#,
    );
    let hub = index(&graph, &nodes, "h");
    let leaves = ["a", "b", "c", "d"].map(|label| index(&graph, &nodes, label));

    let betweenness = betweenness_centrality(&nodes);
    assert_close(betweenness[hub], 6.0); // four choose two leaf pairs
    for leaf in leaves {
        assert_close(betweenness[leaf], 0.0);
    }

    let closeness = closeness_centrality(&nodes);
    assert_close(closeness[hub], 1.0);
    for leaf in leaves {
        // One hub at distance 1 and three leaves at distance 2, divided by V-1.
        assert_close(closeness[leaf], 0.625);
    }
}

#[test]
fn disconnected_graph_uses_zero_for_unreachable_nodes() {
    let (graph, nodes) = build(
        r#"
<http://e/a> <http://p/k> <http://e/b> .
<http://e/c> <http://p/k> <http://e/c> .
"#,
    );
    let (a, b, c) = (
        index(&graph, &nodes, "a"),
        index(&graph, &nodes, "b"),
        index(&graph, &nodes, "c"),
    );
    let closeness = closeness_centrality(&nodes);
    assert_close(closeness[a], 0.5);
    assert_close(closeness[b], 0.5);
    assert_close(closeness[c], 0.0);
    assert!(betweenness_centrality(&nodes).iter().all(|&x| x == 0.0));
}

#[test]
fn reciprocal_edges_and_self_loops_do_not_multiply_shortest_paths() {
    let (_, simple) = build(
        r#"
<http://e/a> <http://p/k> <http://e/b> .
<http://e/b> <http://p/k> <http://e/c> .
"#,
    );
    let (_, redundant) = build(
        r#"
<http://e/a> <http://p/k> <http://e/a> .
<http://e/a> <http://p/k> <http://e/b> .
<http://e/b> <http://p/k> <http://e/a> .
<http://e/b> <http://p/k> <http://e/c> .
<http://e/c> <http://p/k> <http://e/b> .
"#,
    );
    assert_eq!(
        betweenness_centrality(&simple),
        betweenness_centrality(&redundant)
    );
    assert_eq!(
        closeness_centrality(&simple),
        closeness_centrality(&redundant)
    );
}

#[test]
fn empty_and_singleton_graphs_are_zero() {
    let (_, empty) = build("");
    assert!(betweenness_centrality(&empty).is_empty());
    assert!(closeness_centrality(&empty).is_empty());

    let (_, singleton) = build("<http://e/a> <http://p/k> <http://e/a> .\n");
    assert_eq!(betweenness_centrality(&singleton), vec![0.0]);
    assert_eq!(closeness_centrality(&singleton), vec![0.0]);
}

fn tree_graph(parents: &[u8]) -> NodeGraph {
    let n = parents.len() + 1;
    let mut triples = String::new();
    for node in 1..n {
        let parent = usize::from(parents[node - 1]) % node;
        triples.push_str(&format!(
            "<http://e/n{parent}> <http://p/edge> <http://e/n{node}> .\n"
        ));
    }
    if n == 1 {
        triples.push_str("<http://e/n0> <http://p/self> <http://e/n0> .\n");
    }
    NodeGraph::build(&Graph::load_str(&triples, "nt").unwrap())
}

fn tree_distances(graph: &NodeGraph, source: usize) -> Vec<usize> {
    let mut distances = vec![usize::MAX; graph.len()];
    let mut queue = VecDeque::new();
    distances[source] = 0;
    queue.push_back(source);
    while let Some(node) = queue.pop_front() {
        for &neighbor in graph
            .out_neighbors(node)
            .iter()
            .chain(graph.in_neighbors(node))
        {
            let neighbor = neighbor as usize;
            if distances[neighbor] == usize::MAX {
                distances[neighbor] = distances[node] + 1;
                queue.push_back(neighbor);
            }
        }
    }
    distances
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    #[test]
    fn tree_centrality_obeys_nonnegative_and_pair_sum_invariants(
        parents in prop::collection::vec(any::<u8>(), 0..9)
    ) {
        let graph = tree_graph(&parents);
        let n = graph.len();
        let betweenness = betweenness_centrality(&graph);
        let closeness = closeness_centrality(&graph);

        prop_assert!(betweenness.iter().all(|score| score.is_finite() && *score >= 0.0));
        prop_assert!(closeness.iter().all(|score| score.is_finite() && (0.0..=1.0).contains(score)));

        let mut internal_vertices_over_pairs = 0usize;
        let mut inverse_distance_over_pairs = 0.0;
        for source in 0..n {
            let distances = tree_distances(&graph, source);
            for &distance in distances.iter().skip(source + 1) {
                internal_vertices_over_pairs += distance.saturating_sub(1);
                inverse_distance_over_pairs += 1.0 / distance as f64;
            }
        }

        prop_assert!((betweenness.iter().sum::<f64>() - internal_vertices_over_pairs as f64).abs() < 1e-10);
        if n > 1 {
            let normalized_ordered_sum = closeness.iter().sum::<f64>() * (n - 1) as f64;
            prop_assert!((normalized_ordered_sum - 2.0 * inverse_distance_over_pairs).abs() < 1e-10);
        }
    }
}
