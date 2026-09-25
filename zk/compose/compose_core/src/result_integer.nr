// [GPT-6] Canonical full-u64 private literal binding; research-stage, not externally audited.
//! Retains the existing string commitment and hides the exact decimal length.
//! Each fixed-length BLAKE3 candidate is constrained. The public circuit capacity
//! does not select or reveal the witness's actual digit count.

use crate::filter_int::filter_int_check;

fn check_digits<let D: u32>(operand: Field, op: u32, bound: u64, digits: [u8; 20]) {
    let mut selected = [0; D];
    for i in 0..D { selected[i] = digits[20 - D + i]; }
    filter_int_check::<D>(operand, op, bound, true, selected);
}

/// Opens the exact canonical xsd:integer token for a private u64 value.
/// Full-width arithmetic and lexical binding are enforced together; no host-only
/// conversion or newly attested commitment substitutes for the signed RDF term.
pub fn full_u64_check(operand: Field, op: u32, bound: u64, value: u64) {
    let mut digits: [u8; 20] = [48; 20];
    let mut remaining = value;
    for i in 0..20 {
        digits[19 - i] = 48 + (remaining % 10) as u8;
        remaining = remaining / 10;
    }
    if value < 10 {
        check_digits::<1>(operand, op, bound, digits);
    } else if value < 100 {
        check_digits::<2>(operand, op, bound, digits);
    } else if value < 1000 {
        check_digits::<3>(operand, op, bound, digits);
    } else if value < 10000 {
        check_digits::<4>(operand, op, bound, digits);
    } else if value < 100000 {
        check_digits::<5>(operand, op, bound, digits);
    } else if value < 1000000 {
        check_digits::<6>(operand, op, bound, digits);
    } else if value < 10000000 {
        check_digits::<7>(operand, op, bound, digits);
    } else if value < 100000000 {
        check_digits::<8>(operand, op, bound, digits);
    } else if value < 1000000000 {
        check_digits::<9>(operand, op, bound, digits);
    } else if value < 10000000000 {
        check_digits::<10>(operand, op, bound, digits);
    } else if value < 100000000000 {
        check_digits::<11>(operand, op, bound, digits);
    } else if value < 1000000000000 {
        check_digits::<12>(operand, op, bound, digits);
    } else if value < 10000000000000 {
        check_digits::<13>(operand, op, bound, digits);
    } else if value < 100000000000000 {
        check_digits::<14>(operand, op, bound, digits);
    } else if value < 1000000000000000 {
        check_digits::<15>(operand, op, bound, digits);
    } else if value < 10000000000000000 {
        check_digits::<16>(operand, op, bound, digits);
    } else if value < 100000000000000000 {
        check_digits::<17>(operand, op, bound, digits);
    } else if value < 1000000000000000000 {
        check_digits::<18>(operand, op, bound, digits);
    } else if value < 10000000000000000000 {
        check_digits::<19>(operand, op, bound, digits);
    } else {
        check_digits::<20>(operand, op, bound, digits);
    }
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
