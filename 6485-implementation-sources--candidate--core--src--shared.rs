//! [OPUS-4.8] (sq-yj76l, gh #1121) Ergonomic thread-safe sharing of a [`Graph`] across the
//! async handlers of a web server (axum / actix / tower) — opt-in behind the `shared`
//! feature so the lean default / wasm build never pays for it.
//!
//! # Why this exists
//!
//! A [`Graph`] is already [`Send`] + [`Sync`] (guaranteed by a compile-time assertion in the
//! crate root), so it *can* be shared with the standard library — `Arc<RwLock<Graph>>`. The
//! external request behind this module ([gh #1121]) is not about *whether* that works but
//! about the boilerplate: every axum/actix `State` ends up re-deriving the same
//! `Arc<RwLock<…>>` plumbing and the same lock-discipline decisions. [`SharedGraph`] packages
//! the recommended pattern once, with the read-heavy default baked in.
//!
//! [gh #1121]: https://github.com/sparq-org/sparq/issues/1121
//!
//! # Which pattern to use
//!
//! sparq gives you two complementary ways to serve one store from many threads. Pick by your
//! read/write ratio:
//!
//! * **Snapshot-per-read (recommended for read-heavy serving).** Take a cheap, immutable
//!   [`GraphSnapshot`] with [`SharedGraph::snapshot`] and query *that* with no lock held for
//!   the duration of the request. Snapshotting is O(pending delta) (see [`Graph::snapshot`]),
//!   the write lock is held only for that O(overlay) clone, and readers never block each
//!   other or a writer once they hold their snapshot. This is the production serving shape:
//!   one mutable master + a cheap immutable snapshot per request / per commit.
//! * **Direct shared access.** [`SharedGraph::read`] / [`SharedGraph::write`] hand out RAII
//!   [`std::sync::RwLock`] guards for ad-hoc access when a snapshot would be overkill
//!   (e.g. a one-off `len()` or a quick in-place `apply_delta`). Many readers proceed
//!   concurrently; a writer is exclusive.
//!
//! # Example (axum-style state)
//!
//! ```
//! # #[cfg(feature = "shared")] {
//! use sparq_core::shared::SharedGraph;
//! use sparq_core::Graph;
//!
//! // Build once, share the handle into every handler (clone is an `Arc` bump).
//! let g = Graph::load_str("<http://ex/a> <http://ex/p> <http://ex/b> .", "ntriples").unwrap();
//! let shared = SharedGraph::new(g);
//!
//! // A read-heavy handler: snapshot, then query lock-free.
//! let handler_state = shared.clone();
//! let snap = handler_state.snapshot();
//! assert_eq!(snap.len(), 1);
//!
//! // A write handler: take the write lock only for the mutation.
//! shared
//!     .write()
//!     .apply_delta(&[], &[])
//!     .unwrap();
//! # }
//! ```
//!
//! # Poisoning
//!
//! [`SharedGraph`] wraps a [`std::sync::RwLock`]. If a thread panics while holding the write
//! lock the lock is *poisoned*; the accessors here recover the guard with
//! `unwrap_or_else(PoisonError::into_inner)` rather than propagating the poison, because a
//! `Graph` mutation that panics part-way still leaves the store in a structurally valid state
//! (the delta-overlay and dictionary are append-only and a failed `apply_delta` is reported
//! as an `Err` *before* any mmap/WAL write — see [`Graph::apply_delta`]). Reads are always
//! safe; a torn write surfaces as a normal `Err` from the mutating method, not as a poisoned
//! lock you must special-case in every handler.

use std::sync::{Arc, PoisonError, RwLock, RwLockReadGuard, RwLockWriteGuard};

use crate::{Graph, GraphSnapshot};

/// A cheaply-cloneable, thread-safe handle to one shared [`Graph`], ready to drop into an
/// axum / actix / tower `State`.
///
/// Cloning is an [`Arc`] bump — every clone refers to the *same* underlying graph, so an
/// `apply_delta` through one handle is visible through all of them. See the [module
/// docs](crate::shared) for when to prefer [`snapshot`](Self::snapshot) (lock-free reads)
/// over [`read`](Self::read) / [`write`](Self::write) (direct locked access).
#[derive(Clone)]
pub struct SharedGraph {
    inner: Arc<RwLock<Graph>>,
}

impl SharedGraph {
    /// Wraps an owned [`Graph`] in a shareable handle.
    #[inline]
    pub fn new(graph: Graph) -> Self {
        Self { inner: Arc::new(RwLock::new(graph)) }
    }

