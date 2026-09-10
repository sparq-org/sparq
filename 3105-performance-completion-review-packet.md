# Issue3105 exact-candidate local performance completion

Actual GPT-6 Astra xhigh. Candidate6e86f1d0ba447aa78a50800337a7a379702fa999 against main e53464c73f31f7aca800f3867ac054c36408e346; both worktrees clean. No production source changed. Actual Opus source verdict already APPROVE_FOR_VALIDATION; this packet does not claim admission.

The fixed timing and requested-allocation screen is positive. Admission remains incomplete until path-observed coverage and authoritative full gates. The table uses all100 original timing samples and all60 amended allocation samples. Original59 successful allocation samples plus one startup calibration failure are preserved separately; no replacement/discarding of that historical evidence.

Local advisory screening; timing uses original unchanged System binaries (five samples, three queries each). Allocation columns use the amended complete matrix (three samples, one query each).

| Case / view | Baseline ms/query range | Candidate ms/query range | C/B median | Timing CV B/C % | Requested bytes B/C | Peak requested live growth B/C | Allocations B/C | Reallocations B/C |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| first / base | 1.7761–1.8458 | 1.7668–1.8288 | 0.9948 | 1.67 / 1.45 | 21,370,986 / 21,371,162 | 8,712,554 / 8,712,730 | 1,242 / 1,243 | 200 / 200 |
| first / overlay | 1.7977–1.8576 | 1.8147–1.8524 | 1.0019 | 1.43 / 0.88 | 21,370,986 / 21,371,162 | 8,712,554 / 8,712,730 | 1,242 / 1,243 | 200 / 200 |
| second / base | 11.2952–11.5878 | 10.4790–10.6593 | 0.9326 | 0.98 / 0.69 | 38,767,905 / 28,139,966 | 10,777,001 / 8,712,742 | 65,828 / 65,805 | 390 / 294 |
| second / overlay | 11.3322–11.4750 | 10.3481–10.4684 | 0.9172 | 0.49 / 0.54 | 38,767,905 / 28,139,966 | 10,777,001 / 8,712,742 | 65,828 / 65,805 | 390 / 294 |
| miss / base | 12.2525–12.6864 | 10.6167–11.4521 | 0.8549 | 1.60 / 3.10 | 50,090,494 / 28,834,440 | 10,776,989 / 8,712,730 | 70,347 / 70,300 | 497 / 305 |
| miss / overlay | 12.4730–12.7045 | 10.5019–11.1092 | 0.8435 | 0.66 / 2.26 | 50,090,494 / 28,834,440 | 10,776,989 / 8,712,730 | 70,347 / 70,300 | 497 / 305 |
| late / base | 22.6154–23.2352 | 20.7066–21.2647 | 0.9279 | 1.06 / 0.99 | 70,275,472 / 49,019,418 | 15,851,195 / 15,851,195 | 332,467 / 332,420 | 497 / 305 |
| late / overlay | 22.7286–23.4327 | 20.8689–21.3586 | 0.9129 | 1.21 / 0.95 | 70,275,472 / 49,019,418 | 15,851,195 / 15,851,195 | 332,467 / 332,420 | 497 / 305 |
| multi / base | 15.8299–16.3201 | 12.6078–12.8911 | 0.7888 | 1.17 / 0.84 | 86,762,344 / 44,250,148 | 10,810,085 / 10,953,245 | 70,499 / 70,404 | 818 / 434 |
| multi / overlay | 16.1594–16.4652 | 12.5823–12.8839 | 0.7713 | 0.79 / 0.96 | 86,762,344 / 44,250,148 | 10,810,085 / 10,953,245 | 70,499 / 70,404 | 818 / 434 |

Full min/max/median/mean/sample-standard-deviation/CV and cumulative setup/before/after RSS ranges are in summary.json. Count metrics are identical across the three completed amended repetitions of every case. live_after is total process requested live bytes after the query, not query-retained bytes: the harness does not emit the begin baseline. Peak live growth subtracts that actual baseline internally. RSS is cumulative process high-water; setup includes the full oracle, and cannot isolate query physical memory.

## Exact interpretation and remaining work

