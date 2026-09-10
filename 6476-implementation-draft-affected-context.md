
### exec.rs lines 68-505
```rust
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
            ACTIVE.with(|a| a.set(self.previous));
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
```

### exec.rs lines 505-760
```rust
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

```

### exec.rs lines 2448-2478
```rust
impl sjoin::Budget for EngineBudget {
    #[inline]
    fn exhausted(&self, rows: usize) -> bool {
        budget::exhausted(rows)
    }
}

/// [OPUS-4.8] sq-hknqs: the parallel hash-join's per-worker exhaustion snapshot. Wraps the
/// flattened [`budget::Limits`] (the installing thread's sticky flag is invisible to rayon
/// workers) so a worker that hits the limits stops adding to its accumulator; the caller's next
/// on-thread check raises the actual error. Generic ([`sjoin::BudgetSnapshot`]), so the rayon
/// fold carries no vtable.
#[cfg(feature = "parallel")]
struct EngineSnapshot(budget::Limits);

#[cfg(feature = "parallel")]
impl sjoin::BudgetSnapshot for EngineSnapshot {
    #[inline]
    fn hit(&self, rows: usize) -> bool {
        self.0.hit(rows)
    }
}

/// Ids at or above this base index into the per-query [`LocalVocab`] instead of the graph
/// dictionary. It sits ABOVE the dictionary range `[1, INLINE_BASE)` and the inline-integer
/// range `[INLINE_BASE, INLINE_BASE + 2^30)`, i.e. at `INLINE_BASE + 2^30 = 3·2^30`, leaving
/// the local vocab `[3·2^30, 2^32)` (≈1.07B query-computed terms — far more than any query).
const LOCAL_BASE: Id = dict::INLINE_BASE + (1 << 30);

#[inline]
fn is_local(id: Id) -> bool {
```

### exec.rs lines 2820-2910
```rust
    // [OPUS-4.8] roborev 1538 / sq-7d3dj.10 (audit item 6): fan the JSON serialize out
    // across cores when the installed budget cannot be violated by doing so — NO budget,
    // or a DEADLINE-ONLY budget (the default HTTP server's 30s timeout, which has no
    // row/byte cap). The fan-out builds every matching fragment before it can know a row
    // or byte count, so a ROW / BYTE cap stays on the cooperative serial loop below
    // (which checks `budget::exhausted` every 1024 rows and stops early). A deadline-only
    // budget is admitted because the coarse `limits.hit(0)` re-check at each par-chunk
    // boundary stops launching new chunks once the wall-clock deadline passes, bounding
    // the overrun to ~one chunk per worker. A blanket !budget-active → true flip is
    // REJECTED (see `parallel_json_fanout`).
    #[cfg(feature = "parallel")]
    if scan_rows.len() >= PAR_THRESHOLD {
        if let Some(limits) = budget::parallel_json_fanout() {
            use rayon::prelude::*;
            // One string per chunk (≈ per worker), not per row — avoids one heap
            // allocation per result cell. Chunks stay in order, so on the success path
            // the bytes are identical to the serial path.
            let chunk = scan_rows.len().div_ceil(rayon::current_num_threads() * 4).max(1);
            let frags: Vec<(usize, String)> = scan_rows
                .par_chunks(chunk)
                .map(|rows| {
                    // Coarse deadline re-check at the chunk boundary: once the wall-clock
                    // deadline has passed, every later chunk produces nothing, so at most
                    // the chunks already in flight (~one per worker) run to completion.
                    // The installing thread's post-fan-out gate turns the passed deadline
                    // into the timeout error, discarding this (now partial) result — so a
                    // skipped chunk NEVER escapes as a truncated body. Under no budget / an
                    // unexpired deadline this is one non-tripping `Instant` read per chunk.
                    if limits.hit(0) {
                        return (0usize, String::new());
                    }
                    let mut n = 0usize;
                    let mut f = String::new();
                    for row in rows {
                        if !passes(row) {
                            continue;
                        }
                        if !f.is_empty() {
                            f.push(',');
                        }
                        n += 1;
                        write_row(row, &mut f);
                    }
                    (n, f)
                })
                .collect();
            // Budget gate on the installing thread over the total row count — and, for a
            // deadline-only budget, the now-past wall clock: sets the sticky flag the
            // caller's `budget::check(0)` converts into the budget error (a chunk skipped
            // above means the deadline is globally past, so this fires deterministically).
            let _ = budget::exhausted(frags.iter().map(|(n, _)| n).sum());
            // Accumulate into `pending` and hand a chunk to `emit` at each flush boundary
            // (byte-identical concatenation to the old `emit_chunk` Vec layout — only the
            // chunk *boundaries* differ, and the concat is what the byte-identity contract
            // covers). `s` already holds the head.
            let mut pending = s;
            let mut wrote = false;
            for (_, f) in frags {
                if f.is_empty() {
                    continue;
                }
                if wrote {
                    pending.push(',');
                }
                wrote = true;
                pending.push_str(&f);
                if flush.is_some_and(|n| pending.len() >= n)
                    && emit(std::mem::take(&mut pending)).is_break()
                {
                    return Some(());
                }
            }
            pending.push_str("]}}");
            let _ = emit(pending);
            return Some(());
        }
    }
    let mut written = 0usize;
    for (i, row) in scan_rows.iter().enumerate() {
        // Coarse budget check every 1024 scanned rows; the caller's sticky check
        // turns an early stop into the budget error (never a truncated result).
        if i & 1023 == 0 && budget::exhausted(written) {
            break;
        }
        if !passes(row) {
            continue;
        }
        if written > 0 {
            s.push(',');
        }
        written += 1;
```

