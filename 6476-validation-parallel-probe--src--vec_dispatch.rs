//! [SONNET-4.6] (sq-pntvh.5) Columnar dispatch gate: the `VEC_MIN_BATCH` / `VEC_MORSEL`
//! constants and the I5 probe counters that every M4 seam shares.
//!
//! This module is the **single owner** of the decline hierarchy (I1–I5) described in
//! `research/vector-at-a-time-m4-completion-design.md` §4. Every current and future
//! seam (filter, aggregate, project, join) **inherits** it here by construction rather
//! than re-implementing per-seam discipline.
//!
//! ## Constants (placeholders — EC2-tuned in `sq-pntvh.9`)
//!
//! - `VEC_MIN_BATCH` (256): minimum batch size for the columnar path. Batches smaller
//!   than this are typically cheaper on the scalar row path (transpose overhead).
//! - `VEC_MORSEL` (2048): decode-buffer length for morsel-by-morsel `apply_filter`
//!   execution. Each morsel decodes only the filter column, not the full row width.
//!
//! ## I5 probe counters (unstable / test-facing)
//!
//! Thread-local counters `{chunks_built, rows_columnar, rows_delegated,
//! declines_by_reason}` that the test suite asserts to confirm the seams are
//! non-vacuous (see `tests/differentials/vectorized_byte_identity.rs`). They are
//! compiled out entirely when the `vectorized` feature is OFF and are **NOT
//! semver-stable** — callers outside the acceptance-test suite must not build stable
//! logic over them.
//!
//! Thread-locals are used (not global atomics) so each test thread gets isolated
//! counters — parallel `#[test]` invocations cannot interfere. The query evaluator
//! calls the seams from the installing thread, so thread-local counters capture the
//! full per-query activity correctly.
//!
//! This module is only compiled when `feature = "vectorized"` is active; the whole
//! file is a `#[cfg(feature = "vectorized")]` compilation unit (registered in
//! `lib.rs` under the same gate). When the feature is OFF, zero code from here
//! compiles and the default native + wasm builds are byte-identical.

use std::cell::Cell;

// ---- Morsel constants -------------------------------------------------------
// Placeholders: EC2-tuned in sq-pntvh.9. NO perf claim attaches to these values.
// They are referenced from the two exec.rs seam regions; putting them here ensures
// every seam uses the same constants and they only need to be changed in one place.

/// Minimum batch size (row count) for the columnar path.
///
/// Batches with fewer than `VEC_MIN_BATCH` rows are processed by the scalar row path:
/// the per-column transpose overhead is a net loss on small intermediates, and the
/// batch-size eligibility check (I4 per the design record §1) sits at this threshold.
///
/// **Placeholder value** (256) — to be EC2-measured and adjusted in `sq-pntvh.9`.
/// No performance claim is implied by this number.
pub(crate) const VEC_MIN_BATCH: usize = 256;

/// Morsel length: the number of rows the `apply_filter` seam decodes at once.
///
/// Each morsel extracts only the single filter column into a contiguous `Vec<f64>`
/// buffer (O(morsel × 1), not a full-width transpose). This keeps the decode buffer
/// in L1/L2 across the decode→compare→gather pipeline step.
///
/// **Placeholder value** (2048) — to be EC2-measured and adjusted in `sq-pntvh.9`.
/// No performance claim is implied by this number.
pub(crate) const VEC_MORSEL: usize = 2048;

// ---- I5 probe counters (thread-local) ---------------------------------------
//
// Thread-local so parallel test threads do not interfere with each other.
// The query evaluator calls `columnar_filter` / `columnar_aggregate` from the
// thread that runs the query, so thread-local capture is correct for queries.

thread_local! {
    /// DataChunk morsels built and run through a columnar kernel.
    static TL_CHUNKS_BUILT: Cell<u64> = const { Cell::new(0) };
    /// Total rows processed on the columnar path.
    static TL_ROWS_COLUMNAR: Cell<u64> = const { Cell::new(0) };
    /// Rows delegated to the scalar predicate (reserved for sq-y5ew5; always 0 in Phase 5).
    static TL_ROWS_DELEGATED: Cell<u64> = const { Cell::new(0) };
    /// Operator invocations that declined the columnar path (all reasons).
    static TL_DECLINES: Cell<u64> = const { Cell::new(0) };
}

