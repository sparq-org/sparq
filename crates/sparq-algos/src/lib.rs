#![doc = include_str!("../README.md")]
#![forbid(unsafe_code)] // [OPUS-4.8] sq-mqvm: crate has zero `unsafe`

pub mod centrality;
#[cfg(feature = "centrality-extended")]
pub mod centrality_extended;
pub mod community;
pub mod graph;
pub mod pagerank;
#[cfg(feature = "ranking")]
pub mod ranking;
#[cfg(feature = "topology")]
pub mod topology;

pub use centrality::{degree_centrality, degree_centrality_normalized, top_k, Direction};
#[cfg(feature = "centrality-extended")]
pub use centrality_extended::{betweenness_centrality, closeness_centrality, core_number};
pub use community::{
    label_propagation, num_communities, weakly_connected_components, LabelPropConfig,
};
pub use graph::{NodeFilter, NodeGraph};
pub use pagerank::{pagerank, PageRankConfig};
#[cfg(feature = "ranking")]
pub use ranking::{pagerank_scores_by_dict_id, scores_by_dict_id, ScoreBudget};
#[cfg(feature = "topology")]
pub use topology::{
    is_acyclic, num_strongly_connected_components, strongly_connected_components, topological_sort,
    CycleError,
};

#[cfg(test)]
mod tests {
    use super::*;
    use oxrdf::{NamedNode, Term};
    use sparq_core::Graph;

    /// Builds a graph from N-Triples and the matching node-graph view (entities only).
    fn build(nt: &str) -> (Graph, NodeGraph) {
        let g = Graph::load_str(nt, "nt").expect("parse");
        let ng = NodeGraph::build(&g);
        (g, ng)
    }

    fn iri(s: &str) -> Term {
        Term::NamedNode(NamedNode::new(s).unwrap())
    }

    /// A small cycle a → b → c → a plus an extra edge a → c.
    const CYCLE: &str = r#"
<http://e/a> <http://p/knows> <http://e/b> .
<http://e/b> <http://p/knows> <http://e/c> .
<http://e/c> <http://p/knows> <http://e/a> .
<http://e/a> <http://p/knows> <http://e/c> .
"#;

    #[test]
    fn node_graph_basics() {
        let (_g, ng) = build(CYCLE);
        assert_eq!(ng.len(), 3);
        assert_eq!(ng.edge_count(), 4);
        assert!(!ng.is_empty());
    }

    /// [OPUS-4.8] sq-lqty mutation-kill: an EMPTY graph must report `is_empty() == true` and
    /// `len() == 0`. The existing tests only ever assert `!is_empty()` on populated graphs,
    /// so the `is_empty -> false` constant-replacement mutant (graph.rs:125) survived. This
    /// pins the empty side.
    #[test]
    fn empty_node_graph_is_empty() {
        let g = Graph::load_str("", "nt").unwrap();
        let ng = NodeGraph::build(&g);
        assert!(ng.is_empty());
        assert_eq!(ng.len(), 0);
        assert_eq!(ng.edge_count(), 0);
    }

    #[test]
    fn literals_excluded_by_default_but_included_with_all() {
        let nt = r#"
<http://e/a> <http://p/name> "Alice" .
<http://e/a> <http://p/knows> <http://e/b> .
"#;
        let g = Graph::load_str(nt, "nt").unwrap();
        // EntitiesOnly: only a and b are nodes; the "Alice" literal is dropped.
        let entities = NodeGraph::build(&g);
        assert_eq!(entities.len(), 2);
        assert_eq!(entities.edge_count(), 1);
        // All: the literal becomes a third node + a second edge.
        let all = NodeGraph::build_with(&g, NodeFilter::All);
        assert_eq!(all.len(), 3);
        assert_eq!(all.edge_count(), 2);
    }

    /// [GPT-5.6] sq-lsp7k.20: a data-only subject is still an entity node in the default
    /// projection. This makes a genuinely isolated node representable without a self-loop.
    #[test]
    fn data_only_subject_is_an_isolated_entity_node() {
        let g = Graph::load_str("<http://e/a> <http://p/name> \"Alice\" .\n", "nt").unwrap();
        let ng = NodeGraph::build(&g);
        assert_eq!(ng.len(), 1);
        assert_eq!(ng.edge_count(), 0);
        assert_eq!(ng.out_degree(0), 0);
        assert_eq!(ng.in_degree(0), 0);
    }

