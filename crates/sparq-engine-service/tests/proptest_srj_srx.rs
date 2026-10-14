//! (sq-zvavi) Value-level differential tests for remote result codecs.
#![cfg(feature = "service")]

use oxrdf::{BlankNode, Literal, NamedNode, Term, Variable};
use proptest::prelude::*;
use serde_json::{json, Map, Value};

// Compile the crate-private codec module into this integration-test crate. This keeps the
// production surface private while giving the test access to its collected seams.
// (sq-zvavi)
#[path = "../src/service.rs"]
#[allow(dead_code)]
mod service;

use service::{parse_srj_into, parse_srx_into};

/// A fully-collected remote SELECT result (`None` = unbound).
#[derive(Debug, PartialEq)]
struct ServiceRelation {
    vars: Vec<Variable>,
    rows: Vec<Vec<Option<Term>>>,
}

fn parse_srj(text: &str) -> Result<ServiceRelation, String> {
    let mut rows = Vec::new();
    let vars = parse_srj_into(text, &mut |row| {
        rows.push(row);
        Ok(())
    })?;
    Ok(ServiceRelation { vars, rows })
}

fn parse_srx(text: &str) -> Result<ServiceRelation, String> {
    let mut rows = Vec::new();
    let vars = parse_srx_into(text, &mut |row| {
        rows.push(row);
        Ok(())
    })?;
    Ok(ServiceRelation { vars, rows })
}

fn variable_strategy() -> impl Strategy<Value = Vec<Variable>> {
    (1_usize..=4).prop_map(|len| {
        (0..len)
            .map(|index| Variable::new_unchecked(format!("v{index}")))
            .collect()
    })
}

fn term_strategy() -> impl Strategy<Value = Term> {
    prop_oneof![
        (0_u16..1000)
            .prop_map(|n| NamedNode::new_unchecked(format!("https://example.test/i{n}")).into()),
        (0_u16..1000).prop_map(|n| BlankNode::new_unchecked(format!("b{n}")).into()),
        "[a-zA-Z0-9 ]{0,20}".prop_map(|text| Literal::new_simple_literal(text).into()),
        (any::<i32>()).prop_map(|n| {
            Literal::new_typed_literal(
                n.to_string(),
                NamedNode::new_unchecked("http://www.w3.org/2001/XMLSchema#integer"),
            )
            .into()
        }),
        "[a-z]{0,12}"
            .prop_map(|text| Literal::new_language_tagged_literal_unchecked(text, "en").into()),
    ]
}

fn relation_strategy() -> impl Strategy<Value = ServiceRelation> {
    variable_strategy().prop_flat_map(|vars| {
        let width = vars.len();
        proptest::collection::vec(
            proptest::collection::vec(prop::option::of(term_strategy()), width),
            0..12,
        )
        .prop_map(move |rows| ServiceRelation {
            vars: vars.clone(),
            rows,
        })
    })
}

fn srj_term(term: &Term) -> Value {
    match term {
        Term::NamedNode(node) => json!({"type": "uri", "value": node.as_str()}),
        Term::BlankNode(node) => json!({"type": "bnode", "value": node.as_str()}),
        Term::Literal(literal) => {
            let mut value = Map::from_iter([
                ("type".into(), Value::String("literal".into())),
                ("value".into(), Value::String(literal.value().into())),
            ]);
            if let Some(language) = literal.language() {
                value.insert("xml:lang".into(), Value::String(language.into()));
            } else {
                value.insert(
                    "datatype".into(),
                    Value::String(literal.datatype().as_str().into()),
                );
            }
            Value::Object(value)
        }
        Term::Triple(_) => unreachable!("the generator emits ground non-triple terms"),
    }
}

fn render_srj(relation: &ServiceRelation) -> String {
    let bindings: Vec<Value> = relation
        .rows
        .iter()
        .map(|row| {
            let entries = relation
                .vars
                .iter()
                .zip(row)
                .filter_map(|(var, term)| {
                    term.as_ref()
                        .map(|term| (var.as_str().into(), srj_term(term)))
                })
                .collect();
            Value::Object(entries)
        })
        .collect();
    json!({
        "head": {"vars": relation.vars.iter().map(Variable::as_str).collect::<Vec<_>>()},
        "results": {"bindings": bindings}
    })
    .to_string()
}

fn xml_escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn srx_term(term: &Term) -> String {
    match term {
        Term::NamedNode(node) => format!("<uri>{}</uri>", xml_escape(node.as_str())),
        Term::BlankNode(node) => format!("<bnode>{}</bnode>", xml_escape(node.as_str())),
        Term::Literal(literal) => {
            let attribute = if let Some(language) = literal.language() {
                format!(" xml:lang=\"{}\"", xml_escape(language))
            } else {
                format!(" datatype=\"{}\"", xml_escape(literal.datatype().as_str()))
            };
            format!(
                "<literal{attribute}>{}</literal>",
                xml_escape(literal.value())
            )
        }
        Term::Triple(_) => unreachable!("the generator emits ground non-triple terms"),
    }
}

fn render_srx(relation: &ServiceRelation) -> String {
    let mut xml = String::from(
        "<?xml version=\"1.0\"?><sparql xmlns=\"http://www.w3.org/2005/sparql-results#\"><head>",
    );
    for var in &relation.vars {
        xml.push_str(&format!(
            "<variable name=\"{}\"/>",
            xml_escape(var.as_str())
        ));
    }
    xml.push_str("</head><results>");
    for row in &relation.rows {
        xml.push_str("<result>");
        for (var, term) in relation.vars.iter().zip(row) {
            if let Some(term) = term {
                xml.push_str(&format!(
                    "<binding name=\"{}\">{}</binding>",
                    xml_escape(var.as_str()),
                    srx_term(term)
                ));
            }
        }
        xml.push_str("</result>");
    }
    xml.push_str("</results></sparql>");
    xml
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    #[test]
    fn srj_srx_parse_render_value_agreement(relation in relation_strategy()) {
        let from_srj = parse_srj(&render_srj(&relation)).expect("generated SRJ must parse");
        let from_srx = parse_srx(&render_srx(&relation)).expect("generated SRX must parse");

        prop_assert_eq!(&from_srj, &relation);
        prop_assert_eq!(&from_srx, &relation);
        prop_assert_eq!(from_srj, from_srx);
    }
}
