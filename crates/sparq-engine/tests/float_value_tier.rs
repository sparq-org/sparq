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
