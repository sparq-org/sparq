use oxrdf::{vocab::xsd, Literal, Term};
use sparq_core::Graph;
use sparq_engine::{
    construct_ntriples, json::to_sparql_json, query, query_json, query_json_chunks_with_budget,
    QueryBudget,
};

fn decimal(query_text: &str, expected: &str) {
    let result = query(&Graph::new(), query_text).unwrap();
    assert_eq!(
        result.rows,
        vec![vec![Some(Term::Literal(Literal::new_typed_literal(
            expected,
            xsd::DECIMAL
        )))]],
        "{query_text}"
    );
}

#[test]
fn multiply_rule_trims_fractional_zeros() {
    decimal("SELECT (2.0 * 3.28084 AS ?x) {}", "6.56168");
}

#[test]
fn arithmetic_and_numeric_functions_trim_fractional_zeros() {
    for (expr, expected) in [
        ("1.50 + 0.00", "1.5"),
        ("2.50 - 1.50", "1.0"),
        ("2.0 * 50.00", "100.0"),
        ("1.00 / 2.0", "0.5"),
        ("4.00 / 2.0", "2.0"),
        ("1 / 2", "0.5"),
        ("-1.50 * 1.50", "-2.25"),
        ("-0.00 * 1.0", "0.0"),
        ("-0.00", "0.0"),
        ("ABS(-2.2500)", "2.25"),
        ("CEIL(2.50)", "3"),
        ("FLOOR(-2.50)", "-3"),
        ("ROUND(-0.25)", "0"),
        ("100000000000000000000000000000.00 + 0.0", "100000000000000000000000000000.0"),
        ("0.0000000000000000000000000001 * 0.1", "0.00000000000000000000000000001"),
        ("SECONDS(\"2026-01-01T00:00:06.5600Z\"^^xsd:dateTime)", "6.56"),
        ("SECONDS(\"2026-01-01T00:00:00Z\"^^xsd:dateTime)", "0"),
        ("SECONDS(\"2026-01-01T00:00:00.00000000000000000000000000000000000000001Z\"^^xsd:dateTime)", "0.00000000000000000000000000000000000000001"),
    ] {
        decimal(&format!("PREFIX xsd:<http://www.w3.org/2001/XMLSchema#> SELECT ({expr} AS ?x) {{}}"), expected);
    }
}

#[test]
fn sum_and_avg_trim_fractional_zeros() {
    for (aggregate, values, expected) in [
        ("SUM", "1.50 2.50", "4.0"),
        ("AVG", "1.50 2.50", "2.0"),
        ("SUM", "0.10 0.20", "0.3"),
        ("AVG", "0.10 0.20", "0.15"),
        ("SUM", "-1.50 -2.50", "-4.0"),
        ("AVG", "-1.50 -2.50", "-2.0"),
        ("SUM", "-0.00 0.00", "0.0"),
        ("AVG", "-0.00 0.00", "0.0"),
    ] {
        decimal(
            &format!("SELECT ({aggregate}(?v) AS ?x) {{ VALUES ?v {{ {values} }} }}"),
            expected,
        );
    }
}

#[test]
fn distinct_aggregates_use_computed_term_identity() {
    let g = Graph::new();
    for (expression, expected) in [("?v", 2), ("?v + 0.0", 1)] {
        let q =
            format!("SELECT (COUNT(DISTINCT {expression}) AS ?n) {{ VALUES ?v {{ 1.50 1.5 }} }}");
        assert_eq!(
            query(&g, &q).unwrap().rows,
            vec![vec![Some(Literal::from(expected).into())]],
            "{q}"
        );
    }
    for aggregate in ["SUM", "AVG"] {
        decimal(
            &format!(
                "SELECT ({aggregate}(DISTINCT (?v + 0.0)) AS ?x) {{ VALUES ?v {{ 1.50 1.5 }} }}"
            ),
            "1.5",
        );
    }
}

#[test]
fn decimal_casts_keep_their_source_type_spellings() {
    for (source, expected) in [
        ("0", "0"),
        ("100", "100.0"),
        ("-2", "-2.0"),
        ("true", "1.0"),
        ("false", "0.0"),
        ("1.5000", "1.5000"),
        ("-0.00", "0.00"),
        ("\"+001.5000\"", "1.5"),
        ("\"-.5000\"", "-0.5"),
        ("\"-0.000\"", "0.0"),
        ("\"100\"", "100.0"),
        ("1.5e0", "1.5"),
        ("-0e0", "-0.0"),
        ("1e20", "100000000000000000000"),
        ("1e-20", "0.00000000000000000001"),
        ("\"-0\"^^xsd:float", "-0.0"),
        ("\"0.5\"^^xsd:float", "0.5"),
    ] {
        decimal(&format!("PREFIX xsd:<http://www.w3.org/2001/XMLSchema#> SELECT (xsd:decimal({source}) AS ?x) {{}}"), expected);
    }
}