{
  "completed_utc": "2026-09-10T01:24:15.069137+00:00",
  "scope": "Single fixed local diagnostic of unchanged candidate; no production changes, tuning, supplementary coverage, remote actions or admission.",
  "candidate_head": "6e86f1d0ba447aa78a50800337a7a379702fa999",
  "baseline_head": "e53464c73f31f7aca800f3867ac054c36408e346",
  "author": "GPT-6 Astra xhigh (actual implementation runtime)",
  "decision": "Positive local timing/heap screen supports proceeding to bounded coverage and full validation. Performance admission remains INCONCLUSIVE pending physical-path gaps and authoritative gates; no merge/issue-closure claim.",
  "counts": {
    "timing_successes": 100,
    "original_allocation_successes": 59,
    "original_allocation_startup_failures": 1,
    "amended_allocation_successes": 60,
    "amended_allocation_failures": 0,
    "measurement_commands": 220
  },
  "failure": {
    "command": {
      "name": "count-multi-overlay-2-candidate",
      "argv": [
        "/private/tmp/sparq-pr6049/.throughput-monitor/direct-3105/performance-completion/binary/candidate-count",
        "multi",
        "overlay",
        "2"
      ],
      "binary_sha256": "8496cf1d66d48c631f1226d987dc7c415e18fbc8ca2b8d9df8fdc5ee4ed5b221",
      "exit": 101,
      "seconds": 0.018918167000002484,
      "order": 159,
      "free_bytes": 14449561600
    },
    "phase": "Startup calibration immediately after build_global, before fixture/query or measured workload.",
    "observed": [
      2,
      1,
      448,
      320,
      12804
    ],
    "expected": [
      2,
      1,
      448,
      320,
      10500
    ],
    "interpretation": "LIVE differed by 2304 bytes while alloc/realloc/request/peak counts matched. Consistent with worker startup outside ACTIVE; precise cause not proven. Not an optimizer failure or established allocator bug.",
    "response": "Root-authorized predeclared harness correction moved unchanged calibration after fixture/oracle/two warmups and explicit global broadcast barrier; reran the entire allocation matrix once, not the failed point alone. All original evidence retained."
  },
  "findings": [
    "Full miss candidate/base median latency ratios 0.8549 base and 0.8435 overlay; two-RHS residual-false ratios 0.7888 and 0.7713. All four timing ranges disjoint.",
    "First-hit ratios 0.9948 and 1.0019 with overlapping ranges; second-hit and late-LIMIT improved in this fixture. No observed predeclared favorable-case regression.",
    "Two-RHS peak requested live growth increased 143160 bytes (1.3243%), far below this experiment's 25% AND 8MiB screen. Sum-of-RHS retention with more/larger steps remains unbounded by this fixture.",
    "Completed original and amended allocations/reallocations/requested bytes/peak live measurements agree exactly at every point; amended three repetitions agree exactly.",
    "All successful processes passed generated oracle and query result assertions. All100 timing processes and all60 amended allocation processes succeeded. This is local advisory screening, not statistical or canonical published performance proof."
  ],
  "measurement_limits": [
    "Original timing binaries and measured block unchanged; allocation amendment has its own source and binary hashes. No timing claims from count-instrumented durations.",
    "Whole-query public ASK/query calls are timed, including parsing/planning. Graph/overlay construction, uncapped oracle and two warmups are excluded. Query-local RHS reuse cache is cold on every query; persistent graph and code paths are warm.",
    "Only two maximum reached RHS relations, 70000 seed rows, one thread, one macOS arm64 host and one default feature configuration were measured; no cold-process, concurrent query, many-RHS or variable-cardinality sweep.",
    "Allocator metrics count requested bytes, excluding allocator metadata and transient realloc internals. Query peak is growth over actual begin baseline. live_after is total process live bytes, not retained-query growth because begin baseline is not emitted.",
    "RSS is cumulative process high-water and setup includes oracle; it cannot isolate physical query memory. In amended multi-overlay samples candidate after-RSS 49266688\u201349299456 exceeds baseline47054848\u201347104000 despite modest requested peak growth. No claim that RSS equals heap.",
    "Broadcast plus complete warmups and successful calibration reduce demonstrated startup noise; they do not formally prove absence of all external/background activity. Original failure retained.",
    "Five paired timing repetitions with min/max, sample standard deviation and CV are advisory; alternatingAB/BA reduces order bias but does not remove host noise. No retry/noise hunting beyond one expressly authorized complete allocation rerun.",
    "Generic process listing and hardware sysctl were sandbox-denied, without retry; exact Cargo-lock lsof had no owners before build. CPU model/RAM metadata not independently collected; uname and rustc target are recorded."
  ],
  "path_and_semantic_limits": [
    "Measured queries retain two shared variables, arithmetic residual filter and actual subject-ordered seed scan; source route is ASK/SLICE -> try_capped -> eval_bgp_binary_capped. Prior phase1 compiled counters established this shape on a related 70000-row fixture, not a new physical trace for each performance point.",
    "Three-pattern multi uses duplicate {s,o} variable edges, which GYO reduction classifies acyclic/binary. Every generated subject/object matches both RHS relations; only final arithmetic residual is false. It reaches both RHS relations by source/data construction; no first-join miss makes retention vacuous.",
    "LIMIT65537 checks count and every row's generated identity; oracle uses uncapped query and generated per-row identity. Limited-row uniqueness/multiset equality is not separately checked in this harness.",
    "Actual changing scan_sort across blocks, mixed bind/non-bind kernels, disconnected cross, restricted permutations, named graphs and residual EXISTS remain pending explicit path-observed coverage. Ordinary base/fork with unrelated tombstones are measured here.",
    "Phase1 budget/row/cancellation controls remain unchanged. Root identified separate baseline nested public-query budget::Guard issue6476 by source review, without claimed runtime reproduction; no fix included here."
  ],
  "next_smallest_step": "Root-authorized later phase: add actual changing-sort/mixed-kernel/disconnected-cross path-observed tests, then relevant feature checks; evaluate any slot-release-before-replacement change separately on exact delta. Obtain full authoritative workspace/wasm/perf ratchet gates before admission. No further work started in this phase.",
  "resources": {
    "phase_start_utc": "2026-09-10T01:10:55.552996Z",
    "phase_deadline_utc": "2026-09-10T01:30:55Z",
    "free_bytes_at_freeze": 13478789120,
    "floor_bytes": 6509559808,
    "build_jobs": 2,
    "incremental": false,
    "offline_locked": true,
    "commands_pending": false,
    "cache": "direct-5983/implementation/target is regenerable private cache only; no cache cleanup occurred"
  },
  "prior_frozen_verification": [
    {
      "manifest": "/private/tmp/sparq-pr6049/.throughput-monitor/direct-3105/manifest.json",
      "sha256": "188a6cffeb664756aec40923d18da3ea905ca217b76f654fcb42e7a863676f48",
      "verified_files": 59
    },
    {
      "manifest": "/private/tmp/sparq-pr6049/.throughput-monitor/direct-3105/performance/manifest.json",
      "sha256": "9cd9164058fa95e142633787c30cd9d00a65463e975de28f3b46166503f87bed",
      "verified_files": 21
    }
  ]
}

