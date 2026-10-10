use super::*;

/// A non-empty graph whose triples never match the test queries' predicate, so the
/// WHERE clause produces an EMPTY multiset while the store itself is non-trivial
/// (rules out "0 rows because the store is empty" as the cause).
fn g() -> Graph {
    Graph::load_str("@prefix ex: <http://ex/> . ex:keep ex:other 1 .", "turtle").unwrap()
}

fn run(q: &str) -> QueryResult {
    crate::query(&g(), &format!("PREFIX ex: <http://ex/> {q}")).unwrap()
}

/// The single cell of a one-row, one-projected-variable result.
fn one_cell(q: &str) -> Option<Term> {
    let r = run(q);
    assert_eq!(r.rows.len(), 1, "expected exactly one solution row for: {q}");
    assert_eq!(r.rows[0].len(), 1, "expected exactly one projected cell for: {q}");
    r.rows[0][0].clone()
}

fn int_lit(n: i64) -> Term {
    Term::Literal(oxrdf::Literal::new_typed_literal(n.to_string(), oxrdf::vocab::xsd::INTEGER))
}

// ---- No GROUP BY over an empty match: ONE row, COUNT=0, others per the split above ----

#[test]
fn count_star_over_empty_is_zero_one_row() {
    assert_eq!(one_cell("SELECT (COUNT(*) AS ?v) WHERE { ?s ex:nomatch ?o }"), Some(int_lit(0)));
}

#[test]
fn count_var_over_empty_is_zero_one_row() {
    // COUNT(?x) and COUNT(*) coincide over an empty multiset: both 0.
    assert_eq!(one_cell("SELECT (COUNT(?o) AS ?v) WHERE { ?s ex:nomatch ?o }"), Some(int_lit(0)));
}

#[test]
fn count_star_vs_count_var_agree_over_empty() {
    let star = one_cell("SELECT (COUNT(*) AS ?v) WHERE { ?s ex:nomatch ?o }");
    let var = one_cell("SELECT (COUNT(?o) AS ?v) WHERE { ?s ex:nomatch ?o }");
    assert_eq!(star, var);
    assert_eq!(star, Some(int_lit(0)));
}

#[test]
fn sum_over_empty_is_integer_zero_one_row() {
    // §18.5.1.4: Sum({}) = "0"^^xsd:integer — a BOUND 0, NOT unbound. This is the cell
    // PSS usage() reads (directly, or via COALESCE(SUM(?size), 0) which still sees 0).
    assert_eq!(one_cell("SELECT (SUM(?o) AS ?v) WHERE { ?s ex:nomatch ?o }"), Some(int_lit(0)));
}

#[test]
fn avg_over_empty_is_integer_zero_one_row() {
    // §18.5.1.5: Avg({}) = "0"^^xsd:integer.
    assert_eq!(one_cell("SELECT (AVG(?o) AS ?v) WHERE { ?s ex:nomatch ?o }"), Some(int_lit(0)));
}

#[test]
fn min_over_empty_is_unbound_one_row() {
    // §18.5.1.6: Min over {} errors ⇒ unbound cell, but still ONE row.
    assert_eq!(one_cell("SELECT (MIN(?o) AS ?v) WHERE { ?s ex:nomatch ?o }"), None);
}

#[test]
fn max_over_empty_is_unbound_one_row() {
    // §18.5.1.7. Mirrors the W3C `agg-empty-group-max-2` conformance test.
    assert_eq!(one_cell("SELECT (MAX(?o) AS ?v) WHERE { ?s ex:nomatch ?o }"), None);
}

#[test]
fn sample_over_empty_is_unbound_one_row() {
    // §18.5.1.8: no element to sample ⇒ unbound cell, ONE row.
    assert_eq!(one_cell("SELECT (SAMPLE(?o) AS ?v) WHERE { ?s ex:nomatch ?o }"), None);
}

#[test]
fn group_concat_over_empty_is_empty_string_one_row() {
    // §18.5.1.9: concatenation of zero strings is the empty simple literal "".
    assert_eq!(
        one_cell("SELECT (GROUP_CONCAT(?o) AS ?v) WHERE { ?s ex:nomatch ?o }"),
        Some(Term::Literal(oxrdf::Literal::new_simple_literal("")))
    );
}

#[test]
fn group_concat_with_separator_over_empty_is_empty_string() {
    // A custom separator does not change the empty-input result: still "".
    assert_eq!(
        one_cell("SELECT (GROUP_CONCAT(?o; SEPARATOR=\",\") AS ?v) WHERE { ?s ex:nomatch ?o }"),
        Some(Term::Literal(oxrdf::Literal::new_simple_literal("")))
    );
}

/// The PSS `usage()` shape end-to-end: several aggregates in one SELECT over an empty
/// match must yield a SINGLE row whose COUNT is 0 and SUM is 0 (so `{resourceCount:0,
/// bytes:0}` falls out of that one row), with no extra/zero rows.
#[test]
fn mixed_aggregates_no_group_by_yield_one_row() {
    let r = run("SELECT (COUNT(?o) AS ?n) (SUM(?o) AS ?total) WHERE { ?s ex:nomatch ?o }");
    assert_eq!(r.vars.iter().map(|v| v.as_str()).collect::<Vec<_>>(), ["n", "total"]);
    assert_eq!(r.rows.len(), 1, "aggregates with no GROUP BY over empty must be ONE row");
    assert_eq!(r.rows[0], vec![Some(int_lit(0)), Some(int_lit(0))]);
}

// ---- WITH GROUP BY over empty input: ZERO groups ⇒ ZERO rows ----

#[test]
fn count_star_with_group_by_over_empty_is_zero_rows() {
    // Mirrors W3C `agg-empty-group-count-1`.
    let r = run("SELECT ?s (COUNT(*) AS ?v) WHERE { ?s ex:nomatch ?o } GROUP BY ?s");
    assert!(r.rows.is_empty(), "GROUP BY over empty input must yield zero groups (zero rows)");
}

#[test]
fn sum_with_group_by_over_empty_is_zero_rows() {
    let r = run("SELECT ?s (SUM(?o) AS ?v) WHERE { ?s ex:nomatch ?o } GROUP BY ?s");
    assert!(r.rows.is_empty());
}

#[test]
fn max_with_group_by_over_empty_is_zero_rows() {
    // Mirrors W3C `agg-empty-group-max-1`.
    let r = run("SELECT ?s (MAX(?o) AS ?v) WHERE { ?s ex:nomatch ?o } GROUP BY ?s");
    assert!(r.rows.is_empty());
}

/// The single-implicit-group rule must NOT fire when a GROUP BY is written, even if the
/// grouping key is a constant expression — empty input still means zero groups.
#[test]
fn group_by_constant_over_empty_is_zero_rows() {
    let r = run("SELECT ?k (COUNT(*) AS ?v) WHERE { ?s ex:nomatch ?o } GROUP BY (1 AS ?k)");
    assert!(r.rows.is_empty());
}
