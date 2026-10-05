use crate::FunctionRegistry;
use std::cell::RefCell;
use std::sync::Arc;

thread_local! {
    static ACTIVE: RefCell<Option<Arc<FunctionRegistry>>> = const { RefCell::new(None) };
}

/// Uninstalls the registry when the installing entry point returns (also on
/// error/unwind, so a poisoned thread never leaks a stale registry).
pub(crate) struct Guard;
impl Drop for Guard {
    fn drop(&mut self) {
        ACTIVE.with(|a| a.borrow_mut().take());
    }
}

pub(crate) fn install(fns: &FunctionRegistry) -> Guard {
    ACTIVE.with(|a| *a.borrow_mut() = Some(Arc::new(fns.clone())));
    Guard
}

/// Snapshot of the installed registry for the rayon-parallel branches
/// (`None` — the overwhelmingly common case — makes [`worker_install`] free).
// Only the `parallel`-gated worker branches snapshot the registry (see
// `worker_install`, gated the same way); match the sibling `limits::snapshot` so the
// `-D warnings` clippy gate stays clean in no-parallel/wasm builds.
#[cfg_attr(not(feature = "parallel"), allow(dead_code))]
pub(crate) fn snapshot() -> Option<Arc<FunctionRegistry>> {
    ACTIVE.with(|a| a.borrow().clone())
}

/// Scoped re-install of a snapshot inside a rayon worker item. Restores the
/// PREVIOUS thread-local value on drop: rayon runs some items on the
/// installing thread itself, whose registry must survive the item.
pub(crate) struct WorkerGuard(Option<Option<Arc<FunctionRegistry>>>);
impl Drop for WorkerGuard {
    fn drop(&mut self) {
        if let Some(prev) = self.0.take() {
            ACTIVE.with(|a| *a.borrow_mut() = prev);
        }
    }
}

#[cfg_attr(not(feature = "parallel"), allow(dead_code))]
pub(crate) fn worker_install(snap: &Option<Arc<FunctionRegistry>>) -> WorkerGuard {
    match snap {
        None => WorkerGuard(None),
        Some(fns) => WorkerGuard(Some(ACTIVE.with(|a| a.borrow_mut().replace(fns.clone())))),
    }
}

/// The extension function registered for `iri`, if a registry is installed
/// and contains it.
pub(crate) fn lookup(iri: &str) -> Option<crate::ExtFn> {
    ACTIVE.with(|a| a.borrow().as_ref().and_then(|fns| fns.get(iri).cloned()))
}
