//! IN-CIRCUIT recompute of a `DualLeafV2` term encoding from its COMPONENTS,
//! with the S3.3 reserved-sentinel band check and the S14.1 reserved
//! language-tag check applied at the witnessed component sites
//! (`research/zk-field-native-encoding.md` S11 bead 3 / S14).
//!
//! # Why a recompute at all
//!
//! The shipped `scan` / `join` relations witness each term encoding as ONE
//! opaque `Field` and only re-hash the triple leaf (`scan.nr` step 1). That is
//! sound for the string-canonical method, whose encoding is a flat
//! `h2(TYPE_CODE, h_s(value))`, but it means a V2 dual-leaf graph's INTERNAL
//! leaf structure is never a circuit operand: the value component, the datatype
//! constant and the language constant are folded off-circuit, so nothing
//! in-circuit can say whether a leaf's slot-0 element is a real value handle or
//! the reserved no-value sentinel. This module makes those components real
//! operands.
//!
//! # The two V2 leaf shapes (mirror of `sparq_zk::dual_leaf_v2`)
//!
//! A V2 term encoding is always an outer `h3`, but slot 0 differs by term kind:
//!
//! ```text
//! IRI / blank node:  Enc = h3(VALUE_NONE_V2, identity, TYPE_CODE)
//! literal:           Enc = h3(value_component, identity, TYPE_CODE_LITERAL)
//!                    value_component = h3(value_slot0, DATATYPE_CONST, LANG_CONST)
//! ```
//!
//! -- so an IRI or blank node carries the sentinel FLAT in slot 0, while a
//! literal carries a nested value component whose own slot 0 is either a real
//! value handle (a value-lane literal) or the sentinel (a degenerate
//! string-lane literal). `identity` is the off-circuit identity hash: the
//! BLAKE3 field of the IRI, the salted blank-node inner, or the BLAKE3 field of
//! the canonical N-Triples token. It is NOT reconstructed here (that is the
//! in-circuit BLAKE3 this method exists to avoid) and stays a free witness,
//! exactly as the value-lane FILTER members take `lexical_component`.
//!
//! # What is enforced here, and what is NOT
//!
//! Enforced, per ACTIVE term slot:
//!
//! 1. the type code is one of the three committed term kinds;
//! 2. only a literal may carry a value handle or a language tag;
//! 3. **S3.3 band** -- a slot declared to carry a value handle must carry one
//!    the value lane can actually range-check (`in_handle_band`), and a slot
//!    declared NOT to is constrained to be EXACTLY `VALUE_NONE_V2`. Because the
//!    sentinel lies outside the band (machine-checked on both sides of the
//!    host/circuit seam), those two cases are mutually exclusive, so a
//!    degenerate component is a constrained operand rather than a free field;
//! 4. **S14.1 reserved tag** -- a language constant a leaf CARRIES is never the
//!    reserved `LANG_NONE` element, and a language-free leaf carries exactly
//!    `LANG_NONE`. This is the in-circuit form of the host's
//!    `dual_leaf_v2::checked_lang_const` refusal;
//! 5. canonical padding on the slots a non-literal does not use.
//!
//! NOT enforced, and deliberately so: nothing here binds `identity` to the
//! term's bytes, so the documented **INV-VL downgrade** is unchanged --
//! value<->lexical agreement on a value-lane leaf still rests on trusted-issuer
//! honesty, not machine enforcement. This module narrows which COMPONENT
//! SHAPES a committed leaf may be opened with; it is not a soundness result and
//! the whole ZK estate remains research-grade and NOT externally audited
//! (sq-qhy4).
//!
//! # Predicated asserts
//!
//! Every check is written against a per-slot `active` flag, so padding slots
//! (which the host zeroes) are exempt. That is why this module uses
//! `sparq_dual_leaf_v2::in_handle_band` -- the library's band PREDICATE, the
//! same single-sourced constants its own `assert_handle_in_band` uses -- rather
//! than the unconditional assert, which cannot be predicated.
//!
//! `crates/sparq-zk/tests/dual_leaf_v2_noir_pin.rs` pins this recompute to the
//! host encoder in two halves: a Rust mirror of the folds below, fed the
//! constants parsed out of the Noir sources and asserted equal to the host leaf
//! for a value-lane literal (the VALUES), plus a source-text assertion that the
//! two folds are still the expressions that mirror models (the SHAPE). The
//! IRI / blank-node shapes reduce to the sentinel and type-code pins in the same
//! file, because their leaf is the identity folded under those two constants.

