# Whole-change independent review request: PR5983 narrow indexed top-k recovery

Frozen head `9e8bdfc95c916b62550fb8c3142f804bbbfb2444` on `cf19a52c6880c496cefaca24046174902bc30958`. Actual implementation runtime: GPT-6 Astra xhigh. Historical Luke Dary / Claude Sonnet5 source is preserved and attributed; timing claims are unverified. This is a local candidate, not reviewed, measured, published or admitted.

Please assess correctness and resource admission of the entire two-file change, including stage-one repairs (issue6465), then this test extension. Treat implementation claims below as claims to check. Return blocking findings with exact source locations and concrete counterexamples; distinguish a known conformance/performance validation limit from an observed defect. Do not infer a performance gain from the name or historical comments.

The whole raw diff below contains the complete new indexed function and complete test file. The companion context carries full critical executable functions/callers from this exact Git head. Only standalone full-line comments are omitted from the context copy inside this packet to reduce repeated prose; `source-context.md` preserves the complete unabridged source excerpts with Git blobs and ranges. No statements are omitted. The budget excerpt explicitly omits unchanged test submodules. Deeper unchanged evaluator/type/parser/storage code is not a recursive source audit; report a specific missing dependency if a decision needs it.

## Evidence, contract and limitations

```json
{
  "task": "PR5983 bounded semantic/resource extension; issue6465 tracks stage-one confirmed findings",
  "status": "Clean local whole-change candidate for independent Opus5 xhigh review, not publication/admission or a measured performance claim",
  "model": "OpenAI GPT-6 Astra, actual xhigh runtime; no other model sessions or agents",
  "head": "9e8bdfc95c916b62550fb8c3142f804bbbfb2444",
  "base": "cf19a52c6880c496cefaca24046174902bc30958",
  "local_origin_main_observed": "cf19a52c6880c496cefaca24046174902bc30958",
  "stage1_head": "803bb795201782551d984221a48b32ed391f7d9c",
  "branch": "codex/perf-indexed-topk-recovery",
  "worktree": "/private/tmp/sparq-pr6049/.throughput-monitor/worktrees/pr5983-topk",
  "provenance": {
    "source_pr5983": "59bfc6c44b25a4e107d5d957ca3287ba0bc22c1a",
    "historical_mergebase": "fbbd310326f1f083c33a13deee19f95cb96c55f6",
    "historical_author": "Luke Dary <ldary@redhat.com>, historical Claude Sonnet5 coauthorship retained in import checkpoint d3ee2b1b6059008e185ef946357b6cf77c21341a",
    "astra": "Three-way mechanical import, stage-one tests/guards and this semantic extension honestly attributed; original PR timing claims remain unverified",
    "semantic_commits": [
      "eae19f2b5c571a3fed83f60ce63a96ef215348e1",
      "9e8bdfc95c916b62550fb8c3142f804bbbfb2444"
    ]
  },
  "scope": {
    "whole_change": "crates/sparq-engine/src/exec.rs                    | 434 +++++++++++++++++\n .../tests/topk_orderby_indexed_differential.rs     | 519 +++++++++++++++++++++\n 2 files changed, 953 insertions(+)",
    "files": [
      "crates/sparq-engine/src/exec.rs",
      "crates/sparq-engine/tests/topk_orderby_indexed_differential.rs"
    ],
    "semantic_delta": "Seven additional tests, permutation-aware engagement assertion, test-contract prose and a five-line tie-soundness clarification. No executable runtime change from stage1 (verified after excluding full-line comments). No new runtime guard added.",
    "outside_scope_untouched": [
      "PreparedGraphApplier",
      "transaction retry helpers",
      "dependency/version metadata",
      "public API",
      "main bind_join changes",
      "CI/gate/approval files",
      "other branches/shared checkout",
      "registry/release/EC2/PSS"
    ]
  },
  "semantic_evidence": [
    "Total-order DESC primary/secondary keys: eight OFFSET/LIMIT windows including zero, primary-group boundary, tail and beyond-end; compare full ORDER BY window.",
    "Exact full-key ties: five windows check valid member ids, distinct multiplicity, key value and permitted count; no assertion of unspecified stable tie order or particular surviving subset.",
    "Three negative/mixed/typed-lexical fixtures: exact full-order prefix plus actual fallback trace. Inline-guard deletion is killed by admission assertion on first fixture; do not claim that control proved wrong numeric results on all fixtures.",
    "StoreDefault and Empty default views; visible GRAPH subquery; hidden/absent GRAPH and FROM; plain API after scope release. Empty-default-guard deletion actually leaks one row where zero is required.",
    "Fork overlay tombstone plus new task insertion; immutable original snapshot; four k windows against independent Rust-sort oracle; compaction preserves answers.",
    "Armed false cancellation flag and future deadline require actual BGP fallback. Already-set cancellation and already-expired deadline error deterministically; later unlimited query succeeds. No mid-query cancellation-latency guarantee is claimed.",
    "Restricted compact-index executes all26 tests, explicitly asserts fallback when this star fixture lacks PSO; default/no-default feature builds positively assert indexed engagement."
  ],
  "validation": {
    "focused": {
      "default": {
        "passed": 26,
        "failed": 0,
        "ignored": 0,
        "log": "default.txt"
      },
      "no-default": {
        "passed": 26,
        "failed": 0,
        "ignored": 0,
        "log": "no-default.txt"
      },
      "compact-index": {
        "passed": 26,
        "failed": 0,
        "ignored": 0,
        "log": "compact-index.txt"
      },
      "final-default": {
        "passed": 26,
        "failed": 0,
        "ignored": 0,
        "log": "final-default.txt"
      }
    },
    "existing": [
      {
        "name": "dataset-view",
        "command": [
          "/Users/jesght/.cargo/bin/cargo",
          "test",
          "--locked",
          "--offline",
          "-p",
          "sparq-engine",
          "--test",
          "query_features",
          "dataset_view::",
          "--",
          "--test-threads=1"
        ],
        "exit_code": 0,
        "results": [
          "test result: ok. 7 passed; 0 failed; 1 ignored; 0 measured; 32 filtered out; finished in 1.52s"
        ]
      },
      {
        "name": "fork-generations",
        "command": [
          "/Users/jesght/.cargo/bin/cargo",
          "test",
          "--locked",
          "--offline",
          "-p",
          "sparq-engine",
          "--test",
          "storage",
          "fork_generations::",
          "--",
          "--test-threads=1"
        ],
        "exit_code": 0,
        "results": [
          "test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 17 filtered out; finished in 0.03s"
        ]
      },
      {
        "name": "existing-orderby",
        "command": [
          "/Users/jesght/.cargo/bin/cargo",
          "test",
          "--locked",
          "--offline",
          "-p",
          "sparq-engine",
          "--test",
          "topk_orderby",
          "--test",
          "orderby_limit_zero",
          "--",
          "--test-threads=1"
        ],
        "exit_code": 0,
        "results": [
          "test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s",
          "test result: ok. 14 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.08s"
        ]
      }
    ],
    "unique_passed_tests": 51,
    "existing_ignored": "One pre-existing dataset-view microbenchmark bench_view_vs_from_named_copy is ignored; not counted as passed.",
    "baseline_executions": "26 default +26 no-default +26 compact-index;25 additional existing tests. Final comment-only head reruns26 default. Mutation baselines and mutant executions are reported separately.",
    "mutations": {
      "baseline": {
        "name": "baseline",
        "exit_code": 0,
        "tests": 26,
        "passed": 26,
        "failed": 0,
        "classification": "PASS",
        "source_sha256": "3e18e292af378a402a7bbc8e72e7ca5b99cc46bbce6f94ef48310e9ea8773b0e"
      },
      "mutants": [
        {
          "name": "repeated_deleted",
          "exit_code": 101,
          "tests": 26,
          "passed": 25,
          "failed": 1,
          "classification": "KILLED",
          "source_sha256": "79bcd27265918886fe68da45e93811a6c46d22eed6c68ab082f86c7a9873d978"
        },
        {
          "name": "repeated_inert",
          "exit_code": 101,
          "tests": 26,
          "passed": 25,
          "failed": 1,
          "classification": "KILLED",
          "source_sha256": "dfddb3a280a3a5fc8ae9c7d551efca4fca9ba7000f06156b648cc3eaaa2da36a"
        },
        {
          "name": "budget_deleted",
          "exit_code": 101,
          "tests": 26,
          "passed": 23,
          "failed": 3,
          "classification": "KILLED",
          "source_sha256": "30e04f7b2cab042da2210f251e6a781c6efddb4803739356f08eefe391a4763a"
        },
        {
          "name": "budget_inert",
          "exit_code": 101,
          "tests": 26,
          "passed": 23,
          "failed": 3,
          "classification": "KILLED",
          "source_sha256": "78d3b54123f93bc7a81df0a41b9bfa77ca194708c564d23a7a9682a50718c14a"
        },
        {
          "name": "fanout_guard_deleted",
          "exit_code": 101,
          "tests": 26,
          "passed": 25,
          "failed": 1,
          "classification": "KILLED",
          "source_sha256": "5bae6730db6f6c0c26285cf2ceab1a75e3e5ce5bd71f7eda281dd2bf701b513d"
        },
        {
          "name": "indexed_path_disabled",
          "exit_code": 101,
          "tests": 26,
          "passed": 25,
          "failed": 1,
          "classification": "KILLED",
          "source_sha256": "d51291bfe0b66f77f609f85e7a87ca3ed19d38590610708b4d07ed3884c7d27e"
        },
        {
          "name": "inline_guard_deleted",
          "exit_code": 101,
          "tests": 26,
          "passed": 25,
          "failed": 1,
          "classification": "KILLED",
          "source_sha256": "406bfb9f9a6acb1b7ea665a99c3cd4a92c30080f479f0a03bb3e0edcf255a5d7"
        },
        {
          "name": "empty_default_guard_deleted",
          "exit_code": 101,
          "tests": 26,
          "passed": 25,
          "failed": 1,
          "classification": "KILLED",
          "source_sha256": "f17369b7768306772c384e65d91b26ad0f52f8b0953879fb1c41e40d2d154a3b"
        }
      ],
      "restored": {
        "name": "restored",
        "exit_code": 0,
        "tests": 26,
        "passed": 26,
        "failed": 0,
        "classification": "PASS",
        "source_sha256": "3e18e292af378a402a7bbc8e72e7ca5b99cc46bbce6f94ef48310e9ea8773b0e"
      },
      "counts": {
        "KILLED": 8,
        "SURVIVED": 0,
        "INVALID_OR_ERROR": 0
      }
    },
    "mutant_notes": "Eight controls each executed all26 tests: repeated deleted/inert, budget deleted/inert, multivalue guard deleted, indexed path disabled, inline guard deleted, empty-default guard deleted. No survivor or invalid/compile-only kill. Budget deletion/inert each fail3 tests; all others fail1. Controls ran eae19f2b5; final exec differs only in five documentation lines. Exact source restored before final26-pass run.",
    "cargo": "Pinned installed rustc1.97.1/cargo1.97.1, --locked --offline, two build jobs, RAYON_NUM_THREADS=2, test-threads=1, debug info0, task-local stage1 target reused. No installations.",
    "feature_limit": "--no-default-features disables engine parallel/regex/digest defaults, but its existing core dev-dependency still enables core parallel/mmap/dict-spill. This is not a wholly single-threaded-core or wasm build claim.",
    "preflight": "Final preflight exits1 only on local Bash3.2 missing mapfile in scripts/check-privacy-claims.sh:92. Other applicable checks ran. No hook/gate bypass or script changes.",
    "diff_check": "PASS",
    "stage1_integrity": {
      "head": "803bb795201782551d984221a48b32ed391f7d9c",
      "verified_files": [
        "indexed-function.txt",
        "focused-repaired.txt",
        "recovery.diff",
        "report.json",
        "t1-main-fallback.txt",
        "mutant-indexed_path_disabled.txt",
        "mutant-repeated_inert.txt",
        "mutant-baseline.txt",
        "astra-repair.diff",
        "import.txt",
        "mutant-budget_inert.txt",
        "mutant-budget_deleted.txt",
        "mutant-restored.txt",
        "resource-raw.txt",
        "mutate.py",
        "preflight.txt",
        "current-tests.txt",
        "historical-topk.patch",
        "mutations.txt",
        "existing-orderby-tests.txt",
        "mutation-report.json",
        "mutant-fanout_guard_deleted.txt",
        "t1-raw.txt",
        "mutant-repeated_deleted.txt"
      ],
      "count": 24,
      "result": "All stage1 manifest entries unchanged; cache target intentionally reused."
    }
  },
  "limits": [
    "No latency, throughput, CPU, allocation-count or peak-memory measurement. Incidental EXPLAIN timing text is not benchmark evidence. No O(k) or PostgreSQL-gap claim.",
    "Upfront full seed inline validation and per-probe subject-id vectors remain linear. Overlay/compressed scans may allocate. Selectivity/tie/failure guards can spend work before fallback. These costs require measurement before a performance claim.",
    "No new resource arithmetic: every armed budget declines to existing accounting/cancellation. The unlimited shortcut still performs unbudgeted setup allocations.",
    "Full-key tie results can differ from stable fallback input order while remaining valid SPARQL. Callers requiring a deterministic subset must supply a distinguishing ORDER BY key.",
    "W3C fixture checkout and conformance binary are absent in this isolated checkout/cache. No fetch, conformance run, full-workspace build/clippy/test, wasm, ZK, algebra-rewrite-specific, mmap/compressed-file or release-feature test is claimed.",
    "The packet includes complete critical executable callers/helpers and raw diff. It does not recursively duplicate the entire unchanged expression evaluator, substrate comparator, parser, storage serialization or optional feature implementation. Those boundaries remain normal whole-project review/conformance responsibilities.",
    "No independent review, remote branch/issue/comment mutation, admission, dispatch, live CI operation or infrastructure change."
  ],
  "next_step": "Root obtains actual independent Opus5 xhigh whole-change review. Resolve concrete blockers before measurement or publication. Then prepare one reproducible local benchmark using the existing registry: compare exact candidate, same candidate with indexed admission disabled, and current main on identical generated single-valued star data. Vary depth/k/tie density/selectivity/drained prefix; report full raw data, features, host, repetitions and controlled threads, including setup/allocation costs. Full conformance/workspace gates remain required before admission."
}
```

## Raw whole-change diff

