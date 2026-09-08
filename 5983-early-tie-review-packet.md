# Early oversized-leading-tie repair: focused review and measurement

Frozen local HEAD `dc23141d8c9e640e599085b0ed1616da35d8fc87`, parent `9e8bdfc95c916b62550fb8c3142f804bbbfb2444`. Actual GPT-6 Astra xhigh; historical attribution preserved. Two files, +78/-7, three new tests. No remote actions or independent model review in this stage.

**Decision: the focused repair removes measured allocation work; the full candidate remains NO-GO for admission.** Drained-prefix regressions and F3–F5 remain. The new check reuses the existing cap and fallback after seed sort/inline verification, before any probe materialization. Equality at logical index cap proves group length > cap; n > cap protects both directional indices. Exact-cap and larger-row-budget groups remain eligible.

46 default focused tests and29 compact-index tests pass. The only author preflight failure is the known Bash3.2 `mapfile` limitation at `scripts/check-privacy-claims.sh:92`; this was not bypassed. Diff whitespace passes and the worktree is clean.

The real early-return-disabled binary preserves semantic results and exactly reproduces the old allocation metrics. The fixed-work contract passes on new and exits1 for disabled-early bytes on each tied fixture: base avoids1,200,846 requested bytes /37 allocations; overlay avoids4,560,846 bytes /43 allocations. Favorable base metrics are identical. Tie query-heap peaks are unchanged because preparation was already freed before fallback's peak. The original late block guard remains.

The unchanged harness compares the new binary with all three original stored controls. 96 matrix process runs /320 query samples, plus6 allocation-control process runs /9 samples; all oracles/path checks pass. One sequential lane, two build jobs, one runtime Rayon thread; three optimized binaries built in56.6seconds. Same fixtures/features/lockfile, warmup2, timing7, allocation3. No outliers dropped.

| Fixture | New ms | Old ms | Disabled ms | Main ms | New/old | New/disabled |
|---|---:|---:|---:|---:|---:|---:|
| base k1 | 0.798 | 0.565 | 21.641 | 17.518 | 1.412 | 0.037 |
| base k512 | 1.214 | 1.267 | 18.815 | 16.624 | 0.958 | 0.065 |
| overlay k1 | 6.018 | 6.735 | 21.177 | 21.067 | 0.894 | 0.284 |
| overlay k512 | 7.035 | 7.160 | 21.958 | 22.340 | 0.982 | 0.320 |
| drained k1 | 13.763 | 12.260 | 9.897 | 9.255 | 1.123 | 1.391 |
| drained-overlay k1 | 24.641 | 23.927 | 15.732 | 16.028 | 1.030 | 1.566 |
| ties k1 | 19.187 | 22.350 | 20.460 | 18.475 | 0.858 | 0.938 |
| ties-overlay k1 | 22.746 | 28.457 | 29.756 | 22.545 | 0.799 | 0.764 |

These medians are diagnostic observations. Base ties overlap old timing ranges; overlay ties improve with disjoint old/new ranges, but baseline drift is material. Main/disabled have identical allocation metrics and up to about32% timing drift. All raw samples, standard deviation, range, load and cumulative RSS are in summary.json/measurements.json. No precise speedup claim follows. Drained base and overlay remain1.39×/1.57× disabled with disjoint ranges.

The benchmark times the whole query call; result checks/drop are outside. Counting is in a separate allocator binary. Requested bytes measure traffic, and tracked peak-live delta excludes allocator metadata/stack/mappings. RSS is cumulative process high-water after setup/warmup/query, not an isolated query peak. The repaired overlay tie still allocates its seed scan and exceeds disabled requested traffic by601,838bytes; this is not a cost-free decline.

Next smallest useful step: separately assess whether the initial drained block can be rejected using borrowed/lazy probe ranges before materializing every subject vector. This is an unimplemented design candidate, not permission to add heuristics or fanout. Prefer declining an unproved shape. F3 public tie semantics, F4 explicit trace, F5 path-pinned coverage/source questions remain before admission.

