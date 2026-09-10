# Issue3105 final-head validation — focused evidence

Prior whole-change source review: ../admission/opus-final-result.json. No source patch is proposed here. The newly measured resource result requires engineering reassessment.

## report.json

```
{
  "author": "GPT-6 Astra xhigh",
  "head": "19763bfab1dce196a654c899b172e7b24d70bc59",
  "baseline": "e53464c73f31f7aca800f3867ac054c36408e346",
  "decision": "NO_GO for performance/memory admission under the predeclared local experiment screen; no correctness failure observed.",
  "scope": "One bounded exact-final validation phase, no production edits or commit. Test-only temporary insertion restored byte-for-byte; no remote/model operations. Previous evidence unchanged.",
  "allocation": {
    "samples": 12,
    "case_total_patterns": [
      5,
      8
    ],
    "repetitions": 3,
    "order_each_case": [
      "baseline/candidate",
      "candidate/baseline",
      "baseline/candidate"
    ],
    "subjects": 70000,
    "auxiliary_unrelated_triples": 8192,
    "views": [
      "ordinary base graph"
    ],
    "body": "every predicate shares ?s and ?o; all joins match; final arithmetic ?o + 0 < 0 rejects every row",
    "correctness": "Every sample passes independently generated false ASK oracle and uncapped result check. Separate positive query probe checks 70000 rows and each subject/object correspondence before tracing the residual-false ASK.",
    "calibration": "All twelve samples passed the unchanged allocator calibration after fixture/oracle, two query warmups and explicit Rayon broadcast barrier. No failed sample or retry.",
    "summary_file": "summary.json",
    "peak_growth": {
      "5": {
        "baseline": 10811556,
        "candidate": 15435090,
        "additional": 4623534,
        "fraction": 0.427647417263528,
        "screen_no_go": false
      },
      "8": {
        "baseline": 10813029,
        "candidate": 22157124,
        "additional": 11344095,
        "fraction": 1.0491135277635897,
        "screen_no_go": true
      }
    },
    "ranges": "Requested allocation counts, reallocations, requested bytes and peak live growth each have identical min/median/max across their three repetitions. Full RSS ranges and all raw samples are retained.",
    "tradeoff": "Cumulative requested allocation bytes decrease while live peak grows. The 8-pattern case violates BOTH >25% and >8MiB extra-peak criteria. Linear retention growth is enough to fail this screen; superlinear growth is not required.",
    "timing": "Harness retains its original elapsed-nanos output, but these are allocation-counted runs only. No timing claim or timing comparison is made for 19763.",
    "memory_limits": "Requested heap counts exclude allocator metadata and transient allocator internals. live_after is process-wide and includes setup. RSS is process high-water including fixture/oracle/warmup, not query heap. Small argv/source-name differences affect process live_after; query peak is baseline-relative."
  },
  "actual_path": {
    "tests": 2,
    "log": "path-and-zk.log",
    "source": "probe.rs inserted only within cfg(test) capped_rhs_tests; exact instrumented-exec.rs and original candidate-exec.rs frozen",
    "five_patterns": "work=[1,3,4,0], twelve RHS steps; four RHS relations reached in each of starts 0,1024,65536; four initial scans, eight cache reuses.",
    "eight_patterns": "work=[1,3,7,0], twenty-one RHS steps; seven RHS relations reached in each of starts 0,1024,65536; seven initial scans, fourteen cache reuses.",
    "branches": "Every observed step is merge, requested Some(0), actual s. No bind branch; successful positive joins ensure later relations are not vacuously skipped.",
    "limits": "Physical instrumentation executed in a native debug cfg(test) build with zk feature present but recorder unarmed for the 5/8 fixtures and normal engine test dependency feature unification. Allocation binaries are uninstrumented default production builds. Source guard is inactive while recorder unarmed; do not present debug work counts as release timing."
  },
  "zk": {
    "assessment": "The review finding omitted the earlier try_capped recorder guard. Its claimed capped-cache recording suppression is unreachable through the public query path while the recorder is armed; no new gate is needed.",
    "guard": "baseline-zk-guard.rs and candidate-zk-guard.rs are byte-identical: #[cfg(feature=\"zk\")] if crate::zk::enabled() { return Ok(None); } before view/match/capped dispatch. zk.rs module documentation explicitly disables capped shortcuts while recording.",
    "probe": "2000 seed rows, two shared-variable predicates, residual-false ASK. Unarmed work=[1,2,1,0]; armed work=[0,0,0,0], no capped steps; two complete 2000-triple witness input sets and filter obligation.",
    "exact_main_final_witness": {
      "equal": true,
      "baseline_sha256": "4c512c5daec0c921b726b9cd15ece3c9860b10527059e56bc501492a541e6d9b",
      "candidate_sha256": "4c512c5daec0c921b726b9cd15ece3c9860b10527059e56bc501492a541e6d9b",
      "bytes": 1229030
    },
    "witness_method": "Identical standalone Rust harness against unmodified main and final, release default+zk, one Rayon worker, same input/query. Full drained ZkTrace Debug bytes compare equal. This is a fixture-specific trace comparison, not cryptographic soundness evidence.",
    "existing_tests": {
      "unit": 3,
      "integration": 17,
      "integration_includes": "differential_zk_on_equals_off_10k plus exact operator trace tests"
    },
    "all_passed": true
  },
  "source_and_resources": {
    "no_new_commit": true,
    "final_clean": true,
    "baseline_clean": true,
    "final_exec_restored": true,
    "free_bytes_finish": 10944319488,
    "min_free_limit": 6509559808,
    "jobs": 2,
    "incremental": false,
    "offline_locked": true,
    "all_commands_completed": true,
    "finished_utc": "2026-09-10T02:27:18.732853+00:00"
  },
  "next_step": "Root engineering reassessment of a bounded RHS retention policy or reduced reuse scope, preserving bags/sort/budget/zk guards. Do not implement an arbitrary cap in this phase. Subsequent candidate requires focused review and the same many-RHS allocation controls before any admission.",
  "remaining": "Authoritative remote full-workspace/feature/conformance/wasm/performance gates remain separate; their success cannot erase this measured memory tradeoff. No other optional review suggestions were implemented."
}

```

