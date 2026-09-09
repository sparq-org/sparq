# Option C experiment: NO-GO for top-k admission

Frozen local HEAD `d08295b6c06ca6f6829de82a210708bab8e44a28`, parent `dc23141d8c9e640e599085b0ed1616da35d8fc87`. Actual GPT-6 Astra xhigh. This is the single authorized lazy-preparation experiment; no new model review or remote mutation. Stop implementation/measurement after this report.

The patch delays each prepared scan/vector until a candidate reaches its pattern. Pattern order and all existing block/tie/failure/shape/budget/view/fanout rules remain; per-probe sort/cardinality guards run before use. A successful row must visit every probe. Unvisited exclusions conservatively fall back. No new storage API, point scout or threshold. Main bind_join is byte-identical.

Default focused validation:48 integration +3 preparation tests. Compact-index:31 integration +3 preparation tests. Three actual mutants were killed: eager initialization, omitted late cardinality, and omitted unvisited exclusion. Restored controls pass. The initial fixture update separator error was corrected before commit and preserved in its log. Author preflight has only the known Bash3 mapfile limitation and is not presented as green.

Eager control restores800,000 requested bytes/4 allocations in drained base and3,040,000 bytes/8 allocations in drained overlay. Favorable new/eager metrics are equal. Against dc, larger metadata adds144 bytes on admitted fixtures. Query heap peaks in the drained fallback cases do not improve; the removed preparation had already been freed before fallback peak.

All180 processes /578 measured query samples passed their result/path checks (168 matrix processes/560 samples plus12 allocation-control processes/18 samples). Identical extended harness compiled against all four sources; nine optimized binaries fit under the15-minute total cap. One sequential lane, two build jobs, Rayon1, warmup2, timing7/count3 repetitions. All samples retained.

| Fixture | New ms | dc ms | Disabled ms | Main ms | New/dc | New/disabled |
|---|---:|---:|---:|---:|---:|---:|
| base k1 | 0.547 | 0.549 | 26.122 | 14.385 | 0.996 | 0.021 |
| base k512 | 1.223 | 1.202 | 14.394 | 17.976 | 1.017 | 0.085 |
| overlay k1 | 5.994 | 5.709 | 17.982 | 17.895 | 1.050 | 0.333 |
| overlay k512 | 6.443 | 6.439 | 18.179 | 18.304 | 1.001 | 0.354 |
| drained k1 | 10.561 | 10.939 | 8.609 | 8.367 | 0.965 | 1.227 |
| drained-overlay k1 | 17.849 | 21.043 | 13.765 | 14.463 | 0.848 | 1.297 |
| ties k1 | 16.560 | 16.278 | 14.505 | 14.616 | 1.017 | 1.142 |
| ties-overlay k1 | 20.732 | 20.626 | 17.938 | 17.968 | 1.005 | 1.156 |
| prefix128 k1 | 0.587 | 0.581 | 13.402 | 13.443 | 1.010 | 0.044 |
| prefix128-overlay k1 | 5.875 | 5.741 | 17.955 | 17.890 | 1.023 | 0.327 |
| prefix800 k1 | 0.648 | 0.639 | 13.442 | 13.676 | 1.014 | 0.048 |
| prefix800-overlay k1 | 5.920 | 5.816 | 17.697 | 17.732 | 1.018 | 0.335 |
| intermittent k1 | 0.545 | 0.537 | 12.706 | 12.742 | 1.016 | 0.043 |
| intermittent-overlay k1 | 6.012 | 5.708 | 17.541 | 17.562 | 1.053 | 0.343 |

The material drained penalty remains (~1.23× base and1.30× overlay versus disabled, disjoint ranges). Versus dc, base-k512(+1.7%), overlay-k1(+5.0%) and prefix128-overlay(+2.3%) also have slower, disjoint within-process ranges; intermittent-overlay(+5.3%) overlaps. Tie medians remain1.14×/1.16× disabled. First base-k1 disabled/main timing differs1.82× despite identical allocation metrics, so causal precision is limited. These observations reinforce the stop; benefits elsewhere do not authorize admission. Timing ratios are local diagnostic observations; full ranges/standard deviations and baseline drift are in summary.json. No noisy-run confirmation or broader sweep was performed.

Whole-query timing excludes result checking/drop. Allocation counting uses a separate binary and measures requested traffic/live heap, not allocator overhead. RSS is cumulative process high-water after setup/warmup/query, not an isolated query-only peak. Source/harness/binary hashes, raw outputs and exact build commands accompany this packet. The old semantic, diagnostic, early-tie and design artifacts remain immutable.

Next lane recommendation: after root dedupe, investigate ordinary SPARQL scan/bind-join tombstone range-counting cost separately. No further top-k design/implementation iteration is authorized here. F3–F5 remain unresolved.

## Combined runtime delta from the independently reviewed semantic ancestor

