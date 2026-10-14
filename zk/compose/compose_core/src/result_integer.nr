// [GPT-6] Canonical full-u64 private literal binding; research-stage, not externally audited.
//! Retains the existing string commitment and hides the exact decimal length:
//! one variable-length BLAKE3 opening covers every digit count, so the circuit
//! does not select or reveal the witness's actual digit count.

use crate::blake3_literal::integer_literal_hs;
use crate::filter_int::{filter_int_check, OP_EQ, OP_GE, OP_GT, OP_LE, OP_LT, OP_NE};
use crate::hashes::{h2, TYPE_CODE_LITERAL};

/// Opens the exact canonical xsd:integer token for a private u64 value.
/// Full-width arithmetic and lexical binding are enforced together; no host-only
/// conversion or newly attested commitment substitutes for the signed RDF term.
pub fn full_u64_check(operand: Field, op: u32, bound: u64, value: u64) {
    assert(op <= OP_NE, "unknown comparison operator");
    let hs = integer_literal_hs(value, false);
    assert_eq(h2(TYPE_CODE_LITERAL, hs), operand, "operand encoding mismatch");
    let lt = value < bound;
    let eq = value == bound;
    let result = if op == OP_LT {
        lt
    } else if op == OP_LE {
        lt | eq
    } else if op == OP_GT {
        !lt & !eq
    } else if op == OP_GE {
        !lt
    } else if op == OP_EQ {
        eq
    } else {
        !eq
    };
    assert(result, "filter verdict mismatch");
}

// Independent token construction for test expectations; production derives digits from u64.
fn encoded_token<let D: u32>(digits: [u8; D]) -> Field {
    let suffix = "^^<http://www.w3.org/2001/XMLSchema#integer>".as_bytes();
    let mut token: [u8; D + 46] = [0; D + 46];
    token[0] = 34;
    for i in 0..D { token[i + 1] = digits[i]; }
    token[D + 1] = 34;
    for i in 0..44 { token[D + 2 + i] = suffix[i]; }
    let digest = std::hash::blake3(token);
    let mut hs: Field = 0;
    for i in 1..32 { hs = hs * 256 + digest[i] as Field; }
    crate::hashes::h2(crate::hashes::TYPE_CODE_LITERAL, hs)
}

#[test]
fn canonical_boundaries_bind_exact_token() {
    full_u64_check(encoded_token("0".as_bytes()), 4, 0, 0);
    full_u64_check(encoded_token("99".as_bytes()), 4, 99, 99);
    full_u64_check(encoded_token("100".as_bytes()), 4, 100, 100);
    full_u64_check(encoded_token("9223372036854775808".as_bytes()), 4, 9223372036854775808, 9223372036854775808);
    full_u64_check(encoded_token("10000000000000000000".as_bytes()), 4, 10000000000000000000, 10000000000000000000);
    full_u64_check(encoded_token("18446744073709551615".as_bytes()), 4, 18446744073709551615, 18446744073709551615);
}

#[test(should_fail)]
fn twenty_digits_cannot_overflow_u64() {
    // If accumulation wrapped, this exact token would incorrectly satisfy equality with zero.
    let digits = "18446744073709551616".as_bytes();
    filter_int_check::<20>(encoded_token(digits), 4, 0, true, digits);
}

#[test(should_fail_with = "non-canonical leading zero")]
fn twenty_digit_leading_zero_is_not_a_shorter_token() {
    let digits = "00000000000000000000".as_bytes();
    filter_int_check::<20>(encoded_token(digits), 4, 0, true, digits);
}

#[test(should_fail_with = "operand encoding mismatch")]
fn full_width_value_cannot_detach_from_committed_token() {
    full_u64_check(encoded_token("18446744073709551614".as_bytes()), 1, 18446744073709551615, 18446744073709551615);
}

#[test(should_fail_with = "filter verdict mismatch")]
fn maximum_does_not_satisfy_strict_upper_bound() {
    full_u64_check(encoded_token("18446744073709551615".as_bytes()), 0, 18446744073709551615, 18446744073709551615);
}
