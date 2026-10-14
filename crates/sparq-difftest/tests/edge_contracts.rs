//! Integration pins for the differential comparator's boundary contracts (sq-bif.25).

use sparq_difftest::{
    canonical_double_string, multiset_equal, numeric_equal, parse_results_json, NumericValue,
    QueryResults, Solution,
};

#[test]
fn results_json_errors_and_boolean_contract() {
    let error = parse_results_json("[]").expect_err("an array root must be rejected");
    assert!(error.to_string().contains("root is not an object"));

    let error = parse_results_json("{}").expect_err("a SELECT result must declare head.vars");
    assert!(error.to_string().contains("missing head.vars"));

    assert!(matches!(
        parse_results_json(r#"{"boolean":true}"#),
        Ok(QueryResults::Boolean(true))
    ));

    let error = parse_results_json(r#"{"boolean":"yes"}"#)
        .expect_err("a non-boolean ASK result must be rejected");
    assert!(error.to_string().contains("boolean"));
}

#[test]
fn empty_select_has_no_variables_or_solutions() {
    let parsed = parse_results_json(r#"{"head":{"vars":[]},"results":{"bindings":[]}}"#)
        .expect("an empty SELECT result is valid");

    match parsed {
        QueryResults::Solutions { vars, solutions } => {
            assert!(vars.is_empty());
            assert!(solutions.is_empty());
        }
        QueryResults::Boolean(value) => panic!("expected SELECT solutions, got boolean {value}"),
    }
}

#[test]
fn multiset_empty_and_length_mismatch_contract() {
    assert!(multiset_equal(&[], &[]));
    assert!(!multiset_equal(&[], &[Solution::new()]));
}

#[test]
fn numeric_special_values_signed_zero_and_promotion_contract() {
    assert_eq!(canonical_double_string(f64::NAN), "NaN");
    assert_eq!(canonical_double_string(f64::INFINITY), "INF");
    assert_eq!(canonical_double_string(f64::NEG_INFINITY), "-INF");
    assert_eq!(canonical_double_string(0.0), "0.0");
    assert_eq!(canonical_double_string(-0.0), "0.0");

    let nan = NumericValue::Double(f64::NAN);
    assert!(!numeric_equal(&nan, &nan));
    assert!(numeric_equal(
        &NumericValue::Integer(2.into()),
        &NumericValue::Double(2.0),
    ));
}
