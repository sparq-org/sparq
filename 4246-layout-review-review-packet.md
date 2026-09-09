# Focused assessment: Copilot3970001492 on fa371

No implementation change is proposed in this bundle. Root owns the response and any independent review.
The test is an intentional **physical-footprint regression check**, and the metadata-only control proves it detects a cost that heap/row checks miss: **440 versus 248 bytes**, after all earlier behavioral checks pass. No current-target runtime defect is demonstrated.

Copilot is correct about the language limit: `repr(Rust)` does not promise fixed padding or alignment. The comment currently overstates that guarantee. Recommend only a scope/comment clarification (and descriptive failure messages if authorized), preserving the footprint check as an empirical budget across the supported pinned configurations. The expected size is computed from current field types, not hard-coded 248, and no field order/offset is tested.

A portable exhaustive-field/type check on the **actual** `Overlay` can reject extra metadata fields without a layout assumption, but cannot detect layout/attribute-only physical growth. It is a different contract; no alternative was implemented or tested. A mirrored nominal struct is not a portable equivalence proof.

Stable unit tests use pinned 1.97.1; Miri explicitly uses pinned nightly-2026-06-11 and can execute this test. One native control does not prove all-target portability, and no nightly/target matrix was run. Source and declaration remain frozen.

Primary references: [Rust representation](https://doc.rust-lang.org/reference/type-layout.html#the-rust-representation), [size_of](https://doc.rust-lang.org/std/mem/fn.size_of.html).

## review-comment.json

````json
{
  "head": "fa3712df77e8acda4c447c6d0d4f87bc3376cc12",
  "thread": {
    "id": "PRRT_kwDOSz3qKM6gtyhC",
    "isResolved": false,
    "isOutdated": false,
    "path": "crates/sparq-core/src/store/overlay_deleted_tests.rs",
    "line": 386,
    "comments": {
      "pageInfo": {
        "hasNextPage": false
      },
      "nodes": [
        {
          "databaseId": 3970001492,
          "body": "This test asserts `size_of::<Overlay>()` and `align_of::<Overlay>()` based on an assumed field layout/no-padding guarantee, but `Overlay` is not `repr(C)` and Rust does not guarantee field order or padding across compiler versions/targets. This makes the default-off test brittle and can fail without any behavioral regression.",
          "url": "https://github.com/sparq-org/sparq/pull/6469#discussion_r3970001492",
          "author": {
            "login": "copilot-pull-request-reviewer"
          },
          "createdAt": "2026-09-09T15:09:45Z"
        }
      ]
    }
  }
}
````

## assessment.json

````json
{
  "head": "fa3712df77e8acda4c447c6d0d4f87bc3376cc12",
  "comment_id": 3970001492,
  "thread": "PRRT_kwDOSz3qKM6gtyhC",
  "assessment": "Intentional and effective empirical default-off inline-footprint tripwire; no demonstrated correctness defect or current-target failure. Copilot correctly identifies an unsupported portability claim in the no-padding/pointer-alignment prose. Recommend a narrowly scoped comment/diagnostic clarification while retaining the assertions, subject to root/independent review; do not claim repr(Rust) guarantees this footprint.",
  "evidence": {
    "test": "crates/sparq-core/src/store/overlay_deleted_tests.rs:350\u2013386; comments explicitly say representation/linear behavior, not merely rows. The expected size is recomputed from the three existing field TYPES in the active build; it is not a fixed248-byte cross-platform constant, and no field offsets/order are asserted.",
    "field_types": "Default Overlay has Vec<[Id;3]>, FxHashSet<[Id;3]> (rustc_hash alias to std HashSet with FxBuildHasher), and6 OnceLock<Vec<[Id;3]>> slots for existing added projections. New deletion slots are cfg(feature) only. These standard-library/container layouts are not stable ABI contracts.",
    "heap": "Overlay::heap_bytes counts owned vector/hashset backing storage and initialized projection capacities. TripleStore::heap_bytes adds permutation buffers and overlay backing storage. Neither counts inline Overlay metadata; an empty OnceLock array contributes no owned backing allocation. Removing the size assertion in favor of these heap checks alone loses real coverage.",
    "control": "Exact unchanged default-off test compiled and executed on pinned1.97.1 aarch64-apple-darwin; one metadata-only cfg-not field added to an exported source copy. Exit101 after13.497s at size assertion381:440 versus248 (+192bytes). All preceding rows, borrowed scans, repeated read heap checks and fork heap checks passed. The later alignment assertion was not reached; it was not separately calibrated by this control.",
    "baseline": "Reused prior actual6334 four-case default-off suite, including this exact test. Git provesfa371 changes only declaration JSON, so runtime/test/manifests/toolchain are identical. No fresh baseline run claimed.",
    "source_frozen": "Current fa371 source and declaration remain clean and unchanged. No implementation edit, commit, remote mutation, extra build, optimized/wasm build or independent review call."
  },
  "workflow_context": {
    "stable": "rust-toolchain.toml135 pins1.97.1. ci.yml397\u2013429 and521 build/archive workspace tests on ubuntu-latest with ordinary cargo and no higher-priority toolchain override at this job; the pin governs rather than floating rustup default stable. Other ordinary core feature legs except the explicit cache-on leg also compile this cfg-not test.",
    "nightly": "miri.yml193/205 explicitly selects nightly-2026-06-11;248\u2013265 invokes cargo +nightly-2026-06-11 miri nextest run -p sparq-core across24 shards with no test-name exclusion. This default-off unit test is eligible there; genuine assertion failures are classified fatal. Thus the stable pin alone does not cover every executing compiler. No Miri run was performed or inspected here.",
    "asan_boundary": "asan.yml153\u2013156 selects only mmap_corruption_oracle integration tests, not the core unit-test module; do not call that an execution of this assertion.",
    "limits": "No randomized-layout, new architecture, nightly, Miri, MSRV test run or compiler-version matrix was attempted. The native control is evidence for the observed host only. Source wiring is not current run-success evidence."
  },
  "recommendation": {
    "smallest": "Correct the comment to state this is an empirical inline-size/alignment regression budget on the supported toolchains/targets, not a repr(Rust) or std-container-layout guarantee. Add descriptive assertion messages if root authorizes. Preserve the existing size and heap behavioral checks; do not change repr, fields, cfg/defaults, runtime or equality limits.",
    "possible_comment": "These checks are empirical inline-footprint tripwires for the supported toolchains and targets, not repr(Rust) layout guarantees. A compiler or target can change padding or alignment; investigate a failure against the exact main source under the same configuration before changing the budget, and retain coverage for unintended default-off cache metadata.",
    "policy_limit": "If maintainers require every semantically valid target/compiler layout to pass without examining footprint changes, keeping universal physical-layout assertions conflicts with that requirement. That policy has not been established by this comment; root chooses it. No assertion suppression or target exemption proposed here.",
    "portable_alternative": "An exhaustive pattern/constructor of the actual default-off Overlay, with all three expected fields, no `..`, and explicit checks of their field types, can reject an extra cache field at compile time without requiring any padding/offset guarantee. It would preserve field-shape/default-off absence coverage and existing heap/row tests, but not detect layout/attribute-only physical growth or all field-type-internal layout growth. It is a different, narrower contract; not implemented or tested.",
    "rejected_shortcuts": "A mirrored nominal repr(Rust) struct is not a portable proof of identical layout; different nominal types need not lay out alike. Adding repr(C), a hand-sized tolerance, removing all footprint coverage, or hiding the test by compiler/target would expand or weaken scope and is not justified by current evidence."
  },
  "primary_sources": [
    {
      "url": "https://doc.rust-lang.org/reference/type-layout.html#the-rust-representation",
      "title": "Rust Reference: Rust representation",
      "accessed": "2026-09-09",
      "finding": "Layout may change between compilations. Default repr(Rust) guarantees appropriate field alignment, aggregate alignment at least the field maximum, and nonoverlap; it does not guarantee field order, minimum aggregate size, or absence of extra padding."
    },
    {
      "url": "https://doc.rust-lang.org/std/mem/fn.size_of.html",
      "title": "std::mem::size_of",
      "accessed": "2026-09-09",
      "finding": "Reports the actual type size, including alignment padding, for the compilation. A check of a measured footprint is meaningful without that footprint being a language guarantee."
    }
  ],
  "control_result": {
    "head": "fa3712df77e8acda4c447c6d0d4f87bc3376cc12",
    "tree": "/private/tmp/sparq-pr6049/.throughput-monitor/worktrees/layout-metadata-control-fa371",
    "command": [
      "/Users/jesght/.cargo/bin/cargo",
      "test",
      "--locked",
      "--offline",
      "-p",
      "sparq-core",
      "--lib",
      "store::overlay_deleted_tests::deleted_projection_feature_off_preserves_main_layout_and_heap",
      "--",
      "--exact",
      "--test-threads=1",
      "--nocapture"
    ],
    "started_at": 1788967777.208362,
    "free_disk_bytes_before": 6423900160,
    "mutant_sha256": "8c0acbe4ef9f58c21d8f8078f2ed7a68278005fc8e666ee600a1f164821c9d38",
    "test_sha256": "f51cb2e74a1af411517b9f744260f2cb69986d724e772e8b22d6c0deaf05c5b1",
    "environment": {
      "CARGO_BUILD_JOBS": "2",
      "RAYON_NUM_THREADS": "2",
      "CARGO_TARGET_DIR": "/private/tmp/sparq-pr6049/.throughput-monitor/direct-5983/implementation/target",
      "CARGO_PROFILE_DEV_DEBUG": "0",
      "CARGO_PROFILE_TEST_DEBUG": "0",
      "CARGO_NET_OFFLINE": "true",
      "RUSTUP_TOOLCHAIN": "1.97.1"
    },
    "scope": "Only one metadata field added under cfg-not in exported task-private source. No changes to methods, reads, heap accounting, tests or other source.",
    "exit": 101,
    "seconds": 13.497494917000001,
    "free_disk_bytes_after": 6418264064
  },
  "source_state": {
    "head": "fa3712df77e8acda4c447c6d0d4f87bc3376cc12",
    "source_status": "clean",
    "delta_since_native_baseline_head": [
      "bench/feature-off-declarations/6469.json"
    ],
    "native_baseline_head": "6334b338587fe5c635c69a09e134917ec35eaaca",
    "native_baseline_scope": "Prior executed four-test feature-off suite on6334, including this exact test. Currentfa371 adds only the declaration; all runtime/test/manifest/toolchain files byte-identical. No new baseline build claimed.",
    "store_sha256": "6c70a6d4f13de468143f19b75972c28b312a8a2110bf95deb502d784f9b9284c",
    "test_sha256": "f51cb2e74a1af411517b9f744260f2cb69986d724e772e8b22d6c0deaf05c5b1",
    "declaration_sha256": "dfbdf0cb231e671d136b01c8b6f8d1e89198f12c0177abb7a8e35c11733d71ed",
    "control_binary_path": "/private/tmp/sparq-pr6049/.throughput-monitor/direct-5983/implementation/target/debug/deps/sparq_core-53b43ac9cb50e22c",
    "control_binary_sha256": "4fd17c02be04a3a9b083a6b9a2f19c1fc7a1196b3d8a561f2fde6e9753bc0da8",
    "control_binary_bytes": 9899632
  },
  "cleanup_inventory": {
    "source_copy": {
      "path": "/private/tmp/sparq-pr6049/.throughput-monitor/worktrees/layout-metadata-control-fa371",
      "du_allocated_kib": 94772,
      "du_allocated_bytes": 97046528,
      "ownership": "Created solely for this one negative control by exporting exactfa371; only store.rs differs. Full mutant source/diff and exact test are archived in this bundle. No .git metadata/branch was created."
    },
    "warm_target": {
      "path": "/private/tmp/sparq-pr6049/.throughput-monitor/direct-5983/implementation/target",
      "du_allocated_kib": 1893228,
      "du_allocated_bytes": 1938665472,
      "ownership": "Existing task-private native cache reused by prior lanes; NOT wholly attributable to this control. Do not infer all of it is expendable without root decisions. Specific control binary path/hash is in source-state.json."
    },
    "measurement": "du -sk on the two exact paths after the test. Allocated-size reporting may not equal reclaimable physical bytes on APFS. No unrelated directories inventoried or files removed.",
    "free_bytes_after_control": 6418264064,
    "cleanup": "No cleanup performed; root verifies/backups first."
  },
  "decision_owner": "Root independently decides response and any focused actual Opus review; this is Astra assessment, not independent review/admission."
}
````

## source-state.json

````json
{
  "head": "fa3712df77e8acda4c447c6d0d4f87bc3376cc12",
  "source_status": "clean",
  "delta_since_native_baseline_head": [
    "bench/feature-off-declarations/6469.json"
  ],
  "native_baseline_head": "6334b338587fe5c635c69a09e134917ec35eaaca",
  "native_baseline_scope": "Prior executed four-test feature-off suite on6334, including this exact test. Currentfa371 adds only the declaration; all runtime/test/manifest/toolchain files byte-identical. No new baseline build claimed.",
  "store_sha256": "6c70a6d4f13de468143f19b75972c28b312a8a2110bf95deb502d784f9b9284c",
  "test_sha256": "f51cb2e74a1af411517b9f744260f2cb69986d724e772e8b22d6c0deaf05c5b1",
  "declaration_sha256": "dfbdf0cb231e671d136b01c8b6f8d1e89198f12c0177abb7a8e35c11733d71ed",
  "control_binary_path": "/private/tmp/sparq-pr6049/.throughput-monitor/direct-5983/implementation/target/debug/deps/sparq_core-53b43ac9cb50e22c",
  "control_binary_sha256": "4fd17c02be04a3a9b083a6b9a2f19c1fc7a1196b3d8a561f2fde6e9753bc0da8",
  "control_binary_bytes": 9899632
}
````

## control.diff

````diff
--- fa371/store.rs
+++ control/store.rs
@@ -170,6 +170,9 @@
 struct Overlay {
     added: Vec<[Id; 3]>,
     deleted: FxHashSet<[Id; 3]>,
+    // [GPT-6 Astra] Negative control only: uninitialized cache metadata, no reads.
+    #[cfg(not(feature = "overlay-deleted-projections"))]
+    _layout_control_deleted_by_perm: [std::sync::OnceLock<Vec<[Id; 3]>>; 6],
     /// CACHED perm-sorted projections of `added`, indexed by `perm as usize`
     /// (sq-7d3dj.16). See [`Overlay::added_sorted`].
     added_by_perm: [std::sync::OnceLock<Vec<[Id; 3]>>; 6],
````

## control.log

````text
   Compiling sparq-core v0.1.1 (/private/tmp/sparq-pr6049/.throughput-monitor/worktrees/layout-metadata-control-fa371/crates/sparq-core)
    Finished `test` profile [unoptimized] target(s) in 12.57s
     Running unittests src/lib.rs (/private/tmp/sparq-pr6049/.throughput-monitor/direct-5983/implementation/target/debug/deps/sparq_core-53b43ac9cb50e22c)

running 1 test
test store::overlay_deleted_tests::deleted_projection_feature_off_preserves_main_layout_and_heap ... 
thread 'store::overlay_deleted_tests::deleted_projection_feature_off_preserves_main_layout_and_heap' (2806709) panicked at crates/sparq-core/src/store/overlay_deleted_tests.rs:381:5:
assertion `left == right` failed
  left: 440
 right: 248
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace
FAILED

failures:

failures:
    store::overlay_deleted_tests::deleted_projection_feature_off_preserves_main_layout_and_heap

test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 149 filtered out; finished in 0.00s

error: test failed, to rerun pass `-p sparq-core --lib`
````

## prior-identical-runtime-baseline.log

````text
   Compiling sparq-core v0.1.1 (/private/tmp/sparq-pr6049/.throughput-monitor/worktrees/issue4246/crates/sparq-core)
    Finished `test` profile [unoptimized] target(s) in 8.54s
     Running unittests src/lib.rs (/private/tmp/sparq-pr6049/.throughput-monitor/direct-5983/implementation/target/debug/deps/sparq_core-53b43ac9cb50e22c)

running 4 tests
test store::overlay_deleted_tests::deleted_cache_compressed_base_matches_rebuild ... ok
test store::overlay_deleted_tests::deleted_cache_graph_snapshot_retains_warm_generation ... ok
test store::overlay_deleted_tests::deleted_cache_matches_rebuild_after_mixed_deltas ... ok
test store::overlay_deleted_tests::deleted_projection_feature_off_preserves_main_layout_and_heap ... ok

test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 146 filtered out; finished in 0.04s
````

## cleanup-inventory.json

````json
{
  "source_copy": {
    "path": "/private/tmp/sparq-pr6049/.throughput-monitor/worktrees/layout-metadata-control-fa371",
    "du_allocated_kib": 94772,
    "du_allocated_bytes": 97046528,
    "ownership": "Created solely for this one negative control by exporting exactfa371; only store.rs differs. Full mutant source/diff and exact test are archived in this bundle. No .git metadata/branch was created."
  },
  "warm_target": {
    "path": "/private/tmp/sparq-pr6049/.throughput-monitor/direct-5983/implementation/target",
    "du_allocated_kib": 1893228,
    "du_allocated_bytes": 1938665472,
    "ownership": "Existing task-private native cache reused by prior lanes; NOT wholly attributable to this control. Do not infer all of it is expendable without root decisions. Specific control binary path/hash is in source-state.json."
  },
  "measurement": "du -sk on the two exact paths after the test. Allocated-size reporting may not equal reclaimable physical bytes on APFS. No unrelated directories inventoried or files removed.",
  "free_bytes_after_control": 6418264064,
  "cleanup": "No cleanup performed; root verifies/backups first."
}
````

## crates/sparq-core/src/store.rs:160–186

````rust
160: /// triples INSERTED since the last compaction, and base triples DELETED since then.
161: /// Consulted at scan time — the base stays immutable (and mmap-able), and an update
162: /// batch costs O(batch) instead of the O(n) full rebuild. Invariants kept by
163: /// [`TripleStore::apply_delta`]: `added` is canonical-SPO sorted + deduplicated and
164: /// DISJOINT from both the base and `deleted`; `deleted` only ever holds base triples.
165: ///
166: /// `Clone` because [`TripleStore::fork`] carries the overlay into the forked store
167: /// BY VALUE (O(overlay), bounded by the compaction policy) while the base indexes are
168: /// shared structurally.
169: #[derive(Default, Clone)]
170: struct Overlay {
171:     added: Vec<[Id; 3]>,
172:     deleted: FxHashSet<[Id; 3]>,
173:     /// CACHED perm-sorted projections of `added`, indexed by `perm as usize`
174:     /// (sq-7d3dj.16). See [`Overlay::added_sorted`].
175:     added_by_perm: [std::sync::OnceLock<Vec<[Id; 3]>>; 6],
176:     /// [GPT-6 Astra] Lazy deletion projections for range counts (#4246). Keep the
177:     /// hash set above for merge membership; even SPO needs its own sorted projection.
178:     /// Full use retains up to `BUILT.len()` vectors, each with `deleted.len()`
179:     /// twelve-byte rows plus capacity slack, alongside that hash set. The sole
180:     /// production in-place mutator is `TripleStore::apply_delta`, which invalidates
181:     /// these projections when a tombstone is inserted or removed.
182:     #[cfg(feature = "overlay-deleted-projections")]
183:     deleted_by_perm: [std::sync::OnceLock<Vec<[Id; 3]>>; 6],
184: }
185: 
186: impl Overlay {
````

## crates/sparq-core/src/store.rs:325–341

````rust
325:     fn heap_bytes(&self) -> usize {
326:         // The cached perm-sorted projections are part of the overlay's footprint; SPO
327:         // aliases `added`; [GPT-6 Astra] deleted projections own all requested perms.
328:         let cached: usize = self
329:             .added_by_perm
330:             .iter()
331:             .filter_map(|slot| slot.get())
332:             .map(|rows| rows.capacity() * std::mem::size_of::<[Id; 3]>())
333:             .sum();
334:         #[cfg(feature = "overlay-deleted-projections")]
335:         let cached = cached + self.deleted_by_perm.iter()
336:             .filter_map(|slot| slot.get())
337:             .map(|rows| rows.capacity() * std::mem::size_of::<[Id; 3]>())
338:             .sum::<usize>();
339:         self.added.capacity() * std::mem::size_of::<[Id; 3]>()
340:             + self.deleted.capacity() * 13
341:             + cached
````

## crates/sparq-core/src/store.rs:1011–1039

````rust
1011:     /// A structural FORK of this store: the immutable base permutation indexes and
1012:     /// planner stats are SHARED (Arc bumps, O(1)); the pending delta-overlay is
1013:     /// carried by value (O(overlay), bounded by the compaction policy). The fork and
1014:     /// the original then evolve independently through [`apply_delta`](Self::apply_delta)
1015:     /// — neither ever mutates the shared base, so existing readers are unaffected.
1016:     pub fn fork(&self) -> TripleStore {
1017:         TripleStore {
1018:             perms: std::sync::Arc::clone(&self.perms),
1019:             pred_stats: std::sync::Arc::clone(&self.pred_stats),
1020:             overlay: self.overlay.clone(),
1021:         }
1022:     }
1023: 
1024:     /// Number of pending overlay entries (insertions + deletions) — the input to a
1025:     /// compaction threshold policy (a fork costs O(this); folding it costs O(n)).
1026:     pub fn overlay_len(&self) -> usize {
1027:         self.overlay.as_ref().map_or(0, |ov| ov.added.len() + ov.deleted.len())
1028:     }
1029: 
1030:     /// Heap footprint of the permutation indexes in bytes (for benchmarking). Memory-
1031:     /// mapped permutations contribute 0 — their resident pages are OS page cache.
1032:     pub fn heap_bytes(&self) -> usize {
1033:         self.perms.iter().map(PermData::heap_bytes).sum::<usize>()
1034:             + self.overlay.as_ref().map_or(0, |ov| ov.heap_bytes())
1035:     }
1036: 
1037:     /// Chooses the permutation whose sort order places all bound pattern
1038:     /// positions as a contiguous prefix, so the matches form one range. Returns
1039:     /// the permutation and the number of leading bound columns.
````

## crates/sparq-core/src/store/overlay_deleted_tests.rs:350–386

````rust
350: 
351: // [GPT-6 Astra] Default-off must preserve main's representation and linear-count
352: // behavior, not merely return the same rows after allocating a hidden projection.
353: #[cfg(not(feature = "overlay-deleted-projections"))]
354: #[test]
355: fn deleted_projection_feature_off_preserves_main_layout_and_heap() {
356:     let mut store = TripleStore::from_triples(triples());
357:     store.apply_delta(&[], &[[1, 1, 2], [2, 2, 4], [3, 3, 6]]);
358:     let before = store.heap_bytes();
359:     for &perm in BUILT {
360:         for _ in 0..3 {
361:             let scan = store
362:                 .scan_perm(&[Some(12), Some(4), Some(16)], perm)
363:                 .unwrap();
364:             assert_eq!(scan.rows.len(), 1);
365:             assert!(matches!(scan.rows, Cow::Borrowed(_)));
366:             assert_eq!(store.estimate(&[Some(1), Some(1), Some(2)]), 0);
367:             assert_eq!(
368:                 store.heap_bytes(),
369:                 before,
370:                 "default reads retain no deletion projection"
371:             );
372:         }
373:     }
374:     let frozen = store.fork();
375:     assert_eq!(frozen.heap_bytes(), before);
376:     // These are the three fields of main's Overlay. All have pointer alignment
377:     // and sizes divisible by that alignment; there is no inter-field padding.
378:     let main_fields = std::mem::size_of::<Vec<[Id; 3]>>()
379:         + std::mem::size_of::<rustc_hash::FxHashSet<[Id; 3]>>()
380:         + std::mem::size_of::<[std::sync::OnceLock<Vec<[Id; 3]>>; 6]>();
381:     assert_eq!(std::mem::size_of::<Overlay>(), main_fields);
382:     assert_eq!(
383:         std::mem::align_of::<Overlay>(),
384:         std::mem::align_of::<usize>()
385:     );
386: }
````

## rust-toolchain.toml:76–87

````text
76: #
77: # Precedence matters and is easy to get wrong. rustup resolves, highest first:
78: #   1. `cargo +<toolchain>` / `rustc +<toolchain>`
79: #   2. the `RUSTUP_TOOLCHAIN` environment variable
80: #   3. a `rustup override set` directory override
81: #   4. THIS FILE
82: #   5. `rustup default`
83: # So this file BEATS every `rustup default stable` in the workflow tree — all 40
84: # of those lines are now COMPLETE no-ops, and is in turn BEATEN by `+toolchain`
85: # and `RUSTUP_TOOLCHAIN`.
86: # [OPUS-5] They add no components either: `rustup default stable` only selects.
87: # Measured: of the 40 `run:` blocks containing `rustup default stable`, 7 also
````

## rust-toolchain.toml:125–135

````text
125: # bump as a change that can red the whole workspace, and verify it as one.
126: #
127: # =============================================================================
128: 
129: [toolchain]
130: # The exact compiler sparq CI is currently GREEN on: `rustup show
131: # active-toolchain` in the `lint` job reported
132: # `stable-x86_64-unknown-linux-gnu unchanged - rustc 1.97.1 (8bab26f4f 2026-07-14)`
133: # on three consecutive successful runs. Pinning the version that passes, not
134: # the newest that exists.
135: channel = "1.97.1"
````

## .github/workflows/ci.yml:397–410

````text
397:   build-archive:
398:     name: build + archive test binaries (+ doctests once)
399:     # [FABLE-5] sq-fmx4u.3: the workspace archive runs whenever the affected
400:     # closure is NON-EMPTY (design §5.2) — the bulk shards it feeds are narrowed,
401:     # not skipped, so they need the archive for any non-empty selection. Only a
402:     # provably-empty closure (every changed path SAFE-listed/non-crate) skips it;
403:     # the shards then skip too via `needs`. Fail-closed: an empty/missing `mode`
404:     # or `affected` output satisfies a disjunct => RUN.
405:     needs: [changes, select]
406:     if: >-
407:       needs.changes.outputs.rust_changed == 'true' &&
408:       (needs.select.outputs.mode != 'selected' || needs.select.outputs.affected != '[]')
409:     runs-on: ubuntu-latest
410:     steps:
````

## .github/workflows/ci.yml:420–429

````text
420:       - name: Rust (preinstalled stable)
421:         # [FABLE-5] CI-economy (2026-07-18 directive): the ubuntu runner image
422:         # ships a rustup stable toolchain; the SHA-pinned action's `rustup
423:         # toolchain install stable` re-downloads the whole toolchain whenever
424:         # the image lags the newest release. Use the preinstalled one —
425:         # `rustup default`/`component add` never trigger a toolchain download.
426:         run: |
427:           rustup default stable
428:           rustup show active-toolchain
429:       - name: Cache cargo + target
````

## .github/workflows/ci.yml:503–522

````text
503:       # .tar.zst is self-contained (binaries + the metadata nextest needs to run them
504:       # on another runner with `--archive-file`), so the shards never recompile.
505:       #
506:       # [OPUS-4.8] The archive runs `--features approx-ann,filtered-ann,vec-predicate`
507:       # (#363, LOAD-BEARING): sparq-vectors' heavy recall/over-fetch/vec-predicate tests are
508:       # MODULE-gated (`#![cfg(feature = ...)]`) and MUST run in this sharded lane — without
509:       # these the gated binaries compile EMPTY and the heavy-hnsw shard's exact filter matches
510:       # ZERO tests (nextest exit 4). These three are the ONLY opt-in features carried here.
511:       # GUARD (sq-vya1): the archive carries NO OTHER opt-in features (we deliberately avoid
512:       # `--all-features` — cross-crate conflicts + heavy/native/network deps would not even
513:       # resolve; see feature-matrix.yml's SCOPE). Any test behind a DEFAULT-OFF feature OTHER
514:       # than those three compiles EMPTY here and runs SILENTLY-zero in the shards — its coverage
515:       # is the JOB OF feature-matrix.yml (per-leg `cargo test -p <crate> --features <set>`,
516:       # gated by ci-summary). When you add/feature-gate a test: a sparq-vectors recall/vec test
517:       # rides these archive features; ANYTHING ELSE must be wired into a feature-matrix.yml leg
518:       # (read that file's GUARD block). Prove a suite is reached: `cargo nextest list -p <crate>
519:       # --features <set>` must SHOW its test names.
520:       - name: Build + archive test binaries (nextest archive)
521:         run: cargo nextest archive --workspace --all-targets --features approx-ann,filtered-ann,vec-predicate --archive-file nextest.tar.zst
522:       # [OPUS-4.8] Upload the archive immediately after the build, BEFORE doctests, so
````

## .github/workflows/miri.yml:145–151

````text
145: env:
146:   CARGO_TERM_COLOR: always
147:   RUST_BACKTRACE: 1
148:   # See the header for why each flag is load-bearing. Kept in one place so a future toolchain
149:   # bump only edits here.
150:   MIRIFLAGS: "-Zmiri-tree-borrows -Zmiri-ignore-leaks -Zmiri-disable-isolation"
151:
````

## .github/workflows/miri.yml:183–206

````text
183:     # [OPUS-5] PINNED-TOOLCHAIN ESCAPE. The repo root carries a `rust-toolchain.toml`
184:     # pinning STABLE. rustup resolves that file (precedence 4) ABOVE the `rustup default
185:     # <toolchain>` that `dtolnay/rust-toolchain` performs (precedence 5), so with the pin
186:     # in place a plain `cargo`/`rustc` in this job resolves to pinned stable, NOT the
187:     # nightly this lane declares. The `cargo +nightly-2026-06-11 miri …` calls below are
188:     # precedence 1 and were already immune; this env var (precedence 2) restores the
189:     # pre-pin behaviour for everything else in the job — notably the `rustc -vV` that
190:     # Swatinem/rust-cache derives its cache key from, which would otherwise silently
191:     # re-key the Miri cache onto stable. Keep in sync with the `toolchain:` input below.
192:     env:
193:       RUSTUP_TOOLCHAIN: nightly-2026-06-11
194:     steps:
195:       - uses: actions/checkout@9c091bb21b7c1c1d1991bb908d89e4e9dddfe3e0 # v7.0.0
196:         with:
197:           persist-credentials: false
198:       # Miri ships ONLY on nightly. Pin the nightly channel by DATE so the lane is
199:       # reproducible and a future nightly regression cannot silently break it; bump the date
200:       # deliberately (same pinned dtolnay action + nightly the `fuzz` lane uses). The `miri`
201:       # + `rust-src` components are required: `cargo miri` builds a Miri sysroot from source.
202:       - name: Install Rust (nightly, pinned) + miri
203:         uses: dtolnay/rust-toolchain@29eef336d9b2848a0b548edc03f92a220660cdb8 # nightly via toolchain input
204:         with:
205:           toolchain: nightly-2026-06-11
206:           components: miri, rust-src
````

## .github/workflows/miri.yml:248–266

````text
248:       - name: cargo miri nextest run (shard ${{ matrix.shard }}/24)
249:         env:
250:           NEXTEST_EXPERIMENTAL_LIBTEST_JSON: "1"
251:         run: >-
252:           cargo +nightly-2026-06-11 miri nextest run -p sparq-core
253:           --partition count:${{ matrix.shard }}/24 --no-fail-fast
254:           --message-format libtest-json-plus --message-format-version 0.1
255:           > "miri-shard-${{ matrix.shard }}.jsonl" 2> "miri-shard-${{ matrix.shard }}.stderr"
256:           || true
257:       # Classify: a genuine UB/assertion failure reds the shard (exit 1); timeout-only debt is
258:       # tolerated (exit 0). This step's exit code IS the shard verdict, so a real UB regression
259:       # still reds the workflow and re-fires the no-verdict alarm.
260:       - name: Classify shard verdict (UB failure = fatal, timeout debt = non-fatal)
261:         run: |
262:           cat "miri-shard-${{ matrix.shard }}.stderr" || true
263:           python3 scripts/miri_classify_verdict.py \
264:             --shard "${{ matrix.shard }}/24" \
265:             --events "miri-shard-${{ matrix.shard }}.jsonl"
266:
````

## .github/workflows/asan.yml:107–119

````text
107:     # default <toolchain>` that `dtolnay/rust-toolchain` performs (precedence 5). The
108:     # `cargo +nightly-2026-06-11 test -Zbuild-std …` calls below are precedence 1 and were
109:     # already immune; this env var (precedence 2) restores the pre-pin behaviour for every
110:     # other cargo/rustc invocation in the job, including the `rustc -vV` that
111:     # Swatinem/rust-cache keys its cache on. `-Zbuild-std` requires nightly, so a silent
112:     # fallback to stable here would be a hard failure rather than a quiet one — but it
113:     # would still be caused by the pin, not by the sanitizer. Keep in sync with the
114:     # `toolchain:` input below.
115:     env:
116:       RUSTUP_TOOLCHAIN: nightly-2026-06-11
117:     steps:
118:       - uses: actions/checkout@9c091bb21b7c1c1d1991bb908d89e4e9dddfe3e0 # v7.0.0
119:       # [OPUS-4.8] sq-czlh: reclaim ~20-30 GB of pre-installed SDKs/toolchains BEFORE the
````

## .github/workflows/asan.yml:144–158

````text
144:           # the miri sysroot.
145:           key: asan
146:       # The sparq-core half of the corruption corpus: the deterministic
147:       # mmap_corruption_oracle (truncate + bit-flip sweep over every on-disk section) under
148:       # --features mmap,dict-spill — the exact B5 mmap surface Miri structurally cannot run.
149:       # -Zbuild-std rebuilds std under ASan; --target makes that apply. A heap OOB / UAF / UB
150:       # in the validators on a corrupt fixture fails the run with an ASan report.
151:       - name: ASan — sparq-core mmap corruption oracle
152:         run: |
153:           cargo +nightly-2026-06-11 test -Zbuild-std \
154:             --target "$ASAN_TARGET" \
155:             -p sparq-core --features mmap,dict-spill \
156:             --test mmap_corruption_oracle
157:       # The sparq-vectors half: the VectorStore + DiskAnnIndex open-validation corpus
158:       # (truncated files / wrong magic / oversized lengths). These read their own mmap/
````

## .github/feature-matrix.d/sparq-core.yml:78–79

````text
78:   features: "overlay-deleted-projections"
79:   test: true
````
