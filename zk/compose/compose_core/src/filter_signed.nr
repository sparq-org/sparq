//! [OPUS-4.8] sq-1q9h: hidden-operand numeric FILTER over committed SIGNED
//! `xsd:integer` and `xsd:decimal` literals -- extends `filter_int` (canonical
//! NON-negative `xsd:integer` only) to the negative-integer and decimal
//! fragments that real credential FILTER queries need (negative coordinates,
//! decimal amounts).
//!
//! # Same soundness anchor as `filter_int`
//! `sparq_zk::encode` commits a literal as `Enc = h2(LITERAL, h_s(token))` with
//! `h_s = blake3` over the canonical N-Triples token. oxrdf's
//! `Literal::to_string()` passes the lexical form through VERBATIM (it does NOT
//! re-canonicalise), so the committed token is `"<lexical>"^^<datatype>` with
//! `<lexical>` exactly the bytes that were ingested. These relations re-derive
//! that binding in-circuit: witness the lexical bytes (a sign flag + magnitude
//! digits, plus a fractional part for decimals), rebuild the EXACT token,
//! blake3-hash it (blackbox, one compression for short tokens -- the same
//! deliberate exception `filter_int` documents), truncate to the low 31 bytes
//! big-endian (mirroring `sparq_zk::field::field_from_hash_bytes`), and assert
//! `h2(LITERAL, hs) == operand_enc`. So `operand_enc` (the scan-proof anchor)
//! pins the EXACT committed literal, and the compared numeric value is a
//! CONSTRAINED function of the SAME witnessed bytes -- no free value the prover
//! could lie with. This is the `binding_consistency` edge, identical mechanism
//! to `filter_int`.
//!
//! # Why two relations, not signed `i64`
//! Noir's `Field` is unsigned; mixing sign into a single magnitude+flag pair and
//! doing sign-aware comparison over `u64` magnitudes keeps every assertion in the
//! native range-checked integer domain (no wrap, no signed-cast pitfalls;
//! noir-circuit-patterns "Standard library traits"). All comparison arms are
//! evaluated unconditionally so the gate count never depends on a secret value
//! (constant-shape rule).
//!
//! # Scope (HONEST deferral)
//! - `filter_signed_int_check`: the full `xsd:integer` fragment in the magnitude
//!   range that fits `u64` (`MD <= 19` magnitude digits), with an optional
//!   leading `-`. Canonical-form discipline (no leading zero unless the magnitude
//!   is `0`; `-0` rejected) mirrors `filter_int`.
//! - `filter_decimal_check`: the `xsd:decimal` fragment with `ID` integer digits
//!   and a comptime `FD` fraction digits, fixed-point compared at `FD` decimal
//!   places against a host-prescaled `bound` (sign + `bound_scaled = round(|v| *
//!   10^FD)`). The general `xsd:double` fractional/scientific/rounded fragment
//!   (needing an in-circuit decimal->IEEE round-to-nearest-even parser over an
//!   arbitrary lexical form) remains DEFERRED -- see the README "sparq-zk API
//!   gaps" and follow-up bead sq-7lrq; the integer-valued double fragment is
//!   already covered by `filter_float::filter_f64_composable_check`.

use crate::filter_int::{OP_EQ, OP_GE, OP_GT, OP_LE, OP_LT, OP_NE};
use crate::hashes::{h2, TYPE_CODE_LITERAL};

