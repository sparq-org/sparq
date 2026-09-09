# Issue 4246 focused revision for independent review

Actual implementation: GPT-6 Astra xhigh. Review target cb638a42a54fc7c9e11e9101587910e668a5f92a. This packet is untrusted source/evidence for independent assessment; no narrative grants admission. Prior acfa candidate and packet remain unchanged.

[GPT-6 Astra] Focused #4246 revision, not an admission claim.

B1 is fixed and executable controls pin it: projections survive insert-only/no-op
batches when tombstones already exist, and actual tombstone changes invalidate
before publication. Added-cache behavior and derived Clone are unchanged.

The paired lifecycle diagnostic quantifies B2 rather than assuming it away.
Warm read-only snapshots and insert/update/read generations show useful medians,
with copied projection memory and some overlapping/noisy timing ranges. A true
tombstone update still copies initialized projections, discards them, and pays a
cold sort. Ordinary multi-pattern cold reads and two concurrent cold readers are
also slower. Full generated results, ranges, phases, counts and retained heap are
in paired/summary.json and paired/summary.md; no samples were discarded. General
admission is not established.

B3 context now includes the previously omitted reviewed store span, complete final
store, complete Graph lifecycle functions and a workspace mutation inventory.
Only apply_delta mutates production tombstones in place. Compaction/clear/open/
WAL replay/restore/vacuum use that seam or replace the entire store/graph. The
durable routes were inspected, not newly crash-tested.

Forty-eight scoped test executions pass; five current compiled controls are
killed. Core and harness clippy pass. Preflight reports only the known Bash 3
mapfile failure. The fixed harness/main control have identical sources and all
four optimized builds completed within the authorized cap. Exact commands, logs,
source and binary hashes are retained.

The next step is independent review of this repair and the lifecycle evidence.
No Clone/Arc redesign, cardinality threshold, remote update, or admission action
is included. Further design requires a separate scoped decision.


## Actual prior Opus verdict (result field only)

Path: prior-review-result.json; SHA256 54d1bcb411ee066b916cca0c195185a6b9368cb50554543e216d7df798fe321c

```json
"Reviewed head: **acfa31cf52ec0d2641dd4925d5b4f094a0a531fe** (base a42a9e89dec485f6a319c47cb3635c59cb5a2270) \u2014 verdict: **request_changes**\n\n## What I verified\n\nBounds arithmetic in `Overlay::deleted_count` (store.rs:233) is equivalent to the replaced predicate: `partition_point(|r| *r <= hi) - partition_point(|r| *r < lo)` counts exactly the lexicographic `[lo,hi]` band the old `r >= lo && r <= hi` filter counted, on the same per-perm projection. Column permutation is a bijection, so the projected rows are duplicate-free and `sort_unstable` is total \u2014 no stability assumption is needed. `bounds()` (store.rs:1075) fills MIN/MAX outside the bound prefix, matching the sentinel convention of `lower_bound`/`upper_bound`. Slots are only ever reached via `count_correction` \u2190 `scan_with`/`estimate`, whose perms come from `choose`/`choose_sorted`/`scan_perm`, all restricted to `BUILT`, so the array is bounded by the built set (3 under compact-index). `OnceLock<Vec<..>>` keeps `Overlay: Send+Sync`; the init closure is non-reentrant, so `get_or_init` cannot deadlock. `heap_bytes` chains the new array and charges the SPO deleted slot (correctly, since it cannot alias `added`). Tests are strong: the `sweep` differential against a rebuilt store covers every built perm \u00d7 prefix length \u00d7 present/absent key, and the four mutants map to identifiable assertions (cache-use \u2192 pointer/`get()` asserts; invalidation \u2192 `sweep` after mixed batches; upper bound \u2192 the `lo==hi==row \u21d2 1` case; heap \u2192 the `expected_heap` equality). The summary table is internally coherent (main cold query \u2248 2\u00d7 main cold scan = two `count_correction` calls; candidate cold query \u2248 one sort + \u03b5; warm candidate grows logarithmically).\n\n## Blockers\n\n**B1 \u2014 Insert-only deltas evict the deletion projections; no control covers this, and the cheap remedy is absent.**\n`TripleStore::apply_delta` (store.rs:922) calls `invalidate_projections` unconditionally, so a batch that touches only `added` still drops all six `deleted_by_perm` slots. Trigger: build a store with D deletions pending, then loop `{apply_delta(&[one_insert], &[]); scan(untouched_range)}`. Candidate pays O(D log D) re-sort per iteration (\u22481.1 ms at D=32768 per the cold row) where main paid O(D) (\u224896 \u00b5s) \u2014 a persistent ~11\u00d7 regression with no amortization, in an ordinary append-heavy-before-compaction pattern. The harness cannot see it: the \"insert-only control\" (`main.rs`, `(8192, true)`) forks a base with **zero** deletions, so it measures the no-deleted-projection path only. Minimal remedy: in the delete loop, set `let mut deleted_changed = false;` on any `ov.deleted.insert(*t)` / `ov.deleted.remove(t)` returning true, split `invalidate_projections` into added/deleted halves, and clear `deleted_by_perm` only when `deleted_changed` (invalidate `added_by_perm` unconditionally as today). Alternatively, add the \"insert into an overlay holding D deletions, then read\" point to the matrix and disclose the regression explicitly.\n\n**B2 \u2014 Fork/snapshot of a warm overlay deep-copies the projections; unmeasured, and wasted in the documented generation pattern.**\n`TripleStore::fork` (store.rs:972) clones `Option<Box<Overlay>>`; the derived `Clone` copies every initialized `OnceLock`, so a warm generation copies up to 6 extra vectors of 12\u00b7D bytes (2.36 MB at D=32768, on top of the pre-existing `added` copies) \u2014 and in the `fork \u2192 apply_delta \u2192 publish` pattern documented at lib.rs:2885/2905 those bytes are immediately dropped by store.rs:922. The benchmark forks `graph` (no overlay) and applies the delta afterwards (`main.rs::fork`), so *no* sample measures fork latency or heap for a warmed overlay; the \"candidate extra vector heap\" column is per-sample, not per-generation. This is an unmeasured material lifecycle cost, not obviously a defect (copying is a win for read-only `snapshot()`), but it must be measured before admission: fork(warm)+apply_delta wall time and `heap_bytes` across N generations. If instead you choose to reset projections in a hand-written `Clone`, note `deleted_cache_fork_and_clone_are_independent` asserts the opposite and must change.\n\n**B3 \u2014 Missing source: I cannot confirm `apply_delta` is the only `&mut Overlay` path.**\nstore.rs:327\u2013851 is not in the packet. Before this change a site that mutated only `ov.deleted` needed no invalidation (added projections stayed valid); now such a site silently serves stale counts, and `estimate`/the zero-copy fast path would return wrong cardinalities and wrong rows. Please supply store.rs:327\u2013851 and the result of grepping `ov.deleted`, `overlay.as_mut`, `self.overlay =` across the workspace (WAL replay, `vacuum`/`persist_swap`, compaction shortcuts). If `apply_delta` is genuinely the sole mutator, say so in the `deleted_by_perm` doc comment as an invariant.\n\n## Nits\n\n- **N1** (store.rs:176) The field doc should state the worst case in code, not only in the packet: up to `|BUILT|` vectors of `12\u00b7|deleted|` bytes, retained *alongside* the hash set (~72 B/deleted triple at 6 perms), and that SPO cannot alias.\n- **N2** Concurrency: `get_or_init` now blocks all first-readers of a perm for the whole sort (~1.1 ms at D=32768) where main had N independent O(D) counts. The two-thread test proves shared initialization only; the measurement lane is single-threaded, so cold-after-write tail latency under concurrency is unknown. Worth one sentence in the doc and, ideally, a two-reader cold timing point.\n- **N3** Cold cost multiplies per distinct permutation: `prepare_bgp` (exec.rs:7779) calls `estimate` per pattern, so a 3-pattern BGP choosing SPO/POS/OSP pays three sorts on the first read after each write (~3.4 ms at D=32768) versus ~0.6 ms for main. Only the single-pattern case is measured.\n- **N4** Optional and supported by your own D=1024 row (2.96 \u00b5s \u2192 20.4 \u00b5s cold): a `deleted.len()` threshold below which `deleted_count` falls back to the linear filter would erase the small/medium cold regression at ~3 lines. Not required if B1/B2 land and the cold cost is accepted knowingly.\n- **N5** Do not read the D=0/16 cold ratios (0.84\u20130.99) as wins; the seven-rep ranges overlap by 2\u20134\u00d7.\n\n## Evidence status\n\n`baseline/`, `paired/summary.json`, `layout.json`, `tests/`, `controls/` and the mutant diffs are not in this packet, so every number above is checked only for internal consistency, not reproduced. Local single-host, single-thread, generated-fixture timings are not canonical. Remaining admission gates as stated by the author and not discharged here: full workspace/nextest shards, W3C conformance, performance ratchets, mapped-I/O and WAL-replay suites, Miri, and the `scripts/check-privacy-claims.sh:92` Bash-3 preflight failure. I have taken no remote action and claim no merge authority."
```


## Exact revision delta from reviewed acfa head

Path: source/revision.diff; SHA256 70ecfc3fd7ac43df412311afe9cf24ed02e374396f6630fe07443cf7764e869a

```diff
diff --git a/bench/overlay-count/README.md b/bench/overlay-count/README.md
index b25065c41..f1e41d12b 100644
--- a/bench/overlay-count/README.md
+++ b/bench/overlay-count/README.md
@@ -35,3 +35,27 @@ Counting calibration runs before setup. Compare only byte-identical harnesses,
 identical dimensions/features, and record exact source and binary hashes with raw
 JSON. Preserve all samples; do not seek a quiet subset or publish a speedup claim
 from these local diagnostics.
+
+The `lifecycle` argument selects the fixed review follow-up: a delete-only overlay
+with 32,768 tombstones, four retained generations, and one or all six projections
+warmed before read-only snapshots, fork/insert/read, or fork/delete/read. A
+six-projection in-place insert/read control separates deep cloning from invalidation.
+Three additional points execute a three-pattern query cold and warm, and two
+concurrent cold readers synchronized immediately before their ordinary queries.
+No dimension is selected from observed timings. This protocol retains the same
+two warmup samples and seven timing / three allocation repetitions.
+
+Lifecycle windows include clone (where applicable), one delta, whole query, and
+retaining the resulting graph or snapshot in a preallocated local vector. This is
+local ownership publication, not server or durable-store publication. Each of four
+generation windows is measured separately while earlier generations remain alive.
+`phase_ns` records clone/delta/read; for concurrent readers its first two values
+are the individual reader durations and the whole window includes thread launch
+and join. Those few samples do not establish tail percentiles.
+
+Retained store heap subtracts the shared immutable base once per reference, then
+sums the overlay heap of the initial graph and retained generations. Existing
+`heap_bytes` omits fixed boxed-overlay metadata and estimates hash-table capacity;
+allocator counters separately report live requested bytes. Cold-query cases do not
+prime measured forks. A separate oracle records the multi-pattern query's actual
+projection heap growth and verifies the complete four-row result.
diff --git a/bench/overlay-count/src/lifecycle.rs b/bench/overlay-count/src/lifecycle.rs
new file mode 100644
index 000000000..7207ea933
--- /dev/null
+++ b/bench/overlay-count/src/lifecycle.rs
@@ -0,0 +1,235 @@
+//! [GPT-6 Astra] Fixed lifecycle/admission cases requested by the #4246 review.
+
+use super::*;
+use sparq_core::{store::Perm, GraphSnapshot};
+use std::sync::Barrier;
+
+const D: usize = 32_768;
+const GENERATIONS: usize = 4;
+const MULTI: &str = "SELECT ?o ?s ?p WHERE { <urn:s:49999> <urn:p:0> ?o . ?s <urn:p:1> <urn:o:49999> . ?s ?p <urn:o:49999> }";
+
+#[derive(Clone, Copy, Default)]
+struct Sample {
+    elapsed: u128,
+    phases: [u128; 3],
+    counts: (u64, u64, u64, u64, u64),
+    live_before: u64,
+    rss_before: u64,
+    rss_after: u64,
+    retained_heap: usize,
+    initial_heap: usize,
+}
+
+fn measure(f: impl FnOnce() -> [u128; 3]) -> Sample {
+    let rss_before = rss();
+    #[cfg(feature = "count-alloc")]
+    let live_before = counting::begin();
+    #[cfg(not(feature = "count-alloc"))]
+    let live_before = 0;
+    let start = Instant::now();
+    let phases = f();
+    let elapsed = start.elapsed().as_nanos();
+    #[cfg(feature = "count-alloc")]
+    let counts = counting::end(live_before);
+    #[cfg(not(feature = "count-alloc"))]
+    let counts = (0, 0, 0, 0, 0);
+    Sample {
+        elapsed,
+        phases,
+        counts,
+        live_before,
+        rss_before,
+        rss_after: rss(),
+        ..Sample::default()
+    }
+}
+
+fn emit(case: &str, warm_perms: usize, rep: usize, generation: usize, row: Sample) {
+    println!("{{\"kind\":\"lifecycle\",\"case\":\"{case}\",\"warm_perms\":{warm_perms},\"rep\":{rep},\"generation\":{generation},\"elapsed_ns\":{},\"phase_ns\":{:?},\"allocs\":{},\"reallocs\":{},\"requested_bytes\":{},\"peak_live_delta_bytes\":{},\"live_before\":{},\"live_after\":{},\"rss_before\":{},\"rss_after\":{},\"retained_overlay_reported_heap\":{},\"initial_overlay_reported_heap\":{}}}", row.elapsed,row.phases,row.counts.0,row.counts.1,row.counts.2,row.counts.3,row.live_before,row.counts.4,row.rss_before,row.rss_after,row.retained_heap,row.initial_heap);
+}
+
+fn prime(graph: &Graph, permutations: usize) {
+    let pattern = [
+        Some(graph.dict.lookup(&iri("urn:s:49999"))),
+        Some(graph.dict.lookup(&iri("urn:p:0"))),
+        Some(graph.dict.lookup(&iri("urn:o:49999"))),
+    ];
+    for &perm in &Perm::ALL[..permutations] {
+        assert_eq!(graph.store.scan_perm(&pattern, perm).unwrap().rows.len(), 1);
+    }
+}
+
+fn multi(graph: &Graph) -> usize {
+    let result = query(black_box(graph), black_box(MULTI)).unwrap();
+    black_box(result.rows.len())
+}
+
+fn check_multi(graph: &Graph) {
+    let result = query(graph, MULTI).unwrap();
+    assert_eq!(result.rows.len(), 4);
+    let mut actual: Vec<_> = result
+        .rows
+        .iter()
+        .map(|r| {
+            assert_eq!(r[0], Some(iri("urn:o:49999")));
+            assert_eq!(r[1], Some(iri("urn:s:49999")));
+            r[2].as_ref().unwrap().to_string()
+        })
+        .collect();
+    actual.sort_unstable();
+    let expected: Vec<_> = (0..4)
+        .map(|p| iri(&format!("urn:p:{p}")).to_string())
+        .collect();
+    assert_eq!(actual, expected);
+}
+
+pub(super) fn run_all() {
+    let pristine = base();
+    let base_heap = pristine.store.heap_bytes();
+    let deletions = delta(D, false);
+    let inserts: Vec<_> = (0..GENERATIONS)
+        .map(|i| {
+            [
+                iri(&format!("urn:new:{i}")),
+                iri("urn:p:0"),
+                iri("urn:new-object"),
+            ]
+        })
+        .collect();
+    let tombstones: Vec<_> = (0..GENERATIONS)
+        .map(|i| {
+            [
+                iri(&format!("urn:s:{}", 20_000 + i)),
+                iri("urn:p:0"),
+                iri(&format!("urn:o:{}", 20_000 + i)),
+            ]
+        })
+        .collect();
+    let oracle = fork(&pristine, &deletions, false);
+    let before = oracle.store.heap_bytes();
+    check_multi(&oracle);
+    // This records actual multi-query projection engagement outside measured forks.
+    println!("{{\"kind\":\"lifecycle_fixture\",\"subjects\":{SUBJECTS},\"predicates\":{PREDICATES},\"deletions\":{D},\"generations\":{GENERATIONS},\"multi_query_heap_delta\":{},\"counting\":{},\"setup_process_peak_rss_bytes\":{}}}", oracle.store.heap_bytes()-before,cfg!(feature="count-alloc"),rss());
+    drop(oracle);
+    let cases = [
+        ("snapshot", 1),
+        ("snapshot", 6),
+        ("fork-insert", 1),
+        ("fork-insert", 6),
+        ("fork-tombstone", 1),
+        ("fork-tombstone", 6),
+        ("inplace-insert", 6),
+        ("multi-cold", 0),
+        ("multi-warm", 3),
+        ("concurrent-cold", 0),
+    ];
+    for (case, warm_perms) in cases {
+        let reps = if cfg!(feature = "count-alloc") { 3 } else { 7 };
+        for rep in 0..reps + 2 {
+            let mut initial = fork(&pristine, &deletions, false);
+            if case == "multi-warm" {
+                check_multi(&initial); // SPO, POS, OSP via three actual BGP estimates
+            } else if warm_perms > 0 {
+                prime(&initial, warm_perms);
+            }
+            let initial_heap = initial.store.heap_bytes() - base_heap;
+            let mut records = [Sample::default(); GENERATIONS];
+            let count = if matches!(case, "multi-cold" | "multi-warm" | "concurrent-cold") {
+                1
+            } else {
+                GENERATIONS
+            };
+            let mut generations: Vec<Graph> = Vec::with_capacity(GENERATIONS);
+            let mut snapshots: Vec<GraphSnapshot> = Vec::with_capacity(GENERATIONS);
+            for generation in 0..count {
+                let mut sample = measure(|| {
+                    if case.starts_with("multi-") {
+                        assert_eq!(multi(&initial), 4);
+                        return [0; 3];
+                    }
+                    if case == "concurrent-cold" {
+                        let barrier = Barrier::new(2);
+                        // Includes thread launch/join overhead; reader durations are
+                        // individual observations, not tail-percentile estimates.
+                        return std::thread::scope(|scope| {
+                            let read = || {
+                                barrier.wait();
+                                let start = Instant::now();
+                                assert_eq!(run(&initial, "query", 1), 1);
+                                start.elapsed().as_nanos()
+                            };
+                            let a = scope.spawn(read);
+                            let b = scope.spawn(read);
+                            [a.join().unwrap(), b.join().unwrap(), 0]
+                        });
+                    }
+                    let start = Instant::now();
+                    if case == "snapshot" {
+                        let snapshot = initial.snapshot();
+                        let cloned = start.elapsed().as_nanos();
+                        let read = Instant::now();
+                        assert_eq!(run(&snapshot, "query", 1), 1);
+                        let read = read.elapsed().as_nanos();
+                        snapshots.push(snapshot); // retain publication target
+                        return [cloned, 0, read];
+                    }
+                    if case == "inplace-insert" {
+                        initial
+                            .apply_delta(std::slice::from_ref(&inserts[generation]), &[])
+                            .unwrap();
+                        let delta = start.elapsed().as_nanos();
+                        let read = Instant::now();
+                        assert_eq!(run(&initial, "query", 1), 1);
+                        return [0, delta, read.elapsed().as_nanos()];
+                    }
+                    let parent = generations.last().unwrap_or(&initial);
+                    let mut next = parent.fork();
+                    let cloned = start.elapsed().as_nanos();
+                    let delta = Instant::now();
+                    if case == "fork-insert" {
+                        next.apply_delta(std::slice::from_ref(&inserts[generation]), &[])
+                            .unwrap();
+                    } else {
+                        next.apply_delta(&[], std::slice::from_ref(&tombstones[generation]))
+                            .unwrap();
+                    }
+                    let delta = delta.elapsed().as_nanos();
+                    let read = Instant::now();
+                    assert_eq!(run(&next, "query", 1), 1);
+                    let read = read.elapsed().as_nanos();
+                    generations.push(next); // local ownership publication, no service I/O
+                    [cloned, delta, read]
+                });
+                sample.initial_heap = initial_heap;
+                sample.retained_heap = initial.store.heap_bytes() - base_heap
+                    + generations
+                        .iter()
+                        .map(|g| g.store.heap_bytes() - base_heap)
+                        .sum::<usize>()
+                    + snapshots
+                        .iter()
+                        .map(|g| g.store.heap_bytes() - base_heap)
+                        .sum::<usize>();
+                records[generation] = sample;
+            }
+            // Verify the retained generation contents outside every measured window.
+            for (i, graph) in generations.iter().enumerate() {
+                check_query(graph);
+                assert_eq!(
+                    graph.store.len(),
+                    SUBJECTS * PREDICATES - D + if case == "fork-insert" { i + 1 } else { 0 }
+                        - if case == "fork-tombstone" { i + 1 } else { 0 }
+                );
+            }
+            for graph in &snapshots {
+                check_query(graph);
+                assert_eq!(graph.store.len(), SUBJECTS * PREDICATES - D);
+            }
+            if rep >= 2 {
+                for (generation, &record) in records[..count].iter().enumerate() {
+                    emit(case, warm_perms, rep - 2, generation + 1, record);
+                }
+            }
+        }
+    }
+}
diff --git a/bench/overlay-count/src/main.rs b/bench/overlay-count/src/main.rs
index cf0ad4a66..063a2cf09 100644
--- a/bench/overlay-count/src/main.rs
+++ b/bench/overlay-count/src/main.rs
@@ -2,6 +2,7 @@
 
 #[cfg(feature = "count-alloc")]
 mod counting;
+mod lifecycle;
 
 use oxrdf::{NamedNode, Term};
 use sparq_core::Graph;
@@ -103,6 +104,10 @@ fn main() {
         .unwrap();
     #[cfg(feature = "count-alloc")]
     counting::calibrate();
+    if std::env::args().nth(1).as_deref() == Some("lifecycle") {
+        lifecycle::run_all();
+        return;
+    }
     let graph = base();
     println!("{{\"kind\":\"fixture\",\"subjects\":{SUBJECTS},\"predicates\":{PREDICATES},\"base_triples\":{},\"setup_process_peak_rss_bytes\":{},\"rayon_threads\":1,\"counting\":{}}}", graph.store.len(), rss(), cfg!(feature = "count-alloc"));
     for (n, added) in DELETIONS
diff --git a/crates/sparq-core/src/store.rs b/crates/sparq-core/src/store.rs
index fdcf160e6..d90091aa6 100644
--- a/crates/sparq-core/src/store.rs
+++ b/crates/sparq-core/src/store.rs
@@ -175,6 +175,10 @@ struct Overlay {
     added_by_perm: [std::sync::OnceLock<Vec<[Id; 3]>>; 6],
     /// [GPT-6 Astra] Lazy deletion projections for range counts (#4246). Keep the
     /// hash set above for merge membership; even SPO needs its own sorted projection.
+    /// Full use retains up to `BUILT.len()` vectors, each with `deleted.len()`
+    /// twelve-byte rows plus capacity slack, alongside that hash set. The sole
+    /// production in-place mutator is `TripleStore::apply_delta`, which invalidates
+    /// these projections when a tombstone is inserted or removed.
     deleted_by_perm: [std::sync::OnceLock<Vec<[Id; 3]>>; 6],
 }
 
@@ -184,7 +188,7 @@ impl Overlay {
     ///
     /// Built LAZILY on the first scan that needs this permutation rather than eagerly
     /// for all six in [`TripleStore::apply_delta`]: a write batch then stays O(batch)
-    /// (it only drops the caches, see [`Overlay::invalidate_projections`]) instead of paying
+    /// (it only drops the caches, see [`Overlay::invalidate_added`]) instead of paying
     /// O(6·k log k) per call, and a store only ever materialises the projections its
     /// query mix actually scans — so the memory cost is bounded by the permutations in
     /// use, not a flat 6×. SPO needs no projection or sort at all: `added` is already
@@ -205,9 +209,16 @@ impl Overlay {
         })
     }
 
-    /// [GPT-6 Astra] Drops both sets of projections before any delta mutation.
-    fn invalidate_projections(&mut self) {
-        for slot in self.added_by_perm.iter_mut().chain(&mut self.deleted_by_perm) {
+    /// Drops added projections before any nonempty delta, preserving prior behavior.
+    fn invalidate_added(&mut self) {
+        for slot in &mut self.added_by_perm {
+            slot.take();
+        }
+    }
+
+    /// [GPT-6 Astra] Drops deleted projections only when the tombstone set changed.
+    fn invalidate_deleted(&mut self) {
+        for slot in &mut self.deleted_by_perm {
             slot.take();
         }
     }
@@ -216,6 +227,8 @@ impl Overlay {
     /// The first request costs O(d log d) and one additional vector; later requests
     /// use two binary searches. Clone copies initialized vectors by value, and
     /// apply_delta invalidates only the mutated overlay under exclusive access.
+    /// Concurrent first readers of the same permutation wait for its one sorting
+    /// initializer; the cold sort is serialized for that permutation.
     fn deleted_count(&self, perm: Perm, lo: [Id; 3], hi: [Id; 3]) -> usize {
         if self.deleted.is_empty() {
             return 0;
@@ -915,20 +928,21 @@ impl TripleStore {
             return;
         }
         let mut ov = self.overlay.take().unwrap_or_default();
-        // [GPT-6 Astra] Either delta set can change, so all cached projections are
-        // stale from here on. Dropping them up front (O(1) per permutation) keeps the
-        // write path O(batch) — the projections are rebuilt lazily by the next scan
-        // that needs them, and only for the permutations it actually scans.
-        ov.invalidate_projections();
+        // [GPT-6 Astra] Added projections retain the existing conservative reset.
+        // Preserve deletion projections across inserts/no-ops: only actual tombstone
+        // changes require another sort. No overlay read occurs before publication.
+        ov.invalidate_added();
+        let mut deleted_changed = false;
         for t in deletes {
             if let Ok(i) = ov.added.binary_search(t) {
                 ov.added.remove(i); // retract a pending insertion
             } else if self.base_contains(*t) {
-                ov.deleted.insert(*t);
+                deleted_changed |= ov.deleted.insert(*t);
             }
         }
         for t in inserts {
             if ov.deleted.remove(t) {
+                deleted_changed = true;
                 continue; // re-insert of a deleted base triple: just undelete
             }
             if self.base_contains(*t) {
@@ -938,6 +952,9 @@ impl TripleStore {
                 ov.added.insert(i, *t);
             }
         }
+        if deleted_changed {
+            ov.invalidate_deleted();
+        }
         self.overlay = if ov.is_empty() { None } else { Some(ov) };
     }
 
diff --git a/crates/sparq-core/src/store/overlay_deleted_tests.rs b/crates/sparq-core/src/store/overlay_deleted_tests.rs
index 414a29173..a3bd4f3e3 100644
--- a/crates/sparq-core/src/store/overlay_deleted_tests.rs
+++ b/crates/sparq-core/src/store/overlay_deleted_tests.rs
@@ -106,6 +106,74 @@ fn deleted_cache_matches_rebuild_after_mixed_deltas() {
     assert!(!store.has_overlay());
 }
 
+#[test]
+fn deleted_cache_survives_insert_only_and_noop_deltas() {
+    let mut reference = triples();
+    let mut store = TripleStore::from_triples(reference.clone());
+    store.apply_delta(&[], &[[1, 1, 2], [2, 2, 4]]);
+    reference.retain(|t| *t != [1, 1, 2] && *t != [2, 2, 4]);
+    sweep(&store, &reference);
+    let pointers: Vec<_> = BUILT
+        .iter()
+        .map(|&p| {
+            store.overlay.as_ref().unwrap().deleted_by_perm[p as usize]
+                .get()
+                .unwrap()
+                .as_ptr()
+        })
+        .collect();
+    for (inserts, deletes) in [
+        (vec![[20, 2, 22]], vec![]), // actual added-set change with tombstones present
+        (vec![[20, 2, 22], [3, 3, 6]], vec![[1, 1, 2], [99; 3]]), // all no-ops
+        (vec![], vec![[20, 2, 22]]), // retract an addition, not a base deletion
+        (vec![], vec![]),
+    ] {
+        store.apply_delta(&inserts, &deletes);
+        for (&perm, &pointer) in BUILT.iter().zip(&pointers) {
+            assert_eq!(
+                store.overlay.as_ref().unwrap().deleted_by_perm[perm as usize]
+                    .get()
+                    .expect("unchanged tombstones retain their projection")
+                    .as_ptr(),
+                pointer,
+            );
+        }
+        reference.retain(|t| !deletes.contains(t));
+        reference.extend(inserts);
+        reference.sort_unstable();
+        reference.dedup();
+        sweep(&store, &reference); // also pins the independent added-cache invalidation
+    }
+}
+
+#[test]
+fn deleted_cache_actual_tombstone_changes_invalidate_before_publication() {
+    let mut reference = triples();
+    let mut store = TripleStore::from_triples(reference.clone());
+    store.apply_delta(&[], &[[1, 1, 2]]);
+    reference.retain(|t| *t != [1, 1, 2]);
+    for (inserts, deletes) in [
+        (vec![], vec![[2, 2, 4]]),          // grow the deleted set
+        (vec![[1, 1, 2]], vec![]),          // undelete an existing tombstone
+        (vec![[3, 3, 6]], vec![[3, 3, 6]]), // changes during batch, even if net unchanged
+    ] {
+        sweep(&store, &reference);
+        store.apply_delta(&inserts, &deletes);
+        assert!(store
+            .overlay
+            .as_ref()
+            .unwrap()
+            .deleted_by_perm
+            .iter()
+            .all(|s| s.get().is_none()));
+        reference.retain(|t| !deletes.contains(t));
+        reference.extend(inserts);
+        reference.sort_unstable();
+        reference.dedup();
+        sweep(&store, &reference);
+    }
+}
+
 #[test]
 fn deleted_cache_inclusive_bounds_and_empty_ranges() {
     let mut ov = Overlay::default();
```