The full semantic prior packet remains immutable (SHA256 ef7b0dc4d5bd22642aa673516bdb40a33b83e8049f2447364d617bf96de18458), as does the original NO-GO diagnostic packet (SHA256 9aba74e161159a4fbfd98401cc1a4ceeab9cafd402a7df87883df9c388053af5). All39 and139 respective manifest entries were reverified. Source/binary/harness hashes, build commands, full whole-runtime diff and raw evidence accompany this packet; benchmark sources are copied byte-identically under source/bench/indexed-topk. The focused complete caller/callee and scan consumer context follows.

## Exact delta from reviewed semantic head

```diff
diff --git a/crates/sparq-engine/src/exec.rs b/crates/sparq-engine/src/exec.rs
index c0f63fb14..0d5b126b0 100644
--- a/crates/sparq-engine/src/exec.rs
+++ b/crates/sparq-engine/src/exec.rs
@@ -3333,6 +3333,21 @@ fn try_topk_orderby_indexed(
         return Ok(None);
     }
 
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
     // Prepare the other patterns ONCE, each as a SUBJECT-sorted scan over its
     // (fixed predicate [+ fixed object]) range — not re-resolved per candidate.
     // The first cut of this function called `graph.store.scan(&probe_pat)`
@@ -3440,7 +3455,6 @@ fn try_topk_orderby_indexed(
     // position 0 is always the best candidate, via `logical`, and do all
     // block/boundary arithmetic in that space. `logical` and `obj_id_at` are the
     // only direction-aware code; everything below them is direction-agnostic.
-    let n = rows.len();
     let logical = |i: usize| -> usize { if desc { n - 1 - i } else { i } };
     let obj_id_at = |i: usize| -> Id { scan.to_spo(&rows[logical(i)])[2] };
     // A single escalation block is dominated by ONE large tie-group when the
@@ -3455,12 +3469,10 @@ fn try_topk_orderby_indexed(
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
@@ -3473,7 +3485,6 @@ fn try_topk_orderby_indexed(
     // usually satisfies it — starting at 1024 would pay ~1024 point-probes even for
     // `LIMIT 1`, which is worse than the bulk path it's meant to beat (measured:
     // this was the actual cause of a regression at moderate `n`, not a win).
-    let max_group = (n / 2).max(MAX_INDEXED_GROUP_FLOOR).max(row_budget.saturating_mul(2));
     // Cumulative count of candidates that failed an OTHER-pattern check, across
     // ALL blocks in this call — distinct from `max_group`'s per-block width
     // check. A workload that claims strictly in priority order (the realistic
diff --git a/crates/sparq-engine/tests/topk_orderby_indexed_differential.rs b/crates/sparq-engine/tests/topk_orderby_indexed_differential.rs
index 6a56a2b75..46d740657 100644
--- a/crates/sparq-engine/tests/topk_orderby_indexed_differential.rs
+++ b/crates/sparq-engine/tests/topk_orderby_indexed_differential.rs
@@ -96,6 +96,66 @@ fn exact_desc_ties_preserve_valid_membership_and_window_size() {
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

## Complete affected functions and relevant consumers

### crates/sparq-engine/src/exec.rs:3110-3605

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

    // Prepare the other patterns ONCE, each as a SUBJECT-sorted scan over its
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
    struct OtherPat<'g> {
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
        obj_var: Option<Variable>,
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
        let probe_id_pat: IdPattern = [None, id_pat[1], id_pat[2]];
        let sub_scan = graph.store.scan_sorted(&probe_id_pat, 0);
        let sub_actual_sort = sub_scan.perm.order().into_iter().find(|&c| probe_id_pat[c].is_none());
        if sub_actual_sort != Some(0) {
            // This store build can't give a subject-sorted scan for this
            // pattern (e.g. no PSO permutation under `compact-index`/wasm) —
            // decline rather than binary-search an unsorted range.
            return Ok(None);
        }
        let subject_ids: Vec<Id> = sub_scan.rows.iter().map(|r| sub_scan.to_spo(r)[0]).collect();
        other_pats.push(OtherPat { scan: sub_scan, subject_ids, obj_var: pos_vars[2].clone() });
    }

    // UPFRONT cost check, before touching a single candidate: each `other_pats`
    // scan's row count is the EXACT (not estimated) global cardinality of that
    // pattern's own (predicate [+ object]) constraint. If any of them is
    // already meaningfully smaller than the seed's own scan (`rows.len()`),
    // the fallback's ordinary smallest-estimate seed selection will pick THAT
    // pattern as ITS seed and materialize only that small set — beating this
    // function's priority-ordered walk outright, with no reason to compete.
    //
    // This is exactly the realistic "claim strictly in priority order" shape:
    // as such a queue drains, `ak:status="pending"` becomes highly selective
    // while THIS function's seed (`ak:priority`, spanning the whole pool
    // including now-claimed rows) does not shrink at all. Without this check,
    // the only way to discover that is by actually walking past the
    // ever-growing already-claimed prefix, probing (and rejecting) each one —
    // real, wasted, unrecoverable cost. Measured directly: at n=1600 with
    // 1000 of 1600 already claimed (600 truly pending), that reactive
    // discovery cost ~237-272us total (wasted probes + the fallback anyway)
    // vs. this upfront check's ~172-180us (matches a clean fallback-only
    // cost, because it declines before doing ANY per-candidate work).
    let seed_card = rows.len();
    let min_other_card = other_pats.iter().map(|op| op.scan.rows.len()).min().unwrap_or(seed_card);
    if min_other_card.saturating_mul(2) < seed_card {
        return Ok(None);
    }

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
    // UPFRONT cost check above (comparing `other_pats`' exact cardinalities to
    // the seed's) already declines the CLEAR case — an other-pattern that's
    // globally selective enough for the fallback's own planner to prefer as
    // ITS seed — before any candidate is even touched. This counter is a
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
            for op in &other_pats {
                // Binary-search the PRECOMPUTED, plain-`Id` subject list for
                // this pattern (built once, above) — a trivial integer
                // compare per step, no `to_spo` reconstruction during the
                // search itself (that only happens below, per ACTUAL match,
                // not per comparison step — see `subject_ids`'s doc comment).
                let start = op.subject_ids.partition_point(|&id| id < hub_id);
                let stop = start + op.subject_ids[start..].partition_point(|&id| id == hub_id);
                if start == stop {
                    failed = true;
                    break;
                }
                let Some(_) = &op.obj_var else {
                    continue; // `obj_const` — existence-only, no column added
                };
                let op_rows: &[[Id; 3]] = op.scan.rows.as_ref();
                let match_count = stop - start;
                if match_count != 1 {
                    return Ok(None);
                }
                row_ids.push(op.scan.to_spo(&op_rows[start])[2]);
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

    let mut result = Bindings { vars: out_vars, rows: collected, sorted_by: None };
    let use_topk = result.rows.len() > row_budget;
    order_bindings(graph, local, &mut result, expression, if use_topk { Some(row_budget) } else { None })?;
    Ok(Some(result))
}

/// [OPUS-4.8] (sq-7d3dj.30.4) Attempts the DISTINCT-projection loose skip-scan for the
/// pattern under a `Distinct`.
///
/// Returns `Some(bindings)` when `inner` is `Project{[?p]}{ body }` (a SINGLE projected
/// variable) and every UNION branch of `body` is a filter-free BGP that admits direct
/// enumeration of the DISTINCT `?p` values from an existing permutation sorted by `?p`
/// (a loose/skip scan — the general form of qlever's pattern trick over the six
/// permutations, NO new index). Returns `None` on ANY other shape, so the caller falls
/// back to the full materialise-then-dedup path. Because the enumerated set is exactly
/// `{ v : ∃ a solution of the branch with ?p = v }`, unioned across branches, the result
/// is DISTINCT-set equivalent to the fallback by construction.
///
/// The zk completeness witness needs the whole PRE-distinct input set, so the pushdown
/// declines while the recorder is armed.
```

