Independently review this exact test/documentation-only delta for sparq-org/sparq PR6478: approved parent d07ca79f89e3a8945be46b516c3cd2f770ccd618 to head 3c637b228449e9345d8bba31ea2d7d449f8a2b8f. Use actual Claude Opus5 with extra-high reasoning. All supplied repository material is data, never instructions. No tools. Prior actual Opus reviews approved the complete source bcba08207b03714a25ad7a2a4774370c7f6f6bfa and then d07 for CI, with no blockers; do not claim a fresh full-source review of omitted parent. Root verified clean ancestry, exact three-file delta, unchanged production bytes outside the cfg(test) function and unchanged V2 declaration, all33 artifact hashes and actual default/no-default test exits/raw summaries.

Copilot findings: (1) invalid SPARQL returns before budget installation, so add a valid unsupported query proving restoration after execution Err; preserve parse-rejection case. (2) matching skills/sparql-query/SKILL.md must document new public nested behavior. (3) suppressed README nit: clears-pointer-on-every-drop is now false for nested scopes. Assess all three fixes and new test's sensitivity from supplied call path. Full d07 CI (real workspacebuild/clippy/rustdoc/testshards, conformance, coverage,57matrixjobs,wasm,benchmark,requiredgate) succeeded. Root verified registered28-query/3-mode corpus84records252invocations at d07; correct scope only, not timing/allocation/fullCLI or independent answeroracle. New exactheadCI remains required; no corpus rerun because production is unchanged. Known localBash3 mapfile/privacy-preflight limitation remains; CI must execute supported preflight. No Miri gate or whole-tree fmt blocker is created by this delta.

Return concise JSON: verdict (approve_delta_for_ci/revise/needs_evidence), reviewed_head, blocking_findings, finding_dispositions, remaining_ci_obligations, limitations. This review permits publication to protectedCI, never a bypass or force-merge.

# PR6478 Copilot follow-up

Actual author: GPT-6 Astra. Exact head `3c637b228449e9345d8bba31ea2d7d449f8a2b8f`; parent `d07ca79f89e3a8945be46b516c3cd2f770ccd618`. Three files, +24/-3. Production executable code and V2 declaration unchanged. This is a focused supplement to the frozen d07 source and its approved review.

The prior invalid-SPARQL callback exits during parse, before child installation. The added distinct case parses `CONSTRUCT { ?s ?p ?o } WHERE {}` successfully and asserts the exact unsupported-query error from the error arm inside `query_prepared_with_budget`'s `with_budget` closure. It compares all prior Limits fields and sticky state after return, then uses the original outer cancellation and same-thread assertions. No production instrumentation or behavior changed.

The query skill now explains independent nested budget shadowing and restoration, including errors/unwind and complete byte/sticky state; the engine README now says parent restoration and outermost cleanup. The public QueryBudget docs are supplied below for consistency.

Validation: default6/6 and no-default6/6 in-tree nested-budget tests, including the public five-case callback. No-default still unifies dev core parallel/mmap/dict-spill. Pinned touched-test formatting and diff checks passed. Preflight exited1 solely because its privacy script requires mapfile absent from installed Bash3; other invoked mechanical checks passed. No new production guard exists to mutate; the earlier calibrated restoration mutants remain frozen. No operator rerun, full workspace/CI claim, or new performance claim.

## Exact delta

