# Issue 6476 — resource-stopped implementation draft

Not a validated or review-ready candidate. No engine compile, new test execution, mutation control or sealing result is claimed. Existing baseline witness and independent design review remain frozen in sibling directories. Complete affected caller/snapshot/SERVICE excerpts are in affected-context.md; all seven edited source files are under source/.

## Report
```json
{
  "issue": 6476,
  "phase": "implementation first bounded phase; resource-stopped before engine compilation",
  "base": "d41ec9fcb796504d85f5a5247a5fdc9eb3e65de5",
  "model": "GPT-6 Astra xhigh",
  "source_status": "uncommitted draft; not validated or review-ready",
  "implemented": [
    "Private Guard/install; crate-visible synchronous FnOnce boundary; full Limits and EXCEEDED restored by RAII",
    "14 production callers migrated; exact unchanged-body audit passed for all six caller files; view guards remain outside and trace guards inside",
    "Limits/snapshot/fanout/hit/why limited to exec; owning-frame safety invariant documented",
    "Seven new test functions cover nested state, sticky errors, distinct cancellation, deadline, SERVICE savepoint, two scoped workers, and real public callback cases"
  ],
  "static_checks": {
    "diff_check": "pass",
    "preflight": "exit1: existing Bash3 privacy checker mapfile unavailable; no other finding",
    "caller_scope_audit": "14sites match canonical mechanical wrapping"
  },
  "build": {
    "command": [
      "/Users/jesght/.rustup/toolchains/1.97.1-aarch64-apple-darwin/bin/cargo",
      "build",
      "--release",
      "--locked",
      "--offline",
      "-p",
      "sparq-engine",
      "--lib",
      "-vv"
    ],
    "exit_code": -15,
    "seconds": 26.463041292,
    "min_free": 6504886272,
    "stop": "disk-floor",
    "env": {
      "CARGO_BUILD_JOBS": "2",
      "CARGO_INCREMENTAL": "0",
      "CARGO_NET_OFFLINE": "true",
      "CARGO_TARGET_DIR": "/private/tmp/sparq-pr6049/.throughput-monitor/direct-5983/implementation/target",
      "RUSTC": "/Users/jesght/.rustup/toolchains/1.97.1-aarch64-apple-darwin/bin/rustc"
    }
  },
  "fresh_engine_compiled": false,
  "new_tests_executed": 0,
  "compiled_mutation_controls_executed": 0,
  "resource_limit": "Stop at free <6509559808B; first release build recompiled dependencies because workspace profile is panic=abort/LTO/codegen1, while warm diagnostic cache is unwind/no-LTO/codegen16. No compiler source failure established. No cleanup by agent.",
  "service_audit": {
    "sites": [
      {
        "capture": 5628,
        "restore": 5649,
        "path": "remote verbatim synchronous streaming"
      },
      {
        "capture": 5728,
        "restore": 5756,
        "path": "local handler, can synchronously invoke a public query"
      },
      {
        "capture": 5970,
        "restore": 6005,
        "path": "remote bound-join synchronous chunk loop"
      }
    ],
    "argument": "Savepoint owns only extra_bytes and sticky-error values, not raw cancellation pointers. A nested with_budget has completed and restored the parent before the surrounding parent handler result is interned or its speculative burst is rolled back. No current production restore runs while a different budget frame is active. Preserve parent max_rows/max_bytes/width/deadline/cancel on rollback. Deadline/cancel may trip anew at next poll.",
    "dynamic_proof": "New private savepoint test drafted; not executed. Existing SERVICE suites remain required."
  },
  "snapshot_audit": {
    "sites": [
      2832,
      3034,
      9098,
      10063
    ],
    "argument": "All four production fanouts synchronously collect/fold/reduce before enclosing evaluation returns. Parent budget owner remains borrowed throughout. Workers do not install parent TLS. Limits remains a lifetime-free Copy trusted within exec: visibility restriction reduces surface but is not a type-level lifetime proof against arbitrary future escapes.",
    "dynamic_proof": "New two-thread snapshot test is a scoped-module witness, not proof of production Rayon branch engagement; not executed yet."
  },
  "next_checks": [
    "Build exact draft with cached unwind/no-LTO/codegen16 native profile after reserve recovers; freshlink original 4-case callback harness and require all true",
    "Run full focused actual budget module tests and relevant no-parallel/SERVICE-local/params/vectorized/cs-planner paths as affordable; top-level unwind cleanup deserves an explicit additional assertion",
    "Run omit-Limits, omit-sticky, omit-byte restoration mutants; internal sibling-module sealing compile-fail plus visibility-open compile-only control; never execute dangling-pointer sequences",
    "Actual >1-worker production Rayon snapshot path witness and scoped clippy pending",
    "Full Linux workspace/wasm/ratchet gates remain authoritative after reviewed publication"
  ],
  "holds": "No remote changes; 6095/5357 holds and registry OFF preserved",
  "known_followups": [
    "No claim of measured performance, successful source compilation, fresh regression green, or executed feature coverage. Prior frozen public witness still proves baseline defect only."
  ],
  "source_files": {
    "crates/sparq-engine/src/cache.rs": "ce140e15be0ad8f72584dccca0d33eb93acc526756f8cb1a2ec69e872c4da62e",
    "crates/sparq-engine/src/exec.rs": "207f3abe106301d11a9d57da212b7026612e53bd17c1b4b4a03072a7cd5ca247",
    "crates/sparq-engine/src/lib.rs": "074740149d0649b0334d1814ca289feaad54070ca0d90bf5737601f7021a7538",
    "crates/sparq-engine/src/update.rs": "5f638ac92382082e6a65080eb579ee61ce0861d843805a761236b73d81e842fe",
    "crates/sparq-engine/src/explain.rs": "953c36345e5e54a99b04478a2515b4ea2ba06cdd6ebffe9f376153e1ef0bffd4",
    "crates/sparq-engine/src/explain_json.rs": "59951ec3db080ec591697398e89586b1d29e5d6377883bfa1a3400efc807470d",
    "crates/sparq-engine/src/construct.rs": "8b4c28da9fd8cb0b2d6f22439113ebe5f89b2b42c8938c02a0e6ef6ddbd3f6e5"
  },
  "final_free_bytes": 5761134592,
  "production_edits_after_build_attempt": false
}

```

