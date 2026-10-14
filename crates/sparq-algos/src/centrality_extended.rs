//! Exact centrality and cohesion measures over a [`NodeGraph`].
//!
//! sq-lsp7k.15: these algorithms use the weak (undirected) topology: an RDF
//! edge `a → b` connects `a` and `b` in either traversal direction. This matches the
//! crate's community-analysis topology and makes the result independent of which RDF
//! direction was chosen to encode a relationship. Parallel edges were already collapsed
//! by [`NodeGraph`], and reciprocal edges are collapsed again here, so neither changes the
//! topology. Self-loops do not contribute.

use std::collections::VecDeque;

use crate::graph::NodeGraph;

/// Exact, unnormalised betweenness centrality of every node.
///
/// A node receives its share of each shortest path between an **unordered** pair of
/// distinct endpoints. For example, the middle of a three-node path scores `1.0` and
/// both endpoints score `0.0`. The implementation is unweighted Brandes over the weak
/// (undirected) topology and runs in `O(V * (V + E))` time.
///
/// The returned vector is indexed by the graph's dense node index. Empty and
/// single-node graphs return an all-zero vector of the corresponding length.
pub fn betweenness_centrality(g: &NodeGraph) -> Vec<f64> {
    let adjacency = undirected_adjacency(g);
    let n = adjacency.len();
    let mut centrality = vec![0.0; n];

    for source in 0..n {
        let mut stack = Vec::with_capacity(n);
        let mut predecessors = vec![Vec::<usize>::new(); n];
        let mut path_count = vec![0.0; n];
        let mut distance = vec![usize::MAX; n];
        let mut queue = VecDeque::with_capacity(n);

        path_count[source] = 1.0;
        distance[source] = 0;
        queue.push_back(source);

        while let Some(node) = queue.pop_front() {
            stack.push(node);
            let next_distance = distance[node] + 1;
            for &neighbor in &adjacency[node] {
                if distance[neighbor] == usize::MAX {
                    distance[neighbor] = next_distance;
                    queue.push_back(neighbor);
                }
                if distance[neighbor] == next_distance {
                    path_count[neighbor] += path_count[node];
                    predecessors[neighbor].push(node);
                }
            }
        }

        let mut dependency = vec![0.0; n];
        while let Some(node) = stack.pop() {
            for &predecessor in &predecessors[node] {
                dependency[predecessor] +=
                    (path_count[predecessor] / path_count[node]) * (1.0 + dependency[node]);
            }
            if node != source {
                centrality[node] += dependency[node];
            }
        }
    }

    // Every unordered endpoint pair is visited from both endpoints.
    for score in &mut centrality {
        *score *= 0.5;
    }
    centrality
}

/// Normalised harmonic closeness centrality of every node.
///
/// The score for node `u` is the mean inverse shortest-path distance
/// `sum(1 / distance(u, v)) / (V - 1)`. Unreachable nodes contribute zero, making the
/// definition finite and useful for disconnected graphs. Distances use the weak
/// (undirected), unweighted topology. Scores are in `[0, 1]`; graphs with fewer than two
/// nodes return zeros.
///
/// The returned vector is indexed by the graph's dense node index. The exact all-pairs
/// breadth-first searches run in `O(V * (V + E))` time.
pub fn closeness_centrality(g: &NodeGraph) -> Vec<f64> {
    let adjacency = undirected_adjacency(g);
    let n = adjacency.len();
    if n < 2 {
        return vec![0.0; n];
    }

    let denominator = (n - 1) as f64;
    (0..n)
        .map(|source| {
            let distances = breadth_first_distances(&adjacency, source);
            distances
                .into_iter()
                .filter(|&distance| distance != 0 && distance != usize::MAX)
                .map(|distance| 1.0 / distance as f64)
                .sum::<f64>()
                / denominator
        })
        .collect()
}