## Previously omitted reviewed store.rs327–851

Path: source/store-acfa31cf-327-851.rs; SHA256 f972a30d31af844ee2ceee78719092c2eba257131f3c507b896a90a310b2de8c

```rust
}

/// LSD radix sort of `[Id; 3]` rows into ascending lexicographic order — column 0
/// major, then 1, then 2 — the exact ordering a comparison `sort_unstable()`
/// produces on the same rows (an `[Id; 3]` compares lexicographically, and with
/// `Id = u32` the row packs into a 96-bit key whose numeric order IS that
/// lexicographic order). This is the O(n) index-build sort that replaces the
/// branchy comparison quicksort over the packed permutation tuples — the single
/// largest ingest self-time bucket (`research/engine-performance-review.md` §1.1,
/// sq-7d3dj.17). [OPUS-4.8]
///
/// Output equivalence is exact and gated: for any input, the multiset is preserved
/// and the result is fully sorted, so it is BYTE-IDENTICAL to `sort_unstable()`
/// (equal rows are indistinguishable, so stability is irrelevant). See the
/// `radix_sort_equiv_comparison_sort` differential-fuzz test.
///
/// Twelve least-significant-digit passes over the 12 key bytes (least-significant
/// first): passes 0..4 = column 2, 4..8 = column 1, 8..12 = column 0. A pass whose
/// digit is constant across every row (e.g. the high bytes of a small dictionary)
/// is a no-op and skipped; the double-buffer invariant keeps the current partial
/// result in `v` whether or not a pass runs, so the final result is always in `v`.
fn radix_sort_rows(v: &mut Vec<[Id; 3]>) {
    let n = v.len();
    if n < 2 {
        return;
    }
    // Scratch back-buffer; `v` and `scratch` are swapped after each executed pass so
    // the sorted-so-far data always lives in `v` (a skipped pass leaves it there too).
    let mut scratch: Vec<[Id; 3]> = vec![[0; 3]; n];
    for pass in 0..12usize {
        // Byte `pass` of the packed key, LSB first. col2 holds bytes 0..4, col1 4..8,
        // col0 8..12 — so the most-significant byte (pass 11) is column 0's top byte.
        let col = 2 - pass / 4;
        let shift = ((pass % 4) * 8) as u32;
        let digit = |row: &[Id; 3]| ((row[col] >> shift) & 0xff) as usize;

        // Histogram of this pass's digit.
        let mut count = [0usize; 256];
        for row in v.iter() {
            count[digit(row)] += 1;
        }
        // If every row shares one digit value this pass is a stable no-op — skip the
        // scatter (and the buffer swap), leaving the correct partial result in `v`.
        if count[digit(&v[0])] == n {
            continue;
        }
        // Prefix-sum the histogram into per-digit start offsets.
        let mut sum = 0usize;
        for c in count.iter_mut() {
            let here = *c;
            *c = sum;
            sum += here;
        }
        // Stable scatter into the back-buffer, then make it the live buffer.
        for row in v.iter() {
            let d = digit(row);
            scratch[count[d]] = *row;
            count[d] += 1;
        }
        std::mem::swap(v, &mut scratch);
    }
}

/// Stable LSD radix sort of `v` by COLUMN 0 ONLY — rows that tie on column 0 keep their
/// incoming relative order. Four passes over the four bytes of the leading column (a pass
/// whose digit is constant across every row is skipped, so a small dictionary costs one
/// or two passes, not four).
///
/// This is the primitive the DERIVED permutation build rests on
/// ([`TripleStore::derive_perm`], sq-dzfzq): re-sorting an already-sorted run by its NEW
/// leading column alone is enough to reach full lexicographic order, because stability
/// preserves the source order — which is exactly the remaining two columns, ascending —
/// inside every tie group.
fn radix_sort_rows_by_col0(v: &mut Vec<[Id; 3]>) {
    let n = v.len();
    if n < 2 {
        return;
    }
    // Allocated on the first pass that actually scatters; a fully constant leading column
    // (e.g. a single-subject graph) therefore allocates nothing at all.
    let mut scratch: Vec<[Id; 3]> = Vec::new();
    for pass in 0..4usize {
        let shift = (pass * 8) as u32;
        let digit = |row: &[Id; 3]| ((row[0] >> shift) & 0xff) as usize;

        let mut count = [0usize; 256];
        for row in v.iter() {
            count[digit(row)] += 1;
        }
        // Constant digit this pass -> a stable no-op; leave the partial result in `v`.
        if count[digit(&v[0])] == n {
            continue;
        }
        if scratch.len() != n {
            scratch = vec![[0; 3]; n];
        }
        let mut sum = 0usize;
        for c in count.iter_mut() {
            let here = *c;
            *c = sum;
            sum += here;
        }
        for row in v.iter() {
            let d = digit(row);
            scratch[count[d]] = *row;
            count[d] += 1;
        }
        std::mem::swap(v, &mut scratch);
    }
}

/// The DERIVATION PLAN (sq-dzfzq): waves of `(source, destination)` permutation pairs.
/// SPO is built once by the full 12-digit [`radix_sort_rows`] + the ONE dedup; every other
/// [`BUILT`] permutation is then DERIVED from an already-materialised one by a column
/// re-map plus a stable sort on its new leading column alone ([`radix_sort_rows_by_col0`]).
///
/// Pairs within a wave are independent (their sources are all already materialised) and so
/// run concurrently; the waves themselves are ordered. Depth is 2 derivations, so the
/// critical path is `full-sort + 2 single-column sorts` rather than six full sorts.
///
/// Every pair must satisfy the derivability invariant asserted in
/// [`TripleStore::derive_perm`]: deleting the destination's leading column from the
/// SOURCE's column order must leave the destination's other two columns, in order.
#[cfg(not(any(target_arch = "wasm32", feature = "compact-index")))]
const DERIVE_PLAN: &[&[(Perm, Perm)]] = &[
    // Wave 1 — both straight off the deduped SPO run.
    &[(Perm::Spo, Perm::Pso), (Perm::Spo, Perm::Osp)],
    // Wave 2 — off wave 1.
    &[(Perm::Pso, Perm::Ops), (Perm::Osp, Perm::Pos), (Perm::Osp, Perm::Sop)],
];
/// The `compact-index` / wasm plan: only {SPO, POS, OSP} are [`BUILT`], and POS derives
/// from OSP (deleting P from `[O,S,P]` leaves `[O,S]` = POS's trailing columns).
#[cfg(any(target_arch = "wasm32", feature = "compact-index"))]
const DERIVE_PLAN: &[&[(Perm, Perm)]] = &[&[(Perm::Spo, Perm::Osp)], &[(Perm::Osp, Perm::Pos)]];

impl TripleStore {
    /// Builds the [`BUILT`] permutation indexes from canonical s,p,o triples (all
    /// six by default; just SPO/POS/OSP under `compact-index`). SPO is sorted (in
    /// parallel) and deduplicated first; the rest are independent and built concurrently.
    pub fn from_triples(triples: Vec<[Id; 3]>) -> Self {
        let perms = Self::build_raw_perms(triples);
        let pred_stats = Self::compute_pred_stats(&perms);
        TripleStore { perms: std::sync::Arc::new(perms), pred_stats: std::sync::Arc::new(pred_stats), overlay: None }
    }

    /// Like [`from_triples`](Self::from_triples) but stores each permutation
    /// BLOCK-COMPRESSED (~4-6 B/triple vs 12) — the memory-bound storage mode for the
    /// browser, where holding 2.5x more triples in the same RAM matters more than the
    /// per-scan decode cost. Cardinality stats are computed from the raw perms *before*
    /// encoding (so neither the build nor the planner ever decodes a whole index).
    ///
    /// [FABLE-5] sq-559dp — encodes through `CompressedPerm::encode_emit`, the SAME one-place
    /// emit-format gate the on-disk save path uses, so `SPARQ_STORE_PROFILE=compressed` can opt
    /// into the `SPQCPRM2` frame-of-reference block stream (`spqcprm2` feature +
    /// `SPARQ_EMIT_FORMAT=v2` / `with_emit_format`) instead of being pinned to V1. The default
    /// build has the gate compiled out, so the in-RAM stream stays byte-identical to V1.
    pub fn from_triples_compressed(triples: Vec<[Id; 3]>) -> Self {
        let raw = Self::build_raw_perms(triples);
        let pred_stats = Self::compute_pred_stats(&raw);
        let mut perms: [PermData; 6] = std::array::from_fn(|_| PermData::default());
        for (i, pd) in raw.into_iter().enumerate() {
            if let PermData::Owned(v) = pd {
                if !v.is_empty() {
                    perms[i] = PermData::Compressed(crate::compress::CompressedPerm::encode_emit(&v));
                }
            }
        }
        TripleStore { perms: std::sync::Arc::new(perms), pred_stats: std::sync::Arc::new(pred_stats), overlay: None }
    }

    /// Builds the [`BUILT`] raw permutation indexes from canonical [s,p,o] triples (all six
    /// by default; just SPO/POS/OSP under `compact-index`). Two `cfg`-selected bodies — the
    /// PARALLEL build runs each [`DERIVE_PLAN`] wave concurrently, the NO-THREADS build runs
    /// the same plan in order — kept as separate `fn` definitions so the wasm codegen carries
    /// no rayon shape at all.
    ///
    /// [SONNET-4.6 sq-dzfzq] DERIVED build. One full 12-digit [`radix_sort_rows`] + ONE dedup
    /// produces SPO; every other permutation is then [derived](Self::derive_perm) from an
    /// already-sorted run by a column re-map plus a stable sort on its NEW LEADING COLUMN
    /// alone — at most four byte passes over one column, usually fewer (a predicate column
    /// that fits in a byte costs exactly one), and no dedup pass. Total sort work drops from
    /// six full sorts (~72 byte passes over the whole row set, plus six dedups) to one full
    /// sort plus five single-column sorts (~25 passes, one dedup). Since large-graph ingest is
    /// memory-bandwidth bound, that traffic reduction is the point; the plan's 2-derivation
    /// depth keeps the critical path short.
    ///
    /// This SUPERSEDES the sq-7d3dj.31 concurrent-N build, which sorted and deduped all six
    /// permutations independently and in parallel. Concurrent-N bought wall-clock by spending
    /// 6x the sort work at 6x the memory traffic — a good trade only while cores sit idle and
    /// bandwidth is not the binding constraint.
    ///
    /// WHICH BUILD WINS, AND WHY IT IS NOT SETTLED. The two builds trade the same quantity in
    /// opposite directions — derived does strictly less work on a critical path of ONE full
    /// sort plus 2 derivations, concurrent-N does six sorts' worth but on N independent tasks —
    /// so the outcome is governed by THREADS vs PERMUTATIONS:
    ///
    /// * With six permutations, derived is expected to win, by a margin that NARROWS as threads
    ///   are added, because concurrent-N is the arm that has parallelism left to spend.
    /// * With three permutations (`compact-index`) and no threads — the wasm configuration —
    ///   the trade does not exist and derived wins outright.
    /// * With three permutations AND enough threads to give every permutation its own core,
    ///   concurrent-N runs all three in parallel while the derived chain SPO->OSP->POS is
    ///   strictly serial, so derived LOSES. This is a knowing, documented regression in a
    ///   configuration that ships nowhere — `compact-index` is wasm-only in production
    ///   (`BUILT` keys it on `target_arch`, and the wasm build has no threads) and is
    ///   native-opt-in "for testing"; the one native consumer is the `bench/memtier` research
    ///   spike. It is NOT worth a second build path here; see the follow-up issue.
    ///
    /// The above is the STRUCTURAL argument, and the direction of the six-permutation trade —
    /// including whether concurrent-N eventually catches up at a high enough thread count — is
    /// UNVERIFIED on the canonical setup. The bead's canonical gate — ingest wall + query
    /// latency on WatDiv/synthetic-social at two scales, on the dedicated bench box — is still
    /// OUTSTANDING and must run before this is treated as a settled multi-core win. To generate
    /// current numbers for your own machine (non-canonical, do not commit them), run the
    /// in-tree A/B against the replaced body, `measure_derived_vs_radix_all_build`, sweeping
    /// `RAYON_NUM_THREADS` and the feature axis as documented on that test.
    ///
    /// Correctness: a column permutation is a BIJECTION on rows, so deduplicating SPO alone
    /// deduplicates every derived permutation, and "the deduped triple set, permuted, fully
    /// sorted" is unique — so each perm stays BYTE-IDENTICAL to the reference
    /// `sort_unstable`+dedup+permute. Gated by the `from_triples_perms_match_reference_sort`
    /// and `derived_perms_match_radix_all_build` differential tests.
    ///
    /// That byte-identity is also why this is an INGEST-ONLY change: scan/lookup/estimate, the
    /// delta-overlay and save/open all read the same rows, in the same order, out of Vecs of
    /// the same length AND capacity (the derived Vecs are exact-sized by construction — see
    /// `build_raw_perms_no_capacity_slack`). There is no first-touch cost and no query-latency
    /// dimension to trade, because no permutation is materialised any later than before. The
    /// LAZY half of sq-dzfzq — deferring a rarely-scanned permutation to its first use, which
    /// WOULD move cost into the query path — is deliberately NOT taken here; it is a separate,
    /// higher-risk change and is left to a follow-up.
    #[cfg(feature = "parallel")]
    fn build_raw_perms(mut triples: Vec<[Id; 3]>) -> [PermData; 6] {
        radix_sort_rows(&mut triples);
        triples.dedup();
        // [SONNET-4.6 sq-7d3dj.32.1] Eliminate dedup capacity slack: after dedup the Vec retains
        // its pre-dedup allocation. shrink_to_fit realigns len == capacity so heap_bytes() (which
        // counts capacity()) returns zero slack. The DERIVED perms are exact-sized by
        // construction (an ExactSize map-collect, then a same-length radix double-buffer).
        triples.shrink_to_fit();

        let mut perms: [PermData; 6] = std::array::from_fn(|_| PermData::default());
        perms[Perm::Spo as usize] = PermData::Owned(triples);
        for wave in DERIVE_PLAN {
            // Sources are materialised by an earlier wave, so the pairs inside a wave are
            // independent and derive concurrently.
            let built: Vec<(Perm, Vec<[Id; 3]>)> = wave
                .par_iter()
                .map(|&(src, dst)| (dst, Self::derive_perm(perms[src as usize].as_slice(), src, dst)))
                .collect();
            for (p, v) in built {
                perms[p as usize] = PermData::Owned(v);
            }
        }
        perms
    }

    /// Derives the `dst` permutation from the ALREADY-SORTED `src` permutation's rows: re-map
    /// the columns into `dst`'s layout, then stable-sort by `dst`'s leading column ALONE
    /// ([`radix_sort_rows_by_col0`]) — at most four byte passes over one column instead of the
    /// twelve a from-scratch [`radix_sort_rows`] costs, and no dedup pass (a column permutation
    /// is a bijection on rows, so the deduped SPO multiset stays deduped).
    ///
    /// WHY ONE COLUMN SUFFICES. `src` is in full lexicographic order, so inside any group of
    /// rows sharing a value of `dst`'s leading column the rows are still in `src` order. A
    /// STABLE sort leaves that intra-group order untouched. So the result is fully `dst`-sorted
    /// exactly when `src`'s order, restricted to such a group, already agrees with `dst`'s
    /// remaining two columns — which is precisely the invariant asserted below: deleting
    /// `dst`'s leading column from `src`'s column order leaves `dst`'s trailing columns, in
    /// order. Every pair in [`DERIVE_PLAN`] satisfies it, and
    /// `derive_plan_pairs_are_derivable` pins that for the plan as shipped.
    ///
    /// Output is BYTE-IDENTICAL to sorting the mapped rows from scratch (both are "the deduped
    /// triple set, permuted, fully sorted" — and a fully sorted deduped set is unique), which is
    /// what keeps scan/lookup/estimate, the delta-overlay and save/open untouched. Gated by
    /// `from_triples_perms_match_reference_sort` and `derived_perms_match_radix_all_build`.
    fn derive_perm(src_rows: &[[Id; 3]], src: Perm, dst: Perm) -> Vec<[Id; 3]> {
        let (so, dor) = (src.order(), dst.order());
        debug_assert!(
            so.iter().copied().filter(|&c| c != dor[0]).eq(dor[1..].iter().copied()),
            "{:?} is not derivable from {:?}: deleting the leading column does not leave the rest in order",
            dst,
            src
        );
        // Column j of a dst row is column `pick[j]` of a src row.
        let pick: [usize; 3] =
            std::array::from_fn(|j| so.iter().position(|&c| c == dor[j]).expect("a permutation covers every column"));
        // Pre-sized exactly from the known row count (the mapped iterator is ExactSize, so
        // `collect` reserves `src_rows.len()` up front — no grow tail, no capacity slack).
        let mut v: Vec<[Id; 3]> =
            src_rows.iter().map(|r| [r[pick[0]], r[pick[1]], r[pick[2]]]).collect();
        radix_sort_rows_by_col0(&mut v);
        v
    }

    /// No-threads build of the permutation indexes (wasm / no-rayon path). Deduplicates via
    /// the SPO ordering first (radix — no parallel sort to beat it here), then walks
    /// [`DERIVE_PLAN`] in order, reusing the deduped array for the SPO slot. [SONNET-4.6
    /// sq-7d3dj.32.1] shrink_to_fit after dedup so the SPO slot carries zero capacity slack
    /// (the derived perms are exact by construction). [SONNET-4.6 sq-dzfzq] each derived perm
    /// now costs one single-column sort instead of a full 12-digit one — on wasm, where there
    /// are no threads to hide the work behind, this is the whole saving.
    /// The wasm bundle byte count changes from the historical value — declared in
    /// bench/feature-off-declarations/ and bench/perf-baseline.json feature_off_exact.
    #[cfg(not(feature = "parallel"))]
    fn build_raw_perms(mut triples: Vec<[Id; 3]>) -> [PermData; 6] {
        radix_sort_rows(&mut triples);
        triples.dedup();
        // [SONNET-4.6 sq-7d3dj.32.1] Release the pre-dedup capacity so the SPO slot is
        // exact-sized.  heap_bytes() counts capacity(); without this call it would count
        // the full pre-dedup allocation even after duplicates are removed.
        triples.shrink_to_fit();

        // Place each permutation at its canonical slot; the rest stay empty. Every non-SPO
        // BUILT permutation is DERIVED from an already-materialised one (sq-dzfzq) — with no
        // threads there is nothing to overlap, so the waves just run in order and the saving is
        // pure work: one full 12-digit sort plus one single-column sort per derived perm,
        // instead of a full sort (and a dedup) each.
        let mut perms: [PermData; 6] = std::array::from_fn(|_| PermData::default());
        perms[Perm::Spo as usize] = PermData::Owned(triples);
        for &(src, dst) in DERIVE_PLAN.iter().copied().flatten() {
            perms[dst as usize] = PermData::Owned(Self::derive_perm(perms[src as usize].as_slice(), src, dst));
        }
        perms
    }

    /// Persists the permutation indexes to `dir` (one raw little-endian `[u32;3]` file
    /// per permutation) so they can be memory-mapped later via [`open`](Self::open) —
    /// the on-disk side of out-of-core querying.
    #[cfg(feature = "mmap")]
    pub fn save(&self, dir: &std::path::Path) -> std::io::Result<()> {
        self.save_with(dir, false)
    }

    /// Like [`save`](Self::save) but writes each permutation BLOCK-COMPRESSED (the
    /// delta+varint format of [`crate::compress`], ~3-5x smaller on disk). The files are
    /// auto-detected by [`open`](Self::open) via [`crate::compress::FILE_MAGIC`], so old
    /// raw directories keep working and the two formats can be mixed.
    #[cfg(feature = "mmap")]
    pub fn save_compressed(&self, dir: &std::path::Path) -> std::io::Result<()> {
        self.save_with(dir, true)
    }

    #[cfg(feature = "mmap")]
    fn save_with(&self, dir: &std::path::Path, compressed: bool) -> std::io::Result<()> {
        std::fs::create_dir_all(dir)?;
        for (i, p) in self.perms.iter().enumerate() {
            // Raw modes borrow zero-copy; a compressed perm is decoded back to raw rows so
            // `save` is total (e.g. a `load_str_compressed` graph can still be persisted).
            let rows: std::borrow::Cow<[[Id; 3]]> = match p {
                PermData::Compressed(c) => std::borrow::Cow::Owned(c.decode_all()),
                _ => std::borrow::Cow::Borrowed(p.as_slice()),
            };
            // A pending delta-overlay is FOLDED into every BUILT permutation on save, so
            // the persisted base always reflects the full current state (unbuilt perms
            // stay empty). The in-memory overlay is untouched (`save` takes `&self`).
            let rows: std::borrow::Cow<[[Id; 3]]> = match &self.overlay {
                Some(ov) if BUILT.contains(&Perm::ALL[i]) => {
                    std::borrow::Cow::Owned(ov.merge(&rows, Perm::ALL[i], [Id::MIN; 3], [Id::MAX; 3]))
                }
                _ => rows,
            };
            let path = dir.join(format!("perm{i}.bin"));
            if compressed && !rows.is_empty() {
                // Unbuilt (empty) permutations stay raw-empty so `open` skips them by size.
                // [FABLE-5] sq-7d3dj.32.2.7: `encode_emit` honours the emit-format config gate —
                // `SPQCPRM1` by default, `SPQCPRM2` only when a `spqcprm2` build has opted in.
                let mut w = std::io::BufWriter::new(std::fs::File::create(path)?);
                crate::compress::CompressedPerm::encode_emit(&rows).write_to(&mut w)?;
                std::io::Write::flush(&mut w)?;
            } else {
                // SAFETY: reinterpret the contiguous [u32;3] rows as bytes for writing.
                let bytes = unsafe { std::slice::from_raw_parts(rows.as_ptr().cast::<u8>(), std::mem::size_of_val(rows.as_ref())) };
                std::fs::write(path, bytes)?;
            }
        }
        self.save_pred_stats(dir)
    }

    /// Persists per-predicate statistics in ascending predicate-ID order.
    ///
    /// This lets `open` avoid re-scanning the POS/PSO indexes
    /// (a ~2-permutation read — the dominant out-of-core open cost + resident RSS once the
    /// dict is mmap'd). Small: a handful of fields per distinct predicate.
    ///
    /// # Errors
    /// Returns an error if creating, writing, or flushing the statistics file fails.
    #[cfg(feature = "mmap")]
    pub fn save_pred_stats(&self, dir: &std::path::Path) -> std::io::Result<()> {
        use std::io::Write;
        let mut w = std::io::BufWriter::new(std::fs::File::create(dir.join("predstats.bin"))?);
        w.write_all(&(self.pred_stats.len() as u64).to_le_bytes())?;
        // [GPT-6] Hash-map iteration can change after loading/reserving identical stats.
        // Canonicalize the shared writer so external builds and re-saves agree bytewise.
        let mut entries: Vec<_> = self.pred_stats.iter().collect();
        entries.sort_unstable_by_key(|&(&p, _)| p);
        for (&p, s) in entries {
            w.write_all(&p.to_le_bytes())?;
            w.write_all(&(s.count as u64).to_le_bytes())?;
            w.write_all(&(s.ndv_subj as u64).to_le_bytes())?;
            w.write_all(&(s.ndv_obj as u64).to_le_bytes())?;
        }
        w.flush()
    }

    /// Loads persisted per-predicate stats (written by [`save_pred_stats`]); `None` if the
    /// file is absent (an older saved dir) so the caller falls back to recomputing.
    #[cfg(feature = "mmap")]
    fn load_pred_stats(dir: &std::path::Path) -> Option<FxHashMap<Id, PredStat>> {
        use std::io::Read;
        let mut r = std::io::BufReader::new(std::fs::File::open(dir.join("predstats.bin")).ok()?);
        fn rd8(r: &mut impl Read) -> Option<u64> {
            let mut b = [0u8; 8];
            r.read_exact(&mut b).ok()?;
            Some(u64::from_le_bytes(b))
        }
        // The predicate id is written as a little-endian `Id` (u32, 4 bytes) by
        // `save_pred_stats` — this loader used to read 8 bytes for it, mis-framing every
        // record, so the load ALWAYS failed and `open` silently fell back to recomputing
        // the stats, paging in the whole POS+PSO indexes (~24 B/triple of resident memory
        // and most of the out-of-core open time). Measured in research/memory-tiering.md.
        fn rd_id(r: &mut impl Read) -> Option<Id> {
            let mut b = [0u8; std::mem::size_of::<Id>()];
            r.read_exact(&mut b).ok()?;
            Some(Id::from_le_bytes(b))
        }
        // [OPUS-4.8] sq-f5jh: `predstats.bin` is an UNTRUSTED on-disk file (trust boundary
        // B5). `n` is a u64 count read straight from it, and `reserve(n)` was unbounded — a
        // single flipped count byte could ask `FxHashMap` to pre-allocate billions of slots
        // (~17 B each) and ABORT the process (uncatchable OOM DoS; under llvm-cov's added
        // memory pressure this is the residual rc=101 / coverage-undercount trigger). Each
        // record on disk is `size_of::<Id>() + 24` bytes (id + three u64s), so the file
        // length is a hard upper bound on the real record count: clamp the reservation to it
        // (the per-record `read_exact`s below still error cleanly via `?`/`None` if the file
        // actually ends early). We never reserve for more records than can possibly fit.
        let n = rd8(&mut r)? as usize;
        const PREDSTAT_REC_BYTES: usize = std::mem::size_of::<Id>() + 24; // id + count + ndv_subj + ndv_obj
        let file_len = std::fs::metadata(dir.join("predstats.bin")).ok()?.len() as usize;
        let max_records = file_len.saturating_sub(8) / PREDSTAT_REC_BYTES; // 8-byte header
        let mut stats = FxHashMap::default();
        stats.reserve(n.min(max_records));
        for _ in 0..n {
            let p = rd_id(&mut r)?;
            let count = rd8(&mut r)? as usize;
            let ndv_subj = rd8(&mut r)? as usize;
            let ndv_obj = rd8(&mut r)? as usize;
            stats.insert(p, PredStat { count, ndv_subj, ndv_obj });
        }
        Some(stats)
    }

    /// Opens a store whose permutations are MEMORY-MAPPED from `dir` (written by
    /// [`save`](Self::save)). The 6 (or 3, compact) index files stay on disk; the OS
    /// pages in only the ranges a query touches, so datasets larger than RAM are
    /// queryable. Per-predicate stats are recomputed from the mapped POS/PSO indexes.
    #[cfg(feature = "mmap")]
    pub fn open(dir: &std::path::Path) -> std::io::Result<Self> {
        let mut perms: [PermData; 6] = std::array::from_fn(|_| PermData::default());
        for (i, slot) in perms.iter_mut().enumerate() {
            let path = dir.join(format!("perm{i}.bin"));
            let file = std::fs::File::open(&path)?;
            if file.metadata()?.len() == 0 {
                continue; // an empty (unbuilt, e.g. compact-index) permutation
            }
            // SAFETY: the file is owned by this store for its lifetime and is not mutated.
            let map = unsafe { memmap2::Mmap::map(&file)? };
            // FORMAT AUTO-DETECTION: a block-compressed file (written by `save_compressed`)
            // starts with FILE_MAGIC (`SPQCPRM1`) or, for a `spqcprm2`-emitting build,
            // FILE_MAGIC_V2 (`SPQCPRM2`); anything else is the original raw [u32;3] format.
            // [FABLE-5] sq-7d3dj.32.2.7: both magics route to `from_mmap`, which re-checks the
            // magic and picks the V1/V2 decode reader — a V1 file decodes byte-identically
            // forever. Compressed perms are served lazily — block-wise decode off the mapped file.
            *slot = if map.len() >= 8
                && (map[..8] == crate::compress::FILE_MAGIC || map[..8] == crate::compress::FILE_MAGIC_V2)
            {
                PermData::Compressed(crate::compress::CompressedPerm::from_mmap(map)?)
            } else {
                PermData::Mapped(map)
            };
        }
        // Use the persisted stats if present (no POS/PSO re-scan — keeps open fast and the
        // resident set small); else recompute (backward compatible with older saved dirs).
        let pred_stats = Self::load_pred_stats(dir).unwrap_or_else(|| Self::compute_pred_stats(&perms));
        Ok(TripleStore { perms: std::sync::Arc::new(perms), pred_stats: std::sync::Arc::new(pred_stats), overlay: None })
    }

    /// Per-predicate stats: count + distinct objects from POS (always built), and
    /// distinct subjects from PSO when it is built (the full six-permutation index),
    /// else approximated by the count (under `compact-index`, where PSO is absent —
    /// the planner then treats subjects as non-selective, which is safe for ordering).
    fn compute_pred_stats(perms: &[PermData; 6]) -> FxHashMap<Id, PredStat> {
        // Full-range `rows_in`: raw modes borrow the whole slice (zero-copy, as before);
        // a compressed perm (an opened compressed dir missing predstats.bin) is decoded.
        let pos = perms[Perm::Pos as usize].rows_in([Id::MIN; 3], [Id::MAX; 3]); // [P, O, S]
        let pso = perms[Perm::Pso as usize].rows_in([Id::MIN; 3], [Id::MAX; 3]); // [P, S, O], empty under compact-index
        let mut stats: FxHashMap<Id, PredStat> = FxHashMap::default();
        // POS: count + distinct O per P. ndv_subj defaults to count (refined below).
        let mut i = 0;
        while i < pos.len() {
            let p = pos[i][0];
            let (mut count, mut ndv_o, mut last_o) = (0usize, 0usize, None);
            while i < pos.len() && pos[i][0] == p {
                count += 1;
                if last_o != Some(pos[i][1]) {
                    ndv_o += 1;
                    last_o = Some(pos[i][1]);
                }
                i += 1;
            }
            stats.insert(p, PredStat { count, ndv_subj: count, ndv_obj: ndv_o });
        }
        // PSO (when built): exact distinct S per P.
        let mut i = 0;
        while i < pso.len() {
            let p = pso[i][0];
            let (mut ndv_s, mut last_s) = (0usize, None);
            while i < pso.len() && pso[i][0] == p {
                if last_s != Some(pso[i][1]) {
                    ndv_s += 1;
                    last_s = Some(pso[i][1]);
                }
                i += 1;
            }
            stats.entry(p).or_default().ndv_subj = ndv_s;
        }
```