```diff
diff --git a/crates/sparq-engine/src/exec.rs b/crates/sparq-engine/src/exec.rs
index c0f63fb14..f2ad08e45 100644
--- a/crates/sparq-engine/src/exec.rs
+++ b/crates/sparq-engine/src/exec.rs
@@ -3333,7 +3333,22 @@ fn try_topk_orderby_indexed(
         return Ok(None);
     }
 
-    // Prepare the other patterns ONCE, each as a SUBJECT-sorted scan over its
+    // [GPT-6 Astra] The first block must include its entire leading key group.
+    // Reject a group larger than the EXISTING block cap before retaining probe
+    // scans and subject vectors. Sorted object ids make equality at the cap's
+    // zero-based edge equivalent to group_len > max_group, in either direction.
+    const MAX_INDEXED_GROUP_FLOOR: usize = 256;
+    let n = rows.len();
+    let max_group = (n / 2).max(MAX_INDEXED_GROUP_FLOOR).max(row_budget.saturating_mul(2));
+    if n > max_group {
+        let first = if desc { n - 1 } else { 0 };
+        let edge = if desc { first - max_group } else { max_group };
+        if scan.to_spo(&rows[first])[2] == scan.to_spo(&rows[edge])[2] {
+            return Ok(None);
+        }
+    }
+
+    // Prepare the other patterns ONCE ON DEMAND, each as a SUBJECT-sorted scan over its
     // (fixed predicate [+ fixed object]) range — not re-resolved per candidate.
     // The first cut of this function called `graph.store.scan(&probe_pat)`
     // fresh for every (candidate, other-pattern) pair, which re-runs
@@ -3344,7 +3359,7 @@ fn try_topk_orderby_indexed(
     // fallback's bulk merge-join (which resolves the permutation ONCE per
     // pattern, not once per row). Resolving it once here and binary-searching
     // the resulting sorted slice per candidate removes that repeated cost.
-    struct OtherPat<'g> {
+    struct OtherScan<'g> {
         scan: sparq_core::store::Scan<'g>,
         // Precomputed ONCE (not per candidate, not per binary-search
         // comparison step): the subject id of every row in `scan`, in the
@@ -3357,7 +3372,34 @@ fn try_topk_orderby_indexed(
         // full [S,P,O] triple just to read column 0 is wasted work when the
         // search only ever needs that one column.
         subject_ids: Vec<Id>,
+    }
+    // [GPT-6 Astra] Resolve metadata eagerly, but retain a scan/vector only when
+    // a candidate reaches this pattern in the original order. A failed constant
+    // prefix therefore does not prepare later variable probes that it never uses.
+    struct OtherPat<'g> {
+        id_pat: IdPattern,
         obj_var: Option<Variable>,
+        prepared: Option<OtherScan<'g>>,
+    }
+    fn prepare_other<'g>(graph: &'g Graph, op: &mut OtherPat<'g>, seed_card: usize) -> bool {
+        if op.prepared.is_some() {
+            return true;
+        }
+        let sub_scan = graph.store.scan_sorted(&op.id_pat, 0);
+        let sub_actual_sort = sub_scan.perm.order().into_iter().find(|&c| op.id_pat[c].is_none());
+        if sub_actual_sort != Some(0) {
+            return false;
+        }
+        // The original exact-cardinality admission rule still applies to EVERY
+        // probe before it can contribute to a row, including a late selective one.
+        if sub_scan.rows.len().saturating_mul(2) < seed_card {
+            return false;
+        }
+        let subject_ids: Vec<Id> = sub_scan.rows.iter().map(|r| sub_scan.to_spo(r)[0]).collect();
+        #[cfg(test)]
+        indexed_topk_preparation_tests::observe(subject_ids.len());
+        op.prepared = Some(OtherScan { scan: sub_scan, subject_ids });
+        true
     }
     let mut other_pats: Vec<OtherPat> = Vec::with_capacity(patterns.len().saturating_sub(1));
     for (i, tp) in patterns.iter().enumerate() {
@@ -3370,43 +3412,17 @@ fn try_topk_orderby_indexed(
             // WHOLE conjunction empty (a BGP join against an empty relation).
             return Ok(Some(Bindings::unsorted(out_vars, vec![])));
         }
-        let probe_id_pat: IdPattern = [None, id_pat[1], id_pat[2]];
-        let sub_scan = graph.store.scan_sorted(&probe_id_pat, 0);
-        let sub_actual_sort = sub_scan.perm.order().into_iter().find(|&c| probe_id_pat[c].is_none());
-        if sub_actual_sort != Some(0) {
-            // This store build can't give a subject-sorted scan for this
-            // pattern (e.g. no PSO permutation under `compact-index`/wasm) —
-            // decline rather than binary-search an unsorted range.
-            return Ok(None);
-        }
-        let subject_ids: Vec<Id> = sub_scan.rows.iter().map(|r| sub_scan.to_spo(r)[0]).collect();
-        other_pats.push(OtherPat { scan: sub_scan, subject_ids, obj_var: pos_vars[2].clone() });
+        other_pats.push(OtherPat {
+            id_pat: [None, id_pat[1], id_pat[2]],
+            obj_var: pos_vars[2].clone(),
+            prepared: None,
+        });
     }
 
-    // UPFRONT cost check, before touching a single candidate: each `other_pats`
-    // scan's row count is the EXACT (not estimated) global cardinality of that
-    // pattern's own (predicate [+ object]) constraint. If any of them is
-    // already meaningfully smaller than the seed's own scan (`rows.len()`),
-    // the fallback's ordinary smallest-estimate seed selection will pick THAT
-    // pattern as ITS seed and materialize only that small set — beating this
-    // function's priority-ordered walk outright, with no reason to compete.
-    //
-    // This is exactly the realistic "claim strictly in priority order" shape:
-    // as such a queue drains, `ak:status="pending"` becomes highly selective
-    // while THIS function's seed (`ak:priority`, spanning the whole pool
-    // including now-claimed rows) does not shrink at all. Without this check,
-    // the only way to discover that is by actually walking past the
-    // ever-growing already-claimed prefix, probing (and rejecting) each one —
-    // real, wasted, unrecoverable cost. Measured directly: at n=1600 with
-    // 1000 of 1600 already claimed (600 truly pending), that reactive
-    // discovery cost ~237-272us total (wasted probes + the fallback anyway)
-    // vs. this upfront check's ~172-180us (matches a clean fallback-only
-    // cost, because it declines before doing ANY per-candidate work).
-    let seed_card = rows.len();
-    let min_other_card = other_pats.iter().map(|op| op.scan.rows.len()).min().unwrap_or(seed_card);
-    if min_other_card.saturating_mul(2) < seed_card {
-        return Ok(None);
-    }
+    // [GPT-6 Astra] The previous global min-cardinality check is now applied
+    // by prepare_other to each exact scan count before that probe is used. A
+    // successful row must visit every pattern, so it passes the same admission
+    // rule. Unvisited patterns stay unproved and cannot authorize an empty result.
 
     // The output column list is FIXED across every candidate (hub, order, then
     // each other pattern's object variable, in pattern order) — compute it and
@@ -3440,7 +3456,6 @@ fn try_topk_orderby_indexed(
     // position 0 is always the best candidate, via `logical`, and do all
     // block/boundary arithmetic in that space. `logical` and `obj_id_at` are the
     // only direction-aware code; everything below them is direction-agnostic.
-    let n = rows.len();
     let logical = |i: usize| -> usize { if desc { n - 1 - i } else { i } };
     let obj_id_at = |i: usize| -> Id { scan.to_spo(&rows[logical(i)])[2] };
     // A single escalation block is dominated by ONE large tie-group when the
@@ -3455,12 +3470,10 @@ fn try_topk_orderby_indexed(
     // (half of n) still beat the fallback (~228us vs. the fallback's ~269us),
     // but a tie-group of 1600 (all of n) lost (would be ~450-700us vs. the
     // fallback's own ~269us) — and the same ~0.5-0.75 fraction held at n=8000
-    // (4000 still competitive, 8000 clearly lost). `max_group` below is
+    // (4000 still competitive, 8000 clearly lost). `max_group` is
     // therefore `n / 2` (with a floor for small `n`, and never below
-    // `row_budget` itself) — declining past it defers to the fallback, whose
-    // cost at that point is exactly its normal (tie-structure-independent)
-    // cost, not a new regression.
-    const MAX_INDEXED_GROUP_FLOOR: usize = 256;
+    // `row_budget` itself) — declining past it defers to the fallback, with
+    // any preparation and failed probes already performed adding to its cost.
     let mut collected: Vec<Row> = Vec::new();
     let mut visited_to: usize = 0;
     // Block sizes grow GEOMETRICALLY from a small multiple of `row_budget`, not
@@ -3473,7 +3486,6 @@ fn try_topk_orderby_indexed(
     // usually satisfies it — starting at 1024 would pay ~1024 point-probes even for
     // `LIMIT 1`, which is worse than the bulk path it's meant to beat (measured:
     // this was the actual cause of a regression at moderate `n`, not a win).
-    let max_group = (n / 2).max(MAX_INDEXED_GROUP_FLOOR).max(row_budget.saturating_mul(2));
     // Cumulative count of candidates that failed an OTHER-pattern check, across
     // ALL blocks in this call — distinct from `max_group`'s per-block width
     // check. A workload that claims strictly in priority order (the realistic
@@ -3483,10 +3495,10 @@ fn try_topk_orderby_indexed(
     // being rejected. That prefix is made of individually DISTINCT priority
     // values, so it never forms one oversized tie-group `max_group` would
     // catch; it spreads across many small geometric-growth blocks instead. The
-    // UPFRONT cost check above (comparing `other_pats`' exact cardinalities to
+    // per-pattern cost check (comparing exact scan cardinalities to
     // the seed's) already declines the CLEAR case — an other-pattern that's
     // globally selective enough for the fallback's own planner to prefer as
-    // ITS seed — before any candidate is even touched. This counter is a
+    // ITS seed — before that probe is used. This counter is a
     // SAFETY NET for what that check can't see: the seed's global cardinality
     // vs. an other-pattern's global cardinality doesn't capture every
     // possible skip-prefix shape (e.g. a correlation between scan order and
@@ -3534,14 +3546,18 @@ fn try_topk_orderby_indexed(
             // General Cartesian expansion remains with the existing evaluator.
             let mut row_ids: SmallVec<[Id; 8]> = SmallVec::from_slice(&[hub_id, order_id]);
             let mut failed = false;
-            for op in &other_pats {
+            for op in &mut other_pats {
+                if !prepare_other(graph, op, rows.len()) {
+                    return Ok(None);
+                }
+                let prepared = op.prepared.as_ref().expect("successful preparation");
                 // Binary-search the PRECOMPUTED, plain-`Id` subject list for
                 // this pattern (built once, above) — a trivial integer
                 // compare per step, no `to_spo` reconstruction during the
                 // search itself (that only happens below, per ACTUAL match,
                 // not per comparison step — see `subject_ids`'s doc comment).
-                let start = op.subject_ids.partition_point(|&id| id < hub_id);
-                let stop = start + op.subject_ids[start..].partition_point(|&id| id == hub_id);
+                let start = prepared.subject_ids.partition_point(|&id| id < hub_id);
+                let stop = start + prepared.subject_ids[start..].partition_point(|&id| id == hub_id);
                 if start == stop {
                     failed = true;
                     break;
@@ -3549,12 +3565,12 @@ fn try_topk_orderby_indexed(
                 let Some(_) = &op.obj_var else {
                     continue; // `obj_const` — existence-only, no column added
                 };
-                let op_rows: &[[Id; 3]] = op.scan.rows.as_ref();
+                let op_rows: &[[Id; 3]] = prepared.scan.rows.as_ref();
                 let match_count = stop - start;
                 if match_count != 1 {
                     return Ok(None);
                 }
-                row_ids.push(op.scan.to_spo(&op_rows[start])[2]);
+                row_ids.push(prepared.scan.to_spo(&op_rows[start])[2]);
             }
             if failed {
                 failed_count += 1;
@@ -3572,12 +3588,83 @@ fn try_topk_orderby_indexed(
         block_target = block_target.saturating_mul(4).max(visited_to + 1);
     }
 
+    // [GPT-6 Astra] An empty walk may never visit later patterns. Preserve their
+    // unproved static-sort/cardinality exclusions through the existing fallback.
+    if other_pats.iter().any(|op| op.prepared.is_none()) {
+        return Ok(None);
+    }
     let mut result = Bindings { vars: out_vars, rows: collected, sorted_by: None };
     let use_topk = result.rows.len() > row_budget;
     order_bindings(graph, local, &mut result, expression, if use_topk { Some(row_budget) } else { None })?;
     Ok(Some(result))
 }
 
+// [GPT-6 Astra] Test-only observations pin retained preparation work, not timing.
+#[cfg(test)]
+mod indexed_topk_preparation_tests {
+    use sparq_core::Graph;
+    use std::cell::Cell;
+    use std::fmt::Write;
+
+    thread_local! { static PREPARED: Cell<(usize, usize)> = const { Cell::new((0, 0)) }; }
+    pub(super) fn observe(rows: usize) {
+        PREPARED.with(|c| { let (scans, ids) = c.get(); c.set((scans + 1, ids + rows)); });
+    }
+    const TEXT: &str = "SELECT ?s WHERE { ?s <urn:peer> <urn:X> ; <urn:status> \"pending\" ; <urn:priority> ?p ; <urn:seq> ?seq ; <urn:a> ?a ; <urn:b> ?b ; <urn:c> ?c } ORDER BY DESC(?p)";
+
+    fn graph(prefix: usize, overlay: bool, selective_last: bool) -> Graph {
+        let mut ttl = String::new();
+        for i in 0..4096 {
+            let status = if i < 4096 - prefix { "pending" } else { "done" };
+            writeln!(ttl, "<urn:s{i}> <urn:peer> <urn:X> ; <urn:status> \"{status}\" ; <urn:priority> {i} ; <urn:seq> {i} ; <urn:a> {i} ; <urn:b> {i} .").unwrap();
+            if !selective_last || i == 4095 { writeln!(ttl, "<urn:s{i}> <urn:c> {i} .").unwrap(); }
+        }
+        let base = Graph::load_str(&ttl, "turtle").unwrap();
+        if !overlay { return base; }
+        let mut changed = base.fork();
+        let mut deletes = String::from("DELETE DATA {");
+        for i in (0..4096).step_by(5) { writeln!(deletes, "<urn:s{i}> <urn:priority> {i} .").unwrap(); }
+        deletes.push('}');
+        crate::update_in_place(&mut changed, &deletes).unwrap();
+        assert_eq!(changed.pending_delta_len(), 820);
+        changed
+    }
+
+    #[test]
+    fn later_variable_preparation_is_avoided_before_block_decline() {
+        for overlay in [false, true] {
+            let g = graph(2047, overlay, false);
+            let full = crate::query(&g, TEXT).unwrap();
+            PREPARED.with(|c| c.set((0, 0)));
+            let got = crate::query(&g, &format!("{TEXT} LIMIT 1")).unwrap();
+            assert_eq!(got.rows, full.rows[..1]);
+            assert_eq!(PREPARED.with(Cell::get), (2, 4096 + 2049), "later variable scans/vectors must remain unprepared");
+        }
+    }
+
+    #[test]
+    fn successful_rows_prepare_each_probe_once() {
+        let g = graph(0, false, false);
+        for k in [1, 32] {
+            PREPARED.with(|c| c.set((0, 0)));
+            assert_eq!(crate::query(&g, &format!("{TEXT} LIMIT {k}")).unwrap().rows.len(), k);
+            let expected = if sparq_core::store::BUILT.contains(&sparq_core::store::Perm::Pso) { (6, 6 * 4096) } else { (2, 2 * 4096) };
+            assert_eq!(PREPARED.with(Cell::get), expected);
+        }
+    }
+
+    #[test]
+    fn late_selective_probe_still_declines_before_emitting() {
+        let g = graph(0, false, true);
+        let full = crate::query(&g, TEXT).unwrap();
+        assert_eq!(full.rows.len(), 1);
+        let limited = format!("{TEXT} LIMIT 1");
+        assert_eq!(crate::query(&g, &limited).unwrap().rows, full.rows);
+        let trace = crate::explain_analyze(&g, &limited).unwrap();
+        assert!(trace.contains("BGP [binary GOO]"), "late cardinality guard must decline: {trace}");
+    }
+}
+
 /// [OPUS-4.8] (sq-7d3dj.30.4) Attempts the DISTINCT-projection loose skip-scan for the
 /// pattern under a `Distinct`.
 ///
diff --git a/crates/sparq-engine/tests/topk_orderby_indexed_differential.rs b/crates/sparq-engine/tests/topk_orderby_indexed_differential.rs
index 6a56a2b75..48e4d395e 100644
--- a/crates/sparq-engine/tests/topk_orderby_indexed_differential.rs
+++ b/crates/sparq-engine/tests/topk_orderby_indexed_differential.rs
@@ -13,6 +13,52 @@ use sparq_engine::query;
 
 const PFX: &str = "PREFIX ak: <http://example.org/ak#>\n";
 
+// [GPT-6 Astra] Delayed preparation must preserve modest/intermittent misses,
+// original pattern order, complete key groups, and OFFSET in either direction.
+#[test]
+fn lazy_probes_preserve_miss_patterns_and_overlay_windows() {
+    use std::fmt::Write;
+    for desc in [false, true] {
+        for prefix in [128, 800, 0] {
+            let mut ttl = String::new();
+            for i in 0..2048 {
+                let rank = if desc { 2047 - i } else { i };
+                let pending = if prefix == 0 { rank % 8 != 0 } else { rank >= prefix };
+                let status = if pending { "pending" } else { "done" };
+                writeln!(ttl, "<urn:s{i}> <urn:p> {} ; <urn:status> \"{status}\" ; <urn:seq> {i} .", i / 3).unwrap();
+            }
+            let base = Graph::load_str(&ttl, "turtle").unwrap();
+            let mut changed = base.fork();
+            sparq_engine::update_in_place(&mut changed, "DELETE DATA { <urn:s1024> <urn:p> 341 . <urn:s1025> <urn:p> 341 . }; INSERT DATA { <urn:extra> <urn:p> 400 ; <urn:status> \"pending\" ; <urn:seq> 9999 . }").unwrap();
+            for graph in [&base, &changed] {
+                for variable_first in [false, true] {
+                    let other = if variable_first { "?s <urn:seq> ?seq ; <urn:status> \"pending\"" } else { "?s <urn:status> \"pending\" ; <urn:seq> ?seq" };
+                    let direction = if desc { "DESC" } else { "ASC" };
+                    let text = format!("SELECT ?s WHERE {{ ?s <urn:p> ?p . {other} }} ORDER BY {direction}(?p) DESC(?seq)");
+                    let full = query(graph, &text).unwrap();
+                    let limited = format!("{text} OFFSET 2 LIMIT 4");
+                    assert_eq!(query(graph, &limited).unwrap().rows, full.rows[2..6], "prefix={prefix}, desc={desc}, variable_first={variable_first}");
+                }
+            }
+        }
+    }
+}
+
+// [GPT-6 Astra] An all-rejected walk cannot silently waive unvisited exclusions.
+#[test]
+fn unvisited_lazy_probe_keeps_empty_result_on_fallback() {
+    use std::fmt::Write;
+    let mut ttl = String::new();
+    for i in 0..32 {
+        writeln!(ttl, "<urn:s{i}> <urn:p> {i} ; <urn:later> 1, 2 . <urn:foreign{i}> <urn:status> \"pending\" .").unwrap();
+    }
+    let graph = Graph::load_str(&ttl, "turtle").unwrap();
+    let text = "SELECT ?s WHERE { ?s <urn:p> ?p ; <urn:status> \"pending\" ; <urn:later> ?v } ORDER BY DESC(?p) LIMIT 1";
+    assert!(query(&graph, text).unwrap().rows.is_empty());
+    let trace = sparq_engine::explain_analyze(&graph, text).unwrap();
+    assert!(trace.contains("BGP [binary GOO]"), "unvisited probe must not authorize indexed admission: {trace}");
+}
+
 // [GPT-6 Astra] A repeated variable must bind one term in both positions.
 // An IRI subject and an integer object can never satisfy this triple pattern.
 #[test]
@@ -96,6 +142,66 @@ fn exact_desc_ties_preserve_valid_membership_and_window_size() {
     }
 }
 
+// [GPT-6 Astra] Exactly-at-cap groups remain eligible; one more must decline.
+// A distinguishing secondary key makes the OFFSET oracle deterministic.
+#[test]
+fn leading_tie_cap_boundaries_preserve_both_directions_and_offset() {
+    let supported = sparq_core::store::BUILT.contains(&sparq_core::store::Perm::Pso);
+    for (n, cap) in [(400, 256), (600, 300)] {
+        for desc in [false, true] {
+            for group in [cap, cap + 1] {
+                let tasks: Vec<_> = (0..n).map(|i| {
+                    let key = if i < group { if desc { 1000 } else { 0 } } else { i + 1 };
+                    (i, key)
+                }).collect();
+                let graph = build_graph("X", &tasks, "");
+                let direction = if desc { "DESC" } else { "ASC" };
+                let text = format!("{PFX}SELECT ?t WHERE {{ ?t ak:peer <urn:peer:X> ; ak:status \"pending\" ; ak:priority ?p ; ak:seq ?s }} ORDER BY {direction}(?p) DESC(?s)");
+                let full = query(&graph, &text).unwrap();
+                let limited = format!("{text} OFFSET 1 LIMIT 1");
+                assert_eq!(query(&graph, &limited).unwrap().rows, full.rows[1..2]);
+                let trace = sparq_engine::explain_analyze(&graph, &limited).unwrap();
+                assert_eq!(trace.contains("BGP [binary GOO]"), group > cap || !supported,
+                           "n={n} group={group} direction={direction}: {trace}");
+            }
+        }
+    }
+}
+
+// [GPT-6 Astra] The cap still includes twice the requested row budget.
+#[test]
+fn leading_tie_cap_preserves_large_row_budget_admission() {
+    let tasks: Vec<_> = (0..600).map(|i| (i, if i < 400 { 1000 } else { i })).collect();
+    let graph = build_graph("X", &tasks, "");
+    assert_eq!(actual_top_k(&graph, 300), expected_top_k(&graph, 300));
+    let trace = sparq_engine::explain_analyze(&graph, &format!("{PFX}{CLAIM_QUERY}300")).unwrap();
+    let supported = sparq_core::store::BUILT.contains(&sparq_core::store::Perm::Pso);
+    assert_eq!(!trace.contains("BGP [binary GOO]"), supported, "{trace}");
+}
+
+// [GPT-6 Astra] Tombstones alter both the group extent and the existing n/2 cap.
+#[test]
+fn leading_tie_cap_observes_overlay_deletions() {
+    for desc in [false, true] {
+        let key = if desc { 1000 } else { 0 };
+        let tasks: Vec<_> = (0..600).map(|i| (i, if i < 301 { key } else { i + 1 })).collect();
+        let base = build_graph("X", &tasks, "");
+        let mut changed = base.fork();
+        sparq_engine::update_in_place(&mut changed, &format!("{PFX}DELETE DATA {{ <urn:task:X:0> ak:priority {key} . <urn:task:X:1> ak:priority {key} }}")).unwrap();
+        assert_eq!(changed.pending_delta_len(), 2);
+        let direction = if desc { "DESC" } else { "ASC" };
+        let text = format!("{PFX}SELECT ?t WHERE {{ ?t ak:peer <urn:peer:X> ; ak:status \"pending\" ; ak:priority ?p ; ak:seq ?s }} ORDER BY {direction}(?p) DESC(?s)");
+        let limited = format!("{text} OFFSET 1 LIMIT 1");
+        let full = query(&changed, &text).unwrap();
+        assert_eq!(query(&changed, &limited).unwrap().rows, full.rows[1..2]);
+        let before = sparq_engine::explain_analyze(&base, &limited).unwrap();
+        let after = sparq_engine::explain_analyze(&changed, &limited).unwrap();
+        assert!(before.contains("BGP [binary GOO]"), "original 301 > 300: {before}");
+        let supported = sparq_core::store::BUILT.contains(&sparq_core::store::Perm::Pso);
+        assert_eq!(!after.contains("BGP [binary GOO]"), supported, "remaining 299 == 598/2: {after}");
+    }
+}
+
 // [GPT-6 Astra] Each fixture has distinct numeric values, avoiding unspecified ties.
 #[test]
 fn negative_mixed_and_typed_lexical_values_use_the_fallback() {

```