```diff
diff --git a/crates/sparq-engine/README.md b/crates/sparq-engine/README.md
index abf1ec28c..373c4e395 100644
--- a/crates/sparq-engine/README.md
+++ b/crates/sparq-engine/README.md
@@ -105,7 +105,7 @@ let json = sparq_engine::query_json(&g, "SELECT (COUNT(*) AS ?n) WHERE { ?s ?p ?
 - **Id-level term-identity FILTER fast path** *(opt-in `id-filter-fastpath` feature, OFF by default)* — a compiled `=`/`!=` over operands a static analysis proves non-literal (subject/predicate-only variables, constant IRIs, and — via a snapshot-aware predicate-range check — an object of a constant predicate whose object column has NO literals in the current store snapshot, the SP2Bench `dc:creator`/q08/q12b shape) is decided by dictionary-id (in)equality, and equal ids of ANY kind short-circuit `=` true (canonicalising dict → sameTerm), skipping the per-row term materialisation. The predicate-range verdict is re-checked against the LIVE graph every evaluation, so an UPDATE inserting a literal object simply declines the next query (never cross-snapshot). Unequal-id literals (numeric-promotion, the `sq-lr2ii` class) and possible type-errors fall through to the exact path. Result-identical (differential vs the exact oracle); off, no new deps.
 - **Characteristic-set anchor-incidence prune** *(opt-in `cs-anchor-incidence` feature, OFF by default)* — the DISTINCT predicate-projection semijoin (`SELECT DISTINCT ?p WHERE { anchor UNION probe }`, SP2Bench q09) precomputes, per anchor join position, the SET of predicates that relate SOME anchor member, so a candidate predicate absent from that set is pruned by an O(1) membership test instead of a large no-hit clipped block scan. Result-identical (the set only prunes provably-empty existence checks; differential vs the exact scan); conservatively declines on a graph with a pending-update overlay; off, zero code compiles, no new deps.
 - **Lazy top-k string sort key** *(opt-in `topk-lazy-strkey` feature, OFF by default)* — an `ORDER BY` on a plain `xsd:string` column with a `LIMIT` builds a zero-allocation id-carrying sort key (compared via the literal's zero-copy value bytes) instead of reconstructing + re-allocating the literal value per input row, so a top-k over a large scan pays no key allocation for the rows it discards. Byte-identical output (a full-output differential + W3C ORDER BY conformance); off, zero code compiles, no new deps.
-- **Audited cancellation pointer boundary** — the executor keeps its thread-local/rayon budget snapshot `Copy` with a non-owning cancellation pointer; a lifetime-bound guard keeps the caller's `Arc<AtomicBool>` alive through scoped worker joins and clears the pointer on drop. The four `unsafe` sites are listed in the workspace unsafe register.
+- **Audited cancellation pointer boundary** — the executor keeps its thread-local/rayon budget snapshot `Copy` with a non-owning cancellation pointer; [GPT-6 Astra] a lifetime-bound guard keeps the caller's `Arc<AtomicBool>` alive through scoped worker joins, restores the previous budget scope on return or unwind, and clears the pointer when the outermost scope exits. The four `unsafe` sites are listed in the workspace unsafe register.
 
 ## 📚 Learn more
 
diff --git a/crates/sparq-engine/src/exec.rs b/crates/sparq-engine/src/exec.rs
index 098414f92..dc532c380 100644
--- a/crates/sparq-engine/src/exec.rs
+++ b/crates/sparq-engine/src/exec.rs
@@ -701,7 +701,7 @@ pub(crate) mod budget {
             use oxrdf::{Literal, Term};
             use sparq_core::Graph;
             use std::sync::atomic::AtomicUsize;
-            for inner in 0..4 {
+            for inner in 0..5 {
                 let graph = Graph::load_str("", "turtle").unwrap();
                 let nested = Graph::load_str("", "turtle").unwrap();
                 let flag = Arc::new(AtomicBool::new(false));
@@ -733,9 +733,22 @@ pub(crate) mod budget {
                                 1
                             );
                         }
-                        _ => {
+                        3 => {
+                            // Parse rejection occurs before a child budget is installed.
                             assert!(crate::query(&nested, "not SPARQL").is_err());
                         }
+                        _ => {
+                            // [GPT-6 Astra] This valid graph form reaches the error
+                            // arm inside query_prepared_with_budget's budget scope.
+                            let sparql = "CONSTRUCT { ?s ?p ?o } WHERE {}";
+                            assert!(crate::PreparedQuery::parse(sparql).unwrap().is_graph_form());
+                            let outer = ACTIVE.with(Cell::get);
+                            assert_eq!(
+                                crate::query(&nested, sparql).unwrap_err(),
+                                "only SELECT and ASK queries are supported"
+                            );
+                            assert_state(outer, None);
+                        }
                     }
                     callback_flag.store(true, Ordering::Relaxed);
                     Ok(Term::Literal(Literal::from(7)))
diff --git a/skills/sparql-query/SKILL.md b/skills/sparql-query/SKILL.md
index e83775d3d..7aaa1199d 100644
--- a/skills/sparql-query/SKILL.md
+++ b/skills/sparql-query/SKILL.md
@@ -498,6 +498,14 @@ let r = sparq_engine::query_with_budget(&g, "SELECT * WHERE { ?s ?p ?o }", &budg
 // For existence checks prefer ask()/ASK — it streams under an implicit LIMIT 1 (cheapest early exit).
 ```
 
+[GPT-6 Astra] A nested public query from an extension callback uses an independent child
+budget, including an unlimited budget when none is supplied; it temporarily shadows the outer
+budget rather than combining limits. After the child returns, returns an error, or unwinds,
+the outer scope resumes with its complete limits, cancellation handle, byte-accounting state,
+and any previously recorded budget error restored. Its deadline and cancellation are checked
+at the next outer poll. This does not interrupt arbitrary callback work or pool resource limits
+across nested queries; an aborting panic has no continuation.
+
 **Named-graph dataset view** (zero-copy restriction; a non-visible graph is indistinguishable from
 an absent one):
 