```diff
diff --git a/crates/sparq-engine/src/exec.rs b/crates/sparq-engine/src/exec.rs
index c25e391ac..c0f63fb14 100644
--- a/crates/sparq-engine/src/exec.rs
+++ b/crates/sparq-engine/src/exec.rs
@@ -3131,6 +3131,17 @@ fn try_topk_orderby(
             try_topk_orderby(graph, local, red_inner, row_budget)
         }
         GraphPattern::OrderBy { inner: ord_inner, expression } => {
+            // [SONNET-5] (sq-artifact-keeper-topk) Try the index-ordered seed path
+            // first: for the narrow "star BGP, ORDER BY a variable bound by exactly
+            // one pattern's object" shape, it walks a value-sorted permutation scan
+            // directly instead of materialising every matching row — see
+            // `try_topk_orderby_indexed`'s doc comment for the soundness argument.
+            // Declines (`Ok(None)`) for any shape/datatype it can't prove safe, in
+            // which case the existing materialize-then-select path below runs
+            // unchanged.
+            if let Some(b) = try_topk_orderby_indexed(graph, local, ord_inner, expression, row_budget)? {
+                return Ok(Some(b));
+            }
             let mut b = eval_modified(graph, local, ord_inner)?;
             // Use the top-k path only when the budget is strictly less than n;
             // otherwise the heap adds overhead with no benefit.
@@ -3144,6 +3155,429 @@ fn try_topk_orderby(
     }
 }
 
+/// [SONNET-5] (sq-artifact-keeper-topk) Attempts INDEX-ORDERED top-k evaluation for a
+/// `Slice{OrderBy{BGP}}` shape whose primary sort key is bound by exactly one triple
+/// pattern's OBJECT. `try_topk_orderby`'s existing bounded-heap path still calls
+/// `eval_modified` to fully evaluate (and materialise) the WHOLE matching candidate
+/// set before selecting the top `row_budget` rows — `O(n)` in the number of rows
+/// CURRENTLY matching the BGP's equality filters, not in `row_budget`. That is fine
+/// when some pattern is selective, but when every pattern matches nearly the same
+/// rows (e.g. a job-queue "claim the next pending task for peer X, ordered by
+/// priority" query, where `peer` + `status` don't shrink the candidate set at all),
+/// there is no cheaper seed for the cardinality-based planner to pick, and the cost
+/// scales with the size of the whole pending queue on every single claim — profiled
+/// and confirmed via `EXPLAIN ANALYZE` (the BGP operator reports touching every
+/// matching row) in the sparq/artifact-keeper migration's claim-queue throughput
+/// investigation.
+///
+/// SOUNDNESS. Only activates for a narrow, syntactically-verified "star" shape, and
+/// declines (`Ok(None)`) at the first sign of anything it cannot prove safe — the
+/// caller's `eval_modified`-then-select path is always correct, just `O(n)`; this
+/// function's only job is to be a provably-equivalent shortcut, never a new source
+/// of results:
+/// - `ord_inner` is a plain triple-pattern conjunction with NO residual FILTER and
+///   no blank nodes (kept out of scope for this first cut).
+/// - The FIRST order key is a bare `Variable` (secondary tie-break keys, if any,
+///   are resolved by re-using `order_bindings` over the small collected set below —
+///   not reimplemented here).
+/// - Exactly one pattern (the "seed") binds that variable as its OBJECT, with a
+///   CONSTANT predicate and a VARIABLE subject (the join "hub").
+/// - Every OTHER pattern has that same hub variable as its SUBJECT and a CONSTANT
+///   predicate (a star join around the hub); any object variable it introduces must
+///   not appear anywhere else in the BGP. Any other shape (the hub appearing
+///   elsewhere, a variable predicate, a reused object variable) declines.
+/// - Every object id on the seed's sorted scan is an INLINE small integer
+///   (`dict::is_inline`) — the only case where ascending dictionary-id order is
+///   proven to equal ascending SPARQL `value()` order (the same guard
+///   `scan_to_bindings`'s range-pruning already relies on for the same reason).
+///   Checked for the WHOLE scan up front, before any result is built, so a
+///   non-inline id never leaks a partially-computed (and potentially
+///   wrongly-ordered) answer.
+///
+/// Under those conditions, walking the seed's sorted scan in the required
+/// direction, testing each candidate hub value against the OTHER patterns via a
+/// direct bound lookup (index-nested-loop / bind join over a per-pattern
+/// subject-sorted range resolved once, not re-resolved per candidate — see the
+/// `other_pats` construction below), and stopping once a COMPLETE sort-key group
+/// (never split mid-tie) has produced at least `row_budget` confirmed joins
+/// yields valid top rows: an unvisited group is strictly worse in the requested
+/// primary-key direction, so it cannot displace the collected top `row_budget`.
+/// [GPT-6 Astra] If all ORDER BY keys tie, SPARQL permits different surviving
+/// subsets and tie order; equivalence does not require the fallback's input-index
+/// stability. Finishing a primary-key group preserves secondary-key selection.
+///
+/// PERFORMANCE, not soundness: a single escalation block is also capped at a
+/// fraction of the candidate pool (see `max_group` below) — exceeding it
+/// declines (`Ok(None)`) even though continuing would still be CORRECT, because
+/// past that point this function's per-candidate cost stops being cheaper than
+/// the fallback's bulk join (measured; see `max_group`'s comment). This keeps
+/// the walk bounded, but setup and failed probes still precede a decline.
+///
+/// [GPT-6 Astra] Recovery-stage restriction: budgeted queries, repeated variables
+/// and multi-valued probes use the existing evaluator. The historical timing notes
+/// below are unverified on current main; this candidate establishes no speedup.
+fn try_topk_orderby_indexed(
+    graph: &Graph,
+    local: &mut LocalVocab,
+    ord_inner: &GraphPattern,
+    expression: &[OrderExpression],
+    row_budget: usize,
+) -> Result<Option<Bindings>, String> {
+    #[cfg(feature = "zk")]
+    if crate::zk::enabled() {
+        return Ok(None);
+    }
+    // [GPT-6 Astra] Preserve the existing evaluator's intermediate row/byte
+    // accounting and cooperative cancellation schedule before doing any new work.
+    if budget::active() {
+        return Ok(None);
+    }
+    if view::default_is_empty() {
+        return Ok(None);
+    }
+    let Some((desc, order_var)) = expression.first().and_then(|oe| match oe {
+        OrderExpression::Asc(Expression::Variable(v)) => Some((false, v.clone())),
+        OrderExpression::Desc(Expression::Variable(v)) => Some((true, v.clone())),
+        _ => None,
+    }) else {
+        return Ok(None);
+    };
+    if !is_conjunctive(ord_inner) {
+        return Ok(None);
+    }
+    let mut patterns = Vec::new();
+    let mut filters = Vec::new();
+    flatten_conjunction(ord_inner, &mut patterns, &mut filters);
+    if !filters.is_empty() || patterns.is_empty() {
+        return Ok(None);
+    }
+    // [GPT-6 Astra] prepare_pattern records repeated slots but does not enforce
+    // their equality. Reuse the existing fast-path guard before bypassing build_row.
+    if patterns.iter().any(has_intra_triple_repeated_var) {
+        return Ok(None);
+    }
+    // Out of scope for this first cut: blank nodes are treated as synthetic
+    // variables by `prepare_pattern` (`bnode_var`) but not necessarily by
+    // `collect_vars`'s header-construction — decline rather than risk a header
+    // mismatch.
+    let has_blank_node = patterns.iter().any(|tp| {
+        matches!(tp.subject, TermPattern::BlankNode(_)) || matches!(tp.object, TermPattern::BlankNode(_))
+    });
+    if has_blank_node {
+        return Ok(None);
+    }
+
+    // Locate the ONE seed pattern binding `order_var` as its object with a
+    // constant predicate and a variable subject.
+    let mut seed_idx = None;
+    for (i, tp) in patterns.iter().enumerate() {
+        if !matches!(&tp.object, TermPattern::Variable(v) if *v == order_var) {
+            continue;
+        }
+        if !matches!(&tp.predicate, NamedNodePattern::NamedNode(_)) {
+            return Ok(None);
+        }
+        if !matches!(&tp.subject, TermPattern::Variable(_)) {
+            return Ok(None);
+        }
+        if seed_idx.is_some() {
+            return Ok(None); // order_var bound by more than one pattern: not this shape
+        }
+        seed_idx = Some(i);
+    }
+    let Some(seed_idx) = seed_idx else { return Ok(None) };
+    let hub_var = match &patterns[seed_idx].subject {
+        TermPattern::Variable(v) => v.clone(),
+        _ => unreachable!("checked above"),
+    };
+
+    // Every OTHER pattern must be `hub_var <constant predicate> (var|const)` — a
+    // star around the hub — and any variable it introduces must be fresh.
+    let mut other_vars: FxHashSet<Variable> = FxHashSet::default();
+    for (i, tp) in patterns.iter().enumerate() {
+        if i == seed_idx {
+            continue;
+        }
+        if !matches!(&tp.subject, TermPattern::Variable(v) if *v == hub_var) {
+            return Ok(None);
+        }
+        if !matches!(&tp.predicate, NamedNodePattern::NamedNode(_)) {
+            return Ok(None);
+        }
+        if let TermPattern::Variable(v) = &tp.object {
+            if *v == hub_var || *v == order_var || !other_vars.insert(v.clone()) {
+                return Ok(None);
+            }
+        }
+    }
+
+    let out_vars = collect_vars(&patterns);
+    if row_budget == 0 {
+        return Ok(Some(Bindings::unsorted(out_vars, vec![])));
+    }
+
+    // Resolve + scan the seed pattern sorted by its OBJECT (canonical column 2).
+    let (seed_id_pat, _seed_pos_vars, seed_unsat) = prepare_pattern(graph, &patterns[seed_idx])?;
+    if seed_unsat {
+        return Ok(Some(Bindings::unsorted(out_vars, vec![])));
+    }
+    let scan = graph.store.scan_sorted(&seed_id_pat, 2);
+    let actual_sort = scan.perm.order().into_iter().find(|&c| seed_id_pat[c].is_none());
+    if actual_sort != Some(2) {
+        return Ok(None); // this store build can't give us an object-sorted scan
+    }
+    let rows: &[[Id; 3]] = scan.rows.as_ref();
+    // Guard: EVERY object id on this scan must be an inline integer, or ascending
+    // id order is not proven to be ascending value order.
+    if !rows.iter().all(|r| dict::is_inline(scan.to_spo(r)[2])) {
+        return Ok(None);
+    }
+
+    // Prepare the other patterns ONCE, each as a SUBJECT-sorted scan over its
+    // (fixed predicate [+ fixed object]) range — not re-resolved per candidate.
+    // The first cut of this function called `graph.store.scan(&probe_pat)`
+    // fresh for every (candidate, other-pattern) pair, which re-runs
+    // `Store::choose`'s permutation selection + bound computation every single
+    // time even though the choice is IDENTICAL across all candidates for a
+    // given pattern (only the subject varies). That repeated per-call setup —
+    // not the underlying lookup — is what made a large tie-group lose to the
+    // fallback's bulk merge-join (which resolves the permutation ONCE per
+    // pattern, not once per row). Resolving it once here and binary-searching
+    // the resulting sorted slice per candidate removes that repeated cost.
+    struct OtherPat<'g> {
+        scan: sparq_core::store::Scan<'g>,
+        // Precomputed ONCE (not per candidate, not per binary-search
+        // comparison step): the subject id of every row in `scan`, in the
+        // SAME (subject-sorted) order. `partition_point` over this plain
+        // `&[Id]` does a trivial integer compare per step; searching `scan`
+        // directly would call `to_spo` (a permutation-order reconstruction)
+        // on EVERY comparison, i.e. ~log(m) reconstructions per candidate per
+        // pattern — measured as a real remaining cost for a long skip-prefix
+        // (many candidates probed and rejected in a row): reconstructing the
+        // full [S,P,O] triple just to read column 0 is wasted work when the
+        // search only ever needs that one column.
+        subject_ids: Vec<Id>,
+        obj_var: Option<Variable>,
+    }
+    let mut other_pats: Vec<OtherPat> = Vec::with_capacity(patterns.len().saturating_sub(1));
+    for (i, tp) in patterns.iter().enumerate() {
+        if i == seed_idx {
+            continue;
+        }
+        let (id_pat, pos_vars, unsat) = prepare_pattern(graph, tp)?;
+        if unsat {
+            // A constant elsewhere in the BGP absent from the dictionary makes the
+            // WHOLE conjunction empty (a BGP join against an empty relation).
+            return Ok(Some(Bindings::unsorted(out_vars, vec![])));
+        }
+        let probe_id_pat: IdPattern = [None, id_pat[1], id_pat[2]];
+        let sub_scan = graph.store.scan_sorted(&probe_id_pat, 0);
+        let sub_actual_sort = sub_scan.perm.order().into_iter().find(|&c| probe_id_pat[c].is_none());
+        if sub_actual_sort != Some(0) {
+            // This store build can't give a subject-sorted scan for this
+            // pattern (e.g. no PSO permutation under `compact-index`/wasm) —
+            // decline rather than binary-search an unsorted range.
+            return Ok(None);
+        }
+        let subject_ids: Vec<Id> = sub_scan.rows.iter().map(|r| sub_scan.to_spo(r)[0]).collect();
+        other_pats.push(OtherPat { scan: sub_scan, subject_ids, obj_var: pos_vars[2].clone() });
+    }
+
+    // UPFRONT cost check, before touching a single candidate: each `other_pats`
+    // scan's row count is the EXACT (not estimated) global cardinality of that
+    // pattern's own (predicate [+ object]) constraint. If any of them is
+    // already meaningfully smaller than the seed's own scan (`rows.len()`),
+    // the fallback's ordinary smallest-estimate seed selection will pick THAT
+    // pattern as ITS seed and materialize only that small set — beating this
+    // function's priority-ordered walk outright, with no reason to compete.
+    //
+    // This is exactly the realistic "claim strictly in priority order" shape:
+    // as such a queue drains, `ak:status="pending"` becomes highly selective
+    // while THIS function's seed (`ak:priority`, spanning the whole pool
+    // including now-claimed rows) does not shrink at all. Without this check,
+    // the only way to discover that is by actually walking past the
+    // ever-growing already-claimed prefix, probing (and rejecting) each one —
+    // real, wasted, unrecoverable cost. Measured directly: at n=1600 with
+    // 1000 of 1600 already claimed (600 truly pending), that reactive
+    // discovery cost ~237-272us total (wasted probes + the fallback anyway)
+    // vs. this upfront check's ~172-180us (matches a clean fallback-only
+    // cost, because it declines before doing ANY per-candidate work).
+    let seed_card = rows.len();
+    let min_other_card = other_pats.iter().map(|op| op.scan.rows.len()).min().unwrap_or(seed_card);
+    if min_other_card.saturating_mul(2) < seed_card {
+        return Ok(None);
+    }
+
+    // The output column list is FIXED across every candidate (hub, order, then
+    // each other pattern's object variable, in pattern order) — compute it and
+    // the out_vars -> column-position mapping ONCE, not per candidate.
+    let mut cols: Vec<Variable> = Vec::with_capacity(2 + other_pats.len());
+    cols.push(hub_var.clone());
+    cols.push(order_var.clone());
+    for op in &other_pats {
+        if let Some(ov) = &op.obj_var {
+            cols.push(ov.clone());
+        }
+    }
+    let out_to_cols: Vec<Option<usize>> = out_vars.iter().map(|v| cols.iter().position(|c| c == v)).collect();
+    let emit = |ids: &[Id], collected: &mut Vec<Row>| {
+        let mut row = Row::with_capacity(out_to_cols.len());
+        for pos in &out_to_cols {
+            row.push(pos.map(|i| ids[i]).unwrap_or(NO_ID));
+        }
+        collected.push(row);
+    };
+
+    // Walk the sorted scan in the required direction, in escalating blocks aligned
+    // to complete sort-key (object-id) groups, joining each candidate against the
+    // other patterns via a direct bound lookup (index-nested-loop / bind join).
+    //
+    // `rows` is always ASCENDING by object id (the scan's actual sort order). For
+    // DESC, "best first" means walking it back-to-front. Rather than slicing from
+    // the front and reversing per block (which — subtly, and wrongly — visits the
+    // SMALLEST values first regardless of direction, since the slice bounds
+    // themselves were never flipped), define a single LOGICAL index `0..n` where
+    // position 0 is always the best candidate, via `logical`, and do all
+    // block/boundary arithmetic in that space. `logical` and `obj_id_at` are the
+    // only direction-aware code; everything below them is direction-agnostic.
+    let n = rows.len();
+    let logical = |i: usize| -> usize { if desc { n - 1 - i } else { i } };
+    let obj_id_at = |i: usize| -> Id { scan.to_spo(&rows[logical(i)])[2] };
+    // A single escalation block is dominated by ONE large tie-group when the
+    // sort key has low cardinality (e.g. a handful of discrete priority tiers:
+    // "urgent/high/normal/low", not a near-unique value per row). This
+    // function's per-candidate cost is roughly CONSTANT per candidate (a
+    // binary search per other pattern, resolved against a scan fetched once —
+    // see `other_pats` below), while the fallback's bulk merge-join cost is
+    // roughly constant PER `n` regardless of tie structure (it materialises the
+    // whole matching set before it can sort). So the crossover is a FRACTION of
+    // `n`, not an absolute row count: measured at n=1600, a tie-group of 800
+    // (half of n) still beat the fallback (~228us vs. the fallback's ~269us),
+    // but a tie-group of 1600 (all of n) lost (would be ~450-700us vs. the
+    // fallback's own ~269us) — and the same ~0.5-0.75 fraction held at n=8000
+    // (4000 still competitive, 8000 clearly lost). `max_group` below is
+    // therefore `n / 2` (with a floor for small `n`, and never below
+    // `row_budget` itself) — declining past it defers to the fallback, whose
+    // cost at that point is exactly its normal (tie-structure-independent)
+    // cost, not a new regression.
+    const MAX_INDEXED_GROUP_FLOOR: usize = 256;
+    let mut collected: Vec<Row> = Vec::new();
+    let mut visited_to: usize = 0;
+    // Block sizes grow GEOMETRICALLY from a small multiple of `row_budget`, not
+    // from `CAPPED_SEED_BLOCK` (1024): each candidate here costs a real
+    // per-pattern binary search (against the scan `other_pats` already resolved
+    // once, above), unlike the bulk merge-join the fallback path uses, which
+    // amortises its own permutation selection over the WHOLE scan in one pass.
+    // For the common case (small `row_budget`, most candidates
+    // join successfully), starting near `row_budget` means the first block alone
+    // usually satisfies it — starting at 1024 would pay ~1024 point-probes even for
+    // `LIMIT 1`, which is worse than the bulk path it's meant to beat (measured:
+    // this was the actual cause of a regression at moderate `n`, not a win).
+    let max_group = (n / 2).max(MAX_INDEXED_GROUP_FLOOR).max(row_budget.saturating_mul(2));
+    // Cumulative count of candidates that failed an OTHER-pattern check, across
+    // ALL blocks in this call — distinct from `max_group`'s per-block width
+    // check. A workload that claims strictly in priority order (the realistic
+    // shape: highest priority first) leaves an ever-growing prefix of
+    // ALREADY-CLAIMED, high-priority-but-no-longer-`pending` rows at the head
+    // of the seed scan — each one still costs a real per-pattern probe before
+    // being rejected. That prefix is made of individually DISTINCT priority
+    // values, so it never forms one oversized tie-group `max_group` would
+    // catch; it spreads across many small geometric-growth blocks instead. The
+    // UPFRONT cost check above (comparing `other_pats`' exact cardinalities to
+    // the seed's) already declines the CLEAR case — an other-pattern that's
+    // globally selective enough for the fallback's own planner to prefer as
+    // ITS seed — before any candidate is even touched. This counter is a
+    // SAFETY NET for what that check can't see: the seed's global cardinality
+    // vs. an other-pattern's global cardinality doesn't capture every
+    // possible skip-prefix shape (e.g. a correlation between scan order and
+    // an other-pattern's matches that isn't visible from cardinality alone).
+    //
+    // The budget here must stay LARGE (matching `max_group`, not a small
+    // constant): every failed probe before declining is pure waste stacked ON
+    // TOP OF the fallback's full cost, but a SMALL budget bails on the far
+    // more common "moderate skip-prefix" case too — one that would have
+    // SUCCEEDED cheaply if allowed to keep going (a candidate ~100-800
+    // positions in costs only tens of us to walk to, vastly cheaper than a
+    // ~100-350us fallback). Measured directly: tightening this to a small
+    // constant (64) made the COMMON moderate case (already-claimed ~100-400)
+    // cost ~225-275us (forced into a fallback the upfront check didn't catch
+    // and completion would have avoided entirely) instead of the ~15-70us it
+    // gets by simply being allowed to finish. A large budget's own worst case
+    // (a skip-prefix near `n/2`, right at the edge) is bounded and modest by
+    // comparison (~30-50% over a clean fallback, not multiples of it) — an
+    // acceptable price for a case the upfront check should catch almost
+    // always in practice anyway.
+    let failure_budget = max_group;
+    let mut failed_count: usize = 0;
+    let mut block_target = row_budget.saturating_mul(4).max(16);
+    loop {
+        if visited_to >= n {
+            break;
+        }
+        let mut end = block_target.clamp(visited_to, n);
+        // Extend to the next object-id group boundary so a tie is never split
+        // across blocks (the early-stop check below requires a COMPLETE group).
+        if end < n && end > 0 {
+            let boundary_obj = obj_id_at(end - 1);
+            while end < n && obj_id_at(end) == boundary_obj {
+                end += 1;
+            }
+        }
+        if end - visited_to > max_group {
+            return Ok(None);
+        }
+        for i in visited_to..end {
+            let spo = scan.to_spo(&rows[logical(i)]);
+            let hub_id = spo[0];
+            let order_id = spo[2];
+            // [GPT-6 Astra] Only single-valued probes are admitted in this slice.
+            // General Cartesian expansion remains with the existing evaluator.
+            let mut row_ids: SmallVec<[Id; 8]> = SmallVec::from_slice(&[hub_id, order_id]);
+            let mut failed = false;
+            for op in &other_pats {
+                // Binary-search the PRECOMPUTED, plain-`Id` subject list for
+                // this pattern (built once, above) — a trivial integer
+                // compare per step, no `to_spo` reconstruction during the
+                // search itself (that only happens below, per ACTUAL match,
+                // not per comparison step — see `subject_ids`'s doc comment).
+                let start = op.subject_ids.partition_point(|&id| id < hub_id);
+                let stop = start + op.subject_ids[start..].partition_point(|&id| id == hub_id);
+                if start == stop {
+                    failed = true;
+                    break;
+                }
+                let Some(_) = &op.obj_var else {
+                    continue; // `obj_const` — existence-only, no column added
+                };
+                let op_rows: &[[Id; 3]] = op.scan.rows.as_ref();
+                let match_count = stop - start;
+                if match_count != 1 {
+                    return Ok(None);
+                }
+                row_ids.push(op.scan.to_spo(&op_rows[start])[2]);
+            }
+            if failed {
+                failed_count += 1;
+                if failed_count > failure_budget {
+                    return Ok(None);
+                }
+                continue;
+            }
+            emit(&row_ids, &mut collected);
+        }
+        visited_to = end;
+        if collected.len() >= row_budget {
+            break; // a COMPLETE group boundary was just finished — safe to stop
+        }
+        block_target = block_target.saturating_mul(4).max(visited_to + 1);
+    }
+
+    let mut result = Bindings { vars: out_vars, rows: collected, sorted_by: None };
+    let use_topk = result.rows.len() > row_budget;
+    order_bindings(graph, local, &mut result, expression, if use_topk { Some(row_budget) } else { None })?;
+    Ok(Some(result))
+}
+
 /// [OPUS-4.8] (sq-7d3dj.30.4) Attempts the DISTINCT-projection loose skip-scan for the
 /// pattern under a `Distinct`.
 ///
diff --git a/crates/sparq-engine/tests/topk_orderby_indexed_differential.rs b/crates/sparq-engine/tests/topk_orderby_indexed_differential.rs
new file mode 100644
index 000000000..6a56a2b75
--- /dev/null
+++ b/crates/sparq-engine/tests/topk_orderby_indexed_differential.rs
@@ -0,0 +1,519 @@
+//! Differential + correctness tests for `try_topk_orderby_indexed` (the
+//! index-ordered top-k seed path added for the artifact-keeper claim-queue
+//! throughput investigation).
+//!
+//! `try_topk_orderby_indexed` is a pure SHORTCUT: it either returns exactly the
+//! valid rows the pre-existing `eval_modified`-then-`order_bindings` path would,
+//! or declines (`Ok(None)`) and lets that path run. [GPT-6 Astra] Total-order
+//! tests use an independent Rust sort or full ORDER BY oracle; exact ties assert
+//! permitted membership and size. Execution traces pin admission/fallback guards.
+
+use sparq_core::Graph;
+use sparq_engine::query;
+
+const PFX: &str = "PREFIX ak: <http://example.org/ak#>\n";
+
+// [GPT-6 Astra] A repeated variable must bind one term in both positions.
+// An IRI subject and an integer object can never satisfy this triple pattern.
+#[test]
+fn repeated_seed_variable_full_orderby_is_empty() {
+    let graph = Graph::load_str("<urn:s> <urn:p> 1 .", "turtle").unwrap();
+    let rows = query(&graph, "SELECT ?x WHERE { ?x <urn:p> ?x } ORDER BY ?x").unwrap();
+    assert!(rows.rows.is_empty(), "full ORDER BY must enforce subject/object equality: {rows:?}");
+}
+
+// [GPT-6 Astra] The LIMIT path must preserve the full evaluator's equality guard.
+#[test]
+fn repeated_seed_variable_limit_is_empty() {
+    let graph = Graph::load_str("<urn:s> <urn:p> 1 .", "turtle").unwrap();
+    let rows = query(&graph, "SELECT ?x WHERE { ?x <urn:p> ?x } ORDER BY ?x LIMIT 1").unwrap();
+    assert!(rows.rows.is_empty(), "LIMIT must enforce subject/object equality: {rows:?}");
+}
+
+// [GPT-6 Astra] LIMIT cannot hide an oversized intermediate working set.
+#[test]
+fn indexed_topk_preserves_intermediate_resource_limits() {
+    let graph = build_graph("X", &(0..40).map(|i| (i, i)).collect::<Vec<_>>(), "");
+    let query_text = format!("{PFX}{CLAIM_QUERY}1");
+    for (budget, expected) in [
+        (sparq_engine::QueryBudget { max_rows: Some(2), ..Default::default() }, "max-rows"),
+        (sparq_engine::QueryBudget { max_bytes: Some(16), ..Default::default() }, "max-bytes"),
+    ] {
+        let result = sparq_engine::query_with_budget(&graph, &query_text, &budget);
+        assert!(result.as_ref().is_err_and(|error| error.contains(expected)), "{expected}: {result:?}");
+    }
+    let generous = sparq_engine::QueryBudget { max_rows: Some(1000), ..Default::default() };
+    let result = sparq_engine::query_with_budget(&graph, &query_text, &generous).unwrap();
+    assert_eq!(task_seq(result.rows[0][0].as_ref().unwrap()), 39);
+}
+
+// [GPT-6 Astra] Keep the initial shortcut out of Cartesian fanout. Check both
+// the ordered answers and the actual fallback execution, not static plan text.
+#[test]
+fn multivalued_probe_uses_fallback_and_preserves_cartesian_rows() {
+    let graph = Graph::load_str(
+        "<urn:s> <urn:p> 2 ; <urn:b> 10, 11 ; <urn:c> 20, 21 .", "turtle",
+    ).unwrap();
+    let query_text = "SELECT ?s ?b ?c WHERE { ?s <urn:p> ?p ; <urn:b> ?b ; <urn:c> ?c } ORDER BY DESC(?p) ?b ?c";
+    let full = query(&graph, query_text).unwrap();
+    assert_eq!(full.rows.len(), 4);
+    let limited_query = format!("{query_text} LIMIT 3");
+    let limited = query(&graph, &limited_query).unwrap();
+    assert_eq!(limited.rows, full.rows[..3]);
+    let trace = sparq_engine::explain_analyze(&graph, &limited_query).unwrap();
+    assert!(trace.contains("BGP [binary GOO]"), "multivalued probe must execute the fallback: {trace}");
+}
+
+// [GPT-6 Astra] These two keys give a total order, so every OFFSET/LIMIT window
+// must equal the same full-order prefix/window, including complete tie boundaries.
+#[test]
+fn total_order_desc_offset_windows_match_full_sort() {
+    let graph = build_graph("X", &(0..96).map(|i| (i, i % 12)).collect::<Vec<_>>(), "");
+    let text = format!("{PFX}SELECT ?t WHERE {{ ?t ak:peer <urn:peer:X> ; ak:status \"pending\" ; ak:priority ?p ; ak:seq ?s }} ORDER BY DESC(?p) DESC(?s)");
+    let full = query(&graph, &text).unwrap();
+    assert_eq!(full.rows.len(), 96);
+    for (offset, limit) in [(0, 0), (0, 1), (7, 2), (8, 9), (15, 3), (31, 17), (95, 4), (96, 1)] {
+        let got = query(&graph, &format!("{text} OFFSET {offset} LIMIT {limit}")).unwrap();
+        let expected: Vec<_> = full.rows.iter().skip(offset).take(limit).cloned().collect();
+        assert_eq!(got.rows, expected, "offset={offset}, limit={limit}");
+    }
+}
+
+// [GPT-6 Astra] With no secondary key, SPARQL does not specify which tied
+// subjects survive. Assert valid membership, multiplicity and size, not stability.
+#[test]
+fn exact_desc_ties_preserve_valid_membership_and_window_size() {
+    let graph = build_graph("X", &(0..32).map(|i| (i, 7)).collect::<Vec<_>>(), "");
+    let text = format!("{PFX}SELECT ?t ?p WHERE {{ ?t ak:peer <urn:peer:X> ; ak:status \"pending\" ; ak:priority ?p }} ORDER BY DESC(?p)");
+    for (offset, limit) in [(0usize, 5usize), (3, 9), (28, 10), (40, 2), (0, 0)] {
+        let got = query(&graph, &format!("{text} OFFSET {offset} LIMIT {limit}")).unwrap();
+        assert_eq!(got.rows.len(), 32usize.saturating_sub(offset).min(limit));
+        let ids: std::collections::HashSet<_> = got.rows.iter()
+            .map(|row| task_seq(row[0].as_ref().unwrap())).collect();
+        assert_eq!(ids.len(), got.rows.len(), "no duplicate solution may be manufactured");
+        assert!(ids.iter().all(|id| (0..32).contains(id)));
+        assert!(got.rows.iter().all(|row| as_int(row[1].as_ref().unwrap()) == 7));
+    }
+}
+
+// [GPT-6 Astra] Each fixture has distinct numeric values, avoiding unspecified ties.
+#[test]
+fn negative_mixed_and_typed_lexical_values_use_the_fallback() {
+    for values in [
+        ["-5", "-1", "-3"],
+        ["\"02\"^^xsd:integer", "\"4\"^^xsd:int", "1.5"],
+        ["1", "-2", "3.5"],
+    ] {
+        let mut ttl = String::from("@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .\n");
+        for (i, value) in values.iter().enumerate() {
+            ttl.push_str(&format!("<urn:s{i}> <urn:p> {value} .\n"));
+        }
+        let graph = Graph::load_str(&ttl, "turtle").unwrap();
+        let text = "SELECT ?s ?p WHERE { ?s <urn:p> ?p } ORDER BY DESC(?p) ?s";
+        let full = query(&graph, text).unwrap();
+        let limited = format!("{text} LIMIT 2");
+        assert_eq!(query(&graph, &limited).unwrap().rows, full.rows[..2]);
+        let trace = sparq_engine::explain_analyze(&graph, &limited).unwrap();
+        assert!(trace.contains("BGP [binary GOO]"), "non-inline fixture must decline: {values:?}, {trace}");
+    }
+}
+
+// [GPT-6 Astra] Exercise an indexed subquery inside GRAPH, where the empty-default
+// view is suspended for a visible named graph. Hidden data must remain inaccessible.
+#[test]
+fn ordered_subqueries_preserve_default_and_named_view_boundaries() {
+    use sparq_engine::{DatasetView, DefaultGraphMode, query_view};
+    let graph = Graph::load_dataset(
+        "<urn:d> <urn:p> \"1\"^^<http://www.w3.org/2001/XMLSchema#integer> .\n\
+         <urn:v> <urn:p> \"2\"^^<http://www.w3.org/2001/XMLSchema#integer> <urn:visible> .\n\
+         <urn:h> <urn:p> \"99\"^^<http://www.w3.org/2001/XMLSchema#integer> <urn:hidden> .\n",
+        "nquads",
+    ).unwrap();
+    let named = std::sync::Arc::new([oxrdf::Term::NamedNode(oxrdf::NamedNode::new("urn:visible").unwrap())].into_iter().collect());
+    let default_query = "SELECT ?s WHERE { ?s <urn:p> ?p } ORDER BY DESC(?p) LIMIT 1";
+    let named_query = |name: &str| format!("SELECT ?s WHERE {{ GRAPH <{name}> {{ SELECT ?s WHERE {{ ?s <urn:p> ?p }} ORDER BY DESC(?p) LIMIT 1 }} }}");
+    for default in [DefaultGraphMode::StoreDefault, DefaultGraphMode::Empty] {
+        let view = DatasetView { base: &graph, named: std::sync::Arc::clone(&named), default };
+        let rows = query_view(&view, default_query).unwrap().rows;
+        assert_eq!(rows.len(), usize::from(default == DefaultGraphMode::StoreDefault));
+        if let Some(row) = rows.first() { assert_eq!(row[0].as_ref().unwrap().to_string(), "<urn:d>"); }
+        let visible = query_view(&view, &named_query("urn:visible")).unwrap();
+        assert_eq!(visible.rows[0][0].as_ref().unwrap().to_string(), "<urn:v>");
+        assert_eq!(visible.rows.len(), 1);
+        for hidden in ["urn:hidden", "urn:absent"] {
+            assert!(query_view(&view, &named_query(hidden)).unwrap().rows.is_empty());
+            let from = format!("SELECT ?s FROM <{hidden}> WHERE {{ ?s <urn:p> ?p }} ORDER BY DESC(?p) LIMIT 1");
+            assert!(query_view(&view, &from).unwrap().rows.is_empty());
+        }
+    }
+    assert_eq!(query(&graph, &named_query("urn:hidden")).unwrap().rows[0][0].as_ref().unwrap().to_string(), "<urn:h>");
+}
+
+// [GPT-6 Astra] A scan must observe both overlay tombstones and newly inserted ids,
+// while a retained snapshot and the subsequently compacted result stay equivalent.
+#[test]
+fn forked_overlay_and_compaction_preserve_ordered_answers() {
+    let base = build_graph("X", &(0..40).map(|i| (i, i)).collect::<Vec<_>>(), "");
+    let mut changed = base.fork();
+    sparq_engine::update_in_place(&mut changed, &format!("{PFX}DELETE DATA {{ <urn:task:X:39> ak:priority 39 }}; INSERT DATA {{ <urn:task:X:90> ak:peer <urn:peer:X> ; ak:status \"pending\" ; ak:priority 90 ; ak:seq 90 }}")).unwrap();
+    assert!(changed.pending_delta_len() > 0);
+    for k in [1, 7, 40, 50] { assert_eq!(actual_top_k(&changed, k), expected_top_k(&changed, k)); }
+    assert_eq!(actual_top_k(&changed, 1), vec![90]);
+    assert_eq!(actual_top_k(&base, 1), vec![39]);
+    let before = actual_top_k(&changed, 50);
+    changed.compact().unwrap();
+    assert_eq!(changed.pending_delta_len(), 0);
+    assert_eq!(actual_top_k(&changed, 50), before);
+}
+
+// [GPT-6 Astra] A live cancellation flag arms the fallback even while false.
+// No sleeps or concurrent timing race is needed to prove cancellation and cleanup.
+#[test]
+fn armed_cancellation_uses_fallback_and_does_not_leak() {
+    use std::sync::{Arc, atomic::{AtomicBool, Ordering}};
+    let graph = build_graph("X", &(0..40).map(|i| (i, i)).collect::<Vec<_>>(), "");
+    let text = format!("{PFX}{CLAIM_QUERY}1");
+    let flag = Arc::new(AtomicBool::new(false));
+    let budget = sparq_engine::QueryBudget::cancelled_by(Arc::clone(&flag));
+    let trace = sparq_engine::explain_analyze_with_budget(&graph, &text, &budget).unwrap();
+    assert!(trace.contains("BGP [binary GOO]"), "armed cancellation must decline: {trace}");
+    flag.store(true, Ordering::Relaxed);
+    let error = sparq_engine::query_with_budget(&graph, &text, &budget).unwrap_err();
+    assert!(error.contains("query budget exceeded (cancelled)"), "{error}");
+    assert_eq!(actual_top_k(&graph, 1), vec![39]);
+}
+
+// [GPT-6 Astra] Expiration is established before execution, never by a timed race.
+#[cfg(not(target_arch = "wasm32"))]
+#[test]
+fn armed_deadline_uses_fallback_and_expired_deadline_errors() {
+    use std::time::{Duration, Instant};
+    let graph = build_graph("X", &(0..40).map(|i| (i, i)).collect::<Vec<_>>(), "");
+    let text = format!("{PFX}{CLAIM_QUERY}1");
+    let future = sparq_engine::QueryBudget { deadline: Some(Instant::now() + Duration::from_secs(3600)), ..Default::default() };
+    let trace = sparq_engine::explain_analyze_with_budget(&graph, &text, &future).unwrap();
+    assert!(trace.contains("BGP [binary GOO]"), "armed deadline must decline: {trace}");
+    let expired = sparq_engine::QueryBudget { deadline: Some(Instant::now()), ..Default::default() };
+    let error = sparq_engine::query_with_budget(&graph, &text, &expired).unwrap_err();
+    assert!(error.contains("query budget exceeded (timeout)"), "{error}");
+    assert_eq!(actual_top_k(&graph, 1), vec![39]);
+}
+
+/// One synthetic pending task: `(seq, priority)`. `seq` doubles as a stable,
+/// unique task identifier so results can be checked by task number.
+fn build_graph(peer: &str, tasks: &[(i64, i64)], extra_ttl: &str) -> Graph {
+    let mut ttl = String::from("@prefix ak: <http://example.org/ak#> .\n");
+    for &(seq, priority) in tasks {
+        ttl.push_str(&format!(
+            "<urn:task:{peer}:{seq}> ak:peer <urn:peer:{peer}> ; ak:status \"pending\" ; ak:priority {priority} ; ak:seq {seq} .\n"
+        ));
+    }
+    ttl.push_str(extra_ttl);
+    Graph::load_str(&ttl, "turtle").expect("load turtle")
+}
+
+const CLAIM_QUERY: &str = "
+SELECT ?t WHERE {
+  ?t ak:peer <urn:peer:X> ; ak:status \"pending\" ; ak:priority ?p ; ak:seq ?s .
+}
+ORDER BY DESC(?p) ASC(?s)
+LIMIT ";
+
+fn task_seq(t: &oxrdf::Term) -> i64 {
+    match t {
+        oxrdf::Term::NamedNode(n) => n
+            .as_str()
+            .rsplit(':')
+            .next()
+            .unwrap()
+            .parse()
+            .expect("task IRI ends in :<seq>"),
+        other => panic!("expected a task IRI, got {other:?}"),
+    }
+}
+
+/// Ground truth: fetch every pending task's (seq, priority) via a query the new
+/// path never activates on (no ORDER BY/LIMIT at all), sort in test code by the
+/// exact same key (DESC priority, ASC seq), and return the expected task-seq
+/// order for the first `k`.
+fn expected_top_k(graph: &Graph, k: usize) -> Vec<i64> {
+    let r = query(
+        graph,
+        &format!("{PFX} SELECT ?s ?p WHERE {{ ?t ak:peer <urn:peer:X> ; ak:status \"pending\" ; ak:priority ?p ; ak:seq ?s . }}"),
+    )
+    .unwrap();
+    let mut rows: Vec<(i64, i64)> = r
+        .rows
+        .iter()
+        .map(|row| {
+            let s = as_int(row[0].as_ref().unwrap());
+            let p = as_int(row[1].as_ref().unwrap());
+            (s, p)
+        })
+        .collect();
+    rows.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0))); // DESC priority, ASC seq
+    rows.into_iter().take(k).map(|(s, _)| s).collect()
+}
+
+fn as_int(t: &oxrdf::Term) -> i64 {
+    match t {
+        oxrdf::Term::Literal(l) => l.value().parse().unwrap(),
+        other => panic!("expected an integer literal, got {other:?}"),
+    }
+}
+
+fn actual_top_k(graph: &Graph, k: usize) -> Vec<i64> {
+    let r = query(graph, &format!("{PFX}{CLAIM_QUERY}{k}")).unwrap();
+    r.rows.iter().map(|row| task_seq(row[0].as_ref().unwrap())).collect()
+}
+
+fn check(tasks: &[(i64, i64)], ks: &[usize]) {
+    let graph = build_graph("X", tasks, "");
+    for &k in ks {
+        let expected = expected_top_k(&graph, k);
+        let actual = actual_top_k(&graph, k);
+        assert_eq!(actual, expected, "k={k}, n={}", tasks.len());
+    }
+}
+
+#[test]
+fn unique_priorities_small() {
+    // Deliberately unsorted insertion order.
+    let tasks: Vec<(i64, i64)> = vec![(0, 5), (1, 20), (2, 1), (3, 15), (4, 9), (5, 30), (6, 0)];
+    check(&tasks, &[1, 2, 3, 7, 100]);
+}
+
+#[test]
+fn unique_priorities_pseudo_random_medium() {
+    let n = 300;
+    let tasks: Vec<(i64, i64)> = (0..n)
+        .map(|i| (i as i64, ((i as i64) * 2654435761i64) % (n as i64 * 7)))
+        .collect();
+    check(&tasks, &[1, 5, 50, 299, 300, 400]);
+}
+
+#[test]
+fn ties_broken_by_seq_ascending() {
+    // Three groups of tied priorities; within a group, seq must break the tie
+    // ascending regardless of insertion order.
+    let tasks: Vec<(i64, i64)> = vec![
+        (5, 10), (2, 10), (8, 10), // priority 10: expect order 2,5,8
+        (1, 20), (0, 20),          // priority 20: expect order 0,1
+        (9, 5),                    // priority 5: alone
+    ];
+    check(&tasks, &[1, 2, 3, 4, 5, 6, 100]);
+}
+
+#[test]
+fn crosses_block_escalation_boundary() {
+    // CAPPED_SEED_BLOCK is 1024; exercise a queue depth comfortably on both
+    // sides of that boundary, including k values that force multiple blocks.
+    let n = 2500;
+    let tasks: Vec<(i64, i64)> = (0..n)
+        .map(|i| (i as i64, ((i as i64) * 2654435761i64) % (n as i64 * 3)))
+        .collect();
+    check(&tasks, &[1, 1000, 1023, 1024, 1025, 2000, 2500, 3000]);
+}
+
+#[test]
+fn multi_peer_isolation() {
+    // Peer Y's tasks (higher priorities than any of X's) must never appear in
+    // an X-scoped claim, and must not affect X's ordering.
+    let mut ttl = String::new();
+    for (seq, priority) in [(0i64, 999i64), (1, 998), (2, 997)] {
+        ttl.push_str(&format!(
+            "<urn:task:Y:{seq}> ak:peer <urn:peer:Y> ; ak:status \"pending\" ; ak:priority {priority} ; ak:seq {seq} .\n"
+        ));
+    }
+    let x_tasks: Vec<(i64, i64)> = vec![(10, 1), (11, 5), (12, 3)];
+    let graph = build_graph("X", &x_tasks, &ttl);
+    let expected = expected_top_k(&graph, 10);
+    let actual = actual_top_k(&graph, 10);
+    assert_eq!(actual, expected);
+    assert_eq!(actual.len(), 3, "peer Y's tasks must not leak into an X-scoped claim");
+}
+
+#[test]
+fn status_filter_excludes_done_tasks() {
+    // A `done` task with the highest priority must never be claimed — exercises
+    // the fully-bound (`obj_const`) branch of the other-pattern join.
+    let mut ttl = String::from(
+        "<urn:task:X:999> ak:peer <urn:peer:X> ; ak:status \"done\" ; ak:priority 9999 ; ak:seq 999 .\n",
+    );
+    ttl.push_str("");
+    let tasks: Vec<(i64, i64)> = vec![(0, 1), (1, 2), (2, 3)];
+    let graph = build_graph("X", &tasks, &ttl);
+    let actual = actual_top_k(&graph, 10);
+    assert_eq!(actual, vec![2, 1, 0]); // DESC priority among the PENDING tasks only
+}
+
+#[test]
+fn fewer_pending_than_k_returns_all_sorted() {
+    let tasks: Vec<(i64, i64)> = vec![(0, 5), (1, 1), (2, 3)];
+    check(&tasks, &[3, 4, 100, 1024]);
+}
+
+#[test]
+fn empty_pending_queue() {
+    let graph = build_graph("X", &[], "");
+    let actual = actual_top_k(&graph, 5);
+    assert!(actual.is_empty());
+}
+
+#[test]
+fn ascending_order_direction() {
+    let tasks: Vec<(i64, i64)> = vec![(0, 5), (1, 20), (2, 1), (3, 15)];
+    let graph = build_graph("X", &tasks, "");
+    let r = query(
+        &graph,
+        &format!("{PFX} SELECT ?t WHERE {{ ?t ak:peer <urn:peer:X> ; ak:status \"pending\" ; ak:priority ?p ; ak:seq ?s . }} ORDER BY ASC(?p) LIMIT 2"),
+    )
+    .unwrap();
+    let actual: Vec<i64> = r.rows.iter().map(|row| task_seq(row[0].as_ref().unwrap())).collect();
+    assert_eq!(actual, vec![2, 0]); // priorities 1, 5 ascending
+}
+
+#[test]
+fn non_inline_priority_values_still_correct() {
+    // Priorities far outside the small-inline-integer range must still produce
+    // correct results — this must exercise the DECLINE guard (dict::is_inline)
+    // and fall back to the general path, not silently mis-sort.
+    let tasks: Vec<(i64, i64)> = vec![
+        (0, 9_000_000_000_000_000),
+        (1, 1_000_000_000_000_000),
+        (2, 5_000_000_000_000_000),
+    ];
+    check(&tasks, &[1, 2, 3]);
+}
+
+#[test]
+fn large_tie_group_still_correct() {
+    // Low priority cardinality (a handful of tiers over many tasks) forces a
+    // large tie-group — the MAX_INDEXED_GROUP cap should decline the fast path
+    // here and defer to the fallback. This test only asserts correctness (the
+    // regression this cap fixes was a PERFORMANCE regression, not a correctness
+    // one — verified separately via profiling, not asserted here to avoid a
+    // flaky timing-based test).
+    for tiers in [1usize, 2, 5, 16] {
+        let n = 600;
+        let tasks: Vec<(i64, i64)> = (0..n).map(|i| (i as i64, (i as i64) % tiers as i64)).collect();
+        check(&tasks, &[1, 2, 10, 100, n as usize]);
+    }
+}
+
+#[test]
+fn randomized_sweep() {
+    // A cheap xorshift so this test has no extra dependency and is fully
+    // deterministic (fixed seed) across runs, while still covering a wide
+    // spread of n / tie-density / k combinations in one pass.
+    fn xorshift(state: &mut u64) -> u64 {
+        *state ^= *state << 13;
+        *state ^= *state >> 7;
+        *state ^= *state << 17;
+        *state
+    }
+    let mut state: u64 = 0x9E3779B97F4A7C15;
+    for trial in 0..40 {
+        let n = 1 + (xorshift(&mut state) % 3000) as usize;
+        // Tie density: smaller modulus => more ties among priorities.
+        let priority_modulus = 1 + (xorshift(&mut state) % (n as u64 * 4 + 1));
+        let tasks: Vec<(i64, i64)> = (0..n)
+            .map(|i| (i as i64, (xorshift(&mut state) % priority_modulus) as i64))
+            .collect();
+        let graph = build_graph("X", &tasks, "");
+        let ks = [
+            1,
+            1 + (xorshift(&mut state) % (n as u64 + 3)) as usize,
+            n,
+            n + 5,
+        ];
+        for k in ks {
+            let expected = expected_top_k(&graph, k);
+            let actual = actual_top_k(&graph, k);
+            assert_eq!(actual, expected, "trial={trial} n={n} k={k} priority_modulus={priority_modulus}");
+        }
+    }
+}
+
+#[test]
+fn extra_pattern_beyond_star_shape_still_correct() {
+    // A second, unrelated variable-object pattern on the hub (not just the
+    // seed + status) — still a valid star shape, exercises multi-pattern join.
+    let mut ttl = String::from("@prefix ak: <http://example.org/ak#> .\n");
+    for (seq, priority, region) in [(0i64, 5i64, "us"), (1, 20, "eu"), (2, 1, "us")] {
+        ttl.push_str(&format!(
+            "<urn:task:X:{seq}> ak:peer <urn:peer:X> ; ak:status \"pending\" ; ak:priority {priority} ; ak:seq {seq} ; ak:region \"{region}\" .\n"
+        ));
+    }
+    let graph = Graph::load_str(&ttl, "turtle").unwrap();
+    let r = query(
+        &graph,
+        &format!("{PFX} SELECT ?t ?r WHERE {{ ?t ak:peer <urn:peer:X> ; ak:status \"pending\" ; ak:priority ?p ; ak:seq ?s ; ak:region ?r . }} ORDER BY DESC(?p) LIMIT 3"),
+    )
+    .unwrap();
+    let actual: Vec<i64> = r.rows.iter().map(|row| task_seq(row[0].as_ref().unwrap())).collect();
+    assert_eq!(actual, vec![1, 0, 2]);
+    // Region for the top result (task 1) must be "eu", proving the extra
+    // pattern's binding survived the join correctly.
+    if let oxrdf::Term::Literal(l) = r.rows[0][1].as_ref().unwrap() {
+        assert_eq!(l.value(), "eu");
+    } else {
+        panic!("expected a literal region");
+    }
+}
+
+#[test]
+fn large_already_claimed_prefix_still_correct() {
+    // A queue drained strictly in priority order leaves a large CONTIGUOUS
+    // prefix of already-claimed (status != "pending") tasks at the head of
+    // the priority-sorted scan -- this must still return correct results
+    // (regardless of the upfront-selectivity / cumulative-failure guards'
+    // exact thresholds) across a range of already-claimed prefix sizes.
+    let n = 1600;
+    for already_claimed in [0usize, 50, 100, 400, 800, 1200, 1599] {
+        let mut ttl = String::from("@prefix ak: <http://example.org/ak#> .\n");
+        for i in 0..n {
+            // priority = i, so "top `already_claimed`" = highest-numbered tasks.
+            let status = if i >= n - already_claimed { "in_progress" } else { "pending" };
+            ttl.push_str(&format!(
+                "<urn:task:{i}> ak:peer <urn:peer:X> ; ak:status \"{status}\" ; ak:priority {i} ; ak:seq {i} .\n"
+            ));
+        }
+        let graph = Graph::load_str(&ttl, "turtle").unwrap();
+        let expected = expected_top_k(&graph, 5);
+        let actual = actual_top_k(&graph, 5);
+        assert_eq!(actual, expected, "already_claimed={already_claimed}");
+    }
+}
+
+#[test]
+fn fast_path_actually_engages_not_just_correct() {
+    // Every other test in this file checks CORRECTNESS, which the fallback
+    // path (eval_modified + order_bindings) also satisfies on its own -- none
+    // of them would fail if try_topk_orderby_indexed were deleted entirely.
+    // This test checks that the fast path actually FIRES for the shape it
+    // exists for, using the engine's own EXPLAIN ANALYZE instrumentation.
+    //
+    // The "Plan:" section is a STATIC description of the general planner's
+    // choice and always names a "BGP [...]" step regardless of which path
+    // actually runs -- checking for its absence would be a vacuous assertion
+    // (confirmed by first writing this test with exactly that check: it
+    // failed even with the fast path correctly firing, because the static
+    // plan text matched). The real signal is the "Execution trace" section,
+    // which only reports a "BGP [binary GOO] (... patterns ...) rows=N" line
+    // when the general BGP evaluator was ACTUALLY EXECUTED (it materializes
+    // and reports the touched row count) -- the indexed fast path bypasses
+    // that evaluator entirely, so this line is present iff the fallback ran.
+    let n = 800;
+    let graph = build_graph("X", &(0..n as i64).map(|i| (i, i)).collect::<Vec<_>>(), "");
+    let explained = sparq_engine::explain_analyze(
+        &graph,
+        &format!("{PFX}{CLAIM_QUERY}1"),
+    )
+    .unwrap();
+    // [GPT-6 Astra] compact-index lacks the PSO scan needed by variable-object
+    // probes. Exercise its real decline instead of a zero-test feature pass.
+    let supported = sparq_core::store::BUILT.contains(&sparq_core::store::Perm::Pso);
+    assert_eq!(!explained.contains("BGP [binary GOO]"), supported,
+               "indexed engagement must match the built permutations:\n{explained}");
+}
```