## Exact draft diff
```diff
diff --git a/crates/sparq-engine/src/cache.rs b/crates/sparq-engine/src/cache.rs
index 805e46f68..163b4fa14 100644
--- a/crates/sparq-engine/src/cache.rs
+++ b/crates/sparq-engine/src/cache.rs
@@ -365,18 +365,19 @@ fn eval(
     let active = crate::active_dataset(graph, query);
     let graph = active.as_ref().unwrap_or(graph);
     let _view_scope = crate::view_scope(&active);
-    let _guard = exec::budget::install(budget);
-    exec::set_query_base(query.base_iri().map(|b| b.as_str()));
-    match query {
-        Query::Select { pattern, .. } => exec::eval_select(graph, pattern),
-        Query::Ask { pattern, .. } => Ok(QueryResult {
-            vars: Vec::new(),
-            rows: if exec::eval_ask(graph, pattern)? {
-                vec![Vec::new()]
-            } else {
-                Vec::new()
-            },
-        }),
-        _ => Err("result cache only stores SELECT and ASK queries".into()),
-    }
+    exec::budget::with_budget(budget, || {
+        exec::set_query_base(query.base_iri().map(|b| b.as_str()));
+        match query {
+            Query::Select { pattern, .. } => exec::eval_select(graph, pattern),
+            Query::Ask { pattern, .. } => Ok(QueryResult {
+                vars: Vec::new(),
+                rows: if exec::eval_ask(graph, pattern)? {
+                    vec![Vec::new()]
+                } else {
+                    Vec::new()
+                },
+            }),
+            _ => Err("result cache only stores SELECT and ASK queries".into()),
+        }
+    })
 }
diff --git a/crates/sparq-engine/src/construct.rs b/crates/sparq-engine/src/construct.rs
index 0e64c0923..6cc1e7e47 100644
--- a/crates/sparq-engine/src/construct.rs
+++ b/crates/sparq-engine/src/construct.rs
@@ -57,9 +57,10 @@ pub fn construct_prepared_with_budget(
     let _view_scope = crate::view_scope(&active);
     match q {
         Query::Construct { template, pattern, .. } => {
-            let _guard = crate::exec::budget::install(budget);
-            let solutions = crate::exec::eval_select(graph, pattern)?;
-            Ok(instantiate(template, &solutions))
+            crate::exec::budget::with_budget(budget, || {
+                let solutions = crate::exec::eval_select(graph, pattern)?;
+                Ok(instantiate(template, &solutions))
+            })
         }
         _ => Err("construct() requires a CONSTRUCT query".into()),
     }
@@ -93,9 +94,10 @@ pub fn describe_prepared_with_budget(
     let _view_scope = crate::view_scope(&active);
     match q {
         Query::Describe { pattern, .. } => {
-            let _guard = crate::exec::budget::install(budget);
-            let solutions = crate::exec::eval_select(graph, pattern)?;
-            cbd(graph, &solutions)
+            crate::exec::budget::with_budget(budget, || {
+                let solutions = crate::exec::eval_select(graph, pattern)?;
+                cbd(graph, &solutions)
+            })
         }
         _ => Err("describe() requires a DESCRIBE query".into()),
     }
@@ -120,18 +122,19 @@ pub fn construct_or_describe_with_budget(
     let active = crate::active_dataset(graph, &q);
     let graph = active.as_ref().unwrap_or(graph);
     let _view_scope = crate::view_scope(&active);
-    let _guard = crate::exec::budget::install(budget);
-    match q {
-        Query::Construct { template, pattern, .. } => {
-            let solutions = crate::exec::eval_select(graph, &pattern)?;
-            Ok(instantiate(&template, &solutions))
-        }
-        Query::Describe { pattern, .. } => {
-            let solutions = crate::exec::eval_select(graph, &pattern)?;
-            cbd(graph, &solutions)
+    crate::exec::budget::with_budget(budget, || {
+        match q {
+            Query::Construct { template, pattern, .. } => {
+                let solutions = crate::exec::eval_select(graph, &pattern)?;
+                Ok(instantiate(&template, &solutions))
+            }
+            Query::Describe { pattern, .. } => {
+                let solutions = crate::exec::eval_select(graph, &pattern)?;
+                cbd(graph, &solutions)
+            }
+            _ => Err("construct_or_describe() requires a CONSTRUCT or DESCRIBE query".to_string()),
         }
-        _ => Err("construct_or_describe() requires a CONSTRUCT or DESCRIBE query".to_string()),
-    }
+    })
 }
 
 /// Executes a CONSTRUCT *or* DESCRIBE query and serialises the resulting graph as
diff --git a/crates/sparq-engine/src/exec.rs b/crates/sparq-engine/src/exec.rs
index ae52aac00..d239a258a 100644
--- a/crates/sparq-engine/src/exec.rs
+++ b/crates/sparq-engine/src/exec.rs
@@ -74,19 +74,21 @@ pub(crate) mod budget {
 
     /// Copyable view of a cancellation flag owned by the installed [`QueryBudget`].
     ///
-    /// The [`Guard`] lifetime keeps that budget (and therefore its `Arc<AtomicBool>`)
-    /// alive until the pointer has been cleared from the thread-local state. Rayon
-    /// snapshots are consumed only by scoped parallel iterators that join before the
-    /// guard is dropped. The pointer is dereferenced only for atomic loads.
+    /// [GPT-6 Astra] ACTIVE belongs to the innermost live `with_budget` frame.
+    /// Each pointer is owned by a QueryBudget borrowed by a still-live frame on
+    /// this thread's stack. Private installation/restoration enforces nesting:
+    /// a restored parent's borrow outlives the child. Only the innermost frame
+    /// writes ACTIVE. Snapshots are used only in synchronous parallel work that
+    /// joins before the owning frame returns; dereferences are atomic loads.
     #[derive(Clone, Copy)]
     struct CancelPtr(NonNull<AtomicBool>);
 
     // SAFETY: `AtomicBool` is `Sync`; moving this shared pointer to a worker is
-    // sound because it is only dereferenced for atomic loads while `Guard` keeps
-    // the owning `Arc` alive, including across scoped rayon work.
+    // sound because only atomic loads occur while the owning `with_budget` frame
+    // borrows its QueryBudget, including until scoped rayon work joins.
     unsafe impl Send for CancelPtr {}
     // SAFETY: `AtomicBool` is `Sync`; all shared access through `CancelPtr` is an
-    // atomic load, and `Guard` keeps the allocation alive until worker joins finish.
+    // atomic load; the owning `with_budget` frame outlives all worker joins.
     unsafe impl Sync for CancelPtr {}
 
     /// Bytes one id-level binding cell occupies in a materialised `Row`. The
@@ -98,7 +100,7 @@ pub(crate) mod budget {
 
     /// The installed limits, flattened for a cheap per-check read.
     #[derive(Clone, Copy)]
-    pub(crate) struct Limits {
+    pub(in crate::exec) struct Limits {
         on: bool,
         #[cfg(not(target_arch = "wasm32"))]
         deadline: Option<std::time::Instant>,
@@ -136,7 +138,8 @@ pub(crate) mod budget {
         /// byte size compared against `max_bytes`. [OPUS-4.8] (sq-s5is)
         #[inline]
         fn bytes(&self, rows: usize) -> usize {
-            rows.saturating_mul(self.byte_width).saturating_add(self.extra_bytes)
+            rows.saturating_mul(self.byte_width)
+                .saturating_add(self.extra_bytes)
         }
 
         /// WHY the limits are hit at `rows`, or `None` when they are not — the pure (no
@@ -147,7 +150,7 @@ pub(crate) mod budget {
         /// (sq-qk6ac)
         #[cfg_attr(not(feature = "parallel"), allow(dead_code))]
         #[inline]
-        pub(crate) fn why(&self, rows: usize) -> Option<&'static str> {
+        pub(in crate::exec) fn why(&self, rows: usize) -> Option<&'static str> {
             if !self.on {
                 return None;
             }
@@ -158,12 +161,15 @@ pub(crate) mod budget {
                 return Some("max-bytes");
             }
             #[cfg(not(target_arch = "wasm32"))]
-            if self.deadline.is_some_and(|d| std::time::Instant::now() >= d) {
+            if self
+                .deadline
+                .is_some_and(|d| std::time::Instant::now() >= d)
+            {
                 return Some("timeout");
             }
             if let Some(cancel) = self.cancel {
-                // SAFETY: `CancelPtr`'s invariant and the lifetime-bound `Guard`
-                // keep the `AtomicBool` alive for this scoped snapshot load.
+                // SAFETY: `CancelPtr`'s nested-frame invariant keeps the owning
+                // QueryBudget alive until this scoped snapshot load finishes.
                 if unsafe { cancel.0.as_ref() }.load(Ordering::Relaxed) {
                     return Some("cancelled");
                 }
@@ -179,7 +185,7 @@ pub(crate) mod budget {
         /// (and `snapshot`); the non-parallel (wasm) build compiles them out.
         #[cfg_attr(not(feature = "parallel"), allow(dead_code))]
         #[inline]
-        pub(crate) fn hit(&self, rows: usize) -> bool {
+        pub(in crate::exec) fn hit(&self, rows: usize) -> bool {
             self.why(rows).is_some()
         }
     }
@@ -189,20 +195,21 @@ pub(crate) mod budget {
         static EXCEEDED: Cell<Option<&'static str>> = const { Cell::new(None) };
     }
 
-    /// Clears the budget when the `*_with_budget` entry point returns (also on
-    /// error/unwind, so a poisoned thread never leaks a stale budget).
-    pub(crate) struct Guard<'a> {
+    // [GPT-6 Astra] Private: callers cannot forget or drop a frame out of order.
+    struct Guard<'a> {
+        previous: Limits,
+        exceeded: Option<&'static str>,
         _budget: std::marker::PhantomData<&'a QueryBudget>,
         _not_send: std::marker::PhantomData<std::rc::Rc<()>>,
     }
     impl Drop for Guard<'_> {
         fn drop(&mut self) {
-            ACTIVE.with(|a| a.set(OFF));
-            EXCEEDED.with(|e| e.set(None));
+            ACTIVE.with(|a| a.set(self.previous));
+            EXCEEDED.with(|e| e.set(self.exceeded));
         }
     }
 
-    pub(crate) fn install(b: &QueryBudget) -> Guard<'_> {
+    fn install(b: &QueryBudget) -> Guard<'_> {
         let cancel = b
             .cancel
             .as_ref()
@@ -214,8 +221,8 @@ pub(crate) mod budget {
             || cancel.is_some();
         #[cfg(target_arch = "wasm32")]
         let on = b.max_rows.is_some() || b.max_bytes.is_some() || cancel.is_some();
-        ACTIVE.with(|a| {
-            a.set(Limits {
+        let previous = ACTIVE.with(|a| {
+            a.replace(Limits {
                 on,
                 #[cfg(not(target_arch = "wasm32"))]
                 deadline: b.deadline,
@@ -226,13 +233,25 @@ pub(crate) mod budget {
                 cancel,
             })
         });
-        EXCEEDED.with(|e| e.set(None));
+        let exceeded = EXCEEDED.with(|e| e.replace(None));
         Guard {
+            previous,
+            exceeded,
             _budget: std::marker::PhantomData,
             _not_send: std::marker::PhantomData,
         }
     }
 
+    /// [GPT-6 Astra] Runs a child budget, restoring its parent on return or unwind.
+    ///
+    /// Guard never escapes this frame. This enforces LIFO even for reentrant
+    /// callbacks, keeps each borrowed cancellation owner alive, and restores idle
+    /// OFF/None after a top-level call. Aborting panics have no continuation.
+    pub(crate) fn with_budget<T>(b: &QueryBudget, f: impl FnOnce() -> T) -> T {
+        let _guard = install(b);
+        f()
+    }
+
     /// [OPUS-4.8] (sq-s5is) Sets the per-row byte width (= `width_in_ids ×
     /// BYTES_PER_ID`) of the working set the next row-count checks price. Called once
     /// per operator with that operator's output arity, so a check on `rows` correctly