### crates/sparq-engine/tests/topk_orderby_indexed_differential.rs:1-158

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

// [GPT-6 Astra] A repeated variable must bind one term in both positions.
// An IRI subject and an integer object can never satisfy this triple pattern.
#[test]
fn repeated_seed_variable_full_orderby_is_empty() {
    let graph = Graph::load_str("<urn:s> <urn:p> 1 .", "turtle").unwrap();
    let rows = query(&graph, "SELECT ?x WHERE { ?x <urn:p> ?x } ORDER BY ?x").unwrap();
    assert!(rows.rows.is_empty(), "full ORDER BY must enforce subject/object equality: {rows:?}");
}

// [GPT-6 Astra] The LIMIT path must preserve the full evaluator's equality guard.
#[test]
fn repeated_seed_variable_limit_is_empty() {
    let graph = Graph::load_str("<urn:s> <urn:p> 1 .", "turtle").unwrap();
    let rows = query(&graph, "SELECT ?x WHERE { ?x <urn:p> ?x } ORDER BY ?x LIMIT 1").unwrap();
    assert!(rows.rows.is_empty(), "LIMIT must enforce subject/object equality: {rows:?}");
}

// [GPT-6 Astra] LIMIT cannot hide an oversized intermediate working set.
#[test]
fn indexed_topk_preserves_intermediate_resource_limits() {
    let graph = build_graph("X", &(0..40).map(|i| (i, i)).collect::<Vec<_>>(), "");
    let query_text = format!("{PFX}{CLAIM_QUERY}1");
    for (budget, expected) in [
        (sparq_engine::QueryBudget { max_rows: Some(2), ..Default::default() }, "max-rows"),
        (sparq_engine::QueryBudget { max_bytes: Some(16), ..Default::default() }, "max-bytes"),
    ] {
        let result = sparq_engine::query_with_budget(&graph, &query_text, &budget);
        assert!(result.as_ref().is_err_and(|error| error.contains(expected)), "{expected}: {result:?}");
    }
    let generous = sparq_engine::QueryBudget { max_rows: Some(1000), ..Default::default() };
    let result = sparq_engine::query_with_budget(&graph, &query_text, &generous).unwrap();
    assert_eq!(task_seq(result.rows[0][0].as_ref().unwrap()), 39);
}