## Predeclared protocol and one authorized amendment

{
  "declared_utc": "2026-09-10T00:51:27.182048+00:00",
  "phase_minutes": 20,
  "source": {
    "baseline": "e53464c73f31f7aca800f3867ac054c36408e346",
    "candidate": "6e86f1d0ba447aa78a50800337a7a379702fa999"
  },
  "profiles": {
    "release": "existing bench/overlay-count convention: opt-level3, debugfalse,ltofalse,codegen-units16",
    "timing": "System allocator, no counting module",
    "allocations": "byte-identical fixture/workload with existing owner-guard counting.rs, calibration before measurement"
  },
  "matrix": {
    "subjects": 70000,
    "query_predicates": 3,
    "auxiliary_triples": 8192,
    "views": [
      "base",
      "fork with4096unrelated tombstones"
    ],
    "cases": [
      "ASK first seed row hit",
      "ASK seed position2048hit",
      "ASK all residual-filter miss",
      "SELECT LIMIT65537 with residual true filter",
      "ASK three-pattern/two-RHS residual-filter miss"
    ]
  },
  "protocol": {
    "rayon_threads": 1,
    "warmups": 2,
    "timing_repetitions": 5,
    "queries_per_timing_sample": 3,
    "allocation_repetitions": 3,
    "queries_per_allocation_sample": 1,
    "order": "Five paired samples per case: AB,BA,AB,BA,AB (A=baseline B=candidate). Each sample process runs its own setup/oracle/two warmups before one recorded sample. No concurrent builds or measurement.",
    "correctness": "exact ASK boolean; LIMIT row count and each returned binding checked against generated subject/object identity; full-result oracle outside timed windows",
    "path_evidence": "derive seed subject-order from actual scan; preserve arithmetic residual filter/two shared vars. Prior actual-query phase1 witness establishes this capped shape. Additional per-case physical instrumentation only if affordable; absence must remain explicit.",
    "rss": "cumulative process high-water only, report separately at setup/before/after; never equate to query heap"
  },
  "admission": {
    "correctness": "all paired answers must satisfy generated oracle; otherwise NO-GO",
    "benefit": "at least full-miss or two-RHS miss median improves10percent with disjoint min/max ranges",
    "regression": "any favorable/second-hit/late-LIMIT median regression>10percent with disjoint ranges is NO-GO",
    "memory": "Conservative experiment screen: >25percent and >8MiB additional peak query live heap is NO-GO for this candidate pending root engineering evaluation; not repository policy or owner-approval requirement.",
    "uncertainty": "Five-sample min/max and medians are advisory screening, not statistical proof or canonical admission; missing exact path/resource evidence is inconclusive.",
    "no_tuning": true
  },
  "resource": {
    "initial_free_bytes": 6779858944,
    "minimum_free_bytes": 6509559808,
    "build_jobs": 2,
    "incremental": false,
    "offline_locked": true,
    "build_cap_seconds": 600,
    "no_installs": true,
    "no_cache_cleanup": true
  },
  "restricted_order": "affordable correctness-only assessment; no new feature build if reserve insufficient",
  "amendment_before_any_measured_samples": {
    "utc": "2026-09-10T00:53:43.039716+00:00",
    "reason": "Root requested balanced paired order and clarified screening thresholds before measurements",
    "samples_recorded": 0
  }
}

