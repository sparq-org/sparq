use super::*;

#[test]
fn dec_arithmetic_is_exact() {
    use std::cmp::Ordering::*;
    let d = |s: &str| Dec::parse(s).unwrap();
    // The classic: 0.1 + 0.2 == 0.3 exactly (false in f64).
    assert_eq!(d("0.1").checked_add(d("0.2")).unwrap().cmp(d("0.3")), Some(Equal));
    // 0.3 - 0.1 == 0.2 exactly.
    assert_eq!(d("0.3").checked_sub(d("0.1")).unwrap().cmp(d("0.2")), Some(Equal));
    // Mixed scales + integer/decimal.
    assert_eq!(d("1").checked_add(d("0.5")).unwrap().cmp(d("1.5")), Some(Equal));
    assert_eq!(d("83").checked_mul(d("0.5")).unwrap().cmp(d("41.5")), Some(Equal));
    assert_eq!(d("0.1").checked_mul(d("0.1")).unwrap().cmp(d("0.01")), Some(Equal));
    // Ordering across scales and signs.
    assert_eq!(d("0.30000000000000001").cmp(d("0.3")), Some(Greater));
    assert_eq!(d("-2.5").checked_add(d("1")).unwrap().cmp(d("-1.5")), Some(Equal));
    // Large integers beyond 2^53 stay exact.
    assert_eq!(d("9007199254740992").checked_add(d("1")).unwrap().cmp(d("9007199254740993")), Some(Equal));
    // Non-decimal lexical (exponent) -> not parseable here -> None.
    assert!(Dec::parse("1e5").is_none());
}

#[test]
fn cmp_decimal_str_is_exact() {
    use std::cmp::Ordering::*;
    // Integers beyond f64 precision, and high-precision decimals that share an f64.
    assert_eq!(cmp_decimal_str("9007199254740992", "9007199254740993"), Some(Less));
    assert_eq!(cmp_decimal_str("0.123456789012345678", "0.123456789012345679"), Some(Less));
    // Equality incl. non-canonical zeros / trailing zeros / signed zero.
    assert_eq!(cmp_decimal_str("1.50", "1.5"), Some(Equal));
    assert_eq!(cmp_decimal_str("007", "7"), Some(Equal));
    assert_eq!(cmp_decimal_str("-0", "0"), Some(Equal));
    assert_eq!(cmp_decimal_str("0.0", "-0.0"), Some(Equal));
    // Sign + magnitude.
    assert_eq!(cmp_decimal_str("-3", "2"), Some(Less));
    assert_eq!(cmp_decimal_str("-2", "-3"), Some(Greater));
    assert_eq!(cmp_decimal_str("10", "9"), Some(Greater));
    assert_eq!(cmp_decimal_str("1.1", "1.09"), Some(Greater));
    assert_eq!(cmp_decimal_str("0.1", "0.2"), Some(Less));
    // Integer vs decimal of equal value.
    assert_eq!(cmp_decimal_str("5", "5.0"), Some(Equal));
    assert_eq!(cmp_decimal_str("5", "5.0000000000000001"), Some(Less));
    // Malformed -> None (falls back to f64).
    assert_eq!(cmp_decimal_str("1.2.3", "1"), None);
    assert_eq!(cmp_decimal_str("1e5", "1"), None);
}

#[test]
fn sig_digits_counts() {
    assert_eq!(sig_digits("120"), 3);
    assert_eq!(sig_digits("0.5"), 1);
    assert_eq!(sig_digits("9007199254740992"), 16);
    assert_eq!(sig_digits("0.123456789012345679"), 18);
    assert_eq!(sig_digits("0.00123"), 3); // leading fraction zeros are not significant
    assert_eq!(sig_digits("100.0"), 3);
}
