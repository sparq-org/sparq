//! Columnar aggregate reducer kernels for the M4 Phase 4 seam (`sq-pntvh.4`). [SONNET-4.6]
//!
//! These operate over gathered per-group id slices (all-inline-integer gate):
//! exact integer arithmetic, gather-free value decoding via the inline encoding.
//! Byte-identity with the scalar `eval_aggregate` path is proven per-reducer — see
//! the per-function doc comments. Used only under the `vectorized` feature.

use sparq_core::dict::{is_inline, Id, INLINE_BASE, NO_ID};

/// The maximum value encodable by an inline integer id.
/// `id - INLINE_BASE ∈ [0, INLINE_MAX_I64]` for every inline id.
pub(crate) const INLINE_MAX_I64: i64 = (1_u64 << 30) as i64 - 1;

/// Exact i128 SUM over an all-inline-integer id slice.
///
/// Returns `None` if any id is non-inline (the caller should decline to the scalar path).
/// Returns `Some(0)` for an empty slice (the identity element, matching the empty-SUM
/// SPARQL rule `SUM({}) = 0`).
///
/// **Bound argument:** per-element values are bounded by `INLINE_MAX_I64` (< 2^30), so
/// the i128 accumulation is exact with no risk of i128 overflow. However, group
/// cardinality is NOT bounded by dictionary capacity — the same inline value can repeat
/// arbitrarily many times across rows — so the i128 sum can exceed `i64::MAX`. The
/// caller (`columnar_aggregate`) guards the narrowing with [`narrow_sum_to_i64`] and
/// declines to the scalar path on overflow, ensuring no silent truncation in release
/// builds. [SONNET-4.6]
pub(crate) fn reduce_sum(ids: &[Id]) -> Option<i128> {
    let mut acc: i128 = 0;
    for &id in ids {
        if !is_inline(id) {
            return None;
        }
        // Each inline integer value is in [0, INLINE_MAX_I64] (load-bearing bound).
        debug_assert!((id - INLINE_BASE) as i64 <= INLINE_MAX_I64);
        acc += i128::from(id - INLINE_BASE);
    }
    Some(acc)
}

/// Narrow an exact i128 SUM result to `i64`, returning `None` on overflow.
///
/// `columnar_aggregate` calls this instead of `sum_i128 as i64` (which silently
/// wraps in release builds). On `None` the caller declines to the scalar path, which
/// promotes through `Num::Double` — the same value and datatype the scalar
/// `sum_values` / `Num::Int::checked_add` fall-through produces. [SONNET-4.6]
///
/// Extracted as a separate `pub(crate)` function so the overflow-decline invariant
/// is unit-testable without constructing a group large enough to overflow i64 at the
/// caller level (which would require an unbounded number of rows in a test).
pub(crate) fn narrow_sum_to_i64(sum: i128) -> Option<i64> {
    i64::try_from(sum).ok()
}

/// COUNT of non-`NO_ID` ids in the slice.
///
/// For an all-inline-integer group every id is non-`NO_ID` (since `NO_ID = 0 < INLINE_BASE`),
/// so this equals `ids.len()` in that case. Provided as a generic count primitive for
/// callers that do not need the all-inline guard.
///
/// **Byte-identity:** the scalar `CountSolutions { distinct: false }` returns
/// `Value::Num(Num::Int(members.len() as i64))`. For an all-inline group every member is
/// bound, so `reduce_count(group_ids) == members.len()`.
pub(crate) fn reduce_count(ids: &[Id]) -> usize {
    ids.iter().filter(|&&id| id != NO_ID).count()
}

