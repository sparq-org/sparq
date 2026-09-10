# PR6478 focused supplement

Author/runtime: GPT-6 Astra.
Reviewed parent: bcba08207b03714a25ad7a2a4774370c7f6f6bfa
New head: d07ca79f89e3a8945be46b516c3cd2f770ccd618

Prior complete packet SHA256: 35d79d3037e767a6bf8df9a43f6c8eb531020423a15a38efba61d4b5e88de200
This supplement supplies only the delta, affected test bodies and comment/declaration context. The prior complete source/consumer packet remains required context. No new independent review is claimed.

## Delta from reviewed parent
```diff
diff --git a/bench/feature-off-declarations/6478.json b/bench/feature-off-declarations/6478.json
new file mode 100644
index 000000000..5ea489830
--- /dev/null
+++ b/bench/feature-off-declarations/6478.json
@@ -0,0 +1,5 @@
+{
+  "pr": 6478,
+  "date": "2026-09-10",
+  "reason": "[GPT-6 Astra] Intentional always-compiled engine change: nested evaluations restore outer query limits and sticky errors through a private scoped budget boundary. This is not a byte-neutrality assertion; the separate wasm size ratchet remains unchanged."
+}
diff --git a/crates/sparq-engine/src/exec.rs b/crates/sparq-engine/src/exec.rs
index be8bfb606..098414f92 100644
--- a/crates/sparq-engine/src/exec.rs
+++ b/crates/sparq-engine/src/exec.rs
@@ -77,18 +77,23 @@ pub(crate) mod budget {
     /// [GPT-6 Astra] ACTIVE belongs to the innermost live `with_budget` frame.
     /// Each pointer is owned by a QueryBudget borrowed by a still-live frame on
     /// this thread's stack. Private installation/restoration enforces nesting:
-    /// a restored parent's borrow outlives the child. Only the innermost frame
-    /// writes ACTIVE. Snapshots are used only in synchronous parallel work that
-    /// joins before the owning frame returns; dereferences are atomic loads.
+    /// a restored parent's borrow outlives the child. Only `install`/`Guard::drop`
+    /// change ACTIVE's `cancel` pointer; other writers update width/bytes for the
+    /// innermost frame. Private construction and the four synchronous snapshot
+    /// consumers documented on `snapshot` uphold this usage-level invariant:
+    /// their parallel work joins before the owning frame returns, and pointer
+    /// dereferences are atomic loads.
     #[derive(Clone, Copy)]
     struct CancelPtr(NonNull<AtomicBool>);
 
     // SAFETY: `AtomicBool` is `Sync`; moving this shared pointer to a worker is
     // sound because only atomic loads occur while the owning `with_budget` frame
-    // borrows its QueryBudget, including until scoped rayon work joins.
+    // borrows its QueryBudget, including until scoped rayon work joins. This relies
+    // on private construction and the four audited consumers documented on `snapshot`.
     unsafe impl Send for CancelPtr {}
     // SAFETY: `AtomicBool` is `Sync`; all shared access through `CancelPtr` is an
-    // atomic load; the owning `with_budget` frame outlives all worker joins.
+    // atomic load; private construction and the four audited scoped-join consumers
+    // documented on `snapshot` keep the owning `with_budget` frame alive.
     unsafe impl Sync for CancelPtr {}
 
     /// Bytes one id-level binding cell occupies in a materialised `Row`. The
@@ -138,8 +143,7 @@ pub(crate) mod budget {
         /// byte size compared against `max_bytes`. [OPUS-4.8] (sq-s5is)
         #[inline]
         fn bytes(&self, rows: usize) -> usize {
-            rows.saturating_mul(self.byte_width)
-                .saturating_add(self.extra_bytes)
+            rows.saturating_mul(self.byte_width).saturating_add(self.extra_bytes)
         }
 
         /// WHY the limits are hit at `rows`, or `None` when they are not — the pure (no
@@ -161,10 +165,7 @@ pub(crate) mod budget {
                 return Some("max-bytes");
             }
             #[cfg(not(target_arch = "wasm32"))]
-            if self
-                .deadline
-                .is_some_and(|d| std::time::Instant::now() >= d)
-            {
+            if self.deadline.is_some_and(|d| std::time::Instant::now() >= d) {
                 return Some("timeout");
             }
             if let Some(cancel) = self.cancel {
@@ -374,8 +375,7 @@ pub(crate) mod budget {
     pub(crate) fn remaining_timeout() -> Option<std::time::Duration> {
         ACTIVE.with(|a| {
             let lim = a.get();
-            lim.deadline
-                .map(|d| d.saturating_duration_since(std::time::Instant::now()))
+            lim.deadline.map(|d| d.saturating_duration_since(std::time::Instant::now()))
         })
     }
 
@@ -502,9 +502,7 @@ pub(crate) mod budget {
             .checked_div(a.byte_width.max(1))
             .unwrap_or(usize::MAX)
             .saturating_add(1);
-        cap.min(a.max_rows.saturating_add(1))
-            .min(by_bytes)
-            .min(1 << 20)
+        cap.min(a.max_rows.saturating_add(1)).min(by_bytes).min(1 << 20)
     }
 
     // [GPT-6 Astra] Reentrant scopes must restore the complete owning frame.
@@ -671,6 +669,7 @@ pub(crate) mod budget {
                 assert_state(original, None);
                 assert_eq!(check(0), Ok(()));
             });
+            assert_state(OFF, None);
         }
 
         #[test]
@@ -712,6 +711,8 @@ pub(crate) mod budget {
                 let owner = std::thread::current().id();
                 let mut functions = crate::FunctionRegistry::new();
                 functions.register("urn:nested", move |_| {
+                    // Pin this one-row plan to serial callback evaluation: the
+                    // nested call must overwrite the same TLS as the outer query.
                     assert_eq!(std::thread::current().id(), owner);
                     callback_calls.fetch_add(1, Ordering::Relaxed);
                     match inner {
@@ -866,13 +867,22 @@ pub(crate) mod budget {
             let flag = Arc::new(AtomicBool::new(false));
             let budget = QueryBudget::unlimited().with_cancel(Arc::clone(&flag));
             with_budget(&budget, || {
-
-                assert!(!snapshot().hit(0), "false control must not trip the rayon snapshot");
-                assert_eq!(check(0), Ok(()), "false control must not trip the local poll");
+                assert!(
+                    !snapshot().hit(0),
+                    "false control must not trip the rayon snapshot"
+                );
+                assert_eq!(
+                    check(0),
+                    Ok(()),
+                    "false control must not trip the local poll"
+                );
 
                 flag.store(true, Ordering::Relaxed);
                 assert!(snapshot().hit(0), "true flag must trip the rayon snapshot");
-                assert_eq!(check(0), Err("query budget exceeded (cancelled)".to_owned()));
+                assert_eq!(
+                    check(0),
+                    Err("query budget exceeded (cancelled)".to_owned())
+                );
                 assert_eq!(EXCEEDED.with(Cell::get), Some("cancelled"));
             })
         }
@@ -18303,7 +18313,10 @@ mod service_exec_tests {
         };
         budget::with_budget(&b, || {
             let r = budget::remaining_timeout().expect("deadline installed");
-            assert!(r <= Duration::from_secs(10) && r > Duration::from_secs(8), "got {r:?}");
+            assert!(
+                r <= Duration::from_secs(10) && r > Duration::from_secs(8),
+                "got {r:?}"
+            );
             // An expired deadline saturates to ZERO (never panics / underflows).
             let b2 = crate::QueryBudget {
                 deadline: Some(Instant::now() - Duration::from_millis(1)),
@@ -18311,7 +18324,7 @@ mod service_exec_tests {
             };
             budget::with_budget(&b2, || {
                 assert_eq!(budget::remaining_timeout(), Some(Duration::ZERO));
-        })
+            })
         })
     }
 }
```

