// GPT-6 Astra: exact final budget module; public calls link production library.
#![allow(dead_code)]
pub use sparq_engine::*;
mod exec {
pub(crate) mod budget {
    use crate::QueryBudget;
    use sparq_core::dict::Id;
    use std::cell::Cell;
    use std::ptr::NonNull;
    use std::sync::atomic::{AtomicBool, Ordering};

    /// Copyable view of a cancellation flag owned by the installed [`QueryBudget`].
    ///
    /// [GPT-6 Astra] ACTIVE belongs to the innermost live `with_budget` frame.
    /// Each pointer is owned by a QueryBudget borrowed by a still-live frame on
    /// this thread's stack. Private installation/restoration enforces nesting:
    /// a restored parent's borrow outlives the child. Only the innermost frame
    /// writes ACTIVE. Snapshots are used only in synchronous parallel work that
    /// joins before the owning frame returns; dereferences are atomic loads.
    #[derive(Clone, Copy)]
    struct CancelPtr(NonNull<AtomicBool>);

    // SAFETY: `AtomicBool` is `Sync`; moving this shared pointer to a worker is
    // sound because only atomic loads occur while the owning `with_budget` frame
    // borrows its QueryBudget, including until scoped rayon work joins.
    unsafe impl Send for CancelPtr {}
    // SAFETY: `AtomicBool` is `Sync`; all shared access through `CancelPtr` is an
    // atomic load; the owning `with_budget` frame outlives all worker joins.
    unsafe impl Sync for CancelPtr {}

    /// Bytes one id-level binding cell occupies in a materialised `Row`. The
    /// byte-accounted cap ([OPUS-4.8] sq-s5is) costs the id-level working set as
    /// `rows × width × BYTES_PER_ID` — a portable LOWER bound on real heap (it
    /// ignores allocator overhead / `SmallVec` inline-vs-spill), conservative in the
    /// same direction the row cap is.
    pub(crate) const BYTES_PER_ID: usize = std::mem::size_of::<Id>();

    /// The installed limits, flattened for a cheap per-check read.
    #[derive(Clone, Copy)]
    pub(in crate::exec) struct Limits {
        on: bool,
        #[cfg(not(target_arch = "wasm32"))]
        deadline: Option<std::time::Instant>,
        max_rows: usize,
        /// [OPUS-4.8] (sq-s5is) Byte ceiling on the estimated working set; `usize::MAX`
        /// when no byte cap is set. Compared against `rows × byte_width + extra_bytes`.
        max_bytes: usize,
        /// [OPUS-4.8] (sq-s5is) Bytes per row of the working set CURRENTLY being checked
        /// — `width(in ids) × BYTES_PER_ID`. Set per operator by [`set_width`] so the
        /// row-count check sites also price WIDTH (the dimension the row cap misses). A
        /// scalar/streaming path that never sets a width leaves this at `BYTES_PER_ID`
        /// (one id per "row"), so the byte cap degrades to the row cap there, never wider.
        byte_width: usize,
        /// [OPUS-4.8] (sq-s5is) Bytes of query-computed terms interned into the per-query
        /// local vocabulary (BIND / aggregate / CONSTRUCT scratch) — the NON-row dimension
        /// the row cap also misses. A running high-water sum, added to the working-set
        /// estimate on every check.
        extra_bytes: usize,
        cancel: Option<CancelPtr>,
    }

    const OFF: Limits = Limits {
        on: false,
        #[cfg(not(target_arch = "wasm32"))]
        deadline: None,
        max_rows: usize::MAX,
        max_bytes: usize::MAX,
        byte_width: BYTES_PER_ID,
        extra_bytes: 0,
        cancel: None,
    };

    impl Limits {
        /// `rows × byte_width + extra_bytes`, saturating — the estimated working-set
        /// byte size compared against `max_bytes`. [OPUS-4.8] (sq-s5is)
        #[inline]
        fn bytes(&self, rows: usize) -> usize {
            rows.saturating_mul(self.byte_width)
                .saturating_add(self.extra_bytes)
        }