#[test]
fn asserted_and_query_literals_and_selected_aggregate_terms_keep_their_spelling() {
    let g = Graph::load_str(
        "<urn:s> <urn:p> \"1.50\"^^<http://www.w3.org/2001/XMLSchema#decimal> .",
        "turtle",
    )
    .unwrap();
    let literal = Some(Term::Literal(Literal::new_typed_literal(
        "1.50",
        xsd::DECIMAL,
    )));
    for text in [
        "SELECT ?x { ?s ?p ?x }",
        "SELECT ?x { VALUES ?x { 1.50 } }",
        "SELECT (1.50 AS ?x) {}",
        "SELECT (+1.50 AS ?x) {}",
        "SELECT (MIN(?v) AS ?x) { ?s ?p ?v }",
        "SELECT (MAX(?v) AS ?x) { ?s ?p ?v }",
        "SELECT (SAMPLE(?v) AS ?x) { ?s ?p ?v }",
    ] {
        assert_eq!(
            query(&g, text).unwrap().rows,
            vec![vec![literal.clone()]],
            "{text}"
        );
    }
    assert_eq!(
        query(&g, "SELECT (STR(?x) AS ?lexical) { ?s ?p ?x }")
            .unwrap()
            .rows,
        vec![vec![Some(Literal::new_simple_literal("1.50").into())]]
    );
    // The computed term must resolve to the trimmed dictionary entry, rather
    // than an equal-valued asserted term with a different spelling.
    let g = Graph::load_str("<urn:a> <urn:p> 1.50 . <urn:b> <urn:p> 1.5 .", "turtle").unwrap();
    let r = query(&g, "SELECT ?s { BIND(1.50 + 0.0 AS ?x) ?s <urn:p> ?x }").unwrap();
    assert_eq!(
        r.rows,
        vec![vec![Some(oxrdf::NamedNode::new("urn:b").unwrap().into())]]
    );
}

#[test]
fn string_observation_and_result_writers_preserve_the_computed_asserted_boundary() {
    let g = Graph::new();
    let q = "SELECT (STR(2.0 * 3.280840) AS ?computed) (STR(1.50) AS ?asserted) {}";
    let r = query(&g, q).unwrap();
    assert_eq!(
        r.rows,
        vec![vec![
            Some(Literal::new_simple_literal("6.56168").into()),
            Some(Literal::new_simple_literal("1.50").into())
        ]]
    );
    let q = "SELECT (2.0 * 3.280840 AS ?computed) (1.50 AS ?asserted) {}";
    let json = query_json(&g, q).unwrap();
    assert_eq!(json, to_sparql_json(&query(&g, q).unwrap()));
    assert!(json.contains("\"value\":\"6.56168\""), "{json}");
    assert!(json.contains("\"value\":\"1.50\""), "{json}");
    assert_eq!(
        json,
        query_json_chunks_with_budget(&g, q, &QueryBudget::unlimited())
            .unwrap()
            .concat()
    );
    let nt = construct_ntriples(&g, "CONSTRUCT { <urn:s> <urn:computed> ?x ; <urn:asserted> 1.50 } WHERE { BIND(2.0 * 3.28084 AS ?x) }").unwrap();
    assert!(
        nt.contains("\"6.56168\"^^<http://www.w3.org/2001/XMLSchema#decimal>"),
        "{nt}"
    );
    assert!(
        nt.contains("\"1.50\"^^<http://www.w3.org/2001/XMLSchema#decimal>"),
        "{nt}"
    );
}

// These data/query shapes mirror the approved expr-ops fixtures. Bare decimal
// inputs have scale zero; adding a point would change the expected RDF terms.
#[test]
fn scale_zero_arithmetic_preserves_fixture_spellings() {
    let g = Graph::load_str(
        r#"@prefix : <http://example.org/> .
           @prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
           :integer :p "3"^^xsd:integer . :decimal :p "3"^^xsd:decimal .
           :float :p "3"^^xsd:float . :double :p "3"^^xsd:double ."#,
        "turtle",
    )
    .unwrap();
    for (op, expected) in [("+", "6"), ("-", "0"), ("*", "9")] {
        let q = format!("PREFIX : <http://example.org/> SELECT ?left ?right (?left {op} ?right AS ?result) WHERE {{ _:l :p ?left . _:r :p ?right . }}");
        let r = query(&g, &q).unwrap();
        let decimals: Vec<_> = r
            .rows
            .iter()
            .filter_map(|row| match &row[2] {
                Some(Term::Literal(l)) if l.datatype() == xsd::DECIMAL => Some(l.value()),
                _ => None,
            })
            .collect();
        assert_eq!(decimals, vec![expected; 3], "{q}");
    }
    let r = query(
        &g,
        "PREFIX : <http://example.org/> SELECT ?v (-?v AS ?result) { _:l :p ?v }",
    )
    .unwrap();
    let decimal_row = r
        .rows
        .iter()
        .find(|row| row[0] == Some(Literal::new_typed_literal("3", xsd::DECIMAL).into()))
        .unwrap();
    assert_eq!(
        decimal_row[1],
        Some(Literal::new_typed_literal("-3", xsd::DECIMAL).into())
    );
}