{
  "declared_utc": "2026-09-10T01:18:50.135042+00:00",
  "reason": "Authorized single full allocation rerun after one original startup calibration failed before fixture/query. Keep original 59 successes and one failure. No timing rerun.",
  "original_successful_count_samples": 59,
  "original_count_commands": 60,
  "new_count_samples_before_declaration": 0,
  "change": "Move unchanged counting::calibrate after fixture, full oracle, and two unchanged query warmups; execute global rayon::broadcast barrier before calibration. Counting module and measured query body unchanged.",
  "matrix": "Same all ten case/view points, three pairs each, AB/BA/AB; stop on any calibration failure, no further retries.",
  "limit": "Within original 20-minute phase ending 2026-09-10T01:30:55Z; 2 build jobs, incremental false, offline locked, disk floor 6509559808 bytes."
}

## Harness delta (both comparison sources identical)

```diff
--- baseline/old-main.rs
+++ baseline/quiescent-main.rs
@@ -59,8 +59,6 @@
         .num_threads(1)
         .build_global()
         .unwrap();
-    #[cfg(feature = "count-alloc")]
-    counting::calibrate();
     let args: Vec<String> = std::env::args().collect();
     let case = &args[1];
     let overlay = args[2] == "overlay";
@@ -135,6 +133,12 @@
     for _ in 0..2 {
         assert_eq!(run(&graph, &q, select), expected);
     }
+    #[cfg(feature = "count-alloc")]
+    {
+        // Complete worker startup after fixture/oracle and both query warmups.
+        rayon::broadcast(|_| ());
+        counting::calibrate();
+    }
     for _ in 0..reps {
         let rep = sample;
         let before_rss = rss();
```

## Full final allocation harness; timing harness differs only by the shown delta

