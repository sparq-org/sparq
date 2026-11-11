// Capacity exhaustion must not masquerade as a completed SPARQL answer.
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
        "\"340282366920938463463374607431768211456000\"^^xsd:integer < 1e0",
        "xsd:integer(\"9223372036854775808\")",
        "xsd:integer(\"9223372036854775808\"^^xsd:double)",
        "xsd:decimal(\"100000000000000000000000000000000000000000000\")",
        "SUBSTR(\"abcd\", 1, \"100000000000000000000000000000000000000000000\"^^xsd:integer)",
        "SUBSTR(\"abcd\", \"-100000000000000000000000000000000000000000000\"^^xsd:integer, \"100000000000000000000000000000000000000000002\"^^xsd:integer)",
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
        "\"0.0000000000000000000000000000000000000001\"^^xsd:decimal < 1",
        "SECONDS(\"2024-01-01T00:00:00.1234567890123456789012345678901234567890123Z\"^^xsd:dateTime) = 0.0",
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
fn stored_aggregates_cannot_return_partial_results() {
    for literal in ["\"9223372036854775808\"^^<http://www.w3.org/2001/XMLSchema#integer>",
                    "\"0.1234567890123456789012345678901234567890123\"^^<http://www.w3.org/2001/XMLSchema#decimal>"] {
        for compressed in [false, true] {
            let graph = Graph::load_str(&format!("<http://ex/s> <http://ex/p> {literal} ."), "n-triples").unwrap();
            let graph = if compressed { graph.into_compressed() } else { graph };
            for q in ["SELECT (SUM(?n) AS ?c) {?s ?p ?n}", "SELECT (AVG(?n) AS ?c) {?s ?p ?n}",
                      "SELECT (MIN(?n) AS ?c) {?s ?p ?n}", "SELECT (MAX(?n) AS ?c) {?s ?p ?n}"] {
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

#[test]
fn exact_comparisons_past_i128() {
    let low = "340282366920938463463374607431768211456000";
    let high = "340282366920938463463374607431768211456001";
    for datatype in ["integer", "decimal", "nonNegativeInteger"] {
        let literal = |s: &str| format!("\"{s}\"^^<http://www.w3.org/2001/XMLSchema#{datatype}>");
        let lo = literal(low);
        let hi = literal(high);
        let data = format!("<http://ex/s> <http://ex/p> {hi} .");
        let graph = Graph::load_str(&data, "n-triples").unwrap();
        let compressed = Graph::load_str(&data, "n-triples")
            .unwrap()
            .into_compressed();
        for graph in [&graph, &compressed] {
            for budget in [QueryBudget::unlimited(), bounded()] {
                for (op, expected) in [
                    ("=", false),
                    ("!=", true),
                    ("<", false),
                    ("<=", false),
                    (">", true),
                    (">=", true),
                ] {
                    for (left, right, expect) in [
                        ("?v".to_owned(), lo.clone(), expected),
                        (
                            lo.clone(),
                            "?v".to_owned(),
                            match op {
                                "=" | "!=" => expected,
                                _ => !expected,
                            },
                        ),
                    ] {
                        let expr = format!("{left} {op} {right}");
                        let q = format!("SELECT ?v {{?s ?p ?v FILTER({expr})}}");
                        let result = sparq_engine::query_with_budget(graph, &q, &budget).unwrap();
                        assert_eq!(result.rows.len(), usize::from(expect), "{q}");
                        // VALUES uses local terms; BIND exercises materialised value comparison.
                        let q = format!("SELECT ?b {{VALUES ?v {{{hi}}} BIND(({expr}) AS ?b)}}");
                        let result = sparq_engine::query_with_budget(graph, &q, &budget).unwrap();
                        assert_eq!(
                            result.rows,
                            vec![vec![Some(oxrdf::Literal::from(expect).into())]],
                            "{q}"
                        );
                    }
                }
                // Numeric equality crosses datatype, leading-zero and scale spellings.
                let q = format!("SELECT (\"+0{high}.00\"^^<http://www.w3.org/2001/XMLSchema#decimal> = {hi} AS ?b) {{}}");
                let result = sparq_engine::query_with_budget(graph, &q, &budget).unwrap();
                assert_eq!(
                    result.rows,
                    vec![vec![Some(oxrdf::Literal::from(true).into())]],
                    "{q}"
                );
            }
        }
    }
    let graph = Graph::load_str("", "n-triples").unwrap();
    for budget in [QueryBudget::unlimited(), bounded()] {
        for (a, b) in [
            (
                "\"-340282366920938463463374607431768211456001\"^^xsd:integer",
                "\"-340282366920938463463374607431768211456000.0\"^^xsd:decimal",
            ),
            (
                "\"0.1234567890123456789012345678901234567890\"^^xsd:decimal",
                "\"0.1234567890123456789012345678901234567891\"^^xsd:decimal",
            ),
            (
                "1",
                "\"340282366920938463463374607431768211456000\"^^xsd:integer",
            ),
            (
                "\"17014118346046923173168730371588410572.7\"^^xsd:decimal",
                "\"170141183460469231731687303715884105727\"^^xsd:decimal",
            ),
        ] {
            let q = query(&format!("{a} < {b}"));
            let result = sparq_engine::query_with_budget(&graph, &q, &budget).unwrap();
            assert_eq!(
                result.rows,
                vec![vec![Some(oxrdf::Literal::from(true).into())]],
                "{q}"
            );
        }
        for bad in [
            "\"340282366920938463463374607431768211456000.5\"^^xsd:integer",
            "\" 340282366920938463463374607431768211456000\"^^xsd:decimal",
            "\"340282366920938463463374607431768211456000\"^^xsd:byte",
            "\"340282366920938463463374607431768211456000\"^^xsd:negativeInteger",
        ] {
            for op in ["=", "!=", "<", "<=", ">", ">="] {
                let q = query(&format!("{bad} {op} 1"));
                let result = sparq_engine::query_with_budget(&graph, &q, &budget).unwrap();
                assert_eq!(result.rows, vec![vec![None]], "ill-typed operand: {q}");
            }
        }
    }
}

#[test]
fn order_by_past_i128_is_exact_in_both_sort_paths() {
    let mut ordered: Vec<String> = [
        "-340282366920938463463374607431768211456001",
        "-340282366920938463463374607431768211456000",
        "-0.00000000000000000000000000000000000000001",
        "0",
        "0.00000000000000000000000000000000000000001",
        "340282366920938463463374607431768211456000",
        "340282366920938463463374607431768211456001",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect();
    let low_infinite_image = format!("{}0", "8".repeat(399));
    let high_infinite_image = format!("{}1", "8".repeat(399));
    ordered.insert(0, format!("-{low_infinite_image}"));
    ordered.insert(0, format!("-{high_infinite_image}"));
    ordered.extend([low_infinite_image, high_infinite_image]);
    let data = ordered
        .iter()
        .rev()
        .enumerate()
        .map(|(i, v)| {
            format!(
        "<http://ex/{i}> <http://ex/p> \"{v}\"^^<http://www.w3.org/2001/XMLSchema#decimal> .\n"
    )
        })
        .collect::<String>();
    let graph = Graph::load_str(&data, "n-triples").unwrap();
    let compressed = Graph::load_str(&data, "n-triples")
        .unwrap()
        .into_compressed();
    for graph in [&graph, &compressed] {
        for budget in [QueryBudget::unlimited(), bounded()] {
            for key in ["?v", "COALESCE(?v)"] {
                for desc in [false, true] {
                    let order = if desc {
                        format!("DESC({key})")
                    } else {
                        key.to_string()
                    };
                    let q = format!("SELECT ?v {{?s ?p ?v}} ORDER BY {order}");
                    let result = sparq_engine::query_with_budget(graph, &q, &budget).unwrap();
                    let actual: Vec<_> = result
                        .rows
                        .iter()
                        .map(|row| match row[0].as_ref().unwrap() {
                            oxrdf::Term::Literal(l) => l.value(),
                            other => panic!("expected literal: {other}"),
                        })
                        .collect();
                    let mut expected: Vec<_> = ordered.iter().map(String::as_str).collect();
                    if desc {
                        expected.reverse();
                    }
                    assert_eq!(actual, expected, "{q}");
                }
            }
        }
    }
}

#[test]
fn strict_mixed_comparisons_keep_capacity_failures() {
    for exact in [
        "\"9223372036854775808\"^^xsd:integer",
        "\"340282366920938463463374607431768211456000\"^^xsd:integer",
        "\"340282366920938463463374607431768211456000.1\"^^xsd:decimal",
    ] {
        let graph = Graph::load_str(
            &format!("PREFIX xsd: <http://www.w3.org/2001/XMLSchema#> <http://ex/s> <http://ex/p> {exact} ."),
            "turtle",
        ).unwrap();
        for other in [
            "\"text\"",
            "<http://ex/iri>",
            "true",
            "1e0",
            "\"NaN\"^^xsd:double",
        ] {
            for op in ["=", "!=", "<", "<=", ">", ">="] {
                for expression in [
                    format!("{exact} {op} {other}"),
                    format!("{other} {op} {exact}"),
                ] {
                    for expression in [expression.clone(), format!("COALESCE({expression}, false)")]
                    {
                        for scalar in [expression.clone(), format!("MAX({expression})")] {
                            let q = query(&scalar);
                            let error = sparq_engine::query_with_budget(&graph, &q, &bounded())
                                .unwrap_err();
                            assert!(error.contains("numeric-representation"), "{q}: {error}");
                        }
                        let filter = expression.replace(exact, "?v");
                        let q = format!("PREFIX xsd:<http://www.w3.org/2001/XMLSchema#> ASK {{?s ?p ?v FILTER({filter})}}");
                        let error =
                            sparq_engine::ask_with_budget(&graph, &q, &bounded()).unwrap_err();
                        assert!(error.contains("numeric-representation"), "{q}: {error}");
                    }
                }
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

#[cfg(feature = "result-cache")]
#[test]
fn result_cache_keys_on_semantic_capacity_limits() {
    let graph = Graph::load_str("", "n-triples").unwrap();
    let prepared = sparq_engine::PreparedQuery::parse(&query("9223372036854775807 + 1")).unwrap();
    let cache = sparq_engine::ResultCache::new(4);
    cache
        .get_or_eval(&graph, prepared.query(), 0, &QueryBudget::unlimited())
        .unwrap();
    assert!(cache
        .get_or_eval(&graph, prepared.query(), 0, &bounded())
        .is_err());
    let years = QueryBudget {
        temporal_year_range: Some((1900, 2100)),
        ..QueryBudget::unlimited()
    };
    let dated = sparq_engine::PreparedQuery::parse(
        "PREFIX xsd:<http://www.w3.org/2001/XMLSchema#> SELECT (YEAR(\"3000-01-01T00:00:00Z\"^^xsd:dateTime) AS ?v) {}",
    )
    .unwrap();
    cache
        .get_or_eval(&graph, dated.query(), 0, &QueryBudget::unlimited())
        .unwrap();
    assert!(cache.get_or_eval(&graph, dated.query(), 0, &years).is_err());
}