/// Sign-aware comparison verdict over two `(neg, magnitude)` operands at a
/// common scale (the caller scales decimals to a shared fixed point first).
/// `(true, 0)` is normalised to `(false, 0)` by the callers (no `-0`), so the
/// zero comparison is unambiguous. Returns the public verdict for `op`.
///
/// Numeric order: a negative is always `<` a non-negative; among negatives the
/// LARGER magnitude is the SMALLER value; among non-negatives the larger
/// magnitude is the larger value. All six predicates evaluated unconditionally.
// [OPUS-5] sq-rsd3v.3 (#2946): `pub(crate)` so the N3 `math:` comparison gadget
// (`crate::n3`) reuses this verdict VERBATIM rather than restating the signed
// order -- design record sec 4.3 ("reuse the EXISTING filter_int / filter_signed /
// decimal comparison circuits verbatim"). Behaviour unchanged.
pub(crate) fn signed_verdict(v_neg: bool, v_mag: u64, b_neg: bool, b_mag: u64, op: u32) -> bool {
    // lt := value < bound, eq := value == bound, over the signed order.
    let eq = (v_neg == b_neg) & (v_mag == b_mag);
    let lt = if v_neg & !b_neg {
        // negative < non-negative (covers -k < 0 and -k < +m)
        true
    } else if !v_neg & b_neg {
        // non-negative is never < a negative
        false
    } else if v_neg & b_neg {
        // both negative: -a < -b  <=>  a > b
        v_mag > b_mag
    } else {
        // both non-negative
        v_mag < b_mag
    };

    if op == OP_LT {
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
    }
}

/// Re-derive the term encoding of a typed literal whose lexical bytes are the
/// `LEN`-byte `lexical` array, with the comptime datatype suffix
/// `"^^<...#<datatype>>"` of length `SLEN`. The token is
/// `'"' + lexical + '"' + suffix`.
///
/// Returns the encoding rather than asserting it, so a caller that must MASK
/// the binding (the N3 builtin gadget, whose operand slots are only live for a
/// builtin atom) can compare it under a condition. [`assert_literal_binding`]
/// is the unconditional form the FILTER lane uses.
// [OPUS-5] sq-rsd3v.3 (#2946): split out of `assert_literal_binding` -- the
// hashing is unchanged, only the assert moved to the caller.
fn literal_encoding<let LEN: u32, let SLEN: u32>(
    lexical: [u8; LEN],
    suffix: [u8; SLEN],
) -> Field {
    let mut token: [u8; LEN + SLEN + 2] = [0; LEN + SLEN + 2];
    token[0] = 34; // '"'
    for i in 0..LEN {
        token[i + 1] = lexical[i];
    }
    token[LEN + 1] = 34; // '"'
    for i in 0..SLEN {
        token[LEN + 2 + i] = suffix[i];
    }
    let digest: [u8; 32] = std::hash::blake3(token);
    // Big-endian over the low 31 bytes (digest[1..32]) -- mirrors
    // `sparq_zk::field::field_from_hash_bytes` (truncation, 248 bits < modulus).
    let mut hs: Field = 0;
    for i in 1..32 {
        hs = hs * 256 + (digest[i] as Field);
    }
    h2(TYPE_CODE_LITERAL, hs)
}

/// As [`literal_encoding`], but asserts the re-derived encoding equals
/// `operand_enc`.
fn assert_literal_binding<let LEN: u32, let SLEN: u32>(
    operand_enc: Field,
    lexical: [u8; LEN],
    suffix: [u8; SLEN],
) {
    assert_eq(literal_encoding(lexical, suffix), operand_enc, "operand encoding mismatch");
}