    #[test]
    fn parallel_edges_collapse() {
        // Two predicates assert a → b; the view sees one edge.
        let nt = r#"
<http://e/a> <http://p/p1> <http://e/b> .
<http://e/a> <http://p/p2> <http://e/b> .
"#;
        let (_g, ng) = build(nt);
        assert_eq!(ng.edge_count(), 1);
    }

    #[test]
    fn in_out_neighbors_are_symmetric() {
        let (_g, ng) = build(CYCLE);
        // For every out-edge i→j there must be a matching in-edge j←i.
        for i in 0..ng.len() {
            for &j in ng.out_neighbors(i) {
                assert!(
                    ng.in_neighbors(j as usize).contains(&(i as u32)),
                    "missing reverse edge"
                );
            }
        }
    }

    #[test]
    fn pagerank_sums_to_one_and_symmetric_cycle_is_uniform() {
        // A pure 3-cycle: every node has in=out=1, so ranks must be uniform = 1/3.
        let nt = r#"
<http://e/a> <http://p/k> <http://e/b> .
<http://e/b> <http://p/k> <http://e/c> .
<http://e/c> <http://p/k> <http://e/a> .
"#;
        let (_g, ng) = build(nt);
        let r = pagerank(&ng, PageRankConfig::default());
        let sum: f64 = r.iter().sum();
        assert!((sum - 1.0).abs() < 1e-6, "sum {sum}");
        for v in &r {
            assert!((v - 1.0 / 3.0).abs() < 1e-6, "rank {v} not uniform");
        }
    }

    #[test]
    fn pagerank_authority_beats_periphery() {
        // A "star into a hub": x, y, z all point at h. h is the authority.
        let nt = r#"
<http://e/x> <http://p/k> <http://e/h> .
<http://e/y> <http://p/k> <http://e/h> .
<http://e/z> <http://p/k> <http://e/h> .
"#;
        let g = Graph::load_str(nt, "nt").unwrap();
        let ng = NodeGraph::build(&g);
        let r = pagerank(&ng, PageRankConfig::default());
        let h = ng.index_of(g.id_of(&iri("http://e/h")).unwrap()).unwrap();
        let x = ng.index_of(g.id_of(&iri("http://e/x")).unwrap()).unwrap();
        assert!(r[h] > r[x], "hub {} should outrank leaf {}", r[h], r[x]);
        let sum: f64 = r.iter().sum();
        assert!((sum - 1.0).abs() < 1e-6);
    }

    #[test]
    fn pagerank_empty_graph() {
        let g = Graph::load_str("", "nt").unwrap();
        let ng = NodeGraph::build(&g);
        assert!(pagerank(&ng, PageRankConfig::default()).is_empty());
    }

    /// Resolves the node index of an IRI in `(graph, node_graph)` built from N-Triples.
    fn idx(g: &Graph, ng: &NodeGraph, name: &str) -> usize {
        ng.index_of(g.id_of(&iri(name)).unwrap()).unwrap()
    }