/// MIN inline id in the slice (= MIN value, since value = id − INLINE_BASE and the base
/// is the same for every inline id).
///
/// Returns `None` if the slice is empty OR if any id is non-inline. The caller should:
/// * for an EMPTY result — treat as unbound (`NO_ID`), matching `minmax_values([], _) == Value::Unbound`;
/// * for a non-empty `None` — decline to the scalar path (non-inline id present).
///
/// **Byte-identity:** for an all-inline-integer group the scalar `minmax_values` path
/// folds `Num::Int(v)` values; the minimum value maps to the minimum id (monotone offset).
/// `value_to_id(graph, local, &Value::Num(Num::Int(v_min)))` equals the id this function
/// returns, because `inline_id_of_int(v_min) = INLINE_BASE + v_min` and
/// `min id = INLINE_BASE + min value`.
pub(crate) fn reduce_min_id(ids: &[Id]) -> Option<Id> {
    if ids.is_empty() {
        return None;
    }
    let mut best: Id = Id::MAX;
    for &id in ids {
        if !is_inline(id) {
            return None;
        }
        if id < best {
            best = id;
        }
    }
    Some(best)
}

/// MAX inline id in the slice (= MAX value). Returns `None` if the slice is empty or
/// any id is non-inline (see `reduce_min_id` for the caller's responsibility).
///
/// **Byte-identity:** symmetric to `reduce_min_id`.
pub(crate) fn reduce_max_id(ids: &[Id]) -> Option<Id> {
    if ids.is_empty() {
        return None;
    }
    let mut best: Id = 0;
    let mut found = false;
    for &id in ids {
        if !is_inline(id) {
            return None;
        }
        if !found || id > best {
            best = id;
            found = true;
        }
    }
    if found { Some(best) } else { None }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mk_inline(v: u32) -> Id {
        INLINE_BASE + v
    }

    /// T1: Exactness (mutation-proof). Verifies the exact i128 result for a known set of
    /// inline ids. If someone changes the accumulator to `f64`, this test will fail to
    /// compile when the return type changes, OR fail at runtime when large-value sums
    /// exceed f64 precision (sum > 2^53, achievable with many max-value rows in production
    /// — see the overflow bound in `reduce_sum`'s doc comment). Pinning the exact i128
    /// value is the soundest mutation kill short of runtime f64-divergence at this scale.
    #[test]
    fn t1_reduce_sum_exact_i128_result() {
        // Three distinct inline values; sum is exact and pinned.
        // 1_000_000_000 + 73_741_823 + 500_000 = 1_074_241_823
        let ids = vec![mk_inline(1_000_000_000), mk_inline(73_741_823), mk_inline(500_000)];
        let sum = reduce_sum(&ids).unwrap();
        assert_eq!(sum, 1_074_241_823_i128, "exact i128 sum must equal 1_074_241_823");

        // Two max-inline values: 2 * (2^30 - 1) = 2^31 - 2 = 2_147_483_646
        let v_max = INLINE_MAX_I64 as u32;
        let ids2 = vec![mk_inline(v_max), mk_inline(v_max)];
        let sum2 = reduce_sum(&ids2).unwrap();
        let expected2: i128 = 2 * (INLINE_MAX_I64 as i128);
        assert_eq!(sum2, expected2, "two max-inline values must sum to 2*(2^30-1)");

        // Empty sum = 0 (identity).
        assert_eq!(reduce_sum(&[]), Some(0_i128), "empty sum must be 0");
    }

    /// T2: Min/max tie-freeness: multiple rows with the same inline id — both reducers
    /// return that same id (no ambiguity, no sentinel confusion).
    #[test]
    fn t2_min_max_tie_free() {
        let id = mk_inline(42);
        let ids = vec![id, id, id];
        assert_eq!(reduce_min_id(&ids), Some(id));
        assert_eq!(reduce_max_id(&ids), Some(id));
    }

    /// T2b: Min/max order correctness across a spread of values.
    #[test]
    fn t2b_min_max_spread() {
        let ids = vec![mk_inline(100), mk_inline(1), mk_inline(50), mk_inline(200), mk_inline(3)];
        assert_eq!(reduce_min_id(&ids), Some(mk_inline(1)));
        assert_eq!(reduce_max_id(&ids), Some(mk_inline(200)));
        // Mutation check: a MIN returning MAX would fail.
        assert_ne!(reduce_min_id(&ids), reduce_max_id(&ids), "MIN != MAX over spread");
    }

    /// T3: Empty / non-inline decline.
    #[test]
    fn t3_empty_and_non_inline_decline() {
        // Empty: sum = Some(0), min/max = None.
        assert_eq!(reduce_sum(&[]), Some(0_i128));
        assert_eq!(reduce_min_id(&[]), None);
        assert_eq!(reduce_max_id(&[]), None);

        // Non-inline dict id (id = 1 < INLINE_BASE).
        let non_inline: Id = 1;
        assert!(!is_inline(non_inline), "id=1 must be non-inline (dict id)");

        assert_eq!(reduce_sum(&[mk_inline(5), non_inline]), None);
        assert_eq!(reduce_min_id(&[mk_inline(5), non_inline]), None);
        assert_eq!(reduce_max_id(&[mk_inline(5), non_inline]), None);

        // NO_ID (= 0) is also non-inline.
        let no_id: Id = NO_ID;
        assert!(!is_inline(no_id));
        assert_eq!(reduce_sum(&[mk_inline(7), no_id]), None);
    }

    /// T3b: `narrow_sum_to_i64` declines (returns `None`) for values exceeding `i64::MAX`
    /// and succeeds for values within range. [SONNET-4.6]
    ///
    /// Mutation proof: restoring `as i64` gives `Some(-9223372036854775808)` for
    /// `just_over`, so the `assert_eq!(…, None)` fails — the guard compiles out on
    /// `as i64` exactly like the old `debug_assert!` did.
    #[test]
    fn t3b_narrow_overflow_decline() {
        let just_over: i128 = i64::MAX as i128 + 1;
        assert_eq!(
            narrow_sum_to_i64(just_over),
            None,
            "i64::MAX + 1 must decline (overflow guard)"
        );
        assert_eq!(
            narrow_sum_to_i64(i64::MAX as i128),
            Some(i64::MAX),
            "i64::MAX itself must succeed"
        );
        assert_eq!(narrow_sum_to_i64(0), Some(0));
        assert_eq!(narrow_sum_to_i64(190), Some(190), "typical small SUM must succeed");
        // Negative sums can arise if the caller somehow passes a negative i128.
        // We do not currently generate these (all inline values are non-negative),
        // but try_from is still defined: any i128 in [i64::MIN, i64::MAX] succeeds.
        assert_eq!(narrow_sum_to_i64(-1_i128), Some(-1));
        assert_eq!(narrow_sum_to_i64(i64::MIN as i128), Some(i64::MIN));
    }

    /// T4: reduce_count counts non-NO_ID ids.
    #[test]
    fn t4_reduce_count() {
        let ids = vec![mk_inline(1), mk_inline(2), mk_inline(3)];
        assert_eq!(reduce_count(&ids), 3);

        // With NO_ID: only 2 of 3 are non-NO_ID.
        let ids_with_noid = vec![mk_inline(1), NO_ID, mk_inline(3)];
        assert_eq!(reduce_count(&ids_with_noid), 2);

        // Empty.
        assert_eq!(reduce_count(&[]), 0);

        // All NO_ID.
        assert_eq!(reduce_count(&[NO_ID, NO_ID]), 0);
    }
}

