//! sq-lr2ii — regression tests for the sargable FILTER fast path collapsing a
//! high-precision `xsd:decimal` value onto its (lossy) f64 image.
//!
//! The engine pushes a `?v OP constant` numeric FILTER over a single pattern down into the
//! scan, deciding each row via the O(1) f64 `numerics` cache (`exec::extract_sargable` ->
//! `ScanCmp::Num`). For a decimal whose value CANNOT be represented exactly in f64 — e.g.
//! `"1.000000000000000001"^^xsd:decimal`, whose nearest f64 is exactly `1.0` — the f64
//! verdict is wrong: `?v = 1` wrongly matches, `?v > 1` wrongly misses. The EXPRESSION path
//! (`BIND((?v = 1) AS ?b)`) is correct (it does the exact `cmp_decimal_str` recheck), so the
//! two paths DIVERGED on the same data — the load-bearing invariant these tests pin.
//!
//! The fix DECLINES the sargable fast path whenever the graph holds an f64-inexact decimal,
//! routing the comparison to the exact general evaluator (which the BIND oracle below shows
//! is correct). These assertions are RED before the fix and GREEN after.

use sparq_core::Graph;
use sparq_engine::query;

const HP: &str = "1.000000000000000001"; // 19 sig digits; nearest f64 is exactly 1.0

fn graph_with(object: &str) -> Graph {
    let ttl = format!(
        "@prefix ex: <http://example.org/> .\n\
         @prefix xsd: <http://www.w3.org/2001/XMLSchema#> .\n\
         ex:n ex:val {object} .\n"
    );
    Graph::load_str(&ttl, "turtle").expect("load turtle")
}

/// Rows returned by `SELECT ?v WHERE { ex:n ex:val ?v . FILTER(<cond>) }` — the SARGABLE
/// single-pattern + numeric-FILTER shape that `split_sargable`/`extract_sargable` push down.
fn sargable_count(g: &Graph, cond: &str) -> usize {
    let q = format!(
        "PREFIX ex: <http://example.org/> \
         SELECT ?v WHERE {{ ex:n ex:val ?v . FILTER({cond}) }}"
    );
    query(g, &q).expect("query error").rows.len()
}

/// The oracle: the EXPRESSION path via BIND, which does the exact recheck. `true` iff the
/// stored value satisfies the condition. Never rides the sargable scan pushdown.
fn oracle_true(g: &Graph, cond: &str) -> bool {
    let q = format!(
        "PREFIX ex: <http://example.org/> \
         SELECT ?b WHERE {{ ex:n ex:val ?v . BIND(({cond}) AS ?b) }}"
    );
    let r = query(g, &q).expect("query error");
    assert_eq!(r.rows.len(), 1);
    r.rows[0][0]
        .as_ref()
        .map(|t| t.to_string().contains("true"))
        .unwrap_or(false)
}

#[test]
fn high_precision_decimal_equality_does_not_match_integer() {
    // "1.000000000000000001" != 1, so FILTER(?v = 1) must return ZERO rows.
    let g = graph_with(&format!(r#""{HP}"^^xsd:decimal"#));
    assert_eq!(sargable_count(&g, "?v = 1"), 0, "?v = 1 must not match {HP}");
}

#[test]
fn high_precision_decimal_greater_than_integer_matches() {
    // "1.000000000000000001" > 1, so FILTER(?v > 1) must return ONE row.
    let g = graph_with(&format!(r#""{HP}"^^xsd:decimal"#));
    assert_eq!(sargable_count(&g, "?v > 1"), 1, "?v > 1 must match {HP}");
}

#[test]
fn high_precision_decimal_all_operators_match_oracle() {
    // Every comparison operator: the sargable count must equal the exact BIND oracle.
    // Expected (from the bead): = 0, > 1, < 0, >= 1, <= 0.
    let g = graph_with(&format!(r#""{HP}"^^xsd:decimal"#));
    for (cond, want) in [("?v = 1", 0), ("?v > 1", 1), ("?v < 1", 0), ("?v >= 1", 1), ("?v <= 1", 0)] {
        let got = sargable_count(&g, cond);
        let oracle = usize::from(oracle_true(&g, cond));
        assert_eq!(got, want, "sargable {cond}: got {got} rows, want {want}");
        assert_eq!(got, oracle, "sargable {cond} ({got}) diverged from BIND oracle ({oracle})");
    }
}

#[test]
fn ordinary_decimals_still_use_fast_path_correctly() {
    // A graph WITHOUT any f64-inexact decimal must keep the fast path AND stay correct:
    // guards against the fix over-declining / regressing exact-representable values.
    let g = graph_with(r#""1.5"^^xsd:decimal"#);
    assert_eq!(sargable_count(&g, "?v = 1.5"), 1);
    assert_eq!(sargable_count(&g, "?v > 1"), 1);
    assert_eq!(sargable_count(&g, "?v < 2"), 1);
    assert_eq!(sargable_count(&g, "?v = 1"), 0);
    assert_eq!(sargable_count(&g, "?v >= 1.5"), 1);
    assert_eq!(sargable_count(&g, "?v <= 1.4"), 0);
}

#[test]
fn integer_fast_path_unaffected() {
    // Plain integers (no decimal in the graph) keep working across operators.
    let g = graph_with(r#""3"^^xsd:integer"#);
    assert_eq!(sargable_count(&g, "?v = 3"), 1);
    assert_eq!(sargable_count(&g, "?v > 2"), 1);
    assert_eq!(sargable_count(&g, "?v < 3"), 0);
    assert_eq!(sargable_count(&g, "?v >= 3"), 1);
}

#[test]
fn property_high_precision_decimals_match_oracle() {
    // Deterministic property guard: for a spread of decimals that collapse onto a small
    // integer's f64 (fraction just above / just below), the sargable path must agree with
    // the exact BIND oracle for every operator. No proptest dep — a fixed crafted sample.
    let cases = [
        "1.000000000000000001",  // > 1, f64 -> 1.0
        "0.999999999999999999",  // < 1, f64 -> 1.0
        "2.000000000000000003",  // > 2, f64 -> 2.0
        "9.999999999999999999",  // < 10, f64 -> 10.0
        "1.0000000000000000",    // == 1 exactly (trailing zeros; f64 -> 1.0)
    ];
    for val in cases {
        let g = graph_with(&format!(r#""{val}"^^xsd:decimal"#));
        for cond in ["?v = 1", "?v > 1", "?v < 1", "?v >= 1", "?v <= 1", "?v = 2", "?v > 2", "?v < 10"] {
            let got = sargable_count(&g, cond);
            let oracle = usize::from(oracle_true(&g, cond));
            assert_eq!(got, oracle, "value {val}, {cond}: sargable {got} != oracle {oracle}");
        }
    }
}
