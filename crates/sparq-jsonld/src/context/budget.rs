//! A per-call bound on context-processing work. Each public entry point (`expand`,
//! `compact`, `compact_expanded`, `frame`, `frame_match`, `ActiveContext::process`) runs
//! under [`with_budget`];
//! context processing charges every term table it copies and every definition it creates,
//! including definitions loaded from remote contexts or later discarded by a `null`, and
//! fails with `context overflow` once the call's budget is spent. Nested entry points
//! share the outermost budget.
//!
//! It also bounds how deeply the recursive walks nest ([`nest`]), so a deeply nested
//! document or context fails with `context overflow` instead of exhausting the stack.

use std::cell::Cell;

use crate::error::{JsonLdError, JsonLdErrorCode};

/// Most context-processing work units one call may spend.
#[cfg(not(test))]
pub(crate) const WORK_BUDGET: usize = 1 << 22;
#[cfg(test)]
pub(crate) const WORK_BUDGET: usize = 1 << 19;

thread_local! {
    static LEFT: Cell<Option<usize>> = const { Cell::new(None) };
}

/// Spends `n` units of the current call's budget (a no-op outside any entry point).
pub(crate) fn charge(n: usize) -> Result<(), JsonLdError> {
    LEFT.with(|left| match left.get() {
        Some(rem) if rem < n => Err(JsonLdError::with_detail(
            JsonLdErrorCode::ContextOverflow,
            "context processing exceeds the per-call work budget",
        )),
        Some(rem) => {
            left.set(Some(rem - n));
            Ok(())
        }
        None => Ok(()),
    })
}

/// Runs `f` with a fresh budget, unless one is already running on this thread.
pub(crate) fn with_budget<T>(f: impl FnOnce() -> T) -> T {
    struct Disarm;
    impl Drop for Disarm {
        fn drop(&mut self) {
            LEFT.with(|left| left.set(None));
        }
    }
    if LEFT.with(|left| left.get().is_some()) {
        return f();
    }
    LEFT.with(|left| left.set(Some(WORK_BUDGET)));
    let _disarm = Disarm;
    f()
}

/// How deeply the recursive document walks (expansion, compaction, frame matching) may
/// nest on one thread before they fail with `context overflow` instead of exhausting the
/// stack. Parsed documents nest at most [`MAX_DEPTH`](crate::json::MAX_DEPTH) deep.
pub(crate) const MAX_NESTING: usize = 128;

thread_local! {
    static NESTING: Cell<usize> = const { Cell::new(0) };
}

/// One level of a recursive document walk; dropping it leaves the level.
pub(crate) struct Nested(());

impl Drop for Nested {
    fn drop(&mut self) {
        NESTING.with(|n| n.set(n.get() - 1));
    }
}

/// Enters one more level of a recursive document walk.
pub(crate) fn nest() -> Result<Nested, JsonLdError> {
    NESTING.with(|n| {
        let depth = n.get();
        if depth >= MAX_NESTING {
            return Err(JsonLdError::with_detail(
                JsonLdErrorCode::ContextOverflow,
                "the document nests too deeply",
            ));
        }
        n.set(depth + 1);
        Ok(Nested(()))
    })
}
