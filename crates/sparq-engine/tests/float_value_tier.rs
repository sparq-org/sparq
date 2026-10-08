//! #3825 — an `xsd:float` literal is valued as its `f32` (then promoted exactly), on the
//! comparison / join / sort fast paths as on the arithmetic path. `4611686568183201792`
//! (2^62 + 2^39) is the correctly-rounded `f32` of `4611686293305294849`, so the float and
//! that double are EQUAL; the nearest-`f64` of the float's lexical made them unequal.

use sparq_core::Graph;
use sparq_engine::query;

const F: &str = "\"4611686293305294849\"^^<http://www.w3.org/2001/XMLSchema#float>";
const D: &str = "\"4611686568183201792\"^^<http://www.w3.org/2001/XMLSchema#double>";

fn graph() -> Graph {
    Graph::load_str(&format!("<http://ex/a> <http://ex/f> {F} .\n<http://ex/b> <http://ex/d> {D} .\n"), "turtle").unwrap()
}

fn ask(g: &Graph, q: &str) -> bool {
    !query(g, q).unwrap().rows.is_empty()
}

#[test]
fn float_compares_by_its_f32_value_on_every_path() {
    let g = graph();
    // Constant operands (the cmp_expr fast path).
    assert!(ask(&g, &format!("ASK {{ FILTER({F} = {D}) }}")));
    assert!(!ask(&g, &format!("ASK {{ FILTER({F} < {D}) }}")));
    assert!(!ask(&g, &format!("ASK {{ FILTER({F} != {D}) }}")));
    // Graph terms (the numeric-value cache).
    assert!(ask(&g, "ASK { ?a <http://ex/f> ?f . ?b <http://ex/d> ?d FILTER(?f = ?d) }"));
    assert!(!ask(&g, "ASK { ?a <http://ex/f> ?f . ?b <http://ex/d> ?d FILTER(?f < ?d) }"));
    assert!(!ask(&g, "ASK { ?a <http://ex/f> ?f . ?b <http://ex/d> ?d FILTER(?f > ?d) }"));
    // Sargable constant FILTER over a graph term.
    assert!(ask(&g, &format!("ASK {{ ?a <http://ex/f> ?f FILTER(?f = {D}) }}")));
    assert!(ask(&g, &format!("ASK {{ ?b <http://ex/d> ?d FILTER(?d = {F}) }}")));
    // Arithmetic already agreed: the float plus zero is that double.
    assert!(ask(&g, &format!("ASK {{ FILTER({F} + 0 = {D}) }}")));
}

#[test]
fn tiny_float_whose_f32_value_is_zero_has_false_ebv() {
    let g = graph();
    let tiny = "\"1e-50\"^^<http://www.w3.org/2001/XMLSchema#float>";
    assert!(!ask(&g, &format!("ASK {{ FILTER({tiny}) }}")));
    assert!(ask(&g, "ASK { FILTER(\"1e-50\"^^<http://www.w3.org/2001/XMLSchema#double>) }"));
}

// ── Float-tier promotion against integer / decimal operands ─────────────────────────────
// XPath compares an `xs:float` against an integer or decimal in the FLOAT tier: the other
// operand is rounded to `f32` first. `"0.1"^^xsd:float` is the f32 0.100000001490116…, and
// `0.1` promoted to float is that same f32, so the two are EQUAL although their f64 images
// differ. Likewise `"16777217"^^xsd:float` is 16777216.0f32, which the integer 16777217
// rounds to. Constants, graph terms, the sargable FILTER pushdown and its COUNT scan, and the
// equality join must all agree.

const XSD: &str = "http://www.w3.org/2001/XMLSchema#";

fn promo_graph() -> Graph {
    let nt = format!(
        "<http://ex/a> <http://ex/f> \"0.1\"^^<{XSD}float> .\n\
         <http://ex/b> <http://ex/d> \"0.1\"^^<{XSD}decimal> .\n\
         <http://ex/c> <http://ex/bigf> \"16777217\"^^<{XSD}float> .\n\
         <http://ex/d> <http://ex/bigi> \"16777217\"^^<{XSD}integer> .\n\
         <http://ex/e> <http://ex/dbl> \"0.1\"^^<{XSD}double> .\n"
    );
    Graph::load_str(&nt, "turtle").unwrap()
}

