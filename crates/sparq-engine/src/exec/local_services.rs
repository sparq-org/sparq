use crate::LocalServiceRegistry;
use std::cell::RefCell;
use std::sync::Arc;

thread_local! {
    static ACTIVE: RefCell<Option<Arc<LocalServiceRegistry>>> = const { RefCell::new(None) };
}

/// Uninstalls on drop by restoring the PREVIOUS registry rather than clearing —
/// `with_local_services` is documented as scoped RAII, so a NESTED install must
/// hand the outer scope its own registry back when the inner one ends (clearing
/// would silently unregister the outer handlers for the rest of the outer scope,
/// sending a formerly-local IRI down the HTTP path). Restores on unwind too.
pub(crate) struct Guard(Option<Arc<LocalServiceRegistry>>);
impl Drop for Guard {
    fn drop(&mut self) {
        let prev = self.0.take();
        ACTIVE.with(|a| *a.borrow_mut() = prev);
    }
}

pub(crate) fn install(reg: &LocalServiceRegistry) -> Guard {
    Guard(ACTIVE.with(|a| a.borrow_mut().replace(Arc::new(reg.clone()))))
}

#[cfg_attr(not(feature = "parallel"), allow(dead_code))]
pub(crate) fn snapshot() -> Option<Arc<LocalServiceRegistry>> {
    ACTIVE.with(|a| a.borrow().clone())
}

pub(crate) struct WorkerGuard(Option<Option<Arc<LocalServiceRegistry>>>);
impl Drop for WorkerGuard {
    fn drop(&mut self) {
        if let Some(prev) = self.0.take() {
            ACTIVE.with(|a| *a.borrow_mut() = prev);
        }
    }
}

#[cfg_attr(not(feature = "parallel"), allow(dead_code))]
pub(crate) fn worker_install(snap: &Option<Arc<LocalServiceRegistry>>) -> WorkerGuard {
    match snap {
        None => WorkerGuard(None),
        Some(reg) => WorkerGuard(Some(ACTIVE.with(|a| a.borrow_mut().replace(reg.clone())))),
    }
}

/// The local handler registered for `iri`, if a registry is installed and
/// contains it.
pub(crate) fn lookup(iri: &str) -> Option<crate::LocalServiceFn> {
    ACTIVE.with(|a| a.borrow().as_ref().and_then(|reg| reg.get(iri).cloned()))
}

/// Whether `iri` is served locally — the cheap check the SERVICE bind-join uses to
/// DECLINE pushing a `VALUES` block at an IRI that never reaches the network.
#[cfg_attr(not(feature = "service"), allow(dead_code))]
pub(crate) fn handles(iri: &str) -> bool {
    ACTIVE.with(|a| a.borrow().as_ref().is_some_and(|reg| reg.contains(iri)))
}