```rust
//! [GPT-6 Astra] Fixed local whole-query diagnostic for issue3105; not canonical.
#[cfg(feature = "count-alloc")]
mod counting;
use oxrdf::{NamedNode, Term};
use sparq_core::Graph;
use sparq_engine::{ask, query};
use std::{fmt::Write, hint::black_box, time::Instant};

fn iri(s: &str) -> Term {
    NamedNode::new(s).unwrap().into()
}
fn fixture(overlay: bool) -> Graph {
    let mut ttl = String::new();
    for i in 0..70_000 {
        writeln!(ttl, "<urn:s:{i}> <urn:p> {i} ; <urn:q> {i} ; <urn:r> {i} .").unwrap();
    }
    for i in 0..8192 {
        writeln!(ttl, "<urn:d:{i}> <urn:dead> <urn:o> .").unwrap();
    }
    let base = Graph::load_str(&ttl, "turtle").unwrap();
    if !overlay {
        return base;
    }
    let removed: Vec<[Term; 3]> = (0..4096)
        .map(|i| [iri(&format!("urn:d:{i}")), iri("urn:dead"), iri("urn:o")])
        .collect();
    let mut graph = base.fork();
    graph.apply_delta(&[], &removed).unwrap();
    assert_eq!(graph.store.overlay_len(), 4096);
    graph
}
fn run(graph: &Graph, q: &str, select: bool) -> usize {
    if select {
        black_box(query(black_box(graph), black_box(q)).unwrap())
            .rows
            .len()
    } else {
        usize::from(black_box(ask(black_box(graph), black_box(q)).unwrap()))
    }
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
    let args: Vec<String> = std::env::args().collect();
    let case = &args[1];
    let overlay = args[2] == "overlay";
    let graph = fixture(overlay);
    let pid = graph.dict.lookup(&iri("urn:p"));
    let scan = graph.store.scan_sorted(&[None, Some(pid), None], 0);
    assert_eq!(scan.rows.len(), 70_000);
    assert_eq!(scan.perm.order().into_iter().find(|&c| c != 1), Some(0));
    let position = if case == "second" { 2048 } else { 0 };
    let value = graph
        .dict
        .term(scan.to_spo(&scan.rows[position])[2])
        .to_string();
    let threshold = value.split('"').nth(1).unwrap().parse::<usize>().unwrap();
    drop(scan);
    let patterns = if case == "multi" {
        "?s <urn:p> ?o . ?s <urn:q> ?o . ?s <urn:r> ?o ."
    } else {
        "?s <urn:p> ?o . ?s <urn:q> ?o ."
    };
    let filter = if case == "first" || case == "second" {
        format!("?o + 0 = {threshold}")
    } else if case == "late" {
        "?o + 0 >= 0".to_string()
    } else {
        "?o + 0 < 0".to_string()
    };
    let body = format!("{patterns} FILTER({filter})");
    let select = case == "late";
    let q = if select {
        format!("SELECT ?s ?o WHERE {{ {body} }} LIMIT 65537")
    } else {
        format!("ASK {{ {body} }}")
    };
    let expected = if select {
        65537
    } else {
        usize::from(case == "first" || case == "second")
    };
    // Full, uncapped oracle is outside all measurement windows.
    let oracle = query(&graph, &format!("SELECT ?s ?o WHERE {{ {body} }}")).unwrap();
    assert_eq!(oracle.rows.len(), if select { 70_000 } else { expected });
    for row in &oracle.rows {
        let s = row[0].as_ref().unwrap().to_string();
        let o = row[1].as_ref().unwrap().to_string();
        let i = o.split('"').nth(1).unwrap().parse::<usize>().unwrap();
        assert_eq!(s, format!("<urn:s:{i}>"));
    }
    if select {
        let checked = query(&graph, &q).unwrap();
        assert_eq!(checked.rows.len(), expected);
        for row in &checked.rows {
            let i = row[1]
                .as_ref()
                .unwrap()
                .to_string()
                .split('"')
                .nth(1)
                .unwrap()
                .parse::<usize>()
                .unwrap();
            assert!(i < 70_000);
            assert_eq!(row[0].as_ref().unwrap().to_string(), format!("<urn:s:{i}>"));
        }
    }
    drop(oracle);
    println!("{{\"kind\":\"fixture\",\"query\":\"{q}\",\"seed_position\":{position},\"threshold\":{threshold},\"triples\":{}}}",graph.store.len());
    let setup_rss = rss();
    let sample: usize = args[3].parse().unwrap();
    let reps = 1;
    let iterations = if cfg!(feature = "count-alloc") { 1 } else { 3 };
    for _ in 0..2 {
        assert_eq!(run(&graph, &q, select), expected);
    }
    #[cfg(feature = "count-alloc")]
    {
        // Complete worker startup after fixture/oracle and both query warmups.
        rayon::broadcast(|_| ());
        counting::calibrate();
    }
    for _ in 0..reps {
        let rep = sample;
        let before_rss = rss();
        #[cfg(feature = "count-alloc")]
        let baseline = counting::begin();
        let start = Instant::now();
        let mut rows = 0;
        for _ in 0..iterations {
            rows += run(&graph, &q, select);
        }
        let nanos = start.elapsed().as_nanos();
        #[cfg(feature = "count-alloc")]
        let (allocs, reallocs, bytes, peak, live) = counting::end(baseline);
        #[cfg(not(feature = "count-alloc"))]
        let (allocs, reallocs, bytes, peak, live) = (0u64, 0u64, 0u64, 0u64, 0u64);
        assert_eq!(rows, iterations * expected);
        println!("{{\"case\":\"{case}\",\"view\":\"{}\",\"counting\":{},\"rep\":{rep},\"iterations\":{iterations},\"nanos\":{nanos},\"rows\":{rows},\"allocs\":{allocs},\"reallocs\":{reallocs},\"requested_bytes\":{bytes},\"peak_live_growth\":{peak},\"live_after\":{live},\"setup_peak_rss\":{setup_rss},\"before_peak_rss\":{before_rss},\"after_peak_rss\":{}}}",if overlay {"overlay"} else {"base"},cfg!(feature="count-alloc"),rss());
    }
}
```