### exec.rs lines 3020-3090
```rust
    let mut s = head;
    // [SONNET-4.6] (sq-yfcu2) The serialize loops below are budget-checked too: the
    // pre-serialize `budget::check` above prices the ROW / BYTE caps exactly (the rows
    // are already materialised, so the count is known — unlike the single-pattern
    // streaming path, which is why this path may fan out under any budget), but
    // serialising a large materialised set is itself unbounded WORK, so a DEADLINE (or a
    // cancellation) can fall due *during* it. Both branches therefore re-check the budget
    // mid-serialize and gate the final chunk on `budget::check` — a late-but-complete
    // result is reported as the budget error, never returned as if it were in time.
    #[cfg(feature = "parallel")]
    if bindings.rows.len() >= PAR_THRESHOLD {
        use rayon::prelude::*;
        // Limit snapshot the workers re-check at each par-chunk boundary (the installing
        // thread's sticky flag is out of reach inside rayon).
        let limits = budget::snapshot();
        // One string per chunk (≈ per worker), not per row. Chunks stay in order → identical bytes.
        let chunk = bindings.rows.len().div_ceil(rayon::current_num_threads() * 4).max(1);
        let frags: Vec<String> = bindings
            .rows
            .par_chunks(chunk)
            .map(|rows| {
                // Coarse deadline/cancel re-check at the chunk boundary: once the budget is
                // past, every later chunk produces nothing, so at most the chunks already in
                // flight (~one per worker) run to completion. The post-fan-out gate below
                // turns that into the budget error and discards this (now partial) result —
                // a skipped chunk never escapes as a truncated body. Under no budget this is
                // one non-tripping read per chunk.
                if limits.hit(0) {
                    return String::new();
                }
                let mut f = String::new();
                for (k, row) in rows.iter().enumerate() {
                    if k > 0 {
                        f.push(',');
                    }
                    write_row(row, &mut f);
                }
                f
            })
            .collect();
        // Accumulate into `s` (which already holds the head) and hand a chunk to `emit` at
        // each flush boundary. The concatenation is byte-identical to the old `emit_chunk`
        // Vec layout; only the chunk boundaries differ. A skipped (empty) fragment is
        // dropped rather than separated by a comma; every non-skipped chunk holds at least
        // one row object, so on the untripped path the `wrote` flag is exactly `i > 0`.
        let mut wrote = false;
        for f in frags {
            if f.is_empty() {
                continue;
            }
            if wrote {
                s.push(',');
            }
            wrote = true;
            s.push_str(&f);
            if flush.is_some_and(|n| s.len() >= n) && emit(std::mem::take(&mut s)).is_break() {
                return Ok(());
            }
        }
        // Post-serialization gate: a deadline that fell due (or a cancellation raised)
        // while the fan-out ran is the query's answer, not this now-late result.
        budget::check(bindings.rows.len())?;
        s.push_str("]}}");
        let _ = emit(s);
        return Ok(());
    }
    for (i, row) in bindings.rows.iter().enumerate() {
        // Coarse re-check every 1024 serialised rows; the post-loop gate turns the early
        // stop into the budget error, so a truncated body is never returned as success.
        if i & 1023 == 0 && budget::exhausted(bindings.rows.len()) {
            break;
```