## Findings and disposition
```json
{
  "NB1": "Source check only: QUERY_BASE setter and resolver are unscoped, byte-identical to d41. exec::view::Guard already saves/restores State. Root reports source-only issue6479 created; no duplicate or runtime BASE claim. No broader TLS audit in this patch.",
  "NB2": "Defer lifetime-typed snapshot redesign; previous four-consumer synchronous scope audit remains unchanged. Future unsafe misuse risk acknowledged.",
  "NB3": "No frame-counter addition. All three actual SERVICE owners preserve frame ordering; a fully returned nested scope restores parent before savepoint restoration. Guard hardening is separate future work.",
  "NB4": "Shadow-not-inherit behavior remains documented. No bound on callback/nested-unlimited duration or combined child+parent memory claimed.",
  "NB5": "Clarified only install/Drop change cancel pointer; other writers affect current frame width/bytes. Safety comments explicitly rely on private construction and four audited scoped-join consumers.",
  "NB6": "Pinned rustfmt applied only to new module and two migrated tests; removed blank closure line/corrected indentation. Four incidental production reflows restored to base. Whole-tree formatting remains informational and was not applied.",
  "NB7": "Added final idle assertion and comment pinning serial one-row callback. Discriminating thread assertion retained; actual service-local in-tree7/7 executes new assertion.",
  "NB8": "Dynamic production observer still proves one of four snapshot paths; other three remain source-audited, no new claim."
}
```

