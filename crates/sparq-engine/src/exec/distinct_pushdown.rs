use std::cell::Cell;

thread_local! {
    static ENABLED: Cell<bool> = const { Cell::new(true) };
    static FIRED: Cell<bool> = const { Cell::new(false) };
    static ROWS_EMITTED: Cell<usize> = const { Cell::new(0) };
    static ROWS_SCANNED: Cell<usize> = const { Cell::new(0) };
}

/// Whether the DISTINCT-projection pushdown is enabled on this thread (default on).
#[inline]
pub(crate) fn enabled() -> bool {
    ENABLED.with(|e| e.get())
}

/// Enables/disables the pushdown on this thread, returning the previous value. The
/// differential acceptance test uses it to compare pushdown-on vs -off results.
pub(crate) fn set_enabled(v: bool) -> bool {
    ENABLED.with(|e| e.replace(v))
}

/// Clears the per-query pushdown statistics (call before a measured/traced run).
pub(crate) fn reset_stats() {
    FIRED.with(|f| f.set(false));
    ROWS_EMITTED.with(|c| c.set(0));
    ROWS_SCANNED.with(|c| c.set(0));
}

/// Records one fired pushdown: `emitted` distinct values produced, having TOUCHED
/// `scanned` permutation rows (the skip-scan's galloped-through rows — far fewer than
/// the materialised join when the pushdown is non-vacuous).
pub(crate) fn record(emitted: usize, scanned: usize) {
    FIRED.with(|f| f.set(true));
    ROWS_EMITTED.with(|c| c.set(c.get().saturating_add(emitted)));
    ROWS_SCANNED.with(|c| c.set(c.get().saturating_add(scanned)));
}

/// `(fired, distinct_values_emitted, permutation_rows_scanned)` since the last
/// [`reset_stats`]. The anti-vacuity acceptance test asserts `fired` and that
/// `permutation_rows_scanned` is far smaller than the full-join row count.
pub(crate) fn stats() -> (bool, usize, usize) {
    (
        FIRED.with(|f| f.get()),
        ROWS_EMITTED.with(|c| c.get()),
        ROWS_SCANNED.with(|c| c.get()),
    )
}