### exec.rs lines 5610-5665
```rust
    let query = format!("SELECT * WHERE {{ {inner} }}");

    // [FABLE-5] (sq-my8wd.4) STREAMING consumption: each remote row is interned to a
    // compact id-level `Row` AS IT IS PARSED (the owned terms are dropped immediately)
    // instead of collecting the whole remote relation as `Term`s first. Result-identical
    // to the collect-then-intern path — same rows, multiplicity and order (pinned by the
    // service.rs `streaming_equivalence` tests) — but the per-response peak memory is
    // the response body plus the id-level relation the join needs anyway, not a
    // whole-document DOM plus a second term-level copy.
    //
    // [OPUS-4.8] (sq-my8wd.5) READER-SEAM: use `eval_remote_into_read` so the HTTP body
    // is consumed as a STREAM (never buffered as a full `String`). Peak memory now stays
    // BELOW the response body size (not just O(body)) — the body String is eliminated.
    // Test transports are wrapped via `TransportAsReader` so all existing tests pass.
    let mut id_rows: Vec<Row> = Vec::new();
    // [OPUS-4.8] (sq-my8wd.4) Savepoint the local vocab + byte budget BEFORE streaming so
    // a SILENT error can roll the partially-interned rows back out — see the SILENT arm.
    let vocab_mark = local.savepoint();
    let byte_mark = budget::byte_savepoint();
    let fetched = service_reader_transport::with(|t| {
        sparq_engine_service::service::eval_remote_into_read(t, &endpoint, &query, &mut |row| {
            id_rows.push(intern_remote_row(graph, local, &row));
            Ok(())
        })
    });
    match fetched {
        Ok(vars) => Ok(Bindings::unsorted(vars, id_rows)),
        Err(e) if silent => {
            // SILENT: swallow the error, keep the surrounding bindings. Rows already
            // interned from a partially-parsed response are discarded with `id_rows` —
            // and, so the discard is behaviour-NEUTRAL with the pre-streaming
            // collect-then-intern-on-success path (which interned nothing on a swallowed
            // error), we ROLL the interned terms out of the local vocab and REFUND the
            // byte budget they charged. Without this a partial stream would retain memory
            // and could trip `max_bytes`, turning a query the old path answered into a
            // "query budget exceeded" error. [OPUS-4.8] (sq-my8wd.4)
            let _ = e;
            id_rows.clear();
            local.rollback_to(vocab_mark);
            budget::restore_bytes(byte_mark);
            Ok(identity())
        }
        Err(e) => Err(e),
    }
}

/// Answer `SERVICE <iri> { inner }` from a LOCAL in-process handler, if one is
/// registered for `iri`. [OPUS-5] (sq-lsp7k.2.2)
///
/// Returns:
/// * `Ok(Some(rel))` — a handler served the IRI; `rel` is its rows interned against
///   this query's dictionaries (exactly as `VALUES` and the remote SERVICE relation
///   are), ready for the caller's ordinary join.
/// * `Ok(None)` — no handler applies (a variable endpoint, or an IRI absent from the
///   registry / no registry installed). The caller proceeds EXACTLY as it did before
///   this feature existed: the `service` HTTP path, egress allowlist included.
```

### exec.rs lines 5702-5763
```rust
    let NamedNodePattern::NamedNode(iri) = name else {
        return Ok(None);
    };
    let Some(handler) = local_services::lookup(iri.as_str()) else {
        return Ok(None);
    };

    // The in-scope variables of the SERVICE group, first-occurrence order — the same
    // set the bind-join computes for its VALUES head.
    let mut vars: Vec<Variable> = Vec::new();
    inner.on_in_scope_variable(|v| {
        if !vars.contains(v) {
            vars.push(v.clone());
        }
    });
    // Byte-for-byte the string the HTTP transport would have sent for this SERVICE.
    let query = format!("SELECT * WHERE {{ {} }}", inner);
    let patterns = local_service_patterns(inner);
    let req = crate::LocalServiceRequest {
        service: iri.as_str(),
        query: &query,
        vars: &vars,
        patterns: &patterns,
    };

    let vocab_mark = local.savepoint();
    let byte_mark = budget::byte_savepoint();
    let produced = handler(&req).and_then(|rows| {
        rows.validate()
            .and_then(|()| {
                // The returned relation may only name columns the SERVICE group itself
                // puts in scope (see the doc comment): anything else would join with a
                // same-named variable OUTSIDE the group. [SONNET-4.6]
                match rows.vars.iter().find(|v| !vars.contains(v)) {
                    Some(v) => Err(format!(
                        "variable ?{} is not in scope in the SERVICE group",
                        v.as_str()
                    )),
                    None => Ok(()),
                }
            })
            .map_err(|e| {
                format!("local SERVICE <{}> returned an invalid relation: {}", iri.as_str(), e)
            })?;
        let id_rows: Vec<Row> =
            rows.rows.iter().map(|r| intern_remote_row(graph, local, r)).collect();
        budget::check(id_rows.len())?;
        Ok(Bindings::unsorted(rows.vars, id_rows))
    });
    match produced {
        Ok(b) => Ok(Some(b)),
        Err(e) if silent => {
            let _ = e;
            local.rollback_to(vocab_mark);
            budget::restore_bytes(byte_mark);
            Ok(Some(Bindings::unsorted(Vec::new(), vec![Row::new()])))
        }
        Err(e) => Err(e),
    }
}

/// Decompose a SERVICE group into the neutral triple-pattern view handed to a local
```

