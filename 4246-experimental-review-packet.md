# Issue4246 default-off experimental boundary review

Actual implementation runtime: OpenAI GPT-6 Astra xhigh. Exact target b86b5d5ad84bce900762defa094630eb358317a6. Parent explicitly selected experimental opt-in after the prior independent review deferred default-on performance admission. This packet is evidence for review, not an admission authorization.

[GPT-6 Astra] #4246 experimental opt-in revision.

Deletion projection caching is now the explicit, default-off core Cargo feature
`overlay-deleted-projections`. Disabled builds retain main's linear counting and
compile out deletion projection state/bookkeeping. Added caches, Clone policy and
storage APIs are unchanged. The matching README/skill explain the experiment's
cold, synchronization, unbounded-tombstone and retained-generation costs.

The same default semantic/layout/heap fixture executes on main and candidate.
Feature-on tests and compiled mutation controls still protect the reviewed repair;
a forced-cache default build fails the actual heap assertion. The opt-in has an
explicit CI test leg with generated gate-name registration. Exact current test,
clippy, matrix, and known Bash3 preflight results are in report.json and tests/.

The only new measurement varies reads per generation in the declared five values.
Generated paired ranges show the cold loss at low read counts and lower medians
at higher read counts, while retained memory/copy-discard costs remain. See
paired/review-summary.md and the complete JSON: no universal crossover, tail
latency, canonical speedup or default-on admission follows from this fixture.

The source and evidence are ready for focused independent review. Full integration
gates and any publication/admission remain with root.


## Actual prior Opus result only

prior-review-result.json; SHA256 04abb7e199b78c12d15ff9efc76d32ab3ba5e4b3bc02d03168bba0b350012a78

```json
"# Independent review \u2014 sparq #4246\n\n**Reviewed head:** `cb638a42a54fc7c9e11e9101587910e668a5f92a` (base `a42a9e89dec485f6a319c47cb3635c59cb5a2270`)\n**Verdict: `defer_performance_admission`** \u2014 the source repair is sound and I request no correctness change; what I withhold is admission of the default-on behavior. I claim no merge readiness.\n\n## Prior blockers \u2014 status\n\n**B1 \u2014 RESOLVED.** `invalidate_projections` is split into `invalidate_added` (store.rs:~213, unconditional, preserving prior behavior) and `invalidate_deleted` (store.rs:220, conditional). The `deleted_changed` flag is set at exactly the two sites that can mutate the tombstone set \u2014 `deleted_changed |= ov.deleted.insert(*t)` (store.rs:940) and `deleted_changed = true` on `ov.deleted.remove(t)` (store.rs:944) \u2014 and `FxHashSet::insert`/`remove` return true iff the set actually changed, so no mutation escapes the flag. Nothing reads the overlay between mutation and publication: the loop touches only `ov.added` and `self.base_contains` (a `perms[Spo]` probe), and `self.overlay` is `take()`n (store.rs:930) until store.rs:958. The two new tests pin both directions (retention by pointer identity across insert-only/no-op/added-retraction batches; emptiness plus a full `sweep` on grow/undelete/net-unchanged), and four of the five compiled controls kill exactly those assertions (`unconditional_deleted_invalidation`, `remove_deleted_invalidation`, `ignore_tombstone_insert_flag`, `ignore_tombstone_remove_flag`). Killed-mutant coverage here is genuine, not a marker assertion.\n\n**B3 \u2014 RESOLVED as an inspection claim.** With store.rs:327\u2013851, the complete final `TripleStore` impl, the Graph lifecycle bodies and the two grep inventories, `TripleStore::apply_delta` is the only in-place mutator of `ov.deleted`/`deleted_by_perm`; every other route replaces the whole store or graph (`self.store = TripleStore::from_triples(...)` at lib.rs:3059/3429, `*self = reopened` at lib.rs:3469, `std::mem::swap` at lib.rs:3564) or funnels through the same seam (WAL replay lib.rs:1952\u20131956 and `redo_txn_record` lib.rs:2006\u20132033 both reach `store.apply_delta` via `apply_delta_mem` lib.rs:3372). `decompress_to_ram` mutates `perms` only, byte-identically. The invariant is now stated in the field doc (store.rs:176\u2013181). Durable routes were inspected, not crash-tested \u2014 correctly disclosed, and separately gated.\n\n**B2 \u2014 the review gap is discharged; the answer is unfavorable.** The lifecycle harness measures what I asked for and does not launder it: ranges, phases, per-generation retained heap, allocator-requested bytes, no discarded samples, disjointness marked per row. The internal cross-checks hold \u2014 `inplace-insert` candidate retained heap exceeds main by exactly 2,359,300 B \u2248 6 \u00d7 393,216 (six 32,768-row projections); `multi-cold` by exactly 3 \u00d7 393,216 (three patterns, three sorts, confirming prior N3); `concurrent-cold` by one projection (shared initializer). N1/N2/N5 are addressed. N4 was correctly not taken.\n\n## Why default-on admission is unsupported\n\n**A1 \u2014 Ordinary paths regress with disjoint ranges.** `Overlay::deleted_count` (store.rs:~232) pays an O(D log D) sort plus a 12\u00b7D-byte allocation on the first read of each permutation after any tombstone change, reached from `count_correction` \u2192 both the `scan_with` zero-copy predicate and `estimate`.\nTrigger: `Graph::fork()` (lib.rs:2885) \u2192 `apply_delta` whose batch reaches store.rs:940 or :944 \u2192 one query on the child. Measured medians, seven reps, all four generations, both warm settings: **1.19\u20131.31 ms vs 0.195\u20130.237 ms (5.1\u20136.5\u00d7), ranges disjoint.** Multi-pattern first read: **3.83 ms vs 0.96 ms (4.0\u00d7), disjoint.** Two concurrent cold readers: **1.354 ms vs 0.337 ms (4.0\u00d7), disjoint** \u2014 the `OnceLock` initializer replaces N independent O(D) scans with one serialized sort that every first reader blocks on. This is precisely the `fork \u2192 apply_delta \u2192 publish` generation pattern the code itself documents.\n\n**A2 \u2014 Retained and duplicated projection memory is unbounded, and B1's fix extends it.** `#[derive(Default, Clone)]` (store.rs:169) plus `overlay: self.overlay.clone()` in `TripleStore::fork` deep-copies every initialized projection per fork/snapshot. Measured retained overlay heap at generation 4, six warm perms: **15.52 MB vs 3.73 MB (\u22484.2\u00d7)**; per-window requested bytes 3.22 MB vs 0.86 MB. Because projections now survive insert-only batches, steady-state footprint is strictly above the acfa candidate \u2014 the B1 fix buys 2\u00d7 on `fork-insert` latency by retaining 2.36 MB per generation. There is no cap, eviction, or `deleted.len()` bound in code.\n\n**A3 \u2014 D is not bounded in-code, so A1 and A2 scale with the graph, not the batch.** `clear_default_durable` (lib.rs:3038\u20133061) on a WAL-backed graph collects every live triple and issues one `apply_delta(&[], &triples)`, making `deleted` the entire base at store.rs:940; the same holds for `DELETE WHERE { ?s ?p ?o }`. The first query afterwards then sorts the whole tombstone set per permutation touched and retains 12\u00b7|G| bytes per permutation, per live fork. This trigger is derived from the supplied source path and is **not measured in this packet**; I extrapolate only its direction from the D=32768 row and flag it as a hazard to bound, not a number.\n\nThe warm gains are real and large (multi-warm 28.8 \u00b5s vs 874 \u00b5s; in-place insert 4.8 \u00b5s vs 188 \u00b5s), and I do not discount them. But `paired/amortization.json` is algebra over saved medians with its own stated assumptions \u2014 it is not a measured read/write distribution, carries no tail guarantee, and does not repay retained memory, clone copies, or cold tail latency. A 5\u20137-read break-even asserted from medians cannot admit a change whose losses are concentrated exactly on the first read after every write.\n\n## Recommendation\n\n**Gate the scope; do not stop the candidate and do not redesign now.** The repair is correct and worth keeping as the basis for a decision. Do not enable the deleted projection unconditionally.\n\nI specifically do **not** recommend the two obvious \"fixes\":\n- a `deleted.len()` threshold would erase only the small/medium cold regression; every material finding above sits at D far past any such threshold, and thresholds are out of scope here;\n- resetting `deleted_by_perm` in a hand-written `Clone` would remove the copy-then-discard waste in `fork-tombstone` gen 1 and the retention multiplier, but it converts the measured `fork-insert` win (118 \u00b5s vs 239 \u00b5s) into a cold sort, i.e. it trades one measured win for one measured loss. It also contradicts `deleted_cache_fork_and_clone_are_independent`.\n\n## Nits (not required; do not bundle)\n\n- `invalidate_added` still evicts on delete-only and all-no-op batches even though `added` is unchanged \u2014 the mirror image of B1, pre-existing in main, so not a regression. A symmetric `added_changed` flag is ~3 lines, but it belongs to a separate decision.\n- The store.rs:176\u2013181 invariant is prose-only. A `cfg(debug_assertions)` cross-check of `deleted_count` against the linear filter would make a future unflagged mutation of `ov.deleted` fail loudly instead of silently returning wrong rows through the `scan_with` zero-copy predicate. Optional.\n\n## Evidence status\n\nLocal single-host macOS/aarch64, `lto=false`, one rayon thread (two raw threads in the concurrent case), generated fixture \u2014 non-canonical. I read the logs, controls, summaries and provenance as evidence and checked them for internal consistency only; I did not rerun anything and take no remote action. `control_production_diff_empty` and the harness byte-identity are asserted by `provenance.json`, not independently verified by me. Undischarged: full workspace/nextest shards, W3C conformance, wasm and canonical performance ratchets, mapped-I/O and WAL-replay crash suites, Miri, and Linux preflight (the local failure is the known Bash-3 `mapfile` at `scripts/check-privacy-claims.sh:92`, environmental).\n\n## Smallest bounded next action\n\nOne scoped decision, no new code and no new local timing. Put exactly three facts to the owner of the overlay/compaction policy \u2014 the disjoint 4\u20136.5\u00d7 cold regressions on fork\u2192tombstone\u2192read, multi-pattern cold and two-reader cold; the 4.2\u00d7 retained overlay heap at four generations; and the unbounded-D trigger at lib.rs:3038\u20133061 \u2014 and obtain one binding answer: default-on, opt-in only, or drop. If and only if that answer is \"continue,\" the single measurement I would sanction is a reads-per-generation sweep R \u2208 {1, 2, 4, 8, 16} on the existing `fork-tombstone` case at the same D and reps, replacing the algebra in `paired/amortization.json` with a measured crossover reported with ranges. Nothing else: no threshold tuning, no `Clone`/`Arc` change, no new permutation strategy, no broader optimization search."
```


## Complete focused delta from cb638a42

source/revision.diff; SHA256 a1a9ac96ce41cbb047708392b8baf16b153c5f29dc5beccaae2b94d339046f55

```diff
diff --git a/.github/feature-matrix.d/sparq-core.yml b/.github/feature-matrix.d/sparq-core.yml
index bb0bb9011..fb8381eea 100644
--- a/.github/feature-matrix.d/sparq-core.yml
+++ b/.github/feature-matrix.d/sparq-core.yml
@@ -70,3 +70,10 @@
   crate: "sparq-core"
   features: "spqcprm2,mmap"
   test: true
+
+# [GPT-6 Astra] #4246: explicitly exercise the experimental deletion cache;
+# ordinary/core defaults remain feature-off. Includes ownership/invalidation tests.
+- name: "sparq-core (overlay-deleted-projections)"
+  crate: "sparq-core"
+  features: "overlay-deleted-projections"
+  test: true
diff --git a/bench/overlay-count/README.md b/bench/overlay-count/README.md
index f1e41d12b..8868f7e59 100644
--- a/bench/overlay-count/README.md
+++ b/bench/overlay-count/README.md
@@ -7,8 +7,8 @@ included. Build timing and `count-alloc` binaries separately with the same sourc
 lockfile, release profile, two build jobs, and one Rayon runtime thread.
 
 ```sh
-CARGO_BUILD_JOBS=2 cargo build --locked --offline --release --manifest-path bench/overlay-count/Cargo.toml
-CARGO_BUILD_JOBS=2 cargo build --locked --offline --release --manifest-path bench/overlay-count/Cargo.toml --features count-alloc
+CARGO_BUILD_JOBS=2 cargo build --locked --offline --release --manifest-path bench/overlay-count/Cargo.toml --features sparq-core/overlay-deleted-projections
+CARGO_BUILD_JOBS=2 cargo build --locked --offline --release --manifest-path bench/overlay-count/Cargo.toml --features count-alloc,sparq-core/overlay-deleted-projections
 ```
 
 Save each binary before the next build. Each invocation emits the entire fixed
@@ -59,3 +59,16 @@ sums the overlay heap of the initial graph and retained generations. Existing
 allocator counters separately report live requested bytes. Cold-query cases do not
 prime measured forks. A separate oracle records the multi-pattern query's actual
 projection heap growth and verifies the complete four-row result.
+
+The core experiment is OFF by default. Omit `sparq-core/overlay-deleted-projections`
+for the ordinary linear control; the benchmark does not forward or enable it
+implicitly. Timing and allocator comparisons use the same harness, with the core
+feature state recorded separately in build provenance.
+
+The `reads-per-generation` argument runs only the fixed fork/tombstone/read case:
+32,768 initial tombstones, six initially warmed permutations, four retained child
+generations, and reads per generation in `{1, 2, 4, 8, 16}`. Each child adds one
+tombstone, executes the same ordinary single-pattern SELECT that many times, then
+is retained locally. Earlier generations and the initial graph stay alive. The
+existing two warmups, seven timing and three allocation repetitions apply. This
+measures the declared read counts, not a recommended crossover or tuning threshold.
diff --git a/bench/overlay-count/src/lifecycle.rs b/bench/overlay-count/src/lifecycle.rs
index 7207ea933..beb1bfdbd 100644
--- a/bench/overlay-count/src/lifecycle.rs
+++ b/bench/overlay-count/src/lifecycle.rs
@@ -233,3 +233,80 @@ pub(super) fn run_all() {
         }
     }
 }