#[test]
fn float_vs_decimal_and_integer_constants_compare_in_the_float_tier() {
    let g = promo_graph();
    let f = format!("\"0.1\"^^<{XSD}float>");
    let bigf = format!("\"16777217\"^^<{XSD}float>");
    for (expr, want) in [
        (format!("{f} = 0.1"), true),
        (format!("0.1 = {f}"), true),
        (format!("{f} != 0.1"), false),
        (format!("{f} < 0.1"), false),
        (format!("{f} > 0.1"), false),
        (format!("{f} <= 0.1"), true),
        (format!("{f} >= 0.1"), true),
        (format!("{f} < 0.2"), true),
        (format!("{bigf} = 16777217"), true),
        (format!("{bigf} != 16777217"), false),
        (format!("{bigf} < 16777217"), false),
        (format!("16777217 > {bigf}"), false),
        // A double operand promotes the float to double: 0.1f32 is NOT the double 0.1.
        (format!("{f} = \"0.1\"^^<{XSD}double>"), false),
        (format!("{f} > \"0.1\"^^<{XSD}double>"), true),
        // Arithmetic keeps the float type.
        (format!("{f} + 0 = 0.1"), true),
    ] {
        assert_eq!(ask(&g, &format!("ASK {{ FILTER({expr}) }}")), want, "{expr}");
    }
}

#[test]
fn stored_float_vs_decimal_and_integer_compare_in_the_float_tier() {
    let g = promo_graph();
    for (filter, want) in [
        ("?x = ?y", true),
        ("?x != ?y", false),
        ("?x < ?y", false),
        ("?x > ?y", false),
        ("?y = ?x", true),
        ("?y > ?x", false),
    ] {
        for (px, py) in [("f", "d"), ("bigf", "bigi")] {
            let q = format!("ASK {{ ?a <http://ex/{px}> ?x . ?b <http://ex/{py}> ?y FILTER({filter}) }}");
            assert_eq!(ask(&g, &q), want, "{q}");
        }
    }
    // The float against the stored double: double tier, unequal.
    assert!(!ask(&g, "ASK { ?a <http://ex/f> ?x . ?b <http://ex/dbl> ?y FILTER(?x = ?y) }"));
}

#[test]
fn sargable_float_filters_compare_in_the_float_tier() {
    let g = promo_graph();
    let f = format!("\"0.1\"^^<{XSD}float>");
    let bigf = format!("\"16777217\"^^<{XSD}float>");
    for (pat, filter, want) in [
        ("f", "?v = 0.1".to_string(), 1),
        ("f", "?v > 0.1".to_string(), 0),
        ("f", "?v < 0.1".to_string(), 0),
        ("f", "?v >= 0.1".to_string(), 1),
        ("f", "?v != 0.1".to_string(), 0),
        ("f", "0.1 = ?v".to_string(), 1),
        ("bigf", "?v = 16777217".to_string(), 1),
        ("bigf", "?v < 16777217".to_string(), 0),
        ("d", format!("?v = {f}"), 1),
        ("d", format!("?v < {f}"), 0),
        ("d", format!("?v > {f}"), 0),
        ("bigi", format!("?v = {bigf}"), 1),
        ("bigi", format!("?v > {bigf}"), 0),
        ("dbl", format!("?v = {f}"), 0),
        ("dbl", format!("?v < {f}"), 1),
    ] {
        let body = format!("?s <http://ex/{pat}> ?v FILTER({filter})");
        let rows = query(&g, &format!("SELECT ?s WHERE {{ {body} }}")).unwrap().rows.len();
        assert_eq!(rows, want, "rows: {body}");
        let c = query(&g, &format!("SELECT (COUNT(*) AS ?c) WHERE {{ {body} }}")).unwrap();
        let got = c.rows[0][0].as_ref().map(|t| t.to_string()).unwrap_or_default();
        assert!(got.starts_with(&format!("\"{want}\"")), "COUNT {body}: {got}");
    }
}

// ── Arithmetic in a comparison is evaluated in its promoted tier ────────────────────────
// `"16777217"^^xsd:float` is 16777216.0f32; `+ 1` is FLOAT arithmetic, so the sum rounds back
// to 16777216, which is not the double 16777217. Untyped f64 arithmetic gives 16777217.

