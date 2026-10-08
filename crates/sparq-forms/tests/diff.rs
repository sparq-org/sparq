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

/// Codex review of #6707: `oxrdf::Term::from_str` rejected blank-node labels
/// with internal dots, which RDF 1.2 allows, so a valid value suppressed the
/// whole update.
#[test]
fn triple_terms_accept_full_rdf12_grammar() {
    let ok = update_with_greeting(term("bnode", "a..b", None));
    assert!(ok.contains("_:a..b ."), "{ok}");
    update(&graph(DATA), &ok).unwrap();
    for (text, expected) in [
        (
            "<<( <http://a> <http://b> _:a..b )>>",
            "<<( <http://a> <http://b> _:a..b )>>",
        ),
        (
            "<<(_:a.b <http://b> <<( <http://c> <http://d> \"e\\\"f\"@en-GB )>>)>>",
            "<<( _:a.b <http://b> <<( <http://c> <http://d> \"e\\\"f\"@en-gb )>> )>>",
        ),
        (
            "<<( <http://a> <http://b> \"1\"^^<http://www.w3.org/2001/XMLSchema#integer> )>>",
            "<<( <http://a> <http://b> \"1\"^^<http://www.w3.org/2001/XMLSchema#integer> )>>",
        ),
        (
            "<<( <http://a> <http://b> \"x\"@ar--rtl )>>",
            "<<( <http://a> <http://b> \"x\"@ar--rtl )>>",
        ),
    ] {
        let ok = update_with_greeting(term("triple", text, None));
        assert!(ok.contains(&format!("{expected} .")), "{text}: {ok}");
        update(&graph(DATA), &ok).unwrap();
    }
}

/// Codex round 2 on #6707: nesting is capped like sparq-core's N-Triples
/// parser (128 levels), so deep or unfinished input fails closed instead of
/// overflowing the stack.
#[test]
fn triple_term_nesting_is_capped() {
    fn nested(depth: usize) -> String {
        format!(
            "{}<http://c>{}",
            "<<( <http://a> <http://b> ".repeat(depth),
            " )>>".repeat(depth)
        )
    }
    assert_ne!(update_with_greeting(term("triple", &nested(128), None)), "");
    assert_eq!(update_with_greeting(term("triple", &nested(129), None)), "");
    assert_eq!(
        update_with_greeting(term("triple", &nested(100_000), None)),
        ""
    );
    let prefixes = "<<( <http://a> <http://b> ".repeat(1_000_000);
    assert_eq!(update_with_greeting(term("triple", &prefixes, None)), "");
}

/// Codex round 2 on #6707: labels follow `BLANK_NODE_LABEL` (no `:`, which
/// oxrdf accepts but SPARQL rejects) and short strings reject raw line breaks.
#[test]
fn nt_lexical_rules_are_enforced() {
    for bad in ["a:b", "a.", ".a", "-a", "a b", "\u{B7}a"] {
        let update_text = update_with_greeting(term("bnode", bad, None));
        assert_eq!(update_text, "", "_:{bad} must not render: {update_text}");
        let nested = format!("<<( <http://a> <http://b> _:{bad} )>>");
        let update_text = update_with_greeting(term("triple", &nested, None));
        assert_eq!(update_text, "", "{nested} must not render: {update_text}");
    }
    for good in ["a..b", "0a", "_x", "a-b\u{B7}c", "\u{e9}t\u{e9}"] {
        let ok = update_with_greeting(term("bnode", good, None));
        assert!(ok.contains(&format!("_:{good} .")), "{ok}");
        update(&graph(DATA), &ok).unwrap();
        let ok = update_with_greeting(term(
            "triple",
            &format!("<<( _:{good} <http://b> <http://c> )>>"),
            None,
        ));
        assert_ne!(ok, "", "_:{good} in a triple term must render");
        update(&graph(DATA), &ok).unwrap();
    }
    for bad in [
        "<<( <http://a> <http://b> \"x\ny\" )>>",
        "<<( <http://a> <http://b> \"x\ry\" )>>",
    ] {
        assert_eq!(
            update_with_greeting(term("triple", bad, None)),
            "",
            "{bad:?}"
        );
    }
    let ok = update_with_greeting(term(
        "triple",
        "<<( <http://a> <http://b> \"x\\ny\" )>>",
        None,
    ));
    assert_ne!(ok, "", "escaped line break must still render");
    update(&graph(DATA), &ok).unwrap();
}