## Exact harness extension from the prior diagnostic harness

```diff
diff --git a/bench/indexed-topk/README.md b/bench/indexed-topk/README.md
index 07abb5383..70dbafb00 100644
--- a/bench/indexed-topk/README.md
+++ b/bench/indexed-topk/README.md
@@ -15,3 +15,10 @@ Timing covers the full `sparq_engine::query` call, including parse, evaluation a
 RSS is the OS cumulative process high-water mark, recorded after fixture construction and before/after measured queries. Setup can dominate it, so a zero increase cannot establish zero query memory. Allocation-window peaks provide a separate query-specific requested-heap estimate. Start each case/mode in a fresh process, retain setup and query observations separately, and report this limitation rather than relabeling cumulative RSS as a resettable query peak.
 
 Warm up, retain every repetition, and report dispersion rather than best-of-only results. Preserve host/toolchain/source/harness/binary hashes and fixture dimensions alongside raw output. The allocator is a minimal forwarding wrapper inspired by the existing harness; calibration does not substitute for Miri or independent review. No speedup, allocation reduction or pre-admission clearance is established by adding this harness.
+
+[GPT-6 Astra] The lazy-preparation experiment adds `prefix128`, `prefix800`, and
+`intermittent`, each with an `-overlay` variant. Prefixes count live rows after
+deletions; intermittent fixtures reject every eighth live priority rank, starting
+with the best row. Existing cases, whole-query window, oracle, features and
+allocator remain unchanged. Build this exact extended harness against each
+comparison source; old binaries with fewer fixture cases are not comparable.
diff --git a/bench/indexed-topk/src/main.rs b/bench/indexed-topk/src/main.rs
index 5770186cf..6e5c4c7db 100644
--- a/bench/indexed-topk/src/main.rs
+++ b/bench/indexed-topk/src/main.rs
@@ -42,18 +42,26 @@ fn fixture(case: &str, n: usize, k: usize) -> Fixture {
     assert!(matches!(
         case,
         "base" | "overlay" | "drained" | "drained-overlay" | "ties" | "ties-overlay"
+            | "prefix128" | "prefix128-overlay" | "prefix800" | "prefix800-overlay"
+            | "intermittent" | "intermittent-overlay"
     ));
     assert!(n >= 1024 && n <= 100_000 && k > 0 && k <= 1024);
     let overlay = case.contains("overlay");
     let drained = case.starts_with("drained");
     let ties = case.starts_with("ties");
+    // [GPT-6 Astra] Predeclared miss fixtures use LIVE rank so the overlay
+    // has exactly the same leading miss count after its every-fifth deletions.
+    let prefix = if case.starts_with("prefix128") { 128 } else if case.starts_with("prefix800") { 800 } else { 0 };
+    let intermittent = case.starts_with("intermittent");
     let mut ttl = String::new();
     let mut deletes = Vec::new();
     let mut expected = Vec::new();
     let (mut seed_rows, mut pending_rows, mut tied_rows) = (0, 0, 0);
     for i in 0..n {
         let p = priority(i, n, ties);
-        let status = if pending(i, n, drained) {
+        let live_rank = n - 1 - i - if overlay { (n - 1) / 5 - i / 5 } else { 0 };
+        let is_pending = if prefix > 0 { live_rank >= prefix } else if intermittent { live_rank % 8 != 0 } else { pending(i, n, drained) };
+        let status = if is_pending {
             "pending"
         } else {
             "done"
@@ -75,7 +83,7 @@ fn fixture(case: &str, n: usize, k: usize) -> Fixture {
         } else {
             seed_rows += 1;
             tied_rows += usize::from(ties && p == n + 1);
-            if pending(i, n, drained) {
+            if is_pending {
                 pending_rows += 1;
                 expected.push((p, i));
             }
@@ -97,6 +105,12 @@ fn fixture(case: &str, n: usize, k: usize) -> Fixture {
         assert!(pending_rows * 2 >= seed_rows);
         assert!(pending_rows * 2 - seed_rows <= 4);
     }
+    if prefix > 0 {
+        assert_eq!(seed_rows - pending_rows, prefix);
+    }
+    if intermittent {
+        assert_eq!(seed_rows - pending_rows, seed_rows.div_ceil(8));
+    }
     if ties {
         assert!(tied_rows > (seed_rows / 2).max(256).max(2 * k));
     }

```

