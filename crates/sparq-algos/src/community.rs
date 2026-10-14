//! Community detection over a [`NodeGraph`].
//!
//! Two complementary passes:
//!
//! * [`weakly_connected_components`] — the exact partition into **weakly connected
//!   components** (treat every edge as undirected; two nodes share a component iff a path
//!   of edges connects them ignoring direction). Computed with a near-linear union-find.
//!   This is a *structural* community: it answers "which entities are reachable from each
//!   other at all".
//!
//! * [`label_propagation`] — the classic near-linear **label-propagation** community
//!   heuristic (Raghavan, Albert, Kumara 2007): each node adopts the label held by the
//!   plurality of its neighbours, iterated to a fixed point. This finds *denser*
//!   communities within a connected component (a connected component can contain several
//!   label-propagation communities). It is a heuristic, made deterministic here by a fixed
//!   visiting order and an ascending-id tie-break, so repeated runs on the same graph give
//!   the same partition. sq-lqty: the "fixed visiting order" is the node index
//!   order, which [`NodeGraph`] now assigns CANONICALLY (ascending term), so the partition
//!   is also identical across hosts — it no longer depends on the dictionary-id order that
//!   the parallel loader assigns thread-count-dependently.
//!
//! Both return a `Vec<usize>` labelling: `labels[i]` is the community id of node `i`.
//! Community ids are dense (`0..num_communities`) and assigned in ascending-node order so
//! the smallest member's first-seen position determines the id — again for determinism.

use crate::graph::NodeGraph;
use rustc_hash::FxHashMap;

/// Disjoint-set (union-find) with path halving + union by size. Near-linear total cost.
struct UnionFind {
    parent: Vec<u32>,
    size: Vec<u32>,
}

impl UnionFind {
    fn new(n: usize) -> UnionFind {
        UnionFind {
            parent: (0..n as u32).collect(),
            size: vec![1; n],
        }
    }

    fn find(&mut self, mut x: u32) -> u32 {
        while self.parent[x as usize] != x {
            // Path halving: point x at its grandparent as we climb.
            let gp = self.parent[self.parent[x as usize] as usize];
            self.parent[x as usize] = gp;
            x = gp;
        }
        x
    }

    fn union(&mut self, a: u32, b: u32) {
        let (mut ra, mut rb) = (self.find(a), self.find(b));
        if ra == rb {
            return;
        }
        if self.size[ra as usize] < self.size[rb as usize] {
            std::mem::swap(&mut ra, &mut rb);
        }
        self.parent[rb as usize] = ra;
        self.size[ra as usize] += self.size[rb as usize];
    }
}

/// Re-labels an array of arbitrary root ids into dense `0..k` ids, assigned in
/// ascending-node order (the first node carrying a given root gets the next free id).
fn densify(roots: &[u32]) -> Vec<usize> {
    let mut remap: FxHashMap<u32, usize> = FxHashMap::default();
    let mut out = vec![0usize; roots.len()];
    for (i, &r) in roots.iter().enumerate() {
        let next = remap.len();
        let id = *remap.entry(r).or_insert(next);
        out[i] = id;
    }
    out
}

/// The exact weakly-connected-component partition. `labels[i]` is the component id of node
/// `i`; ids are dense and assigned in ascending-node order. `O(V + E · α(V))`.
pub fn weakly_connected_components(g: &NodeGraph) -> Vec<usize> {
    let n = g.len();
    let mut uf = UnionFind::new(n);
    // Union across out-edges only — every undirected edge is covered, since an in-edge of
    // one node is the out-edge of another.
    for i in 0..n {
        for &j in g.out_neighbors(i) {
            uf.union(i as u32, j);
        }
    }
    let roots: Vec<u32> = (0..n as u32).map(|x| uf.find(x)).collect();
    densify(&roots)
}

/// Number of distinct communities in a labelling produced by this module (dense ids, so
/// this is `max + 1`, or `0` for an empty labelling).
pub fn num_communities(labels: &[usize]) -> usize {
    labels.iter().copied().max().map_or(0, |m| m + 1)
}

/// Tuning for [`label_propagation`].
#[derive(Clone, Copy, Debug)]
pub struct LabelPropConfig {
    /// Maximum sweeps over all nodes before stopping even if not yet stable. `50` is
    /// ample — the heuristic typically stabilises within a handful of sweeps.
    pub max_iterations: usize,
}

impl Default for LabelPropConfig {
    fn default() -> LabelPropConfig {
        LabelPropConfig { max_iterations: 50 }
    }
}

/// Deterministic label-propagation communities. `labels[i]` is the community id of node
/// `i`; ids are dense and assigned in ascending-node order.
///
/// Each sweep, in ascending node order, every node adopts the label held by the most of
/// its (undirected) neighbours; ties break toward the **smallest label** so the result is
/// reproducible (the original paper randomises — we do not, by design). Isolated nodes
/// keep their own label and so form singleton communities. `O((V + E) · iterations)`.
pub fn label_propagation(g: &NodeGraph, cfg: LabelPropConfig) -> Vec<usize> {
    let n = g.len();
    if n == 0 {
        return Vec::new();
    }
    // Start with every node in its own label.
    let mut label: Vec<u32> = (0..n as u32).collect();

    for _ in 0..cfg.max_iterations {
        let mut changed = false;
        for i in 0..n {
            // Tally neighbour labels (both directions — undirected community view).
            let mut counts: FxHashMap<u32, u32> = FxHashMap::default();
            for &j in g.out_neighbors(i) {
                *counts.entry(label[j as usize]).or_insert(0) += 1;
            }
            for &j in g.in_neighbors(i) {
                *counts.entry(label[j as usize]).or_insert(0) += 1;
            }
            if counts.is_empty() {
                continue; // isolated node keeps its own label
            }
            // Pick the most frequent label; ties → smallest label id.
            let best = counts
                .iter()
                .max_by(|a, b| a.1.cmp(b.1).then(b.0.cmp(a.0)))
                .map(|(&l, _)| l)
                .unwrap();
            if best != label[i] {
                label[i] = best;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    densify(&label)
}
