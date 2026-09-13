// [GPT-6] Typed lexical, explicit datatype-extension and constructor-capacity controls.
use sparq_core::Graph;

#[test]
fn temporal_typed_lexicals_and_string_constructors_remain_distinct() {
    let corpus: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/temporal_lexical.json")).unwrap();
    let mut failures = Vec::new();
    for compressed in [false, true] {
        for case in corpus["cases"].as_array().unwrap() {
            let graph =
                Graph::load_str(case["dataset_ntriples"].as_str().unwrap(), "ntriples").unwrap();
            let graph = if compressed {
                graph.into_compressed()
            } else {
                graph
            };
            let result = sparq_engine::query(&graph, case["query"].as_str().unwrap()).unwrap();
            let actual = serde_json::json!(
                result
                    .rows
                    .into_iter()
                    .map(|row| {
                        row.into_iter()
                            .map(|term| term.map(|term| term.to_string()))
                            .collect::<Vec<_>>()
                    })
                    .collect::<Vec<_>>()
            );
            if actual != case["expected_rows"] {
                failures.push(format!(
                    "{} compressed={compressed}: expected {}, actual {actual}",
                    case["id"], case["expected_rows"]
                ));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn constructor_preprocessing_cannot_bypass_temporal_capacity() {
    let graph = Graph::load_str("", "nt").unwrap();
    let budget = sparq_engine::QueryBudget {
        temporal_year_range: Some((1, 1_000_000_000)),
        ..Default::default()
    };
    for expression in [
        "STR(xsd:dateTime(CONCAT(\" 1000000001\", \"-01-01T00:00:00Z \")))",
        "COALESCE(YEAR(xsd:dateTime(CONCAT(\"\\t1000000001\", \"-01-01T00:00:00Z\\n\"))),7)",
        "STR(xsd:dateTime(\" 0000-01-01T00:00:00Z \"))",
    ] {
        let query = format!(
            "PREFIX xsd:<http://www.w3.org/2001/XMLSchema#> SELECT ({expression} AS ?v) {{}}"
        );
        let error = sparq_engine::query_with_budget(&graph, &query, &budget).unwrap_err();
        assert!(
            error.contains("query evaluation capacity exceeded (temporal-year)"),
            "{error}"
        );
    }
    for form in ["ASK", "SELECT (COUNT(*) AS ?n)"] {
        let query = format!(
            "PREFIX xsd:<http://www.w3.org/2001/XMLSchema#> {form} {{ VALUES ?t {{\" 1000000001-01-01T00:00:00Z \"^^xsd:dateTime}} FILTER(YEAR(?t)=2024) }}"
        );
        let result = sparq_engine::query_with_budget(&graph, &query, &budget).unwrap();
        if form == "ASK" {
            assert_eq!(result.rows.len(), 0);
        } else {
            assert_eq!(
                result.rows[0][0].as_ref().unwrap().to_string(),
                "\"0\"^^<http://www.w3.org/2001/XMLSchema#integer>"
            );
        }
    }
}
