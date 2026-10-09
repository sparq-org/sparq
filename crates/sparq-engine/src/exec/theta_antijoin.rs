use std::cell::Cell;

thread_local! {
    static ENABLED: Cell<bool> = const { Cell::new(true) };
    static FIRED: Cell<bool> = const { Cell::new(false) };
    static CORRELATED_ROWS: Cell<usize> = const { Cell::new(0) };
    static BINDINGS_EVALUATED: Cell<usize> = const { Cell::new(0) };
    /// (sq-7d3dj.30.20) Count of anti-join shapes the STATIC early-decline
    /// gate (opt-in `antijoin-static-decline`) rejected BEFORE the mandatory left side
    /// was evaluated — i.e. the redundant left evaluations this feature saved. Test-only
    /// signal for the non-vacuity check; only ever incremented under the feature.
    #[cfg(feature = "antijoin-static-decline")]
    static EARLY_DECLINED: Cell<usize> = const { Cell::new(0) };
}

/// Whether the correlated theta anti-join is enabled on this thread (default on).
#[inline]
pub(crate) fn enabled() -> bool {
    ENABLED.with(|e| e.get())
}

/// Enables/disables the path on this thread, returning the previous value. The
/// differential acceptance test uses it to compare fast-path vs cold results.
pub(crate) fn set_enabled(v: bool) -> bool {
    ENABLED.with(|e| e.replace(v))
}

/// Records one STATIC early-decline (the feature saved one redundant left evaluation).
#[cfg(feature = "antijoin-static-decline")]
pub(crate) fn record_early_decline() {
    EARLY_DECLINED.with(|e| e.set(e.get().saturating_add(1)));
}

/// Number of static early-declines since the last [`reset_stats`] (feature-gated).
#[cfg(feature = "antijoin-static-decline")]
pub(crate) fn early_declined() -> usize {
    EARLY_DECLINED.with(|e| e.get())
}

/// Clears the per-query firing statistics (call before a measured/traced run).
pub(crate) fn reset_stats() {
    FIRED.with(|f| f.set(false));
    CORRELATED_ROWS.with(|c| c.set(0));
    BINDINGS_EVALUATED.with(|b| b.set(0));
    #[cfg(feature = "antijoin-static-decline")]
    EARLY_DECLINED.with(|e| e.set(0));
}

/// Records one fired anti-join: `rows` right-side solutions produced across
/// `bindings` distinct correlation tuples evaluated.
pub(crate) fn record(rows: usize, bindings: usize) {
    FIRED.with(|f| f.set(true));
    CORRELATED_ROWS.with(|c| c.set(c.get().saturating_add(rows)));
    BINDINGS_EVALUATED.with(|b| b.set(b.get().saturating_add(bindings)));
}

/// `(fired, correlated_right_rows, distinct_correlations_evaluated)` since the last
/// [`reset_stats`]. The anti-vacuity acceptance test asserts `fired` and that the
/// correlated right side produced far fewer rows than the cold (blind) right side.
pub(crate) fn stats() -> (bool, usize, usize) {
    (
        FIRED.with(|f| f.get()),
        CORRELATED_ROWS.with(|c| c.get()),
        BINDINGS_EVALUATED.with(|b| b.get()),
    )
}
