// [GPT-6] Raw RDF lexical validity is separate from XPath string construction.
use oxrdf::{Literal, NamedNode};
use sparq_core::Graph;
use sparq_engine::{QueryBudget, ask_with_budget, query_with_budget};

const XSD: &str = "http://www.w3.org/2001/XMLSchema#";

fn typed(value: &str, datatype: &str) -> String {
    Literal::new_typed_literal(value, NamedNode::new(format!("{XSD}{datatype}")).unwrap())
        .to_string()
}

fn budget(strict: bool) -> QueryBudget {
    QueryBudget {
        strict_numeric_capacity: strict,
        ..QueryBudget::unlimited()
    }
}

fn rows(graph: &Graph, query: &str, strict: bool) -> Vec<Vec<Option<String>>> {
    query_with_budget(graph, query, &budget(strict))
        .unwrap()
        .rows
        .into_iter()
        .map(|row| {
            row.into_iter()
                .map(|term| term.map(|t| t.to_string()))
                .collect()
        })
        .collect()
}

#[test]
fn raw_typed_boolean_whitespace_is_not_a_value() {
    for lexical in [
        " true ",
        "\t1\r\n",
        " false ",
        "\n0\t",
        "t rue",
        "TRUE",
        "yes",
        "\u{a0}true",
        "\u{b}1",
    ] {
        let term = typed(lexical, "boolean");
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
                for binding in [
                    format!("VALUES ?v {{{term}}}"),
                    "?s ?p ?v".into(),
                    format!("BIND({term} AS ?v)"),
                ] {
                    let ask = format!("ASK {{{binding} FILTER(?v)}}");
                    assert!(
                        !ask_with_budget(&graph, &ask, &budget(strict)).unwrap(),
                        "{ask}"
                    );
                    let query = format!(
                        "SELECT (IF(?v,1,2) AS ?a) (!?v AS ?b) (?v && true AS ?c) (?v || false AS ?d) (?v = true AS ?e) (<{XSD}boolean>(?v) AS ?f) (<{XSD}integer>(?v) AS ?g) {{{binding}}}"
                    );
                    assert_eq!(
                        rows(&graph, &query, strict),
                        vec![vec![
                            Some(typed("2", "integer")),
                            Some(typed("true", "boolean")),
                            Some(typed("false", "boolean")),
                            Some(typed("false", "boolean")),
                            None,
                            None,
                            None
                        ]],
                        "{query}"
                    );
                }
                assert!(
                    !ask_with_budget(&graph, &format!("ASK {{FILTER({term})}}"), &budget(strict))
                        .unwrap()
                );
            }
        }
    }
}