    /// A cheap, immutable, point-in-time [`GraphSnapshot`] of the current state — the
    /// recommended read path for a request handler.
    ///
    /// Holds the write lock only for the duration of [`Graph::snapshot`] (O(pending delta),
    /// never O(triples)); the returned snapshot is itself [`Send`] + [`Sync`] and can then be
    /// queried with no lock held, so concurrent readers never block one another or a later
    /// writer. The write lock (not the read lock) is taken because `snapshot` populates an
    /// interior cache the first time a flat graph is frozen; subsequent snapshots are O(overlay).
    #[inline]
    pub fn snapshot(&self) -> GraphSnapshot {
        self.inner.write().unwrap_or_else(PoisonError::into_inner).snapshot()
    }

    /// A shared (many-reader) RAII guard for ad-hoc read access. Prefer
    /// [`snapshot`](Self::snapshot) for anything beyond a one-off read — a held read guard
    /// blocks a concurrent writer for as long as the guard lives.
    #[inline]
    pub fn read(&self) -> RwLockReadGuard<'_, Graph> {
        self.inner.read().unwrap_or_else(PoisonError::into_inner)
    }

    /// An exclusive (single-writer) RAII guard for in-place mutation, e.g.
    /// [`Graph::apply_delta`] / [`Graph::compact`]. Held exclusively, so keep the critical
    /// section short.
    #[inline]
    pub fn write(&self) -> RwLockWriteGuard<'_, Graph> {
        self.inner.write().unwrap_or_else(PoisonError::into_inner)
    }

    /// The number of handles (this one included) sharing the underlying graph — the
    /// [`Arc::strong_count`] of the inner handle. Diagnostic only.
    #[inline]
    pub fn handle_count(&self) -> usize {
        Arc::strong_count(&self.inner)
    }
}

impl From<Graph> for SharedGraph {
    #[inline]
    fn from(graph: Graph) -> Self {
        Self::new(graph)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc as StdArc;
    use std::thread;

    fn sample() -> Graph {
        Graph::load_str(
            "<http://ex/a> <http://ex/p> <http://ex/b> .\n\
             <http://ex/a> <http://ex/p> <http://ex/c> .",
            "ntriples",
        )
        .expect("load")
    }

    #[test]
    fn snapshot_is_consistent_under_concurrent_reads() {
        let shared = SharedGraph::new(sample());
        let mut handles = Vec::new();
        for _ in 0..8 {
            let s = shared.clone();
            handles.push(thread::spawn(move || {
                // Each thread snapshots and queries lock-free; the real read path runs.
                let snap = s.snapshot();
                assert_eq!(snap.len(), 2);
                snap.len()
            }));
        }
        let total: usize = handles.into_iter().map(|h| h.join().unwrap()).sum();
        assert_eq!(total, 16);
    }

    #[test]
    fn snapshot_is_isolated_from_a_concurrent_write() {
        // Load-bearing invariant: a snapshot taken before a write reflects the OLD state,
        // and a snapshot taken after reflects the NEW state — the real apply_delta path.
        let shared = SharedGraph::new(sample());
        let before = shared.snapshot();
        assert_eq!(before.len(), 2);

        shared
            .write()
            .apply_delta(
                &[[
                    oxrdf::NamedNode::new("http://ex/a").unwrap().into(),
                    oxrdf::NamedNode::new("http://ex/p").unwrap().into(),
                    oxrdf::NamedNode::new("http://ex/d").unwrap().into(),
                ]],
                &[],
            )
            .expect("apply_delta");

        // The earlier snapshot is unaffected by the later mutation.
        assert_eq!(before.len(), 2);
        // A fresh snapshot sees the insert.
        assert_eq!(shared.snapshot().len(), 3);
        // And the shared base now reports 3 through a direct read guard too.
        assert_eq!(shared.read().len(), 3);
    }

    #[test]
    fn clones_share_one_underlying_graph() {
        let a = SharedGraph::new(sample());
        let b = a.clone();
        assert_eq!(a.handle_count(), 2);
        b.write()
            .apply_delta(
                &[[
                    oxrdf::NamedNode::new("http://ex/a").unwrap().into(),
                    oxrdf::NamedNode::new("http://ex/p").unwrap().into(),
                    oxrdf::NamedNode::new("http://ex/e").unwrap().into(),
                ]],
                &[],
            )
            .expect("apply_delta");
        // Mutation through `b` is visible through `a` — same store.
        assert_eq!(a.read().len(), 3);
    }

    #[test]
    fn shared_graph_is_send_and_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<SharedGraph>();
        // It is usable as `Arc<SharedGraph>` state too (axum wraps state in an Arc).
        let _: StdArc<SharedGraph> = StdArc::new(SharedGraph::new(sample()));
    }

    #[test]
    fn from_graph_builds_a_handle() {
        let shared: SharedGraph = sample().into();
        assert_eq!(shared.snapshot().len(), 2);
    }
}