## Source, binary and toolchain provenance

{
  "utc": "2026-09-10T01:21:42.636513+00:00",
  "author": "GPT-6 Astra, xhigh (actual inherited implementation runtime)",
  "baseline": "e53464c73f31f7aca800f3867ac054c36408e346",
  "candidate": "6e86f1d0ba447aa78a50800337a7a379702fa999",
  "harness_and_binaries": {
    "binary/baseline-count": {
      "sha256": "42154fda76c51e67bc3d9047531025e9f8637c20255b973f246a9f9b0f70d314",
      "bytes": 5333776
    },
    "binary/baseline-timing": {
      "sha256": "ccab79f940c63edf7c364579cc0cf18cc38d24dc5f7d4870c3234465459f5688",
      "bytes": 5333312
    },
    "binary/candidate-count": {
      "sha256": "8496cf1d66d48c631f1226d987dc7c415e18fbc8ca2b8d9df8fdc5ee4ed5b221",
      "bytes": 5336656
    },
    "binary/candidate-timing": {
      "sha256": "6dae967649c53915958817e5dae066c08bb7ce6f0e188146883071a89f0bd043",
      "bytes": 5336224
    },
    "baseline/Cargo.toml": {
      "sha256": "dc87fe224e013edc8ae4be15e745e846bece1b4f7cef7fbba113ab27342ed8f5",
      "bytes": 739
    },
    "baseline/Cargo.lock": {
      "sha256": "2a9682bdd546f59634fd412c474fca14badf6a3bed63c58e44c866351e389cb9",
      "bytes": 23149
    },
    "baseline/src/main.rs": {
      "sha256": "3f7e24f029adbf6142fe1aafec4bab524705dd89d39cd2724bf0e9b16fd37e84",
      "bytes": 5878
    },
    "baseline/src/counting.rs": {
      "sha256": "65cd7693a9adb8a4c45f52d939db0b932892ceec4b4806423ee1472c061bc911",
      "bytes": 7734
    },
    "candidate/Cargo.toml": {
      "sha256": "9460117f23653079a9d9747255bc3641e9910e1caa18fc05a5a3f0b74e123ff3",
      "bytes": 739
    },
    "candidate/Cargo.lock": {
      "sha256": "2a9682bdd546f59634fd412c474fca14badf6a3bed63c58e44c866351e389cb9",
      "bytes": 23149
    },
    "candidate/src/main.rs": {
      "sha256": "3f7e24f029adbf6142fe1aafec4bab524705dd89d39cd2724bf0e9b16fd37e84",
      "bytes": 5878
    },
    "candidate/src/counting.rs": {
      "sha256": "65cd7693a9adb8a4c45f52d939db0b932892ceec4b4806423ee1472c061bc911",
      "bytes": 7734
    },
    "allocation-quiescent/binary/baseline-count": {
      "sha256": "67fbeebeb389a2e1e6a751d6519618d72b895dbf5648bc273852852ed71ac348",
      "bytes": 5341008
    },
    "allocation-quiescent/binary/candidate-count": {
      "sha256": "f8803d2528cf472e60f8805d7a2b135115eece79da640a498cbc0cc23db299a8",
      "bytes": 5360416
    },
    "allocation-quiescent/baseline/Cargo.toml": {
      "sha256": "dc87fe224e013edc8ae4be15e745e846bece1b4f7cef7fbba113ab27342ed8f5",
      "bytes": 739
    },
    "allocation-quiescent/baseline/Cargo.lock": {
      "sha256": "2a9682bdd546f59634fd412c474fca14badf6a3bed63c58e44c866351e389cb9",
      "bytes": 23149
    },
    "allocation-quiescent/baseline/src/main.rs": {
      "sha256": "ff240a481cab58e8f5115086ec03b4f8bd618ef5ca85d9f4518080dc21f00d40",
      "bytes": 6008
    },
    "allocation-quiescent/baseline/src/counting.rs": {
      "sha256": "65cd7693a9adb8a4c45f52d939db0b932892ceec4b4806423ee1472c061bc911",
      "bytes": 7734
    },
    "allocation-quiescent/candidate/Cargo.toml": {
      "sha256": "9460117f23653079a9d9747255bc3641e9910e1caa18fc05a5a3f0b74e123ff3",
      "bytes": 739
    },
    "allocation-quiescent/candidate/Cargo.lock": {
      "sha256": "2a9682bdd546f59634fd412c474fca14badf6a3bed63c58e44c866351e389cb9",
      "bytes": 23149
    },
    "allocation-quiescent/candidate/src/main.rs": {
      "sha256": "ff240a481cab58e8f5115086ec03b4f8bd618ef5ca85d9f4518080dc21f00d40",
      "bytes": 6008
    },
    "allocation-quiescent/candidate/src/counting.rs": {
      "sha256": "65cd7693a9adb8a4c45f52d939db0b932892ceec4b4806423ee1472c061bc911",
      "bytes": 7734
    }
  },
  "runtime_sources": {
    "baseline/crates/sparq-engine/src/exec.rs": {
      "sha256": "71f7279359bf982dad1f751fcbc6b42cd68200daebf0de2f417bca1a20d867fd",
      "bytes": 1023539
    },
    "baseline/crates/sparq-core/src/store.rs": {
      "sha256": "6c70a6d4f13de468143f19b75972c28b312a8a2110bf95deb502d784f9b9284c",
      "bytes": 112315
    },
    "candidate/crates/sparq-engine/src/exec.rs": {
      "sha256": "661d12d64dadcf90bb49b780116f614517cde373bd3bd24ab48194d32da69575",
      "bytes": 1032328
    },
    "candidate/crates/sparq-core/src/store.rs": {
      "sha256": "6c70a6d4f13de468143f19b75972c28b312a8a2110bf95deb502d784f9b9284c",
      "bytes": 112315
    }
  },
  "free_bytes": 13647691776,
  "baseline_git_tree": "92c9cbc06ff009905e1707fab77bd50406a596d1",
  "candidate_git_tree": "5bdfb4b59b525b7807163df5c61d14f23426a7dd",
  "checks": {
    "paired_harness_byte_equal": true,
    "counting_all_four_byte_equal": true,
    "old_new_measured_block_byte_equal": true,
    "both_worktrees_clean": true
  }
}
rustc 1.97.1 (8bab26f4f 2026-07-14)
binary: rustc
commit-hash: 8bab26f4f68e0e26f0bb7960be334d5b520ea452
commit-date: 2026-07-14
host: aarch64-apple-darwin
release: 1.97.1
LLVM version: 22.1.6

{
  "capped-rhs-diagnostic": [],
  "libc": [
    "default",
    "std"
  ],
  "oxrdf": [
    "default",
    "rdf-12"
  ],
  "rayon": [],
  "spargebra": [
    "default",
    "sep-0006",
    "sparql-12"
  ],
  "sparq-core": [
    "default",
    "parallel"
  ],
  "sparq-engine": [
    "default",
    "digest",
    "parallel",
    "regex"
  ]
}

## Evidence map and scope

Original raw/ contains160 exact stdout/stderr pairs and measurement-commands.json records every argv/exit/binary hash. allocation-quiescent/raw/ contains60 pairs and its own receipts. samples.json retains159 successful original records; allocation-quiescent/samples.json retains60. summary.py reproduces all displayed summaries from those files; summary.json contains full statistics including cumulative RSS. builds.json, quiescent-build-*.json and allocation-quiescent/builds.json record actual sequential optimized builds and reserve monitoring. All six binaries are frozen; all four counting.rs copies are byte-identical. The complete production source/test review context remains in prior59-file phase1 packet, not duplicated here; new physical-path coverage was not executed. No source repair, next-phase test, model call, publication, workflow or remote change occurred.