## Critical unchanged caller/helper context

### crates/sparq-engine/src/exec.rs:2332-2366 — eval_select
Blob c0f63fb14243bc403092fb9146674f8dbb669c35 at 9e8bdfc95c916b62550fb8c3142f804bbbfb2444.

```rust
pub fn eval_select(graph: &Graph, pattern: &GraphPattern) -> Result<QueryResult, String> {
    let mut local = LocalVocab::default();
    let bindings = eval_modified(graph, &mut local, pattern)?;
    budget::check(bindings.rows.len())?;

    let out_vars: Vec<Variable> = bindings
        .vars
        .iter()
        .filter(|v| !v.as_str().starts_with(BNODE_VAR_PREFIX))
        .cloned()
        .collect();

    let col_of: Vec<Option<usize>> = out_vars.iter().map(|v| bindings.col(v)).collect();
    let materialise = |row: &Row| -> Vec<Option<Term>> {
        col_of.iter().map(|c| c.and_then(|i| term_of(graph, &local, row[i]))).collect()
    };
    #[cfg(feature = "parallel")]
    let rows: Vec<Vec<Option<Term>>> = if bindings.rows.len() >= PAR_THRESHOLD {
        use rayon::prelude::*;
        bindings.rows.par_iter().map(materialise).collect()
    } else {
        bindings.rows.iter().map(materialise).collect()
    };
    #[cfg(not(feature = "parallel"))]
    let rows: Vec<Vec<Option<Term>>> = bindings.rows.iter().map(materialise).collect();
    Ok(QueryResult { vars: out_vars, rows })
}
```

