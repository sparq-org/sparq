use std::cell::{Cell, RefCell};

/// One traced operator (pre-order position + depth reconstruct the tree).
pub(crate) struct Node {
    pub(crate) label: String,
    pub(crate) depth: usize,
    pub(crate) rows: usize,
    pub(crate) nanos: u64,
    /// The planner's estimated output cardinality for this operator, when the
    /// operator has a cardinality model (BGP nodes); `None` for operators whose
    /// output size the planner does not estimate. Used by the structured EXPLAIN
    /// (`explain-json`) to compute the per-operator q-error; the text trace
    /// ignores it. Present only under `explain-json` so the default trace node is
    /// byte-identical. sq-u4lgr
    #[cfg(feature = "explain-json")]
    pub(crate) est: Option<f64>,
}

thread_local! {
    static ENABLED: Cell<bool> = const { Cell::new(false) };
    static DEPTH: Cell<usize> = const { Cell::new(0) };
    static NODES: RefCell<Vec<Node>> = const { RefCell::new(Vec::new()) };
}

/// Disables tracing (and clears any partial trace) when the installing entry
/// point returns — also on error/unwind, so a failed query never leaks a trace.
pub(crate) struct Guard;
impl Drop for Guard {
    fn drop(&mut self) {
        ENABLED.with(|e| e.set(false));
        DEPTH.with(|d| d.set(0));
        NODES.with(|n| n.borrow_mut().clear());
    }
}

pub(crate) fn install() -> Guard {
    ENABLED.with(|e| e.set(true));
    DEPTH.with(|d| d.set(0));
    NODES.with(|n| n.borrow_mut().clear());
    Guard
}

#[inline]
pub(crate) fn enabled() -> bool {
    ENABLED.with(|e| e.get())
}

/// Opens a node, returning its index for [`exit`] to fill in.
pub(crate) fn enter(label: String) -> usize {
    let depth = DEPTH.with(|d| {
        let v = d.get();
        d.set(v + 1);
        v
    });
    NODES.with(|n| {
        let mut n = n.borrow_mut();
        n.push(Node {
            label,
            depth,
            rows: 0,
            nanos: 0,
            #[cfg(feature = "explain-json")]
            est: None,
        });
        n.len() - 1
    })
}

pub(crate) fn exit(idx: usize, rows: usize, nanos: u64) {
    DEPTH.with(|d| d.set(d.get().saturating_sub(1)));
    NODES.with(|n| {
        if let Some(node) = n.borrow_mut().get_mut(idx) {
            node.rows = rows;
            node.nanos = nanos;
        }
    });
}

/// Records the planner's estimated output cardinality for the node opened at
/// `idx` (called by `eval_graph_pattern_traced` only when the BGP estimate is
/// computed). Separate from [`exit`] so the estimate is captured at ENTER time,
/// before the (recursive) child evaluation runs. sq-u4lgr
#[cfg(feature = "explain-json")]
pub(crate) fn set_est(idx: usize, est: f64) {
    NODES.with(|n| {
        if let Some(node) = n.borrow_mut().get_mut(idx) {
            node.est = Some(est);
        }
    });
}

/// Drains the recorded nodes (pre-order). Call before the guard drops.
pub(crate) fn take() -> Vec<Node> {
    NODES.with(|n| std::mem::take(&mut *n.borrow_mut()))
}
