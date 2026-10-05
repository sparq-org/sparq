use super::*;

#[test]
fn minmax_empty_is_unbound() {
    // Value does not impl PartialEq; use matches! for the pattern check.
    assert!(
        matches!(minmax_values(vec![], std::cmp::Ordering::Less), Value::Unbound),
        "MIN(empty set) must be Unbound"
    );
    assert!(
        matches!(minmax_values(vec![], std::cmp::Ordering::Greater), Value::Unbound),
        "MAX(empty set) must be Unbound"
    );
}

#[test]
fn min_over_integers_is_smallest() {
    let vals = vec![Value::Num(Num::Int(5)), Value::Num(Num::Int(2)), Value::Num(Num::Int(8))];
    let min = minmax_values(vals, std::cmp::Ordering::Less);
    // MIN over an all-integer set is the smallest member (2). `minmax_values` returns
    // it via `num_canonical_term`, i.e. a `Value::Term` carrying the canonical
    // xsd:integer literal "2". Assert the EXACT lexical value AND datatype — NOT a
    // substring `contains('2')`, which is vacuous because the xsd:integer datatype
    // IRI (…/2001/XMLSchema#integer) already contains '2' regardless of the value.
    match min {
        Value::Num(Num::Int(n)) => assert_eq!(n, 2, "MIN of [5,2,8] must be exactly 2, got {}", n),
        Value::Term(Term::Literal(ref l)) => {
            assert_eq!(l.value(), "2", "MIN of [5,2,8] must have lexical value \"2\", got {}", l);
            assert_eq!(
                l.datatype(),
                xsd::INTEGER,
                "MIN of integers must keep xsd:integer, got {}",
                l.datatype()
            );
        }
        other => panic!("MIN of integers must be the integer 2 (numeric or integer term), got {other:?}"),
    }
}

#[test]
fn max_over_integers_is_largest() {
    let vals = vec![Value::Num(Num::Int(5)), Value::Num(Num::Int(2)), Value::Num(Num::Int(8))];
    let max = minmax_values(vals, std::cmp::Ordering::Greater);
    // MAX over an all-integer set is the largest member (8), returned as a canonical
    // xsd:integer `Value::Term`. Assert the EXACT lexical value AND datatype — a
    // substring `contains('8')` would also pass for wrong values like 18 or 80, so it
    // is not an acceptable check for the `Value::Term` representation.
    match max {
        Value::Num(Num::Int(n)) => assert_eq!(n, 8, "MAX of [5,2,8] must be exactly 8, got {}", n),
        Value::Term(Term::Literal(ref l)) => {
            assert_eq!(l.value(), "8", "MAX of [5,2,8] must have lexical value \"8\", got {}", l);
            assert_eq!(
                l.datatype(),
                xsd::INTEGER,
                "MAX of integers must keep xsd:integer, got {}",
                l.datatype()
            );
        }
        other => panic!("MAX of integers must be the integer 8 (numeric or integer term), got {other:?}"),
    }
}

#[test]
fn min_over_mixed_types_falls_back_to_compare_values() {
    // A mix of IRI and literal forces the "not all numeric" path → compare_values fallback.
    use oxrdf::NamedNode;
    let iri = Value::Term(Term::NamedNode(NamedNode::new_unchecked("http://ex/x")));
    let lit = Value::Term(Term::Literal(Literal::new_simple_literal("abc")));
    let vals = vec![iri, lit];
    // Should not panic; result is the "smaller" by compare_values total order.
    let min = minmax_values(vals, std::cmp::Ordering::Less);
    // IRI < plain literal in SPARQL total order, so MIN should be the IRI.
    match &min {
        Value::Term(Term::NamedNode(_)) => {} // expected: IRI
        other => panic!("MIN of (IRI, string) should be the IRI, got {other:?}"),
    }
}