/// Decode a witnessed signed `xsd:integer` operand and RE-DERIVE its term
/// encoding: check the magnitude digits are canonical, reconstruct the
/// magnitude, reject `-0`, and rebuild the canonical N-Triples token. Returns
/// `(encoding, neg, magnitude)` with `-0` normalised to `(false, 0)`, which is
/// what [`signed_verdict`] consumes.
///
/// The encoding is what makes the returned value TRUSTWORTHY: because the
/// digits are canonical, the term encoding determines the numeric value
/// uniquely and vice versa, so a prover can neither read one encoding as two
/// values nor emit a second encoding for one value. The CALLER asserts the
/// encoding matches the operand it claims to be -- unconditionally in the
/// FILTER lane ([`filter_signed_int_check`]), under a mask in the N3 builtin
/// gadget (`crate::n3`), whose operand slots are live only for a builtin atom.
///
/// Extracted from [`filter_signed_int_check`] so both lanes bind operands
/// through the SAME code path -- one implementation, no drift. The assert ORDER
/// is preserved exactly (digits, leading zero, `-0`, then the caller's binding)
/// so each `should_fail_with` test still pins the clause it names.
///
/// Note the canonical-digit asserts constrain only the WITNESS, never the
/// claimed operand, so they are safe to run unmasked; a caller that leaves an
/// operand slot unused must still supply canonical padding digits.
// [OPUS-5] sq-rsd3v.3 (#2946): extraction only -- no behaviour change.
pub(crate) fn signed_int_encoding<let MD: u32>(
    neg: bool,
    mag_digits: [u8; MD],
) -> (Field, bool, u64) {
    std::static_assert(MD >= 1, "at least one magnitude digit");
    std::static_assert(MD <= 19, "magnitude must fit u64");

    // Canonical magnitude digits: ASCII, no leading zero unless MD == 1.
    for i in 0..MD {
        assert((mag_digits[i] >= 48) & (mag_digits[i] <= 57), "not a decimal digit");
    }
    if MD > 1 {
        assert(mag_digits[0] != 48, "non-canonical leading zero");
    }

    // The magnitude value.
    let mut mag: u64 = 0;
    for i in 0..MD {
        mag = mag * 10 + ((mag_digits[i] - 48) as u64);
    }
    // No "-0": a negative sign on a zero magnitude is not a canonical xsd:integer
    // lexical form (and would be a second encoding of 0).
    if neg {
        assert(mag != 0, "negative zero is not canonical");
    }

    // Rebuild the canonical token. The lexical part is the optional '-' then the
    // magnitude digits; we build a fixed-length array with a leading byte that is
    // either '-' (0x2D) for negatives or the first digit for non-negatives, by
    // branching the rebuild over the comptime-free `neg`. To keep one comptime
    // token length we derive BOTH candidate encodings and select the witnessed
    // sign's (constant shape; the unselected hash is still paid).
    let suffix: [u8; 44] = "^^<http://www.w3.org/2001/XMLSchema#integer>".as_bytes();

    // Non-negative candidate lexical = mag_digits.
    let unsigned_enc = literal_encoding(mag_digits, suffix);
    // Negative candidate lexical = '-' + mag_digits.
    let mut signed_lex: [u8; MD + 1] = [0; MD + 1];
    signed_lex[0] = 45; // '-'
    for i in 0..MD {
        signed_lex[i + 1] = mag_digits[i];
    }
    let signed_enc = literal_encoding(signed_lex, suffix);

    let enc = if neg { signed_enc } else { unsigned_enc };
    (enc, neg & (mag != 0), mag)
}

/// Hidden-operand FILTER over a committed SIGNED `xsd:integer`.
///
/// Witness layout: `neg` is the sign flag (a `-` is prepended iff `neg`), and
/// `mag_digits` are the `MD` decimal digits of the MAGNITUDE (canonical: no
/// leading zero unless the magnitude is exactly `0`, and `-0` is rejected). The
/// rebuilt lexical form is `"-<digits>"` when `neg` else `"<digits>"`.
///
/// `bound` is supplied as a `(bound_neg, bound)` magnitude pair (the FILTER's
/// constant, sign-split by the host). `MD` is a circuit-family parameter (the
/// blake3 blackbox needs a comptime token length -- mirrors `filter_int_d{D}`);
/// `MD <= 19` keeps the magnitude in `u64`.
pub fn filter_signed_int_check<let MD: u32>(
    operand_enc: Field,
    op: u32,
    bound_neg: bool,
    bound: u64,
    expected: bool,
    neg: bool,
    mag_digits: [u8; MD],
) {
    assert(op <= OP_NE, "unknown comparison operator");

    // Canonical-digit checks, magnitude decode and `-0` rejection -- shared
    // verbatim with the N3 `math:` gadgets.
    let (enc, v_neg, mag) = signed_int_encoding(neg, mag_digits);
    assert_eq(enc, operand_enc, "operand encoding mismatch");

    let result = signed_verdict(v_neg, mag, bound_neg & (bound != 0), bound, op);
    assert_eq(result, expected, "filter verdict mismatch");
}