## Actual in-tree outcomes
```json
[
  {
    "label": "in-tree-default",
    "passed": 6,
    "failed": 0,
    "filtered": 328,
    "engine_cfg": [
      "feature=\"default\"",
      "feature=\"digest\"",
      "feature=\"parallel\"",
      "feature=\"regex\""
    ],
    "binary_sha256": "725905f8ca7f2935465c6528c8085cd4dbd65906b7ae45c617c1a64a8ef66d71",
    "test_results": [
      "test exec::budget::nested_budget_tests::expired_parent_deadline_returns_after_unlimited_child_without_sleep ... ok",
      "test exec::budget::nested_budget_tests::live_parent_cancel_is_distinct_from_child_cancel ... ok",
      "test exec::budget::nested_budget_tests::sticky_parent_errors_survive_clean_and_exhausted_children ... ok",
      "test exec::budget::nested_budget_tests::snapshots_join_two_workers_before_parent_cancel_owner_returns ... ok",
      "test exec::budget::nested_budget_tests::exact_parent_state_survives_three_levels_ok_err_and_unwind ... ok",
      "test exec::budget::nested_budget_tests::public_extension_nested_query_preserves_outer_cancel ... ok"
    ],
    "summary": [
      "test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 328 filtered out; finished in 0.01s"
    ]
  },
  {
    "label": "in-tree-no-default",
    "passed": 6,
    "failed": 0,
    "filtered": 314,
    "engine_cfg": [],
    "binary_sha256": "e55ed8a980d8bcc7dd6e7804532fd97c3d5468f3058b90305319ddd28dea4898",
    "test_results": [
      "test exec::budget::nested_budget_tests::expired_parent_deadline_returns_after_unlimited_child_without_sleep ... ok",
      "test exec::budget::nested_budget_tests::live_parent_cancel_is_distinct_from_child_cancel ... ok",
      "test exec::budget::nested_budget_tests::sticky_parent_errors_survive_clean_and_exhausted_children ... ok",
      "test exec::budget::nested_budget_tests::snapshots_join_two_workers_before_parent_cancel_owner_returns ... ok",
      "test exec::budget::nested_budget_tests::exact_parent_state_survives_three_levels_ok_err_and_unwind ... ok",
      "test exec::budget::nested_budget_tests::public_extension_nested_query_preserves_outer_cancel ... ok"
    ],
    "summary": [
      "test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 314 filtered out; finished in 0.00s"
    ]
  },
  {
    "label": "in-tree-service-local",
    "passed": 7,
    "failed": 0,
    "filtered": 332,
    "engine_cfg": [
      "feature=\"default\"",
      "feature=\"digest\"",
      "feature=\"parallel\"",
      "feature=\"regex\"",
      "feature=\"service-local\""
    ],
    "binary_sha256": "8b9d7b3f2839c14ce60b3afa5730034a1345467dc10ba8338662f3b9cd123f05",
    "test_results": [
      "test exec::budget::nested_budget_tests::service_savepoint_brackets_a_fully_returned_nested_budget ... ok",
      "test exec::budget::nested_budget_tests::live_parent_cancel_is_distinct_from_child_cancel ... ok",
      "test exec::budget::nested_budget_tests::expired_parent_deadline_returns_after_unlimited_child_without_sleep ... ok",
      "test exec::budget::nested_budget_tests::sticky_parent_errors_survive_clean_and_exhausted_children ... ok",
      "test exec::budget::nested_budget_tests::snapshots_join_two_workers_before_parent_cancel_owner_returns ... ok",
      "test exec::budget::nested_budget_tests::exact_parent_state_survives_three_levels_ok_err_and_unwind ... ok",
      "test exec::budget::nested_budget_tests::public_extension_nested_query_preserves_outer_cancel ... ok"
    ],
    "summary": [
      "test result: ok. 7 passed; 0 failed; 0 ignored; 0 measured; 332 filtered out; finished in 0.00s"
    ]
  }
]
```

