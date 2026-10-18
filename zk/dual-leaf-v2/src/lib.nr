//! `sparq_dual_leaf_v2` -- the IN-CIRCUIT mirror of the host `DualLeafV2`
//! reserved no-value sentinel and of the scale-free value-lane constants.
//!
//! The host side lives in `crates/sparq-zk/src/dual_leaf_v2.rs` (behind the
//! default-off `dual-leaf` cargo feature). That module derives
//!
//! ```text
//! VALUE_NONE_V2 = derive_key("sparq dual-leaf v2 value-none", "")      // band-excluded
//! lane constant = derive_key("sparq dual-leaf v2 lane constant", framed(iri, params))
//! ```
//!
//! and asserts, on the HOST side, that the sentinel lies OUTSIDE the reachable
//! value-handle band
//!
//! ```text
//! [0, 2^64)  U  (p - 2^64, p)        (p = the BN254 scalar-field modulus)
//! ```
//!
//! -- the set of field elements a shipped value-lane member can range-check
//! (`assert_max_bit_size::<64>()`, with the signed lanes folding the sign by
//! field negation).
//!
//! This library carries the same constants as Noir `global`s and re-checks the
//! band exclusion IN NOIR, so the sentinel's defining invariant is machine-
//! checked on both sides of the host/circuit seam rather than on the host alone.
//! `crates/sparq-zk/tests/dual_leaf_v2_noir_pin.rs` reads the literals out of
//! THIS file and asserts each one equals its host derivation, so the two can
//! never drift apart silently.
//!
//! # Scope -- what this does and does NOT do (read this)
//!
//! This is a CONSTANTS + PREDICATE library. `sparq_zk_compose_core` depends on it
//! and `compose_core/src/dual_leaf_v2_leaf.nr` uses [`in_handle_band`],
//! [`VALUE_NONE_V2`] and [`LANG_NONE`] to assert the band exclusion and the
//! `LANG_NONE` separation at the witnessed component sites of the
//! `scan_v2dl_k1_n16_r4` / `join_eq_v2dl_na16_nb16` members.
//!
//! Those two members are not reachable from any verifier path: no `CircuitId`
//! names either package, so nothing here constrains a proof that is verified
//! today. The shipped `scan_*` / `join_eq_*` members are untouched (their sources
//! are hash-pinned by the `bench/zk-compose` measurement evidence), so adding this
//! library changed no existing circuit.
//!
//! The DECIMAL lane constant is scale-parameterised
//! (`decimal_lane_const(fd)`, `params = "scale=<fd>"`) and so is not a single
//! constant; it stays host-derived and has no `global` here.
//!
//! # Honesty (load-bearing)
//!
//! The dual-leaf value lane carries the documented INV-VL downgrade
//! (`zk/compose/compose_core/src/filter_value.nr`): value<->lexical agreement on
//! the value-FILTER lane rests on trusted-issuer honesty, not machine
//! enforcement. The whole ZK estate is research-grade and NOT externally
//! audited. Nothing in this file is a soundness or privacy guarantee.

use std::field::bn254::lt;

/// `2^64` -- the exclusive upper end of the `u64` half of the reachable
/// value-handle band. See [`in_handle_band`].
pub global HANDLE_BAND_LIMIT: Field = 0x10000000000000000;

/// The V2 reserved **no-value** sentinel, `dual_leaf_v2::value_none()` on the
/// host: the slot-0 tag a degenerate (non-value-lane) leaf carries, and the flat
/// slot-0 tag of an IRI / blank-node leaf.
///
/// It lies OUTSIDE the band every value-lane member can range-check -- asserted
/// by [`value_none_lies_outside_the_handle_band`] below, and by the host test of
/// the same name.
pub global VALUE_NONE_V2: Field =
    0x00fe2f1f6b7aca564b1ccd5dc072a739e14179b3abee2e971bd03acb54de1016;

/// The "no language" tag of the value lane's `LANG_CONST` slot, unchanged from
/// V1 -- the same reserved element
/// `zk/compose/compose_core/src/filter_value.nr::LANG_NONE` carries.
pub global LANG_NONE: Field = 1;

/// The V2 `xsd:integer` lane constant (`params = ""`).
pub global INTEGER_LANE_CONST: Field =
    0x007726bb24c933495816d0df140f6b81e0b86b4f67b9158cb900e2df862faee3;

/// The V2 `xsd:double` lane constant (`params = ""`).
pub global DOUBLE_LANE_CONST: Field =
    0x00512a8930674f4b44782714c6efe7542ef4b86e2293a419dc5ad8ba5cb31396;

/// The V2 `xsd:boolean` lane constant (`params = "bit"`).
pub global BOOLEAN_LANE_CONST: Field =
    0x00749541bb2649a47da52bc3f83b6e7bc44ce704e53e4ba3f7e7df0dfa1c58fb;

/// The V2 `xsd:dateTime` lane constant (`params = "epochscale=3"`).
pub global DATETIME_LANE_CONST: Field =
    0x00d145858f37bb8aa0f78d31a8a8dea3fcd433256e99ec7a7c9655df731d6b8d;

