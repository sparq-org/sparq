# sparq-algos

**Graph analytics** for the sparq RDF engine — an **opt-in** crate (vendor-parity,
epic sq-3183) that runs classic graph algorithms directly over a `sparq_core::Graph`.
PageRank, degree/in/out centrality, weakly-connected-component and label-propagation
community detection — plus feature-gated exact betweenness, harmonic closeness, strongly
connected components, boolean acyclicity checks, and topological sorting — computed from
sparq-core's permutation indexes with no model and no network. Nothing in the workspace
depends on it; the default engine build does not even compile it.

## 🚀 Quickstart

```rust,ignore
use sparq_algos::{
    NodeGraph, NodeFilter,
    pagerank, PageRankConfig,
    degree_centrality, Direction, top_k,
    betweenness_centrality, closeness_centrality,
    weakly_connected_components, label_propagation, LabelPropConfig, num_communities,
    is_acyclic, num_strongly_connected_components, strongly_connected_components,
    topological_sort,
};

// Project the RDF graph onto a directed node graph (subjects + entity objects = nodes,
// each triple (s,p,o) = an edge s → o; predicates erased, parallel edges collapsed).
let g = NodeGraph::build(&graph);                 // default: entities only (no literals)
let g = NodeGraph::build_with(&graph, NodeFilter::All); // include literal objects too

// PageRank — stationary distribution, sums to ~1.0, indexed by node index.
let ranks = pagerank(&g, PageRankConfig::default());     // d = 0.85
let term  = g.term(&graph, best_index);                  // node index → oxrdf::Term

// Degree centrality (In / Out / Total) and the top-k entities.
let deg = degree_centrality(&g, Direction::In);          // Vec<usize>, per node
let top = top_k(&deg, 10);                               // Vec<(node_index, score)>

// Exact shortest-path centralities; requires feature `centrality-extended`.
let between = betweenness_centrality(&g);                 // unnormalised Brandes scores
let close   = closeness_centrality(&g);                   // harmonic mean in [0, 1]

// Community detection.
let comp = weakly_connected_components(&g);              // exact, union-find
let comm = label_propagation(&g, LabelPropConfig::default()); // heuristic, deterministic
let k    = num_communities(&comm);

// Directed topology; requires feature `topology`.
let scc = strongly_connected_components(&g);              // dense component id per node
let scc_count = num_strongly_connected_components(&scc);  // number of components
let dag = is_acyclic(&g);                                  // false for any directed cycle
let order = topological_sort(&g)?;                         // Err(CycleError) on a cycle
```

## ✨ Features

- **`NodeGraph`** — a directed, predicate-erased CSR view of the graph keyed by dense node
  indices, built from a single pass over `Graph::iter_ids`; forward + reverse adjacency,
  parallel edges collapsed, self-loops kept. Maps each node back to its dictionary `Id` /
  `Term`. Node indices are assigned in **canonical ascending-term order**, so the view (and
  every algorithm over it) is reproducible across hosts regardless of the dictionary-id order
  the parallel loader picks — see `NodeGraph::build_with`.
- **PageRank** — the random-surfer power method with correct dangling-node mass
  redistribution; deterministic (no RNG), converges in L1 to a configurable tolerance.
- **Degree centrality** — In / Out / Total, raw counts or normalised to `[0, 1]`, plus a
  deterministic `top_k`.
- **Extended centrality** — with the default-OFF `centrality-extended` feature, exact
  unweighted Brandes betweenness, normalised harmonic closeness, and `core_number`
  (k-core decomposition, O(V + E)). All three treat each directed
  edge as an undirected connection; reciprocal edges and self-loops do not multiply paths.
  Harmonic closeness assigns unreachable nodes a zero contribution, so disconnected graphs
  have finite scores.
- **Community detection** — exact weakly-connected components (near-linear union-find) and
  a deterministic label-propagation heuristic; dense, ascending-order community ids.
- **Directed topology** — with the default-OFF `topology` feature, iterative Tarjan
  strongly connected components, their count accessor, and a canonical topological sort.
  SCC ids are densified by ascending node index; topological-sort ties choose the smallest
  ready node and cycles, including self-loops, return `CycleError`; `is_acyclic` exposes the
  same check as a boolean. sq-awq7n.
- **Opt-in & lean** — consumes only sparq-core's public read API; the only dependencies
  are `sparq-core`, `oxrdf`, and `rustc-hash`. The heavier all-pairs algorithms are behind
  `centrality-extended`, and directed topology is behind `topology`; both features are OFF
  by default. No engine, wasm, or network code enters the build.

## 📚 Learn more

- The capability skill: [`skills/graph-analytics/SKILL.md`](../../skills/graph-analytics/SKILL.md).
- Source: `src/graph.rs` (the view), `src/pagerank.rs`, `src/centrality.rs`,
  `src/centrality_extended.rs`, `src/community.rs`, and `src/topology.rs`. Tests live in
  `src/lib.rs` and `tests/`.

These are **topology** algorithms: edges are unweighted and predicate-erased. To analyse a
sub-graph (e.g. only `foaf:knows` edges), filter the source graph first; predicate-weighted
and predicate-projected views are tracked as follow-up beads. Extended centrality is exact,
not sampled: both algorithms perform an all-pairs traversal and are intended for graphs where
that cost is acceptable. SCC and topological sorting preserve edge direction; the community
and extended-centrality algorithms intentionally use weak topology.

### Mutation-testing note (`bench/mutants-baseline.json`)

The crate is on the cargo-mutants quality ratchet. The few mutants that still survive the
suite are **equivalent mutants** — they cannot change any observable output, so no
black-box test can kill them, and they are *expected* to sit under the committed ceiling
rather than being chased:

- **`UnionFind` union-by-size internals** (`community.rs`, the `size[ra] < size[rb]` swap
  and the `size[ra] += size[rb]` bookkeeping). Union by size is a *performance* heuristic:
  it only decides which root becomes the parent, never the partition. `weakly_connected_components`
  re-densifies the labelling by first-seen node order, so the chosen root is invisible in
  the result — mutating the size comparison or the size accumulator leaves every partition
  identical.
- **PageRank's `delta < tolerance` → `<=`** (`pagerank.rs`). `delta` is a sum of
  floating-point absolute differences; it never lands *exactly* on the tolerance, so `<`
  and `<=` stop on the same iteration and return bit-identical ranks.

Every *behaviour-changing* mutant on these files is covered by an assertion in the
`#[test]` module of `src/lib.rs` (search `sq-lqty`).

## License

Licensed under the MIT license, same as the rest of the sparq workspace.
