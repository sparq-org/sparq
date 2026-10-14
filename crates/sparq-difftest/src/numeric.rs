//! Exact numeric value equality, independent of sparq's numeric tower. sq-qcnn.4
//!
//! Integers are compared as arbitrary-precision [`num_bigint::BigInt`] (so distinct integers
//! **beyond `i128`** — where sparq's own exact tier and any `f64` collapse them — stay distinct);
//! decimals as arbitrary-precision [`bigdecimal::BigDecimal`] (so high-precision decimals that
//! share an `f64` stay distinct); `xsd:double`/`xsd:float` as `f64`, with the XSD special values
//! `INF` / `-INF` / `NaN` handled explicitly.
//!
//! Two notions of double equality are exposed on purpose (they differ, and the design calls it out):
//!
//! * [`numeric_equal`] uses XSD `=` semantics for the **value/decision** regime — `NaN` is unequal
//!   to everything (including itself), `+0.0 == -0.0`. This is what a `FILTER`/relational decision
//!   would use.
//! * [`canonical_double_string`] maps `NaN` to a single `"NaN"` token so a multiset key buckets
//!   `NaN` **self-equally** — otherwise two results that each legitimately contain a `NaN` row would
//!   never match. `+0.0` and `-0.0` fold to one token.

use std::cmp::Ordering;
use std::str::FromStr;

use bigdecimal::num_traits::ToPrimitive;
use bigdecimal::BigDecimal;
use num_bigint::BigInt;

use crate::term::XSD;

/// A numeric value parsed from a `(lexical, datatype)` pair, in an engine-independent exact form.
#[derive(Debug, Clone)]
pub enum NumericValue {
    /// `xsd:integer` and its derived types (`long`/`int`/`short`/`byte`, the `nonNegative…` /
    /// `unsigned…` family): compared as an arbitrary-precision integer.
    Integer(BigInt),
    /// `xsd:decimal`: compared as an arbitrary-precision decimal (value-equal, scale-insensitive).
    Decimal(BigDecimal),
    /// `xsd:double` / `xsd:float`: compared as `f64`, incl. `INF`/`-INF`/`NaN`.
    Double(f64),
}

/// The XSD value-space class a datatype IRI maps to.
enum Kind {
    Integer,
    Decimal,
    Double,
}

/// Classify an XSD numeric datatype IRI by value space; `None` for a non-numeric datatype.
fn classify(datatype: &str) -> Option<Kind> {
    let local = datatype.strip_prefix(XSD)?;
    Some(match local {
        "integer" | "long" | "int" | "short" | "byte" | "nonNegativeInteger"
        | "positiveInteger" | "nonPositiveInteger" | "negativeInteger" | "unsignedLong"
        | "unsignedInt" | "unsignedShort" | "unsignedByte" => Kind::Integer,
        "decimal" => Kind::Decimal,
        "double" | "float" => Kind::Double,
        _ => return None,
    })
}

/// Parse a `(lexical, datatype-IRI)` pair into an exact [`NumericValue`], or `None` if the datatype
/// is not numeric or the lexical is not a valid value of it.
///
/// `datatype` is the full IRI (e.g. `http://www.w3.org/2001/XMLSchema#integer`). The lexical is
/// whitespace-trimmed and a leading `+` accepted (both are permitted by the XSD lexical space);
/// `xsd:double`/`float` accept the XSD specials `INF` / `-INF` / `NaN` (exact case) but reject the
/// Rust-only spellings `inf` / `infinity` / `nan`, keeping the parse spec-faithful and independent.
pub fn parse_numeric(lexical: &str, datatype: &str) -> Option<NumericValue> {
    let kind = classify(datatype)?;
    let s = lexical.trim();
    match kind {
        Kind::Integer => {
            let s = s.strip_prefix('+').unwrap_or(s);
            BigInt::from_str(s).ok().map(NumericValue::Integer)
        }
        Kind::Decimal => {
            let s = s.strip_prefix('+').unwrap_or(s);
            // xsd:decimal has no exponent; reject one so a `double`-shaped lexical mislabelled as
            // decimal does not silently parse.
            if s.contains(['e', 'E']) {
                return None;
            }
            BigDecimal::from_str(s).ok().map(NumericValue::Decimal)
        }
        Kind::Double => parse_double(s).map(NumericValue::Double),
    }
}

