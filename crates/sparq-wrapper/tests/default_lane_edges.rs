// sq-bif.39: default-lane empty-traversal contract witnesses.

use oxrdf::{NamedNode, Term};
use sparq_core::Graph;
use sparq_wrapper::Store;

fn iri(local: &str) -> NamedNode {
    NamedNode::new(format!("http://example.org/{local}")).unwrap()
}

fn one_triple_graph() -> Graph {
    Graph::load_str(
        "@prefix ex: <http://example.org/> . ex:s ex:p ex:o .",
        "turtle",
    )
    .unwrap()
}

#[test]
fn outgoing_traversal_with_absent_predicate_is_empty() {
    let graph = one_triple_graph();
    let store = Store::borrowed(&graph);

    assert_eq!(store.node(iri("s")).out(&iri("absent")).count(), 0);
}

#[test]
fn absent_focus_has_no_incoming_or_outgoing_traversal() {
    let graph = one_triple_graph();
    let store = Store::borrowed(&graph);
    let absent = store.node(iri("absent"));

    assert_eq!(absent.out(&iri("p")).count(), 0);
    assert_eq!(absent.r#in(&iri("p")).count(), 0);
}

#[test]
fn values_yields_absent_focus_exactly_once() {
    let graph = one_triple_graph();
    let store = Store::borrowed(&graph);
    let absent = iri("absent");

    assert_eq!(
        store.node(absent.clone()).values().collect::<Vec<_>>(),
        vec![Term::from(absent)]
    );
}

#[test]
fn subject_only_focus_has_outgoing_but_no_incoming_match() {
    let graph = one_triple_graph();
    let store = Store::borrowed(&graph);
    let subject = store.node(iri("s"));

    assert_eq!(subject.r#in(&iri("p")).count(), 0);
    assert_eq!(
        subject.out(&iri("p")).values().collect::<Vec<_>>(),
        vec![Term::from(iri("o"))]
    );
}
