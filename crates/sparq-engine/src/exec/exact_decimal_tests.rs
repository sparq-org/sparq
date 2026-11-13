use super::*;

fn decimal_term(lexical: &str) -> Term {
    Term::Literal(Literal::new_typed_literal(lexical, xsd::DECIMAL))
}

fn decimal_graph(lexical: &str) -> Graph {
    Graph::load_str(
        &format!("<urn:s> <urn:v> \"{lexical}\"^^<http://www.w3.org/2001/XMLSchema#decimal> ."),
        "ntriples",
    )
    .unwrap()
}

fn decimal_filter_bind_result(graph: &Graph, expression: &str) -> Option<bool> {
    let bound = crate::query(
        graph,
        &format!("SELECT ?b WHERE {{ ?s <urn:v> ?v BIND({expression} AS ?b) }}"),
    )
    .unwrap();
    assert_eq!(bound.rows.len(), 1);
    let result = match &bound.rows[0][0] {
        Some(term) if term == &Term::Literal(Literal::from(true)) => Some(true),
        Some(term) if term == &Term::Literal(Literal::from(false)) => Some(false),
        None => None,
        other => panic!("unexpected comparison result {other:?}"),
    };
    let query = format!("SELECT ?s WHERE {{ ?s <urn:v> ?v FILTER({expression}) }}");
    let expected = usize::from(result == Some(true));
    assert_eq!(
        crate::query(graph, &query).unwrap().rows.len(),
        expected,
        "FILTER {expression}"
    );
    assert_eq!(
        crate::count(graph, &query).unwrap(),
        expected,
        "count FILTER {expression}"
    );
    result
}

fn assert_decimal_filter_bind(graph: &Graph, expression: &str, expected: bool) {
    assert_eq!(
        decimal_filter_bind_result(graph, expression),
        Some(expected),
        "BIND {expression}"
    );
}

#[test]
fn decimal_underflow_filter_agrees_with_bind() {
    let tiny = format!("0.{}1", "0".repeat(324));
    assert_decimal_filter_bind(&decimal_graph(&tiny), "?v > 0", true);
    assert_decimal_filter_bind(&decimal_graph(&format!("-{tiny}")), "?v < 0", true);
}

#[test]
fn decimal_underflow_constant_agrees_with_bind() {
    let graph = decimal_graph("0.0");
    for sign in ["", "-"] {
        let tiny = format!("{sign}0.{}1", "0".repeat(324));
        let constant = format!("\"{tiny}\"^^<http://www.w3.org/2001/XMLSchema#decimal>");
        assert_decimal_filter_bind(&graph, &format!("?v = {constant}"), false);
        assert_decimal_filter_bind(&graph, &format!("?v < {constant}"), sign.is_empty());
        assert_decimal_filter_bind(&graph, &format!("{constant} < ?v"), !sign.is_empty());
    }
}

#[test]
fn decimal_gate_remains_correct_after_fork_and_update() {
    let parent = decimal_graph("0.0");
    assert!(!parent.has_decimal_needing_exact_comparison());
    let fork = parent.fork();
    assert!(!fork.has_decimal_needing_exact_comparison());
    let tiny = format!("0.{}1", "0".repeat(324));
    let updated = crate::update(
        &fork,
        &format!("INSERT DATA {{ <urn:tiny> <urn:v> \"{tiny}\"^^<http://www.w3.org/2001/XMLSchema#decimal> }}"),
    ).unwrap();
    assert!(updated.has_decimal_needing_exact_comparison());
    assert!(!parent.has_decimal_needing_exact_comparison());
    assert!(!fork.has_decimal_needing_exact_comparison());
    let query = "SELECT ?v WHERE { ?s <urn:v> ?v FILTER(?v > 0) }";
    assert_eq!(
        crate::query(&updated, query).unwrap().rows,
        vec![vec![Some(decimal_term(&tiny))]]
    );
    assert_eq!(crate::count(&updated, query).unwrap(), 1);
}