### crates/sparq-engine/src/exec.rs:3012-3108 — eval_modified
Blob c0f63fb14243bc403092fb9146674f8dbb669c35 at 9e8bdfc95c916b62550fb8c3142f804bbbfb2444.

```rust
fn eval_modified(graph: &Graph, local: &mut LocalVocab, p: &GraphPattern) -> Result<Bindings, String> {
    #[cfg(not(target_arch = "wasm32"))]
    let _query_now = query_now::scope();
    match p {
        GraphPattern::Project { inner, variables } => {
            let b = eval_modified(graph, local, inner)?;
            Ok(project_bindings(b, variables))
        }
        GraphPattern::Distinct { inner } => {
            #[cfg(feature = "zk")]
            let _zk = crate::zk::op_scope(crate::zk::Op::Distinct);
            if distinct_pushdown::enabled() {
                if let Some(b) = try_distinct_pushdown(graph, local, inner)? {
                    return Ok(b);
                }
            }
            let mut b = eval_modified(graph, local, inner)?;
            distinct_bindings(&mut b);
            Ok(b)
        }
        GraphPattern::Reduced { inner } => eval_modified(graph, local, inner),
        GraphPattern::Slice { inner, start, length } => {
            if let Some(len) = length {
                if let Some(cap) = start.checked_add(*len) {
                    if let Some(mut b) = try_capped(graph, local, inner, cap)? {
                        slice_bindings(&mut b, *start, *length);
                        return Ok(b);
                    }
                    if cap <= TOP_K_ORDER_BY_THRESHOLD {
                        if let Some(mut b) = try_topk_orderby(graph, local, inner, cap)? {
                            slice_bindings(&mut b, *start, *length);
                            return Ok(b);
                        }
                    }
                }
            }
            let mut b = eval_modified(graph, local, inner)?;
            slice_bindings(&mut b, *start, *length);
            Ok(b)
        }
        GraphPattern::OrderBy { inner, expression } => {
            let mut b = eval_modified(graph, local, inner)?;
            order_bindings(graph, local, &mut b, expression, None)?;
            Ok(b)
        }
        GraphPattern::Group { inner, variables, aggregates } => {
            #[cfg(feature = "zk")]
            let _zk = crate::zk::op_scope(crate::zk::Op::Group);
            if variables.is_empty() && aggregates.len() == 1 {
                if let (av, AggregateExpression::CountSolutions { distinct: false }) = (&aggregates[0].0, &aggregates[0].1) {
                    if let Some(n) = try_count(graph, inner) {
                        let id = value_to_id(graph, local, &Value::Num(Num::Int(n as i64)));
                        let row: Row = std::iter::once(id).collect();
                        return Ok(Bindings { vars: vec![av.clone()], rows: vec![row], sorted_by: None });
                    }
                }
            }
            let b = eval_graph_pattern(graph, local, inner)?;
            group_aggregate(graph, local, b, variables, aggregates)
        }
        other => eval_graph_pattern(graph, local, other),
    }
}
```