use crate::hashes::{h3, TYPE_CODE_BLANK_NODE, TYPE_CODE_IRI, TYPE_CODE_LITERAL};
use sparq_dual_leaf_v2::{in_handle_band, LANG_NONE, VALUE_NONE_V2};

/// The witnessed components of ONE V2 term encoding.
///
/// `has_value` and `has_lang` are DECLARATIONS the relation holds the witness
/// to, not free annotations: each one selects which of two mutually-exclusive
/// constraints applies to the component beside it (checks 3 and 4 above).
pub struct TermComponents {
    /// `TYPE_CODE_IRI`, `TYPE_CODE_LITERAL` or `TYPE_CODE_BLANK_NODE`.
    pub type_code: Field,
    /// Whether this term is a value-lane literal carrying a real value handle.
    pub has_value: bool,
    /// Whether this term is a literal carrying a language tag.
    pub has_lang: bool,
    /// The value component's slot 0: the value handle, or `VALUE_NONE_V2`.
    pub value_slot0: Field,
    /// The lane constant (value lane) or degenerate datatype constant; `0` on a
    /// non-literal.
    pub datatype_const: Field,
    /// The language constant, or `LANG_NONE`.
    pub lang_const: Field,
    /// The off-circuit identity hash (see the module docs); a free witness.
    pub identity: Field,
}

/// Recompute one V2 term encoding, enforcing checks 1-5 when `active`.
///
/// Returns `0` for an inactive (padding) slot, matching the zero padding the
/// host writes and the shipped relations already tolerate.
pub fn recompute_term(c: TermComponents, active: bool) -> Field {
    let is_iri = c.type_code == TYPE_CODE_IRI;
    let is_literal = c.type_code == TYPE_CODE_LITERAL;
    let is_blank = c.type_code == TYPE_CODE_BLANK_NODE;

    // (1) A committed term is an IRI, a literal or a blank node. Nothing else
    //     has a V2 encoding, so no other type code may open a leaf.
    assert(!active | (is_iri | is_literal | is_blank), "term type code is not a committed term kind");

    // (2) The value lane and language tags belong to literals alone. An IRI or
    //     blank node carries the FLAT sentinel and no language.
    assert(!active | is_literal | !c.has_value, "a non-literal term declares a value handle");
    assert(!active | is_literal | !c.has_lang, "a non-literal term declares a language tag");

    // (3) S3.3 THE BAND CHECK. A declared value handle must be one the value
    //     lane can range-check; anything else must be exactly the reserved
    //     no-value sentinel. The sentinel lies outside the band, so these two
    //     cases cannot both hold.
    assert(
        !active | !c.has_value | in_handle_band(c.value_slot0),
        "value-lane hook is outside the handle band",
    );
    assert(
        !active | c.has_value | (c.value_slot0 == VALUE_NONE_V2),
        "degenerate value component is not the reserved no-value sentinel",
    );

    // (4) S14.1 THE RESERVED-TAG CHECK. A carried language constant is never
    //     LANG_NONE, and a language-free leaf carries exactly LANG_NONE.
    assert(
        !active | !c.has_lang | (c.lang_const != LANG_NONE),
        "a carried language constant is the reserved LANG_NONE tag",
    );
    assert(
        !active | c.has_lang | (c.lang_const == LANG_NONE),
        "a language-free term does not carry the LANG_NONE tag",
    );

    // (5) A non-literal uses no datatype slot; pin its padding so one leaf has
    //     one component witness.
    assert(!active | is_literal | (c.datatype_const == 0), "non-literal term carries a datatype constant");

    // (6) The leaf. A literal folds its nested value component; an IRI or blank
    //     node carries the sentinel flat in slot 0.
    let value_component =
        if is_literal { h3(c.value_slot0, c.datatype_const, c.lang_const) } else { VALUE_NONE_V2 };
    let enc = h3(value_component, c.identity, c.type_code);
    if active {
        enc
    } else {
        0
    }
}

