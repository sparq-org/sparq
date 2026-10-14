//! Dual-leaf BIT-DOMAIN value-lane FILTER over a committed `xsd:boolean`
//! literal -- the dedicated boolean sibling of `filter_value::filter_value_dl_int`.
//!
//! WHY A SEPARATE MEMBER. The integer member range-decomposes its `VALUE_HOOK`
//! to the full `u64` domain, so a witnessed handle of `2` (or any other `u64`)
//! satisfies its range constraint even though no `xsd:boolean` value denotes it.
//! This member constrains BOTH the witnessed handle and the public `bound` to the
//! bit domain `{0, 1}`:
//!
//! ```text
//! hook  * (hook  - 1) == 0
//! bound * (bound - 1) == 0
//! ```
//!
//! so the relation itself refuses a handle or a bound outside the two boolean
//! values, rather than leaving that to the host encoder and the verifier. Adding
//! these constraints to the SHARED integer member was not an option: it would
//! change that member's gate count and its measured evidence, and the integer
//! lane legitimately uses the whole `u64` domain.
//!
//! PUBLIC LAYOUT. Identical to `filter_value_dl_int`'s -- `operand_enc`, `op`,
//! `bound`, `datatype_const`, `expected` (plus the prepended `challenge` in the
//! bin package) with the same ABI types -- so the host serialisation and the
//! verifier's public-input reconstruction are the integer lane's, field for
//! field. The LANE is carried by the public `datatype_const`, exactly as for the
//! other value lanes.
//!
//! THE VERDICT. On `{0, 1}` the XSD boolean order is `false < true`, which is
//! the integer order restricted to the bit domain -- so `EQ`/`NE` are ordinary
//! boolean equality and `LT`/`LE`/`GT`/`GE` are XPath's degenerate
//! `op:boolean-less-than` / `op:boolean-greater-than` results, meaningful rather
//! than errors. `bit_verdict` below restates `filter_value::integer_verdict`'s
//! shape over `bool` operands; it is a deliberate, small restatement because
//! that function is private to its module and this member must not edit it.
//!
//! HONESTY (load-bearing). This member inherits the dual-leaf method's
//! documented INV-VL downgrade: value<->lexical agreement on the value-FILTER
//! lane rests on TRUSTED-ISSUER HONESTY, not machine enforcement (#769 accepted
//! at research grade; gap CR-G8 / sq-qhy4). The bit constraints narrow what a
//! PROVER may witness on this lane; they are not a soundness or privacy
//! guarantee, and the whole ZK estate is NOT externally audited.
//!
//! Relation: `sparq_zk_compose_core::filter_value_bool::filter_value_dl_bool`;
//! compiled member: `zk/compose/filter_value_dl_bool`. Host half:
//! `sparq_zk::dual_leaf_v2::encode_boolean` +
//! `sparq_zk_compose::build::build_filter_value_dl_bool`.

use crate::filter_value::{LANG_NONE, OP_EQ, OP_GE, OP_GT, OP_LE, OP_LT, OP_NE};
use crate::hashes::{h3, TYPE_CODE_LITERAL};

/// Constrain `v` to the bit domain `{0, 1}`: `v * (v - 1) == 0` has exactly those
/// two roots over the field, so this is a complete characterisation and not a
/// range approximation. Two multiplications; no range decomposition, no lookup.
fn assert_bit(v: Field) {
    assert(v * (v - 1) == 0, "boolean operand outside the bit domain");
}