#[test]
fn decimal_subnormal_collision_agrees_with_bind() {
    let low = format!("0.{}10", "0".repeat(322));
    let high = format!("0.{}11", "0".repeat(322));
    assert_eq!(low.parse::<f64>().unwrap(), high.parse::<f64>().unwrap());
    let graph = decimal_graph(&low);
    let constant = format!("\"{high}\"^^<http://www.w3.org/2001/XMLSchema#decimal>");
    assert_decimal_filter_bind(&graph, &format!("?v = {constant}"), false);
    assert_decimal_filter_bind(&graph, &format!("?v < {constant}"), true);
}

#[test]
fn distinct_underflow_decimals_compare_and_sort_exactly() {
    let low = format!("0.{}1", "0".repeat(324));
    let high = format!("0.{}2", "0".repeat(324));
    let graph = decimal_graph(&low);
    let constant = format!("\"{high}\"^^<http://www.w3.org/2001/XMLSchema#decimal>");
    assert_decimal_filter_bind(&graph, &format!("?v = {constant}"), false);
    assert_decimal_filter_bind(&graph, &format!("?v < {constant}"), true);
    assert_decimal_filter_bind(&graph, &format!("?v > {constant}"), false);
    // Both variable operands also exercise the f64-tie recheck in the evaluator.
    let graph = Graph::load_str(
        &format!("<urn:b> <urn:v> {constant} .\n<urn:a> <urn:v> \"{low}\"^^<http://www.w3.org/2001/XMLSchema#decimal> ."),
        "ntriples",
    ).unwrap();
    let pair = crate::query(
        &graph,
        "SELECT ?x ?y WHERE { ?s <urn:v> ?x . ?t <urn:v> ?y FILTER(?x < ?y) }",
    )
    .unwrap();
    assert_eq!(
        pair.rows,
        vec![vec![Some(decimal_term(&low)), Some(decimal_term(&high))]]
    );
    let equal = crate::query(
        &graph,
        "SELECT ?x ?y WHERE { ?s <urn:v> ?x . ?t <urn:v> ?y FILTER(?x = ?y) }",
    )
    .unwrap();
    assert_eq!(equal.rows.len(), 2);
    let sorted = crate::query(&graph, "SELECT ?v WHERE { ?s <urn:v> ?v } ORDER BY ?v").unwrap();
    assert_eq!(
        sorted.rows,
        vec![
            vec![Some(decimal_term(&low))],
            vec![Some(decimal_term(&high))]
        ]
    );
    let extrema = crate::query(
        &graph,
        "SELECT (MIN(?v) AS ?lo) (MAX(?v) AS ?hi) WHERE { ?s <urn:v> ?v }",
    )
    .unwrap();
    assert_eq!(
        extrema.rows,
        vec![vec![Some(decimal_term(&low)), Some(decimal_term(&high))]]
    );
}

// A plain decimal coefficient times 10^exponent, without exponent notation.
fn scaled_decimal(coefficient: u64, exponent: i32, negative: bool) -> String {
    let digits = coefficient.to_string();
    let magnitude = if exponent >= 0 {
        format!("{digits}{}", "0".repeat(exponent as usize))
    } else {
        let scale = (-exponent) as usize;
        if scale < digits.len() {
            format!(
                "{}.{}",
                &digits[..digits.len() - scale],
                &digits[digits.len() - scale..]
            )
        } else {
            format!("0.{}{digits}", "0".repeat(scale - digits.len()))
        }
    };
    format!("{}{magnitude}", if negative { "-" } else { "" })
}