## protocol.json

```
{
  "declared_utc": "2026-09-10T02:20:27.780731+00:00",
  "deadline_utc": "2026-09-10T02:38:59Z",
  "source": {
    "baseline": "e53464c73f31f7aca800f3867ac054c36408e346",
    "candidate": "19763bfab1dce196a654c899b172e7b24d70bc59"
  },
  "matrix": {
    "total_patterns": [
      5,
      8
    ],
    "seeds": 70000,
    "views": [
      "base"
    ],
    "joins": "all predicates share s and o; every join matches; arithmetic residual o+0<0 rejects all final rows",
    "allocation_repetitions": 3,
    "order": "AB, BA, AB per case, A=baseline B=candidate",
    "warmup": 2,
    "queries_per_sample": 1,
    "calibration": "unchanged counting.rs; after fixture/full oracle/two warmups and rayon broadcast barrier"
  },
  "metrics": [
    "requested allocations",
    "requested bytes",
    "peak requested live growth",
    "process-wide live_after",
    "cumulative process max RSS at setup/before/after"
  ],
  "timing_claim": false,
  "screen": "NO_GO pending engineering reassessment if candidate extra peak >25% AND >8MiB; experiment criterion, not repo policy. All results reported, no tuning/retries.",
  "path": "test-only actual same fixture probe if affordable; clearly separate instrumented/prod hashes.",
  "zk": "capture exact unchanged early try_capped guard; existing zk tests and armed/unarmed >1024 seed probe if affordable; no redundant production gate.",
  "limits": {
    "seconds": 1200,
    "min_free": 6509559808,
    "jobs": 2,
    "incremental": false,
    "offline_locked": true,
    "no_production_changes": true
  }
}

```

## summary.json

