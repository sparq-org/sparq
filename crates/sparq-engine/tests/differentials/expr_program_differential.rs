//! [FABLE-5] (sq-7d3dj.11) Differential + spec-pinned tests for the opt-in flat compiled
//! scalar-expression programs (`expr-program` feature).
//!
//! # What the feature does
//!
//! `compile_expr` (sq-7d3dj.4) resolves every `Variable` in a FILTER/BIND expression to a
//! column index once per operator, but leaves a `Box`-linked TREE that `eval_compiled`
//! re-walks recursively for every row. With `expr-program` on, that tree is lowered ONCE per
//! operator into a FLAT postfix program over the same resolved columns plus pre-built
//! constants, evaluated per row by a straight loop over a reusable value stack.
//!
//! # Why these assertions are a real differential
//!
//! The load-bearing invariant is EXACT RESULT IDENTITY — not "equivalent", identical rows in
//! the same multiset, because the lowered path must preserve SPARQL's three-valued logic,
//! type-error propagation and TERM IDENTITY (`sameTerm` / `STR` / BIND passthrough see the
//! original term, not a value-canonicalised one). `expr_program_testing::set_enabled` flips
//! lowering at RUNTIME, so every query below runs BOTH evaluators in ONE binary against the
//! same graph, and the two row multisets must be equal.
//!
//! Two things keep this from being a vacuous "the engine equals itself" test:
//!
//! * ANTI-VACUITY (feature-ON build only): `expr_program_testing::lowered()` must be `> 0`
//!   after the "on" leg (the flat evaluator really ran) and `0` after the "off" leg (the tree
//!   evaluator really ran). Without that counter an on/off differential would still pass if
//!   the toggle did nothing.
//! * SPEC-PINNED EXPECTATIONS: the shapes whose semantics the lowering could plausibly break
//!   (3VL short-circuiting, `!error`, `IF`/`COALESCE` over a type error, `IN` with an
//!   erroring member, `sameTerm` vs `=`) also carry the answer SPARQL 1.1 requires, asserted
//!   in BOTH feature states. So the suite fails if the tree evaluator and the program agree
//!   on a WRONG answer, and it is a live conformance suite when the feature is off.

use sparq_core::Graph;
use sparq_engine::query;

const PFX: &str = "PREFIX ex: <http://example.org/>\n\
                   PREFIX foaf: <http://xmlns.com/foaf/0.1/>\n\
                   PREFIX xsd: <http://www.w3.org/2001/XMLSchema#>\n\
                   PREFIX rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#>\n";

type Table = Vec<Vec<Option<String>>>;

fn load(ttl: &str) -> Graph {
    Graph::load_str(&format!("{PFX}{ttl}"), "turtle").expect("test graph")
}

/// Result rows as a SORTED multiset (order-insensitive, MULTIPLICITY-preserving). Cells keep
/// their full term serialisation, so a lost language tag / datatype / IRI-vs-literal
/// distinction — i.e. any term-identity drift — shows up as an inequality.
fn multiset(g: &Graph, q: &str) -> Table {
    let r = query(g, &format!("{PFX}{q}")).expect("query failed");
    let mut rows: Table = r
        .rows
        .iter()
        .map(|row| row.iter().map(|c| c.as_ref().map(|t| t.to_string())).collect())
        .collect();
    rows.sort();
    rows
}

/// `(tree, program, lowered)` — the same query evaluated with lowering OFF then ON, plus how
/// many operators the ON leg lowered. With the feature compiled out both legs are the tree
/// evaluator (the equality is then trivially true, but the spec-pinned expectations below
/// still run, so the file doubles as a conformance suite in the default build).
fn tree_vs_program(g: &Graph, q: &str) -> (Table, Table, usize) {
    #[cfg(feature = "expr-program")]
    {
        use sparq_engine::expr_program_testing as ep;
        let prev = ep::set_enabled(false);
        ep::reset_stats();
        let tree = multiset(g, q);
        let tree_lowered = ep::lowered();
        ep::set_enabled(true);
        ep::reset_stats();
        let program = multiset(g, q);
        let program_lowered = ep::lowered();
        ep::set_enabled(prev);
        // The toggle must really select the evaluator: the tree leg must lower NOTHING.
        assert_eq!(tree_lowered, 0, "lowering must be OFF for the tree leg of `{q}`");
        (tree, program, program_lowered)
    }
    #[cfg(not(feature = "expr-program"))]
    {
        let t = multiset(g, q);
        (t.clone(), t, 0)
    }
}

