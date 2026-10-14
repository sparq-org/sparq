//! sq-bif.8: integration suite for `sparq-algos` over the crate's PUBLIC API.
//!
//! The crate already has a strong body of inline unit tests in `src/lib.rs` (a
//! hand-computed PageRank stationary-distribution oracle, LP exact-membership,
//! determinism / tie-break, NaN guards). This `tests/` suite deliberately exercises the
//! TOPOLOGY classes those unit tests do NOT cover, with known-answer / property oracles
//! that fail on a real regression:
//!
//! * **Dangling-node PageRank** — sink (out-degree-0) nodes that would otherwise leak rank
//!   mass; the result must still be a probability distribution (sums to 1) and the sink
//!   must out-rank its sources.
//! * **Disconnected + isolated-node + chain topologies** — WCC partition correctness and
//!   LP membership across components that the connected unit-test graphs never reach.
//! * **All-singleton community case** — a graph whose only edges are self-loops, so every
//!   node is its own community in both WCC and LP.
//! * **`NodeFilter::All` literal-inclusion `build_with` path** — literals promoted to
//!   sink nodes, and their downstream effect on degree / PageRank.
//!
//! Everything goes through the published surface (`Graph::load_str`, `NodeGraph`,
//! `pagerank`, `weakly_connected_components`, `label_propagation`, the centralities) — the
//! same entry points a downstream analytics consumer uses.

use oxrdf::{NamedNode, Term};
use sparq_algos::{
    degree_centrality, label_propagation, num_communities, pagerank, weakly_connected_components,
    Direction, LabelPropConfig, NodeFilter, NodeGraph, PageRankConfig,
};
use sparq_core::Graph;

/// Builds a graph from N-Triples and the default entity node-graph view.
fn build(nt: &str) -> (Graph, NodeGraph) {
    let g = Graph::load_str(nt, "nt").expect("parse N-Triples");
    let ng = NodeGraph::build(&g);
    (g, ng)
}

fn iri(s: &str) -> Term {
    Term::NamedNode(NamedNode::new(s).unwrap())
}

/// Resolves the dense node index of an IRI in `(graph, node_graph)`.
fn idx(g: &Graph, ng: &NodeGraph, name: &str) -> usize {
    ng.index_of(g.id_of(&iri(name)).unwrap())
        .unwrap_or_else(|| panic!("node {name} not in view"))
}

// ---------------------------------------------------------------------------
// Dangling-node PageRank.
// ---------------------------------------------------------------------------

/// A "star INTO a sink": three sources each point at one sink `s`; `s` has NO out-edges,
/// so it is a dangling node. The dangling-mass redistribution must keep the result a
/// probability distribution (sum == 1), and the sink must collect more rank than any of
/// its leaves. This is the property the unit tests' connected (cyclic) graphs cannot show,
/// because there every node has an out-edge.
#[test]
fn pagerank_dangling_sink_redistributes_mass_and_sums_to_one() {
    let nt = r#"
<http://e/p> <http://r/k> <http://e/s> .
<http://e/q> <http://r/k> <http://e/s> .
<http://e/t> <http://r/k> <http://e/s> .
"#;
    let (g, ng) = build(nt);
    // `s` is dangling (out-degree 0); the three sources are dangling too (they only emit),
    // so EVERY node is a sink here except via teleport — the redistribution path is fully
    // exercised.
    let s = idx(&g, &ng, "http://e/s");
    assert_eq!(ng.out_degree(s), 0, "sink must have no out-edges");

    let r = pagerank(&ng, PageRankConfig::default());
    let sum: f64 = r.iter().sum();
    assert!(
        (sum - 1.0).abs() < 1e-9,
        "dangling-node PageRank must still sum to 1, got {}",
        sum
    );
    // The sink is the only node with in-edges, so it must hold the most mass.
    for name in ["http://e/p", "http://e/q", "http://e/t"] {
        let leaf = idx(&g, &ng, name);
        assert!(
            r[s] > r[leaf],
            "sink rank {} must exceed leaf {} rank {}",
            r[s],
            name,
            r[leaf]
        );
    }
}