/// The V2 `xsd:date` lane constant (`params = "epochscale=3"`).
pub global DATE_LANE_CONST: Field =
    0x005a67a9e487ed97ada70709960ebfcc1cb9e6b54527b3ff37d62e480ed9968d;

/// Whether `f` lies in the REACHABLE value-handle band
/// `[0, 2^64) U (p - 2^64, p)` -- the Noir mirror of the host
/// `dual_leaf_v2::is_in_handle_band`.
///
/// `lt(f, 2^64)` covers `[0, 2^64)`; `lt(-f, 2^64)` covers `(p - 2^64, p)` (and,
/// for `f == 0`, is the same case as the first).
pub fn in_handle_band(f: Field) -> bool {
    lt(f, HANDLE_BAND_LIMIT) | lt(0 - f, HANDLE_BAND_LIMIT)
}

/// Asserts a witnessed value-lane handle is not the reserved no-value sentinel.
///
/// This is the assertion a dual-leaf leaf recompute would place on a value-lane
/// hook. Nothing calls it today: the leaf recompute in
/// `compose_core/src/dual_leaf_v2_leaf.nr` needs a PREDICATED assert -- its checks
/// are gated on a slot being active and declaring a handle -- which this
/// unconditional form cannot express, so it uses [`in_handle_band`] and
/// [`VALUE_NONE_V2`] directly.
pub fn assert_handle_not_value_none(handle: Field) {
    assert(handle != VALUE_NONE_V2, "value-lane hook is the no-value sentinel");
}

/// Asserts a witnessed value-lane handle is a value a member can range-check --
/// the stronger in-band form of [`assert_handle_not_value_none`], which refuses
/// the sentinel and every other unreachable field element alike.
pub fn assert_handle_in_band(handle: Field) {
    assert(in_handle_band(handle), "value-lane hook is outside the handle band");
}

mod tests {
    use super::{
        assert_handle_in_band, assert_handle_not_value_none, in_handle_band, BOOLEAN_LANE_CONST,
        DATE_LANE_CONST, DATETIME_LANE_CONST, DOUBLE_LANE_CONST, HANDLE_BAND_LIMIT,
        INTEGER_LANE_CONST, LANG_NONE, VALUE_NONE_V2,
    };

    /// The band predicate itself: the two ends of the `u64` half, the top of the
    /// negated half, and the first element outside each.
    #[test]
    fn handle_band_covers_exactly_the_range_checkable_values() {
        assert(in_handle_band(0));
        assert(in_handle_band(1));
        assert(in_handle_band(HANDLE_BAND_LIMIT - 1));
        assert(in_handle_band(0 - 1));
        assert(in_handle_band(0 - (HANDLE_BAND_LIMIT - 1)));
        // 2^64 and -2^64 are the first elements past each end.
        assert(!in_handle_band(HANDLE_BAND_LIMIT));
        assert(!in_handle_band(0 - HANDLE_BAND_LIMIT));
    }

    /// THE INVARIANT. The in-circuit mirror of the host's
    /// `value_none_lies_outside_the_handle_band`.
    #[test]
    fn value_none_lies_outside_the_handle_band() {
        assert(!in_handle_band(VALUE_NONE_V2));
    }

    /// The sentinel is distinct from every constant that shares a leaf slot with
    /// it or sits next to it in the value component.
    #[test]
    fn value_none_differs_from_every_lane_constant_and_from_lang_none() {
        assert(VALUE_NONE_V2 != LANG_NONE);
        assert(VALUE_NONE_V2 != INTEGER_LANE_CONST);
        assert(VALUE_NONE_V2 != DOUBLE_LANE_CONST);
        assert(VALUE_NONE_V2 != BOOLEAN_LANE_CONST);
        assert(VALUE_NONE_V2 != DATETIME_LANE_CONST);
        assert(VALUE_NONE_V2 != DATE_LANE_CONST);
    }

    /// The five scale-free lane constants are pairwise distinct -- the dateTime /
    /// date pair is the load-bearing one, since a date's handle is numerically
    /// equal to the dateTime handle of its starting instant and only the lane
    /// constant separates the two lanes.
    #[test]
    fn the_scale_free_lane_constants_are_pairwise_distinct() {
        let lanes = [
            INTEGER_LANE_CONST,
            DOUBLE_LANE_CONST,
            BOOLEAN_LANE_CONST,
            DATETIME_LANE_CONST,
            DATE_LANE_CONST,
        ];
        for i in 0..5 {
            for j in 0..5 {
                if i != j {
                    assert(lanes[i] != lanes[j]);
                }
            }
        }
    }

    #[test]
    fn a_real_handle_passes_both_assertions() {
        assert_handle_not_value_none(18);
        assert_handle_in_band(18);
        // A negative (sign-folded) handle is in band too.
        assert_handle_in_band(0 - 18);
    }

    #[test(should_fail_with = "value-lane hook is the no-value sentinel")]
    fn the_sentinel_is_refused_as_a_handle() {
        assert_handle_not_value_none(VALUE_NONE_V2);
    }

    #[test(should_fail_with = "value-lane hook is outside the handle band")]
    fn the_sentinel_is_refused_by_the_band_assertion() {
        assert_handle_in_band(VALUE_NONE_V2);
    }
}