/// Asserts tree == program, that the ON leg really lowered an operator (unless `lowers` is
/// false), and — when given — that both equal `expected`.
#[track_caller]
fn check(g: &Graph, q: &str, expected: Option<Table>, lowers: bool) {
    let (tree, program, lowered) = tree_vs_program(g, q);
    assert_eq!(tree, program, "tree vs flat program disagree on `{q}`");
    if cfg!(feature = "expr-program") && lowers {
        // ANTI-VACUITY: without this an on/off differential would pass even if the toggle,
        // or the lowering itself, did nothing.
        assert!(lowered > 0, "the program leg of `{q}` lowered NOTHING — vacuous differential");
    }
    if let Some(mut expected) = expected {
        expected.sort();
        assert_eq!(tree, expected, "unexpected result for `{q}`");
    }
}

/// tree == program == `expected`, with the anti-vacuity check on.
#[track_caller]
fn assert_same(g: &Graph, q: &str, expected: Table) {
    check(g, q, Some(expected), true);
}

/// tree == program only (for shapes whose expected table is long or engine-specific).
#[track_caller]
fn assert_identical(g: &Graph, q: &str) {
    check(g, q, None, true);
}

/// tree == program == `expected` for a query whose FILTER is SARGABLE and therefore pushed
/// down into the BGP scan — no scalar FILTER operator survives for the lowering to see, so
/// the anti-vacuity counter is legitimately zero. The assertion still earns its place: it
/// pins that enabling the feature does not disturb the pushdown decision.
#[track_caller]
fn assert_same_pushed_down(g: &Graph, q: &str, expected: Table) {
    check(g, q, Some(expected), false);
}

/// A PLAIN (simple) literal cell, in the engine's term serialisation.
fn lit(s: &str) -> Option<String> {
    Some(format!("\"{s}\""))
}

/// A one-cell row holding a plain literal.
fn one(s: &str) -> Vec<Option<String>> {
    vec![lit(s)]
}

/// A one-cell row holding a term written out VERBATIM (IRI, typed or language-tagged
/// literal) — the spelling that pins datatype / language / IRI-ness.
fn raw(s: &str) -> Vec<Option<String>> {
    vec![Some(s.to_string())]
}

fn dataset() -> Graph {
    load(
        "ex:a foaf:name \"alice\" ; ex:age 30 ; ex:score \"1\"^^xsd:integer ; ex:tag \"x\"@en .
         ex:b foaf:name \"bob\"   ; ex:age 25 ; ex:score \"1.0\"^^xsd:decimal .
         ex:c foaf:name \"carol\" ; ex:age 41 ; ex:score \"01\"^^xsd:integer ; ex:tag \"x\" .
         ex:d foaf:name \"dave\"  ; ex:age 25 ; ex:nick \"dodgy\"^^ex:weird .
         ex:e foaf:name \"erin\"  ; ex:homepage <http://example.org/erin> .",
    )
}

// ── Three-valued logic and short-circuiting ─────────────────────────────────────────────
// `&&` is false-dominant and `||` true-dominant EVEN WHEN the other operand is a type error,
// and the engine short-circuits, so the erroring arm is never evaluated. The lowered `&&`/`||`
// carry the left operand's THREE-VALUED EBV across the jump; if that normalisation lost the
// error/true distinction these would flip.

#[test]
fn and_is_false_dominant_over_an_error() {
    // `?nope` is never bound -> its EBV is a type error. `false && error` is FALSE, so every
    // row survives the outer `!`.
    assert_same(
        &dataset(),
        "SELECT ?n WHERE { ?s foaf:name ?n . OPTIONAL { ?s ex:missing ?nope } \
         FILTER (!(false && ?nope)) } ORDER BY ?n",
        vec![one("alice"), one("bob"), one("carol"), one("dave"), one("erin")],
    );
}

#[test]
fn or_is_true_dominant_over_an_error() {
    assert_same(
        &dataset(),
        "SELECT ?n WHERE { ?s foaf:name ?n . OPTIONAL { ?s ex:missing ?nope } \
         FILTER (true || ?nope) }",
        vec![one("alice"), one("bob"), one("carol"), one("dave"), one("erin")],
    );
}

#[test]
fn and_with_an_error_and_true_is_an_error_not_true() {
    // `error && true` is an ERROR, whose effective boolean value is false -> NO rows.
    assert_same(
        &dataset(),
        "SELECT ?n WHERE { ?s foaf:name ?n . OPTIONAL { ?s ex:missing ?nope } \
         FILTER (?nope && true) }",
        vec![],
    );
}

