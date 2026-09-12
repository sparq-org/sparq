// [GPT-6] Datatype validity and bounded arithmetic capacity are separate contracts.
#![cfg(feature = "numeric")]

use oxrdf::{Literal, NamedNode};
use sparq_core::{numeric_cache_value, numeric_literal_valid};
use sparq_substrate::numeric::Num;

#[test]
fn numeric_facets_and_lexicals_agree_across_cache_and_arithmetic() {
    for (text, suffix, valid) in [
        ("127", "byte", true),
        ("128", "byte", false),
        ("-128", "byte", true),
        ("-129", "byte", false),
        ("1200", "byte", false),
        ("32767", "short", true),
        ("32768", "short", false),
        ("2147483647", "int", true),
        ("2147483648", "int", false),
        ("9223372036854775807", "long", true),
        ("9223372036854775808", "long", false),
        ("18446744073709551615", "unsignedLong", true),
        ("18446744073709551616", "unsignedLong", false),
        ("-1", "unsignedLong", false),
        ("4294967295", "unsignedInt", true),
        ("4294967296", "unsignedInt", false),
        ("65535", "unsignedShort", true),
        ("65536", "unsignedShort", false),
        // XSD 1.1 unsigned lexical forms retain + and negative zero.
        ("+1", "unsignedLong", true),
        ("-0", "unsignedLong", true),
        ("+1", "unsignedInt", true),
        ("-0", "unsignedInt", true),
        ("+1", "unsignedShort", true),
        ("-0", "unsignedShort", true),
        ("+1", "unsignedByte", true),
        ("-0", "unsignedByte", true),
        ("255", "unsignedByte", true),
        ("256", "unsignedByte", false),
        ("0", "positiveInteger", false),
        ("-1", "nonNegativeInteger", false),
        ("-0", "nonNegativeInteger", true),
        ("-0", "negativeInteger", false),
        ("0", "nonPositiveInteger", true),
        ("1", "nonPositiveInteger", false),
        ("+007", "integer", true),
        ("5.", "integer", false),
        ("5.0", "integer", false),
        ("\t5\r\n", "integer", true),
        ("\u{a0}5", "integer", false),
        ("é", "integer", false),
        ("１", "integer", false),
        ("1 0", "integer", false),
        (".5", "decimal", true),
        ("5.", "decimal", true),
        ("1E2", "decimal", false),
        (".", "decimal", false),
        ("+", "decimal", false),
        ("1.2.3", "decimal", false),
        ("5", "string", false),
    ] {
        let datatype = format!("http://www.w3.org/2001/XMLSchema#{suffix}");
        assert_eq!(
            numeric_literal_valid(text, &datatype),
            valid,
            "validity {text:?}^^{suffix}"
        );
        assert_eq!(
            numeric_cache_value(text, &datatype).is_some(),
            valid,
            "cache {text:?}^^{suffix}"
        );
        assert_eq!(
            Num::of_parts(text, &datatype).is_some(),
            valid,
            "borrowed arithmetic {text:?}^^{suffix}"
        );
        assert_eq!(
            Num::of_literal(&Literal::new_typed_literal(
                text,
                NamedNode::new(&datatype).unwrap()
            ))
            .is_some(),
            valid,
            "arithmetic {text:?}^^{suffix}"
        );
    }
}

#[test]
fn valid_large_values_remain_numeric_without_arithmetic_capacity() {
    for (text, suffix) in [
        ("9999999999999999999999999999999999999999999999", "integer"),
        (
            "9999999999999999999999999999999999999999999999",
            "positiveInteger",
        ),
        (
            "-9999999999999999999999999999999999999999999999",
            "negativeInteger",
        ),
        (
            "9999999999999999999999999999999999999999999999.1",
            "decimal",
        ),
    ] {
        let datatype = format!("http://www.w3.org/2001/XMLSchema#{suffix}");
        assert!(numeric_literal_valid(text, &datatype), "{text}^^{suffix}");
        assert!(
            numeric_cache_value(text, &datatype).is_none(),
            "{text}^^{suffix}"
        );
        assert!(
            Num::of_literal(&Literal::new_typed_literal(
                text,
                NamedNode::new(&datatype).unwrap()
            ))
            .is_none(),
            "{text}^^{suffix}"
        );
    }
}