    /// [OPUS-4.8] sq-lqty mutation-kill: pins the EXACT converged PageRank on a graph with a
    /// node of out-degree 2 (`a → b`, `a → c`, `b → a`). Loose ordering/sum checks let the
    /// per-in-edge contribution `rank[src] / outdeg` be corrupted without notice; the exact
    /// stationary values do not. This kills the `/`→`*` and `/`→`%` mutants on pagerank.rs:76
    /// (both leave `outdeg == 1` contributions unchanged, so only a multi-out-degree node
    /// exposes them: `0.39/2 = 0.197`, but `0.39*2` and `0.39 % 2 = 0.39` are wildly off). It
    /// also kills the convergence mutants that break after a single power iteration
    /// (84 `+=`→`-=`/`*=`, 87 `<`→`>`): the one-iteration vector
    /// `[0.4278, 0.2861, 0.2861]` differs from the converged `[0.3936, 0.3032, 0.3032]`.
    #[test]
    fn pagerank_exact_values_with_multi_out_degree_node() {
        let nt = r#"
<http://e/a> <http://p/k> <http://e/b> .
<http://e/a> <http://p/k> <http://e/c> .
<http://e/b> <http://p/k> <http://e/a> .
"#;
        let g = Graph::load_str(nt, "nt").unwrap();
        let ng = NodeGraph::build(&g);
        let (a, b, c) = (
            idx(&g, &ng, "http://e/a"),
            idx(&g, &ng, "http://e/b"),
            idx(&g, &ng, "http://e/c"),
        );
        let r = pagerank(&ng, PageRankConfig::default());
        // Hand-computed stationary distribution (d = 0.85), verified by an independent
        // reference implementation. `a` has out-degree 2, so its mass is split in half.
        assert!((r[a] - 0.393_617_021).abs() < 1e-7, "a = {}", r[a]);
        assert!((r[b] - 0.303_191_489).abs() < 1e-7, "b = {}", r[b]);
        assert!((r[c] - 0.303_191_489).abs() < 1e-7, "c = {}", r[c]);
        let sum: f64 = r.iter().sum();
        assert!((sum - 1.0).abs() < 1e-9, "sum {sum}");
    }

    /// [OPUS-4.8] sq-lqty mutation-kill: makes the convergence *break point* observable so
    /// the L1-delta accumulation (pagerank.rs:84 `delta += (next - r).abs()`) and the
    /// `delta < tolerance` early-stop (pagerank.rs:87) cannot be silently broken into a
    /// never-break loop. A loose tolerance breaks the power method *before* it has fully
    /// converged, so the loose-tolerance result must DIFFER from the fully-converged result
    /// (tolerance 0, which can only ever stop at `max_iterations`). Mutants that turn the
    /// stop condition into one that never fires — 84 `-`→`+` and `-`→`/` (delta no longer
    /// shrinks toward 0) and 87 `<`→`==` (equality essentially never holds) — make the loose
    /// run also run to `max_iterations`, collapsing the two results to equal and failing this
    /// assertion.
    #[test]
    fn pagerank_loose_tolerance_breaks_before_convergence() {
        // Star into a hub: x, y, z → h. Converges smoothly over many iterations, so a loose
        // tolerance demonstrably stops early.
        let nt = r#"
<http://e/x> <http://p/k> <http://e/h> .
<http://e/y> <http://p/k> <http://e/h> .
<http://e/z> <http://p/k> <http://e/h> .
"#;
        let g = Graph::load_str(nt, "nt").unwrap();
        let ng = NodeGraph::build(&g);
        let converged = pagerank(
            &ng,
            PageRankConfig {
                damping: 0.85,
                tolerance: 0.0, // never satisfied -> always runs the full max_iterations
                max_iterations: 200,
            },
        );
        let loose = pagerank(
            &ng,
            PageRankConfig {
                damping: 0.85,
                tolerance: 0.05, // satisfied well before the fixpoint -> stops early
                max_iterations: 200,
            },
        );
        // The loose run MUST stop before the fixpoint, so at least one rank differs.
        let differ = converged
            .iter()
            .zip(&loose)
            .any(|(c, l)| (c - l).abs() > 1e-9);
        assert!(
            differ,
            "loose-tolerance run did not stop early: {loose:?} == {converged:?}"
        );
        // Sanity: both are still probability distributions.
        for v in [&converged, &loose] {
            let s: f64 = v.iter().sum();
            assert!((s - 1.0).abs() < 1e-9, "sum {s}");
        }
    }