#[test]
fn or_with_an_error_and_false_is_an_error_not_false() {
    // `error || false` is an ERROR (not `false`), so `!(…)` stays an error and drops the row —
    // this is the case a naive two-valued lowering turns into `true`.
    assert_same(
        &dataset(),
        "SELECT ?n WHERE { ?s foaf:name ?n . OPTIONAL { ?s ex:missing ?nope } \
         FILTER (!(?nope || false)) }",
        vec![],
    );
}

#[test]
fn not_of_an_error_stays_an_error() {
    // `?nope = 1` is a type error on an unbound operand; `!error` is an error, NOT `true`.
    assert_same(
        &dataset(),
        "SELECT ?n WHERE { ?s foaf:name ?n . OPTIONAL { ?s ex:missing ?nope } \
         FILTER (!(?nope = 1)) }",
        vec![],
    );
}

#[test]
fn bound_and_nested_boolean_spine() {
    // A deeper spine: the lowered form is a chain of jumps, so an off-by-one patch shows up
    // as the wrong survivor set.
    assert_same(
        &dataset(),
        "SELECT ?n WHERE { ?s foaf:name ?n ; ex:age ?a . \
         FILTER ((?a > 20 && ?a < 40) || (?a = 41 && !(?n = \"zed\"))) } ORDER BY ?n",
        vec![one("alice"), one("bob"), one("carol"), one("dave")],
    );
}

// ── IF / COALESCE over type errors ──────────────────────────────────────────────────────

#[test]
fn if_over_a_type_error_is_an_error_and_binds_unbound() {
    // A type-error condition makes IF a type error; BIND of an error leaves ?v UNBOUND.
    assert_same(
        &dataset(),
        "SELECT ?n ?v WHERE { ?s foaf:name ?n . OPTIONAL { ?s ex:missing ?nope } \
         BIND(IF(?nope, \"t\", \"f\") AS ?v) FILTER (?n = \"alice\") }",
        vec![vec![lit("alice"), None]],
    );
}

#[test]
fn if_selects_the_taken_arm_only() {
    assert_same(
        &dataset(),
        "SELECT ?n ?v WHERE { ?s foaf:name ?n ; ex:age ?a . BIND(IF(?a > 28, \"old\", \"young\") AS ?v) } \
         ORDER BY ?n",
        vec![
            vec![lit("alice"), lit("old")],
            vec![lit("bob"), lit("young")],
            vec![lit("carol"), lit("old")],
            vec![lit("dave"), lit("young")],
        ],
    );
}

#[test]
fn coalesce_skips_unbound_and_error_arms() {
    // Arm 1 is unbound, arm 2 is a type error (arithmetic on a non-numeric literal), arm 3 wins.
    assert_same(
        &dataset(),
        "SELECT ?v WHERE { ex:a foaf:name ?n . OPTIONAL { ex:a ex:missing ?nope } \
         BIND(COALESCE(?nope, ?n + 1, \"fallback\") AS ?v) }",
        vec![one("fallback")],
    );
}

#[test]
fn coalesce_with_no_usable_arm_is_unbound() {
    assert_same(
        &dataset(),
        "SELECT ?v WHERE { ex:a foaf:name ?n . OPTIONAL { ex:a ex:missing ?nope } \
         BIND(COALESCE(?nope, ?n + 1) AS ?v) }",
        vec![vec![None]],
    );
}

// ── IN ──────────────────────────────────────────────────────────────────────────────────

#[test]
fn in_is_true_when_a_later_member_matches_after_an_erroring_one() {
    // `IN` returns true if ANY member is equal, even when another member's comparison errors.
    assert_same(
        &dataset(),
        "SELECT ?n WHERE { ?s foaf:name ?n ; ex:age ?a . OPTIONAL { ?s ex:missing ?nope } \
         FILTER (?a IN (?nope, 41)) }",
        vec![one("carol")],
    );
}

#[test]
fn in_with_no_match_but_an_erroring_member_is_an_error() {
    // No member matches and one comparison errored -> the whole `IN` is a type error, so the
    // row is dropped rather than treated as `false` (which `!(…)` would then let through).
    assert_same(
        &dataset(),
        "SELECT ?n WHERE { ?s foaf:name ?n ; ex:age ?a . OPTIONAL { ?s ex:missing ?nope } \
         FILTER (!(?a IN (?nope, 999))) }",
        vec![],
    );
}

#[test]
fn not_in_with_only_clean_members() {
    assert_same(
        &dataset(),
        "SELECT ?n WHERE { ?s foaf:name ?n ; ex:age ?a . FILTER (?a NOT IN (25, 41)) }",
        vec![one("alice")],
    );
}

// ── Term identity: sameTerm / STR / BIND passthrough ────────────────────────────────────

