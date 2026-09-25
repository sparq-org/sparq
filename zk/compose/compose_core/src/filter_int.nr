//! Hidden-operand numeric FILTER over committed `xsd:integer` literals.
//!
//! The encoding gap this circuit closes: `sparq_zk::encode` commits a
//! literal as `Enc = h2(TYPE_CODE_LITERAL, h_s(token))` with
//! `h_s = blake3` over the canonical N-Triples token -- a STRING hash, so a
//! committed numeric value has no field-arithmetic binding. To prove a
//! predicate about the hidden value we re-derive the binding in-circuit for
//! exactly one short token: witness the digit bytes, rebuild the canonical
//! token `"<digits>"^^<http://www.w3.org/2001/XMLSchema#integer>`, hash it
//! with the `blake3` BLACKBOX (~one compression for tokens <= 64 bytes -- a
//! measured, deliberate exception to the "never hash strings in-circuit"
//! house rule, ~3 orders below the standard-cryptosuite cliff of plan S2.3),
//! truncate the digest to the low 31 bytes big-endian (mirroring
//! `sparq_zk::field::field_from_hash_bytes`), and assert
//! `h2(2, digest_field) == operand_enc`.
//!
//! `operand_enc` is PUBLIC: it is the hidden column's disclosed term
//! encoding in the manifest's binding table, which the scan proof binds to a
//! committed triple slot. Composition: scan proof ties `operand_enc` to the
//! commitment; this proof ties `operand_enc` to the numeric value and the
//! verdict. The verifier checks the same `operand_enc` appears in both
//! public-input vectors (the `binding_consistency` edge of the modular
//! architecture, realised as a plain public-input equality).
//!
//! The digit count `D` is a circuit-family parameter (blake3's blackbox
//! needs a comptime byte length). The verifier accepts any `D` member -- the
//! token binding makes a wrong-D proof unsatisfiable -- at the documented
//! cost of leaking ceil(log10(value)) of the hidden operand.
//!
//! v1 limits: canonical non-negative `xsd:integer` only (no sign, no leading
//! zeros, D <= 20 with checked u64 accumulation). Negative integers and
//! xsd:decimal/double operands are deferred (README).

use crate::hashes::{h2, TYPE_CODE_LITERAL};

/// Comparison operator selector (public input).
pub global OP_LT: u32 = 0;
pub global OP_LE: u32 = 1;
pub global OP_GT: u32 = 2;
pub global OP_GE: u32 = 3;
pub global OP_EQ: u32 = 4;
pub global OP_NE: u32 = 5;

/// The generic relation; each `filter_int_d{D}` bin package is one
/// monomorphisation.
pub fn filter_int_check<let D: u32>(
    operand_enc: Field,
    op: u32,
    bound: u64,
    expected: bool,
    digits: [u8; D],
) {
    std::static_assert(D >= 1, "at least one digit");
    // [GPT-6] Twenty digits cover u64::MAX; checked accumulation rejects overflow.
    std::static_assert(D <= 20, "at most twenty decimal digits");
    assert(op <= OP_NE, "unknown comparison operator");

    // Canonical digit string: ASCII digits, no leading zero (unless D == 1).
    for i in 0..D {
        assert((digits[i] >= 48) & (digits[i] <= 57), "not a decimal digit");
    }
    if D > 1 {
        // comptime condition -- folded per member.
        assert(digits[0] != 48, "non-canonical leading zero");
    }

    // The numeric value.
    let mut value: u64 = 0;
    for i in 0..D {
        value = value * 10 + ((digits[i] - 48) as u64);
    }

    // Rebuild the canonical N-Triples token and re-derive the encoding.
    // Token layout: '"' + digits + '"' + "^^<http://www.w3.org/2001/XMLSchema#integer>".
    let suffix: [u8; 44] = "^^<http://www.w3.org/2001/XMLSchema#integer>".as_bytes();
    let mut token: [u8; D + 46] = [0; D + 46];
    token[0] = 34; // '"'
    for i in 0..D {
        token[i + 1] = digits[i];
    }
    token[D + 1] = 34; // '"'
    for i in 0..44 {
        token[D + 2 + i] = suffix[i];
    }
    let digest: [u8; 32] = std::hash::blake3(token);

    // sparq_zk::field::field_from_hash_bytes: big-endian over the low 31
    // bytes (digest[1..32]) -- truncation, not reduction (248 bits < modulus).
    let mut hs: Field = 0;
    for i in 1..32 {
        hs = hs * 256 + (digest[i] as Field);
    }
    assert_eq(h2(TYPE_CODE_LITERAL, hs), operand_enc, "operand encoding mismatch");

    // The comparison verdict. All comparisons are evaluated unconditionally
    // (flatten_cfg pays both arms anyway; noir-optimisation S13).
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
    assert_eq(result, expected, "filter verdict mismatch");
}
