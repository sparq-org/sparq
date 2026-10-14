// sq-1rg2q.3: direct coverage for every proposed cardinality function.

#![cfg(feature = "proposed-cardinality")]

use oxrdf::{Literal, NamedNode, Term};
use sparq_wrapper::proposed::cardinality::{
    live_mapped, many, optional, required, CardinalityViewError, CollectionError,
};
use sparq_wrapper::{Cardinality, Store};

fn iri(local: &str) -> NamedNode {
    NamedNode::new(format!("http://example.org/{local}")).unwrap()
}

fn decode_string(node: sparq_wrapper::Node<'_>) -> Result<String, sparq_wrapper::AccessError> {
    node.as_str().map(str::to_owned)
}

fn encode_string(value: &String) -> Result<Term, &'static str> {
    if value == "reject" {
        Err("rejected by test codec")
    } else {
        Ok(Literal::new_simple_literal(value).into())
    }
}

#[test]
fn required_directly_witnesses_zero_one_and_two_values() {
    let mut store = Store::new();
    let alice = iri("alice");
    let name = iri("name");

    let error = required(&store.node(alice.clone()), &name, decode_string).unwrap_err();
    assert!(matches!(
        error,
        CardinalityViewError::Cardinality(error)
            if error.expected == Cardinality::ExactlyOne && error.found == 0
    ));

    store
        .insert(
            alice.clone(),
            name.clone(),
            Literal::new_simple_literal("Alice"),
        )
        .unwrap();
    assert_eq!(
        required(&store.node(alice.clone()), &name, decode_string).unwrap(),
        "Alice"
    );

    store
        .insert(
            alice.clone(),
            name.clone(),
            Literal::new_simple_literal("Alicia"),
        )
        .unwrap();
    let error = required(&store.node(alice), &name, decode_string).unwrap_err();
    assert!(matches!(
        error,
        CardinalityViewError::Cardinality(error)
            if error.expected == Cardinality::ExactlyOne && error.found == 2
    ));
}

#[test]
fn optional_directly_witnesses_zero_one_and_two_values() {
    let mut store = Store::new();
    let alice = iri("alice");
    let nick = iri("nick");

    assert_eq!(
        optional(&store.node(alice.clone()), &nick, decode_string).unwrap(),
        None
    );
    store
        .insert(
            alice.clone(),
            nick.clone(),
            Literal::new_simple_literal("Al"),
        )
        .unwrap();
    assert_eq!(
        optional(&store.node(alice.clone()), &nick, decode_string).unwrap(),
        Some("Al".to_owned())
    );
    store
        .insert(
            alice.clone(),
            nick.clone(),
            Literal::new_simple_literal("Lise"),
        )
        .unwrap();
    let error = optional(&store.node(alice), &nick, decode_string).unwrap_err();
    assert!(matches!(
        error,
        CardinalityViewError::Cardinality(error)
            if error.expected == Cardinality::AtMostOne && error.found == 2
    ));
}

#[test]
fn many_directly_preserves_distinct_terms_with_equal_mapped_values() {
    let mut store = Store::new();
    let alice = iri("alice");
    let label = iri("label");
    store
        .insert(
            alice.clone(),
            label.clone(),
            Literal::new_simple_literal("one"),
        )
        .unwrap();
    store
        .insert(
            alice.clone(),
            label.clone(),
            Literal::new_language_tagged_literal_unchecked("one", "en"),
        )
        .unwrap();

    let values = many(&store.node(alice), &label, |_| Ok::<_, ()>(1_u8)).unwrap();
    assert_eq!(values, vec![1, 1]);
}

#[test]
fn live_mapped_constructor_returns_a_live_view() {
    let mut store = Store::new();
    let alice = iri("alice");
    let tag = iri("tag");
    let mut values = live_mapped(&mut store, alice, tag, decode_string, encode_string);

    assert!(values.is_empty());
    assert!(values.insert(&"rust".to_owned()).unwrap());
    assert_eq!(values.values().unwrap(), vec!["rust"]);
}

#[test]
fn live_values_requery_after_mutation_without_reconstruction() {
    let mut store = Store::new();
    let mut values = live_mapped(
        &mut store,
        iri("alice"),
        iri("tag"),
        decode_string,
        encode_string,
    );

    assert_eq!(values.values().unwrap(), Vec::<String>::new());
    values.insert(&"rdf".to_owned()).unwrap();
    assert_eq!(values.values().unwrap(), vec!["rdf"]);
    values.remove(&"rdf".to_owned()).unwrap();
    assert_eq!(values.values().unwrap(), Vec::<String>::new());
}

#[test]
fn live_len_requeries_after_each_mutation() {
    let mut store = Store::new();
    let mut values = live_mapped(
        &mut store,
        iri("alice"),
        iri("tag"),
        decode_string,
        encode_string,
    );

    assert_eq!(values.len(), 0);
    values.insert(&"one".to_owned()).unwrap();
    assert_eq!(values.len(), 1);
    values.insert(&"two".to_owned()).unwrap();
    assert_eq!(values.len(), 2);
}

#[test]
fn live_is_empty_requeries_after_mutation() {
    let mut store = Store::new();
    let mut values = live_mapped(
        &mut store,
        iri("alice"),
        iri("tag"),
        decode_string,
        encode_string,
    );

    assert!(values.is_empty());
    values.insert(&"present".to_owned()).unwrap();
    assert!(!values.is_empty());
}

#[test]
fn live_contains_maps_to_rdf_term_identity() {
    let mut store = Store::new();
    let mut values = live_mapped(
        &mut store,
        iri("alice"),
        iri("tag"),
        decode_string,
        encode_string,
    );

    values.insert(&"present".to_owned()).unwrap();
    assert!(values.contains(&"present".to_owned()).unwrap());
    assert!(!values.contains(&"absent".to_owned()).unwrap());
}

#[test]
fn live_insert_reports_effective_change_and_conversion_is_atomic() {
    let mut store = Store::new();
    let mut values = live_mapped(
        &mut store,
        iri("alice"),
        iri("tag"),
        decode_string,
        encode_string,
    );

    assert!(values.insert(&"one".to_owned()).unwrap());
    assert!(!values.insert(&"one".to_owned()).unwrap());
    assert_eq!(
        values.insert(&"reject".to_owned()),
        Err(CollectionError::Conversion("rejected by test codec"))
    );
    assert_eq!(values.values().unwrap(), vec!["one"]);
}

#[test]
fn live_remove_reports_effective_change_and_conversion_is_atomic() {
    let mut store = Store::new();
    let mut values = live_mapped(
        &mut store,
        iri("alice"),
        iri("tag"),
        decode_string,
        encode_string,
    );
    values.insert(&"one".to_owned()).unwrap();

    assert!(!values.remove(&"absent".to_owned()).unwrap());
    assert_eq!(
        values.remove(&"reject".to_owned()),
        Err(CollectionError::Conversion("rejected by test codec"))
    );
    assert_eq!(values.values().unwrap(), vec!["one"]);
    assert!(values.remove(&"one".to_owned()).unwrap());
    assert!(values.values().unwrap().is_empty());
}

#[test]
fn live_clear_removes_every_current_term() {
    let mut store = Store::new();
    let mut values = live_mapped(
        &mut store,
        iri("alice"),
        iri("tag"),
        decode_string,
        encode_string,
    );
    values.insert(&"one".to_owned()).unwrap();
    values.insert(&"two".to_owned()).unwrap();

    assert_eq!(values.clear().unwrap(), 2);
    assert!(values.values().unwrap().is_empty());
    assert_eq!(values.clear().unwrap(), 0);
}