### exec.rs lines 5950-6015
```rust
    }

    // Render the inner pattern once; each block re-uses it with a fresh VALUES head.
    let inner_sparql = format!("{inner}");
    let block = sparq_engine_service::service::bind_block_size();

    // Accumulate the union of the per-block remote relations, interning each row to the
    // id level AS IT ARRIVES from the streaming parser ([FABLE-5] sq-my8wd.4) — no
    // block's relation is ever held as owned `Term` rows. All blocks share the remote
    // `head.vars`, so we keep the first block's var list and concatenate rows
    // positionally: the same accumulation, and the same row order, as the previous
    // collect-then-intern path.
    let mut acc_vars: Option<Vec<Variable>> = None;
    let mut acc_rows: Vec<Row> = Vec::new();
    // [OPUS-4.8] (sq-my8wd.4) Savepoint the local vocab + byte budget BEFORE the first
    // block so a SILENT failure in ANY block can roll EVERY block's interns back out —
    // a SILENT failure discards all blocks' rows together (see the SILENT arm), so the
    // interns must all be rolled back too, or the discarded stream would retain memory
    // and charge `max_bytes`.
    let vocab_mark = local.savepoint();
    let byte_mark = budget::byte_savepoint();

    for chunk in tuples.chunks(block) {
        let values = sparq_engine_service::service::render_values_block(&join_vars, chunk);
        // Inject the VALUES inside the SELECT * group, alongside the inner pattern, so
        // the remote inner-joins the pushed bindings with its pattern.
        let query = format!("SELECT * WHERE {{ {values} {inner_sparql} }}");
        // [OPUS-4.8] (sq-my8wd.5) Use the reader seam here too: the bound-join path
        // fetches one block per VALUES chunk; each block's body is streamed, not buffered.
        let fetched = service_reader_transport::with(|t| {
            sparq_engine_service::service::eval_remote_into_read(t, endpoint, &query, &mut |row| {
                acc_rows.push(intern_remote_row(graph, local, &row));
                Ok(())
            })
        });
        match fetched {
            Ok(vars) => {
                if acc_vars.is_none() {
                    acc_vars = Some(vars);
                }
            }
            // SILENT: a failed block means the SERVICE as a whole must behave EXACTLY
            // as the verbatim single-request SILENT path — which yields the JOIN
            // IDENTITY (a single empty solution) so the surrounding bindings are KEPT
            // unchanged. We therefore discard any partial block results and hand the
            // caller the identity relation (one zero-column row); joining / left-outer
            // joining `left` with it leaves `left` exactly as it was. This matches the
            // unbound-then-local-join path's SILENT semantics precisely. [OPUS-4.8]
            // (Rows already interned from earlier blocks — or a partially-parsed
            // failing block — are ROLLED BACK out of the local vocab below, and their
            // byte-budget charge refunded, so no stray entry or `max_bytes` charge
            // survives the discard. [OPUS-4.8] sq-my8wd.4)
            Err(_) if silent => {
                acc_rows.clear();
                local.rollback_to(vocab_mark);
                budget::restore_bytes(byte_mark);
                return Ok(Some(Bindings::unsorted(Vec::new(), vec![Row::new()])));
            }
            Err(e) => return Err(e),
        }
    }

    // Vars: a non-empty `tuples` always produced at least one successful block above
    // (failures returned early), so `acc_vars` is set unless every block returned an
    // EMPTY result with no head — in which case the join vars are a safe head (the
    // relation has zero rows, so the var list only names columns that, being the join
```

