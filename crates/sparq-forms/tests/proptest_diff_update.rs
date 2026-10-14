//! Property tests for the form-diff -> SPARQL Update round trip. sq-ly6xq
//!
//! Mutation witness: making `to_sparql_update` omit either its DELETE or INSERT
//! triples makes `random_edits_roundtrip_without_collateral_mutation` fail.

use std::collections::BTreeSet;

use oxrdf::{Literal, NamedNode, Term};
use proptest::prelude::*;
use proptest::test_runner::RngSeed;
use sparq_core::Graph;
use sparq_engine::update;
use sparq_forms::{
    derive_form, to_sparql_update, FormDescription, FormDiff, FormOptions, FormValue, TermRef,
};

const PREFIXES: &str = r#"
  @prefix sh: <http://www.w3.org/ns/shacl#> .
  @prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
  @prefix ex: <http://example.org/> .
"#;

const SHAPES: &str = r#"
  ex:PersonShape a sh:NodeShape ; sh:targetClass ex:Person ;
    sh:property [ sh:path ex:name ; sh:name "Name" ; sh:datatype xsd:string ] ;
    sh:property [ sh:path ex:age ; sh:name "Age" ; sh:datatype xsd:integer ] ;
    sh:property [ sh:path ex:homepage ; sh:name "Homepage" ; sh:nodeKind sh:IRI ] .
"#;

const NAME: &str = "<http://example.org/name>";
const AGE: &str = "<http://example.org/age>";
const HOMEPAGE: &str = "<http://example.org/homepage>";

#[derive(Clone, Debug)]
enum Operation {
    Add(u8, u8),
    Remove(u8),
    Change(u8, u8),
}

fn operations() -> impl Strategy<Value = Vec<Operation>> {
    prop::collection::vec(
        prop_oneof![
            (0u8..3, any::<u8>()).prop_map(|(field, value)| Operation::Add(field, value)),
            (0u8..3).prop_map(Operation::Remove),
            (0u8..3, any::<u8>()).prop_map(|(field, value)| Operation::Change(field, value)),
        ],
        1..10,
    )
}

fn graph(ttl: &str) -> Graph {
    Graph::load_str(&format!("{PREFIXES}{ttl}"), "turtle").unwrap()
}

fn focus() -> Term {
    Term::from(NamedNode::new_unchecked("http://example.org/alice"))
}

fn form_value(term: Term) -> FormValue {
    FormValue {
        term: TermRef::from_term(&term),
        nested: None,
    }
}

fn generated_value(field: u8, seed: u8) -> FormValue {
    match field % 3 {
        0 => form_value(Term::from(Literal::new_simple_literal(format!(
            "name-{seed}"
        )))),
        1 => form_value(Term::from(Literal::new_typed_literal(
            i64::from(seed).to_string(),
            NamedNode::new_unchecked("http://www.w3.org/2001/XMLSchema#integer"),
        ))),
        _ => form_value(Term::from(NamedNode::new_unchecked(format!(
            "http://example.org/page/{seed}"
        )))),
    }
}

fn path(field: u8) -> &'static str {
    [NAME, AGE, HOMEPAGE][usize::from(field % 3)]
}

fn apply_operations(form: &mut FormDescription, operations: &[Operation]) -> BTreeSet<String> {
    let mut edited = BTreeSet::new();
    for operation in operations {
        let (field, replacement) = match operation {
            Operation::Add(field, seed) => (*field, Some((false, *seed))),
            Operation::Remove(field) => (*field, None),
            Operation::Change(field, seed) => (*field, Some((true, *seed))),
        };
        let field_path = path(field);
        let target = form
            .groups
            .iter_mut()
            .flat_map(|group| &mut group.fields)
            .find(|candidate| candidate.path == field_path)
            .unwrap();
        match replacement {
            None => {
                target.values.pop();
            }
            Some((true, seed)) => target.values = vec![generated_value(field, seed)],
            Some((false, seed)) => {
                let value = generated_value(field, seed);
                if !target.values.iter().any(|old| old.term == value.term) {
                    target.values.push(value);
                }
            }
        }
        edited.insert(field_path.to_string());
    }
    edited
}

fn field_values(form: &FormDescription, path: &str) -> Vec<TermRef> {
    let mut values = form
        .groups
        .iter()
        .flat_map(|group| &group.fields)
        .find(|field| field.path == path)
        .unwrap()
        .values
        .iter()
        .map(|value| value.term.clone())
        .collect::<Vec<_>>();
    values.sort_by(|left, right| format!("{left:?}").cmp(&format!("{right:?}")));
    values
}

fn triples(graph: &Graph) -> BTreeSet<(String, String, String)> {
    graph
        .iter_ids()
        .map(|[subject, predicate, object]| {
            (
                graph.dict.term(subject).to_string(),
                graph.dict.term(predicate).to_string(),
                graph.dict.term(object).to_string(),
            )
        })
        .collect()
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 128,
        rng_seed: RngSeed::Fixed(0x5a17_d1ff),
        ..Default::default()
    })]

    #[test]
    fn random_edits_roundtrip_without_collateral_mutation(
        initial_name in "[a-z]{1,10}",
        initial_age in 0u16..130,
        initial_page in 0u8..20,
        operations in operations(),
    ) {
        let data = graph(&format!(r#"
          ex:alice a ex:Person ; ex:name "{initial_name}" ; ex:age {initial_age} ;
            ex:homepage <http://example.org/page/{initial_page}> ; ex:note "read only" .
          ex:bob ex:knows ex:alice .
        "#));
        let shapes = graph(SHAPES);
        let before = derive_form(&data, &shapes, &focus(), &FormOptions::default());
        let mut after = before.clone();
        let edited = apply_operations(&mut after, &operations);

        let updated = update(&data, &to_sparql_update(&before, &after)).unwrap();
        let derived = derive_form(&updated, &shapes, &focus(), &FormOptions::default());
        for path in &edited {
            prop_assert_eq!(field_values(&derived, path), field_values(&after, path));
        }

        let before_triples = triples(&data);
        let after_triples = triples(&updated);
        for triple in before_triples.symmetric_difference(&after_triples) {
            prop_assert_eq!(triple.0.as_str(), "<http://example.org/alice>");
            prop_assert!(edited.contains(&triple.1));
        }
        prop_assert!(after_triples.contains(&(
            "<http://example.org/alice>".into(),
            "<http://example.org/note>".into(),
            "\"read only\"".into(),
        )));
    }
}

#[test]
fn identity_diff_is_an_applied_byte_level_noop() {
    let data = graph(
        r#"
      ex:alice a ex:Person ; ex:name "Alice" ; ex:age 41 ;
        ex:homepage <http://example.org/page/alice> ; ex:note "read only" .
    "#,
    );
    let shapes = graph(SHAPES);
    let form = derive_form(&data, &shapes, &focus(), &FormOptions::default());
    let diff = FormDiff::between(&form, &form);
    assert!(diff.added.is_empty());
    assert!(diff.removed.is_empty());

    let update_text = to_sparql_update(&form, &form);
    assert!(update_text.is_empty());
    let updated = update(&data, &update_text).unwrap();
    assert_eq!(triples(&updated), triples(&data));
}
