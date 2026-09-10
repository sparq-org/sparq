# Issue3105 focused coverage and stale-slot lifetime delta

Actual GPT-6 Astra xhigh. This extends the reviewed6e86 candidate; prior source review approved validation, not admission. Full phase1/performance bundles remain unchanged.

{
  "head": "19763bfab1dce196a654c899b172e7b24d70bc59",
  "parent": "6e86f1d0ba447aa78a50800337a7a379702fa999",
  "main_base": "e53464c73f31f7aca800f3867ac054c36408e346",
  "author": "Actual GPT-6 Astra xhigh; existing historical attribution preserved; ordinary unsigned local commit, no persistent config or hook changes.",
  "scope": "One file: crates/sparq-engine/src/exec.rs, +260/-2 from reviewed6e86. One production statement clears stale slot before scan; other changes are cfg(test) instrumentation/tests. Existing test suffix byte-identical; no API/dependency/storage/cache-policy/planner/hash-build change.",
  "results": {
    "focused_default": "7 passed",
    "focused_no_engine_defaults": "7 passed; core dev dependencies still enable parallel/mmap/dict-spill, not a lean wasm run",
    "focused_compact_index": "7 passed; explicit three-permutation hash/order/reuse expectations",
    "focused_semijoin_bitmap": "7 passed",
    "ask_early_exit": "21 passed, 1 existing ignored timing probe",
    "clippy": "cargo clippy --locked --offline -p sparq-engine --lib --tests -- -D warnings: PASS",
    "diff_check": "PASS",
    "preflight": "FAIL solely because Bash3 lacks mapfile in privacy-claims script; no workaround/weakening. Linux preflight required."
  },
  "actual_paths": {
    "six_permutations": "70000-row public SELECT LIMIT70001, projecting35000 integer values twice each; full query bag AND generated exact bag agree. q pattern1: block0 bind, block1024 merge(requestSome0, scanned), block65536 bind. r pattern2: hash(None,scan), merge(Some0,scan), hash(None,scan); actual RHS order s throughout.",
    "three_permutations": "Same complete oracle and multiplicity pass. q bind/hash/bind; r requestsNone in allthree blocks, actual orderx, scans once then reuses twice. Requested and actual order are not conflated. Explicit branch differs because the built index set lacks PSO.",
    "disconnected": "Public capped query executes one cross_product_ref RHS step, produces six rows and exact two-value bag with multiplicitythree; full query agrees. Small one-block cross, not a multi-block memory stress.",
    "named_view_overlay": "Eligible named subquery reaches RHS scan; visible view with empty default retains same bag; hidden view returns zero rows and no RHS steps. Tombstone removes one q row from a fork and eligible count decreases3\u21922. Correlated EXISTS variant returns projected multiplicity2 then1 after deletion.",
    "exists_boundary": "Initial desired EXISTS engagement assertion failed: is_conjunctive/filter_scope_ok deliberately reject EXISTS. Final test pins zero RHS-cache steps for this fallback and separately exercises eligible named/view/overlay queries. No guard broadened or forced eligibility."
  },
  "lifetime": "Rust assignment evaluates replacement RHS before dropping old LHS; old Some((sort,Bindings)) therefore retained rows through scan(). New *slot=None drops old ownership before calling scan. Actual-helper catch_unwind sentinel proves slot empty after replacement scan aborts; normal replacement still yields expected row. This is observable ownership-state proof, not allocator/time measurement, and catches unwind only.",
  "controls": [
    {
      "name": "ignore-requested-sort",
      "test": "capped_rhs_changing_sort_mixed_kernels_preserves_full_bag",
      "killed_by_executed_assertion": true,
      "exit": 101,
      "binary_sha256": "d5e1b4a21decea1fd2da895975ebd0a83b0825b86b69522f7826b9a99061f077",
      "binary_bytes": 28091616,
      "interpretation": "Path/invalidation assertion after all full-bag assertions passed, not a demonstrated wrong result."
    },
    {
      "name": "retain-old-through-scan",
      "test": "capped_rhs_replacement_releases_old_slot_before_scan",
      "killed_by_executed_assertion": true,
      "exit": 101,
      "binary_sha256": "06a68809c7220d5b83abf8395250df06b23789a44f08d46e7657c9b9f5b504fe",
      "binary_bytes": 28091616,
      "interpretation": "Actual slot remains populated after replacement scan panic; candidate empties it before calling scan."
    }
  ],
  "controls_limits": "Requested-sort mutant returns the same full bag on this fixture, then fails actual scanned/requested-path assertion. This is an invalidation/work control, not a demonstrated semantic failure. Lifetime mutant keeps actual old slot populated after sentinel panic. Expected caught panic output is not an unexpected test failure. Initial and final control executions retained; final controls use exact final source plus documented mutation.",
  "iterations": "First two compile attempts corrected test import placement and query_view arity; third execution exposed genuine existing EXISTS fallback, not production failure. Initial compact run preserved full bag but correctly differed from assumed six-index path; final index-set-specific assertions document actual three-index behavior. All initial logs/source snapshots retained. Old test formatting restored to keep prior source byte-identical.",
  "measurements": "No new timings or allocation matrix. Prior exact6e86 positive local performance screen and all frozen binaries/artifacts remain unchanged and verified. Do not attribute those timings to this new head or claim a measured peak reduction from slot clear.",
  "limits": [
    "No full workspace clippy/tests, wasm build/byte equality, conformance or canonical perf ratchet run here; authoritative CI required before admission.",
    "No midquery asynchronous cancellation injection; existing cancellation/deadline/rowbudget tests and armed-budget nonreuse checks passed unchanged.",
    "Sum of retained per-pattern RHS remains a resource risk outside the measured two-RHS fixture. Releasing stale per-slot data only avoids replacement overlap; it is not a retention cap or budget policy change.",
    "Nested-public-query budget issue6476 remains untouched. No production query policy or guard changes beyond stale slot lifetime.",
    "No blanket all-features claim: tested default, no engine defaults, core compact-index and semijoin-bitmap configurations only.",
    "No remote action, model call, registry/release/EC2 operation or further lane started."
  ],
  "decision": "Ready for actual focused Opus review and subsequent full validation; no merge, performance-admission or issue-closure claim.",
  "resource": {
    "started_utc": "2026-09-10T01:36:22Z",
    "completed_utc": "2026-09-10T01:52:21.964093+00:00",
    "free_bytes": 11899613184,
    "min_observed_command_free_bytes": 12185391104,
    "floor_bytes": 6509559808,
    "jobs": 2,
    "incremental": false,
    "offline_locked": true,
    "commands_pending": false
  },
  "prior_verified": [
    {
      "path": "/private/tmp/sparq-pr6049/.throughput-monitor/direct-3105/manifest.json",
      "sha256": "188a6cffeb664756aec40923d18da3ea905ca217b76f654fcb42e7a863676f48",
      "verified_files": 59
    },
    {
      "path": "/private/tmp/sparq-pr6049/.throughput-monitor/direct-3105/performance/manifest.json",
      "sha256": "9cd9164058fa95e142633787c30cd9d00a65463e975de28f3b46166503f87bed",
      "verified_files": 21
    },
    {
      "path": "/private/tmp/sparq-pr6049/.throughput-monitor/direct-3105/performance-completion/manifest.json",
      "sha256": "e97fa98a8fcaae33c78db2c40f85881c2f0417d41e177972f82979c02f24d874",
      "verified_files": 515
    }
  ]
}

## Exact delta from6e86