mod tests {
    use super::{recompute_term, TermComponents};
    use crate::hashes::{h3, TYPE_CODE_BLANK_NODE, TYPE_CODE_IRI, TYPE_CODE_LITERAL};
    use sparq_dual_leaf_v2::{INTEGER_LANE_CONST, LANG_NONE, VALUE_NONE_V2};

    /// A value-lane integer literal: hook 42 under the integer lane constant.
    fn value_lane_literal() -> TermComponents {
        TermComponents {
            type_code: TYPE_CODE_LITERAL,
            has_value: true,
            has_lang: false,
            value_slot0: 42,
            datatype_const: INTEGER_LANE_CONST,
            lang_const: LANG_NONE,
            identity: 777,
        }
    }

    /// A degenerate (string-lane) literal: the sentinel in slot 0.
    fn degenerate_literal() -> TermComponents {
        TermComponents {
            type_code: TYPE_CODE_LITERAL,
            has_value: false,
            has_lang: false,
            value_slot0: VALUE_NONE_V2,
            datatype_const: 999,
            lang_const: LANG_NONE,
            identity: 888,
        }
    }

    fn iri_term() -> TermComponents {
        TermComponents {
            type_code: TYPE_CODE_IRI,
            has_value: false,
            has_lang: false,
            value_slot0: VALUE_NONE_V2,
            datatype_const: 0,
            lang_const: LANG_NONE,
            identity: 555,
        }
    }

    /// The recomputed encodings are EXACTLY the two documented shapes -- a
    /// literal folds a nested value component, an IRI carries the sentinel flat.
    #[test]
    fn the_recompute_reproduces_both_documented_leaf_shapes() {
        let lit = value_lane_literal();
        assert_eq(
            recompute_term(lit, true),
            h3(h3(42, INTEGER_LANE_CONST, LANG_NONE), 777, TYPE_CODE_LITERAL),
        );
        let iri = iri_term();
        assert_eq(recompute_term(iri, true), h3(VALUE_NONE_V2, 555, TYPE_CODE_IRI));
        // A blank node takes the IRI shape under its own type code.
        let mut b = iri_term();
        b.type_code = TYPE_CODE_BLANK_NODE;
        assert_eq(recompute_term(b, true), h3(VALUE_NONE_V2, 555, TYPE_CODE_BLANK_NODE));
    }

    /// The value lane and the degenerate lane produce DIFFERENT leaves for the
    /// same identity, so the value component is genuinely part of the leaf.
    #[test]
    fn the_value_component_changes_the_leaf() {
        let mut a = value_lane_literal();
        let mut b = value_lane_literal();
        b.value_slot0 = 43;
        assert(recompute_term(a, true) != recompute_term(b, true));
        a.identity = 888;
        assert(recompute_term(a, true) != recompute_term(degenerate_literal(), true));
    }

    /// The honest witnesses pass every check -- without this the refusals below
    /// could pass vacuously.
    #[test]
    fn the_honest_component_witnesses_are_accepted() {
        let _ = recompute_term(value_lane_literal(), true);
        let _ = recompute_term(degenerate_literal(), true);
        let _ = recompute_term(iri_term(), true);
        // A negative (sign-folded) handle is in band too.
        let mut neg = value_lane_literal();
        neg.value_slot0 = 0 - 42;
        let _ = recompute_term(neg, true);
        // A language-tagged degenerate literal carries a non-LANG_NONE constant.
        let mut tagged = degenerate_literal();
        tagged.has_lang = true;
        tagged.lang_const = 4242;
        let _ = recompute_term(tagged, true);
    }