        /// WHY the limits are hit at `rows`, or `None` when they are not — the pure (no
        /// thread-local) counterpart of [`exhausted`]'s reason, for rayon closures where
        /// the installing thread's sticky flag is out of reach. The reasons are the SAME
        /// strings [`exhausted`] records, so a worker can raise EXACTLY the error
        /// [`check`] would rather than inventing one (or guessing a result). [SONNET-4.6]
        /// (sq-qk6ac)
        #[cfg_attr(not(feature = "parallel"), allow(dead_code))]
        #[inline]
        pub(in crate::exec) fn why(&self, rows: usize) -> Option<&'static str> {
            if !self.on {
                return None;
            }
            if rows > self.max_rows {
                return Some("max-rows");
            }
            if self.bytes(rows) > self.max_bytes {
                return Some("max-bytes");
            }
            #[cfg(not(target_arch = "wasm32"))]
            if self
                .deadline
                .is_some_and(|d| std::time::Instant::now() >= d)
            {
                return Some("timeout");
            }
            if let Some(cancel) = self.cancel {
                // SAFETY: `CancelPtr`'s nested-frame invariant keeps the owning
                // QueryBudget alive until this scoped snapshot load finishes.
                if unsafe { cancel.0.as_ref() }.load(Ordering::Relaxed) {
                    return Some("cancelled");
                }
            }
            None
        }

        /// Pure (no thread-local) exhaustion test for rayon closures, where the
        /// installing thread's sticky flag is out of reach: a worker that sees
        /// `hit` stops producing, and the caller's next on-thread check fires
        /// (the deadline is global time; a hit row/byte cap leaves the snapshot's
        /// estimate over the limit). Only the rayon-parallel branches call this
        /// (and `snapshot`); the non-parallel (wasm) build compiles them out.
        #[cfg_attr(not(feature = "parallel"), allow(dead_code))]
        #[inline]
        pub(in crate::exec) fn hit(&self, rows: usize) -> bool {
            self.why(rows).is_some()
        }
    }

    thread_local! {
        static ACTIVE: Cell<Limits> = const { Cell::new(OFF) };
        static EXCEEDED: Cell<Option<&'static str>> = const { Cell::new(None) };
    }