### crates/sparq-engine/src/exec.rs:3119-3156 — try_topk_orderby
Blob c0f63fb14243bc403092fb9146674f8dbb669c35 at 9e8bdfc95c916b62550fb8c3142f804bbbfb2444.

```rust
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
            if let Some(b) = try_topk_orderby_indexed(graph, local, ord_inner, expression, row_budget)? {
                return Ok(Some(b));
            }
            let mut b = eval_modified(graph, local, ord_inner)?;
            let use_topk = b.rows.len() > row_budget;
            order_bindings(graph, local, &mut b, expression, if use_topk { Some(row_budget) } else { None })?;
            Ok(Some(b))
        }
        _ => Ok(None),
    }
}
```

### crates/sparq-engine/src/exec.rs:3845-3864 — has_intra_triple_repeated_var
Blob c0f63fb14243bc403092fb9146674f8dbb669c35 at 9e8bdfc95c916b62550fb8c3142f804bbbfb2444.

```rust
fn has_intra_triple_repeated_var(tp: &TriplePattern) -> bool {
    let sv = if let TermPattern::Variable(v) = &tp.subject {
        Some(v)
    } else {
        None
    };
    let pv = if let NamedNodePattern::Variable(v) = &tp.predicate {
        Some(v)
    } else {
        None
    };
    let ov = if let TermPattern::Variable(v) = &tp.object {
        Some(v)
    } else {
        None
    };
    (sv.is_some() && pv.is_some() && sv == pv)
        || (sv.is_some() && ov.is_some() && sv == ov)
        || (pv.is_some() && ov.is_some() && pv == ov)
}
```

### crates/sparq-engine/src/exec.rs:6286-6305 — is_conjunctive
Blob c0f63fb14243bc403092fb9146674f8dbb669c35 at 9e8bdfc95c916b62550fb8c3142f804bbbfb2444.

```rust
pub(crate) fn is_conjunctive(p: &GraphPattern) -> bool {
    match p {
        GraphPattern::Bgp { .. } => true,
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
```

### crates/sparq-engine/src/exec.rs:6346-6359 — flatten_conjunction
Blob c0f63fb14243bc403092fb9146674f8dbb669c35 at 9e8bdfc95c916b62550fb8c3142f804bbbfb2444.

```rust
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
```

### crates/sparq-engine/src/exec.rs:8595-8605 — collect_vars
Blob c0f63fb14243bc403092fb9146674f8dbb669c35 at 9e8bdfc95c916b62550fb8c3142f804bbbfb2444.

```rust
pub(crate) fn collect_vars(patterns: &[TriplePattern]) -> Vec<Variable> {
    let mut vars = Vec::new();
    for tp in patterns {
        for v in [tp_var(&tp.subject), nnp_var(&tp.predicate), tp_var(&tp.object)].into_iter().flatten() {
            if !vars.contains(&v) {
                vars.push(v);
            }
        }
    }
    vars
}
```

### crates/sparq-engine/src/exec.rs:8844-8873 — prepare_pattern
Blob c0f63fb14243bc403092fb9146674f8dbb669c35 at 9e8bdfc95c916b62550fb8c3142f804bbbfb2444.

```rust
fn prepare_pattern(graph: &Graph, tp: &TriplePattern) -> Result<(IdPattern, [Option<Variable>; 3], bool), String> {
    let mut id_pat: IdPattern = [None, None, None];
    let mut pos_vars: [Option<Variable>; 3] = [None, None, None];
    let mut unsat = false;

    let mut bind_term = |slot: usize, tp: &TermPattern| -> Result<(), String> {
        match tp {
            TermPattern::Variable(v) => pos_vars[slot] = Some(v.clone()),
            TermPattern::BlankNode(b) => pos_vars[slot] = Some(bnode_var(b)),
            other => match graph.id_of(&term_pattern_to_term(other)?) {
                Some(id) => id_pat[slot] = Some(id),
                None => unsat = true,
            },
        }
        Ok(())
    };
    bind_term(0, &tp.subject)?;
    bind_term(2, &tp.object)?;

    match &tp.predicate {
        NamedNodePattern::Variable(v) => pos_vars[1] = Some(v.clone()),
        NamedNodePattern::NamedNode(n) => match graph.id_of(&Term::NamedNode(n.clone())) {
            Some(id) => id_pat[1] = Some(id),
            None => unsat = true,
        },
    }
    Ok((id_pat, pos_vars, unsat))
}
```

### crates/sparq-engine/src/exec.rs:8875-9008 — scan_to_bindings
Blob c0f63fb14243bc403092fb9146674f8dbb669c35 at 9e8bdfc95c916b62550fb8c3142f804bbbfb2444.

```rust
fn scan_to_bindings(
    graph: &Graph,
    id_pat: &IdPattern,
    pos_vars: &[Option<Variable>; 3],
    sort_col: Option<usize>,
    filter: Option<(usize, ScanCmp)>,
    limit: Option<usize>,
    #[cfg(feature = "semijoin-bitmap")] prefilter: Option<(usize, &crate::semijoin::KeyFilter)>,
) -> Bindings {
    let mut vars: Vec<Variable> = Vec::new();
    let mut var_positions: Vec<Vec<usize>> = Vec::new();
    for (pos, v) in pos_vars.iter().enumerate() {
        if let Some(v) = v {
            if let Some(idx) = vars.iter().position(|x| x == v) {
                var_positions[idx].push(pos);
            } else {
                vars.push(v.clone());
                var_positions.push(vec![pos]);
            }
        }
    }
    let scan = match sort_col {
        Some(c) => graph.store.scan_sorted(id_pat, c),
        None => graph.store.scan(id_pat),
    };
    let actual_sort = scan.perm.order().into_iter().find(|&c| id_pat[c].is_none());
    let sorted_by = actual_sort.and_then(|c| pos_vars[c].clone());

    let mut scan_rows: &[[Id; 3]] = scan.rows.as_ref();
    if let Some((fpos, cmp)) = filter {
        if actual_sort == Some(fpos) && scan_rows.first().is_some_and(|r| dict::is_inline(scan.to_spo(r)[fpos])) {
            scan_rows = match inline_pass_values(cmp) {
                Some((lo, hi)) => {
                    let (lo_id, hi_id) = (dict::INLINE_BASE + lo, dict::INLINE_BASE + hi);
                    let start = scan_rows.partition_point(|r| scan.to_spo(r)[fpos] < lo_id);
                    let end = scan_rows.partition_point(|r| scan.to_spo(r)[fpos] <= hi_id);
                    &scan_rows[start..end]
                }
                None => &[],
            };
        }
    }

    let build_row = |row: &[Id; 3]| -> Option<Row> {
        let spo = scan.to_spo(row);
        #[cfg(feature = "semijoin-bitmap")]
        if let Some((jpos, kf)) = prefilter {
            if !kf.contains(spo[jpos]) {
                return None;
            }
        }
        if let Some((fpos, cmp)) = filter {
            if !cmp.test_id(graph, spo[fpos]) {
                return None;
            }
        }
        let mut out = Row::with_capacity(vars.len());
        for positions in &var_positions {
            let v0 = spo[positions[0]];
            if positions.iter().any(|&p| spo[p] != v0) {
                return None;
            }
            out.push(v0);
        }
        Some(out)
    };

    #[cfg(feature = "zk")]
    if crate::zk::enabled() {
        let kept: Vec<[Id; 3]> = scan_rows
            .iter()
            .filter(|r| build_row(r).is_some())
            .map(|r| scan.to_spo(r))
            .collect();
        crate::zk::record_scan_ids(graph, id_pat, pos_vars, &kept, false);
    }

    #[cfg(feature = "parallel")]
    if limit.is_none() && scan_rows.len() >= PAR_THRESHOLD {
        use rayon::prelude::*;
        let rows: Vec<Row> = scan_rows.par_iter().filter_map(build_row).collect();
        return Bindings { vars, rows, sorted_by };
    }

    let cap = limit.map_or(scan_rows.len(), |n| n.min(scan_rows.len()));
    let mut rows: Vec<Row> = Vec::with_capacity(budget::cap_alloc(cap));
    for (i, row) in scan_rows.iter().enumerate() {
        if i & 4095 == 0 && budget::exhausted(rows.len()) {
            break;
        }
        if let Some(out) = build_row(row) {
            rows.push(out);
            if let Some(n) = limit {
                if rows.len() >= n {
                    break;
                }
            }
        }
    }
    Bindings { vars, rows, sorted_by }
}
```

### crates/sparq-engine/src/exec.rs:4991-4998 — eval_graph_named
Blob c0f63fb14243bc403092fb9146674f8dbb669c35 at 9e8bdfc95c916b62550fb8c3142f804bbbfb2444.

```rust
fn eval_graph_named(
    graph: &Graph,
    local: &mut LocalVocab,
    name: &NamedNodePattern,
    inner: &GraphPattern,
) -> Result<Bindings, String> {
    eval_graph_named_pref(graph, local, name, inner, None)
}
```

### crates/sparq-engine/src/exec.rs:5049-5233 — eval_graph_named_pref
Blob c0f63fb14243bc403092fb9146674f8dbb669c35 at 9e8bdfc95c916b62550fb8c3142f804bbbfb2444.

```rust
fn eval_graph_named_pref(
    graph: &Graph,
    local: &mut LocalVocab,
    name: &NamedNodePattern,
    inner: &GraphPattern,
    prefix: Option<&str>,
) -> Result<Bindings, String> {
    fn eval_translated(
        graph: &Graph,
        local: &mut LocalVocab,
        sub: &Graph,
        #[cfg(feature = "zk")] gname: &Term,
        inner: &GraphPattern,
    ) -> Result<Bindings, String> {
        let _scope = view::enter_graph();
        #[cfg(feature = "zk")]
        let _zk = crate::zk::graph_scope(gname);
        let mut sub_local = LocalVocab::default();
        let b = eval_graph_pattern(sub, &mut sub_local, inner)?;
        let rows: Vec<Row> = b
            .rows
            .iter()
            .map(|r| {
                r.iter()
                    .map(|&id| match term_of(sub, &sub_local, id) {
                        Some(t) => value_to_id(graph, local, &Value::Term(t)),
                        None => NO_ID,
                    })
                    .collect()
            })
            .collect();
        Ok(Bindings::unsorted(b.vars, rows))
    }
    match name {
        NamedNodePattern::NamedNode(n) => {
            let target = Term::NamedNode(n.clone());
            let sub = if view::allows(&target) {
                graph.named.iter().find(|(t, _)| *t == target).map(|(_, sub)| sub)
            } else {
                None
            };
            match sub {
                Some(sub) => eval_translated(
                    graph,
                    local,
                    sub,
                    #[cfg(feature = "zk")]
                    &target,
                    inner,
                ),
                None => {
                    let _scope = view::enter_graph(); // schema eval matches the present-graph path
                    #[cfg(feature = "zk")]
                    let _zk = crate::zk::graph_scope(&target);
                    let empty = Graph::load_str("", "ntriples").map_err(|e| e.to_string())?;
                    let mut el = LocalVocab::default();
                    let mut b = eval_graph_pattern(&empty, &mut el, inner)?;
                    b.rows.clear();
                    Ok(b)
                }
            }
        }
        NamedNodePattern::Variable(v) => {
            let mut out_vars: Option<Vec<Variable>> = None;
            let mut out_rows: Vec<Row> = Vec::new();
            let mut per_graph = |graph: &Graph, local: &mut LocalVocab, gname: &Term, sub: &Graph| -> Result<(), String> {
                let mut b = eval_translated(
                    graph,
                    local,
                    sub,
                    #[cfg(feature = "zk")]
                    gname,
                    inner,
                )?;
                let gid = value_to_id(graph, local, &Value::Term(gname.clone()));
                let schema = out_vars.get_or_insert_with(|| {
                    let mut s = Vec::with_capacity(b.vars.len() + 1);
                    s.push(v.clone());
                    for var in &b.vars {
                        if var != v {
                            s.push(var.clone());
                        }
                    }
                    s
                });
                match b.col(v) {
                    Some(c) => {
                        b.rows.retain_mut(|row| {
                            if row[c] == NO_ID {
                                row[c] = gid;
                                true
                            } else {
                                row[c] == gid
                            }
                        });
                    }
                    None => {
                        b.vars.insert(0, v.clone());
                        for row in &mut b.rows {
                            row.insert(0, gid);
                        }
                    }
                }
                let col_map: Vec<Option<usize>> = schema
                    .iter()
                    .map(|var| b.vars.iter().position(|x| x == var))
                    .collect();
                for row in &b.rows {
                    out_rows.push(
                        col_map
                            .iter()
                            .map(|&pos| pos.map(|i| row[i]).unwrap_or(NO_ID))
                            .collect(),
                    );
                }
                Ok(())
            };
            match prefix {
                Some(pref) => {
                    let mut err: Option<String> = None;
                    graph.for_named_graphs_with_prefix(pref, |gname, sub| {
                        if err.is_some() || !view::allows(gname) {
                            return;
                        }
                        if let Err(e) = per_graph(graph, local, gname, sub) {
                            err = Some(e);
                        }
                    });
                    if let Some(e) = err {
                        return Err(e);
                    }
                }
                None => {
                    for (gname, sub) in &graph.named {
                        if !view::allows(gname) {
                            continue; // not visible under the installed dataset view (L1)
                        }
                        per_graph(graph, local, gname, sub)?;
                    }
                }
            }
            let vars = out_vars.unwrap_or_else(|| vec![v.clone()]);
            Ok(Bindings::unsorted(vars, out_rows))
        }
    }
}
```

### crates/sparq-engine/src/exec.rs:11683-11936 — order_bindings
Blob c0f63fb14243bc403092fb9146674f8dbb669c35 at 9e8bdfc95c916b62550fb8c3142f804bbbfb2444.

```rust
fn order_bindings(
    graph: &Graph,
    local: &LocalVocab,
    b: &mut Bindings,
    exprs: &[OrderExpression],
    row_budget: Option<usize>,
) -> Result<(), String> {
    let compiled_order: Vec<(bool, CompiledExpr)> = exprs
        .iter()
        .map(|oe| match oe {
            OrderExpression::Asc(e) => (false, compile_expr(e, b)),
            OrderExpression::Desc(e) => (true, compile_expr(e, b)),
        })
        .collect();

    let cell_of = |row: &Row, e: &CompiledExpr| -> Result<SortCell, String> {
        if let CompiledExpr::Var(Some(c)) = e {
            let id = row[*c];
            if id != NO_ID && !is_local(id) {
                if let Some(n) = graph.numeric_value(id) {
                    return Ok(SortCell::Num { f: n, id });
                }
                if let Some(t) = graph.temporal_value(id) {
                    return Ok(SortCell::Temp { t, id });
                }
                #[cfg(feature = "topk-lazy-strkey")]
                if graph.dict.plain_string_value(id).is_some() {
                    return Ok(SortCell::StrId(id));
                }
                let term = term_of(graph, local, id).expect("bound id resolves");
                if let Term::NamedNode(n) = &term {
                    return Ok(SortCell::Iri(n.as_str().into()));
                }
                return Ok(sort_cell_val(Value::Term(term)));
            }
        }
        Ok(match eval_compiled_numeric(graph, local, row, e) {
            Some(n) => sort_cell_val(Value::Num(Num::Double(n))),
            None => sort_cell_val(eval_compiled(graph, local, b, row, e)?),
        })
    };
    let key_of = |row: &Row| -> Result<Vec<(bool, SortCell)>, String> {
        let mut key = Vec::with_capacity(compiled_order.len());
        for (desc, ce) in &compiled_order {
            key.push((*desc, cell_of(row, ce)?));
        }
        Ok(key)
    };

    let n = b.rows.len();

    if let Some(k) = row_budget {
        if k == 0 {
            b.rows.clear();
            b.sorted_by = None;
            return Ok(());
        }
        if k < n {
            #[cfg(feature = "parallel")]
            let mut keyed: Vec<_> = if n >= PAR_THRESHOLD {
                use rayon::prelude::*;
                let fns = functions::snapshot();
                let vw = view::snapshot();
                let spx = spatial::snapshot();
                #[cfg(feature = "service-local")]
                let lsv = local_services::snapshot();
                #[cfg(not(target_arch = "wasm32"))]
                let qn = query_now::snapshot(); // sq-98w7z.1: keep NOW() pinned on workers
                b.rows
                    .par_iter()
                    .enumerate()
                    .map(|(i, row)| {
                        let _fns = functions::worker_install(&fns);
                        let _vw = view::worker_install(&vw);
                        let _spx = spatial::worker_install(&spx);
                        #[cfg(feature = "service-local")]
                        let _lsv = local_services::worker_install(&lsv);
                        #[cfg(not(target_arch = "wasm32"))]
                        let _qn = query_now::worker_install(qn);
                        Ok((key_of(row)?, i))
                    })
                    .collect::<Result<Vec<_>, String>>()?
            } else {
                b.rows
                    .iter()
                    .enumerate()
                    .map(|(i, row)| Ok((key_of(row)?, i)))
                    .collect::<Result<Vec<_>, String>>()?
            };
            #[cfg(not(feature = "parallel"))]
            let mut keyed: Vec<_> = b
                .rows
                .iter()
                .enumerate()
                .map(|(i, row)| Ok((key_of(row)?, i)))
                .collect::<Result<_, String>>()?;

            let cmp_total = |a: &(Vec<(bool, SortCell)>, usize), c: &(Vec<(bool, SortCell)>, usize)| {
                for ((desc, av), (_, cv)) in a.0.iter().zip(c.0.iter()) {
                    let ord = cmp_sort_cells(graph, local, av, cv);
                    let ord = if *desc { ord.reverse() } else { ord };
                    if ord != Ordering::Equal {
                        return ord;
                    }
                }
                a.1.cmp(&c.1)
            };

            debug_assert!(k > 0 && k <= keyed.len());
            keyed.select_nth_unstable_by(k - 1, |a, c| cmp_total(a, c));

            keyed[..k].sort_by(|a, c| cmp_total(a, c));

            b.rows = keyed[..k].iter().map(|(_, i)| b.rows[*i].clone()).collect();
            b.sorted_by = None;
            return Ok(());
        }
    }

    #[cfg(feature = "parallel")]
    let mut keyed: Vec<(Vec<(bool, SortCell)>, Row)> = if n >= PAR_THRESHOLD {
        use rayon::prelude::*;
        let fns = functions::snapshot();
        let vw = view::snapshot();
        let spx = spatial::snapshot();
        #[cfg(feature = "service-local")]
        let lsv = local_services::snapshot();
        #[cfg(not(target_arch = "wasm32"))]
        let qn = query_now::snapshot(); // sq-98w7z.1: keep NOW() pinned on workers
        b.rows
            .par_iter()
            .map(|row| {
                let _fns = functions::worker_install(&fns);
                let _vw = view::worker_install(&vw);
                let _spx = spatial::worker_install(&spx);
                #[cfg(feature = "service-local")]
                let _lsv = local_services::worker_install(&lsv);
                #[cfg(not(target_arch = "wasm32"))]
                let _qn = query_now::worker_install(qn);
                Ok((key_of(row)?, row.clone()))
            })
            .collect::<Result<_, String>>()?
    } else {
        b.rows.iter().map(|row| Ok((key_of(row)?, row.clone()))).collect::<Result<_, String>>()?
    };
    #[cfg(not(feature = "parallel"))]
    let mut keyed: Vec<(Vec<(bool, SortCell)>, Row)> =
        b.rows.iter().map(|row| Ok((key_of(row)?, row.clone()))).collect::<Result<_, String>>()?;

    let cmp = |a: &(Vec<(bool, SortCell)>, Row), c: &(Vec<(bool, SortCell)>, Row)| {
        for ((desc, av), (_, cv)) in a.0.iter().zip(c.0.iter()) {
            let ord = cmp_sort_cells(graph, local, av, cv);
            let ord = if *desc { ord.reverse() } else { ord };
            if ord != Ordering::Equal {
                return ord;
            }
        }
        Ordering::Equal
    };
    #[cfg(feature = "parallel")]
    if keyed.len() >= PAR_THRESHOLD {
        use rayon::prelude::*;
        keyed.par_sort_by(cmp);
    } else {
        keyed.sort_by(cmp);
    }
    #[cfg(not(feature = "parallel"))]
    keyed.sort_by(cmp);

    b.rows = keyed.into_iter().map(|(_, r)| r).collect();
    b.sorted_by = None;
    Ok(())
}
```