    /// An INACTIVE slot is exempt and encodes to zero, so padding needs no
    /// canonical component witness.
    #[test]
    fn an_inactive_slot_is_exempt_and_encodes_to_zero() {
        let padding = TermComponents {
            type_code: 0,
            has_value: false,
            has_lang: false,
            value_slot0: 0,
            datatype_const: 0,
            lang_const: 0,
            identity: 0,
        };
        assert_eq(recompute_term(padding, false), 0);
    }

    // ------------------------------------------------- S3.3 the band check

    /// THE SENTINEL CANNOT BE PASSED OFF AS A VALUE HANDLE. This is the S3.3
    /// invariant the recompute exists to enforce in-circuit.
    #[test(should_fail_with = "value-lane hook is outside the handle band")]
    fn the_sentinel_is_refused_as_a_value_handle() {
        let mut c = value_lane_literal();
        c.value_slot0 = VALUE_NONE_V2;
        let _ = recompute_term(c, true);
    }

    /// Any out-of-band element is refused as a value handle, not just the
    /// sentinel -- so the check is a band check and not a sentinel blocklist.
    #[test(should_fail_with = "value-lane hook is outside the handle band")]
    fn an_out_of_band_element_is_refused_as_a_value_handle() {
        let mut c = value_lane_literal();
        c.value_slot0 = 0x10000000000000000; // 2^64, the first element past the band
        let _ = recompute_term(c, true);
    }

    /// A DEGENERATE component must be exactly the sentinel: a prover cannot put
    /// an arbitrary field element in a slot it declares carries no value.
    #[test(should_fail_with = "degenerate value component is not the reserved no-value sentinel")]
    fn a_degenerate_component_must_be_the_sentinel() {
        let mut c = degenerate_literal();
        c.value_slot0 = 42;
        let _ = recompute_term(c, true);
    }

    // --------------------------------------------- S14.1 the reserved tag

    /// A carried language constant equal to LANG_NONE is refused -- the
    /// in-circuit form of the host `checked_lang_const` refusal.
    #[test(should_fail_with = "a carried language constant is the reserved LANG_NONE tag")]
    fn a_carried_language_constant_may_not_be_lang_none() {
        let mut c = degenerate_literal();
        c.has_lang = true;
        c.lang_const = LANG_NONE;
        let _ = recompute_term(c, true);
    }

    /// And a language-FREE leaf may not carry some other element in the tag
    /// slot, so the two cases together pin the slot exactly.
    #[test(should_fail_with = "a language-free term does not carry the LANG_NONE tag")]
    fn a_language_free_term_may_not_carry_another_tag() {
        let mut c = degenerate_literal();
        c.lang_const = 4242;
        let _ = recompute_term(c, true);
    }

    // ------------------------------------------------------- shape checks

    #[test(should_fail_with = "term type code is not a committed term kind")]
    fn an_unknown_type_code_is_refused() {
        let mut c = iri_term();
        c.type_code = 4;
        let _ = recompute_term(c, true);
    }

    #[test(should_fail_with = "a non-literal term declares a value handle")]
    fn an_iri_may_not_declare_a_value_handle() {
        let mut c = iri_term();
        c.has_value = true;
        c.value_slot0 = 42;
        let _ = recompute_term(c, true);
    }

    #[test(should_fail_with = "a non-literal term declares a language tag")]
    fn an_iri_may_not_declare_a_language_tag() {
        let mut c = iri_term();
        c.has_lang = true;
        c.lang_const = 4242;
        let _ = recompute_term(c, true);
    }

    #[test(should_fail_with = "non-literal term carries a datatype constant")]
    fn an_iri_may_not_carry_a_datatype_constant() {
        let mut c = iri_term();
        c.datatype_const = 7;
        let _ = recompute_term(c, true);
    }
}
