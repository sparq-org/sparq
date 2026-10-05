//! #3198 — a COMPUTED numeric ORDER BY key is ordered by its exact value, like a plain
//! numeric variable key, `<` and MIN/MAX. An `f64` key collapsed integers beyond 2^53
//! and high-precision decimals into ties that kept their input order.

use sparq_core::Graph;
use sparq_engine::query;

fn subjects(g: &Graph, q: &str) -> Vec<String> {
    query(g, q)
        .unwrap()
        .rows
        .iter()
        .map(|r| r[0].as_ref().map(|t| t.to_string()).unwrap_or_default())
        .collect()
}

fn graph(values: &[(&str, &str)]) -> Graph {
    let mut ttl = String::from("@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .\n");
    for (s, v) in values {
        ttl.push_str(&format!("<http://ex/{s}> <http://ex/v> {v} .\n"));
    }
    Graph::load_str(&ttl, "turtle").unwrap()
}

const EXPECT_ASC: [&str; 3] = ["<http://ex/b>", "<http://ex/a>", "<http://ex/c>"];

fn check(g: &Graph) {
    for key in ["?v", "(?v + 0)", "(?v * 1)", "(-(-?v))", "?k"] {
        let q = format!(
            "SELECT ?s WHERE {{ ?s <http://ex/v> ?v BIND(?v + 0 AS ?k) }} ORDER BY {key}"
        );
        assert_eq!(subjects(g, &q), EXPECT_ASC, "ascending by {key}");
        let q = format!(
            "SELECT ?s WHERE {{ ?s <http://ex/v> ?v BIND(?v + 0 AS ?k) }} ORDER BY DESC({key})"
        );
        let mut desc = EXPECT_ASC.to_vec();
        desc.reverse();
        assert_eq!(subjects(g, &q), desc, "descending by {key}");
        let q = format!(
            "SELECT ?s WHERE {{ ?s <http://ex/v> ?v BIND(?v + 0 AS ?k) }} ORDER BY {key} LIMIT 1"
        );
        assert_eq!(subjects(g, &q), EXPECT_ASC[..1], "top-1 by {key}");
    }
}

#[test]
fn high_precision_decimals_order_exactly_under_a_computed_key() {
    // All three share one f64. Input order a, b, c differs from value order b, a, c.
    check(&graph(&[
        ("a", "\"0.100000000000000000002\"^^xsd:decimal"),
        ("b", "\"0.100000000000000000001\"^^xsd:decimal"),
        ("c", "\"0.100000000000000000003\"^^xsd:decimal"),
    ]));
}

#[test]
fn integers_beyond_2_pow_53_order_exactly_under_a_computed_key() {
    check(&graph(&[("a", "9007199254740994"), ("b", "9007199254740993"), ("c", "9007199254740995")]));
}

#[test]
fn integer_division_by_zero_is_an_unbound_key_not_infinity() {
    // Unbound sorts first; an `f64` key made 1/0 positive infinity, which sorts last.
    let g = graph(&[("a", "1"), ("b", "0"), ("c", "2")]);
    let q = "SELECT ?s WHERE { ?s <http://ex/v> ?v } ORDER BY (1 / ?v)";
    assert_eq!(subjects(&g, q)[0], "<http://ex/b>");
}