/// A chain that ENDS in a dangling node: `a -> b -> c`, `c` has no out-edge. Without the
/// dangling-mass fix, `c`'s rank would leak away each iteration and the vector would NOT
/// sum to 1. Pin both: sum == 1, and the strictly-monotone rank along the chain (`c`
/// accumulates the most, `a` the least) since rank flows downstream.
#[test]
fn pagerank_chain_terminating_in_dangling_node_conserves_mass() {
    let nt = r#"
<http://e/a> <http://r/k> <http://e/b> .
<http://e/b> <http://r/k> <http://e/c> .
"#;
    let (g, ng) = build(nt);
    let (a, b, c) = (
        idx(&g, &ng, "http://e/a"),
        idx(&g, &ng, "http://e/b"),
        idx(&g, &ng, "http://e/c"),
    );
    assert_eq!(ng.out_degree(c), 0, "chain end is dangling");

    let r = pagerank(&ng, PageRankConfig::default());
    let sum: f64 = r.iter().sum();
    assert!((sum - 1.0).abs() < 1e-9, "mass must be conserved: {}", sum);
    // Rank flows a -> b -> c, so it accumulates downstream.
    assert!(r[c] > r[b], "tail {} should outrank middle {}", r[c], r[b]);
    assert!(r[b] > r[a], "middle {} should outrank head {}", r[b], r[a]);
}

// ---------------------------------------------------------------------------
// Disconnected / isolated-node / chain topologies (WCC + LP).
// ---------------------------------------------------------------------------

/// Three disjoint components of DIFFERENT shapes — a 3-chain `{a,b,c}`, a single edge
/// `{d,e}`, and a self-loop singleton `{f}` (an isolated-by-others node). WCC must produce
/// exactly three communities with the expected membership, and LP must AGREE on the
/// coarse partition (no node ends in a community spanning two WCCs — LP only ever refines
/// within a component, never merges across one).
#[test]
fn wcc_partitions_three_disjoint_components_and_lp_does_not_cross_them() {
    let nt = r#"
<http://e/a> <http://r/k> <http://e/b> .
<http://e/b> <http://r/k> <http://e/c> .
<http://e/d> <http://r/k> <http://e/e> .
<http://e/f> <http://r/k> <http://e/f> .
"#;
    let (g, ng) = build(nt);
    assert_eq!(ng.len(), 6, "a,b,c,d,e,f");

    let comp = weakly_connected_components(&ng);
    assert_eq!(
        num_communities(&comp),
        3,
        "three disjoint components: {comp:?}"
    );
    let (a, b, c) = (
        idx(&g, &ng, "http://e/a"),
        idx(&g, &ng, "http://e/b"),
        idx(&g, &ng, "http://e/c"),
    );
    let (d, e, f) = (
        idx(&g, &ng, "http://e/d"),
        idx(&g, &ng, "http://e/e"),
        idx(&g, &ng, "http://e/f"),
    );
    // Chain {a,b,c} all together.
    assert_eq!(comp[a], comp[b]);
    assert_eq!(comp[b], comp[c]);
    // Edge {d,e} together, distinct from the chain.
    assert_eq!(comp[d], comp[e]);
    assert_ne!(comp[a], comp[d]);
    // Self-loop singleton {f} alone, distinct from both.
    assert_ne!(comp[f], comp[a]);
    assert_ne!(comp[f], comp[d]);

    // LP must NOT merge two distinct WCCs into one community: two nodes in different WCCs
    // can never share an LP community (LP only propagates along edges, which never cross
    // components). Assert it for every cross-component pair.
    let lp = label_propagation(&ng, LabelPropConfig::default());
    assert_eq!(lp.len(), ng.len());
    for &x in &[a, b, c] {
        for &y in &[d, e, f] {
            assert_ne!(
                lp[x], lp[y],
                "LP community spilled across two WCC components"
            );
        }
    }
}

/// A pure chain `a -> b -> c -> d` is ONE weakly connected component even though no two
/// non-adjacent nodes are directly connected — WCC ignores direction and follows the path.
/// This is the chain-topology case the bead calls out for WCC.
#[test]
fn chain_is_one_weakly_connected_component() {
    let nt = r#"
<http://e/a> <http://r/k> <http://e/b> .
<http://e/b> <http://r/k> <http://e/c> .
<http://e/c> <http://r/k> <http://e/d> .
"#;
    let (_g, ng) = build(nt);
    assert_eq!(ng.len(), 4);
    let comp = weakly_connected_components(&ng);
    assert_eq!(num_communities(&comp), 1, "a 4-chain is one WCC: {comp:?}");
    // Every node carries the SAME (densified) component id 0.
    assert!(comp.iter().all(|&c| c == comp[0]));
}

