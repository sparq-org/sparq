//! Named-graph attribution of the zk-trace (plan §4.E + §2.4): the trace's
//! per-pattern `graph` tag and the consumer's leaf-resolution must agree on
//! which committed named graph each consumed triple belongs to, so the
//! per-property proof witnesses the right per-graph commitment block.
//!
//! These complement `trace_guard.rs` (the Q6 bnode anti-correlation tests):
//! here the focus is correct ATTRIBUTION, not rejection.
//!
//! written while Fable 5 unavailable — re-review when Fable returns.

use oxrdf::NamedNode;
use sparq_zk::encode::salt_from_bytes;
use sparq_zk::trace::ZkDataset;
use std::collections::BTreeSet;

/// Two credential graphs; cross-graph joins on an IRI only (the well-behaved
/// case from trace_guard, reused for attribution checks).
const DATA: &str = r#"
<http://ex/alice> <http://ex/worksAt> <http://ex/acme> <http://ex/g/employment> .
<http://ex/alice> <http://ex/title> "Engineer" <http://ex/g/employment> .
<http://ex/acme> <http://ex/sector> "robotics" <http://ex/g/registry> .
<http://ex/acme> <http://ex/founded> "1990" <http://ex/g/registry> .
"#;

fn dataset() -> ZkDataset {
    ZkDataset::from_dataset_str(
        DATA,
        "n-quads",
        &[
            NamedNode::new("http://ex/g/employment").unwrap(), // graph 0
            NamedNode::new("http://ex/g/registry").unwrap(),   // graph 1
        ],
        &[salt_from_bytes(&[1u8; 32]), salt_from_bytes(&[2u8; 32])],
    )
    .unwrap()
}

/// Each pattern's consumed triples resolve to the CORRECT named graph: the
/// employment-side pattern attributes to graph 0, the registry-side to graph 1.
#[test]
fn cross_graph_join_attributes_each_pattern_to_its_graph() {
    let ds = dataset();
    let exec = ds
        .query_traced(
            "SELECT ?sector WHERE { \
               <http://ex/alice> <http://ex/worksAt> ?org . \
               ?org <http://ex/sector> ?sector }",
        )
        .unwrap();
    assert_eq!(exec.result.rows.len(), 1);
    let attrs = exec.attributions();
    assert_eq!(attrs.len(), 2);

    // The worksAt pattern is in employment (graph 0); the sector pattern is in
    // registry (graph 1). Find them by predicate.
    let graph_of = |pred: &str| -> BTreeSet<usize> {
        attrs
            .iter()
            .find(|(k, _)| k.slots[1].to_string() == pred)
            .map(|(_, s)| s.clone())
            .unwrap_or_else(|| panic!("pattern with predicate {pred} not traced"))
    };
    assert_eq!(graph_of("<http://ex/worksAt>"), BTreeSet::from([0]));
    assert_eq!(graph_of("<http://ex/sector>"), BTreeSet::from([1]));
}

/// A single-graph query: every consumed triple resolves within ONE graph; the
/// other graph contributes nothing.
#[test]
fn single_graph_query_attributes_to_one_graph_only() {
    let ds = dataset();
    let exec = ds
        .query_traced(
            "SELECT ?t WHERE { <http://ex/alice> <http://ex/title> ?t }",
        )
        .unwrap();
    assert_eq!(exec.result.rows.len(), 1);
    let all: BTreeSet<usize> = exec
        .attributions()
        .iter()
        .flat_map(|(_, s)| s.iter().copied())
        .collect();
    assert_eq!(all, BTreeSet::from([0]), "title lives only in employment");
}

/// Every resolved leaf index is within range of its graph's commitment — the
/// witness references are valid leaf offsets, not dangling.
#[test]
fn resolved_leaves_are_in_commitment_range() {
    let ds = dataset();
    let exec = ds
        .query_traced(
            "SELECT * WHERE { <http://ex/acme> <http://ex/sector> ?s . \
                              <http://ex/acme> <http://ex/founded> ?f }",
        )
        .unwrap();
    for rp in &exec.patterns {
        for refs in &rp.leaves {
            assert!(!refs.is_empty(), "every traced triple must attribute somewhere");
            for r in refs {
                assert!(
                    r.leaf < ds.graphs[r.graph].commitment.leaves.len(),
                    "leaf {} out of range for graph {}",
                    r.leaf,
                    r.graph
                );
            }
        }
    }
}