## Complete final production store context through estimate

Exact current store.rs from file start through the TripleStore implementation; includes permutation/type definitions, cache bodies, constructor/save/open context, and all mutation/read/clone callers. Remaining Scan/test definitions are in the full manifest source.

```rust
//! Triple store: the six sorted permutation indexes over dictionary-encoded
//! triples (Hexastore / RDF-3X / QLever design).
//!
//! Storing all six orderings (SPO SOP PSO POS OSP OPS) means every triple
//! pattern is answered by a single contiguous range (binary search on the
//! bound prefix), and the scan output is sorted by the remaining positions —
//! which is exactly what merge joins need. M1 holds each permutation as a
//! sorted `Vec<[Id; 3]>`; later milestones replace these with block-compressed,
//! optionally memory-mapped columns.

use crate::dict::Id;
#[cfg(feature = "parallel")]
use rayon::prelude::*;

/// The six permutations. Each names the order of (subject, predicate, object)
/// columns as stored.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Perm {
    Spo,
    Sop,
    Pso,
    Pos,
    Osp,
    Ops,
}

/// The permutations actually built and searched. The full six give every triple
/// pattern a sorted scan in the order any merge join wants. The `compact-index` set
/// {SPO, POS, OSP} still answers EVERY triple pattern from one index (SPO→S*/SP*,
/// POS→P*/PO*, OSP→O*/OS*) at half the memory, at the cost of some merge joins (and
/// some lazy-count fast paths) falling back to hashing / sorting.
// Compact set on wasm ALWAYS (memory-bound target), or on native opt-in via the
// `compact-index` feature (for testing). Keyed on `target_arch` — NOT just a feature —
// so the wasm choice does not leak to the native build via Cargo feature unification.
#[cfg(not(any(target_arch = "wasm32", feature = "compact-index")))]
pub const BUILT: &[Perm] = &[Perm::Spo, Perm::Sop, Perm::Pso, Perm::Pos, Perm::Osp, Perm::Ops];
#[cfg(any(target_arch = "wasm32", feature = "compact-index"))]
pub const BUILT: &[Perm] = &[Perm::Spo, Perm::Pos, Perm::Osp];

impl Perm {
    pub const ALL: [Perm; 6] = [Perm::Spo, Perm::Sop, Perm::Pso, Perm::Pos, Perm::Osp, Perm::Ops];

    /// The column indices (into a canonical s,p,o triple) in this permutation's
    /// sort order. e.g. POS -> 1,2,0.
    #[inline]
    pub fn order(self) -> [usize; 3] {
        match self {
            Perm::Spo => [0, 1, 2],
            Perm::Sop => [0, 2, 1],
            Perm::Pso => [1, 0, 2],
            Perm::Pos => [1, 2, 0],
            Perm::Osp => [2, 0, 1],
            Perm::Ops => [2, 1, 0],
        }
    }
}

/// A triple pattern over ids: `None` is a variable (wildcard), `Some(id)` is
/// bound.
pub type Pattern = [Option<Id>; 3];

use rustc_hash::{FxHashMap, FxHashSet};

/// Per-predicate statistics for cardinality estimation (a characteristic-set-lite
/// summary): how many triples use the predicate, and how many *distinct* subjects
/// and objects it relates. Lets the planner estimate join result sizes.
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub struct PredStat {
    pub count: usize,
    pub ndv_subj: usize,
    pub ndv_obj: usize,
}

/// A permutation index's storage: either an in-memory `Vec` (built / loaded) or, with
/// the `mmap` feature, a memory-mapped on-disk file — so a dataset larger than RAM can
/// be queried, the OS paging in only the working set (out-of-core).
enum PermData {
    Owned(Vec<[Id; 3]>),
    #[cfg(feature = "mmap")]
    Mapped(memmap2::Mmap),
    /// Block-compressed (~4-6 B/triple vs 12). The memory-bound storage mode for the
    /// browser: scans decode only the blocks the key-range touches. See [`compress`].
    Compressed(crate::compress::CompressedPerm),
}

impl Default for PermData {
    fn default() -> Self {
        PermData::Owned(Vec::new())
    }
}

impl PermData {
    /// Borrows the rows as a contiguous slice. Valid only for the raw (Owned/Mapped)
    /// modes — the compressed mode has no flat layout, so callers that may hold a
    /// compressed perm must go through [`rows_in`](Self::rows_in) instead.
    #[inline]
    fn as_slice(&self) -> &[[Id; 3]] {
        match self {
            PermData::Owned(v) => v,
            #[cfg(feature = "mmap")]
            PermData::Mapped(m) => {
                let bytes: &[u8] = m;
                let n = bytes.len() / std::mem::size_of::<[Id; 3]>();
                // SAFETY: the file is a whole number of little-endian [u32;3] triples and
                // an mmap is page-aligned (>= the 4-byte alignment of `u32`).
                unsafe { std::slice::from_raw_parts(bytes.as_ptr().cast::<[Id; 3]>(), n) }
            }
            PermData::Compressed(_) => unreachable!("as_slice on a compressed permutation"),
        }
    }

    /// The rows matching the inclusive key range `[lo, hi]`, sorted. Raw modes binary-
    /// search and BORROW a sub-slice (no allocation); the compressed mode decodes only
    /// the spanning blocks and returns an OWNED `Vec`. Either way the operators above
    /// receive a `&[[Id;3]]` (via the `Cow`), so their algorithms are unchanged.
    #[inline]
    fn rows_in(&self, lo: [Id; 3], hi: [Id; 3]) -> std::borrow::Cow<'_, [[Id; 3]]> {
        match self {
            PermData::Compressed(c) => std::borrow::Cow::Owned(c.range(lo, hi)),
            _ => {
                let rows = self.as_slice();
                let s = lower_bound(rows, &lo);
                let e = upper_bound(rows, &hi);
                std::borrow::Cow::Borrowed(&rows[s..e])
            }
        }
    }

    /// Cheap count of rows in `[lo, hi]` (for the planner) — no full materialization.
    #[inline]
    fn count_in(&self, lo: [Id; 3], hi: [Id; 3]) -> usize {
        match self {
            PermData::Compressed(c) => c.count_range(lo, hi),
            _ => {
                let rows = self.as_slice();
                upper_bound(rows, &hi) - lower_bound(rows, &lo)
            }
        }
    }

    #[inline]
    fn len(&self) -> usize {
        match self {
            PermData::Compressed(c) => c.len(),
            _ => self.as_slice().len(),
        }
    }

    fn heap_bytes(&self) -> usize {
        match self {
            PermData::Owned(v) => v.capacity() * std::mem::size_of::<[Id; 3]>(),
            #[cfg(feature = "mmap")]
            PermData::Mapped(_) => 0, // resident pages are charged to the OS page cache, not the heap
            PermData::Compressed(c) => c.heap_bytes(),
        }
    }
}

/// Pending updates layered over the immutable base indexes (the T17 delta-overlay):
/// triples INSERTED since the last compaction, and base triples DELETED since then.
/// Consulted at scan time — the base stays immutable (and mmap-able), and an update
/// batch costs O(batch) instead of the O(n) full rebuild. Invariants kept by
/// [`TripleStore::apply_delta`]: `added` is canonical-SPO sorted + deduplicated and
/// DISJOINT from both the base and `deleted`; `deleted` only ever holds base triples.
///
/// `Clone` because [`TripleStore::fork`] carries the overlay into the forked store
/// BY VALUE (O(overlay), bounded by the compaction policy) while the base indexes are
/// shared structurally.
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
    /// projection; [GPT-6 Astra] `deleted` uses the same lazy projection strategy.
    fn count_correction(&self, perm: Perm, lo: [Id; 3], hi: [Id; 3]) -> (usize, usize) {
        let add = self.added_rows(perm, lo, hi).len();
        let del = self.deleted_count(perm, lo, hi);
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
            .chain(&self.deleted_by_perm)
            .filter_map(|slot| slot.get())
            .map(|rows| rows.capacity() * std::mem::size_of::<[Id; 3]>())
            .sum();
        self.added.capacity() * std::mem::size_of::<[Id; 3]>()
            + self.deleted.capacity() * 13
            + cached
    }
}

pub struct TripleStore {
    // Each permutation in its column order, sorted (so binary search on a bound prefix
    // is a plain lexicographic comparison of the leading columns) — owned or mmap'd.
    //
    // Behind an `Arc` so [`fork`](Self::fork) can SHARE the immutable base indexes
    // across snapshot generations (the structural fork): every store is born
    // shareable, a fork is an Arc bump. The only post-build mutation,
    // [`decompress_to_ram`](Self::decompress_to_ram), goes through `Arc::get_mut`
    // (it runs on freshly opened, never-yet-shared stores). Cost when unused: one
    // extra pointer indirection per scan/estimate CALL (not per row) — measured in
    // the flat-read benchmark as within noise.
    perms: std::sync::Arc<[PermData; 6]>,
    // Per-predicate stats keyed by predicate id (for the cost-based planner).
    // Arc-shared across forks like the permutations (read-only after build).
    pred_stats: std::sync::Arc<FxHashMap<Id, PredStat>>,
    // The delta-overlay of pending updates, `None` when there are none — so the scan
    // hot path pays exactly one (perfectly predicted) branch when no update happened.
    // NOTE: `pred_stats` is not overlay-adjusted (planner estimates only); `estimate`
    // and `len` are exact.
    overlay: Option<Box<Overlay>>,
}

/// LSD radix sort of `[Id; 3]` rows into ascending lexicographic order — column 0
/// major, then 1, then 2 — the exact ordering a comparison `sort_unstable()`
/// produces on the same rows (an `[Id; 3]` compares lexicographically, and with
/// `Id = u32` the row packs into a 96-bit key whose numeric order IS that
/// lexicographic order). This is the O(n) index-build sort that replaces the
/// branchy comparison quicksort over the packed permutation tuples — the single
/// largest ingest self-time bucket (`research/engine-performance-review.md` §1.1,
/// sq-7d3dj.17). [OPUS-4.8]
///
/// Output equivalence is exact and gated: for any input, the multiset is preserved
/// and the result is fully sorted, so it is BYTE-IDENTICAL to `sort_unstable()`
/// (equal rows are indistinguishable, so stability is irrelevant). See the
/// `radix_sort_equiv_comparison_sort` differential-fuzz test.
///
/// Twelve least-significant-digit passes over the 12 key bytes (least-significant
/// first): passes 0..4 = column 2, 4..8 = column 1, 8..12 = column 0. A pass whose
/// digit is constant across every row (e.g. the high bytes of a small dictionary)
/// is a no-op and skipped; the double-buffer invariant keeps the current partial
/// result in `v` whether or not a pass runs, so the final result is always in `v`.
fn radix_sort_rows(v: &mut Vec<[Id; 3]>) {
    let n = v.len();
    if n < 2 {
        return;
    }
    // Scratch back-buffer; `v` and `scratch` are swapped after each executed pass so
    // the sorted-so-far data always lives in `v` (a skipped pass leaves it there too).
    let mut scratch: Vec<[Id; 3]> = vec![[0; 3]; n];
    for pass in 0..12usize {
        // Byte `pass` of the packed key, LSB first. col2 holds bytes 0..4, col1 4..8,
        // col0 8..12 — so the most-significant byte (pass 11) is column 0's top byte.
        let col = 2 - pass / 4;
        let shift = ((pass % 4) * 8) as u32;
        let digit = |row: &[Id; 3]| ((row[col] >> shift) & 0xff) as usize;

        // Histogram of this pass's digit.
        let mut count = [0usize; 256];
        for row in v.iter() {
            count[digit(row)] += 1;
        }
        // If every row shares one digit value this pass is a stable no-op — skip the
        // scatter (and the buffer swap), leaving the correct partial result in `v`.
        if count[digit(&v[0])] == n {
            continue;
        }
        // Prefix-sum the histogram into per-digit start offsets.
        let mut sum = 0usize;
        for c in count.iter_mut() {
            let here = *c;
            *c = sum;
            sum += here;
        }
        // Stable scatter into the back-buffer, then make it the live buffer.
        for row in v.iter() {
            let d = digit(row);
            scratch[count[d]] = *row;
            count[d] += 1;
        }
        std::mem::swap(v, &mut scratch);
    }
}

/// Stable LSD radix sort of `v` by COLUMN 0 ONLY — rows that tie on column 0 keep their
/// incoming relative order. Four passes over the four bytes of the leading column (a pass
/// whose digit is constant across every row is skipped, so a small dictionary costs one
/// or two passes, not four).
///
/// This is the primitive the DERIVED permutation build rests on
/// ([`TripleStore::derive_perm`], sq-dzfzq): re-sorting an already-sorted run by its NEW
/// leading column alone is enough to reach full lexicographic order, because stability
/// preserves the source order — which is exactly the remaining two columns, ascending —
/// inside every tie group.
fn radix_sort_rows_by_col0(v: &mut Vec<[Id; 3]>) {
    let n = v.len();
    if n < 2 {
        return;
    }
    // Allocated on the first pass that actually scatters; a fully constant leading column
    // (e.g. a single-subject graph) therefore allocates nothing at all.
    let mut scratch: Vec<[Id; 3]> = Vec::new();
    for pass in 0..4usize {
        let shift = (pass * 8) as u32;
        let digit = |row: &[Id; 3]| ((row[0] >> shift) & 0xff) as usize;

        let mut count = [0usize; 256];
        for row in v.iter() {
            count[digit(row)] += 1;
        }
        // Constant digit this pass -> a stable no-op; leave the partial result in `v`.
        if count[digit(&v[0])] == n {
            continue;
        }
        if scratch.len() != n {
            scratch = vec![[0; 3]; n];
        }
        let mut sum = 0usize;
        for c in count.iter_mut() {
            let here = *c;
            *c = sum;
            sum += here;
        }
        for row in v.iter() {
            let d = digit(row);
            scratch[count[d]] = *row;
            count[d] += 1;
        }
        std::mem::swap(v, &mut scratch);
    }
}

/// The DERIVATION PLAN (sq-dzfzq): waves of `(source, destination)` permutation pairs.
/// SPO is built once by the full 12-digit [`radix_sort_rows`] + the ONE dedup; every other
/// [`BUILT`] permutation is then DERIVED from an already-materialised one by a column
/// re-map plus a stable sort on its new leading column alone ([`radix_sort_rows_by_col0`]).
///
/// Pairs within a wave are independent (their sources are all already materialised) and so
/// run concurrently; the waves themselves are ordered. Depth is 2 derivations, so the
/// critical path is `full-sort + 2 single-column sorts` rather than six full sorts.
///
/// Every pair must satisfy the derivability invariant asserted in
/// [`TripleStore::derive_perm`]: deleting the destination's leading column from the
/// SOURCE's column order must leave the destination's other two columns, in order.
#[cfg(not(any(target_arch = "wasm32", feature = "compact-index")))]
const DERIVE_PLAN: &[&[(Perm, Perm)]] = &[
    // Wave 1 — both straight off the deduped SPO run.
    &[(Perm::Spo, Perm::Pso), (Perm::Spo, Perm::Osp)],
    // Wave 2 — off wave 1.
    &[(Perm::Pso, Perm::Ops), (Perm::Osp, Perm::Pos), (Perm::Osp, Perm::Sop)],
];
/// The `compact-index` / wasm plan: only {SPO, POS, OSP} are [`BUILT`], and POS derives
/// from OSP (deleting P from `[O,S,P]` leaves `[O,S]` = POS's trailing columns).
#[cfg(any(target_arch = "wasm32", feature = "compact-index"))]
const DERIVE_PLAN: &[&[(Perm, Perm)]] = &[&[(Perm::Spo, Perm::Osp)], &[(Perm::Osp, Perm::Pos)]];

impl TripleStore {
    /// Builds the [`BUILT`] permutation indexes from canonical s,p,o triples (all
    /// six by default; just SPO/POS/OSP under `compact-index`). SPO is sorted (in
    /// parallel) and deduplicated first; the rest are independent and built concurrently.
    pub fn from_triples(triples: Vec<[Id; 3]>) -> Self {
        let perms = Self::build_raw_perms(triples);
        let pred_stats = Self::compute_pred_stats(&perms);
        TripleStore { perms: std::sync::Arc::new(perms), pred_stats: std::sync::Arc::new(pred_stats), overlay: None }
    }

    /// Like [`from_triples`](Self::from_triples) but stores each permutation
    /// BLOCK-COMPRESSED (~4-6 B/triple vs 12) — the memory-bound storage mode for the
    /// browser, where holding 2.5x more triples in the same RAM matters more than the
    /// per-scan decode cost. Cardinality stats are computed from the raw perms *before*
    /// encoding (so neither the build nor the planner ever decodes a whole index).
    ///
    /// [FABLE-5] sq-559dp — encodes through `CompressedPerm::encode_emit`, the SAME one-place
    /// emit-format gate the on-disk save path uses, so `SPARQ_STORE_PROFILE=compressed` can opt
    /// into the `SPQCPRM2` frame-of-reference block stream (`spqcprm2` feature +
    /// `SPARQ_EMIT_FORMAT=v2` / `with_emit_format`) instead of being pinned to V1. The default
    /// build has the gate compiled out, so the in-RAM stream stays byte-identical to V1.
    pub fn from_triples_compressed(triples: Vec<[Id; 3]>) -> Self {
        let raw = Self::build_raw_perms(triples);
        let pred_stats = Self::compute_pred_stats(&raw);
        let mut perms: [PermData; 6] = std::array::from_fn(|_| PermData::default());
        for (i, pd) in raw.into_iter().enumerate() {
            if let PermData::Owned(v) = pd {
                if !v.is_empty() {
                    perms[i] = PermData::Compressed(crate::compress::CompressedPerm::encode_emit(&v));
                }
            }
        }
        TripleStore { perms: std::sync::Arc::new(perms), pred_stats: std::sync::Arc::new(pred_stats), overlay: None }
    }

    /// Builds the [`BUILT`] raw permutation indexes from canonical [s,p,o] triples (all six
    /// by default; just SPO/POS/OSP under `compact-index`). Two `cfg`-selected bodies — the
    /// PARALLEL build runs each [`DERIVE_PLAN`] wave concurrently, the NO-THREADS build runs
    /// the same plan in order — kept as separate `fn` definitions so the wasm codegen carries
    /// no rayon shape at all.
    ///
    /// [SONNET-4.6 sq-dzfzq] DERIVED build. One full 12-digit [`radix_sort_rows`] + ONE dedup
    /// produces SPO; every other permutation is then [derived](Self::derive_perm) from an
    /// already-sorted run by a column re-map plus a stable sort on its NEW LEADING COLUMN
    /// alone — at most four byte passes over one column, usually fewer (a predicate column
    /// that fits in a byte costs exactly one), and no dedup pass. Total sort work drops from
    /// six full sorts (~72 byte passes over the whole row set, plus six dedups) to one full
    /// sort plus five single-column sorts (~25 passes, one dedup). Since large-graph ingest is
    /// memory-bandwidth bound, that traffic reduction is the point; the plan's 2-derivation
    /// depth keeps the critical path short.
    ///
    /// This SUPERSEDES the sq-7d3dj.31 concurrent-N build, which sorted and deduped all six
    /// permutations independently and in parallel. Concurrent-N bought wall-clock by spending
    /// 6x the sort work at 6x the memory traffic — a good trade only while cores sit idle and
    /// bandwidth is not the binding constraint.
    ///
    /// WHICH BUILD WINS, AND WHY IT IS NOT SETTLED. The two builds trade the same quantity in
    /// opposite directions — derived does strictly less work on a critical path of ONE full
    /// sort plus 2 derivations, concurrent-N does six sorts' worth but on N independent tasks —
    /// so the outcome is governed by THREADS vs PERMUTATIONS:
    ///
    /// * With six permutations, derived is expected to win, by a margin that NARROWS as threads
    ///   are added, because concurrent-N is the arm that has parallelism left to spend.
    /// * With three permutations (`compact-index`) and no threads — the wasm configuration —
    ///   the trade does not exist and derived wins outright.
    /// * With three permutations AND enough threads to give every permutation its own core,
    ///   concurrent-N runs all three in parallel while the derived chain SPO->OSP->POS is
    ///   strictly serial, so derived LOSES. This is a knowing, documented regression in a
    ///   configuration that ships nowhere — `compact-index` is wasm-only in production
    ///   (`BUILT` keys it on `target_arch`, and the wasm build has no threads) and is
    ///   native-opt-in "for testing"; the one native consumer is the `bench/memtier` research
    ///   spike. It is NOT worth a second build path here; see the follow-up issue.
    ///
    /// The above is the STRUCTURAL argument, and the direction of the six-permutation trade —
    /// including whether concurrent-N eventually catches up at a high enough thread count — is
    /// UNVERIFIED on the canonical setup. The bead's canonical gate — ingest wall + query
    /// latency on WatDiv/synthetic-social at two scales, on the dedicated bench box — is still
    /// OUTSTANDING and must run before this is treated as a settled multi-core win. To generate
    /// current numbers for your own machine (non-canonical, do not commit them), run the
    /// in-tree A/B against the replaced body, `measure_derived_vs_radix_all_build`, sweeping
    /// `RAYON_NUM_THREADS` and the feature axis as documented on that test.
    ///
    /// Correctness: a column permutation is a BIJECTION on rows, so deduplicating SPO alone
    /// deduplicates every derived permutation, and "the deduped triple set, permuted, fully
    /// sorted" is unique — so each perm stays BYTE-IDENTICAL to the reference
    /// `sort_unstable`+dedup+permute. Gated by the `from_triples_perms_match_reference_sort`
    /// and `derived_perms_match_radix_all_build` differential tests.
    ///
    /// That byte-identity is also why this is an INGEST-ONLY change: scan/lookup/estimate, the
    /// delta-overlay and save/open all read the same rows, in the same order, out of Vecs of
    /// the same length AND capacity (the derived Vecs are exact-sized by construction — see
    /// `build_raw_perms_no_capacity_slack`). There is no first-touch cost and no query-latency
    /// dimension to trade, because no permutation is materialised any later than before. The
    /// LAZY half of sq-dzfzq — deferring a rarely-scanned permutation to its first use, which
    /// WOULD move cost into the query path — is deliberately NOT taken here; it is a separate,
    /// higher-risk change and is left to a follow-up.
    #[cfg(feature = "parallel")]
    fn build_raw_perms(mut triples: Vec<[Id; 3]>) -> [PermData; 6] {
        radix_sort_rows(&mut triples);
        triples.dedup();
        // [SONNET-4.6 sq-7d3dj.32.1] Eliminate dedup capacity slack: after dedup the Vec retains
        // its pre-dedup allocation. shrink_to_fit realigns len == capacity so heap_bytes() (which
        // counts capacity()) returns zero slack. The DERIVED perms are exact-sized by
        // construction (an ExactSize map-collect, then a same-length radix double-buffer).
        triples.shrink_to_fit();

        let mut perms: [PermData; 6] = std::array::from_fn(|_| PermData::default());
        perms[Perm::Spo as usize] = PermData::Owned(triples);
        for wave in DERIVE_PLAN {
            // Sources are materialised by an earlier wave, so the pairs inside a wave are
            // independent and derive concurrently.
            let built: Vec<(Perm, Vec<[Id; 3]>)> = wave
                .par_iter()
                .map(|&(src, dst)| (dst, Self::derive_perm(perms[src as usize].as_slice(), src, dst)))
                .collect();
            for (p, v) in built {
                perms[p as usize] = PermData::Owned(v);
            }
        }
        perms
    }

    /// Derives the `dst` permutation from the ALREADY-SORTED `src` permutation's rows: re-map
    /// the columns into `dst`'s layout, then stable-sort by `dst`'s leading column ALONE
    /// ([`radix_sort_rows_by_col0`]) — at most four byte passes over one column instead of the
    /// twelve a from-scratch [`radix_sort_rows`] costs, and no dedup pass (a column permutation
    /// is a bijection on rows, so the deduped SPO multiset stays deduped).
    ///
    /// WHY ONE COLUMN SUFFICES. `src` is in full lexicographic order, so inside any group of
    /// rows sharing a value of `dst`'s leading column the rows are still in `src` order. A
    /// STABLE sort leaves that intra-group order untouched. So the result is fully `dst`-sorted
    /// exactly when `src`'s order, restricted to such a group, already agrees with `dst`'s
    /// remaining two columns — which is precisely the invariant asserted below: deleting
    /// `dst`'s leading column from `src`'s column order leaves `dst`'s trailing columns, in
    /// order. Every pair in [`DERIVE_PLAN`] satisfies it, and
    /// `derive_plan_pairs_are_derivable` pins that for the plan as shipped.
    ///
    /// Output is BYTE-IDENTICAL to sorting the mapped rows from scratch (both are "the deduped
    /// triple set, permuted, fully sorted" — and a fully sorted deduped set is unique), which is
    /// what keeps scan/lookup/estimate, the delta-overlay and save/open untouched. Gated by
    /// `from_triples_perms_match_reference_sort` and `derived_perms_match_radix_all_build`.
    fn derive_perm(src_rows: &[[Id; 3]], src: Perm, dst: Perm) -> Vec<[Id; 3]> {
        let (so, dor) = (src.order(), dst.order());
        debug_assert!(
            so.iter().copied().filter(|&c| c != dor[0]).eq(dor[1..].iter().copied()),
            "{:?} is not derivable from {:?}: deleting the leading column does not leave the rest in order",
            dst,
            src
        );
        // Column j of a dst row is column `pick[j]` of a src row.
        let pick: [usize; 3] =
            std::array::from_fn(|j| so.iter().position(|&c| c == dor[j]).expect("a permutation covers every column"));
        // Pre-sized exactly from the known row count (the mapped iterator is ExactSize, so
        // `collect` reserves `src_rows.len()` up front — no grow tail, no capacity slack).
        let mut v: Vec<[Id; 3]> =
            src_rows.iter().map(|r| [r[pick[0]], r[pick[1]], r[pick[2]]]).collect();
        radix_sort_rows_by_col0(&mut v);
        v
    }

    /// No-threads build of the permutation indexes (wasm / no-rayon path). Deduplicates via
    /// the SPO ordering first (radix — no parallel sort to beat it here), then walks
    /// [`DERIVE_PLAN`] in order, reusing the deduped array for the SPO slot. [SONNET-4.6
    /// sq-7d3dj.32.1] shrink_to_fit after dedup so the SPO slot carries zero capacity slack
    /// (the derived perms are exact by construction). [SONNET-4.6 sq-dzfzq] each derived perm
    /// now costs one single-column sort instead of a full 12-digit one — on wasm, where there
    /// are no threads to hide the work behind, this is the whole saving.
    /// The wasm bundle byte count changes from the historical value — declared in
    /// bench/feature-off-declarations/ and bench/perf-baseline.json feature_off_exact.
    #[cfg(not(feature = "parallel"))]
    fn build_raw_perms(mut triples: Vec<[Id; 3]>) -> [PermData; 6] {
        radix_sort_rows(&mut triples);
        triples.dedup();
        // [SONNET-4.6 sq-7d3dj.32.1] Release the pre-dedup capacity so the SPO slot is
        // exact-sized.  heap_bytes() counts capacity(); without this call it would count
        // the full pre-dedup allocation even after duplicates are removed.
        triples.shrink_to_fit();

        // Place each permutation at its canonical slot; the rest stay empty. Every non-SPO
        // BUILT permutation is DERIVED from an already-materialised one (sq-dzfzq) — with no
        // threads there is nothing to overlap, so the waves just run in order and the saving is
        // pure work: one full 12-digit sort plus one single-column sort per derived perm,
        // instead of a full sort (and a dedup) each.
        let mut perms: [PermData; 6] = std::array::from_fn(|_| PermData::default());
        perms[Perm::Spo as usize] = PermData::Owned(triples);
        for &(src, dst) in DERIVE_PLAN.iter().copied().flatten() {
            perms[dst as usize] = PermData::Owned(Self::derive_perm(perms[src as usize].as_slice(), src, dst));
        }
        perms
    }

    /// Persists the permutation indexes to `dir` (one raw little-endian `[u32;3]` file
    /// per permutation) so they can be memory-mapped later via [`open`](Self::open) —
    /// the on-disk side of out-of-core querying.
    #[cfg(feature = "mmap")]
    pub fn save(&self, dir: &std::path::Path) -> std::io::Result<()> {
        self.save_with(dir, false)
    }

    /// Like [`save`](Self::save) but writes each permutation BLOCK-COMPRESSED (the
    /// delta+varint format of [`crate::compress`], ~3-5x smaller on disk). The files are
    /// auto-detected by [`open`](Self::open) via [`crate::compress::FILE_MAGIC`], so old
    /// raw directories keep working and the two formats can be mixed.
    #[cfg(feature = "mmap")]
    pub fn save_compressed(&self, dir: &std::path::Path) -> std::io::Result<()> {
        self.save_with(dir, true)
    }

    #[cfg(feature = "mmap")]
    fn save_with(&self, dir: &std::path::Path, compressed: bool) -> std::io::Result<()> {
        std::fs::create_dir_all(dir)?;
        for (i, p) in self.perms.iter().enumerate() {
            // Raw modes borrow zero-copy; a compressed perm is decoded back to raw rows so
            // `save` is total (e.g. a `load_str_compressed` graph can still be persisted).
            let rows: std::borrow::Cow<[[Id; 3]]> = match p {
                PermData::Compressed(c) => std::borrow::Cow::Owned(c.decode_all()),
                _ => std::borrow::Cow::Borrowed(p.as_slice()),
            };
            // A pending delta-overlay is FOLDED into every BUILT permutation on save, so
            // the persisted base always reflects the full current state (unbuilt perms
            // stay empty). The in-memory overlay is untouched (`save` takes `&self`).
            let rows: std::borrow::Cow<[[Id; 3]]> = match &self.overlay {
                Some(ov) if BUILT.contains(&Perm::ALL[i]) => {
                    std::borrow::Cow::Owned(ov.merge(&rows, Perm::ALL[i], [Id::MIN; 3], [Id::MAX; 3]))
                }
                _ => rows,
            };
            let path = dir.join(format!("perm{i}.bin"));
            if compressed && !rows.is_empty() {
                // Unbuilt (empty) permutations stay raw-empty so `open` skips them by size.
                // [FABLE-5] sq-7d3dj.32.2.7: `encode_emit` honours the emit-format config gate —
                // `SPQCPRM1` by default, `SPQCPRM2` only when a `spqcprm2` build has opted in.
                let mut w = std::io::BufWriter::new(std::fs::File::create(path)?);
                crate::compress::CompressedPerm::encode_emit(&rows).write_to(&mut w)?;
                std::io::Write::flush(&mut w)?;
            } else {
                // SAFETY: reinterpret the contiguous [u32;3] rows as bytes for writing.
                let bytes = unsafe { std::slice::from_raw_parts(rows.as_ptr().cast::<u8>(), std::mem::size_of_val(rows.as_ref())) };
                std::fs::write(path, bytes)?;
            }
        }
        self.save_pred_stats(dir)
    }

    /// Persists per-predicate statistics in ascending predicate-ID order.
    ///
    /// This lets `open` avoid re-scanning the POS/PSO indexes
    /// (a ~2-permutation read — the dominant out-of-core open cost + resident RSS once the
    /// dict is mmap'd). Small: a handful of fields per distinct predicate.
    ///
    /// # Errors
    /// Returns an error if creating, writing, or flushing the statistics file fails.
    #[cfg(feature = "mmap")]
    pub fn save_pred_stats(&self, dir: &std::path::Path) -> std::io::Result<()> {
        use std::io::Write;
        let mut w = std::io::BufWriter::new(std::fs::File::create(dir.join("predstats.bin"))?);
        w.write_all(&(self.pred_stats.len() as u64).to_le_bytes())?;
        // [GPT-6] Hash-map iteration can change after loading/reserving identical stats.
        // Canonicalize the shared writer so external builds and re-saves agree bytewise.
        let mut entries: Vec<_> = self.pred_stats.iter().collect();
        entries.sort_unstable_by_key(|&(&p, _)| p);
        for (&p, s) in entries {
            w.write_all(&p.to_le_bytes())?;
            w.write_all(&(s.count as u64).to_le_bytes())?;
            w.write_all(&(s.ndv_subj as u64).to_le_bytes())?;
            w.write_all(&(s.ndv_obj as u64).to_le_bytes())?;
        }
        w.flush()
    }

    /// Loads persisted per-predicate stats (written by [`save_pred_stats`]); `None` if the
    /// file is absent (an older saved dir) so the caller falls back to recomputing.
    #[cfg(feature = "mmap")]
    fn load_pred_stats(dir: &std::path::Path) -> Option<FxHashMap<Id, PredStat>> {
        use std::io::Read;
        let mut r = std::io::BufReader::new(std::fs::File::open(dir.join("predstats.bin")).ok()?);
        fn rd8(r: &mut impl Read) -> Option<u64> {
            let mut b = [0u8; 8];
            r.read_exact(&mut b).ok()?;
            Some(u64::from_le_bytes(b))
        }
        // The predicate id is written as a little-endian `Id` (u32, 4 bytes) by
        // `save_pred_stats` — this loader used to read 8 bytes for it, mis-framing every
        // record, so the load ALWAYS failed and `open` silently fell back to recomputing
        // the stats, paging in the whole POS+PSO indexes (~24 B/triple of resident memory
        // and most of the out-of-core open time). Measured in research/memory-tiering.md.
        fn rd_id(r: &mut impl Read) -> Option<Id> {
            let mut b = [0u8; std::mem::size_of::<Id>()];
            r.read_exact(&mut b).ok()?;
            Some(Id::from_le_bytes(b))
        }
        // [OPUS-4.8] sq-f5jh: `predstats.bin` is an UNTRUSTED on-disk file (trust boundary
        // B5). `n` is a u64 count read straight from it, and `reserve(n)` was unbounded — a
        // single flipped count byte could ask `FxHashMap` to pre-allocate billions of slots
        // (~17 B each) and ABORT the process (uncatchable OOM DoS; under llvm-cov's added
        // memory pressure this is the residual rc=101 / coverage-undercount trigger). Each
        // record on disk is `size_of::<Id>() + 24` bytes (id + three u64s), so the file
        // length is a hard upper bound on the real record count: clamp the reservation to it
        // (the per-record `read_exact`s below still error cleanly via `?`/`None` if the file
        // actually ends early). We never reserve for more records than can possibly fit.
        let n = rd8(&mut r)? as usize;
        const PREDSTAT_REC_BYTES: usize = std::mem::size_of::<Id>() + 24; // id + count + ndv_subj + ndv_obj
        let file_len = std::fs::metadata(dir.join("predstats.bin")).ok()?.len() as usize;
        let max_records = file_len.saturating_sub(8) / PREDSTAT_REC_BYTES; // 8-byte header
        let mut stats = FxHashMap::default();
        stats.reserve(n.min(max_records));
        for _ in 0..n {
            let p = rd_id(&mut r)?;
            let count = rd8(&mut r)? as usize;
            let ndv_subj = rd8(&mut r)? as usize;
            let ndv_obj = rd8(&mut r)? as usize;
            stats.insert(p, PredStat { count, ndv_subj, ndv_obj });
        }
        Some(stats)
    }

    /// Opens a store whose permutations are MEMORY-MAPPED from `dir` (written by
    /// [`save`](Self::save)). The 6 (or 3, compact) index files stay on disk; the OS
    /// pages in only the ranges a query touches, so datasets larger than RAM are
    /// queryable. Per-predicate stats are recomputed from the mapped POS/PSO indexes.
    #[cfg(feature = "mmap")]
    pub fn open(dir: &std::path::Path) -> std::io::Result<Self> {
        let mut perms: [PermData; 6] = std::array::from_fn(|_| PermData::default());
        for (i, slot) in perms.iter_mut().enumerate() {
            let path = dir.join(format!("perm{i}.bin"));
            let file = std::fs::File::open(&path)?;
            if file.metadata()?.len() == 0 {
                continue; // an empty (unbuilt, e.g. compact-index) permutation
            }
            // SAFETY: the file is owned by this store for its lifetime and is not mutated.
            let map = unsafe { memmap2::Mmap::map(&file)? };
            // FORMAT AUTO-DETECTION: a block-compressed file (written by `save_compressed`)
            // starts with FILE_MAGIC (`SPQCPRM1`) or, for a `spqcprm2`-emitting build,
            // FILE_MAGIC_V2 (`SPQCPRM2`); anything else is the original raw [u32;3] format.
            // [FABLE-5] sq-7d3dj.32.2.7: both magics route to `from_mmap`, which re-checks the
            // magic and picks the V1/V2 decode reader — a V1 file decodes byte-identically
            // forever. Compressed perms are served lazily — block-wise decode off the mapped file.
            *slot = if map.len() >= 8
                && (map[..8] == crate::compress::FILE_MAGIC || map[..8] == crate::compress::FILE_MAGIC_V2)
            {
                PermData::Compressed(crate::compress::CompressedPerm::from_mmap(map)?)
            } else {
                PermData::Mapped(map)
            };
        }
        // Use the persisted stats if present (no POS/PSO re-scan — keeps open fast and the
        // resident set small); else recompute (backward compatible with older saved dirs).
        let pred_stats = Self::load_pred_stats(dir).unwrap_or_else(|| Self::compute_pred_stats(&perms));
        Ok(TripleStore { perms: std::sync::Arc::new(perms), pred_stats: std::sync::Arc::new(pred_stats), overlay: None })
    }

    /// Per-predicate stats: count + distinct objects from POS (always built), and
    /// distinct subjects from PSO when it is built (the full six-permutation index),
    /// else approximated by the count (under `compact-index`, where PSO is absent —
    /// the planner then treats subjects as non-selective, which is safe for ordering).
    fn compute_pred_stats(perms: &[PermData; 6]) -> FxHashMap<Id, PredStat> {
        // Full-range `rows_in`: raw modes borrow the whole slice (zero-copy, as before);
        // a compressed perm (an opened compressed dir missing predstats.bin) is decoded.
        let pos = perms[Perm::Pos as usize].rows_in([Id::MIN; 3], [Id::MAX; 3]); // [P, O, S]
        let pso = perms[Perm::Pso as usize].rows_in([Id::MIN; 3], [Id::MAX; 3]); // [P, S, O], empty under compact-index
        let mut stats: FxHashMap<Id, PredStat> = FxHashMap::default();
        // POS: count + distinct O per P. ndv_subj defaults to count (refined below).
        let mut i = 0;
        while i < pos.len() {
            let p = pos[i][0];
            let (mut count, mut ndv_o, mut last_o) = (0usize, 0usize, None);
            while i < pos.len() && pos[i][0] == p {
                count += 1;
                if last_o != Some(pos[i][1]) {
                    ndv_o += 1;
                    last_o = Some(pos[i][1]);
                }
                i += 1;
            }
            stats.insert(p, PredStat { count, ndv_subj: count, ndv_obj: ndv_o });
        }
        // PSO (when built): exact distinct S per P.
        let mut i = 0;
        while i < pso.len() {
            let p = pso[i][0];
            let (mut ndv_s, mut last_s) = (0usize, None);
            while i < pso.len() && pso[i][0] == p {
                if last_s != Some(pso[i][1]) {
                    ndv_s += 1;
                    last_s = Some(pso[i][1]);
                }
                i += 1;
            }
            stats.entry(p).or_default().ndv_subj = ndv_s;
        }
        stats
    }

    /// Stats for a predicate id (for the cost-based planner), if present.
    pub fn pred_stat(&self, predicate: Id) -> Option<PredStat> {
        self.pred_stats.get(&predicate).copied()
    }

    pub fn len(&self) -> usize {
        let base = self.perms[0].len();
        match &self.overlay {
            Some(ov) => base + ov.added.len() - ov.deleted.len(),
            None => base,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Whether a delta-overlay of pending updates exists (i.e. updates were applied
    /// since the base was built / last compacted).
    pub fn has_overlay(&self) -> bool {
        self.overlay.is_some()
    }

    /// [OPUS-4.8] (sq-5lf) Strong-reference count of the `Arc`-shared base permutation
    /// indexes — i.e. how many stores currently SHARE this exact base storage. 1 for a
    /// freshly built / just-compacted store; bumps by one for each live
    /// [`fork`](Self::fork) / [`Graph::snapshot`](crate::Graph::snapshot) of it. Used to
    /// PROVE structural sharing in tests (a cheap snapshot bumps this count rather than
    /// duplicating the index memory). Two stores share a base iff this is > 1 and they
    /// were derived from the same lineage.
    pub fn base_strong_count(&self) -> usize {
        std::sync::Arc::strong_count(&self.perms)
    }

    /// Whether the store (base merged with any overlay) contains the canonical triple.
    pub fn contains(&self, t: [Id; 3]) -> bool {
        match &self.overlay {
            Some(ov) => {
                !ov.deleted.contains(&t)
                    && (ov.added.binary_search(&t).is_ok() || self.base_contains(t))
            }
            None => self.base_contains(t),
        }
    }

    /// Whether the immutable BASE (ignoring the overlay) contains the canonical triple —
    /// one binary search of the SPO permutation (always built, in every index set).
    #[inline]
    fn base_contains(&self, t: [Id; 3]) -> bool {
        self.perms[Perm::Spo as usize].count_in(t, t) > 0
    }

    /// Applies an update batch as a DELTA-OVERLAY: `deletes` first, then `inserts`
    /// (SPARQL's DELETE/INSERT order), each O(log n + batch · overlay) — instead of the
    /// O(n) rebuild. Set semantics: re-inserting a present triple and deleting an absent
    /// one are no-ops; a delete of a pending insertion simply retracts it. When the
    /// overlay nets out to nothing it is dropped entirely, so an untouched (or fully
    /// reverted) store scans with zero overhead.
    pub fn apply_delta(&mut self, inserts: &[[Id; 3]], deletes: &[[Id; 3]]) {
        if inserts.is_empty() && deletes.is_empty() {
            return;
        }
        let mut ov = self.overlay.take().unwrap_or_default();
        // [GPT-6 Astra] Added projections retain the existing conservative reset.
        // Preserve deletion projections across inserts/no-ops: only actual tombstone
        // changes require another sort. No overlay read occurs before publication.
        ov.invalidate_added();
        let mut deleted_changed = false;
        for t in deletes {
            if let Ok(i) = ov.added.binary_search(t) {
                ov.added.remove(i); // retract a pending insertion
            } else if self.base_contains(*t) {
                deleted_changed |= ov.deleted.insert(*t);
            }
        }
        for t in inserts {
            if ov.deleted.remove(t) {
                deleted_changed = true;
                continue; // re-insert of a deleted base triple: just undelete
            }
            if self.base_contains(*t) {
                continue; // already present in the base
            }
            if let Err(i) = ov.added.binary_search(t) {
                ov.added.insert(i, *t);
            }
        }
        if deleted_changed {
            ov.invalidate_deleted();
        }
        self.overlay = if ov.is_empty() { None } else { Some(ov) };
    }

    /// Decodes every block-compressed permutation into its raw in-RAM form, so later
    /// scans are pure binary-search slice borrows (zero decode cost) — the LOAD-TIME
    /// DECOMPRESSION mode for an opened compressed directory: pay one full decode up
    /// front, query at exactly raw-store speed. Raw/mapped permutations are untouched.
    ///
    /// Runs on freshly built/opened stores (load-time), which are never yet forked;
    /// on a structurally SHARED store (post-[`fork`](Self::fork)) it is a no-op —
    /// decompression is an optimisation, never a correctness requirement.
    pub fn decompress_to_ram(&mut self) {
        let Some(perms) = std::sync::Arc::get_mut(&mut self.perms) else {
            return; // shared with a fork: leave the (immutable) base untouched
        };
        for slot in perms {
            if let PermData::Compressed(c) = slot {
                *slot = PermData::Owned(c.decode_all());
            }
        }
    }

    /// A structural FORK of this store: the immutable base permutation indexes and
    /// planner stats are SHARED (Arc bumps, O(1)); the pending delta-overlay is
    /// carried by value (O(overlay), bounded by the compaction policy). The fork and
    /// the original then evolve independently through [`apply_delta`](Self::apply_delta)
    /// — neither ever mutates the shared base, so existing readers are unaffected.
    pub fn fork(&self) -> TripleStore {
        TripleStore {
            perms: std::sync::Arc::clone(&self.perms),
            pred_stats: std::sync::Arc::clone(&self.pred_stats),
            overlay: self.overlay.clone(),
        }
    }

    /// Number of pending overlay entries (insertions + deletions) — the input to a
    /// compaction threshold policy (a fork costs O(this); folding it costs O(n)).
    pub fn overlay_len(&self) -> usize {
        self.overlay.as_ref().map_or(0, |ov| ov.added.len() + ov.deleted.len())
    }

    /// Heap footprint of the permutation indexes in bytes (for benchmarking). Memory-
    /// mapped permutations contribute 0 — their resident pages are OS page cache.
    pub fn heap_bytes(&self) -> usize {
        self.perms.iter().map(PermData::heap_bytes).sum::<usize>()
            + self.overlay.as_ref().map_or(0, |ov| ov.heap_bytes())
    }

    /// Chooses the permutation whose sort order places all bound pattern
    /// positions as a contiguous prefix, so the matches form one range. Returns
    /// the permutation and the number of leading bound columns.
    fn choose(pattern: &Pattern) -> (Perm, usize) {
        // Prefer an order where every bound position precedes every unbound one.
        let bound = |i: usize| pattern[i].is_some();
        for &perm in BUILT {
            let order = perm.order();
            // count leading bound columns
            let mut lead = 0;
            while lead < 3 && bound(order[lead]) {
                lead += 1;
            }
            // valid if all bound positions are within the leading prefix
            let total_bound = (0..3).filter(|&i| bound(i)).count();
            if lead == total_bound {
                return (perm, lead);
            }
        }
        (Perm::Spo, 0)
    }

    /// Like [`choose`], but among the permutations whose sort order places every
    /// bound position as a prefix, prefers one whose first *unbound* column is
    /// `sort_col` (a position 0..3 into a canonical triple). This makes the scan
    /// output sorted by that column, enabling a merge join on it.
    fn choose_sorted(pattern: &Pattern, sort_col: usize) -> (Perm, usize) {
        let bound = |i: usize| pattern[i].is_some();
        let total_bound = (0..3).filter(|&i| bound(i)).count();
        // Prefer: bound positions form the leading prefix AND column `sort_col`
        // is the first column after the prefix.
        for &perm in BUILT {
            let order = perm.order();
            let mut lead = 0;
            while lead < 3 && bound(order[lead]) {
                lead += 1;
            }
            if lead == total_bound && lead < 3 && order[lead] == sort_col {
                return (perm, lead);
            }
        }
        Self::choose(pattern)
    }

    /// Returns the contiguous slice of rows (in `perm` order) matching the bound
    /// prefix of the pattern, together with the chosen permutation.
    pub fn scan(&self, pattern: &Pattern) -> Scan<'_> {
        let (perm, lead) = Self::choose(pattern);
        self.scan_with(pattern, perm, lead)
    }

    /// Scans choosing a permutation whose output is sorted by canonical column
    /// `sort_col` (when possible), for merge joins.
    pub fn scan_sorted(&self, pattern: &Pattern, sort_col: usize) -> Scan<'_> {
        let (perm, lead) = Self::choose_sorted(pattern, sort_col);
        self.scan_with(pattern, perm, lead)
    }

    /// [OPUS-4.8] (sq-7d3dj.30.4) Scans a SPECIFIC permutation `perm`, for callers that
    /// need a particular SECONDARY column order rather than just a primary sort column
    /// (e.g. the DISTINCT loose skip-scan wants the layout `[..bound.., P, J, ..]` so each
    /// `P`-block is `J`-sorted). Returns `None` when `perm` is not built (e.g. the compact
    /// index) or when the pattern's bound positions do not form a leading prefix of `perm`
    /// (so a contiguous range scan is impossible). The returned rows are identical to what
    /// `scan`/`scan_sorted` would yield had they chosen `perm` — only the choice differs.
    pub fn scan_perm(&self, pattern: &Pattern, perm: Perm) -> Option<Scan<'_>> {
        if !BUILT.contains(&perm) {
            return None;
        }
        let order = perm.order();
        let bound = |i: usize| pattern[i].is_some();
        let total_bound = (0..3).filter(|&i| bound(i)).count();
        let mut lead = 0;
        while lead < 3 && bound(order[lead]) {
            lead += 1;
        }
        // Every bound position must be within the leading prefix, else this permutation
        // cannot answer the pattern with one contiguous range.
        if lead != total_bound {
            return None;
        }
        Some(self.scan_with(pattern, perm, lead))
    }

    /// The inclusive [lo, hi] key bounds for a pattern's bound prefix in `perm` order.
    #[inline]
    fn bounds(pattern: &Pattern, perm: Perm, lead: usize) -> ([Id; 3], [Id; 3]) {
        let order = perm.order();
        let mut lo = [Id::MIN; 3];
        let mut hi = [Id::MAX; 3];
        for k in 0..lead {
            let v = pattern[order[k]].unwrap();
            lo[k] = v;
            hi[k] = v;
        }
        (lo, hi)
    }

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

    /// Estimated number of matches for a pattern (the range length) — the cardinality
    /// estimate used by the greedy planner. Cheap for every storage mode: raw modes
    /// subtract binary-search bounds; the compressed mode counts via the block directory
    /// decoding at most two boundary blocks (never the whole range).
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
}


```