```diff
diff --git a/crates/sparq-engine/src/exec.rs b/crates/sparq-engine/src/exec.rs
index f163494cd..d4d496f9d 100644
--- a/crates/sparq-engine/src/exec.rs
+++ b/crates/sparq-engine/src/exec.rs
@@ -4244,6 +4244,8 @@ fn eval_bgp_binary_capped(
                 let pp = prepared[i].var_pos(jv).unwrap();
                 #[cfg(test)]
                 capped_rhs_tests::observe(3);
+                #[cfg(test)]
+                capped_rhs_tests::step(start, i, "bind", None, false, None);
                 result = bind_join(graph, result, &prepared[i].id_pat, &prepared[i].pos_vars, rk, pp, pfilter(i));
             } else {
                 let filt = pfilter(i);
@@ -4251,9 +4253,14 @@ fn eval_bgp_binary_capped(
                 let scan_sort = filt.map(|(c, _)| c).or_else(|| merge_var.as_ref().map(|jv| prepared[i].var_pos(jv).unwrap()));
                 let mut uncached = None;
                 let slot = if reuse_rhs { &mut rhs_cache[i] } else { &mut uncached };
+                #[cfg(test)]
+                let mut scanned = false;
                 let rhs = capped_rhs(slot, scan_sort, || {
                     #[cfg(test)]
-                    capped_rhs_tests::observe(2);
+                    {
+                        capped_rhs_tests::observe(2);
+                        scanned = true;
+                    }
                     scan_to_bindings(
                         graph,
                         &prepared[i].id_pat,
@@ -4267,10 +4274,16 @@ fn eval_bgp_binary_capped(
                 });
                 let connected = prepared[i].pos_vars.iter().flatten().any(|v| result.vars.contains(v));
                 if let Some(jv) = merge_var.filter(|jv| rhs.sorted_by.as_ref() == Some(jv)) {
+                    #[cfg(test)]
+                    capped_rhs_tests::step(start, i, "merge", scan_sort, scanned, rhs.sorted_by.as_ref());
                     result = merge_join_ref(&result, rhs, &jv);
                 } else if connected {
+                    #[cfg(test)]
+                    capped_rhs_tests::step(start, i, "hash", scan_sort, scanned, rhs.sorted_by.as_ref());
                     result = hash_join_ref(&result, rhs);
                 } else {
+                    #[cfg(test)]
+                    capped_rhs_tests::step(start, i, "cross", scan_sort, scanned, rhs.sorted_by.as_ref());
                     result = cross_product_ref(&result, rhs);
                 }
             }
@@ -4325,6 +4338,9 @@ fn capped_rhs(
         .as_ref()
         .is_none_or(|(cached_sort, _)| *cached_sort != sort)
     {
+        // [GPT-6 Astra] Release the stale relation before materializing its replacement.
+        // No subsequent step can borrow the old requested order from this slot.
+        *slot = None;
         *slot = Some((sort, scan()));
     }
     &slot.as_ref().unwrap().1
@@ -21233,12 +21249,254 @@ mod order_bindings_worker_reinstall {
 #[cfg(test)]
 mod capped_rhs_tests {
     use super::*;
-    use std::cell::Cell;
+    use std::cell::{Cell, RefCell};
 
     thread_local! {
         static WORK: Cell<[usize; 4]> = const { Cell::new([0; 4]) };
+        static STEPS: RefCell<Option<Vec<Step>>> = const { RefCell::new(None) };
+    }
+
+    // [GPT-6 Astra] Observe the actual branch, scan closure and returned metadata.
+    #[derive(Debug, PartialEq, Eq)]
+    struct Step {
+        start: usize,
+        pattern: usize,
+        kernel: &'static str,
+        requested: Option<usize>,
+        scanned: bool,
+        actual: Option<String>,
+    }
+
+    pub(super) fn step(
+        start: usize,
+        pattern: usize,
+        kernel: &'static str,
+        requested: Option<usize>,
+        scanned: bool,
+        actual: Option<&Variable>,
+    ) {
+        STEPS.with_borrow_mut(|steps| {
+            if let Some(steps) = steps {
+                steps.push(Step {
+                    start,
+                    pattern,
+                    kernel,
+                    requested,
+                    scanned,
+                    actual: actual.map(|v| v.as_str().to_owned()),
+                });
+            }
+        });
+    }
+
+    fn trace<T>(f: impl FnOnce() -> T) -> (T, Vec<Step>) {
+        STEPS.with_borrow_mut(|steps| *steps = Some(Vec::new()));
+        let result = f();
+        let steps = STEPS.with_borrow_mut(|steps| steps.take().unwrap());
+        (result, steps)
+    }
+
+    fn bag(result: &crate::QueryResult) -> std::collections::BTreeMap<Vec<String>, usize> {
+        let mut bag = std::collections::BTreeMap::new();
+        for row in &result.rows {
+            *bag.entry(
+                row.iter()
+                    .map(|v| v.as_ref().unwrap().to_string())
+                    .collect(),
+            )
+            .or_default() += 1;
+        }
+        bag
     }
 
+    #[test]
+    fn capped_rhs_changing_sort_mixed_kernels_preserves_full_bag() {
+        let mut ttl = String::from("@prefix : <http://ex/> .\n");
+        for i in 0..70_000 {
+            // Projection deliberately collapses pairs, making multiplicity observable.
+            ttl.push_str(&format!(":s{i} :p {} ; :q {i} ; :r {i} .\n", i / 2));
+        }
+        ttl.push_str(":extra1 :q 70001 ; :r 70001 . :extra2 :r 70002 .");
+        let graph = Graph::load_str(&ttl, "turtle").unwrap();
+        let query = "PREFIX : <http://ex/> SELECT ?o WHERE { ?s :p ?o . ?s :q ?x . ?s :r ?x . FILTER(?o + 0 >= 0) }";
+        let full = crate::query(&graph, query).unwrap();
+        let (limited, steps) =
+            trace(|| crate::query(&graph, &format!("{query} LIMIT 70001")).unwrap());
+        println!("changing-sort actual steps: {steps:?}");
+        assert_eq!(limited.rows.len(), 70_000);
+        assert_eq!(bag(&limited), bag(&full));
+        let expected: std::collections::BTreeMap<_, _> = (0..35_000)
+            .map(|i| (vec![oxrdf::Literal::from(i).to_string()], 2))
+            .collect();
+        assert_eq!(bag(&limited), expected);
+        let q: Vec<_> = steps
+            .iter()
+            .filter(|s| s.pattern == 1)
+            .map(|s| {
+                (
+                    s.start,
+                    s.kernel,
+                    s.requested,
+                    s.scanned,
+                    s.actual.as_deref(),
+                )
+            })
+            .collect();
+        let r: Vec<_> = steps
+            .iter()
+            .filter(|s| s.pattern == 2)
+            .map(|s| (s.start, s.requested, s.scanned, s.actual.as_deref()))
+            .collect();
+        if sparq_core::store::BUILT.contains(&sparq_core::store::Perm::Pso) {
+            assert_eq!(
+                q,
+                [
+                    (0, "bind", None, false, None),
+                    (1024, "merge", Some(0), true, Some("s")),
+                    (65536, "bind", None, false, None)
+                ]
+            );
+            assert_eq!(
+                r,
+                [
+                    (0, None, true, Some("s")),
+                    (1024, Some(0), true, Some("s")),
+                    (65536, None, true, Some("s"))
+                ]
+            );
+        } else {
+            // Three permutations return object order: no false subject-order claim,
+            // and the unchanged None request must reuse the actually unsorted-for-s RHS.
+            assert_eq!(
+                q,
+                [
+                    (0, "bind", None, false, None),
+                    (1024, "hash", None, true, Some("x")),
+                    (65536, "bind", None, false, None)
+                ]
+            );
+            assert_eq!(
+                r,
+                [
+                    (0, None, true, Some("x")),
+                    (1024, None, false, Some("x")),
+                    (65536, None, false, Some("x"))
+                ]
+            );
+        }
+    }
+
+    #[test]
+    fn capped_rhs_replacement_releases_old_slot_before_scan() {
+        let mut slot = Some((
+            None,
+            Bindings::unsorted(
+                vec![Variable::new("x").unwrap()],
+                vec![Row::from_slice(&[1])],
+            ),
+        ));
+        // A failed replacement leaves the actual slot empty only if old ownership
+        // was released before invoking the scan. This is not a timing/heap estimate.
+        let failed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
+            capped_rhs(&mut slot, Some(0), || panic!("replacement scan sentinel"));
+        }));
+        assert!(failed.is_err());
+        assert!(
+            slot.is_none(),
+            "old RHS remained live through replacement scan"
+        );
+        let replacement = capped_rhs(&mut slot, Some(0), || {
+            Bindings::unsorted(
+                vec![Variable::new("x").unwrap()],
+                vec![Row::from_slice(&[2])],
+            )
+        });
+        assert_eq!(replacement.rows, [Row::from_slice(&[2])]);
+    }
+
+    #[test]
+    fn capped_rhs_disconnected_cross_preserves_multiplicity() {
+        let graph = Graph::load_str(
+            "@prefix : <http://ex/> . :a :p 1 . :b :p 2 . :c :q 3, 4, 5 .",
+            "turtle",
+        )
+        .unwrap();
+        let query =
+            "PREFIX : <http://ex/> SELECT ?s WHERE { ?s :p ?o . ?t :q ?x . FILTER(?o + 0 >= 0) }";
+        let (limited, steps) = trace(|| crate::query(&graph, &format!("{query} LIMIT 7")).unwrap());
+        assert_eq!(bag(&limited), bag(&crate::query(&graph, query).unwrap()));
+        assert_eq!(limited.rows.len(), 6);
+        assert_eq!(bag(&limited).values().copied().collect::<Vec<_>>(), [3, 3]);
+        assert_eq!(steps.len(), 1);
+        assert_eq!(steps[0].kernel, "cross");
+        assert!(steps[0].scanned);
+        println!("disconnected actual steps: {steps:?}");
+    }
+
+    #[test]
+    fn capped_rhs_named_view_overlay_and_residual_exists() {
+        let mut graph = Graph::load_dataset("@prefix : <http://ex/> . :g { :a :p 1, 2 ; :q 1, 2 ; :visible true . :b :p 3 ; :q 3 . }", "trig").unwrap();
+        let query = "PREFIX : <http://ex/> SELECT ?s WHERE { GRAPH :g { SELECT ?s WHERE { ?s :p ?o . ?s :q ?o . FILTER(?o + 0 >= 0 && EXISTS { ?s :visible true }) } LIMIT 10 } }";
+        let (base, steps) = trace(|| crate::query(&graph, query).unwrap());
+        assert_eq!(base.rows.len(), 2);
+        assert_eq!(base.rows[0], base.rows[1]);
+        // EXISTS is deliberately non-conjunctive: prove the existing fallback remains.
+        assert!(
+            steps.is_empty(),
+            "EXISTS must retain the scope-safe fallback"
+        );
+        let eligible = query.replace(" && EXISTS { ?s :visible true }", "");
+        let (eligible_base, eligible_steps) = trace(|| crate::query(&graph, &eligible).unwrap());
+        assert_eq!(eligible_base.rows.len(), 3);
+        assert!(
+            eligible_steps.iter().any(|s| s.scanned),
+            "named subquery must reach capped RHS: {eligible_steps:?}"
+        );
+        let visible = crate::DatasetView {
+            base: &graph,
+            named: std::sync::Arc::new([graph.named[0].0.clone()].into_iter().collect()),
+            default: crate::DefaultGraphMode::Empty,
+        };
+        assert_eq!(
+            bag(&crate::query_view(&visible, query).unwrap()),
+            bag(&base)
+        );
+        let (visible_rows, visible_steps) =
+            trace(|| crate::query_view(&visible, &eligible).unwrap());
+        assert_eq!(bag(&visible_rows), bag(&eligible_base));
+        assert!(visible_steps.iter().any(|s| s.scanned));
+        let hidden = crate::DatasetView {
+            base: &graph,
+            named: std::sync::Arc::new(Default::default()),
+            default: crate::DefaultGraphMode::StoreDefault,
+        };
+        assert!(crate::query_view(&hidden, query).unwrap().rows.is_empty());
+        let (hidden_rows, hidden_steps) = trace(|| crate::query_view(&hidden, &eligible).unwrap());
+        assert!(hidden_rows.rows.is_empty());
+        assert!(
+            hidden_steps.is_empty(),
+            "hidden graph must not enter RHS cache"
+        );
+        let mut fork = graph.named[0].1.fork();
+        fork.apply_delta(
+            &[],
+            &[[
+                oxrdf::NamedNode::new("http://ex/a").unwrap().into(),
+                oxrdf::NamedNode::new("http://ex/q").unwrap().into(),
+                oxrdf::Literal::from(2).into(),
+            ]],
+        )
+        .unwrap();
+        graph.named[0].1 = fork;
+        let (changed, changed_steps) = trace(|| crate::query(&graph, query).unwrap());
+        assert_eq!(changed.rows.len(), 1);
+        assert_eq!(changed.rows[0], base.rows[0]);
+        assert!(changed_steps.is_empty(), "overlay EXISTS retains fallback");
+        let (eligible_overlay, overlay_steps) = trace(|| crate::query(&graph, &eligible).unwrap());
+        assert_eq!(eligible_overlay.rows.len(), 2);
+        assert!(overlay_steps.iter().any(|s| s.scanned));
+        println!("named EXISTS fallback: {steps:?}, {changed_steps:?}; eligible base/overlay: {eligible_steps:?}, {overlay_steps:?}");
+    }
     pub(super) fn observe(index: usize) {
         WORK.with(|cell| {
             let mut counts = cell.get();
```