```
{
  "5": {
    "baseline": {
      "allocs": {
        "min": 70783,
        "median": 70783,
        "max": 70783
      },
      "reallocs": {
        "min": 1464,
        "median": 1464,
        "max": 1464
      },
      "requested_bytes": {
        "min": 160108511,
        "median": 160108511,
        "max": 160108511
      },
      "peak_live_growth": {
        "min": 10811556,
        "median": 10811556,
        "max": 10811556
      },
      "live_after": {
        "min": 34089883,
        "median": 34089883,
        "max": 34089883
      },
      "setup_peak_rss": {
        "min": 59752448,
        "median": 59768832,
        "max": 61882368
      },
      "before_peak_rss": {
        "min": 62930944,
        "median": 62930944,
        "max": 65060864
      },
      "after_peak_rss": {
        "min": 62930944,
        "median": 62947328,
        "max": 65077248
      }
    },
    "candidate": {
      "allocs": {
        "min": 70592,
        "median": 70592,
        "max": 70592
      },
      "reallocs": {
        "min": 696,
        "median": 696,
        "max": 696
      },
      "requested_bytes": {
        "min": 75084031,
        "median": 75084031,
        "max": 75084031
      },
      "peak_live_growth": {
        "min": 15435090,
        "median": 15435090,
        "max": 15435090
      },
      "live_after": {
        "min": 34089884,
        "median": 34089884,
        "max": 34089884
      },
      "setup_peak_rss": {
        "min": 59703296,
        "median": 59752448,
        "max": 61882368
      },
      "before_peak_rss": {
        "min": 67436544,
        "median": 67502080,
        "max": 69632000
      },
      "after_peak_rss": {
        "min": 67436544,
        "median": 67502080,
        "max": 69648384
      }
    },
    "additional_peak_bytes": 4623534,
    "additional_peak_fraction": 0.427647417263528,
    "screen_no_go": false
  },
  "8": {
    "baseline": {
      "allocs": {
        "min": 71210,
        "median": 71210,
        "max": 71210
      },
      "reallocs": {
        "min": 2427,
        "median": 2427,
        "max": 2427
      },
      "requested_bytes": {
        "min": 270124930,
        "median": 270124930,
        "max": 270124930
      },
      "peak_live_growth": {
        "min": 10813029,
        "median": 10813029,
        "max": 10813029
      },
      "live_after": {
        "min": 52432201,
        "median": 52432201,
        "max": 52432201
      },
      "setup_peak_rss": {
        "min": 86081536,
        "median": 86081536,
        "max": 86130688
      },
      "before_peak_rss": {
        "min": 89243648,
        "median": 89243648,
        "max": 89292800
      },
      "after_peak_rss": {
        "min": 89276416,
        "median": 89276416,
        "max": 89341952
      }
    },
    "candidate": {
      "allocs": {
        "min": 70874,
        "median": 70874,
        "max": 70874
      },
      "reallocs": {
        "min": 1083,
        "median": 1083,
        "max": 1083
      },
      "requested_bytes": {
        "min": 121330504,
        "median": 121330504,
        "max": 121330504
      },
      "peak_live_growth": {
        "min": 22157124,
        "median": 22157124,
        "max": 22157124
      },
      "live_after": {
        "min": 52432202,
        "median": 52432202,
        "max": 52432202
      },
      "setup_peak_rss": {
        "min": 86097920,
        "median": 86114304,
        "max": 86147072
      },
      "before_peak_rss": {
        "min": 100581376,
        "median": 100581376,
        "max": 100614144
      },
      "after_peak_rss": {
        "min": 100597760,
        "median": 100614144,
        "max": 100630528
      }
    },
    "additional_peak_bytes": 11344095,
    "additional_peak_fraction": 1.0491135277635897,
    "screen_no_go": true
  }
}

```

## candidate-zk-guard.rs

```
fn try_capped(
    graph: &Graph,
    local: &mut LocalVocab,
    inner: &GraphPattern,
    cap: usize,
) -> Result<Option<Bindings>, String> {
    // zk-trace: early termination would consume only part of each scan range,
    // recording a TRUNCATED input set — but the completeness witness (the
    // linear-sweep circuit) must see the whole scan range. Disable the cap
    // while recording; the full path is result-equivalent (LIMIT is
    // order-insensitive without ORDER BY).
    #[cfg(feature = "zk")]
    if crate::zk::enabled() {
        return Ok(None);
    }

```

## probe.rs

