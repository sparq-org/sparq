//! [SONNET-4.6] (sq-y5ew5) Hybrid tri-mask FILTER kernel for **general decoded columns**.
//!
//! Phase 3 (`columnar_filter`) required every id in the filter column to be an
//! inline integer — a conservative gate that ensured the f64 verdict was always
//! unambiguous. This module extends the eligible set to any column whose constant
//! decodes to a finite f64, while preserving byte-identity via the **tri-mask** design
//! described in `research/vector-at-a-time-m4-completion-design.md` §3 ("sq-y5ew5").
//!
//! ## The tri-mask design (§3 of the completion-design record)
//!
//! One columnar pass over the decoded f64 column classifies every lane into exactly
//! one of three buckets:
//!
//! 1. **Confident** — `decoded[i]` is **finite** AND `decoded[i] != c` (bit-exact).
//!    By the monotone-rounding lemma, the f64 verdict on a non-tie lane is unambiguous:
//!    `f64(x) < f64(c) ⇒ exact(x) < exact(c)`, and so on for all six operators.
//!    The comparison `cmp.test(decoded[i])` gives the correct exact answer.
//! 2. **Tie** — `decoded[i]` is finite AND `decoded[i] == c` (also catches +0.0/−0.0).
//!    The f64 verdict is ambiguous: the exact value of `x` could be strictly less than,
//!    equal to, or strictly greater than the exact constant. **Delegated.**
//! 3. **Unknown** — `decoded[i]` is `NaN` (non-numeric term, derived/local-vocab id,
//!    `NO_ID` unbound, or a genuine `NaN`-valued double — all conflate safely).
//!    No f64 verdict is possible. **Delegated.**
//!
//! Confident lanes whose comparison passes join the confident-pass list.
//! Delegated (Tie + Unknown) lanes are returned to the caller, which evaluates the
//! FULL scalar row predicate on the original `Bindings` rows and collects
//! delegated-pass indices.
//!
//! The final selection vector is the ascending merge of confident-passes and
//! delegated-passes (both lists are ascending within each source). One
//! order-preserving `apply_selection` materialises survivors.
//!
//! ## Byte-identity argument (§3, by construction)
//!
//! Premise: decode parity (I4) — the decoded f64 agrees with `graph.numeric_value`.
//!
//! - **Confident pass/fail:** the scalar predicate on the same row sees the same two
//!   f64 values (lane value and constant); because the lane is a non-tie, no
//!   exact-lexical recheck changes the verdict. So `cmp.test(x)` gives the same
//!   answer as the scalar path.
//! - **Tie / Unknown:** the hybrid's verdict IS the scalar predicate's verdict,
//!   because it calls it directly.
//! - **Order:** both confident-pass and delegated-pass lists are built in ascending
//!   index order; the merge is ascending; `apply_selection` is order-preserving.
//!
//! ## ZK / budget coupling (I2 / I3)
//!
//! The dispatcher in `exec.rs` already handles I2 (zk-decline) and I3 (budget-armed
//! decline) before this kernel is called. When a zk trace is armed the whole seam
//! never runs; when a budget is armed the seam declines entirely. The delegation here
//! calls only the scalar predicate with NO additional loop state — it records no
//! obligations and cannot interfere with the zk trace even if it were armed (which it
//! cannot be, by I2).
//!
//! This module is only compiled when `feature = "vectorized"` is active
//! (registered in `lib.rs` under the same gate). When the feature is OFF, zero code
//! from here compiles and the default native + wasm builds are byte-identical.

use crate::chunk::VecCmp;

// ---- Tri-mask lane classification ------------------------------------------

/// The result of one tri-mask pass over a decoded morsel column.
///
/// Both lists are in **ascending** index order (indices are morsel-local).
/// The caller merges `confident_passes` and `delegated` from the scalar evaluation
/// (in ascending order) to produce the final ascending selection for this morsel.
#[derive(Debug, Default)]
pub(crate) struct TriMaskResult {
    /// Morsel-local row indices that pass the comparison with a confident (unambiguous)
    /// f64 verdict. Ascending. These rows do NOT need scalar delegation.
    pub confident_passes: Vec<usize>,
    /// Morsel-local row indices whose verdict is ambiguous (Tie: `x == c` as f64)
    /// or impossible (Unknown: `x` is NaN). Ascending. The caller must evaluate the
    /// full scalar predicate on each delegated row and collect the passes.
    pub delegated: Vec<usize>,
}