## context-driver-helper.rs

```rust
fn eval_bgp_binary_capped(
    graph: &Graph,
    local: &mut LocalVocab,
    patterns: &[TriplePattern],
    filters: &[Expression],
    cap: usize,
) -> Result<Option<Bindings>, String> {
    if !bgp_uses_binary(patterns) {
        return Ok(None); // cyclic -> LFTJ: no capped variant (v1 boundary)
    }
    let (_, constraints) = extract_quoted_constraints(patterns);
    if !constraints.is_empty() {
        return Ok(None); // triple-term decomposition: keep the full path
    }
    let (pat_filters, residual) = split_sargable(graph, patterns, filters);
    let pfilter = |i: usize| -> Option<(usize, ScanCmp)> { pat_filters.get(i).copied().flatten() };
    let prepared = prepare_bgp(graph, patterns)?;
    if prepared.iter().any(|p| p.unsatisfiable) {
        return Ok(Some(Bindings::unsorted(collect_vars(patterns), vec![])));
    }
    if cap == 0 {
        // Zero rows requested: an empty partial answer is valid under the contract
        // (`|result| >= cap` — the caller's slice truncates to nothing either way).
        return Ok(Some(Bindings::unsorted(collect_vars(patterns), vec![])));
    }
    let seed = goo_seed(&prepared);
    let seed_sort_col = goo_seed_sort(&prepared, seed, pfilter(seed).map(|(c, _)| c));
    let seed_all = scan_to_bindings(
        graph,
        &prepared[seed].id_pat,
        &prepared[seed].pos_vars,
        seed_sort_col,
        pfilter(seed),
        None,
        #[cfg(feature = "semijoin-bitmap")]
        None,
    );

    #[cfg(test)]
    capped_rhs_tests::observe(0);
    // [GPT-6 Astra] Query-local and lazy: never scan an unreached step. Pattern
    // filters are immutable here; each slot also keys the requested scan order.
    // Armed budgets retain the old per-step lifetime and polling: their working-set
    // estimate does not account for multiple retained RHS relations.
    let reuse_rhs = !budget::active();
    let mut rhs_cache: Vec<Option<(Option<usize>, Bindings)>> =
        std::iter::repeat_with(|| None).take(if reuse_rhs { prepared.len() } else { 0 }).collect();
    let mut acc: Option<Bindings> = None;
    let mut start = 0usize;
    while start < seed_all.rows.len() {
        #[cfg(test)]
        capped_rhs_tests::observe(1);
        let end = if start == 0 {
            CAPPED_SEED_BLOCK.min(seed_all.rows.len())
        } else if start == CAPPED_SEED_BLOCK {
            CAPPED_SEED_BLOCK_2.min(seed_all.rows.len())
        } else {
            seed_all.rows.len()
        };
        // One seed slice through the whole chain. A contiguous slice of a sorted
        // scan stays sorted, so merge-join eligibility is preserved.
        let mut result = Bindings {
            vars: seed_all.vars.clone(),
            rows: seed_all.rows[start..end].to_vec(),
            sorted_by: seed_all.sorted_by.clone(),
        };
        let mut cs_ctx = CsCtx::new(&prepared);
        let mut done = vec![false; prepared.len()];
        done[seed] = true;
        cs_ctx.note_done(seed);
        let mut cur_card = prepared[seed].est as f64;
        let mut var_ndv: FxHashMap<Variable, f64> = FxHashMap::default();
        record_pattern_ndv(graph, &prepared, seed, cur_card, &mut var_ndv, &cs_ctx);
        for _ in 1..prepared.len() {
            let (i, new_card, _connected) = goo_pick(graph, &prepared, &done, &var_ndv, cur_card, &cs_ctx);
            cur_card = new_card;
            done[i] = true;
            cs_ctx.note_done(i);
            // Same step selection as `eval_bgp_binary`: bind-join when the running
            // block is much smaller than the pattern; else merge / hash / cross.
            let connecting: Vec<Variable> =
                result.vars.iter().filter(|v| prepared[i].var_pos(v).is_some()).cloned().collect();
            if connecting.len() == 1
                && distinct_pattern_vars(&prepared[i].pos_vars)
                && result.rows.len().saturating_mul(8) < prepared[i].est
            {
                let jv = &connecting[0];
                let rk = result.col(jv).unwrap();
                let pp = prepared[i].var_pos(jv).unwrap();
                #[cfg(test)]
                capped_rhs_tests::observe(3);
                #[cfg(test)]
                capped_rhs_tests::step(start, i, "bind", None, false, None);
                result = bind_join(graph, result, &prepared[i].id_pat, &prepared[i].pos_vars, rk, pp, pfilter(i));
            } else {
                let filt = pfilter(i);
                let merge_var = result.sorted_by.clone().filter(|sv| prepared[i].var_pos(sv).is_some());
                let scan_sort = filt.map(|(c, _)| c).or_else(|| merge_var.as_ref().map(|jv| prepared[i].var_pos(jv).unwrap()));
                let mut uncached = None;
                let slot = if reuse_rhs { &mut rhs_cache[i] } else { &mut uncached };
                #[cfg(test)]
                let mut scanned = false;
                let rhs = capped_rhs(slot, scan_sort, || {
                    #[cfg(test)]
                    {
                        capped_rhs_tests::observe(2);
                        scanned = true;
                    }
                    scan_to_bindings(
                        graph,
                        &prepared[i].id_pat,
                        &prepared[i].pos_vars,
                        scan_sort,
                        filt,
                        None,
                        #[cfg(feature = "semijoin-bitmap")]
                        None,
                    )
                });
                let connected = prepared[i].pos_vars.iter().flatten().any(|v| result.vars.contains(v));
                if let Some(jv) = merge_var.filter(|jv| rhs.sorted_by.as_ref() == Some(jv)) {
                    #[cfg(test)]
                    capped_rhs_tests::step(start, i, "merge", scan_sort, scanned, rhs.sorted_by.as_ref());
                    result = merge_join_ref(&result, rhs, &jv);
                } else if connected {
                    #[cfg(test)]
                    capped_rhs_tests::step(start, i, "hash", scan_sort, scanned, rhs.sorted_by.as_ref());
                    result = hash_join_ref(&result, rhs);
                } else {
                    #[cfg(test)]
                    capped_rhs_tests::step(start, i, "cross", scan_sort, scanned, rhs.sorted_by.as_ref());
                    result = cross_product_ref(&result, rhs);
                }
            }
            record_pattern_ndv(graph, &prepared, i, cur_card, &mut var_ndv, &cs_ctx);
            if result.rows.is_empty() {
                break;
            }
        }
        // Residual FILTERs per block — a row only counts toward the cap once it has
        // passed EVERY filter (the late-FILTER safety property: a partial solution
        // never fires the early exit).
        // [FABLE-5] (sq-1ivw7) Install the snapshot-aware non-literal column set (indexing THIS
        // block's `result` layout — the BGP variables) so the id fast path fires on capped fused
        // BGP+FILTER shapes too. Drain-safe as in `eval_flat_conjunctive`.
        #[cfg(feature = "id-filter-fastpath")]
        let idfast_cols = {
            let bgp = GraphPattern::Bgp { patterns: patterns.to_vec() };
            nonliteral_filter_cols(graph, &bgp, &result)
        };
        for f in &residual {
            if result.rows.is_empty() {
                break;
            }
            #[cfg(feature = "id-filter-fastpath")]
            with_idfast_nonlit_cols(idfast_cols.clone(), || apply_filter(graph, local, &mut result, f))?;
            #[cfg(not(feature = "id-filter-fastpath"))]
            apply_filter(graph, local, &mut result, f)?;
        }
        let merged = match acc.take() {
            None => result,
            Some(a) => union_bindings(a, result),
        };
        budget::check(merged.rows.len())?;
        let satisfied = merged.rows.len() >= cap;
        acc = Some(merged);
        if satisfied {
            break;
        }
        start = end;
    }
    Ok(Some(acc.unwrap_or_else(|| Bindings::unsorted(collect_vars(patterns), vec![]))))
}

// [GPT-6 Astra] Reuse only the same requested order of one immutable prepared pattern.
// Retain the actual sorted_by metadata returned by the scan, including fallback orders.
fn capped_rhs(
    slot: &mut Option<(Option<usize>, Bindings)>,
    sort: Option<usize>,
    scan: impl FnOnce() -> Bindings,
) -> &Bindings {
    if slot
        .as_ref()
        .is_none_or(|(cached_sort, _)| *cached_sort != sort)
    {
        // [GPT-6 Astra] Release the stale relation before materializing its replacement.
        // No subsequent step can borrow the old requested order from this slot.
        *slot = None;
        *slot = Some((sort, scan()));
    }
    &slot.as_ref().unwrap().1
}

```

