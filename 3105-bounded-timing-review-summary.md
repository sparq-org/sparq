# Bounded capped-RHS timing screen

Read the structured report and observed ranges below. This is local advisory evidence for exact ed66, not a canonical or admission claim.

## report.json

```json
{
  "author": "GPT-6 Astra xhigh",
  "head": "ed66ef0931fa19dd521fac433870c86a78687a30",
  "baseline": "e53464c73f31f7aca800f3867ac054c36408e346",
  "scope": "One fixed local whole-query System-allocator timing screen. No source changes, commit, model/network operation, allocator instrumentation, extra sweep, or retries.",
  "completed_samples": 20,
  "paired_repetitions": 5,
  "queries_per_sample": 3,
  "warmups_per_process": 2,
  "pair_order": "AB/BA/AB/BA/AB per case",
  "host": "macOS arm64, rustc1.97.1, one Rayon worker; exact host/compiler output supplied",
  "build_profile": "release O3, debugfalse, ltofalse, codegen-units16, count-alloc feature OFF",
  "fixture": "Unchanged previous two-pattern public ASK harness:70000 subjects with p/q/r data and8192 unrelated triples, query p/q shares s,o; first physical seed hit or full arithmetic residualfalse miss. Fixture generation and uncapped correctness oracle are outside measurement windows.",
  "results": {
    "first_hit": {
      "main_median_ns": 1754902.6666666667,
      "candidate_median_ns": 1791083.3333333333,
      "candidate_to_main": 1.0206169079082827,
      "ranges_overlap": false,
      "assessment": "Small measured first-hit cost: candidate median +2.0617%. All five paired ratios exceed1. Do not claim first-hit latency neutrality."
    },
    "full_miss": {
      "main_median_ns": 12344500.0,
      "candidate_median_ns": 10557264.0,
      "candidate_to_main": 0.8552200575154927,
      "ranges_overlap": false,
      "assessment": "Candidate median -14.4780% in this local full-miss fixture. All five paired ratios are below1."
    }
  },
  "correctness": "All twenty samples passed generated expected ASK and existing uncapped oracle checks. Recorded rows are3 for first-hit and0 for full-miss across the three queries per sample. Exact actual capped-path evidence is in the frozen bounded-retention tests; this timing binary carries no cfg(test) observer.",
  "build_provenance": "Both roots use the unique capped-rhs-two-timing package identity and freshly written identical Rust harness bytes. Cargo -vv records actual separate rustc root invocations and flags. Each links a distinct cached engine rlib, whose SHA256 and dep-info are captured and point to its exact source tree. Full root/engine source and executable hashes recorded. Neither build simply copied the other root output. No freshness anomaly occurred in this phase.",
  "limits": [
    "Local advisory screen, not a canonical benchmark or merge/performance admission authorization.",
    "Five pairs with three queries each; min/max and population stdev describe observed variation, not a statistical guarantee. No retries or noisy-run selection.",
    "The first-hit increase may include the additional row-storage eligibility check and other bounded-cache overhead; this experiment does not isolate their individual causal costs.",
    "Only two-pattern base-view first-hit and miss are timed here. No claim for many-RHS, overlay, alternate feature/layout, concurrency, or every query shape.",
    "Raw RSS fields remain cumulative process high-water including setup/oracle/warmups; System-allocator outputs do not measure allocation counts or query heap. The separate frozen allocation phase supplies those metrics.",
    "Full authoritative CI, feature matrix, conformance, wasm size ratchet and independent review remain root-owned and required. Existing PR review:changes hold is untouched."
  ],
  "prior_preservation": {
    "bounded_retention_files": 162,
    "all_hashes_verified_unchanged": true,
    "manifest_sha256": "9807d2c72b46c5597ea719a1c00e73100d51a0747ced9bbc7280ab0ebd868ecb"
  },
  "finished_utc": "2026-09-10T02:53:54.397557+00:00",
  "free_bytes_finish": 9986560000,
  "resource_floor": 6509559808,
  "commands_running": false,
  "source_clean": true
}

```