/// Classify each lane in a pre-decoded f64 column against the comparison `cmp`.
///
/// The threshold `c` is the f64 value embedded in `cmp` (identical for all five
/// comparison operators). A lane is classified as:
///
/// - **Unknown** if `decoded[i].is_nan()` — NaN sentinel; delegated.
/// - **Tie** if `decoded[i] == c` (IEEE-754 bit-equality, also catches +0.0/−0.0);
///   delegated.
/// - **Confident pass** if `decoded[i]` is finite, `!= c`, and `cmp.test(x)` is true.
/// - **Confident fail** if `decoded[i]` is finite, `!= c`, and `cmp.test(x)` is false.
///   (Confident fails are not tracked — they are simply absent from both lists.)
///
/// Both output vectors are ascending; their union is the complete delegated + pass set
/// for this morsel. Caller merges `confident_passes` with the delegated-pass set after
/// scalar evaluation to get the full morsel-local survivor list.
///
/// ## Inline gather-free fast path
///
/// This function does not special-case the all-inline column; the inline gather-free
/// fast path was already executed by `DataChunk::decode_numeric_column` before this
/// call. For an all-inline column the decoded column contains no NaN sentinels; the
/// only delegated lanes are exact ties (`x == c` as f64), which for inline integers
/// means `exact(x) == exact(c)` (no ambiguity either way — delegate is safe and
/// correct). In practice, inline integers are dense-valued and a threshold that
/// matches one exactly means the tie case is always delegated safely.
pub(crate) fn tri_mask_select(decoded: &[f64], cmp: VecCmp) -> TriMaskResult {
    // Extract the threshold from the VecCmp so we can test for the tie condition.
    // This is the f64 constant the filter compares against.
    let c = match cmp {
        VecCmp::Gt(t) | VecCmp::Ge(t) | VecCmp::Lt(t) | VecCmp::Le(t) | VecCmp::Eq(t) => t,
    };

    let mut confident_passes: Vec<usize> = Vec::new();
    let mut delegated: Vec<usize> = Vec::new();

    for (i, &x) in decoded.iter().enumerate() {
        if x.is_nan() {
            // Unknown: NaN sentinel (non-numeric term, unbound, local-vocab id, genuine NaN).
            // Delegate to scalar predicate.
            delegated.push(i);
        } else if x == c {
            // Tie: f64-equal to the constant (also catches +0.0 vs −0.0 since IEEE-754 ==
            // treats them as equal). The exact comparison could go any way; delegate.
            delegated.push(i);
        } else {
            // Confident: `x` is finite and `x != c`. By the monotone-rounding lemma
            // (§3 of the completion-design record), the f64 verdict is unambiguous.
            if cmp.test(x) {
                confident_passes.push(i);
            }
            // Confident fail: not added to either list.
        }
    }

    TriMaskResult { confident_passes, delegated }
}

/// Merges two ascending index lists into a single ascending list (standard merge step).
///
/// Used to combine confident-passes and delegated-passes into the final morsel-local
/// selection vector. Both inputs must be strictly ascending (indices within a morsel
/// are always distinct and generated in index order, so this holds by construction).
pub(crate) fn merge_ascending(a: &[usize], b: &[usize]) -> Vec<usize> {
    let mut result: Vec<usize> = Vec::with_capacity(a.len() + b.len());
    let (mut ai, mut bi) = (0usize, 0usize);
    while ai < a.len() && bi < b.len() {
        if a[ai] <= b[bi] {
            result.push(a[ai]);
            ai += 1;
        } else {
            result.push(b[bi]);
            bi += 1;
        }
    }
    result.extend_from_slice(&a[ai..]);
    result.extend_from_slice(&b[bi..]);
    result
}