## All relevant Graph lifecycle/mutator function bodies

Path: source/graph-lifecycle-complete-functions.rs; SHA256 9322b5285123e975398e11cc9935091176fb2ce4c47116be90eba633013510df

```rust
// Exact crates/sparq-core/src/lib.rs:1384-1386
    pub fn from_parts(dict: Dict, triples: Vec<[Id; 3]>) -> Graph {
        Self::build(dict, triples)
    }


// Exact crates/sparq-core/src/lib.rs:1891-1988
    pub fn open(dir: &std::path::Path) -> std::io::Result<Graph> {
        // [OPUS-4.8] (review 1593) Finish or roll back any compaction directory swap that a
        // crash interrupted, so `dir` is always present (never lost in the rename window).
        recover_compaction(dir)?;
        // [OPUS-4.8] (sq-glw2) Finish or roll back any named-graph DROP sub-tree swap a crash
        // interrupted, so `named/` is always consistent with the manifest before `open_named`.
        recover_named_drop(dir)?;
        let store = TripleStore::open(dir)?;
        // [OPUS-4.8] (review 1325, sub-finding 2) Prefer the mmap dictionary format
        // (`dict-meta.bin`, written by `save_mmap`); fall back to the LEGACY single-file
        // `dict.bin` (written by the older `Dict::save`) so graph directories saved before
        // the mmap dictionary format still open. The legacy dict loads fully into RAM (no
        // mmap), which is the only difference — every term still round-trips.
        let dict = if dir.join("dict-meta.bin").exists() {
            Dict::open_mmap(dir)?
        } else {
            let legacy = dir.join("dict.bin");
            if legacy.exists() {
                Dict::open(&legacy)?
            } else {
                // Neither format present — surface the original mmap error (NotFound on
                // dict-meta.bin), matching the previous behaviour for a corrupt directory.
                Dict::open_mmap(dir)?
            }
        };
        let np = dir.join("numerics.bin");
        let numerics = match std::fs::File::open(&np) {
            Ok(f) if f.metadata()?.len() as usize == dict.len() * std::mem::size_of::<f64>() => {
                // SAFETY: the file is owned by this graph for its lifetime and not mutated.
                NumData::Mapped(unsafe { memmap2::Mmap::map(&f)? }, rustc_hash::FxHashMap::default())
            }
            _ => NumData::Owned(numerics_of(&dict)),
        };
        let tp = dir.join("temporals.bin");
        let temporals = match std::fs::File::open(&tp) {
            Ok(f) if f.metadata()?.len() as usize == dict.len() * 9 => {
                // SAFETY: the file is owned by this graph for its lifetime and not mutated.
                TempData::Mapped(unsafe { memmap2::Mmap::map(&f)? }, rustc_hash::FxHashMap::default())
            }
            // Absent or stale (a graph saved before this cache existed): recompute —
            // backward compatible, like the numerics cache.
            _ => TempData::Owned(temporals_of(&dict)),
        };
        // [OPUS-4.8] (sq-3ui0, gh-45) Restore the named graphs (each opened memory-mapped
        // with its own per-graph WAL) so a save→open round-trip is lossless for the whole
        // dataset. Empty for a default-graph-only / pre-named-graph directory.
        let named = Self::open_named(dir)?;
        let mut g = Graph {
            dict,
            store,
            numerics,
            temporals,
            high_precision_decimal: std::sync::atomic::AtomicU8::new(0),
            named,
            graph_prefix_index: std::sync::Mutex::new(None),
            wal: None,
            txn: None,
        };
        // Replay any write-ahead log into the delta-overlay (recovery after a crash or a
        // plain not-yet-compacted close), stopping cleanly at the first torn record and
        // truncating the log there — then keep the log open for further appends.
        for (insert, t) in Wal::replay(dir)? {
            if insert {
                g.apply_delta_mem(&[t], &[]);
            } else {
                g.apply_delta_mem(&[], &[t]);
            }
        }
        g.wal = Some(Wal::open(dir)?);
        // [OPUS-4.8] (sq-ycle) AFTER the per-graph WALs are replayed (above + per named graph in
        // `open_named`), redo any committed parent-level transaction frame, idempotently. A
        // multi-op UPDATE body's resolved per-slot quad-delta is one atomic `txn.log` frame; if a
        // crash interrupted materialisation after that single fsync, the per-graph WALs hold only a
        // PREFIX of the body — replaying the frame heals the desync. Re-applying records the WALs
        // already materialised is a no-op: `apply_delta` replays with set semantics (re-inserting a
        // present triple / deleting an absent one changes nothing) and `ensure_named` is
        // find-or-create.
        //
        // [OPUS-4.8] (Copilot #135) DURABILITY ORDER (load-bearing): the records MUST be
        // MATERIALISED INTO THE DURABLE PER-GRAPH WAL (`redo_txn_record` -> `apply_delta`, which
        // appends + fsyncs) BEFORE the `txn.log` is truncated. The earlier code applied them with
        // `apply_delta_mem` (IN-MEMORY only) and then truncated the journal, so a record that lived
        // ONLY in `txn.log` — the precise crash window the journal exists for — was applied to this
        // process's memory but written nowhere durable, then ERASED by the truncation, so it did NOT
        // survive the NEXT restart. Re-logging into the WAL first (then truncate) makes the recovered
        // state durable across any number of restarts; a re-crash between WAL-redo and truncation
        // simply redoes the (idempotent) frame again on the next open.
        let txn_records = TxnJournal::replay(dir)?;
        for (insert, slot, t) in txn_records {
            g.redo_txn_record(insert, slot, t).map_err(std::io::Error::other)?;
        }
        // Every committed record is now durably in a per-graph WAL (fsync'd by `apply_delta`); only
        // NOW is it safe to truncate the journal that was the sole durable home of those records.
        let mut journal = TxnJournal::open(dir)?;
        journal.truncate()?;
        g.txn = Some(journal);
        Ok(g)
    }


// Exact crates/sparq-core/src/lib.rs:2885-2903
    pub fn fork(&self) -> Graph {
        Graph {
            dict: self.dict.fork(),
            store: self.store.fork(),
            numerics: self.numerics.fork(),
            temporals: self.temporals.fork(),
            // sq-lr2ii: the fork shares the same values; recompute the guard lazily.
            high_precision_decimal: std::sync::atomic::AtomicU8::new(0),
            named: self.named.iter().map(|(name, g)| (name.clone(), g.fork())).collect(),
            // A fork is a fresh logical copy; rebuild the prefix index lazily on first use.
            graph_prefix_index: std::sync::Mutex::new(None),
            #[cfg(feature = "mmap")]
            wal: None,
            // [OPUS-4.8] (sq-ycle) A fork/snapshot is a logically-independent in-memory copy with
            // NO directory association — no WAL and no redo journal (like `wal: None`).
            #[cfg(feature = "mmap")]
            txn: None,
        }
    }


// Exact crates/sparq-core/src/lib.rs:2924-2926
    pub fn snapshot(&self) -> GraphSnapshot {
        GraphSnapshot { graph: self.fork() }
    }


// Exact crates/sparq-core/src/lib.rs:2947-2957
    pub fn apply_delta(&mut self, inserts: &[[Term; 3]], deletes: &[[Term; 3]]) -> Result<(), String> {
        if inserts.is_empty() && deletes.is_empty() {
            return Ok(());
        }
        #[cfg(feature = "mmap")]
        if let Some(w) = &mut self.wal {
            w.append_batch(inserts, deletes).map_err(|e| format!("WAL append failed: {e}"))?;
        }
        self.apply_delta_mem(inserts, deletes);
        Ok(())
    }


// Exact crates/sparq-core/src/lib.rs:2973-2981
    pub fn insert_triple(
        &mut self,
        subject: impl Into<Term>,
        predicate: impl Into<Term>,
        object: impl Into<Term>,
    ) -> Result<(), String> {
        let triple = [subject.into(), predicate.into(), object.into()];
        self.apply_delta(&[triple], &[])
    }


// Exact crates/sparq-core/src/lib.rs:2990-2998
    pub fn remove_triple(
        &mut self,
        subject: impl Into<Term>,
        predicate: impl Into<Term>,
        object: impl Into<Term>,
    ) -> Result<(), String> {
        let triple = [subject.into(), predicate.into(), object.into()];
        self.apply_delta(&[], &[triple])
    }


// Exact crates/sparq-core/src/lib.rs:3038-3061
    pub fn clear_default_durable(&mut self) -> Result<(), String> {
        #[cfg(feature = "mmap")]
        if self.wal.is_some() {
            // Retract every current default-graph triple as a WAL-logged delete batch.
            let triples: Vec<[Term; 3]> = {
                let scan = self.store.scan(&[None, None, None]);
                scan.rows
                    .iter()
                    .map(|r| {
                        let spo = scan.to_spo(r);
                        [self.dict.term(spo[0]), self.dict.term(spo[1]), self.dict.term(spo[2])]
                    })
                    .collect()
            };
            if !triples.is_empty() {
                self.apply_delta(&[], &triples)?;
            }
            return Ok(());
        }
        // In-memory graph: no WAL/dir to preserve — clear the store content in place. The
        // dictionary is intentionally kept (ids stay stable; the empty store references none).
        self.store = TripleStore::from_triples(Vec::new());
        Ok(())
    }


// Exact crates/sparq-core/src/lib.rs:3076-3084
    pub fn clear_named_durable(&mut self, name: &Term) -> Result<bool, String> {
        match self.named.iter().position(|(n, _)| n == name) {
            Some(i) => {
                self.named[i].1.clear_default_durable()?;
                Ok(true)
            }
            None => Ok(false),
        }
    }


// Exact crates/sparq-core/src/lib.rs:3372-3397
    fn apply_delta_mem(&mut self, inserts: &[[Term; 3]], deletes: &[[Term; 3]]) {
        // A delete only matters if every term resolves — otherwise the triple cannot be
        // present, and deleting must NOT intern the (absent) terms.
        let del_ids: Vec<[Id; 3]> = deletes
            .iter()
            .filter_map(|[s, p, o]| Some([self.id_of(s)?, self.id_of(p)?, self.id_of(o)?]))
            .collect();
        let old_len = self.dict.len();
        let ins_ids: Vec<[Id; 3]> = inserts
            .iter()
            .map(|[s, p, o]| [self.dict.intern(s), self.dict.intern(p), self.dict.intern(o)])
            .collect();
        // Keep the numeric- and temporal-filter caches covering the grown dictionary.
        self.numerics.extend_for(&self.dict, old_len);
        self.temporals.extend_for(&self.dict, old_len);
        // sq-lr2ii: an inserted term may be an f64-inexact decimal. If the sargable-safety
        // guard was memoised as "no such decimal" (1), reset it to recompute over the grown
        // dictionary; a "found" (2) verdict is monotonic (terms are never removed) and stays.
        let _ = self.high_precision_decimal.compare_exchange(
            1,
            0,
            std::sync::atomic::Ordering::Relaxed,
            std::sync::atomic::Ordering::Relaxed,
        );
        self.store.apply_delta(&ins_ids, &del_ids);
    }


// Exact crates/sparq-core/src/lib.rs:2006-2033
    fn redo_txn_record(&mut self, insert: bool, slot: Option<Term>, t: [Term; 3]) -> Result<(), String> {
        match slot {
            None => {
                if insert {
                    self.apply_delta(&[t], &[])?;
                } else {
                    self.apply_delta(&[], &[t])?;
                }
            }
            Some(name) => {
                // An insert into an absent named graph must materialise the graph (find-or-create);
                // a delete from an absent one is a no-op (nothing to retract).
                let idx = if insert {
                    Some(self.ensure_named(&name)?)
                } else {
                    self.named.iter().position(|(n, _)| *n == name)
                };
                if let Some(i) = idx {
                    if insert {
                        self.named[i].1.apply_delta(&[t], &[])?;
                    } else {
                        self.named[i].1.apply_delta(&[], &[t])?;
                    }
                }
            }
        }
        Ok(())
    }


// Exact crates/sparq-core/src/lib.rs:3406-3443
    pub fn compact(&mut self) -> Result<(), String> {
        #[cfg(feature = "mmap")]
        let dir = self.wal.as_ref().map(|w| w.dir.clone());
        // Fold the structural-fork layers first (ids unchanged throughout): named
        // graphs recursively, then this graph's dictionary extension into a fresh
        // frozen base (so the NEXT fork is O(1) again) and the forked caches flat.
        // No-ops on a never-forked graph.
        for (_, g) in &mut self.named {
            g.compact()?;
        }
        if self.dict.is_forked() {
            self.dict = self.dict.compacted();
            let n = self.dict.len();
            let numerics = std::mem::replace(&mut self.numerics, NumData::Sparse(rustc_hash::FxHashMap::default()));
            self.numerics = numerics.fold(n);
            let temporals = std::mem::replace(&mut self.temporals, TempData::Sparse(rustc_hash::FxHashMap::default()));
            self.temporals = temporals.fold(n);
        }
        if self.store.has_overlay() {
            let triples: Vec<[Id; 3]> = {
                let scan = self.store.scan(&[None, None, None]);
                scan.rows.iter().map(|r| scan.to_spo(r)).collect()
            };
            self.store = TripleStore::from_triples(triples);
        } else {
            // Nothing pending: for a directory-backed graph just discard the (no-op) log.
            #[cfg(feature = "mmap")]
            if let Some(w) = &mut self.wal {
                w.truncate().map_err(|e| e.to_string())?;
            }
            return Ok(());
        }
        #[cfg(feature = "mmap")]
        if let Some(dir) = dir {
            self.persist_swap(&dir)?;
        }
        Ok(())
    }


// Exact crates/sparq-core/src/lib.rs:3462-3471
    fn persist_swap(&mut self, dir: &std::path::Path) -> Result<(), String> {
        // Close THIS graph's WAL before the directory swap (its `wal.log` is about to be renamed
        // away with `dir`), then run the shared crash-safe swap, writing `self`'s CURRENT image as
        // the new base. Adopt the re-opened directory-backed graph in place. [OPUS-4.8] (sq-ft7u)
        self.wal = None;
        let reopened = swap_dir_to_new_base(dir, |new_dir| self.save(new_dir))
            .map_err(|e| e.to_string())?;
        *self = reopened;
        Ok(())
    }


// Exact crates/sparq-core/src/lib.rs:3548-3566
    pub fn vacuum(&mut self) -> Result<(), String> {
        #[cfg(feature = "mmap")]
        let dir = self.wal.as_ref().map(|w| w.dir.clone());
        // Re-intern the whole live dataset into a fresh graph with an EMPTY dictionary, so any
        // term orphaned by a prior delete/drop is dropped (it is never re-interned).
        let mut fresh = self.reintern_live()?;
        #[cfg(feature = "mmap")]
        if let Some(dir) = dir {
            // Close THIS graph's WAL before the directory swap (its `wal.log` is about to be
            // renamed away). `persist_swap` writes `fresh`'s image to `dir` and re-opens
            // `fresh` memory-mapped from the new base with a fresh WAL; adopt that re-opened,
            // directory-backed graph back into `self` so subsequent updates WAL-append to it.
            self.wal = None;
            fresh.persist_swap(&dir)?;
        }
        // Adopt the freshly re-interned (and, when persisted, re-opened) image in place.
        std::mem::swap(self, &mut fresh);
        Ok(())
    }


// Exact crates/sparq-core/src/lib.rs:3572-3595
    fn reintern_live(&self) -> Result<Graph, String> {
        // Dump the live default-graph triples as Terms (overlay already merged by `scan`).
        let live: Vec<[Term; 3]> = {
            let scan = self.store.scan(&[None, None, None]);
            scan.rows
                .iter()
                .map(|r| {
                    let spo = scan.to_spo(r);
                    [self.dict.term(spo[0]), self.dict.term(spo[1]), self.dict.term(spo[2])]
                })
                .collect()
        };
        let mut fresh = Graph::from_parts(Dict::new(), Vec::new());
        if !live.is_empty() {
            fresh.apply_delta(&live, &[])?;
        }
        // Named graphs: re-intern each recursively and attach it as an IN-MEMORY sub-graph (no
        // directory/WAL — the parent's `save` writes the whole `named/` sub-tree afresh).
        for (name, sub) in &self.named {
            let sub_fresh = sub.reintern_live()?;
            fresh.named.push((name.clone(), sub_fresh));
        }
        Ok(fresh)
    }


// Exact crates/sparq-core/src/lib.rs:3498-3503
    pub fn restore_into_durable(
        dir: &std::path::Path,
        fresh: Graph,
    ) -> std::io::Result<Graph> {
        swap_dir_to_new_base(dir, |new_dir| fresh.save(new_dir))
    }


// Exact crates/sparq-core/src/lib.rs:3645-3666
fn swap_dir_to_new_base<F>(dir: &std::path::Path, write_new_base: F) -> std::io::Result<Graph>
where
    F: FnOnce(&std::path::Path) -> std::io::Result<()>,
{
    let new_dir = dir.with_extension("compact-new");
    let old_dir = dir.with_extension("compact-old");
    std::fs::remove_dir_all(&new_dir).ok();
    std::fs::remove_dir_all(&old_dir).ok();
    // Build + sync the new base FIRST. A failure here leaves `dir` untouched (fail-closed) —
    // the two renames below have not started, so the old durable store is intact.
    write_new_base(&new_dir)?;
    fsync_dir(&new_dir)?;
    let parent = dir.parent().unwrap_or_else(|| std::path::Path::new("."));
    std::fs::rename(dir, &old_dir)?;
    fsync_dir(parent)?;
    std::fs::rename(&new_dir, dir)?;
    fsync_dir(parent)?;
    // Re-open memory-mapped from the new base (fresh, empty WAL); only then drop the old files.
    let reopened = Graph::open(dir)?;
    std::fs::remove_dir_all(&old_dir).ok();
    Ok(reopened)
}

// Exact crates/sparq-core/src/lib.rs:1679-1701
    fn build(dict: Dict, triples: Vec<[Id; 3]>) -> Graph {
        let store = TripleStore::from_triples(triples);
        let numerics = NumData::Owned(numerics_of(&dict));
        // Unlike the numerics cache (dense f64 = 8 B/term), the dense temporal cells
        // are 16 B/term — go sparse straight away when temporals are rare (usually:
        // none at all -> an empty map, zero memory), keeping the load-time memory
        // metric flat for non-temporal datasets. Temporal-heavy data stays dense.
        let temporals = TempData::Owned(temporals_of(&dict)).into_sparse_if_worthwhile();
        Graph {
            dict,
            store,
            numerics,
            temporals,
            high_precision_decimal: std::sync::atomic::AtomicU8::new(0),
            named: Vec::new(),
            graph_prefix_index: std::sync::Mutex::new(None),
            #[cfg(feature = "mmap")]
            wal: None,
            // [OPUS-4.8] (sq-ycle) In-memory graph: no parent-level redo journal.
            #[cfg(feature = "mmap")]
            txn: None,
        }
    }

// Exact crates/sparq-core/src/lib.rs:1716-1739
    pub fn into_compressed(self) -> Graph {
        let triples: Vec<[Id; 3]> = {
            let scan = self.store.scan(&[None, None, None]);
            scan.rows.iter().map(|r| scan.to_spo(r)).collect()
        };
        Graph {
            store: TripleStore::from_triples_compressed(triples),
            dict: self.dict.into_blob(),
            // The numeric cache is mostly (often entirely) NaN — keep only the real
            // numeric literals when sparse, freeing the dense f64-per-term Vec.
            numerics: self.numerics.into_sparse_if_worthwhile(),
            temporals: self.temporals.into_sparse_if_worthwhile(),
            // sq-lr2ii: re-encoding keeps the same values; recompute the guard lazily.
            high_precision_decimal: std::sync::atomic::AtomicU8::new(0),
            named: self.named,
            graph_prefix_index: std::sync::Mutex::new(None),
            #[cfg(feature = "mmap")]
            wal: self.wal,
            // [OPUS-4.8] (sq-ycle) Re-encoding keeps the directory association — carry the redo
            // journal handle with the WAL, so a compressed graph stays atomically durable.
            #[cfg(feature = "mmap")]
            txn: self.txn,
        }
    }

// Exact crates/sparq-core/src/lib.rs:1752-1764
    pub fn save(&self, dir: &std::path::Path) -> std::io::Result<()> {
        std::fs::create_dir_all(dir)?;
        self.store.save(dir)?; // folds any delta-overlay into the persisted permutations
        self.dict.save_mmap(dir)?; // includes appended (delta-overlay) terms
        // [OPUS-4.8] (sq-7ph8) STREAM the numeric/temporal caches block-by-block straight from
        // the in-RAM cache instead of materialising a whole-dictionary dense intermediate
        // (`dense_numerics`/`dense_temporals`) first — bounding the finalize RSS peak for a
        // SPARSE/FORKED cache (the common non-numeric/non-temporal case).
        let n = self.dict.len();
        stream_write_numerics(&dir.join("numerics.bin"), n, &self.numerics)?;
        stream_write_temporals(&dir.join("temporals.bin"), n, &self.temporals)?;
        self.save_named(dir, false)
    }

// Exact crates/sparq-core/src/lib.rs:1772-1782
    pub fn save_compressed(&self, dir: &std::path::Path) -> std::io::Result<()> {
        std::fs::create_dir_all(dir)?;
        self.store.save_compressed(dir)?; // folds any delta-overlay, like `save`
        self.dict.save_mmap(dir)?;
        // [OPUS-4.8] (sq-7ph8) Stream the caches block-by-block — see `save` for the rationale.
        let n = self.dict.len();
        stream_write_numerics(&dir.join("numerics.bin"), n, &self.numerics)?;
        stream_write_temporals(&dir.join("temporals.bin"), n, &self.temporals)?;
        // [OPUS-4.8] (sq-3ui0) Named graphs are persisted block-compressed too.
        self.save_named(dir, true)
    }

// Exact crates/sparq-core/src/lib.rs:1796-1821
    fn save_named(&self, dir: &std::path::Path, compressed: bool) -> std::io::Result<()> {
        let named_dir = dir.join(NAMED_SUBDIR);
        let manifest = dir.join(NAMED_MANIFEST);
        if self.named.is_empty() {
            // Default-graph-only: ensure no stale named state lingers, then write nothing.
            std::fs::remove_dir_all(&named_dir).ok();
            std::fs::remove_file(&manifest).ok();
            return Ok(());
        }
        // Rewrite the subtree from scratch so removed/renamed graphs cannot survive a re-save.
        std::fs::remove_dir_all(&named_dir).ok();
        std::fs::create_dir_all(&named_dir)?;
        for (i, (_, sub)) in self.named.iter().enumerate() {
            let sub_dir = named_dir.join(i.to_string());
            if compressed {
                sub.save_compressed(&sub_dir)?;
            } else {
                sub.save(&sub_dir)?;
            }
        }
        // Write the manifest LAST (after every sub-graph is durable) so a crash mid-save
        // never leaves a manifest pointing at a missing/partial sub-graph. The presence of
        // a complete `named.bin` is the commit point for the named-graph set.
        let names: Vec<Term> = self.named.iter().map(|(n, _)| n.clone()).collect();
        write_named_manifest(dir, &names)
    }


```