@@ -289,9 +308,15 @@ pub(crate) mod budget {
     }
 
     /// Snapshot of the installed limits, for the rayon-parallel branches.
+    ///
+    /// [GPT-6 Astra] This lifetime-free Copy is trusted only inside exec. It must
+    /// not escape its owning with_budget frame or enter detached work. The four
+    /// consumers are scan SELECT-JSON, bindings SELECT-JSON, parallel hash join,
+    /// and the parallel residual anti-join; all join before returning. A new
+    /// consumer must establish the same owner lifetime and scoped-join invariant.
     #[cfg_attr(not(feature = "parallel"), allow(dead_code))]
     #[inline]
-    pub(crate) fn snapshot() -> Limits {
+    pub(in crate::exec) fn snapshot() -> Limits {
         ACTIVE.with(|a| a.get())
     }
 
@@ -314,10 +339,12 @@ pub(crate) mod budget {
     ///   every 1024 rows and stops early). A blanket "fan out whenever a budget is
     ///   installed" was REJECTED for exactly this reason (roborev 1538 / audit item 6).
     ///
+    /// The returned snapshot has the same owning-frame/scoped-join invariant as
+    /// [`snapshot`]; scan SELECT-JSON is its sole production consumer.
     /// Compiled only for the `parallel` feature — the wasm/serial build never fans out.
     #[cfg(feature = "parallel")]
     #[inline]
-    pub(crate) fn parallel_json_fanout() -> Option<Limits> {
+    pub(in crate::exec) fn parallel_json_fanout() -> Option<Limits> {
         ACTIVE.with(|a| {
             let l = a.get();
             if l.on && (l.max_rows != usize::MAX || l.max_bytes != usize::MAX) {
@@ -347,7 +374,8 @@ pub(crate) mod budget {
     pub(crate) fn remaining_timeout() -> Option<std::time::Duration> {
         ACTIVE.with(|a| {
             let lim = a.get();
-            lim.deadline.map(|d| d.saturating_duration_since(std::time::Instant::now()))
+            lim.deadline
+                .map(|d| d.saturating_duration_since(std::time::Instant::now()))
         })
     }
 
@@ -383,6 +411,9 @@ pub(crate) mod budget {
     /// here — so the pre-burst snapshot is exactly the current state minus this burst.
     /// A deadline that elapsed during the burst is not masked: the next `exhausted`
     /// re-derives it from the wall clock. [OPUS-4.8] (sq-my8wd.4)
+    /// [GPT-6 Astra] A synchronous nested with_budget may finish between capture
+    /// and restore: it restores this frame verbatim first. Never apply a savepoint
+    /// while a different budget frame is active.
     #[cfg(any(feature = "service", feature = "service-local"))]
     #[inline]
     pub(crate) fn restore_bytes(sp: ByteSavepoint) {
@@ -419,8 +450,8 @@ pub(crate) mod budget {
             return true;
         }
         if let Some(cancel) = a.cancel {
-            // SAFETY: `CancelPtr`'s invariant and the lifetime-bound `Guard` keep
-            // the `AtomicBool` alive until this thread-local pointer is cleared.
+            // SAFETY: `CancelPtr`'s nested-frame invariant keeps this pointer
+            // owned by a live QueryBudget, also when restoring a parent frame.
             // Relaxed is sufficient because cancellation gates control flow only;
             // it never publishes or guards a shared query buffer. If that changes,
             // the load/store pair must become Acquire/Release.
@@ -471,7 +502,250 @@ pub(crate) mod budget {
             .checked_div(a.byte_width.max(1))
             .unwrap_or(usize::MAX)
             .saturating_add(1);
-        cap.min(a.max_rows.saturating_add(1)).min(by_bytes).min(1 << 20)
+        cap.min(a.max_rows.saturating_add(1))
+            .min(by_bytes)
+            .min(1 << 20)
+    }
+
+    // [GPT-6 Astra] Reentrant scopes must restore the complete owning frame.
+    #[cfg(test)]
+    mod nested_budget_tests {
+        use super::*;
+        use std::sync::Arc;
+
+        fn assert_state(want: Limits, sticky: Option<&'static str>) {
+            let got = ACTIVE.with(Cell::get);
+            assert_eq!(got.on, want.on);
+            assert_eq!(got.max_rows, want.max_rows);
+            assert_eq!(got.max_bytes, want.max_bytes);
+            assert_eq!(got.byte_width, want.byte_width);
+            assert_eq!(got.extra_bytes, want.extra_bytes);
+            assert_eq!(got.cancel.map(|p| p.0), want.cancel.map(|p| p.0));
+            #[cfg(not(target_arch = "wasm32"))]
+            assert_eq!(got.deadline, want.deadline);
+            assert_eq!(EXCEEDED.with(Cell::get), sticky);
+        }
+
+        #[test]
+        fn exact_parent_state_survives_three_levels_ok_err_and_unwind() {
+            let outer = QueryBudget {
+                max_rows: Some(7),
+                max_bytes: Some(4096),
+                ..QueryBudget::cancelled_by(Arc::new(AtomicBool::new(false)))
+            };
+            with_budget(&outer, || {
+                set_width(5);
+                add_bytes(37);
+                let parent = snapshot();
+                let child = QueryBudget {
+                    max_rows: Some(2),
+                    max_bytes: Some(128),
+                    ..QueryBudget::unlimited()
+                };
+                let result: Result<(), &str> = with_budget(&child, || {
+                    set_width(2);
+                    add_bytes(11);
+                    let middle = snapshot();
+                    with_budget(&QueryBudget::unlimited(), || {
+                        assert_eq!(check(usize::MAX), Ok(()))
+                    });
+                    assert_state(middle, None);
+                    assert!(check(3).is_err());
+                    Err("child error")
+                });
+                assert_eq!(result, Err("child error"));
+                assert_state(parent, None);
+                let panic = std::panic::catch_unwind(|| {
+                    with_budget(&child, || {
+                        add_bytes(1000);
+                        panic!("controlled child unwind");
+                    })
+                });
+                assert!(panic.is_err());
+                assert_state(parent, None);
+                assert_eq!(check(7), Ok(()));
+                assert!(check(8).is_err());
+            });
+            assert_state(OFF, None);
+        }
+
+        #[test]
+        fn sticky_parent_errors_survive_clean_and_exhausted_children() {
+            for reason in ["max-rows", "max-bytes"] {
+                let outer = QueryBudget {
+                    max_rows: Some(2),
+                    max_bytes: Some(128),
+                    ..QueryBudget::unlimited()
+                };
+                with_budget(&outer, || {
+                    if reason == "max-rows" {
+                        assert!(check(3).is_err());
+                    } else {
+                        add_bytes(129);
+                    }
+                    let parent = snapshot();
+                    with_budget(&QueryBudget::unlimited(), || {
+                        assert_eq!(check(usize::MAX), Ok(()))
+                    });
+                    assert_state(parent, Some(reason));
+                    with_budget(
+                        &QueryBudget::cancelled_by(Arc::new(AtomicBool::new(true))),
+                        || {
+                            assert_eq!(
+                                check(0),
+                                Err("query budget exceeded (cancelled)".to_owned())
+                            );
+                        },
+                    );
+                    assert_state(parent, Some(reason));
+                    assert_eq!(check(0), Err(format!("query budget exceeded ({})", reason)));
+                });
+                assert_state(OFF, None);
+            }
+        }
+
+        #[test]
+        fn live_parent_cancel_is_distinct_from_child_cancel() {
+            let parent_flag = Arc::new(AtomicBool::new(false));
+            let child_flag = Arc::new(AtomicBool::new(false));
+            let outer = QueryBudget::cancelled_by(Arc::clone(&parent_flag));
+            let child = QueryBudget::cancelled_by(Arc::clone(&child_flag));
+            with_budget(&outer, || {
+                let parent = snapshot();
+                with_budget(&child, || {
+                    parent_flag.store(true, Ordering::Relaxed);
+                    assert_eq!(check(0), Ok(()));
+                    child_flag.store(true, Ordering::Relaxed);
+                    assert!(check(0).is_err());
+                });
+                assert_state(parent, None);
+                assert_eq!(
+                    check(0),
+                    Err("query budget exceeded (cancelled)".to_owned())
+                );
+            });
+            assert_state(OFF, None);
+        }
+
+        #[test]
+        #[cfg(not(target_arch = "wasm32"))]
+        fn expired_parent_deadline_returns_after_unlimited_child_without_sleep() {
+            let parent = QueryBudget {
+                deadline: Some(std::time::Instant::now()),
+                ..QueryBudget::unlimited()
+            };
+            with_budget(&parent, || {
+                with_budget(&QueryBudget::unlimited(), || assert_eq!(check(0), Ok(())));
+                assert_eq!(check(0), Err("query budget exceeded (timeout)".to_owned()));
+            });
+            assert_state(OFF, None);
+        }
+
+        #[test]
+        #[cfg(any(feature = "service", feature = "service-local"))]
+        fn service_savepoint_brackets_a_fully_returned_nested_budget() {
+            let parent = QueryBudget {
+                max_bytes: Some(128),
+                ..QueryBudget::unlimited()
+            };
+            with_budget(&parent, || {
+                set_width(3);
+                add_bytes(19);
+                let original = snapshot();
+                let mark = byte_savepoint();
+                with_budget(&QueryBudget::unlimited(), || {
+                    assert_eq!(check(usize::MAX), Ok(()))
+                });
+                add_bytes(200);
+                assert!(check(0).is_err());
+                restore_bytes(mark);
+                assert_state(original, None);
+                assert_eq!(check(0), Ok(()));
+            });
+        }
+
+        #[test]
+        fn snapshots_join_two_workers_before_parent_cancel_owner_returns() {
+            let flag = Arc::new(AtomicBool::new(true));
+            let parent = QueryBudget::cancelled_by(flag);
+            with_budget(&parent, || {
+                let snap = snapshot();
+                let owner = std::thread::current().id();
+                std::thread::scope(|scope| {
+                    let a = scope.spawn(|| (std::thread::current().id(), snap.why(0), check(0)));
+                    let b = scope.spawn(|| (std::thread::current().id(), snap.why(0), check(0)));
+                    let a = a.join().unwrap();
+                    let b = b.join().unwrap();
+                    assert_ne!(a.0, owner);
+                    assert_ne!(b.0, owner);
+                    assert_ne!(a.0, b.0);
+                    assert_eq!(a.1, Some("cancelled"));
+                    assert_eq!(b.1, Some("cancelled"));
+                    assert_eq!(a.2, Ok(()));
+                    assert_eq!(b.2, Ok(()));
+                });
+            });
+            assert_state(OFF, None);
+        }
+
+        #[test]
+        fn public_extension_nested_query_preserves_outer_cancel() {
+            use oxrdf::{Literal, Term};
+            use sparq_core::Graph;
+            use std::sync::atomic::AtomicUsize;
+            for inner in 0..4 {
+                let graph = Graph::load_str("", "turtle").unwrap();
+                let nested = Graph::load_str("", "turtle").unwrap();
+                let flag = Arc::new(AtomicBool::new(false));
+                let calls = Arc::new(AtomicUsize::new(0));
+                let callback_flag = Arc::clone(&flag);
+                let callback_calls = Arc::clone(&calls);
+                let owner = std::thread::current().id();
+                let mut functions = crate::FunctionRegistry::new();
+                functions.register("urn:nested", move |_| {
+                    assert_eq!(std::thread::current().id(), owner);
+                    callback_calls.fetch_add(1, Ordering::Relaxed);
+                    match inner {
+                        0 => {}
+                        1 => {
+                            assert_eq!(crate::query(&nested, "ASK {}").unwrap().rows.len(), 1);
+                        }
+                        2 => {
+                            let child = QueryBudget {
+                                max_rows: Some(1),
+                                ..QueryBudget::unlimited()
+                            };
+                            assert_eq!(
+                                crate::query_with_budget(&nested, "ASK {}", &child)
+                                    .unwrap()
+                                    .rows
+                                    .len(),
+                                1
+                            );
+                        }
+                        _ => {
+                            assert!(crate::query(&nested, "not SPARQL").is_err());
+                        }
+                    }
+                    callback_flag.store(true, Ordering::Relaxed);
+                    Ok(Term::Literal(Literal::from(7)))
+                });
+                let result = crate::query_with_functions_and_budget(
+                    &graph,
+                    "SELECT (<urn:nested>() AS ?value) WHERE {}",
+                    &functions,
+                    &QueryBudget::cancelled_by(flag),
+                );
+                assert_eq!(calls.load(Ordering::Relaxed), 1);
+                assert_eq!(
+                    result.unwrap_err(),
+                    "query budget exceeded (cancelled)",
+                    "inner={}",
+                    inner
+                );
+                assert_eq!(crate::query(&graph, "ASK {}").unwrap().rows.len(), 1);
+            }
+        }
     }
 
     /// [SONNET-4.6] (sq-qk6ac) Direct tests for `Limits::why` — the pure gate the
@@ -487,27 +761,29 @@ pub(crate) mod budget {
         /// Asserts that under `budget` the snapshot reports `want` at `rows`, that `hit`
         /// agrees, and that the reason is the very string `check` puts in its error.
         fn assert_reason(budget: &QueryBudget, rows: usize, want: &'static str) {
-            let _guard = install(budget);
-            let snap = snapshot();
-            assert_eq!(snap.why(rows), Some(want), "wrong snapshot reason for {}", want);
-            assert!(snap.hit(rows), "hit must agree with why for {}", want);
-            assert_eq!(
-                check(rows),
-                Err(format!("query budget exceeded ({})", want)),
-                "the worker-visible reason must match the on-thread error for {}",
-                want
-            );
+            with_budget(budget, || {
+                let snap = snapshot();
+                assert_eq!(snap.why(rows), Some(want), "wrong snapshot reason for {}", want);
+                assert!(snap.hit(rows), "hit must agree with why for {}", want);
+                assert_eq!(
+                    check(rows),
+                    Err(format!("query budget exceeded ({})", want)),
+                    "the worker-visible reason must match the on-thread error for {}",
+                    want
+                );
+            })
         }
 
         /// `why` and `hit` are one decision, and an unbudgeted snapshot never trips.
         #[test]
         fn unbudgeted_snapshot_has_no_reason() {
             let budget = QueryBudget::unlimited();
-            let _guard = install(&budget);
-            let snap = snapshot();
-            assert_eq!(snap.why(0), None, "an unlimited budget must report no reason");
-            assert_eq!(snap.why(usize::MAX), None, "no row cap ⇒ no reason at any row count");
-            assert!(!snap.hit(usize::MAX), "hit must agree with why");
+            with_budget(&budget, || {
+                let snap = snapshot();
+                assert_eq!(snap.why(0), None, "an unlimited budget must report no reason");
+                assert_eq!(snap.why(usize::MAX), None, "no row cap ⇒ no reason at any row count");
+                assert!(!snap.hit(usize::MAX), "hit must agree with why");
+            })
         }
 
         /// Each limit reports ITS OWN reason, and the string matches `check`'s message.
@@ -538,10 +814,11 @@ pub(crate) mod budget {
         #[test]
         fn a_limit_not_yet_crossed_reports_nothing() {
             let budget = QueryBudget { max_rows: Some(4), ..QueryBudget::unlimited() };
-            let _guard = install(&budget);
-            let snap = snapshot();
-            assert_eq!(snap.why(4), None, "a row count AT the cap is still admitted");
-            assert_eq!(snap.why(5), Some("max-rows"), "one past the cap trips");
+            with_budget(&budget, || {
+                let snap = snapshot();
+                assert_eq!(snap.why(4), None, "a row count AT the cap is still admitted");
+                assert_eq!(snap.why(5), Some("max-rows"), "one past the cap trips");
+            })
         }
 
         /// The crux the parallel verdict loop depends on: a WORKER thread has no budget
@@ -551,19 +828,20 @@ pub(crate) mod budget {
         fn snapshot_is_the_only_signal_a_worker_thread_can_see() {
             let flag = Arc::new(AtomicBool::new(true));
             let budget = QueryBudget::cancelled_by(Arc::clone(&flag));
-            let _guard = install(&budget);
-            let snap = snapshot();
+            with_budget(&budget, || {
+                let snap = snapshot();
 
-            let (worker_poll, worker_reason) = std::thread::scope(|s| {
-                s.spawn(|| (check(0), snap.why(0))).join().expect("worker must not panic")
-            });
-            assert_eq!(worker_poll, Ok(()), "the thread-local budget is invisible to a worker");
-            assert_eq!(
-                worker_reason,
-                Some("cancelled"),
-                "the captured snapshot must carry the cancellation across threads"
-            );
-            assert_eq!(check(0), Err("query budget exceeded (cancelled)".to_owned()));
+                let (worker_poll, worker_reason) = std::thread::scope(|s| {
+                    s.spawn(|| (check(0), snap.why(0))).join().expect("worker must not panic")
+                });
+                assert_eq!(worker_poll, Ok(()), "the thread-local budget is invisible to a worker");
+                assert_eq!(
+                    worker_reason,
+                    Some("cancelled"),
+                    "the captured snapshot must carry the cancellation across threads"
+                );
+                assert_eq!(check(0), Err("query budget exceeded (cancelled)".to_owned()));
+            })
         }
     }
 
@@ -578,15 +856,16 @@ pub(crate) mod budget {
         fn cancel_flag_zero_vs_one_trips_both_poll_paths() {
             let flag = Arc::new(AtomicBool::new(false));
             let budget = QueryBudget::unlimited().with_cancel(Arc::clone(&flag));
-            let _guard = install(&budget);
+            with_budget(&budget, || {
 
-            assert!(!snapshot().hit(0), "false control must not trip the rayon snapshot");
-            assert_eq!(check(0), Ok(()), "false control must not trip the local poll");
+                assert!(!snapshot().hit(0), "false control must not trip the rayon snapshot");
+                assert_eq!(check(0), Ok(()), "false control must not trip the local poll");
 
-            flag.store(true, Ordering::Relaxed);
-            assert!(snapshot().hit(0), "true flag must trip the rayon snapshot");
-            assert_eq!(check(0), Err("query budget exceeded (cancelled)".to_owned()));
-            assert_eq!(EXCEEDED.with(Cell::get), Some("cancelled"));
+                flag.store(true, Ordering::Relaxed);
+                assert!(snapshot().hit(0), "true flag must trip the rayon snapshot");
+                assert_eq!(check(0), Err("query budget exceeded (cancelled)".to_owned()));
+                assert_eq!(EXCEEDED.with(Cell::get), Some("cancelled"));
+            })
         }
 
         #[test]
@@ -17475,10 +17754,11 @@ mod service_exec_tests {
         // A cap the ONE discarded remote literal (interned twice) blows, but the SILENT
         // fallback's own working set (one identity row) fits well under.
         let b = crate::QueryBudget { max_bytes: Some(64), ..crate::QueryBudget::unlimited() };
-        let _bg = budget::install(&b);
-        let res = eval_select(&g, &p)
-            .expect("SILENT mid-stream error must roll back the discarded row's byte charge");
-        assert_eq!(res.rows.len(), 1, "SILENT verbatim -> identity -> one (unbound ?o) row");
+        budget::with_budget(&b, || {
+            let res = eval_select(&g, &p)
+                .expect("SILENT mid-stream error must roll back the discarded row's byte charge");
+            assert_eq!(res.rows.len(), 1, "SILENT verbatim -> identity -> one (unbound ?o) row");
+        })
     }
 
     #[test]
@@ -17497,10 +17777,11 @@ mod service_exec_tests {
              { ?s a ex:T . SERVICE SILENT <http://remote/> { ?s ex:p ?o } }",
         );
         let b = crate::QueryBudget { max_bytes: Some(64), ..crate::QueryBudget::unlimited() };
-        let _bg = budget::install(&b);
-        let res = eval_select(&g, &p)
-            .expect("bind-join SILENT mid-stream error must roll back the discarded row's byte charge");
-        assert_eq!(res.rows.len(), 1, "SILENT block failure -> identity -> local ?s=ex:a row survives");
+        budget::with_budget(&b, || {
+            let res = eval_select(&g, &p)
+                .expect("bind-join SILENT mid-stream error must roll back the discarded row's byte charge");
+            assert_eq!(res.rows.len(), 1, "SILENT block failure -> identity -> local ?s=ex:a row survives");
+        })
     }
 
     #[test]
@@ -18011,16 +18292,18 @@ mod service_exec_tests {
             deadline: Some(Instant::now() + Duration::from_secs(10)),
             ..crate::QueryBudget::unlimited()
         };
-        let _g = budget::install(&b);
-        let r = budget::remaining_timeout().expect("deadline installed");
-        assert!(r <= Duration::from_secs(10) && r > Duration::from_secs(8), "got {r:?}");
-        // An expired deadline saturates to ZERO (never panics / underflows).
-        let b2 = crate::QueryBudget {
-            deadline: Some(Instant::now() - Duration::from_millis(1)),
-            ..crate::QueryBudget::unlimited()
-        };
-        let _g2 = budget::install(&b2);
-        assert_eq!(budget::remaining_timeout(), Some(Duration::ZERO));
+        budget::with_budget(&b, || {
+            let r = budget::remaining_timeout().expect("deadline installed");
+            assert!(r <= Duration::from_secs(10) && r > Duration::from_secs(8), "got {r:?}");
+            // An expired deadline saturates to ZERO (never panics / underflows).
+            let b2 = crate::QueryBudget {
+                deadline: Some(Instant::now() - Duration::from_millis(1)),
+                ..crate::QueryBudget::unlimited()
+            };
+            budget::with_budget(&b2, || {
+                assert_eq!(budget::remaining_timeout(), Some(Duration::ZERO));
+        })
+        })
     }
 }
 
diff --git a/crates/sparq-engine/src/explain.rs b/crates/sparq-engine/src/explain.rs
index 2f6952ec3..9ce46fc0b 100644
--- a/crates/sparq-engine/src/explain.rs
+++ b/crates/sparq-engine/src/explain.rs
@@ -90,27 +90,28 @@ pub fn explain_analyze_with_budget(graph: &Graph, sparql: &str, budget: &QueryBu
     render_pattern(graph, pattern, &mut out, 1)?;
 
     // Execute under the budget with the operator trace installed.
-    let _bguard = exec::budget::install(budget);
-    let _tguard = exec::trace::install();
-    #[cfg(not(target_arch = "wasm32"))]
-    let start = std::time::Instant::now();
-    let total_rows = match &q {
-        Query::Select { pattern, .. } => exec::eval_select(graph, pattern)?.rows.len(),
-        Query::Ask { pattern, .. } => usize::from(exec::eval_ask(graph, pattern)?),
-        _ => unreachable!(),
-    };
-    #[cfg(not(target_arch = "wasm32"))]
-    let total_nanos = start.elapsed().as_nanos() as u64;
-    #[cfg(target_arch = "wasm32")]
-    let total_nanos = 0u64;
-    let nodes = exec::trace::take();
-
-    let _ = writeln!(out, "Execution trace (operator → output rows, wall time):");
-    for n in &nodes {
-        let _ = writeln!(out, "{}{}  rows={}  time={}", indent(n.depth + 1), n.label, n.rows, fmt_nanos(n.nanos));
-    }
-    let _ = writeln!(out, "Total: {} result row(s) in {}", total_rows, fmt_nanos(total_nanos));
-    Ok(out)
+    exec::budget::with_budget(budget, || {
+        let _tguard = exec::trace::install();
+        #[cfg(not(target_arch = "wasm32"))]
+        let start = std::time::Instant::now();
+        let total_rows = match &q {
+            Query::Select { pattern, .. } => exec::eval_select(graph, pattern)?.rows.len(),
+            Query::Ask { pattern, .. } => usize::from(exec::eval_ask(graph, pattern)?),
+            _ => unreachable!(),
+        };
+        #[cfg(not(target_arch = "wasm32"))]
+        let total_nanos = start.elapsed().as_nanos() as u64;
+        #[cfg(target_arch = "wasm32")]
+        let total_nanos = 0u64;
+        let nodes = exec::trace::take();
+
+        let _ = writeln!(out, "Execution trace (operator → output rows, wall time):");
+        for n in &nodes {
+            let _ = writeln!(out, "{}{}  rows={}  time={}", indent(n.depth + 1), n.label, n.rows, fmt_nanos(n.nanos));
+        }
+        let _ = writeln!(out, "Total: {} result row(s) in {}", total_rows, fmt_nanos(total_nanos));
+        Ok(out)
+    })
 }
 
 fn query_form_pattern(q: &Query) -> (&'static str, &GraphPattern) {
diff --git a/crates/sparq-engine/src/explain_json.rs b/crates/sparq-engine/src/explain_json.rs
index e4abc0b05..934cf0bbd 100644
--- a/crates/sparq-engine/src/explain_json.rs
+++ b/crates/sparq-engine/src/explain_json.rs
@@ -219,19 +219,20 @@ pub fn explain_plan_analyze_with_budget(graph: &Graph, sparql: &str, budget: &Qu
 
     // Execute under the budget with the operator trace installed (exactly as the
     // text `explain_analyze` does), then reconstruct the typed tree from the trace.
-    let _bguard = exec::budget::install(budget);
-    let _tguard = exec::trace::install();
-    match &q {
-        Query::Select { pattern, .. } => {
-            exec::eval_select(graph, pattern)?;
-        }
-        Query::Ask { pattern, .. } => {
-            exec::eval_ask(graph, pattern)?;
+    exec::budget::with_budget(budget, || {
+        let _tguard = exec::trace::install();
+        match &q {
+            Query::Select { pattern, .. } => {
+                exec::eval_select(graph, pattern)?;
+            }
+            Query::Ask { pattern, .. } => {
+                exec::eval_ask(graph, pattern)?;
+            }
+            _ => unreachable!(),
         }
-        _ => unreachable!(),
-    }
-    let nodes = exec::trace::take();
-    tree_from_trace(&nodes).ok_or_else(|| "empty execution trace".to_string())
+        let nodes = exec::trace::take();
+        tree_from_trace(&nodes).ok_or_else(|| "empty execution trace".to_string())
+    })
 }
 
 /// The query's root graph pattern (independent of form).
diff --git a/crates/sparq-engine/src/lib.rs b/crates/sparq-engine/src/lib.rs
index e6f51e636..613454a8d 100644
--- a/crates/sparq-engine/src/lib.rs
+++ b/crates/sparq-engine/src/lib.rs
@@ -268,6 +268,11 @@ use spargebra::{Query, SparqlParser};
 /// trips, evaluation stops and the query fails with
 /// `"query budget exceeded (timeout)"` / `"query budget exceeded (max-rows)"` /
 /// `"query budget exceeded (max-bytes)"` / `"query budget exceeded (cancelled)"`.
+///
+/// [GPT-6 Astra] A nested engine call from a callback uses its own budget. The
+/// outer budget resumes when that call returns (also after errors or unwind).
+/// Its deadline and cancellation are checked at the next outer poll; this does
+/// not interrupt arbitrary callback work or combine budgets across queries.
 #[derive(Debug, Clone, Default)]
 pub struct QueryBudget {
     /// Wall-clock deadline. Native only: `std::time::Instant` is unusable on
@@ -1033,18 +1038,19 @@ pub fn query_prepared_with_budget(
     let active = active_dataset(graph, q);
     let graph = active.as_ref().unwrap_or(graph);
     let _view_scope = view_scope(&active);
-    let _guard = exec::budget::install(budget);
-    exec::set_query_base(q.base_iri().map(|b| b.as_str()));
-    match q {
-        Query::Select { pattern, .. } => exec::eval_select(graph, pattern),
-        // ASK as a QueryResult: zero variables, and one (empty) row iff the pattern
-        // is satisfiable — the standard "unit row" encoding of a boolean result.
-        Query::Ask { pattern, .. } => Ok(QueryResult {
-            vars: Vec::new(),
-            rows: if exec::eval_ask(graph, pattern)? { vec![Vec::new()] } else { Vec::new() },
-        }),
-        _ => Err("only SELECT and ASK queries are supported".into()),
-    }
+    exec::budget::with_budget(budget, || {
+        exec::set_query_base(q.base_iri().map(|b| b.as_str()));
+        match q {
+            Query::Select { pattern, .. } => exec::eval_select(graph, pattern),
+            // ASK as a QueryResult: zero variables, and one (empty) row iff the pattern
+            // is satisfiable — the standard "unit row" encoding of a boolean result.
+            Query::Ask { pattern, .. } => Ok(QueryResult {
+                vars: Vec::new(),
+                rows: if exec::eval_ask(graph, pattern)? { vec![Vec::new()] } else { Vec::new() },
+            }),
+            _ => Err("only SELECT and ASK queries are supported".into()),
+        }
+    })
 }
 
 /// Executes an ASK query: `true` iff the pattern has at least one solution.
@@ -1070,12 +1076,13 @@ pub fn ask_prepared_with_budget(graph: &Graph, prepared: &PreparedQuery, budget:
     let active = active_dataset(graph, q);
     let graph = active.as_ref().unwrap_or(graph);
     let _view_scope = view_scope(&active);
-    let _guard = exec::budget::install(budget);
-    exec::set_query_base(q.base_iri().map(|b| b.as_str()));
-    match q {
-        Query::Ask { pattern, .. } => exec::eval_ask(graph, pattern),
-        _ => Err("ask() requires an ASK query".into()),
-    }
+    exec::budget::with_budget(budget, || {
+        exec::set_query_base(q.base_iri().map(|b| b.as_str()));
+        match q {
+            Query::Ask { pattern, .. } => exec::eval_ask(graph, pattern),
+            _ => Err("ask() requires an ASK query".into()),
+        }
+    })
 }
 
 /// Executes a SELECT and serialises it directly to a SPARQL 1.1 JSON results string,
@@ -1106,14 +1113,15 @@ pub fn query_json_prepared_with_budget(
     let active = active_dataset(graph, q);
     let graph = active.as_ref().unwrap_or(graph);
     let _view_scope = view_scope(&active);
-    let _guard = exec::budget::install(budget);
-    exec::set_query_base(q.base_iri().map(|b| b.as_str()));
-    match q {
-        Query::Select { pattern, .. } => exec::eval_select_json(graph, pattern),
-        // The SPARQL 1.1 JSON results boolean form.
-        Query::Ask { pattern, .. } => Ok(format!("{{\"head\":{{}},\"boolean\":{}}}", exec::eval_ask(graph, pattern)?)),
-        _ => Err("only SELECT and ASK queries are supported".into()),
-    }
+    exec::budget::with_budget(budget, || {
+        exec::set_query_base(q.base_iri().map(|b| b.as_str()));
+        match q {
+            Query::Select { pattern, .. } => exec::eval_select_json(graph, pattern),
+            // The SPARQL 1.1 JSON results boolean form.
+            Query::Ask { pattern, .. } => Ok(format!("{{\"head\":{{}},\"boolean\":{}}}", exec::eval_ask(graph, pattern)?)),
+            _ => Err("only SELECT and ASK queries are supported".into()),
+        }
+    })
 }
 
 /// Flush threshold for [`query_json_chunks_with_budget`]: large enough that the
@@ -1131,15 +1139,16 @@ pub fn query_json_chunks_with_budget(graph: &Graph, sparql: &str, budget: &Query
     let active = active_dataset(graph, q);
     let graph = active.as_ref().unwrap_or(graph);
     let _view_scope = view_scope(&active);
-    let _guard = exec::budget::install(budget);
-    exec::set_query_base(q.base_iri().map(|b| b.as_str()));
-    match q {
-        Query::Select { pattern, .. } => exec::eval_select_json_chunks(graph, pattern, Some(JSON_CHUNK_BYTES)),
-        Query::Ask { pattern, .. } => {
-            Ok(vec![format!("{{\"head\":{{}},\"boolean\":{}}}", exec::eval_ask(graph, pattern)?)])
+    exec::budget::with_budget(budget, || {
+        exec::set_query_base(q.base_iri().map(|b| b.as_str()));
+        match q {
+            Query::Select { pattern, .. } => exec::eval_select_json_chunks(graph, pattern, Some(JSON_CHUNK_BYTES)),
+            Query::Ask { pattern, .. } => {
+                Ok(vec![format!("{{\"head\":{{}},\"boolean\":{}}}", exec::eval_ask(graph, pattern)?)])
+            }
+            _ => Err("only SELECT and ASK queries are supported".into()),
         }
-        _ => Err("only SELECT and ASK queries are supported".into()),
-    }
+    })
 }
 
 /// Streams the SPARQL-JSON serialisation of a SELECT (or ASK) result, invoking `sink` for
@@ -1186,19 +1195,20 @@ pub fn query_json_stream_prepared_with_budget(
     let active = active_dataset(graph, q);
     let graph = active.as_ref().unwrap_or(graph);
     let _view_scope = view_scope(&active);
-    let _guard = exec::budget::install(budget);
-    exec::set_query_base(q.base_iri().map(|b| b.as_str()));
-    match q {
-        Query::Select { pattern, .. } => {
-            exec::eval_select_json_emit(graph, pattern, Some(JSON_CHUNK_BYTES), &mut sink)
-        }
-        Query::Ask { pattern, .. } => {
-            let doc = format!("{{\"head\":{{}},\"boolean\":{}}}", exec::eval_ask(graph, pattern)?);
-            let _ = sink(doc);
-            Ok(())
+    exec::budget::with_budget(budget, || {
+        exec::set_query_base(q.base_iri().map(|b| b.as_str()));
+        match q {
+            Query::Select { pattern, .. } => {
+                exec::eval_select_json_emit(graph, pattern, Some(JSON_CHUNK_BYTES), &mut sink)
+            }
+            Query::Ask { pattern, .. } => {
+                let doc = format!("{{\"head\":{{}},\"boolean\":{}}}", exec::eval_ask(graph, pattern)?);
+                let _ = sink(doc);
+                Ok(())
+            }
+            _ => Err("only SELECT and ASK queries are supported".into()),
         }
-        _ => Err("only SELECT and ASK queries are supported".into()),
-    }
+    })
 }
 
 /// Counts the solutions of a SELECT query *without* materialising the result
@@ -1224,14 +1234,15 @@ pub fn count_prepared_with_budget(graph: &Graph, prepared: &PreparedQuery, budge
     let active = active_dataset(graph, q);
     let graph = active.as_ref().unwrap_or(graph);
     let _view_scope = view_scope(&active);
-    let _guard = exec::budget::install(budget);
-    exec::set_query_base(q.base_iri().map(|b| b.as_str()));
-    match q {
-        Query::Select { pattern, .. } => exec::count_select(graph, pattern),
-        // An ASK counts its unit row: 1 when satisfiable, 0 otherwise.
-        Query::Ask { pattern, .. } => Ok(usize::from(exec::eval_ask(graph, pattern)?)),
-        _ => Err("only SELECT and ASK queries are supported".into()),
-    }
+    exec::budget::with_budget(budget, || {
+        exec::set_query_base(q.base_iri().map(|b| b.as_str()));
+        match q {
+            Query::Select { pattern, .. } => exec::count_select(graph, pattern),
+            // An ASK counts its unit row: 1 when satisfiable, 0 otherwise.
+            Query::Ask { pattern, .. } => Ok(usize::from(exec::eval_ask(graph, pattern)?)),
+            _ => Err("only SELECT and ASK queries are supported".into()),
+        }
+    })
 }
 
 #[derive(Debug)]
diff --git a/crates/sparq-engine/src/update.rs b/crates/sparq-engine/src/update.rs
index f1da29400..b270c2f93 100644
--- a/crates/sparq-engine/src/update.rs
+++ b/crates/sparq-engine/src/update.rs
@@ -532,8 +532,9 @@ pub(crate) fn update_in_place_prepared_with_budget(
     upd: &Update,
     budget: &crate::QueryBudget,
 ) -> Result<(), String> {
-    let _budget = crate::exec::budget::install(budget);
-    apply_update_in_place(graph, upd, None)
+    crate::exec::budget::with_budget(budget, || {
+        apply_update_in_place(graph, upd, None)
+    })
 }
 
 // --- the delta-overlay path ------------------------------------------------------------------
@@ -725,9 +726,10 @@ fn update_in_place_core(
     budget: &crate::QueryBudget,
     sink: EffectSink,
 ) -> Result<(), String> {
-    let _budget = crate::exec::budget::install(budget);
-    let upd = SparqlParser::new().parse_update(sparql).map_err(|e| e.to_string())?;
-    apply_update_in_place(graph, &upd, sink)
+    crate::exec::budget::with_budget(budget, || {
+        let upd = SparqlParser::new().parse_update(sparql).map_err(|e| e.to_string())?;
+        apply_update_in_place(graph, &upd, sink)
+    })
 }
 
 /// The shared per-operation in-place apply loop over an ALREADY-PARSED `Update`.

```

## Mechanical scope preservation
```json
[
  {
    "path": "crates/sparq-engine/src/cache.rs",
    "production_sites": 1,
    "exact_mechanical_match": true
  },
  {
    "path": "crates/sparq-engine/src/construct.rs",
    "production_sites": 3,
    "exact_mechanical_match": true
  },
  {
    "path": "crates/sparq-engine/src/explain.rs",
    "production_sites": 1,
    "exact_mechanical_match": true
  },
  {
    "path": "crates/sparq-engine/src/explain_json.rs",
    "production_sites": 1,
    "exact_mechanical_match": true
  },
  {
    "path": "crates/sparq-engine/src/lib.rs",
    "production_sites": 6,
    "exact_mechanical_match": true
  },
  {
    "path": "crates/sparq-engine/src/update.rs",
    "production_sites": 2,
    "exact_mechanical_match": true
  }
]

```