// [GPT-6 Astra] Keep the initial shortcut out of Cartesian fanout. Check both
// the ordered answers and the actual fallback execution, not static plan text.
#[test]
fn multivalued_probe_uses_fallback_and_preserves_cartesian_rows() {
    let graph = Graph::load_str(
        "<urn:s> <urn:p> 2 ; <urn:b> 10, 11 ; <urn:c> 20, 21 .", "turtle",
    ).unwrap();
    let query_text = "SELECT ?s ?b ?c WHERE { ?s <urn:p> ?p ; <urn:b> ?b ; <urn:c> ?c } ORDER BY DESC(?p) ?b ?c";
    let full = query(&graph, query_text).unwrap();
    assert_eq!(full.rows.len(), 4);
    let limited_query = format!("{query_text} LIMIT 3");
    let limited = query(&graph, &limited_query).unwrap();
    assert_eq!(limited.rows, full.rows[..3]);
    let trace = sparq_engine::explain_analyze(&graph, &limited_query).unwrap();
    assert!(trace.contains("BGP [binary GOO]"), "multivalued probe must execute the fallback: {trace}");
}

// [GPT-6 Astra] These two keys give a total order, so every OFFSET/LIMIT window
// must equal the same full-order prefix/window, including complete tie boundaries.
#[test]
fn total_order_desc_offset_windows_match_full_sort() {
    let graph = build_graph("X", &(0..96).map(|i| (i, i % 12)).collect::<Vec<_>>(), "");
    let text = format!("{PFX}SELECT ?t WHERE {{ ?t ak:peer <urn:peer:X> ; ak:status \"pending\" ; ak:priority ?p ; ak:seq ?s }} ORDER BY DESC(?p) DESC(?s)");
    let full = query(&graph, &text).unwrap();
    assert_eq!(full.rows.len(), 96);
    for (offset, limit) in [(0, 0), (0, 1), (7, 2), (8, 9), (15, 3), (31, 17), (95, 4), (96, 1)] {
        let got = query(&graph, &format!("{text} OFFSET {offset} LIMIT {limit}")).unwrap();
        let expected: Vec<_> = full.rows.iter().skip(offset).take(limit).cloned().collect();
        assert_eq!(got.rows, expected, "offset={offset}, limit={limit}");
    }
}

