//! Form edit → SPARQL Update integration tests. [GPT-5.6] sq-wn788

use oxrdf::{Literal, NamedNode, Term};
use sparq_core::Graph;
use sparq_engine::update;
use sparq_forms::{derive_form, to_sparql_update, FormOptions, FormValue, TermRef};

const PREFIXES: &str = r#"
  @prefix sh: <http://www.w3.org/ns/shacl#> .
  @prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
  @prefix ex: <http://example.org/> .
"#;

const SHAPES: &str = r#"
  ex:PersonShape a sh:NodeShape ; sh:targetClass ex:Person ;
    sh:property [ sh:path ex:name ; sh:name "Name" ; sh:datatype xsd:string ] ;
    sh:property [ sh:path ex:age ; sh:name "Age" ; sh:datatype xsd:integer ] ;
    sh:property [ sh:path ex:greeting ; sh:name "Greeting" ] ;
    sh:property [ sh:path [ sh:inversePath ex:knows ] ; sh:name "Known by" ] .
"#;

const DATA: &str = r#"
  ex:alice a ex:Person ; ex:name "Alice" ; ex:age 41 ; ex:note "read only" .
  ex:bob ex:knows ex:alice .
"#;

fn graph(ttl: &str) -> Graph {
    Graph::load_str(&format!("{PREFIXES}{ttl}"), "turtle").unwrap()
}

fn focus() -> Term {
    Term::from(NamedNode::new_unchecked("http://example.org/alice"))
}

fn value(term: Term) -> FormValue {
    FormValue {
        term: TermRef::from_term(&term),
        nested: None,
    }
}

#[test]
fn diff_add_remove_single_predicate_roundtrips() {
    let data = graph(DATA);
    let shapes = graph(SHAPES);
    let before = derive_form(&data, &shapes, &focus(), &FormOptions::default());
    let mut after = before.clone();

    let name = after
        .groups
        .iter_mut()
        .flat_map(|group| &mut group.fields)
        .find(|field| field.path == "<http://example.org/name>")
        .unwrap();
    name.values = vec![
        value(Term::from(Literal::new_simple_literal("Alicia \"Ace\""))),
        value(Term::from(
            Literal::new_language_tagged_literal("Ally", "en").unwrap(),
        )),
    ];

    let update_text = to_sparql_update(&before, &after);
    assert!(update_text.contains("DELETE {"));
    assert!(update_text.contains("INSERT {"));
    assert!(update_text.contains("\\\"Ace\\\""));
    assert!(update_text.contains("\"Ally\"@en"));

    let updated = update(&data, &update_text).unwrap();
    let derived = derive_form(&updated, &shapes, &focus(), &FormOptions::default());
    let expected = after
        .groups
        .iter()
        .flat_map(|group| &group.fields)
        .filter(|field| field.editable && !field.inverse)
        .map(|field| (&field.path, &field.values))
        .collect::<Vec<_>>();
    let actual = derived
        .groups
        .iter()
        .flat_map(|group| &group.fields)
        .filter(|field| field.editable && !field.inverse)
        .map(|field| (&field.path, &field.values))
        .collect::<Vec<_>>();
    assert_eq!(actual, expected);
}

#[test]
fn diff_noop_when_before_equals_after() {
    let data = graph(DATA);
    let shapes = graph(SHAPES);
    let form = derive_form(&data, &shapes, &focus(), &FormOptions::default());
    assert_eq!(to_sparql_update(&form, &form), "");
}

#[test]
fn diff_excludes_readonly_and_inverse_fields() {
    let data = graph(DATA);
    let shapes = graph(SHAPES);
    let before = derive_form(&data, &shapes, &focus(), &FormOptions::default());
    let mut after = before.clone();

    for field in after.groups.iter_mut().flat_map(|group| &mut group.fields) {
        if !field.editable || field.inverse {
            field.values.clear();
        }
        // A future computed field is represented as non-editable; exercise that
        // contract explicitly on an otherwise ordinary forward property field.
        if field.path == "<http://example.org/greeting>" {
            field.editable = false;
            field
                .values
                .push(value(Term::from(Literal::new_simple_literal("Hello"))));
        }
    }

    assert_eq!(to_sparql_update(&before, &after), "");
}

/// Replaces the `ex:greeting` values of a freshly derived form with `term`
/// (a renderer round-trip can hand back any deserialized `TermRef`).
fn update_with_greeting(term: TermRef) -> String {
    let data = graph(DATA);
    let shapes = graph(SHAPES);
    let before = derive_form(&data, &shapes, &focus(), &FormOptions::default());
    let mut after = before.clone();
    let greeting = after
        .groups
        .iter_mut()
        .flat_map(|group| &mut group.fields)
        .find(|field| field.path == "<http://example.org/greeting>")
        .unwrap();
    greeting.values = vec![FormValue { term, nested: None }];
    to_sparql_update(&before, &after)
}

fn term(kind: &str, value: &str, language: Option<&str>) -> TermRef {
    TermRef {
        kind: kind.into(),
        value: value.into(),
        datatype: None,
        language: language.map(str::to_string),
    }
}

const PAYLOAD: &str = "x . } ; DROP ALL ; INSERT { <a> <b> \"c\"";

/// #5651: blank-node labels, language tags and triple-term text have no
/// escape form in the update template, so an invalid one must fail closed
/// (empty update) instead of being spliced in verbatim.
#[test]
fn deserialized_terms_cannot_inject_update_syntax() {
    for bad in [
        term("bnode", PAYLOAD, None),
        term("bnode", "", None),
        term(
            "literal",
            "hi",
            Some("en . } ; DROP ALL ; INSERT { <a> <b> <c>"),
        ),
        term("triple", PAYLOAD, None),
        term("triple", "<http://example.org/x>", None),
        term(
            "triple",
            "<<( <http://a> <http://b> <http://c> )>> . } ; DROP ALL",
            None,
        ),
        term("unknown-kind", "<http://example.org/x>", None),
    ] {
        let update_text = update_with_greeting(bad.clone());
        assert_eq!(update_text, "", "{bad:?} must not render: {update_text}");
    }
}

#[test]
fn valid_bnode_lang_and_triple_terms_still_render() {
    let ok = update_with_greeting(term("bnode", "b0", None));
    assert!(ok.contains("_:b0 ."), "{ok}");
    let ok = update_with_greeting(term("literal", "hi", Some("en-GB")));
    assert!(ok.contains("\"hi\"@en-GB ."), "{ok}");
    let ok = update_with_greeting(term("triple", "<<( <http://a> <http://b> \"c\" )>>", None));
    assert!(ok.contains("<<( <http://a> <http://b> \"c\" )>> ."), "{ok}");
    update(&graph(DATA), &ok).unwrap();
}