### crates/sparq-engine/src/exec.rs:68-476 — budget production portion (unchanged test modules omitted)
Blob c0f63fb14243bc403092fb9146674f8dbb669c35 at 9e8bdfc95c916b62550fb8c3142f804bbbfb2444.

```rust
pub(crate) mod budget {
    use crate::QueryBudget;
    use sparq_core::dict::Id;
    use std::cell::Cell;
    use std::ptr::NonNull;
    use std::sync::atomic::{AtomicBool, Ordering};

    #[derive(Clone, Copy)]
    struct CancelPtr(NonNull<AtomicBool>);

    unsafe impl Send for CancelPtr {}
    unsafe impl Sync for CancelPtr {}

    pub(crate) const BYTES_PER_ID: usize = std::mem::size_of::<Id>();

    #[derive(Clone, Copy)]
    pub(crate) struct Limits {
        on: bool,
        #[cfg(not(target_arch = "wasm32"))]
        deadline: Option<std::time::Instant>,
        max_rows: usize,
        max_bytes: usize,
        byte_width: usize,
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
        #[inline]
        fn bytes(&self, rows: usize) -> usize {
            rows.saturating_mul(self.byte_width).saturating_add(self.extra_bytes)
        }

        #[cfg_attr(not(feature = "parallel"), allow(dead_code))]
        #[inline]
        pub(crate) fn why(&self, rows: usize) -> Option<&'static str> {
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
            if self.deadline.is_some_and(|d| std::time::Instant::now() >= d) {
                return Some("timeout");
            }
            if let Some(cancel) = self.cancel {
                if unsafe { cancel.0.as_ref() }.load(Ordering::Relaxed) {
                    return Some("cancelled");
                }
            }
            None
        }

        #[cfg_attr(not(feature = "parallel"), allow(dead_code))]
        #[inline]
        pub(crate) fn hit(&self, rows: usize) -> bool {
            self.why(rows).is_some()
        }
    }

    thread_local! {
        static ACTIVE: Cell<Limits> = const { Cell::new(OFF) };
        static EXCEEDED: Cell<Option<&'static str>> = const { Cell::new(None) };
    }

    pub(crate) struct Guard<'a> {
        _budget: std::marker::PhantomData<&'a QueryBudget>,
        _not_send: std::marker::PhantomData<std::rc::Rc<()>>,
    }
    impl Drop for Guard<'_> {
        fn drop(&mut self) {
            ACTIVE.with(|a| a.set(OFF));
            EXCEEDED.with(|e| e.set(None));
        }
    }

    pub(crate) fn install(b: &QueryBudget) -> Guard<'_> {
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
        ACTIVE.with(|a| {
            a.set(Limits {
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
        EXCEEDED.with(|e| e.set(None));
        Guard {
            _budget: std::marker::PhantomData,
            _not_send: std::marker::PhantomData,
        }
    }

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

    #[cfg_attr(not(feature = "parallel"), allow(dead_code))]
    #[inline]
    pub(crate) fn snapshot() -> Limits {
        ACTIVE.with(|a| a.get())
    }

    #[cfg(feature = "parallel")]
    #[inline]
    pub(crate) fn parallel_json_fanout() -> Option<Limits> {
        ACTIVE.with(|a| {
            let l = a.get();
            if l.on && (l.max_rows != usize::MAX || l.max_bytes != usize::MAX) {
                None // a row/byte cap the fan-out cannot enforce mid-serialize → serial loop
            } else {
                Some(l)
            }
        })
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[cfg_attr(not(feature = "service"), allow(dead_code))]
    #[inline]
    pub(crate) fn remaining_timeout() -> Option<std::time::Duration> {
        ACTIVE.with(|a| {
            let lim = a.get();
            lim.deadline.map(|d| d.saturating_duration_since(std::time::Instant::now()))
        })
    }

    #[cfg(any(feature = "service", feature = "service-local"))]
    #[derive(Clone, Copy)]
    pub(crate) struct ByteSavepoint {
        extra_bytes: usize,
        exceeded: Option<&'static str>,
    }

    #[cfg(any(feature = "service", feature = "service-local"))]
    #[inline]
    pub(crate) fn byte_savepoint() -> ByteSavepoint {
        ByteSavepoint {
            extra_bytes: ACTIVE.with(|c| c.get().extra_bytes),
            exceeded: EXCEEDED.with(|e| e.get()),
        }
    }

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
            if unsafe { cancel.0.as_ref() }.load(Ordering::Relaxed) {
                EXCEEDED.with(|e| e.set(Some("cancelled")));
                return true;
            }
        }
        false
    }

    #[inline]
    pub(crate) fn check(rows: usize) -> Result<(), String> {
        if exhausted(rows) {
            let why = EXCEEDED.with(|e| e.get()).unwrap_or("timeout");
            return Err(format!("query budget exceeded ({why})"));
        }
        Ok(())
    }

    #[cfg_attr(not(feature = "vectorized"), allow(dead_code))]
    #[inline]
    pub(crate) fn active() -> bool {
        ACTIVE.with(|c| c.get().on)
    }

    #[inline]
    pub(crate) fn cap_alloc(cap: usize) -> usize {
        let a = ACTIVE.with(|c| c.get());
        if !a.on {
            return cap;
        }
        let by_bytes = a
            .max_bytes
            .saturating_sub(a.extra_bytes)
            .checked_div(a.byte_width.max(1))
            .unwrap_or(usize::MAX)
            .saturating_add(1);
        cap.min(a.max_rows.saturating_add(1)).min(by_bytes).min(1 << 20)
    }

```

### crates/sparq-engine/src/exec.rs:2021-2129 — view
Blob c0f63fb14243bc403092fb9146674f8dbb669c35 at 9e8bdfc95c916b62550fb8c3142f804bbbfb2444.

```rust
pub(crate) mod view {
    use crate::{DatasetView, DefaultGraphMode};
    use oxrdf::Term;
    use rustc_hash::FxHashSet;
    use std::cell::RefCell;
    use std::sync::Arc;

    #[derive(Clone, Default)]
    pub(crate) struct State {
        named: Option<Arc<FxHashSet<Term>>>,
        default_empty: bool,
        suspended: bool,
    }

    thread_local! {
        static ACTIVE: RefCell<State> = RefCell::new(State::default());
    }

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

    pub(crate) fn suspend_all() -> Guard {
        Guard(ACTIVE.with(|a| std::mem::take(&mut *a.borrow_mut())))
    }

    pub(crate) struct GraphScope(bool);
    impl Drop for GraphScope {
        fn drop(&mut self) {
            ACTIVE.with(|a| a.borrow_mut().suspended = self.0);
        }
    }

    pub(crate) fn enter_graph() -> GraphScope {
        GraphScope(ACTIVE.with(|a| std::mem::replace(&mut a.borrow_mut().suspended, true)))
    }

    #[inline]
    pub(crate) fn allows(name: &Term) -> bool {
        ACTIVE.with(|a| a.borrow().named.as_ref().is_none_or(|s| s.contains(name)))
    }

    #[inline]
    pub(crate) fn default_is_empty() -> bool {
        ACTIVE.with(|a| {
            let s = a.borrow();
            s.default_empty && !s.suspended
        })
    }

    #[cfg_attr(not(feature = "parallel"), allow(dead_code))]
    pub(crate) fn snapshot() -> Option<State> {
        ACTIVE.with(|a| {
            let s = a.borrow();
            (s.named.is_some() || s.default_empty).then(|| s.clone())
        })
    }

    pub(crate) struct WorkerGuard(Option<State>);
    impl Drop for WorkerGuard {
        fn drop(&mut self) {
            if let Some(prev) = self.0.take() {
                ACTIVE.with(|a| *a.borrow_mut() = prev);
            }
        }
    }

    #[cfg_attr(not(feature = "parallel"), allow(dead_code))]
    pub(crate) fn worker_install(snap: &Option<State>) -> WorkerGuard {
        match snap {
            None => WorkerGuard(None),
            Some(s) => WorkerGuard(Some(ACTIVE.with(|a| std::mem::replace(&mut *a.borrow_mut(), s.clone())))),
        }
    }
}
```

### crates/sparq-engine/src/exec.rs:11312-11365 — SortCell
Blob c0f63fb14243bc403092fb9146674f8dbb669c35 at 9e8bdfc95c916b62550fb8c3142f804bbbfb2444.

```rust
enum SortCell {
    Temp { t: Temporal, id: Id },
    Num { f: f64, id: Id },
    Iri(Box<str>),
    #[cfg(feature = "topk-lazy-strkey")]
    StrId(Id),
    Val { class: u8, kind: u8, key: Option<Box<str>>, v: Value },
}
```

### crates/sparq-engine/src/exec.rs:11374-11401 — sort_cell_val
Blob c0f63fb14243bc403092fb9146674f8dbb669c35 at 9e8bdfc95c916b62550fb8c3142f804bbbfb2444.

```rust
fn sort_cell_val(v: Value) -> SortCell {
    let class = v.term_class() as u8;
    let kind = if class == TermClass::Literal as u8 { v.literal_kind() as u8 } else { 0 };
    let key = if class == TermClass::Blank as u8
        || (class == TermClass::Literal as u8
            && (kind == LiteralKind::String as u8
                || kind == LiteralKind::Lang as u8
                || kind == LiteralKind::Other as u8))
    {
        value_str(&v).map(String::into_boxed_str)
    } else {
        None
    };
    SortCell::Val { class, kind, key, v }
}
```

### crates/sparq-engine/src/exec.rs:11410-11550 — cmp_sort_cells
Blob c0f63fb14243bc403092fb9146674f8dbb669c35 at 9e8bdfc95c916b62550fb8c3142f804bbbfb2444.

```rust
fn cmp_sort_cells(graph: &Graph, local: &LocalVocab, a: &SortCell, c: &SortCell) -> Ordering {
    match (a, c) {
        (SortCell::Temp { t: ta, .. }, SortCell::Temp { t: tb, .. }) => {
            Temporal::cmp_t_total(*ta, *tb)
        }
        (SortCell::Num { f: fa, id: ia }, SortCell::Num { f: fb, id: ib }) => {
            cmp_sort_num(graph, *fa, *ia, *fb, *ib)
        }
        (SortCell::Temp { id, .. }, SortCell::Val { v, .. }) => {
            compare_values(&sort_cell_term(graph, local, *id), v).unwrap_or(Ordering::Equal)
        }
        (SortCell::Val { v, .. }, SortCell::Temp { id, .. }) => {
            compare_values(v, &sort_cell_term(graph, local, *id)).unwrap_or(Ordering::Equal)
        }
        (SortCell::Num { f, .. }, SortCell::Temp { id, .. }) => {
            compare_values(&Value::Num(Num::Double(*f)), &sort_cell_term(graph, local, *id)).unwrap_or(Ordering::Equal)
        }
        (SortCell::Temp { id, .. }, SortCell::Num { f, .. }) => {
            compare_values(&sort_cell_term(graph, local, *id), &Value::Num(Num::Double(*f))).unwrap_or(Ordering::Equal)
        }
        (SortCell::Num { f, .. }, SortCell::Val { v, .. }) => {
            compare_values(&Value::Num(Num::Double(*f)), v).unwrap_or(Ordering::Equal)
        }
        (SortCell::Val { v, .. }, SortCell::Num { f, .. }) => {
            compare_values(v, &Value::Num(Num::Double(*f))).unwrap_or(Ordering::Equal)
        }
        (SortCell::Iri(a), SortCell::Iri(b)) => a.as_ref().cmp(b.as_ref()),
        (SortCell::Iri(_), SortCell::Num { .. }) => Ordering::Less,
        (SortCell::Num { .. }, SortCell::Iri(_)) => Ordering::Greater,
        (SortCell::Iri(_), SortCell::Temp { .. }) => Ordering::Less,
        (SortCell::Temp { .. }, SortCell::Iri(_)) => Ordering::Greater,
        (SortCell::Iri(a), SortCell::Val { v, .. }) => match v {
            Value::Unbound | Value::Error => Ordering::Greater, // IRI after unbound
            Value::Term(Term::BlankNode(_)) => Ordering::Greater, // IRI after blank node
            Value::Term(Term::NamedNode(n)) => a.as_ref().cmp(n.as_str()), // same class
            _ => Ordering::Less, // IRI before literals and triple terms
        },
        (SortCell::Val { v, .. }, SortCell::Iri(a)) => match v {
            Value::Unbound | Value::Error => Ordering::Less,
            Value::Term(Term::BlankNode(_)) => Ordering::Less,
            Value::Term(Term::NamedNode(n)) => n.as_str().cmp(a.as_ref()),
            _ => Ordering::Greater,
        },
        #[cfg(feature = "topk-lazy-strkey")]
        (SortCell::StrId(a), SortCell::StrId(b)) => {
            str_id_value(graph, *a).cmp(str_id_value(graph, *b))
        }
        #[cfg(feature = "topk-lazy-strkey")]
        (SortCell::StrId(_), SortCell::Iri(_)) => Ordering::Greater,
        #[cfg(feature = "topk-lazy-strkey")]
        (SortCell::Iri(_), SortCell::StrId(_)) => Ordering::Less,
        #[cfg(feature = "topk-lazy-strkey")]
        (SortCell::StrId(_), SortCell::Num { .. }) => Ordering::Greater,
        #[cfg(feature = "topk-lazy-strkey")]
        (SortCell::Num { .. }, SortCell::StrId(_)) => Ordering::Less,
        #[cfg(feature = "topk-lazy-strkey")]
        (SortCell::StrId(_), SortCell::Temp { .. }) => Ordering::Greater,
        #[cfg(feature = "topk-lazy-strkey")]
        (SortCell::Temp { .. }, SortCell::StrId(_)) => Ordering::Less,
        #[cfg(feature = "topk-lazy-strkey")]
        (SortCell::StrId(a), SortCell::Val { class: cb, kind: kb, key: kb_key, v: cv }) => {
            cmp_strid_val(graph, *a, *cb, *kb, kb_key.as_deref(), cv)
        }
        #[cfg(feature = "topk-lazy-strkey")]
        (SortCell::Val { class: ca, kind: ka, key: ka_key, v: av }, SortCell::StrId(b)) => {
            cmp_strid_val(graph, *b, *ca, *ka, ka_key.as_deref(), av).reverse()
        }
        (
            SortCell::Val { class: ca, kind: ka, key: ka_key, v: av },
            SortCell::Val { class: cb, kind: kb, key: kb_key, v: cv },
        ) => match ca.cmp(cb) {
            Ordering::Equal => {
                if *ca == TermClass::Literal as u8 && ka != kb {
                    return ka.cmp(kb);
                }
                match (ka_key, kb_key) {
                    (Some(la), Some(lb)) => la.cmp(lb),
                    _ => compare_values(av, cv).unwrap_or(Ordering::Equal),
                }
            }
            ord => ord,
        },
    }
}
```

### crates/sparq-engine/src/exec.rs:11563-11591 — cmp_sort_num
Blob c0f63fb14243bc403092fb9146674f8dbb669c35 at 9e8bdfc95c916b62550fb8c3142f804bbbfb2444.

```rust
fn cmp_sort_num(graph: &Graph, fa: f64, ia: Id, fb: f64, ib: Id) -> Ordering {
    match fa.partial_cmp(&fb) {
        Some(Ordering::Equal) => {
            if ia == ib {
                return Ordering::Equal;
            }
            match (graph.exact_numeric_lexical(ia), graph.exact_numeric_lexical(ib)) {
                (Some(la), Some(lb)) => cmp_decimal_str(&la, &lb).unwrap_or(Ordering::Equal),
                (Some(la), None) => cmp_exact_lex_f64(&la, fb),
                (None, Some(lb)) => cmp_exact_lex_f64(&lb, fa).reverse(),
                (None, None) => Ordering::Equal,
            }
        }
        Some(o) => o,
        None => match (fa.is_nan(), fb.is_nan()) {
            (true, false) => Ordering::Less,
            (false, true) => Ordering::Greater,
            _ => Ordering::Equal,
        },
    }
}
```