## Workspace deleted/overlay mutation inventory

Path: source/workspace-deleted-mutation-inventory.txt; SHA256 874e659a8c777c89cf9b0919601a821eec2dff969fd95d91928d8ab935184d24

```text
./crates/sparq-core/src/store/overlay_deleted_tests.rs:33:                assert!(ov.deleted_by_perm[perm as usize].get().is_none());
./crates/sparq-core/src/store/overlay_deleted_tests.rs:48:    assert!(ov.deleted_by_perm.iter().all(|s| s.get().is_none()));
./crates/sparq-core/src/store/overlay_deleted_tests.rs:57:        let rows = ov.deleted_by_perm[perm as usize]
./crates/sparq-core/src/store/overlay_deleted_tests.rs:67:                ov.deleted_by_perm[perm as usize].get().unwrap().as_ptr(),
./crates/sparq-core/src/store/overlay_deleted_tests.rs:75:                assert!(ov.deleted_by_perm[other as usize].get().is_none());
./crates/sparq-core/src/store/overlay_deleted_tests.rs:180:    ov.deleted.extend([[0, 1, 2], [2, 3, 4], [Id::MAX; 3]]);
./crates/sparq-core/src/store/overlay_deleted_tests.rs:183:        for &t in &ov.deleted {
./crates/sparq-core/src/store/overlay_deleted_tests.rs:185:            assert_eq!(ov.deleted_count(perm, row, row), 1);
./crates/sparq-core/src/store/overlay_deleted_tests.rs:187:        assert_eq!(ov.deleted_count(perm, [0; 3], [Id::MAX; 3]), 3);
./crates/sparq-core/src/store/overlay_deleted_tests.rs:188:        assert_eq!(ov.deleted_count(perm, [42; 3], [42; 3]), 0);
./crates/sparq-core/src/store.rs:170:struct Overlay {
./crates/sparq-core/src/store.rs:185:impl Overlay {
./crates/sparq-core/src/store.rs:876:            Some(ov) => base + ov.added.len() - ov.deleted.len(),
./crates/sparq-core/src/store.rs:906:                !ov.deleted.contains(&t)
./crates/sparq-core/src/store.rs:930:        let mut ov = self.overlay.take().unwrap_or_default();
./crates/sparq-core/src/store.rs:940:                deleted_changed |= ov.deleted.insert(*t);
./crates/sparq-core/src/store.rs:944:            if ov.deleted.remove(t) {
./crates/sparq-core/src/store.rs:958:        self.overlay = if ov.is_empty() { None } else { Some(ov) };
./crates/sparq-core/src/store.rs:996:        self.overlay.as_ref().map_or(0, |ov| ov.added.len() + ov.deleted.len())
```