```

    // [GPT-6 Astra] Temporary validation sidecar; not part of the production commit.
    #[test]
    fn final_validation_many_rhs() {
        for n in [5usize, 8] {
            let mut ttl = String::new();
            for i in 0..70_000 {
                for j in 0..n {
                    ttl.push_str(&format!("<urn:s:{i}> <urn:p{j}> {i} .\n"));
                }
            }
            for i in 0..8192 {
                ttl.push_str(&format!("<urn:d:{i}> <urn:dead> <urn:o> .\n"));
            }
            let graph = Graph::load_str(&ttl, "turtle").unwrap();
            let patterns: String = (0..n).map(|j| format!("?s <urn:p{j}> ?o . ")).collect();
            let positive = crate::query(&graph, &format!("SELECT ?s ?o WHERE {{ {patterns} }}")).unwrap();
            assert_eq!(positive.rows.len(), 70_000);
            for row in &positive.rows {
                let o = row[1].as_ref().unwrap().to_string();
                let i = o.split('"').nth(1).unwrap().parse::<usize>().unwrap();
                assert!(i < 70_000);
                assert_eq!(row[0].as_ref().unwrap().to_string(), format!("<urn:s:{i}>"));
            }
            take_work();
            let (answer, steps) = trace(|| crate::ask(&graph, &format!("ASK {{ {patterns} FILTER(?o + 0 < 0) }}")).unwrap());
            assert!(!answer);
            let work = take_work();
            assert_eq!(work, [1, 3, n-1, 0]);
            assert_eq!(steps.len(), 3*(n-1));
            for start in [0usize, 1024, 65536] {
                let reached: Vec<_> = steps.iter().filter(|s| s.start == start).collect();
                assert_eq!(reached.len(), n-1);
                assert!(reached.iter().all(|s| s.kernel != "bind"));
                let ids: std::collections::BTreeSet<_> = reached.iter().map(|s| s.pattern).collect();
                assert_eq!(ids.len(), n-1);
            }
            println!("patterns={n} work={work:?} steps={steps:?}");
        }
    }

    #[cfg(feature = "zk")]
    #[test]
    fn final_validation_armed_zk_excludes_cache() {
        let mut ttl = String::new();
        for i in 0..2000 {
            ttl.push_str(&format!("<urn:s:{i}> <urn:p> {i} ; <urn:q> {i} .\n"));
        }
        let graph = Graph::load_str(&ttl, "turtle").unwrap();
        let q = "ASK { ?s <urn:p> ?o . ?s <urn:q> ?o . FILTER(?o + 0 < 0) }";
        take_work();
        assert!(!crate::ask(&graph, q).unwrap());
        let unarmed = take_work();
        assert_eq!(unarmed, [1,2,1,0]);
        let guard = crate::zk::install();
        let (answer, steps) = trace(|| crate::ask(&graph, q).unwrap());
        let armed = take_work();
        assert!(!answer);
        assert_eq!(armed, [0;4]);
        assert!(steps.is_empty());
        let witness = crate::zk::take();
        assert_eq!(witness.patterns.len(), 2);
        assert!(witness.patterns.iter().all(|p| p.triples.len() == 2000));
        assert!(witness.first_uncaptured().is_none());
        assert!(!witness.filters.is_empty());
        assert!(witness.filters.iter().flat_map(|f| &f.rows).all(|(_,pass)| !pass));
        println!("unarmed={unarmed:?} armed={armed:?} patterns={} triple_counts={:?} steps={:?}",witness.patterns.len(), witness.patterns.iter().map(|p|p.triples.len()).collect::<Vec<_>>(),witness.steps);
        drop(guard);
    }

```

## path-and-zk.log

