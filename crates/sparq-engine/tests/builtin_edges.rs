// [GPT-6] Manual value/error goldens, independent of engine output generation.
use sparq_core::Graph;

#[test]
fn deterministic_builtin_edges() {
    let corpus: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/builtin_edges.json")).unwrap();
    let graph = Graph::load_str("", "turtle").unwrap();
    let mut failures = Vec::new();
    for case in corpus["cases"].as_array().unwrap() {
        let query = case["query"].as_str().unwrap();
        let result = std::panic::catch_unwind(|| sparq_engine::query(&graph, query));
        let actual = match result {
            Ok(Ok(result)) => serde_json::json!(
                result
                    .rows
                    .into_iter()
                    .map(|row| {
                        row.into_iter()
                            .map(|term| term.map(|term| term.to_string()))
                            .collect::<Vec<_>>()
                    })
                    .collect::<Vec<_>>()
            ),
            Ok(Err(error)) => serde_json::json!({"query_error": error}),
            Err(_) => serde_json::json!({"panic": true}),
        };
        if actual != case["expected_rows"] {
            failures.push(format!(
                "{}: expected {}, actual {}",
                case["id"], case["expected_rows"], actual
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn stored_invalid_literals_fail_soft_in_dense_and_compressed_caches() {
    for (literal, filter, valid) in [
        ("\"1200\"^^xsd:byte", "?v > 0", false),
        ("\"5.0\"^^xsd:integer", "?v = 5", false),
        ("\"-1\"^^xsd:unsignedLong", "?v < 0", false),
        ("\"1\"^^xsd:nonPositiveInteger", "?v > 0", false),
        ("\"é\"^^xsd:integer", "?v > 0", false),
        ("\" 5 \"^^xsd:integer", "?v = 5", true),
        ("\"127\"^^xsd:byte", "?v > 0", true),
        (
            "\"2023-02-29T00:00:00Z\"^^xsd:dateTime",
            "?v > \"2023-01-01T00:00:00Z\"^^xsd:dateTime",
            false,
        ),
        (
            "\"2024-é-01T00:00:00Z\"^^xsd:dateTime",
            "YEAR(?v) = 2024",
            false,
        ),
        (
            "\"2024-01-01T00:00:00+é:00\"^^xsd:dateTime",
            "TZ(?v) = \"+é:00\"",
            false,
        ),
        (
            "\"2024-02-29T00:00:00Z\"^^xsd:dateTime",
            "?v > \"2023-01-01T00:00:00Z\"^^xsd:dateTime",
            true,
        ),
    ] {
        let data = format!(
            "@prefix xsd: <http://www.w3.org/2001/XMLSchema#> . <http://ex/s> <http://ex/p> {literal} ."
        );
        let query = format!(
            "PREFIX xsd: <http://www.w3.org/2001/XMLSchema#> SELECT ?v WHERE {{?s <http://ex/p> ?v FILTER({filter})}}"
        );
        for compressed in [false, true] {
            let graph = Graph::load_str(&data, "turtle").unwrap();
            let graph = if compressed {
                graph.into_compressed()
            } else {
                graph
            };
            let result = sparq_engine::query(&graph, &query).unwrap();
            assert_eq!(
                result.rows.len(),
                usize::from(valid),
                "{literal}, compressed={compressed}"
            );
        }
    }
}
