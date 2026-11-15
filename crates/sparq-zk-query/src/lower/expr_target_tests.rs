//! Expression-level acceptance cases. Aggregate and absence inputs come from lane C.
use super::expr::{Gadget, LaneType, from_spargebra_with_boolean_inputs, lower_bind};
use super::expr_semantics::{self as mirror, Error};
use oxrdf::Term;
use spargebra::algebra::{Expression, GraphPattern};
use std::collections::BTreeMap;

fn projected(pattern: &GraphPattern) -> Expression {
    match pattern {
        GraphPattern::Extend { expression, .. } => expression.clone(),
        GraphPattern::Project { inner, .. } => projected(inner),
        _ => panic!("projection fixture"),
    }
}
fn exists(expr: &Expression, out: &mut Vec<(GraphPattern, u32)>) {
    match expr {
        Expression::Exists(p) => out.push((p.as_ref().clone(), 99)),
        Expression::And(a, b) => {
            exists(a, out);
            exists(b, out);
        }
        Expression::Not(a) => exists(a, out),
        _ => {}
    }
}
fn check(text: &str, bindings: &[(&str, Option<Term>)], absent: bool, expected: Option<bool>) {
    let mut slots = BTreeMap::new();
    let mut vars = BTreeMap::new();
    for (i, (name, value)) in bindings.iter().enumerate() {
        let slot = u32::try_from(i).unwrap();
        slots.insert((*name).into(), (slot, LaneType::Dynamic));
        if let Some(value) = value {
            vars.insert(slot, value.clone());
        }
    }
    vars.insert(99, mirror::bool_term(!absent));
    let values = format!(
        "VALUES ({}) {{ ({}) }}",
        bindings
            .iter()
            .map(|(n, _)| format!("?{n}"))
            .collect::<Vec<_>>()
            .join(" "),
        bindings
            .iter()
            .map(|(_, v)| v
                .as_ref()
                .map_or_else(|| "UNDEF".into(), ToString::to_string))
            .collect::<Vec<_>>()
            .join(" ")
    );
    let query = format!(
        "PREFIX xsd:<http://www.w3.org/2001/XMLSchema#> SELECT ({text} AS ?result) {{ {values} }}"
    );
    let spargebra::Query::Select { pattern, .. } =
        spargebra::SparqlParser::new().parse_query(&query).unwrap()
    else {
        panic!("SELECT");
    };
    let ast = projected(&pattern);
    let mut booleans = Vec::new();
    exists(&ast, &mut booleans);
    let ir = from_spargebra_with_boolean_inputs(&ast, &slots, &booleans).unwrap();
    let dag = lower_bind(&ir).unwrap();
    assert_eq!(dag.nodes[dag.root].gadget, Gadget::ToTerm);
    assert!(
        dag.nodes
            .iter()
            .enumerate()
            .all(|(i, n)| n.inputs.iter().all(|&j| j < i))
    );
    assert_eq!(
        dag.constants.len(),
        dag.nodes
            .iter()
            .filter(|n| matches!(n.gadget, Gadget::PublicInput(_)))
            .count()
    );
    let value = match mirror::eval(&ir, &vars) {
        Ok(v) => Some(v),
        Err(Error::Sparql) => None,
        Err(Error::Capacity) => panic!("target outside capacity"),
    };
    assert_eq!(
        value,
        expected.map(mirror::bool_term),
        "{text}, {bindings:?}, absence={absent}"
    );
    let data = if absent {
        ""
    } else {
        "<urn:s> <urn:excluded> <urn:o> ."
    };
    let graph = sparq_core::Graph::load_str(data, "n-triples").unwrap();
    let rows = sparq_engine::query(&graph, &query).unwrap().rows;
    assert_eq!(rows, vec![vec![value]], "{query}");
}
fn n(value: i64) -> Option<Term> {
    Some(mirror::typed(value.to_string(), "integer"))
}
fn d(value: &str) -> Option<Term> {
    Some(mirror::typed(value, "date"))
}
fn string(value: &str) -> Option<Term> {
    Some(oxrdf::Literal::new_simple_literal(value).into())
}
const ABSENT: &str = "NOT EXISTS { ?s <urn:excluded> ?o }";
#[test]
fn target_1a_insurance_stated_lowering_mirror_engine() {
    let text = "?v0 >= 5 && ?v1 <= 3 && ?v2 = 0 && ?v3 = true";
    let base = [
        ("v0", n(12)),
        ("v1", n(0)),
        ("v2", n(0)),
        ("v3", Some(mirror::bool_term(true))),
    ];
    check(text, &base, true, Some(true));
    for (slot, value) in [
        (0, n(4)),
        (1, n(4)),
        (2, n(1)),
        (3, Some(mirror::bool_term(false))),
    ] {
        let mut values = base.clone();
        values[slot].1 = value;
        check(text, &values, true, Some(false));
    }
    for slot in 0..base.len() {
        let mut values = base.clone();
        values[slot].1 = None;
        check(text, &values, true, None);
    }
    let mut values = base;
    values[0].1 = n(1_i64 << 48);
    check(text, &values, true, Some(true));
}
#[test]
fn target_1b_insurance_closed_lowering_mirror_engine() {
    let text = format!(
        "?v0 >= 5 && ?v1 <= 3 && ?from <= \"2021-11-13\"^^xsd:date && ?to >= \"2026-11-13\"^^xsd:date && {ABSENT}"
    );
    let base = [
        ("v0", n(12)),
        ("v1", n(0)),
        ("from", d("2021-11-13")),
        ("to", d("2026-11-13")),
    ];
    check(&text, &base, true, Some(true));
    check(&text, &base, false, Some(false));
    for (slot, value) in [
        (0, n(4)),
        (1, n(4)),
        (2, d("2021-11-14")),
        (3, d("2026-11-12")),
    ] {
        let mut values = base.clone();
        values[slot].1 = value;
        check(&text, &values, true, Some(false));
    }
    for slot in 0..base.len() {
        let mut values = base.clone();
        values[slot].1 = None;
        check(&text, &values, true, None);
        check(&text, &values, false, Some(false));
    }
    check(
        "?d >= \"2021-11-13\"^^xsd:date",
        &[("d", d("2021-11-12"))],
        true,
        Some(false),
    );
    check(
        "?d >= \"2021-11-13\"^^xsd:date",
        &[("d", d("2021-11-13"))],
        true,
        Some(true),
    );
}
#[test]
fn target_2_trial_lowering_mirror_engine() {
    let text = format!(
        "?age >= 18 && ?age <= 65 && ?dx = \"condition-a\" && ?from <= \"2026-11-01\"^^xsd:date && ?to >= \"2026-11-13\"^^xsd:date && {ABSENT}"
    );
    let base = [
        ("age", n(45)),
        ("dx", string("condition-a")),
        ("from", d("2026-11-01")),
        ("to", d("2026-11-13")),
    ];
    check(&text, &base, true, Some(true));
    check(&text, &base, false, Some(false));
    for (slot, value) in [
        (0, n(17)),
        (0, n(66)),
        (1, string("condition-b")),
        (2, d("2026-11-02")),
        (3, d("2026-11-12")),
    ] {
        let mut values = base.clone();
        values[slot].1 = value;
        check(&text, &values, true, Some(false));
    }
    for slot in 0..base.len() {
        let mut values = base.clone();
        values[slot].1 = None;
        check(&text, &values, true, None);
    }
    for value in ["excluded-x", "excluded-y", "medicine-a"] {
        check(
            "?m IN (\"excluded-x\",\"excluded-y\")",
            &[("m", string(value))],
            true,
            Some(value != "medicine-a"),
        );
    }
    let list = (0..32)
        .map(|i| format!("\"entry-{i}\""))
        .collect::<Vec<_>>()
        .join(",");
    check(
        &format!("?m IN ({list})"),
        &[("m", string("entry-31"))],
        true,
        Some(true),
    );
}
#[test]
fn target_3_recycled_lowering_mirror_engine() {
    let text = "?value >= 0.3 && ?issuers = 3 && ?count = 3";
    let base = [
        ("value", Some(mirror::typed("0.35", "decimal"))),
        ("issuers", n(3)),
        ("count", n(3)),
    ];
    check(text, &base, true, Some(true));
    for (slot, value) in [
        (0, Some(mirror::typed("0.29", "decimal"))),
        (1, n(2)),
        (2, n(4)),
    ] {
        let mut values = base.clone();
        values[slot].1 = value;
        check(text, &values, true, Some(false));
    }
    for slot in 0..base.len() {
        let mut values = base.clone();
        values[slot].1 = None;
        check(text, &values, true, None);
    }
    let text = "?iss IN (<urn:fictional:recycled-content:tier>,<urn:fictional:recycled-content:tier-2>,<urn:fictional:recycled-content:tier-3>)";
    for (iri, expected) in [
        ("urn:fictional:recycled-content:tier", true),
        ("urn:other", false),
    ] {
        check(
            text,
            &[("iss", Some(oxrdf::NamedNode::new(iri).unwrap().into()))],
            true,
            Some(expected),
        );
    }
    check(text, &[("iss", None)], true, None);
}
#[test]
fn target_sum_having_lowering_mirror_engine() {
    let text = "?total >= 500 && ?n = 3";
    let base = [("total", n(500)), ("n", n(3))];
    check(text, &base, true, Some(true));
    for (slot, value) in [(0, n(499)), (1, n(2))] {
        let mut values = base.clone();
        values[slot].1 = value;
        check(text, &values, true, Some(false));
    }
    for slot in 0..base.len() {
        let mut values = base.clone();
        values[slot].1 = None;
        check(text, &values, true, None);
    }
    for (recycled, total, expected) in [
        (150, 500, true),
        (149, 500, false),
        (1_i64 << 48, 1_i64 << 48, true),
    ] {
        check(
            "?recycled * 10 >= ?total * 3",
            &[("recycled", n(recycled)), ("total", n(total))],
            true,
            Some(expected),
        );
    }
    check(
        "?recycled * 10 >= ?total * 3",
        &[("recycled", None), ("total", n(500))],
        true,
        None,
    );
}