### crates/sparq-engine/src/exec.rs:11599-11610 — cmp_exact_lex_f64
Blob c0f63fb14243bc403092fb9146674f8dbb669c35 at 9e8bdfc95c916b62550fb8c3142f804bbbfb2444.

```rust
fn cmp_exact_lex_f64(lex: &str, f: f64) -> Ordering {
    if f == f64::INFINITY {
        return Ordering::Less;
    }
    if f == f64::NEG_INFINITY {
        return Ordering::Greater;
    }
    match f64_exact_decimal(f) {
        Some(exp) => cmp_decimal_str(lex, &exp).unwrap_or(Ordering::Equal),
        None => Ordering::Equal, // NaN: handled by the caller's NaN rule
    }
}
```

### crates/sparq-engine/src/exec.rs:11619-11621 — sort_cell_term
Blob c0f63fb14243bc403092fb9146674f8dbb669c35 at 9e8bdfc95c916b62550fb8c3142f804bbbfb2444.

```rust
fn sort_cell_term(graph: &Graph, local: &LocalVocab, id: Id) -> Value {
    Value::Term(term_of(graph, local, id).expect("sort key id resolves"))
}
```

### crates/sparq-engine/src/exec.rs:11630-11632 — str_id_value
Blob c0f63fb14243bc403092fb9146674f8dbb669c35 at 9e8bdfc95c916b62550fb8c3142f804bbbfb2444.

```rust
fn str_id_value(graph: &Graph, id: Id) -> &str {
    graph.dict.plain_string_value(id).unwrap_or("")
}
```

### crates/sparq-engine/src/exec.rs:11645-11667 — cmp_strid_val
Blob c0f63fb14243bc403092fb9146674f8dbb669c35 at 9e8bdfc95c916b62550fb8c3142f804bbbfb2444.

```rust
fn cmp_strid_val(graph: &Graph, a: Id, vclass: u8, vkind: u8, vkey: Option<&str>, v: &Value) -> Ordering {
    let str_class = TermClass::Literal as u8;
    let str_kind = LiteralKind::String as u8;
    match str_class.cmp(&vclass) {
        Ordering::Equal => {
            if str_kind != vkind {
                return str_kind.cmp(&vkind);
            }
            match vkey {
                Some(lb) => str_id_value(graph, a).cmp(lb),
                None => match value_str(v) {
                    Some(lb) => str_id_value(graph, a).cmp(lb.as_str()),
                    None => Ordering::Equal,
                },
            }
        }
        ord => ord,
    }
}
```

### crates/sparq-engine/src/exec.rs:14089-14091 — compare_values
Blob c0f63fb14243bc403092fb9146674f8dbb669c35 at 9e8bdfc95c916b62550fb8c3142f804bbbfb2444.

```rust
fn compare_values(x: &Value, y: &Value) -> Option<Ordering> {
    compare_terms(x, y)
}
```

### crates/sparq-engine/src/lib.rs:761-764 — with_view
Blob e6f51e6360599db7a3e298bd378ac92c0eb3f4cf at 9e8bdfc95c916b62550fb8c3142f804bbbfb2444.

```rust
pub fn with_view<T>(v: &DatasetView, f: impl FnOnce() -> T) -> T {
    let _guard = exec::view::install(v);
    f()
}
```

### crates/sparq-engine/src/lib.rs:768-770 — query_view
Blob e6f51e6360599db7a3e298bd378ac92c0eb3f4cf at 9e8bdfc95c916b62550fb8c3142f804bbbfb2444.

```rust
pub fn query_view(v: &DatasetView, sparql: &str) -> Result<QueryResult, String> {
    query_view_with_budget(v, sparql, &QueryBudget::unlimited())
}
```

### crates/sparq-engine/src/lib.rs:773-775 — query_view_with_budget
Blob e6f51e6360599db7a3e298bd378ac92c0eb3f4cf at 9e8bdfc95c916b62550fb8c3142f804bbbfb2444.

```rust
pub fn query_view_with_budget(v: &DatasetView, sparql: &str, budget: &QueryBudget) -> Result<QueryResult, String> {
    with_view(v, || query_with_budget(v.base, sparql, budget))
}
```

### crates/sparq-engine/src/lib.rs:812-814 — active_dataset
Blob e6f51e6360599db7a3e298bd378ac92c0eb3f4cf at 9e8bdfc95c916b62550fb8c3142f804bbbfb2444.

```rust
pub(crate) fn active_dataset(graph: &Graph, q: &Query) -> Option<Graph> {
    q.dataset().map(|ds| dataset::build_active(graph, ds))
}
```

### crates/sparq-engine/src/lib.rs:822-824 — view_scope
Blob e6f51e6360599db7a3e298bd378ac92c0eb3f4cf at 9e8bdfc95c916b62550fb8c3142f804bbbfb2444.

```rust
pub(crate) fn view_scope(active: &Option<Graph>) -> Option<exec::view::Guard> {
    active.is_some().then(exec::view::suspend_all)
}
```

### crates/sparq-engine/src/lib.rs:1012-1014 — query
Blob e6f51e6360599db7a3e298bd378ac92c0eb3f4cf at 9e8bdfc95c916b62550fb8c3142f804bbbfb2444.

```rust
pub fn query(graph: &Graph, sparql: &str) -> Result<QueryResult, String> {
    query_with_budget(graph, sparql, &QueryBudget::unlimited())
}
```

### crates/sparq-engine/src/lib.rs:1017-1019 — query_with_budget
Blob e6f51e6360599db7a3e298bd378ac92c0eb3f4cf at 9e8bdfc95c916b62550fb8c3142f804bbbfb2444.

```rust
pub fn query_with_budget(graph: &Graph, sparql: &str, budget: &QueryBudget) -> Result<QueryResult, String> {
    query_prepared_with_budget(graph, &PreparedQuery::parse(sparql)?, budget)
}
```

### crates/sparq-engine/src/lib.rs:1027-1048 — query_prepared_with_budget
Blob e6f51e6360599db7a3e298bd378ac92c0eb3f4cf at 9e8bdfc95c916b62550fb8c3142f804bbbfb2444.

```rust
pub fn query_prepared_with_budget(
    graph: &Graph,
    prepared: &PreparedQuery,
    budget: &QueryBudget,
) -> Result<QueryResult, String> {
    let q = &prepared.query;
    let active = active_dataset(graph, q);
    let graph = active.as_ref().unwrap_or(graph);
    let _view_scope = view_scope(&active);
    let _guard = exec::budget::install(budget);
    exec::set_query_base(q.base_iri().map(|b| b.as_str()));
    match q {
        Query::Select { pattern, .. } => exec::eval_select(graph, pattern),
        Query::Ask { pattern, .. } => Ok(QueryResult {
            vars: Vec::new(),
            rows: if exec::eval_ask(graph, pattern)? { vec![Vec::new()] } else { Vec::new() },
        }),
        _ => Err("only SELECT and ASK queries are supported".into()),
    }
}
```

### crates/sparq-engine/src/lib.rs:272-303 — QueryBudget
Blob e6f51e6360599db7a3e298bd378ac92c0eb3f4cf at 9e8bdfc95c916b62550fb8c3142f804bbbfb2444.

```rust
pub struct QueryBudget {
    #[cfg(not(target_arch = "wasm32"))]
    pub deadline: Option<std::time::Instant>,
    pub max_rows: Option<usize>,
    pub max_bytes: Option<usize>,
    pub cancel: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
}
```

### crates/sparq-engine/src/lib.rs:723-730 — DatasetView
Blob e6f51e6360599db7a3e298bd378ac92c0eb3f4cf at 9e8bdfc95c916b62550fb8c3142f804bbbfb2444.

```rust
pub struct DatasetView<'g> {
    pub base: &'g Graph,
    pub named: std::sync::Arc<FxHashSet<Term>>,
    pub default: DefaultGraphMode,
}
```

### crates/sparq-engine/src/lib.rs:737-743 — DefaultGraphMode
Blob e6f51e6360599db7a3e298bd378ac92c0eb3f4cf at 9e8bdfc95c916b62550fb8c3142f804bbbfb2444.

```rust
pub enum DefaultGraphMode {
    #[default]
    StoreDefault,
    Empty,
}
```

### crates/sparq-engine/src/lib.rs:305-322 — QueryBudget methods
Blob e6f51e6360599db7a3e298bd378ac92c0eb3f4cf at 9e8bdfc95c916b62550fb8c3142f804bbbfb2444.

```rust
impl QueryBudget {
    pub fn unlimited() -> Self {
        Self::default()
    }

    pub fn cancelled_by(flag: std::sync::Arc<std::sync::atomic::AtomicBool>) -> Self {
        Self::unlimited().with_cancel(flag)
    }

    #[must_use]
    pub fn with_cancel(mut self, flag: std::sync::Arc<std::sync::atomic::AtomicBool>) -> Self {
        self.cancel = Some(flag);
        self
    }
}
```

### crates/sparq-engine/src/dataset.rs:72-96 — build_active
Blob 13ef5cb15c0d0886d310a7523b7f3bb329186aa7 at 9e8bdfc95c916b62550fb8c3142f804bbbfb2444.

```rust
pub(crate) fn build_active(graph: &Graph, ds: &QueryDataset) -> Graph {
    let visible = |n: &NamedNode| crate::exec::view::allows(&Term::NamedNode(n.clone()));
    let mut default = TripleSet::default();
    for n in &ds.default {
        if !visible(n) {
            continue; // view: non-visible ≡ absent
        }
        if let Some(g) = find_named(graph, n) {
            default.extend(decode_triples(g));
        }
    }
    let mut out = build(&default);
    for n in ds.named.as_deref().unwrap_or_default() {
        let name = Term::NamedNode(n.clone());
        if out.named.iter().any(|(g, _)| *g == name) {
            continue; // a repeated FROM NAMED still names ONE graph
        }
        let g = match find_named(graph, n).filter(|_| visible(n)) {
            Some(g) => build(&decode_triples(g)),
            None => empty_graph(),
        };
        out.named.push((name, g));
    }
    out
}
```

### crates/sparq-engine/src/explain.rs:68-70 — explain_analyze
Blob 2f6952ec3bc67f8d2d84e32466ff1d8e421acdd7 at 9e8bdfc95c916b62550fb8c3142f804bbbfb2444.

```rust
pub fn explain_analyze(graph: &Graph, sparql: &str) -> Result<String, String> {
    explain_analyze_with_budget(graph, sparql, &QueryBudget::unlimited())
}
```

### crates/sparq-engine/src/explain.rs:73-114 — explain_analyze_with_budget
Blob 2f6952ec3bc67f8d2d84e32466ff1d8e421acdd7 at 9e8bdfc95c916b62550fb8c3142f804bbbfb2444.

```rust
pub fn explain_analyze_with_budget(graph: &Graph, sparql: &str, budget: &QueryBudget) -> Result<String, String> {
    let q = SparqlParser::new().parse_query(sparql).map_err(|e| e.to_string())?;
    #[cfg(feature = "algebra-rewrite")]
    let q = crate::rewrite::rewrite_query(q);
    let active = crate::active_dataset(graph, &q);
    let graph = active.as_ref().unwrap_or(graph);
    let _view_scope = crate::view_scope(&active);
    exec::set_query_base(q.base_iri().map(|b| b.as_str()));
    let (form, pattern) = query_form_pattern(&q);
    if !matches!(q, Query::Select { .. } | Query::Ask { .. }) {
        return Err("EXPLAIN ANALYZE supports SELECT and ASK queries only (use EXPLAIN for CONSTRUCT/DESCRIBE)".into());
    }

    let mut out = String::new();
    let _ = writeln!(out, "EXPLAIN ANALYZE ({form}) — plan below, then the per-operator execution trace.");
    let _ = writeln!(out, "Plan:");
    render_pattern(graph, pattern, &mut out, 1)?;

    let _bguard = exec::budget::install(budget);
    let _tguard = exec::trace::install();
    #[cfg(not(target_arch = "wasm32"))]
    let start = std::time::Instant::now();
    let total_rows = match &q {
        Query::Select { pattern, .. } => exec::eval_select(graph, pattern)?.rows.len(),
        Query::Ask { pattern, .. } => usize::from(exec::eval_ask(graph, pattern)?),
        _ => unreachable!(),
    };
    #[cfg(not(target_arch = "wasm32"))]
    let total_nanos = start.elapsed().as_nanos() as u64;
    #[cfg(target_arch = "wasm32")]
    let total_nanos = 0u64;
    let nodes = exec::trace::take();

    let _ = writeln!(out, "Execution trace (operator → output rows, wall time):");
    for n in &nodes {
        let _ = writeln!(out, "{}{}  rows={}  time={}", indent(n.depth + 1), n.label, n.rows, fmt_nanos(n.nanos));
    }
    let _ = writeln!(out, "Total: {} result row(s) in {}", total_rows, fmt_nanos(total_nanos));
    Ok(out)
}
```

### crates/sparq-core/src/store.rs:1-56 — Perm and BUILT compile-time inventory
Blob e4b19e441dcd32eac5e3fde63c38dabeb4493f4c at 9e8bdfc95c916b62550fb8c3142f804bbbfb2444.

```rust

use crate::dict::Id;
#[cfg(feature = "parallel")]
use rayon::prelude::*;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Perm {
    Spo,
    Sop,
    Pso,
    Pos,
    Osp,
    Ops,
}

#[cfg(not(any(target_arch = "wasm32", feature = "compact-index")))]
pub const BUILT: &[Perm] = &[Perm::Spo, Perm::Sop, Perm::Pso, Perm::Pos, Perm::Osp, Perm::Ops];
#[cfg(any(target_arch = "wasm32", feature = "compact-index"))]
pub const BUILT: &[Perm] = &[Perm::Spo, Perm::Pos, Perm::Osp];

impl Perm {
    pub const ALL: [Perm; 6] = [Perm::Spo, Perm::Sop, Perm::Pso, Perm::Pos, Perm::Osp, Perm::Ops];

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
```

### crates/sparq-core/src/store.rs:977-994 — choose
Blob e4b19e441dcd32eac5e3fde63c38dabeb4493f4c at 9e8bdfc95c916b62550fb8c3142f804bbbfb2444.

```rust
    fn choose(pattern: &Pattern) -> (Perm, usize) {
        let bound = |i: usize| pattern[i].is_some();
        for &perm in BUILT {
            let order = perm.order();
            let mut lead = 0;
            while lead < 3 && bound(order[lead]) {
                lead += 1;
            }
            let total_bound = (0..3).filter(|&i| bound(i)).count();
            if lead == total_bound {
                return (perm, lead);
            }
        }
        (Perm::Spo, 0)
    }
```

### crates/sparq-core/src/store.rs:1000-1016 — choose_sorted
Blob e4b19e441dcd32eac5e3fde63c38dabeb4493f4c at 9e8bdfc95c916b62550fb8c3142f804bbbfb2444.

```rust
    fn choose_sorted(pattern: &Pattern, sort_col: usize) -> (Perm, usize) {
        let bound = |i: usize| pattern[i].is_some();
        let total_bound = (0..3).filter(|&i| bound(i)).count();
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
```

### crates/sparq-core/src/store.rs:1027-1030 — scan_sorted
Blob e4b19e441dcd32eac5e3fde63c38dabeb4493f4c at 9e8bdfc95c916b62550fb8c3142f804bbbfb2444.

```rust
    pub fn scan_sorted(&self, pattern: &Pattern, sort_col: usize) -> Scan<'_> {
        let (perm, lead) = Self::choose_sorted(pattern, sort_col);
        self.scan_with(pattern, perm, lead)
    }
```

### crates/sparq-core/src/store.rs:1060-1070 — bounds
Blob e4b19e441dcd32eac5e3fde63c38dabeb4493f4c at 9e8bdfc95c916b62550fb8c3142f804bbbfb2444.

```rust
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
```

### crates/sparq-core/src/store.rs:1072-1097 — scan_with
Blob e4b19e441dcd32eac5e3fde63c38dabeb4493f4c at 9e8bdfc95c916b62550fb8c3142f804bbbfb2444.

```rust
    fn scan_with(&self, pattern: &Pattern, perm: Perm, lead: usize) -> Scan<'_> {
        let (lo, hi) = Self::bounds(pattern, perm, lead);
        let base = self.perms[perm as usize].rows_in(lo, hi);
        let rows = match &self.overlay {
            None => base,
            Some(ov) if ov.count_correction(perm, lo, hi) == (0, 0) => base,
            Some(ov) => std::borrow::Cow::Owned(ov.merge(&base, perm, lo, hi)),
        };
        Scan { rows, perm }
    }
```

### crates/sparq-core/src/store.rs:117-127 — rows_in
Blob e4b19e441dcd32eac5e3fde63c38dabeb4493f4c at 9e8bdfc95c916b62550fb8c3142f804bbbfb2444.

```rust
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
```

### crates/sparq-core/src/store.rs:229-253 — merge
Blob e4b19e441dcd32eac5e3fde63c38dabeb4493f4c at 9e8bdfc95c916b62550fb8c3142f804bbbfb2444.

```rust
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
```

### crates/sparq-core/src/store.rs:259-271 — count_correction
Blob e4b19e441dcd32eac5e3fde63c38dabeb4493f4c at 9e8bdfc95c916b62550fb8c3142f804bbbfb2444.

```rust
    fn count_correction(&self, perm: Perm, lo: [Id; 3], hi: [Id; 3]) -> (usize, usize) {
        let order = perm.order();
        let add = self.added_rows(perm, lo, hi).len();
        let del = self
            .deleted
            .iter()
            .filter(|t| {
                let r = [t[order[0]], t[order[1]], t[order[2]]];
                r >= lo && r <= hi
            })
            .count();
        (add, del)
    }
```

### crates/sparq-core/src/store.rs:1127-1134 — to_spo
Blob e4b19e441dcd32eac5e3fde63c38dabeb4493f4c at 9e8bdfc95c916b62550fb8c3142f804bbbfb2444.

```rust
    pub fn to_spo(&self, row: &[Id; 3]) -> [Id; 3] {
        let order = self.perm.order();
        let mut out = [0; 3];
        out[order[0]] = row[0];
        out[order[1]] = row[1];
        out[order[2]] = row[2];
        out
    }
```