proptest::proptest! {
    #![proptest_config(proptest::test_runner::Config { cases: 256, failure_persistence: None, ..Default::default() })]
    #[test]
    fn decimal_filter_matches_exact_across_exponents(
        coefficient in 1_u64..1_000_000_000_000_000,
        exponent in -1100_i32..=310,
        negative in proptest::bool::ANY,
    ) {
        let lexical = scaled_decimal(coefficient, exponent, negative);
        let neighbor = scaled_decimal(coefficient + 1, exponent, negative);
        let graph = decimal_graph(&lexical);
        // Covers all binary64 exponents, gradual underflow, zero underflow and
        // overflow. BIND checks the exact evaluator independently of pushdown.
        for rhs in ["0", lexical.as_str(), neighbor.as_str()] {
            let order = cmp_decimal_str(&lexical, rhs).unwrap();
            let constant = format!("\"{rhs}\"^^<http://www.w3.org/2001/XMLSchema#decimal>");
            for (op, expected) in [
                ("=", order == Ordering::Equal),
                ("<", order == Ordering::Less),
                (">", order == Ordering::Greater),
                ("<=", order != Ordering::Greater),
                (">=", order != Ordering::Less),
            ] {
                let result = decimal_filter_bind_result(&graph, &format!("?v {op} {constant}"));
                // Beyond arithmetic capacity, the property still checks FILTER/BIND
                // agreement, including unbound/error results. Within capacity it also
                // checks the arbitrary-scale lexical comparison oracle.
                if Num::of_parts(&lexical, xsd::DECIMAL.as_str()).is_some()
                    && Num::of_parts(rhs, xsd::DECIMAL.as_str()).is_some()
                {
                    proptest::prop_assert_eq!(result, Some(expected));
                }
            }
        }
    }
}

#[test]
fn dec_arithmetic_is_exact() {
    use std::cmp::Ordering::*;
    let d = |s: &str| Dec::parse(s).unwrap();
    // The classic: 0.1 + 0.2 == 0.3 exactly (false in f64).
    assert_eq!(d("0.1").checked_add(d("0.2")).unwrap().cmp(d("0.3")), Some(Equal));
    // 0.3 - 0.1 == 0.2 exactly.
    assert_eq!(d("0.3").checked_sub(d("0.1")).unwrap().cmp(d("0.2")), Some(Equal));
    // Mixed scales + integer/decimal.
    assert_eq!(d("1").checked_add(d("0.5")).unwrap().cmp(d("1.5")), Some(Equal));
    assert_eq!(d("83").checked_mul(d("0.5")).unwrap().cmp(d("41.5")), Some(Equal));
    assert_eq!(d("0.1").checked_mul(d("0.1")).unwrap().cmp(d("0.01")), Some(Equal));
    // Ordering across scales and signs.
    assert_eq!(d("0.30000000000000001").cmp(d("0.3")), Some(Greater));
    assert_eq!(d("-2.5").checked_add(d("1")).unwrap().cmp(d("-1.5")), Some(Equal));
    // Large integers beyond 2^53 stay exact.
    assert_eq!(d("9007199254740992").checked_add(d("1")).unwrap().cmp(d("9007199254740993")), Some(Equal));
    // Non-decimal lexical (exponent) -> not parseable here -> None.
    assert!(Dec::parse("1e5").is_none());
}

#[test]
fn cmp_decimal_str_is_exact() {
    use std::cmp::Ordering::*;
    // Integers beyond f64 precision, and high-precision decimals that share an f64.
    assert_eq!(cmp_decimal_str("9007199254740992", "9007199254740993"), Some(Less));
    assert_eq!(cmp_decimal_str("0.123456789012345678", "0.123456789012345679"), Some(Less));
    // Equality incl. non-canonical zeros / trailing zeros / signed zero.
    assert_eq!(cmp_decimal_str("1.50", "1.5"), Some(Equal));
    assert_eq!(cmp_decimal_str("007", "7"), Some(Equal));
    assert_eq!(cmp_decimal_str("-0", "0"), Some(Equal));
    assert_eq!(cmp_decimal_str("0.0", "-0.0"), Some(Equal));
    // Sign + magnitude.
    assert_eq!(cmp_decimal_str("-3", "2"), Some(Less));
    assert_eq!(cmp_decimal_str("-2", "-3"), Some(Greater));
    assert_eq!(cmp_decimal_str("10", "9"), Some(Greater));
    assert_eq!(cmp_decimal_str("1.1", "1.09"), Some(Greater));
    assert_eq!(cmp_decimal_str("0.1", "0.2"), Some(Less));
    // Integer vs decimal of equal value.
    assert_eq!(cmp_decimal_str("5", "5.0"), Some(Equal));
    assert_eq!(cmp_decimal_str("5", "5.0000000000000001"), Some(Less));
    // Malformed -> None (falls back to f64).
    assert_eq!(cmp_decimal_str("1.2.3", "1"), None);
    assert_eq!(cmp_decimal_str("1e5", "1"), None);
}
