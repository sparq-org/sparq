//! sq-bfjr0: metamorphic graph-algorithm invariants over generated RDF graphs.

use std::collections::BTreeSet;

use oxrdf::{NamedNode, Term};
use proptest::prelude::*;
use sparq_algos::{
    degree_centrality, label_propagation, num_communities, pagerank, weakly_connected_components,
    Direction, LabelPropConfig, NodeGraph, PageRankConfig,
};
use sparq_core::Graph;

fn graph(n: usize, edges: &[(usize, usize)], permutation: &[usize]) -> (Graph, NodeGraph) {
    let mut triples = String::new();
    for &node in permutation.iter().take(n) {
        triples.push_str(&format!(
            "<http://e/n{node}> <http://p/self> <http://e/n{node}> .\n"
        ));
    }
    for &(source, target) in edges {
        let source = permutation[source];
        let target = permutation[target];
        triples.push_str(&format!(
            "<http://e/n{source}> <http://p/edge> <http://e/n{target}> .\n"
        ));
    }
    let rdf = Graph::load_str(&triples, "nt").expect("generated N-Triples must parse");
    let nodes = NodeGraph::build(&rdf);
    (rdf, nodes)
}

fn index(rdf: &Graph, nodes: &NodeGraph, label: usize) -> usize {
    let term = Term::NamedNode(NamedNode::new(format!("http://e/n{label}")).unwrap());
    nodes.index_of(rdf.id_of(&term).unwrap()).unwrap()
}

fn cases() -> impl Strategy<Value = (usize, Vec<(usize, usize)>)> {
    (1usize..9).prop_flat_map(|n| {
        prop::collection::vec((0..n, 0..n), 0..=(n * n)).prop_map(move |edges| (n, edges))
    })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(96))]

    #[test]
    fn pagerank_conserves_mass_and_is_permutation_equivariant((n, edges) in cases()) {
        let identity: Vec<_> = (0..n).collect();
        let reverse: Vec<_> = (0..n).rev().collect();
        let (rdf, nodes) = graph(n, &edges, &identity);
        let (permuted_rdf, permuted_nodes) = graph(n, &edges, &reverse);
        let cfg = PageRankConfig::default();
        let ranks = pagerank(&nodes, cfg);
        let permuted_ranks = pagerank(&permuted_nodes, cfg);

        prop_assert!((ranks.iter().sum::<f64>() - 1.0).abs() < 1e-8);
        for original in 0..n {
            let original_rank = ranks[index(&rdf, &nodes, original)];
            let renamed_rank = permuted_ranks[index(&permuted_rdf, &permuted_nodes, reverse[original])];
            prop_assert!((original_rank - renamed_rank).abs() < 1e-8);
        }
    }

    #[test]
    fn degree_is_exact_and_reversal_swaps_in_and_out((n, edges) in cases()) {
        let identity: Vec<_> = (0..n).collect();
        let reversed_edges: Vec<_> = edges.iter().map(|&(a, b)| (b, a)).collect();
        let (rdf, nodes) = graph(n, &edges, &identity);
        let (_, reversed) = graph(n, &reversed_edges, &identity);
        let unique: BTreeSet<_> = edges.iter().copied().collect();
        let incoming = degree_centrality(&nodes, Direction::In);
        let outgoing = degree_centrality(&nodes, Direction::Out);
        let total = degree_centrality(&nodes, Direction::Total);
        let reversed_incoming = degree_centrality(&reversed, Direction::In);
        let reversed_outgoing = degree_centrality(&reversed, Direction::Out);

        for node in 0..n {
            let i = index(&rdf, &nodes, node);
            // Every generated node also has one self-loop, including when the random edge set does not.
            let expected_in = unique.iter().filter(|(_, b)| *b == node).count() + usize::from(!unique.contains(&(node, node)));
            let expected_out = unique.iter().filter(|(a, _)| *a == node).count() + usize::from(!unique.contains(&(node, node)));
            prop_assert_eq!(incoming[i], expected_in);
            prop_assert_eq!(outgoing[i], expected_out);
            prop_assert_eq!(total[i], expected_in + expected_out);
            prop_assert_eq!(incoming[i], reversed_outgoing[i]);
            prop_assert_eq!(outgoing[i], reversed_incoming[i]);
        }
    }

    #[test]
    fn community_partitions_are_total_and_edges_only_merge((n, edges) in cases(), extra in 0usize..64) {
        let identity: Vec<_> = (0..n).collect();
        let (_, nodes) = graph(n, &edges, &identity);
        let components = weakly_connected_components(&nodes);
        let before = num_communities(&components);
        let mut sizes = vec![0usize; before];
        for &component in &components { sizes[component] += 1; }
        prop_assert_eq!(sizes.iter().sum::<usize>(), n);

        let mut augmented = edges.clone();
        augmented.push((extra % n, (extra / n) % n));
        let (_, augmented_nodes) = graph(n, &augmented, &identity);
        prop_assert!(num_communities(&weakly_connected_components(&augmented_nodes)) <= before);

        let labels = label_propagation(&nodes, LabelPropConfig::default());
        prop_assert_eq!(labels.len(), n);
        let communities = num_communities(&labels);
        prop_assert!(labels.iter().all(|&label| label < communities));
    }
}

#[test]
fn isolated_vertex_is_a_singleton_component() {
    let identity = [0, 1, 2];
    let (_, nodes) = graph(3, &[(0, 1)], &identity);
    let components = weakly_connected_components(&nodes);
    assert_ne!(components[2], components[0]);
    assert_ne!(components[2], components[1]);
    assert_eq!(
        components.iter().filter(|&&c| c == components[2]).count(),
        1
    );
}