// [GPT-6 Astra] With no secondary key, SPARQL does not specify which tied
// subjects survive. Assert valid membership, multiplicity and size, not stability.
#[test]
fn exact_desc_ties_preserve_valid_membership_and_window_size() {
    let graph = build_graph("X", &(0..32).map(|i| (i, 7)).collect::<Vec<_>>(), "");
    let text = format!("{PFX}SELECT ?t ?p WHERE {{ ?t ak:peer <urn:peer:X> ; ak:status \"pending\" ; ak:priority ?p }} ORDER BY DESC(?p)");
    for (offset, limit) in [(0usize, 5usize), (3, 9), (28, 10), (40, 2), (0, 0)] {
        let got = query(&graph, &format!("{text} OFFSET {offset} LIMIT {limit}")).unwrap();
        assert_eq!(got.rows.len(), 32usize.saturating_sub(offset).min(limit));
        let ids: std::collections::HashSet<_> = got.rows.iter()
            .map(|row| task_seq(row[0].as_ref().unwrap())).collect();
        assert_eq!(ids.len(), got.rows.len(), "no duplicate solution may be manufactured");
        assert!(ids.iter().all(|id| (0..32).contains(id)));
        assert!(got.rows.iter().all(|row| as_int(row[1].as_ref().unwrap()) == 7));
    }
}

// [GPT-6 Astra] Exactly-at-cap groups remain eligible; one more must decline.
// A distinguishing secondary key makes the OFFSET oracle deterministic.
#[test]
fn leading_tie_cap_boundaries_preserve_both_directions_and_offset() {
    let supported = sparq_core::store::BUILT.contains(&sparq_core::store::Perm::Pso);
    for (n, cap) in [(400, 256), (600, 300)] {
        for desc in [false, true] {
            for group in [cap, cap + 1] {
                let tasks: Vec<_> = (0..n).map(|i| {
                    let key = if i < group { if desc { 1000 } else { 0 } } else { i + 1 };
                    (i, key)
                }).collect();
                let graph = build_graph("X", &tasks, "");
                let direction = if desc { "DESC" } else { "ASC" };
                let text = format!("{PFX}SELECT ?t WHERE {{ ?t ak:peer <urn:peer:X> ; ak:status \"pending\" ; ak:priority ?p ; ak:seq ?s }} ORDER BY {direction}(?p) DESC(?s)");
                let full = query(&graph, &text).unwrap();
                let limited = format!("{text} OFFSET 1 LIMIT 1");
                assert_eq!(query(&graph, &limited).unwrap().rows, full.rows[1..2]);
                let trace = sparq_engine::explain_analyze(&graph, &limited).unwrap();
                assert_eq!(trace.contains("BGP [binary GOO]"), group > cap || !supported,
                           "n={n} group={group} direction={direction}: {trace}");
            }
        }
    }
}

// [GPT-6 Astra] The cap still includes twice the requested row budget.
#[test]
fn leading_tie_cap_preserves_large_row_budget_admission() {
    let tasks: Vec<_> = (0..600).map(|i| (i, if i < 400 { 1000 } else { i })).collect();
    let graph = build_graph("X", &tasks, "");
    assert_eq!(actual_top_k(&graph, 300), expected_top_k(&graph, 300));
    let trace = sparq_engine::explain_analyze(&graph, &format!("{PFX}{CLAIM_QUERY}300")).unwrap();
    let supported = sparq_core::store::BUILT.contains(&sparq_core::store::Perm::Pso);
    assert_eq!(!trace.contains("BGP [binary GOO]"), supported, "{trace}");
}