#[test]
fn sameterm_is_identity_where_equality_is_value_equality() {
    // "1"^^xsd:integer and "01"^^xsd:integer are VALUE-equal but NOT the same term; "1.0"^^decimal
    // is value-equal to both across datatypes. `sameTerm` must see the original terms.
    let g = dataset();
    assert_same(
        &g,
        "SELECT ?n WHERE { ?s foaf:name ?n ; ex:score ?v . FILTER (sameTerm(?v, \"1\"^^xsd:integer)) }",
        vec![one("alice")],
    );
    assert_same_pushed_down(
        &g,
        "SELECT ?n WHERE { ?s foaf:name ?n ; ex:score ?v . FILTER (?v = \"1\"^^xsd:integer) } ORDER BY ?n",
        vec![one("alice"), one("bob"), one("carol")],
    );
}

#[test]
fn bind_passthrough_preserves_the_exact_term() {
    // A language tag, a plain literal and an IRI must all survive `BIND(?x AS ?y)` unchanged —
    // `Op::Var` must materialise the row's term, not a canonicalised value.
    assert_same(
        &dataset(),
        "SELECT ?t WHERE { ?s ex:tag ?x . BIND(?x AS ?t) } ORDER BY ?t",
        vec![one("x"), raw("\"x\"@en")],
    );
    assert_same(
        &dataset(),
        "SELECT ?h WHERE { ?s ex:homepage ?p . BIND(?p AS ?h) }",
        vec![raw("<http://example.org/erin>")],
    );
}

#[test]
fn str_and_lang_see_the_original_term() {
    assert_same(
        &dataset(),
        "SELECT ?s2 ?l WHERE { ?s ex:tag ?x . BIND(STR(?x) AS ?s2) BIND(LANG(?x) AS ?l) } ORDER BY ?l",
        vec![
            vec![lit("x"), lit("")],
            vec![lit("x"), lit("en")],
        ],
    );
}

// ── Arithmetic and unary sign (lowered ops) vs comparisons (delegated) ──────────────────

#[test]
fn arithmetic_keeps_its_exact_typed_result() {
    // Integer + integer stays integer, integer / integer is DECIMAL per SPARQL — the lowered
    // `Op::Arith` must reuse the same typed `Num::binop`, not an f64 shortcut.
    assert_same(
        &dataset(),
        "SELECT ?v WHERE { ex:a ex:age ?a . BIND(?a + 12 AS ?v) }",
        vec![raw("\"42\"^^<http://www.w3.org/2001/XMLSchema#integer>")],
    );
    assert_same(
        &dataset(),
        "SELECT ?v WHERE { ex:a ex:age ?a . BIND(?a / 4 AS ?v) }",
        vec![raw("\"7.5\"^^<http://www.w3.org/2001/XMLSchema#decimal>")],
    );
    assert_same(
        &dataset(),
        "SELECT ?v WHERE { ex:a ex:age ?a . BIND(-?a AS ?v) }",
        vec![raw("\"-30\"^^<http://www.w3.org/2001/XMLSchema#integer>")],
    );
}

#[test]
fn arithmetic_on_a_non_numeric_is_an_error_binding_unbound() {
    assert_same(
        &dataset(),
        "SELECT ?v WHERE { ex:d ex:nick ?k . BIND(?k + 1 AS ?v) }",
        vec![vec![None]],
    );
}

#[test]
fn comparison_over_arithmetic_keeps_the_exact_decimal_path() {
    // The comparison operators are deliberately NOT lowered because their fast paths dispatch
    // on the operand TREE (`compiled_expr_has_arith` -> exact decimal). This pins that the
    // lowering did not accidentally reroute them.
    assert_identical(
        &dataset(),
        "SELECT ?n WHERE { ?s foaf:name ?n ; ex:age ?a . FILTER (?a * 0.1 > 2.5) } ORDER BY ?n",
    );
    assert_same(
        &dataset(),
        "SELECT ?n WHERE { ?s foaf:name ?n ; ex:age ?a . FILTER (?a * 0.1 = 2.5) }",
        vec![one("bob"), one("dave")],
    );
}

// ── Function calls: lowered ARGUMENTS, unchanged lazy body ──────────────────────────────

#[test]
fn string_functions_over_lowered_arguments() {
    assert_same(
        &dataset(),
        "SELECT ?n WHERE { ?s foaf:name ?n . FILTER (STRSTARTS(?n, \"a\") || CONTAINS(?n, \"ro\")) } ORDER BY ?n",
        vec![one("alice"), one("carol")],
    );
    assert_same(
        &dataset(),
        "SELECT ?v WHERE { ex:a foaf:name ?n . BIND(CONCAT(UCASE(SUBSTR(?n, 1, 1)), SUBSTR(?n, 2)) AS ?v) }",
        vec![one("Alice")],
    );
}

