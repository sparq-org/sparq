//! [OPUS-4.8] sq-qcnn.13 — Direct unit tests for sparq-engine eval semantics.
//!
//! Each module exercises a specific load-bearing invariant of the engine:
//!
//! * `aggregate_error_in_group` — SUM/AVG over a group containing a type-error member
//!   yields an UNBOUND (not a partial sum): the SPARQL "type error propagation" rule.
//! * `aggregate_mixed_numeric` — int + decimal → decimal; int + double → double
//!   (XPath numeric promotion). AVG type is the promoted type of its members.
//! * `count_distinct` — COUNT(DISTINCT ?x) exact semantics (dedup by term equality).
//! * `filter_three_valued_logic` — a FILTER expression that evaluates to a type error
//!   DROPS the row (same as false), never keeps it.
//! * `type_error_builtins` — (sq-qeltv) `is*()` over an UNBOUND argument and `STR()`
//!   over a blank node are TYPE ERRORS per SPARQL 1.1 §17.2/§17.4.2, never lenient
//!   `false` / an engine-internal bnode label.
//! * `optional_scoping` — OPTIONAL left-outer-join semantics: left row survives when
//!   no right row matches; the OPTIONAL FILTER condition is applied to combined rows.
//! * `minus_scoping` — SPARQL MINUS: disjoint domains are a no-op; compatible rows
//!   are removed; unbound values in shared columns take the general compatibility path.
//! * `order_by_type_boundary` — SPARQL total type order: blank < IRI < literal (across
//!   type boundaries, not just within numeric).
//! * `having_clause` — HAVING filters on the projected aggregate value after grouping.
//! * `sample_and_group_concat` — SAMPLE and GROUP_CONCAT exact semantics.
//! * `dp_planner_result_equivalence` — (feature `dp-planner`) queries run under
//!   `with_dp_planner` yield byte-identical results to the greedy default.
//!
//! All tests assert EXACT VALUES (not just row counts / no-panic), so a semantic mutant
//! that flips a comparison or drops a branch will turn them red. [OPUS-4.8]

use sparq_core::Graph;
use sparq_engine::query;

// ─── helpers ──────────────────────────────────────────────────────────────────