/// The k-core number of every node.
///
/// A node's core number is the largest `k` for which it belongs to a subgraph where every
/// node has undirected degree at least `k`. The implementation uses the
/// Batagelj–Zaversnik bucket-peeling algorithm over the weak (undirected) topology and runs
/// in `O(V + E)` time.
///
/// The returned vector is indexed by the graph's dense node index. Empty and single-node
/// graphs return an all-zero vector of the corresponding length. Reciprocal edges are one
/// connection, and self-loops do not increase a node's degree.
pub fn core_number(g: &NodeGraph) -> Vec<usize> {
    let adjacency = undirected_adjacency(g);
    let mut degree: Vec<_> = adjacency.iter().map(Vec::len).collect();
    let max_degree = degree.iter().copied().max().unwrap_or(0);

    let mut bin = vec![0; max_degree + 1];
    for &value in &degree {
        bin[value] += 1;
    }

    let mut start = 0;
    for count in &mut bin {
        let next = start + *count;
        *count = start;
        start = next;
    }

    let mut next = bin.clone();
    let mut position = vec![0; degree.len()];
    let mut vertices = vec![0; degree.len()];
    for node in 0..degree.len() {
        let slot = next[degree[node]];
        position[node] = slot;
        vertices[slot] = node;
        next[degree[node]] += 1;
    }

    let mut core = vec![0; degree.len()];
    for order in 0..vertices.len() {
        let node = vertices[order];
        core[node] = degree[node];

        for &neighbor in &adjacency[node] {
            if degree[neighbor] <= degree[node] {
                continue;
            }

            let neighbor_degree = degree[neighbor];
            let neighbor_position = position[neighbor];
            let bucket_position = bin[neighbor_degree];
            let bucket_node = vertices[bucket_position];

            if neighbor != bucket_node {
                vertices.swap(neighbor_position, bucket_position);
                position[neighbor] = bucket_position;
                position[bucket_node] = neighbor_position;
            }
            bin[neighbor_degree] += 1;
            degree[neighbor] -= 1;
        }
    }
    core
}

fn breadth_first_distances(adjacency: &[Vec<usize>], source: usize) -> Vec<usize> {
    let mut distances = vec![usize::MAX; adjacency.len()];
    let mut queue = VecDeque::with_capacity(adjacency.len());
    distances[source] = 0;
    queue.push_back(source);

    while let Some(node) = queue.pop_front() {
        let next_distance = distances[node] + 1;
        for &neighbor in &adjacency[node] {
            if distances[neighbor] == usize::MAX {
                distances[neighbor] = next_distance;
                queue.push_back(neighbor);
            }
        }
    }
    distances
}

/// Merges the sorted outgoing and incoming CSR rows, removing reciprocal duplicates.
fn undirected_adjacency(g: &NodeGraph) -> Vec<Vec<usize>> {
    (0..g.len())
        .map(|node| {
            let outgoing = g.out_neighbors(node);
            let incoming = g.in_neighbors(node);
            let mut neighbors = Vec::with_capacity(outgoing.len() + incoming.len());
            let (mut out, mut inc) = (0, 0);

            while out < outgoing.len() || inc < incoming.len() {
                let next = match (outgoing.get(out), incoming.get(inc)) {
                    (Some(&a), Some(&b)) if a < b => {
                        out += 1;
                        a
                    }
                    (Some(&a), Some(&b)) if b < a => {
                        inc += 1;
                        b
                    }
                    (Some(&a), Some(_)) => {
                        out += 1;
                        inc += 1;
                        a
                    }
                    (Some(&a), None) => {
                        out += 1;
                        a
                    }
                    (None, Some(&b)) => {
                        inc += 1;
                        b
                    }
                    (None, None) => break,
                };
                if next as usize != node {
                    neighbors.push(next as usize);
                }
            }
            neighbors
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::core_number;
    use crate::NodeGraph;
    use sparq_core::Graph;

    fn build(nt: &str) -> NodeGraph {
        NodeGraph::build(&Graph::load_str(nt, "nt").expect("parse N-Triples"))
    }

    const K4: &str = r#"
<http://e/a> <http://p/k> <http://e/b> .
<http://e/a> <http://p/k> <http://e/c> .
<http://e/a> <http://p/k> <http://e/d> .
<http://e/b> <http://p/k> <http://e/c> .
<http://e/b> <http://p/k> <http://e/d> .
<http://e/c> <http://p/k> <http://e/d> .
"#;

    #[test]
    fn empty_and_singleton_graphs_have_zero_core_numbers() {
        assert_eq!(core_number(&build("")), Vec::<usize>::new());
        let singleton = build("<http://e/a> <http://p/name> \"Alice\" .\n");
        assert_eq!(core_number(&singleton), vec![0]);
    }

    #[test]
    fn path_nodes_all_have_core_number_one() {
        let path = build(
            r#"
<http://e/a> <http://p/k> <http://e/b> .
<http://e/b> <http://p/k> <http://e/c> .
"#,
        );
        assert_eq!(core_number(&path), vec![1, 1, 1]);
    }

    #[test]
    fn four_clique_nodes_all_have_core_number_three() {
        let cores = core_number(&build(K4));
        assert_eq!(cores, vec![3, 3, 3, 3]);
        assert_eq!(cores.iter().copied().max(), Some(3));
    }

    #[test]
    fn pendant_peels_without_reducing_four_clique_core() {
        let mut fixture = K4.to_owned();
        fixture.push_str("<http://e/a> <http://p/k> <http://e/e> .\n");
        let cores = core_number(&build(&fixture));

        assert_eq!(cores, vec![3, 3, 3, 3, 1]);
        assert!(cores[4] < cores[0]);
        assert_eq!(cores.iter().copied().max(), Some(3));
    }
}
