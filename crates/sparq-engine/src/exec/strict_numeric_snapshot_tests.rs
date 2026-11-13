use super::*;

fn check_profiles(query: &str, data: &str, rows: Vec<Vec<Option<Term>>>, capacity: bool) {
    let graph = Graph::load_str(data, "n-triples").unwrap();
    let prepared = crate::PreparedQuery::parse(query).unwrap();
    for strict in [false, true] {
        let limits = crate::QueryBudget {
            strict_numeric_capacity: strict,
            ..crate::QueryBudget::unlimited()
        };
        let result = crate::query_prepared_with_budget_detailed(&graph, &prepared, &limits);
        if strict && capacity {
            assert!(
                matches!(
                    result,
                    Err(crate::QueryFailure::Capacity(
                        crate::EvaluationCapacity::NumericRepresentation
                    ))
                ),
                "{query}, {data}: {result:?}"
            );
        } else {
            assert_eq!(
                result.unwrap().rows,
                rows,
                "strict={strict}: {query}, {data}"
            );
        }
    }
}

#[test]
fn strict_numeric_snapshot_bound_checks_seven_consumer_paths() {
    // The stored cases exercise false ASK/count, identical-term equality and
    // both full and top-k ordering at the integer operand boundary.
    for (integer, capacity) in [
        ("9223372036854775807", false),
        ("9223372036854775808", true),
    ] {
        let value: Term = Literal::new_typed_literal(integer, xsd::INTEGER).into();
        let data = format!("<http://ex/s> <http://ex/p> {value} .");
        for (query, rows) in [
            ("ASK {?s ?p ?n FILTER(?n < 0)}", vec![]),
            (
                "SELECT (COUNT(*) AS ?v) {?s ?p ?n FILTER(?n = 0)}",
                vec![vec![Some(Literal::from(0).into())]],
            ),
            (
                "SELECT ?n {?s ?p ?n FILTER(?n = ?n)}",
                vec![vec![Some(value.clone())]],
            ),
            (
                "SELECT ?n {?s ?p ?n} ORDER BY ?n",
                vec![vec![Some(value.clone())]],
            ),
            (
                "SELECT ?n {?s ?p ?n} ORDER BY ?n LIMIT 1",
                vec![vec![Some(value.clone())]],
            ),
        ] {
            check_profiles(query, &data, rows, capacity);
        }
    }
    // The largest accepted scale difference is relative to its comparison
    // partner. SECONDS constructs that decimal at runtime, after admission.
    for (scale, capacity) in [(38, false), (39, true)] {
        let tiny = format!("0.{}1", "0".repeat(scale - 1));
        let query = format!("PREFIX xsd:<http://www.w3.org/2001/XMLSchema#> SELECT (COALESCE(\"{tiny}\"^^xsd:decimal < 1, false) AS ?v) {{}}");
        check_profiles(
            &query,
            "",
            vec![vec![Some(Literal::from(true).into())]],
            capacity,
        );
        // Comparing SECONDS with 0.0 permits one additional fractional digit.
        let fraction = format!("0.{}1", "0".repeat(scale));
        let query = format!("PREFIX xsd:<http://www.w3.org/2001/XMLSchema#> SELECT (COALESCE(SECONDS(\"2024-01-01T00:00:0{fraction}Z\"^^xsd:dateTime) = 0.0, true) AS ?v) {{}}");
        check_profiles(
            &query,
            "",
            vec![vec![Some(Literal::from(false).into())]],
            capacity,
        );
    }
    // Lenient comparison stays exact even when adjacent values share an f64
    // image and the larger decimal is outside the i128 operand lane.
    for (decimal, capacity, less) in [
        ("170141183460469231731687303715884105727", false, false),
        ("170141183460469231731687303715884105728", true, true),
    ] {
        let query = format!("PREFIX xsd:<http://www.w3.org/2001/XMLSchema#> SELECT (\"170141183460469231731687303715884105727\"^^xsd:decimal < \"{decimal}\"^^xsd:decimal AS ?v) {{}}");
        check_profiles(
            &query,
            "",
            vec![vec![Some(Literal::from(less).into())]],
            capacity,
        );
    }
}

fn check_multirow_comparator(query: &str, selected: &[usize]) {
    // Each operand fits the decimal lane. Comparing them requires aligning the
    // large mantissa, so even distinct f64 images must observe strict capacity.
    let large: Term = Literal::new_typed_literal("100000000000000000000.1", xsd::DECIMAL).into();
    for (scale, capacity) in [(18, false), (19, true), (38, true)] {
        let tiny: Term =
            Literal::new_typed_literal(format!("0.{}7", "0".repeat(scale - 1)), xsd::DECIMAL)
                .into();
        for values in [[&tiny, &large], [&large, &tiny]] {
            let data = format!(
                "<http://ex/a> <http://ex/p> {} .\n<http://ex/b> <http://ex/p> {} .",
                values[0], values[1]
            );
            let ordered = [&tiny, &large];
            let rows = selected
                .iter()
                .map(|&index| vec![Some(ordered[index].clone())])
                .collect();
            check_profiles(query, &data, rows, capacity);
        }
    }
}

#[test]
fn strict_numeric_snapshot_bound_checks_multirow_order() {
    check_multirow_comparator("SELECT ?v {?s <http://ex/p> ?v} ORDER BY ?v", &[0, 1]);
}

#[test]
fn strict_numeric_snapshot_bound_checks_multirow_topk() {
    check_multirow_comparator(
        "SELECT ?v {?s <http://ex/p> ?v} ORDER BY DESC(?v) LIMIT 1",
        &[1],
    );
}

#[test]
fn strict_numeric_snapshot_bound_checks_multirow_min() {
    check_multirow_comparator("SELECT (MIN(?v) AS ?min) {?s <http://ex/p> ?v}", &[0]);
}