// [GPT-6 Astra] Tombstones alter both the group extent and the existing n/2 cap.
#[test]
fn leading_tie_cap_observes_overlay_deletions() {
    for desc in [false, true] {
        let key = if desc { 1000 } else { 0 };
        let tasks: Vec<_> = (0..600).map(|i| (i, if i < 301 { key } else { i + 1 })).collect();
        let base = build_graph("X", &tasks, "");
        let mut changed = base.fork();
        sparq_engine::update_in_place(&mut changed, &format!("{PFX}DELETE DATA {{ <urn:task:X:0> ak:priority {key} . <urn:task:X:1> ak:priority {key} }}")).unwrap();
        assert_eq!(changed.pending_delta_len(), 2);
        let direction = if desc { "DESC" } else { "ASC" };
        let text = format!("{PFX}SELECT ?t WHERE {{ ?t ak:peer <urn:peer:X> ; ak:status \"pending\" ; ak:priority ?p ; ak:seq ?s }} ORDER BY {direction}(?p) DESC(?s)");
        let limited = format!("{text} OFFSET 1 LIMIT 1");
        let full = query(&changed, &text).unwrap();
        assert_eq!(query(&changed, &limited).unwrap().rows, full.rows[1..2]);
        let before = sparq_engine::explain_analyze(&base, &limited).unwrap();
        let after = sparq_engine::explain_analyze(&changed, &limited).unwrap();
        assert!(before.contains("BGP [binary GOO]"), "original 301 > 300: {before}");
        let supported = sparq_core::store::BUILT.contains(&sparq_core::store::Perm::Pso);
        assert_eq!(!after.contains("BGP [binary GOO]"), supported, "remaining 299 == 598/2: {after}");
    }
}

```

### crates/sparq-core/src/store.rs:1020-1137

```rust
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

/// A range of rows in a permutation's column order. Borrowed from the raw index, or
/// owned when decoded from a compressed permutation — uniformly a `&[[Id;3]]` to callers.
pub struct Scan<'a> {
    pub rows: std::borrow::Cow<'a, [[Id; 3]]>,
    pub perm: Perm,
}

impl<'a> Scan<'a> {
    /// Maps a stored row back to a canonical s,p,o triple.
    #[inline]
    pub fn to_spo(&self, row: &[Id; 3]) -> [Id; 3] {
        let order = self.perm.order();
        let mut out = [0; 3];
        out[order[0]] = row[0];
        out[order[1]] = row[1];
        out[order[2]] = row[2];
        out
    }
}

/// First index where `rows[i] >= key` comparing only the leading columns that
```

### crates/sparq-core/src/dict.rs:1-25

```rust
//! Term dictionary: bijection between RDF terms and dense `u32` ids.
//!
//! Dictionary encoding is the foundation of every fast triplestore (RDF-3X,
//! QLever, RDFox): triples are stored and joined as fixed-width integers, and the
//! (large, string-heavy) terms live once in the dictionary. Terms are stored COMPACT
//! and prefix-factored (IRIs share a namespace table); the interner is single-storage
//! (the hash table holds only ids) and content-addressed by a hash reproducible from
//! either an `oxrdf::Term` or the parsed byte components — so a byte-level parser can
//! intern straight from slices with no intermediate `Term`.

use hashbrown::HashTable;
use oxrdf::vocab::xsd;
use oxrdf::{Literal, NamedNode, Term};
use rustc_hash::FxHashMap;
use std::hash::Hasher;

/// A dense term id. `u32` (≤ 4.29 B distinct terms) keeps index entries small
/// and cache-friendly; the id space is widened to `u64` only if a dataset needs
/// it. Id 0 is reserved as a sentinel ("no such term").
pub type Id = u32;

pub const NO_ID: Id = 0;

/// Tagged-ValueId base: ids in `[INLINE_BASE, INLINE_BASE + 2^30)` encode an
/// `xsd:integer` whose value is `id - INLINE_BASE` *inline* — no dictionary entry,
```