/// Codex round 3 on #6707: N-Triples allows space/tab between any two
/// tokens, including between a string and its `@lang` / `^^` suffix and after
/// `^^`; any other whitespace is not N-Triples whitespace.
#[test]
fn triple_term_whitespace_follows_ntriples() {
    const DT: &str = "<http://www.w3.org/2001/XMLSchema#integer>";
    for text in [
        format!("<<( <http://a> <http://b> \"1\"^^ {DT} )>>"),
        format!("<<( <http://a> <http://b> \"1\" ^^{DT} )>>"),
        format!("<<( <http://a> <http://b> \"1\"\t^^\t \t{DT} )>>"),
        "<<( <http://a> <http://b> \"c\" @en )>>".to_string(),
        "<<( <http://a> <http://b> \"c\"\t@en--ltr )>>".to_string(),
        "\t <<(\t<http://a>\t<http://b>\t<<(_:x <http://b> \"c\")>>\t)>> \t".to_string(),
        "<<(<http://a><http://b><http://c>)>>".to_string(),
    ] {
        let ok = update_with_greeting(term("triple", &text, None));
        assert_ne!(ok, "", "{text:?} must render");
        update(&graph(DATA), &ok).unwrap();
    }
    for bad in [
        "<<( <http://a> <http://b> \"c\"\n@en )>>",
        "<<( <http://a> <http://b> \"c\" @ en )>>",
        "<<( <http://a> <http://b> \"1\"^^\r<http://d> )>>",
        "<<( <http://a> <http://b> \"1\" ^ ^<http://d> )>>",
        "<<(\u{c}<http://a> <http://b> <http://c> )>>",
        "<<( <http://a>\u{a0}<http://b> <http://c> )>>",
        "<<( <http://a> <http://b> <http://c> )>>\n",
        "< <( <http://a> <http://b> <http://c> )>>",
    ] {
        let update_text = update_with_greeting(term("triple", bad, None));
        assert_eq!(update_text, "", "{bad:?} must not render: {update_text}");
    }
}

/// Codex round 3 on #6707: a malformed predicate path must fail the whole
/// update rather than being dropped while another field's edit is saved.
#[test]
fn malformed_path_with_valid_edit_fails_closed() {
    let data = graph(DATA);
    let shapes = graph(SHAPES);
    let before = derive_form(&data, &shapes, &focus(), &FormOptions::default());
    let edit = |bad_path: Option<&str>| {
        let mut after = before.clone();
        for field in after.groups.iter_mut().flat_map(|group| &mut group.fields) {
            if field.path == "<http://example.org/name>" {
                field.values = vec![FormValue {
                    term: term("literal", "edited", None),
                    nested: None,
                }];
            } else if field.path == "<http://example.org/greeting>" {
                if let Some(bad_path) = bad_path {
                    field.path = bad_path.into();
                }
            }
        }
        to_sparql_update(&before, &after)
    };
    let ok = edit(None);
    assert!(ok.contains("\"edited\""), "valid edit alone renders: {ok}");
    for bad_path in [
        "<http://example.org/greet ing>",
        "<relative>",
        "<http://example.org/x\\u003E>",
    ] {
        let update_text = edit(Some(bad_path));
        assert_eq!(
            update_text, "",
            "{bad_path} must fail closed: {update_text}"
        );
    }
}