```
   Compiling sparq-engine v0.1.1 (/private/tmp/sparq-pr6049/.throughput-monitor/worktrees/issue3105/crates/sparq-engine)
    Finished `test` profile [unoptimized + debuginfo] target(s) in 10.34s
     Running unittests src/lib.rs (/private/tmp/sparq-pr6049/.throughput-monitor/direct-5983/implementation/target/debug/deps/sparq_engine-d95f3e0da8293817)

running 2 tests
test exec::capped_rhs_tests::final_validation_armed_zk_excludes_cache ... unarmed=[1, 2, 1, 0] armed=[0, 0, 0, 0] patterns=2 triple_counts=[2000, 2000] steps=[Scan { pattern: 0 }, Scan { pattern: 1 }, Filter { filter: 0 }]
ok
test exec::capped_rhs_tests::final_validation_many_rhs ... patterns=5 work=[1, 3, 4, 0] steps=[Step { start: 0, pattern: 1, kernel: "merge", requested: Some(0), scanned: true, actual: Some("s") }, Step { start: 0, pattern: 2, kernel: "merge", requested: Some(0), scanned: true, actual: Some("s") }, Step { start: 0, pattern: 3, kernel: "merge", requested: Some(0), scanned: true, actual: Some("s") }, Step { start: 0, pattern: 4, kernel: "merge", requested: Some(0), scanned: true, actual: Some("s") }, Step { start: 1024, pattern: 1, kernel: "merge", requested: Some(0), scanned: false, actual: Some("s") }, Step { start: 1024, pattern: 2, kernel: "merge", requested: Some(0), scanned: false, actual: Some("s") }, Step { start: 1024, pattern: 3, kernel: "merge", requested: Some(0), scanned: false, actual: Some("s") }, Step { start: 1024, pattern: 4, kernel: "merge", requested: Some(0), scanned: false, actual: Some("s") }, Step { start: 65536, pattern: 1, kernel: "merge", requested: Some(0), scanned: false, actual: Some("s") }, Step { start: 65536, pattern: 2, kernel: "merge", requested: Some(0), scanned: false, actual: Some("s") }, Step { start: 65536, pattern: 3, kernel: "merge", requested: Some(0), scanned: false, actual: Some("s") }, Step { start: 65536, pattern: 4, kernel: "merge", requested: Some(0), scanned: false, actual: Some("s") }]
patterns=8 work=[1, 3, 7, 0] steps=[Step { start: 0, pattern: 1, kernel: "merge", requested: Some(0), scanned: true, actual: Some("s") }, Step { start: 0, pattern: 2, kernel: "merge", requested: Some(0), scanned: true, actual: Some("s") }, Step { start: 0, pattern: 3, kernel: "merge", requested: Some(0), scanned: true, actual: Some("s") }, Step { start: 0, pattern: 4, kernel: "merge", requested: Some(0), scanned: true, actual: Some("s") }, Step { start: 0, pattern: 5, kernel: "merge", requested: Some(0), scanned: true, actual: Some("s") }, Step { start: 0, pattern: 6, kernel: "merge", requested: Some(0), scanned: true, actual: Some("s") }, Step { start: 0, pattern: 7, kernel: "merge", requested: Some(0), scanned: true, actual: Some("s") }, Step { start: 1024, pattern: 1, kernel: "merge", requested: Some(0), scanned: false, actual: Some("s") }, Step { start: 1024, pattern: 2, kernel: "merge", requested: Some(0), scanned: false, actual: Some("s") }, Step { start: 1024, pattern: 3, kernel: "merge", requested: Some(0), scanned: false, actual: Some("s") }, Step { start: 1024, pattern: 4, kernel: "merge", requested: Some(0), scanned: false, actual: Some("s") }, Step { start: 1024, pattern: 5, kernel: "merge", requested: Some(0), scanned: false, actual: Some("s") }, Step { start: 1024, pattern: 6, kernel: "merge", requested: Some(0), scanned: false, actual: Some("s") }, Step { start: 1024, pattern: 7, kernel: "merge", requested: Some(0), scanned: false, actual: Some("s") }, Step { start: 65536, pattern: 1, kernel: "merge", requested: Some(0), scanned: false, actual: Some("s") }, Step { start: 65536, pattern: 2, kernel: "merge", requested: Some(0), scanned: false, actual: Some("s") }, Step { start: 65536, pattern: 3, kernel: "merge", requested: Some(0), scanned: false, actual: Some("s") }, Step { start: 65536, pattern: 4, kernel: "merge", requested: Some(0), scanned: false, actual: Some("s") }, Step { start: 65536, pattern: 5, kernel: "merge", requested: Some(0), scanned: false, actual: Some("s") }, Step { start: 65536, pattern: 6, kernel: "merge", requested: Some(0), scanned: false, actual: Some("s") }, Step { start: 65536, pattern: 7, kernel: "merge", requested: Some(0), scanned: false, actual: Some("s") }]
ok

test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 328 filtered out; finished in 8.41s


```

## existing-zk-unit.log

```
    Finished `test` profile [unoptimized + debuginfo] target(s) in 2.42s
     Running unittests src/lib.rs (/private/tmp/sparq-pr6049/.throughput-monitor/direct-5983/implementation/target/debug/deps/sparq_engine-d95f3e0da8293817)

running 3 tests
test zk::tests::guard_clears_state ... ok
test zk::tests::query_records_post_scan_input_sets ... ok
test zk::tests::trace_is_deterministic_across_runs ... ok

test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 327 filtered out; finished in 0.00s


```

## existing-zk-integration.log