## Complete affected functions, tests and extended measurement harness

### crates/sparq-engine/src/exec.rs:3110

```rust
/// Attempts the bounded-heap ORDER BY optimisation for a `Slice` with a known
/// row budget `k = offset + limit`.
///
/// Peeks through `Project` / `Reduced` transparent wrappers to find an `OrderBy`
/// node. When found, evaluates the pattern below the `OrderBy`, sorts using
/// `order_bindings` with the top-k hint so only `k` rows survive, applies any
/// projection, and returns `Some(Bindings)`. Returns `None` when no `OrderBy`
/// is present at the transparent-wrapper boundary — the caller falls through to
/// full `eval_modified`. [SONNET-4.6] sq-7d3dj.30.2
fn try_topk_orderby(
    graph: &Graph,
    local: &mut LocalVocab,
    inner: &GraphPattern,
    row_budget: usize,
) -> Result<Option<Bindings>, String> {
    match inner {
        GraphPattern::Project { inner: proj_inner, variables } => {
            Ok(try_topk_orderby(graph, local, proj_inner, row_budget)?
                .map(|b| project_bindings(b, variables)))
        }
        GraphPattern::Reduced { inner: red_inner } => {
            try_topk_orderby(graph, local, red_inner, row_budget)
        }
        GraphPattern::OrderBy { inner: ord_inner, expression } => {
            // [SONNET-5] (sq-artifact-keeper-topk) Try the index-ordered seed path
            // first: for the narrow "star BGP, ORDER BY a variable bound by exactly
            // one pattern's object" shape, it walks a value-sorted permutation scan
            // directly instead of materialising every matching row — see
            // `try_topk_orderby_indexed`'s doc comment for the soundness argument.
            // Declines (`Ok(None)`) for any shape/datatype it can't prove safe, in
            // which case the existing materialize-then-select path below runs
            // unchanged.
            if let Some(b) = try_topk_orderby_indexed(graph, local, ord_inner, expression, row_budget)? {
                return Ok(Some(b));
            }
            let mut b = eval_modified(graph, local, ord_inner)?;
            // Use the top-k path only when the budget is strictly less than n;
            // otherwise the heap adds overhead with no benefit.
            let use_topk = b.rows.len() > row_budget;
            order_bindings(graph, local, &mut b, expression, if use_topk { Some(row_budget) } else { None })?;
            Ok(Some(b))
        }
        // Not an OrderBy under transparent wrappers: decline so the caller
        // falls through to the regular eval_modified path.
        _ => Ok(None),
    }
}

/// [SONNET-5] (sq-artifact-keeper-topk) Attempts INDEX-ORDERED top-k evaluation for a
/// `Slice{OrderBy{BGP}}` shape whose primary sort key is bound by exactly one triple
/// pattern's OBJECT. `try_topk_orderby`'s existing bounded-heap path still calls
/// `eval_modified` to fully evaluate (and materialise) the WHOLE matching candidate
/// set before selecting the top `row_budget` rows — `O(n)` in the number of rows
/// CURRENTLY matching the BGP's equality filters, not in `row_budget`. That is fine
/// when some pattern is selective, but when every pattern matches nearly the same
/// rows (e.g. a job-queue "claim the next pending task for peer X, ordered by
/// priority" query, where `peer` + `status` don't shrink the candidate set at all),
/// there is no cheaper seed for the cardinality-based planner to pick, and the cost
/// scales with the size of the whole pending queue on every single claim — profiled
/// and confirmed via `EXPLAIN ANALYZE` (the BGP operator reports touching every
/// matching row) in the sparq/artifact-keeper migration's claim-queue throughput
/// investigation.
///
/// SOUNDNESS. Only activates for a narrow, syntactically-verified "star" shape, and
/// declines (`Ok(None)`) at the first sign of anything it cannot prove safe — the
/// caller's `eval_modified`-then-select path is always correct, just `O(n)`; this
/// function's only job is to be a provably-equivalent shortcut, never a new source
/// of results:
/// - `ord_inner` is a plain triple-pattern conjunction with NO residual FILTER and
///   no blank nodes (kept out of scope for this first cut).
/// - The FIRST order key is a bare `Variable` (secondary tie-break keys, if any,
///   are resolved by re-using `order_bindings` over the small collected set below —
///   not reimplemented here).
/// - Exactly one pattern (the "seed") binds that variable as its OBJECT, with a
///   CONSTANT predicate and a VARIABLE subject (the join "hub").
/// - Every OTHER pattern has that same hub variable as its SUBJECT and a CONSTANT
///   predicate (a star join around the hub); any object variable it introduces must
///   not appear anywhere else in the BGP. Any other shape (the hub appearing
///   elsewhere, a variable predicate, a reused object variable) declines.
/// - Every object id on the seed's sorted scan is an INLINE small integer
///   (`dict::is_inline`) — the only case where ascending dictionary-id order is
///   proven to equal ascending SPARQL `value()` order (the same guard
///   `scan_to_bindings`'s range-pruning already relies on for the same reason).
///   Checked for the WHOLE scan up front, before any result is built, so a
///   non-inline id never leaks a partially-computed (and potentially
///   wrongly-ordered) answer.
///
/// Under those conditions, walking the seed's sorted scan in the required
/// direction, testing each candidate hub value against the OTHER patterns via a
/// direct bound lookup (index-nested-loop / bind join over a per-pattern
/// subject-sorted range resolved once, not re-resolved per candidate — see the
/// `other_pats` construction below), and stopping once a COMPLETE sort-key group
/// (never split mid-tie) has produced at least `row_budget` confirmed joins
/// yields valid top rows: an unvisited group is strictly worse in the requested
/// primary-key direction, so it cannot displace the collected top `row_budget`.
/// [GPT-6 Astra] If all ORDER BY keys tie, SPARQL permits different surviving
/// subsets and tie order; equivalence does not require the fallback's input-index
/// stability. Finishing a primary-key group preserves secondary-key selection.
///
/// PERFORMANCE, not soundness: a single escalation block is also capped at a
/// fraction of the candidate pool (see `max_group` below) — exceeding it
/// declines (`Ok(None)`) even though continuing would still be CORRECT, because
/// past that point this function's per-candidate cost stops being cheaper than
/// the fallback's bulk join (measured; see `max_group`'s comment). This keeps
/// the walk bounded, but setup and failed probes still precede a decline.
///
/// [GPT-6 Astra] Recovery-stage restriction: budgeted queries, repeated variables
/// and multi-valued probes use the existing evaluator. The historical timing notes
/// below are unverified on current main; this candidate establishes no speedup.
fn try_topk_orderby_indexed(
    graph: &Graph,
    local: &mut LocalVocab,
    ord_inner: &GraphPattern,
    expression: &[OrderExpression],
    row_budget: usize,
) -> Result<Option<Bindings>, String> {
    #[cfg(feature = "zk")]
    if crate::zk::enabled() {
        return Ok(None);
    }
    // [GPT-6 Astra] Preserve the existing evaluator's intermediate row/byte
    // accounting and cooperative cancellation schedule before doing any new work.
    if budget::active() {
        return Ok(None);
    }
    if view::default_is_empty() {
        return Ok(None);
    }
    let Some((desc, order_var)) = expression.first().and_then(|oe| match oe {
        OrderExpression::Asc(Expression::Variable(v)) => Some((false, v.clone())),
        OrderExpression::Desc(Expression::Variable(v)) => Some((true, v.clone())),
        _ => None,
    }) else {
        return Ok(None);
    };
    if !is_conjunctive(ord_inner) {
        return Ok(None);
    }
    let mut patterns = Vec::new();
    let mut filters = Vec::new();
    flatten_conjunction(ord_inner, &mut patterns, &mut filters);
    if !filters.is_empty() || patterns.is_empty() {
        return Ok(None);
    }
    // [GPT-6 Astra] prepare_pattern records repeated slots but does not enforce
    // their equality. Reuse the existing fast-path guard before bypassing build_row.
    if patterns.iter().any(has_intra_triple_repeated_var) {
        return Ok(None);
    }
    // Out of scope for this first cut: blank nodes are treated as synthetic
    // variables by `prepare_pattern` (`bnode_var`) but not necessarily by
    // `collect_vars`'s header-construction — decline rather than risk a header
    // mismatch.
    let has_blank_node = patterns.iter().any(|tp| {
        matches!(tp.subject, TermPattern::BlankNode(_)) || matches!(tp.object, TermPattern::BlankNode(_))
    });
    if has_blank_node {
        return Ok(None);
    }

    // Locate the ONE seed pattern binding `order_var` as its object with a
    // constant predicate and a variable subject.
    let mut seed_idx = None;
    for (i, tp) in patterns.iter().enumerate() {
        if !matches!(&tp.object, TermPattern::Variable(v) if *v == order_var) {
            continue;
        }
        if !matches!(&tp.predicate, NamedNodePattern::NamedNode(_)) {
            return Ok(None);
        }
        if !matches!(&tp.subject, TermPattern::Variable(_)) {
            return Ok(None);
        }
        if seed_idx.is_some() {
            return Ok(None); // order_var bound by more than one pattern: not this shape
        }
        seed_idx = Some(i);
    }
    let Some(seed_idx) = seed_idx else { return Ok(None) };
    let hub_var = match &patterns[seed_idx].subject {
        TermPattern::Variable(v) => v.clone(),
        _ => unreachable!("checked above"),
    };

    // Every OTHER pattern must be `hub_var <constant predicate> (var|const)` — a
    // star around the hub — and any variable it introduces must be fresh.
    let mut other_vars: FxHashSet<Variable> = FxHashSet::default();
    for (i, tp) in patterns.iter().enumerate() {
        if i == seed_idx {
            continue;
        }
        if !matches!(&tp.subject, TermPattern::Variable(v) if *v == hub_var) {
            return Ok(None);
        }
        if !matches!(&tp.predicate, NamedNodePattern::NamedNode(_)) {
            return Ok(None);
        }
        if let TermPattern::Variable(v) = &tp.object {
            if *v == hub_var || *v == order_var || !other_vars.insert(v.clone()) {
                return Ok(None);
            }
        }
    }

    let out_vars = collect_vars(&patterns);
    if row_budget == 0 {
        return Ok(Some(Bindings::unsorted(out_vars, vec![])));
    }

    // Resolve + scan the seed pattern sorted by its OBJECT (canonical column 2).
    let (seed_id_pat, _seed_pos_vars, seed_unsat) = prepare_pattern(graph, &patterns[seed_idx])?;
    if seed_unsat {
        return Ok(Some(Bindings::unsorted(out_vars, vec![])));
    }
    let scan = graph.store.scan_sorted(&seed_id_pat, 2);
    let actual_sort = scan.perm.order().into_iter().find(|&c| seed_id_pat[c].is_none());
    if actual_sort != Some(2) {
        return Ok(None); // this store build can't give us an object-sorted scan
    }
    let rows: &[[Id; 3]] = scan.rows.as_ref();
    // Guard: EVERY object id on this scan must be an inline integer, or ascending
    // id order is not proven to be ascending value order.
    if !rows.iter().all(|r| dict::is_inline(scan.to_spo(r)[2])) {
        return Ok(None);
    }

    // [GPT-6 Astra] The first block must include its entire leading key group.
    // Reject a group larger than the EXISTING block cap before retaining probe
    // scans and subject vectors. Sorted object ids make equality at the cap's
    // zero-based edge equivalent to group_len > max_group, in either direction.
    const MAX_INDEXED_GROUP_FLOOR: usize = 256;
    let n = rows.len();
    let max_group = (n / 2).max(MAX_INDEXED_GROUP_FLOOR).max(row_budget.saturating_mul(2));
    if n > max_group {
        let first = if desc { n - 1 } else { 0 };
        let edge = if desc { first - max_group } else { max_group };
        if scan.to_spo(&rows[first])[2] == scan.to_spo(&rows[edge])[2] {
            return Ok(None);
        }
    }

    // Prepare the other patterns ONCE ON DEMAND, each as a SUBJECT-sorted scan over its
    // (fixed predicate [+ fixed object]) range — not re-resolved per candidate.
    // The first cut of this function called `graph.store.scan(&probe_pat)`
    // fresh for every (candidate, other-pattern) pair, which re-runs
    // `Store::choose`'s permutation selection + bound computation every single
    // time even though the choice is IDENTICAL across all candidates for a
    // given pattern (only the subject varies). That repeated per-call setup —
    // not the underlying lookup — is what made a large tie-group lose to the
    // fallback's bulk merge-join (which resolves the permutation ONCE per
    // pattern, not once per row). Resolving it once here and binary-searching
    // the resulting sorted slice per candidate removes that repeated cost.
    struct OtherScan<'g> {
        scan: sparq_core::store::Scan<'g>,
        // Precomputed ONCE (not per candidate, not per binary-search
        // comparison step): the subject id of every row in `scan`, in the
        // SAME (subject-sorted) order. `partition_point` over this plain
        // `&[Id]` does a trivial integer compare per step; searching `scan`
        // directly would call `to_spo` (a permutation-order reconstruction)
        // on EVERY comparison, i.e. ~log(m) reconstructions per candidate per
        // pattern — measured as a real remaining cost for a long skip-prefix
        // (many candidates probed and rejected in a row): reconstructing the
        // full [S,P,O] triple just to read column 0 is wasted work when the
        // search only ever needs that one column.
        subject_ids: Vec<Id>,
    }
    // [GPT-6 Astra] Resolve metadata eagerly, but retain a scan/vector only when
    // a candidate reaches this pattern in the original order. A failed constant
    // prefix therefore does not prepare later variable probes that it never uses.
    struct OtherPat<'g> {
        id_pat: IdPattern,
        obj_var: Option<Variable>,
        prepared: Option<OtherScan<'g>>,
    }
    fn prepare_other<'g>(graph: &'g Graph, op: &mut OtherPat<'g>, seed_card: usize) -> bool {
        if op.prepared.is_some() {
            return true;
        }
        let sub_scan = graph.store.scan_sorted(&op.id_pat, 0);
        let sub_actual_sort = sub_scan.perm.order().into_iter().find(|&c| op.id_pat[c].is_none());
        if sub_actual_sort != Some(0) {
            return false;
        }
        // The original exact-cardinality admission rule still applies to EVERY
        // probe before it can contribute to a row, including a late selective one.
        if sub_scan.rows.len().saturating_mul(2) < seed_card {
            return false;
        }
        let subject_ids: Vec<Id> = sub_scan.rows.iter().map(|r| sub_scan.to_spo(r)[0]).collect();
        #[cfg(test)]
        indexed_topk_preparation_tests::observe(subject_ids.len());
        op.prepared = Some(OtherScan { scan: sub_scan, subject_ids });
        true
    }
    let mut other_pats: Vec<OtherPat> = Vec::with_capacity(patterns.len().saturating_sub(1));
    for (i, tp) in patterns.iter().enumerate() {
        if i == seed_idx {
            continue;
        }
        let (id_pat, pos_vars, unsat) = prepare_pattern(graph, tp)?;
        if unsat {
            // A constant elsewhere in the BGP absent from the dictionary makes the
            // WHOLE conjunction empty (a BGP join against an empty relation).
            return Ok(Some(Bindings::unsorted(out_vars, vec![])));
        }
        other_pats.push(OtherPat {
            id_pat: [None, id_pat[1], id_pat[2]],
            obj_var: pos_vars[2].clone(),
            prepared: None,
        });
    }

    // [GPT-6 Astra] The previous global min-cardinality check is now applied
    // by prepare_other to each exact scan count before that probe is used. A
    // successful row must visit every pattern, so it passes the same admission
    // rule. Unvisited patterns stay unproved and cannot authorize an empty result.

    // The output column list is FIXED across every candidate (hub, order, then
    // each other pattern's object variable, in pattern order) — compute it and
    // the out_vars -> column-position mapping ONCE, not per candidate.
    let mut cols: Vec<Variable> = Vec::with_capacity(2 + other_pats.len());
    cols.push(hub_var.clone());
    cols.push(order_var.clone());
    for op in &other_pats {
        if let Some(ov) = &op.obj_var {
            cols.push(ov.clone());
        }
    }
    let out_to_cols: Vec<Option<usize>> = out_vars.iter().map(|v| cols.iter().position(|c| c == v)).collect();
    let emit = |ids: &[Id], collected: &mut Vec<Row>| {
        let mut row = Row::with_capacity(out_to_cols.len());
        for pos in &out_to_cols {
            row.push(pos.map(|i| ids[i]).unwrap_or(NO_ID));
        }
        collected.push(row);
    };

    // Walk the sorted scan in the required direction, in escalating blocks aligned
    // to complete sort-key (object-id) groups, joining each candidate against the
    // other patterns via a direct bound lookup (index-nested-loop / bind join).
    //
    // `rows` is always ASCENDING by object id (the scan's actual sort order). For
    // DESC, "best first" means walking it back-to-front. Rather than slicing from
    // the front and reversing per block (which — subtly, and wrongly — visits the
    // SMALLEST values first regardless of direction, since the slice bounds
    // themselves were never flipped), define a single LOGICAL index `0..n` where
    // position 0 is always the best candidate, via `logical`, and do all
    // block/boundary arithmetic in that space. `logical` and `obj_id_at` are the
    // only direction-aware code; everything below them is direction-agnostic.
    let logical = |i: usize| -> usize { if desc { n - 1 - i } else { i } };
    let obj_id_at = |i: usize| -> Id { scan.to_spo(&rows[logical(i)])[2] };
    // A single escalation block is dominated by ONE large tie-group when the
    // sort key has low cardinality (e.g. a handful of discrete priority tiers:
    // "urgent/high/normal/low", not a near-unique value per row). This
    // function's per-candidate cost is roughly CONSTANT per candidate (a
    // binary search per other pattern, resolved against a scan fetched once —
    // see `other_pats` below), while the fallback's bulk merge-join cost is
    // roughly constant PER `n` regardless of tie structure (it materialises the
    // whole matching set before it can sort). So the crossover is a FRACTION of
    // `n`, not an absolute row count: measured at n=1600, a tie-group of 800
    // (half of n) still beat the fallback (~228us vs. the fallback's ~269us),
    // but a tie-group of 1600 (all of n) lost (would be ~450-700us vs. the
    // fallback's own ~269us) — and the same ~0.5-0.75 fraction held at n=8000
    // (4000 still competitive, 8000 clearly lost). `max_group` is
    // therefore `n / 2` (with a floor for small `n`, and never below
    // `row_budget` itself) — declining past it defers to the fallback, with
    // any preparation and failed probes already performed adding to its cost.
    let mut collected: Vec<Row> = Vec::new();
    let mut visited_to: usize = 0;
    // Block sizes grow GEOMETRICALLY from a small multiple of `row_budget`, not
    // from `CAPPED_SEED_BLOCK` (1024): each candidate here costs a real
    // per-pattern binary search (against the scan `other_pats` already resolved
    // once, above), unlike the bulk merge-join the fallback path uses, which
    // amortises its own permutation selection over the WHOLE scan in one pass.
    // For the common case (small `row_budget`, most candidates
    // join successfully), starting near `row_budget` means the first block alone
    // usually satisfies it — starting at 1024 would pay ~1024 point-probes even for
    // `LIMIT 1`, which is worse than the bulk path it's meant to beat (measured:
    // this was the actual cause of a regression at moderate `n`, not a win).
    // Cumulative count of candidates that failed an OTHER-pattern check, across
    // ALL blocks in this call — distinct from `max_group`'s per-block width
    // check. A workload that claims strictly in priority order (the realistic
    // shape: highest priority first) leaves an ever-growing prefix of
    // ALREADY-CLAIMED, high-priority-but-no-longer-`pending` rows at the head
    // of the seed scan — each one still costs a real per-pattern probe before
    // being rejected. That prefix is made of individually DISTINCT priority
    // values, so it never forms one oversized tie-group `max_group` would
    // catch; it spreads across many small geometric-growth blocks instead. The
    // per-pattern cost check (comparing exact scan cardinalities to
    // the seed's) already declines the CLEAR case — an other-pattern that's
    // globally selective enough for the fallback's own planner to prefer as
    // ITS seed — before that probe is used. This counter is a
    // SAFETY NET for what that check can't see: the seed's global cardinality
    // vs. an other-pattern's global cardinality doesn't capture every
    // possible skip-prefix shape (e.g. a correlation between scan order and
    // an other-pattern's matches that isn't visible from cardinality alone).
    //
    // The budget here must stay LARGE (matching `max_group`, not a small
    // constant): every failed probe before declining is pure waste stacked ON
    // TOP OF the fallback's full cost, but a SMALL budget bails on the far
    // more common "moderate skip-prefix" case too — one that would have
    // SUCCEEDED cheaply if allowed to keep going (a candidate ~100-800
    // positions in costs only tens of us to walk to, vastly cheaper than a
    // ~100-350us fallback). Measured directly: tightening this to a small
    // constant (64) made the COMMON moderate case (already-claimed ~100-400)
    // cost ~225-275us (forced into a fallback the upfront check didn't catch
    // and completion would have avoided entirely) instead of the ~15-70us it
    // gets by simply being allowed to finish. A large budget's own worst case
    // (a skip-prefix near `n/2`, right at the edge) is bounded and modest by
    // comparison (~30-50% over a clean fallback, not multiples of it) — an
    // acceptable price for a case the upfront check should catch almost
    // always in practice anyway.
    let failure_budget = max_group;
    let mut failed_count: usize = 0;
    let mut block_target = row_budget.saturating_mul(4).max(16);
    loop {
        if visited_to >= n {
            break;
        }
        let mut end = block_target.clamp(visited_to, n);
        // Extend to the next object-id group boundary so a tie is never split
        // across blocks (the early-stop check below requires a COMPLETE group).
        if end < n && end > 0 {
            let boundary_obj = obj_id_at(end - 1);
            while end < n && obj_id_at(end) == boundary_obj {
                end += 1;
            }
        }
        if end - visited_to > max_group {
            return Ok(None);
        }
        for i in visited_to..end {
            let spo = scan.to_spo(&rows[logical(i)]);
            let hub_id = spo[0];
            let order_id = spo[2];
            // [GPT-6 Astra] Only single-valued probes are admitted in this slice.
            // General Cartesian expansion remains with the existing evaluator.
            let mut row_ids: SmallVec<[Id; 8]> = SmallVec::from_slice(&[hub_id, order_id]);
            let mut failed = false;
            for op in &mut other_pats {
                if !prepare_other(graph, op, rows.len()) {
                    return Ok(None);
                }
                let prepared = op.prepared.as_ref().expect("successful preparation");
                // Binary-search the PRECOMPUTED, plain-`Id` subject list for
                // this pattern (built once, above) — a trivial integer
                // compare per step, no `to_spo` reconstruction during the
                // search itself (that only happens below, per ACTUAL match,
                // not per comparison step — see `subject_ids`'s doc comment).
                let start = prepared.subject_ids.partition_point(|&id| id < hub_id);
                let stop = start + prepared.subject_ids[start..].partition_point(|&id| id == hub_id);
                if start == stop {
                    failed = true;
                    break;
                }
                let Some(_) = &op.obj_var else {
                    continue; // `obj_const` — existence-only, no column added
                };
                let op_rows: &[[Id; 3]] = prepared.scan.rows.as_ref();
                let match_count = stop - start;
                if match_count != 1 {
                    return Ok(None);
                }
                row_ids.push(prepared.scan.to_spo(&op_rows[start])[2]);
            }
            if failed {
                failed_count += 1;
                if failed_count > failure_budget {
                    return Ok(None);
                }
                continue;
            }
            emit(&row_ids, &mut collected);
        }
        visited_to = end;
        if collected.len() >= row_budget {
            break; // a COMPLETE group boundary was just finished — safe to stop
        }
        block_target = block_target.saturating_mul(4).max(visited_to + 1);
    }

    // [GPT-6 Astra] An empty walk may never visit later patterns. Preserve their
    // unproved static-sort/cardinality exclusions through the existing fallback.
    if other_pats.iter().any(|op| op.prepared.is_none()) {
        return Ok(None);
    }
    let mut result = Bindings { vars: out_vars, rows: collected, sorted_by: None };
    let use_topk = result.rows.len() > row_budget;
    order_bindings(graph, local, &mut result, expression, if use_topk { Some(row_budget) } else { None })?;
    Ok(Some(result))
}

// [GPT-6 Astra] Test-only observations pin retained preparation work, not timing.
#[cfg(test)]
mod indexed_topk_preparation_tests {
    use sparq_core::Graph;
    use std::cell::Cell;
    use std::fmt::Write;

    thread_local! { static PREPARED: Cell<(usize, usize)> = const { Cell::new((0, 0)) }; }
    pub(super) fn observe(rows: usize) {
        PREPARED.with(|c| { let (scans, ids) = c.get(); c.set((scans + 1, ids + rows)); });
    }
    const TEXT: &str = "SELECT ?s WHERE { ?s <urn:peer> <urn:X> ; <urn:status> \"pending\" ; <urn:priority> ?p ; <urn:seq> ?seq ; <urn:a> ?a ; <urn:b> ?b ; <urn:c> ?c } ORDER BY DESC(?p)";

    fn graph(prefix: usize, overlay: bool, selective_last: bool) -> Graph {
        let mut ttl = String::new();
        for i in 0..4096 {
            let status = if i < 4096 - prefix { "pending" } else { "done" };
            writeln!(ttl, "<urn:s{i}> <urn:peer> <urn:X> ; <urn:status> \"{status}\" ; <urn:priority> {i} ; <urn:seq> {i} ; <urn:a> {i} ; <urn:b> {i} .").unwrap();
            if !selective_last || i == 4095 { writeln!(ttl, "<urn:s{i}> <urn:c> {i} .").unwrap(); }
        }
        let base = Graph::load_str(&ttl, "turtle").unwrap();
        if !overlay { return base; }
        let mut changed = base.fork();
        let mut deletes = String::from("DELETE DATA {");
        for i in (0..4096).step_by(5) { writeln!(deletes, "<urn:s{i}> <urn:priority> {i} .").unwrap(); }
        deletes.push('}');
        crate::update_in_place(&mut changed, &deletes).unwrap();
        assert_eq!(changed.pending_delta_len(), 820);
        changed
    }

    #[test]
    fn later_variable_preparation_is_avoided_before_block_decline() {
        for overlay in [false, true] {
            let g = graph(2047, overlay, false);
            let full = crate::query(&g, TEXT).unwrap();
            PREPARED.with(|c| c.set((0, 0)));
            let got = crate::query(&g, &format!("{TEXT} LIMIT 1")).unwrap();
            assert_eq!(got.rows, full.rows[..1]);
            assert_eq!(PREPARED.with(Cell::get), (2, 4096 + 2049), "later variable scans/vectors must remain unprepared");
        }
    }

    #[test]
    fn successful_rows_prepare_each_probe_once() {
        let g = graph(0, false, false);
        for k in [1, 32] {
            PREPARED.with(|c| c.set((0, 0)));
            assert_eq!(crate::query(&g, &format!("{TEXT} LIMIT {k}")).unwrap().rows.len(), k);
            let expected = if sparq_core::store::BUILT.contains(&sparq_core::store::Perm::Pso) { (6, 6 * 4096) } else { (2, 2 * 4096) };
            assert_eq!(PREPARED.with(Cell::get), expected);
        }
    }

    #[test]
    fn late_selective_probe_still_declines_before_emitting() {
        let g = graph(0, false, true);
        let full = crate::query(&g, TEXT).unwrap();
        assert_eq!(full.rows.len(), 1);
        let limited = format!("{TEXT} LIMIT 1");
        assert_eq!(crate::query(&g, &limited).unwrap().rows, full.rows);
        let trace = crate::explain_analyze(&g, &limited).unwrap();
        assert!(trace.contains("BGP [binary GOO]"), "late cardinality guard must decline: {trace}");
    }
}

```