## context-tests.rs

```rust
// [GPT-6 Astra] Actual public-query path and physical RHS-work witness for #3105.
#[cfg(test)]
mod capped_rhs_tests {
    use super::*;
    use std::cell::{Cell, RefCell};

    thread_local! {
        static WORK: Cell<[usize; 4]> = const { Cell::new([0; 4]) };
        static STEPS: RefCell<Option<Vec<Step>>> = const { RefCell::new(None) };
    }

    // [GPT-6 Astra] Observe the actual branch, scan closure and returned metadata.
    #[derive(Debug, PartialEq, Eq)]
    struct Step {
        start: usize,
        pattern: usize,
        kernel: &'static str,
        requested: Option<usize>,
        scanned: bool,
        actual: Option<String>,
    }

    pub(super) fn step(
        start: usize,
        pattern: usize,
        kernel: &'static str,
        requested: Option<usize>,
        scanned: bool,
        actual: Option<&Variable>,
    ) {
        STEPS.with_borrow_mut(|steps| {
            if let Some(steps) = steps {
                steps.push(Step {
                    start,
                    pattern,
                    kernel,
                    requested,
                    scanned,
                    actual: actual.map(|v| v.as_str().to_owned()),
                });
            }
        });
    }

    fn trace<T>(f: impl FnOnce() -> T) -> (T, Vec<Step>) {
        STEPS.with_borrow_mut(|steps| *steps = Some(Vec::new()));
        let result = f();
        let steps = STEPS.with_borrow_mut(|steps| steps.take().unwrap());
        (result, steps)
    }

    fn bag(result: &crate::QueryResult) -> std::collections::BTreeMap<Vec<String>, usize> {
        let mut bag = std::collections::BTreeMap::new();
        for row in &result.rows {
            *bag.entry(
                row.iter()
                    .map(|v| v.as_ref().unwrap().to_string())
                    .collect(),
            )
            .or_default() += 1;
        }
        bag
    }

    #[test]
    fn capped_rhs_changing_sort_mixed_kernels_preserves_full_bag() {
        let mut ttl = String::from("@prefix : <http://ex/> .\n");
        for i in 0..70_000 {
            // Projection deliberately collapses pairs, making multiplicity observable.
            ttl.push_str(&format!(":s{i} :p {} ; :q {i} ; :r {i} .\n", i / 2));
        }
        ttl.push_str(":extra1 :q 70001 ; :r 70001 . :extra2 :r 70002 .");
        let graph = Graph::load_str(&ttl, "turtle").unwrap();
        let query = "PREFIX : <http://ex/> SELECT ?o WHERE { ?s :p ?o . ?s :q ?x . ?s :r ?x . FILTER(?o + 0 >= 0) }";
        let full = crate::query(&graph, query).unwrap();
        let (limited, steps) =
            trace(|| crate::query(&graph, &format!("{query} LIMIT 70001")).unwrap());
        println!("changing-sort actual steps: {steps:?}");
        assert_eq!(limited.rows.len(), 70_000);
        assert_eq!(bag(&limited), bag(&full));
        let expected: std::collections::BTreeMap<_, _> = (0..35_000)
            .map(|i| (vec![oxrdf::Literal::from(i).to_string()], 2))
            .collect();
        assert_eq!(bag(&limited), expected);
        let q: Vec<_> = steps
            .iter()
            .filter(|s| s.pattern == 1)
            .map(|s| {
                (
                    s.start,
                    s.kernel,
                    s.requested,
                    s.scanned,
                    s.actual.as_deref(),
                )
            })
            .collect();
        let r: Vec<_> = steps
            .iter()
            .filter(|s| s.pattern == 2)
            .map(|s| (s.start, s.requested, s.scanned, s.actual.as_deref()))
            .collect();
        if sparq_core::store::BUILT.contains(&sparq_core::store::Perm::Pso) {
            assert_eq!(
                q,
                [
                    (0, "bind", None, false, None),
                    (1024, "merge", Some(0), true, Some("s")),
                    (65536, "bind", None, false, None)
                ]
            );
            assert_eq!(
                r,
                [
                    (0, None, true, Some("s")),
                    (1024, Some(0), true, Some("s")),
                    (65536, None, true, Some("s"))
                ]
            );
        } else {
            // Three permutations return object order: no false subject-order claim,
            // and the unchanged None request must reuse the actually unsorted-for-s RHS.
            assert_eq!(
                q,
                [
                    (0, "bind", None, false, None),
                    (1024, "hash", None, true, Some("x")),
                    (65536, "bind", None, false, None)
                ]
            );
            assert_eq!(
                r,
                [
                    (0, None, true, Some("x")),
                    (1024, None, false, Some("x")),
                    (65536, None, false, Some("x"))
                ]
            );
        }
    }

    #[test]
    fn capped_rhs_replacement_releases_old_slot_before_scan() {
        let mut slot = Some((
            None,
            Bindings::unsorted(
                vec![Variable::new("x").unwrap()],
                vec![Row::from_slice(&[1])],
            ),
        ));
        // A failed replacement leaves the actual slot empty only if old ownership
        // was released before invoking the scan. This is not a timing/heap estimate.
        let failed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            capped_rhs(&mut slot, Some(0), || panic!("replacement scan sentinel"));
        }));
        assert!(failed.is_err());
        assert!(
            slot.is_none(),
            "old RHS remained live through replacement scan"
        );
        let replacement = capped_rhs(&mut slot, Some(0), || {
            Bindings::unsorted(
                vec![Variable::new("x").unwrap()],
                vec![Row::from_slice(&[2])],
            )
        });
        assert_eq!(replacement.rows, [Row::from_slice(&[2])]);
    }

    #[test]
    fn capped_rhs_disconnected_cross_preserves_multiplicity() {
        let graph = Graph::load_str(
            "@prefix : <http://ex/> . :a :p 1 . :b :p 2 . :c :q 3, 4, 5 .",
            "turtle",
        )
        .unwrap();
        let query =
            "PREFIX : <http://ex/> SELECT ?s WHERE { ?s :p ?o . ?t :q ?x . FILTER(?o + 0 >= 0) }";
        let (limited, steps) = trace(|| crate::query(&graph, &format!("{query} LIMIT 7")).unwrap());
        assert_eq!(bag(&limited), bag(&crate::query(&graph, query).unwrap()));
        assert_eq!(limited.rows.len(), 6);
        assert_eq!(bag(&limited).values().copied().collect::<Vec<_>>(), [3, 3]);
        assert_eq!(steps.len(), 1);
        assert_eq!(steps[0].kernel, "cross");
        assert!(steps[0].scanned);
        println!("disconnected actual steps: {steps:?}");
    }

    #[test]
    fn capped_rhs_named_view_overlay_and_residual_exists() {
        let mut graph = Graph::load_dataset("@prefix : <http://ex/> . :g { :a :p 1, 2 ; :q 1, 2 ; :visible true . :b :p 3 ; :q 3 . }", "trig").unwrap();
        let query = "PREFIX : <http://ex/> SELECT ?s WHERE { GRAPH :g { SELECT ?s WHERE { ?s :p ?o . ?s :q ?o . FILTER(?o + 0 >= 0 && EXISTS { ?s :visible true }) } LIMIT 10 } }";
        let (base, steps) = trace(|| crate::query(&graph, query).unwrap());
        assert_eq!(base.rows.len(), 2);
        assert_eq!(base.rows[0], base.rows[1]);
        // EXISTS is deliberately non-conjunctive: prove the existing fallback remains.
        assert!(
            steps.is_empty(),
            "EXISTS must retain the scope-safe fallback"
        );
        let eligible = query.replace(" && EXISTS { ?s :visible true }", "");
        let (eligible_base, eligible_steps) = trace(|| crate::query(&graph, &eligible).unwrap());
        assert_eq!(eligible_base.rows.len(), 3);
        assert!(
            eligible_steps.iter().any(|s| s.scanned),
            "named subquery must reach capped RHS: {eligible_steps:?}"
        );
        let visible = crate::DatasetView {
            base: &graph,
            named: std::sync::Arc::new([graph.named[0].0.clone()].into_iter().collect()),
            default: crate::DefaultGraphMode::Empty,
        };
        assert_eq!(
            bag(&crate::query_view(&visible, query).unwrap()),
            bag(&base)
        );
        let (visible_rows, visible_steps) =
            trace(|| crate::query_view(&visible, &eligible).unwrap());
        assert_eq!(bag(&visible_rows), bag(&eligible_base));
        assert!(visible_steps.iter().any(|s| s.scanned));
        let hidden = crate::DatasetView {
            base: &graph,
            named: std::sync::Arc::new(Default::default()),
            default: crate::DefaultGraphMode::StoreDefault,
        };
        assert!(crate::query_view(&hidden, query).unwrap().rows.is_empty());
        let (hidden_rows, hidden_steps) = trace(|| crate::query_view(&hidden, &eligible).unwrap());
        assert!(hidden_rows.rows.is_empty());
        assert!(
            hidden_steps.is_empty(),
            "hidden graph must not enter RHS cache"
        );
        let mut fork = graph.named[0].1.fork();
        fork.apply_delta(
            &[],
            &[[
                oxrdf::NamedNode::new("http://ex/a").unwrap().into(),
                oxrdf::NamedNode::new("http://ex/q").unwrap().into(),
                oxrdf::Literal::from(2).into(),
            ]],
        )
        .unwrap();
        graph.named[0].1 = fork;
        let (changed, changed_steps) = trace(|| crate::query(&graph, query).unwrap());
        assert_eq!(changed.rows.len(), 1);
        assert_eq!(changed.rows[0], base.rows[0]);
        assert!(changed_steps.is_empty(), "overlay EXISTS retains fallback");
        let (eligible_overlay, overlay_steps) = trace(|| crate::query(&graph, &eligible).unwrap());
        assert_eq!(eligible_overlay.rows.len(), 2);
        assert!(overlay_steps.iter().any(|s| s.scanned));
        println!("named EXISTS fallback: {steps:?}, {changed_steps:?}; eligible base/overlay: {eligible_steps:?}, {overlay_steps:?}");
    }
    pub(super) fn observe(index: usize) {
        WORK.with(|cell| {
            let mut counts = cell.get();
            counts[index] += 1;
            cell.set(counts);
        });
    }

    fn take_work() -> [usize; 4] {
        WORK.with(|cell| cell.replace([0; 4]))
    }

    #[test]
    fn capped_rhs_three_block_miss() {
        let mut ttl = String::from("@prefix : <http://ex/> .\n");
        // More than the second block boundary; two shared variables prohibit bind join.
        for i in 0..70_000 {
            ttl.push_str(&format!(":s{i} :p {i} ; :q {} .\n", i + 1));
        }
        let graph = Graph::load_str(&ttl, "turtle").unwrap();
        // Arithmetic keeps this filter residual and defeats the count shortcut.
        let body = "?s :p ?o . ?s :q ?o . FILTER(?o + 0 >= 0)";
        take_work();
        assert!(!crate::ask(&graph, &format!("PREFIX : <http://ex/> ASK {{ {body} }}")).unwrap());
        let ask_work = take_work();
        assert_eq!(ask_work, [1, 3, 1, 0]);
        assert!(
            crate::query(
                &graph,
                &format!("PREFIX : <http://ex/> SELECT * WHERE {{ {body} }} LIMIT 1")
            )
            .unwrap()
            .rows
            .is_empty()
        );
        let limit_work = take_work();
        assert_eq!(limit_work, [1, 3, 1, 0]);
        println!(
            "ASK and LIMIT miss: entries/blocks/RHS scans/bind calls = {ask_work:?}, {limit_work:?}"
        );
        // The budget permits the old scan. Its presence must disable retention,
        // independently of whether it actually trips on this query.
        let generous = crate::QueryBudget {
            max_rows: Some(100_000),
            ..Default::default()
        };
        assert!(
            !crate::ask_with_budget(
                &graph,
                &format!("PREFIX : <http://ex/> ASK {{ {body} }}"),
                &generous
            )
            .unwrap()
        );
        assert_eq!(
            take_work(),
            [1, 3, 3, 0],
            "armed budget must retain original scan behavior"
        );
        let row_limited = crate::QueryBudget {
            max_rows: Some(10),
            ..Default::default()
        };
        // A nonempty intermediate, unlike the all-miss witness, exercises the row ceiling.
        let expansion =
            "PREFIX : <http://ex/> ASK { ?s :p ?o . ?s :q ?other . FILTER(?o + 0 >= 0) }";
        assert!(
            crate::ask_with_budget(&graph, expansion, &row_limited)
                .unwrap_err()
                .contains("max-rows")
        );
        take_work();
        let cancelled = crate::QueryBudget {
            cancel: Some(std::sync::Arc::new(std::sync::atomic::AtomicBool::new(
                true,
            ))),
            ..Default::default()
        };
        assert!(
            crate::ask_with_budget(
                &graph,
                &format!("PREFIX : <http://ex/> ASK {{ {body} }}"),
                &cancelled
            )
            .unwrap_err()
            .contains("cancelled")
        );
        take_work();
        #[cfg(not(target_arch = "wasm32"))]
        {
            let expired = crate::QueryBudget {
                deadline: Some(std::time::Instant::now()),
                ..Default::default()
            };
            assert!(
                crate::ask_with_budget(
                    &graph,
                    &format!("PREFIX : <http://ex/> ASK {{ {body} }}"),
                    &expired
                )
                .is_err()
            );
            take_work();
        }
    }
    #[test]
    fn capped_rhs_keeps_rows_and_actual_order_until_request_changes() {
        let mut slot = None;
        let variable = Variable::new("x").unwrap();
        let first = capped_rhs(&mut slot, Some(0), || Bindings {
            vars: vec![variable.clone()],
            rows: vec![Row::from_slice(&[1]), Row::from_slice(&[1])],
            // Requested and actual order need not agree on restricted permutations.
            sorted_by: None,
        });
        let pointer = first.rows.as_ptr();
        assert_eq!(first.rows.len(), 2);
        assert_eq!(first.sorted_by, None);
        let reused = capped_rhs(&mut slot, Some(0), || panic!("identical scan was repeated"));
        assert_eq!(
            reused.rows.as_ptr(),
            pointer,
            "reuse must not deep-clone rows"
        );
        assert_eq!(
            reused.rows[0], reused.rows[1],
            "bag multiplicity is retained"
        );
        let changed = capped_rhs(&mut slot, Some(2), || Bindings {
            vars: vec![variable.clone()],
            rows: vec![Row::from_slice(&[2])],
            sorted_by: Some(variable.clone()),
        });
        assert_eq!(changed.rows, vec![Row::from_slice(&[2])]);
        assert_eq!(changed.sorted_by, Some(variable));
    }

    #[test]
    fn capped_rhs_public_bags_repeated_variables_and_first_block_hit() {
        let graph = Graph::load_str(
            "@prefix : <http://ex/> . :a :p 1, 2 ; :q 1, 2 . :b :p :b ; :q :b . :c :p :b .",
            "turtle",
        )
        .unwrap();
        let body = "?s :p ?o . ?s :q ?o . FILTER(?o + 0 >= 0)";
        let full = crate::query(
            &graph,
            &format!("PREFIX : <http://ex/> SELECT ?s WHERE {{ {body} }}"),
        )
        .unwrap();
        take_work();
        let limited = crate::query(
            &graph,
            &format!("PREFIX : <http://ex/> SELECT ?s WHERE {{ {body} }} LIMIT 10"),
        )
        .unwrap();
        assert_eq!(take_work(), [1, 1, 1, 0]);
        assert_eq!(limited.rows, full.rows);
        assert_eq!(limited.rows.len(), 2);
        assert_eq!(
            limited.rows[0], limited.rows[1],
            "projection preserves multiplicity"
        );
        take_work();
        assert!(crate::ask(&graph, &format!("PREFIX : <http://ex/> ASK {{ {body} }}")).unwrap());
        assert_eq!(take_work(), [1, 1, 1, 0]);
        let repeated = "?s :p ?s . ?s :q ?o . FILTER(?s != <http://ex/missing>)";
        let result = crate::query(
            &graph,
            &format!("PREFIX : <http://ex/> SELECT ?s WHERE {{ {repeated} }} LIMIT 10"),
        )
        .unwrap();
        assert_eq!(
            result.rows.len(),
            1,
            "off-diagonal repeated-variable rows must not survive"
        );
    }
}
```