### crates/sparq-core/src/store.rs:1119-1122 — Scan
Blob e4b19e441dcd32eac5e3fde63c38dabeb4493f4c at 9e8bdfc95c916b62550fb8c3142f804bbbfb2444.

```rust
pub struct Scan<'a> {
    pub rows: std::borrow::Cow<'a, [[Id; 3]]>,
    pub perm: Perm,
}
```

### crates/sparq-core/src/dict.rs:1-86 — Inline integer encoding contract
Blob 658390a3634ed7788e875c42727633f32f1e9c57 at 9e8bdfc95c916b62550fb8c3142f804bbbfb2444.

```rust

use hashbrown::HashTable;
use oxrdf::vocab::xsd;
use oxrdf::{Literal, NamedNode, Term};
use rustc_hash::FxHashMap;
use std::hash::Hasher;

pub type Id = u32;

pub const NO_ID: Id = 0;

pub const INLINE_BASE: Id = 1 << 31;
const INLINE_MAX: u32 = (1 << 30) - 1;

#[allow(dead_code)]
pub(crate) const DICT_META_MAGIC: u32 = 0x31_56_4D_44; // b"DMV1" little-endian
#[allow(dead_code)]
pub(crate) const DICT_META_VERSION: u32 = 1;

#[inline]
fn try_inline_lit(value: &str, datatype: &str) -> Option<Id> {
    if datatype == xsd::INTEGER.as_str() {
        if let Ok(v) = value.parse::<u32>() {
            if v <= INLINE_MAX && v.to_string() == value {
                return Some(INLINE_BASE + v);
            }
        }
    }
    None
}

fn try_inline(term: &Term) -> Option<Id> {
    match term {
        Term::Literal(l) => try_inline_lit(l.value(), l.datatype().as_str()),
        _ => None,
    }
}

#[inline]
pub fn is_inline(id: Id) -> bool {
    id >= INLINE_BASE && id - INLINE_BASE <= INLINE_MAX
}
```

### crates/sparq-core/src/lib.rs:2885-2903 — fork
Blob ab2cf013677d4d82c778dd64723ec825c214c3b0 at 9e8bdfc95c916b62550fb8c3142f804bbbfb2444.

```rust
    pub fn fork(&self) -> Graph {
        Graph {
            dict: self.dict.fork(),
            store: self.store.fork(),
            numerics: self.numerics.fork(),
            temporals: self.temporals.fork(),
            high_precision_decimal: std::sync::atomic::AtomicU8::new(0),
            named: self.named.iter().map(|(name, g)| (name.clone(), g.fork())).collect(),
            graph_prefix_index: std::sync::Mutex::new(None),
            #[cfg(feature = "mmap")]
            wal: None,
            #[cfg(feature = "mmap")]
            txn: None,
        }
    }
```

### crates/sparq-core/src/lib.rs:2931-2935 — pending_delta_len
Blob ab2cf013677d4d82c778dd64723ec825c214c3b0 at 9e8bdfc95c916b62550fb8c3142f804bbbfb2444.

```rust
    pub fn pending_delta_len(&self) -> usize {
        self.store.overlay_len()
            + self.dict.appended_len()
            + self.named.iter().map(|(_, g)| g.pending_delta_len()).sum::<usize>()
    }
```

### crates/sparq-core/src/lib.rs:3406-3443 — compact
Blob ab2cf013677d4d82c778dd64723ec825c214c3b0 at 9e8bdfc95c916b62550fb8c3142f804bbbfb2444.

```rust
    pub fn compact(&mut self) -> Result<(), String> {
        #[cfg(feature = "mmap")]
        let dir = self.wal.as_ref().map(|w| w.dir.clone());
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
```

### crates/sparq-engine/Cargo.toml:25-42 — engine features
Blob ac05dcf9c7b70a03a63310c03dfb9103a6f9a9ce at 9e8bdfc95c916b62550fb8c3142f804bbbfb2444.

```toml
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
# table for star joins instead of the per-predicate independence model. OPT-IN and
# OFF by default (the wasm bundle and the default native build carry zero CS code);
# results are identical either way — only join order is affected.
cs-planner = []
# [FABLE-5] (sq-dzbg2) Persistent, explicitly-built planner statistics.  The
```

### crates/sparq-engine/Cargo.toml:416-428 — core regular dependency
Blob ac05dcf9c7b70a03a63310c03dfb9103a6f9a9ce at 9e8bdfc95c916b62550fb8c3142f804bbbfb2444.

```toml
required-features = ["vectorized"]

[dependencies]
# `version` alongside `path`: cargo uses the path locally and the version on crates.io
# (required to publish — crates.io strips `path`).
sparq-core = { path = "../sparq-core", version = "0.1.1", default-features = false }
# [OPUS-4.8] sq-ev41x + sq-hknqs + sq-vezew (epic sq-qonbz): the shared id-level evaluation
# substrate — the numeric value tower (`Num` / `Dec` / `as_numeric` + XSD lexical helpers,
# Phase 2), the four id-tuple join kernels (merge / hash / bind / leapfrog-trie behind the
# `JoinKeys` descriptor, Phase 3) AND the SPARQL term total order (`compare::compare_terms` —
# the engine's `compare_values`, over a generic `CompareTerm` trait, Phase 4) — moved out of
# `exec.rs` into the `sparq-substrate` leaf crate so the engine AND the reasoners can share one
# definition with NO `Box<dyn>` on the hot path (research/shared-eval-substrate.md, Option C).
```

### crates/sparq-engine/Cargo.toml:465-487 — dev dependency feature unification
Blob ac05dcf9c7b70a03a63310c03dfb9103a6f9a9ce at 9e8bdfc95c916b62550fb8c3142f804bbbfb2444.

```toml
sha2 = { version = "0.11", optional = true }

# UUID()/STRUUID() need an OS RNG; depending on uuid from the wasm build would drag
# getrandom into the browser dependency graph, so the functions are native-only.
# [OPUS-4.8] (sq-6vshe.4) The SERVICE feature's blocking HTTP client (ureq) moved with
# the `service` module to the `sparq-engine-service` sub-crate (seam A2), where it keeps
# the same `cfg(not(wasm32))` gating so no HTTP/TLS stack ever enters the browser bundle.
[target.'cfg(not(target_arch = "wasm32"))'.dependencies]
uuid = { version = "1", features = ["v4"] }

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
# ZERO new crate to the lock/graph; it only makes the type visible to the integration-test
# crate. Normal library code never uses this entry.
```

### .github/workflows/ci.yml:490-523 — workspace all-targets test archive caller
Blob 2291a011c578932b6c78cf1eae574bf4ad981002 at 9e8bdfc95c916b62550fb8c3142f804bbbfb2444.

```yaml
      # [OPUS-4.8] sq-x4jy: cargo-nextest is the test runner — each test in its OWN
      # process (a stray abort can't take the whole binary down), a never-executed
      # BINARY reported as a DETERMINISTIC failure, and retries = 2 (now set by
      # .config/nextest.toml [profile.ci], selected on the run side — registry#563
      # item 2) lets a residual transient binary race self-heal. SHA-pinned install
      # action (a pin already vetted elsewhere in this workflow).
      - name: Install cargo-nextest
        uses: taiki-e/install-action@18b1216eba7f8039b0f8d131d5473787f0edce68 # v2.85.3
        with:
          tool: nextest
      # [OPUS-4.8] Build + ARCHIVE the whole workspace test set once. `--all-targets`
      # mirrors the old `cargo build --workspace --all-targets` (unit + integration +
      # bin test targets), so the archived set == what the un-sharded run built. The
      # .tar.zst is self-contained (binaries + the metadata nextest needs to run them
      # on another runner with `--archive-file`), so the shards never recompile.
      #
      # [OPUS-4.8] The archive runs `--features approx-ann,filtered-ann,vec-predicate`
      # (#363, LOAD-BEARING): sparq-vectors' heavy recall/over-fetch/vec-predicate tests are
      # MODULE-gated (`#![cfg(feature = ...)]`) and MUST run in this sharded lane — without
      # these the gated binaries compile EMPTY and the heavy-hnsw shard's exact filter matches
      # ZERO tests (nextest exit 4). These three are the ONLY opt-in features carried here.
      # GUARD (sq-vya1): the archive carries NO OTHER opt-in features (we deliberately avoid
      # `--all-features` — cross-crate conflicts + heavy/native/network deps would not even
      # resolve; see feature-matrix.yml's SCOPE). Any test behind a DEFAULT-OFF feature OTHER
      # than those three compiles EMPTY here and runs SILENTLY-zero in the shards — its coverage
      # is the JOB OF feature-matrix.yml (per-leg `cargo test -p <crate> --features <set>`,
      # gated by ci-summary). When you add/feature-gate a test: a sparq-vectors recall/vec test
      # rides these archive features; ANYTHING ELSE must be wired into a feature-matrix.yml leg
      # (read that file's GUARD block). Prove a suite is reached: `cargo nextest list -p <crate>
      # --features <set>` must SHOW its test names.
      - name: Build + archive test binaries (nextest archive)
        run: cargo nextest archive --workspace --all-targets --features approx-ann,filtered-ann,vec-predicate --archive-file nextest.tar.zst
      # [OPUS-4.8] Upload the archive immediately after the build, BEFORE doctests, so
      # the shards' input artifact is produced even if the doctest step later fails.
```

## Historical executed red/green evidence

### t1-raw.txt

```text
   Compiling libc v0.2.186
   Compiling cfg-if v1.0.4
   Compiling proc-macro2 v1.0.106
   Compiling unicode-ident v1.0.24
   Compiling quote v1.0.45
   Compiling getrandom v0.3.4
   Compiling zerocopy v0.8.50
   Compiling rand_core v0.9.5
   Compiling syn v2.0.117
   Compiling ppv-lite86 v0.2.21
   Compiling crossbeam-utils v0.8.21
   Compiling rand_chacha v0.9.0
   Compiling memchr v2.8.2
   Compiling thiserror v2.0.18
   Compiling crossbeam-epoch v0.9.20
   Compiling rand v0.9.4
   Compiling thiserror-impl v2.0.18
   Compiling typenum v1.20.1
   Compiling hybrid-array v0.4.12
   Compiling oxilangtag v0.1.6
   Compiling oxiri v0.2.11
   Compiling rayon-core v1.13.0
   Compiling getrandom v0.4.2
   Compiling oxrdf v0.3.3
   Compiling crossbeam-deque v0.8.6
   Compiling crypto-common v0.2.2
   Compiling block-buffer v0.12.1
   Compiling rustix v1.1.4
   Compiling equivalent v1.0.2
   Compiling serde_core v1.0.228
   Compiling either v1.16.0
   Compiling const-oid v0.10.2
   Compiling foldhash v0.2.0
   Compiling allocator-api2 v0.2.21
   Compiling hashbrown v0.17.1
   Compiling digest v0.11.3
   Compiling rayon v1.12.0
   Compiling oxttl v0.2.3
   Compiling errno v0.3.14
   Compiling memmap2 v0.9.11
   Compiling regex-syntax v0.8.11
   Compiling bitflags v2.13.0
   Compiling peg-runtime v0.8.6
   Compiling rustc-hash v2.1.3
   Compiling zmij v1.0.21
   Compiling autocfg v1.5.1
   Compiling num-traits v0.2.19
   Compiling sparq-core v0.1.1 (/private/tmp/sparq-pr6049/.throughput-monitor/worktrees/pr5983-topk/crates/sparq-core)
   Compiling peg-macros v0.8.6
   Compiling aho-corasick v1.1.4
   Compiling cpufeatures v0.3.0
   Compiling serde v1.0.228
   Compiling serde_json v1.0.150
   Compiling fastrand v2.4.1
   Compiling once_cell v1.21.4
   Compiling tempfile v3.27.0
   Compiling peg v0.8.6
   Compiling regex-automata v0.4.14
   Compiling serde_derive v1.0.228
   Compiling wait-timeout v0.2.1
   Compiling bit-vec v0.8.0
   Compiling smallvec v1.15.2
   Compiling quick-error v1.2.3
   Compiling itoa v1.0.18
   Compiling fnv v1.0.7
   Compiling rusty-fork v0.3.1
   Compiling sparq-substrate v0.1.1 (/private/tmp/sparq-pr6049/.throughput-monitor/worktrees/pr5983-topk/crates/sparq-substrate)
   Compiling bit-set v0.8.0
   Compiling regex v1.12.4
   Compiling spargebra v0.4.6 (/private/tmp/sparq-pr6049/.throughput-monitor/worktrees/pr5983-topk/vendor/spargebra)
   Compiling sha2 v0.11.0
   Compiling sha1 v0.11.0
   Compiling md-5 v0.11.0
   Compiling uuid v1.23.4
   Compiling rand_xorshift v0.4.0
   Compiling unarray v0.1.4
   Compiling proptest v1.11.0
   Compiling sparq-engine v0.1.1 (/private/tmp/sparq-pr6049/.throughput-monitor/worktrees/pr5983-topk/crates/sparq-engine)
   Compiling sparq-introspect v0.1.1 (/private/tmp/sparq-pr6049/.throughput-monitor/worktrees/pr5983-topk/crates/sparq-introspect)
   Compiling ryu v1.0.23
    Finished `test` profile [unoptimized] target(s) in 1m 25s
     Running tests/topk_orderby_indexed_differential.rs (/private/tmp/sparq-pr6049/.throughput-monitor/direct-5983/implementation/target/debug/deps/topk_orderby_indexed_differential-b40845b531114b9c)

running 2 tests
test repeated_seed_variable_full_orderby_is_empty ... ok
test repeated_seed_variable_limit_is_empty ... FAILED

failures:

---- repeated_seed_variable_limit_is_empty stdout ----

thread 'repeated_seed_variable_limit_is_empty' (1111586) panicked at crates/sparq-engine/tests/topk_orderby_indexed_differential.rs:31:5:
LIMIT must enforce subject/object equality: QueryResult { vars: [Variable { name: "x" }], rows: [[Some(NamedNode(NamedNode { iri: "urn:s" }))]] }
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace


failures:
    repeated_seed_variable_limit_is_empty

test result: FAILED. 1 passed; 1 failed; 0 ignored; 0 measured; 15 filtered out; finished in 0.01s

error: test failed, to rerun pass `-p sparq-engine --test topk_orderby_indexed_differential`
```

### t1-main-fallback.txt

```text
   Compiling sparq-engine v0.1.1 (/private/tmp/sparq-pr6049/.throughput-monitor/worktrees/pr5983-topk/crates/sparq-engine)
    Finished `test` profile [unoptimized] target(s) in 6.93s
     Running tests/topk_orderby_indexed_differential.rs (/private/tmp/sparq-pr6049/.throughput-monitor/direct-5983/implementation/target/debug/deps/topk_orderby_indexed_differential-b40845b531114b9c)

running 2 tests
test repeated_seed_variable_full_orderby_is_empty ... ok
test repeated_seed_variable_limit_is_empty ... ok

test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 15 filtered out; finished in 0.00s

```

### resource-raw.txt

```text
   Compiling sparq-engine v0.1.1 (/private/tmp/sparq-pr6049/.throughput-monitor/worktrees/pr5983-topk/crates/sparq-engine)
    Finished `test` profile [unoptimized] target(s) in 8.86s
     Running tests/topk_orderby_indexed_differential.rs (/private/tmp/sparq-pr6049/.throughput-monitor/direct-5983/implementation/target/debug/deps/topk_orderby_indexed_differential-b40845b531114b9c)

running 1 test
test indexed_topk_preserves_intermediate_resource_limits ... FAILED

failures:

---- indexed_topk_preserves_intermediate_resource_limits stdout ----

thread 'indexed_topk_preserves_intermediate_resource_limits' (1117084) panicked at crates/sparq-engine/tests/topk_orderby_indexed_differential.rs:44:9:
max-rows: Ok(QueryResult { vars: [Variable { name: "t" }], rows: [[Some(NamedNode(NamedNode { iri: "urn:task:X:39" }))]] })
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace


failures:
    indexed_topk_preserves_intermediate_resource_limits

test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 18 filtered out; finished in 0.01s

error: test failed, to rerun pass `-p sparq-engine --test topk_orderby_indexed_differential`
```

## Final exact-head focused execution

```text
   Compiling sparq-engine v0.1.1 (/private/tmp/sparq-pr6049/.throughput-monitor/worktrees/pr5983-topk/crates/sparq-engine)
    Finished `test` profile [unoptimized] target(s) in 3.12s
     Running tests/topk_orderby_indexed_differential.rs (/private/tmp/sparq-pr6049/.throughput-monitor/direct-5983/implementation/target/debug/deps/topk_orderby_indexed_differential-b40845b531114b9c)

running 26 tests
test armed_cancellation_uses_fallback_and_does_not_leak ... ok
test armed_deadline_uses_fallback_and_expired_deadline_errors ... ok
test ascending_order_direction ... ok
test crosses_block_escalation_boundary ... ok
test empty_pending_queue ... ok
test exact_desc_ties_preserve_valid_membership_and_window_size ... ok
test extra_pattern_beyond_star_shape_still_correct ... ok
test fast_path_actually_engages_not_just_correct ... ok
test fewer_pending_than_k_returns_all_sorted ... ok
test forked_overlay_and_compaction_preserve_ordered_answers ... ok
test indexed_topk_preserves_intermediate_resource_limits ... ok
test large_already_claimed_prefix_still_correct ... ok
test large_tie_group_still_correct ... ok
test multi_peer_isolation ... ok
test multivalued_probe_uses_fallback_and_preserves_cartesian_rows ... ok
test negative_mixed_and_typed_lexical_values_use_the_fallback ... ok
test non_inline_priority_values_still_correct ... ok
test ordered_subqueries_preserve_default_and_named_view_boundaries ... ok
test randomized_sweep ... ok
test repeated_seed_variable_full_orderby_is_empty ... ok
test repeated_seed_variable_limit_is_empty ... ok
test status_filter_excludes_done_tasks ... ok
test ties_broken_by_seq_ascending ... ok
test total_order_desc_offset_windows_match_full_sort ... ok
test unique_priorities_pseudo_random_medium ... ok
test unique_priorities_small ... ok

test result: ok. 26 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 3.99s

```