+
+/// [GPT-6 Astra] The sole additional measurement authorized after the opt-in
+/// decision: fixed deletion lineage, varying only reads per retained generation.
+pub(super) fn run_reads() {
+    let pristine = base();
+    let base_heap = pristine.store.heap_bytes();
+    let deletions = delta(D, false);
+    let tombstones: Vec<_> = (0..GENERATIONS)
+        .map(|i| {
+            [
+                iri(&format!("urn:s:{}", 20_000 + i)),
+                iri("urn:p:0"),
+                iri(&format!("urn:o:{}", 20_000 + i)),
+            ]
+        })
+        .collect();
+    check_query(&pristine);
+    println!("{{\"kind\":\"reads_fixture\",\"subjects\":{SUBJECTS},\"predicates\":{PREDICATES},\"deletions\":{D},\"generations\":{GENERATIONS},\"initial_warm_perms\":6,\"rayon_threads\":1,\"counting\":{},\"setup_process_peak_rss_bytes\":{}}}",cfg!(feature="count-alloc"),rss());
+    for (reads, name) in [
+        (1, "fork-tombstone-R1"),
+        (2, "fork-tombstone-R2"),
+        (4, "fork-tombstone-R4"),
+        (8, "fork-tombstone-R8"),
+        (16, "fork-tombstone-R16"),
+    ] {
+        let reps = if cfg!(feature = "count-alloc") { 3 } else { 7 };
+        for rep in 0..reps + 2 {
+            let initial = fork(&pristine, &deletions, false);
+            prime(&initial, 6);
+            let initial_heap = initial.store.heap_bytes() - base_heap;
+            let mut generations: Vec<Graph> = Vec::with_capacity(GENERATIONS);
+            let mut records = [Sample::default(); GENERATIONS];
+            for generation in 0..GENERATIONS {
+                let mut sample = measure(|| {
+                    let start = Instant::now();
+                    let mut next = generations.last().unwrap_or(&initial).fork();
+                    let cloned = start.elapsed().as_nanos();
+                    let delta = Instant::now();
+                    next.apply_delta(&[], std::slice::from_ref(&tombstones[generation]))
+                        .unwrap();
+                    let delta = delta.elapsed().as_nanos();
+                    let read = Instant::now();
+                    assert_eq!(run(&next, "query", reads), reads);
+                    let read = read.elapsed().as_nanos();
+                    generations.push(next);
+                    [cloned, delta, read]
+                });
+                sample.initial_heap = initial_heap;
+                sample.retained_heap = initial_heap
+                    + generations
+                        .iter()
+                        .map(|g| g.store.heap_bytes() - base_heap)
+                        .sum::<usize>();
+                records[generation] = sample;
+            }
+            for (generation, graph) in generations.iter().enumerate() {
+                check_query(graph);
+                assert_eq!(
+                    graph.store.len(),
+                    SUBJECTS * PREDICATES - D - generation - 1
+                );
+                for (i, tombstone) in tombstones.iter().enumerate() {
+                    let pattern = tombstone.clone().map(|t| Some(graph.dict.lookup(&t)));
+                    assert_eq!(
+                        graph.store.scan(&pattern).rows.len(),
+                        usize::from(i > generation)
+                    );
+                }
+            }
+            if rep >= 2 {
+                for (generation, &sample) in records.iter().enumerate() {
+                    emit(name, 6, rep - 2, generation + 1, sample);
+                }
+            }
+        }
+    }
+}
diff --git a/bench/overlay-count/src/main.rs b/bench/overlay-count/src/main.rs
index 063a2cf09..0857ef8f7 100644
--- a/bench/overlay-count/src/main.rs
+++ b/bench/overlay-count/src/main.rs
@@ -104,6 +104,10 @@ fn main() {
         .unwrap();
     #[cfg(feature = "count-alloc")]
     counting::calibrate();
+    if std::env::args().nth(1).as_deref() == Some("reads-per-generation") {
+        lifecycle::run_reads();
+        return;
+    }
     if std::env::args().nth(1).as_deref() == Some("lifecycle") {
         lifecycle::run_all();
         return;
diff --git a/crates/sparq-core/Cargo.toml b/crates/sparq-core/Cargo.toml
index dc26565f6..860b41edf 100644
--- a/crates/sparq-core/Cargo.toml
+++ b/crates/sparq-core/Cargo.toml
@@ -30,6 +30,9 @@ parallel = ["dep:rayon", "dep:memchr"]
 # pattern is still answered by one of them, but some merge joins fall back to hashing.
 # ~halves index memory (36 vs 72 B/triple) — for the memory-constrained browser target.
 compact-index = []
+# [GPT-6 Astra] Experimental lazy tombstone projections. OFF by default: cold
+# reads sort and allocate; forks copy retained projections. See the crate README.
+overlay-deleted-projections = []
 # Out-of-core querying: persist the indexes and memory-map them (native only).
 mmap = ["dep:memmap2"]
 # [OPUS-4.8] sq-wihld (survey §A1): OPT-IN per-block Bloom filters on the high-NDV
diff --git a/crates/sparq-core/README.md b/crates/sparq-core/README.md
index 104180743..4a63f6c2c 100644
--- a/crates/sparq-core/README.md
+++ b/crates/sparq-core/README.md
@@ -50,6 +50,19 @@ assert_eq!(count, 1);
 - **Incremental updates** — start from `Graph::new()` / `Graph::default()` (an empty graph) and
   `insert_triple(s, p, o)` / `remove_triple(s, p, o)` a single triple from `oxrdf` terms, or apply
   a whole batch with `apply_delta` — in place, with an optional write-ahead log.
+- **Experimental deletion projections** — [GPT-6 Astra] enable the default-off
+  `overlay-deleted-projections` Cargo feature on `sparq-core` to cache sorted
+  tombstone projections for repeated range counts. The default uses the original
+  linear deletion scan and carries no deletion projection slots. Each requested
+  permutation first sorts the full tombstone set; an actual tombstone change
+  invalidates those projections, and concurrent first readers wait for the same
+  initializer. Each initialized vector retains twelve bytes per deleted triple
+  plus capacity slack, alongside the deletion hash set; up to all built
+  permutations may be retained. Forks and snapshots deep-copy initialized vectors,
+  so memory grows with each retained generation. There is no deletion-count cap
+  or eviction policy. This experiment can regress cold reads and update/read
+  cycles; opt-in is not a general performance recommendation. Reproduce local
+  lifecycle measurements with `bench/overlay-count`; results are noncanonical.
 - **Out-of-core store** — query datasets larger than RAM from a memory-mapped on-disk store,
   with optional block compression and near-zero resident heap. The opt-in `block-bloom` feature
   adds per-block Bloom filters on high-NDV columns to skip the block decode on equality-bound
diff --git a/crates/sparq-core/src/store.rs b/crates/sparq-core/src/store.rs
index d90091aa6..c495a305b 100644
--- a/crates/sparq-core/src/store.rs
+++ b/crates/sparq-core/src/store.rs
@@ -179,6 +179,7 @@ struct Overlay {
     /// twelve-byte rows plus capacity slack, alongside that hash set. The sole
     /// production in-place mutator is `TripleStore::apply_delta`, which invalidates
     /// these projections when a tombstone is inserted or removed.
+    #[cfg(feature = "overlay-deleted-projections")]
     deleted_by_perm: [std::sync::OnceLock<Vec<[Id; 3]>>; 6],
 }
 
@@ -217,6 +218,7 @@ impl Overlay {
     }
 
     /// [GPT-6 Astra] Drops deleted projections only when the tombstone set changed.
+    #[cfg(feature = "overlay-deleted-projections")]
     fn invalidate_deleted(&mut self) {
         for slot in &mut self.deleted_by_perm {
             slot.take();
@@ -229,6 +231,7 @@ impl Overlay {
     /// apply_delta invalidates only the mutated overlay under exclusive access.
     /// Concurrent first readers of the same permutation wait for its one sorting
     /// initializer; the cold sort is serialized for that permutation.
+    #[cfg(feature = "overlay-deleted-projections")]
     fn deleted_count(&self, perm: Perm, lo: [Id; 3], hi: [Id; 3]) -> usize {
         if self.deleted.is_empty() {
             return 0;
@@ -290,10 +293,23 @@ impl Overlay {
 
     /// How many overlay triples fall in the `[lo, hi]` range of `perm` — the exact
     /// correction to a base range count. The `added` side rides the cached perm-sorted
-    /// projection; [GPT-6 Astra] `deleted` uses the same lazy projection strategy.
+    /// projection. [GPT-6 Astra] Deleted triples use the original linear filter by
+    /// default; the experimental feature opts into lazy sorted projections.
     fn count_correction(&self, perm: Perm, lo: [Id; 3], hi: [Id; 3]) -> (usize, usize) {
         let add = self.added_rows(perm, lo, hi).len();
+        #[cfg(feature = "overlay-deleted-projections")]
         let del = self.deleted_count(perm, lo, hi);
+        #[cfg(not(feature = "overlay-deleted-projections"))]
+        let del = {
+            let order = perm.order();
+            self.deleted
+                .iter()
+                .filter(|t| {
+                    let r = [t[order[0]], t[order[1]], t[order[2]]];
+                    r >= lo && r <= hi
+                })
+                .count()
+        };
         (add, del)
     }
 
@@ -307,10 +323,14 @@ impl Overlay {
         let cached: usize = self
             .added_by_perm
             .iter()
-            .chain(&self.deleted_by_perm)
             .filter_map(|slot| slot.get())
             .map(|rows| rows.capacity() * std::mem::size_of::<[Id; 3]>())
             .sum();
+        #[cfg(feature = "overlay-deleted-projections")]
+        let cached = cached + self.deleted_by_perm.iter()
+            .filter_map(|slot| slot.get())
+            .map(|rows| rows.capacity() * std::mem::size_of::<[Id; 3]>())
+            .sum::<usize>();
         self.added.capacity() * std::mem::size_of::<[Id; 3]>()
             + self.deleted.capacity() * 13
             + cached
@@ -932,17 +952,22 @@ impl TripleStore {
         // Preserve deletion projections across inserts/no-ops: only actual tombstone
         // changes require another sort. No overlay read occurs before publication.
         ov.invalidate_added();
+        #[cfg(feature = "overlay-deleted-projections")]
         let mut deleted_changed = false;
         for t in deletes {
             if let Ok(i) = ov.added.binary_search(t) {
                 ov.added.remove(i); // retract a pending insertion
             } else if self.base_contains(*t) {
-                deleted_changed |= ov.deleted.insert(*t);
+                #[cfg(feature = "overlay-deleted-projections")]
+                { deleted_changed |= ov.deleted.insert(*t); }
+                #[cfg(not(feature = "overlay-deleted-projections"))]
+                { ov.deleted.insert(*t); }
             }
         }
         for t in inserts {
             if ov.deleted.remove(t) {
-                deleted_changed = true;
+                #[cfg(feature = "overlay-deleted-projections")]
+                { deleted_changed = true; }
                 continue; // re-insert of a deleted base triple: just undelete
             }
             if self.base_contains(*t) {
@@ -952,6 +977,7 @@ impl TripleStore {
                 ov.added.insert(i, *t);
             }
         }
+        #[cfg(feature = "overlay-deleted-projections")]
         if deleted_changed {
             ov.invalidate_deleted();
         }
diff --git a/crates/sparq-core/src/store/overlay_deleted_tests.rs b/crates/sparq-core/src/store/overlay_deleted_tests.rs
index a3bd4f3e3..c5bb1aab2 100644
--- a/crates/sparq-core/src/store/overlay_deleted_tests.rs
+++ b/crates/sparq-core/src/store/overlay_deleted_tests.rs
@@ -29,6 +29,7 @@ fn sweep(store: &TripleStore, reference: &[[Id; 3]]) {
     for perm in Perm::ALL {
         if !BUILT.contains(&perm) {
             assert!(store.scan_perm(&[None; 3], perm).is_none());
+            #[cfg(feature = "overlay-deleted-projections")]
             if let Some(ov) = &store.overlay {
                 assert!(ov.deleted_by_perm[perm as usize].get().is_none());
             }
@@ -40,6 +41,7 @@ fn sweep(store: &TripleStore, reference: &[[Id; 3]]) {
     }
 }
 
+#[cfg(feature = "overlay-deleted-projections")]
 #[test]
 fn deleted_cache_is_lazy_reused_and_accounted() {
     let mut store = TripleStore::from_triples(triples());
@@ -106,6 +108,7 @@ fn deleted_cache_matches_rebuild_after_mixed_deltas() {
     assert!(!store.has_overlay());
 }
 
+#[cfg(feature = "overlay-deleted-projections")]
 #[test]
 fn deleted_cache_survives_insert_only_and_noop_deltas() {
     let mut reference = triples();
@@ -146,6 +149,7 @@ fn deleted_cache_survives_insert_only_and_noop_deltas() {
     }
 }
 
+#[cfg(feature = "overlay-deleted-projections")]
 #[test]
 fn deleted_cache_actual_tombstone_changes_invalidate_before_publication() {
     let mut reference = triples();
@@ -174,6 +178,7 @@ fn deleted_cache_actual_tombstone_changes_invalidate_before_publication() {
     }
 }
 
+#[cfg(feature = "overlay-deleted-projections")]
 #[test]
 fn deleted_cache_inclusive_bounds_and_empty_ranges() {
     let mut ov = Overlay::default();
@@ -203,6 +208,7 @@ fn deleted_cache_compressed_base_matches_rebuild() {
     sweep(&store, &reference);
 }
 
+#[cfg(feature = "overlay-deleted-projections")]
 #[test]
 fn deleted_cache_fork_and_clone_are_independent() {
     let mut original = TripleStore::from_triples(triples());
@@ -272,6 +278,7 @@ fn deleted_cache_fork_and_clone_are_independent() {
     }
 }
 
+#[cfg(feature = "overlay-deleted-projections")]
 #[test]
 fn deleted_cache_concurrent_first_reads_share_initialized_projection() {
     let mut store = TripleStore::from_triples(triples());
@@ -340,3 +347,40 @@ fn deleted_cache_graph_snapshot_retains_warm_generation() {
         }
     }
 }
+
+// [GPT-6 Astra] Default-off must preserve main's representation and linear-count
+// behavior, not merely return the same rows after allocating a hidden projection.
+#[cfg(not(feature = "overlay-deleted-projections"))]
+#[test]
+fn deleted_projection_feature_off_preserves_main_layout_and_heap() {
+    let mut store = TripleStore::from_triples(triples());
+    store.apply_delta(&[], &[[1, 1, 2], [2, 2, 4], [3, 3, 6]]);
+    let before = store.heap_bytes();
+    for &perm in BUILT {
+        for _ in 0..3 {
+            let scan = store
+                .scan_perm(&[Some(12), Some(4), Some(16)], perm)
+                .unwrap();
+            assert_eq!(scan.rows.len(), 1);
+            assert!(matches!(scan.rows, Cow::Borrowed(_)));
+            assert_eq!(store.estimate(&[Some(1), Some(1), Some(2)]), 0);
+            assert_eq!(
+                store.heap_bytes(),
+                before,
+                "default reads retain no deletion projection"
+            );
+        }
+    }
+    let frozen = store.fork();
+    assert_eq!(frozen.heap_bytes(), before);
+    // These are the three fields of main's Overlay. All have pointer alignment
+    // and sizes divisible by that alignment; there is no inter-field padding.
+    let main_fields = std::mem::size_of::<Vec<[Id; 3]>>()
+        + std::mem::size_of::<rustc_hash::FxHashSet<[Id; 3]>>()
+        + std::mem::size_of::<[std::sync::OnceLock<Vec<[Id; 3]>>; 6]>();
+    assert_eq!(std::mem::size_of::<Overlay>(), main_fields);
+    assert_eq!(
+        std::mem::align_of::<Overlay>(),
+        std::mem::align_of::<usize>()
+    );
+}
diff --git a/scripts/tests/feature-matrix-legnames.golden.txt b/scripts/tests/feature-matrix-legnames.golden.txt
index 63c630c74..2fb05b12d 100644
--- a/scripts/tests/feature-matrix-legnames.golden.txt
+++ b/scripts/tests/feature-matrix-legnames.golden.txt
@@ -19,6 +19,7 @@ opt-in sparq-core (iri-fast)
 opt-in sparq-core (jsonld)
 opt-in sparq-core (mmap, dict-spill)
 opt-in sparq-core (native-ttl)
+opt-in sparq-core (overlay-deleted-projections)
 opt-in sparq-core (spqcprm2, mmap)
 opt-in sparq-engine (algebra-rewrite)
 opt-in sparq-engine (antijoin-static-decline)
diff --git a/skills/sparql-query/SKILL.md b/skills/sparql-query/SKILL.md
index 7269a8c5a..e83775d3d 100644
--- a/skills/sparql-query/SKILL.md
+++ b/skills/sparql-query/SKILL.md
@@ -1049,6 +1049,24 @@ let r = query_view(&v, "SELECT ?s WHERE { GRAPH ?g { ?s ?p ?o } }").unwrap(); //
   let r3 = cache.get_or_eval(&graph, &q, version, &QueryBudget::unlimited())?; // miss (fresh)
   # Ok::<(), String>(())
   ```
+- **Experimental deletion projection caching** — [GPT-6 Astra] opt in only on the
+  direct core dependency:
+
+  ```toml
+  sparq-core = { version = "0.1", features = ["overlay-deleted-projections"] }
+  ```
+
+  Cargo unifies this feature for engine queries using that same core package.
+  No engine, CLI or runtime flag is required or added. It is off in default builds,
+  which keep linear deletion counting and no deletion-cache state. The experiment
+  sorts all tombstones on first use of each permutation; actual tombstone changes
+  invalidate projections, and concurrent first readers share a blocking initializer.
+  Each requested vector retains twelve bytes per tombstone plus capacity slack,
+  in addition to the hash set. Every live fork/snapshot copies initialized vectors;
+  retained generations multiply this cost. No cap or eviction is provided. Cold
+  reads and update/read cycles can regress; measure the intended workload using
+  `bench/overlay-count` before choosing this opt-in. No universal crossover or
+  canonical speedup is claimed.
 - **Sharing one `Graph` across server threads** — a `sparq_core::Graph` (and its read-only
   `GraphSnapshot`) is **`Send + Sync`** (guaranteed by a compile-time assertion in `sparq-core`), so
   it can be shared across the async handlers of an axum/actix/tower server directly with
```


## Core feature declaration and defaults

source/crates/sparq-core/Cargo.toml; SHA256 bacc2b9877e8d12703578b5c28bba1211514b973bb0edd07ad822bf8b125d362

```toml
[package]
name = "sparq-core"
version.workspace = true
edition.workspace = true
license.workspace = true
rust-version.workspace = true
description = "Dictionary-encoded RDF triplestore core: six sorted permutation indexes, parallel + streaming loaders, and an out-of-core memory-mapped store"
repository.workspace = true
homepage.workspace = true
keywords.workspace = true
categories.workspace = true
# [OPUS-4.8] crate-local README so crates.io (package.readme) matches the docs.rs front
# page (lib.rs include_str!("../README.md")) — same content on both registries.
readme = "README.md"

# [OPUS-4.8] wire docs.rs to build all-features + the `docsrs` cfg so the README
# front page and every feature-gated item render on docs.rs.
[package.metadata.docs.rs]
all-features = true
rustdoc-args = ["--cfg", "docsrs"]

[features]
# Parallel index construction via rayon. On by default for native builds; turn it
# off (e.g. for wasm, which has no threads by default) for a smaller, simpler build.
default = ["parallel"]
# `memchr` rides `parallel`: it is only used by the parallel Turtle terminator pre-scan
# (T2), so the wasm build (which disables `parallel`) never links it. [OPUS-4.8]
parallel = ["dep:rayon", "dep:memchr"]
# Build only THREE permutation indexes (SPO, POS, OSP) instead of six. Every triple
# pattern is still answered by one of them, but some merge joins fall back to hashing.
# ~halves index memory (36 vs 72 B/triple) — for the memory-constrained browser target.
compact-index = []
# [GPT-6 Astra] Experimental lazy tombstone projections. OFF by default: cold
# reads sort and allocate; forks copy retained projections. See the crate README.
overlay-deleted-projections = []
# Out-of-core querying: persist the indexes and memory-map them (native only).
mmap = ["dep:memmap2"]
# [OPUS-4.8] sq-wihld (survey §A1): OPT-IN per-block Bloom filters on the high-NDV
# (subject/object) leading column of a BLOCK-COMPRESSED permutation. The directory's
# first-triple already gives an implicit min/max zone map that prunes RANGE scans, but for
# an EQUALITY-BOUND leading column the zone map still leaves a candidate block that must be
# decoded just to find it holds no matching row; the Bloom filter drops that decode when the id
# is provably absent. ([SONNET-4.6] sq-v8ixk measured the block-SKIP opportunity, on synthetic
# high-NDV columns, to be on ABSENT-key lookups: a PRESENT id is already zone-map-pruned to ~one
# block. Skip counts only — no latency result is claimed. See `compress::block_bloom`.)
# OFF by default so the lean default
# / wasm build pays no filter bytes; it is a pure-std bitset (NO new dependency). Zero false
# negatives by construction, so results are identical to the feature-off path — only fewer
# blocks decoded. The filter is in-RAM only and never serialised, so the on-disk SPQCPRM1
# format is unchanged. See `compress::block_bloom`.
block-bloom = []
# [SONNET-4.6] sq-96hp1 (survey §A2 / research/data-structures.md §B1): OPT-IN Elias-Fano and
# Partitioned-Elias-Fano column codecs (`eliasfano` module) supporting **compressed seek** —
# `next_geq(target)` answered directly on the compressed data, which the incumbent
# delta+varint block codec cannot do (it must decode a whole block to seek).
#
# THIS IS A MEASUREMENT-GATED PROTOTYPE. It is deliberately NOT routed as a `PermData` variant
# and no join calls it: the survey's own honest risk is that EF's pointer-chasing select/rank
# LOSES to a linear varint walk on cache-resident streaming scans, so adoption is conditional on
# the A/B (seek latency, scan throughput, bytes moved, heap) in
# `eliasfano::tests::measure_ef_vs_block_varint` — which is NATIVE-ONLY (it times with
# `std::time::Instant`, absent on wasm32), so the wasm32 half of the comparison is UNRESOLVED and
# must be answered before adoption. OFF by default so the shipped store keeps the
# incumbent codec bit-exactly and the lean default / wasm bundle links none of it. Pure `std` —
# NO new dependency.
elias-fano = []
# [FABLE-5] sq-7d3dj.32.2.6 / sq-7d3dj.32.2.7: SPQCPRM2 frame-of-reference col2-reset
# encoding. The SPQCPRM1 block stream writes a col2 that resets after a middle-column
# change (`reset_d1`) as an ABSOLUTE varint; sq-7d3dj.32.2.4 attribution found that bucket
# the dominant grower at scale. SPQCPRM2 instead frame-of-reference encodes that col2 as a
# zigzag delta from the block's FIRST-row col2, which is cheaper when a block's objects
# cluster.
#
# THE V2 *READER* SHIPS UNCONDITIONALLY WITH `mmap` (sq-7d3dj.32.2.7): `open` auto-detects
# `FILE_MAGIC_V2` and decodes it via `decode_block_v2_at`, so any V2 file ever written
# always decodes; a `SPQCPRM1` file decodes byte-identically forever (the backward-compat
# soundness invariant). What `spqcprm2` gates is the *EMITTER*: turning it on lets a build
# request V2 emission (via `with_emit_format` / `SPARQ_EMIT_FORMAT=v2`) plus the `encode_v2`
# in-RAM writer and the round-trip/measurement tests. OFF by default so the shipped store
# keeps *writing* SPQCPRM1 bit-exactly (V2 is NOT defaulted on — a separate decision pending
# a positive real-data B/triple measurement). Requires `mmap` (the on-disk store is where
# the versioned format lives). See `compress::CompressedPerm::encode_v2` and `EmitFormat`.
spqcprm2 = ["mmap"]
# [OPUS-4.8] sq-dvyi: JSON-LD ingest for the in-memory loaders (`load_str` /
# `load_dataset` / `load_reader`). OPT-IN and OFF by default so the DEFAULT lean wasm
# bundle (which the perf-gate tracks via `cargo build -p sparq-wasm`, no features) does
# NOT link `oxjsonld` and stays under the wasm_bundle_bytes floor. JSON-LD is a
# whole-document JSON parse (not chunkable), so it has no parallel/streaming variant to
# gate further. Consumers that want JSON-LD (the site REPL upload/URL path) enable this.
jsonld = ["dep:oxjsonld"]
# [OPUS-4.8] sq-f47w1 (survey §B1): RDF/XML ingest for the in-memory loaders
# (`load_str` / `load_str_with_base` / `parse_to_triples`). OPT-IN and OFF by default,
# mirroring `jsonld`, so the DEFAULT lean wasm bundle (which the perf-gate tracks via
# `cargo build -p sparq-wasm`, no features) does NOT link `oxrdfxml` (it pulls `quick-xml`)
# and stays under the wasm_bundle_bytes floor. RDF/XML is a whole-document XML parse (not
# line-delimited / chunkable), so it has no parallel/streaming variant to gate further.
# The dependency is already proven in-tree (sparq-server's GSP + conformance paths use it).
rdfxml = ["dep:oxrdfxml"]
# Spillable BUILD-time term dictionary (native only, off by default): bound the peak
# build RSS by a configurable memory budget — term occurrences spill to disk and are
# externally sorted/deduplicated into the SAME ids (byte-identical output) as the
# sharded in-RAM consolidation. See research/external-dictionary.md. Rides the
# parallel parse pipeline and the mmap on-disk store, hence the feature deps.
dict-spill = ["mmap", "parallel", "dep:libc"]
# [OPUS-4.8] sq-yj76l (gh #1121): the ergonomic `shared::SharedGraph` handle — an
# `Arc<RwLock<Graph>>` packaging the recommended read-heavy serving pattern for sharing one
# store across the async handlers of an axum/actix/tower server. `Graph` is ALREADY Send+Sync
# (a compile-time assertion in the crate root guarantees it stays so), so this adds NO new
# dependency — it is `std::sync` only — and is OFF by default so the lean default / wasm build
# pays nothing. See the `shared` module docs.
shared = []
# [OPUS-4.8] sq-7d3dj.18: prefix-memoized IRI-validation fast path in FRONT of the full
# `oxiri` RFC-3987 automaton (see the `iri` module). OFF by default so the lean default / wasm
# build never NAMES `oxiri` as a direct dep (it is already in-tree transitively via oxttl, so
# enabling this adds ZERO new compilation) and the parser keeps its current unvalidated-fast
# behaviour. When ON, the byte-level N-Triples / N-Quads loader validates each IRI through the
# fast path (accepting EXACTLY what `oxiri` accepts — a mandatory differential-fuzz gate), so
# the parallel loader reaches conformance parity with the serial oxttl path at fast-path cost.
iri-fast = ["dep:oxiri"]
# [OPUS-4.8] sq-jocpn: a native, hand-rolled byte-level Turtle tokenizer/parser (`ttl` module)
# that interns S/P/O directly into the `Dict`, replacing the oxttl per-chunk worker on the
# serial Turtle path. OFF by default so the incumbent oxttl path stays the shipped behaviour and
# the native parser can be A/B'd first; it must be a byte-identical drop-in (same triples, same
# term canonicalisation, same accept/reject on the W3C TurtleTests) before any thought of
# defaulting it on. Pulls `oxiri` for RFC-3987 base resolution + IRI validation IDENTICAL to
# oxttl's (oxttl resolves through the same `oxiri` automaton) — so IRI resolution is byte-exact by
# construction rather than a hand-rolled re-derivation. `oxiri` is already in-tree transitively via
# oxttl, so naming it here adds no new compiled crate. The default / wasm build never references
# any of this — sparq-core stays lean. See the `ttl` module.
native-ttl = ["dep:oxiri"]

[dependencies]
oxrdf.workspace = true
oxttl.workspace = true
# [OPUS-4.8] sq-7d3dj.18: RFC-3987 IRI automaton, OPT-IN behind `iri-fast` (OFF by default).
# Already present in-tree transitively (oxttl depends on oxiri), so naming it as a direct dep
# under the feature adds no new compiled crate; the default build never references it.
oxiri = { version = "0.2", optional = true }
# [OPUS-4.8] sq-dvyi: JSON-LD ingest for the in-memory loaders (`load_str` /
# `load_dataset` / `load_reader`). Pure-Rust + wasm-portable, but OPT-IN behind the
# `jsonld` feature (OFF by default) so the tracked lean wasm bundle never links it and
# stays under the wasm_bundle_bytes perf floor — the site REPL enables the feature.
oxjsonld = { workspace = true, optional = true }
# [OPUS-4.8] sq-f47w1 (survey §B1): RDF/XML ingest for the in-memory loaders. Same
# oxigraph family as oxrdf/oxttl, OPT-IN behind the `rdfxml` feature (OFF by default) so
# the tracked lean wasm bundle never links it (it pulls `quick-xml`) and stays under the
# wasm_bundle_bytes perf floor. Already proven in-tree (sparq-server pins the same version).
oxrdfxml = { workspace = true, optional = true }
rustc-hash.workspace = true
hashbrown.workspace = true
rayon = { workspace = true, optional = true }
memchr = { workspace = true, optional = true }
memmap2 = { workspace = true, optional = true }
libc = { workspace = true, optional = true }

# [SONNET-4.6] sq-3dyje.2: property-based testing harness — DEV/TEST ONLY (does NOT appear
# in [dependencies] or in any feature, so it is never pulled into the shipped crate or wasm
# bundle). The supply-chain exemptions for proptest 1.11.0 + its rusty-fork 0.3.1 transitive
# are declared in supply-chain/config.toml (safe-to-run, dev-only scope).
[dev-dependencies]
proptest = "1"

# [OPUS-4.8] sq-ueuk (epic sq-toze, gap MS-G4) — register the `kani` cfg so the
# `#[cfg(kani)]` bounded-proof harness in `dict.rs` (the mmap-free `validate_dict_bytes`
# seam) does not trip `-D warnings` (unexpected_cfgs) on the NORMAL build. `cargo kani` sets
# this cfg itself when it model-checks; under a normal build the harness is stripped. Mirrors
# crates/sparq-vectors/Cargo.toml, which registers the same cfg for its `.spqv` validator proof.
[lints.rust]
unexpected_cfgs = { level = "warn", check-cfg = ["cfg(kani)"] }
```


## Default/dependency/identical-main proof

source/default-and-control-proof.json; SHA256 4b7d3867b469ff43e67e3733cb9fe12d861f5240ad6168179d5524238b6e1afc

```json
{
  "feature": "overlay-deleted-projections",
  "core_default": [
    "parallel"
  ],
  "manifest_mentions": [
    "crates/sparq-core/Cargo.toml"
  ],
  "no_dependency_enables_feature": true,
  "core_feature_has_no_dependencies": true,
  "forwarding": "No forwarding added. Direct sparq-core opt-in unifies into its engine consumers; diagnostic uses explicit --features sparq-core/overlay-deleted-projections.",
  "main_control_runtime_exact_after_test_only_suffix": true,
  "test_fixture_identical": true,
  "benchmark_sources_identical": true,
  "baseline_test_limitation": "The exact copied test fixture names an unknown feature in baseline cfg attributes, causing expected check-cfg warnings. Baseline Cargo manifests/runtime are unchanged; false cfg branches are omitted and four default tests execute."
}
```


## Complete affected state/count/mutation/clone/heap and production read bodies

source/complete-affected-store-context.rs; SHA256 0db5f0e20dcd996ba6dd9b4242efd4faccda77e91d7a24db42abb60c8dd0c904

```rust
// Exact store.rs:169-339
#[derive(Default, Clone)]
struct Overlay {
    added: Vec<[Id; 3]>,
    deleted: FxHashSet<[Id; 3]>,
    /// CACHED perm-sorted projections of `added`, indexed by `perm as usize`
    /// (sq-7d3dj.16). See [`Overlay::added_sorted`].
    added_by_perm: [std::sync::OnceLock<Vec<[Id; 3]>>; 6],
    /// [GPT-6 Astra] Lazy deletion projections for range counts (#4246). Keep the
    /// hash set above for merge membership; even SPO needs its own sorted projection.
    /// Full use retains up to `BUILT.len()` vectors, each with `deleted.len()`
    /// twelve-byte rows plus capacity slack, alongside that hash set. The sole
    /// production in-place mutator is `TripleStore::apply_delta`, which invalidates
    /// these projections when a tombstone is inserted or removed.
    #[cfg(feature = "overlay-deleted-projections")]
    deleted_by_perm: [std::sync::OnceLock<Vec<[Id; 3]>>; 6],
}

impl Overlay {
    /// All of `added` projected into `perm` column order and SORTED in it — computed
    /// ONCE per permutation and reused until `added` changes (sq-7d3dj.16).
    ///
    /// Built LAZILY on the first scan that needs this permutation rather than eagerly
    /// for all six in [`TripleStore::apply_delta`]: a write batch then stays O(batch)
    /// (it only drops the caches, see [`Overlay::invalidate_added`]) instead of paying
    /// O(6·k log k) per call, and a store only ever materialises the projections its
    /// query mix actually scans — so the memory cost is bounded by the permutations in
    /// use, not a flat 6×. SPO needs no projection or sort at all: `added` is already
    /// canonical-SPO sorted, so that permutation ALIASES it and costs nothing.
    ///
    /// `OnceLock` (not `RefCell`) because scans take `&self` and `TripleStore` must stay
    /// `Sync`; concurrent readers share the synchronized initialization.
    fn added_sorted(&self, perm: Perm) -> &[[Id; 3]] {
        let order = perm.order();
        if order == [0, 1, 2] {
            return &self.added; // SPO: `added` is already the projection, already sorted
        }
        self.added_by_perm[perm as usize].get_or_init(|| {
            let mut rows: Vec<[Id; 3]> =
                self.added.iter().map(|t| [t[order[0]], t[order[1]], t[order[2]]]).collect();
            rows.sort_unstable();
            rows
        })
    }

    /// Drops added projections before any nonempty delta, preserving prior behavior.
    fn invalidate_added(&mut self) {
        for slot in &mut self.added_by_perm {
            slot.take();
        }
    }

    /// [GPT-6 Astra] Drops deleted projections only when the tombstone set changed.
    #[cfg(feature = "overlay-deleted-projections")]
    fn invalidate_deleted(&mut self) {
        for slot in &mut self.deleted_by_perm {
            slot.take();
        }
    }

    /// [GPT-6 Astra] Counts deletions using a lazily sorted projection (#4246).
    /// The first request costs O(d log d) and one additional vector; later requests
    /// use two binary searches. Clone copies initialized vectors by value, and
    /// apply_delta invalidates only the mutated overlay under exclusive access.
    /// Concurrent first readers of the same permutation wait for its one sorting
    /// initializer; the cold sort is serialized for that permutation.
    #[cfg(feature = "overlay-deleted-projections")]
    fn deleted_count(&self, perm: Perm, lo: [Id; 3], hi: [Id; 3]) -> usize {
        if self.deleted.is_empty() {
            return 0;
        }
        let rows = self.deleted_by_perm[perm as usize].get_or_init(|| {
            let order = perm.order();
            let mut rows: Vec<[Id; 3]> = self
                .deleted
                .iter()
                .map(|t| [t[order[0]], t[order[1]], t[order[2]]])
                .collect();
            rows.sort_unstable();
            rows
        });
        rows.partition_point(|r| *r <= hi) - rows.partition_point(|r| *r < lo)
    }

    /// The `added` triples matching the inclusive `[lo, hi]` key range, as rows in
    /// `perm` column order, SORTED in that order. A BORROWED sub-slice of the cached
    /// perm-sorted projection located by two binary searches — O(log k + m) on k
    /// insertions and m matches, and allocation-free.
    fn added_rows(&self, perm: Perm, lo: [Id; 3], hi: [Id; 3]) -> &[[Id; 3]] {
        let rows = self.added_sorted(perm);
        let start = rows.partition_point(|r| *r < lo);
        let end = rows.partition_point(|r| *r <= hi);
        &rows[start..end]
    }

    /// Merges the (perm-sorted) base rows with the overlay for one scan: base rows whose
    /// canonical triple is deleted are dropped, and the matching `added` rows are merge-
    /// interleaved — so the output keeps the permutation's sort order, preserving the
    /// guarantees downstream merge joins rely on. `added` is disjoint from the base, so
    /// no duplicate handling is needed.
    fn merge(&self, base: &[[Id; 3]], perm: Perm, lo: [Id; 3], hi: [Id; 3]) -> Vec<[Id; 3]> {
        let add = self.added_rows(perm, lo, hi);
        let order = perm.order();
        let mut out = Vec::with_capacity(base.len() + add.len());
        let mut ai = 0;
        let check_deleted = !self.deleted.is_empty();
        for &row in base {
            if check_deleted {
                let mut spo = [0; 3];
                spo[order[0]] = row[0];
                spo[order[1]] = row[1];
                spo[order[2]] = row[2];
                if self.deleted.contains(&spo) {
                    continue;
                }
            }
            while ai < add.len() && add[ai] < row {
                out.push(add[ai]);
                ai += 1;
            }
            out.push(row);
        }
        out.extend_from_slice(&add[ai..]);
        out
    }

    /// How many overlay triples fall in the `[lo, hi]` range of `perm` — the exact
    /// correction to a base range count. The `added` side rides the cached perm-sorted
    /// projection. [GPT-6 Astra] Deleted triples use the original linear filter by
    /// default; the experimental feature opts into lazy sorted projections.
    fn count_correction(&self, perm: Perm, lo: [Id; 3], hi: [Id; 3]) -> (usize, usize) {
        let add = self.added_rows(perm, lo, hi).len();
        #[cfg(feature = "overlay-deleted-projections")]
        let del = self.deleted_count(perm, lo, hi);
        #[cfg(not(feature = "overlay-deleted-projections"))]
        let del = {
            let order = perm.order();
            self.deleted
                .iter()
                .filter(|t| {
                    let r = [t[order[0]], t[order[1]], t[order[2]]];
                    r >= lo && r <= hi
                })
                .count()
        };
        (add, del)
    }

    fn is_empty(&self) -> bool {
        self.added.is_empty() && self.deleted.is_empty()
    }

    fn heap_bytes(&self) -> usize {
        // The cached perm-sorted projections are part of the overlay's footprint; SPO
        // aliases `added`; [GPT-6 Astra] deleted projections own all requested perms.
        let cached: usize = self
            .added_by_perm
            .iter()
            .filter_map(|slot| slot.get())
            .map(|rows| rows.capacity() * std::mem::size_of::<[Id; 3]>())
            .sum();
        #[cfg(feature = "overlay-deleted-projections")]
        let cached = cached + self.deleted_by_perm.iter()
            .filter_map(|slot| slot.get())
            .map(|rows| rows.capacity() * std::mem::size_of::<[Id; 3]>())
            .sum::<usize>();
        self.added.capacity() * std::mem::size_of::<[Id; 3]>()
            + self.deleted.capacity() * 13
            + cached
    }
}



// Exact store.rs:946-985
    pub fn apply_delta(&mut self, inserts: &[[Id; 3]], deletes: &[[Id; 3]]) {
        if inserts.is_empty() && deletes.is_empty() {
            return;
        }
        let mut ov = self.overlay.take().unwrap_or_default();
        // [GPT-6 Astra] Added projections retain the existing conservative reset.
        // Preserve deletion projections across inserts/no-ops: only actual tombstone
        // changes require another sort. No overlay read occurs before publication.
        ov.invalidate_added();
        #[cfg(feature = "overlay-deleted-projections")]
        let mut deleted_changed = false;
        for t in deletes {
            if let Ok(i) = ov.added.binary_search(t) {
                ov.added.remove(i); // retract a pending insertion
            } else if self.base_contains(*t) {
                #[cfg(feature = "overlay-deleted-projections")]
                { deleted_changed |= ov.deleted.insert(*t); }
                #[cfg(not(feature = "overlay-deleted-projections"))]
                { ov.deleted.insert(*t); }
            }
        }
        for t in inserts {
            if ov.deleted.remove(t) {
                #[cfg(feature = "overlay-deleted-projections")]
                { deleted_changed = true; }
                continue; // re-insert of a deleted base triple: just undelete
            }
            if self.base_contains(*t) {
                continue; // already present in the base
            }
            if let Err(i) = ov.added.binary_search(t) {
                ov.added.insert(i, *t);
            }
        }
        #[cfg(feature = "overlay-deleted-projections")]
        if deleted_changed {
            ov.invalidate_deleted();
        }
        self.overlay = if ov.is_empty() { None } else { Some(ov) };
    }


// Exact store.rs:1011-1017
    pub fn fork(&self) -> TripleStore {
        TripleStore {
            perms: std::sync::Arc::clone(&self.perms),
            pred_stats: std::sync::Arc::clone(&self.pred_stats),
            overlay: self.overlay.clone(),
        }
    }


// Exact store.rs:1027-1030
    pub fn heap_bytes(&self) -> usize {
        self.perms.iter().map(PermData::heap_bytes).sum::<usize>()
            + self.overlay.as_ref().map_or(0, |ov| ov.heap_bytes())
    }


// Exact store.rs:1130-1155
    fn scan_with(&self, pattern: &Pattern, perm: Perm, lead: usize) -> Scan<'_> {
        let (lo, hi) = Self::bounds(pattern, perm, lead);
        let base = self.perms[perm as usize].rows_in(lo, hi);
        // The single overlay branch on the scan hot path: with no pending updates the
        // base range is returned untouched (borrowed, zero copies); with an overlay the
        // deleted triples are filtered out and the inserted ones merge-interleaved, so
        // the rows keep the permutation's sort order (merge joins stay valid).
        //
        // ZERO-COPY FAST PATH (sq-7d3dj.3) [OPUS-4.8]: even WITH an overlay, most ranges a small
        // overlay does not touch. `count_correction` tells us exactly how many
        // `added`/`deleted` triples fall in this range; when it is `(0, 0)` the
        // overlay contributes nothing here — no `added` row projects into `[lo, hi]` (so
        // nothing is interleaved) and no in-range base row is deleted (so nothing is
        // dropped) — hence `merge` would reproduce `base` verbatim, rows AND sort order.
        // We therefore return the BORROWED base slice directly, restoring allocation-free
        // scans for every untouched range (the read-mostly mutated-server common case)
        // instead of paying the owned merge path — which copies the whole base range into a
        // fresh `Vec` and merge-interleaves the (separately, already perm-sorted) in-range
        // `added` rows. It never re-sorts the range; the cost is the copy plus the interleave.
        let rows = match &self.overlay {
            None => base,
            Some(ov) if ov.count_correction(perm, lo, hi) == (0, 0) => base,
            Some(ov) => std::borrow::Cow::Owned(ov.merge(&base, perm, lo, hi)),
        };
        Scan { rows, perm }
    }


// Exact store.rs:1161-1172
    pub fn estimate(&self, pattern: &Pattern) -> usize {
        let (perm, lead) = Self::choose(pattern);
        let (lo, hi) = Self::bounds(pattern, perm, lead);
        let base = self.perms[perm as usize].count_in(lo, hi);
        match &self.overlay {
            None => base,
            Some(ov) => {
                let (add, del) = ov.count_correction(perm, lo, hi);
                base + add - del
            }
        }
    }
```


## Complete cfg-aware semantic and default representation tests

source/crates/sparq-core/src/store/overlay_deleted_tests.rs; SHA256 f51cb2e74a1af411517b9f744260f2cb69986d724e772e8b22d6c0deaf05c5b1

```rust
//! [GPT-6 Astra] Deletion projection behavior, ownership, and invalidation (#4246).

use super::{Id, Overlay, Perm, TripleStore, BUILT};
use std::borrow::Cow;

fn triples() -> Vec<[Id; 3]> {
    (1..=12)
        .flat_map(|s| (1..=4).map(move |p| [s, p, s + p]))
        .collect()
}

fn sweep(store: &TripleStore, reference: &[[Id; 3]]) {
    let rebuilt = TripleStore::from_triples(reference.to_vec());
    // Every built permutation, every leading-prefix length, present/absent keys.
    for &perm in BUILT {
        for triple in reference.iter().copied().chain([[0; 3], [Id::MAX; 3]]) {
            for lead in 0..=3 {
                let mut pattern = [None; 3];
                for &col in &perm.order()[..lead] {
                    pattern[col] = Some(triple[col]);
                }
                let actual = store.scan_perm(&pattern, perm).unwrap();
                let expected = rebuilt.scan_perm(&pattern, perm).unwrap();
                assert_eq!(actual.rows, expected.rows, "{perm:?} {pattern:?}");
                assert_eq!(store.estimate(&pattern), rebuilt.estimate(&pattern));
            }
        }
    }
    for perm in Perm::ALL {
        if !BUILT.contains(&perm) {
            assert!(store.scan_perm(&[None; 3], perm).is_none());
            #[cfg(feature = "overlay-deleted-projections")]
            if let Some(ov) = &store.overlay {
                assert!(ov.deleted_by_perm[perm as usize].get().is_none());
            }
        }
    }
    assert_eq!(store.len(), reference.len());
    for &t in reference {
        assert!(store.contains(t));
    }
}

#[cfg(feature = "overlay-deleted-projections")]
#[test]
fn deleted_cache_is_lazy_reused_and_accounted() {
    let mut store = TripleStore::from_triples(triples());
    store.apply_delta(&[], &[[1, 1, 2], [2, 2, 4], [3, 3, 6]]);
    let ov = store.overlay.as_ref().unwrap();
    assert!(ov.deleted_by_perm.iter().all(|s| s.get().is_none()));
    let cold_heap = ov.heap_bytes();
    let mut expected_heap = cold_heap;
    for &perm in BUILT {
        // An untouched exact range still needs its correction before borrowing base.
        let pattern = [Some(12), Some(4), Some(16)];
        let scan = store.scan_perm(&pattern, perm).unwrap();
        assert_eq!(scan.rows.len(), 1);
        assert!(matches!(scan.rows, Cow::Borrowed(_)));
        let rows = ov.deleted_by_perm[perm as usize]
            .get()
            .expect("scan uses cached deletion count");
        assert_eq!(rows.len(), 3);
        assert!(rows.windows(2).all(|w| w[0] < w[1]));
        let pointer = rows.as_ptr();
        // The requested slot is stable across subsequent counts and scans.
        for _ in 0..5 {
            assert_eq!(ov.count_correction(perm, [0; 3], [Id::MAX; 3]), (0, 3));
            assert_eq!(
                ov.deleted_by_perm[perm as usize].get().unwrap().as_ptr(),
                pointer
            );
        }
        expected_heap += rows.capacity() * std::mem::size_of::<[Id; 3]>();
        assert_eq!(ov.heap_bytes(), expected_heap);
        for other in Perm::ALL {
            if other as usize > perm as usize {
                assert!(ov.deleted_by_perm[other as usize].get().is_none());
            }
        }
    }
    let empty = Overlay::default();
    assert_eq!(empty.deleted_count(Perm::Spo, [0; 3], [Id::MAX; 3]), 0);
    assert!(empty.deleted_by_perm.iter().all(|s| s.get().is_none()));
}

#[test]
fn deleted_cache_matches_rebuild_after_mixed_deltas() {
    let mut reference = triples();
    let mut store = TripleStore::from_triples(reference.clone());
    sweep(&store, &reference);
    // Growing deletions, undeleting base, retracting additions, duplicate/no-op
    // updates and delete-then-insert of the same row all traverse warmed caches.
    let batches = [
        (vec![[20, 2, 22]], vec![[1, 1, 2], [2, 2, 4]]),
        (vec![[1, 1, 2]], vec![[3, 3, 6], [20, 2, 22]]),
        (vec![[4, 4, 8], [4, 4, 8]], vec![[4, 4, 8], [99; 3]]),
        (vec![[2, 2, 4], [3, 3, 6]], vec![]),
    ];
    for (inserts, deletes) in batches {
        store.apply_delta(&inserts, &deletes);
        reference.retain(|t| !deletes.contains(t));
        reference.extend(inserts);
        reference.sort_unstable();
        reference.dedup();
        // Correct rows/counts, not merely empty-slot observations, pin invalidation.
        sweep(&store, &reference);
    }
    assert!(!store.has_overlay());
}

#[cfg(feature = "overlay-deleted-projections")]
#[test]
fn deleted_cache_survives_insert_only_and_noop_deltas() {
    let mut reference = triples();
    let mut store = TripleStore::from_triples(reference.clone());
    store.apply_delta(&[], &[[1, 1, 2], [2, 2, 4]]);
    reference.retain(|t| *t != [1, 1, 2] && *t != [2, 2, 4]);
    sweep(&store, &reference);
    let pointers: Vec<_> = BUILT
        .iter()
        .map(|&p| {
            store.overlay.as_ref().unwrap().deleted_by_perm[p as usize]
                .get()
                .unwrap()
                .as_ptr()
        })
        .collect();
    for (inserts, deletes) in [
        (vec![[20, 2, 22]], vec![]), // actual added-set change with tombstones present
        (vec![[20, 2, 22], [3, 3, 6]], vec![[1, 1, 2], [99; 3]]), // all no-ops
        (vec![], vec![[20, 2, 22]]), // retract an addition, not a base deletion
        (vec![], vec![]),
    ] {
        store.apply_delta(&inserts, &deletes);
        for (&perm, &pointer) in BUILT.iter().zip(&pointers) {
            assert_eq!(
                store.overlay.as_ref().unwrap().deleted_by_perm[perm as usize]
                    .get()
                    .expect("unchanged tombstones retain their projection")
                    .as_ptr(),
                pointer,
            );
        }
        reference.retain(|t| !deletes.contains(t));
        reference.extend(inserts);
        reference.sort_unstable();
        reference.dedup();
        sweep(&store, &reference); // also pins the independent added-cache invalidation
    }
}

#[cfg(feature = "overlay-deleted-projections")]
#[test]
fn deleted_cache_actual_tombstone_changes_invalidate_before_publication() {
    let mut reference = triples();
    let mut store = TripleStore::from_triples(reference.clone());
    store.apply_delta(&[], &[[1, 1, 2]]);
    reference.retain(|t| *t != [1, 1, 2]);
    for (inserts, deletes) in [
        (vec![], vec![[2, 2, 4]]),          // grow the deleted set
        (vec![[1, 1, 2]], vec![]),          // undelete an existing tombstone
        (vec![[3, 3, 6]], vec![[3, 3, 6]]), // changes during batch, even if net unchanged
    ] {
        sweep(&store, &reference);
        store.apply_delta(&inserts, &deletes);
        assert!(store
            .overlay
            .as_ref()
            .unwrap()
            .deleted_by_perm
            .iter()
            .all(|s| s.get().is_none()));
        reference.retain(|t| !deletes.contains(t));
        reference.extend(inserts);
        reference.sort_unstable();
        reference.dedup();
        sweep(&store, &reference);
    }
}

#[cfg(feature = "overlay-deleted-projections")]
#[test]
fn deleted_cache_inclusive_bounds_and_empty_ranges() {
    let mut ov = Overlay::default();
    ov.deleted.extend([[0, 1, 2], [2, 3, 4], [Id::MAX; 3]]);
    for perm in Perm::ALL {
        let order = perm.order();
        for &t in &ov.deleted {
            let row = [t[order[0]], t[order[1]], t[order[2]]];
            assert_eq!(ov.deleted_count(perm, row, row), 1);
        }
        assert_eq!(ov.deleted_count(perm, [0; 3], [Id::MAX; 3]), 3);
        assert_eq!(ov.deleted_count(perm, [42; 3], [42; 3]), 0);
    }
}

#[test]
fn deleted_cache_compressed_base_matches_rebuild() {
    let mut reference = triples();
    let mut store = TripleStore::from_triples_compressed(reference.clone());
    store.apply_delta(&[[20, 2, 22]], &[[1, 1, 2], [2, 2, 4]]);
    reference.retain(|t| *t != [1, 1, 2] && *t != [2, 2, 4]);
    reference.push([20, 2, 22]);
    sweep(&store, &reference);
    store.apply_delta(&[[1, 1, 2]], &[[3, 3, 6]]);
    reference.push([1, 1, 2]);
    reference.retain(|t| *t != [3, 3, 6]);
    sweep(&store, &reference);
}

#[cfg(feature = "overlay-deleted-projections")]
#[test]
fn deleted_cache_fork_and_clone_are_independent() {
    let mut original = TripleStore::from_triples(triples());
    original.apply_delta(&[], &[[1, 1, 2]]);
    let cold_fork = original.fork();
    for &perm in BUILT {
        original.scan_perm(&[None; 3], perm).unwrap();
    }
    assert!(cold_fork
        .overlay
        .as_ref()
        .unwrap()
        .deleted_by_perm
        .iter()
        .all(|s| s.get().is_none()));
    let warm_fork = original.fork();
    let cloned_overlay = original.overlay.clone().unwrap();
    for &perm in BUILT {
        let slot = perm as usize;
        let original_rows = original.overlay.as_ref().unwrap().deleted_by_perm[slot]
            .get()
            .unwrap();
        let fork_rows = warm_fork.overlay.as_ref().unwrap().deleted_by_perm[slot]
            .get()
            .unwrap();
        let clone_rows = cloned_overlay.deleted_by_perm[slot].get().unwrap();
        assert_eq!(original_rows, fork_rows);
        assert_ne!(original_rows.as_ptr(), fork_rows.as_ptr());
        assert_ne!(original_rows.as_ptr(), clone_rows.as_ptr());
    }
    original.apply_delta(&[[1, 1, 2]], &[[2, 2, 4]]);
    for &perm in BUILT {
        for frozen in [&cold_fork, &warm_fork] {
            assert_eq!(
                frozen
                    .scan_perm(&[Some(1), Some(1), Some(2)], perm)
                    .unwrap()
                    .rows
                    .len(),
                0
            );
            assert_eq!(
                frozen
                    .scan_perm(&[Some(2), Some(2), Some(4)], perm)
                    .unwrap()
                    .rows
                    .len(),
                1
            );
        }
        assert_eq!(
            original
                .scan_perm(&[Some(1), Some(1), Some(2)], perm)
                .unwrap()
                .rows
                .len(),
            1
        );
        assert_eq!(
            original
                .scan_perm(&[Some(2), Some(2), Some(4)], perm)
                .unwrap()
                .rows
                .len(),
            0
        );
    }
}

#[cfg(feature = "overlay-deleted-projections")]
#[test]
fn deleted_cache_concurrent_first_reads_share_initialized_projection() {
    let mut store = TripleStore::from_triples(triples());
    store.apply_delta(&[], &[[1, 1, 2], [2, 2, 4]]);
    let barrier = std::sync::Barrier::new(2);
    std::thread::scope(|scope| {
        let handles: Vec<_> = (0..2)
            .map(|_| {
                scope.spawn(|| {
                    barrier.wait();
                    BUILT
                        .iter()
                        .map(|&perm| {
                            assert_eq!(store.scan_perm(&[None; 3], perm).unwrap().rows.len(), 46);
                            let rows = store.overlay.as_ref().unwrap().deleted_by_perm
                                [perm as usize]
                                .get()
                                .unwrap();
                            (rows.as_ptr() as usize, rows.clone())
                        })
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        let mut results = handles.into_iter().map(|h| h.join().unwrap());
        assert_eq!(results.next().unwrap(), results.next().unwrap());
    });
}

#[test]
fn deleted_cache_graph_snapshot_retains_warm_generation() {
    use crate::Graph;
    use oxrdf::{NamedNode, Term};
    let term = |s| Term::from(NamedNode::new(s).unwrap());
    let a = [term("urn:a"), term("urn:p"), term("urn:o")];
    let b = [term("urn:b"), term("urn:p"), term("urn:o")];
    let mut graph = Graph::load_str(
        "<urn:a> <urn:p> <urn:o> . <urn:b> <urn:p> <urn:o> .",
        "turtle",
    )
    .unwrap();
    graph.apply_delta(&[], std::slice::from_ref(&a)).unwrap();
    for &perm in BUILT {
        assert_eq!(
            graph.store.scan_perm(&[None; 3], perm).unwrap().rows.len(),
            1
        );
    }
    let snapshot = graph.snapshot();
    graph
        .apply_delta(std::slice::from_ref(&a), std::slice::from_ref(&b))
        .unwrap();
    for (terms, snapshot_len, current_len) in [(a, 0, 1), (b, 1, 0)] {
        let pattern = terms.map(|t| Some(graph.dict.lookup(&t)));
        assert_eq!(snapshot.store.estimate(&pattern), snapshot_len);
        assert_eq!(graph.store.estimate(&pattern), current_len);
        for &perm in BUILT {
            assert_eq!(
                snapshot.store.scan_perm(&pattern, perm).unwrap().rows.len(),
                snapshot_len
            );
            assert_eq!(
                graph.store.scan_perm(&pattern, perm).unwrap().rows.len(),
                current_len
            );
        }
    }
}

// [GPT-6 Astra] Default-off must preserve main's representation and linear-count
// behavior, not merely return the same rows after allocating a hidden projection.
#[cfg(not(feature = "overlay-deleted-projections"))]
#[test]
fn deleted_projection_feature_off_preserves_main_layout_and_heap() {
    let mut store = TripleStore::from_triples(triples());
    store.apply_delta(&[], &[[1, 1, 2], [2, 2, 4], [3, 3, 6]]);
    let before = store.heap_bytes();
    for &perm in BUILT {
        for _ in 0..3 {
            let scan = store
                .scan_perm(&[Some(12), Some(4), Some(16)], perm)
                .unwrap();
            assert_eq!(scan.rows.len(), 1);
            assert!(matches!(scan.rows, Cow::Borrowed(_)));
            assert_eq!(store.estimate(&[Some(1), Some(1), Some(2)]), 0);
            assert_eq!(
                store.heap_bytes(),
                before,
                "default reads retain no deletion projection"
            );
        }
    }
    let frozen = store.fork();
    assert_eq!(frozen.heap_bytes(), before);
    // These are the three fields of main's Overlay. All have pointer alignment
    // and sizes divisible by that alignment; there is no inter-field padding.
    let main_fields = std::mem::size_of::<Vec<[Id; 3]>>()
        + std::mem::size_of::<rustc_hash::FxHashSet<[Id; 3]>>()
        + std::mem::size_of::<[std::sync::OnceLock<Vec<[Id; 3]>>; 6]>();
    assert_eq!(std::mem::size_of::<Overlay>(), main_fields);
    assert_eq!(
        std::mem::align_of::<Overlay>(),
        std::mem::align_of::<usize>()
    );
}
```


## Explicit feature test-matrix entry

source/.github/feature-matrix.d/sparq-core.yml; SHA256 58ca6d0a460976a7a48e502dceeed203e8d8f9292d708a9bdb180dbbcf419e10

```yaml
# Feature-matrix fragment for `sparq-core`.
#
# One self-contained leg mapping per opt-in feature(group). Assembled into the
# feature-matrix `opt-in-features` strategy by scripts/assemble-feature-matrix.py
# (the `setup` job emits the combined matrix as JSON consumed via fromJSON). Adding a
# leg for THIS crate edits no OTHER crate's fragment, so concurrent opt-in-feature PRs for
# DIFFERENT crates never textually conflict (bead sq-ibrze). [OPUS-4.8]
# SCOPE ([SONNET-4.6] issue #2384): adding/removing/renaming a leg changes the emitted
# leg-name set, and THAT is a TWO-FILE change — this fragment AND
# scripts/tests/feature-matrix-legnames.golden.txt. Every other edit here (features,
# test, tier, tier-reason, comments) leaves the name set alone and is a SINGLE-file
# change. See README.md in this directory.
#
# CONTRACT (gate-critical): each leg's emitted check-run NAME is `opt-in <name>`,
# discovered as a REQUIRED check by the ci-summary `gate` aggregator BY NAME. Keep
# `name` free of the whole words "advisory"/"informational" or the leg stops gating.
# Fields: name (required -> check-run name), crate (required), features (required,
# non-empty comma list), test (bool — run `cargo test` for the leg).

# sparq-core: the compact 3-index browser build, and the native out-of-core
# mmap + spillable build-time dictionary.
- name: "sparq-core (compact-index)"
  crate: "sparq-core"
  features: "compact-index"
  test: true

- name: "sparq-core (mmap, dict-spill)"
  crate: "sparq-core"
  features: "mmap,dict-spill"
  test: true

# [OPUS-4.8] sq-dvyi: opt-in JSON-LD ingest (oxjsonld). OFF by default so the
# lean wasm bundle stays under the perf floor; this leg runs the gated
# jsonld_* loader tests (object/array/base-IRI/`@graph`/malformed).
- name: "sparq-core (jsonld)"
  crate: "sparq-core"
  features: "jsonld"
  test: true

# [SONNET-4.6] #3547: exercise the native Turtle parser's inline differential,
# malformed-input parity, and loader tests on every feature-matrix run.
- name: "sparq-core (native-ttl)"
  crate: "sparq-core"
  features: "native-ttl"
  test: true

# [SONNET-4.6] #3547: exercise the opt-in RFC-3987 IRI validation path. These
# tests live in inline cfg(test) modules, so the structural C1 guard cannot infer
# their need for a dedicated feature executor.
- name: "sparq-core (iri-fast)"
  crate: "sparq-core"
  features: "iri-fast"
  test: true

# [OPUS-4.8] sq-wihld (survey §A1): opt-in per-block Bloom filters on the high-NDV
# leading column of a block-compressed permutation. OFF by default (lean core); this
# leg runs the bloom equivalence + block-skip tests (compress::tests::bloom_*).
- name: "sparq-core (block-bloom)"
  crate: "sparq-core"
  features: "block-bloom"
  test: true

# [FABLE-5] sq-7d3dj.32.2.6 / sq-7d3dj.32.2.7: SPQCPRM2 frame-of-reference col2-reset format.
# The V2 EMITTER is OFF by default (the store still WRITES SPQCPRM1 bit-exactly); this leg runs
# the V2 emit-gate + write/open round-trip + streaming-writer byte-identity + cross-version +
# corrupt-magic mutation-witness + SPQCPRM1-vs-SPQCPRM2 differential + zigzag tests, PLUS the
# V2 corruption oracle (`mmap_loader_survives_corruption_compressed_v2`). `mmap` is co-enabled
# (spqcprm2 implies it) so the on-disk write/open path and FILE_MAGIC_V2 tests run.
- name: "sparq-core (spqcprm2, mmap)"
  crate: "sparq-core"
  features: "spqcprm2,mmap"
  test: true

# [GPT-6 Astra] #4246: explicitly exercise the experimental deletion cache;
# ordinary/core defaults remain feature-off. Includes ownership/invalidation tests.
- name: "sparq-core (overlay-deleted-projections)"
  crate: "sparq-core"
  features: "overlay-deleted-projections"
  test: true
```


## Existing CI executor passes declared feature to test/clippy

source/feature-executor-context.py; SHA256 c87dd9e486430c8a497331170b5708250ced07c292bd5917d0c3724c25a11a4e

```python
# Complete existing build_with_retry and run_leg production bodies.
def build_with_retry(crate, features):
    # [OPUS-4.8] sq-hhxc heritage: bounded retry absorbs transient crates.io
    # hiccups during any residual resolution the build performs.
    for attempt in range(1, BUILD_ATTEMPTS + 1):
        rc = run([CARGO, "build", "-p", crate, "--features", features])
        if rc == 0:
            return 0
        if attempt < BUILD_ATTEMPTS:
            print(
                f"cargo build attempt {attempt} failed; retrying...",
                file=sys.stderr,
                flush=True,
            )
    return rc


def run_leg(leg):
    """Build -> (test) -> clippy, exactly the per-leg matrix step set.
    Returns None on success, else the name of the failing step."""
    crate, features = leg["crate"], leg["features"]
    if build_with_retry(crate, features) != 0:
        return "build"
    if leg["test"]:
        if run([CARGO, "test", "-p", crate, "--features", features]) != 0:
            return "test"
    if (
        run(
            [
                CARGO,
                "clippy",
                "-p",
                crate,
                "--features",
                features,
                "--all-targets",
                "--",
                "-D",
                "warnings",
            ]
        )
        != 0
    ):
        return "clippy"
    return None

```


## Benchmark lifecycle and fixed R extension

source/bench/overlay-count/src/lifecycle.rs; SHA256 d02c77e89674cf79f351f419540f1a69d192adae16bdbbc23a8fbce97c587e2b

```rust
//! [GPT-6 Astra] Fixed lifecycle/admission cases requested by the #4246 review.

use super::*;
use sparq_core::{store::Perm, GraphSnapshot};
use std::sync::Barrier;

const D: usize = 32_768;
const GENERATIONS: usize = 4;
const MULTI: &str = "SELECT ?o ?s ?p WHERE { <urn:s:49999> <urn:p:0> ?o . ?s <urn:p:1> <urn:o:49999> . ?s ?p <urn:o:49999> }";

#[derive(Clone, Copy, Default)]
struct Sample {
    elapsed: u128,
    phases: [u128; 3],
    counts: (u64, u64, u64, u64, u64),
    live_before: u64,
    rss_before: u64,
    rss_after: u64,
    retained_heap: usize,
    initial_heap: usize,
}

fn measure(f: impl FnOnce() -> [u128; 3]) -> Sample {
    let rss_before = rss();
    #[cfg(feature = "count-alloc")]
    let live_before = counting::begin();
    #[cfg(not(feature = "count-alloc"))]
    let live_before = 0;
    let start = Instant::now();
    let phases = f();
    let elapsed = start.elapsed().as_nanos();
    #[cfg(feature = "count-alloc")]
    let counts = counting::end(live_before);
    #[cfg(not(feature = "count-alloc"))]
    let counts = (0, 0, 0, 0, 0);
    Sample {
        elapsed,
        phases,
        counts,
        live_before,
        rss_before,
        rss_after: rss(),
        ..Sample::default()
    }
}

fn emit(case: &str, warm_perms: usize, rep: usize, generation: usize, row: Sample) {
    println!("{{\"kind\":\"lifecycle\",\"case\":\"{case}\",\"warm_perms\":{warm_perms},\"rep\":{rep},\"generation\":{generation},\"elapsed_ns\":{},\"phase_ns\":{:?},\"allocs\":{},\"reallocs\":{},\"requested_bytes\":{},\"peak_live_delta_bytes\":{},\"live_before\":{},\"live_after\":{},\"rss_before\":{},\"rss_after\":{},\"retained_overlay_reported_heap\":{},\"initial_overlay_reported_heap\":{}}}", row.elapsed,row.phases,row.counts.0,row.counts.1,row.counts.2,row.counts.3,row.live_before,row.counts.4,row.rss_before,row.rss_after,row.retained_heap,row.initial_heap);
}

fn prime(graph: &Graph, permutations: usize) {
    let pattern = [
        Some(graph.dict.lookup(&iri("urn:s:49999"))),
        Some(graph.dict.lookup(&iri("urn:p:0"))),
        Some(graph.dict.lookup(&iri("urn:o:49999"))),
    ];
    for &perm in &Perm::ALL[..permutations] {
        assert_eq!(graph.store.scan_perm(&pattern, perm).unwrap().rows.len(), 1);
    }
}

fn multi(graph: &Graph) -> usize {
    let result = query(black_box(graph), black_box(MULTI)).unwrap();
    black_box(result.rows.len())
}

fn check_multi(graph: &Graph) {
    let result = query(graph, MULTI).unwrap();
    assert_eq!(result.rows.len(), 4);
    let mut actual: Vec<_> = result
        .rows
        .iter()
        .map(|r| {
            assert_eq!(r[0], Some(iri("urn:o:49999")));
            assert_eq!(r[1], Some(iri("urn:s:49999")));
            r[2].as_ref().unwrap().to_string()
        })
        .collect();
    actual.sort_unstable();
    let expected: Vec<_> = (0..4)
        .map(|p| iri(&format!("urn:p:{p}")).to_string())
        .collect();
    assert_eq!(actual, expected);
}

pub(super) fn run_all() {
    let pristine = base();
    let base_heap = pristine.store.heap_bytes();
    let deletions = delta(D, false);
    let inserts: Vec<_> = (0..GENERATIONS)
        .map(|i| {
            [
                iri(&format!("urn:new:{i}")),
                iri("urn:p:0"),
                iri("urn:new-object"),
            ]
        })
        .collect();
    let tombstones: Vec<_> = (0..GENERATIONS)
        .map(|i| {
            [
                iri(&format!("urn:s:{}", 20_000 + i)),
                iri("urn:p:0"),
                iri(&format!("urn:o:{}", 20_000 + i)),
            ]
        })
        .collect();
    let oracle = fork(&pristine, &deletions, false);
    let before = oracle.store.heap_bytes();
    check_multi(&oracle);
    // This records actual multi-query projection engagement outside measured forks.
    println!("{{\"kind\":\"lifecycle_fixture\",\"subjects\":{SUBJECTS},\"predicates\":{PREDICATES},\"deletions\":{D},\"generations\":{GENERATIONS},\"multi_query_heap_delta\":{},\"counting\":{},\"setup_process_peak_rss_bytes\":{}}}", oracle.store.heap_bytes()-before,cfg!(feature="count-alloc"),rss());
    drop(oracle);
    let cases = [
        ("snapshot", 1),
        ("snapshot", 6),
        ("fork-insert", 1),
        ("fork-insert", 6),
        ("fork-tombstone", 1),
        ("fork-tombstone", 6),
        ("inplace-insert", 6),
        ("multi-cold", 0),
        ("multi-warm", 3),
        ("concurrent-cold", 0),
    ];
    for (case, warm_perms) in cases {
        let reps = if cfg!(feature = "count-alloc") { 3 } else { 7 };
        for rep in 0..reps + 2 {
            let mut initial = fork(&pristine, &deletions, false);
            if case == "multi-warm" {
                check_multi(&initial); // SPO, POS, OSP via three actual BGP estimates
            } else if warm_perms > 0 {
                prime(&initial, warm_perms);
            }
            let initial_heap = initial.store.heap_bytes() - base_heap;
            let mut records = [Sample::default(); GENERATIONS];
            let count = if matches!(case, "multi-cold" | "multi-warm" | "concurrent-cold") {
                1
            } else {
                GENERATIONS
            };
            let mut generations: Vec<Graph> = Vec::with_capacity(GENERATIONS);
            let mut snapshots: Vec<GraphSnapshot> = Vec::with_capacity(GENERATIONS);
            for generation in 0..count {
                let mut sample = measure(|| {
                    if case.starts_with("multi-") {
                        assert_eq!(multi(&initial), 4);
                        return [0; 3];
                    }
                    if case == "concurrent-cold" {
                        let barrier = Barrier::new(2);
                        // Includes thread launch/join overhead; reader durations are
                        // individual observations, not tail-percentile estimates.
                        return std::thread::scope(|scope| {
                            let read = || {
                                barrier.wait();
                                let start = Instant::now();
                                assert_eq!(run(&initial, "query", 1), 1);
                                start.elapsed().as_nanos()
                            };
                            let a = scope.spawn(read);
                            let b = scope.spawn(read);
                            [a.join().unwrap(), b.join().unwrap(), 0]
                        });
                    }
                    let start = Instant::now();
                    if case == "snapshot" {
                        let snapshot = initial.snapshot();
                        let cloned = start.elapsed().as_nanos();
                        let read = Instant::now();
                        assert_eq!(run(&snapshot, "query", 1), 1);
                        let read = read.elapsed().as_nanos();
                        snapshots.push(snapshot); // retain publication target
                        return [cloned, 0, read];
                    }
                    if case == "inplace-insert" {
                        initial
                            .apply_delta(std::slice::from_ref(&inserts[generation]), &[])
                            .unwrap();
                        let delta = start.elapsed().as_nanos();
                        let read = Instant::now();
                        assert_eq!(run(&initial, "query", 1), 1);
                        return [0, delta, read.elapsed().as_nanos()];
                    }
                    let parent = generations.last().unwrap_or(&initial);
                    let mut next = parent.fork();
                    let cloned = start.elapsed().as_nanos();
                    let delta = Instant::now();
                    if case == "fork-insert" {
                        next.apply_delta(std::slice::from_ref(&inserts[generation]), &[])
                            .unwrap();
                    } else {
                        next.apply_delta(&[], std::slice::from_ref(&tombstones[generation]))
                            .unwrap();
                    }
                    let delta = delta.elapsed().as_nanos();
                    let read = Instant::now();
                    assert_eq!(run(&next, "query", 1), 1);
                    let read = read.elapsed().as_nanos();
                    generations.push(next); // local ownership publication, no service I/O
                    [cloned, delta, read]
                });
                sample.initial_heap = initial_heap;
                sample.retained_heap = initial.store.heap_bytes() - base_heap
                    + generations
                        .iter()
                        .map(|g| g.store.heap_bytes() - base_heap)
                        .sum::<usize>()
                    + snapshots
                        .iter()
                        .map(|g| g.store.heap_bytes() - base_heap)
                        .sum::<usize>();
                records[generation] = sample;
            }
            // Verify the retained generation contents outside every measured window.
            for (i, graph) in generations.iter().enumerate() {
                check_query(graph);
                assert_eq!(
                    graph.store.len(),
                    SUBJECTS * PREDICATES - D + if case == "fork-insert" { i + 1 } else { 0 }
                        - if case == "fork-tombstone" { i + 1 } else { 0 }
                );
            }
            for graph in &snapshots {
                check_query(graph);
                assert_eq!(graph.store.len(), SUBJECTS * PREDICATES - D);
            }
            if rep >= 2 {
                for (generation, &record) in records[..count].iter().enumerate() {
                    emit(case, warm_perms, rep - 2, generation + 1, record);
                }
            }
        }
    }
}

/// [GPT-6 Astra] The sole additional measurement authorized after the opt-in
/// decision: fixed deletion lineage, varying only reads per retained generation.
pub(super) fn run_reads() {
    let pristine = base();
    let base_heap = pristine.store.heap_bytes();
    let deletions = delta(D, false);
    let tombstones: Vec<_> = (0..GENERATIONS)
        .map(|i| {
            [
                iri(&format!("urn:s:{}", 20_000 + i)),
                iri("urn:p:0"),
                iri(&format!("urn:o:{}", 20_000 + i)),
            ]
        })
        .collect();
    check_query(&pristine);
    println!("{{\"kind\":\"reads_fixture\",\"subjects\":{SUBJECTS},\"predicates\":{PREDICATES},\"deletions\":{D},\"generations\":{GENERATIONS},\"initial_warm_perms\":6,\"rayon_threads\":1,\"counting\":{},\"setup_process_peak_rss_bytes\":{}}}",cfg!(feature="count-alloc"),rss());
    for (reads, name) in [
        (1, "fork-tombstone-R1"),
        (2, "fork-tombstone-R2"),
        (4, "fork-tombstone-R4"),
        (8, "fork-tombstone-R8"),
        (16, "fork-tombstone-R16"),
    ] {
        let reps = if cfg!(feature = "count-alloc") { 3 } else { 7 };
        for rep in 0..reps + 2 {
            let initial = fork(&pristine, &deletions, false);
            prime(&initial, 6);
            let initial_heap = initial.store.heap_bytes() - base_heap;
            let mut generations: Vec<Graph> = Vec::with_capacity(GENERATIONS);
            let mut records = [Sample::default(); GENERATIONS];
            for generation in 0..GENERATIONS {
                let mut sample = measure(|| {
                    let start = Instant::now();
                    let mut next = generations.last().unwrap_or(&initial).fork();
                    let cloned = start.elapsed().as_nanos();
                    let delta = Instant::now();
                    next.apply_delta(&[], std::slice::from_ref(&tombstones[generation]))
                        .unwrap();
                    let delta = delta.elapsed().as_nanos();
                    let read = Instant::now();
                    assert_eq!(run(&next, "query", reads), reads);
                    let read = read.elapsed().as_nanos();
                    generations.push(next);
                    [cloned, delta, read]
                });
                sample.initial_heap = initial_heap;
                sample.retained_heap = initial_heap
                    + generations
                        .iter()
                        .map(|g| g.store.heap_bytes() - base_heap)
                        .sum::<usize>();
                records[generation] = sample;
            }
            for (generation, graph) in generations.iter().enumerate() {
                check_query(graph);
                assert_eq!(
                    graph.store.len(),
                    SUBJECTS * PREDICATES - D - generation - 1
                );
                for (i, tombstone) in tombstones.iter().enumerate() {
                    let pattern = tombstone.clone().map(|t| Some(graph.dict.lookup(&t)));
                    assert_eq!(
                        graph.store.scan(&pattern).rows.len(),
                        usize::from(i > generation)
                    );
                }
            }
            if rep >= 2 {
                for (generation, &sample) in records.iter().enumerate() {
                    emit(name, 6, rep - 2, generation + 1, sample);
                }
            }
        }
    }
}
```


## Fixture/query/time/allocation callers

source/bench/overlay-count/src/main.rs; SHA256 946188525809298d068450911e93b3d0c3bed2e9a7c6cbcd91b713a71e6c5e48

```rust
//! [GPT-6 Astra] Local overlay-count diagnostic; no canonical performance claim.

#[cfg(feature = "count-alloc")]
mod counting;
mod lifecycle;

use oxrdf::{NamedNode, Term};
use sparq_core::Graph;
use sparq_engine::query;
use std::{fmt::Write, hint::black_box, time::Instant};

const SUBJECTS: usize = 50_000;
const PREDICATES: usize = 4;
const DELETIONS: [usize; 5] = [0, 16, 1024, 8192, 32768];
const QUERY: &str = "SELECT ?o WHERE { <urn:s:49999> <urn:p:0> ?o }";

fn iri(value: &str) -> Term {
    NamedNode::new(value).unwrap().into()
}

fn base() -> Graph {
    let mut ttl = String::new();
    for s in 0..SUBJECTS {
        for p in 0..PREDICATES {
            writeln!(ttl, "<urn:s:{s}> <urn:p:{p}> <urn:o:{s}> .").unwrap();
        }
    }
    let graph = Graph::load_str(&ttl, "turtle").unwrap();
    assert_eq!(graph.store.len(), SUBJECTS * PREDICATES);
    // Freeze the dictionary outside every measured window, including no-delta cases.
    drop(graph.fork());
    graph
}

fn delta(n: usize, added: bool) -> Vec<[Term; 3]> {
    (0..n)
        .map(|i| {
            let s = i / PREDICATES + if added { SUBJECTS } else { 0 };
            [
                iri(&format!("urn:s:{s}")),
                iri(&format!("urn:p:{}", i % PREDICATES)),
                iri(&format!("urn:o:{s}")),
            ]
        })
        .collect()
}

fn fork(base: &Graph, delta: &[[Term; 3]], added: bool) -> Graph {
    let mut graph = base.fork();
    if added {
        graph.apply_delta(delta, &[]).unwrap();
    } else {
        graph.apply_delta(&[], delta).unwrap();
    }
    assert_eq!(graph.store.overlay_len(), delta.len());
    graph
}

fn check_query(graph: &Graph) {
    let b = query(graph, QUERY).unwrap();
    assert_eq!(b.rows.len(), 1);
    assert_eq!(b.rows[0][0], Some(iri("urn:o:49999")));
}

fn run(graph: &Graph, workload: &str, iterations: usize) -> usize {
    let pattern = [
        Some(graph.dict.lookup(&iri("urn:s:49999"))),
        Some(graph.dict.lookup(&iri("urn:p:0"))),
        None,
    ];
    let mut count = 0;
    for _ in 0..iterations {
        if workload == "scan" {
            count += black_box(graph.store.scan(black_box(&pattern)).rows.len());
        } else {
            let result = query(black_box(graph), black_box(QUERY)).unwrap();
            let b = black_box(result);
            count += b.rows.len();
        }
    }
    black_box(count)
}

fn rss() -> u64 {
    let mut usage = std::mem::MaybeUninit::<libc::rusage>::uninit();
    // SAFETY: getrusage initializes the valid, writable output on success.
    assert_eq!(
        unsafe { libc::getrusage(libc::RUSAGE_SELF, usage.as_mut_ptr()) },
        0
    );
    // SAFETY: the successful call above initialized the complete structure.
    let bytes = unsafe { usage.assume_init() }.ru_maxrss as u64;
    if cfg!(target_os = "macos") {
        bytes
    } else {
        bytes * 1024
    }
}

fn main() {
    rayon::ThreadPoolBuilder::new()
        .num_threads(1)
        .build_global()
        .unwrap();
    #[cfg(feature = "count-alloc")]
    counting::calibrate();
    if std::env::args().nth(1).as_deref() == Some("reads-per-generation") {
        lifecycle::run_reads();
        return;
    }
    if std::env::args().nth(1).as_deref() == Some("lifecycle") {
        lifecycle::run_all();
        return;
    }
    let graph = base();
    println!("{{\"kind\":\"fixture\",\"subjects\":{SUBJECTS},\"predicates\":{PREDICATES},\"base_triples\":{},\"setup_process_peak_rss_bytes\":{},\"rayon_threads\":1,\"counting\":{}}}", graph.store.len(), rss(), cfg!(feature = "count-alloc"));
    for (n, added) in DELETIONS
        .into_iter()
        .map(|n| (n, false))
        .chain([(8192, true)])
    {
        let changes = delta(n, added);
        // Independent oracle fork: verification cannot initialize a sample's caches.
        let oracle = fork(&graph, &changes, added);
        check_query(&oracle);
        assert_eq!(run(&oracle, "scan", 1), 1);
        drop(oracle);
        for workload in ["scan", "query"] {
            for phase in ["cold", "warm"] {
                let iterations = if phase == "cold" {
                    1
                } else if workload == "scan" {
                    10_000
                } else {
                    100
                };
                let reps = if cfg!(feature = "count-alloc") { 3 } else { 7 };
                // Two unrecorded samples warm code/pages, each on an independent fork.
                for rep in 0..reps + 2 {
                    let sample = fork(&graph, &changes, added);
                    let heap_cold = sample.store.heap_bytes();
                    if phase == "warm" {
                        assert_eq!(run(&sample, workload, 1), 1);
                    }
                    let heap_before = sample.store.heap_bytes();
                    let rss_before = rss();
                    #[cfg(feature = "count-alloc")]
                    let baseline = counting::begin();
                    let start = Instant::now();
                    let rows = run(&sample, workload, iterations);
                    let elapsed = start.elapsed().as_nanos();
                    #[cfg(feature = "count-alloc")]
                    let counts = counting::end(baseline);
                    #[cfg(not(feature = "count-alloc"))]
                    let counts = (0_u64, 0_u64, 0_u64, 0_u64, 0_u64);
                    assert_eq!(rows, iterations);
                    if rep >= 2 {
                        println!("{{\"kind\":\"sample\",\"delta\":{n},\"added\":{added},\"workload\":\"{workload}\",\"phase\":\"{phase}\",\"rep\":{},\"iterations\":{iterations},\"elapsed_ns\":{elapsed},\"allocs\":{},\"reallocs\":{},\"requested_bytes\":{},\"peak_live_delta_bytes\":{},\"store_heap_cold\":{heap_cold},\"store_heap_before\":{heap_before},\"store_heap_after\":{},\"process_peak_rss_before\":{rss_before},\"process_peak_rss_after\":{}}}", rep - 2, counts.0, counts.1, counts.2, counts.3, sample.store.heap_bytes(), rss());
                    }
                }
            }
        }
    }
}
```


## Generated paired ranges and retained memory

paired/review-summary.md; SHA256 5ca56cfedaa6d6f89cad3128e70956a2b43f57ca91cf3dcff36ad09ceb11a2d1

```text
Generated from summary.json and lineage-summary.json. Ranges are min/median/max; all samples are retained.

| R | Four-generation main ns | Four-generation opt-in ns | Median ratio |
|---:|---:|---:|---:|
| 1 | 800459/821000/885791 | 4.94733e+06/5.19854e+06/5.38862e+06 | 6.332 |
| 2 | 1.52462e+06/1.57688e+06/1.794e+06 | 4.84304e+06/5.09096e+06/5.59017e+06 | 3.229 |
| 4 | 2.89433e+06/3.08842e+06/3.66425e+06 | 4.86875e+06/5.35875e+06/5.54346e+06 | 1.735 |
| 8 | 5.72946e+06/5.9655e+06/6.19058e+06 | 4.91437e+06/5.28537e+06/5.38608e+06 | 0.886 |
| 16 | 1.14012e+07/1.22554e+07/1.2867e+07 | 4.98504e+06/5.31346e+06/5.63492e+06 | 0.434 |

| R / generation | Main ns | Opt-in ns | Ratio | Disjoint timing ranges | Main / opt-in requested bytes (median) | Main / opt-in peak live delta (median) | Main / opt-in retained overlay heap (median) |
|---|---:|---:|---:|---|---:|---:|---:|
| 1 / 1 | 223125/227292/290500 | 1.28567e+06/1.38829e+06/1.57121e+06 | 6.108 | True | 858247 / 3.61096e+06 | 854519 / 3.21188e+06 | 1.49094e+06 / 4.24347e+06 |
| 1 / 2 | 193625/196833/200541 | 1.20592e+06/1.23429e+06/1.47112e+06 | 6.271 | True | 858247 / 1.64491e+06 | 854519 / 1.24795e+06 | 2.23642e+06 / 5.38218e+06 |
| 1 / 3 | 191916/192750/200917 | 1.19133e+06/1.22138e+06/1.36096e+06 | 6.337 | True | 858247 / 1.64493e+06 | 854519 / 1.24796e+06 | 2.98189e+06 / 6.5209e+06 |
| 1 / 4 | 190583/193833/204042 | 1.19175e+06/1.23454e+06/1.37221e+06 | 6.369 | True | 858247 / 1.64496e+06 | 854519 / 1.24798e+06 | 3.72736e+06 / 7.65964e+06 |
| 2 / 1 | 398417/429417/455791 | 1.26158e+06/1.30617e+06/1.35992e+06 | 3.042 | True | 864080 / 3.6168e+06 | 854519 / 3.21188e+06 | 1.49094e+06 / 4.24347e+06 |
| 2 / 2 | 373916/382708/423708 | 1.192e+06/1.24971e+06/1.40229e+06 | 3.265 | True | 864080 / 1.65074e+06 | 854519 / 1.24795e+06 | 2.23642e+06 / 5.38218e+06 |
| 2 / 3 | 364417/380792/461583 | 1.17754e+06/1.22088e+06/1.47138e+06 | 3.206 | True | 864080 / 1.65076e+06 | 854519 / 1.24796e+06 | 2.98189e+06 / 6.5209e+06 |
| 2 / 4 | 369583/394334/478833 | 1.19121e+06/1.22729e+06/1.58371e+06 | 3.112 | True | 864080 / 1.65079e+06 | 854519 / 1.24798e+06 | 3.72736e+06 / 7.65964e+06 |
| 4 / 1 | 749084/763542/811750 | 1.28438e+06/1.44675e+06/1.62804e+06 | 1.895 | True | 875746 / 3.62846e+06 | 854519 / 3.21188e+06 | 1.49094e+06 / 4.24347e+06 |
| 4 / 2 | 714542/814125/987917 | 1.19562e+06/1.23979e+06/1.39712e+06 | 1.523 | True | 875746 / 1.66241e+06 | 854519 / 1.24795e+06 | 2.23642e+06 / 5.38218e+06 |
| 4 / 3 | 711875/745709/953041 | 1.18612e+06/1.22996e+06/1.39283e+06 | 1.649 | True | 875746 / 1.66243e+06 | 854519 / 1.24796e+06 | 2.98189e+06 / 6.5209e+06 |
| 4 / 4 | 718833/728000/972500 | 1.19896e+06/1.224e+06/1.46258e+06 | 1.681 | True | 875746 / 1.66245e+06 | 854519 / 1.24798e+06 | 3.72736e+06 / 7.65964e+06 |
| 8 / 1 | 1.45233e+06/1.55446e+06/1.73467e+06 | 1.28517e+06/1.30929e+06/1.35925e+06 | 0.842 | True | 899078 / 3.65179e+06 | 854519 / 3.21188e+06 | 1.49094e+06 / 4.24347e+06 |
| 8 / 2 | 1.41338e+06/1.44129e+06/1.55829e+06 | 1.20754e+06/1.22792e+06/1.42912e+06 | 0.852 | False | 899078 / 1.68574e+06 | 854519 / 1.24795e+06 | 2.23642e+06 / 5.38218e+06 |
| 8 / 3 | 1.41242e+06/1.45938e+06/1.77025e+06 | 1.20721e+06/1.34167e+06/1.42267e+06 | 0.919 | False | 899078 / 1.68576e+06 | 854519 / 1.24796e+06 | 2.98189e+06 / 6.5209e+06 |
| 8 / 4 | 1.40275e+06/1.44217e+06/1.54438e+06 | 1.20854e+06/1.26192e+06/1.58483e+06 | 0.875 | False | 899078 / 1.68579e+06 | 854519 / 1.24798e+06 | 3.72736e+06 / 7.65964e+06 |
| 16 / 1 | 2.86117e+06/2.88917e+06/2.91579e+06 | 1.309e+06/1.33458e+06/1.42675e+06 | 0.462 | True | 945742 / 3.69846e+06 | 854519 / 3.21188e+06 | 1.49094e+06 / 4.24347e+06 |
| 16 / 2 | 2.82783e+06/3.05175e+06/3.49912e+06 | 1.23421e+06/1.25621e+06/1.45696e+06 | 0.412 | True | 945742 / 1.7324e+06 | 854519 / 1.24795e+06 | 2.23642e+06 / 5.38218e+06 |
| 16 / 3 | 2.79075e+06/2.90538e+06/3.36962e+06 | 1.21908e+06/1.26946e+06/1.55883e+06 | 0.437 | True | 945742 / 1.73243e+06 | 854519 / 1.24796e+06 | 2.98189e+06 / 6.5209e+06 |
| 16 / 4 | 2.8155e+06/2.91046e+06/4.13346e+06 | 1.22212e+06/1.27262e+06/1.69446e+06 | 0.437 | True | 945742 / 1.73245e+06 | 854519 / 1.24798e+06 | 3.72736e+06 / 7.65964e+06 |
```


Allocation/peak-live/retained-heap columns were identical in all three allocator repetitions at each point (verified against min/max); the table gives their median. Complete counts, phase timings, requested live growth, RSS, variance and all raw samples remain in paired/summary.json and paired/*.jsonl.


## Outside-window path engagement proof

paired/path-engagement-proof.json; SHA256 5940aef5812f1962ebe49b6bb6f51043a9324134f2c74dd97c81fc3f4033d8f7

```json
{
  "initial_projection_heap_difference": 2359296,
  "all_twenty_points_match_six_requested_projections": true,
  "timed_child_queries": "SPO exact subject/predicate query, after true tombstone mutation invalidates cloned projections; one projection reinitialized per child.",
  "feature_off_proof": "Separate compiled default-off/main fixtures and gate-disabled mutant; no additional off timing matrix performed."
}
```


## Exact build/binary/harness/source provenance

review-provenance.json; SHA256 afda50e7cc1e6323c9a9785cdd9d20f6a7dbec0e1a50778bd46131d21fcd9522

```json
{
  "head": "b86b5d5ad84bce900762defa094630eb358317a6",
  "control_head": "0613b5c3d1f2b1174bcccfcd1a6bb64551ca9485",
  "main_runtime": "a42a9e89dec485f6a319c47cb3635c59cb5a2270",
  "prior_reviewed": "cb638a42a54fc7c9e11e9101587910e668a5f92a",
  "model": "OpenAI GPT-6 Astra xhigh, actual implementation runtime",
  "harness_identical": true,
  "main_test_only_suffix": "Source/main runtime equality after stripping only appended #[cfg(test)] fixture module proven in source/default-and-control-proof.json; baseline Cargo.toml unchanged.",
  "harness_files": {
    "bench/overlay-count/Cargo.toml": "f0cbcc2c7f6ea5e1e576254dc9b3904540833e03ac796505d31bf926f8b1c040",
    "bench/overlay-count/Cargo.lock": "b319e3eafeffff389ed020f2213f89dbc0f7041405a3dc9d42facec6e74681bf",
    "bench/overlay-count/README.md": "73d93c0b757e2186a094768f82cdff9006209598e3442e070f831ebb83c90f4d",
    "bench/overlay-count/src/counting.rs": "25b3712e37c4142edac164b32ce47308123bac8201f8a3ff2e3078cd4b5416f2",
    "bench/overlay-count/src/main.rs": "946188525809298d068450911e93b3d0c3bed2e9a7c6cbcd91b713a71e6c5e48",
    "bench/overlay-count/src/lifecycle.rs": "d02c77e89674cf79f351f419540f1a69d192adae16bdbbc23a8fbce97c587e2b"
  },
  "runtime_files": {
    "candidate": {
      "crates/sparq-core/src/store.rs": "f1fb1b676cb31948879246e6a0d91958da62f6b7491a430aec7918cf65a0209e",
      "crates/sparq-core/src/lib.rs": "3679c762c28c956df60ee519682420c2800993a3b3674b77ed335bb9acba6a45",
      "crates/sparq-core/Cargo.toml": "bacc2b9877e8d12703578b5c28bba1211514b973bb0edd07ad822bf8b125d362",
      "crates/sparq-engine/src/lib.rs": "2efd087a5bb3ba10461ceedcf67172bc34287ce15b26eac95b13b555a14736fb",
      "crates/sparq-engine/src/exec.rs": "71f7279359bf982dad1f751fcbc6b42cd68200daebf0de2f417bca1a20d867fd"
    },
    "main": {
      "crates/sparq-core/src/store.rs": "fed800fce5c9b9cc7332f8494d1507b8f1d07138861ee1d00a2547f8cb227474",
      "crates/sparq-core/src/lib.rs": "3679c762c28c956df60ee519682420c2800993a3b3674b77ed335bb9acba6a45",
      "crates/sparq-core/Cargo.toml": "81b61ab780d45b36e188b5d5fc0ad9bb3c50ff2227fee6b650f34584a415a4df",
      "crates/sparq-engine/src/lib.rs": "2efd087a5bb3ba10461ceedcf67172bc34287ce15b26eac95b13b555a14736fb",
      "crates/sparq-engine/src/exec.rs": "71f7279359bf982dad1f751fcbc6b42cd68200daebf0de2f417bca1a20d867fd"
    }
  },
  "builds": [
    {
      "name": "candidate-time",
      "command": [
        "/Users/jesght/.cargo/bin/cargo",
        "build",
        "--locked",
        "--offline",
        "--release",
        "--manifest-path",
        "bench/overlay-count/Cargo.toml",
        "--features",
        "sparq-core/overlay-deleted-projections"
      ],
      "head": "b86b5d5ad84bce900762defa094630eb358317a6",
      "started": 1788959136.107661,
      "ended": 1788959173.7172859,
      "sha256": "dd806a1c96a6d00325717f620d602bd464e8b1367936afa86ea983bf684d91d3",
      "bytes": 5419136
    },
    {
      "name": "candidate-count",
      "command": [
        "/Users/jesght/.cargo/bin/cargo",
        "build",
        "--locked",
        "--offline",
        "--release",
        "--manifest-path",
        "bench/overlay-count/Cargo.toml",
        "--features",
        "sparq-core/overlay-deleted-projections,count-alloc"
      ],
      "head": "b86b5d5ad84bce900762defa094630eb358317a6",
      "started": 1788959173.7882528,
      "ended": 1788959177.283675,
      "sha256": "00d593358bf0a898866f94a6eb4514fe0f54097864beff27fa883890a69f0f21",
      "bytes": 5420784
    },
    {
      "name": "main-time",
      "command": [
        "/Users/jesght/.cargo/bin/cargo",
        "build",
        "--locked",
        "--offline",
        "--release",
        "--manifest-path",
        "bench/overlay-count/Cargo.toml"
      ],
      "head": "0613b5c3d1f2b1174bcccfcd1a6bb64551ca9485",
      "started": 1788959177.315407,
      "ended": 1788959223.972341,
      "sha256": "77f06415fa9b672a79fa17819715aeefe510258879d4f5178ca24530808d4fe8",
      "bytes": 5401520
    },
    {
      "name": "main-count",
      "command": [
        "/Users/jesght/.cargo/bin/cargo",
        "build",
        "--locked",
        "--offline",
        "--release",
        "--manifest-path",
        "bench/overlay-count/Cargo.toml",
        "--features",
        "count-alloc"
      ],
      "head": "0613b5c3d1f2b1174bcccfcd1a6bb64551ca9485",
      "started": 1788959223.999806,
      "ended": 1788959227.041208,
      "sha256": "c182be236d47dc07a083230a823d23b87f646add545cbae0f488b06097181d07",
      "bytes": 5419824
    }
  ],
  "executions": [
    {
      "name": "main-time",
      "exit": 0,
      "started": 1788959236.712,
      "ended": 1788959238.2788649,
      "binary_sha256": "77f06415fa9b672a79fa17819715aeefe510258879d4f5178ca24530808d4fe8"
    },
    {
      "name": "candidate-time",
      "exit": 0,
      "started": 1788959238.2891881,
      "ended": 1788959240.035935,
      "binary_sha256": "dd806a1c96a6d00325717f620d602bd464e8b1367936afa86ea983bf684d91d3"
    },
    {
      "name": "candidate-count",
      "exit": 0,
      "started": 1788959240.048128,
      "ended": 1788959241.302473,
      "binary_sha256": "00d593358bf0a898866f94a6eb4514fe0f54097864beff27fa883890a69f0f21"
    },
    {
      "name": "main-count",
      "exit": 0,
      "started": 1788959241.311748,
      "ended": 1788959242.452029,
      "binary_sha256": "c182be236d47dc07a083230a823d23b87f646add545cbae0f488b06097181d07"
    }
  ],
  "toolchain": "rustc 1.97.1 (8bab26f4f 2026-07-14)\nbinary: rustc\ncommit-hash: 8bab26f4f68e0e26f0bb7960be334d5b520ea452\ncommit-date: 2026-07-14\nhost: aarch64-apple-darwin\nrelease: 1.97.1\nLLVM version: 22.1.6\n",
  "cargo": "cargo 1.97.1 (c980f4866 2026-06-30)\n",
  "profile": "opt-level=3,lto=false,codegen-units=16,debug=false",
  "build_jobs": 2,
  "rayon_threads": 1,
  "warmups": 2,
  "timing_reps": 7,
  "allocation_reps": 3,
  "optimized_build_wall_seconds": 90.962619708,
  "no_build_during_timing": true,
  "measurement_count": 400,
  "host_evidence": "host.json (same Apple M1/16GiB/macOS host as prior reviewed stage)",
  "generated_at": "2026-09-09T13:09:49.187046+00:00"
}
```


## Full test command/status record

tests/results.json; SHA256 f702359187bf49fc6e18a643bc86566aab43e707bf08d134095d8dc178f7ef68

```json
[
  {
    "name": "store-off",
    "command": [
      "/Users/jesght/.cargo/bin/cargo",
      "test",
      "--locked",
      "--offline",
      "-p",
      "sparq-core",
      "--lib",
      "store::",
      "--",
      "--test-threads=1"
    ],
    "exit": 0,
    "seconds": 7.4393259579999995
  },
  {
    "name": "store-on",
    "command": [
      "/Users/jesght/.cargo/bin/cargo",
      "test",
      "--locked",
      "--offline",
      "-p",
      "sparq-core",
      "--features",
      "overlay-deleted-projections",
      "--lib",
      "store::",
      "--",
      "--test-threads=1"
    ],
    "exit": 0,
    "seconds": 5.032448790999999
  },
  {
    "name": "store-compact-off",
    "command": [
      "/Users/jesght/.cargo/bin/cargo",
      "test",
      "--locked",
      "--offline",
      "-p",
      "sparq-core",
      "--no-default-features",
      "--features",
      "compact-index",
      "--lib",
      "store::",
      "--",
      "--test-threads=1"
    ],
    "exit": 0,
    "seconds": 5.393832249999999
  },
  {
    "name": "store-compact-on",
    "command": [
      "/Users/jesght/.cargo/bin/cargo",
      "test",
      "--locked",
      "--offline",
      "-p",
      "sparq-core",
      "--no-default-features",
      "--features",
      "compact-index,overlay-deleted-projections",
      "--lib",
      "store::",
      "--",
      "--test-threads=1"
    ],
    "exit": 0,
    "seconds": 7.1293376670000015
  },
  {
    "name": "snapshot-fork-off",
    "command": [
      "/Users/jesght/.cargo/bin/cargo",
      "test",
      "--locked",
      "--offline",
      "-p",
      "sparq-core",
      "--test",
      "snapshot",
      "--test",
      "fork_differential",
      "--",
      "--test-threads=1"
    ],
    "exit": 0,
    "seconds": 11.401729583000002
  },
  {
    "name": "clippy-core-off",
    "command": [
      "/Users/jesght/.cargo/bin/cargo",
      "clippy",
      "--locked",
      "--offline",
      "-p",
      "sparq-core",
      "--lib",
      "--",
      "-D",
      "warnings"
    ],
    "exit": 0,
    "seconds": 2.0668088749999995
  },
  {
    "name": "snapshot-fork-on",
    "command": [
      "/Users/jesght/.cargo/bin/cargo",
      "test",
      "--locked",
      "--offline",
      "-p",
      "sparq-core",
      "--features",
      "overlay-deleted-projections",
      "--test",
      "snapshot",
      "--test",
      "fork_differential",
      "--",
      "--test-threads=1"
    ],
    "exit": 0,
    "seconds": 23.340536125
  },
  {
    "name": "clippy-core-on",
    "command": [
      "/Users/jesght/.cargo/bin/cargo",
      "clippy",
      "--locked",
      "--offline",
      "-p",
      "sparq-core",
      "--features",
      "overlay-deleted-projections",
      "--lib",
      "--",
      "-D",
      "warnings"
    ],
    "exit": 0,
    "seconds": 1.8588882499999997
  },
  {
    "name": "clippy-harness",
    "command": [
      "/Users/jesght/.cargo/bin/cargo",
      "clippy",
      "--locked",
      "--offline",
      "--manifest-path",
      "bench/overlay-count/Cargo.toml",
      "--features",
      "count-alloc,sparq-core/overlay-deleted-projections",
      "--",
      "-D",
      "warnings"
    ],
    "exit": 0,
    "seconds": 4.974330958000003
  },
  {
    "name": "feature-matrix",
    "command": [
      "/opt/homebrew/bin/python3",
      "scripts/tests/test_feature_matrix_assemble.py"
    ],
    "exit": 0,
    "seconds": 23.934785250000004
  },
  {
    "name": "preflight",
    "command": [
      "python3",
      "scripts/preflight.py",
      "--base",
      "a42a9e89dec485f6a319c47cb3635c59cb5a2270"
    ],
    "exit": 1,
    "seconds": 1.6346569159999973
  }
]
```


## Actual store-off output

tests/store-off.log; SHA256 17097ad8aea6c940fdbd175a253fe4c6eda164ed2387169d6922b3d6b8a10297

```text
   Compiling sparq-core v0.1.1 (/private/tmp/sparq-pr6049/.throughput-monitor/worktrees/issue4246/crates/sparq-core)
    Finished `test` profile [unoptimized] target(s) in 6.03s
     Running unittests src/lib.rs (/private/tmp/sparq-pr6049/.throughput-monitor/direct-5983/implementation/target/debug/deps/sparq_core-53b43ac9cb50e22c)

running 15 tests
test store::overlay_deleted_tests::deleted_cache_compressed_base_matches_rebuild ... ok
test store::overlay_deleted_tests::deleted_cache_graph_snapshot_retains_warm_generation ... ok
test store::overlay_deleted_tests::deleted_cache_matches_rebuild_after_mixed_deltas ... ok
test store::overlay_deleted_tests::deleted_projection_feature_off_preserves_main_layout_and_heap ... ok
test store::tests::build_raw_perms_no_capacity_slack ... ok
test store::tests::compressed_scans_match_raw ... ok
test store::tests::derive_plan_pairs_are_derivable ... ok
test store::tests::derived_perms_match_radix_all_build ... ok
test store::tests::from_triples_independent_dedup_removes_all_duplicates ... ok
test store::tests::from_triples_perms_match_reference_sort ... ok
test store::tests::measure_derived_vs_radix_all_build ... ignored, timing measurement; non-canonical, run explicitly
test store::tests::overlay_scans_match_rebuild ... ok
test store::tests::overlay_zero_copy_fast_path ... ok
test store::tests::radix_sort_equiv_comparison_sort ... ok
test store::tests::scan_perm_selects_named_permutation ... ok

test result: ok. 14 passed; 0 failed; 1 ignored; 0 measured; 135 filtered out; finished in 0.73s
```


## Actual store-on output

tests/store-on.log; SHA256 8f83db433f7352e68abe351b048e48f2d735b4c3fdd8581910b8d28dc0de875b

```text
   Compiling sparq-core v0.1.1 (/private/tmp/sparq-pr6049/.throughput-monitor/worktrees/issue4246/crates/sparq-core)
    Finished `test` profile [unoptimized] target(s) in 4.07s
     Running unittests src/lib.rs (/private/tmp/sparq-pr6049/.throughput-monitor/direct-5983/implementation/target/debug/deps/sparq_core-5d77b6001c8fb29f)

running 20 tests
test store::overlay_deleted_tests::deleted_cache_actual_tombstone_changes_invalidate_before_publication ... ok
test store::overlay_deleted_tests::deleted_cache_compressed_base_matches_rebuild ... ok
test store::overlay_deleted_tests::deleted_cache_concurrent_first_reads_share_initialized_projection ... ok
test store::overlay_deleted_tests::deleted_cache_fork_and_clone_are_independent ... ok
test store::overlay_deleted_tests::deleted_cache_graph_snapshot_retains_warm_generation ... ok
test store::overlay_deleted_tests::deleted_cache_inclusive_bounds_and_empty_ranges ... ok
test store::overlay_deleted_tests::deleted_cache_is_lazy_reused_and_accounted ... ok
test store::overlay_deleted_tests::deleted_cache_matches_rebuild_after_mixed_deltas ... ok
test store::overlay_deleted_tests::deleted_cache_survives_insert_only_and_noop_deltas ... ok
test store::tests::build_raw_perms_no_capacity_slack ... ok
test store::tests::compressed_scans_match_raw ... ok
test store::tests::derive_plan_pairs_are_derivable ... ok
test store::tests::derived_perms_match_radix_all_build ... ok
test store::tests::from_triples_independent_dedup_removes_all_duplicates ... ok
test store::tests::from_triples_perms_match_reference_sort ... ok
test store::tests::measure_derived_vs_radix_all_build ... ignored, timing measurement; non-canonical, run explicitly
test store::tests::overlay_scans_match_rebuild ... ok
test store::tests::overlay_zero_copy_fast_path ... ok
test store::tests::radix_sort_equiv_comparison_sort ... ok
test store::tests::scan_perm_selects_named_permutation ... ok

test result: ok. 19 passed; 0 failed; 1 ignored; 0 measured; 135 filtered out; finished in 0.58s
```


## Actual store-compact-off output

tests/store-compact-off.log; SHA256 e13023ffd94457e9d4a29da96501c6f004f9bad15f52817a7de010342a3d9ebc

```text
   Compiling sparq-core v0.1.1 (/private/tmp/sparq-pr6049/.throughput-monitor/worktrees/issue4246/crates/sparq-core)
    Finished `test` profile [unoptimized] target(s) in 4.25s
     Running unittests src/lib.rs (/private/tmp/sparq-pr6049/.throughput-monitor/direct-5983/implementation/target/debug/deps/sparq_core-6cd45531027c75c9)

running 15 tests
test store::overlay_deleted_tests::deleted_cache_compressed_base_matches_rebuild ... ok
test store::overlay_deleted_tests::deleted_cache_graph_snapshot_retains_warm_generation ... ok
test store::overlay_deleted_tests::deleted_cache_matches_rebuild_after_mixed_deltas ... ok
test store::overlay_deleted_tests::deleted_projection_feature_off_preserves_main_layout_and_heap ... ok
test store::tests::build_raw_perms_no_capacity_slack ... ok
test store::tests::compressed_scans_match_raw ... ok
test store::tests::derive_plan_pairs_are_derivable ... ok
test store::tests::derived_perms_match_radix_all_build ... ok
test store::tests::from_triples_independent_dedup_removes_all_duplicates ... ok
test store::tests::from_triples_perms_match_reference_sort ... ok
test store::tests::measure_derived_vs_radix_all_build ... ignored, timing measurement; non-canonical, run explicitly
test store::tests::overlay_scans_match_rebuild ... ok
test store::tests::overlay_zero_copy_fast_path ... ok
test store::tests::radix_sort_equiv_comparison_sort ... ok
test store::tests::scan_perm_selects_named_permutation ... ok

test result: ok. 14 passed; 0 failed; 1 ignored; 0 measured; 105 filtered out; finished in 0.60s
```


## Actual store-compact-on output

tests/store-compact-on.log; SHA256 e8a3f13995828fe2efefb21de76ec761b1596af335b982d838b0df5b9393afc5

```text
   Compiling sparq-core v0.1.1 (/private/tmp/sparq-pr6049/.throughput-monitor/worktrees/issue4246/crates/sparq-core)
    Finished `test` profile [unoptimized] target(s) in 6.15s
     Running unittests src/lib.rs (/private/tmp/sparq-pr6049/.throughput-monitor/direct-5983/implementation/target/debug/deps/sparq_core-0d277481354d9443)

running 20 tests
test store::overlay_deleted_tests::deleted_cache_actual_tombstone_changes_invalidate_before_publication ... ok
test store::overlay_deleted_tests::deleted_cache_compressed_base_matches_rebuild ... ok
test store::overlay_deleted_tests::deleted_cache_concurrent_first_reads_share_initialized_projection ... ok
test store::overlay_deleted_tests::deleted_cache_fork_and_clone_are_independent ... ok
test store::overlay_deleted_tests::deleted_cache_graph_snapshot_retains_warm_generation ... ok
test store::overlay_deleted_tests::deleted_cache_inclusive_bounds_and_empty_ranges ... ok
test store::overlay_deleted_tests::deleted_cache_is_lazy_reused_and_accounted ... ok
test store::overlay_deleted_tests::deleted_cache_matches_rebuild_after_mixed_deltas ... ok
test store::overlay_deleted_tests::deleted_cache_survives_insert_only_and_noop_deltas ... ok
test store::tests::build_raw_perms_no_capacity_slack ... ok
test store::tests::compressed_scans_match_raw ... ok
test store::tests::derive_plan_pairs_are_derivable ... ok
test store::tests::derived_perms_match_radix_all_build ... ok
test store::tests::from_triples_independent_dedup_removes_all_duplicates ... ok
test store::tests::from_triples_perms_match_reference_sort ... ok
test store::tests::measure_derived_vs_radix_all_build ... ignored, timing measurement; non-canonical, run explicitly
test store::tests::overlay_scans_match_rebuild ... ok
test store::tests::overlay_zero_copy_fast_path ... ok
test store::tests::radix_sort_equiv_comparison_sort ... ok
test store::tests::scan_perm_selects_named_permutation ... ok

test result: ok. 19 passed; 0 failed; 1 ignored; 0 measured; 105 filtered out; finished in 0.46s
```


## Actual snapshot-fork-off output

tests/snapshot-fork-off.log; SHA256 fb8344e6b338d30b899c699dff258212bb622180675114a7c7a5f4b596ff9f5c

```text
   Compiling sparq-core v0.1.1 (/private/tmp/sparq-pr6049/.throughput-monitor/worktrees/issue4246/crates/sparq-core)
    Finished `test` profile [unoptimized] target(s) in 7.81s
     Running tests/fork_differential.rs (/private/tmp/sparq-pr6049/.throughput-monitor/direct-5983/implementation/target/debug/deps/fork_differential-216e2d0e0f466ae3)

running 3 tests
test fork_chain_matches_flat_rebuild ... ok
test fork_pending_delta_accounting ... ok
test named_graphs_fork_isolated ... ok

test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 2.44s

     Running tests/snapshot.rs (/private/tmp/sparq-pr6049/.throughput-monitor/direct-5983/implementation/target/debug/deps/snapshot-a52f141cc1305978)

running 7 tests
test base_mutation_after_snapshot_invisible_to_snapshot ... ok
test snapshot_covers_named_graphs ... ok
test snapshot_includes_pending_overlay_and_survives_base_compaction ... ok
test snapshot_is_send_sync ... ok
test snapshot_mutation_invisible_to_base ... ok
test snapshot_sees_point_in_time_triples ... ok
test snapshot_shares_base_indexes_no_duplication ... ok

test result: ok. 7 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.21s
```


## Actual snapshot-fork-on output

tests/snapshot-fork-on.log; SHA256 54e3debaaa98ca61ca2e39fff2ba72098e755adf528113fef86a7fde5f778025

```text
   Compiling sparq-core v0.1.1 (/private/tmp/sparq-pr6049/.throughput-monitor/worktrees/issue4246/crates/sparq-core)
    Finished `test` profile [unoptimized] target(s) in 20.10s
     Running tests/fork_differential.rs (/private/tmp/sparq-pr6049/.throughput-monitor/direct-5983/implementation/target/debug/deps/fork_differential-76afb4e7db84ec2b)

running 3 tests
test fork_chain_matches_flat_rebuild ... ok
test fork_pending_delta_accounting ... ok
test named_graphs_fork_isolated ... ok

test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 2.21s

     Running tests/snapshot.rs (/private/tmp/sparq-pr6049/.throughput-monitor/direct-5983/implementation/target/debug/deps/snapshot-2d486a1d329d9038)

running 7 tests
test base_mutation_after_snapshot_invisible_to_snapshot ... ok
test snapshot_covers_named_graphs ... ok
test snapshot_includes_pending_overlay_and_survives_base_compaction ... ok
test snapshot_is_send_sync ... ok
test snapshot_mutation_invisible_to_base ... ok
test snapshot_sees_point_in_time_triples ... ok
test snapshot_shares_base_indexes_no_duplication ... ok

test result: ok. 7 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.20s
```


## Actual clippy-core-off output

tests/clippy-core-off.log; SHA256 1c165e40743690fde4c30cdc0e0f876fc2767ec0fbe4719aedb523f55b97bd54

```text
    Checking sparq-core v0.1.1 (/private/tmp/sparq-pr6049/.throughput-monitor/worktrees/issue4246/crates/sparq-core)
    Finished `dev` profile [unoptimized] target(s) in 1.86s
```


## Actual clippy-core-on output

tests/clippy-core-on.log; SHA256 487027c00e43bbcf782001a04af719731a74db1077cbcf392f1bbfea56c7df13

```text
    Checking sparq-core v0.1.1 (/private/tmp/sparq-pr6049/.throughput-monitor/worktrees/issue4246/crates/sparq-core)
    Finished `dev` profile [unoptimized] target(s) in 1.67s
```


## Actual clippy-harness output

tests/clippy-harness.log; SHA256 f8b8a0da44de0a21231403f090b83c1ca72fc5d07ad06fa43575de3a07b7196f

```text
    Checking sparq-core v0.1.1 (/private/tmp/sparq-pr6049/.throughput-monitor/worktrees/issue4246/crates/sparq-core)
    Checking sparq-substrate v0.1.1 (/private/tmp/sparq-pr6049/.throughput-monitor/worktrees/issue4246/crates/sparq-substrate)
    Checking sparq-engine v0.1.1 (/private/tmp/sparq-pr6049/.throughput-monitor/worktrees/issue4246/crates/sparq-engine)
    Checking overlay-count-diagnostic v0.0.0 (/private/tmp/sparq-pr6049/.throughput-monitor/worktrees/issue4246/bench/overlay-count)
    Finished `dev` profile [unoptimized] target(s) in 4.81s
```


## Actual preflight output

tests/preflight.log; SHA256 1985cb6f29927e86652bbab54103925ffb00ddc67fd233147e5368ec8240754b

```text
preflight: ran  G1 new-crate-completeness, G2 public-api-to-skill, G6 new-config-to-docs, no-perf-numbers, readme-template, privacy-claims, guard-untested

preflight: FAIL — 1 mechanical finding(s):

  [privacy-claims] (diff)
      scripts/check-privacy-claims.sh: line 92: mapfile: command not found
      Exception ignored in: <_io.TextIOWrapper name='<stdout>' mode='w' encoding='utf-8'>
      BrokenPipeError: [Errno 32] Broken pipe
      fix: reproduce with: bash scripts/check-privacy-claims.sh


NOT CHECKED BY THIS SCRIPT — you must execute these yourself before opening the PR.
They are the two largest preventable review-failure classes in the verdict corpus
(GUARD-NOT-PINNED 63 findings, CLAIM-vs-CODE 67 findings) and neither is decidable
by static analysis:

  1. MUTATE YOUR HEADLINE GUARD. Take the feature named in your PR title. DELETE or
     INVERT it in the worktree and RUN the suite. If nothing goes red, your test is
     vacuous — that is a blocking defect, and it is the single most common one the
     reviewers find. Do not reason about it; execute it. Report which test died.
     (`guard-untested` above only catches a guard with NO test at all. A test that
     exists but asserts a bound, a type, or a marker string instead of the behaviour
     passes this script and fails review.)

  2. READ YOUR OWN PROSE AGAINST YOUR OWN DIFF. For every line of documentation,
     rustdoc, README, SKILL.md, comment, research record or PR-body claim you added:
     point at the code in THIS diff that makes it true. If you cannot, delete the
     sentence or fix the code. Overclaiming is blocking. The corpus is full of
     diffs whose docs describe a module, flag, constant or test file that the diff
     does not contain.
```


## Actual identical-main test result

Full log tests/main-identical-default-tests.log SHA256 38ae3e49995dbb20346d55c113fee969167d9bedfb110276f24c29819524e82f. Baseline has no such Cargo feature; identical tests produce expected unknown-feature cfg warnings. No warning is a compile failure; four off tests actually run.

```text
running 4 tests
test store::overlay_deleted_tests::deleted_cache_compressed_base_matches_rebuild ... ok
test store::overlay_deleted_tests::deleted_cache_graph_snapshot_retains_warm_generation ... ok
test store::overlay_deleted_tests::deleted_cache_matches_rebuild_after_mixed_deltas ... ok
test store::overlay_deleted_tests::deleted_projection_feature_off_preserves_main_layout_and_heap ... ok

test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 146 filtered out; finished in 0.05s


```


## Actual matrix-test result

Full 105-test log tests/feature-matrix.log SHA256 ea5e8376e8ee3f2210f2a2a1db56395615708c63721e0cc930774024571a9e1a. Intentional invalid-fragment errors belong to negative fixtures.

```text
rate-size heuristic
error: ../../../../../../var/folders/bg/h3_9072n3hq_xjcw2_f17m3m0000gp/T/tmp8do6c8p8/frag.yml[0]: `weight` must be a positive finite number (got -1); omit it to use the crate-size heuristic
error: ../../../../../../var/folders/bg/h3_9072n3hq_xjcw2_f17m3m0000gp/T/tmp8do6c8p8/frag.yml[0]: `weight` must be a positive finite number (got 'abc'); omit it to use the crate-size heuristic
error: ../../../../../../var/folders/bg/h3_9072n3hq_xjcw2_f17m3m0000gp/T/tmp8do6c8p8/frag.yml[0]: `weight` must be a positive finite number (got True); omit it to use the crate-size heuristic
error: ../../../../../../var/folders/bg/h3_9072n3hq_xjcw2_f17m3m0000gp/T/tmp8do6c8p8/frag.yml[0]: `weight` must be a positive finite number (got nan); omit it to use the crate-size heuristic
error: ../../../../../../var/folders/bg/h3_9072n3hq_xjcw2_f17m3m0000gp/T/tmp8do6c8p8/frag.yml[0]: `weight` must be a positive finite number (got inf); omit it to use the crate-size heuristic
error: ../../../../../../var/folders/bg/h3_9072n3hq_xjcw2_f17m3m0000gp/T/tmp8do6c8p8/frag.yml[0]: `weight` must be a positive finite number (got [1]); omit it to use the crate-size heuristic
ok
test_missing_weight_defaults_to_heuristic (__main__.TestWeightFieldValidation.test_missing_weight_defaults_to_heuristic) ... ok

----------------------------------------------------------------------
Ran 105 tests in 23.436s

OK

```


## Compiled controls, exact commands and calibrated counts

controls/results.json; SHA256 e9ebdaddf93cc95d08ceccd58373ad2bcf425b634982f0826a120ab2ba6dcbd9

```json
{
  "command": [
    "/Users/jesght/.cargo/bin/cargo",
    "test",
    "--locked",
    "--offline",
    "-p",
    "sparq-core",
    "--features",
    "overlay-deleted-projections",
    "--lib",
    "store::overlay_deleted_tests::",
    "--",
    "--test-threads=1"
  ],
  "results": [
    {
      "name": "unconditional_deleted_invalidation",
      "exit": 101,
      "seconds": 8.753635375000002,
      "compiled": true,
      "command": [
        "/Users/jesght/.cargo/bin/cargo",
        "test",
        "--locked",
        "--offline",
        "-p",
        "sparq-core",
        "--features",
        "overlay-deleted-projections",
        "--lib",
        "store::overlay_deleted_tests::",
        "--",
        "--test-threads=1"
      ],
      "summaries": [
        "test result: FAILED. 8 passed; 1 failed; 0 ignored; 0 measured; 146 filtered out; finished in 0.09s"
      ]
    },
    {
      "name": "remove_deleted_invalidation",
      "exit": 101,
      "seconds": 6.043293457999999,
      "compiled": true,
      "command": [
        "/Users/jesght/.cargo/bin/cargo",
        "test",
        "--locked",
        "--offline",
        "-p",
        "sparq-core",
        "--features",
        "overlay-deleted-projections",
        "--lib",
        "store::overlay_deleted_tests::",
        "--",
        "--test-threads=1"
      ],
      "summaries": [
        "test result: FAILED. 4 passed; 5 failed; 0 ignored; 0 measured; 146 filtered out; finished in 0.08s"
      ]
    },
    {
      "name": "ignore_tombstone_insert_flag",
      "exit": 101,
      "seconds": 5.311953875,
      "compiled": true,
      "command": [
        "/Users/jesght/.cargo/bin/cargo",
        "test",
        "--locked",
        "--offline",
        "-p",
        "sparq-core",
        "--features",
        "overlay-deleted-projections",
        "--lib",
        "store::overlay_deleted_tests::",
        "--",
        "--test-threads=1"
      ],
      "summaries": [
        "test result: FAILED. 8 passed; 1 failed; 0 ignored; 0 measured; 146 filtered out; finished in 0.11s"
      ]
    },
    {
      "name": "ignore_tombstone_remove_flag",
      "exit": 101,
      "seconds": 4.926250875000001,
      "compiled": true,
      "command": [
        "/Users/jesght/.cargo/bin/cargo",
        "test",
        "--locked",
        "--offline",
        "-p",
        "sparq-core",
        "--features",
        "overlay-deleted-projections",
        "--lib",
        "store::overlay_deleted_tests::",
        "--",
        "--test-threads=1"
      ],
      "summaries": [
        "test result: FAILED. 8 passed; 1 failed; 0 ignored; 0 measured; 146 filtered out; finished in 0.11s"
      ]
    },
    {
      "name": "remove_deleted_cache_use",
      "exit": 101,
      "seconds": 6.247343416,
      "compiled": true,
      "command": [
        "/Users/jesght/.cargo/bin/cargo",
        "test",
        "--locked",
        "--offline",
        "-p",
        "sparq-core",
        "--features",
        "overlay-deleted-projections",
        "--lib",
        "store::overlay_deleted_tests::",
        "--",
        "--test-threads=1"
      ],
      "summaries": [
        "test result: FAILED. 5 passed; 4 failed; 0 ignored; 0 measured; 146 filtered out; finished in 0.11s"
      ]
    },
    {
      "name": "force_cache_in_feature_off",
      "exit": 101,
      "seconds": 10.563433084000003,
      "compiled": true,
      "command": [
        "/Users/jesght/.cargo/bin/cargo",
        "test",
        "--locked",
        "--offline",
        "-p",
        "sparq-core",
        "--lib",
        "deleted_projection_feature_off_preserves_main_layout_and_heap",
        "--",
        "--test-threads=1"
      ],
      "summaries": [
        "test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 149 filtered out; finished in 0.01s"
      ]
    }
  ]
}
```


## Mutation force_cache_in_feature_off diff

controls/force_cache_in_feature_off.diff; SHA256 a26fcf8c488f1cc6e1e71d72431213848261cb701964d1d10cb5c88193ee1cdd

```diff
--- store.rs
+++ force_cache_in_feature_off
@@ -179,7 +179,7 @@
     /// twelve-byte rows plus capacity slack, alongside that hash set. The sole
     /// production in-place mutator is `TripleStore::apply_delta`, which invalidates
     /// these projections when a tombstone is inserted or removed.
-    #[cfg(feature = "overlay-deleted-projections")]
+    
     deleted_by_perm: [std::sync::OnceLock<Vec<[Id; 3]>>; 6],
 }
 
@@ -218,7 +218,7 @@
     }
 
     /// [GPT-6 Astra] Drops deleted projections only when the tombstone set changed.
-    #[cfg(feature = "overlay-deleted-projections")]
+    
     fn invalidate_deleted(&mut self) {
         for slot in &mut self.deleted_by_perm {
             slot.take();
@@ -231,7 +231,7 @@
     /// apply_delta invalidates only the mutated overlay under exclusive access.
     /// Concurrent first readers of the same permutation wait for its one sorting
     /// initializer; the cold sort is serialized for that permutation.
-    #[cfg(feature = "overlay-deleted-projections")]
+    
     fn deleted_count(&self, perm: Perm, lo: [Id; 3], hi: [Id; 3]) -> usize {
         if self.deleted.is_empty() {
             return 0;
@@ -297,9 +297,9 @@
     /// default; the experimental feature opts into lazy sorted projections.
     fn count_correction(&self, perm: Perm, lo: [Id; 3], hi: [Id; 3]) -> (usize, usize) {
         let add = self.added_rows(perm, lo, hi).len();
-        #[cfg(feature = "overlay-deleted-projections")]
+        
         let del = self.deleted_count(perm, lo, hi);
-        #[cfg(not(feature = "overlay-deleted-projections"))]
+        #[cfg(any())]
         let del = {
             let order = perm.order();
             self.deleted
@@ -326,7 +326,7 @@
             .filter_map(|slot| slot.get())
             .map(|rows| rows.capacity() * std::mem::size_of::<[Id; 3]>())
             .sum();
-        #[cfg(feature = "overlay-deleted-projections")]
+        
         let cached = cached + self.deleted_by_perm.iter()
             .filter_map(|slot| slot.get())
             .map(|rows| rows.capacity() * std::mem::size_of::<[Id; 3]>())
@@ -952,21 +952,21 @@
         // Preserve deletion projections across inserts/no-ops: only actual tombstone
         // changes require another sort. No overlay read occurs before publication.
         ov.invalidate_added();
-        #[cfg(feature = "overlay-deleted-projections")]
+        
         let mut deleted_changed = false;
         for t in deletes {
             if let Ok(i) = ov.added.binary_search(t) {
                 ov.added.remove(i); // retract a pending insertion
             } else if self.base_contains(*t) {
-                #[cfg(feature = "overlay-deleted-projections")]
+                
                 { deleted_changed |= ov.deleted.insert(*t); }
-                #[cfg(not(feature = "overlay-deleted-projections"))]
+                #[cfg(any())]
                 { ov.deleted.insert(*t); }
             }
         }
         for t in inserts {
             if ov.deleted.remove(t) {
-                #[cfg(feature = "overlay-deleted-projections")]
+                
                 { deleted_changed = true; }
                 continue; // re-insert of a deleted base triple: just undelete
             }
@@ -977,7 +977,7 @@
                 ov.added.insert(i, *t);
             }
         }
-        #[cfg(feature = "overlay-deleted-projections")]
+        
         if deleted_changed {
             ov.invalidate_deleted();
         }
```


## Mutation force_cache_in_feature_off actual output

controls/force_cache_in_feature_off.log; SHA256 cbb992bbdb044f5ffa89df6d04c17efdf5059c5f95718168e73392e775d385fa

```text
   Compiling sparq-core v0.1.1 (/private/tmp/sparq-pr6049/.throughput-monitor/worktrees/issue4246/crates/sparq-core)
    Finished `test` profile [unoptimized] target(s) in 9.90s
     Running unittests src/lib.rs (/private/tmp/sparq-pr6049/.throughput-monitor/direct-5983/implementation/target/debug/deps/sparq_core-53b43ac9cb50e22c)

running 1 test
test store::overlay_deleted_tests::deleted_projection_feature_off_preserves_main_layout_and_heap ... FAILED

failures:

---- store::overlay_deleted_tests::deleted_projection_feature_off_preserves_main_layout_and_heap stdout ----

thread 'store::overlay_deleted_tests::deleted_projection_feature_off_preserves_main_layout_and_heap' (2456074) panicked at crates/sparq-core/src/store/overlay_deleted_tests.rs:367:13:
assertion `left == right` failed: default reads retain no deletion projection
  left: 3543
 right: 3495
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace


failures:
    store::overlay_deleted_tests::deleted_projection_feature_off_preserves_main_layout_and_heap

test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 149 filtered out; finished in 0.01s

error: test failed, to rerun pass `-p sparq-core --lib`
```


## Mutation ignore_tombstone_insert_flag diff

controls/ignore_tombstone_insert_flag.diff; SHA256 8b10eb674298ea74e8527742e9639a4b5d46d592b932b142388e97c1b4423fec

```diff
--- store.rs
+++ ignore_tombstone_insert_flag
@@ -959,7 +959,7 @@
                 ov.added.remove(i); // retract a pending insertion
             } else if self.base_contains(*t) {
                 #[cfg(feature = "overlay-deleted-projections")]
-                { deleted_changed |= ov.deleted.insert(*t); }
+                { ov.deleted.insert(*t); }
                 #[cfg(not(feature = "overlay-deleted-projections"))]
                 { ov.deleted.insert(*t); }
             }
```


## Mutation ignore_tombstone_insert_flag actual output

controls/ignore_tombstone_insert_flag.log; SHA256 0954cf74acd1179712c94476eac10b386f668499025bdab7705487f0fb77d43f

```text
   Compiling sparq-core v0.1.1 (/private/tmp/sparq-pr6049/.throughput-monitor/worktrees/issue4246/crates/sparq-core)
    Finished `test` profile [unoptimized] target(s) in 4.40s
     Running unittests src/lib.rs (/private/tmp/sparq-pr6049/.throughput-monitor/direct-5983/implementation/target/debug/deps/sparq_core-5d77b6001c8fb29f)

running 9 tests
test store::overlay_deleted_tests::deleted_cache_actual_tombstone_changes_invalidate_before_publication ... FAILED
test store::overlay_deleted_tests::deleted_cache_compressed_base_matches_rebuild ... ok
test store::overlay_deleted_tests::deleted_cache_concurrent_first_reads_share_initialized_projection ... ok
test store::overlay_deleted_tests::deleted_cache_fork_and_clone_are_independent ... ok
test store::overlay_deleted_tests::deleted_cache_graph_snapshot_retains_warm_generation ... ok
test store::overlay_deleted_tests::deleted_cache_inclusive_bounds_and_empty_ranges ... ok
test store::overlay_deleted_tests::deleted_cache_is_lazy_reused_and_accounted ... ok
test store::overlay_deleted_tests::deleted_cache_matches_rebuild_after_mixed_deltas ... ok
test store::overlay_deleted_tests::deleted_cache_survives_insert_only_and_noop_deltas ... ok

failures:

---- store::overlay_deleted_tests::deleted_cache_actual_tombstone_changes_invalidate_before_publication stdout ----

thread 'store::overlay_deleted_tests::deleted_cache_actual_tombstone_changes_invalidate_before_publication' (2454665) panicked at crates/sparq-core/src/store/overlay_deleted_tests.rs:166:9:
assertion failed: store.overlay.as_ref().unwrap().deleted_by_perm.iter().all(|s|
        s.get().is_none())
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace


failures:
    store::overlay_deleted_tests::deleted_cache_actual_tombstone_changes_invalidate_before_publication

test result: FAILED. 8 passed; 1 failed; 0 ignored; 0 measured; 146 filtered out; finished in 0.11s

error: test failed, to rerun pass `-p sparq-core --lib`
```


## Mutation ignore_tombstone_remove_flag diff

controls/ignore_tombstone_remove_flag.diff; SHA256 f0e8fdeac8ceaec8de85735545300c117cae190abbf082dc81924a2b8f5a53e5

```diff
--- store.rs
+++ ignore_tombstone_remove_flag
@@ -967,7 +967,7 @@
         for t in inserts {
             if ov.deleted.remove(t) {
                 #[cfg(feature = "overlay-deleted-projections")]
-                { deleted_changed = true; }
+                {}
                 continue; // re-insert of a deleted base triple: just undelete
             }
             if self.base_contains(*t) {
```


## Mutation ignore_tombstone_remove_flag actual output

controls/ignore_tombstone_remove_flag.log; SHA256 dcb69ce4f7830b7155224864648c7e7f859c0d5282b3b84aa0fd4324569716ec

```text
   Compiling sparq-core v0.1.1 (/private/tmp/sparq-pr6049/.throughput-monitor/worktrees/issue4246/crates/sparq-core)
    Finished `test` profile [unoptimized] target(s) in 4.17s
     Running unittests src/lib.rs (/private/tmp/sparq-pr6049/.throughput-monitor/direct-5983/implementation/target/debug/deps/sparq_core-5d77b6001c8fb29f)

running 9 tests
test store::overlay_deleted_tests::deleted_cache_actual_tombstone_changes_invalidate_before_publication ... FAILED
test store::overlay_deleted_tests::deleted_cache_compressed_base_matches_rebuild ... ok
test store::overlay_deleted_tests::deleted_cache_concurrent_first_reads_share_initialized_projection ... ok
test store::overlay_deleted_tests::deleted_cache_fork_and_clone_are_independent ... ok
test store::overlay_deleted_tests::deleted_cache_graph_snapshot_retains_warm_generation ... ok
test store::overlay_deleted_tests::deleted_cache_inclusive_bounds_and_empty_ranges ... ok
test store::overlay_deleted_tests::deleted_cache_is_lazy_reused_and_accounted ... ok
test store::overlay_deleted_tests::deleted_cache_matches_rebuild_after_mixed_deltas ... ok
test store::overlay_deleted_tests::deleted_cache_survives_insert_only_and_noop_deltas ... ok

failures:

---- store::overlay_deleted_tests::deleted_cache_actual_tombstone_changes_invalidate_before_publication stdout ----

thread 'store::overlay_deleted_tests::deleted_cache_actual_tombstone_changes_invalidate_before_publication' (2455100) panicked at crates/sparq-core/src/store/overlay_deleted_tests.rs:166:9:
assertion failed: store.overlay.as_ref().unwrap().deleted_by_perm.iter().all(|s|
        s.get().is_none())
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace


failures:
    store::overlay_deleted_tests::deleted_cache_actual_tombstone_changes_invalidate_before_publication

test result: FAILED. 8 passed; 1 failed; 0 ignored; 0 measured; 146 filtered out; finished in 0.11s

error: test failed, to rerun pass `-p sparq-core --lib`
```


## Mutation remove_deleted_cache_use diff

controls/remove_deleted_cache_use.diff; SHA256 7d48a13efd93033842f76704b6d75e3bbf86b78dd429c5ba21751b708304adb3

```diff
--- store.rs
+++ remove_deleted_cache_use
@@ -233,20 +233,11 @@
     /// initializer; the cold sort is serialized for that permutation.
     #[cfg(feature = "overlay-deleted-projections")]
     fn deleted_count(&self, perm: Perm, lo: [Id; 3], hi: [Id; 3]) -> usize {
-        if self.deleted.is_empty() {
-            return 0;
-        }
-        let rows = self.deleted_by_perm[perm as usize].get_or_init(|| {
-            let order = perm.order();
-            let mut rows: Vec<[Id; 3]> = self
-                .deleted
-                .iter()
-                .map(|t| [t[order[0]], t[order[1]], t[order[2]]])
-                .collect();
-            rows.sort_unstable();
-            rows
-        });
-        rows.partition_point(|r| *r <= hi) - rows.partition_point(|r| *r < lo)
+        let order = perm.order();
+        self.deleted.iter().filter(|t| {
+            let row = [t[order[0]], t[order[1]], t[order[2]]];
+            row >= lo && row <= hi
+        }).count()
     }
 
     /// The `added` triples matching the inclusive `[lo, hi]` key range, as rows in
```


## Mutation remove_deleted_cache_use actual output

controls/remove_deleted_cache_use.log; SHA256 fec8a43ffdbb685f85e70866f0a7688d730692afd04bd0f8490cb2fda5360a8e

```text
   Compiling sparq-core v0.1.1 (/private/tmp/sparq-pr6049/.throughput-monitor/worktrees/issue4246/crates/sparq-core)
    Finished `test` profile [unoptimized] target(s) in 5.55s
     Running unittests src/lib.rs (/private/tmp/sparq-pr6049/.throughput-monitor/direct-5983/implementation/target/debug/deps/sparq_core-5d77b6001c8fb29f)

running 9 tests
test store::overlay_deleted_tests::deleted_cache_actual_tombstone_changes_invalidate_before_publication ... ok
test store::overlay_deleted_tests::deleted_cache_compressed_base_matches_rebuild ... ok
test store::overlay_deleted_tests::deleted_cache_concurrent_first_reads_share_initialized_projection ... FAILED
test store::overlay_deleted_tests::deleted_cache_fork_and_clone_are_independent ... FAILED
test store::overlay_deleted_tests::deleted_cache_graph_snapshot_retains_warm_generation ... ok
test store::overlay_deleted_tests::deleted_cache_inclusive_bounds_and_empty_ranges ... ok
test store::overlay_deleted_tests::deleted_cache_is_lazy_reused_and_accounted ... FAILED
test store::overlay_deleted_tests::deleted_cache_matches_rebuild_after_mixed_deltas ... ok
test store::overlay_deleted_tests::deleted_cache_survives_insert_only_and_noop_deltas ... FAILED

failures:

---- store::overlay_deleted_tests::deleted_cache_concurrent_first_reads_share_initialized_projection stdout ----

thread '<unnamed>' (2455495) panicked at crates/sparq-core/src/store/overlay_deleted_tests.rs:299:34:
called `Option::unwrap()` on a `None` value
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace

thread '<unnamed>' (2455494) panicked at crates/sparq-core/src/store/overlay_deleted_tests.rs:299:34:
called `Option::unwrap()` on a `None` value

thread 'store::overlay_deleted_tests::deleted_cache_concurrent_first_reads_share_initialized_projection' (2455493) panicked at crates/sparq-core/src/store/overlay_deleted_tests.rs:306:64:
called `Result::unwrap()` on an `Err` value: Any { .. }

---- store::overlay_deleted_tests::deleted_cache_fork_and_clone_are_independent stdout ----

thread 'store::overlay_deleted_tests::deleted_cache_fork_and_clone_are_independent' (2455496) panicked at crates/sparq-core/src/store/overlay_deleted_tests.rs:233:14:
called `Option::unwrap()` on a `None` value

---- store::overlay_deleted_tests::deleted_cache_is_lazy_reused_and_accounted stdout ----

thread 'store::overlay_deleted_tests::deleted_cache_is_lazy_reused_and_accounted' (2455499) panicked at crates/sparq-core/src/store/overlay_deleted_tests.rs:61:14:
scan uses cached deletion count

---- store::overlay_deleted_tests::deleted_cache_survives_insert_only_and_noop_deltas stdout ----

thread 'store::overlay_deleted_tests::deleted_cache_survives_insert_only_and_noop_deltas' (2455501) panicked at crates/sparq-core/src/store/overlay_deleted_tests.rs:124:18:
called `Option::unwrap()` on a `None` value


failures:
    store::overlay_deleted_tests::deleted_cache_concurrent_first_reads_share_initialized_projection
    store::overlay_deleted_tests::deleted_cache_fork_and_clone_are_independent
    store::overlay_deleted_tests::deleted_cache_is_lazy_reused_and_accounted
    store::overlay_deleted_tests::deleted_cache_survives_insert_only_and_noop_deltas

test result: FAILED. 5 passed; 4 failed; 0 ignored; 0 measured; 146 filtered out; finished in 0.11s

error: test failed, to rerun pass `-p sparq-core --lib`
```


## Mutation remove_deleted_invalidation diff

controls/remove_deleted_invalidation.diff; SHA256 d82ce6806f57294aa4fa3b3b61365c4a89cf720fd9f4030b77454e62c48b4af0

```diff
--- store.rs
+++ remove_deleted_invalidation
@@ -979,7 +979,7 @@
         }
         #[cfg(feature = "overlay-deleted-projections")]
         if deleted_changed {
-            ov.invalidate_deleted();
+            let _ = deleted_changed;
         }
         self.overlay = if ov.is_empty() { None } else { Some(ov) };
     }
```


## Mutation remove_deleted_invalidation actual output

controls/remove_deleted_invalidation.log; SHA256 4cb253c5c8dde6daa91e80cb7d59e433ec0e4691cad10e9f9ce56ee87eceeeee

```text
   Compiling sparq-core v0.1.1 (/private/tmp/sparq-pr6049/.throughput-monitor/worktrees/issue4246/crates/sparq-core)
warning: method `invalidate_deleted` is never used
   --> crates/sparq-core/src/store.rs:222:8
    |
186 | impl Overlay {
    | ------------ method in this implementation
...
222 |     fn invalidate_deleted(&mut self) {
    |        ^^^^^^^^^^^^^^^^^^
    |
    = note: `#[warn(dead_code)]` (part of `#[warn(unused)]`) on by default

warning: `sparq-core` (lib test) generated 1 warning
    Finished `test` profile [unoptimized] target(s) in 5.20s
     Running unittests src/lib.rs (/private/tmp/sparq-pr6049/.throughput-monitor/direct-5983/implementation/target/debug/deps/sparq_core-5d77b6001c8fb29f)

running 9 tests
test store::overlay_deleted_tests::deleted_cache_actual_tombstone_changes_invalidate_before_publication ... FAILED
test store::overlay_deleted_tests::deleted_cache_compressed_base_matches_rebuild ... FAILED
test store::overlay_deleted_tests::deleted_cache_concurrent_first_reads_share_initialized_projection ... ok
test store::overlay_deleted_tests::deleted_cache_fork_and_clone_are_independent ... FAILED
test store::overlay_deleted_tests::deleted_cache_graph_snapshot_retains_warm_generation ... FAILED
test store::overlay_deleted_tests::deleted_cache_inclusive_bounds_and_empty_ranges ... ok
test store::overlay_deleted_tests::deleted_cache_is_lazy_reused_and_accounted ... ok
test store::overlay_deleted_tests::deleted_cache_matches_rebuild_after_mixed_deltas ... FAILED
test store::overlay_deleted_tests::deleted_cache_survives_insert_only_and_noop_deltas ... ok

failures:

---- store::overlay_deleted_tests::deleted_cache_actual_tombstone_changes_invalidate_before_publication stdout ----

thread 'store::overlay_deleted_tests::deleted_cache_actual_tombstone_changes_invalidate_before_publication' (2454271) panicked at crates/sparq-core/src/store/overlay_deleted_tests.rs:166:9:
assertion failed: store.overlay.as_ref().unwrap().deleted_by_perm.iter().all(|s|
        s.get().is_none())
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace

---- store::overlay_deleted_tests::deleted_cache_compressed_base_matches_rebuild stdout ----

thread 'store::overlay_deleted_tests::deleted_cache_compressed_base_matches_rebuild' (2454274) panicked at crates/sparq-core/src/store/overlay_deleted_tests.rs:25:17:
assertion `left == right` failed
  left: 3
 right: 4

---- store::overlay_deleted_tests::deleted_cache_fork_and_clone_are_independent stdout ----

thread 'store::overlay_deleted_tests::deleted_cache_fork_and_clone_are_independent' (2454279) panicked at crates/sparq-core/src/store/overlay_deleted_tests.rs:270:9:
assertion `left == right` failed
  left: 1
 right: 0

---- store::overlay_deleted_tests::deleted_cache_graph_snapshot_retains_warm_generation stdout ----

thread 'store::overlay_deleted_tests::deleted_cache_graph_snapshot_retains_warm_generation' (2454281) panicked at crates/sparq-core/src/store/overlay_deleted_tests.rs:337:9:
assertion `left == right` failed
  left: 0
 right: 1

---- store::overlay_deleted_tests::deleted_cache_matches_rebuild_after_mixed_deltas stdout ----

thread 'store::overlay_deleted_tests::deleted_cache_matches_rebuild_after_mixed_deltas' (2454284) panicked at crates/sparq-core/src/store/overlay_deleted_tests.rs:25:17:
assertion `left == right` failed
  left: 3
 right: 4


failures:
    store::overlay_deleted_tests::deleted_cache_actual_tombstone_changes_invalidate_before_publication
    store::overlay_deleted_tests::deleted_cache_compressed_base_matches_rebuild
    store::overlay_deleted_tests::deleted_cache_fork_and_clone_are_independent
    store::overlay_deleted_tests::deleted_cache_graph_snapshot_retains_warm_generation
    store::overlay_deleted_tests::deleted_cache_matches_rebuild_after_mixed_deltas

test result: FAILED. 4 passed; 5 failed; 0 ignored; 0 measured; 146 filtered out; finished in 0.08s

error: test failed, to rerun pass `-p sparq-core --lib`
```


## Mutation unconditional_deleted_invalidation diff

controls/unconditional_deleted_invalidation.diff; SHA256 c1461c3c761c9f82c1c0ff7a5e774cebd06904d106bf6371184b6c89e24e0b8c

```diff
--- store.rs
+++ unconditional_deleted_invalidation
@@ -978,7 +978,7 @@
             }
         }
         #[cfg(feature = "overlay-deleted-projections")]
-        if deleted_changed {
+        if deleted_changed || !ov.deleted.is_empty() {
             ov.invalidate_deleted();
         }
         self.overlay = if ov.is_empty() { None } else { Some(ov) };
```


## Mutation unconditional_deleted_invalidation actual output

controls/unconditional_deleted_invalidation.log; SHA256 38a4b4fe9a84c24472cc49a1d6ca5e5ecce8f3589b4651db9f30fc6417fbbd36

```text
   Compiling sparq-core v0.1.1 (/private/tmp/sparq-pr6049/.throughput-monitor/worktrees/issue4246/crates/sparq-core)
    Finished `test` profile [unoptimized] target(s) in 7.90s
     Running unittests src/lib.rs (/private/tmp/sparq-pr6049/.throughput-monitor/direct-5983/implementation/target/debug/deps/sparq_core-5d77b6001c8fb29f)

running 9 tests
test store::overlay_deleted_tests::deleted_cache_actual_tombstone_changes_invalidate_before_publication ... ok
test store::overlay_deleted_tests::deleted_cache_compressed_base_matches_rebuild ... ok
test store::overlay_deleted_tests::deleted_cache_concurrent_first_reads_share_initialized_projection ... ok
test store::overlay_deleted_tests::deleted_cache_fork_and_clone_are_independent ... ok
test store::overlay_deleted_tests::deleted_cache_graph_snapshot_retains_warm_generation ... ok
test store::overlay_deleted_tests::deleted_cache_inclusive_bounds_and_empty_ranges ... ok
test store::overlay_deleted_tests::deleted_cache_is_lazy_reused_and_accounted ... ok
test store::overlay_deleted_tests::deleted_cache_matches_rebuild_after_mixed_deltas ... ok
test store::overlay_deleted_tests::deleted_cache_survives_insert_only_and_noop_deltas ... FAILED

failures:

---- store::overlay_deleted_tests::deleted_cache_survives_insert_only_and_noop_deltas stdout ----

thread 'store::overlay_deleted_tests::deleted_cache_survives_insert_only_and_noop_deltas' (2453763) panicked at crates/sparq-core/src/store/overlay_deleted_tests.rs:139:22:
unchanged tombstones retain their projection
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace


failures:
    store::overlay_deleted_tests::deleted_cache_survives_insert_only_and_noop_deltas

test result: FAILED. 8 passed; 1 failed; 0 ignored; 0 measured; 146 filtered out; finished in 0.09s

error: test failed, to rerun pass `-p sparq-core --lib`
```


## Scope and limits for independent assessment

Evaluate whether the cfg boundary actually restores default behavior and removes deletion-cache representation/bookkeeping, while keeping reviewed feature-on correctness. The main comparison has an identical test-only module and standalone harness; stripping that cfg(test) suffix leaves byte-exact a42 store runtime, with main Cargo manifests and engine source unchanged. No broad forwarding/default enablement exists. The new CI leg explicitly tests the opt-in; it does not change Cargo defaults.

B1/B3 were resolved in the prior review. Their source/complete lifecycle context remains frozen in ../revision; this stage does not change their algorithms or Clone/Arc policy. It adds neither a cardinality threshold nor a production runtime API/flag.

The one permitted measurement varies only R, keeps all five values and all repetitions, and retains four children plus the initial graph. Four-window sums exclude setup priming and between-window accounting; per-generation measurements include clone, one true deletion, R whole queries, and local retained ownership. It is not server publication, storage I/O, or a deployment workload. Low-R regressions and memory/copy-discard costs remain. Larger-R fixture medians do not define a recommended crossover, production read/write ratio, tail bound or canonical speedup.

Allocator requested bytes exclude allocator metadata/stack/mapped memory. Store heap estimates omit fixed boxed metadata and estimate hash-set capacity. RSS is cumulative setup-inclusive process high-water, not query heap. Off-state semantics/representation are executed against main; no separate feature-off timing matrix was added.

Candidate Rust tests:86 passes, four ignored existing timing-test instances. Exact-main identical fixture:4 passes. Matrix assembly:105 tests. Six compiled controls killed, including forced caching in an off build failing actual retained heap. Scoped off/on core and feature-on counting harness clippy pass. Preflight retains only the known Bash3 mapfile failure. An initial assembler interpreter lacked PyYAML; existing Homebrew Python regenerated the exact final golden, with no installation.

No fullworkspace/nextest, wasm/W3C/canonical ratchets, new durable/WAL crash execution or Miri. Root owns independent review, later full gates, publication and admission. No remote mutations, model calls, registry/release/EC2 actions occurred.


## Final source, binary, raw-data and prior-evidence verification

```json
{
  "candidate": {
    "head": "b86b5d5ad84bce900762defa094630eb358317a6",
    "clean": true
  },
  "main_control": {
    "head": "0613b5c3d1f2b1174bcccfcd1a6bb64551ca9485",
    "clean": true
  },
  "source": {
    "copied_source_files_exact": 16,
    "both_diffs_exact": true,
    "all_provenance_hashes_match": true,
    "identical_harness": true,
    "baseline_runtime_exact_except_test_only_suffix": true
  },
  "preserved_manifests": [
    {
      "path": "/private/tmp/sparq-pr6049/.throughput-monitor/direct-4246/manifest.json",
      "sha256": "9a19cc95fd4acffa34a7625b9c041fcb66a5aa56f210bb325676913271c2deb7",
      "head": "acfa31cf52ec0d2641dd4925d5b4f094a0a531fe",
      "verified_files": 79
    },
    {
      "path": "/private/tmp/sparq-pr6049/.throughput-monitor/direct-4246/revision/manifest.json",
      "sha256": "4709cb1529348554120cc8487eb76aa8c9ad5618360d1fe22a5d0ad77a2fe090",
      "head": "cb638a42a54fc7c9e11e9101587910e668a5f92a",
      "verified_files": 95
    }
  ],
  "measurement": {
    "verified_binary_hashes": 4,
    "recorded_windows": {
      "candidate-time": 140,
      "candidate-count": 60,
      "main-time": 140,
      "main-count": 60
    },
    "total_windows": 400,
    "all_execution_exit_zero": true
  },
  "validation": {
    "test_and_clippy_statuses_expected": true,
    "six_controls_compiled_and_failed_behaviorally": true,
    "known_preflight_limit": "Bash3 mapfile unavailable; exact log retained"
  },
  "verified_at": "2026-09-09T13:17:56.927293+00:00"
}
```