## context-built.rs

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
```

## context-features.toml

```rust
# Forwards sparq-core's parallel index build. Default on for native; the wasm
# crate disables defaults so rayon is never pulled into the bundle.
# `regex` powers SPARQL REGEX/REPLACE; default-on for native, off for wasm (the wasm crate
# disables defaults) so the regex automata don't bloat the browser bundle.
# `digest` powers the SPARQL hash builtins (MD5/SHA1/SHA256/SHA384/SHA512); default-on
# for native, off for wasm (the wasm crate disables defaults) so the hash cores never
# enter the browser bundle.
default = ["parallel", "regex", "digest"]
parallel = ["dep:rayon", "sparq-core/parallel"]
regex = ["dep:regex"]
digest = ["dep:md-5", "dep:sha1", "dep:sha2"]
# Characteristic-set star-join cardinality estimation (Neumann & Moerkotte):
# `cs::CsTable` + `with_cs_table` make the greedy planner consult an injected CS
[dev-dependencies]
# The dict-consolidation differential test compares the serial, sharded-parallel and
# external (out-of-core) build paths — the tests need sparq-core's mmap feature even
# though the library itself doesn't. `dict-spill` adds the spilled-dictionary build's
# byte-identity differential (tests/dict_spill_differential.rs).
sparq-core = { path = "../sparq-core", version = "0.1.1", features = ["mmap", "parallel", "dict-spill"] }
rayon.workspace = true
# [OPUS-4.8] (sq-7d3dj.30.1) DEV-only re-listing of the ALREADY-present `spargebra` dep so
# tests/rewrite_pass.rs can parse a query to RAW `spargebra` algebra (the un-rewritten
# baseline) and feed it through `PreparedQuery::from` — the on-vs-off oracle for the
# `algebra-rewrite` pass. `spargebra` is already a normal dependency (above), so this adds
```

## Existing conjunctive/EXISTS scope boundary

```rust
pub(crate) fn is_conjunctive(p: &GraphPattern) -> bool {
    match p {
        GraphPattern::Bgp { .. } => true,
        // A FILTER may only be flattened into the enclosing conjunction when every
        // variable it mentions is bound INSIDE its own group — otherwise hoisting it
        // changes scope (`{ :x :p ?v . { FILTER(?v = 1) } }` must see ?v UNBOUND).
        // EXISTS is conservatively never flattened (it evaluates against the group's
        // in-scope bindings).
        GraphPattern::Filter { inner, expr } => {
            if !is_conjunctive(inner) {
                return false;
            }
            let mut inner_vars: FxHashSet<Variable> = FxHashSet::default();
            collect_pattern_vars(inner, &mut inner_vars);
            filter_scope_ok(expr, &inner_vars)
        }
        GraphPattern::Join { left, right } => is_conjunctive(left) && is_conjunctive(right),
        _ => false,
    }
}