### crates/sparq-engine/tests/topk_orderby_indexed_differential.rs:1

```rust
//! Differential + correctness tests for `try_topk_orderby_indexed` (the
//! index-ordered top-k seed path added for the artifact-keeper claim-queue
//! throughput investigation).
//!
//! `try_topk_orderby_indexed` is a pure SHORTCUT: it either returns exactly the
//! valid rows the pre-existing `eval_modified`-then-`order_bindings` path would,
//! or declines (`Ok(None)`) and lets that path run. [GPT-6 Astra] Total-order
//! tests use an independent Rust sort or full ORDER BY oracle; exact ties assert
//! permitted membership and size. Execution traces pin admission/fallback guards.

use sparq_core::Graph;
use sparq_engine::query;

const PFX: &str = "PREFIX ak: <http://example.org/ak#>\n";

// [GPT-6 Astra] Delayed preparation must preserve modest/intermittent misses,
// original pattern order, complete key groups, and OFFSET in either direction.
#[test]
fn lazy_probes_preserve_miss_patterns_and_overlay_windows() {
    use std::fmt::Write;
    for desc in [false, true] {
        for prefix in [128, 800, 0] {
            let mut ttl = String::new();
            for i in 0..2048 {
                let rank = if desc { 2047 - i } else { i };
                let pending = if prefix == 0 { rank % 8 != 0 } else { rank >= prefix };
                let status = if pending { "pending" } else { "done" };
                writeln!(ttl, "<urn:s{i}> <urn:p> {} ; <urn:status> \"{status}\" ; <urn:seq> {i} .", i / 3).unwrap();
            }
            let base = Graph::load_str(&ttl, "turtle").unwrap();
            let mut changed = base.fork();
            sparq_engine::update_in_place(&mut changed, "DELETE DATA { <urn:s1024> <urn:p> 341 . <urn:s1025> <urn:p> 341 . }; INSERT DATA { <urn:extra> <urn:p> 400 ; <urn:status> \"pending\" ; <urn:seq> 9999 . }").unwrap();
            for graph in [&base, &changed] {
                for variable_first in [false, true] {
                    let other = if variable_first { "?s <urn:seq> ?seq ; <urn:status> \"pending\"" } else { "?s <urn:status> \"pending\" ; <urn:seq> ?seq" };
                    let direction = if desc { "DESC" } else { "ASC" };
                    let text = format!("SELECT ?s WHERE {{ ?s <urn:p> ?p . {other} }} ORDER BY {direction}(?p) DESC(?seq)");
                    let full = query(graph, &text).unwrap();
                    let limited = format!("{text} OFFSET 2 LIMIT 4");
                    assert_eq!(query(graph, &limited).unwrap().rows, full.rows[2..6], "prefix={prefix}, desc={desc}, variable_first={variable_first}");
                }
            }
        }
    }
}

// [GPT-6 Astra] An all-rejected walk cannot silently waive unvisited exclusions.
#[test]
fn unvisited_lazy_probe_keeps_empty_result_on_fallback() {
    use std::fmt::Write;
    let mut ttl = String::new();
    for i in 0..32 {
        writeln!(ttl, "<urn:s{i}> <urn:p> {i} ; <urn:later> 1, 2 . <urn:foreign{i}> <urn:status> \"pending\" .").unwrap();
    }
    let graph = Graph::load_str(&ttl, "turtle").unwrap();
    let text = "SELECT ?s WHERE { ?s <urn:p> ?p ; <urn:status> \"pending\" ; <urn:later> ?v } ORDER BY DESC(?p) LIMIT 1";
    assert!(query(&graph, text).unwrap().rows.is_empty());
    let trace = sparq_engine::explain_analyze(&graph, text).unwrap();
    assert!(trace.contains("BGP [binary GOO]"), "unvisited probe must not authorize indexed admission: {trace}");
}

```