// ---- Unit tests --------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chunk::VecCmp;

    /// `tri_mask_select` over a column with no NaN and no tie: every lane is either a
    /// confident pass or a confident fail. Delegated list must be empty. [SONNET-4.6]
    #[test]
    fn no_nan_no_tie_all_confident() {
        let decoded = [1.0f64, 3.0, 5.0, 7.0, 9.0];
        // FILTER(?x > 4.0): 5.0, 7.0, 9.0 pass (indices 2, 3, 4); no ties, no NaN.
        let r = tri_mask_select(&decoded, VecCmp::Gt(4.0));
        assert_eq!(r.confident_passes, vec![2, 3, 4]);
        assert!(r.delegated.is_empty(), "no NaN or tie => delegated must be empty");
    }

    /// NaN sentinel lanes are classified Unknown and go into `delegated`, never
    /// into `confident_passes`. [SONNET-4.6]
    #[test]
    fn nan_lanes_go_to_delegated() {
        let decoded = [1.0f64, f64::NAN, 5.0, f64::NAN, 9.0];
        let r = tri_mask_select(&decoded, VecCmp::Ge(0.0));
        // Confident passes: 1.0, 5.0, 9.0 (indices 0, 2, 4)
        assert_eq!(r.confident_passes, vec![0, 2, 4]);
        // Delegated: NaN at indices 1 and 3
        assert_eq!(r.delegated, vec![1, 3]);
    }

    /// Tie lanes (lane value == constant as f64) go into `delegated`, even when the
    /// comparison `cmp.test(x)` would return true for them (e.g. `Le` or `Eq`). The
    /// exact comparison must be delegated to the scalar path. [SONNET-4.6]
    #[test]
    fn tie_lanes_go_to_delegated() {
        // Threshold is 5.0; lanes with value 5.0 are ties.
        let decoded = [1.0f64, 5.0, 9.0];
        // FILTER(?x <= 5.0): 1.0 is confident pass; 5.0 is a tie; 9.0 is confident fail.
        let r = tri_mask_select(&decoded, VecCmp::Le(5.0));
        assert_eq!(r.confident_passes, vec![0]);
        assert_eq!(r.delegated, vec![1], "tie lane must go to delegated");
    }

    /// For `Eq` operator: non-tie lanes whose value != c are confident fails (not in
    /// confident_passes); the tie lane is delegated. [SONNET-4.6]
    #[test]
    fn eq_operator_tie_is_delegated_nonties_are_confident_fail() {
        let decoded = [1.0f64, 5.0, 9.0];
        // FILTER(?x = 5.0): only the tie (5.0) is delegated; others are confident fails.
        let r = tri_mask_select(&decoded, VecCmp::Eq(5.0));
        assert!(r.confident_passes.is_empty(), "x=1 and x=9 are confident fails for Eq(5)");
        assert_eq!(r.delegated, vec![1]);
    }

    /// `+0.0 == -0.0` under IEEE-754 means a lane value of +0.0 is a tie when the
    /// constant is -0.0 (and vice versa). Both must go to delegated. [SONNET-4.6]
    #[test]
    fn pos_neg_zero_are_ties() {
        let decoded = [1.0f64, 0.0f64]; // +0.0
        let r = tri_mask_select(&decoded, VecCmp::Lt(-0.0f64)); // constant is -0.0
        // 1.0 > -0.0, not a tie, confident fail for Lt(-0)
        assert!(r.confident_passes.is_empty());
        // +0.0 == -0.0, so index 1 is a tie -> delegated
        assert_eq!(r.delegated, vec![1]);
    }

    /// `merge_ascending` of two ascending lists produces one ascending list. [SONNET-4.6]
    #[test]
    fn merge_ascending_interleaves_correctly() {
        assert_eq!(merge_ascending(&[0, 3, 6], &[1, 2, 5]), vec![0, 1, 2, 3, 5, 6]);
        assert_eq!(merge_ascending(&[], &[1, 2, 3]), vec![1, 2, 3]);
        assert_eq!(merge_ascending(&[1, 2, 3], &[]), vec![1, 2, 3]);
        // Both-empty case: annotate types so the compiler can resolve the generic. [SONNET-4.6]
        let e: [usize; 0] = [];
        assert_eq!(merge_ascending(&e, &e), Vec::<usize>::new());
    }

    /// A column with mixed NaN, tie, and confident lanes round-trips through the full
    /// tri-mask classification correctly. [SONNET-4.6]
    #[test]
    fn mixed_column_full_classification() {
        // Threshold = 5.0
        // Lane 0: 1.0  → confident fail (Ge(5.0): 1.0 < 5.0)
        // Lane 1: NaN  → unknown/delegated
        // Lane 2: 5.0  → tie/delegated
        // Lane 3: 7.0  → confident pass (7.0 >= 5.0)
        // Lane 4: NaN  → unknown/delegated
        // Lane 5: 9.0  → confident pass
        let decoded = [1.0f64, f64::NAN, 5.0, 7.0, f64::NAN, 9.0];
        let r = tri_mask_select(&decoded, VecCmp::Ge(5.0));
        assert_eq!(r.confident_passes, vec![3, 5]);
        assert_eq!(r.delegated, vec![1, 2, 4]);
    }

    /// The tie-exactness case from §3 T2 of the completion-design record:
    /// `v = 9007199254740992` and `c = 9007199254740993` are distinct integers but
    /// both decode to the same f64 (2^53 is the last integer exactly representable;
    /// 2^53+1 rounds to 2^53). With `FILTER(?x < c)`, `v < c` exactly, so the row
    /// SHOULD survive. A naive pure-f64 path would drop it (confident fail for Lt).
    /// The tri-mask must classify this as a TIE (delegated), not a confident fail.
    ///
    /// After scalar delegation with the exact-lexical recheck, the row survives.
    ///
    /// This test proves the classification kernel is correct; the end-to-end survival
    /// is asserted by `T2_tie_exactness_witness` in the integration test file. [SONNET-4.6]
    #[test]
    fn tie_exactness_2_to_53_classified_as_tie() {
        // 2^53 = 9007199254740992 (the lane value)
        // 2^53 + 1 = 9007199254740993 (the constant)
        // Both parse to the same f64 value: 9007199254740992.0
        let v_f64 = 9007199254740992_f64;
        let c_f64 = 9007199254740993_f64;
        // Confirm they parse to the same f64 (the tie precondition):
        assert_eq!(
            v_f64, c_f64,
            "9007199254740992 and 9007199254740993 must share the same f64 (tie precondition)"
        );
        let decoded = [v_f64];
        // FILTER(?x < c): c_f64 == v_f64, so this is a TIE, not a confident fail.
        let r = tri_mask_select(&decoded, VecCmp::Lt(c_f64));
        assert!(
            r.confident_passes.is_empty(),
            "2^53 lane vs 2^53+1 constant must be a TIE, not a confident pass or fail"
        );
        assert_eq!(r.delegated, vec![0], "tie must go to delegated for scalar exact check");
    }
}