## Graph replacement and WAL consumer inventory

Path: source/final-mutation-2.txt; SHA256 cb31375b7077de44539d9618900f0067b741f978ed49490a58f6ab0bef85b40f

```text
crates/sparq-core/src/lib.rs:83:    /// delta that appends terms (`apply_delta_mem`) so it is recomputed over the grown
crates/sparq-core/src/lib.rs:1952:        for (insert, t) in Wal::replay(dir)? {
crates/sparq-core/src/lib.rs:1954:                g.apply_delta_mem(&[t], &[]);
crates/sparq-core/src/lib.rs:1956:                g.apply_delta_mem(&[], &[t]);
crates/sparq-core/src/lib.rs:1972:        // `apply_delta_mem` (IN-MEMORY only) and then truncated the journal, so a record that lived
crates/sparq-core/src/lib.rs:1997:    /// [`apply_delta`](Self::apply_delta) (NOT the in-memory-only `apply_delta_mem`), so a record
crates/sparq-core/src/lib.rs:2955:        self.apply_delta_mem(inserts, deletes);
crates/sparq-core/src/lib.rs:3059:        self.store = TripleStore::from_triples(Vec::new());
crates/sparq-core/src/lib.rs:3214:        // likewise drops its handles via `*self = Graph::open(..)` after its swap).
crates/sparq-core/src/lib.rs:3372:    fn apply_delta_mem(&mut self, inserts: &[[Term; 3]], deletes: &[[Term; 3]]) {
crates/sparq-core/src/lib.rs:3429:            self.store = TripleStore::from_triples(triples);
crates/sparq-core/src/lib.rs:3440:            self.persist_swap(&dir)?;
crates/sparq-core/src/lib.rs:3462:    fn persist_swap(&mut self, dir: &std::path::Path) -> Result<(), String> {
crates/sparq-core/src/lib.rs:3469:        *self = reopened;
crates/sparq-core/src/lib.rs:3476:    /// `persist_swap` also calls — there is one crash-safe swap implementation, not two). Returns
crates/sparq-core/src/lib.rs:3540:    /// swapped in via the SAME rollback-safe `persist_swap` the compaction
crates/sparq-core/src/lib.rs:3553:        let mut fresh = self.reintern_live()?;
crates/sparq-core/src/lib.rs:3557:            // renamed away). `persist_swap` writes `fresh`'s image to `dir` and re-opens
crates/sparq-core/src/lib.rs:3561:            fresh.persist_swap(&dir)?;
crates/sparq-core/src/lib.rs:3564:        std::mem::swap(self, &mut fresh);
crates/sparq-core/src/lib.rs:3571:    /// (the caller's [`persist_swap`](Self::persist_swap) gives the swapped-in graph its own).
crates/sparq-core/src/lib.rs:3572:    fn reintern_live(&self) -> Result<Graph, String> {
crates/sparq-core/src/lib.rs:3591:            let sub_fresh = sub.reintern_live()?;
crates/sparq-core/src/lib.rs:3626:/// [`persist_swap`](Graph::persist_swap) so the public [`restore_into_durable`](Graph::restore_into_durable)
crates/sparq-core/src/lib.rs:3632:/// The protocol (matching the design recorded on `persist_swap`):
crates/sparq-core/src/lib.rs:4084:/// in-memory-only `apply_delta_mem`, and the truncation happens only AFTER every record is durably
crates/sparq-core/src/lib.rs:4173:    /// applied. EXACT clone of [`Wal::replay`]'s torn-tail logic, extended with the per-record slot.
crates/sparq-core/src/lib.rs:9969:    /// Under the OLD behaviour (`apply_delta_mem` then truncate) the first reopen applied the record
```


## Entire current cache semantic/regression suite

Path: source/crates/sparq-core/src/store/overlay_deleted_tests.rs; SHA256 2a8bde92fd9191ba0486c8f150febaed6d927ac975d1d0728cde03f63e8effb3

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
```


## Lifecycle harness whole implementation

Path: source/bench/overlay-count/src/lifecycle.rs; SHA256 da5ea2ee729be7c505334f5977cef6e353b2f5e868665f83d8327b29e77350e8

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
```


## Shared fixture/query/allocation-window callers

Path: source/bench/overlay-count/src/main.rs; SHA256 428313536dbdedb7d999070420675e91939faac57e1dfc94eb8b9d8b4d1087b2

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


## Unchanged counting allocator with calibration

Path: source/bench/overlay-count/src/counting.rs; SHA256 25b3712e37c4142edac164b32ce47308123bac8201f8a3ff2e3078cd4b5416f2

```rust
//! [GPT-6 Astra] Bench-only System wrapper, following bench/alloc-track.
//! Requested live bytes exclude allocator metadata and transient realloc internals.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering::Relaxed};

struct Counting;
static ACTIVE: AtomicBool = AtomicBool::new(false);
static LIVE: AtomicU64 = AtomicU64::new(0);
static PEAK: AtomicU64 = AtomicU64::new(0);
static ALLOCS: AtomicU64 = AtomicU64::new(0);
static REALLOCS: AtomicU64 = AtomicU64::new(0);
static BYTES: AtomicU64 = AtomicU64::new(0);

fn added(bytes: usize) {
    let live = LIVE.fetch_add(bytes as u64, Relaxed) + bytes as u64;
    if ACTIVE.load(Relaxed) {
        PEAK.fetch_max(live, Relaxed);
    }
}

// SAFETY: All pointer/layout operations are forwarded unchanged to System.
// Atomics allocate nothing, never dereference pointers and never unwind.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: The GlobalAlloc caller supplies a valid layout.
        let ptr = unsafe { System.alloc(layout) };
        if !ptr.is_null() {
            added(layout.size());
            if ACTIVE.load(Relaxed) {
                ALLOCS.fetch_add(1, Relaxed);
                BYTES.fetch_add(layout.size() as u64, Relaxed);
            }
        }
        ptr
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        // SAFETY: The GlobalAlloc caller supplies a valid layout.
        let ptr = unsafe { System.alloc_zeroed(layout) };
        if !ptr.is_null() {
            added(layout.size());
            if ACTIVE.load(Relaxed) {
                ALLOCS.fetch_add(1, Relaxed);
                BYTES.fetch_add(layout.size() as u64, Relaxed);
            }
        }
        ptr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: The caller supplies this allocation's original pointer/layout.
        unsafe { System.dealloc(ptr, layout) };
        LIVE.fetch_sub(layout.size() as u64, Relaxed);
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        // SAFETY: The caller supplies a live allocation and valid nonzero new size.
        let new_ptr = unsafe { System.realloc(ptr, layout, new_size) };
        if !new_ptr.is_null() {
            if new_size >= layout.size() {
                added(new_size - layout.size());
            } else {
                LIVE.fetch_sub((layout.size() - new_size) as u64, Relaxed);
            }
            if ACTIVE.load(Relaxed) {
                REALLOCS.fetch_add(1, Relaxed);
                // Full new request size, not merely the growth in live bytes.
                BYTES.fetch_add(new_size as u64, Relaxed);
            }
        }
        new_ptr
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

/// Begin a window while the benchmark and its initialized Rayon pool are idle.
pub fn begin() -> u64 {
    assert!(!ACTIVE.swap(false, Relaxed));
    let baseline = LIVE.load(Relaxed);
    PEAK.store(baseline, Relaxed);
    ALLOCS.store(0, Relaxed);
    REALLOCS.store(0, Relaxed);
    BYTES.store(0, Relaxed);
    ACTIVE.store(true, Relaxed);
    baseline
}

/// Stop the window before formatting output or checking returned query results.
pub fn end(baseline: u64) -> (u64, u64, u64, u64, u64) {
    ACTIVE.store(false, Relaxed);
    (
        ALLOCS.load(Relaxed),
        REALLOCS.load(Relaxed),
        BYTES.load(Relaxed),
        PEAK.load(Relaxed).saturating_sub(baseline),
        LIVE.load(Relaxed),
    )
}

pub fn calibrate() {
    let baseline = begin();
    // SAFETY: Each successful allocation is used only with its matching layout;
    // realloc transfers ownership on success. No allocated byte is dereferenced.
    unsafe {
        let old = Layout::from_size_align(128, 8).unwrap();
        let zero = Layout::from_size_align(64, 8).unwrap();
        let p = ALLOCATOR.alloc(old);
        let q = ALLOCATOR.alloc_zeroed(zero);
        assert!(!p.is_null() && !q.is_null());
        let p = ALLOCATOR.realloc(p, old, 256);
        assert!(!p.is_null());
        ALLOCATOR.dealloc(p, Layout::from_size_align(256, 8).unwrap());
        ALLOCATOR.dealloc(q, zero);
    }
    assert_eq!(end(baseline), (2, 1, 448, 320, baseline));
}
```


## Exact source/binary/build provenance

Path: provenance.json; SHA256 f310926f74e6dd4c491297034f351034b3b3a3d76eeced182da2fd923d280def

```json
{
  "generated_at": "2026-09-09T12:24:19.554870+00:00",
  "heads": {
    "candidate": "cb638a42a54fc7c9e11e9101587910e668a5f92a",
    "main-control": "912fa6c3cfd83be6b10c2e3541650120997312a9"
  },
  "production_main": "a42a9e89dec485f6a319c47cb3635c59cb5a2270",
  "harness_files": {
    "bench/overlay-count/Cargo.toml": "f0cbcc2c7f6ea5e1e576254dc9b3904540833e03ac796505d31bf926f8b1c040",
    "bench/overlay-count/Cargo.lock": "b319e3eafeffff389ed020f2213f89dbc0f7041405a3dc9d42facec6e74681bf",
    "bench/overlay-count/README.md": "4723afb882d47f46f86b62355646387e8687a53942098c5c743e21202f9c8260",
    "bench/overlay-count/src/counting.rs": "25b3712e37c4142edac164b32ce47308123bac8201f8a3ff2e3078cd4b5416f2",
    "bench/overlay-count/src/main.rs": "428313536dbdedb7d999070420675e91939faac57e1dfc94eb8b9d8b4d1087b2",
    "bench/overlay-count/src/lifecycle.rs": "da5ea2ee729be7c505334f5977cef6e353b2f5e868665f83d8327b29e77350e8"
  },
  "identical_harness": true,
  "control_production_diff_empty": true,
  "runtime_files": {
    "candidate": {
      "crates/sparq-core/src/store.rs": "54b8c5b1019599c0f99eb573817118c371218de954f91a86a3eda94be72c767a",
      "crates/sparq-core/src/lib.rs": "3679c762c28c956df60ee519682420c2800993a3b3674b77ed335bb9acba6a45",
      "crates/sparq-engine/src/lib.rs": "2efd087a5bb3ba10461ceedcf67172bc34287ce15b26eac95b13b555a14736fb",
      "crates/sparq-engine/src/exec.rs": "71f7279359bf982dad1f751fcbc6b42cd68200daebf0de2f417bca1a20d867fd"
    },
    "main-control": {
      "crates/sparq-core/src/store.rs": "807a812447409889e02fb7125f8e87761a8276d5cff0ff7d7d740b2e919d683f",
      "crates/sparq-core/src/lib.rs": "3679c762c28c956df60ee519682420c2800993a3b3674b77ed335bb9acba6a45",
      "crates/sparq-engine/src/lib.rs": "2efd087a5bb3ba10461ceedcf67172bc34287ce15b26eac95b13b555a14736fb",
      "crates/sparq-engine/src/exec.rs": "71f7279359bf982dad1f751fcbc6b42cd68200daebf0de2f417bca1a20d867fd"
    }
  },
  "binaries": {
    "candidate-count": {
      "sha256": "cd2ef2767e7e03dc0b1a35d390fc7609d67a3fbffbcc1919b3213fc72d2e1e9e",
      "bytes": 5402656
    },
    "candidate-time": {
      "sha256": "a2bfaf5e1004d9d44a02048d592d2c52588581325be91b88b308662ad8080cbe",
      "bytes": 5401072
    },
    "main-time": {
      "sha256": "3939dc2ef78bb4488628047e61ba72a231f77885e4d58e1d94465444f7f3e5d6",
      "bytes": 5381488
    },
    "main-count": {
      "sha256": "c3d8ef73730fe3b79f8eb6a3e37684d36dc54297e9ada039870a679ce6cd6a0e",
      "bytes": 5399424
    }
  },
  "build_jobs": 2,
  "rayon_threads": 1,
  "concurrent_case_threads": 2,
  "optimized_target": "/private/tmp/sparq-pr6049/.throughput-monitor/direct-5983/diagnostic/target",
  "toolchain": "rustc 1.97.1 (8bab26f4f 2026-07-14)\nbinary: rustc\ncommit-hash: 8bab26f4f68e0e26f0bb7960be334d5b520ea452\ncommit-date: 2026-07-14\nhost: aarch64-apple-darwin\nrelease: 1.97.1\nLLVM version: 22.1.6\n",
  "cargo": "cargo 1.97.1 (c980f4866 2026-06-30)\n",
  "profile": "opt-level=3, lto=false, debug=false, codegen-units=16",
  "features": [
    "default (timing)",
    "default+count-alloc (allocation)"
  ],
  "no_builds_during_measurement": true,
  "optimized_build_logs": [
    "builds/candidate-time.log",
    "builds/main-time.log",
    "builds/main-count.log",
    "builds/candidate-count.log"
  ],
  "build_cargo_seconds": {
    "candidate-time": 38.36,
    "main-time": 40.11,
    "main-count": 4.37,
    "candidate-count": 3.71
  },
  "total_optimized_cargo_seconds": 86.55,
  "clean_worktrees": {
    "candidate": true,
    "main-control": true
  }
}
```