#[test]
fn numeric_functions_preserve_fixture_spellings() {
    let g = Graph::load_str(
        r#"@prefix : <http://example.org/> .
           @prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
           :n5 :num 2.5 .
           :d1 :date "2010-06-21T11:28:01Z"^^xsd:dateTime .
           :d2 :date "2010-06-21T11:28:02Z"^^xsd:dateTime ."#,
        "turtle",
    )
    .unwrap();
    for (function, expected) in [("CEIL", "3"), ("FLOOR", "2"), ("ROUND", "3")] {
        let q = format!("PREFIX : <http://example.org/> SELECT ?s ?num ({function}(?num) AS ?result) {{ ?s :num ?num }}");
        assert_eq!(
            query(&g, &q).unwrap().rows[0][2],
            Some(Literal::new_typed_literal(expected, xsd::DECIMAL).into()),
            "{q}"
        );
    }
    let r = query(&g, "PREFIX : <http://example.org/> SELECT ?s (SECONDS(?date) AS ?x) { ?s :date ?date } ORDER BY ?s").unwrap();
    for (row, expected) in r.rows.iter().zip(["1", "2"]) {
        assert_eq!(
            row[1],
            Some(Literal::new_typed_literal(expected, xsd::DECIMAL).into())
        );
    }
    assert_eq!(r.rows.len(), 2);
}

#[test]
fn division_keeps_first_fractional_digit() {
    // coalesce01's division shape, with integral operands and an OPTIONAL RHS.
    let g = Graph::load_str(
        "<urn:a> <urn:p> 0 ; <urn:q> 2 . <urn:b> <urn:p> 4 ; <urn:q> 2 .",
        "turtle",
    )
    .unwrap();
    let r = query(&g, "SELECT (COALESCE(?o / ?x, -2) AS ?div) { ?s <urn:p> ?o OPTIONAL { ?s <urn:q> ?x } } ORDER BY ?s").unwrap();
    assert_eq!(
        r.rows,
        vec![
            vec![Some(Literal::new_typed_literal("0.0", xsd::DECIMAL).into())],
            vec![Some(Literal::new_typed_literal("2.0", xsd::DECIMAL).into())]
        ]
    );
    decimal("SELECT (4 / 2 AS ?x) {}", "2.0");
}

#[test]
fn fractional_inputs_keep_plus_fixture_spellings() {
    let g = Graph::load_str("<urn:s> <urn:p> 1.0 ; <urn:q> 2 .", "turtle").unwrap();
    let r = query(
        &g,
        "SELECT ?x ?y (?x + ?y AS ?sum) { ?s <urn:p> ?x ; <urn:q> ?y }",
    )
    .unwrap();
    assert_eq!(
        r.rows,
        vec![vec![
            Some(Literal::new_typed_literal("1.0", xsd::DECIMAL).into()),
            Some(Literal::from(2).into()),
            Some(Literal::new_typed_literal("3.0", xsd::DECIMAL).into())
        ]]
    );
    // plus-2's STR addition is an error, with the input decimal echoed verbatim.
    let r = query(
        &g,
        "SELECT ?x ?y (STR(?x) + STR(?y) AS ?sum) { ?s <urn:p> ?x ; <urn:q> ?y }",
    )
    .unwrap();
    assert_eq!(
        r.rows,
        vec![vec![
            Some(Literal::new_typed_literal("1.0", xsd::DECIMAL).into()),
            Some(Literal::from(2).into()),
            None
        ]]
    );
}

#[test]
fn integer_and_string_zero_casts_keep_distinct_fixture_spellings() {
    for (source, expected) in [("0", "0"), ("\"0\"", "0.0")] {
        decimal(&format!("PREFIX xsd:<http://www.w3.org/2001/XMLSchema#> SELECT (xsd:decimal({source}) AS ?x) {{}}"), expected);
    }
    let g = Graph::load_str("<urn:n01> <urn:p> 0 . <urn:s06> <urn:p> \"0\" .", "turtle").unwrap();
    let r = query(&g, "PREFIX xsd:<http://www.w3.org/2001/XMLSchema#> SELECT (xsd:decimal(?v) AS ?decimal) { ?a <urn:p> ?v } ORDER BY ?a").unwrap();
    assert_eq!(
        r.rows,
        vec![
            vec![Some(Literal::new_typed_literal("0", xsd::DECIMAL).into())],
            vec![Some(Literal::new_typed_literal("0.0", xsd::DECIMAL).into())]
        ]
    );
}
