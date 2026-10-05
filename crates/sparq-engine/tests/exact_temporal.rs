// [GPT-6] REC-derived literal, computed and stored temporal comparison matrix.
use sparq_core::Graph;

#[test]
fn temporal_value_precision_survives_every_evaluation_path() {
    let corpus: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/exact_temporal.json")).unwrap();
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
fn graph_exact_temporal_keys_borrow_the_original_fraction() {
    for compressed in [false, true] {
        let graph = Graph::load_str("<http://ex/s> <http://ex/p> \"2024-01-01T00:00:00.000000001Z\"^^<http://www.w3.org/2001/XMLSchema#dateTime> .", "ntriples").unwrap();
        let graph = if compressed {
            graph.into_compressed()
        } else {
            graph
        };
        let scan = graph.store.scan(&[None, None, None]);
        let id = scan.to_spo(&scan.rows[0])[2];
        let exact = graph.exact_temporal_value(id).unwrap();
        let zero = sparq_core::temporal::ExactTemporal::of_lit(
            "2024-01-01T00:00:00Z",
            "http://www.w3.org/2001/XMLSchema#dateTime",
        )
        .unwrap();
        assert_eq!(exact.compare(zero), Some(std::cmp::Ordering::Greater));
    }
}

fn bounded() -> sparq_engine::QueryBudget {
    sparq_engine::QueryBudget {
        temporal_year_range: Some((1, 1_000_000_000)),
        ..Default::default()
    }
}

#[test]
fn check_temporal_is_a_query_failure_even_when_expression_errors_are_handled() {
    let graph = Graph::load_str("", "ntriples").unwrap();
    let expression = "STRDT(CONCAT(\"1000000001\", \"-01-01T00:00:00Z\"), xsd:dateTime)";
    let cast = "xsd:dateTime(CONCAT(\"1000000001\", \"-01-01T00:00:00Z\"))";
    for body in [
        format!("SELECT ({expression} AS ?x) {{}}"),
        format!("SELECT (COALESCE({expression}, 1) AS ?x) {{}}"),
        format!("SELECT (IF(true, {expression}, 1) AS ?x) {{}}"),
        format!("ASK {{ BIND({expression} AS ?x) FILTER(!BOUND(?x)) }}"),
        format!("ASK {{ FILTER({expression} = {expression}) }}"),
        format!("ASK {{ FILTER({cast} = {cast}) }}"),
        "ASK { VALUES ?x { \"1000000001-01-01T00:00:00Z\"^^xsd:dateTime } FILTER(?x < \"2024-01-01T00:00:00Z\"^^xsd:dateTime) }".into(),
    ] {
        let query = format!("PREFIX xsd:<http://www.w3.org/2001/XMLSchema#> {body}");
        let error = sparq_engine::query_with_budget(&graph, &query, &bounded()).unwrap_err();
        assert!(error.contains("query evaluation capacity exceeded (temporal-year)"), "{query}: {error}");
    }
    // A short-circuited expression never constructs a temporal value.
    let query = format!(
        "PREFIX xsd:<http://www.w3.org/2001/XMLSchema#> SELECT (IF(false, {expression}, 1) AS ?x) {{}}"
    );
    assert_eq!(
        sparq_engine::query_with_budget(&graph, &query, &bounded())
            .unwrap()
            .rows
            .len(),
        1
    );
    // Ordinary lexical errors remain ordinary unbound expressions; no capacity state leaks.
    let query = "PREFIX xsd:<http://www.w3.org/2001/XMLSchema#> SELECT (COALESCE(xsd:dateTime(\"invalid\"), 1) AS ?x) {}";
    assert!(
        sparq_engine::query_with_budget(&graph, query, &bounded())
            .unwrap()
            .rows[0][0]
            .is_some()
    );
}

#[test]
fn stored_capacity_cannot_become_false_ask_or_zero_count() {
    let graph = Graph::load_str("<http://ex/s> <http://ex/p> \"1000000001-01-01T00:00:00Z\"^^<http://www.w3.org/2001/XMLSchema#dateTime> .", "ntriples").unwrap();
    for form in ["ASK", "SELECT (COUNT(*) AS ?n)", "SELECT ?x"] {
        let query = format!(
            "PREFIX xsd:<http://www.w3.org/2001/XMLSchema#> {form} {{ ?s ?p ?x FILTER(?x < \"2024-01-01T00:00:00Z\"^^xsd:dateTime) }}"
        );
        let error = sparq_engine::query_with_budget(&graph, &query, &bounded()).unwrap_err();
        assert!(
            error.contains("evaluation capacity exceeded"),
            "{query}: {error}"
        );
    }
    // Unlimited native execution keeps its original checked year range.
    assert!(sparq_engine::query(&graph, "SELECT ?x { ?s ?p ?x }").is_ok());
}

#[cfg(feature = "parallel")]
#[test]
fn bounded_expression_evaluation_does_not_lose_capacity_on_rayon_workers() {
    let mut source = String::new();
    for index in 0..50_010 {
        source.push_str(&format!("<http://ex/{index}> <http://ex/p> {index} .\n"));
    }
    let graph = Graph::load_str(&source, "turtle").unwrap();
    let query = "PREFIX xsd:<http://www.w3.org/2001/XMLSchema#> SELECT ?s (COALESCE(IF(STRDT(CONCAT(\"1000000001\", \"-01-01T00:00:00Z\"), xsd:dateTime) = STRDT(CONCAT(\"1000000001\", \"-01-01T00:00:00Z\"), xsd:dateTime), 1, 0), 1) AS ?d) { ?s ?p ?x }";
    let Err(error) = sparq_engine::query_with_budget(&graph, query, &bounded()) else {
        panic!("parallel expression capacity must fail the query");
    };
    assert!(error.contains("evaluation capacity exceeded"), "{error}");
    assert_eq!(
        sparq_engine::query(&graph, query).unwrap().rows.len(),
        50_010,
        "unlimited native parallel evaluation retains its checked year domain"
    );
}

#[test]
fn year_zero_is_a_capacity_failure_before_expression_error_handling() {
    let graph = Graph::load_str("", "nt").unwrap();
    for expression in [
        "\"0000-01-01T00:00:00Z\"^^xsd:dateTime",
        "STRDT(CONCAT(\"0000\",\"-01-01T00:00:00Z\"),xsd:dateTime)",
        "xsd:dateTime(CONCAT(\"0000\",\"-01-01T00:00:00Z\"))",
    ] {
        let query = format!(
            "PREFIX xsd:<http://www.w3.org/2001/XMLSchema#> SELECT (COALESCE({expression}, 1) AS ?t) {{}}"
        );
        let error = sparq_engine::query_with_budget(&graph, &query, &bounded()).unwrap_err();
        assert!(
            error.contains("query evaluation capacity exceeded (temporal-year)"),
            "{error}"
        );
    }
}

#[test]
fn year_zero_stored_filter_cannot_become_false_ask_or_zero_count() {
    let graph = Graph::load_str("<http://ex/s> <http://ex/p> \"0000-01-01T00:00:00Z\"^^<http://www.w3.org/2001/XMLSchema#dateTime> .", "nt").unwrap();
    for projection in ["ASK", "SELECT (COUNT(*) AS ?n)"] {
        let query = format!(
            "PREFIX xsd:<http://www.w3.org/2001/XMLSchema#> {projection} {{ ?s ?p ?t FILTER(?t < \"0001-01-01T00:00:00Z\"^^xsd:dateTime) }}"
        );
        let error = sparq_engine::query_with_budget(&graph, &query, &bounded()).unwrap_err();
        assert!(
            error.contains("query evaluation capacity exceeded (temporal-year)"),
            "{error}"
        );
    }
}

// The scan pushdown and the general comparison may decide far-apart rows from
// cached f64 instants; both must agree with the exact comparison on every row, including near-ties,
// sub-nanosecond fractions, the fourteen-hour mixed-timezone window and both
// dateTime and date families.
#[test]
fn pushed_down_temporal_filters_match_the_exact_comparison() {
    let constants = [
        "\"2000-01-01T00:00:00Z\"^^<http://www.w3.org/2001/XMLSchema#dateTime>",
        "\"2000-01-01T00:00:00\"^^<http://www.w3.org/2001/XMLSchema#dateTime>",
        "\"2000-01-01T00:00:00.000000000000000000001+05:00\"^^<http://www.w3.org/2001/XMLSchema#dateTime>",
        "\"2000-01-01\"^^<http://www.w3.org/2001/XMLSchema#date>",
        "\"-0044-03-15T12:00:00Z\"^^<http://www.w3.org/2001/XMLSchema#dateTime>",
    ];
    let mut data = String::new();
    let mut n = 0;
    let mut add = |lexical: String, datatype: &str| {
        data.push_str(&format!(
            "<http://ex/s{n}> <http://ex/d> \"{lexical}\"^^<http://www.w3.org/2001/XMLSchema#{datatype}> .\n"
        ));
        n += 1;
    };
    for base in ["1999-12-31T", "2000-01-01T", "2000-01-02T", "1999-12-31T09:", "2000-01-01T14:"] {
        for time in ["00:00:00", "13:59:59", "14:00:00", "14:00:01", "23:59:59", "09:59:59.999999999999"] {
            let time = if base.ends_with(':') { &time[3..] } else { time };
            for zone in ["", "Z", "+14:00", "-14:00", "+05:00"] {
                add(format!("{base}{time}{zone}"), "dateTime");
            }
        }
    }
    for fraction in ["", ".0", ".000000000000000000001", ".999999999999999999999", ".5"] {
        for second in ["59", "00", "01"] {
            add(format!("1999-12-31T23:59:{second}{fraction}Z"), "dateTime");
            add(format!("2000-01-01T00:00:{second}{fraction}"), "dateTime");
        }
    }
    for day in ["1999-12-31", "2000-01-01", "2000-01-02", "-0044-03-15", "-0044-03-16"] {
        for zone in ["", "Z", "+14:00", "-14:00"] {
            add(format!("{day}{zone}"), "date");
        }
    }
    let graph = Graph::load_str(&data, "ntriples").unwrap();
    let mut selected = 0;
    for constant in constants {
        for op in [">", ">=", "<", "<=", "="] {
            let pushed = format!("SELECT ?s WHERE {{ ?s <http://ex/d> ?d FILTER(?d {op} {constant}) }} ORDER BY ?s");
            // COALESCE keeps the comparison out of the scan, on the exact general path.
            let general = format!(
                "SELECT ?s WHERE {{ ?s <http://ex/d> ?d FILTER(COALESCE(?d {op} {constant}, false)) }} ORDER BY ?s"
            );
            assert!(
                sparq_engine::explain(&graph, &pushed).unwrap().contains("pushed into scan"),
                "the first form must exercise the scan pushdown"
            );
            // An active temporal capacity budget disables every cached-instant
            // shortcut, so this is the exact reference.
            let exact = sparq_engine::QueryBudget {
                temporal_year_range: Some((-100_000, 100_000)),
                ..sparq_engine::QueryBudget::unlimited()
            };
            let reference = sparq_engine::query_with_budget(&graph, &general, &exact).unwrap().rows;
            selected += reference.len();
            let conjunct = format!(
                "SELECT ?s WHERE {{ ?s <http://ex/d> ?d FILTER((?d {op} {constant}) && BOUND(?d)) }} ORDER BY ?s"
            );
            for text in [&pushed, &general, &conjunct] {
                assert_eq!(sparq_engine::query(&graph, text).unwrap().rows, reference, "{op} {constant}: {text}");
            }
        }
    }
    assert!(selected > 0, "the corpus must exercise the comparisons");
}

#[test]
fn datetime_accessors_read_validated_components() {
    let graph = Graph::load_str("", "ntriples").unwrap();
    for (lexical, expected) in [
        ("-0044-03-15T12:30:05.250+01:00", ["-44", "3", "15", "12", "30", "5.250"]),
        ("2023-12-31T24:00:00Z", ["2024", "1", "1", "0", "0", "0"]),
        ("123456-02-28T07:08:09", ["123456", "2", "28", "7", "8", "9"]),
    ] {
        let query = format!(
            "PREFIX xsd:<http://www.w3.org/2001/XMLSchema#> SELECT (YEAR(?d) AS ?y) (MONTH(?d) AS ?mo) (DAY(?d) AS ?da) (HOURS(?d) AS ?h) (MINUTES(?d) AS ?mi) (SECONDS(?d) AS ?s) {{ BIND(\"{lexical}\"^^xsd:dateTime AS ?d) }}"
        );
        let result = sparq_engine::query(&graph, &query).unwrap();
        let actual: Vec<String> = result.rows[0]
            .iter()
            .map(|term| match term.as_ref().unwrap() {
                oxrdf::Term::Literal(literal) => literal.value().to_string(),
                other => other.to_string(),
            })
            .collect();
        assert_eq!(actual, expected, "{lexical}");
    }
}

#[test]
fn stored_identical_operands_still_meet_the_temporal_year_range() {
    let graph = Graph::load_str(
        "<http://ex/s> <http://ex/p> \"1000000001-01-01T00:00:00Z\"^^<http://www.w3.org/2001/XMLSchema#dateTime> .",
        "ntriples",
    )
    .unwrap();
    for query in ["SELECT ?x { ?s ?p ?x FILTER(?x = ?x) }", "ASK { ?s ?p ?x FILTER(?x = ?x) }"] {
        let error = sparq_engine::query_with_budget(&graph, query, &bounded()).unwrap_err();
        assert!(error.contains("query evaluation capacity exceeded (temporal-year)"), "{query}: {error}");
    }
}