fn bool_of(g: &Graph, q: &str) -> bool {
    let r = query(g, q).unwrap();
    let t = r.rows[0][0].as_ref().expect("bound").to_string();
    t.starts_with("\"true\"")
}

fn bigf() -> String {
    format!("\"16777217\"^^<{XSD}float>")
}

/// Asserts each `(expr, want)` both as a FILTER and as a BIND-ed value.
fn check_filter_and_bind(g: &Graph, cases: &[(String, bool)]) {
    for (expr, want) in cases {
        assert_eq!(ask(g, &format!("ASK {{ FILTER({expr}) }}")), *want, "FILTER {expr}");
        assert_eq!(bool_of(g, &format!("SELECT ?b WHERE {{ BIND(({expr}) AS ?b) }}")), *want, "BIND {expr}");
    }
}

#[test]
fn float_arithmetic_equality_rounds_in_the_float_tier() {
    let f = bigf();
    check_filter_and_bind(
        &promo_graph(),
        &[
            (format!("{f} + 1 = 16777217e0"), false),
            (format!("{f} + 1 != 16777217e0"), true),
            (format!("{f} + 1 = 16777216e0"), true),
        ],
    );
}

#[test]
fn float_arithmetic_filter_agrees_with_a_bound_sum() {
    let g = promo_graph();
    let f = bigf();
    // The sum built by BIND (typed result construction) is the float 16777216.
    assert!(!ask(&g, &format!("ASK {{ BIND({f} + 1 AS ?s) FILTER(?s = 16777217e0) }}")));
    assert!(ask(&g, &format!("ASK {{ BIND({f} + 1 AS ?s) FILTER(?s = 16777216e0) }}")));
    // The inline FILTER form must give the same answers.
    assert!(!ask(&g, &format!("ASK {{ FILTER({f} + 1 = 16777217e0) }}")));
    assert!(ask(&g, &format!("ASK {{ FILTER({f} + 1 = 16777216e0) }}")));
}

#[test]
fn float_arithmetic_ordering_rounds_in_the_float_tier() {
    let f = bigf();
    check_filter_and_bind(
        &promo_graph(),
        &[
            (format!("{f} + 1 < 16777217e0"), true),
            (format!("{f} + 1 <= 16777217e0"), true),
            (format!("{f} + 1 > 16777217e0"), false),
            (format!("{f} + 1 >= 16777217e0"), false),
            (format!("16777217e0 > {f} + 1"), true),
        ],
    );
}

#[test]
fn integer_and_decimal_operands_are_promoted_before_float_arithmetic() {
    let two = format!("\"2\"^^<{XSD}float>");
    let half = format!("\"0.5\"^^<{XSD}float>");
    check_filter_and_bind(
        &promo_graph(),
        &[
            // 16777215 + 2 = 16777217, which rounds to 16777216 in the float tier.
            (format!("16777215 + {two} = 16777217e0"), false),
            (format!("16777215 + {two} = 16777216e0"), true),
            // 16777216.5 promotes to 16777216.0f32 first; adding 0.5 rounds back to it.
            (format!("16777216.5 + {half} = 16777216e0"), true),
            (format!("16777216.5 + {half} > 16777216e0"), false),
        ],
    );
}

// `"1.00000001"^^xsd:float` is 1.0f32; float division by 3 gives 0.33333334f32
// (0.3333333432674408 as a double), not the double quotient 0.3333333333333333.
const THIRD_F32: &str = "0.3333333432674408e0";
const THIRD_F64: &str = "0.3333333333333333e0";