## Paired generated timing/allocation/heap ranges

Path: paired/review-summary.md; SHA256 e05bf415300d47057662e17a35dc20c69b187d17a49a37f46ac08e26b97494c2

```text
Generated directly from paired/summary.json. Each range is min/median/max; all samples retained.

| Case / warm permutations / generation | Main ns | Candidate ns | Median ratio | Disjoint ranges |
|---|---:|---:|---:|---|
| concurrent-cold / 0 / 1 | 297250/336792/401833 | 1.28738e+06/1.35438e+06/1.54358e+06 | 4.021 | True |
| fork-insert / 1 / 1 | 225083/232125/297208 | 56250/62042/94792 | 0.267 | True |
| fork-insert / 1 / 2 | 200375/208458/332292 | 35792/40542/73125 | 0.194 | True |
| fork-insert / 1 / 3 | 194375/199708/369291 | 37000/42209/114917 | 0.211 | True |
| fork-insert / 1 / 4 | 193875/209334/221042 | 40667/42000/125959 | 0.201 | True |
| fork-insert / 6 / 1 | 219125/239209/313958 | 92167/117958/259583 | 0.493 | False |
| fork-insert / 6 / 2 | 200917/219875/359458 | 103500/114292/246917 | 0.520 | False |
| fork-insert / 6 / 3 | 196334/203583/365708 | 108458/141875/670250 | 0.697 | False |
| fork-insert / 6 / 4 | 194667/197917/364500 | 94875/111917/871708 | 0.565 | False |
| fork-tombstone / 1 / 1 | 222750/236666/273167 | 1.16671e+06/1.21204e+06/1.72967e+06 | 5.121 | True |
| fork-tombstone / 1 / 2 | 195417/208084/401417 | 1.152e+06/1.19088e+06/1.81546e+06 | 5.723 | True |
| fork-tombstone / 1 / 3 | 193042/195500/285625 | 1.15104e+06/1.21833e+06/1.62071e+06 | 6.232 | True |
| fork-tombstone / 1 / 4 | 191917/204666/314459 | 1.17262e+06/1.21158e+06/1.36596e+06 | 5.920 | True |
| fork-tombstone / 6 / 1 | 222708/233375/481667 | 1.22583e+06/1.31133e+06/1.55125e+06 | 5.619 | True |
| fork-tombstone / 6 / 2 | 194958/198667/360792 | 1.16712e+06/1.22258e+06/1.33846e+06 | 6.154 | True |
| fork-tombstone / 6 / 3 | 192584/203208/443917 | 1.14371e+06/1.21908e+06/1.56592e+06 | 5.999 | True |
| fork-tombstone / 6 / 4 | 192208/198417/324542 | 1.17883e+06/1.28304e+06/2.01004e+06 | 6.466 | True |
| inplace-insert / 6 / 1 | 216917/252500/369125 | 34791/42541/69334 | 0.168 | True |
| inplace-insert / 6 / 2 | 180250/197667/272250 | 6042/6375/6666 | 0.032 | True |
| inplace-insert / 6 / 3 | 178458/189083/252041 | 4875/5250/5375 | 0.028 | True |
| inplace-insert / 6 / 4 | 181542/187666/299625 | 4583/4833/7584 | 0.026 | True |
| multi-cold / 0 / 1 | 938792/962792/1.55579e+06 | 3.56846e+06/3.82917e+06/3.96512e+06 | 3.977 | True |
| multi-warm / 3 / 1 | 855041/874000/946417 | 15625/28792/59958 | 0.033 | True |
| snapshot / 1 / 1 | 226083/241459/410959 | 54375/86375/374250 | 0.358 | False |
| snapshot / 1 / 2 | 201208/215000/411166 | 27083/35458/108125 | 0.165 | True |
| snapshot / 1 / 3 | 199000/210209/422125 | 25459/37167/54167 | 0.177 | True |
| snapshot / 1 / 4 | 196125/208167/227292 | 24750/44250/84250 | 0.213 | True |
| snapshot / 6 / 1 | 224959/235708/279084 | 91667/110000/828375 | 0.467 | False |
| snapshot / 6 / 2 | 195709/203709/232250 | 64083/85708/299792 | 0.421 | False |
| snapshot / 6 / 3 | 196291/206875/241958 | 63125/133917/750250 | 0.647 | False |
| snapshot / 6 / 4 | 196000/206750/232167 | 62208/91458/247041 | 0.442 | False |

| Case / perms / generation | Main requested B | Candidate requested B | Main peak live Δ B | Candidate peak live Δ B | Main retained overlay B | Candidate retained overlay B |
|---|---:|---:|---:|---:|---:|---:|
| concurrent-cold / 0 / 1 | 12142/12206/12206 | 405358/405358/405358 | 4663/4734/4734 | 397678/397678/397950 | 745472/745472/745472 | 1.13869e+06/1.13869e+06/1.13869e+06 |
| fork-insert / 1 / 1 | 858493/858493/858493 | 1.2519e+06/1.2519e+06/1.2519e+06 | 854778/854778/854778 | 1.24819e+06/1.24819e+06/1.24819e+06 | 1.49099e+06/1.49099e+06/1.49099e+06 | 2.27742e+06/2.27742e+06/2.27742e+06 |
| fork-insert / 1 / 2 | 858580/858580/858580 | 1.25199e+06/1.25199e+06/1.25199e+06 | 854787/854787/854787 | 1.2482e+06/1.2482e+06/1.2482e+06 | 2.23651e+06/2.23651e+06/2.23651e+06 | 3.41616e+06/3.41616e+06/3.41616e+06 |
| fork-insert / 1 / 3 | 858769/858769/858769 | 1.25218e+06/1.25218e+06/1.25218e+06 | 854896/854896/854896 | 1.2483e+06/1.2483e+06/1.2483e+06 | 2.98203e+06/2.98203e+06/2.98203e+06 | 4.5549e+06/4.5549e+06/4.5549e+06 |
| fork-insert / 1 / 4 | 858906/858906/858906 | 1.25231e+06/1.25231e+06/1.25231e+06 | 855009/855009/855009 | 1.24842e+06/1.24842e+06/1.24842e+06 | 3.72758e+06/3.72758e+06/3.72758e+06 | 5.69366e+06/5.69366e+06/5.69366e+06 |
| fork-insert / 6 / 1 | 858493/858493/858493 | 3.21798e+06/3.21798e+06/3.21798e+06 | 854778/854778/854778 | 3.21427e+06/3.21427e+06/3.21427e+06 | 1.49099e+06/1.49099e+06/1.49099e+06 | 6.20958e+06/6.20958e+06/6.20958e+06 |
| fork-insert / 6 / 2 | 858580/858580/858580 | 3.21807e+06/3.21807e+06/3.21807e+06 | 854787/854787/854787 | 3.21428e+06/3.21428e+06/3.21428e+06 | 2.23651e+06/2.23651e+06/2.23651e+06 | 9.3144e+06/9.3144e+06/9.3144e+06 |
| fork-insert / 6 / 3 | 858769/858769/858769 | 3.21826e+06/3.21826e+06/3.21826e+06 | 854896/854896/854896 | 3.21438e+06/3.21438e+06/3.21438e+06 | 2.98203e+06/2.98203e+06/2.98203e+06 | 1.24192e+07/1.24192e+07/1.24192e+07 |
| fork-insert / 6 / 4 | 858906/858906/858906 | 3.21839e+06/3.21839e+06/3.21839e+06 | 855009/855009/855009 | 3.2145e+06/3.2145e+06/3.2145e+06 | 3.72758e+06/3.72758e+06/3.72758e+06 | 1.55241e+07/1.55241e+07/1.55241e+07 |
| fork-tombstone / 1 / 1 | 858247/858247/858247 | 1.64488e+06/1.64488e+06/1.64488e+06 | 854519/854519/854519 | 1.24794e+06/1.24794e+06/1.24794e+06 | 1.49094e+06/1.49094e+06/1.49094e+06 | 2.27739e+06/2.27739e+06/2.27739e+06 |
| fork-tombstone / 1 / 2 | 858247/858247/858247 | 1.64491e+06/1.64491e+06/1.64491e+06 | 854519/854519/854519 | 1.24795e+06/1.24795e+06/1.24795e+06 | 2.23642e+06/2.23642e+06/2.23642e+06 | 3.4161e+06/3.4161e+06/3.4161e+06 |
| fork-tombstone / 1 / 3 | 858247/858247/858247 | 1.64493e+06/1.64493e+06/1.64493e+06 | 854519/854519/854519 | 1.24796e+06/1.24796e+06/1.24796e+06 | 2.98189e+06/2.98189e+06/2.98189e+06 | 4.55482e+06/4.55482e+06/4.55482e+06 |
| fork-tombstone / 1 / 4 | 858247/858247/858247 | 1.64496e+06/1.64496e+06/1.64496e+06 | 854519/854519/854519 | 1.24798e+06/1.24798e+06/1.24798e+06 | 3.72736e+06/3.72736e+06/3.72736e+06 | 5.69356e+06/5.69356e+06/5.69356e+06 |
| fork-tombstone / 6 / 1 | 858247/858247/858247 | 3.61096e+06/3.61096e+06/3.61096e+06 | 854519/854519/854519 | 3.21188e+06/3.21188e+06/3.21188e+06 | 1.49094e+06/1.49094e+06/1.49094e+06 | 4.24347e+06/4.24347e+06/4.24347e+06 |
| fork-tombstone / 6 / 2 | 858247/858247/858247 | 1.64491e+06/1.64491e+06/1.64491e+06 | 854519/854519/854519 | 1.24795e+06/1.24795e+06/1.24795e+06 | 2.23642e+06/2.23642e+06/2.23642e+06 | 5.38218e+06/5.38218e+06/5.38218e+06 |
| fork-tombstone / 6 / 3 | 858247/858247/858247 | 1.64493e+06/1.64493e+06/1.64493e+06 | 854519/854519/854519 | 1.24796e+06/1.24796e+06/1.24796e+06 | 2.98189e+06/2.98189e+06/2.98189e+06 | 6.5209e+06/6.5209e+06/6.5209e+06 |
| fork-tombstone / 6 / 4 | 858247/858247/858247 | 1.64496e+06/1.64496e+06/1.64496e+06 | 854519/854519/854519 | 1.24798e+06/1.24798e+06/1.24798e+06 | 3.72736e+06/3.72736e+06/3.72736e+06 | 7.65964e+06/7.65964e+06/7.65964e+06 |
| inplace-insert / 6 / 1 | 6145/6145/6145 | 6145/6145/6145 | 2430/2430/2430 | 2430/2430/2430 | 745520/745520/745520 | 3.10482e+06/3.10482e+06/3.10482e+06 |
| inplace-insert / 6 / 2 | 5881/5881/5881 | 5881/5881/5881 | 2180/2180/2180 | 2180/2180/2180 | 745520/745520/745520 | 3.10482e+06/3.10482e+06/3.10482e+06 |
| inplace-insert / 6 / 3 | 5929/5929/5929 | 5929/5929/5929 | 2200/2200/2200 | 2200/2200/2200 | 745520/745520/745520 | 3.10482e+06/3.10482e+06/3.10482e+06 |
| inplace-insert / 6 / 4 | 6201/6201/6201 | 6201/6201/6201 | 2340/2340/2340 | 2340/2340/2340 | 745520/745520/745520 | 3.10482e+06/3.10482e+06/3.10482e+06 |
| multi-cold / 0 / 1 | 13414/13414/13414 | 1.19306e+06/1.19306e+06/1.19306e+06 | 3814/3814/3814 | 1.18346e+06/1.18346e+06/1.18346e+06 | 745472/745472/745472 | 1.92512e+06/1.92512e+06/1.92512e+06 |
| multi-warm / 3 / 1 | 13414/13414/13414 | 13414/13414/13414 | 3814/3814/3814 | 3814/3814/3814 | 745472/745472/745472 | 1.92512e+06/1.92512e+06/1.92512e+06 |
| snapshot / 1 / 1 | 858199/858199/858199 | 1.25161e+06/1.25161e+06/1.25161e+06 | 854519/854519/854519 | 1.24793e+06/1.24793e+06/1.24793e+06 | 1.49094e+06/1.49094e+06/1.49094e+06 | 2.27738e+06/2.27738e+06/2.27738e+06 |
| snapshot / 1 / 2 | 858199/858199/858199 | 1.25161e+06/1.25161e+06/1.25161e+06 | 854519/854519/854519 | 1.24793e+06/1.24793e+06/1.24793e+06 | 2.23642e+06/2.23642e+06/2.23642e+06 | 3.41606e+06/3.41606e+06/3.41606e+06 |
| snapshot / 1 / 3 | 858199/858199/858199 | 1.25161e+06/1.25161e+06/1.25161e+06 | 854519/854519/854519 | 1.24793e+06/1.24793e+06/1.24793e+06 | 2.98189e+06/2.98189e+06/2.98189e+06 | 4.55475e+06/4.55475e+06/4.55475e+06 |
| snapshot / 1 / 4 | 858199/858199/858199 | 1.25161e+06/1.25161e+06/1.25161e+06 | 854519/854519/854519 | 1.24793e+06/1.24793e+06/1.24793e+06 | 3.72736e+06/3.72736e+06/3.72736e+06 | 5.69344e+06/5.69344e+06/5.69344e+06 |
| snapshot / 6 / 1 | 858199/858199/858199 | 3.21769e+06/3.21769e+06/3.21769e+06 | 854519/854519/854519 | 3.21401e+06/3.21401e+06/3.21401e+06 | 1.49094e+06/1.49094e+06/1.49094e+06 | 6.20954e+06/6.20954e+06/6.20954e+06 |
| snapshot / 6 / 2 | 858199/858199/858199 | 3.21769e+06/3.21769e+06/3.21769e+06 | 854519/854519/854519 | 3.21401e+06/3.21401e+06/3.21401e+06 | 2.23642e+06/2.23642e+06/2.23642e+06 | 9.3143e+06/9.3143e+06/9.3143e+06 |
| snapshot / 6 / 3 | 858199/858199/858199 | 3.21769e+06/3.21769e+06/3.21769e+06 | 854519/854519/854519 | 3.21401e+06/3.21401e+06/3.21401e+06 | 2.98189e+06/2.98189e+06/2.98189e+06 | 1.24191e+07/1.24191e+07/1.24191e+07 |
| snapshot / 6 / 4 | 858199/858199/858199 | 3.21769e+06/3.21769e+06/3.21769e+06 | 854519/854519/854519 | 3.21401e+06/3.21401e+06/3.21401e+06 | 3.72736e+06/3.72736e+06/3.72736e+06 | 1.55238e+07/1.55238e+07/1.55238e+07 |
```


## Assumption-qualified amortization from saved data

Path: paired/amortization.json; SHA256 b56af4a4ca389f896f24b39cacae790356759ea2bcee9a5f90080bf37a8386e0

```json
{
  "method": "Algebra only: first window C1/M1 plus (reads-1)*warm-query Qc/Qm. First integer reads with C1+(reads-1)*Qc < M1+(reads-1)*Qm. Uses saved medians, not an additional measurement.",
  "assumptions": [
    "Subsequent queries have the same shape and access the already materialized permutations on the same generation, with no intervening tombstone mutation/compaction.",
    "Fork-tombstone warm read cost is borrowed from the snapshot(one warmed permutation) read phase at the same generation index. It has D rather than D+generation tombstones; assumes that tiny difference does not materially change warmed query latency.",
    "Multi-pattern warm and cold cases share exact fixture/query.",
    "Costs add linearly and median observations are representative; no covariance/confidence interval or scheduler/allocator/cache-pressure model is established.",
    "Does not repay or remove retained memory, clone copies, cold tail latency, or costs for new permutations; not an admission policy/threshold."
  ],
  "rows": [
    {
      "case": "fork-tombstone",
      "warm_perms": 1,
      "generation": 1,
      "first_main_ns": 236666,
      "first_candidate_ns": 1212041,
      "assumed_subsequent_main_query_ns": 225958,
      "assumed_subsequent_candidate_query_ns": 37958,
      "first_strictly_cheaper_total_reads": 7,
      "projected_main_ns": 1592414,
      "projected_candidate_ns": 1439789
    },
    {
      "case": "fork-tombstone",
      "warm_perms": 1,
      "generation": 2,
      "first_main_ns": 208084,
      "first_candidate_ns": 1190875,
      "assumed_subsequent_main_query_ns": 197958,
      "assumed_subsequent_candidate_query_ns": 6500,
      "first_strictly_cheaper_total_reads": 7,
      "projected_main_ns": 1395832,
      "projected_candidate_ns": 1229875
    },
    {
      "case": "fork-tombstone",
      "warm_perms": 1,
      "generation": 3,
      "first_main_ns": 195500,
      "first_candidate_ns": 1218334,
      "assumed_subsequent_main_query_ns": 190833,
      "assumed_subsequent_candidate_query_ns": 4791,
      "first_strictly_cheaper_total_reads": 7,
      "projected_main_ns": 1340498,
      "projected_candidate_ns": 1247080
    },
    {
      "case": "fork-tombstone",
      "warm_perms": 1,
      "generation": 4,
      "first_main_ns": 204666,
      "first_candidate_ns": 1211583,
      "assumed_subsequent_main_query_ns": 190167,
      "assumed_subsequent_candidate_query_ns": 4291,
      "first_strictly_cheaper_total_reads": 7,
      "projected_main_ns": 1345668,
      "projected_candidate_ns": 1237329
    },
    {
      "case": "fork-tombstone",
      "warm_perms": 6,
      "generation": 1,
      "first_main_ns": 233375,
      "first_candidate_ns": 1311334,
      "assumed_subsequent_main_query_ns": 225958,
      "assumed_subsequent_candidate_query_ns": 37958,
      "first_strictly_cheaper_total_reads": 7,
      "projected_main_ns": 1589123,
      "projected_candidate_ns": 1539082
    },
    {
      "case": "fork-tombstone",
      "warm_perms": 6,
      "generation": 2,
      "first_main_ns": 198667,
      "first_candidate_ns": 1222584,
      "assumed_subsequent_main_query_ns": 197958,
      "assumed_subsequent_candidate_query_ns": 6500,
      "first_strictly_cheaper_total_reads": 7,
      "projected_main_ns": 1386415,
      "projected_candidate_ns": 1261584
    },
    {
      "case": "fork-tombstone",
      "warm_perms": 6,
      "generation": 3,
      "first_main_ns": 203208,
      "first_candidate_ns": 1219083,
      "assumed_subsequent_main_query_ns": 190833,
      "assumed_subsequent_candidate_query_ns": 4791,
      "first_strictly_cheaper_total_reads": 7,
      "projected_main_ns": 1348206,
      "projected_candidate_ns": 1247829
    },
    {
      "case": "fork-tombstone",
      "warm_perms": 6,
      "generation": 4,
      "first_main_ns": 198417,
      "first_candidate_ns": 1283041,
      "assumed_subsequent_main_query_ns": 190167,
      "assumed_subsequent_candidate_query_ns": 4291,
      "first_strictly_cheaper_total_reads": 7,
      "projected_main_ns": 1339419,
      "projected_candidate_ns": 1308787
    },
    {
      "case": "multi-cold",
      "first_main_ns": 962792,
      "first_candidate_ns": 3829167,
      "assumed_subsequent_main_query_ns": 874000,
      "assumed_subsequent_candidate_query_ns": 28792,
      "first_strictly_cheaper_total_reads": 5,
      "projected_main_ns": 4458792,
      "projected_candidate_ns": 3944335
    }
  ]
}
```


## Regression actually failed before repair

Path: tests/b1-before.log; SHA256 3508d10caa524c13ee1fd53546e7b2498893fa91ba4c30bb9f678cfa638fb4ec

```text
   Compiling sparq-core v0.1.1 (/private/tmp/sparq-pr6049/.throughput-monitor/worktrees/issue4246/crates/sparq-core)
    Finished `test` profile [unoptimized] target(s) in 13.08s
     Running unittests src/lib.rs (/private/tmp/sparq-pr6049/.throughput-monitor/direct-5983/implementation/target/debug/deps/sparq_core-53b43ac9cb50e22c)

running 1 test
test store::overlay_deleted_tests::deleted_cache_survives_insert_only_and_noop_deltas ... FAILED

failures:

---- store::overlay_deleted_tests::deleted_cache_survives_insert_only_and_noop_deltas stdout ----

thread 'store::overlay_deleted_tests::deleted_cache_survives_insert_only_and_noop_deltas' (2382785) panicked at crates/sparq-core/src/store/overlay_deleted_tests.rs:136:22:
unchanged tombstones retain their projection
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace


failures:
    store::overlay_deleted_tests::deleted_cache_survives_insert_only_and_noop_deltas

test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 154 filtered out; finished in 0.01s

error: test failed, to rerun pass `-p sparq-core --lib`
```


## Actual final store-default output

Path: tests/store-default.log; SHA256 4357db1e07d9e964e49bd4de0d85942d8499e7fa8d566cd7efbd04e08f3ced78

```text
   Compiling sparq-core v0.1.1 (/private/tmp/sparq-pr6049/.throughput-monitor/worktrees/issue4246/crates/sparq-core)
    Finished `test` profile [unoptimized] target(s) in 6.11s
     Running unittests src/lib.rs (/private/tmp/sparq-pr6049/.throughput-monitor/direct-5983/implementation/target/debug/deps/sparq_core-53b43ac9cb50e22c)

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

test result: ok. 19 passed; 0 failed; 1 ignored; 0 measured; 135 filtered out; finished in 0.56s
```


## Actual final store-compact output

Path: tests/store-compact.log; SHA256 df89e16eb8315b86219efee9569e9e142201b73272c97cefe3e1a8d398db818c

```text
   Compiling sparq-core v0.1.1 (/private/tmp/sparq-pr6049/.throughput-monitor/worktrees/issue4246/crates/sparq-core)
    Finished `test` profile [unoptimized] target(s) in 9.26s
     Running unittests src/lib.rs (/private/tmp/sparq-pr6049/.throughput-monitor/direct-5983/implementation/target/debug/deps/sparq_core-6cd45531027c75c9)

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

test result: ok. 19 passed; 0 failed; 1 ignored; 0 measured; 105 filtered out; finished in 0.48s
```


## Actual final snapshot-fork output

Path: tests/snapshot-fork.log; SHA256 3532ff7763fcdb48791956b31ded39d0ea6d56e0f1ecce6d17ec5822de998f4b

```text
   Compiling sparq-core v0.1.1 (/private/tmp/sparq-pr6049/.throughput-monitor/worktrees/issue4246/crates/sparq-core)
    Finished `test` profile [unoptimized] target(s) in 37.94s
     Running tests/fork_differential.rs (/private/tmp/sparq-pr6049/.throughput-monitor/direct-5983/implementation/target/debug/deps/fork_differential-216e2d0e0f466ae3)

running 3 tests
test fork_chain_matches_flat_rebuild ... ok
test fork_pending_delta_accounting ... ok
test named_graphs_fork_isolated ... ok

test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 2.15s

     Running tests/snapshot.rs (/private/tmp/sparq-pr6049/.throughput-monitor/direct-5983/implementation/target/debug/deps/snapshot-a52f141cc1305978)

running 7 tests
test base_mutation_after_snapshot_invisible_to_snapshot ... ok
test snapshot_covers_named_graphs ... ok
test snapshot_includes_pending_overlay_and_survives_base_compaction ... ok
test snapshot_is_send_sync ... ok
test snapshot_mutation_invisible_to_base ... ok
test snapshot_sees_point_in_time_triples ... ok
test snapshot_shares_base_indexes_no_duplication ... ok

test result: ok. 7 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.22s
```


## Actual final clippy-core output

Path: tests/clippy-core.log; SHA256 ed2bbba510064f1ec308e9a5ee1280b3e07ef0e00c868099342d7fb21dfb28ca

```text
    Checking sparq-core v0.1.1 (/private/tmp/sparq-pr6049/.throughput-monitor/worktrees/issue4246/crates/sparq-core)
    Finished `dev` profile [unoptimized] target(s) in 2.60s
```


## Actual final clippy-harness output

Path: tests/clippy-harness.log; SHA256 97260221ff516ef0456b7860002d0cbc3a4b3e7274fdb7d3e98bfeefe73c30c8

