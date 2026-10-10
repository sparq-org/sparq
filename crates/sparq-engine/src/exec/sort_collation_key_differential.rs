use super::*;
use oxrdf::{vocab::xsd, BlankNode, Literal, NamedNode, NamedNodeRef};

/// A `Value::Term(Literal)` of the given lexical + datatype IRI.
fn typed(lex: &str, dt: NamedNodeRef<'_>) -> Value {
    Value::Term(Term::Literal(Literal::new_typed_literal(lex, dt)))
}
fn lang(lex: &str, tag: &str) -> Value {
    Value::Term(Term::Literal(Literal::new_language_tagged_literal(lex, tag).unwrap()))
}
fn iri(s: &str) -> Value {
    Value::Term(Term::NamedNode(NamedNode::new(s).unwrap()))
}
fn bnode(s: &str) -> Value {
    Value::Term(Term::BlankNode(BlankNode::new(s).unwrap()))
}

/// A curated corpus spanning EVERY total-order class and literal kind, plus the exact
/// adversarial pairs the sq-lr2ii / sq-rikm7 / sq-wjl8i traps care about.
fn corpus() -> Vec<Value> {
    let mut v = vec![
        Value::Unbound,
        Value::Error,
        bnode("b0"),
        bnode("b1"),
        iri("http://ex/a"),
        iri("http://ex/b"),
        // Numeric kind — computed and lexical, across the tower.
        Value::Num(Num::Double(1.0)),
        Value::Num(Num::Double(f64::NAN)),
        Value::Num(Num::Double(f64::INFINITY)),
        Value::Num(Num::Double(f64::NEG_INFINITY)),
        typed("0", xsd::INTEGER),
        typed("1", xsd::INTEGER),
        typed("-5", xsd::INTEGER),
        typed("1.5", xsd::DECIMAL),
        typed("1.50", xsd::DECIMAL),
        typed("NaN", xsd::DOUBLE),
        typed("INF", xsd::DOUBLE),
        typed("-INF", xsd::DOUBLE),
        typed("1.0E0", xsd::DOUBLE),
        // The sq-lr2ii f64-COLLISION trap: distinct exact values that share one f64.
        typed("9007199254740992", xsd::INTEGER), // 2^53
        typed("9007199254740993", xsd::INTEGER), // 2^53+1 — collapses onto 2^53 as f64
        typed("9007199254740993", xsd::DECIMAL),
        // High-precision decimals differing past f64 mantissa resolution.
        typed("0.100000000000000005", xsd::DECIMAL),
        typed("0.100000000000000006", xsd::DECIMAL),
        // Ill-formed numeric lexical → LiteralKind::Other, orders lexically.
        typed("notanumber", xsd::INTEGER),
        // Boolean kind.
        Value::Bool(false),
        Value::Bool(true),
        typed("true", xsd::BOOLEAN),
        typed("0", xsd::BOOLEAN),
        // Temporal kinds.
        typed("2020-01-01T00:00:00Z", xsd::DATE_TIME),
        typed("2020-01-02T00:00:00Z", xsd::DATE_TIME),
        typed("2020-01-01", xsd::DATE),
        typed("2020-06-15", xsd::DATE),
        typed("garbage", xsd::DATE_TIME), // ill-formed temporal → Other
        // String kind.
        typed("apple", xsd::STRING),
        typed("banana", xsd::STRING),
        typed("10", xsd::STRING), // lexical "10" < "2" (the sq-wjl8i witness domain)
        typed("2", xsd::STRING),
        // Language-tagged.
        lang("hello", "en"),
        lang("hello", "fr"),
        lang("world", "en"),
        // Other XSD + unknown datatype.
        typed("PT1H", xsd::DURATION),
        typed("PT2H", xsd::DURATION),
        Value::Term(Term::Literal(Literal::new_typed_literal(
            "x",
            NamedNode::new("http://ex/customdt").unwrap(),
        ))),
    ];
    // A few RDF-1.2 triple terms (sort after literals, component-wise).
    v.push(Value::Term(Term::Triple(Box::new(oxrdf::Triple::new(
        NamedNode::new("http://ex/s").unwrap(),
        NamedNode::new("http://ex/p").unwrap(),
        Literal::new_typed_literal("1", xsd::INTEGER),
    )))));
    v.push(Value::Term(Term::Triple(Box::new(oxrdf::Triple::new(
        NamedNode::new("http://ex/s").unwrap(),
        NamedNode::new("http://ex/p").unwrap(),
        Literal::new_typed_literal("2", xsd::INTEGER),
    )))));
    v
}