// ---------------------------------------------------------------------------
// All-singleton community case.
// ---------------------------------------------------------------------------

/// A graph whose only edges are self-loops on distinct nodes: there are no inter-node
/// edges at all, so EVERY node is its own community under BOTH WCC and LP. The bead
/// explicitly calls out the "all-singleton communities" case, which the existing tests
/// (all of which produce >= 1 multi-member community) never reach.
#[test]
fn all_self_loops_yield_all_singleton_communities() {
    let nt = r#"
<http://e/a> <http://r/k> <http://e/a> .
<http://e/b> <http://r/k> <http://e/b> .
<http://e/c> <http://r/k> <http://e/c> .
"#;
    let (_g, ng) = build(nt);
    assert_eq!(ng.len(), 3);

    // WCC: a self-loop unions a node with itself, so it stays alone -> 3 singletons.
    let comp = weakly_connected_components(&ng);
    assert_eq!(
        num_communities(&comp),
        3,
        "self-loops only -> all singletons (WCC): {comp:?}"
    );
    // Every community id is distinct.
    let mut ids = comp.clone();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), 3, "no two nodes share a WCC community");

    // LP: a node's only neighbour (via the self-loop) is ITSELF, so its plurality label is
    // its own — nothing ever changes, every node stays a singleton.
    let lp = label_propagation(&ng, LabelPropConfig::default());
    assert_eq!(
        num_communities(&lp),
        3,
        "self-loops only -> all singletons (LP): {lp:?}"
    );
    let mut lp_ids = lp.clone();
    lp_ids.sort_unstable();
    lp_ids.dedup();
    assert_eq!(lp_ids.len(), 3, "no two nodes share an LP community");
}

// ---------------------------------------------------------------------------
// NodeFilter::All literal-inclusion build_with path.
// ---------------------------------------------------------------------------

/// `NodeFilter::All` promotes literal objects to (sink) nodes, so the same data yields a
/// LARGER view than the default `EntitiesOnly`. Pin the structural difference AND the
/// downstream consequence: the literal becomes a dangling in-edge target, so under `All`
/// the literal node has in-degree 1 / out-degree 0, and PageRank still sums to 1 over the
/// enlarged node set. The unit test `literals_excluded_by_default_but_included_with_all`
/// covers node/edge counts; this drives the literal node through the analytics layer.
#[test]
fn node_filter_all_promotes_literal_to_a_pagerank_sink() {
    let nt = r#"
<http://e/a> <http://r/name> "Alice" .
<http://e/a> <http://r/k> <http://e/b> .
"#;
    let g = Graph::load_str(nt, "nt").unwrap();

    let entities = NodeGraph::build(&g);
    let all = NodeGraph::build_with(&g, NodeFilter::All);
    // The literal is a third node + a second edge only under `All`.
    assert_eq!(entities.len(), 2);
    assert_eq!(all.len(), 3);
    assert_eq!(entities.edge_count(), 1);
    assert_eq!(all.edge_count(), 2);

    // The literal node: in-degree 1 (a -> "Alice"), out-degree 0 (a literal is never a
    // subject), i.e. a dangling sink. Find it as the node that is NOT one of the IRIs.
    let a = all.index_of(g.id_of(&iri("http://e/a")).unwrap()).unwrap();
    let b = all.index_of(g.id_of(&iri("http://e/b")).unwrap()).unwrap();
    let lit = (0..all.len())
        .find(|&i| i != a && i != b)
        .expect("literal node present under NodeFilter::All");
    assert_eq!(all.in_degree(lit), 1, "literal has one in-edge from a");
    assert_eq!(all.out_degree(lit), 0, "literal is a sink (never a subject)");

    // Degree centrality sees the literal as an in-degree-1 node.
    let inc = degree_centrality(&all, Direction::In);
    assert_eq!(inc[lit], 1);

    // PageRank over the enlarged (literal-bearing) view is still a distribution.
    let r = pagerank(&all, PageRankConfig::default());
    assert_eq!(r.len(), 3);
    let sum: f64 = r.iter().sum();
    assert!(
        (sum - 1.0).abs() < 1e-9,
        "PageRank over NodeFilter::All view must sum to 1, got {}",
        sum
    );
}