#[test]
fn nested_function_arguments_are_evaluated_in_order() {
    // Deep nesting exercises the sub-program stack discipline (each argument runs above the
    // caller's base and restores it) — a leak would corrupt the enclosing expression.
    assert_same(
        &dataset(),
        "SELECT ?v WHERE { ex:a foaf:name ?n ; ex:age ?a . \
         BIND(CONCAT(?n, \"-\", STR(ABS(0 - ?a)), \"-\", IF(?a > 10, \"hi\", \"lo\")) AS ?v) }",
        vec![one("alice-30-hi")],
    );
}

#[test]
fn a_function_argument_that_errors_propagates() {
    assert_same(
        &dataset(),
        "SELECT ?v WHERE { ex:d ex:nick ?k . BIND(STRLEN(UCASE(?k)) AS ?v) }",
        vec![vec![None]],
    );
}

#[test]
fn datatype_and_isiri_over_mixed_terms() {
    assert_identical(
        &dataset(),
        "SELECT ?n ?d WHERE { ?s foaf:name ?n . OPTIONAL { ?s ex:score ?v } \
         BIND(IF(BOUND(?v), DATATYPE(?v), \"none\") AS ?d) } ORDER BY ?n",
    );
}

// ── EXISTS (delegated) inside a lowered spine ───────────────────────────────────────────

#[test]
fn exists_inside_a_lowered_boolean_spine() {
    // `EXISTS` re-enters pattern evaluation per row (and thus re-enters FILTER lowering for
    // the inner pattern). It stays an `Op::Tree` leaf; this pins that the re-entry neither
    // corrupts the outer program's stack nor changes the answer.
    assert_same(
        &dataset(),
        "SELECT ?n WHERE { ?s foaf:name ?n . \
         FILTER (EXISTS { ?s ex:age ?a . FILTER (?a > 28) } && !EXISTS { ?s ex:tag \"zzz\" }) } ORDER BY ?n",
        vec![one("alice"), one("carol")],
    );
}

#[test]
fn exists_as_a_coalesce_arm_and_a_bind_source() {
    assert_identical(
        &dataset(),
        "SELECT ?n ?v WHERE { ?s foaf:name ?n . BIND(EXISTS { ?s ex:tag ?t } AS ?v) } ORDER BY ?n",
    );
}

// ── A larger row set: the parallel (rayon) FILTER/BIND path ─────────────────────────────

#[test]
fn parallel_row_counts_agree_between_evaluators() {
    // Well above the engine's parallel threshold, so the rayon path (one reused scratch stack
    // per worker) is the one under test. Row-for-row identity, not just cardinality.
    let mut ttl = String::new();
    for i in 0..4000u32 {
        ttl.push_str(&format!(
            "ex:s{i} foaf:name \"n{i}\" ; ex:age {} ; ex:tag \"t{}\" .\n",
            i % 97,
            i % 7
        ));
    }
    let g = load(&ttl);
    assert_identical(
        &g,
        "SELECT ?n ?v WHERE { ?s foaf:name ?n ; ex:age ?a ; ex:tag ?t . \
         FILTER ((?a > 10 && STRSTARTS(?t, \"t1\")) || (?a < 5 && ?t IN (\"t0\", \"t3\"))) \
         BIND(CONCAT(?n, \"/\", ?t) AS ?v) }",
    );
    assert_identical(
        &g,
        "SELECT ?v WHERE { ?s ex:age ?a . BIND(COALESCE(IF(?a > 50, ?a * 2, 1/0), -1) AS ?v) }",
    );
}

// ── Non-vacuity of the differential itself ──────────────────────────────────────────────

#[test]
#[cfg(feature = "expr-program")]
fn the_counter_distinguishes_the_two_legs() {
    // A mutation check on the harness: if `set_enabled` were a no-op (or `lowered()` always
    // returned 0) every assertion above would pass vacuously. Assert the two legs are
    // observably different runs.
    use sparq_engine::expr_program_testing as ep;
    let g = dataset();
    let q = "SELECT ?n WHERE { ?s foaf:name ?n . FILTER (STRLEN(?n) > 3) }";
    let prev = ep::set_enabled(false);
    ep::reset_stats();
    let _ = multiset(&g, q);
    assert_eq!(ep::lowered(), 0, "lowering fired while disabled");
    ep::set_enabled(true);
    ep::reset_stats();
    let _ = multiset(&g, q);
    assert!(ep::lowered() > 0, "lowering never fired while enabled");
    ep::set_enabled(prev);
}
