use std::cell::Cell;

thread_local! {
    static ENABLED: Cell<bool> = const { Cell::new(true) };
    static BUILT: Cell<bool> = const { Cell::new(false) };
    static PRUNED: Cell<usize> = const { Cell::new(0) };
}

/// Whether the incidence prune is active on this thread (default on with the feature).
#[inline]
pub(crate) fn enabled() -> bool {
    ENABLED.with(|e| e.get())
}

/// Enables/disables the prune on this thread, returning the previous value. The
/// differential test flips it to compare pruned vs exact within one binary.
pub(crate) fn set_enabled(v: bool) -> bool {
    ENABLED.with(|e| e.replace(v))
}

/// Clears the per-query incidence statistics.
pub(crate) fn reset_stats() {
    BUILT.with(|b| b.set(false));
    PRUNED.with(|c| c.set(0));
}

/// Records that an incidence set was BUILT (a set was available for at least one branch).
pub(crate) fn note_built() {
    BUILT.with(|b| b.set(true));
}

/// Records `n` candidate predicates pruned by the incidence set (block scan skipped).
pub(crate) fn note_pruned(n: usize) {
    PRUNED.with(|c| c.set(c.get().saturating_add(n)));
}

/// `(incidence_built, predicates_pruned)` since the last [`reset_stats`]. The acceptance
/// test asserts the set was built AND pruned the value-typed no-hit predicates.
pub(crate) fn stats() -> (bool, usize) {
    (BUILT.with(|b| b.get()), PRUNED.with(|c| c.get()))
}