    #[test]
    fn degree_centrality_directions() {
        // a → b, a → c, b → c. Degrees: a out2/in0, b out1/in1, c out0/in2.
        let nt = r#"
<http://e/a> <http://p/k> <http://e/b> .
<http://e/a> <http://p/k> <http://e/c> .
<http://e/b> <http://p/k> <http://e/c> .
"#;
        let g = Graph::load_str(nt, "nt").unwrap();
        let ng = NodeGraph::build(&g);
        let a = ng.index_of(g.id_of(&iri("http://e/a")).unwrap()).unwrap();
        let c = ng.index_of(g.id_of(&iri("http://e/c")).unwrap()).unwrap();
        let out = degree_centrality(&ng, Direction::Out);
        let inc = degree_centrality(&ng, Direction::In);
        let tot = degree_centrality(&ng, Direction::Total);
        assert_eq!(out[a], 2);
        assert_eq!(inc[a], 0);
        assert_eq!(out[c], 0);
        assert_eq!(inc[c], 2);
        assert_eq!(tot[a], 2);
        assert_eq!(tot[c], 2);
        // Normalised by n-1 = 2.
        let nrm = degree_centrality_normalized(&ng, Direction::In);
        assert!((nrm[c] - 1.0).abs() < 1e-12);
    }

    /// [OPUS-4.8] sq-lqty mutation-kill: exercises the `n < 2` small-graph guard in
    /// [`degree_centrality_normalized`] (centrality.rs:42) from BOTH sides, which the
    /// existing `n == 3` test never does. A single-node graph (`n == 1`) must return
    /// `[0.0]`: the guard short-circuits before the `d / (n - 1)` divide. The `<`→`==`
    /// mutant turns the guard into `n == 2`, which is false for `n == 1`, so it falls through
    /// to `0 / (1 - 1) = 0/0 = NaN` — caught here because `NaN != 0.0`.
    #[test]
    fn degree_centrality_normalized_single_node_is_zero_not_nan() {
        // A self-loop yields exactly one node.
        let nt = "<http://e/a> <http://p/k> <http://e/a> .\n";
        let g = Graph::load_str(nt, "nt").unwrap();
        let ng = NodeGraph::build(&g);
        assert_eq!(ng.len(), 1);
        let nrm = degree_centrality_normalized(&ng, Direction::Total);
        assert_eq!(nrm.len(), 1);
        assert_eq!(
            nrm[0], 0.0,
            "single-node normalised degree must be 0.0, got {}",
            nrm[0]
        );
        assert!(!nrm[0].is_nan(), "guard fell through to a 0/0 divide");
    }

    /// [OPUS-4.8] sq-lqty mutation-kill: a two-node graph (`n == 2`) must NOT take the
    /// small-graph guard — it normalises by `n - 1 = 1`, so the in-degree-1 node scores
    /// exactly `1.0`. The `<`→`<=` mutant (centrality.rs:42) makes the guard `n <= 2`, which
    /// fires at `n == 2` and wrongly returns all zeros; this asserts a non-zero score, so it
    /// is caught.
    #[test]
    fn degree_centrality_normalized_two_nodes_not_short_circuited() {
        let nt = "<http://e/a> <http://p/k> <http://e/b> .\n";
        let g = Graph::load_str(nt, "nt").unwrap();
        let ng = NodeGraph::build(&g);
        assert_eq!(ng.len(), 2);
        let b = idx(&g, &ng, "http://e/b");
        let nrm = degree_centrality_normalized(&ng, Direction::In);
        assert!(
            (nrm[b] - 1.0).abs() < 1e-12,
            "b in-degree normalised = {} (expected 1.0)",
            nrm[b]
        );
    }

    #[test]
    fn top_k_is_deterministic_with_index_tiebreak() {
        let scores = vec![5usize, 5, 1, 5];
        let top = top_k(&scores, 2);
        // Three nodes tie at 5; smallest indices win → (0, 5) then (1, 5).
        assert_eq!(top, vec![(0, 5), (1, 5)]);
    }

    #[test]
    fn weakly_connected_components_partition() {
        // Two disjoint clusters: {a,b} and {c,d}.
        let nt = r#"
<http://e/a> <http://p/k> <http://e/b> .
<http://e/c> <http://p/k> <http://e/d> .
"#;
        let g = Graph::load_str(nt, "nt").unwrap();
        let ng = NodeGraph::build(&g);
        let comp = weakly_connected_components(&ng);
        assert_eq!(num_communities(&comp), 2);
        let a = ng.index_of(g.id_of(&iri("http://e/a")).unwrap()).unwrap();
        let b = ng.index_of(g.id_of(&iri("http://e/b")).unwrap()).unwrap();
        let c = ng.index_of(g.id_of(&iri("http://e/c")).unwrap()).unwrap();
        assert_eq!(comp[a], comp[b]);
        assert_ne!(comp[a], comp[c]);
    }

