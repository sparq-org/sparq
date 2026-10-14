//! sq-wxxkk: generated incremental-completion delta histories.

use oxrdf::{Literal, NamedNode, Term};
use proptest::prelude::*;
use proptest::test_runner::RngSeed;
use sparq_core::Graph;
use sparq_text::CompletionIndex;

const RDFS_LABEL: &str = "http://www.w3.org/2000/01/rdf-schema#label";

#[derive(Clone, Debug)]
struct Step {
    resource: u8,
    insert_label: bool,
    delete_edge: bool,
    probe: u8,
}

fn iri(value: impl Into<String>) -> Term {
    Term::NamedNode(NamedNode::new_unchecked(value.into()))
}

fn resource(n: u8) -> Term {
    iri(format!("http://example.com/resource/item-{n}"))
}

fn label_triple(n: u8) -> [Term; 3] {
    [
        resource(n),
        iri(RDFS_LABEL),
        Term::Literal(Literal::new_simple_literal(format!("Item {n}"))),
    ]
}

fn edge_triple(n: u8) -> [Term; 3] {
    [
        resource(n),
        iri("http://example.com/related"),
        resource(n.wrapping_add(1)),
    ]
}

fn history() -> impl Strategy<Value = (Vec<u8>, Vec<Step>)> {
    (
        prop::collection::vec(0u8..24, 0..12),
        prop::collection::vec(
            (0u8..32, any::<bool>(), any::<bool>(), 0u8..4).prop_map(
                |(resource, insert_label, delete_edge, probe)| Step {
                    resource,
                    insert_label,
                    delete_edge,
                    probe,
                },
            ),
            1..32,
        ),
    )
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 128,
        rng_seed: RngSeed::Fixed(0x5eed_c0de_d311_a001),
        ..ProptestConfig::default()
    })]

    #[test]
    fn incremental_completion_matches_rebuild((base, steps) in history()) {
        let mut graph = Graph::load_str("", "turtle").unwrap();
        let base = base.into_iter().flat_map(|n| [label_triple(n), edge_triple(n)]).collect::<Vec<_>>();
        graph.apply_delta(&base, &[]).unwrap();
        let mut incremental = CompletionIndex::build(&graph);

        for (round, step) in steps.into_iter().enumerate() {
            let inserts = step.insert_label.then(|| label_triple(step.resource)).into_iter().collect::<Vec<_>>();
            let edge = edge_triple(step.resource);
            let deletes = step.delete_edge.then_some(edge).into_iter().collect::<Vec<_>>();

            graph.apply_delta(&inserts, &deletes).unwrap();
            incremental.apply_delta(&graph, &inserts, &deletes);
            let rebuilt = CompletionIndex::build(&graph);
            let prefix = match step.probe {
                0 => "item",
                1 => "http://example.com/resource/item-",
                2 => "i",
                _ => "missing",
            };

            prop_assert_eq!(
                incremental.complete(prefix, usize::MAX, None),
                rebuilt.complete(prefix, usize::MAX, None),
                "completion mismatch after generated batch {}",
                round
            );
            prop_assert!(incremental.is_consistent_with(&graph));

            let before = incremental.clone();
            incremental.apply_delta(&graph, &[], &[]);
            prop_assert_eq!(incremental.clone(), before, "empty delta changed the index");
        }
    }
}