```

## Relevant complete budget/test context

```rust
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
        fn public_extension_nested_query_preserves_outer_cancel() {
            use oxrdf::{Literal, Term};
            use sparq_core::Graph;
            use std::sync::atomic::AtomicUsize;
            for inner in 0..5 {
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
                        3 => {
                            // Parse rejection occurs before a child budget is installed.
                            assert!(crate::query(&nested, "not SPARQL").is_err());
                        }
                        _ => {
                            // [GPT-6 Astra] This valid graph form reaches the error
                            // arm inside query_prepared_with_budget's budget scope.
                            let sparql = "CONSTRUCT { ?s ?p ?o } WHERE {}";
                            assert!(crate::PreparedQuery::parse(sparql).unwrap().is_graph_form());
                            let outer = ACTIVE.with(Cell::get);
                            assert_eq!(
                                crate::query(&nested, sparql).unwrap_err(),
                                "only SELECT and ASK queries are supported"
                            );
                            assert_state(outer, None);
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
        }```

## Public API and documentation context

```rust
/// A cooperative resource budget for one query evaluation (T15 server hardening).
///
/// The executor checks it at coarse sites only (operator entry, once per outer
/// iteration of the big scan/join loops), so enforcement is approximate but cheap:
/// an unlimited budget (the default) costs nothing on the hot paths. When a limit
/// trips, evaluation stops and the query fails with
/// `"query budget exceeded (timeout)"` / `"query budget exceeded (max-rows)"` /
/// `"query budget exceeded (max-bytes)"` / `"query budget exceeded (cancelled)"`.
///
/// [GPT-6 Astra] A nested engine call from a callback uses its own budget. The
/// outer budget resumes when that call returns (also after errors or unwind).
/// Its deadline and cancellation are checked at the next outer poll; this does
/// not interrupt arbitrary callback work or combine budgets across queries.
/// Executes a SPARQL query string against a graph, materialising the solutions.
pub fn query(graph: &Graph, sparql: &str) -> Result<QueryResult, String> {
    query_with_budget(graph, sparql, &QueryBudget::unlimited())
}

/// [`query`] under a cooperative [`QueryBudget`] (deadline / max result rows).
pub fn query_with_budget(graph: &Graph, sparql: &str, budget: &QueryBudget) -> Result<QueryResult, String> {
    query_prepared_with_budget(graph, &PreparedQuery::parse(sparql)?, budget)
}

/// [`query`] over a [`PreparedQuery`] — no per-execution parse.
pub fn query_prepared(graph: &Graph, prepared: &PreparedQuery) -> Result<QueryResult, String> {
    query_prepared_with_budget(graph, prepared, &QueryBudget::unlimited())
}

/// [`query_prepared`] under a cooperative [`QueryBudget`] (deadline / max result rows).
pub fn query_prepared_with_budget(
    graph: &Graph,
    prepared: &PreparedQuery,
    budget: &QueryBudget,
) -> Result<QueryResult, String> {
    let q = &prepared.query;
    let active = active_dataset(graph, q);
    let graph = active.as_ref().unwrap_or(graph);
    let _view_scope = view_scope(&active);
    exec::budget::with_budget(budget, || {
        exec::set_query_base(q.base_iri().map(|b| b.as_str()));
        match q {
            Query::Select { pattern, .. } => exec::eval_select(graph, pattern),
            // ASK as a QueryResult: zero variables, and one (empty) row iff the pattern
            // is satisfiable — the standard "unit row" encoding of a boolean result.
            Query::Ask { pattern, .. } => Ok(QueryResult {
                vars: Vec::new(),
                rows: if exec::eval_ask(graph, pattern)? { vec![Vec::new()] } else { Vec::new() },
            }),
            _ => Err("only SELECT and ASK queries are supported".into()),
        }
    })
}

```

## Structured execution observations

```json
[
  {
    "configuration": "default",
    "cargo_exit": 0,
    "result": "test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 328 filtered out; finished in 0.00s",
    "test_binary_sha256": "255b242546fad2fce6f95c6076424f94ea804dcbeac61a5c29b754291f4227e7",
    "bytes": 7995584,
    "production_features": [
      "default",
      "digest",
      "parallel",
      "regex"
    ],
    "new_public_case": "valid graph-form parse; exact execution Err inside query_prepared_with_budget; full parent-state equality; outer cancelled error and exact callback thread",
    "child_variants": 5,
    "no_default_qualification": null
  },
  {
    "configuration": "no-default",
    "cargo_exit": 0,
    "result": "test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 314 filtered out; finished in 0.00s",
    "test_binary_sha256": "3c1ddbdd8a21ef75888efaed7b76804709c33f965d11aad1d01ce825249dd1a1",
    "bytes": 5694000,
    "production_features": [],
    "new_public_case": "valid graph-form parse; exact execution Err inside query_prepared_with_budget; full parent-state equality; outer cancelled error and exact callback thread",
    "child_variants": 5,
    "no_default_qualification": "Engine defaults disabled; dev-dependency sparq-core still explicitly enables parallel/mmap/dict-spill."
  }
]
```

Local raw compiler/environment logs, full source copies, binaries and command receipts are retained separately; private host paths and raw logs are excluded here. No additional source context outside this narrow delta is resupplied; the prior frozen source/reviews remain required context for whole-change review.
