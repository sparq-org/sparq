//! sq-lsp7k.20: deterministic, mutation-witnessed directed-topology oracles.

#![cfg(feature = "topology")]

use proptest::prelude::*;
use sparq_algos::{
    is_acyclic, num_strongly_connected_components, strongly_connected_components, topological_sort,
    CycleError, NodeGraph,
};
use sparq_core::Graph;

fn build(nt: &str) -> NodeGraph {
    NodeGraph::build(&Graph::load_str(nt, "nt").expect("parse N-Triples"))
}

#[test]
fn directed_cycle_and_isolated_node_have_two_components() {
    let graph = build(
        r#"
<http://e/0> <http://p/edge> <http://e/1> .
<http://e/1> <http://p/edge> <http://e/2> .
<http://e/2> <http://p/edge> <http://e/0> .
<http://e/3> <http://p/data> "isolated entity marker" .
"#,
    );

    assert_eq!(
        graph.len(),
        4,
        "the data-only subject remains an isolated node"
    );
    assert_eq!(strongly_connected_components(&graph), vec![0, 0, 0, 1]);
    assert_eq!(topological_sort(&graph), Err(CycleError));
}

/// sq-awq7n: value-pins the dense-label count, including its empty-input
/// sentinel. The reciprocal d/e pair is the second SCC; a one-way edge would instead
/// make d and e separate singleton SCCs, as `one_way_edge_does_not_merge_strong_components`
/// verifies below.
#[test]
fn strongly_connected_component_count_handles_two_three_and_zero() {
    let two_components = build(
        r#"
<http://e/a> <http://p/edge> <http://e/b> .
<http://e/b> <http://p/edge> <http://e/c> .
<http://e/c> <http://p/edge> <http://e/a> .
<http://e/d> <http://p/edge> <http://e/e> .
<http://e/e> <http://p/edge> <http://e/d> .
"#,
    );
    let two_labels = strongly_connected_components(&two_components);
    assert_eq!(num_strongly_connected_components(&two_labels), 2);

    let three_components = build(
        r#"
<http://e/a> <http://p/edge> <http://e/a> .
<http://e/b> <http://p/edge> <http://e/b> .
<http://e/c> <http://p/edge> <http://e/c> .
"#,
    );
    let three_labels = strongly_connected_components(&three_components);
    assert_eq!(num_strongly_connected_components(&three_labels), 3);
    assert_eq!(num_strongly_connected_components(&[]), 0);
}

#[test]
fn canonical_diamond_dag_has_singleton_components_and_exact_order() {
    let graph = build(
        r#"
<http://e/0> <http://p/edge> <http://e/1> .
<http://e/0> <http://p/edge> <http://e/2> .
<http://e/1> <http://p/edge> <http://e/3> .
<http://e/2> <http://p/edge> <http://e/3> .
"#,
    );

    assert_eq!(strongly_connected_components(&graph), vec![0, 1, 2, 3]);
    assert_eq!(topological_sort(&graph), Ok(vec![0, 1, 2, 3]));
}

#[test]
fn one_way_edge_does_not_merge_strong_components() {
    let graph = build("<http://e/0> <http://p/edge> <http://e/1> .\n");
    assert_eq!(strongly_connected_components(&graph), vec![0, 1]);
    assert_eq!(topological_sort(&graph), Ok(vec![0, 1]));
}

#[test]
fn self_loop_is_a_singleton_component_but_a_cycle() {
    let graph = build("<http://e/0> <http://p/edge> <http://e/0> .\n");
    assert_eq!(strongly_connected_components(&graph), vec![0]);
    assert_eq!(topological_sort(&graph), Err(CycleError));
}

/// sq-ulsqh: pins both boolean outcomes and the self-loop boundary so negating
/// the thin `topological_sort(...).is_ok()` wrapper fails on the DAG and directed cycle.
#[test]
fn acyclicity_predicate_distinguishes_dag_cycle_and_self_loop() {
    let dag = build(
        r#"
<http://e/a> <http://p/edge> <http://e/b> .
<http://e/b> <http://p/edge> <http://e/c> .
"#,
    );
    let cycle = build(
        r#"
<http://e/a> <http://p/edge> <http://e/b> .
<http://e/b> <http://p/edge> <http://e/c> .
<http://e/c> <http://p/edge> <http://e/a> .
"#,
    );
    let self_loop = build("<http://e/a> <http://p/edge> <http://e/a> .\n");

    assert!(is_acyclic(&dag));
    assert!(!is_acyclic(&cycle));
    assert!(!is_acyclic(&self_loop));
}

#[test]
fn empty_graph_has_empty_components_and_order() {
    let graph = build("");
    assert_eq!(strongly_connected_components(&graph), Vec::<usize>::new());
    assert_eq!(topological_sort(&graph), Ok(Vec::new()));
}

#[test]
fn cycle_error_has_a_stable_diagnostic() {
    assert_eq!(CycleError.to_string(), "directed graph contains a cycle");
}

fn generated_graph(node_count: usize, edges: &[bool]) -> NodeGraph {
    let mut triples = String::new();
    for node in 0..node_count {
        triples.push_str(&format!(
            "<http://e/{node}> <http://p/data> \"isolated entity marker\" .\n"
        ));
    }
    for source in 0..node_count {
        for target in 0..node_count {
            if edges.get(source * node_count + target) == Some(&true) {
                triples.push_str(&format!(
                    "<http://e/{source}> <http://p/edge> <http://e/{target}> .\n"
                ));
            }
        }
    }
    build(&triples)
}

fn reachability(graph: &NodeGraph) -> Vec<Vec<bool>> {
    let n = graph.len();
    let mut reachable = vec![vec![false; n]; n];
    for (node, row) in reachable.iter_mut().enumerate() {
        row[node] = true;
        for &neighbor in graph.out_neighbors(node) {
            row[neighbor as usize] = true;
        }
    }
    for intermediate in 0..n {
        for source in 0..n {
            for target in 0..n {
                reachable[source][target] |=
                    reachable[source][intermediate] && reachable[intermediate][target];
            }
        }
    }
    reachable
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(96))]

    #[test]
    fn directed_algorithms_match_reachability_oracles(
        node_count in 0usize..7,
        edges in prop::collection::vec(any::<bool>(), 0..49),
    ) {
        let graph = generated_graph(node_count, &edges);
        let reachable = reachability(&graph);
        let components = strongly_connected_components(&graph);

        prop_assert_eq!(graph.len(), node_count);
        prop_assert_eq!(components.len(), node_count);
        for left in 0..node_count {
            for right in 0..node_count {
                prop_assert_eq!(
                    components[left] == components[right],
                    reachable[left][right] && reachable[right][left],
                );
            }
        }

        let cyclic = (0..node_count).any(|node| {
            graph.out_neighbors(node).contains(&(node as u32))
                || (0..node_count).any(|other| {
                    node != other && components[node] == components[other]
                })
        });
        match topological_sort(&graph) {
            Ok(order) => {
                prop_assert!(!cyclic);
                prop_assert_eq!(order.len(), node_count);
                let mut ordered_nodes = order.clone();
                ordered_nodes.sort_unstable();
                prop_assert_eq!(ordered_nodes, (0..node_count).collect::<Vec<_>>());
                let mut position = vec![0usize; node_count];
                for (offset, &node) in order.iter().enumerate() {
                    position[node] = offset;
                }
                for source in 0..node_count {
                    for &target in graph.out_neighbors(source) {
                        prop_assert!(position[source] < position[target as usize]);
                    }
                }
            }
            Err(CycleError) => prop_assert!(cyclic),
        }
    }
}