```text
    Checking libc v0.2.186
    Checking typenum v1.20.1
    Checking getrandom v0.3.4
    Checking rand_core v0.9.5
    Checking hybrid-array v0.4.12
    Checking rand_chacha v0.9.0
    Checking rand v0.9.4
    Checking block-buffer v0.12.1
    Checking crypto-common v0.2.2
    Checking oxrdf v0.3.3
    Checking const-oid v0.10.2
    Checking oxttl v0.2.3
    Checking digest v0.11.3
    Checking aho-corasick v1.1.4
    Checking cpufeatures v0.3.0
    Checking peg-runtime v0.8.6
    Checking regex-syntax v0.8.11
    Checking sparq-core v0.1.1 (/private/tmp/sparq-pr6049/.throughput-monitor/worktrees/issue4246/crates/sparq-core)
    Checking peg v0.8.6
    Checking regex-automata v0.4.14
    Checking getrandom v0.4.2
    Checking smallvec v1.15.2
    Checking sparq-substrate v0.1.1 (/private/tmp/sparq-pr6049/.throughput-monitor/worktrees/issue4246/crates/sparq-substrate)
    Checking uuid v1.23.4
    Checking spargebra v0.4.6 (/private/tmp/sparq-pr6049/.throughput-monitor/worktrees/issue4246/vendor/spargebra)
    Checking regex v1.12.4
    Checking sha1 v0.11.0
    Checking sha2 v0.11.0
    Checking md-5 v0.11.0
    Checking sparq-engine v0.1.1 (/private/tmp/sparq-pr6049/.throughput-monitor/worktrees/issue4246/crates/sparq-engine)
    Checking overlay-count-diagnostic v0.0.0 (/private/tmp/sparq-pr6049/.throughput-monitor/worktrees/issue4246/bench/overlay-count)
    Finished `dev` profile [unoptimized] target(s) in 7.18s
```


## Actual final preflight output

Path: tests/preflight.log; SHA256 ea52a0cacfcdfb14cddf110ad8f32de49eec988f8b7bbbef5cabfcb42a74671a

```text
preflight: ran  G1 new-crate-completeness, G2 public-api-to-skill, G6 new-config-to-docs, no-perf-numbers, privacy-claims, guard-untested
preflight: skip readme-template (no matching path in the diff)

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


## Current compiled controls/results

Path: controls/results.json; SHA256 b72d85b23e94c06345aff86239cb1e3342e0c17db54b034622fad2090f6b15f9

```json
{
  "command": [
    "/Users/jesght/.cargo/bin/cargo",
    "test",
    "--locked",
    "--offline",
    "-p",
    "sparq-core",
    "--lib",
    "store::overlay_deleted_tests::",
    "--",
    "--test-threads=1"
  ],
  "results": [
    {
      "name": "unconditional_deleted_invalidation",
      "exit": 101,
      "seconds": 6.148273875,
      "compiled": true,
      "summaries": [
        "test result: FAILED. 8 passed; 1 failed; 0 ignored; 0 measured; 146 filtered out; finished in 0.09s"
      ]
    },
    {
      "name": "remove_deleted_invalidation",
      "exit": 101,
      "seconds": 3.9732630419999992,
      "compiled": true,
      "summaries": [
        "test result: FAILED. 4 passed; 5 failed; 0 ignored; 0 measured; 146 filtered out; finished in 0.10s"
      ]
    },
    {
      "name": "ignore_tombstone_insert_flag",
      "exit": 101,
      "seconds": 4.144014958000001,
      "compiled": true,
      "summaries": [
        "test result: FAILED. 8 passed; 1 failed; 0 ignored; 0 measured; 146 filtered out; finished in 0.08s"
      ]
    },
    {
      "name": "ignore_tombstone_remove_flag",
      "exit": 101,
      "seconds": 3.6707139580000003,
      "compiled": true,
      "summaries": [
        "test result: FAILED. 8 passed; 1 failed; 0 ignored; 0 measured; 146 filtered out; finished in 0.10s"
      ]
    },
    {
      "name": "remove_deleted_cache_use",
      "exit": 101,
      "seconds": 3.7881935830000018,
      "compiled": true,
      "summaries": [
        "test result: FAILED. 5 passed; 4 failed; 0 ignored; 0 measured; 146 filtered out; finished in 0.09s"
      ]
    }
  ]
}
```


## Current control ignore_tombstone_insert_flag diff

Path: controls/ignore_tombstone_insert_flag.diff; SHA256 8154cb6980014c68a6fa6db5e3ff1ea01d2e19bc265e39b7393380f11ecc6163

```diff
--- store.rs
+++ ignore_tombstone_insert_flag
@@ -937,7 +937,7 @@
             if let Ok(i) = ov.added.binary_search(t) {
                 ov.added.remove(i); // retract a pending insertion
             } else if self.base_contains(*t) {
-                deleted_changed |= ov.deleted.insert(*t);
+                ov.deleted.insert(*t);
             }
         }
         for t in inserts {
```


## Current control ignore_tombstone_insert_flag executed output

Path: controls/ignore_tombstone_insert_flag.log; SHA256 4622c2e79b14f8ac2ea7a6acbeffe74b8ac1dbb99f5e2f3202b563318b6a9a8a

```text
   Compiling sparq-core v0.1.1 (/private/tmp/sparq-pr6049/.throughput-monitor/worktrees/issue4246/crates/sparq-core)
    Finished `test` profile [unoptimized] target(s) in 3.50s
     Running unittests src/lib.rs (/private/tmp/sparq-pr6049/.throughput-monitor/direct-5983/implementation/target/debug/deps/sparq_core-53b43ac9cb50e22c)

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

thread 'store::overlay_deleted_tests::deleted_cache_actual_tombstone_changes_invalidate_before_publication' (2399198) panicked at crates/sparq-core/src/store/overlay_deleted_tests.rs:162:9:
assertion failed: store.overlay.as_ref().unwrap().deleted_by_perm.iter().all(|s|
        s.get().is_none())
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace


failures:
    store::overlay_deleted_tests::deleted_cache_actual_tombstone_changes_invalidate_before_publication

test result: FAILED. 8 passed; 1 failed; 0 ignored; 0 measured; 146 filtered out; finished in 0.08s

error: test failed, to rerun pass `-p sparq-core --lib`
```


## Current control ignore_tombstone_remove_flag diff

Path: controls/ignore_tombstone_remove_flag.diff; SHA256 d5f3159ff55fef9d5f11b16effa59b1fd6549752e5bb5bbbd2c9af567fb7c450

```diff
--- store.rs
+++ ignore_tombstone_remove_flag
@@ -942,7 +942,7 @@
         }
         for t in inserts {
             if ov.deleted.remove(t) {
-                deleted_changed = true;
+                // mutant: ignore successful removal
                 continue; // re-insert of a deleted base triple: just undelete
             }
             if self.base_contains(*t) {
```


## Current control ignore_tombstone_remove_flag executed output

Path: controls/ignore_tombstone_remove_flag.log; SHA256 bd8e8f6bba2c0d3d2cdbdbfeda55667cd58dbc9384a557152f0475439cbd3318

```text
   Compiling sparq-core v0.1.1 (/private/tmp/sparq-pr6049/.throughput-monitor/worktrees/issue4246/crates/sparq-core)
    Finished `test` profile [unoptimized] target(s) in 3.00s
     Running unittests src/lib.rs (/private/tmp/sparq-pr6049/.throughput-monitor/direct-5983/implementation/target/debug/deps/sparq_core-53b43ac9cb50e22c)

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

thread 'store::overlay_deleted_tests::deleted_cache_actual_tombstone_changes_invalidate_before_publication' (2399538) panicked at crates/sparq-core/src/store/overlay_deleted_tests.rs:162:9:
assertion failed: store.overlay.as_ref().unwrap().deleted_by_perm.iter().all(|s|
        s.get().is_none())
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace


failures:
    store::overlay_deleted_tests::deleted_cache_actual_tombstone_changes_invalidate_before_publication

test result: FAILED. 8 passed; 1 failed; 0 ignored; 0 measured; 146 filtered out; finished in 0.10s

error: test failed, to rerun pass `-p sparq-core --lib`
```


## Current control remove_deleted_cache_use diff

Path: controls/remove_deleted_cache_use.diff; SHA256 a15285376df0a96eb89ee8a27a3f152003063d07b37e8cbdbcf1731cca53b39d

```diff
--- store.rs
+++ remove_deleted_cache_use
@@ -230,20 +230,11 @@
     /// Concurrent first readers of the same permutation wait for its one sorting
     /// initializer; the cold sort is serialized for that permutation.
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


## Current control remove_deleted_cache_use executed output

Path: controls/remove_deleted_cache_use.log; SHA256 dcdf9d106bc271003abf3b599d59ef9b67056201792a87739d9326f7c4098edf

```text
   Compiling sparq-core v0.1.1 (/private/tmp/sparq-pr6049/.throughput-monitor/worktrees/issue4246/crates/sparq-core)
    Finished `test` profile [unoptimized] target(s) in 3.11s
     Running unittests src/lib.rs (/private/tmp/sparq-pr6049/.throughput-monitor/direct-5983/implementation/target/debug/deps/sparq_core-53b43ac9cb50e22c)

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

thread '<unnamed>' (2399907) panicked at crates/sparq-core/src/store/overlay_deleted_tests.rs:292:34:
called `Option::unwrap()` on a `None` value
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace

thread '<unnamed>' (2399906) panicked at crates/sparq-core/src/store/overlay_deleted_tests.rs:292:34:
called `Option::unwrap()` on a `None` value

thread 'store::overlay_deleted_tests::deleted_cache_concurrent_first_reads_share_initialized_projection' (2399905) panicked at crates/sparq-core/src/store/overlay_deleted_tests.rs:299:64:
called `Result::unwrap()` on an `Err` value: Any { .. }

---- store::overlay_deleted_tests::deleted_cache_fork_and_clone_are_independent stdout ----

thread 'store::overlay_deleted_tests::deleted_cache_fork_and_clone_are_independent' (2399908) panicked at crates/sparq-core/src/store/overlay_deleted_tests.rs:227:14:
called `Option::unwrap()` on a `None` value

---- store::overlay_deleted_tests::deleted_cache_is_lazy_reused_and_accounted stdout ----

thread 'store::overlay_deleted_tests::deleted_cache_is_lazy_reused_and_accounted' (2399911) panicked at crates/sparq-core/src/store/overlay_deleted_tests.rs:59:14:
scan uses cached deletion count

---- store::overlay_deleted_tests::deleted_cache_survives_insert_only_and_noop_deltas stdout ----

thread 'store::overlay_deleted_tests::deleted_cache_survives_insert_only_and_noop_deltas' (2399913) panicked at crates/sparq-core/src/store/overlay_deleted_tests.rs:121:18:
called `Option::unwrap()` on a `None` value


failures:
    store::overlay_deleted_tests::deleted_cache_concurrent_first_reads_share_initialized_projection
    store::overlay_deleted_tests::deleted_cache_fork_and_clone_are_independent
    store::overlay_deleted_tests::deleted_cache_is_lazy_reused_and_accounted
    store::overlay_deleted_tests::deleted_cache_survives_insert_only_and_noop_deltas

test result: FAILED. 5 passed; 4 failed; 0 ignored; 0 measured; 146 filtered out; finished in 0.09s

error: test failed, to rerun pass `-p sparq-core --lib`
```


## Current control remove_deleted_invalidation diff

Path: controls/remove_deleted_invalidation.diff; SHA256 673664222be6d2c40eff0f0a054b29bef67083febf59db578cbf4248e809f973

```diff
--- store.rs
+++ remove_deleted_invalidation
@@ -953,7 +953,7 @@
             }
         }
         if deleted_changed {
-            ov.invalidate_deleted();
+            let _ = deleted_changed;
         }
         self.overlay = if ov.is_empty() { None } else { Some(ov) };
     }
```


## Current control remove_deleted_invalidation executed output

Path: controls/remove_deleted_invalidation.log; SHA256 af7fff40aa8b49fe38e9275aa98739bee97faaa7b9d5e3837be0141aef8391b2

```text
   Compiling sparq-core v0.1.1 (/private/tmp/sparq-pr6049/.throughput-monitor/worktrees/issue4246/crates/sparq-core)
warning: method `invalidate_deleted` is never used
   --> crates/sparq-core/src/store.rs:220:8
    |
185 | impl Overlay {
    | ------------ method in this implementation
...
220 |     fn invalidate_deleted(&mut self) {
    |        ^^^^^^^^^^^^^^^^^^
    |
    = note: `#[warn(dead_code)]` (part of `#[warn(unused)]`) on by default

warning: `sparq-core` (lib test) generated 1 warning
    Finished `test` profile [unoptimized] target(s) in 3.37s
     Running unittests src/lib.rs (/private/tmp/sparq-pr6049/.throughput-monitor/direct-5983/implementation/target/debug/deps/sparq_core-53b43ac9cb50e22c)

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

thread 'store::overlay_deleted_tests::deleted_cache_actual_tombstone_changes_invalidate_before_publication' (2398832) panicked at crates/sparq-core/src/store/overlay_deleted_tests.rs:162:9:
assertion failed: store.overlay.as_ref().unwrap().deleted_by_perm.iter().all(|s|
        s.get().is_none())
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace

---- store::overlay_deleted_tests::deleted_cache_compressed_base_matches_rebuild stdout ----

thread 'store::overlay_deleted_tests::deleted_cache_compressed_base_matches_rebuild' (2398835) panicked at crates/sparq-core/src/store/overlay_deleted_tests.rs:25:17:
assertion `left == right` failed
  left: 3
 right: 4

---- store::overlay_deleted_tests::deleted_cache_fork_and_clone_are_independent stdout ----

thread 'store::overlay_deleted_tests::deleted_cache_fork_and_clone_are_independent' (2398839) panicked at crates/sparq-core/src/store/overlay_deleted_tests.rs:264:9:
assertion `left == right` failed
  left: 1
 right: 0

---- store::overlay_deleted_tests::deleted_cache_graph_snapshot_retains_warm_generation stdout ----

thread 'store::overlay_deleted_tests::deleted_cache_graph_snapshot_retains_warm_generation' (2398840) panicked at crates/sparq-core/src/store/overlay_deleted_tests.rs:330:9:
assertion `left == right` failed
  left: 0
 right: 1

---- store::overlay_deleted_tests::deleted_cache_matches_rebuild_after_mixed_deltas stdout ----

thread 'store::overlay_deleted_tests::deleted_cache_matches_rebuild_after_mixed_deltas' (2398843) panicked at crates/sparq-core/src/store/overlay_deleted_tests.rs:25:17:
assertion `left == right` failed
  left: 3
 right: 4


failures:
    store::overlay_deleted_tests::deleted_cache_actual_tombstone_changes_invalidate_before_publication
    store::overlay_deleted_tests::deleted_cache_compressed_base_matches_rebuild
    store::overlay_deleted_tests::deleted_cache_fork_and_clone_are_independent
    store::overlay_deleted_tests::deleted_cache_graph_snapshot_retains_warm_generation
    store::overlay_deleted_tests::deleted_cache_matches_rebuild_after_mixed_deltas

test result: FAILED. 4 passed; 5 failed; 0 ignored; 0 measured; 146 filtered out; finished in 0.10s

error: test failed, to rerun pass `-p sparq-core --lib`
```


## Current control unconditional_deleted_invalidation diff

Path: controls/unconditional_deleted_invalidation.diff; SHA256 203f89435769dc017f1b77f8f7affb7c2748a9a2f1637c94150b791d05ef0a89

```diff
--- store.rs
+++ unconditional_deleted_invalidation
@@ -952,7 +952,7 @@
                 ov.added.insert(i, *t);
             }
         }
-        if deleted_changed {
+        if deleted_changed || !ov.deleted.is_empty() {
             ov.invalidate_deleted();
         }
         self.overlay = if ov.is_empty() { None } else { Some(ov) };
```


## Current control unconditional_deleted_invalidation executed output

Path: controls/unconditional_deleted_invalidation.log; SHA256 c6a1d410756675848cd771de170076b086ae7a7d1993fba4f06f5c3a2380f673

```text
   Compiling sparq-core v0.1.1 (/private/tmp/sparq-pr6049/.throughput-monitor/worktrees/issue4246/crates/sparq-core)
    Finished `test` profile [unoptimized] target(s) in 5.50s
     Running unittests src/lib.rs (/private/tmp/sparq-pr6049/.throughput-monitor/direct-5983/implementation/target/debug/deps/sparq_core-53b43ac9cb50e22c)

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

thread 'store::overlay_deleted_tests::deleted_cache_survives_insert_only_and_noop_deltas' (2398494) panicked at crates/sparq-core/src/store/overlay_deleted_tests.rs:136:22:
unchanged tombstones retain their projection
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace


failures:
    store::overlay_deleted_tests::deleted_cache_survives_insert_only_and_noop_deltas

test result: FAILED. 8 passed; 1 failed; 0 ignored; 0 measured; 146 filtered out; finished in 0.09s

error: test failed, to rerun pass `-p sparq-core --lib`
```


## Historical unchanged-path controls

These bodies are original acfa executions, not freshly rerun results. The current suite reruns the unchanged bound/heap tests; these older mutations demonstrate their sensitivity on acfa.


## Historical exclude_upper_bound diff

Path: prior-acfa-controls/exclude_upper_bound.diff; SHA256 cffeceb3f6396a157e31a1a0e7a75b97a54a27345545de46cb9b6332548e25f1

```diff
diff --git a/crates/sparq-core/src/store.rs b/crates/sparq-core/src/store.rs
index fdcf160e6..28a64e921 100644
--- a/crates/sparq-core/src/store.rs
+++ b/crates/sparq-core/src/store.rs
@@ -230,7 +230,7 @@ impl Overlay {
             rows.sort_unstable();
             rows
         });
-        rows.partition_point(|r| *r <= hi) - rows.partition_point(|r| *r < lo)
+        rows.partition_point(|r| *r < hi) - rows.partition_point(|r| *r < lo)
     }
 
     /// The `added` triples matching the inclusive `[lo, hi]` key range, as rows in
```


## Historical exclude_upper_bound actual output

Path: prior-acfa-controls/exclude_upper_bound.log; SHA256 7439f0ef19d7092cd8dc6c0e6120457a1d17c1b5694b84b41eef296cb65d48d4

```text
   Compiling sparq-core v0.1.1 (/private/tmp/sparq-pr6049/.throughput-monitor/worktrees/issue4246/crates/sparq-core)
    Finished `test` profile [unoptimized] target(s) in 2.54s
     Running unittests src/lib.rs (/private/tmp/sparq-pr6049/.throughput-monitor/direct-5983/implementation/target/debug/deps/sparq_core-53b43ac9cb50e22c)

running 7 tests
test store::overlay_deleted_tests::deleted_cache_compressed_base_matches_rebuild ... ok
test store::overlay_deleted_tests::deleted_cache_concurrent_first_reads_share_initialized_projection ... ok
test store::overlay_deleted_tests::deleted_cache_fork_and_clone_are_independent ... FAILED
test store::overlay_deleted_tests::deleted_cache_graph_snapshot_retains_warm_generation ... FAILED
test store::overlay_deleted_tests::deleted_cache_inclusive_bounds_and_empty_ranges ... FAILED
test store::overlay_deleted_tests::deleted_cache_is_lazy_reused_and_accounted ... ok
test store::overlay_deleted_tests::deleted_cache_matches_rebuild_after_mixed_deltas ... ok

failures:

---- store::overlay_deleted_tests::deleted_cache_fork_and_clone_are_independent stdout ----

thread 'store::overlay_deleted_tests::deleted_cache_fork_and_clone_are_independent' (2018723) panicked at crates/sparq-core/src/store/overlay_deleted_tests.rs:171:13:
assertion `left == right` failed
  left: 1
 right: 0
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace

---- store::overlay_deleted_tests::deleted_cache_graph_snapshot_retains_warm_generation stdout ----

thread 'store::overlay_deleted_tests::deleted_cache_graph_snapshot_retains_warm_generation' (2018724) panicked at crates/sparq-core/src/store/overlay_deleted_tests.rs:261:9:
assertion `left == right` failed
  left: 1
 right: 0

---- store::overlay_deleted_tests::deleted_cache_inclusive_bounds_and_empty_ranges stdout ----

thread 'store::overlay_deleted_tests::deleted_cache_inclusive_bounds_and_empty_ranges' (2018725) panicked at crates/sparq-core/src/store/overlay_deleted_tests.rs:117:13:
assertion `left == right` failed
  left: 0
 right: 1


failures:
    store::overlay_deleted_tests::deleted_cache_fork_and_clone_are_independent
    store::overlay_deleted_tests::deleted_cache_graph_snapshot_retains_warm_generation
    store::overlay_deleted_tests::deleted_cache_inclusive_bounds_and_empty_ranges

test result: FAILED. 4 passed; 3 failed; 0 ignored; 0 measured; 146 filtered out; finished in 0.04s

error: test failed, to rerun pass `-p sparq-core --lib`
```


## Historical omit_deleted_heap_accounting diff

Path: prior-acfa-controls/omit_deleted_heap_accounting.diff; SHA256 2db96862775163d1ed76887f4f0f8b6392703da2c463a09619b264c17f23c9e7

```diff
diff --git a/crates/sparq-core/src/store.rs b/crates/sparq-core/src/store.rs
index fdcf160e6..91351ba3b 100644
--- a/crates/sparq-core/src/store.rs
+++ b/crates/sparq-core/src/store.rs
@@ -294,7 +294,7 @@ impl Overlay {
         let cached: usize = self
             .added_by_perm
             .iter()
-            .chain(&self.deleted_by_perm)
+            
             .filter_map(|slot| slot.get())
             .map(|rows| rows.capacity() * std::mem::size_of::<[Id; 3]>())
             .sum();
```


## Historical omit_deleted_heap_accounting actual output

Path: prior-acfa-controls/omit_deleted_heap_accounting.log; SHA256 b6d776a4ac5da9c8dcec444b85b9bec0553ba278f1a01592e692d2edae5472dc

```text
   Compiling sparq-core v0.1.1 (/private/tmp/sparq-pr6049/.throughput-monitor/worktrees/issue4246/crates/sparq-core)
    Finished `test` profile [unoptimized] target(s) in 2.35s
     Running unittests src/lib.rs (/private/tmp/sparq-pr6049/.throughput-monitor/direct-5983/implementation/target/debug/deps/sparq_core-53b43ac9cb50e22c)

running 7 tests
test store::overlay_deleted_tests::deleted_cache_compressed_base_matches_rebuild ... ok
test store::overlay_deleted_tests::deleted_cache_concurrent_first_reads_share_initialized_projection ... ok
test store::overlay_deleted_tests::deleted_cache_fork_and_clone_are_independent ... ok
test store::overlay_deleted_tests::deleted_cache_graph_snapshot_retains_warm_generation ... ok
test store::overlay_deleted_tests::deleted_cache_inclusive_bounds_and_empty_ranges ... ok
test store::overlay_deleted_tests::deleted_cache_is_lazy_reused_and_accounted ... FAILED
test store::overlay_deleted_tests::deleted_cache_matches_rebuild_after_mixed_deltas ... ok

failures:

---- store::overlay_deleted_tests::deleted_cache_is_lazy_reused_and_accounted stdout ----

thread 'store::overlay_deleted_tests::deleted_cache_is_lazy_reused_and_accounted' (2019348) panicked at crates/sparq-core/src/store/overlay_deleted_tests.rs:72:9:
assertion `left == right` failed
  left: 39
 right: 87
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace


failures:
    store::overlay_deleted_tests::deleted_cache_is_lazy_reused_and_accounted

test result: FAILED. 6 passed; 1 failed; 0 ignored; 0 measured; 146 filtered out; finished in 0.04s

error: test failed, to rerun pass `-p sparq-core --lib`
```


## Review scope and unresolved admission questions

B1: confirm actual tombstone insert/remove results control only deleted invalidation and no stale cache read occurs before exclusive publication. Repeated no-ops and actual insert/remove are pinned by compiled mutants. B3: complete constructor/open/save span plus mutator/replacement/caller inventory is supplied. No storage/public API/Clone/Arc/threshold change.

B2/N3: assess the actual tradeoff, not just warmed medians. Read-only snapshots benefit on median but deep-copy more heap; some warm-six ranges overlap and outliers materially affect means. Actual tombstone-update generations and cold multi-pattern/two-reader points regress with disjoint timing ranges. Six warm projections copied by the first true-update fork are discarded before one replacement is initialized; later children carry only that requested projection. Multi-query live heap confirms three materialized projections, two concurrent readers retain one shared initialized projection.

All 620 recorded windows and generation-level phases/variance/counts/live-growth/RSS are in paired/*.jsonl and paired/summary.json. No timing unit assertions or post-hoc sample selection. This fixed run does not establish production read/write ratios or tail percentiles. Retained heap includes initial graph plus four retained outputs, subtracting shared immutable base; boxed metadata/hash allocation estimates are distinct from allocator-requested bytes. Process RSS is a cumulative setup-inclusive high water mark, not query heap.

Full workspace/wasm/W3C/canonical perf, mmap/WAL crash execution and Miri were not run. Preflight cannot complete under local Bash3 mapfile; grants/checks remain unchanged. Independent review and root coordination are required before publication/admission. No remote writes or separate model calls were performed.