// [SONNET-4.6] (sq-sqtk2.3) Kani result-equivalence harnesses for the columnar reducer
// kernels (SUM/COUNT/MIN/MAX + narrow). Each harness checks that the REAL implementation
// function (reduce_sum / reduce_count / reduce_min_id / reduce_max_id / narrow_sum_to_i64)
// equals an INDEPENDENT in-harness reference written as a plain scalar fold — the reference
// intentionally does NOT call the columnar code to produce the expected value (that would be
// vacuously true), and it does NOT call the exec.rs scalar path (to keep the TCB small).
//
// TIER: PROVED (bounded, slice length ≤ MAX_SLICE = 8; element values fully symbolic in
// their stated domains). Kani/CBMC explores ALL inputs within the stated bounds exhaustively.
// TCB: Kani/CBMC + slice-length bound + in-harness reference-as-spec.
//
// STATED BOUNDS (document honestly; "proved beyond bound" means extrapolated by induction
// on the loop body, not model-checked):
//   * Slice length: 0..=MAX_SLICE = 8 (the same element count per probe, not per-group row
//     count at query time — production groups can be larger; 8 is a pragmatic Kani bound,
//     not a completeness claim: behavior beyond this length is not model-checked — for
//     example, reduce_sum's i128 accumulation can exceed i64::MAX at large cardinality). [SONNET-4.6]
//   * reduce_sum / reduce_min_id / reduce_max_id: each element value is fully symbolic in
//     [0, INLINE_MAX_I64] (= [0, 2^30 - 1]); the id encoding is id = INLINE_BASE + value.
//   * reduce_count: each element is fully symbolic over the entire u32 domain (no assume on
//     values; the count only tests id != NO_ID which covers all three partitions).
//   * narrow_sum_to_i64: the input `sum: i128` is FULLY SYMBOLIC over the complete i128
//     domain — no assume, no loop, proved without a bound (complete domain).
//
// The `kani` cfg is registered in crates/sparq-engine/Cargo.toml [lints.rust] so
// `-D warnings` (unexpected_cfgs) is clean on the normal build. This module compiles ONLY
// under `cargo kani -p sparq-engine --features vectorized` (Kani injects `cfg(kani)` itself);
// under all other builds it is stripped to zero bytes — the default build and wasm bundle are
// byte-identical to before this bead.
//
// NON-VACUITY (documented): locally perturbing one accumulation step — e.g. changing
// `acc += i128::from(id - INLINE_BASE)` to `acc += 1` in `reduce_sum` — causes
// `reduce_sum_equals_reference_fold` to go RED (counterexample: any non-unit value in the
// slice). This mutation spot-check is documented in the PR body per the bead requirement.
#[cfg(all(kani, feature = "vectorized"))]
mod kani_proofs {
    use super::*;
    use sparq_core::dict::{is_inline, Id, INLINE_BASE, NO_ID};