/// All variables bound by the triple patterns of a conjunctive subtree.
fn collect_pattern_vars(p: &GraphPattern, out: &mut FxHashSet<Variable>) {
    match p {
        GraphPattern::Bgp { patterns } => {
            for tp in patterns {
                for v in [tp_var(&tp.subject), nnp_var(&tp.predicate), tp_var(&tp.object)].into_iter().flatten() {
                    out.insert(v);
                }
            }
        }
        GraphPattern::Filter { inner, .. } => collect_pattern_vars(inner, out),
        GraphPattern::Join { left, right } => {
            collect_pattern_vars(left, out);
            collect_pattern_vars(right, out);
        }
        _ => {}
    }
}

/// `true` if a filter expression's variables are all in `bound` (and it has no
/// EXISTS), so applying it at the top of the flattened conjunction is equivalent.
fn filter_scope_ok(e: &Expression, bound: &FxHashSet<Variable>) -> bool {
    use Expression::*;
    match e {
        NamedNode(_) | Literal(_) => true,
        Variable(v) | Bound(v) => bound.contains(v),
        UnaryPlus(a) | UnaryMinus(a) | Not(a) => filter_scope_ok(a, bound),
        And(a, b) | Or(a, b) | Equal(a, b) | SameTerm(a, b) | Greater(a, b) | GreaterOrEqual(a, b) | Less(a, b)
        | LessOrEqual(a, b) | Add(a, b) | Subtract(a, b) | Multiply(a, b) | Divide(a, b) => {
            filter_scope_ok(a, bound) && filter_scope_ok(b, bound)
        }
        In(a, list) => filter_scope_ok(a, bound) && list.iter().all(|c| filter_scope_ok(c, bound)),
        If(c, t, f) => filter_scope_ok(c, bound) && filter_scope_ok(t, bound) && filter_scope_ok(f, bound),
        Coalesce(es) => es.iter().all(|c| filter_scope_ok(c, bound)),
        FunctionCall(_, args) => args.iter().all(|c| filter_scope_ok(c, bound)),
        Exists(_) => false,
    }
}

pub(crate) fn flatten_conjunction(p: &GraphPattern, patterns: &mut Vec<TriplePattern>, filters: &mut Vec<Expression>) {
    match p {
        GraphPattern::Bgp { patterns: tps } => patterns.extend(tps.iter().cloned()),
        GraphPattern::Join { left, right } => {
            flatten_conjunction(left, patterns, filters);
            flatten_conjunction(right, patterns, filters);
        }
        GraphPattern::Filter { expr, inner } => {
            flatten_conjunction(inner, patterns, filters);
            filters.push(expr.clone());
        }
        _ => unreachable!(),
    }
}

// ---- Sargable numeric filters (pushed into the scan) -------------------------

/// A numeric comparison `value OP threshold` that can be pushed down into a
/// pattern scan (FILTER predicate evaluated inline, in the column's sorted order,
/// so the numeric access is sequential rather than a random dictionary gather —
/// the layout fix the hardware research measured as an 8–15× win).
#[derive(Clone, Copy)]
pub(crate) enum NumCmp {
    Gt(f64),
    Ge(f64),
    Lt(f64),
    Le(f64),
    Eq(f64),
}

impl NumCmp {
    /// Human-readable comparison for EXPLAIN output, e.g. `> 28`.
    pub(crate) fn render(&self) -> String {
        match *self {
            NumCmp::Gt(t) => format!("> {t}"),
            NumCmp::Ge(t) => format!(">= {t}"),
            NumCmp::Lt(t) => format!("< {t}"),
            NumCmp::Le(t) => format!("<= {t}"),
            NumCmp::Eq(t) => format!("= {t}"),
        }
    }

    #[inline]
    fn test(&self, x: f64) -> bool {
        match *self {
            NumCmp::Gt(t) => x > t,
            NumCmp::Ge(t) => x >= t,
            NumCmp::Lt(t) => x < t,
            NumCmp::Le(t) => x <= t,
            NumCmp::Eq(t) => x == t,
        }
    }
}

/// A comparison operator, for the temporal pushed-down predicate.
#[derive(Clone, Copy)]
pub(crate) enum CmpOp {
    Gt,
    Ge,
    Lt,
    Le,
    Eq,
}

impl CmpOp {
    #[inline]
    fn eval(self, o: Ordering) -> bool {
        match self {
            CmpOp::Gt => o == Ordering::Greater,
            CmpOp::Ge => o != Ordering::Less,
            CmpOp::Lt => o == Ordering::Less,
            CmpOp::Le => o != Ordering::Greater,
            CmpOp::Eq => o == Ordering::Equal,
        }
    }

    fn render(self) -> &'static str {
        match self {
            CmpOp::Gt => ">",
            CmpOp::Ge => ">=",
            CmpOp::Lt => "<",
            CmpOp::Le => "<=",
            CmpOp::Eq => "=",
        }
    }
}

/// A sargable FILTER predicate pushed down into a pattern scan: numeric (via the f64
/// `numerics` cache) or temporal (via the `temporals` cache — dateTime/date vs a
/// temporal constant).
#[derive(Clone, Copy)]
pub(crate) enum ScanCmp {
    Num(NumCmp),
    /// `value OP temporal-constant`. A row passes when the comparison is DECIDABLE and
    /// satisfies the operator; an indeterminate (mixed-timezone window), cross-family
    /// (dateTime vs date) or non-temporal operand is a FILTER type error — the row is
    /// excluded, which `false` reproduces exactly. (For `=`, cross-family is "known
    /// different" rather than an error — also excluded, also `false`.)
    Temp(CmpOp, Temporal),
}

impl ScanCmp {
    /// Human-readable comparison for EXPLAIN output, e.g. `> 28`.
    pub(crate) fn render(&self) -> String {
        match *self {
            ScanCmp::Num(c) => c.render(),
            ScanCmp::Temp(op, t) => format!("{} temporal(instant {})", op.render(), t.instant),
        }
    }

    /// Evaluates the pushed-down predicate against one scanned column id, through the
    /// graph's numeric / temporal value cache — O(1), no term materialised.
    #[inline]
    fn test_id(&self, graph: &Graph, id: Id) -> bool {
        match *self {
            ScanCmp::Num(c) => graph.numeric_value(id).is_some_and(|x| c.test(x)),
            ScanCmp::Temp(op, t) => {
                graph.temporal_value(id).and_then(|v| Temporal::cmp_t(v, t)).is_some_and(|o| op.eval(o))
            }
        }
    }
}

