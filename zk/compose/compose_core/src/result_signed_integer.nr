//! [GPT-6] Canonical signed-i64 literal binding for the separate signed result profile.
//! Research-stage, not externally audited. Sign and actual lexical length are
//! private within one fixed capacity. This module alone is not a result-proof API.

use crate::filter_int::OP_NE;
use crate::filter_signed::{literal_encoding, signed_verdict};
use crate::result::ResultInteger;

global SIGN_BIT: u64 = 9223372036854775808;

/// A signed value encoded monotonically as mathematical `value + 2^63`.
/// Every u64 maps to exactly one i64; no signed negation or wrapping addition is used.
pub struct SignedInteger {
    biased: u64,
}

impl ResultInteger for SignedInteger {
    fn check(self, operand: Field, op: u32, bound: u64, bits: u32) {
        assert(bits == 64, "signed integer requires full capacity");
        full_i64_check(operand, op, bound, self.biased);
    }
}

fn check_lexical<let LEN: u32>(operand: Field, negative: bool, digits: [u8; 20]) {
    // One token hash per total length, not separate public sign/length buckets.
    let mut lexical = [0; LEN];
    for i in 0..LEN { lexical[i] = digits[20 - LEN + i]; }
    if negative { lexical[0] = 45; }
    let suffix = "^^<http://www.w3.org/2001/XMLSchema#integer>".as_bytes();
    assert_eq(literal_encoding(lexical, suffix), operand, "signed operand encoding mismatch");
}

/// Opens the exact canonical xsd:integer token and checks a biased signed bound.
/// The caller derives that bound from its separately admitted signed query.
pub fn full_i64_check(operand: Field, op: u32, bound: u64, biased: u64) {
    assert(op <= OP_NE, "unknown comparison operator");
    let negative = biased < SIGN_BIT;
    // Both active subtractions are in range; MIN has magnitude 2^63, not a
    // signed absolute value. Noir constrains the chosen branch's arithmetic.
    let magnitude = if negative { SIGN_BIT - biased } else { biased - SIGN_BIT };
    let mut digits: [u8; 20] = [48; 20];
    let mut remaining = magnitude;
    let mut digit_count: u32 = 1;
    for i in 0..19 {
        digits[19 - i] = 48 + (remaining % 10) as u8;
        if remaining >= 10 { digit_count = digit_count + 1; }
        remaining = remaining / 10;
    }
    assert_eq(remaining, 0, "signed magnitude exceeds lexical capacity");
    let length = digit_count + if negative { 1 } else { 0 };
    if length == 1 {
        check_lexical::<1>(operand, negative, digits);
    } else if length == 2 {
        check_lexical::<2>(operand, negative, digits);
    } else if length == 3 {
        check_lexical::<3>(operand, negative, digits);
    } else if length == 4 {
        check_lexical::<4>(operand, negative, digits);
    } else if length == 5 {
        check_lexical::<5>(operand, negative, digits);
    } else if length == 6 {
        check_lexical::<6>(operand, negative, digits);
    } else if length == 7 {
        check_lexical::<7>(operand, negative, digits);
    } else if length == 8 {
        check_lexical::<8>(operand, negative, digits);
    } else if length == 9 {
        check_lexical::<9>(operand, negative, digits);
    } else if length == 10 {
        check_lexical::<10>(operand, negative, digits);
    } else if length == 11 {
        check_lexical::<11>(operand, negative, digits);
    } else if length == 12 {
        check_lexical::<12>(operand, negative, digits);
    } else if length == 13 {
        check_lexical::<13>(operand, negative, digits);
    } else if length == 14 {
        check_lexical::<14>(operand, negative, digits);
    } else if length == 15 {
        check_lexical::<15>(operand, negative, digits);
    } else if length == 16 {
        check_lexical::<16>(operand, negative, digits);
    } else if length == 17 {
        check_lexical::<17>(operand, negative, digits);
    } else if length == 18 {
        check_lexical::<18>(operand, negative, digits);
    } else if length == 19 {
        check_lexical::<19>(operand, negative, digits);
    } else {
        assert_eq(length, 20, "signed lexical length outside capacity");
        check_lexical::<20>(operand, negative, digits);
    }
    // False signs reduce the existing shared comparator to unsigned order,
    // which is signed order after the monotonic bias. No new comparison table.
    assert(signed_verdict(false, biased, false, bound, op), "signed filter verdict mismatch");
}

