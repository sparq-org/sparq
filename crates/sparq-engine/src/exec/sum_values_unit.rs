use super::*;

#[test]
fn sum_empty_is_zero_integer() {
    // Sum({}) = 0^^xsd:integer (SPARQL 18.5.1.4).
    let result = sum_values(&[], false);
    // Num does not impl PartialEq; use matches! for the pattern.
    assert!(matches!(result, Some(Num::Int(0))), "sum of empty set must be 0 integer");
}

#[test]
fn sum_errored_is_none() {
    // A type error in ANY member makes the whole SUM None → unbound aggregate.
    let result = sum_values(&[Value::Num(Num::Int(5))], true);
    assert!(result.is_none(), "sum_values with errored=true must return None");
}

#[test]
fn sum_integers_stays_integer() {
    let vals = vec![Value::Num(Num::Int(3)), Value::Num(Num::Int(7))];
    let result = sum_values(&vals, false);
    assert!(matches!(result, Some(Num::Int(10))), "sum of integers must be an integer");
}

#[test]
fn sum_int_plus_decimal_promotes_to_decimal() {
    let dec = Num::Dec(Dec { mant: 5, scale: 1 }); // 0.5
    let vals = vec![Value::Num(Num::Int(2)), Value::Num(dec)];
    let result = sum_values(&vals, false);
    // 2 + 0.5 = 2.5 decimal; the canonical form depends on Num::binop normalisation.
    match result {
        Some(Num::Dec(d)) => {
            // Verify the decimal value is 2.5: mant / 10^scale == 2.5.
            // Dec fields are (mant: i128, scale: u32). Any representation where
            // mant as f64 / 10_f64.powi(scale as i32) ≈ 2.5 is acceptable.
            let v = (d.mant as f64) / 10_f64.powi(d.scale as i32);
            assert!((v - 2.5_f64).abs() < 1e-12_f64, "int+decimal must sum to 2.5, got mant={} scale={}", d.mant, d.scale);
        }
        other => panic!("int+decimal sum must be Decimal, got {other:?}"),
    }
}

#[test]
fn sum_non_numeric_member_does_not_error_via_sum_values_itself() {
    // `sum_values` receives only the VALUES passed; it doesn't evaluate expressions.
    // A non-numeric Value::Term (e.g. an IRI) causes `as_numeric` to return None,
    // making sum return None (the short-circuit path when a member has no numeric value).
    use oxrdf::NamedNode;
    let iri = Value::Term(Term::NamedNode(NamedNode::new_unchecked("http://ex/x")));
    let result = sum_values(&[Value::Num(Num::Int(5)), iri], false);
    // as_numeric(IRI) = None → sum_values returns None.
    assert!(result.is_none(), "non-numeric member (IRI) must make sum_values return None");
}
