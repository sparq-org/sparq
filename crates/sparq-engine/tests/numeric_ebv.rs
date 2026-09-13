// [GPT-6] SPARQL 1.1 §17.2.2 distinguishes numeric EBV from arithmetic capacity.
use sparq_core::Graph;
use sparq_engine::{QueryBudget, ask_with_budget, query_with_budget};

const XSD: &str = "http://www.w3.org/2001/XMLSchema#";

fn budget(strict: bool) -> QueryBudget {
    QueryBudget {
        strict_numeric_capacity: strict,
        ..QueryBudget::unlimited()
    }
}

fn literal(value: &str, datatype: &str) -> String {
    format!("\"{value}\"^^<{XSD}{datatype}>")
}

fn rows(graph: &Graph, query: &str, strict: bool) -> Vec<Vec<Option<String>>> {
    query_with_budget(graph, query, &budget(strict))
        .unwrap()
        .rows
        .into_iter()
        .map(|row| {
            row.into_iter()
                .map(|term| term.map(|term| term.to_string()))
                .collect()
        })
        .collect()
}

fn boolean(value: bool) -> Option<String> {
    Some(literal(if value { "true" } else { "false" }, "boolean"))
}

#[test]
fn exact_numeric_ebv_uses_value_zero_not_a_float_image() {
    let tiny = format!("0.{}1", "0".repeat(324));
    let huge = "9".repeat(400);
    for (lexical, datatype, expected) in [
        (tiny.clone(), "decimal", true),
        (format!("-{tiny}"), "decimal", true),
        (huge, "integer", true),
        (format!("-0.{}", "0".repeat(400)), "decimal", false),
        ("1e-46".into(), "float", false),
        ("1e-46".into(), "double", true),
        ("NaN".into(), "double", false),
        ("INF".into(), "float", true),
        ("-0".into(), "double", false),
    ] {
        let term = literal(&lexical, datatype);
        for compressed in [false, true] {
            let graph = Graph::load_str(
                &format!("<http://ex/s> <http://ex/p> {term} ."),
                "n-triples",
            )
            .unwrap();
            let graph = if compressed {
                graph.into_compressed()
            } else {
                graph
            };
            for strict in [false, true] {
                for query in [
                    "ASK {?s ?p ?n FILTER(?n)}".to_owned(),
                    format!("ASK {{VALUES ?n {{{term}}} FILTER(?n)}}"),
                    format!("ASK {{FILTER({term})}}"),
                ] {
                    assert_eq!(
                        ask_with_budget(&graph, &query, &budget(strict)).unwrap(),
                        expected,
                        "{query}, compressed={compressed}, strict={strict}"
                    );
                }
                let query = "SELECT (IF(?n,true,false) AS ?a) (!?n AS ?b) (?n && true AS ?c) (?n || false AS ?d) { ?s ?p ?n }";
                assert_eq!(
                    rows(&graph, query, strict),
                    vec![vec![
                        boolean(expected),
                        boolean(!expected),
                        boolean(expected),
                        boolean(expected)
                    ]]
                );
            }
        }
    }
}

#[test]
fn invalid_numeric_and_boolean_ebv_is_false_but_arithmetic_errors() {
    let empty = Graph::load_str("", "n-triples").unwrap();
    for term in [
        literal("1.5", "integer"),
        literal("1200", "byte"),
        literal("-1", "unsignedLong"),
        literal("\u{a0}1\u{a0}", "decimal"),
        literal("é", "integer"),
        literal("yes", "boolean"),
    ] {
        for compressed in [false, true] {
            let graph = Graph::load_str(
                &format!("<http://ex/s> <http://ex/p> {term} ."),
                "n-triples",
            )
            .unwrap();
            let graph = if compressed {
                graph.into_compressed()
            } else {
                graph
            };
            for strict in [false, true] {
                for source in [
                    format!("VALUES ?n {{{term}}}"),
                    format!("BIND({term} AS ?n)"),
                    "?s ?p ?n".to_string(),
                ] {
                    let query = format!(
                        "SELECT (IF(?n,true,false) AS ?a) (!?n AS ?b) (COALESCE(?n+1,7) AS ?c) {{{source}}}"
                    );
                    assert_eq!(
                        rows(&graph, &query, strict),
                        vec![vec![
                            boolean(false),
                            boolean(true),
                            Some(literal("7", "integer"))
                        ]],
                        "{query}"
                    );
                }
            }
        }
    }
    // Unknown datatypes and unbound inputs still have an error EBV.
    for input in ["<http://ex/iri>", "?missing"] {
        let query = format!("SELECT (!{input} AS ?a) (COALESCE(IF({input},1,2),7) AS ?b) {{}}");
        assert_eq!(
            rows(&empty, &query, true),
            vec![vec![None, Some(literal("7", "integer"))]]
        );
    }
}