#[test]
fn malformed_triple_terms_fail_closed() {
    for bad in [
        "<<( <http://a> <http://b> _:a. )>>",
        "<<( <http://a> <http://b> _: )>>",
        "<<( <http://a\\u003E . } ; DROP ALL ; INSERT { <x> <http://b> <http://c> )>>",
        "<<( <http://a> <http://b> \"c\" )>> . } ; DROP ALL",
        "<<( <http://a> <http://b> \"c\"@en . } ; DROP ALL )>>",
        "<<( <http://a> <http://b> \"c\"@en--up )>>",
        "<<( <http://a> <http://b> \"c\\q\" )>>",
        "<<( <http://a> <http://b> \"c\"^^\"d\" )>>",
        "<<( <http://a> <http://b> \"unterminated )>>",
        "<<( \"c\" <http://b> <http://c> )>>",
        "<<( <<( <http://a> <http://b> <http://c> )>> <http://b> <http://c> )>>",
        "<<( <http://a> _:p <http://c> )>>",
        "<<( <http://a> <http://b> <http://c>",
    ] {
        let update_text = update_with_greeting(term("triple", bad, None));
        assert_eq!(update_text, "", "{bad} must not render: {update_text}");
    }
}

/// SPARQL decodes `\uXXXX` escapes before parsing, so escaping an
/// IRIREF-forbidden character (`>` as `\u003E`) still ends the IRI. IRIs must
/// be validated, and an invalid one fails closed (empty update).
#[test]
fn invalid_iris_cannot_inject_update_syntax() {
    let iri_payload =
        "http://example.org/x> . } ; DROP ALL ; INSERT { <http://a> <http://b> <http://c";
    let mut bad_datatype = term("literal", "1", None);
    bad_datatype.datatype = Some(iri_payload.into());
    for bad in [
        term("iri", iri_payload, None),
        term("iri", "http://example.org/a b", None),
        term("iri", "http://example.org/\"{}|^`\\", None),
        term("iri", "not an absolute iri", None),
        bad_datatype,
    ] {
        let update_text = update_with_greeting(bad.clone());
        assert_eq!(update_text, "", "{bad:?} must not render: {update_text}");
    }

    // A deserialized field path is spliced as `<...>` too: escapes in it must
    // not survive into the request either.
    let data = graph(DATA);
    let shapes = graph(SHAPES);
    let before = derive_form(&data, &shapes, &focus(), &FormOptions::default());
    let mut after = before.clone();
    let bad_path = concat!(
        r"<http://example.org/greeting> . } ; DROP ALL",
        r" ; INSERT { <http://a> <http://b>"
    );
    let field = after
        .groups
        .iter_mut()
        .flat_map(|group| &mut group.fields)
        .find(|field| field.path == "<http://example.org/greeting>")
        .unwrap();
    field.path = bad_path.into();
    field.values = vec![FormValue {
        term: term("literal", "hi", None),
        nested: None,
    }];
    let update_text = to_sparql_update(&before, &after);
    assert_eq!(
        update_text, "",
        "invalid path must not render: {update_text}"
    );
}

#[test]
fn valid_iri_terms_still_render() {
    let ok = update_with_greeting(term("iri", "http://example.org/caf\u{e9}?q=1#f", None));
    assert!(
        ok.contains("<http://example.org/caf\u{e9}?q=1#f> ."),
        "{ok}"
    );
    update(&graph(DATA), &ok).unwrap();

    // A literal carrying escape-shaped text stays one literal end to end.
    let tricky = r#"x\u0022 . } ; DROP ALL ; INSERT { <http://a> <http://b> \u0022c"#;
    let ok = update_with_greeting(term("literal", tricky, None));
    let updated = update(&graph(DATA), &ok).unwrap();
    let derived = derive_form(&updated, &graph(SHAPES), &focus(), &FormOptions::default());
    let greeting = derived
        .groups
        .iter()
        .flat_map(|group| &group.fields)
        .find(|field| field.path == "<http://example.org/greeting>")
        .unwrap();
    assert_eq!(greeting.values.len(), 1);
    assert_eq!(greeting.values[0].term.value, tricky);
    let name = derived
        .groups
        .iter()
        .flat_map(|group| &group.fields)
        .find(|field| field.path == "<http://example.org/name>")
        .unwrap();
    assert_eq!(name.values.len(), 1, "existing data must survive");
}
