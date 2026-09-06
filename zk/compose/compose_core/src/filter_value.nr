//! Dual-leaf value-lane FILTER over a committed numeric literal (sq-xojl).
//!
//! THE GATE WIN, and THE PRICE. The blake3-bound `filter_int` family
//! (`filter_int.nr`) re-derives the operand binding by witnessing the digit
//! bytes, rebuilding the canonical N-Triples token, and hashing it with the
//! in-circuit `blake3` BLACKBOX (~14k gates -- the dominant cost). This member
//! AVOIDS that hash entirely: it proves a value-FILTER against the **dual-leaf**
//! commitment shape (`research/zk-field-native-encoding.md` S3.1), in which the
//! committed literal leaf is
//!
//! ```text
//! Enc_literal = h3(value_component, lexical_component, TYPE_CODE_LITERAL)
//!   value_component   = h3(VALUE_HOOK, DATATYPE_CONST, LANG_CONST)
//!   lexical_component = field_from_hash(blake3(canonical N-Triples token))   // OFF-circuit
//! ```
//!
//! so the FILTER binds the operand with TWO Poseidon2 permutations over the
//! witnessed `VALUE_HOOK` instead of one in-circuit blake3 -- and takes
//! `lexical_component` as a FREE witness (it never reconstructs or hashes the
//! lexical bytes; the scan-anchored leaf binding pins it). This is the
//! "value handle" comparison the dual-leaf method exists to enable.
//!
//! ## INV-VL DOWNGRADE -- DOCUMENTED RISK (load-bearing, read this)
//!
//! The blake3 family enforces, IN-CIRCUIT against an arbitrary committer
//! (including a malicious *trusted* issuer), the invariant **INV-VL**: *the
//! compared value equals the parse of the committed lexical bytes* -- because
//! the value AND the leaf binding both derive from the SAME witnessed digit
//! array (`filter_int.nr:67-92`). This member **REMOVES INV-VL**: `VALUE_HOOK`
//! and `lexical_component` are INDEPENDENT witnesses, so the leaf binds *that
//! the leaf contains both* but NOT that `VALUE_HOOK == parse(lexical)`. Binding
//! them in-circuit would re-introduce the exact blake3-over-the-token parse the
//! value handle exists to avoid -- defeating the gate win. So a malicious
//! *trusted* issuer can mint ONE signed credential whose `VALUE_HOOK` answers a
//! value-FILTER as 18 while its `lexical_component` answers
//! sameTerm/DISTINCT/join as "5". This is IMPOSSIBLE in the blake3 family. It is
//! a STRICT trust-model regression for the value-FILTER lane: value<->lexical
//! agreement moves from MACHINE-ENFORCED to TRUSTED-ISSUER-HONESTY. No
//! *untrusted* party gains anything (the issuer-signature chain is intact). The
//! maintainer ACKNOWLEDGED AND ACCEPTED this attack at research grade (#769) and
//! asked for it built WITH documentation. It is registered as an open external-
//! audit obligation (gap CR-G8 / sq-qhy4). The whole ZK estate is remediated +
//! internally re-audited but NOT externally audited -- nothing here is a
//! soundness or privacy guarantee.
//!
//! Honest mitigations (NOT substitutes for the audit): host-side same-leaf
//! co-binding at ingest (`research/zk-field-native-encoding.md` S6, fail-closed)
//! makes sparq's OWN commitments INV-VL-consistent; canonical-form issuance
//! (S5.6) is the named precondition that restores INV-VL for conforming issuers.
//!
//! ## B1 (range-decomposition) is INSTANTIATED, not named
//!
//! `VALUE_HOOK` arrives as a `Field`. A `Field` is NOT a `u64`: a prover could
//! supply a value congruent to a small number mod the BN254 modulus (modular
//! wrap -- reject-list (i)). So before any comparison this member RANGE-
//! DECOMPOSES `VALUE_HOOK` into the typed `u64` domain (`assert_max_bit_size::<64>()`,
//! the in-circuit range proof used across this stack) and the SAME
//! range-decomposed value feeds BOTH the operand binding (`value_component`) AND
//! the typed comparison -- so A1 binds "the same in-range value the comparison
//! uses", not merely "the same field". Omitting B1 would silently downgrade
//! in-range-ness from a prover-side circuit guarantee to an issuer/ingest
//! assumption (the S5.4 (2) escalation). This member does NOT omit it.
//!
//! ## B4 (canonical-form) for the integer datatype class
//!
//! `xsd:integer` VALUE_HOOK is the integer value itself (NOT the digit string),
//! so the value handle is canonical BY CONSTRUCTION: there is exactly one
//! `u64` magnitude per value, and `"05"` and `"5"` map to the SAME VALUE_HOOK.
//! That is precisely the value-collapse the dual leaf carries the
//! `lexical_component` to disambiguate for IDENTITY ops -- and exactly why
//! reject-list (v) (no identity op may read `value_component`) is load-bearing
//! and structurally enforced OUTSIDE this member (the dispatch matrix, sq-cfmv).
//! For this value-FILTER member, the single-encoding property B4 needs holds:
//! one value <-> one VALUE_HOOK, no second integer encoding. The decimal/double
//! datatype classes (whose VALUE_HOOK is many-to-one on the TERM and whose B4
//! needs an explicit canonical-scale / IEEE-bit bind) are a documented follow-up
//! (the integer class is the load-bearing first member; sq follow-up bead).
//!
//! `operand_enc` is PUBLIC: the hidden column's disclosed term encoding in the
//! manifest binding table, which the scan proof ties to a committed triple slot.
//! Composition is unchanged from `filter_int`: the verifier checks the same
//! `operand_enc` appears in both the scan proof's rows and this proof's public
//! inputs (the binding_consistency edge).

use crate::hashes::{h3, TYPE_CODE_LITERAL};
use sparq_ieee754::f64;

/// Comparison operator selectors (public input) -- identical encoding to
/// `filter_int` (`OP_LT=0 .. OP_NE=5`), reused so the host/verifier op mapping
/// is shared across the value lane and the blake3 lane.
pub global OP_LT: u32 = 0;
pub global OP_LE: u32 = 1;
pub global OP_GT: u32 = 2;
pub global OP_GE: u32 = 3;
pub global OP_EQ: u32 = 4;
pub global OP_NE: u32 = 5;

