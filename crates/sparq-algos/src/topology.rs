//! Directed topology algorithms over a [`NodeGraph`].
//!
//! sq-lsp7k.20: unlike the crate's weak community and extended-centrality
//! passes, the algorithms in this module preserve edge direction. Strongly connected
//! components use an iterative Tarjan traversal, and topological sorting uses Kahn's
//! algorithm with an ascending-node ready queue. Neither algorithm uses recursion or RNG.

use std::cmp::Reverse;
use std::collections::BinaryHeap;
use std::fmt;

use crate::graph::NodeGraph;

const UNVISITED: usize = usize::MAX;

#[derive(Clone, Copy, Debug)]
struct DfsFrame {
    node: usize,
    next_neighbor: usize,
}

/// Error returned when a directed graph has no topological ordering.
///
/// A cycle includes a self-loop. The type carries no path because
/// [`topological_sort`] only promises cycle detection, not a cycle witness.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct CycleError;

impl fmt::Display for CycleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("directed graph contains a cycle")
    }
}

impl std::error::Error for CycleError {}

/// Computes the strongly connected component of every node using Tarjan's algorithm.
///
/// Two nodes receive the same component id exactly when each is reachable from the other
/// by following directed out-edges. Returned ids are dense in `0..k` and canonical: the
/// component containing the smallest node index is `0`, the component containing the
/// smallest node not already labelled is `1`, and so on. The traversal is iterative, so
/// a long directed path does not consume the call stack. Runs in `O(V + E)` time.
pub fn strongly_connected_components(g: &NodeGraph) -> Vec<usize> {
    let n = g.len();
    let mut indices = vec![UNVISITED; n];
    let mut lowlink = vec![0usize; n];
    let mut on_stack = vec![false; n];
    let mut tarjan_stack = Vec::with_capacity(n);
    let mut component_root = vec![UNVISITED; n];
    let mut next_index = 0usize;

    for start in 0..n {
        if indices[start] != UNVISITED {
            continue;
        }

        discover(
            start,
            &mut indices,
            &mut lowlink,
            &mut on_stack,
            &mut tarjan_stack,
            &mut next_index,
        );
        let mut dfs = vec![DfsFrame {
            node: start,
            next_neighbor: 0,
        }];

        while let Some(frame) = dfs.last_mut() {
            let node = frame.node;
            if let Some(&neighbor) = g.out_neighbors(node).get(frame.next_neighbor) {
                frame.next_neighbor += 1;
                let neighbor = neighbor as usize;
                if indices[neighbor] == UNVISITED {
                    discover(
                        neighbor,
                        &mut indices,
                        &mut lowlink,
                        &mut on_stack,
                        &mut tarjan_stack,
                        &mut next_index,
                    );
                    dfs.push(DfsFrame {
                        node: neighbor,
                        next_neighbor: 0,
                    });
                } else if on_stack[neighbor] {
                    lowlink[node] = lowlink[node].min(indices[neighbor]);
                }
                continue;
            }

            let finished = dfs.pop().expect("DFS frame is present").node;
            if lowlink[finished] == indices[finished] {
                loop {
                    let member = tarjan_stack
                        .pop()
                        .expect("Tarjan root has a corresponding stack member");
                    on_stack[member] = false;
                    component_root[member] = finished;
                    if member == finished {
                        break;
                    }
                }
            }
            if let Some(parent) = dfs.last() {
                let parent = parent.node;
                lowlink[parent] = lowlink[parent].min(lowlink[finished]);
            }
        }
    }

    densify_roots(&component_root)
}

/// sq-awq7n: Returns the number of strongly connected components in a
/// labelling produced by `strongly_connected_components` (dense ids, so this is
/// `max + 1`, or `0` for an empty labelling).
pub fn num_strongly_connected_components(labels: &[usize]) -> usize {
    labels.iter().copied().max().map_or(0, |m| m + 1)
}

fn discover(
    node: usize,
    indices: &mut [usize],
    lowlink: &mut [usize],
    on_stack: &mut [bool],
    tarjan_stack: &mut Vec<usize>,
    next_index: &mut usize,
) {
    indices[node] = *next_index;
    lowlink[node] = *next_index;
    *next_index += 1;
    on_stack[node] = true;
    tarjan_stack.push(node);
}

fn densify_roots(roots: &[usize]) -> Vec<usize> {
    let mut remap = vec![UNVISITED; roots.len()];
    let mut next_component = 0usize;
    roots
        .iter()
        .map(|&root| {
            if remap[root] == UNVISITED {
                remap[root] = next_component;
                next_component += 1;
            }
            remap[root]
        })
        .collect()
}

/// Returns the canonical topological ordering of a directed acyclic graph.
///
/// For every edge `u → v`, `u` appears before `v`. When several nodes are ready at the
/// same time, the smallest node index is chosen, making the result deterministic and
/// lexicographically smallest among all valid orders. Returns [`CycleError`] for every
/// directed cycle, including a self-loop. Runs in `O((V + E) log V)` time.
pub fn topological_sort(g: &NodeGraph) -> Result<Vec<usize>, CycleError> {
    let n = g.len();
    let mut indegree: Vec<usize> = (0..n).map(|node| g.in_degree(node)).collect();
    let mut ready = BinaryHeap::new();
    for (node, &degree) in indegree.iter().enumerate() {
        if degree == 0 {
            ready.push(Reverse(node));
        }
    }

    let mut order = Vec::with_capacity(n);
    while let Some(Reverse(node)) = ready.pop() {
        order.push(node);
        for &neighbor in g.out_neighbors(node) {
            let neighbor = neighbor as usize;
            indegree[neighbor] -= 1;
            if indegree[neighbor] == 0 {
                ready.push(Reverse(neighbor));
            }
        }
    }

    if order.len() == n {
        Ok(order)
    } else {
        Err(CycleError)
    }
}

/// Returns whether the directed graph is acyclic.
///
/// A self-loop counts as a cycle. This is a convenience wrapper around
/// [`topological_sort`] and has the same `O((V + E) log V)` running time.
/// sq-ulsqh.
pub fn is_acyclic(g: &NodeGraph) -> bool {
    topological_sort(g).is_ok()
}