### Exact extended benchmark fixture/harness

```rust
//! [GPT-6 Astra] Local whole-query top-k diagnostic; never a canonical score.
//! Separate timing and counting builds avoid timing the instrumented allocator.

#[cfg(feature = "count-alloc")]
mod counting;

use oxrdf::{Literal, NamedNode, Term};
use sparq_core::Graph;
use sparq_engine::{query, QueryResult};
use std::fmt::Write;
use std::hint::black_box;
use std::time::Instant;

fn iri(value: &str) -> Term {
    NamedNode::new(value).unwrap().into()
}

fn priority(i: usize, n: usize, ties: bool) -> usize {
    if ties && i >= n - (n / 2 + 1) {
        n + 1
    } else {
        i
    }
}

fn pending(i: usize, n: usize, drained: bool) -> bool {
    !drained || i < n - (n / 2 - 1)
}

struct Fixture {
    graph: Graph,
    // Keep the original snapshot alive in the overlay case, as a live reader would.
    _snapshot: Option<Graph>,
    expected: Vec<usize>,
    seed_rows: usize,
    pending_rows: usize,
    tied_rows: usize,
    tombstones: usize,
}

fn fixture(case: &str, n: usize, k: usize) -> Fixture {
    assert!(matches!(
        case,
        "base" | "overlay" | "drained" | "drained-overlay" | "ties" | "ties-overlay"
            | "prefix128" | "prefix128-overlay" | "prefix800" | "prefix800-overlay"
            | "intermittent" | "intermittent-overlay"
    ));
    assert!(n >= 1024 && n <= 100_000 && k > 0 && k <= 1024);
    let overlay = case.contains("overlay");
    let drained = case.starts_with("drained");
    let ties = case.starts_with("ties");
    // [GPT-6 Astra] Predeclared miss fixtures use LIVE rank so the overlay
    // has exactly the same leading miss count after its every-fifth deletions.
    let prefix = if case.starts_with("prefix128") { 128 } else if case.starts_with("prefix800") { 800 } else { 0 };
    let intermittent = case.starts_with("intermittent");
    let mut ttl = String::new();
    let mut deletes = Vec::new();
    let mut expected = Vec::new();
    let (mut seed_rows, mut pending_rows, mut tied_rows) = (0, 0, 0);
    for i in 0..n {
        let p = priority(i, n, ties);
        let live_rank = n - 1 - i - if overlay { (n - 1) / 5 - i / 5 } else { 0 };
        let is_pending = if prefix > 0 { live_rank >= prefix } else if intermittent { live_rank % 8 != 0 } else { pending(i, n, drained) };
        let status = if is_pending {
            "pending"
        } else {
            "done"
        };
        writeln!(ttl, "<urn:s:{i}> <urn:peer> <urn:X> ; <urn:status> \"{status}\" ; <urn:priority> {p} ; <urn:seq> {i} ; <urn:a> {i} ; <urn:b> {i} ; <urn:c> {i} .").unwrap();
        if overlay && i % 5 == 0 {
            let s = iri(&format!("urn:s:{i}"));
            for (pred, object) in [
                ("peer", iri("urn:X")),
                ("status", Literal::new_simple_literal(status).into()),
                ("priority", Literal::from(p as i64).into()),
                ("seq", Literal::from(i as i64).into()),
                ("a", Literal::from(i as i64).into()),
                ("b", Literal::from(i as i64).into()),
                ("c", Literal::from(i as i64).into()),
            ] {
                deletes.push([s.clone(), iri(&format!("urn:{pred}")), object]);
            }
        } else {
            seed_rows += 1;
            tied_rows += usize::from(ties && p == n + 1);
            if is_pending {
                pending_rows += 1;
                expected.push((p, i));
            }
        }
    }
    expected.sort_unstable_by(|a, b| b.cmp(a));
    let expected = expected.into_iter().take(k).map(|(_, i)| i).collect();
    let base = Graph::load_str(&ttl, "turtle").unwrap();
    let tombstones = deletes.len();
    let (graph, snapshot) = if overlay {
        let mut fork = base.fork();
        fork.apply_delta(&[], &deletes).unwrap();
        assert_eq!(fork.pending_delta_len(), tombstones);
        (fork, Some(base))
    } else {
        (base, None)
    };
    if drained {
        assert!(pending_rows * 2 >= seed_rows);
        assert!(pending_rows * 2 - seed_rows <= 4);
    }
    if prefix > 0 {
        assert_eq!(seed_rows - pending_rows, prefix);
    }
    if intermittent {
        assert_eq!(seed_rows - pending_rows, seed_rows.div_ceil(8));
    }
    if ties {
        assert!(tied_rows > (seed_rows / 2).max(256).max(2 * k));
    }
    Fixture {
        graph,
        _snapshot: snapshot,
        expected,
        seed_rows,
        pending_rows,
        tied_rows,
        tombstones,
    }
}

fn check(result: &QueryResult, expected: &[usize]) {
    let ids: Vec<_> = result
        .rows
        .iter()
        .map(|row| match &row[0] {
            Some(Term::NamedNode(n)) => n
                .as_str()
                .strip_prefix("urn:s:")
                .unwrap()
                .parse::<usize>()
                .unwrap(),
            other => panic!("unexpected binding: {other:?}"),
        })
        .collect();
    assert_eq!(ids, expected, "independent total-order oracle");
}

/// Cumulative process RSS high-water mark; it cannot be reset at a query boundary.
fn max_rss() -> u64 {
    let mut usage = std::mem::MaybeUninit::<libc::rusage>::uninit();
    // SAFETY: getrusage initializes the pointed-to rusage on success; RUSAGE_SELF
    // requests this process only. Assume initialization only after return code zero.
    assert_eq!(
        unsafe { libc::getrusage(libc::RUSAGE_SELF, usage.as_mut_ptr()) },
        0
    );
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
    let args: Vec<_> = std::env::args().collect();
    if args.get(1).is_some_and(|x| x == "selftest") {
        #[cfg(feature = "count-alloc")]
        counting::calibrate();
        println!(
            "kind=selftest\tcounting={}\tstatus=pass",
            cfg!(feature = "count-alloc")
        );
        return;
    }
    assert_eq!(args.len(), 8, "MODE CASE N K WARMUP REPS EXPECTED_PATH");
    let (mode, case) = (args[1].as_str(), args[2].as_str());
    let n = args[3].parse().unwrap();
    let k = args[4].parse().unwrap();
    let warmup: usize = args[5].parse().unwrap();
    let reps: usize = args[6].parse().unwrap();
    assert!(matches!(mode, "verify" | "measure") && reps > 0);
    #[cfg(feature = "count-alloc")]
    let setup_baseline = counting::begin();
    let f = fixture(case, n, k);
    let text = format!("SELECT ?s WHERE {{ ?s <urn:peer> <urn:X> ; <urn:status> \"pending\" ; <urn:priority> ?p ; <urn:seq> ?seq ; <urn:a> ?a ; <urn:b> ?b ; <urn:c> ?c }} ORDER BY DESC(?p) DESC(?seq) LIMIT {k}");
    #[cfg(feature = "count-alloc")]
    let setup_counts = counting::end(setup_baseline);
    let setup_rss = max_rss();
    println!("kind=fixture\tcase={case}\tn={n}\tk={k}\tseed_rows={}\tpending_rows={}\ttied_rows={}\tprobe_predicates=6\ttombstones={}\tsetup_max_rss={setup_rss}\tcounting={}", f.seed_rows, f.pending_rows, f.tied_rows, f.tombstones, cfg!(feature="count-alloc"));
    #[cfg(feature = "count-alloc")]
    println!("kind=setup_alloc\tallocs={}\treallocs={}\trequested_bytes={}\tpeak_live_delta={}\tending_live={}", setup_counts.0, setup_counts.1, setup_counts.2, setup_counts.3, setup_counts.4);
    if mode == "verify" {
        let result = query(&f.graph, &text).unwrap();
        check(&result, &f.expected);
        let trace = sparq_engine::explain_analyze(&f.graph, &text).unwrap();
        let fallback = trace.contains("BGP [binary GOO]");
        assert_eq!(fallback, args[7] == "fallback", "{trace}");
        println!(
            "kind=verify\tpath={}\trows={}\toracle=pass",
            if fallback {
                "fallback"
            } else {
                "indexed-inferred"
            },
            result.rows.len()
        );
        println!("{trace}");
        return;
    }
    for _ in 0..warmup {
        check(&query(&f.graph, &text).unwrap(), &f.expected);
    }
    let pre_query_rss = max_rss();
    for rep in 0..reps {
        #[cfg(feature = "count-alloc")]
        let baseline = counting::begin();
        let start = Instant::now();
        let result = black_box(query(black_box(&f.graph), black_box(&text)).unwrap());
        let nanos = start.elapsed().as_nanos();
        #[cfg(feature = "count-alloc")]
        let counts = counting::end(baseline);
        let rss = max_rss();
        // Validation and output formatting are outside the measured call/window.
        check(&result, &f.expected);
        #[cfg(feature = "count-alloc")]
        println!("kind=sample\trep={rep}\tallocs={}\treallocs={}\trequested_bytes={}\tpeak_live_delta={}\tending_live={}\tpre_query_max_rss={pre_query_rss}\tpost_query_max_rss={rss}\trows={}\toracle=pass", counts.0, counts.1, counts.2, counts.3, counts.4, result.rows.len());
        #[cfg(not(feature = "count-alloc"))]
        println!("kind=sample\trep={rep}\twhole_query_ns={nanos}\tpre_query_max_rss={pre_query_rss}\tpost_query_max_rss={rss}\trows={}\toracle=pass", result.rows.len());
        #[cfg(feature = "count-alloc")]
        let _ = nanos; // Instrumented timing is deliberately not reported.
    }
}
```