    // [GPT-6 Astra] Private: callers cannot forget or drop a frame out of order.
    struct Guard<'a> {
        previous: Limits,
        exceeded: Option<&'static str>,
        _budget: std::marker::PhantomData<&'a QueryBudget>,
        _not_send: std::marker::PhantomData<std::rc::Rc<()>>,
    }
    impl Drop for Guard<'_> {
        fn drop(&mut self) {
            ACTIVE.with(|a| a.set(OFF));
            EXCEEDED.with(|e| e.set(self.exceeded));
        }
    }

    fn install(b: &QueryBudget) -> Guard<'_> {
        let cancel = b
            .cancel
            .as_ref()
            .map(|flag| CancelPtr(NonNull::from(flag.as_ref())));
        #[cfg(not(target_arch = "wasm32"))]
        let on = b.deadline.is_some()
            || b.max_rows.is_some()
            || b.max_bytes.is_some()
            || cancel.is_some();
        #[cfg(target_arch = "wasm32")]
        let on = b.max_rows.is_some() || b.max_bytes.is_some() || cancel.is_some();
        let previous = ACTIVE.with(|a| {
            a.replace(Limits {
                on,
                #[cfg(not(target_arch = "wasm32"))]
                deadline: b.deadline,
                max_rows: b.max_rows.unwrap_or(usize::MAX),
                max_bytes: b.max_bytes.unwrap_or(usize::MAX),
                byte_width: BYTES_PER_ID,
                extra_bytes: 0,
                cancel,
            })
        });
        let exceeded = EXCEEDED.with(|e| e.replace(None));
        Guard {
            previous,
            exceeded,
            _budget: std::marker::PhantomData,
            _not_send: std::marker::PhantomData,
        }
    }

    /// [GPT-6 Astra] Runs a child budget, restoring its parent on return or unwind.
    ///
    /// Guard never escapes this frame. This enforces LIFO even for reentrant
    /// callbacks, keeps each borrowed cancellation owner alive, and restores idle
    /// OFF/None after a top-level call. Aborting panics have no continuation.
    pub(crate) fn with_budget<T>(b: &QueryBudget, f: impl FnOnce() -> T) -> T {
        let _guard = install(b);
        f()
    }

    /// [OPUS-4.8] (sq-s5is) Sets the per-row byte width (= `width_in_ids ×
    /// BYTES_PER_ID`) of the working set the next row-count checks price. Called once
    /// per operator with that operator's output arity, so a check on `rows` correctly
    /// estimates `rows × width` bytes — the WIDE-row dimension the row cap misses.
    /// No-op (and no thread-local write on the unbudgeted hot path) when no budget is
    /// installed. Returns the previous width so callers can restore it.
    #[inline]
    pub(crate) fn set_width(width_in_ids: usize) -> usize {
        ACTIVE.with(|c| {
            let mut a = c.get();
            let prev = a.byte_width;
            if a.on {
                a.byte_width = width_in_ids.max(1).saturating_mul(BYTES_PER_ID);
                c.set(a);
            }
            prev
        })
    }

    /// [OPUS-4.8] (sq-s5is) Restores a byte width previously returned by [`set_width`]
    /// (cheap: one thread-local write, only while budgeted).
    #[inline]
    pub(crate) fn restore_width(prev: usize) {
        ACTIVE.with(|c| {
            let mut a = c.get();
            if a.on {
                a.byte_width = prev;
                c.set(a);
            }
        });
    }

    /// [OPUS-4.8] (sq-s5is) Adds `n` bytes of query-computed terms to the local-vocab
    /// high-water accumulator (the NON-row dimension). Trips the sticky flag immediately
    /// if it pushes the estimate over `max_bytes`, so an oversized CONSTRUCT template /
    /// aggregate scratch is caught even between row-count checks. No-op when unbudgeted.
    #[inline]
    pub(crate) fn add_bytes(n: usize) {
        ACTIVE.with(|c| {
            let mut a = c.get();
            if !a.on {
                return;
            }
            a.extra_bytes = a.extra_bytes.saturating_add(n);
            c.set(a);
            if a.extra_bytes > a.max_bytes {
                EXCEEDED.with(|e| {
                    if e.get().is_none() {
                        e.set(Some("max-bytes"));
                    }
                });
            }
        });
    }

    /// Snapshot of the installed limits, for the rayon-parallel branches.
    ///
    /// [GPT-6 Astra] This lifetime-free Copy is trusted only inside exec. It must
    /// not escape its owning with_budget frame or enter detached work. The four
    /// consumers are scan SELECT-JSON, bindings SELECT-JSON, parallel hash join,
    /// and the parallel residual anti-join; all join before returning. A new
    /// consumer must establish the same owner lifetime and scoped-join invariant.
    #[cfg_attr(not(feature = "parallel"), allow(dead_code))]
    #[inline]
    pub(in crate::exec) fn snapshot() -> Limits {
        ACTIVE.with(|a| a.get())
    }

    /// Decides whether the multi-core SELECT-JSON serializer may fan out under the
    /// CURRENTLY installed budget, returning the limit snapshot its workers re-check at
    /// each par-chunk boundary. [OPUS-4.8] (sq-7d3dj.10, roborev 1538, audit item 6)
    ///
    /// The parallel path builds every matching JSON fragment before it can know a row or
    /// byte count, so it cannot enforce a ROW / BYTE cap mid-serialize:
    ///
    /// * `Some(limits)` — fan out. Either NO budget is installed (`limits.on == false`,
    ///   making the per-chunk `hit` re-check a no-op) OR the budget is DEADLINE-ONLY
    ///   (both row and byte caps at their `usize::MAX` sentinel). Under a deadline-only
    ///   budget the per-chunk `limits.hit(0)` re-check stops launching new chunks once
    ///   the wall-clock deadline has passed, so the worst-case CPU overrun is bounded to
    ///   the chunks already in flight — approximately one per worker, a bounded constant
    ///   — not the unbounded burn an uncheckable fan-out under a row/byte cap would allow.
    /// * `None` — a row and/or byte cap is installed; the caller must take the
    ///   cooperative SERIAL loop, which cannot over-produce (it checks the sticky flag
    ///   every 1024 rows and stops early). A blanket "fan out whenever a budget is
    ///   installed" was REJECTED for exactly this reason (roborev 1538 / audit item 6).
    ///
    /// The returned snapshot has the same owning-frame/scoped-join invariant as
    /// [`snapshot`]; scan SELECT-JSON is its sole production consumer.
    /// Compiled only for the `parallel` feature — the wasm/serial build never fans out.
    #[cfg(feature = "parallel")]
    #[inline]
    pub(in crate::exec) fn parallel_json_fanout() -> Option<Limits> {
        ACTIVE.with(|a| {
            let l = a.get();
            if l.on && (l.max_rows != usize::MAX || l.max_bytes != usize::MAX) {
                None // a row/byte cap the fan-out cannot enforce mid-serialize → serial loop
            } else {
                Some(l)
            }
        })
    }

    /// Time remaining until the installed wall-clock deadline, if any. [OPUS-4.8] (sq-d4p)
    ///
    /// The SERVICE HTTP transport uses this to bound a remote round-trip by the SAME
    /// budget that bounds local evaluation: a query under a 5s deadline must not block
    /// for the transport's fixed default on an unresponsive endpoint. Returns:
    /// * `None` — no deadline installed (no budget, or a row/byte-only budget); the
    ///   transport keeps its own finite default.
    /// * `Some(Duration::ZERO)` — the deadline has already passed; the caller should
    ///   refuse the remote call immediately rather than dial.
    /// * `Some(d)` — the remaining time, which the transport caps its own default to.
    ///
    /// Always compiled only off-wasm (no `Instant` there, and the `service` feature
    /// never reaches a wasm build).
    #[cfg(not(target_arch = "wasm32"))]
    #[cfg_attr(not(feature = "service"), allow(dead_code))]
    #[inline]
    pub(crate) fn remaining_timeout() -> Option<std::time::Duration> {
        ACTIVE.with(|a| {
            let lim = a.get();
            lim.deadline
                .map(|d| d.saturating_duration_since(std::time::Instant::now()))
        })
    }

    /// A savepoint of the local-vocab byte accumulator + the sticky exhaustion flag,
    /// taken BEFORE a speculative interning burst — a streaming SERVICE block whose
    /// rows a SILENT error must discard. [`restore_bytes`] rewinds to it so the
    /// discarded interns leave the byte budget EXACTLY as if they never happened,
    /// keeping SILENT SERVICE behaviour-neutral with the pre-streaming
    /// collect-then-intern-on-success path (which charged nothing on a swallowed
    /// remote error). [OPUS-4.8] (sq-my8wd.4)
    #[cfg(any(feature = "service", feature = "service-local"))]
    #[derive(Clone, Copy)]
    pub(crate) struct ByteSavepoint {
        extra_bytes: usize,
        exceeded: Option<&'static str>,
    }

    /// Capture the current byte accumulator + exhaustion flag. [OPUS-4.8] (sq-my8wd.4)
    #[cfg(any(feature = "service", feature = "service-local"))]
    #[inline]
    pub(crate) fn byte_savepoint() -> ByteSavepoint {
        ByteSavepoint {
            extra_bytes: ACTIVE.with(|c| c.get().extra_bytes),
            exceeded: EXCEEDED.with(|e| e.get()),
        }
    }

    /// Rewind the byte accumulator + exhaustion flag to a [`ByteSavepoint`]. Only the
    /// bytes charged (and any max-bytes exhaustion tripped) SINCE the savepoint are
    /// undone; an exhaustion that fired for an independent reason before it is
    /// preserved. Sound because the interning burst it brackets is synchronous and
    /// single-threaded — the SERVICE sink is the only writer between the savepoint and
    /// here — so the pre-burst snapshot is exactly the current state minus this burst.
    /// A deadline that elapsed during the burst is not masked: the next `exhausted`
    /// re-derives it from the wall clock. [OPUS-4.8] (sq-my8wd.4)
    /// [GPT-6 Astra] A synchronous nested with_budget may finish between capture
    /// and restore: it restores this frame verbatim first. Never apply a savepoint
    /// while a different budget frame is active.
    #[cfg(any(feature = "service", feature = "service-local"))]
    #[inline]
    pub(crate) fn restore_bytes(sp: ByteSavepoint) {
        ACTIVE.with(|c| {
            let mut a = c.get();
            a.extra_bytes = sp.extra_bytes;
            c.set(a);
        });
        EXCEEDED.with(|e| e.set(sp.exceeded));
    }

    /// `true` once the budget is exhausted (sticky) — row-producing loops break
    /// on it; `rows` is the loop's current output size.
    #[inline]
    pub(crate) fn exhausted(rows: usize) -> bool {
        let a = ACTIVE.with(|c| c.get());
        if !a.on {
            return false;
        }
        if EXCEEDED.with(|e| e.get()).is_some() {
            return true;
        }
        if rows > a.max_rows {
            EXCEEDED.with(|e| e.set(Some("max-rows")));
            return true;
        }
        if a.bytes(rows) > a.max_bytes {
            EXCEEDED.with(|e| e.set(Some("max-bytes")));
            return true;
        }
        #[cfg(not(target_arch = "wasm32"))]
        if a.deadline.is_some_and(|d| std::time::Instant::now() >= d) {
            EXCEEDED.with(|e| e.set(Some("timeout")));
            return true;
        }
        if let Some(cancel) = a.cancel {
            // SAFETY: `CancelPtr`'s nested-frame invariant keeps this pointer
            // owned by a live QueryBudget, also when restoring a parent frame.
            // Relaxed is sufficient because cancellation gates control flow only;
            // it never publishes or guards a shared query buffer. If that changes,
            // the load/store pair must become Acquire/Release.
            if unsafe { cancel.0.as_ref() }.load(Ordering::Relaxed) {
                EXCEEDED.with(|e| e.set(Some("cancelled")));
                return true;
            }
        }
        false
    }

    /// Propagates an exhausted budget as the query error.
    #[inline]
    pub(crate) fn check(rows: usize) -> Result<(), String> {
        if exhausted(rows) {
            let why = EXCEEDED.with(|e| e.get()).unwrap_or("timeout");
            return Err(format!("query budget exceeded ({why})"));
        }
        Ok(())
    }

    /// Returns `true` when a budget is currently installed (even if not yet exhausted).
    /// The columnar path uses this for the I3 fallback rule: when a budget is armed the
    /// seam declines to the scalar path (the scalar debit schedule is not uniform-per-row
    /// inside `apply_filter`, so the `k = min(batch_len, budget_remaining)` prefix rule
    /// cannot be applied; the fallback is budget-armed ⇒ decline per the design record
    /// `research/vector-at-a-time-m4-completion-design.md` §1 I3). [SONNET-4.6] (sq-pntvh.5)
    #[cfg_attr(not(feature = "vectorized"), allow(dead_code))]
    #[inline]
    pub(crate) fn active() -> bool {
        ACTIVE.with(|c| c.get().on)
    }

    /// Caps a speculative `Vec` pre-allocation while a budget is active, so a
    /// budgeted cross-product cannot allocate its full (possibly astronomical)
    /// output up front before the first cooperative check fires. Honours BOTH the
    /// row cap and (via `byte_width`) the byte cap — whichever admits fewer rows.
    #[inline]
    pub(crate) fn cap_alloc(cap: usize) -> usize {
        let a = ACTIVE.with(|c| c.get());
        if !a.on {
            return cap;
        }
        // Rows the byte cap still admits, given the current width and accrued extra.
        let by_bytes = a
            .max_bytes
            .saturating_sub(a.extra_bytes)
            .checked_div(a.byte_width.max(1))
            .unwrap_or(usize::MAX)
            .saturating_add(1);
        cap.min(a.max_rows.saturating_add(1))
            .min(by_bytes)
            .min(1 << 20)
    }

    // [GPT-6 Astra] Reentrant scopes must restore the complete owning frame.
    #[cfg(test)]
    mod nested_budget_tests {
        use super::*;
        use std::sync::Arc;

        fn assert_state(want: Limits, sticky: Option<&'static str>) {
            let got = ACTIVE.with(Cell::get);
            assert_eq!(got.on, want.on);
            assert_eq!(got.max_rows, want.max_rows);
            assert_eq!(got.max_bytes, want.max_bytes);
            assert_eq!(got.byte_width, want.byte_width);
            assert_eq!(got.extra_bytes, want.extra_bytes);
            assert_eq!(got.cancel.map(|p| p.0), want.cancel.map(|p| p.0));
            #[cfg(not(target_arch = "wasm32"))]
            assert_eq!(got.deadline, want.deadline);
            assert_eq!(EXCEEDED.with(Cell::get), sticky);
        }

        #[test]
        fn exact_parent_state_survives_three_levels_ok_err_and_unwind() {
            let outer = QueryBudget {
                max_rows: Some(7),
                max_bytes: Some(4096),
                ..QueryBudget::cancelled_by(Arc::new(AtomicBool::new(false)))
            };
            with_budget(&outer, || {
                set_width(5);
                add_bytes(37);
                let parent = snapshot();
                let child = QueryBudget {
                    max_rows: Some(2),
                    max_bytes: Some(128),
                    ..QueryBudget::unlimited()
                };
                let result: Result<(), &str> = with_budget(&child, || {
                    set_width(2);
                    add_bytes(11);
                    let middle = snapshot();
                    with_budget(&QueryBudget::unlimited(), || {
                        assert_eq!(check(usize::MAX), Ok(()))
                    });
                    assert_state(middle, None);
                    assert!(check(3).is_err());
                    Err("child error")
                });
                assert_eq!(result, Err("child error"));
                assert_state(parent, None);
                let panic = std::panic::catch_unwind(|| {
                    with_budget(&child, || {
                        add_bytes(1000);
                        panic!("controlled child unwind");
                    })
                });
                assert!(panic.is_err());
                assert_state(parent, None);
                assert_eq!(check(7), Ok(()));
                assert!(check(8).is_err());
            });
            assert_state(OFF, None);
            let top_level_panic = std::panic::catch_unwind(|| {
                with_budget(&outer, || {
                    set_width(9);
                    add_bytes(5000);
                    panic!("controlled top-level unwind");
                });
            });
            assert!(top_level_panic.is_err());
            assert_state(OFF, None);
        }

        #[test]
        fn sticky_parent_errors_survive_clean_and_exhausted_children() {
            for reason in ["max-rows", "max-bytes"] {
                let outer = QueryBudget {
                    max_rows: Some(2),
                    max_bytes: Some(128),
                    ..QueryBudget::unlimited()
                };
                with_budget(&outer, || {
                    if reason == "max-rows" {
                        assert!(check(3).is_err());
                    } else {
                        add_bytes(129);
                    }
                    let parent = snapshot();
                    with_budget(&QueryBudget::unlimited(), || {
                        assert_eq!(check(usize::MAX), Ok(()))
                    });
                    assert_state(parent, Some(reason));
                    with_budget(
                        &QueryBudget::cancelled_by(Arc::new(AtomicBool::new(true))),
                        || {
                            assert_eq!(
                                check(0),
                                Err("query budget exceeded (cancelled)".to_owned())
                            );
                        },
                    );
                    assert_state(parent, Some(reason));
                    assert_eq!(check(0), Err(format!("query budget exceeded ({})", reason)));
                });
                assert_state(OFF, None);
            }
        }

        #[test]
        fn live_parent_cancel_is_distinct_from_child_cancel() {
            let parent_flag = Arc::new(AtomicBool::new(false));
            let child_flag = Arc::new(AtomicBool::new(false));
            let outer = QueryBudget::cancelled_by(Arc::clone(&parent_flag));
            let child = QueryBudget::cancelled_by(Arc::clone(&child_flag));
            with_budget(&outer, || {
                let parent = snapshot();
                with_budget(&child, || {
                    parent_flag.store(true, Ordering::Relaxed);
                    assert_eq!(check(0), Ok(()));
                    child_flag.store(true, Ordering::Relaxed);
                    assert!(check(0).is_err());
                });
                assert_state(parent, None);
                assert_eq!(
                    check(0),
                    Err("query budget exceeded (cancelled)".to_owned())
                );
            });
            assert_state(OFF, None);
        }

        #[test]
        #[cfg(not(target_arch = "wasm32"))]
        fn expired_parent_deadline_returns_after_unlimited_child_without_sleep() {
            let parent = QueryBudget {
                deadline: Some(std::time::Instant::now()),
                ..QueryBudget::unlimited()
            };
            with_budget(&parent, || {
                with_budget(&QueryBudget::unlimited(), || assert_eq!(check(0), Ok(())));
                assert_eq!(check(0), Err("query budget exceeded (timeout)".to_owned()));
            });
            assert_state(OFF, None);
        }

        #[test]
        #[cfg(any(feature = "service", feature = "service-local"))]
        fn service_savepoint_brackets_a_fully_returned_nested_budget() {
            let parent = QueryBudget {
                max_bytes: Some(128),
                ..QueryBudget::unlimited()
            };
            with_budget(&parent, || {
                set_width(3);
                add_bytes(19);
                let original = snapshot();
                let mark = byte_savepoint();
                with_budget(&QueryBudget::unlimited(), || {
                    assert_eq!(check(usize::MAX), Ok(()))
                });
                add_bytes(200);
                assert!(check(0).is_err());
                restore_bytes(mark);
                assert_state(original, None);
                assert_eq!(check(0), Ok(()));
            });
        }

        #[test]
        fn snapshots_join_two_workers_before_parent_cancel_owner_returns() {
            let flag = Arc::new(AtomicBool::new(true));
            let parent = QueryBudget::cancelled_by(flag);
            with_budget(&parent, || {
                let snap = snapshot();
                let owner = std::thread::current().id();
                std::thread::scope(|scope| {
                    let a = scope.spawn(|| (std::thread::current().id(), snap.why(0), check(0)));
                    let b = scope.spawn(|| (std::thread::current().id(), snap.why(0), check(0)));
                    let a = a.join().unwrap();
                    let b = b.join().unwrap();
                    assert_ne!(a.0, owner);
                    assert_ne!(b.0, owner);
                    assert_ne!(a.0, b.0);
                    assert_eq!(a.1, Some("cancelled"));
                    assert_eq!(b.1, Some("cancelled"));
                    assert_eq!(a.2, Ok(()));
                    assert_eq!(b.2, Ok(()));
                });
            });
            assert_state(OFF, None);
        }

        #[test]
        fn public_extension_nested_query_preserves_outer_cancel() {
            use oxrdf::{Literal, Term};
            use sparq_core::Graph;
            use std::sync::atomic::AtomicUsize;
            for inner in 0..4 {
                let graph = Graph::load_str("", "turtle").unwrap();
                let nested = Graph::load_str("", "turtle").unwrap();
                let flag = Arc::new(AtomicBool::new(false));
                let calls = Arc::new(AtomicUsize::new(0));
                let callback_flag = Arc::clone(&flag);
                let callback_calls = Arc::clone(&calls);
                let owner = std::thread::current().id();
                let mut functions = crate::FunctionRegistry::new();
                functions.register("urn:nested", move |_| {
                    assert_eq!(std::thread::current().id(), owner);
                    callback_calls.fetch_add(1, Ordering::Relaxed);
                    match inner {
                        0 => {}
                        1 => {
                            assert_eq!(crate::query(&nested, "ASK {}").unwrap().rows.len(), 1);
                        }
                        2 => {
                            let child = QueryBudget {
                                max_rows: Some(1),
                                ..QueryBudget::unlimited()
                            };
                            assert_eq!(
                                crate::query_with_budget(&nested, "ASK {}", &child)
                                    .unwrap()
                                    .rows
                                    .len(),
                                1
                            );
                        }
                        _ => {
                            assert!(crate::query(&nested, "not SPARQL").is_err());
                        }
                    }
                    callback_flag.store(true, Ordering::Relaxed);
                    Ok(Term::Literal(Literal::from(7)))
                });
                let result = crate::query_with_functions_and_budget(
                    &graph,
                    "SELECT (<urn:nested>() AS ?value) WHERE {}",
                    &functions,
                    &QueryBudget::cancelled_by(flag),
                );
                assert_eq!(calls.load(Ordering::Relaxed), 1);
                assert_eq!(
                    result.unwrap_err(),
                    "query budget exceeded (cancelled)",
                    "inner={}",
                    inner
                );
                assert_eq!(crate::query(&graph, "ASK {}").unwrap().rows.len(), 1);
            }
        }
    }

    /// [SONNET-4.6] (sq-qk6ac) Direct tests for `Limits::why` — the pure gate the
    /// rayon-parallel loops poll. Its contract has two load-bearing halves: it agrees
    /// with `Limits::hit`, and its reason string is EXACTLY the one `check` would raise,
    /// so a worker that abandons its share of the work reports the same error the
    /// installing thread would (never a fabricated one, and never a guessed result).
    #[cfg(test)]
    mod snapshot_reason_tests {
        use super::*;
        use std::sync::Arc;

        /// Asserts that under `budget` the snapshot reports `want` at `rows`, that `hit`
        /// agrees, and that the reason is the very string `check` puts in its error.
        fn assert_reason(budget: &QueryBudget, rows: usize, want: &'static str) {
            with_budget(budget, || {
                let snap = snapshot();
                assert_eq!(snap.why(rows), Some(want), "wrong snapshot reason for {}", want);
                assert!(snap.hit(rows), "hit must agree with why for {}", want);
                assert_eq!(
                    check(rows),
                    Err(format!("query budget exceeded ({})", want)),
                    "the worker-visible reason must match the on-thread error for {}",
                    want
                );
            })
        }

        /// `why` and `hit` are one decision, and an unbudgeted snapshot never trips.
        #[test]
        fn unbudgeted_snapshot_has_no_reason() {
            let budget = QueryBudget::unlimited();
            with_budget(&budget, || {
                let snap = snapshot();
                assert_eq!(snap.why(0), None, "an unlimited budget must report no reason");
                assert_eq!(snap.why(usize::MAX), None, "no row cap ⇒ no reason at any row count");
                assert!(!snap.hit(usize::MAX), "hit must agree with why");
            })
        }

        /// Each limit reports ITS OWN reason, and the string matches `check`'s message.
        #[test]
        fn each_limit_reports_the_reason_check_would_raise() {
            let rows_capped = QueryBudget { max_rows: Some(4), ..QueryBudget::unlimited() };
            assert_reason(&rows_capped, 5, "max-rows");
            let bytes_capped =
                QueryBudget { max_bytes: Some(BYTES_PER_ID), ..QueryBudget::unlimited() };
            assert_reason(&bytes_capped, 2, "max-bytes");
            let cancelled = QueryBudget::cancelled_by(Arc::new(AtomicBool::new(true)));
            assert_reason(&cancelled, 0, "cancelled");
        }

        /// The wall-clock arm (native only: `Instant` does not exist in a wasm budget).
        #[test]
        #[cfg(not(target_arch = "wasm32"))]
        fn an_elapsed_deadline_reports_timeout() {
            let budget = QueryBudget {
                deadline: Some(std::time::Instant::now() - std::time::Duration::from_secs(1)),
                ..QueryBudget::unlimited()
            };
            assert_reason(&budget, 0, "timeout");
        }

        /// The reason a limit does NOT report: an under-limit row count leaves the
        /// snapshot clean, so the gate cannot abandon work a budget still admits.
        #[test]
        fn a_limit_not_yet_crossed_reports_nothing() {
            let budget = QueryBudget { max_rows: Some(4), ..QueryBudget::unlimited() };
            with_budget(&budget, || {
                let snap = snapshot();
                assert_eq!(snap.why(4), None, "a row count AT the cap is still admitted");
                assert_eq!(snap.why(5), Some("max-rows"), "one past the cap trips");
            })
        }

        /// The crux the parallel verdict loop depends on: a WORKER thread has no budget
        /// installed, so its thread-local poll is blind — only the captured snapshot can
        /// see the cancellation, and it names the same reason as the installing thread.
        #[test]
        fn snapshot_is_the_only_signal_a_worker_thread_can_see() {
            let flag = Arc::new(AtomicBool::new(true));
            let budget = QueryBudget::cancelled_by(Arc::clone(&flag));
            with_budget(&budget, || {
                let snap = snapshot();

                let (worker_poll, worker_reason) = std::thread::scope(|s| {
                    s.spawn(|| (check(0), snap.why(0))).join().expect("worker must not panic")
                });
                assert_eq!(worker_poll, Ok(()), "the thread-local budget is invisible to a worker");
                assert_eq!(
                    worker_reason,
                    Some("cancelled"),
                    "the captured snapshot must carry the cancellation across threads"
                );
                assert_eq!(check(0), Err("query budget exceeded (cancelled)".to_owned()));
            })
        }
    }

    #[cfg(test)]
    mod cancel_tests {
        use super::*;
        use sparq_core::Graph;
        use std::sync::{Arc, Barrier};
        use std::time::{Duration, Instant};

        #[test]
        fn cancel_flag_zero_vs_one_trips_both_poll_paths() {
            let flag = Arc::new(AtomicBool::new(false));
            let budget = QueryBudget::unlimited().with_cancel(Arc::clone(&flag));
            with_budget(&budget, || {

                assert!(!snapshot().hit(0), "false control must not trip the rayon snapshot");
                assert_eq!(check(0), Ok(()), "false control must not trip the local poll");

                flag.store(true, Ordering::Relaxed);
                assert!(snapshot().hit(0), "true flag must trip the rayon snapshot");
                assert_eq!(check(0), Err("query budget exceeded (cancelled)".to_owned()));
                assert_eq!(EXCEEDED.with(Cell::get), Some("cancelled"));
            })
        }

        #[test]
        fn cancel_from_another_thread_stops_large_query_promptly() {
            let mut nt = String::new();
            for i in 0..12_000 {
                nt.push_str(&format!("<http://ex/s{i}> <http://ex/p> <http://ex/o{i}> .\n"));
            }
            let graph = Graph::load_str(&nt, "ntriples").expect("test graph parses");
            let flag = Arc::new(AtomicBool::new(false));
            let budget = QueryBudget::unlimited().with_cancel(Arc::clone(&flag));
            let started = Arc::new(Barrier::new(2));
            let worker_started = Arc::clone(&started);

            let worker = std::thread::spawn(move || {
                worker_started.wait();
                crate::query_with_budget(
                    &graph,
                    "SELECT ?s ?x WHERE { ?s <http://ex/p> ?o . ?x <http://ex/p> ?y }",
                    &budget,
                )
                .map(|_| ())
            });

            started.wait();
            let cancelled_at = Instant::now();
            flag.store(true, Ordering::Relaxed);
            let result = worker.join().expect("query worker must not panic");
            assert_eq!(result, Err("query budget exceeded (cancelled)".to_owned()));
            assert!(
                cancelled_at.elapsed() < Duration::from_secs(5),
                "cancelled query did not return within the cooperative bound"
            );
        }

        #[test]
        fn cancelled_query_does_not_leak_flag_into_next_query_on_same_thread() {
            let graph = Graph::load_str("<http://ex/s> <http://ex/p> <http://ex/o> .", "ntriples")
                .expect("test graph parses");
            let cancelled = QueryBudget::cancelled_by(Arc::new(AtomicBool::new(true)));
            let query = "SELECT * WHERE { ?s ?p ?o }";

            assert_eq!(
                crate::query_with_budget(&graph, query, &cancelled).map(|_| ()),
                Err("query budget exceeded (cancelled)".to_owned())
            );
            let clean = crate::query_with_budget(&graph, query, &QueryBudget::unlimited())
                .expect("guard must clear the stale cancellation pointer");
            assert_eq!(clean.len(), 1);
        }
    }

}
}