    /// Maximum slice length for the bounded proofs. CBMC symbolic exploration is exponential
    /// in slice length, so 8 is chosen as a pragmatic bound; properties are verified
    /// exhaustively up to this length and extrapolated by induction beyond it, but
    /// behavior beyond MAX_SLICE is not model-checked. [SONNET-4.6]
    const MAX_SLICE: usize = 8;

    /// Build a symbolic Vec of `len` inline ids, each constrained to
    /// `[INLINE_BASE, INLINE_BASE + INLINE_MAX_I64]` so every element is a valid inline
    /// integer id. Values (offsets from INLINE_BASE) are fully symbolic in [0, INLINE_MAX_I64].
    fn symbolic_inline_ids(len: usize) -> Vec<Id> {
        let mut v = vec![0u32; len];
        for id in v.iter_mut() {
            let raw: u32 = kani::any();
            kani::assume(raw <= INLINE_MAX_I64 as u32);
            *id = INLINE_BASE + raw;
        }
        v
    }

    /// PROPERTY: `reduce_sum` on an all-inline slice equals the independent scalar reference
    /// fold sum-of-decoded-values over the same slice.
    ///
    /// The reference is written fresh here — it does NOT call `reduce_sum` (no vacuity) and
    /// does NOT call the exec.rs scalar path (to keep TCB minimal).
    ///
    /// Bound: slice length 0..=MAX_SLICE = 8; values symbolic in [0, INLINE_MAX_I64].
    #[kani::proof]
    #[kani::unwind(12)]
    fn reduce_sum_equals_reference_fold() {
        let len: usize = kani::any();
        kani::assume(len <= MAX_SLICE);
        let ids = symbolic_inline_ids(len);

        // Independent reference: a fresh scalar fold over decoded values.
        let mut ref_sum: i128 = 0;
        for &id in ids.iter() {
            // is_inline holds by construction; assert defensively so Kani can confirm it.
            assert!(is_inline(id), "all ids must be inline by construction");
            ref_sum += i128::from(id - INLINE_BASE);
        }

        // The implementation must return Some(ref_sum) for any all-inline slice.
        assert_eq!(reduce_sum(&ids), Some(ref_sum));
    }

