// [GPT-6] Capacity exhaustion must not masquerade as a completed SPARQL answer.
use sparq_core::Graph;
use sparq_engine::QueryBudget;

fn bounded() -> QueryBudget {
    QueryBudget {
        strict_numeric_capacity: true,
        ..QueryBudget::unlimited()
    }
}

fn query(expression: &str) -> String {
    format!("PREFIX xsd:<http://www.w3.org/2001/XMLSchema#> SELECT ({expression} AS ?v) {{}}")
}

#[test]
fn numeric_capacity_survives_expression_error_handling() {
    let graph = Graph::load_str("", "n-triples").unwrap();
    let expressions = [
        "9223372036854775807 + 1",
        "(-9223372036854775807 - 1) - 1",
        "9223372036854775807 * 2",
        "-(xsd:integer(\"-9223372036854775808\"))",
        "ABS(xsd:integer(\"-9223372036854775808\"))",
        "\"170141183460469231731687303715884105727\"^^xsd:decimal + 1.0",
        "\"100000000000000000000\"^^xsd:decimal * \"100000000000000000000\"^^xsd:decimal",
        "xsd:integer(\"9223372036854775808\")",
        "xsd:integer(\"9223372036854775808\"^^xsd:double)",
        "xsd:decimal(\"100000000000000000000000000000000000000000000\")",
        "SUBSTR(\"abcd\", 1, \"100000000000000000000000000000000000000000000\"^^xsd:integer)",
        "SUBSTR(\"abcd\", \"-100000000000000000000000000000000000000000000\"^^xsd:integer, \"100000000000000000000000000000000000000000002\"^^xsd:integer)",
        "\"0.0000000000000000000000000000000000000001\"^^xsd:decimal < 1",
        "SECONDS(\"2024-01-01T00:00:00.1234567890123456789012345678901234567890123Z\"^^xsd:dateTime) = 0.0",
    ];
    for expression in expressions {
        for wrapper in [
            expression.to_owned(),
            format!("COALESCE({expression}, 0)"),
            format!("IF(true, {expression}, 0)"),
            format!("({expression}) = 0"),
        ] {
            let q = query(&wrapper);
            let error = sparq_engine::query_with_budget(&graph, &q, &bounded()).unwrap_err();
            assert!(
                error.contains("evaluation capacity exceeded"),
                "{q}: {error}"
            );
        }
    }
    // Prior failed calls do not poison unrelated ordinary or bounded evaluations.
    for expression in [
        "COALESCE(1/0, 7)",
        "COALESCE(xsd:integer(\"invalid\"), 7)",
        "xsd:integer(\"0.0000000000000000000000000000000000000001\"^^xsd:decimal)",
        "ROUND(\"0.99999999999999999999999999999999999999\"^^xsd:decimal)",
        "1 + 2",
        "0.1 + 0.2",
        "1 / 3",
    ] {
        let q = query(expression);
        let ordinary = sparq_engine::query(&graph, &q).unwrap();
        let strict = sparq_engine::query_with_budget(&graph, &q, &bounded()).unwrap();
        assert_eq!(ordinary.rows, strict.rows, "{q}");
        assert!(strict.rows[0][0].is_some(), "{q}");
    }
}

#[test]
fn stored_consumers_cannot_return_false_ask_or_partial_aggregate() {
    for literal in ["\"9223372036854775808\"^^<http://www.w3.org/2001/XMLSchema#integer>",
                    "\"0.1234567890123456789012345678901234567890123\"^^<http://www.w3.org/2001/XMLSchema#decimal>"] {
        for compressed in [false, true] {
            let graph = Graph::load_str(&format!("<http://ex/s> <http://ex/p> {literal} ."), "n-triples").unwrap();
            let graph = if compressed { graph.into_compressed() } else { graph };
            for q in ["ASK {?s ?p ?n FILTER(?n < 0)}", "SELECT (COUNT(*) AS ?c) {?s ?p ?n FILTER(?n = 0)}",
                      "SELECT ?n {?s ?p ?n} ORDER BY ?n", "SELECT ?n {?s ?p ?n} ORDER BY ?n LIMIT 1",
                      "SELECT (SUM(?n) AS ?c) {?s ?p ?n}", "SELECT (AVG(?n) AS ?c) {?s ?p ?n}",
                      "SELECT (MIN(?n) AS ?c) {?s ?p ?n}", "SELECT (MAX(?n) AS ?c) {?s ?p ?n}",
                      "SELECT ?n {?s ?p ?n FILTER(?n = ?n)}"] {
                let result = sparq_engine::query_with_budget(&graph, q, &bounded());
                assert!(result.is_err(), "{q}: {literal}, compressed={compressed}: {result:?}");
                assert!(result.unwrap_err().contains("evaluation capacity exceeded"));
            }
            for q in ["SELECT ?n {?s ?p ?n}", "SELECT (+?n AS ?v) {?s ?p ?n}",
                      "SELECT (isNumeric(?n) AS ?v) {?s ?p ?n}", "SELECT (sameTerm(+?n, ?n) AS ?v) {?s ?p ?n}"] {
                assert!(sparq_engine::query_with_budget(&graph, q, &bounded()).is_ok(), "{q}");
            }
        }
    }
}

#[cfg(feature = "parallel")]
#[test]
fn parallel_threshold_cannot_hide_numeric_capacity() {
    let mut data = String::new();
    for i in 0..50_010 {
        data.push_str(&format!("<http://ex/{i}> <http://ex/p> {i} .\n"));
    }
    let graph = Graph::load_str(&data, "turtle").unwrap();
    let q = "SELECT ?s (COALESCE(9223372036854775807 + ?n, 0) AS ?v) {?s ?p ?n}";
    let error = sparq_engine::query_with_budget(&graph, q, &bounded()).unwrap_err();
    assert!(error.contains("numeric-representation"), "{error}");
    assert_eq!(sparq_engine::query(&graph, q).unwrap().rows.len(), 50_010);
}
