use super::*;

/// A non-empty graph: three `ex:a`-typed subjects. The WHERE clause below matches all three,
/// so the input multiset is non-empty — but `?kind` is never bound.
fn g() -> Graph {
    Graph::load_str(
        "@prefix ex: <http://ex/> . ex:x1 a ex:Thing . ex:x2 a ex:Thing . ex:x3 a ex:Thing .",
        "turtle",
    )
    .unwrap()
}

fn run(q: &str) -> QueryResult {
    crate::query(&g(), &format!("PREFIX ex: <http://ex/> {q}")).unwrap()
}

fn int_lit(n: i64) -> Term {
    Term::Literal(oxrdf::Literal::new_typed_literal(n.to_string(), oxrdf::vocab::xsd::INTEGER))
}

/// The bead's exact shape: GROUP BY an unbound var with a COUNT(DISTINCT). One group, the
/// grouping var unbound, the count over the whole matched set. Must not panic.
#[test]
fn group_by_never_bound_var_is_one_unbound_group() {
    let r = run("SELECT ?kind (COUNT(DISTINCT ?x) AS ?n) WHERE { ?x a ex:Thing } GROUP BY ?kind");
    assert_eq!(r.vars.iter().map(|v| v.as_str()).collect::<Vec<_>>(), ["kind", "n"]);
    assert_eq!(r.rows.len(), 1, "all rows share the unbound key ⇒ exactly one group");
    // `?kind` unbound, `?n` = number of distinct ?x (3).
    assert_eq!(r.rows[0], vec![None, Some(int_lit(3))]);
}

/// COUNT(*) variant — the count is the size of the (single) group, i.e. all matched rows.
#[test]
fn group_by_never_bound_var_count_star() {
    let r = run("SELECT ?kind (COUNT(*) AS ?n) WHERE { ?x a ex:Thing } GROUP BY ?kind");
    assert_eq!(r.rows.len(), 1);
    assert_eq!(r.rows[0], vec![None, Some(int_lit(3))]);
}

/// A BOUND grouping var mixed with a NEVER-bound one: grouping still partitions by the bound
/// var (here all rows share `?x`'s type-less subject so… group by the bound `?x`), and the
/// unbound `?kind` is unbound in every output row. Three distinct subjects ⇒ three groups.
#[test]
fn mixed_bound_and_unbound_group_vars() {
    let r = run("SELECT ?x ?kind (COUNT(*) AS ?n) WHERE { ?x a ex:Thing } GROUP BY ?x ?kind");
    assert_eq!(r.vars.iter().map(|v| v.as_str()).collect::<Vec<_>>(), ["x", "kind", "n"]);
    assert_eq!(r.rows.len(), 3, "three distinct ?x ⇒ three groups");
    // Every row has `?kind` (the 2nd cell) unbound and a per-subject count of 1.
    for row in &r.rows {
        assert_eq!(row[1], None, "the never-bound ?kind must be unbound");
        assert_eq!(row[2], Some(int_lit(1)));
    }
}

/// SUM over a never-bound grouping var: one group, the unbound key, SUM is unbound (no
/// numeric members) — and crucially, no panic.
#[test]
fn group_by_never_bound_var_with_sum() {
    let r = run("SELECT ?kind (SUM(?missing) AS ?total) WHERE { ?x a ex:Thing } GROUP BY ?kind");
    assert_eq!(r.rows.len(), 1);
    assert_eq!(r.rows[0][0], None, "?kind unbound");
    // SUM over an all-unbound column is "0"^^xsd:integer (Sum({}) = 0).
    assert_eq!(r.rows[0][1], Some(int_lit(0)));
}