/// A reserved "no language" tag constant for the value lane's `LANG_CONST`
/// slot, mirroring the host `sparq_zk::dual_leaf::LANG_NONE`. A real lang tag
/// is `blake3(lang)`; numeric datatypes have no language, so they fold this
/// fixed sentinel. Distinct from any plausible `blake3` output by construction
/// (a small reserved field tag), so a value_component can never collide a
/// language-tagged one.
pub global LANG_NONE: Field = 1;

/// Range-decompose `v` (a `Field` witness) into a `u64`, proving it lies in the
/// `u64` domain with NO modular wrap (B1). `assert_max_bit_size::<64>()` is the
/// in-circuit range proof (the idiom used across this stack, e.g. the vendored
/// `json_parser`): it constrains `v < 2^64`, so the subsequent `v as u64` is the
/// exact value with no truncation / modular reduction. A prover supplying a
/// `VALUE_HOOK` congruent to a small value mod the BN254 modulus is UNPROVABLE.
fn range_decompose_u64(v: Field) -> u64 {
    v.assert_max_bit_size::<64>();
    v as u64
}

/// Evaluate the comparison verdict for `value <op> bound`. Both arms are paid
/// unconditionally (flatten_cfg; noir-optimisation S13). Shared with the blake3
/// lane's verdict shape so the disclosed `expected` semantics are identical.
fn integer_verdict(value: u64, op: u32, bound: u64) -> bool {
    assert(op <= OP_NE, "unknown comparison operator");
    let lt = value < bound;
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

/// Dual-leaf value-FILTER over a committed NON-NEGATIVE `xsd:integer`.
///
/// Proves the disclosed verdict `expected` for `value <op> bound` where `value`
/// is the committed literal's hidden numeric value, WITHOUT an in-circuit
/// blake3 (the gate win) -- by binding the witnessed `VALUE_HOOK` into the
/// dual-leaf `operand_enc` via two Poseidon2 permutations and range-decomposing
/// it (B1) before the typed comparison.
///
/// - `operand_enc`     : PUBLIC. The scan-anchored dual-leaf literal encoding.
/// - `op`, `bound`     : PUBLIC. The FILTER operator + constant.
/// - `expected`        : PUBLIC. The disclosed verdict.
/// - `value_hook`      : PRIVATE. The numeric value handle (a `Field`).
/// - `lexical_component`: PRIVATE. The OFF-circuit blake3 lexical hash, carried
///   as a FREE witness -- the leaf binding pins it; this member NEVER hashes the
///   lexical bytes (the saving). Its INDEPENDENCE from `value_hook` is exactly
///   the removed INV-VL (module doc).
/// - `datatype_const`  : PUBLIC. `blake3(datatype IRI)` as a field, a constant
///   for the known FILTER datatypes; folded into `value_component` so a cross-
///   datatype value collision (integer 5 vs the bits for 5.0) cannot occur.
pub fn filter_value_dl_int(
    operand_enc: Field,
    op: u32,
    bound: u64,
    expected: bool,
    value_hook: Field,
    lexical_component: Field,
    datatype_const: Field,
) {
    // B1: range-decompose VALUE_HOOK to the typed u64 domain, no modular wrap.
    // The SAME decomposed value feeds the binding AND the comparison.
    let value: u64 = range_decompose_u64(value_hook);

    // value_component = h3(VALUE_HOOK, DATATYPE_CONST, LANG_NONE).
    // Rebind from the RANGE-DECOMPOSED value (cast back to Field) so the bound
    // handle is provably the in-range one the comparison uses, not a free field.
    let value_canonical: Field = value as Field;
    let value_component = h3(value_canonical, datatype_const, LANG_NONE);

    // leaf = h3(value_component, lexical_component, TYPE_CODE_LITERAL).
    let leaf = h3(value_component, lexical_component, TYPE_CODE_LITERAL);

    // A1: bind to the scan-anchored committed leaf. No in-circuit blake3.
    assert_eq(leaf, operand_enc, "dual-leaf operand encoding mismatch");

    // B2/B3: the typed comparison verdict, asserted equal to the disclosed one.
    let result = integer_verdict(value, op, bound);
    assert_eq(result, expected, "filter verdict mismatch");
}

// =====================================================================
// xsd:double value-lane member (sq-2ezsx) -- the IEEE-bit canonical bind.
// =====================================================================

/// The canonical IEEE-754 quiet-NaN bit pattern the double value lane folds ALL
/// NaN payloads to (sign | exponent-all-ones | top-mantissa-bit). SPARQL/XPath
/// numeric equality treats every NaN as one unordered class, so the value handle
/// must NOT distinguish NaN payloads -- otherwise two terms that compare EQUAL
/// under `=` could carry distinct value handles (a many-to-one-on-the-value-but-
/// not-on-the-hook hazard). `+0.0` is `0`; `-0.0` (`0x8000000000000000`) folds to
/// `+0.0` for the same reason (`-0.0 = +0.0` numerically). Every OTHER double has
/// a unique bit pattern, so canonicalising only these two classes makes the value
/// handle single-valued per SPARQL-numeric value -- the B4 the integer class got
/// for free (`research/zk-field-native-encoding.md` S3.2 / the sq-2ezsx note).
global F64_CANONICAL_NAN: u64 = 0x7ff8000000000000;
global F64_NEG_ZERO_BITS: u64 = 0x8000000000000000;
/// `exponent == 0x7ff` (all ones) shifted into place -- the NaN/inf exponent.
global F64_EXP_ALL_ONES: u64 = 0x7ff0000000000000;
global F64_MANTISSA_MASK: u64 = 0x000fffffffffffff;

/// Fold an `xsd:double` bit pattern to its SPARQL-numeric CANONICAL bit pattern:
/// any NaN -> `F64_CANONICAL_NAN`, `-0.0` -> `+0.0`, everything else unchanged.
/// This is the explicit IEEE-bit bind (B4) the dual-leaf double class needs: the
/// `value_component` is keyed on the CANONICAL bits, so two bit-distinct but
/// numerically-equal terms (`-0.0`/`+0.0`; distinct NaN payloads) collapse to ONE
/// value handle -- the many-to-one-on-the-term property the `lexical_component`
/// disambiguates for identity ops (reject-list (v), enforced by the dispatch
/// matrix sq-cfmv). Pure `u64` bit arithmetic; no float decode.
fn canonical_f64_bits(bits: u64) -> u64 {
    // NaN iff exponent is all ones AND the mantissa is non-zero.
    let exp_all_ones = (bits & F64_EXP_ALL_ONES) == F64_EXP_ALL_ONES;
    let mantissa_nonzero = (bits & F64_MANTISSA_MASK) != 0;
    let is_nan = exp_all_ones & mantissa_nonzero;
    let is_neg_zero = bits == F64_NEG_ZERO_BITS;
    if is_nan {
        F64_CANONICAL_NAN
    } else if is_neg_zero {
        0
    } else {
        bits
    }
}

/// Dual-leaf value-FILTER over a committed `xsd:double` (sq-2ezsx).
///
/// The double analogue of [`filter_value_dl_int`]: it binds the operand to the
/// dual-leaf commitment via two Poseidon2 permutations over the witnessed
/// `VALUE_HOOK` (here the IEEE-754 bit pattern) and carries `lexical_component` as
/// a FREE witness -- NO in-circuit blake3 (the gate win), NO in-circuit
/// decimal->IEEE parse (the unbudgeted `filter_float` deferral the value lane
/// SIDESTEPS by committing the bits as the value handle).
///
/// ## B4 -- the IEEE-bit canonical bind (the load-bearing difference from int)
/// The integer class got B4 for free (one value <-> one `u64` magnitude). The
/// double class is MANY-TO-ONE on the term: `-0.0`/`+0.0` are numerically equal,
/// and every NaN is one unordered class with `2^52 - 1` payloads. So this member
/// CANONICALISES the witnessed bits ([`canonical_f64_bits`]) BEFORE forming the
/// `value_component`, and binds the leaf with the canonical bits. A prover who
/// witnesses `-0.0`'s bits and a committed leaf built from `+0.0`'s bits is
/// PROVABLE (they are the same value), and a NaN-payload swap cannot change the
/// value handle -- exactly the single-value-handle-per-SPARQL-value property the
/// blake3 family enforces lexically and that reject-list (v) requires.
///
/// ## B1 is the bit-range bind, not a magnitude range
/// `VALUE_HOOK` arrives as a `Field`; `assert_max_bit_size::<64>()` pins it to the
/// 64-bit IEEE domain (no modular wrap -- reject-list (i)), and the SAME 64-bit
/// value feeds BOTH the leaf binding and the typed `f64` comparison.
///
/// - `operand_enc`     : PUBLIC. The scan-anchored dual-leaf double encoding.
/// - `op`, `b_bits`    : PUBLIC. The FILTER operator + constant double (as IEEE
///   bits; NaN-handled by the `sparq_ieee754` predicate contract).
/// - `expected`        : PUBLIC. The disclosed verdict.
/// - `value_hook`      : PRIVATE. The IEEE-754 double bit pattern (a `Field`).
/// - `lexical_component`: PRIVATE. The OFF-circuit blake3 lexical hash, carried
///   as a FREE witness (the leaf binding pins it; never hashed in-circuit). Its
///   INDEPENDENCE from `value_hook` is the removed INV-VL (module doc).
/// - `datatype_const`  : PUBLIC. `blake3(xsd:double IRI)` as a field, folded into
///   `value_component` so a cross-datatype value collision cannot occur.
pub fn filter_value_dl_f64(
    operand_enc: Field,
    op: u32,
    b_bits: u64,
    expected: bool,
    value_hook: Field,
    lexical_component: Field,
    datatype_const: Field,
) {
    assert(op <= OP_NE, "unknown comparison operator");
    // B1: range-decompose VALUE_HOOK to the 64-bit IEEE domain (no modular wrap).
    value_hook.assert_max_bit_size::<64>();
    let raw_bits: u64 = value_hook as u64;

    // B4: fold to the SPARQL-numeric canonical bits BEFORE binding -- so the value
    // handle is single-valued per numeric value (-0.0/+0.0 and NaN payloads
    // collapse). The leaf is bound with the CANONICAL bits.
    let canon_bits = canonical_f64_bits(raw_bits);

    // value_component = h3(canonical VALUE_HOOK, DATATYPE_CONST, LANG_NONE).
    let value_canonical: Field = canon_bits as Field;
    let value_component = h3(value_canonical, datatype_const, LANG_NONE);

    // leaf = h3(value_component, lexical_component, TYPE_CODE_LITERAL).
    let leaf = h3(value_component, lexical_component, TYPE_CODE_LITERAL);

    // A1: bind to the scan-anchored committed leaf. No in-circuit blake3.
    assert_eq(leaf, operand_enc, "dual-leaf operand encoding mismatch");

    // B2/B3: the IEEE comparison verdict over the canonical value. Comparing the
    // canonical bits is sound: f64's predicates already treat -0.0 == +0.0 and a
    // NaN as unordered, and the canonical NaN/zero compare identically to any
    // member of their class.
    let a = f64::new(canon_bits);
    let b = f64::new(b_bits);
    let result = f64_verdict(a, b, op);
    assert_eq(result, expected, "filter verdict mismatch");
}

/// The IEEE-754 double comparison verdict for `a <op> b`. All six predicates are
/// paid unconditionally (constant-shape rule); shared with `filter_float`'s
/// `f64_verdict` shape so the disclosed `expected` semantics are identical across
/// the blake3 double lane and the dual-leaf double lane.
fn f64_verdict(a: f64, b: f64, op: u32) -> bool {
    if op == OP_LT {
        a.lt(b)
    } else if op == OP_LE {
        a.le(b)
    } else if op == OP_GT {
        a.gt(b)
    } else if op == OP_GE {
        a.ge(b)
    } else if op == OP_EQ {
        a.eq(b)
    } else {
        a.ne(b)
    }
}

// =====================================================================
// xsd:decimal value-lane member (sq-2ezsx) -- the canonical-scale bind.
// =====================================================================

/// Sign-aware fixed-point comparison verdict over two `(neg, magnitude)` operands
/// at a COMMON scale -- the dual-leaf decimal analogue of `filter_signed`'s
/// `signed_verdict` (kept local so `filter_value` does not depend on the blake3
/// lane). `(true, 0)` is normalised to `(false, 0)` by the caller (no `-0`). All
/// six predicates evaluated unconditionally.
fn signed_scaled_verdict(v_neg: bool, v_mag: u64, b_neg: bool, b_mag: u64, op: u32) -> bool {
    let eq = (v_neg == b_neg) & (v_mag == b_mag);
    let lt = if v_neg & !b_neg {
        true
    } else if !v_neg & b_neg {
        false
    } else if v_neg & b_neg {
        v_mag > b_mag
    } else {
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

/// Dual-leaf value-FILTER over a committed `xsd:decimal` at a FIXED canonical
/// fraction-digit scale (sq-2ezsx).
///
/// The decimal analogue of [`filter_value_dl_int`]: binds the operand to the
/// dual-leaf commitment via two Poseidon2 permutations over the witnessed
/// `VALUE_HOOK` and carries `lexical_component` as a FREE witness -- NO in-circuit
/// blake3. The `VALUE_HOOK` here is the SIGNED SCALED MAGNITUDE
/// `sign * round(|v| * 10^FD)` carried sign-split as `(value_neg, value_scaled)`.
///
/// ## B4 -- the canonical-scale bind (the load-bearing difference from int)
/// `xsd:decimal` is MANY-TO-ONE on the term at a fixed scale: `"5.0"` and `"5.00"`
/// are the SAME value but distinct lexical forms (and `"5.0"` at FD=1 vs FD=2 are
/// distinct members). This member pins the value handle to ONE canonical scale:
/// `value_scaled` is the magnitude at exactly `FD` fraction places, and the
/// member's `FD` is folded into the `value_component` via the public
/// `datatype_const` (the host MUST derive `datatype_const` from BOTH the
/// `xsd:decimal` IRI AND `FD`, `sparq_zk::dual_leaf::decimal_datatype_const`, so a
/// value at one scale can never collide a value at another). Within a member the
/// scaled value is canonical (one `(neg, value_scaled)` per value at scale `FD`);
/// `"5.0"`/`"5.00"` are disambiguated by `lexical_component` for identity ops
/// (reject-list (v)). The host fail-closed co-binding rejects a lexical form whose
/// fraction width != `FD` so sparq's own ingest cannot self-desync.
///
/// ## B1 -- range-decompose the scaled magnitude
/// `value_hook_scaled` arrives as a `Field`; `assert_max_bit_size::<64>()` pins it
/// to `u64` (no modular wrap -- reject-list (i)). The SAME range-decomposed value
/// feeds the binding AND the comparison.
///
/// - `operand_enc`        : PUBLIC. The scan-anchored dual-leaf decimal encoding.
/// - `op`                 : PUBLIC. The FILTER operator.
/// - `bound_neg`,`bound_scaled`: PUBLIC. The FILTER's constant, host-prescaled to
///   `FD` places (`bound_scaled = round(|bound| * 10^FD)`, sign in `bound_neg`).
/// - `expected`           : PUBLIC. The disclosed verdict.
/// - `value_neg`          : PRIVATE. Sign of the operand value.
/// - `value_hook_scaled`  : PRIVATE. `round(|v| * 10^FD)` as a `Field` -- the value
///   handle at the canonical scale.
/// - `lexical_component`  : PRIVATE. The OFF-circuit blake3 lexical hash (free
///   witness; the leaf binding pins it). Independence from the value handle is the
///   removed INV-VL (module doc).
/// - `datatype_const`     : PUBLIC. `blake3(xsd:decimal IRI || "@scale=FD")` -- folds
///   BOTH the datatype AND the scale so a cross-datatype OR cross-scale value
///   collision cannot occur.
pub fn filter_value_dl_decimal(
    operand_enc: Field,
    op: u32,
    bound_neg: bool,
    bound_scaled: u64,
    expected: bool,
    value_neg: bool,
    value_hook_scaled: Field,
    lexical_component: Field,
    datatype_const: Field,
) {
    assert(op <= OP_NE, "unknown comparison operator");
    // B1: range-decompose the scaled magnitude to u64, no modular wrap.
    value_hook_scaled.assert_max_bit_size::<64>();
    let value_scaled: u64 = value_hook_scaled as u64;

    // No "-0": a negative sign on a zero scaled magnitude is not a canonical
    // decimal value handle (it would be a second encoding of 0).
    if value_neg {
        assert(value_scaled != 0, "negative zero is not canonical");
    }

    // value_component = h3(VALUE_HOOK, DATATYPE_CONST, LANG_NONE), where VALUE_HOOK
    // is the SIGNED scaled magnitude. Fold the sign into the value handle so a
    // sign flip changes the handle (and thus the committed leaf): the signed value
    // handle is `value_scaled` for non-negative, `-value_scaled` (field negation)
    // for negative -- a single field per signed value at this scale. The B4 scale
    // bind lives in `datatype_const` (the host folds FD into it).
    let signed_value: Field = if value_neg {
        0 - (value_scaled as Field)
    } else {
        value_scaled as Field
    };
    let value_component = h3(signed_value, datatype_const, LANG_NONE);

    // leaf = h3(value_component, lexical_component, TYPE_CODE_LITERAL).
    let leaf = h3(value_component, lexical_component, TYPE_CODE_LITERAL);

    // A1: bind to the scan-anchored committed leaf. No in-circuit blake3.
    assert_eq(leaf, operand_enc, "dual-leaf operand encoding mismatch");

    // B2/B3: the sign-aware fixed-point comparison verdict at scale FD.
    let result = signed_scaled_verdict(
        value_neg & (value_scaled != 0),
        value_scaled,
        bound_neg & (bound_scaled != 0),
        bound_scaled,
        op,
    );
    assert_eq(result, expected, "filter verdict mismatch");
}

// =====================================================================
// xsd:dateTime / xsd:date value-lane member (sq-wz99x) -- the scaled-epoch bind.
// =====================================================================

/// Dual-leaf value-FILTER over a committed `xsd:dateTime` OR `xsd:date`
/// (sq-wz99x; `research/zk-field-native-encoding.md` S13.5).
///
/// Structurally the [`filter_value_dl_decimal`] member with a different value
/// handle: the `VALUE_HOOK` is the SIGNED SCALED EPOCH
/// `sign(T) * |T|`, `T` = milliseconds from `1970-01-01T00:00:00Z` on the XSD
/// proleptic-Gregorian `timeOnTimeline` (NO leap seconds), at the lane-fixed
/// sub-second scale `FS = 3` -- carried sign-split as
/// `(value_neg, value_hook_scaled)` exactly like the decimal member's scaled
/// magnitude. Binds the operand with TWO Poseidon2 permutations, carries
/// `lexical_component` as a FREE witness, and takes the verdict from the
/// UNCHANGED [`signed_scaled_verdict`] -- NO in-circuit blake3, no new
/// comparison machinery.
///
/// ## ONE relation serves BOTH lanes (the `datatype_const` is PUBLIC)
/// `xsd:date` is its OWN lane, but only by its lane CONSTANT: the host picks
/// `blake3("<xsd:dateTime IRI>@epochscale=3")` or
/// `blake3("<xsd:date IRI>@epochscale=3")`
/// (`sparq_zk::dual_leaf_datetime::{datetime_datatype_const,
/// date_datatype_const}`) and folds it into `value_component`. Since that
/// constant is a PUBLIC input, a date leaf's honest witness rebinds to a
/// DIFFERENT leaf under the dateTime constant and fails the
/// `assert_eq(leaf, operand_enc)` -- so the two lanes cannot cross even though a
/// date's hook is the scaled epoch of its STARTING instant (midnight UTC) and
/// therefore numerically equal to the dateTime hook of that same instant. No
/// second Noir function is needed (S13.5); this is a BINDING argument resting on
/// Poseidon2 preimage resistance, NOT an audited soundness claim.
///
/// ## B4 -- the scale + timezone bind lives OUTSIDE the circuit
/// `FS` is folded into `datatype_const` exactly like the decimal `@scale=` bind,
/// so a hook at one scale can never collide a hook at another. The hookable
/// DOMAIN (strict XSD-canonical `Z`-timezoned lexicals only; bare / non-`Z` /
/// `24:00:00` / leap-second / non-canonical-year / over-`FS`-fraction /
/// `u64`-overflowing forms are REJECTED fail-closed) is the host's S13.4
/// predicate, not an in-circuit check -- the same division of labour the decimal
/// member uses for its canonical fraction width. Within that domain the hook is
/// injective on the value within the datatype, so the comparison below is the
/// XSD `timeOnTimeline` order.
///
/// ## B1 -- range-decompose the scaled epoch
/// `value_hook_scaled` arrives as a `Field`; `assert_max_bit_size::<64>()` pins
/// it to `u64` (no modular wrap -- reject-list (i)). The SAME range-decomposed
/// value feeds the binding AND the comparison.
///
/// - `operand_enc`          : PUBLIC. The scan-anchored dual-leaf dateTime/date encoding.
/// - `op`                   : PUBLIC. The FILTER operator.
/// - `bound_neg`,`bound_scaled_epoch`: PUBLIC. The FILTER's constant instant, host-
///   converted to the SAME signed scaled epoch (`bound_scaled_epoch = |T_bound|` in
///   milliseconds, sign in `bound_neg`).
/// - `expected`             : PUBLIC. The disclosed verdict.
/// - `value_neg`            : PRIVATE. Sign of the operand's epoch.
/// - `value_hook_scaled`    : PRIVATE. `|T|` in milliseconds as a `Field` -- the
///   value handle at the lane-fixed scale `FS`.
/// - `lexical_component`    : PRIVATE. The OFF-circuit blake3 lexical hash (free
///   witness; the leaf binding pins it). Independence from the value handle is the
///   removed INV-VL (module doc).
/// - `datatype_const`       : PUBLIC. `blake3(<datatype IRI> || "@epochscale=FS")` --
///   selects the dateTime lane or the date lane, and folds the scale (B4).
pub fn filter_value_dl_datetime(
    operand_enc: Field,
    op: u32,
    bound_neg: bool,
    bound_scaled_epoch: u64,
    expected: bool,
    value_neg: bool,
    value_hook_scaled: Field,
    lexical_component: Field,
    datatype_const: Field,
) {
    assert(op <= OP_NE, "unknown comparison operator");
    // B1: range-decompose the scaled epoch to u64, no modular wrap.
    value_hook_scaled.assert_max_bit_size::<64>();
    let value_scaled: u64 = value_hook_scaled as u64;

    // No "-0": a negative sign on a zero scaled epoch is not a canonical value
    // handle (it would be a second encoding of the epoch instant itself). The
    // host encoder reports a zero magnitude as non-negative, so an honest witness
    // never trips this.
    if value_neg {
        assert(value_scaled != 0, "negative zero is not canonical");
    }

    // value_component = h3(VALUE_HOOK, DATATYPE_CONST, LANG_NONE), where VALUE_HOOK
    // is the SIGNED scaled epoch: `value_scaled` for a non-negative instant,
    // `-value_scaled` (field negation) for a pre-epoch one -- a single field per
    // signed instant at this scale, matching the host's `if neg { -mag } else { mag }`.
    let signed_value: Field = if value_neg {
        0 - (value_scaled as Field)
    } else {
        value_scaled as Field
    };
    let value_component = h3(signed_value, datatype_const, LANG_NONE);

    // leaf = h3(value_component, lexical_component, TYPE_CODE_LITERAL).
    let leaf = h3(value_component, lexical_component, TYPE_CODE_LITERAL);

    // A1: bind to the scan-anchored committed leaf. No in-circuit blake3.
    assert_eq(leaf, operand_enc, "dual-leaf operand encoding mismatch");

    // B2/B3: the UNCHANGED sign-aware fixed-point verdict, here over the scaled
    // epoch -- the XSD timeOnTimeline order on the Z-only hookable domain.
    let result = signed_scaled_verdict(
        value_neg & (value_scaled != 0),
        value_scaled,
        bound_neg & (bound_scaled_epoch != 0),
        bound_scaled_epoch,
        op,
    );
    assert_eq(result, expected, "filter verdict mismatch");
}

#[test]
fn accepts_true_ge_18() {
    // age = 18, FILTER age >= 18 -> true. Build the dual-leaf operand_enc the
    // SAME way the member does, then prove the verdict.
    let value_hook: Field = 18;
    let dt: Field = 0xabc; // a stand-in xsd:integer DATATYPE_CONST
    let lexical: Field = 0xdef; // a stand-in lexical_component (blake3 off-circuit)
    let vc = h3(value_hook, dt, LANG_NONE);
    let operand_enc = h3(vc, lexical, TYPE_CODE_LITERAL);
    filter_value_dl_int(operand_enc, OP_GE, 18, true, value_hook, lexical, dt);
}

#[test]
fn accepts_false_ge_18_when_under() {
    // age = 17, FILTER age >= 18 -> false (disclosed verdict false is provable).
    let value_hook: Field = 17;
    let dt: Field = 0xabc;
    let lexical: Field = 0xdef;
    let vc = h3(value_hook, dt, LANG_NONE);
    let operand_enc = h3(vc, lexical, TYPE_CODE_LITERAL);
    filter_value_dl_int(operand_enc, OP_GE, 18, false, value_hook, lexical, dt);
}

#[test(should_fail_with = "filter verdict mismatch")]
fn rejects_lying_verdict() {
    // age = 17 but the prover claims age >= 18 is true -> unprovable.
    let value_hook: Field = 17;
    let dt: Field = 0xabc;
    let lexical: Field = 0xdef;
    let vc = h3(value_hook, dt, LANG_NONE);
    let operand_enc = h3(vc, lexical, TYPE_CODE_LITERAL);
    filter_value_dl_int(operand_enc, OP_GE, 18, true, value_hook, lexical, dt);
}

#[test(should_fail_with = "dual-leaf operand encoding mismatch")]
fn rejects_unbound_value_hook() {
    // A prover supplies a value_hook NOT bound to operand_enc (the operand_enc
    // was built for value 18; the prover tries to compare value 5). A3/A1: the
    // value handle is anchored to the scan-bound leaf, so a free value is
    // unprovable. This is the binding that keeps the removed INV-VL a matter of
    // ISSUER trust only -- an UNTRUSTED prover still cannot swap the value.
    let dt: Field = 0xabc;
    let lexical: Field = 0xdef;
    let vc = h3(18, dt, LANG_NONE);
    let operand_enc = h3(vc, lexical, TYPE_CODE_LITERAL);
    // value_hook = 5 does not reproduce operand_enc.
    filter_value_dl_int(operand_enc, OP_GE, 5, true, 5, lexical, dt);
}

#[test(should_fail_with = "dual-leaf operand encoding mismatch")]
fn rejects_wrong_lexical_component() {
    // The lexical_component is a FREE witness, but the leaf binding pins it: a
    // prover cannot present a different lexical_component than the one committed.
    // (This is the binding; the removed INV-VL is that a *trusted issuer* may
    // have committed a lexical INCONSISTENT with value_hook -- documented risk,
    // NOT a prover-side forgery, which this rejects.)
    let value_hook: Field = 18;
    let dt: Field = 0xabc;
    let committed_lexical: Field = 0xdef;
    let vc = h3(value_hook, dt, LANG_NONE);
    let operand_enc = h3(vc, committed_lexical, TYPE_CODE_LITERAL);
    let attacker_lexical: Field = 0x999;
    filter_value_dl_int(operand_enc, OP_GE, 18, true, value_hook, attacker_lexical, dt);
}

#[test(should_fail_with = "call to assert_max_bit_size")]
fn rejects_modular_wrap_value_hook() {
    // B1: a value_hook of 2^64 (just past the u64 domain) must be rejected by the
    // range-decomposition, NOT silently reduced. Without B1 a prover could supply
    // a field congruent to a small value mod p.
    let value_hook: Field = 0x10000000000000000; // 2^64
    let dt: Field = 0xabc;
    let lexical: Field = 0xdef;
    let vc = h3(value_hook, dt, LANG_NONE);
    let operand_enc = h3(vc, lexical, TYPE_CODE_LITERAL);
    filter_value_dl_int(operand_enc, OP_GE, 0, true, value_hook, lexical, dt);
}

#[test(should_fail_with = "unknown comparison operator")]
fn rejects_unknown_op() {
    let value_hook: Field = 18;
    let dt: Field = 0xabc;
    let lexical: Field = 0xdef;
    let vc = h3(value_hook, dt, LANG_NONE);
    let operand_enc = h3(vc, lexical, TYPE_CODE_LITERAL);
    filter_value_dl_int(operand_enc, 6, 18, true, value_hook, lexical, dt);
}

// ---- xsd:double value-lane member (sq-2ezsx) ----

// Build the dual-leaf operand_enc for a double, with the CANONICAL bits (mirrors
// the member's B4 bind). `dt`/`lexical` are stand-ins for the off-circuit hashes.
fn f64_operand_enc(canon_bits: u64, dt: Field, lexical: Field) -> Field {
    let vc = h3(canon_bits as Field, dt, LANG_NONE);
    h3(vc, lexical, TYPE_CODE_LITERAL)
}

#[test]
fn f64_accepts_true_lt() {
    // value = 2.0 (0x4000000000000000), FILTER value < 3.0 -> true.
    let bits: u64 = 0x4000000000000000;
    let three: u64 = 0x4008000000000000;
    let dt: Field = 0xabc;
    let lexical: Field = 0xdef;
    let operand_enc = f64_operand_enc(bits, dt, lexical);
    filter_value_dl_f64(operand_enc, OP_LT, three, true, bits as Field, lexical, dt);
}

#[test]
fn f64_accepts_false_when_not_lt() {
    // value = 5.0 (0x4014000000000000), FILTER value < 3.0 -> false (provable).
    let bits: u64 = 0x4014000000000000;
    let three: u64 = 0x4008000000000000;
    let dt: Field = 0xabc;
    let lexical: Field = 0xdef;
    let operand_enc = f64_operand_enc(bits, dt, lexical);
    filter_value_dl_f64(operand_enc, OP_LT, three, false, bits as Field, lexical, dt);
}

#[test]
fn f64_neg_zero_collapses_to_pos_zero_for_the_value_handle() {
    // THE B4 INVARIANT (double): a leaf committed under +0.0's canonical bits is
    // PROVABLE by a prover witnessing -0.0's raw bits -- they are numerically
    // equal, and the member canonicalises -0.0 -> +0.0 BEFORE binding. So the
    // value handle is single-valued per numeric value.
    let dt: Field = 0xabc;
    let lexical: Field = 0xdef;
    let operand_enc = f64_operand_enc(0, dt, lexical); // committed under +0.0.
    // value = -0.0, FILTER value == +0.0 -> true (and the leaf binds).
    let pos_zero: u64 = 0;
    filter_value_dl_f64(operand_enc, OP_EQ, pos_zero, true, F64_NEG_ZERO_BITS as Field, lexical, dt);
}

#[test]
fn f64_distinct_nan_payloads_share_the_value_handle() {
    // THE B4 INVARIANT (double): a leaf committed under the canonical NaN is
    // PROVABLE by a prover witnessing a DIFFERENT NaN payload -- all NaNs are one
    // unordered class, so the canonicalisation collapses every payload to one
    // handle. NaN != NaN, so `value != bound` (OP_NE) is true.
    let dt: Field = 0xabc;
    let lexical: Field = 0xdef;
    let operand_enc = f64_operand_enc(F64_CANONICAL_NAN, dt, lexical);
    // A signalling-NaN payload (low mantissa bits set), distinct from canonical.
    let other_nan: u64 = 0x7ff0000000000001;
    // value (NaN) != bound (NaN) -> true (IEEE: NaN unordered against everything).
    filter_value_dl_f64(operand_enc, OP_NE, F64_CANONICAL_NAN, true, other_nan as Field, lexical, dt);
}

#[test(should_fail_with = "filter verdict mismatch")]
fn f64_rejects_lying_verdict() {
    // value = 5.0 but the prover claims value < 3.0 is true -> unprovable.
    let bits: u64 = 0x4014000000000000;
    let three: u64 = 0x4008000000000000;
    let dt: Field = 0xabc;
    let lexical: Field = 0xdef;
    let operand_enc = f64_operand_enc(bits, dt, lexical);
    filter_value_dl_f64(operand_enc, OP_LT, three, true, bits as Field, lexical, dt);
}

#[test(should_fail_with = "dual-leaf operand encoding mismatch")]
fn f64_rejects_unbound_value_hook() {
    // The operand_enc was built for 5.0; the prover tries to compare 2.0. The
    // value handle is anchored to the scan-bound leaf, so a free value is
    // unprovable (an UNTRUSTED prover cannot swap the value).
    let committed: u64 = 0x4014000000000000; // 5.0
    let dt: Field = 0xabc;
    let lexical: Field = 0xdef;
    let operand_enc = f64_operand_enc(committed, dt, lexical);
    let attacker: u64 = 0x4000000000000000; // 2.0
    let three: u64 = 0x4008000000000000;
    filter_value_dl_f64(operand_enc, OP_LT, three, true, attacker as Field, lexical, dt);
}

#[test(should_fail_with = "call to assert_max_bit_size")]
fn f64_rejects_modular_wrap_value_hook() {
    // B1: a value_hook past the 64-bit IEEE domain (2^64) must be rejected, not
    // silently reduced mod p.
    let value_hook: Field = 0x10000000000000000; // 2^64
    let dt: Field = 0xabc;
    let lexical: Field = 0xdef;
    let operand_enc = f64_operand_enc(0, dt, lexical);
    filter_value_dl_f64(operand_enc, OP_EQ, 0, true, value_hook, lexical, dt);
}

// ---- xsd:decimal value-lane member (sq-2ezsx) ----

// Build the dual-leaf operand_enc for a decimal at scale FD, with the SIGNED
// scaled value handle (mirrors the member's B4 canonical-scale bind). `dt` is a
// stand-in for `blake3(xsd:decimal IRI || "@scale=FD")`.
fn decimal_operand_enc(neg: bool, scaled: u64, dt: Field, lexical: Field) -> Field {
    let signed: Field = if neg { 0 - (scaled as Field) } else { scaled as Field };
    let vc = h3(signed, dt, LANG_NONE);
    h3(vc, lexical, TYPE_CODE_LITERAL)
}

#[test]
fn decimal_accepts_true_ge() {
    // value = 123.45 at FD=2 -> scaled 12345; FILTER value >= 100.00 (bound 10000).
    let dt: Field = 0xabc; // stands for blake3(decimal IRI || @scale=2)
    let lexical: Field = 0xdef;
    let operand_enc = decimal_operand_enc(false, 12345, dt, lexical);
    filter_value_dl_decimal(operand_enc, OP_GE, false, 10000, true, false, 12345, lexical, dt);
}

#[test]
fn decimal_accepts_negative_lt_positive() {
    // value = -2.50 (scaled 250, neg) < bound 0.00 -> true. The sign is folded
    // into the value handle, so the leaf for a negative differs from the positive.
    let dt: Field = 0xabc;
    let lexical: Field = 0xdef;
    let operand_enc = decimal_operand_enc(true, 250, dt, lexical);
    filter_value_dl_decimal(operand_enc, OP_LT, false, 0, true, true, 250, lexical, dt);
}

#[test(should_fail_with = "negative zero is not canonical")]
fn decimal_rejects_negative_zero() {
    // -0.00 (neg sign on a zero scaled magnitude) is not a canonical decimal value
    // handle -- fail closed (it would be a second encoding of 0).
    let dt: Field = 0xabc;
    let lexical: Field = 0xdef;
    let operand_enc = decimal_operand_enc(false, 0, dt, lexical);
    filter_value_dl_decimal(operand_enc, OP_EQ, false, 0, true, true, 0, lexical, dt);
}

#[test(should_fail_with = "filter verdict mismatch")]
fn decimal_rejects_lying_verdict() {
    // value = 1.00 (scaled 100) but the prover claims value >= 5.00 is true.
    let dt: Field = 0xabc;
    let lexical: Field = 0xdef;
    let operand_enc = decimal_operand_enc(false, 100, dt, lexical);
    filter_value_dl_decimal(operand_enc, OP_GE, false, 500, true, false, 100, lexical, dt);
}

#[test(should_fail_with = "dual-leaf operand encoding mismatch")]
fn decimal_rejects_sign_flip_unbinds_the_leaf() {
    // THE SIGN BIND: the operand_enc was committed for +2.50; a prover witnessing
    // -2.50 (same magnitude, flipped sign) does NOT reproduce the leaf, because
    // the sign is folded into the value handle. An untrusted prover cannot flip the
    // sign of the committed value.
    let dt: Field = 0xabc;
    let lexical: Field = 0xdef;
    let operand_enc = decimal_operand_enc(false, 250, dt, lexical); // +2.50
    filter_value_dl_decimal(operand_enc, OP_LT, false, 0, true, true, 250, lexical, dt);
}

#[test(should_fail_with = "call to assert_max_bit_size")]
fn decimal_rejects_modular_wrap_scaled_value() {
    // B1: a scaled value past u64 (2^64) must be rejected, not reduced mod p.
    let value: Field = 0x10000000000000000; // 2^64
    let dt: Field = 0xabc;
    let lexical: Field = 0xdef;
    let operand_enc = decimal_operand_enc(false, 0, dt, lexical);
    filter_value_dl_decimal(operand_enc, OP_EQ, false, 0, true, false, value, lexical, dt);
}

#[test(should_fail_with = "unknown comparison operator")]
fn decimal_rejects_unknown_op() {
    let dt: Field = 0xabc;
    let lexical: Field = 0xdef;
    let operand_enc = decimal_operand_enc(false, 100, dt, lexical);
    filter_value_dl_decimal(operand_enc, 6, false, 100, true, false, 100, lexical, dt);
}

// ---- xsd:dateTime / xsd:date value-lane member (sq-wz99x) ----

// Build the dual-leaf operand_enc for an instant at the lane scale FS, with the
// SIGNED scaled-epoch value handle (mirrors the member's bind, and the host
// `sparq_zk::dual_leaf_datetime::encode_datetime`/`encode_date` construction).
// `dt` stands for blake3("<datatype IRI>@epochscale=FS") -- the LANE constant.
fn datetime_operand_enc(neg: bool, scaled_epoch: u64, dt: Field, lexical: Field) -> Field {
    let signed: Field = if neg { 0 - (scaled_epoch as Field) } else { scaled_epoch as Field };
    let vc = h3(signed, dt, LANG_NONE);
    h3(vc, lexical, TYPE_CODE_LITERAL)
}

#[test]
fn datetime_accepts_true_ge() {
    // value = 2001-09-09T01:46:40Z (1_000_000_000_000 ms), FILTER value >=
    // 1970-01-02T00:00:00Z (86_400_000 ms) -> true.
    let dt: Field = 0xabc; // stands for blake3(dateTime IRI || @epochscale=3)
    let lexical: Field = 0xdef;
    let operand_enc = datetime_operand_enc(false, 1_000_000_000_000, dt, lexical);
    filter_value_dl_datetime(
        operand_enc,
        OP_GE,
        false,
        86_400_000,
        true,
        false,
        1_000_000_000_000,
        lexical,
        dt,
    );
}

#[test]
fn datetime_accepts_pre_epoch_lt_epoch() {
    // value = 1969-12-31T23:59:59.5Z -> (neg, 500 ms) < the epoch (0) -> true.
    // The sign is folded into the value handle by field negation, exactly as the
    // host encoder does (`if neg { -mag } else { mag }`).
    let dt: Field = 0xabc;
    let lexical: Field = 0xdef;
    let operand_enc = datetime_operand_enc(true, 500, dt, lexical);
    filter_value_dl_datetime(operand_enc, OP_LT, false, 0, true, true, 500, lexical, dt);
}

#[test]
fn datetime_cross_precision_operands_share_one_ordered_domain() {
    // The lane-fixed FS point: a whole-second lexical and a sub-second one land
    // in ONE scaled domain, so the single signed comparison orders them --
    // T12:00:00Z < T12:00:00.5Z by 500 ms. (2020-06-01T12:00:00Z = 1_590_998_400
    // s = 1_590_998_400_000 ms.)
    let dt: Field = 0xabc;
    let lexical: Field = 0xdef;
    let operand_enc = datetime_operand_enc(false, 1_590_998_400_000, dt, lexical);
    filter_value_dl_datetime(
        operand_enc,
        OP_LT,
        false,
        1_590_998_400_500,
        true,
        false,
        1_590_998_400_000,
        lexical,
        dt,
    );
}

#[test(should_fail_with = "dual-leaf operand encoding mismatch")]
fn date_lane_constant_separates_the_two_lanes() {
    // THE ONE-RELATION-TWO-LANES INVARIANT (S13.3/S13.5): a date's hook is the
    // scaled epoch of its STARTING instant, so 1970-01-02Z and
    // 1970-01-02T00:00:00Z carry the NUMERICALLY IDENTICAL hook (86_400_000).
    // They are still distinct terms, and the ONLY thing keeping them apart is the
    // PUBLIC lane constant folded into value_component: a leaf committed under the
    // DATE constant does not rebind under the DATETIME constant, so the honest
    // witness for the date operand is unprovable against the dateTime lane.
    let date_dt: Field = 0xda7e; // blake3(xsd:date IRI || @epochscale=3)
    let datetime_dt: Field = 0xabc; // blake3(xsd:dateTime IRI || @epochscale=3)
    let lexical: Field = 0xdef;
    let operand_enc = datetime_operand_enc(false, 86_400_000, date_dt, lexical);
    // Same hook, same lexical witness -- only the public lane constant differs.
    filter_value_dl_datetime(
        operand_enc,
        OP_EQ,
        false,
        86_400_000,
        true,
        false,
        86_400_000,
        lexical,
        datetime_dt,
    );
}

#[test(should_fail_with = "negative zero is not canonical")]
fn datetime_rejects_negative_zero() {
    // A negative sign on a zero scaled epoch is a second encoding of the epoch
    // instant -- fail closed (the host encoder never emits it).
    let dt: Field = 0xabc;
    let lexical: Field = 0xdef;
    let operand_enc = datetime_operand_enc(false, 0, dt, lexical);
    filter_value_dl_datetime(operand_enc, OP_EQ, false, 0, true, true, 0, lexical, dt);
}

#[test(should_fail_with = "filter verdict mismatch")]
fn datetime_rejects_lying_verdict() {
    // value = the epoch + 1000 ms, but the prover claims value >= 86_400_000 ms.
    let dt: Field = 0xabc;
    let lexical: Field = 0xdef;
    let operand_enc = datetime_operand_enc(false, 1_000, dt, lexical);
    filter_value_dl_datetime(operand_enc, OP_GE, false, 86_400_000, true, false, 1_000, lexical, dt);
}

#[test(should_fail_with = "dual-leaf operand encoding mismatch")]
fn datetime_rejects_sign_flip_unbinds_the_leaf() {
    // THE SIGN BIND: the operand_enc was committed for a POST-epoch instant; a
    // prover witnessing the same magnitude PRE-epoch does not reproduce the leaf.
    // An untrusted prover cannot move a committed instant across the epoch.
    let dt: Field = 0xabc;
    let lexical: Field = 0xdef;
    let operand_enc = datetime_operand_enc(false, 500, dt, lexical);
    filter_value_dl_datetime(operand_enc, OP_LT, false, 0, true, true, 500, lexical, dt);
}

#[test(should_fail_with = "call to assert_max_bit_size")]
fn datetime_rejects_modular_wrap_scaled_epoch() {
    // B1: a scaled epoch past u64 (2^64) must be rejected, not reduced mod p --
    // the circuit-side mirror of the host's far-proleptic overflow rejection.
    let value: Field = 0x10000000000000000; // 2^64
    let dt: Field = 0xabc;
    let lexical: Field = 0xdef;
    let operand_enc = datetime_operand_enc(false, 0, dt, lexical);
    filter_value_dl_datetime(operand_enc, OP_EQ, false, 0, true, false, value, lexical, dt);
}

#[test(should_fail_with = "unknown comparison operator")]
fn datetime_rejects_unknown_op() {
    let dt: Field = 0xabc;
    let lexical: Field = 0xdef;
    let operand_enc = datetime_operand_enc(false, 1_000, dt, lexical);
    filter_value_dl_datetime(operand_enc, 6, false, 1_000, true, false, 1_000, lexical, dt);
}
