//! #3902: `xsd:dateTimeStamp` requires a timezone (XSD 1.1 §3.4.28), so a timezone-free
//! lexical of that datatype is ill-formed and every comparison on it is a type error. The
//! pushed-down scan (load-time temporal cache) and the general expression path must agree.

use sparq_core::Graph;
use sparq_engine::query;

const DATA: &str = r#"
@prefix : <http://example.org/> .
@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
:floatingStamp :t "2020-01-01T00:00:00"^^xsd:dateTimeStamp .
:zonedStamp    :t "2020-01-01T00:00:00Z"^^xsd:dateTimeStamp .
:floatingDt    :t "2020-01-01T00:00:00"^^xsd:dateTime .
"#;

const PREFIXES: &str =
    "PREFIX : <http://example.org/> PREFIX xsd: <http://www.w3.org/2001/XMLSchema#> ";

fn subjects(q: &str) -> Vec<String> {
    let g = Graph::load_str(DATA, "turtle").unwrap();
    let r = query(&g, &format!("{PREFIXES}{q}")).unwrap();
    let mut out: Vec<String> = r.rows.iter().map(|row| row[0].as_ref().unwrap().to_string()).collect();
    out.sort();
    out
}

#[test]
fn pushed_down_filter_drops_a_timezone_free_datetimestamp() {
    let want = vec!["<http://example.org/floatingDt>".to_string(), "<http://example.org/zonedStamp>".to_string()];
    assert_eq!(subjects(r#"SELECT ?s { ?s :t ?d FILTER(?d < "2021-01-01T00:00:00Z"^^xsd:dateTime) }"#), want);
    assert_eq!(subjects(r#"SELECT ?s { ?s :t ?d FILTER(?d >= "2019-01-01T00:00:00Z"^^xsd:dateTime) }"#), want);
}

#[test]
fn general_comparison_of_a_timezone_free_datetimestamp_is_a_type_error() {
    let one = |e: &str| {
        let g = Graph::load_str("", "turtle").unwrap();
        let r = query(&g, &format!("{PREFIXES}SELECT ?x {{ BIND(({e}) AS ?x) }}")).unwrap();
        r.rows[0][0].as_ref().map(|t| t.to_string())
    };
    assert_eq!(one(r#""2020-01-01T00:00:00"^^xsd:dateTimeStamp < "2021-01-01T00:00:00Z"^^xsd:dateTime"#), None);
    assert_eq!(one(r#""2020-01-01T00:00:00"^^xsd:dateTimeStamp != "2021-01-01T00:00:00Z"^^xsd:dateTime"#), None);
    let t = Some("\"true\"^^<http://www.w3.org/2001/XMLSchema#boolean>".to_string());
    assert_eq!(one(r#""2020-01-01T00:00:00Z"^^xsd:dateTimeStamp < "2021-01-01T00:00:00Z"^^xsd:dateTime"#), t);
    assert_eq!(one(r#""2020-01-01T00:00:00"^^xsd:dateTime < "2021-01-01T00:00:00Z"^^xsd:dateTime"#), t);
}

#[test]
fn ordering_through_the_scan_and_the_expression_path_agree() {
    // A variable-to-variable comparison takes the per-row path; it must drop the same row.
    let want = vec!["<http://example.org/floatingDt>".to_string(), "<http://example.org/zonedStamp>".to_string()];
    assert_eq!(
        subjects(r#"SELECT ?s { ?s :t ?d VALUES ?c { "2021-01-01T00:00:00Z"^^xsd:dateTime } FILTER(?d < ?c) }"#),
        want
    );
}