    #[test]
    fn wcc_ignores_edge_direction() {
        // a → b only; still one weakly connected component.
        let nt = "<http://e/a> <http://p/k> <http://e/b> .\n";
        let (_g, ng) = build(nt);
        let comp = weakly_connected_components(&ng);
        assert_eq!(num_communities(&comp), 1);
    }

    #[test]
    fn label_propagation_finds_two_cliques() {
        // Two triangles joined by a single bridge edge — LP should keep them apart,
        // or at worst find <= the WCC count (one component here).
        let nt = r#"
<http://e/a> <http://p/k> <http://e/b> .
<http://e/b> <http://p/k> <http://e/c> .
<http://e/c> <http://p/k> <http://e/a> .
<http://e/d> <http://p/k> <http://e/e> .
<http://e/e> <http://p/k> <http://e/f> .
<http://e/f> <http://p/k> <http://e/d> .
<http://e/c> <http://p/k> <http://e/d> .
"#;
        let (_g, ng) = build(nt);
        let comm = label_propagation(&ng, LabelPropConfig::default());
        // Deterministic: same labels across runs.
        let comm2 = label_propagation(&ng, LabelPropConfig::default());
        assert_eq!(comm, comm2);
        // Every node is labelled; ids are dense.
        assert_eq!(comm.len(), ng.len());
        assert!(num_communities(&comm) >= 1);
    }

    #[test]
    fn label_propagation_empty() {
        let g = Graph::load_str("", "nt").unwrap();
        let ng = NodeGraph::build(&g);
        assert!(label_propagation(&ng, LabelPropConfig::default()).is_empty());
    }

    /// [OPUS-4.8] sq-lqty mutation-kill: pins the EXACT label-propagation partition of a
    /// directed graph where the neighbour tallies in both directions are load-bearing.
    /// `a → c`, `b → c`, `c → d`, `c → e`, `d ⇄ e`: LP settles into two communities,
    /// `{a, b, c}` and `{d, e}`. The neighbour-count increments
    /// (community.rs:133 over out-neighbours, :136 over in-neighbours) are mutated to `*= 1`,
    /// which — because each count is freshly `or_insert(0)` — pins every tally at `0`. With a
    /// whole direction's votes zeroed the tie-break collapses the graph into a SINGLE
    /// community, so asserting the two distinct communities (and the membership) catches both
    /// the :133 and :136 `+=`→`*=` mutants.
    ///
    /// This is deterministic because [`NodeGraph`] assigns node indices in a CANONICAL
    /// (ascending-term) order — see `NodeGraph::build_with`. Before that fix the index order
    /// followed `Graph::iter_ids`, i.e. dictionary-id order, which the parallel sharded dict
    /// merge (`sparq-core` `parallel` feature, on by default) assigns thread-count-dependently;
    /// on this near-balanced graph that visiting order flipped LP between one and two
    /// communities, so this exact assertion was green locally but collapsed-to-one in CI.
    #[test]
    fn label_propagation_two_communities_exact_membership() {
        let nt = r#"
<http://e/a> <http://p/k> <http://e/c> .
<http://e/b> <http://p/k> <http://e/c> .
<http://e/c> <http://p/k> <http://e/d> .
<http://e/c> <http://p/k> <http://e/e> .
<http://e/d> <http://p/k> <http://e/e> .
<http://e/e> <http://p/k> <http://e/d> .
"#;
        let g = Graph::load_str(nt, "nt").unwrap();
        let ng = NodeGraph::build(&g);
        let comm = label_propagation(&ng, LabelPropConfig::default());
        let (a, b, c) = (
            idx(&g, &ng, "http://e/a"),
            idx(&g, &ng, "http://e/b"),
            idx(&g, &ng, "http://e/c"),
        );
        let (d, e) = (idx(&g, &ng, "http://e/d"), idx(&g, &ng, "http://e/e"));
        assert_eq!(
            num_communities(&comm),
            2,
            "expected exactly two LP communities: {comm:?}"
        );
        // {a, b, c} together, {d, e} together, and the two groups distinct.
        assert_eq!(comm[a], comm[b]);
        assert_eq!(comm[a], comm[c]);
        assert_eq!(comm[d], comm[e]);
        assert_ne!(comm[a], comm[d]);
    }