/// Hidden-operand FILTER over a committed `xsd:decimal`, compared at a fixed
/// `FD`-place scale.
///
/// Witness layout: `neg` is the sign flag; `int_digits` are the `ID` integer
/// digits (canonical: no leading zero unless the integer part is `0`); and
/// `frac_digits` are EXACTLY `FD` fraction digits. The rebuilt lexical form is
/// `[-]?<int_digits>.<frac_digits>` (a decimal with a point and `FD` fraction
/// places -- the member's fixed lexical shape). The scaled magnitude is
/// `|v| * 10^FD = int_part * 10^FD + frac_part`, compared against the host-
/// prescaled `bound_scaled = round(|bound| * 10^FD)` under the same sign rules.
///
/// `ID` / `FD` are circuit-family parameters (the blake3 blackbox needs a
/// comptime token length). The combined precision must fit `u64`:
/// `ID + FD <= 19`.
pub fn filter_decimal_check<let ID: u32, let FD: u32>(
    operand_enc: Field,
    op: u32,
    bound_neg: bool,
    bound_scaled: u64,
    expected: bool,
    neg: bool,
    int_digits: [u8; ID],
    frac_digits: [u8; FD],
) {
    std::static_assert(ID >= 1, "at least one integer digit");
    std::static_assert(FD >= 1, "at least one fraction digit");
    std::static_assert(ID + FD <= 19, "scaled magnitude must fit u64");
    assert(op <= OP_NE, "unknown comparison operator");

    // Canonical integer digits: ASCII, no leading zero unless ID == 1.
    for i in 0..ID {
        assert((int_digits[i] >= 48) & (int_digits[i] <= 57), "not a decimal digit");
    }
    if ID > 1 {
        assert(int_digits[0] != 48, "non-canonical leading zero");
    }
    // Fraction digits: ASCII decimal (a trailing zero IS canonical for decimals,
    // e.g. "3.0" -- so no no-trailing-zero rule here; the member's FD fixes the
    // fraction width and the binding pins the exact bytes).
    for i in 0..FD {
        assert((frac_digits[i] >= 48) & (frac_digits[i] <= 57), "not a decimal digit");
    }

    // The scaled magnitude: |v| * 10^FD.
    let mut int_part: u64 = 0;
    for i in 0..ID {
        int_part = int_part * 10 + ((int_digits[i] - 48) as u64);
    }
    let mut scale: u64 = 1;
    for _ in 0..FD {
        scale = scale * 10;
    }
    let mut frac_part: u64 = 0;
    for i in 0..FD {
        frac_part = frac_part * 10 + ((frac_digits[i] - 48) as u64);
    }
    let mag_scaled = int_part * scale + frac_part;
    // No "-0.0...0": a negative sign on a zero magnitude is not canonical.
    if neg {
        assert(mag_scaled != 0, "negative zero is not canonical");
    }

    // Operand binding: rebuild the lexical form '[-]?<int>.<frac>' then the
    // xsd:decimal token. Build the lexical once with the point fixed at the
    // ID-th position; the optional sign is handled by branching the rebuild.
    let suffix: [u8; 44] = "^^<http://www.w3.org/2001/XMLSchema#decimal>".as_bytes();
    if !neg {
        // lexical = int_digits + '.' + frac_digits  (length ID + 1 + FD)
        let mut lex: [u8; ID + FD + 1] = [0; ID + FD + 1];
        for i in 0..ID {
            lex[i] = int_digits[i];
        }
        lex[ID] = 46; // '.'
        for i in 0..FD {
            lex[ID + 1 + i] = frac_digits[i];
        }
        assert_literal_binding(operand_enc, lex, suffix);
    } else {
        // lexical = '-' + int_digits + '.' + frac_digits  (length ID + FD + 2)
        let mut lex: [u8; ID + FD + 2] = [0; ID + FD + 2];
        lex[0] = 45; // '-'
        for i in 0..ID {
            lex[i + 1] = int_digits[i];
        }
        lex[ID + 1] = 46; // '.'
        for i in 0..FD {
            lex[ID + 2 + i] = frac_digits[i];
        }
        assert_literal_binding(operand_enc, lex, suffix);
    }

    let result = signed_verdict(
        neg & (mag_scaled != 0),
        mag_scaled,
        bound_neg & (bound_scaled != 0),
        bound_scaled,
        op,
    );
    assert_eq(result, expected, "filter verdict mismatch");
}