/// Parse an `xsd:double`/`xsd:float` lexical, honouring the XSD special tokens and rejecting the
/// Rust-only infinity/NaN spellings.
fn parse_double(s: &str) -> Option<f64> {
    match s {
        "INF" | "+INF" => return Some(f64::INFINITY),
        "-INF" => return Some(f64::NEG_INFINITY),
        "NaN" => return Some(f64::NAN),
        _ => {}
    }
    // `f64::from_str` accepts textual `inf`/`infinity`/`nan` (any case) which XSD does not; reject
    // those textual forms. A finite lexical that *overflows* to infinity (e.g. `1e400`) is a
    // legitimate XSD double that rounds to INF, so it is NOT rejected here.
    let lower = s.to_ascii_lowercase();
    if matches!(
        lower.as_str(),
        "inf" | "+inf" | "-inf" | "infinity" | "+infinity" | "-infinity" | "nan"
    ) {
        return None;
    }
    f64::from_str(s).ok()
}

/// XSD numeric `=` (the value/decision regime): cross-type comparison promotes to the wider value
/// space, and a `double`/`float` operand makes the comparison IEEE (`NaN` unequal to everything,
/// `+0.0 == -0.0`). Precision loss on promoting an exact int/decimal to `f64` is per-spec.
pub fn numeric_equal(a: &NumericValue, b: &NumericValue) -> bool {
    use NumericValue::{Decimal, Double, Integer};
    match (a, b) {
        (Double(x), _) => to_f64(b).is_some_and(|y| *x == y),
        (_, Double(y)) => to_f64(a).is_some_and(|x| x == *y),
        (Integer(x), Integer(y)) => x == y,
        (Decimal(x), Decimal(y)) => x.cmp(y) == Ordering::Equal,
        // num-bigint 0.5: bigdecimal 0.4 no longer implements From<BigInt 0.5>
        // (two separate semver-major instances). Convert via the decimal string form, which is
        // lossless for integers and always parses successfully.
        (Integer(x), Decimal(y)) => BigDecimal::from_str(&x.to_string())
            .expect("BigInt::to_string yields a valid decimal literal")
            .cmp(y)
            == Ordering::Equal,
        (Decimal(x), Integer(y)) => x
            .cmp(
                &BigDecimal::from_str(&y.to_string())
                    .expect("BigInt::to_string yields a valid decimal literal"),
            )
            == Ordering::Equal,
    }
}

/// Promote a numeric value to `f64` (per XSD, for a comparison against a `double`/`float`).
fn to_f64(n: &NumericValue) -> Option<f64> {
    match n {
        NumericValue::Double(x) => Some(*x),
        NumericValue::Integer(x) => x.to_f64(),
        NumericValue::Decimal(x) => x.to_f64(),
    }
}

/// A stable, value-canonical string for an `xsd:double`/`xsd:float` — the multiset-keying form.
///
/// `NaN` folds to `"NaN"` (so it buckets self-equally, unlike XSD `=`), `±∞` to `INF`/`-INF`, and
/// `+0.0`/`-0.0` to one `"0.0"` token; every other value uses Rust's shortest round-trippable
/// representation, which is deterministic and distinct for distinct `f64` values.
pub fn canonical_double_string(x: f64) -> String {
    if x.is_nan() {
        "NaN".to_string()
    } else if x.is_infinite() {
        if x > 0.0 {
            "INF".to_string()
        } else {
            "-INF".to_string()
        }
    } else if x == 0.0 {
        "0.0".to_string()
    } else {
        format!("{x:?}")
    }
}