// Expectations independently build tokens from supplied lexical bytes; the
// production relation derives those bytes from the constrained biased value.
fn encoded_token<let D: u32, let S: u32>(lexical: [u8; D], suffix: [u8; S]) -> Field {
    let mut token: [u8; D + S + 2] = [0; D + S + 2];
    token[0] = 34;
    for i in 0..D { token[i + 1] = lexical[i]; }
    token[D + 1] = 34;
    for i in 0..S { token[D + 2 + i] = suffix[i]; }
    let digest = std::hash::blake3(token);
    let mut hs: Field = 0;
    for i in 1..32 { hs = hs * 256 + digest[i] as Field; }
    crate::hashes::h2(crate::hashes::TYPE_CODE_LITERAL, hs)
}

fn integer_token<let D: u32>(lexical: [u8; D]) -> Field {
    encoded_token(lexical, "^^<http://www.w3.org/2001/XMLSchema#integer>".as_bytes())
}

#[test]
fn signed_boundaries_and_cross_zero_order_bind_exact_tokens() {
    let operands = [
        integer_token("-9223372036854775808".as_bytes()),
        integer_token("-1".as_bytes()),
        integer_token("0".as_bytes()),
        integer_token("1".as_bytes()),
        integer_token("9223372036854775807".as_bytes()),
    ];
    let biased = [0, SIGN_BIT - 1, SIGN_BIT, SIGN_BIT + 1, 18446744073709551615];
    for i in 0..5 {
        for j in 0..5 {
            // Independent ordering oracle: positions are ascending signed values.
            let op = if i < j { 0 } else if i == j { 4 } else { 2 };
            full_i64_check(operands[i], op, biased[j], biased[i]);
        }
    }
    full_i64_check(operands[0], 1, biased[0], biased[0]);
    full_i64_check(operands[4], 3, biased[4], biased[4]);
    full_i64_check(operands[1], 5, biased[3], biased[1]);
}

#[test(should_fail_with = "signed operand encoding mismatch")]
fn signed_negative_zero_rejects() {
    full_i64_check(integer_token("-0".as_bytes()), 4, SIGN_BIT, SIGN_BIT);
}

#[test(should_fail_with = "signed operand encoding mismatch")]
fn signed_leading_plus_rejects() {
    full_i64_check(integer_token("+1".as_bytes()), 4, SIGN_BIT + 1, SIGN_BIT + 1);
}

#[test(should_fail_with = "signed operand encoding mismatch")]
fn signed_positive_leading_zero_rejects() {
    full_i64_check(integer_token("01".as_bytes()), 4, SIGN_BIT + 1, SIGN_BIT + 1);
}

#[test(should_fail_with = "signed operand encoding mismatch")]
fn signed_negative_leading_zero_rejects() {
    full_i64_check(integer_token("-01".as_bytes()), 4, SIGN_BIT - 1, SIGN_BIT - 1);
}

#[test(should_fail_with = "signed operand encoding mismatch")]
fn signed_maximum_plus_one_rejects() {
    full_i64_check(integer_token("9223372036854775808".as_bytes()), 4, 18446744073709551615, 18446744073709551615);
}

#[test(should_fail_with = "signed operand encoding mismatch")]
fn signed_minimum_minus_one_rejects() {
    full_i64_check(integer_token("-9223372036854775809".as_bytes()), 4, 0, 0);
}

#[test(should_fail_with = "signed operand encoding mismatch")]
fn signed_datatype_substitution_rejects() {
    let token = encoded_token("1".as_bytes(), "^^<http://www.w3.org/2001/XMLSchema#decimal>".as_bytes());
    full_i64_check(token, 4, SIGN_BIT + 1, SIGN_BIT + 1);
}

#[test(should_fail_with = "signed operand encoding mismatch")]
fn signed_sign_substitution_rejects() {
    full_i64_check(integer_token("1".as_bytes()), 4, SIGN_BIT - 1, SIGN_BIT - 1);
}

#[test(should_fail_with = "signed operand encoding mismatch")]
fn unsigned_value_cannot_be_used_as_signed_bias() {
    full_i64_check(integer_token("42".as_bytes()), 4, 42, 42);
}

#[test(should_fail_with = "signed filter verdict mismatch")]
fn signed_negative_one_is_not_above_zero() {
    full_i64_check(integer_token("-1".as_bytes()), 2, SIGN_BIT, SIGN_BIT - 1);
}

#[test(should_fail_with = "unknown comparison operator")]
fn signed_unknown_operator_rejects() {
    full_i64_check(integer_token("0".as_bytes()), 6, SIGN_BIT, SIGN_BIT);
}
