use crate::CustomAggregateRegistry;
use std::cell::RefCell;
use std::sync::Arc;

thread_local! {
    static ACTIVE: RefCell<Option<Arc<CustomAggregateRegistry>>> = const { RefCell::new(None) };
}

pub(crate) struct Guard;
impl Drop for Guard {
    fn drop(&mut self) {
        ACTIVE.with(|a| a.borrow_mut().take());
    }
}

pub(crate) fn install(reg: &CustomAggregateRegistry) -> Guard {
    ACTIVE.with(|a| *a.borrow_mut() = Some(Arc::new(reg.clone())));
    Guard
}

#[cfg_attr(not(feature = "parallel"), allow(dead_code))]
pub(crate) fn snapshot() -> Option<Arc<CustomAggregateRegistry>> {
    ACTIVE.with(|a| a.borrow().clone())
}

pub(crate) struct WorkerGuard(Option<Option<Arc<CustomAggregateRegistry>>>);
impl Drop for WorkerGuard {
    fn drop(&mut self) {
        if let Some(prev) = self.0.take() {
            ACTIVE.with(|a| *a.borrow_mut() = prev);
        }
    }
}

#[cfg_attr(not(feature = "parallel"), allow(dead_code))]
pub(crate) fn worker_install(snap: &Option<Arc<CustomAggregateRegistry>>) -> WorkerGuard {
    match snap {
        None => WorkerGuard(None),
        Some(reg) => WorkerGuard(Some(ACTIVE.with(|a| a.borrow_mut().replace(reg.clone())))),
    }
}

/// The aggregate registered for `iri`, if a registry is installed and
/// contains it.
pub(crate) fn lookup(iri: &str) -> Option<crate::AggFn> {
    ACTIVE.with(|a| a.borrow().as_ref().and_then(|reg| reg.get(iri).cloned()))
}
