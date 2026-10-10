use crate::{DatasetView, DefaultGraphMode};
use oxrdf::Term;
use rustc_hash::FxHashSet;
use std::cell::RefCell;
use std::sync::Arc;

/// The installed view, plus the "inside GRAPH" suspend flag:
/// `eval_graph_named` swaps evaluation to the named sub-`Graph`, whose inner
/// patterns must NOT be empty-defaulted (only the TOP-LEVEL graph scope is).
#[derive(Clone, Default)]
pub(crate) struct State {
    named: Option<Arc<FxHashSet<Term>>>,
    default_empty: bool,
    suspended: bool,
}

thread_local! {
    static ACTIVE: RefCell<State> = RefCell::new(State::default());
}

/// Restores the pre-install state when the installing entry point returns
/// (also on error/unwind, so a poisoned thread never leaks a stale view).
pub(crate) struct Guard(State);
impl Drop for Guard {
    fn drop(&mut self) {
        ACTIVE.with(|a| *a.borrow_mut() = std::mem::take(&mut self.0));
    }
}

pub(crate) fn install(v: &DatasetView) -> Guard {
    let new = State {
        named: Some(Arc::clone(&v.named)),
        default_empty: matches!(v.default, DefaultGraphMode::Empty),
        suspended: false,
    };
    Guard(ACTIVE.with(|a| std::mem::replace(&mut *a.borrow_mut(), new)))
}

/// Installs a read view of exactly `named` with an empty default graph, without
/// borrowing the store (the update path mutates it while the view is installed).
pub(crate) fn install_reads(named: &Arc<FxHashSet<Term>>) -> Guard {
    let new = State { named: Some(Arc::clone(named)), default_empty: true, suspended: false };
    Guard(ACTIVE.with(|a| std::mem::replace(&mut *a.borrow_mut(), new)))
}

/// Fully suspends the view (named filter AND empty default) for a scope —
/// used by the entry points once `dataset::build_active` has folded the view
/// into a dataset-clause ACTIVE graph: the restriction is already applied,
/// and re-filtering would make a non-visible FROM NAMED graph behave
/// differently from an absent one (both must be the EMPTY active graph).
pub(crate) fn suspend_all() -> Guard {
    Guard(ACTIVE.with(|a| std::mem::take(&mut *a.borrow_mut())))
}

/// RAII suspension of the empty-default short-circuit only, for GRAPH scope
/// (the named-graph visibility filter stays active). Restores the previous
/// flag on drop, so nested scopes compose.
pub(crate) struct GraphScope(bool);
impl Drop for GraphScope {
    fn drop(&mut self) {
        ACTIVE.with(|a| a.borrow_mut().suspended = self.0);
    }
}

pub(crate) fn enter_graph() -> GraphScope {
    GraphScope(ACTIVE.with(|a| std::mem::replace(&mut a.borrow_mut().suspended, true)))
}

/// `true` when `name` is a visible named graph under the installed view
/// (always true with no view installed).
#[inline]
pub(crate) fn allows(name: &Term) -> bool {
    ACTIVE.with(|a| a.borrow().named.as_ref().is_none_or(|s| s.contains(name)))
}

/// `true` when the view's default graph is EMPTY at the current scope —
/// false with no view, under `StoreDefault`, or inside a GRAPH pattern.
#[inline]
pub(crate) fn default_is_empty() -> bool {
    ACTIVE.with(|a| {
        let s = a.borrow();
        s.default_empty && !s.suspended
    })
}

/// Snapshot of the installed view for the rayon-parallel expression branches
/// (`None` — no view, the common case — makes [`worker_install`] free).
#[cfg_attr(not(feature = "parallel"), allow(dead_code))]
pub(crate) fn snapshot() -> Option<State> {
    ACTIVE.with(|a| {
        let s = a.borrow();
        (s.named.is_some() || s.default_empty).then(|| s.clone())
    })
}

/// Scoped re-install of a snapshot inside a rayon worker item. Restores the
/// PREVIOUS thread-local value on drop: rayon runs some items on the
/// installing thread itself, whose view must survive the item.
pub(crate) struct WorkerGuard(Option<State>);
impl Drop for WorkerGuard {
    fn drop(&mut self) {
        if let Some(prev) = self.0.take() {
            ACTIVE.with(|a| *a.borrow_mut() = prev);
        }
    }
}

#[cfg_attr(not(feature = "parallel"), allow(dead_code))]
pub(crate) fn worker_install(snap: &Option<State>) -> WorkerGuard {
    match snap {
        None => WorkerGuard(None),
        Some(s) => WorkerGuard(Some(ACTIVE.with(|a| std::mem::replace(&mut *a.borrow_mut(), s.clone())))),
    }
}