Commands: `cargo test --release --locked --offline -p sparq-engine --lib exec::budget::nested_budget_tests -- --nocapture`, then same with `--no-default-features`, then same with `--features service-local`. Profile overrides: OPT_LEVEL=3, DEBUG=false, LTO=false, CODEGEN_UNITS=16, PANIC=unwind. Pinned compiler1.97.1; jobs1,incrementalOFF,offline. Actual --test invocations/binary hashes frozen separately. Controlled caught child/top-level panic messages are expected; all tests pass.

Local correctness only: pinned1.97.1 O3/unwind/noLTO/codegen16; jobs1,incrementalOFF,locked/offline. Actual rustc has --test, opt-level3, codegen16, no abort/LTO flag. Full argv captured separately.

## Complete changed test items
crates/sparq-engine/src/exec.rs
```rust
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
            assert_state(OFF, None);
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
                    // Pin this one-row plan to serial callback evaluation: the
                    // nested call must overwrite the same TLS as the outer query.
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

        fn cancel_flag_zero_vs_one_trips_both_poll_paths() {
            let flag = Arc::new(AtomicBool::new(false));
            let budget = QueryBudget::unlimited().with_cancel(Arc::clone(&flag));
            with_budget(&budget, || {
                assert!(
                    !snapshot().hit(0),
                    "false control must not trip the rayon snapshot"
                );
                assert_eq!(
                    check(0),
                    Ok(()),
                    "false control must not trip the local poll"
                );

                flag.store(true, Ordering::Relaxed);
                assert!(snapshot().hit(0), "true flag must trip the rayon snapshot");
                assert_eq!(
                    check(0),
                    Err("query budget exceeded (cancelled)".to_owned())
                );
                assert_eq!(EXCEEDED.with(Cell::get), Some("cancelled"));
            })
        }

    fn budget_remaining_timeout_reflects_deadline() {
        use std::time::{Duration, Instant};
        // No budget installed -> None.
        assert!(budget::remaining_timeout().is_none());
        // A future deadline -> Some(positive), bounded by the deadline.
        let b = crate::QueryBudget {
            deadline: Some(Instant::now() + Duration::from_secs(10)),
            ..crate::QueryBudget::unlimited()
        };
        budget::with_budget(&b, || {
            let r = budget::remaining_timeout().expect("deadline installed");
            assert!(
                r <= Duration::from_secs(10) && r > Duration::from_secs(8),
                "got {r:?}"
            );
            // An expired deadline saturates to ZERO (never panics / underflows).
            let b2 = crate::QueryBudget {
                deadline: Some(Instant::now() - Duration::from_millis(1)),
                ..crate::QueryBudget::unlimited()
            };
            budget::with_budget(&b2, || {
                assert_eq!(budget::remaining_timeout(), Some(Duration::ZERO));
            })
        })
    }
```

## Declaration and actual mechanical policy
bench/feature-off-declarations/6478.json
```json
{
  "pr": 6478,
  "date": "2026-09-10",
  "reason": "[GPT-6 Astra] Intentional always-compiled engine change: nested evaluations restore outer query limits and sticky errors through a private scoped budget boundary. This is not a byte-neutrality assertion; the separate wasm size ratchet remains unchanged."
}
```

