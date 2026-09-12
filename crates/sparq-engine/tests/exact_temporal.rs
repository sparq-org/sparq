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
fn temporal_capacity_is_a_query_failure_even_when_expression_errors_are_handled() {
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
    assert_eq!(sparq_engine::query(&graph, query).unwrap().rows.len(), 50_010,
        "unlimited native parallel evaluation retains its checked year domain");
}