## protocol.json

```json
{
  "declared_utc": "2026-09-10T02:50:24.537143+00:00",
  "minutes": 10,
  "source": {
    "baseline": "e53464c73f31f7aca800f3867ac054c36408e346",
    "candidate": "ed66ef0931fa19dd521fac433870c86a78687a30"
  },
  "cases": [
    "first",
    "miss"
  ],
  "fixture": "unchanged original two-pattern ASK harness:70000subjects with p/q/r,8192auxiliarytriples; query uses p/q both shared s,o; first actual subject-order hit or residual arithmeticfalse",
  "profile": "release opt-level3 debugfalse ltofalse codegen-units16; System allocator (count-alloc OFF)",
  "rayon": 1,
  "warmups": 2,
  "queries_per_sample": 3,
  "repetitions": 5,
  "paired_order": [
    "AB",
    "BA",
    "AB",
    "BA",
    "AB"
  ],
  "samples": 20,
  "oracle": "unchanged generated expected ASK result and uncapped oracle outside measurement",
  "no_tuning_or_retries": true,
  "advisory_only": true,
  "build_provenance": "Unique capped-rhs-two-timing package name shared only by identical paired harness, fresh source mtimes, actual cargo -vv compiler commands; source/dependency and artifact hashes recorded. Baseline/candidate engine library artifact identities verified separately.",
  "limits": {
    "free_floor": 6509559808,
    "jobs": 2,
    "incremental": false,
    "offline_locked": true,
    "no_concurrent_build_and_sample": true
  }
}

```

## summary.json

```json
{
  "first": {
    "baseline": {
      "samples": 5,
      "ns_per_query": {
        "min": 1749013.6666666667,
        "median": 1754902.6666666667,
        "max": 1771388.6666666667,
        "mean": 1759555.4666666668,
        "population_stdev": 8588.681757858853,
        "cv": 0.004881165681085009
      },
      "raw_sample_total_ns": [
        5263542,
        5247041,
        5264708,
        5314166,
        5303875
      ]
    },
    "candidate": {
      "samples": 5,
      "ns_per_query": {
        "min": 1777708.3333333333,
        "median": 1791083.3333333333,
        "max": 1809722.3333333333,
        "mean": 1793272.2666666666,
        "population_stdev": 12943.07255021002,
        "cv": 0.007217572473960463
      },
      "raw_sample_total_ns": [
        5429167,
        5343875,
        5333125,
        5373250,
        5419667
      ]
    },
    "candidate_to_baseline_median_ratio": 1.0206169079082827,
    "ranges_overlap": false,
    "paired_ratios": [
      1.0314664535782179,
      1.0184549730028791,
      1.012995402594028,
      1.0111182074477914,
      1.0218315853974689
    ]
  },
  "miss": {
    "baseline": {
      "samples": 5,
      "ns_per_query": {
        "min": 12254069.666666666,
        "median": 12344500.0,
        "max": 12459250.0,
        "mean": 12363644.466666667,
        "population_stdev": 77398.45412943771,
        "cv": 0.006260164981135609
      },
      "raw_sample_total_ns": [
        36952167,
        37329041,
        36762209,
        37377750,
        37033500
      ]
    },
    "candidate": {
      "samples": 5,
      "ns_per_query": {
        "min": 10483013.666666666,
        "median": 10557264.0,
        "max": 10738278.0,
        "mean": 10589516.733333332,
        "population_stdev": 90364.7224931404,
        "cv": 0.008533413258481693
      },
      "raw_sample_total_ns": [
        31921459,
        32214834,
        31449041,
        31585625,
        31671792
      ]
    },
    "candidate_to_baseline_median_ratio": 0.8552200575154927,
    "ranges_overlap": false,
    "paired_ratios": [
      0.8638589179357195,
      0.862996560774224,
      0.8554720147529764,
      0.8450381577275251,
      0.8552200575154927
    ]
  }
}

```