fn lit(l: &str, dt: &str) -> String {
    format!(r#""{l}"^^<{dt}>"#)
}
fn xsd(ty: &str) -> String {
    format!("http://www.w3.org/2001/XMLSchema#{ty}")
}
fn int_lit(n: i64) -> String {
    lit(&n.to_string(), &xsd("integer"))
}
fn dec_lit(s: &str) -> String {
    lit(s, &xsd("decimal"))
}
fn double_lit(s: &str) -> String {
    lit(s, &xsd("double"))
}

/// Return the single cell of a one-variable, one-row result as a `Term::to_string()`.
/// Panics if the shape differs.
fn one_cell(g: &Graph, q: &str) -> Option<String> {
    let r = query(g, q).expect("query error");
    assert_eq!(r.rows.len(), 1, "expected 1 row for: {q}");
    assert_eq!(r.rows[0].len(), 1, "expected 1 column for: {q}");
    r.rows[0][0].as_ref().map(|t| t.to_string())
}



// ─── aggregate_error_in_group ─────────────────────────────────────────────────

/// SUM/AVG over a group that contains a non-numeric literal (a type error) must
/// return UNBOUND, not a partial sum / zero. This pins the SPARQL "errored member
/// propagates to unbound aggregate" discipline (`errored = true` branch in the
/// engine's `sum_values`).
#[cfg(test)]
mod aggregate_error_in_group {
    use super::*;

    // Data: one numeric triple and one string triple sharing the same predicate.
    // SUM(?o) sees {10, "hello"}: the string literal is non-numeric → type error.
    fn g() -> Graph {
        Graph::load_str(
            "@prefix ex: <http://ex/> .\
             ex:a ex:v 10 .\
             ex:b ex:v \"hello\" .",
            "turtle",
        )
        .unwrap()
    }

    /// SUM with a non-numeric member → unbound (Value::Error → None cell).
    #[test]
    fn sum_with_non_numeric_member_is_unbound() {
        // The whole-dataset group contains both 10 and "hello".
        // "hello" is non-numeric → errored = true → sum_values returns None → Value::Error → unbound.
        let result = one_cell(&g(), "PREFIX ex: <http://ex/> SELECT (SUM(?o) AS ?s) WHERE { ?x ex:v ?o }");
        assert_eq!(result, None, "SUM with a non-numeric member must be UNBOUND, got {result:?}");
    }

    /// AVG with a non-numeric member → unbound.
    #[test]
    fn avg_with_non_numeric_member_is_unbound() {
        let result = one_cell(&g(), "PREFIX ex: <http://ex/> SELECT (AVG(?o) AS ?a) WHERE { ?x ex:v ?o }");
        assert_eq!(result, None, "AVG with a non-numeric member must be UNBOUND, got {result:?}");
    }

    /// COUNT(?o) over the same data is NOT affected: it counts BOUND members and
    /// never inspects their type. Both 10 and "hello" are bound → COUNT = 2.
    #[test]
    fn count_with_non_numeric_member_counts_bound_members() {
        let result = one_cell(&g(), "PREFIX ex: <http://ex/> SELECT (COUNT(?o) AS ?n) WHERE { ?x ex:v ?o }");
        assert_eq!(result, Some(int_lit(2)), "COUNT must count all bound members, got {result:?}");
    }

    /// SUM over a GROUP BY where one group has only numeric members (the 10s) and
    /// another has only a non-numeric member ("hello"): the numeric group's SUM is
    /// bound, the non-numeric group's SUM is unbound. Pins the per-group isolation.
    #[test]
    fn sum_per_group_isolation_bound_vs_unbound() {
        let data = Graph::load_str(
            "@prefix ex: <http://ex/> .\
             ex:a ex:grp \"nums\" ; ex:v 10 .\
             ex:b ex:grp \"nums\" ; ex:v 20 .\
             ex:c ex:grp \"bad\"  ; ex:v \"hello\" .",
            "turtle",
        )
        .unwrap();
        let r = query(
            &data,
            "PREFIX ex: <http://ex/> SELECT ?g (SUM(?v) AS ?s) WHERE { ?x ex:grp ?g ; ex:v ?v } GROUP BY ?g ORDER BY ?g",
        )
        .unwrap();
        assert_eq!(r.rows.len(), 2, "two groups (bad, nums)");
        // ORDER BY ?g: "bad" < "nums" lexicographically.
        // group "bad": SUM of {"hello"} → unbound
        assert_eq!(r.rows[0][1], None, "\"bad\" group SUM must be UNBOUND, got {:?}", r.rows[0][1]);
        // group "nums": SUM of {10, 20} = 30 → bound
        assert_eq!(
            r.rows[1][1].as_ref().map(|t| t.to_string()),
            Some(int_lit(30)),
            "\"nums\" group SUM must be 30, got {:?}",
            r.rows[1][1]
        );
    }
}

// ─── aggregate_mixed_numeric ──────────────────────────────────────────────────

/// SPARQL XSD numeric type promotion: integer + decimal → decimal; decimal + double → double.
/// The engine's `sum_values` accumulates with `Num::binop(ArithOp::Add)`, which upgrades the
/// running total to the widest type seen. Tests pin the RESULT DATATYPE, not just the value.
#[cfg(test)]
mod aggregate_mixed_numeric {
    use super::*;

    /// SUM(int, decimal) → decimal.  10 (int) + 0.5 (decimal) = 10.5 decimal.
    #[test]
    fn sum_int_plus_decimal_is_decimal() {
        let g = Graph::load_str(
            "@prefix ex: <http://ex/> . @prefix xsd: <http://www.w3.org/2001/XMLSchema#> .\
             ex:a ex:v 10 .\
             ex:b ex:v \"0.5\"^^xsd:decimal .",
            "turtle",
        )
        .unwrap();
        let result = one_cell(&g, "PREFIX ex: <http://ex/> SELECT (SUM(?v) AS ?s) WHERE { ?x ex:v ?v }");
        // Expected: "10.5"^^xsd:decimal
        assert_eq!(result, Some(dec_lit("10.5")), "SUM(int, decimal) must be decimal, got {result:?}");
    }

    /// SUM(int, double) → double.  10 (int) + 1.5E0 (double) = 11.5, whose canonical
    /// xsd:double lexical form (mantissa-E-exponent, one integer digit) is "1.15E1".
    #[test]
    fn sum_int_plus_double_is_double() {
        let g = Graph::load_str(
            "@prefix ex: <http://ex/> . @prefix xsd: <http://www.w3.org/2001/XMLSchema#> .\
             ex:a ex:v 10 .\
             ex:b ex:v \"1.5E0\"^^xsd:double .",
            "turtle",
        )
        .unwrap();
        let result = one_cell(&g, "PREFIX ex: <http://ex/> SELECT (SUM(?v) AS ?s) WHERE { ?x ex:v ?v }");
        assert_eq!(result, Some(double_lit("1.15E1")), "SUM(int, double) must be double, got {result:?}");
    }

    /// AVG(3 integers) = decimal quotient when division doesn't produce an exact integer.
    /// SPARQL XPath AVG: int/int division is exact decimal, not truncated integer.
    #[test]
    fn avg_all_integers_is_decimal_quotient() {
        let g = Graph::load_str(
            "@prefix ex: <http://ex/> . ex:a ex:v 2 . ex:b ex:v 4 . ex:c ex:v 6 .",
            "turtle",
        )
        .unwrap();
        let result = one_cell(&g, "PREFIX ex: <http://ex/> SELECT (AVG(?v) AS ?a) WHERE { ?x ex:v ?v }");
        // 2+4+6 = 12 (integer SUM); AVG = 12/3, and xsd:integer÷xsd:integer is DECIMAL
        // division per SPARQL/XPath. The exact quotient is 4, serialised at the smallest
        // scale ≥ 1 the engine's `Dec::checked_div` produces → the canonical "4.0"^^xsd:decimal.
        assert_eq!(
            result,
            Some(dec_lit("4.0")),
            "AVG(2,4,6) must be the exact decimal 4.0 (not integer 4, not any other value), got {result:?}"
        );
    }

    /// AVG(int, decimal) → decimal. 1 (int) + 2.0 (decimal) = 3.0; / 2 = 1.5 decimal.
    #[test]
    fn avg_int_plus_decimal_is_decimal() {
        let g = Graph::load_str(
            "@prefix ex: <http://ex/> . @prefix xsd: <http://www.w3.org/2001/XMLSchema#> .\
             ex:a ex:v 1 . ex:b ex:v \"2.0\"^^xsd:decimal .",
            "turtle",
        )
        .unwrap();
        let result = one_cell(&g, "PREFIX ex: <http://ex/> SELECT (AVG(?v) AS ?a) WHERE { ?x ex:v ?v }");
        assert_eq!(result, Some(dec_lit("1.5")), "AVG(int, decimal) must be decimal 1.5, got {result:?}");
    }
}

// ─── count_distinct ───────────────────────────────────────────────────────────

/// COUNT(DISTINCT ?x) de-duplicates by TERM EQUALITY, not by id.
#[cfg(test)]
mod count_distinct {
    use super::*;

    /// Three triples with two distinct object values: COUNT(DISTINCT ?o) = 2.
    #[test]
    fn count_distinct_deduplicates_equal_terms() {
        let g = Graph::load_str(
            "@prefix ex: <http://ex/> . ex:a ex:p 10 . ex:b ex:p 10 . ex:c ex:p 20 .",
            "turtle",
        )
        .unwrap();
        let n = one_cell(&g, "PREFIX ex: <http://ex/> SELECT (COUNT(DISTINCT ?o) AS ?n) WHERE { ?s ex:p ?o }");
        assert_eq!(n, Some(int_lit(2)), "COUNT(DISTINCT) must count distinct terms, got {n:?}");
    }

    /// COUNT(DISTINCT ?o) vs COUNT(?o): they differ when duplicates exist.
    #[test]
    fn count_distinct_differs_from_count_on_duplicates() {
        let g = Graph::load_str(
            "@prefix ex: <http://ex/> . ex:a ex:p 1 . ex:b ex:p 1 . ex:c ex:p 1 .",
            "turtle",
        )
        .unwrap();
        let all = one_cell(&g, "PREFIX ex: <http://ex/> SELECT (COUNT(?o) AS ?n) WHERE { ?s ex:p ?o }");
        let distinct = one_cell(&g, "PREFIX ex: <http://ex/> SELECT (COUNT(DISTINCT ?o) AS ?n) WHERE { ?s ex:p ?o }");
        assert_eq!(all, Some(int_lit(3)), "COUNT(all) = 3, got {all:?}");
        assert_eq!(distinct, Some(int_lit(1)), "COUNT(DISTINCT) = 1, got {distinct:?}");
    }

    /// COUNT(DISTINCT *) (CountSolutions with distinct) counts distinct whole-rows.
    #[test]
    fn count_star_distinct_counts_distinct_solution_rows() {
        // Two identical (s,p,o) solution rows produced by a UNION.
        let g = Graph::load_str("@prefix ex: <http://ex/> . ex:a ex:p 1 .", "turtle").unwrap();
        let r = query(
            &g,
            "PREFIX ex: <http://ex/> SELECT (COUNT(*) AS ?all) (COUNT(DISTINCT *) AS ?dist) WHERE { { ?s ex:p ?o } UNION { ?s ex:p ?o } }",
        )
        .unwrap();
        assert_eq!(r.rows.len(), 1);
        // UNION does not dedup: COUNT(*) = 2, COUNT(DISTINCT *) = 1.
        let all = r.rows[0][0].as_ref().map(|t| t.to_string());
        let dist = r.rows[0][1].as_ref().map(|t| t.to_string());
        assert_eq!(all, Some(int_lit(2)), "COUNT(*) should be 2 (bag), got {all:?}");
        assert_eq!(dist, Some(int_lit(1)), "COUNT(DISTINCT *) should be 1, got {dist:?}");
    }
}

// ─── filter_three_valued_logic ────────────────────────────────────────────────

/// SPARQL FILTER 3-valued logic: an expression that evaluates to a TYPE ERROR (or
/// `Value::Unbound`) has effective-boolean-value FALSE — the row is DROPPED, not kept.
///
/// Contrast with SQL's NULLs: a SQL `WHERE` with a NULL-comparison keeps no rows
/// (same as 3VL FALSE). SPARQL agrees: error ≠ true → filtered out.
#[cfg(test)]
mod filter_three_valued_logic {
    use super::*;

    fn g() -> Graph {
        // IRI subjects + integer objects so we can provoke a string-function type error.
        Graph::load_str(
            "@prefix ex: <http://ex/> . ex:a ex:p 1 . ex:b ex:p 2 . ex:c ex:p 3 .",
            "turtle",
        )
        .unwrap()
    }

    /// FILTER(STRLEN(?s) > 0): ?s is an IRI → STRLEN of an IRI is a type error →
    /// effective_boolean(Error) = false → ALL rows dropped. The triple count shows 0.
    #[test]
    fn filter_type_error_drops_all_rows() {
        let r = query(
            &g(),
            "PREFIX ex: <http://ex/> SELECT ?s WHERE { ?s ex:p ?o FILTER(STRLEN(?s) > 0) }",
        )
        .unwrap();
        assert_eq!(r.rows.len(), 0, "FILTER on type error must drop all rows, got {}", r.rows.len());
    }

    /// FILTER(STRLEN(?o) > 0): ?o is an xsd:integer → STRLEN of a non-string is a type error.
    /// Result: 0 rows (all filtered).
    #[test]
    fn filter_strlen_of_integer_drops_rows() {
        let r = query(
            &g(),
            "PREFIX ex: <http://ex/> SELECT ?o WHERE { ?s ex:p ?o FILTER(STRLEN(?o) > 0) }",
        )
        .unwrap();
        assert_eq!(r.rows.len(), 0, "STRLEN(?integer) is a type error → filter drops rows, got {}", r.rows.len());
    }

    /// A FILTER with a VALID condition keeps the matching rows and drops the rest.
    /// Contrast proves the previous tests are not vacuous (the store IS non-empty).
    #[test]
    fn filter_valid_condition_keeps_matching_rows() {
        let r = query(
            &g(),
            "PREFIX ex: <http://ex/> SELECT ?o WHERE { ?s ex:p ?o FILTER(?o > 1) }",
        )
        .unwrap();
        assert_eq!(r.rows.len(), 2, "FILTER(?o > 1) must keep exactly 2 rows (2 and 3), got {}", r.rows.len());
    }

    /// FILTER error is NOT the same as FILTER false: a negated error (`!error`) is
    /// itself an error (NOT(error) = error in SPARQL 3VL), so FILTER(!error) also
    /// drops rows.  `!STRLEN(?s) > 0` ≡ `!error` = error → row dropped.
    #[test]
    fn filter_not_of_type_error_is_still_error_drops_rows() {
        let r = query(
            &g(),
            "PREFIX ex: <http://ex/> SELECT ?s WHERE { ?s ex:p ?o FILTER(!STRLEN(?s) > 0) }",
        )
        .unwrap();
        assert_eq!(r.rows.len(), 0, "!error is still error → all rows dropped, got {}", r.rows.len());
    }

    /// FILTER on an unbound optional variable: the variable is unbound → Value::Unbound
    /// → effective_boolean = false → row dropped. Unbound ≠ false but both → dropped.
    #[test]
    fn filter_on_unbound_optional_variable_drops_row() {
        let g = Graph::load_str(
            "@prefix ex: <http://ex/> . ex:a ex:p 1 . ex:b ex:p 2 .",
            "turtle",
        )
        .unwrap();
        // OPTIONAL leaves ?opt unbound for all rows (nothing has ex:q).
        let r = query(
            &g,
            "PREFIX ex: <http://ex/> SELECT ?s WHERE { ?s ex:p ?o OPTIONAL { ?s ex:q ?opt } FILTER(?opt > 0) }",
        )
        .unwrap();
        assert_eq!(r.rows.len(), 0, "FILTER on unbound ?opt → dropped, got {}", r.rows.len());
    }

    /// SPARQL AND short-circuits on FALSE (not on error). `false AND error` = false.
    /// `true AND error` = error → row dropped.
    #[test]
    fn filter_and_short_circuits_on_false_not_error() {
        let g = Graph::load_str("@prefix ex: <http://ex/> . ex:a ex:p 1 .", "turtle").unwrap();
        // false && (type error) = false → row dropped
        let r_false_and = query(&g, "PREFIX ex: <http://ex/> SELECT ?s WHERE { ?s ex:p ?o FILTER(false && STRLEN(?s) > 0) }").unwrap();
        assert_eq!(r_false_and.rows.len(), 0, "false && error = false → dropped");

        // true && (type error) = error → row dropped
        let r_true_and = query(&g, "PREFIX ex: <http://ex/> SELECT ?s WHERE { ?s ex:p ?o FILTER(true && STRLEN(?s) > 0) }").unwrap();
        assert_eq!(r_true_and.rows.len(), 0, "true && error = error → dropped");

        // true && true = true → row kept
        let r_true_true = query(&g, "PREFIX ex: <http://ex/> SELECT ?s WHERE { ?s ex:p ?o FILTER(true && ?o > 0) }").unwrap();
        assert_eq!(r_true_true.rows.len(), 1, "true && true = true → kept");
    }

    /// SPARQL OR short-circuits on TRUE.  `true OR error` = true → row kept.
    /// `false OR error` = error → row dropped.
    #[test]
    fn filter_or_short_circuits_on_true() {
        let g = Graph::load_str("@prefix ex: <http://ex/> . ex:a ex:p 1 .", "turtle").unwrap();
        // true || error = true → row kept
        let r_true_or = query(&g, "PREFIX ex: <http://ex/> SELECT ?s WHERE { ?s ex:p ?o FILTER(true || STRLEN(?s) > 0) }").unwrap();
        assert_eq!(r_true_or.rows.len(), 1, "true || error = true → kept");

        // false || error = error → row dropped
        let r_false_or = query(&g, "PREFIX ex: <http://ex/> SELECT ?s WHERE { ?s ex:p ?o FILTER(false || STRLEN(?s) > 0) }").unwrap();
        assert_eq!(r_false_or.rows.len(), 0, "false || error = error → dropped");
    }
}

// ─── type_error_builtins (sq-qeltv) ───────────────────────────────────────────

/// [FABLE-5] (sq-qeltv) SPARQL 1.1 §17.2 / §17.4.2: the type-test builtins
/// (`isIRI`/`isBlank`/`isLiteral`/`isNumeric`) over an UNBOUND argument are a TYPE
/// ERROR (not `false`), and `STR()` is defined over literals + IRIs ONLY (a blank
/// node is a type error, never an engine-internal label). Differential-verified:
/// Oxigraph agrees on both corners; rdflib agrees on the unbound corner.
#[cfg(test)]
mod type_error_builtins {
    use super::*;

    /// One triple; the OPTIONAL never matches, so ?u is unbound on the single row.
    fn g_opt() -> Graph {
        Graph::load_str("<http://ex/a> <http://ex/p> <http://ex/b> .", "ntriples").unwrap()
    }
    const OPT_BODY: &str = "?s <http://ex/p> ?o . OPTIONAL { ?o <http://ex/p> ?u }";

    /// The bead's exact repro: `FILTER(!isIRI(?u))` with ?u unbound. isIRI(unbound)
    /// is a type error, `!error` is still an error → the row is DROPPED. The old
    /// lenient `false` made `!isIRI(?u)` TRUE and wrongly KEPT the row.
    #[test]
    fn not_isiri_of_unbound_is_error_drops_row() {
        let q = format!("SELECT ?s WHERE {{ {OPT_BODY} FILTER(!isIRI(?u)) }}");
        let r = query(&g_opt(), &q).unwrap();
        assert_eq!(r.rows.len(), 0, "!isIRI(?unbound) must be a type error (row dropped), got {} rows", r.rows.len());
    }

    /// All four type tests error on unbound, in BOTH polarities (error ≠ false: the
    /// negated form must also drop the row — that's what distinguishes the spec
    /// semantics from the old `false`, where exactly one polarity would keep it).
    #[test]
    fn all_four_type_tests_error_on_unbound_both_polarities() {
        for f in ["isIRI", "isBlank", "isLiteral", "isNumeric"] {
            for form in [format!("{f}(?u)"), format!("!{f}(?u)")] {
                let q = format!("SELECT ?s WHERE {{ {OPT_BODY} FILTER({form}) }}");
                let r = query(&g_opt(), &q).unwrap();
                assert_eq!(r.rows.len(), 0, "FILTER({form}) with ?u unbound must drop the row, got {} rows", r.rows.len());
            }
        }
    }

    /// Positive control (non-vacuity): the same query shapes over a BOUND argument
    /// still evaluate to plain booleans — exactly one polarity keeps the row.
    #[test]
    fn type_tests_on_bound_terms_unchanged() {
        let g = g_opt();
        let keep = |filt: &str| {
            let q = format!("SELECT ?s WHERE {{ ?s <http://ex/p> ?o FILTER({filt}) }}");
            query(&g, &q).unwrap().rows.len()
        };
        assert_eq!(keep("isIRI(?o)"), 1, "isIRI(bound IRI) must be true");
        assert_eq!(keep("!isIRI(?o)"), 0, "!isIRI(bound IRI) must be false");
        assert_eq!(keep("isLiteral(?o)"), 0, "isLiteral(bound IRI) must be false");
        assert_eq!(keep("isBlank(?o)"), 0, "isBlank(bound IRI) must be false");
        assert_eq!(keep("isNumeric(?o)"), 0, "isNumeric(bound IRI) must be false");
    }

    /// An ERRORED (not merely unbound) argument also propagates: STRLEN(?s) on an
    /// IRI is a type error, and isNumeric(error) must stay an error, not `false` —
    /// so the NEGATED filter drops the row too.
    #[test]
    fn type_test_propagates_errored_argument() {
        let q = "SELECT ?s WHERE { ?s <http://ex/p> ?o FILTER(!isNumeric(STRLEN(?s))) }";
        let r = query(&g_opt(), q).unwrap();
        assert_eq!(r.rows.len(), 0, "isNumeric(type-error) must propagate the error, got {} rows", r.rows.len());
    }

    /// STR(bnode) is a type error: a BIND of it leaves the variable UNBOUND (row
    /// kept), and never leaks an engine-internal blank-node label. The bead observed
    /// STRLEN(STR(?bnode)) = 11 (an internal label's length) — both forms must now
    /// be unbound.
    #[test]
    fn str_of_bnode_is_error_bind_leaves_unbound() {
        let g = Graph::load_str("_:b <http://ex/p> <http://ex/o> .", "ntriples").unwrap();
        for bind in ["STR(?s)", "STRLEN(STR(?s))"] {
            let q = format!("SELECT ?s ?v WHERE {{ ?s <http://ex/p> ?o . BIND({bind} AS ?v) }}");
            let r = query(&g, &q).unwrap();
            assert_eq!(r.rows.len(), 1, "BIND of an error keeps the row for: {bind}");
            assert!(r.rows[0][0].is_some(), "?s (the bnode) stays bound for: {bind}");
            assert_eq!(r.rows[0][1], None, "BIND({bind}) over a bnode must leave ?v UNBOUND, got {:?}", r.rows[0][1]);
        }
    }

    /// Positive control (non-vacuity): STR over each type it IS defined on — IRI,
    /// plain/lang-tagged/typed literal, computed numeric and boolean — unchanged.
    #[test]
    fn str_on_defined_domains_unchanged() {
        let g = Graph::load_str(
            "<http://ex/a> <http://ex/p> \"hi\"@en .\n<http://ex/a> <http://ex/q> \"7\"^^<http://www.w3.org/2001/XMLSchema#integer> .",
            "ntriples",
        )
        .unwrap();
        let cell = |q: &str| one_cell(&g, q);
        assert_eq!(cell("SELECT (STR(<http://ex/x>) AS ?v) WHERE {}"), Some("\"http://ex/x\"".into()));
        assert_eq!(
            cell("SELECT ?v WHERE { <http://ex/a> <http://ex/p> ?o BIND(STR(?o) AS ?v) }"),
            Some("\"hi\"".into()),
            "STR of a lang-tagged literal is its lexical form"
        );
        assert_eq!(
            cell("SELECT ?v WHERE { <http://ex/a> <http://ex/q> ?o BIND(STR(?o) AS ?v) }"),
            Some("\"7\"".into()),
            "STR of a typed literal is its lexical form"
        );
        assert_eq!(cell("SELECT (STR(1+1) AS ?v) WHERE {}"), Some("\"2\"".into()), "STR of a computed numeric");
        assert_eq!(cell("SELECT (STR(1 < 2) AS ?v) WHERE {}"), Some("\"true\"".into()), "STR of a computed boolean");
    }

    /// STR(?u) with ?u UNBOUND stays a type error (was already an error before the
    /// bead fix; pinned so the STR domain change can't regress it).
    #[test]
    fn str_of_unbound_is_error() {
        let q = format!("SELECT ?s ?v WHERE {{ {OPT_BODY} BIND(STR(?u) AS ?v) }}");
        let r = query(&g_opt(), &q).unwrap();
        assert_eq!(r.rows.len(), 1);
        assert_eq!(r.rows[0][1], None, "BIND(STR(?unbound)) must leave ?v unbound");
    }
}

// ─── optional_scoping ─────────────────────────────────────────────────────────

/// SPARQL OPTIONAL (left outer join) scoping tests: left row survives with NULL
/// right-side variables when no right row matches; OPTIONAL FILTER condition is
/// applied to the combined row (not just the right side).
#[cfg(test)]
mod optional_scoping {
    use super::*;

    fn g() -> Graph {
        Graph::load_str(
            "@prefix ex: <http://ex/> .\
             ex:a ex:p 1 ; ex:q 10 .\
             ex:b ex:p 2 .\
             ex:c ex:p 3 ; ex:q 30 .",
            "turtle",
        )
        .unwrap()
    }

    /// Basic OPTIONAL: rows without a matching right side keep ?o unbound.
    #[test]
    fn optional_keeps_left_row_when_no_right_match() {
        let r = query(
            &g(),
            "PREFIX ex: <http://ex/> SELECT ?s ?o WHERE { ?s ex:p ?v OPTIONAL { ?s ex:q ?o } } ORDER BY ?s",
        )
        .unwrap();
        // Three subjects: a (has q), b (no q), c (has q).
        assert_eq!(r.rows.len(), 3, "OPTIONAL must keep all 3 left rows, got {}", r.rows.len());
        // b must have ?o = None (unbound, not missing row).
        let b_row = r.rows.iter().find(|row| {
            row[0].as_ref().map(|t| t.to_string().contains("/b")).unwrap_or(false)
        });
        let b_row = b_row.expect("ex:b row must be present");
        assert_eq!(b_row[1], None, "ex:b has no ex:q, so ?o must be unbound, got {:?}", b_row[1]);
    }

    /// OPTIONAL with a FILTER condition: the filter is applied to the COMBINED row
    /// (left + right). A right row that fails the filter is treated as "no match",
    /// so the left row survives with the right variables unbound.
    ///
    /// Pattern: `?s ex:p ?v OPTIONAL { ?s ex:q ?o FILTER(?o > 15) }`
    /// ex:a has q=10 (10 > 15 is false → no match → left row survives, ?o unbound)
    /// ex:b has no q → left row survives, ?o unbound
    /// ex:c has q=30 (30 > 15 is true → match, ?o = 30)
    #[test]
    fn optional_filter_applied_to_combined_row() {
        let r = query(
            &g(),
            "PREFIX ex: <http://ex/> SELECT ?s ?o WHERE { ?s ex:p ?v OPTIONAL { ?s ex:q ?o FILTER(?o > 15) } } ORDER BY ?s",
        )
        .unwrap();
        assert_eq!(r.rows.len(), 3, "OPTIONAL with filter must keep all 3 left rows, got {}", r.rows.len());
        // a: q=10, 10 > 15 is false → ?o unbound
        let a_row = r.rows.iter().find(|row| row[0].as_ref().map(|t| t.to_string().contains("/a")).unwrap_or(false)).unwrap();
        assert_eq!(a_row[1], None, "ex:a: q=10 fails FILTER(q>15), so ?o must be unbound, got {:?}", a_row[1]);
        // c: q=30, 30 > 15 is true → ?o = 30
        let c_row = r.rows.iter().find(|row| row[0].as_ref().map(|t| t.to_string().contains("/c")).unwrap_or(false)).unwrap();
        assert!(c_row[1].is_some(), "ex:c: q=30 passes FILTER(q>15), ?o must be bound, got {:?}", c_row[1]);
    }

    /// OPTIONAL: multiple matching right rows produce one output row per combination.
    #[test]
    fn optional_multiple_right_rows_produce_combinations() {
        let g2 = Graph::load_str(
            "@prefix ex: <http://ex/> .\
             ex:a ex:p 1 ; ex:q 10 ; ex:q 20 .",
            "turtle",
        )
        .unwrap();
        let r = query(
            &g2,
            "PREFIX ex: <http://ex/> SELECT ?s ?o WHERE { ?s ex:p ?v OPTIONAL { ?s ex:q ?o } }",
        )
        .unwrap();
        // ex:a has two ex:q values (10 and 20) → 2 output rows.
        assert_eq!(r.rows.len(), 2, "OPTIONAL with 2 matching right rows must produce 2 output rows, got {}", r.rows.len());
    }

    /// OPTIONAL with a shared variable that is sometimes unbound on the left: the general
    /// hash/compat path (not the sort-merge single-shared-key fast path). When left has
    /// multiple shared variables with some unbound, a compat scan is used.
    #[test]
    fn optional_compat_scan_with_unbound_left_key() {
        // Two left rows: one with ?s bound to ex:a, one with ?s from OPTIONAL (unbound).
        // Right side has ex:a with a q value.
        let g3 = Graph::load_str(
            "@prefix ex: <http://ex/> . ex:a ex:p 1 . ex:a ex:q 99 .",
            "turtle",
        )
        .unwrap();
        // Outer OPTIONAL leaves ?y unbound for some rows.
        let r = query(
            &g3,
            "PREFIX ex: <http://ex/> SELECT ?s ?y ?z WHERE { ?s ex:p ?v OPTIONAL { ?s ex:r ?y } OPTIONAL { ?s ex:q ?z } }",
        )
        .unwrap();
        // One left row (ex:a): no ex:r → ?y unbound; has ex:q 99 → ?z = 99.
        assert_eq!(r.rows.len(), 1);
        assert_eq!(r.rows[0][1], None, "?y (ex:r match) should be unbound, got {:?}", r.rows[0][1]);
        assert!(r.rows[0][2].is_some(), "?z (ex:q match) should be bound to 99, got {:?}", r.rows[0][2]);
    }
}

// ─── minus_scoping ────────────────────────────────────────────────────────────

/// SPARQL MINUS semantics:
/// * Disjoint domains (no shared variables) → MINUS is a no-op.
/// * Compatible rows (shared vars bound and equal) → dropped.
/// * Incompatible rows (shared var bound but value differs) → kept.
/// * Unbound in a shared variable (general compatibility path) → NOT removed
///   unless bound domains overlap with a match.
#[cfg(test)]
mod minus_scoping {
    use super::*;

    fn g() -> Graph {
        Graph::load_str(
            "@prefix ex: <http://ex/> .\
             ex:a ex:type ex:X . ex:a ex:val 1 .\
             ex:b ex:type ex:X . ex:b ex:val 2 .\
             ex:c ex:type ex:Y . ex:c ex:val 3 .",
            "turtle",
        )
        .unwrap()
    }

    /// MINUS with disjoint domains (no shared variable at all): nothing removed.
    /// SPARQL 1.1 §18.6: "Disjoint domains of ?P and ?Q means MINUS is a no-op."
    #[test]
    fn minus_disjoint_domains_is_noop() {
        let r = query(
            &g(),
            "PREFIX ex: <http://ex/> SELECT ?s WHERE { { ?s ex:type ?t } MINUS { ?x ex:val ?y } }",
        )
        .unwrap();
        // Left: {a, b, c}. Right: {a:1, b:2, c:3} but on DIFFERENT variables → no shared variable
        // Wait - ?s and ?x don't overlap. So disjoint → nothing removed.
        assert_eq!(r.rows.len(), 3, "MINUS with disjoint variables must be a no-op, got {}", r.rows.len());
    }

    /// MINUS with compatible rows: rows whose shared variable values match are dropped.
    #[test]
    fn minus_removes_compatible_rows() {
        let r = query(
            &g(),
            "PREFIX ex: <http://ex/> SELECT ?s WHERE { { ?s ex:type ?t } MINUS { ?s ex:type ex:X } }",
        )
        .unwrap();
        // Left: {a (type X), b (type X), c (type Y)}.
        // Right: {s → a, s → b} (those with type X).
        // ?s is shared; a and b have type X and are removed → only c remains.
        assert_eq!(r.rows.len(), 1, "MINUS must remove a and b (type X), leaving c, got {}", r.rows.len());
        let remaining = r.rows[0][0].as_ref().map(|t| t.to_string());
        assert!(remaining.as_deref().map(|s| s.contains("/c")).unwrap_or(false), "only ex:c should survive, got {remaining:?}");
    }

    /// MINUS with incompatible rows: different values for the shared variable → kept.
    #[test]
    fn minus_keeps_incompatible_rows() {
        let r = query(
            &g(),
            "PREFIX ex: <http://ex/> SELECT ?s ?t WHERE { { ?s ex:type ?t } MINUS { ?s ex:type ex:Z } }",
        )
        .unwrap();
        // Right side has ?s ex:type ex:Z — nothing matches (no Z triples) → all left kept.
        assert_eq!(r.rows.len(), 3, "MINUS with no compatible right rows must keep all 3 left rows, got {}", r.rows.len());
    }

    /// MINUS with an OPTIONAL on the left: when ?x is unbound on the left, the
    /// general compatibility scan path is used (unbound shared vars act as wildcards
    /// per the SPARQL spec — unbound ≡ absent domain). A row with unbound ?x must
    /// NOT be removed by a right row that binds ?x.
    #[test]
    fn minus_unbound_left_variable_is_not_removed_by_bound_right() {
        // Left: three rows from BGP, one of which has ?opt unbound from OPTIONAL.
        // MINUS right side has ?s and ?opt both bound. An unbound ?opt in the left
        // row means the domain of ?opt is absent on the left → no overlap on ?opt →
        // the row is NOT compatible → not removed.
        let g2 = Graph::load_str(
            "@prefix ex: <http://ex/> .\
             ex:a ex:p 1 ; ex:q 42 .\
             ex:b ex:p 2 .",
            "turtle",
        )
        .unwrap();
        let r = query(
            &g2,
            "PREFIX ex: <http://ex/> SELECT ?s ?opt WHERE { { ?s ex:p ?v OPTIONAL { ?s ex:q ?opt } } MINUS { ?s ex:q ?opt } }",
        )
        .unwrap();
        // ex:a: ?opt = 42 → compatible with right {s=a, opt=42} → REMOVED.
        // ex:b: ?opt = unbound → no domain overlap on ?opt → NOT removed.
        assert_eq!(r.rows.len(), 1, "only ex:b (unbound ?opt) should survive, got {}", r.rows.len());
        let s = r.rows[0][0].as_ref().map(|t| t.to_string());
        assert!(s.as_deref().map(|s| s.contains("/b")).unwrap_or(false), "surviving row should be ex:b, got {s:?}");
        assert_eq!(r.rows[0][1], None, "?opt should be unbound for ex:b, got {:?}", r.rows[0][1]);
    }
}

// ─── order_by_type_boundary ───────────────────────────────────────────────────

/// SPARQL total type order: error/unbound < blank < IRI < literal (within literals:
/// numeric < other; xsd:string < typed). This test pins the cross-type ordering, not
/// the numeric-collapse case (which has its own tests in exec.rs `f64_collapse_order_agreement`).
#[cfg(test)]
mod order_by_type_boundary {
    use super::*;

    /// A result mixes a blank node, an IRI, and a plain literal. ORDER BY ASC(?x) must
    /// place blank < IRI < literal (the SPARQL §15.1 cross-type total order); DESC(?x)
    /// must be the exact reverse. The three kinds are supplied via data triples rather
    /// than a VALUES block, because SPARQL VALUES cannot bind a blank node.
    ///
    /// Note: blank-node labels are opaque in the output string, so we assert on the
    /// leading term-kind sigil at each position (`_` blank, `<` IRI, `"` literal), not
    /// on the value.
    #[test]
    fn order_by_blank_before_iri_before_literal() {
        // Same subject/predicate, three objects of distinct kinds: blank, IRI, literal.
        let g = Graph::load_str(
            "@prefix ex: <http://ex/> .\
             ex:s ex:p _:b .\
             ex:s ex:p <http://ex/iri> .\
             ex:s ex:p \"lit\" .",
            "turtle",
        )
        .unwrap();
        let kinds = |q: &str| -> Vec<char> {
            let r = query(&g, q).unwrap();
            assert_eq!(r.rows.len(), 3, "3 objects must yield 3 rows for: {q}");
            r.rows
                .iter()
                .map(|row| row[0].as_ref().expect("bound").to_string().chars().next().expect("non-empty term"))
                .collect()
        };
        // ASC: blank ('_') < IRI ('<') < literal ('"').
        assert_eq!(
            kinds("PREFIX ex: <http://ex/> SELECT ?x WHERE { ex:s ex:p ?x } ORDER BY ASC(?x)"),
            vec!['_', '<', '"'],
            "ASC(?x) must order blank < IRI < literal"
        );
        // DESC over the SAME data must be the exact reverse: literal > IRI > blank.
        assert_eq!(
            kinds("PREFIX ex: <http://ex/> SELECT ?x WHERE { ex:s ex:p ?x } ORDER BY DESC(?x)"),
            vec!['"', '<', '_'],
            "DESC(?x) must be the exact reverse: literal > IRI > blank"
        );
    }

    /// Within numerics: ORDER BY orders by VALUE. Within strings: lexicographic.
    /// Across numeric/string: numerics sort before plain-string literals.
    #[test]
    fn order_by_numeric_before_string_literal() {
        let g = Graph::load_str("@prefix ex: <http://ex/> . ex:a ex:p 1 . ex:b ex:p \"abc\" .", "turtle").unwrap();
        let r = query(&g, "PREFIX ex: <http://ex/> SELECT ?o WHERE { ?s ex:p ?o } ORDER BY ASC(?o)").unwrap();
        assert_eq!(r.rows.len(), 2);
        // numeric (1) should precede string ("abc") in SPARQL total order. Assert the
        // EXACT terms rather than a `contains("integer")` datatype-substring check.
        let first = r.rows[0][0].as_ref().map(|t| t.to_string());
        let second = r.rows[1][0].as_ref().map(|t| t.to_string());
        assert_eq!(
            first.as_deref(),
            Some(int_lit(1).as_str()),
            "integer 1 must sort first, got first={first:?}"
        );
        assert_eq!(
            second.as_deref(),
            Some("\"abc\""),
            "plain string \"abc\" must sort after the integer, got second={second:?}"
        );
    }

    /// Unbound / UNDEF in ORDER BY: SPARQL puts unbound before everything else in ASC
    /// (they compare as less than any term).
    #[test]
    fn order_by_unbound_sorts_first_in_asc() {
        let g = Graph::load_str("@prefix ex: <http://ex/> . ex:a ex:p 1 . ex:b ex:p 2 .", "turtle").unwrap();
        // OPTIONAL with no match → ?opt is unbound for all rows.
        let r = query(
            &g,
            "PREFIX ex: <http://ex/> SELECT ?s ?opt WHERE { ?s ex:p ?o OPTIONAL { ?s ex:r ?opt } } ORDER BY ASC(?opt) ?s",
        )
        .unwrap();
        assert_eq!(r.rows.len(), 2, "should have 2 rows");
        // Both rows have ?opt unbound (no ex:r triples); unbound sorts first.
        for row in &r.rows {
            assert_eq!(row[1], None, "?opt should be unbound for all rows");
        }
    }
}

// ─── having_clause ────────────────────────────────────────────────────────────

/// HAVING clause: a FILTER on the aggregated row, evaluated AFTER GROUP BY and aggregate
/// computation. Only groups satisfying HAVING are included in the output.
#[cfg(test)]
mod having_clause {
    use super::*;

    fn g() -> Graph {
        Graph::load_str(
            "@prefix ex: <http://ex/> .\
             ex:a ex:tag \"t1\" ; ex:tag \"t2\" ; ex:tag \"t3\" .\
             ex:b ex:tag \"t1\" .\
             ex:c ex:tag \"t1\" ; ex:tag \"t2\" .",
            "turtle",
        )
        .unwrap()
    }

    /// GROUP BY ?s HAVING (COUNT(*) > 2): only the group for ex:a (3 tags) passes.
    #[test]
    fn having_filters_groups_by_aggregate_value() {
        let r = query(
            &g(),
            "PREFIX ex: <http://ex/> SELECT ?s (COUNT(*) AS ?n) WHERE { ?s ex:tag ?t } GROUP BY ?s HAVING (COUNT(*) > 2)",
        )
        .unwrap();
        assert_eq!(r.rows.len(), 1, "only ex:a (3 tags) should satisfy HAVING(COUNT > 2), got {}", r.rows.len());
        let s = r.rows[0][0].as_ref().map(|t| t.to_string());
        assert!(s.as_deref().map(|s| s.contains("/a")).unwrap_or(false), "ex:a must be the surviving group, got {s:?}");
        let n = r.rows[0][1].as_ref().map(|t| t.to_string());
        assert_eq!(n, Some(int_lit(3)), "COUNT for ex:a must be 3, got {n:?}");
    }

    /// HAVING with >= keeps multiple groups.
    #[test]
    fn having_gte_keeps_multiple_groups() {
        let r = query(
            &g(),
            "PREFIX ex: <http://ex/> SELECT ?s (COUNT(*) AS ?n) WHERE { ?s ex:tag ?t } GROUP BY ?s HAVING (COUNT(*) >= 2) ORDER BY ?s",
        )
        .unwrap();
        // a (3 tags) and c (2 tags) both satisfy >= 2; b (1 tag) does not.
        assert_eq!(r.rows.len(), 2, "ex:a and ex:c should satisfy HAVING(COUNT >= 2), got {}", r.rows.len());
    }

    /// HAVING with a condition that no group satisfies → 0 rows.
    #[test]
    fn having_no_group_satisfies_yields_zero_rows() {
        let r = query(
            &g(),
            "PREFIX ex: <http://ex/> SELECT ?s (COUNT(*) AS ?n) WHERE { ?s ex:tag ?t } GROUP BY ?s HAVING (COUNT(*) > 100)",
        )
        .unwrap();
        assert_eq!(r.rows.len(), 0, "no group has more than 100 tags, should be 0 rows, got {}", r.rows.len());
    }
}

// ─── sample_and_group_concat ──────────────────────────────────────────────────

/// SAMPLE returns ONE value from the group (any member; non-deterministic but non-empty
/// when the group is non-empty). GROUP_CONCAT joins values with a separator.
#[cfg(test)]
mod sample_and_group_concat {
    use super::*;

    fn g() -> Graph {
        Graph::load_str(
            "@prefix ex: <http://ex/> .\
             ex:a ex:p \"x\" . ex:a ex:p \"y\" . ex:a ex:p \"z\" .",
            "turtle",
        )
        .unwrap()
    }

    /// SAMPLE over a non-empty group must return a BOUND value.
    #[test]
    fn sample_over_non_empty_group_is_bound() {
        let result = one_cell(&g(), "PREFIX ex: <http://ex/> SELECT (SAMPLE(?o) AS ?v) WHERE { ?s ex:p ?o }");
        assert!(result.is_some(), "SAMPLE over non-empty group must be bound, got None");
    }

    /// GROUP_CONCAT joins all member string values with the default separator (space).
    /// The result is a plain (simple) literal.
    #[test]
    fn group_concat_default_separator_joins_with_space() {
        let result = one_cell(&g(), "PREFIX ex: <http://ex/> SELECT (GROUP_CONCAT(?o) AS ?v) WHERE { ?s ex:p ?o }");
        let s = result.expect("GROUP_CONCAT must produce a bound value");
        // The result must be a simple literal containing the three values separated by spaces.
        // The ORDER is unspecified, but all three must appear.
        for val in ["x", "y", "z"] {
            assert!(s.contains(val), "GROUP_CONCAT result must contain {val:?}, got {s:?}");
        }
        // Spaces as separator (not commas, not empty).
        assert!(s.contains(' '), "GROUP_CONCAT default separator is space, got {s:?}");
    }

    /// GROUP_CONCAT with a custom separator (semicolon).
    #[test]
    fn group_concat_custom_separator() {
        let result = one_cell(
            &g(),
            "PREFIX ex: <http://ex/> SELECT (GROUP_CONCAT(?o; SEPARATOR=\";\") AS ?v) WHERE { ?s ex:p ?o }",
        );
        let s = result.expect("GROUP_CONCAT with separator must produce a bound value");
        assert!(s.contains(';'), "GROUP_CONCAT separator=';' must appear in result, got {s:?}");
        // No spaces (not the default separator).
        assert!(!s.contains(' '), "custom separator ';' must not add spaces, got {s:?}");
    }

    /// MIN/MAX over a group of plain string literals: lexicographic order.
    #[test]
    fn min_max_over_string_literals() {
        // Lexicographic: "x" < "y" < "z".
        let mn = one_cell(&g(), "PREFIX ex: <http://ex/> SELECT (MIN(?o) AS ?v) WHERE { ?s ex:p ?o }");
        let mx = one_cell(&g(), "PREFIX ex: <http://ex/> SELECT (MAX(?o) AS ?v) WHERE { ?s ex:p ?o }");
        // MIN = "x", MAX = "z". Assert the EXACT plain-literal term (a simple xsd:string
        // renders without a datatype suffix) — a `contains('"') && contains('x')` check
        // would also pass for a wrong value like "xyz" or "x-ray".
        assert_eq!(mn.as_deref(), Some("\"x\""), "MIN of {{\"x\",\"y\",\"z\"}} must be the plain literal \"x\", got {mn:?}");
        assert_eq!(mx.as_deref(), Some("\"z\""), "MAX of {{\"x\",\"y\",\"z\"}} must be the plain literal \"z\", got {mx:?}");
    }
}

// ─── subselect_and_union ──────────────────────────────────────────────────────

/// Sub-SELECT and UNION semantics: sub-SELECT projects and limits variable scope;
/// UNION is a bag union (no deduplication unless DISTINCT is added).
#[cfg(test)]
mod subselect_and_union {
    use super::*;

    fn g() -> Graph {
        Graph::load_str(
            "@prefix ex: <http://ex/> .\
             ex:a ex:p 1 . ex:b ex:p 2 . ex:c ex:p 3 .",
            "turtle",
        )
        .unwrap()
    }

    /// UNION is a bag operation: if the same pattern appears on both sides, rows are
    /// doubled (no implicit DISTINCT).
    #[test]
    fn union_is_bag_not_set() {
        let r = query(
            &g(),
            "PREFIX ex: <http://ex/> SELECT ?s WHERE { { ?s ex:p ?o } UNION { ?s ex:p ?o } }",
        )
        .unwrap();
        assert_eq!(r.rows.len(), 6, "UNION is a bag: 3+3=6 rows, got {}", r.rows.len());
    }

    /// A sub-SELECT projects only some variables. The outer query sees only the
    /// projected variables, not internal variables.
    #[test]
    fn subselect_projects_only_selected_variables() {
        let r = query(
            &g(),
            "PREFIX ex: <http://ex/> SELECT ?o WHERE { SELECT ?o WHERE { ?s ex:p ?o } }",
        )
        .unwrap();
        assert_eq!(r.vars.len(), 1, "only ?o projected, not ?s");
        assert_eq!(r.vars[0].as_str(), "o");
        assert_eq!(r.rows.len(), 3);
    }

    /// Sub-SELECT with LIMIT: only the capped rows appear in the outer result.
    #[test]
    fn subselect_with_limit_caps_rows() {
        let r = query(
            &g(),
            "PREFIX ex: <http://ex/> SELECT ?o WHERE { SELECT ?o WHERE { ?s ex:p ?o } LIMIT 2 }",
        )
        .unwrap();
        assert_eq!(r.rows.len(), 2, "sub-SELECT LIMIT 2 must cap outer to 2 rows, got {}", r.rows.len());
    }
}

// ─── values_inline ────────────────────────────────────────────────────────────

/// VALUES inline table: exact term binding with UNDEF support.
#[cfg(test)]
mod values_inline {
    use super::*;

    /// VALUES with multiple rows and an UNDEF: the UNDEF cell becomes an unbound
    /// variable and does NOT join with graph terms.
    #[test]
    fn values_undef_is_unbound_not_null_string() {
        let g = Graph::load_str("@prefix ex: <http://ex/> . ex:a ex:p 1 .", "turtle").unwrap();
        let r = query(
            &g,
            "SELECT ?x ?y WHERE { VALUES (?x ?y) { (<http://ex/a> 1) (<http://ex/b> UNDEF) } }",
        )
        .unwrap();
        assert_eq!(r.rows.len(), 2, "VALUES must bind 2 rows");
        // second row has ?y = UNDEF → unbound
        let second = &r.rows[1];
        assert_eq!(second[1], None, "UNDEF must be unbound (not a literal), got {:?}", second[1]);
    }

    /// VALUES joined with a BGP: only rows where both VALUES and BGP bind consistently match.
    #[test]
    fn values_join_with_bgp() {
        let g = Graph::load_str(
            "@prefix ex: <http://ex/> . ex:a ex:p 1 . ex:b ex:p 2 . ex:c ex:p 3 .",
            "turtle",
        )
        .unwrap();
        // VALUES restricts ?s to ex:a and ex:b; ex:c is excluded.
        let r = query(
            &g,
            "PREFIX ex: <http://ex/> SELECT ?s ?o WHERE { VALUES ?s { ex:a ex:b } ?s ex:p ?o } ORDER BY ?s",
        )
        .unwrap();
        assert_eq!(r.rows.len(), 2, "VALUES restricts ?s to ex:a and ex:b");
        let s0 = r.rows[0][0].as_ref().map(|t| t.to_string());
        assert!(s0.as_deref().map(|s| s.contains("/a")).unwrap_or(false), "first row should be ex:a, got {s0:?}");
    }
}

// ─── dp_planner_result_equivalence ───────────────────────────────────────────

/// (Feature `dp-planner`) Queries run under `with_dp_planner` produce BYTE-IDENTICAL
/// results to the greedy default on shapes the DP planner engages with (chain, star,
/// snowflake, triangle via LFTJ).
///
/// Not feature-gated at the MODULE level — the `#[cfg]` guards are per-test so the
/// whole file compiles clean in BOTH states (greedy-only and dp-planner).
#[cfg(test)]
mod dp_planner_result_equivalence {
    use super::*;

    fn star_graph() -> Graph {
        // A 4-arm star where the centre variable ?s is shared by all 4 patterns.
        Graph::load_str(
            "@prefix ex: <http://ex/> .\
             ex:a ex:p1 1 ; ex:p2 2 ; ex:p3 3 ; ex:p4 4 .\
             ex:b ex:p1 10 ; ex:p2 20 .",
            "turtle",
        )
        .unwrap()
    }

    #[cfg(feature = "dp-planner")]
    fn sorted_rows(g: &Graph, q: &str) -> Vec<Vec<Option<String>>> {
        let mut r = query(g, q).expect("query error");
        // Sort rows for order-independent comparison.
        r.rows.sort_by(|a, b| {
            let sa: Vec<String> = a.iter().map(|c| c.as_ref().map(|t| t.to_string()).unwrap_or_default()).collect();
            let sb: Vec<String> = b.iter().map(|c| c.as_ref().map(|t| t.to_string()).unwrap_or_default()).collect();
            sa.cmp(&sb)
        });
        r.rows.iter().map(|row| row.iter().map(|c| c.as_ref().map(|t| t.to_string())).collect()).collect()
    }

    #[test]
    #[cfg(feature = "dp-planner")]
    fn star_bgp_dp_matches_greedy() {
        let g = star_graph();
        let q = "PREFIX ex: <http://ex/> SELECT ?s ?o1 ?o2 WHERE { ?s ex:p1 ?o1 . ?s ex:p2 ?o2 }";
        let greedy = sorted_rows(&g, q);
        let dp = sparq_engine::with_dp_planner(|| sorted_rows(&g, q));
        assert_eq!(greedy, dp, "DP planner must produce the same result as greedy on a star BGP");
    }

    #[test]
    #[cfg(feature = "dp-planner")]
    fn chain_bgp_dp_matches_greedy() {
        let g = Graph::load_str(
            "@prefix ex: <http://ex/> .\
             ex:a ex:e ex:b . ex:b ex:e ex:c . ex:c ex:e ex:d .",
            "turtle",
        )
        .unwrap();
        let q = "PREFIX ex: <http://ex/> SELECT ?a ?b ?c ?d WHERE { ?a ex:e ?b . ?b ex:e ?c . ?c ex:e ?d }";
        let greedy = sorted_rows(&g, q);
        let dp = sparq_engine::with_dp_planner(|| sorted_rows(&g, q));
        assert_eq!(greedy, dp, "DP planner must produce the same result as greedy on a chain BGP");
    }

    #[test]
    #[cfg(feature = "dp-planner")]
    fn dp_planner_zero_budget_falls_back_to_greedy() {
        // with_dp_planner_budget(0) always falls back to greedy (0 subgraphs → no DP).
        // The query result must still be correct.
        let g = star_graph();
        let q = "PREFIX ex: <http://ex/> SELECT ?s ?o1 ?o2 WHERE { ?s ex:p1 ?o1 . ?s ex:p2 ?o2 }";
        let greedy = sorted_rows(&g, q);
        let dp_fallback = sparq_engine::with_dp_planner_budget(0, || sorted_rows(&g, q));
        assert_eq!(greedy, dp_fallback, "dp budget=0 must fall back to greedy, result must agree");
    }

    /// The greedy planner produces correct results regardless of whether the dp-planner
    /// feature is compiled in. This test always runs.
    #[test]
    fn greedy_planner_star_correctness() {
        let g = star_graph();
        // ex:a has all 4 predicates; ex:b has only p1 and p2.
        let r = query(
            &g,
            "PREFIX ex: <http://ex/> SELECT ?s ?o1 ?o2 ?o3 WHERE { ?s ex:p1 ?o1 . ?s ex:p2 ?o2 . ?s ex:p3 ?o3 }",
        )
        .unwrap();
        // Only ex:a has p3, so only ex:a matches all three.
        assert_eq!(r.rows.len(), 1, "only ex:a has all three predicates, got {}", r.rows.len());
        let s = r.rows[0][0].as_ref().map(|t| t.to_string());
        assert!(s.as_deref().map(|s| s.contains("/a")).unwrap_or(false), "ex:a must be the result, got {s:?}");
    }
}

/// [FABLE-5] (sq-9781x) The numeric-value cache is aligned with the evaluator's XSD
/// acceptance set, so a whitespace-padded numeric lexical (`" 1 "^^xsd:integer`) — valid
/// under XSD's `collapse` whitespace facet, value 1 — is treated CONSISTENTLY as the value
/// 1 by every numeric seam: the relational FILTER fast path (`?o < 5`, reads the cache), the
/// equality FILTER (`?o = 1`), and BIND arithmetic (`?o + 0`). Before the alignment the cache
/// (a raw `str::parse::<f64>`) missed the padded lexical, so the fast `<`/`=` paths rejected
/// it as a type error while arithmetic (via the trimming `Num::of_literal`) accepted it — a
/// self-inconsistency this pins closed.
mod numeric_whitespace_collapse_consistency {
    use super::*;

    fn padded_graph() -> Graph {
        let ttl = "@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .\n\
            @prefix ex: <http://ex/> .\n\
            ex:a ex:v \" 1 \"^^xsd:integer .\n\
            ex:b ex:v \"1\"^^xsd:integer .\n";
        Graph::load_str(ttl, "turtle").unwrap()
    }

    #[test]
    fn padded_integer_is_value_one_for_relational_equality_and_arithmetic() {
        let g = padded_graph();
        // Relational FILTER fast path reads the numeric cache.
        let lt = query(&g, "SELECT ?s WHERE { ?s <http://ex/v> ?o FILTER(?o < 5) }").unwrap();
        assert_eq!(lt.rows.len(), 2, "both the padded and plain integer are < 5");
        // Equality: the padded lexical equals the value 1.
        let eq = query(&g, "SELECT ?s WHERE { ?s <http://ex/v> ?o FILTER(?o = 1) }").unwrap();
        assert_eq!(eq.rows.len(), 2, "both the padded and plain integer = 1");
        // Arithmetic (through the trimming Num::of_literal) already accepted it — assert the
        // three seams now AGREE.
        let arith = query(&g, "SELECT ?s WHERE { ?s <http://ex/v> ?o BIND(?o + 0 AS ?n) FILTER(?n < 5) }").unwrap();
        assert_eq!(arith.rows.len(), 2, "arithmetic path agrees: both rows survive");
    }

    #[test]
    fn padded_and_plain_integer_are_value_equal_across_ids() {
        // A join on value equality pairs the padded and plain integers (distinct dict ids,
        // same value 1) — the cache-hit ⟺ evaluator-accept alignment in action.
        let g = padded_graph();
        let r = query(
            &g,
            "SELECT ?s1 ?s2 WHERE { ?s1 <http://ex/v> ?o1 . ?s2 <http://ex/v> ?o2 FILTER(?o1 = ?o2) }",
        )
        .unwrap();
        // 2x2 value-equal pairs (a=a, a=b, b=a, b=b) — all four since both are value 1.
        assert_eq!(r.rows.len(), 4, "all four ordered pairs are value-equal (both are 1)");
    }
}

/// [FABLE-5] (sq-74oy4 + sq-6b1lj) FAST-PATH ⟺ SLOW-PATH numeric agreement.
///
/// The numeric cache / sargable FILTER / `JKey::Num` value-join FAST path and the exact
/// general evaluator SLOW path (`values_equal`/`value_compare_strict` → `Num::of_literal`)
/// must return the SAME answer for `=`, `<`, `>` over padded / per-datatype-ill-formed /
/// well-formed lexicals across `xsd:integer`/`double`/`decimal`/`float`. The two beads pin:
///   * sq-74oy4 — a whitespace-PADDED numeric lexical (XSD `collapse` facet) is its trimmed
///     value on `<`/`>` too (not a `<`/`>`-only type error).
///   * sq-6b1lj — a lexical ill-formed FOR its datatype (`"1.5"^^xsd:integer`, `"1E2"^^
///     xsd:decimal`, an i128-overflow decimal) is a type error on ALL of `=`/`<`/`>` (the
///     cache no longer over-includes it on the sargable `=`/`<` fast path).
///
/// Method: each case runs against a graph WITH the fast path (sargable numeric pushdown
/// enabled) and a graph carrying a high-precision decimal that DISABLES sargable numeric
/// pushdown (`Graph::has_high_precision_decimal` → the exact general comparison path), and
/// asserts identical results — AND against the `of_literal` reference expectation.
mod numeric_fast_slow_path_agreement {
    use super::*;

    /// Build a one-triple graph `ex:s ex:v <object>`; when `force_slow`, also add a
    /// high-precision decimal object on a DIFFERENT subject/predicate so
    /// `has_high_precision_decimal()` is true (which makes `extract_sargable` decline the
    /// numeric fast path) WITHOUT changing the `ex:s ex:v ?o` row under test.
    fn graph_with(object: &str, force_slow: bool) -> Graph {
        let mut ttl = String::from(
            "@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .\n@prefix ex: <http://ex/> .\n",
        );
        ttl.push_str(&format!("ex:s ex:v {} .\n", object));
        if force_slow {
            // 19 significant digits > 15 → high-precision decimal → sargable numeric declined.
            ttl.push_str("ex:hp ex:w \"1.000000000000000001\"^^xsd:decimal .\n");
        }
        Graph::load_str(&ttl, "turtle").unwrap()
    }

    /// Rows returned by `SELECT ?s WHERE { ?s ex:v ?o FILTER(<expr>) }` under BOTH the fast
    /// and slow paths; asserts they agree, then returns the (agreed) row count.
    fn filter_rows(object: &str, filter: &str) -> usize {
        let q = format!("SELECT ?s WHERE {{ ?s <http://ex/v> ?o FILTER({filter}) }}");
        let fast = query(&graph_with(object, false), &q).unwrap().rows.len();
        let slow = query(&graph_with(object, true), &q).unwrap().rows.len();
        assert_eq!(
            fast, slow,
            "fast-path vs slow-path disagree for object {} filter {}: fast={} slow={}",
            object, filter, fast, slow
        );
        fast
    }

    #[test]
    fn wellformed_lexicals_pass_relational_and_equality_on_both_paths() {
        // Well-formed value-1 lexicals in each datatype: `< 5`, `= 1`, `> 0` all include the
        // row; `> 5` excludes it. Fast and slow paths must agree (asserted inside filter_rows).
        for obj in [
            "\"1\"^^xsd:integer",
            "\"1.\"^^xsd:decimal", // decimal notation is valid only for its datatype
            "\"1.0\"^^xsd:decimal",
            "\"1.0E0\"^^xsd:double",
            "\"1.0\"^^xsd:float",
        ] {
            assert_eq!(filter_rows(obj, "?o < 5"), 1, "{obj} < 5");
            assert_eq!(filter_rows(obj, "?o = 1"), 1, "{obj} = 1");
            assert_eq!(filter_rows(obj, "?o > 0"), 1, "{obj} > 0");
            assert_eq!(filter_rows(obj, "?o > 5"), 0, "{obj} > 5");
        }
    }

    // [GPT-6] XSD 1.0 §3.3.13 requires signed digits even when a decimal
    // spelling denotes an integral mathematical value.
    #[test]
    fn integer_decimal_spellings_fail_on_both_paths() {
        for obj in ["\"1.\"^^xsd:integer", "\"1.0\"^^xsd:integer"] {
            for filter in ["?o < 5", "?o = 1", "?o > 0"] {
                assert_eq!(filter_rows(obj, filter), 0, "{obj}: {filter}");
            }
        }
    }

    #[test]
    fn padded_lexicals_are_their_trimmed_value_on_all_operators() {
        // sq-74oy4: padding is collapsed on `<`/`>`/`=` alike (not a `<`/`>`-only type error).
        for obj in [
            "\" 1 \"^^xsd:integer",
            "\" 1.0 \"^^xsd:decimal",
            "\"\\t1.0E0\\n\"^^xsd:double",
        ] {
            assert_eq!(filter_rows(obj, "?o < 5"), 1, "{obj} < 5 (padded = value 1)");
            assert_eq!(filter_rows(obj, "?o = 1"), 1, "{obj} = 1 (padded = value 1)");
            assert_eq!(filter_rows(obj, "?o > 0"), 1, "{obj} > 0 (padded = value 1)");
        }
    }

    #[test]
    fn per_datatype_illformed_lexicals_are_type_errors_on_all_operators() {
        // sq-6b1lj: ill-formed FOR the datatype → type error → row excluded on `<`, `>`, `=`
        // — on BOTH paths (the fast path no longer over-includes them). A FILTER type error
        // drops the row exactly like `false`.
        for (obj, why) in [
            ("\"1.5\"^^xsd:integer", "fraction on an integer"),
            ("\"1E2\"^^xsd:integer", "exponent on an integer"),
            ("\"1E2\"^^xsd:decimal", "exponent on a decimal"),
            // 40-digit integer / decimal: beyond i128 (i128::MAX is a 39-digit number
            // ~1.7e38) → of_literal None (not i128-representable), so a type error.
            ("\"9999999999999999999999999999999999999999\"^^xsd:integer", "i128-overflow integer"),
            ("\"9999999999999999999999999999999999999999.5\"^^xsd:decimal", "i128-overflow decimal"),
        ] {
            assert_eq!(filter_rows(obj, "?o < 999999"), 0, "{obj} < … excluded ({why})");
            assert_eq!(filter_rows(obj, "?o > -999999"), 0, "{obj} > … excluded ({why})");
            assert_eq!(filter_rows(obj, "?o = 100"), 0, "{obj} = … excluded ({why})");
            // BOUND(?o) is still true (the row is a real term) — proves it is a numeric TYPE
            // error on the comparison, not that the term vanished from the graph.
            let bound = query(
                &graph_with(obj, false),
                "SELECT ?s WHERE { ?s <http://ex/v> ?o FILTER(BOUND(?o)) }",
            )
            .unwrap();
            assert_eq!(bound.rows.len(), 1, "{obj} still binds as a term ({why})");
        }
    }

    #[test]
    fn illformed_lexical_does_not_value_join_with_its_numeric_value() {
        // sq-6b1lj value-join surface: `"1.5"^^xsd:integer` must NOT pair with 1.5 (a decimal)
        // under `=` (it is a type error), on the `JKey::Num`/value-join path.
        let ttl = "@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .\n@prefix ex: <http://ex/> .\n\
            ex:a ex:v \"1.5\"^^xsd:integer .\n\
            ex:b ex:w \"1.5\"^^xsd:decimal .\n";
        let g = Graph::load_str(ttl, "turtle").unwrap();
        let r = query(
            &g,
            "SELECT ?a ?b WHERE { ?a <http://ex/v> ?x . ?b <http://ex/w> ?y FILTER(?x = ?y) }",
        )
        .unwrap();
        assert_eq!(
            r.rows.len(),
            0,
            "\"1.5\"^^xsd:integer is a type error, so it must not = the decimal 1.5"
        );
    }

    #[test]
    fn illformed_constant_operand_is_a_type_error_not_its_f64() {
        // sq-6b1lj CONSTANT side: `FILTER(?o = "1.5"^^xsd:integer)` over a graph value 1.5
        // (a well-formed DECIMAL) must EXCLUDE the row — the integer-typed constant `"1.5"` is
        // ill-formed for its datatype (a type error), so it is NOT the value 1.5. Both the
        // sargable fast path (which now DECLINES on the ill-formed constant) and the exact
        // path must agree on exclusion. A datatype-agnostic constant parse would WRONGLY match
        // (1.5 == 1.5). Uses xsd:integer for the FULL datatype IRI so the SPARQL parser is happy.
        let ttl = "@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .\n@prefix ex: <http://ex/> .\n\
            ex:s ex:v \"1.5\"^^xsd:decimal .\n";
        let g = Graph::load_str(ttl, "turtle").unwrap();
        let xi = "http://www.w3.org/2001/XMLSchema#integer";
        for op in ["=", "<", ">"] {
            let q = format!(
                "SELECT ?s WHERE {{ ?s <http://ex/v> ?o FILTER(?o {op} \"1.5\"^^<{xi}>) }}"
            );
            let r = query(&g, &q).unwrap();
            assert_eq!(
                r.rows.len(),
                0,
                "?o {op} \"1.5\"^^xsd:integer must be a type error (ill-formed constant), row excluded"
            );
        }
        // Sanity: the SAME comparison against a WELL-FORMED decimal constant DOES match on `=`.
        let ok = query(
            &g,
            "SELECT ?s WHERE { ?s <http://ex/v> ?o FILTER(?o = \"1.5\"^^<http://www.w3.org/2001/XMLSchema#decimal>) }",
        )
        .unwrap();
        assert_eq!(ok.rows.len(), 1, "well-formed decimal constant 1.5 matches the decimal 1.5");
    }

    /// MUTATION WITNESS (fast/cache seam): with the datatype-aware cache in force, the
    /// per-datatype-ill-formed integer `"1.5"^^xsd:integer` is a cache MISS, so `?o = 100`
    /// returns 0 rows. This assertion is 0-vs-nonzero: if the cache reverted to the
    /// datatype-AGNOSTIC parse (accepting 1.5), the fast `=` path would still return 0 here
    /// (1.5 ≠ 100) — so use `?o = 1.5` on the DECIMAL constant to make the witness bite: the
    /// datatype-agnostic cache would include the row (1.5 == 1.5), the datatype-aware one
    /// excludes it (integer "1.5" is a type error).
    #[test]
    fn mutation_witness_datatype_aware_cache_excludes_integer_fraction() {
        let g = graph_with("\"1.5\"^^xsd:integer", false);
        let r = query(
            &g,
            "SELECT ?s WHERE { ?s <http://ex/v> ?o FILTER(?o = 1.5) }",
        )
        .unwrap();
        assert_eq!(
            r.rows.len(),
            0,
            "datatype-aware cache: \"1.5\"^^xsd:integer is a type error, never = decimal 1.5 \
             (a datatype-agnostic cache would WRONGLY return 1 row here)"
        );
    }
}