#[test]
fn raw_numeric_whitespace_fails_validity_ebv_arithmetic_and_cache() {
    for datatype in [
        "integer",
        "byte",
        "unsignedLong",
        "decimal",
        "float",
        "double",
    ] {
        for lexical in [" 1 ", "\t1\r\n", "\u{a0}1", "\u{b}1"] {
            let term = typed(lexical, datatype);
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
                    for binding in [
                        format!("VALUES ?v {{{term}}}"),
                        "?s ?p ?v".into(),
                        format!("BIND({term} AS ?v)"),
                    ] {
                        for filter in ["?v", "ISNUMERIC(?v)", "?v = 1", "?v < 2", "?v > 0"] {
                            let query = format!("ASK {{{binding} FILTER({filter})}}");
                            assert!(
                                !ask_with_budget(&graph, &query, &budget(strict)).unwrap(),
                                "{query}, strict={strict}, compressed={compressed}"
                            );
                        }
                        let query = format!(
                            "SELECT (?v = 1 AS ?a) (+?v AS ?b) (COALESCE(?v+1,7) AS ?c) (COALESCE(?v*2,7) AS ?d) (COALESCE(?v-1,7) AS ?e) (COALESCE(<{XSD}integer>(?v),7) AS ?f) {{{binding}}}"
                        );
                        assert_eq!(
                            rows(&graph, &query, strict),
                            vec![vec![
                                None,
                                None,
                                Some(typed("7", "integer")),
                                Some(typed("7", "integer")),
                                Some(typed("7", "integer")),
                                Some(typed("7", "integer"))
                            ]],
                            "{query}"
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn string_constructors_normalize_xml_whitespace_only() {
    let graph = Graph::load_str("", "n-triples").unwrap();
    for strict in [false, true] {
        for lexical in [" 1 ", "\t1\r\n"] {
            let string = Literal::new_simple_literal(lexical).to_string();
            for datatype in ["integer", "decimal", "float", "double", "boolean"] {
                let expected = if datatype == "boolean" { "true" } else { "1" };
                let query = format!("ASK {{FILTER(<{XSD}{datatype}>({string}) = {expected})}}");
                assert!(
                    ask_with_budget(&graph, &query, &budget(strict)).unwrap(),
                    "{query}"
                );
            }
        }
        for (lexical, expected) in [(" true ", true), ("\tfalse\r\n", false), (" 0 ", false)] {
            let string = Literal::new_simple_literal(lexical).to_string();
            assert_eq!(
                rows(
                    &graph,
                    &format!("SELECT (<{XSD}boolean>({string}) AS ?v) {{}}"),
                    strict
                ),
                vec![vec![Some(typed(
                    if expected { "true" } else { "false" },
                    "boolean"
                ))]]
            );
        }
        for lexical in ["\u{a0}1", "\u{b}1", "t rue"] {
            let string = Literal::new_simple_literal(lexical).to_string();
            for datatype in ["integer", "decimal", "float", "double", "boolean"] {
                assert_eq!(
                    rows(
                        &graph,
                        &format!("SELECT (<{XSD}{datatype}>({string}) AS ?v) {{}}"),
                        strict
                    ),
                    vec![vec![None]]
                );
            }
        }
    }
}

#[test]
fn original_terms_are_preserved_and_valid_boolean_forms_still_work() {
    for (lexical, datatype) in [
        (" true ", "boolean"),
        (" 1 ", "integer"),
        ("\t1.0\n", "decimal"),
    ] {
        let term = typed(lexical, datatype);
        let graph = Graph::load_str(
            &format!("<http://ex/s> <http://ex/p> {term} ."),
            "n-triples",
        )
        .unwrap();
        for strict in [false, true] {
            let query = format!(
                "SELECT ?v (STR(?v) AS ?raw) (sameTerm(?v,{term}) AS ?same) (DATATYPE(?v) AS ?dt) {{?s ?p ?v}}"
            );
            assert_eq!(
                rows(&graph, &query, strict),
                vec![vec![
                    Some(term.clone()),
                    Some(Literal::new_simple_literal(lexical).to_string()),
                    Some(typed("true", "boolean")),
                    Some(format!("<{XSD}{datatype}>"))
                ]]
            );
        }
    }
    let graph = Graph::load_str("", "n-triples").unwrap();
    for (lexical, value) in [("true", true), ("1", true), ("false", false), ("0", false)] {
        let term = typed(lexical, "boolean");
        for strict in [false, true] {
            assert_eq!(
                ask_with_budget(&graph, &format!("ASK {{FILTER({term})}}"), &budget(strict))
                    .unwrap(),
                value
            );
            assert!(
                ask_with_budget(
                    &graph,
                    &format!("ASK {{FILTER({term} = {value})}}"),
                    &budget(strict)
                )
                .unwrap()
            );
            assert_eq!(
                rows(
                    &graph,
                    &format!("SELECT (<{XSD}integer>({term}) AS ?v) {{}}"),
                    strict
                ),
                vec![vec![Some(typed(if value { "1" } else { "0" }, "integer"))]]
            );
        }
    }
}

// [GPT-6] The same complete result goldens feed the versioned guest runners.
#[test]
fn shared_raw_literal_result_goldens() {
    let corpus: serde_json::Value = serde_json::from_str(include_str!(
        "../../../zk/sparql-evaluator/fixtures/conformance/raw-literal-whitespace.json"
    ))
    .unwrap();
    let cases = corpus["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 32);
    for case in cases {
        let graph =
            Graph::load_str(case["dataset_ntriples"].as_str().unwrap(), "n-triples").unwrap();
        let query = case["query"].as_str().unwrap();
        for strict in [false, true] {
            if let Some(expected) = case["expected_result"]["Ask"].as_bool() {
                assert_eq!(
                    ask_with_budget(&graph, query, &budget(strict)).unwrap(),
                    expected,
                    "{}",
                    case["id"]
                );
            } else {
                assert_eq!(
                    serde_json::json!(rows(&graph, query, strict)),
                    case["expected_result"]["Select"]["rows"],
                    "{}",
                    case["id"]
                );
            }
        }
    }
}