/// The inclusive range of inline-integer *values* `[lo, hi]` (within `[0, INLINE_MAX]`)
/// that satisfy the comparison, or `None` if no integer can. Used to range-prune a
/// scan whose filter column holds inline integers (which sort by value). A TEMPORAL
/// predicate over an all-inline (integer) column is a type error on every row —
/// `None`, the empty range.
fn inline_pass_values(cmp: ScanCmp) -> Option<(u32, u32)> {
    let cmp = match cmp {
        ScanCmp::Num(c) => c,
        ScanCmp::Temp(..) => return None,
    };
    let max = (dict::INLINE_BASE - 1) as i64;
    let (lo, hi): (i64, i64) = match cmp {
        NumCmp::Gt(t) => (t.floor() as i64 + 1, max),
        NumCmp::Ge(t) => (t.ceil() as i64, max),
        NumCmp::Lt(t) => (0, t.ceil() as i64 - 1),
        NumCmp::Le(t) => (0, t.floor() as i64),
        NumCmp::Eq(t) => {
            if t.fract() != 0.0 || t < 0.0 || t > max as f64 {
                return None;
            }
            let v = t as i64;
            (v, v)
        }
    };
    let (lo, hi) = (lo.max(0), hi.min(max));
    (lo <= hi).then_some((lo as u32, hi as u32))
}

/// Recognises a FILTER of the form `?v OP constant` (or the symmetric
/// `constant OP ?v`) over a numeric or temporal constant, returning the variable
/// and the comparison to push down.
///
/// [OPUS-4.8] (sq-lr2ii) A NUMERIC comparison is DECLINED (returns `None`) when the graph
/// holds an f64-inexact decimal ([`Graph::has_high_precision_decimal`]): the f64 `numerics`
/// cache the scan probes could then decide `=`/`<`/`>`/`<=`/`>=` wrongly for that value (e.g.
/// `"1.000000000000000001"^^xsd:decimal` collapses onto the f64 `1.0`), so such comparisons
/// fall back to the exact general evaluator instead. Temporal pushdown is unaffected, and a
/// graph with no f64-inexact decimal keeps the numeric fast path.
```

## Executed restored-default

```text
   Compiling sparq-engine v0.1.1 (<task-root>/worktrees/issue3105/crates/sparq-engine)
    Finished `test` profile [unoptimized + debuginfo] target(s) in 7.51s
     Running unittests src/lib.rs (<task-root>/direct-5983/implementation/target/debug/deps/sparq_engine-a23c250f81b1b746)

running 7 tests
test exec::capped_rhs_tests::capped_rhs_changing_sort_mixed_kernels_preserves_full_bag ... changing-sort actual steps: [Step { start: 0, pattern: 1, kernel: "bind", requested: None, scanned: false, actual: None }, Step { start: 0, pattern: 2, kernel: "hash", requested: None, scanned: true, actual: Some("s") }, Step { start: 1024, pattern: 1, kernel: "merge", requested: Some(0), scanned: true, actual: Some("s") }, Step { start: 1024, pattern: 2, kernel: "merge", requested: Some(0), scanned: true, actual: Some("s") }, Step { start: 65536, pattern: 1, kernel: "bind", requested: None, scanned: false, actual: None }, Step { start: 65536, pattern: 2, kernel: "hash", requested: None, scanned: true, actual: Some("s") }]
ok
test exec::capped_rhs_tests::capped_rhs_disconnected_cross_preserves_multiplicity ... disconnected actual steps: [Step { start: 0, pattern: 1, kernel: "cross", requested: None, scanned: true, actual: Some("t") }]
ok
test exec::capped_rhs_tests::capped_rhs_keeps_rows_and_actual_order_until_request_changes ... ok
test exec::capped_rhs_tests::capped_rhs_named_view_overlay_and_residual_exists ... named EXISTS fallback: [], []; eligible base/overlay: [Step { start: 0, pattern: 1, kernel: "merge", requested: Some(0), scanned: true, actual: Some("s") }], [Step { start: 0, pattern: 0, kernel: "merge", requested: Some(0), scanned: true, actual: Some("s") }]
ok
test exec::capped_rhs_tests::capped_rhs_public_bags_repeated_variables_and_first_block_hit ... ok
test exec::capped_rhs_tests::capped_rhs_replacement_releases_old_slot_before_scan ... 
thread 'exec::capped_rhs_tests::capped_rhs_replacement_releases_old_slot_before_scan' (3638873) panicked at crates/sparq-engine/src/exec.rs:21401:47:
replacement scan sentinel
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace
ok
test exec::capped_rhs_tests::capped_rhs_three_block_miss ... ASK and LIMIT miss: entries/blocks/RHS scans/bind calls = [1, 3, 1, 0], [1, 3, 1, 0]
ok

test result: ok. 7 passed; 0 failed; 0 ignored; 0 measured; 318 filtered out; finished in 3.50s

```

## Executed pinned-compact

```text
   Compiling sparq-engine v0.1.1 (<task-root>/worktrees/issue3105/crates/sparq-engine)
    Finished `test` profile [unoptimized + debuginfo] target(s) in 6.06s
     Running unittests src/lib.rs (<task-root>/direct-5983/implementation/target/debug/deps/sparq_engine-8c5cead58cdf2c43)

running 7 tests
test exec::capped_rhs_tests::capped_rhs_changing_sort_mixed_kernels_preserves_full_bag ... changing-sort actual steps: [Step { start: 0, pattern: 1, kernel: "bind", requested: None, scanned: false, actual: None }, Step { start: 0, pattern: 2, kernel: "hash", requested: None, scanned: true, actual: Some("x") }, Step { start: 1024, pattern: 1, kernel: "hash", requested: None, scanned: true, actual: Some("x") }, Step { start: 1024, pattern: 2, kernel: "hash", requested: None, scanned: false, actual: Some("x") }, Step { start: 65536, pattern: 1, kernel: "bind", requested: None, scanned: false, actual: None }, Step { start: 65536, pattern: 2, kernel: "hash", requested: None, scanned: false, actual: Some("x") }]
ok
test exec::capped_rhs_tests::capped_rhs_disconnected_cross_preserves_multiplicity ... disconnected actual steps: [Step { start: 0, pattern: 1, kernel: "cross", requested: None, scanned: true, actual: Some("x") }]
ok
test exec::capped_rhs_tests::capped_rhs_keeps_rows_and_actual_order_until_request_changes ... ok
test exec::capped_rhs_tests::capped_rhs_named_view_overlay_and_residual_exists ... named EXISTS fallback: [], []; eligible base/overlay: [Step { start: 0, pattern: 1, kernel: "merge", requested: Some(2), scanned: true, actual: Some("o") }], [Step { start: 0, pattern: 0, kernel: "merge", requested: Some(2), scanned: true, actual: Some("o") }]
ok
test exec::capped_rhs_tests::capped_rhs_public_bags_repeated_variables_and_first_block_hit ... ok
test exec::capped_rhs_tests::capped_rhs_replacement_releases_old_slot_before_scan ... 
thread 'exec::capped_rhs_tests::capped_rhs_replacement_releases_old_slot_before_scan' (3636561) panicked at crates/sparq-engine/src/exec.rs:21401:47:
replacement scan sentinel
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace
ok
test exec::capped_rhs_tests::capped_rhs_three_block_miss ... ASK and LIMIT miss: entries/blocks/RHS scans/bind calls = [1, 3, 1, 0], [1, 3, 1, 0]
ok

test result: ok. 7 passed; 0 failed; 0 ignored; 0 measured; 318 filtered out; finished in 4.10s

```

## Executed pinned-no-default

```text
   Compiling sparq-engine v0.1.1 (<task-root>/worktrees/issue3105/crates/sparq-engine)
    Finished `test` profile [unoptimized + debuginfo] target(s) in 4.95s
     Running unittests src/lib.rs (<task-root>/direct-5983/implementation/target/debug/deps/sparq_engine-09aaa1ab56ae56d7)

running 7 tests
test exec::capped_rhs_tests::capped_rhs_changing_sort_mixed_kernels_preserves_full_bag ... changing-sort actual steps: [Step { start: 0, pattern: 1, kernel: "bind", requested: None, scanned: false, actual: None }, Step { start: 0, pattern: 2, kernel: "hash", requested: None, scanned: true, actual: Some("s") }, Step { start: 1024, pattern: 1, kernel: "merge", requested: Some(0), scanned: true, actual: Some("s") }, Step { start: 1024, pattern: 2, kernel: "merge", requested: Some(0), scanned: true, actual: Some("s") }, Step { start: 65536, pattern: 1, kernel: "bind", requested: None, scanned: false, actual: None }, Step { start: 65536, pattern: 2, kernel: "hash", requested: None, scanned: true, actual: Some("s") }]
ok
test exec::capped_rhs_tests::capped_rhs_disconnected_cross_preserves_multiplicity ... disconnected actual steps: [Step { start: 0, pattern: 1, kernel: "cross", requested: None, scanned: true, actual: Some("t") }]
ok
test exec::capped_rhs_tests::capped_rhs_keeps_rows_and_actual_order_until_request_changes ... ok
test exec::capped_rhs_tests::capped_rhs_named_view_overlay_and_residual_exists ... named EXISTS fallback: [], []; eligible base/overlay: [Step { start: 0, pattern: 1, kernel: "merge", requested: Some(0), scanned: true, actual: Some("s") }], [Step { start: 0, pattern: 0, kernel: "merge", requested: Some(0), scanned: true, actual: Some("s") }]
ok
test exec::capped_rhs_tests::capped_rhs_public_bags_repeated_variables_and_first_block_hit ... ok
test exec::capped_rhs_tests::capped_rhs_replacement_releases_old_slot_before_scan ... 
thread 'exec::capped_rhs_tests::capped_rhs_replacement_releases_old_slot_before_scan' (3636815) panicked at crates/sparq-engine/src/exec.rs:21401:47:
replacement scan sentinel
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace
ok
test exec::capped_rhs_tests::capped_rhs_three_block_miss ... ASK and LIMIT miss: entries/blocks/RHS scans/bind calls = [1, 3, 1, 0], [1, 3, 1, 0]
ok