// ---- Public (unstable/test-facing) API --------------------------------------

/// A snapshot of the I5 probe counters for the calling thread.
///
/// **Unstable / test-facing** — NOT semver-stable. Only the acceptance tests
/// (`tests/differentials/vectorized_byte_identity.rs`) should assert on these values.
/// External callers must not build stable logic over them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VecStats {
    /// Number of DataChunk morsels built and processed by a columnar kernel.
    ///
    /// The acceptance gate asserts `chunks_built >= 1` for an eligible operator
    /// invocation to confirm the seam is non-vacuous.
    pub chunks_built: u64,
    /// Total rows processed on the columnar path (sum over all morsel sizes).
    pub rows_columnar: u64,
    /// Rows delegated to the scalar predicate (reserved for `sq-y5ew5`; 0 in Phase 5).
    pub rows_delegated: u64,
    /// Total operator invocations that declined the columnar path (all reasons).
    pub declines_by_reason: u64,
}

/// Resets all I5 probe counters to zero for the calling thread.
///
/// **Unstable / test-facing.** Call before each test that asserts on [`stats_snapshot`].
pub fn reset_stats() {
    TL_CHUNKS_BUILT.with(|c| c.set(0));
    TL_ROWS_COLUMNAR.with(|c| c.set(0));
    TL_ROWS_DELEGATED.with(|c| c.set(0));
    TL_DECLINES.with(|c| c.set(0));
}

/// Returns a point-in-time snapshot of the I5 probe counters for the calling thread.
///
/// **Unstable / test-facing.** See [`VecStats`] for field semantics and the
/// non-vacuity contract each acceptance test pins.
pub fn stats_snapshot() -> VecStats {
    VecStats {
        chunks_built: TL_CHUNKS_BUILT.with(|c| c.get()),
        rows_columnar: TL_ROWS_COLUMNAR.with(|c| c.get()),
        rows_delegated: TL_ROWS_DELEGATED.with(|c| c.get()),
        declines_by_reason: TL_DECLINES.with(|c| c.get()),
    }
}

// ---- Internal counter increments --------------------------------------------

/// Records one morsel of `morsel_rows` rows being processed on the columnar path.
/// Called once per morsel in the `apply_filter` seam, and once per aggregate invocation
/// in the `group_aggregate` seam.
#[inline]
pub(crate) fn record_chunk(morsel_rows: usize) {
    TL_CHUNKS_BUILT.with(|c| c.set(c.get() + 1));
    TL_ROWS_COLUMNAR.with(|c| c.set(c.get() + morsel_rows as u64));
}

/// Records `n` rows delegated to the scalar predicate in the hybrid tri-mask FILTER
/// (sq-y5ew5). Called once per morsel by `columnar_filter` after the tri-mask pass.
/// [SONNET-4.6]
#[inline]
pub(crate) fn record_delegated(n: usize) {
    TL_ROWS_DELEGATED.with(|c| c.set(c.get() + n as u64));
}

/// Records one operator invocation that declined the columnar path.
#[inline]
pub(crate) fn record_decline() {
    TL_DECLINES.with(|c| c.set(c.get() + 1));
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Each public function must compile and work: basic increment + snapshot round-trip. [SONNET-4.6]
    #[test]
    fn counters_round_trip() {
        reset_stats();
        let snap0 = stats_snapshot();
        assert_eq!(snap0.chunks_built, 0);
        assert_eq!(snap0.rows_columnar, 0);
        assert_eq!(snap0.rows_delegated, 0);
        assert_eq!(snap0.declines_by_reason, 0);

        record_chunk(100);
        record_chunk(50);
        record_decline();

        let snap1 = stats_snapshot();
        assert_eq!(snap1.chunks_built, 2, "two record_chunk calls");
        assert_eq!(snap1.rows_columnar, 150, "100+50 rows");
        assert_eq!(snap1.declines_by_reason, 1);
        assert_eq!(snap1.rows_delegated, 0, "always 0 in Phase 5");

        reset_stats();
        let snap2 = stats_snapshot();
        assert_eq!(snap2.chunks_built, 0, "reset clears all counters");
    }
}
