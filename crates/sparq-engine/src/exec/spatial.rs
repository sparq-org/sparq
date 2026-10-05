use crate::SpatialProvider;
use std::cell::RefCell;
use std::sync::Arc;

thread_local! {
    static ACTIVE: RefCell<Option<Arc<dyn SpatialProvider>>> = const { RefCell::new(None) };
}

pub(crate) struct Guard;
impl Drop for Guard {
    fn drop(&mut self) {
        ACTIVE.with(|a| a.borrow_mut().take());
    }
}

pub(crate) fn install(idx: Arc<dyn SpatialProvider>) -> Guard {
    ACTIVE.with(|a| *a.borrow_mut() = Some(idx));
    Guard
}

/// The installed spatial index, if any.
pub(crate) fn active() -> Option<Arc<dyn SpatialProvider>> {
    ACTIVE.with(|a| a.borrow().clone())
}

#[cfg_attr(not(feature = "parallel"), allow(dead_code))]
pub(crate) fn snapshot() -> Option<Arc<dyn SpatialProvider>> {
    ACTIVE.with(|a| a.borrow().clone())
}

pub(crate) struct WorkerGuard(Option<Option<Arc<dyn SpatialProvider>>>);
impl Drop for WorkerGuard {
    fn drop(&mut self) {
        if let Some(prev) = self.0.take() {
            ACTIVE.with(|a| *a.borrow_mut() = prev);
        }
    }
}

#[cfg_attr(not(feature = "parallel"), allow(dead_code))]
pub(crate) fn worker_install(snap: &Option<Arc<dyn SpatialProvider>>) -> WorkerGuard {
    match snap {
        None => WorkerGuard(None),
        Some(idx) => WorkerGuard(Some(ACTIVE.with(|a| a.borrow_mut().replace(idx.clone())))),
    }
}