/// The comparison verdict for `value <op> bound` over the XSD boolean order
/// `false < true`. Restates the shape of `filter_value::integer_verdict` over
/// `bool` operands (that function is private to its module, which this member
/// must not edit). Both arms are paid unconditionally (flatten_cfg).
fn bit_verdict(value: bool, op: u32, bound: bool) -> bool {
    assert(op <= OP_NE, "unknown comparison operator");
    let lt = !value & bound;
    let eq = value == bound;
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

/// Dual-leaf value-FILTER over a committed `xsd:boolean`, bit-constrained.
///
/// Proves the disclosed verdict `expected` for `value <op> bound`, where `value`
/// is the committed literal's hidden boolean value, by binding the witnessed
/// `VALUE_HOOK` into the dual-leaf `operand_enc` via two Poseidon2 permutations
/// -- with NO in-circuit blake3, like the other value-lane members.
///
/// - `operand_enc`      : PUBLIC. The scan-anchored dual-leaf literal encoding.
/// - `op`               : PUBLIC. `0=lt 1=le 2=gt 3=ge 4=eq 5=ne`.
/// - `bound`            : PUBLIC. The FILTER constant, constrained to `{0, 1}`
///   (`0` = `false`, `1` = `true`).
/// - `expected`         : PUBLIC. The disclosed verdict.
/// - `value_hook`       : PRIVATE. The boolean value handle, constrained to
///   `{0, 1}`.
/// - `lexical_component`: PRIVATE. The OFF-circuit blake3 lexical hash, carried
///   as a FREE witness -- the leaf binding pins it; this member never hashes the
///   lexical bytes. Its INDEPENDENCE from `value_hook` is the removed INV-VL.
/// - `datatype_const`   : PUBLIC. The boolean lane's constant, folded into
///   `value_component`; it is what selects this lane.
pub fn filter_value_dl_bool(
    operand_enc: Field,
    op: u32,
    bound: u64,
    expected: bool,
    value_hook: Field,
    lexical_component: Field,
    datatype_const: Field,
) {
    // The bit domain, on BOTH operands. A handle or bound outside `{0, 1}`
    // denotes no boolean value, so the relation refuses it here.
    assert_bit(value_hook);
    let bound_field: Field = bound as Field;
    assert_bit(bound_field);

    // Read the two constrained bits as booleans.
    let value: bool = value_hook == 1;
    let bound_bit: bool = bound_field == 1;

    // value_component = h3(VALUE_HOOK, DATATYPE_CONST, LANG_NONE). Rebind from
    // the bit the comparison uses (cast back to Field), so the bound handle is
    // provably that bit and not a free field element.
    let value_canonical: Field = value as Field;
    let value_component = h3(value_canonical, datatype_const, LANG_NONE);

    // leaf = h3(value_component, lexical_component, TYPE_CODE_LITERAL).
    let leaf = h3(value_component, lexical_component, TYPE_CODE_LITERAL);

    // Bind to the scan-anchored committed leaf. No in-circuit blake3.
    assert_eq(leaf, operand_enc, "dual-leaf operand encoding mismatch");

    // The typed comparison verdict, asserted equal to the disclosed one.
    let result = bit_verdict(value, op, bound_bit);
    assert_eq(result, expected, "filter verdict mismatch");
}

// Build the dual-leaf operand_enc for a boolean exactly as the member does, so a
// test's `operand_enc` is the leaf an honest committer would have produced.
// `dt`/`lexical` are stand-ins for the off-circuit constants.
fn bool_operand_enc(hook: Field, dt: Field, lexical: Field) -> Field {
    let vc = h3(hook, dt, LANG_NONE);
    h3(vc, lexical, TYPE_CODE_LITERAL)
}

#[test]
fn accepts_true() {
    // flag = true, FILTER flag = true -> true.
    let dt: Field = 0xb001;
    let lexical: Field = 0xdef;
    let operand_enc = bool_operand_enc(1, dt, lexical);
    filter_value_dl_bool(operand_enc, OP_EQ, 1, true, 1, lexical, dt);
}

#[test]
fn accepts_false() {
    // flag = false, FILTER flag = true -> false (a disclosed false is provable).
    let dt: Field = 0xb001;
    let lexical: Field = 0xdef;
    let operand_enc = bool_operand_enc(0, dt, lexical);
    filter_value_dl_bool(operand_enc, OP_EQ, 1, false, 0, lexical, dt);
}

#[test]
fn accepts_the_degenerate_order_false_lt_true() {
    // The XSD boolean order on the bit domain: false < true.
    let dt: Field = 0xb001;
    let lexical: Field = 0xdef;
    let f = bool_operand_enc(0, dt, lexical);
    filter_value_dl_bool(f, OP_LT, 1, true, 0, lexical, dt);
    let t = bool_operand_enc(1, dt, lexical);
    filter_value_dl_bool(t, OP_GT, 0, true, 1, lexical, dt);
    filter_value_dl_bool(t, OP_LT, 0, false, 1, lexical, dt);
    filter_value_dl_bool(t, OP_GE, 1, true, 1, lexical, dt);
    filter_value_dl_bool(f, OP_LE, 0, true, 0, lexical, dt);
    filter_value_dl_bool(f, OP_NE, 1, true, 0, lexical, dt);
}

#[test(should_fail_with = "boolean operand outside the bit domain")]
fn rejects_hook_two() {
    // THE REASON THIS MEMBER EXISTS. A witnessed handle of 2 is inside the
    // integer member's u64 range but denotes no boolean value, so the bit
    // constraint refuses it -- even though the operand_enc was built for it and
    // the leaf binding would otherwise be satisfied.
    let dt: Field = 0xb001;
    let lexical: Field = 0xdef;
    let operand_enc = bool_operand_enc(2, dt, lexical);
    filter_value_dl_bool(operand_enc, OP_EQ, 1, false, 2, lexical, dt);
}

#[test(should_fail_with = "boolean operand outside the bit domain")]
fn rejects_bound_two() {
    // The same constraint on the PUBLIC bound: a FILTER constant outside
    // `{0, 1}` denotes no boolean value.
    let dt: Field = 0xb001;
    let lexical: Field = 0xdef;
    let operand_enc = bool_operand_enc(1, dt, lexical);
    filter_value_dl_bool(operand_enc, OP_EQ, 2, false, 1, lexical, dt);
}

#[test(should_fail_with = "filter verdict mismatch")]
fn rejects_lying_verdict() {
    // flag = false but the prover claims flag = true is true -> unprovable.
    let dt: Field = 0xb001;
    let lexical: Field = 0xdef;
    let operand_enc = bool_operand_enc(0, dt, lexical);
    filter_value_dl_bool(operand_enc, OP_EQ, 1, true, 0, lexical, dt);
}

#[test(should_fail_with = "dual-leaf operand encoding mismatch")]
fn rejects_wrong_datatype_const() {
    // LANE SEPARATION. The leaf was committed under the boolean lane constant;
    // recomputing it under a different constant does not reproduce operand_enc,
    // so the honest witness for one lane is unprovable on another. A property
    // under Poseidon2 preimage resistance, not an audited soundness claim.
    let committed_dt: Field = 0xb001;
    let other_dt: Field = 0x1234;
    let lexical: Field = 0xdef;
    let operand_enc = bool_operand_enc(1, committed_dt, lexical);
    filter_value_dl_bool(operand_enc, OP_EQ, 1, true, 1, lexical, other_dt);
}

#[test(should_fail_with = "dual-leaf operand encoding mismatch")]
fn rejects_unbound_value_hook() {
    // The handle is anchored to the scan-bound leaf: a prover cannot swap a
    // committed `true` for a `false` even though both are in the bit domain.
    let dt: Field = 0xb001;
    let lexical: Field = 0xdef;
    let operand_enc = bool_operand_enc(1, dt, lexical);
    filter_value_dl_bool(operand_enc, OP_EQ, 0, true, 0, lexical, dt);
}

#[test(should_fail_with = "dual-leaf operand encoding mismatch")]
fn rejects_wrong_lexical_component() {
    // The lexical_component is a FREE witness, but the leaf binding pins it.
    let dt: Field = 0xb001;
    let committed_lexical: Field = 0xdef;
    let operand_enc = bool_operand_enc(1, dt, committed_lexical);
    filter_value_dl_bool(operand_enc, OP_EQ, 1, true, 1, 0x999, dt);
}

#[test(should_fail_with = "unknown comparison operator")]
fn rejects_unknown_op() {
    let dt: Field = 0xb001;
    let lexical: Field = 0xdef;
    let operand_enc = bool_operand_enc(1, dt, lexical);
    filter_value_dl_bool(operand_enc, 6, 1, true, 1, lexical, dt);
}