    /// [OPUS-4.8] sq-lqty mutation-kill: drives label-propagation to a NON-singleton fixed
    /// point that takes more than one sweep to reach, so the update predicate
    /// (community.rs:147 `best != label[i]`) and the stop condition
    /// (community.rs:152 `if !changed`) are both exercised by their effect on the final
    /// partition. On this 5-node graph LP converges to a single community, and asserting
    /// that single converged community kills both mutants:
    /// the `!=`→`==` mutant (:147) only ever "updates" a node to a label it already holds and
    /// never sets `changed`, so no label propagates and every node stays a singleton (5
    /// communities); deleting the `!` (:152) turns the stop into "break as soon as anything
    /// changed", halting after the first sweep with a half-finished, two-community partition.
    #[test]
    fn label_propagation_converges_past_first_sweep() {
        // Found to require >1 sweep AND to diverge under break-after-first-change.
        let nt = r#"
<http://e/c> <http://p/k> <http://e/e> .
<http://e/d> <http://p/k> <http://e/e> .
<http://e/d> <http://p/k> <http://e/b> .
<http://e/a> <http://p/k> <http://e/d> .
<http://e/c> <http://p/k> <http://e/a> .
<http://e/b> <http://p/k> <http://e/d> .
"#;
        let g = Graph::load_str(nt, "nt").unwrap();
        let ng = NodeGraph::build(&g);
        let comm = label_propagation(&ng, LabelPropConfig::default());
        assert_eq!(ng.len(), 5);
        assert_eq!(
            num_communities(&comm),
            1,
            "LP should converge to one community, got {comm:?}"
        );
    }

    #[test]
    fn node_index_round_trips_to_term() {
        let nt = "<http://e/a> <http://p/k> <http://e/b> .\n";
        let g = Graph::load_str(nt, "nt").unwrap();
        let ng = NodeGraph::build(&g);
        let a_id = g.id_of(&iri("http://e/a")).unwrap();
        let a = ng.index_of(a_id).unwrap();
        assert_eq!(ng.term(&g, a), iri("http://e/a"));
        assert_eq!(ng.dict_id(a), a_id);
    }

    /// [OPUS-4.8] sq-lqty determinism regression: `NodeGraph` must assign node indices in
    /// CANONICAL ascending-term order, independent of the dictionary-id order (which the
    /// parallel sharded loader assigns thread-count-dependently). This is the invariant that
    /// makes the order-sensitive `label_propagation` partition reproducible across hosts; the
    /// failing CI for this PR was exactly its absence. We assert the terms are in ascending
    /// order — the property every `NodeGraph` consumer can now rely on — using IRIs whose
    /// document order (`d`, `a`, `c`, `b`) differs from their sorted order (`a`, `b`, `c`, `d`).
    #[test]
    fn node_indices_are_canonical_term_order() {
        let nt = r#"
<http://e/d> <http://p/k> <http://e/a> .
<http://e/c> <http://p/k> <http://e/b> .
"#;
        let g = Graph::load_str(nt, "nt").unwrap();
        let ng = NodeGraph::build(&g);
        let terms: Vec<String> = (0..ng.len()).map(|i| ng.term(&g, i).to_string()).collect();
        let mut sorted = terms.clone();
        sorted.sort();
        assert_eq!(
            terms, sorted,
            "node indices are not in canonical term order"
        );
        // And the specific expected mapping: a→0, b→1, c→2, d→3 regardless of load order.
        assert_eq!(idx(&g, &ng, "http://e/a"), 0);
        assert_eq!(idx(&g, &ng, "http://e/b"), 1);
        assert_eq!(idx(&g, &ng, "http://e/c"), 2);
        assert_eq!(idx(&g, &ng, "http://e/d"), 3);
    }
}
