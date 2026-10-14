// sq-bif.37: pin absent-term and excluded-predicate signature contracts.

use oxrdf::{NamedNode, Term};
use sparq_core::Graph;
use sparq_sim::{Sim, SimConfig};

const NS: &str = "http://example.com/";

fn named_node(local: &str) -> NamedNode {
    NamedNode::new(format!("{NS}{local}")).unwrap()
}

fn term(local: &str) -> Term {
    Term::NamedNode(named_node(local))
}

fn fixture() -> Graph {
    Graph::load_str(
        "@prefix ex: <http://example.com/> . ex:subject ex:onlyPredicate ex:object .",
        "turtle",
    )
    .unwrap()
}

#[test]
fn absent_terms_have_no_signature_similarity_or_ranked_results() {
    let graph = fixture();
    let sim = Sim::new(&graph);
    let absent = term("absent");
    let present = term("subject");

    assert!(sim.signature(&absent).is_none());
    assert_eq!(sim.similarity(&absent, &present), 0.0);
    assert_eq!(
        sim.similarity(&absent, &present),
        sim.similarity(&present, &absent)
    );
    assert!(sim.most_similar(&absent, 3).is_empty());

    let present_signature = sim
        .signature(&present)
        .expect("fixture subject must have a signature");
    assert!(!present_signature.is_empty());
    assert!(sim.similar_by_signature(&present_signature, 0).is_empty());
}

#[test]
fn excluding_a_subjects_only_predicate_leaves_an_empty_signature() {
    let graph = fixture();
    let sim = Sim::with_config(
        &graph,
        SimConfig {
            exclude_predicates: vec![named_node("onlyPredicate")],
            ..SimConfig::default()
        },
    );

    let signature = sim
        .signature(&term("subject"))
        .expect("excluded predicate must not remove the subject from the dictionary");
    assert!(signature.is_empty());
    assert_eq!(signature.len(), 0);
}
