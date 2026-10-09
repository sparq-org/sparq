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

fn eval(e: &str) -> Option<String> {
    let g = Graph::load_str("", "turtle").unwrap();
    let r = query(&g, &format!("{PREFIXES}SELECT ?x {{ BIND(({e}) AS ?x) }}")).unwrap();
    r.rows[0][0].as_ref().map(|t| t.to_string())
}

#[test]
fn accessors_and_casts_reject_a_timezone_free_datetimestamp() {
    let bad = r#""2020-01-01T00:00:00"^^xsd:dateTimeStamp"#;
    for f in ["YEAR", "MONTH", "DAY", "HOURS", "MINUTES", "SECONDS", "TZ", "TIMEZONE", "xsd:dateTime"] {
        assert_eq!(eval(&format!("{f}({bad})")), None, "{f} of a timezone-free dateTimeStamp");
    }
    let good = r#""2020-01-01T00:00:00Z"^^xsd:dateTimeStamp"#;
    assert_eq!(eval(&format!("YEAR({good})")).as_deref(), Some("\"2020\"^^<http://www.w3.org/2001/XMLSchema#integer>"));
    assert_eq!(eval(&format!("TZ({good})")).as_deref(), Some("\"Z\""));
    assert_eq!(
        eval(&format!("xsd:dateTime({good})")).as_deref(),
        Some("\"2020-01-01T00:00:00Z\"^^<http://www.w3.org/2001/XMLSchema#dateTime>")
    );
    // A timezone-free xsd:dateTime is still fine.
    assert_eq!(
        eval(r#"YEAR("2020-01-01T00:00:00"^^xsd:dateTime)"#).as_deref(),
        Some("\"2020\"^^<http://www.w3.org/2001/XMLSchema#integer>")
    );
    assert_eq!(eval(r#"TZ("2020-01-01T00:00:00"^^xsd:dateTime)"#).as_deref(), Some("\"\""));
}

#[test]
fn cross_family_equality_with_a_timezone_free_datetimestamp_is_a_type_error() {
    let f = Some("\"false\"^^<http://www.w3.org/2001/XMLSchema#boolean>".to_string());
    let t = Some("\"true\"^^<http://www.w3.org/2001/XMLSchema#boolean>".to_string());
    assert_eq!(eval(r#""2020-01-01T00:00:00"^^xsd:dateTimeStamp = "2020-01-01"^^xsd:date"#), None);
    assert_eq!(eval(r#""2020-01-01T00:00:00"^^xsd:dateTimeStamp != "2020-01-01"^^xsd:date"#), None);
    // Nor is it known different from a language-tagged literal.
    assert_eq!(eval(r#""2020-01-01T00:00:00"^^xsd:dateTimeStamp != "text"@en"#), None);
    assert_eq!(eval(r#""text"@en = "2020-01-01T00:00:00"^^xsd:dateTimeStamp"#), None);
    assert_eq!(eval(r#""2020-01-01T00:00:00Z"^^xsd:dateTimeStamp != "text"@en"#), t);
    // Well-formed dateTime vs date values stay known-different.
    assert_eq!(eval(r#""2020-01-01T00:00:00Z"^^xsd:dateTimeStamp = "2020-01-01"^^xsd:date"#), f);
    assert_eq!(eval(r#""2020-01-01T00:00:00"^^xsd:dateTime != "2020-01-01"^^xsd:date"#), t);
    // Identical terms are still equal (sameTerm shortcut).
    assert_eq!(
        eval(r#""2020-01-01T00:00:00"^^xsd:dateTimeStamp = "2020-01-01T00:00:00"^^xsd:dateTimeStamp"#),
        t
    );
    // The FILTER form drops the row rather than keeping it.
    assert!(subjects(r#"SELECT ?s { ?s :t ?d FILTER(?d != "2020-01-01"^^xsd:date) }"#)
        .iter()
        .all(|s| !s.contains("floatingStamp")));
    assert!(subjects(r#"SELECT ?s { ?s :t ?d FILTER(YEAR(?d) = 2020) }"#).iter().all(|s| !s.contains("floatingStamp")));
}

#[test]
fn a_far_out_year_does_not_overflow() {
    // A year past the i64-seconds range: whatever the accessors return, they must not panic.
    let _ = eval(r#"YEAR("1000000000000-01-01T00:00:00Z"^^xsd:dateTimeStamp)"#);
    let _ = eval(r#"TZ("1000000000000-01-01T00:00:00Z"^^xsd:dateTimeStamp)"#);
    assert_eq!(eval(r#"YEAR("1000000000000-01-01T00:00:00"^^xsd:dateTimeStamp)"#), None);
    // Comparing it is a type error (no representable instant), not a panic.
    assert_eq!(
        eval(r#""1000000000000-01-01T00:00:00Z"^^xsd:dateTimeStamp < "2021-01-01T00:00:00Z"^^xsd:dateTime"#),
        None
    );
}