/// A stable, value-canonical string for a numeric value, keyed **within** its own value class
/// (so cross-engine canonical-*lexical* variance — `05` vs `5`, `1.50` vs `1.5`, `6` vs `6.0E0` —
/// collapses, while a genuinely different value does not). Used by `crate::term::canonical_key`.
pub(crate) fn canonical_numeric_string(n: &NumericValue) -> String {
    match n {
        NumericValue::Integer(x) => x.to_string(),
        NumericValue::Decimal(x) => x.normalized().to_string(),
        NumericValue::Double(x) => canonical_double_string(*x),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const XSD_INT: &str = "http://www.w3.org/2001/XMLSchema#integer";
    const XSD_DEC: &str = "http://www.w3.org/2001/XMLSchema#decimal";
    const XSD_DBL: &str = "http://www.w3.org/2001/XMLSchema#double";
    const XSD_LONG: &str = "http://www.w3.org/2001/XMLSchema#long";

    #[test]
    fn parse_numeric_integer_beyond_i128() {
        // 2^130 + 1 — far beyond i128::MAX (~1.7e38 = 2^127). Distinct from its neighbour.
        let big = "1361129467683753853853498429727072845825";
        let a = parse_numeric(big, XSD_INT).unwrap();
        let b = parse_numeric("1361129467683753853853498429727072845826", XSD_INT).unwrap();
        assert!(numeric_equal(&a, &parse_numeric(big, XSD_INT).unwrap()));
        assert!(!numeric_equal(&a, &b), "consecutive huge integers must stay distinct");
        // leading zeros / plus sign normalise in value space.
        assert!(numeric_equal(
            &parse_numeric("+007", XSD_INT).unwrap(),
            &parse_numeric("7", XSD_INT).unwrap()
        ));
        assert!(parse_numeric("1.5", XSD_INT).is_none());
        assert!(parse_numeric("hello", "http://example.org/notNumeric").is_none());
    }

    #[test]
    fn numeric_equal_covers_types_and_promotion() {
        // high-precision decimals sharing an f64 stay distinct.
        let d1 = parse_numeric("0.123456789012345678", XSD_DEC).unwrap();
        let d2 = parse_numeric("0.123456789012345679", XSD_DEC).unwrap();
        assert!(!numeric_equal(&d1, &d2));
        // scale-insensitive decimal value equality.
        assert!(numeric_equal(
            &parse_numeric("1.50", XSD_DEC).unwrap(),
            &parse_numeric("1.5", XSD_DEC).unwrap()
        ));
        // cross-type integer/decimal.
        assert!(numeric_equal(
            &parse_numeric("2", XSD_INT).unwrap(),
            &parse_numeric("2.0", XSD_DEC).unwrap()
        ));
        // a double operand promotes both sides.
        assert!(numeric_equal(
            &parse_numeric("2", XSD_LONG).unwrap(),
            &parse_numeric("2.0", XSD_DBL).unwrap()
        ));
        // NaN is unequal to everything under XSD `=` (value regime).
        let nan = parse_numeric("NaN", XSD_DBL).unwrap();
        assert!(!numeric_equal(&nan, &nan));
        // +0.0 == -0.0 under IEEE.
        assert!(numeric_equal(
            &parse_numeric("0.0", XSD_DBL).unwrap(),
            &parse_numeric("-0.0", XSD_DBL).unwrap()
        ));
    }

    #[test]
    fn parse_double_specials_and_spellings() {
        assert!(parse_numeric("INF", XSD_DBL).is_some());
        assert!(parse_numeric("-INF", XSD_DBL).is_some());
        assert!(matches!(
            parse_numeric("NaN", XSD_DBL),
            Some(NumericValue::Double(x)) if x.is_nan()
        ));
        // Rust-only spellings are rejected as non-XSD.
        assert!(parse_numeric("inf", XSD_DBL).is_none());
        assert!(parse_numeric("Infinity", XSD_DBL).is_none());
        assert!(parse_numeric("nan", XSD_DBL).is_none());
        // overflow rounds to INF (legitimate XSD).
        assert!(matches!(
            parse_numeric("1e400", XSD_DBL),
            Some(NumericValue::Double(x)) if x.is_infinite() && x > 0.0
        ));
    }

    #[test]
    fn canonical_double_string_buckets_specials() {
        assert_eq!(canonical_double_string(f64::NAN), "NaN");
        assert_eq!(canonical_double_string(f64::INFINITY), "INF");
        assert_eq!(canonical_double_string(f64::NEG_INFINITY), "-INF");
        // +0.0 and -0.0 fold to one token (unlike the raw Debug of -0.0).
        assert_eq!(canonical_double_string(0.0), canonical_double_string(-0.0));
        // distinct finite values -> distinct tokens.
        assert_ne!(canonical_double_string(6.0), canonical_double_string(6.5));
    }

    #[test]
    fn canonical_numeric_string_collapses_lexical_variance() {
        let i = parse_numeric("007", XSD_INT).unwrap();
        assert_eq!(canonical_numeric_string(&i), "7");
        let d = parse_numeric("1.50", XSD_DEC).unwrap();
        assert_eq!(canonical_numeric_string(&d), "1.5");
        let dbl = parse_numeric("6", XSD_DBL).unwrap();
        assert_eq!(canonical_numeric_string(&dbl), canonical_double_string(6.0));
    }
}