#[test]
fn float_division_rounds_in_the_float_tier() {
    let f = format!("\"1.00000001\"^^<{XSD}float>");
    let three = format!("\"3\"^^<{XSD}float>");
    check_filter_and_bind(
        &promo_graph(),
        &[
            (format!("{f} / 3 = {THIRD_F64}"), false),
            (format!("{f} / 3 != {THIRD_F64}"), true),
            (format!("{f} / 3 = {THIRD_F32}"), true),
            (format!("{f} / 3 > {THIRD_F64}"), true),
            (format!("{f} / 3 <= {THIRD_F64}"), false),
            (format!("{THIRD_F64} < {f} / 3"), true),
            // An integer dividend is promoted to float before the division.
            (format!("1 / {three} = {THIRD_F64}"), false),
            (format!("1 / {three} = {THIRD_F32}"), true),
            // A unary sign over the quotient keeps the float value.
            (format!("-({f} / 3) = -{THIRD_F64}"), false),
            (format!("-({f} / 3) < -{THIRD_F64}"), true),
        ],
    );
}

#[test]
fn stored_float_division_rounds_in_the_float_tier() {
    let nt = format!("<http://ex/a> <http://ex/one> \"1.00000001\"^^<{XSD}float> .\n");
    let g = Graph::load_str(&nt, "turtle").unwrap();
    for (filter, want) in [
        (format!("?v / 3 = {THIRD_F64}"), false),
        (format!("?v / 3 = {THIRD_F32}"), true),
        (format!("?v / 3 > {THIRD_F64}"), true),
    ] {
        let q = format!("ASK {{ ?s <http://ex/one> ?v FILTER({filter}) }}");
        assert_eq!(ask(&g, &q), want, "{q}");
        let q = format!("SELECT ?b WHERE {{ ?s <http://ex/one> ?v BIND(({filter}) AS ?b) }}");
        assert_eq!(bool_of(&g, &q), want, "{q}");
    }
}

#[test]
fn stored_float_arithmetic_rounds_in_the_float_tier() {
    let g = promo_graph();
    for (filter, want) in [("?v + 1 = 16777217e0", false), ("?v + 1 < 16777217e0", true), ("?v + 1 = 16777216e0", true)] {
        let q = format!("ASK {{ ?s <http://ex/bigf> ?v FILTER({filter}) }}");
        assert_eq!(ask(&g, &q), want, "{q}");
        let q = format!("SELECT ?b WHERE {{ ?s <http://ex/bigf> ?v BIND(({filter}) AS ?b) }}");
        assert_eq!(bool_of(&g, &q), want, "{q}");
    }
}

