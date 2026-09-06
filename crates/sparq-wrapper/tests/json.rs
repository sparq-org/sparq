// [SONNET-4.6] sq-1rg2q.11: public-surface witnesses for the cycle-safe JSON
// projection. The in-module unit tests pin each knob; this file pins the two
// properties the proposal actually promises across the crate boundary — the
// projection is deterministic and total on a cyclic graph, and it never drops a
// literal's datatype or language tag.

#![cfg(feature = "proposed-json")]

use oxrdf::{BlankNode, Literal, NamedNode};
use sparq_core::Graph;
use sparq_wrapper::proposed::json::{JsonProjection, RepeatedFocus};
use sparq_wrapper::Store;

fn iri(local: &str) -> NamedNode {
    NamedNode::new(format!("http://example.org/{}", local)).expect("valid IRI")
}

/// `a knows b`, `b knows a` — the two-node cycle the acceptance test needs —
/// carrying one language-tagged and one typed literal on each side.
fn cycle() -> Graph {
    Graph::load_str(
        "@prefix ex: <http://example.org/> .\n\
         ex:a ex:knows ex:b ; ex:label \"Ay\"@en .\n\
         ex:b ex:knows ex:a ; ex:label \"Bee\"@en ; ex:age 42 .\n",
        "turtle",
    )
    .expect("valid turtle")
}

/// The whole document, spelled out. Anything that changes the key order, the
/// object order, the reference shape, or a literal's retained metadata changes
/// this string and fails the test.
const EXPECTED_CYCLE: &str = concat!(
    r#"{"@id":"http://example.org/a","#,
    r#""http://example.org/knows":[{"@id":"http://example.org/b","#,
    r#""http://example.org/age":[{"@value":"42","@type":"http://www.w3.org/2001/XMLSchema#integer"}],"#,
    r#""http://example.org/knows":[{"@ref":"http://example.org/a"}],"#,
    r#""http://example.org/label":[{"@value":"Bee","@language":"en"}]}],"#,
    r#""http://example.org/label":[{"@value":"Ay","@language":"en"}]}"#,
);

#[test]
fn a_to_b_to_a_serializes_identically_twice_and_keeps_literal_metadata() {
    let graph = cycle();
    let store = Store::borrowed(&graph);
    let projection = JsonProjection::new();

    let first = projection.project(&store.node(iri("a")));
    let second = projection.project(&store.node(iri("a")));

    // Deterministic: byte-identical across runs, and equal to the document the
    // test spells out rather than to whatever the implementation produced.
    assert_eq!(first, second);
    assert_eq!(first, EXPECTED_CYCLE);

    // Total: the cycle back to `a` closed as a reference, so `a` is expanded
    // exactly once and the reference names the same stable term as its `@id`.
    assert_eq!(first.matches(r#""@id":"http://example.org/a""#).count(), 1);
    assert!(first.contains(r#"{"@ref":"http://example.org/a"}"#));

    // Metadata: the language tag and the datatype IRI both survive.
    assert!(first.contains(r#"{"@value":"Bee","@language":"en"}"#));
    assert!(first
        .contains(r#"{"@value":"42","@type":"http://www.w3.org/2001/XMLSchema#integer"}"#));
}

#[test]
fn every_repeated_focus_policy_terminates_on_the_same_cycle() {
    let graph = cycle();
    let store = Store::borrowed(&graph);
    let node = store.node(iri("a"));

    for policy in [RepeatedFocus::OnCycle, RepeatedFocus::OnRepeat] {
        let projection = JsonProjection::new().with_repeated_focus(policy);
        let projected = projection.project(&node);

        assert_eq!(projected, projection.project(&node), "{:?} is unstable", policy);
        assert!(
            projected.contains(r#"{"@ref":"http://example.org/a"}"#),
            "{:?} did not close the cycle with a reference: {}",
            policy,
            projected
        );
        // Closed at the FIRST return to `a`, not merely somewhere before the
        // depth bound: every node in the cycle is expanded exactly once.
        for local in ["a", "b"] {
            let expansion = format!(r#""@id":"http://example.org/{}""#, local);
            assert_eq!(
                projected.matches(&expansion).count(),
                1,
                "{:?} expanded {} more than once: {}",
                policy,
                local,
                projected
            );
        }
    }
}

#[test]
fn a_blank_node_cycle_projects_a_stable_reference_to_its_own_identifier() {
    let mut store = Store::new();
    let knows = iri("knows");
    let anchor = BlankNode::default();
    let other = BlankNode::default();
    store
        .insert(anchor.clone(), knows.clone(), other.clone())
        .expect("forward insert");
    store
        .insert(other, knows, anchor.clone())
        .expect("back insert");

    let projection = JsonProjection::new();
    let projected = projection.project(&store.node(anchor.clone()));

    assert_eq!(projected, projection.project(&store.node(anchor.clone())));
    // A blank node is referenced by the same `_:` label it is expanded under,
    // so the reference resolves inside the document.
    let reference = format!(r#"{{"@ref":"_:{}"}}"#, anchor.as_str());
    assert!(
        projected.contains(&reference),
        "expected {} in {}",
        reference,
        projected
    );
    let expansion = format!(r#""@id":"_:{}""#, anchor.as_str());
    assert_eq!(
        projected.matches(&expansion).count(),
        1,
        "the blank-node cycle re-expanded its anchor: {}",
        projected
    );
}

#[test]
fn a_plain_string_and_a_typed_string_project_to_distinguishable_values() {
    let mut store = Store::new();
    let subject = iri("subject");
    let plain = iri("plain");
    let tagged = iri("tagged");
    store
        .insert(
            subject.clone(),
            plain,
            Literal::new_simple_literal("Ambiguous"),
        )
        .expect("plain insert");
    store
        .insert(
            subject.clone(),
            tagged,
            Literal::new_language_tagged_literal("Ambiguous", "en").expect("valid language"),
        )
        .expect("tagged insert");

    let projected = JsonProjection::new().project(&store.node(subject));

    // Same lexical form on both edges: only the retained metadata tells them
    // apart, which is exactly what "never discards metadata" has to mean.
    assert!(projected
        .contains(r#""@value":"Ambiguous","@type":"http://www.w3.org/2001/XMLSchema#string""#));
    assert!(projected.contains(r#""@value":"Ambiguous","@language":"en""#));
}
