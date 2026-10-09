use std::cell::Cell;

thread_local! {
    static ENABLED: Cell<bool> = const { Cell::new(true) };
    static FIRED: Cell<bool> = const { Cell::new(false) };
    static CORRELATED_ROWS: Cell<usize> = const { Cell::new(0) };
    static BINDINGS_EVALUATED: Cell<usize> = const { Cell::new(0) };
}

/// Whether correlated (SIP) join evaluation is enabled on this thread (default on).
#[inline]
pub(crate) fn enabled() -> bool {
    ENABLED.with(|e| e.get())
}

/// Enables/disables SIP on this thread, returning the previous value. The
/// differential acceptance test uses it to compare correlated-on vs -off results.
pub(crate) fn set_enabled(v: bool) -> bool {
    ENABLED.with(|e| e.replace(v))
}

/// Clears the per-query firing statistics (call before a measured/traced run).
pub(crate) fn reset_stats() {
    FIRED.with(|f| f.set(false));
    CORRELATED_ROWS.with(|c| c.set(0));
    BINDINGS_EVALUATED.with(|b| b.set(0));
}

/// Records one fired correlated evaluation: `rows` solutions produced across
/// `bindings` distinct pushed-value combinations.
pub(crate) fn record(rows: usize, bindings: usize) {
    FIRED.with(|f| f.set(true));
    CORRELATED_ROWS.with(|c| c.set(c.get().saturating_add(rows)));
    BINDINGS_EVALUATED.with(|b| b.set(b.get().saturating_add(bindings)));
}

/// `(fired, correlated_child_rows, distinct_bindings_evaluated)` since the last
/// [`reset_stats`]. The anti-vacuity acceptance test asserts `fired` and that the
/// correlated child produced far fewer rows than the cold (blind) child.
pub(crate) fn stats() -> (bool, usize, usize) {
    (
        FIRED.with(|f| f.get()),
        CORRELATED_ROWS.with(|c| c.get()),
        BINDINGS_EVALUATED.with(|b| b.get()),
    )
}