```
   Compiling sparq-engine v0.1.1 (/private/tmp/sparq-pr6049/.throughput-monitor/worktrees/issue3105/crates/sparq-engine)
    Finished `test` profile [unoptimized + debuginfo] target(s) in 7.04s
     Running tests/differentials.rs (/private/tmp/sparq-pr6049/.throughput-monitor/direct-5983/implementation/target/debug/deps/differentials-5b8110d85254d592)

running 17 tests
test zk_trace_differential::differential_zk_on_equals_off_10k ... ok
test zk_trace_operators::ask_disables_count_pushdown_and_captures_input ... ok
test zk_trace_operators::bgp_chain_join_records_both_sides ... ok
test zk_trace_operators::bgp_single_pattern_exact_set ... ok
test zk_trace_operators::distinct_records_boundary_pre_reduction ... ok
test zk_trace_operators::exists_inner_scans_tagged_and_suppressed ... ok
test zk_trace_operators::filter_records_obligation_with_verdicts ... ok
test zk_trace_operators::graph_const_single_boundary_and_tag ... ok
test zk_trace_operators::graph_var_one_boundary_per_iteration ... ok
test zk_trace_operators::group_aggregation_records_pre_aggregation_input ... ok
test zk_trace_operators::limit_disables_early_termination ... ok
test zk_trace_operators::minus_records_boundary ... ok
test zk_trace_operators::optional_records_boundary_and_both_sides ... ok
test zk_trace_operators::property_path_marks_uncaptured ... ok
test zk_trace_operators::trace_is_deterministic_across_runs ... ok
test zk_trace_operators::union_records_boundary_and_both_branches ... ok
test zk_trace_operators::unsatisfiable_pattern_records_empty_set ... ok

test result: ok. 17 passed; 0 failed; 0 ignored; 0 measured; 13 filtered out; finished in 5.97s


```

## zk-comparison.json

```
{
  "equal": true,
  "baseline_sha256": "4c512c5daec0c921b726b9cd15ece3c9860b10527059e56bc501492a541e6d9b",
  "candidate_sha256": "4c512c5daec0c921b726b9cd15ece3c9860b10527059e56bc501492a541e6d9b",
  "bytes": 1229030
}

```

## harness-vs-prior.diff

```
--- frozen-quiescent-harness
+++ 5-and-8-pattern-harness
@@ -9,10 +9,10 @@
 fn iri(s: &str) -> Term {
     NamedNode::new(s).unwrap().into()
 }
-fn fixture(overlay: bool) -> Graph {
+fn fixture(overlay: bool, n: usize) -> Graph {
     let mut ttl = String::new();
     for i in 0..70_000 {
-        writeln!(ttl, "<urn:s:{i}> <urn:p> {i} ; <urn:q> {i} ; <urn:r> {i} .").unwrap();
+        for j in 0..n { writeln!(ttl, "<urn:s:{i}> <urn:p{j}> {i} .").unwrap(); }
     }
     for i in 0..8192 {
         writeln!(ttl, "<urn:d:{i}> <urn:dead> <urn:o> .").unwrap();
@@ -62,8 +62,10 @@
     let args: Vec<String> = std::env::args().collect();
     let case = &args[1];
     let overlay = args[2] == "overlay";
-    let graph = fixture(overlay);
-    let pid = graph.dict.lookup(&iri("urn:p"));
+    let n: usize = case.parse().unwrap();
+    assert!(n == 5 || n == 8);
+    let graph = fixture(overlay, n);
+    let pid = graph.dict.lookup(&iri("urn:p0"));
     let scan = graph.store.scan_sorted(&[None, Some(pid), None], 0);
     assert_eq!(scan.rows.len(), 70_000);
     assert_eq!(scan.perm.order().into_iter().find(|&c| c != 1), Some(0));
@@ -74,11 +76,7 @@
         .to_string();
     let threshold = value.split('"').nth(1).unwrap().parse::<usize>().unwrap();
     drop(scan);
-    let patterns = if case == "multi" {
-        "?s <urn:p> ?o . ?s <urn:q> ?o . ?s <urn:r> ?o ."
-    } else {
-        "?s <urn:p> ?o . ?s <urn:q> ?o ."
-    };
+    let patterns: String = (0..n).map(|j| format!("?s <urn:p{j}> ?o . ")).collect();
     let filter = if case == "first" || case == "second" {
         format!("?o + 0 = {threshold}")
     } else if case == "late" {

```

Complete raw samples, paired witness bytes, exact binaries, source snapshots, dependency locks, test-body context, command receipts and hashes are in the accompanying manifest. The packet omits the already-reviewed full engine body and binary/log bulk; it makes no additional algorithm or timing claim.