#[test]
fn arithmetic_capacity_stays_sticky_through_ebv_and_error_handlers() {
    let tiny = literal(&format!("0.{}1", "0".repeat(324)), "decimal");
    let graph = Graph::load_str(
        &format!("<http://ex/s> <http://ex/p> {tiny} ."),
        "n-triples",
    )
    .unwrap();
    for expression in [
        "IF(?n+1,true,false)",
        "!(?n+1)",
        "(?n+1) && false",
        "(?n+1) || true",
        "COALESCE(IF(?n+1,1,2),7)",
    ] {
        for query in [
            format!("ASK {{?s ?p ?n FILTER({expression})}}"),
            format!("SELECT ({expression} AS ?v) {{?s ?p ?n}}"),
        ] {
            let error = query_with_budget(&graph, &query, &budget(true)).unwrap_err();
            assert!(error.contains("numeric-representation"), "{query}: {error}");
        }
    }
    assert!(ask_with_budget(&graph, "ASK {?s ?p ?n FILTER(?n)}", &budget(true)).unwrap());
}

#[test]
fn interpreted_aggregate_arguments_enforce_arithmetic_and_comparison_capacity() {
    for (value, expression) in [
        ("1".to_owned(), "SUM(?n + 9223372036854775807)"),
        (
            literal(&format!("0.{}1", "0".repeat(324)), "decimal"),
            "MAX(?n < 0)",
        ),
    ] {
        let graph = Graph::load_str(
            &format!("<http://ex/s> <http://ex/p> {value} ."),
            if value == "1" { "turtle" } else { "n-triples" },
        )
        .unwrap();
        let query = format!("SELECT ({expression} AS ?v) {{?s ?p ?n}}");
        let error = query_with_budget(&graph, &query, &budget(true)).unwrap_err();
        assert!(error.contains("numeric-representation"), "{query}: {error}");
    }
    let graph = Graph::load_str("", "n-triples").unwrap();
    assert_eq!(
        rows(
            &graph,
            "SELECT (SUM(?n+1) AS ?a) (MAX(?n<0) AS ?b) {VALUES ?n {1 2}}",
            true
        ),
        vec![vec![Some(literal("5", "integer")), boolean(false)]]
    );
}

#[test]
fn direct_outputs_preserve_lexical_and_datatype_identity() {
    let term = literal(&format!("+0.{}1", "0".repeat(324)), "decimal");
    let graph = Graph::load_str(
        &format!("<http://ex/s> <http://ex/p> {term} ."),
        "n-triples",
    )
    .unwrap();
    for strict in [false, true] {
        assert_eq!(
            rows(
                &graph,
                "SELECT ?n (+?n AS ?p) (STR(?n) AS ?s) (DATATYPE(?n) AS ?d) (isNumeric(?n) AS ?i) (sameTerm(+?n,?n) AS ?t) {?a ?b ?n}",
                strict
            ),
            vec![vec![
                Some(term.clone()),
                Some(term.clone()),
                Some(format!("\"+0.{}1\"", "0".repeat(324))),
                Some(format!("<{XSD}decimal>")),
                boolean(true),
                boolean(true)
            ]]
        );
    }
}

#[test]
fn large_decimal_cast_keeps_exact_mantissa_in_default_mode() {
    let graph = Graph::load_str("", "n-triples").unwrap();
    let value = "17014118346046923173168730371588410573";
    let query = format!("SELECT (<{XSD}decimal>(\"{value}\") AS ?v) {{}}");
    // Appending .0 in an i128 mantissa would overflow; never saturate the final digit.
    assert_eq!(
        rows(&graph, &query, false),
        vec![vec![Some(literal(&format!("{value}.0"), "decimal"))]]
    );
}
