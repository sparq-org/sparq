use sparq_engine_service::service::Transport;
use std::cell::RefCell;

thread_local! {
    // A boxed trait object so a test can install any `Transport`. `None` => use
    // the real HTTP client.
    static ACTIVE: RefCell<Option<Box<dyn Transport>>> = const { RefCell::new(None) };
}

/// RAII install of a test transport; restores the previous value on drop.
pub(crate) struct Guard(Option<Box<dyn Transport>>);
impl Drop for Guard {
    fn drop(&mut self) {
        ACTIVE.with(|a| *a.borrow_mut() = self.0.take());
    }
}

/// Install a transport for the current thread/scope (used by tests).
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn install(t: Box<dyn Transport>) -> Guard {
    Guard(ACTIVE.with(|a| a.borrow_mut().replace(t)))
}

/// Run `f` with the optional installed test transport (`None` = no override, use
/// the production HTTP client). Used by `service_reader_transport` to share the
/// same `ACTIVE` cell without re-exposing its private `RefCell`.
pub(crate) fn with_opt<R>(f: impl FnOnce(Option<&dyn Transport>) -> R) -> R {
    ACTIVE.with(|a| f(a.borrow().as_deref()))
}