/// The reference ordering: the pre-existing comparator over raw `Value`s.
fn reference(a: &Value, c: &Value) -> Ordering {
    compare_values(a, c).unwrap_or(Ordering::Equal)
}

/// The precomputed-key ordering: build `SortCell::Val` cells (ranks precomputed) and
/// compare via `cmp_sort_cells`' `(Val, Val)` arm. The graph / local are never consulted
/// by that arm, so an empty graph is sufficient.
fn keyed(a: &Value, c: &Value, g: &Graph, l: &LocalVocab) -> Ordering {
    let ca = sort_cell_val(a.clone());
    let cc = sort_cell_val(c.clone());
    cmp_sort_cells(g, l, &ca, &cc)
}

#[test]
fn precomputed_key_order_equals_comparator_on_every_pair() {
    let g = Graph::load_str("", "turtle").unwrap();
    let l = LocalVocab::default();
    let vs = corpus();
    for a in &vs {
        for c in &vs {
            let want = reference(a, c);
            let got = keyed(a, c, &g, &l);
            assert_eq!(
                got, want,
                "precomputed-key order diverged from comparator for ({}, {}): got {:?} want {:?}",
                value_key(a),
                value_key(c),
                got,
                want
            );
        }
    }
}

#[test]
fn precomputed_key_reproduces_antisymmetry_and_reflexivity() {
    let g = Graph::load_str("", "turtle").unwrap();
    let l = LocalVocab::default();
    let vs = corpus();
    for a in &vs {
        assert_eq!(keyed(a, a, &g, &l), Ordering::Equal, "reflexivity: {}", value_key(a));
        for c in &vs {
            let ac = keyed(a, c, &g, &l);
            let ca = keyed(c, a, &g, &l);
            assert_eq!(ac, ca.reverse(), "antisymmetry ({}, {})", value_key(a), value_key(c));
        }
    }
}

/// A deterministic pseudo-random fuzz over randomly-generated numeric lexicals — the
/// class the sq-lr2ii trap lives in — asserting the precomputed key never disagrees with
/// the comparator on any generated pair (including f64-colliding and NaN/INF spellings).
#[test]
fn precomputed_key_matches_comparator_on_random_numeric_lexicals() {
    let g = Graph::load_str("", "turtle").unwrap();
    let l = LocalVocab::default();
    // A tiny LCG — no external rand dep, fully deterministic.
    let mut state: u64 = 0x9E3779B97F4A7C15;
    let mut next = || {
        state = state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        state >> 11
    };
    let dts = [xsd::INTEGER, xsd::DECIMAL, xsd::DOUBLE, xsd::FLOAT];
    let specials = ["NaN", "INF", "-INF", "0", "-0", "0.0"];
    let mk = |n: &mut dyn FnMut() -> u64| -> Value {
        let choice = n() % 10;
        let dt = dts[(n() % dts.len() as u64) as usize];
        if choice == 0 {
            // A special spelling (some ill-formed for INTEGER → Other kind).
            return typed(specials[(n() % specials.len() as u64) as usize], dt);
        }
        // Values near the 2^53 f64-collapse boundary + high-precision decimals.
        let base = 9007199254740990u64 + (n() % 8);
        let frac = n() % 1_000_000_000;
        let lex = if choice.is_multiple_of(2) {
            format!("{}", base)
        } else {
            format!("{}.{:018}", base, frac)
        };
        typed(&lex, dt)
    };
    for _ in 0..4000 {
        let a = mk(&mut next);
        let c = mk(&mut next);
        let want = reference(&a, &c);
        let got = keyed(&a, &c, &g, &l);
        assert_eq!(
            got, want,
            "random numeric divergence ({}, {}): got {:?} want {:?}",
            value_key(&a),
            value_key(&c),
            got,
            want
        );
    }
}
