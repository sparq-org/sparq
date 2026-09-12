// [GPT-6] Native deterministic identity tests; no guest/proof claim.
#![cfg(feature = "deterministic-blank-nodes")]

use oxrdf::{NamedOrBlankNode, Term};
use sparq_core::Graph;
use sparq_engine::{QueryBudget, construct, construct_with_budget, query};
use std::collections::BTreeSet;

#[test]
fn template_nodes_are_fresh_per_duplicate_solution_and_disjoint_from_source() {
    let graph = Graph::load_dataset(
        "_:tc0_0_0 <http://ex/p> <http://ex/o> .\n\
         _:tc1_0_0 <http://ex/q> <http://ex/o> .\n\
         _:tc2_0_0 <http://ex/p> <http://ex/o> <http://ex/unselected> .",
        "nquads",
    )
    .unwrap();
    let source = BTreeSet::from(["tc0_0_0", "tc1_0_0", "tc2_0_0"]);
    let text = "PREFIX ex:<http://ex/> CONSTRUCT { _:fresh ex:p ?v . _:fresh ex:q ?v } WHERE { VALUES ?v { ex:o ex:o } }";
    let result = construct(&graph, text).unwrap();
    assert_eq!(
        result.len(),
        4,
        "duplicate solutions need distinct template nodes"
    );
    let subjects: BTreeSet<_> = result
        .iter()
        .map(|triple| {
            let NamedOrBlankNode::BlankNode(node) = &triple.subject else {
                panic!("blank subject")
            };
            assert!(
                !source.contains(node.as_str()),
                "fresh node collides with active input"
            );
            node.as_str()
        })
        .collect();
    assert_eq!(
        subjects.len(),
        2,
        "the same template label must be shared within a row"
    );
    assert_eq!(
        result,
        construct(&graph, text).unwrap(),
        "labels are result-local and deterministic"
    );
}

#[test]
fn anonymous_patterns_remain_hidden_existentials_and_lists_construct_fresh_nodes() {
    let graph = Graph::load_str(
        "<http://ex/a> <http://ex/p> <http://ex/o> .\n_:data <http://ex/p> <http://ex/o> .",
        "ntriples",
    )
    .unwrap();
    let result = query(&graph, "SELECT * { [] <http://ex/p> ?o }").unwrap();
    assert_eq!(result.vars.len(), 1);
    assert_eq!(result.vars[0].as_str(), "o");
    assert_eq!(result.rows.len(), 2);
    let result = construct(
        &graph,
        "PREFIX ex:<http://ex/> CONSTRUCT { ex:s ex:list (ex:a ex:b) } WHERE {}",
    )
    .unwrap();
    assert_eq!(result.len(), 5);
    let nodes: BTreeSet<_> = result
        .iter()
        .flat_map(|triple| {
            [
                match &triple.subject {
                    NamedOrBlankNode::BlankNode(node) => Some(node.as_str()),
                    _ => None,
                },
                match &triple.object {
                    Term::BlankNode(node) => Some(node.as_str()),
                    _ => None,
                },
            ]
            .into_iter()
            .flatten()
        })
        .collect();
    assert_eq!(nodes.len(), 2);
}

#[test]
fn generated_graph_budget_rejects_without_returning_a_partial_graph() {
    let graph = Graph::load_str("", "ntriples").unwrap();
    let budget = QueryBudget {
        max_rows: Some(1),
        ..Default::default()
    };
    let query = "PREFIX ex:<http://ex/> CONSTRUCT { _:x ex:p ex:a . _:x ex:q ex:b } WHERE {}";
    assert!(construct_with_budget(&graph, query, &budget).is_err());
    assert_eq!(construct(&graph, query).unwrap().len(), 2);
}