### exec.rs lines 9085-9130
```rust
    } else {
        vec![sjoin::build_table(&build.rows, &keys)]
    };
    #[cfg(not(feature = "parallel"))]
    let tables = vec![sjoin::build_table(&build.rows, &keys)];
    // The probe is read-only over the (partitioned) table, so for a large probe side build the
    // output in parallel on native.
    #[cfg(feature = "parallel")]
    if probe.rows.len() >= PAR_THRESHOLD {
        use rayon::prelude::*;
        // Budget snapshot for the workers (the installing thread's thread-local is
        // invisible to them): a worker that hits the limits stops adding to its own
        // accumulator; the caller's next on-thread check raises the actual error.
        let snap = EngineSnapshot(budget::snapshot());
        let rows: Vec<Row> = probe
            .rows
            .par_iter()
            .fold(Vec::new, |mut acc, prow| {
                if !sjoin::BudgetSnapshot::hit(&snap, acc.len()) {
                    sjoin::probe_emit(prow, &keys, &build.rows, &tables, &probe_only, &mut acc);
                }
                acc
            })
            .reduce(Vec::new, |mut a, mut b| {
                a.append(&mut b);
                a
            });
        let _ = budget::exhausted(rows.len()); // sticky gate on the combined size
        return Bindings::unsorted(out_vars, rows);
    }
    let mut rows = Vec::new();
    sjoin::hash_probe_serial(&probe.rows, &keys, &build.rows, &tables, &probe_only, &EngineBudget, &mut rows);
    Bindings::unsorted(out_vars, rows)
}

/// Index-nested-loop join of a (small) `result` with a single triple pattern on one
/// shared variable: groups the result by the join value, and for each distinct value
/// looks up the pattern's matches with that variable BOUND (a binary-search range on a
/// permutation index) — so a large, selective pattern is never fully scanned. The
/// pattern must have distinct variables; a pushed-down sargable filter is applied inline.
fn bind_join(
    graph: &Graph,
    result: Bindings,
    id_pat: &IdPattern,
    pos_vars: &[Option<Variable>; 3],
    rk: usize,
```

### exec.rs lines 10045-10103
```rust
        // the snapshot exactly like the FILTER / BIND parallel paths.
        //
        // [SONNET-4.6] (sq-qk6ac) An exhausted budget must block the VERDICT loop, not
        // just the output build: one verdict is a whole probe of `B` (a bucket scan plus
        // 3-valued expression evaluation — a WHOLE-`B` scan for a literal-keyed row), so a
        // timed-out / cancelled query used to grind every remaining left row only for the
        // build below to discard the lot on its FIRST `budget::exhausted` check. A rayon
        // worker cannot see the installing thread's sticky flag, so it re-checks a
        // captured `Limits` snapshot (the parallel hash-join / JSON-serialize pattern) and,
        // when hit, returns the CANONICAL budget error — `collect` into `Result`
        // short-circuits, so the queued probes are abandoned. Raising the error rather
        // than guessing a placeholder verdict also means a skipped row can never escape as
        // a silently truncated result, and it is behaviour-identical on the observable
        // path: today the build truncates to nothing and the caller's operator-exit
        // `budget::check` raises this same message, just after the wasted work.
        #[cfg(feature = "parallel")]
        if left_b.rows.len() >= PAR_THRESHOLD {
            use rayon::prelude::*;
            let limits = budget::snapshot();
            let fns = functions::snapshot();
            let vw = view::snapshot();
            let spx = spatial::snapshot();
            // [OPUS-5] (sq-lsp7k.2.2) Keep LOCAL SERVICE handlers visible on the worker too:
            // a worker that missed the registry would dial the IRI instead of answering it.
            #[cfg(feature = "service-local")]
            let lsv = local_services::snapshot();
            #[cfg(not(target_arch = "wasm32"))]
            let qn = query_now::snapshot(); // sq-98w7z.1: keep NOW() pinned on workers
            let verdicts: Vec<bool> = left_b
                .rows
                .par_iter()
                .map(|lrow| {
                    // One non-tripping `Instant` read per row under a deadline budget, and
                    // a single `on` test when no budget is installed.
                    if let Some(why) = limits.why(0) {
                        return Err(format!("query budget exceeded ({})", why));
                    }
                    let _fns = functions::worker_install(&fns);
                    let _vw = view::worker_install(&vw);
                    let _spx = spatial::worker_install(&spx);
                    #[cfg(feature = "service-local")]
                    let _lsv = local_services::worker_install(&lsv);
                    #[cfg(not(target_arch = "wasm32"))]
                    let _qn = query_now::worker_install(qn);
                    eliminated(lrow)
                })
                .collect::<Result<Vec<bool>, String>>()?;

            // Serial ordered build + budget truncation: identical to the serial probe
            // loop's `if !matched { push } ; break on budget` — the survivor prefix and
            // its order are reproduced exactly whether the verdicts were computed
            // serially or in parallel.
            for (lrow, &elim) in left_b.rows.iter().zip(&verdicts) {
                if budget::exhausted(result_rows.len()) {
                    break;
                }
                if !elim {
                    let mut combined: Row = lrow.clone();
                    combined.extend(std::iter::repeat_n(NO_ID, n_right_only));
```
