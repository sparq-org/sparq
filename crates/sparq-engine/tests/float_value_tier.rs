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
