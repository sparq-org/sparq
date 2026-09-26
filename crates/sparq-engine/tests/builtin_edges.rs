// [GPT-6] Manual value/error goldens, independent of engine output generation.
use sparq_core::Graph;

#[test]
fn deterministic_builtin_edges() {
    let corpus: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/builtin_edges.json")).unwrap();
    let mut failures = Vec::new();
    for case in corpus["cases"].as_array().unwrap() {
        let query = case["query"].as_str().unwrap();
        let graph =
            Graph::load_str(case["dataset_ntriples"].as_str().unwrap_or(""), "n-triples").unwrap();
        let result = std::panic::catch_unwind(|| sparq_engine::query(&graph, query));
        let actual = match result {
            Ok(Ok(result)) => serde_json::json!(result
                .rows
                .into_iter()
                .map(|row| {
                    row.into_iter()
                        .map(|term| term.map(|term| term.to_string()))
                        .collect::<Vec<_>>()
                })
                .collect::<Vec<_>>()),
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

// [GPT-6] Arithmetic comparisons must validate operands before exact-decimal shortcuts.
#[test]
fn stored_arithmetic_validity_agrees_in_dense_and_compressed_graphs() {
    for (literal, valid) in [
        ("\"1200\"^^xsd:byte", false),
        ("\"5.0\"^^xsd:integer", false),
        ("\"-1\"^^xsd:unsignedLong", false),
        ("\"\u{a0}5\"^^xsd:integer", false),
        ("\"é\"^^xsd:integer", false),
        ("\"127\"^^xsd:byte", true),
        ("\" 5 \"^^xsd:integer", true),
        ("\"0.5\"^^xsd:decimal", true),
    ] {
        let data = format!(
            "@prefix xsd: <http://www.w3.org/2001/XMLSchema#> . <http://ex/s> <http://ex/p> {literal} ."
        );
        for compressed in [false, true] {
            let graph = Graph::load_str(&data, "turtle").unwrap();
            let graph = if compressed {
                graph.into_compressed()
            } else {
                graph
            };
            let value_id = graph.iter_ids().next().unwrap()[2];
            assert_eq!(
                graph.exact_numeric_lexical(value_id).is_some(),
                valid,
                "exact lexical {literal}, compressed={compressed}"
            );
            for predicate in [
                "(?n + 1) > 0",
                "0 < (?n + 1)",
                "(?n - 1) < 128",
                "128 > (?n - 1)",
                "(?n * 0) = 0",
                "0 = (?n * 0)",
                "(+?n) > 0",
                "0 < (+?n)",
                "(-?n) < 0",
                "0 > (-?n)",
            ] {
                let body = "?s <http://ex/p> ?n";
                let projected = format!("SELECT ({predicate} AS ?v) WHERE {{ {body} }}");
                let result = sparq_engine::query(&graph, &projected).unwrap();
                assert_eq!(result.rows.len(), 1);
                assert_eq!(
                    result.rows[0][0].is_some(),
                    valid,
                    "{literal} {projected}, compressed={compressed}"
                );
                if valid {
                    assert_eq!(
                        result.rows[0][0].as_ref().unwrap().to_string(),
                        "\"true\"^^<http://www.w3.org/2001/XMLSchema#boolean>"
                    );
                }
                let filtered = format!("SELECT ?s WHERE {{ {body} FILTER({predicate}) }}");
                assert_eq!(
                    sparq_engine::query(&graph, &filtered).unwrap().rows.len(),
                    usize::from(valid),
                    "{literal} {filtered}, compressed={compressed}"
                );
            }
        }
    }
}

// [GPT-6] Identity comparison exposes a unary-plus bypass that arithmetic tests miss.
#[test]
fn stored_unary_plus_and_cast_errors_agree_across_execution_paths() {
    for (literal, expression, valid) in [
        ("\"1200\"^^xsd:byte", "(+?n) = ?n", false),
        ("\"5.0\"^^xsd:integer", "(+?n) = ?n", false),
        ("\"-1\"^^xsd:unsignedLong", "(+?n) = ?n", false),
        ("\"5\"", "(+?n) = ?n", false),
        ("\"+005\"^^xsd:byte", "sameTerm(+?n, ?n)", true),
        (
            "\"10000000000000000000000000000000000000000000000\"^^xsd:integer",
            "sameTerm(+?n, ?n)",
            true,
        ),
        ("\"\u{a0}5\u{a0}\"", "xsd:decimal(?n) = 5", false),
        ("\"é\"", "xsd:decimal(?n) = 5", false),
        ("\" 5 \"", "xsd:decimal(?n) = 5", true),
    ] {
        for compressed in [false, true] {
            let data = format!("@prefix xsd:<http://www.w3.org/2001/XMLSchema#> . <http://ex/s> <http://ex/p> {literal} .");
            let graph = Graph::load_str(&data, "turtle").unwrap();
            let graph = if compressed {
                graph.into_compressed()
            } else {
                graph
            };
            let prefix = "PREFIX xsd:<http://www.w3.org/2001/XMLSchema#>";
            let projected = format!("{prefix} SELECT ({expression} AS ?v) {{?s <http://ex/p> ?n}}");
            let rows = sparq_engine::query(&graph, &projected).unwrap().rows;
            assert_eq!(rows.len(), 1);
            assert_eq!(
                rows[0][0].is_some(),
                valid,
                "{projected} {literal}, compressed={compressed}"
            );
            let filtered =
                format!("{prefix} SELECT ?s {{?s <http://ex/p> ?n FILTER({expression})}}");
            assert_eq!(
                sparq_engine::query(&graph, &filtered).unwrap().rows.len(),
                usize::from(valid),
                "{filtered} {literal}, compressed={compressed}"
            );
        }
    }
}