test result: ok. 7 passed; 0 failed; 0 ignored; 0 measured; 304 filtered out; finished in 3.41s

```

## Executed pinned-semijoin

```text
   Compiling sparq-engine v0.1.1 (<task-root>/worktrees/issue3105/crates/sparq-engine)
    Finished `test` profile [unoptimized + debuginfo] target(s) in 5.82s
     Running unittests src/lib.rs (<task-root>/direct-5983/implementation/target/debug/deps/sparq_engine-da0fb761177b7167)

running 7 tests
test exec::capped_rhs_tests::capped_rhs_changing_sort_mixed_kernels_preserves_full_bag ... changing-sort actual steps: [Step { start: 0, pattern: 1, kernel: "bind", requested: None, scanned: false, actual: None }, Step { start: 0, pattern: 2, kernel: "hash", requested: None, scanned: true, actual: Some("s") }, Step { start: 1024, pattern: 1, kernel: "merge", requested: Some(0), scanned: true, actual: Some("s") }, Step { start: 1024, pattern: 2, kernel: "merge", requested: Some(0), scanned: true, actual: Some("s") }, Step { start: 65536, pattern: 1, kernel: "bind", requested: None, scanned: false, actual: None }, Step { start: 65536, pattern: 2, kernel: "hash", requested: None, scanned: true, actual: Some("s") }]
ok
test exec::capped_rhs_tests::capped_rhs_disconnected_cross_preserves_multiplicity ... disconnected actual steps: [Step { start: 0, pattern: 1, kernel: "cross", requested: None, scanned: true, actual: Some("t") }]
ok
test exec::capped_rhs_tests::capped_rhs_keeps_rows_and_actual_order_until_request_changes ... ok
test exec::capped_rhs_tests::capped_rhs_named_view_overlay_and_residual_exists ... named EXISTS fallback: [], []; eligible base/overlay: [Step { start: 0, pattern: 1, kernel: "merge", requested: Some(0), scanned: true, actual: Some("s") }], [Step { start: 0, pattern: 0, kernel: "merge", requested: Some(0), scanned: true, actual: Some("s") }]
ok
test exec::capped_rhs_tests::capped_rhs_public_bags_repeated_variables_and_first_block_hit ... ok
test exec::capped_rhs_tests::capped_rhs_replacement_releases_old_slot_before_scan ... 
thread 'exec::capped_rhs_tests::capped_rhs_replacement_releases_old_slot_before_scan' (3637752) panicked at crates/sparq-engine/src/exec.rs:21401:47:
replacement scan sentinel
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace
ok
test exec::capped_rhs_tests::capped_rhs_three_block_miss ... ASK and LIMIT miss: entries/blocks/RHS scans/bind calls = [1, 3, 1, 0], [1, 3, 1, 0]
ok

test result: ok. 7 passed; 0 failed; 0 ignored; 0 measured; 324 filtered out; finished in 3.52s

```

## Executed ignore-requested-sort

```text
   Compiling sparq-engine v0.1.1 (<task-root>/worktrees/issue3105/crates/sparq-engine)
    Finished `test` profile [unoptimized + debuginfo] target(s) in 8.22s
     Running unittests src/lib.rs (<task-root>/direct-5983/implementation/target/debug/deps/sparq_engine-a23c250f81b1b746)

running 1 test
test exec::capped_rhs_tests::capped_rhs_changing_sort_mixed_kernels_preserves_full_bag ... changing-sort actual steps: [Step { start: 0, pattern: 1, kernel: "bind", requested: None, scanned: false, actual: None }, Step { start: 0, pattern: 2, kernel: "hash", requested: None, scanned: true, actual: Some("s") }, Step { start: 1024, pattern: 1, kernel: "merge", requested: Some(0), scanned: true, actual: Some("s") }, Step { start: 1024, pattern: 2, kernel: "merge", requested: Some(0), scanned: false, actual: Some("s") }, Step { start: 65536, pattern: 1, kernel: "bind", requested: None, scanned: false, actual: None }, Step { start: 65536, pattern: 2, kernel: "hash", requested: None, scanned: false, actual: Some("s") }]

thread 'exec::capped_rhs_tests::capped_rhs_changing_sort_mixed_kernels_preserves_full_bag' (3637326) panicked at crates/sparq-engine/src/exec.rs:21359:13:
assertion `left == right` failed
  left: [(0, None, true, Some("s")), (1024, Some(0), false, Some("s")), (65536, None, false, Some("s"))]
 right: [(0, None, true, Some("s")), (1024, Some(0), true, Some("s")), (65536, None, true, Some("s"))]
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace
FAILED

failures:

failures:
    exec::capped_rhs_tests::capped_rhs_changing_sort_mixed_kernels_preserves_full_bag

test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 324 filtered out; finished in 2.31s

error: test failed, to rerun pass `-p sparq-engine --lib`
```

## Executed retain-old-through-scan

```text
   Compiling sparq-engine v0.1.1 (<task-root>/worktrees/issue3105/crates/sparq-engine)
    Finished `test` profile [unoptimized + debuginfo] target(s) in 6.19s
     Running unittests src/lib.rs (<task-root>/direct-5983/implementation/target/debug/deps/sparq_engine-a23c250f81b1b746)

running 1 test
test exec::capped_rhs_tests::capped_rhs_replacement_releases_old_slot_before_scan ... 
thread 'exec::capped_rhs_tests::capped_rhs_replacement_releases_old_slot_before_scan' (3637506) panicked at crates/sparq-engine/src/exec.rs:21400:47:
replacement scan sentinel
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace

thread 'exec::capped_rhs_tests::capped_rhs_replacement_releases_old_slot_before_scan' (3637506) panicked at crates/sparq-engine/src/exec.rs:21403:9:
old RHS remained live through replacement scan
FAILED

failures:

failures:
    exec::capped_rhs_tests::capped_rhs_replacement_releases_old_slot_before_scan

test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 324 filtered out; finished in 0.00s

error: test failed, to rerun pass `-p sparq-engine --lib`
```

## Executed ask-early-exit

```text
   Compiling sparq-engine v0.1.1 (<task-root>/worktrees/issue3105/crates/sparq-engine)
    Finished `test` profile [unoptimized + debuginfo] target(s) in 5.29s
     Running tests/ask_early_exit.rs (<task-root>/direct-5983/implementation/target/debug/deps/ask_early_exit-8d23c424a01cff6f)

running 22 tests
test ask_aggregation_and_having ... ok
test ask_bind_over_capped_inner ... ok
test ask_distinct_under_offset_is_not_stripped ... ok
test ask_fail_closed_shapes_match_oracle ... ok
test ask_filter_eliminates_everything ... ok
test ask_graph_pattern_matches_oracle ... ok
test ask_join_early_exit_fires_under_budget ... ok
test ask_join_with_union_child ... ok
test ask_late_filter_never_fires_on_partial_solutions ... ok
test ask_optional_caps_expensive_left ... ok
test ask_optional_true_and_false ... ok
test ask_order_by_is_emptiness_neutral ... ok
test ask_order_by_limit_offset_subselect ... ok
test ask_q12a_shape_matches_select_oracle ... ok
test ask_through_joins_true_and_false ... ok
test ask_union_branch_short_circuit ... ok
test ask_union_skips_expensive_right_branch ... ok
test limit_through_join_returns_a_subset_of_the_full_result ... ok
test limit_with_late_filter_returns_only_filtered_rows ... ok
test limit_zero_through_capped_shapes ... ok
test q12a_shape_timing_probe ... ignored, timing probe, not an assertion (run manually with --release --nocapture)
test union_early_exit_keeps_right_branch_header ... ok

test result: ok. 21 passed; 0 failed; 1 ignored; 0 measured; 0 filtered out; finished in 2.09s

```

## Executed pinned-clippy

```text
    Checking sparq-engine v0.1.1 (<task-root>/worktrees/issue3105/crates/sparq-engine)
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 4.92s
```

## Executed pinned-preflight

```text
preflight: ran  G1 new-crate-completeness, G2 public-api-to-skill, G6 new-config-to-docs, privacy-claims, guard-untested
preflight: skip no-perf-numbers (no matching path in the diff); readme-template (no matching path in the diff)

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

## Evidence context

Full change against main is in full.diff; complete final source is final-source.rs. Exact command argv/environment/reserve records are commands.json and individual receipts. Four executed candidate test binaries are frozen with hashes in binary-provenance.json; control binaries are identified by actual compiled hashes in controls.json, with complete mutated sources and diffs retained. Original compile/fixture failures and first control pair are preserved. Unchanged scan/filter/join/budget/view consumer context remains in the prior phase1 packet and admission/source-validity-context.txt; this focused packet omits duplicated unchanged storage/view bodies. Core dev feature unification is shown above. No external independent review was performed by this author.