scripts/check-vectorized-feature-off.py _new_declaration_files admits new digit-named filename set difference; no general JSON schema in this checker. Manually verified pr integer6478,date,reason only; actual helper returns exactly6478.json against base-name fixture.
All repository feature-off selftest tripwires passed. No wasm build or dynamic equality/size result claimed.

bench/feature-off-declarations/README.md (lines19–54)
```text
```

with the shape:

```json
{
  "pr": 1234,
  "date": "2026-07-07",
  "reason": "one line on what always-compiled change moves the feature-OFF bytes"
}
```

The gate is satisfied when the head tree's declarations directory contains at least one
`<digits>.json` (or `.md`) file the base tree's does **not** — a set difference on the
directory listing. Only names matching `<digits>.json|md` count as declarations, so this
`README.md` (and any `.gitkeep`) is ignored.

## Why per-PR files (V2)

The previous mechanism stored one scalar `change_token` in
`bench/feature-off-declaration.json`. Every declaring PR edited the **same line**, so the
first declared PR to merge made every other declared PR textually **CONFLICTING** in git
(`#1720` and `#1718` both went `DIRTY` after `#1726`'s declaration merged). The gate check
was order-independent but the file was not. Per-PR files remove the shared line: different
PRs add different files, so git never conflicts regardless of merge order.

The legacy scalar file `bench/feature-off-declaration.json` is **retired** (frozen, kept
parseable). During the transition window the gate still accepts a scalar-token inequality
so in-flight pre-V2 branches keep working; new PRs must use a per-PR file here instead.

## Scope

This declaration governs **intent** (did you mean to change the feature-OFF bytes at all?).
The **size** of an accepted change is governed separately, unchanged, by the
`metrics.wasm_bundle_bytes` floor ratchet (±2% band) in `bench/perf-baseline.json` /
`bench.yml`. A change moving the bundle > ±2% must also raise that floor.
```

## Source-only NB1 evidence (unchanged from main)
```rust
crates/sparq-engine/src/exec.rs :: query_base
thread_local! {
    /// The query's BASE IRI (when declared), used by IRI()/URI() to resolve relative
    /// references. Set by the `lib.rs` query entry points after parsing.
    static QUERY_BASE: std::cell::RefCell<Option<oxiri::Iri<String>>> = const { std::cell::RefCell::new(None) };
}

/// Installs the active query's base IRI for expression evaluation (IRI()/URI()
/// relative-reference resolution). Called by the query entry points; `None` clears it.
pub(crate) fn set_query_base(base: Option<&str>) {
    QUERY_BASE.with(|b| *b.borrow_mut() = base.and_then(|s| oxiri::Iri::parse(s.to_string()).ok()));
}

/// `IRI(str)`: absolute IRIs pass through; relative references resolve against the
/// query's BASE (a relative reference without a base is a type error).
fn resolve_iri(s: &str) -> Option<oxrdf::NamedNode> {
    if let Ok(abs) = oxiri::Iri::parse(s.to_string()) {
        return Some(oxrdf::NamedNode::new_unchecked(abs.into_inner()));
    }
    QUERY_BASE.with(|b| {
        b.borrow()
            .as_ref()
            .and_then(|base| base.resolve(s).ok())
            .map(|iri| oxrdf::NamedNode::new_unchecked(iri.into_inner()))
    })
}



crates/sparq-engine/src/exec.rs :: view
pub(crate) mod view {
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


```

## Validation limits
- Full workspace/all-target Cargo clippy and test gates pending normal CI; these were targeted --lib tests only.
- No-default-feature run has no engine feature cfgs but engine dev sparq-core explicitly enables parallel,mmap,dict-spill; not a fully serial dependency graph.
- No all-features/remote HTTP SERVICE test run, wasm, Miri, full conformance/coverage/perf ratchet in this followup.
- Preflight exits1 solely known Bash3 privacy-claims mapfile absence; rest of invoked mechanical checks pass.
- No performance or new wasm byte-size claim. No remote changes in this lane.

No source changes beyond this delta; prior frozen validation is unchanged. The copied-module mutant/public-library distinction in the prior evidence remains explicit: in-tree runs above are additional actual crate execution, not retroactive claims about prior mutants.