    /// PROPERTY: `reduce_count` on any id slice (symbolic over the full u32 domain) equals
    /// the independent scalar reference count of non-NO_ID elements.
    ///
    /// Bound: slice length 0..=MAX_SLICE = 8; element values symbolic over all of u32.
    #[kani::proof]
    #[kani::unwind(12)]
    fn reduce_count_equals_reference() {
        let len: usize = kani::any();
        kani::assume(len <= MAX_SLICE);
        let mut ids = vec![0u32; len];
        for id in ids.iter_mut() {
            *id = kani::any(); // fully symbolic: inline, dict id, or NO_ID
        }

        // Independent reference: count non-NO_ID elements.
        let mut ref_count: usize = 0;
        for &id in ids.iter() {
            if id != NO_ID {
                ref_count += 1;
            }
        }

        assert_eq!(reduce_count(&ids), ref_count);
    }

    /// PROPERTY: `reduce_min_id` on a non-empty all-inline slice returns the id with the
    /// minimum decoded value (equivalently, the minimum id, since all share the same
    /// INLINE_BASE offset and the mapping id → value is strictly monotone).
    ///
    /// Bound: slice length 1..=MAX_SLICE = 8; values symbolic in [0, INLINE_MAX_I64].
    #[kani::proof]
    #[kani::unwind(12)]
    fn reduce_min_id_equals_reference() {
        let len: usize = kani::any();
        kani::assume(len >= 1 && len <= MAX_SLICE);
        let ids = symbolic_inline_ids(len);

        // Independent reference: linear scan for the minimum id.
        // Initialise to u32::MAX (> any inline id: max inline id =
        // INLINE_BASE + INLINE_MAX_I64 = 3_221_225_471 < u32::MAX = 4_294_967_295).
        let mut ref_min: Id = u32::MAX;
        for &id in ids.iter() {
            if id < ref_min {
                ref_min = id;
            }
        }

        assert_eq!(reduce_min_id(&ids), Some(ref_min));
    }

    /// PROPERTY: `reduce_max_id` on a non-empty all-inline slice returns the id with the
    /// maximum decoded value (equivalently, the maximum id).
    ///
    /// Bound: slice length 1..=MAX_SLICE = 8; values symbolic in [0, INLINE_MAX_I64].
    #[kani::proof]
    #[kani::unwind(12)]
    fn reduce_max_id_equals_reference() {
        let len: usize = kani::any();
        kani::assume(len >= 1 && len <= MAX_SLICE);
        let ids = symbolic_inline_ids(len);

        // Independent reference: linear scan for the maximum id.
        // Initialise to 0 (= NO_ID < INLINE_BASE <= any inline id), updated on first element.
        let mut ref_max: Id = 0;
        for &id in ids.iter() {
            if id > ref_max {
                ref_max = id;
            }
        }

        assert_eq!(reduce_max_id(&ids), Some(ref_max));
    }