/// A store saved while `numerics.bin` had no semantics header valued the float at its nearest
/// f64 (`0.1`). After the upgrade, the reopened store must value it as its f32.
#[test]
fn reopened_pre_header_store_values_floats_as_f32() {
    use oxrdf::{Literal, NamedNode, Term};
    let g = promo_graph();
    let dir = std::env::temp_dir().join(format!("sparq_float_tier_old_store_{}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    g.save(&dir).unwrap();
    let float = Term::Literal(Literal::new_typed_literal("0.1", NamedNode::new_unchecked(format!("{XSD}float"))));
    let fid = g.id_of(&float).unwrap();
    // The pre-header layout: `n` native f64, the float at the nearest f64 of its lexical.
    let old: Vec<u8> = (1..=g.dict.len() as u32)
        .map(|id| if id == fid { 0.1f64 } else { g.numeric_value(id).unwrap_or(f64::NAN) })
        .flat_map(f64::to_ne_bytes)
        .collect();
    std::fs::write(dir.join("numerics.bin"), old).unwrap();
    let g2 = Graph::open(&dir).unwrap();
    let f = format!("\"0.1\"^^<{XSD}float>");
    assert!(ask(&g2, &format!("ASK {{ ?a <http://ex/f> ?v FILTER(?v = {f}) }}")));
    assert!(ask(&g2, "ASK { ?a <http://ex/f> ?v FILTER(?v = 0.1) }"));
    assert!(!ask(&g2, &format!("ASK {{ ?a <http://ex/f> ?v FILTER(?v = \"0.1\"^^<{XSD}double>) }}")));
    drop(g2);
    std::fs::remove_dir_all(&dir).ok();
}

// ── Unordered (NaN) numeric comparisons are false, not errors ───────────────────────────
// XPath `op:numeric-less-than` and friends return false when an operand is NaN, and
// `op:numeric-equal` returns false (so `!=` is true). Incompatible operand types are a type
// error. The difference shows under negation and in a BIND.

/// The BIND-ed value of `expr`: `Some(bool)`, or `None` when unbound (an error).
fn bound_bool(g: &Graph, expr: &str) -> Option<bool> {
    let r = query(g, &format!("SELECT ?b WHERE {{ BIND(({expr}) AS ?b) }}")).unwrap();
    r.rows[0][0].as_ref().map(|t| t.to_string().starts_with("\"true\""))
}

#[test]
fn nan_arithmetic_comparisons_are_false_not_errors() {
    let g = promo_graph();
    let nanf = format!("\"NaN\"^^<{XSD}float>");
    for (expr, want) in [
        ("0e0 / 0e0 < 1".to_string(), false),
        ("0e0 / 0e0 > 1".to_string(), false),
        ("0e0 / 0e0 <= 1".to_string(), false),
        ("0e0 / 0e0 >= 1".to_string(), false),
        ("0e0 / 0e0 = 1".to_string(), false),
        ("0e0 / 0e0 != 1".to_string(), true),
        ("0e0 / 0e0 = 0e0 / 0e0".to_string(), false),
        (format!("{nanf} + 1 < 1"), false),
        (format!("{nanf} + 1 >= 1"), false),
        (format!("{nanf} + 1 = {nanf} + 1"), false),
        (format!("{nanf} + 1 != 1"), true),
    ] {
        assert_eq!(bound_bool(&g, &expr), Some(want), "BIND {expr}");
        assert_eq!(ask(&g, &format!("ASK {{ FILTER({expr}) }}")), want, "FILTER {expr}");
        assert_eq!(ask(&g, &format!("ASK {{ FILTER(!({expr})) }}")), !want, "FILTER !({expr})");
    }
}

/// Stored operands: the FILTER runs over pattern rows, so it is compiled.
#[test]
fn stored_nan_arithmetic_comparisons_are_false_not_errors() {
    let nt = format!("<http://ex/a> <http://ex/n> \"NaN\"^^<{XSD}float> .\n");
    let g = Graph::load_str(&nt, "turtle").unwrap();
    for (filter, want) in [("?v + 1 < 1", false), ("?v / 1 >= 1", false), ("?v + 1 = 1", false), ("?v + 1 != 1", true)] {
        let q = format!("ASK {{ ?s <http://ex/n> ?v FILTER(!({filter})) }}");
        assert_eq!(ask(&g, &q), !want, "{q}");
        let q = format!("SELECT ?b WHERE {{ ?s <http://ex/n> ?v BIND(({filter}) AS ?b) }}");
        let r = query(&g, &q).unwrap();
        let got = r.rows[0][0].as_ref().map(|t| t.to_string().starts_with("\"true\""));
        assert_eq!(got, Some(want), "{q}");
    }
}

#[test]
fn incompatible_operands_of_an_ordering_stay_errors() {
    let g = promo_graph();
    // A numeric against a string is a type error: unbound in BIND, excluded even when negated.
    assert_eq!(bound_bool(&g, "1 + 1 < \"a\""), None);
    assert!(!ask(&g, "ASK { FILTER(!(1 + 1 < \"a\")) }"));
    assert_eq!(bound_bool(&g, "0e0 / 0e0 < \"a\""), None);
}

// ── MIN/MAX keep the ORDER BY order, not the float-tier tie ─────────────────────────────
// MAX is defined through ORDER BY DESC. Promoting `0.1` to float makes it tie with the float
// `0.1` (0.10000000149…), but that tie must not let the later double 0.1000000001, which is
// strictly smaller than the float, displace it.

#[test]
fn max_keeps_the_float_over_a_smaller_later_double() {
    let g = promo_graph();
    let q = |agg: &str, vals: &str| {
        let r = query(&g, &format!("SELECT ({agg}(?v) AS ?m) WHERE {{ VALUES ?v {{ {vals} }} }}")).unwrap();
        r.rows[0][0].as_ref().map(|t| t.to_string()).unwrap_or_default()
    };
    let f = format!("\"0.1\"^^<{XSD}float>");
    let m = q("MAX", &format!("0.1 {f} 0.1000000001e0"));
    assert!(m.contains("float"), "MAX must be the float, got {m}");
    let m = q("MIN", &format!("0.1 {f} 0.0999999999e0"));
    assert!(m.contains("double"), "MIN must be the smaller double, got {m}");
}