    /// PROPERTY: `narrow_sum_to_i64(s)` returns `Some(s as i64)` exactly when
    /// `s ∈ [i64::MIN, i64::MAX]`, and `None` for every value outside that range.
    ///
    /// TIER: PROVED over the COMPLETE i128 domain — no slice, no loop, no unwind bound;
    /// CBMC handles the 128-bit integer comparison exhaustively.
    #[kani::proof]
    fn narrow_sum_to_i64_iff_in_range() {
        let s: i128 = kani::any(); // fully symbolic, no assume — complete i128 domain
        let result = narrow_sum_to_i64(s);
        if s >= i64::MIN as i128 && s <= i64::MAX as i128 {
            assert_eq!(result, Some(s as i64));
        } else {
            assert_eq!(result, None);
        }
    }

    /// DOMAIN-COVERAGE SELF-CHECK (the sq-og8u8 anti-vacuity pattern). Lesson of sq-sqtk2.1
    /// (2026-07-04): a too-tight `#[kani::unwind]` on an `assume`-bounded generator can
    /// SILENTLY `assume(false)`-prune the very inputs a harness means to cover, so the harness
    /// passes VACUOUSLY while reporting nothing wrong (there, the 32–39-byte security-relevant
    /// principals were pruned; a mutation spot-check cannot catch it — the mutant is pruned
    /// too). Every harness suite must therefore ship a self-check proving its interesting
    /// inputs genuinely survive the bounds. Two obligations here:
    ///
    ///   1. **The reducer value domain is genuinely ADVERSARIAL** — `MAX_SLICE >= 2` (a
    ///      multi-element fold, not a degenerate singleton), `INLINE_MAX_I64 > 0` (a
    ///      non-trivial value range, so `min != max` and a multi-term sum are reachable), and
    ///      the id encoding classes correctly at BOTH ends of the inline range (`is_inline`
    ///      on `INLINE_BASE` and on `INLINE_BASE + INLINE_MAX_I64`) but NOT on the `NO_ID`
    ///      sentinel (`reduce_count` keys on `id != NO_ID`). These are concrete assertions
    ///      over the domain constants — no loop, no unwind, so nothing here can itself be
    ///      pruned; they go red if a future re-scope collapses the value/id domain.
    ///
    ///   2. **The MAXIMAL-length slice SURVIVES the `unwind(12)` bound** — a `kani::cover!`
    ///      that a full `len == MAX_SLICE` slice with two DISTINCT endpoint values is
    ///      reachable. If a future bound tightening pruned the full-length varied slice (the
    ///      sq-sqtk2.1 failure mode), this cover becomes UNREACHABLE and goes red, rather than
    ///      the equivalence harnesses silently proving the reducers ≡ a reference over only
    ///      the short slices the bound still admits.
    ///
    /// Cost: one small harness at the same `unwind(12)` the reducer harnesses use. [OPUS-4.8]
    #[kani::proof]
    #[kani::unwind(12)]
    fn domain_reducer_slice_is_adversarial_and_survives_the_bound() {
        // (1) The domain constants are adversarial (concrete — cannot be unwind-pruned).
        assert!(MAX_SLICE >= 2, "a singleton domain would not exercise the multi-element fold");
        assert!(INLINE_MAX_I64 > 0, "a zero-width value range makes min == max == sum trivial");
        assert!(INLINE_BASE > NO_ID, "the inline region must sit above the NO_ID sentinel");
        assert!(is_inline(INLINE_BASE), "INLINE_BASE is the low end of the inline region");
        assert!(
            is_inline(INLINE_BASE + INLINE_MAX_I64 as u32),
            "INLINE_BASE + INLINE_MAX_I64 is the high end of the inline region"
        );
        assert!(!is_inline(NO_ID), "NO_ID must not be classed inline (reduce_count keys on it)");

        // (2) The maximal-length, value-varied slice SURVIVES unwind(12): a full MAX_SLICE
        // slice whose endpoints can differ is reachable, i.e. NOT assume(false)-pruned.
        let ids = symbolic_inline_ids(MAX_SLICE);
        kani::cover!(
            ids[0] != ids[MAX_SLICE - 1],
            "a full-length slice with distinct endpoint values survives the unwind bound"
        );
    }
}
