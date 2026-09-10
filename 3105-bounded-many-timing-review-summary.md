# Bounded many-RHS timing screen

This is exact-head local advisory evidence, not a canonical or admission claim. The first-hit tradeoff remains in the separate two-pattern report.

## report.json

```json
{
  "author": "GPT-6 Astra xhigh",
  "head": "ed66ef0931fa19dd521fac433870c86a78687a30",
  "baseline": "e53464c73f31f7aca800f3867ac054c36408e346",
  "scope": "One fixed System-allocator timing phase for the same5/8pattern memory-tradeoff cases. No source/commit/model/network changes, retesting of live-budget arms, extra cases, tuning or retries.",
  "completed_samples": 20,
  "paired_repetitions": 5,
  "queries_per_sample": 3,
  "warmups_per_process": 2,
  "pair_order": "AB/BA/AB/BA/AB per case",
  "fixture": "Identical many-pattern Rust harness to frozen allocation phase:70000subject/object pairs matching across every predicate,8192unrelatedtriples,ordinary base graph,allRHS/all3blocks reached before arithmeticresidualfalse. Committed physical-path tests provide separate engagement proof.",
  "profile": "releaseO3/debugfalse/ltofalse/codegen-units16;System allocator,count-allocOFF,oneRayonworker",
  "results": {
    "5": {
      "main_median_ns": 22691430.666666668,
      "candidate_median_ns": 21018083.333333332,
      "candidate_to_main": 0.9262564199712866,
      "candidate_percent_change": -7.37435800287134,
      "ranges_overlap": false,
      "all_five_paired_ratios_below_one": true
    },
    "8": {
      "main_median_ns": 33355778.0,
      "candidate_median_ns": 31675722.333333332,
      "candidate_to_main": 0.9496322446244045,
      "candidate_percent_change": -5.036775537559546,
      "ranges_overlap": false,
      "all_five_paired_ratios_below_one": true
    }
  },
  "assessment": "No latency regression observed in either fixed many-pattern case. Both candidate ranges are below main in this small local screen; no new no-go is established here. This is not canonical or standalone admission approval.",
  "build_provenance": "Both unique capped-rhs-many-timing roots actually compile in -vv logs, with identical freshly written harness source. Actual rustc flags omit count-alloc. Distinct engine rlib identities/dep-info point to the correct source trees and hash-match the previously verified two-pattern timing libraries. Full source, root binary, harness, dependency lock and command hashes are retained.",
  "correctness": "All20samples passed expectedfalseASK and uncapped-oracle checks; eachsample executes3queries. No sampled run failed.",
  "limits": [
    "LocalmacOSarm64/rustc1.97.1 screen with5pairs and3queries permeasurement; observed ranges/variation are not a statistical guarantee or universal speedup.",
    "These timings cover only the declared5/8pattern matching-base-graph residual-miss fixtures. Earlier two-pattern timing separately records a2.06%first-hit median increase; this phase does not erase that cost.",
    "Requested heap measurements remain in bounded-retention/measurement/summary.json; rawRSS in this phase is cumulative process high-water including setup/oracle/warmups and is not queryheap.",
    "The private4MiB allowance is per capped-evaluation invocation, not a global/nonnested-query memory limit. Existing transient scans, results, planner/AST storage and allocator metadata are outside it. No global-budget claim or framework was introduced.",
    "FullCI/conformance/feature/wasm ratchets and root engineering/review decisions remain separate. Existing reviewhold is untouched by this agent."
  ],
  "prior_preservation": [
    {
      "bundle": "bounded-retention",
      "files": 162,
      "manifest_sha256": "9807d2c72b46c5597ea719a1c00e73100d51a0747ced9bbc7280ab0ebd868ecb",
      "all_hashes_match": true
    },
    {
      "bundle": "bounded-timing",
      "files": 75,
      "manifest_sha256": "c384be7f1e142dc18673cf72cf6cb9074a15d651a5036b211ca4c893fd97da17",
      "all_hashes_match": true
    }
  ],
  "finished_utc": "2026-09-10T03:03:28.582150+00:00",
  "free_bytes_finish": 9584074752,
  "resource_floor": 6509559808,
  "commands_running": false,
  "source_clean": true
}

```

## protocol.json

```json
{
  "declared_utc": "2026-09-10T03:00:38.970823+00:00",
  "max_minutes": 8,
  "source": {
    "baseline": "e53464c73f31f7aca800f3867ac054c36408e346",
    "candidate": "ed66ef0931fa19dd521fac433870c86a78687a30"
  },
  "cases_total_patterns": [
    5,
    8
  ],
  "fixture": "Identical to frozen bounded-retention many-pattern allocation harness:70000 seed subjects,5/8predicates all share s,o andmatch,8192unrelatedtriples,final arithmeticresidualfalse",
  "profile": "release O3, debugfalse,ltofalse,codegen-units16;System allocator,count-alloc OFF",
  "rayon_threads": 1,
  "warmups": 2,
  "queries_per_sample": 3,
  "paired_reps": 5,
  "pair_order": [
    "AB",
    "BA",
    "AB",
    "BA",
    "AB"
  ],
  "total_samples": 20,
  "correctness": "same generated false ASK oracle and uncapped query outside measurement; actual all-RHS/all-block path separately pinned by committed tests",
  "no_tuning_retries_or_extra_cases": true,
  "assessment": "Report medians/ranges/paired ratios and any regression plainly. Local advisory screen only, not canonical or standalone admission approval.",
  "build_provenance": "Unique capped-rhs-many-timing package in both paired manifests with fresh source mtimes; actual -vv root compiler invocations and distinct engine rlib/dep-info source checks",
  "limits": {
    "min_free_bytes": 6509559808,
    "jobs": 2,
    "incremental": false,
    "offline_locked": true,
    "no_concurrent_build_and_measurement": true
  }
}

```

## summary.json

```json
{
  "5": {
    "baseline": {
      "samples": 5,
      "ns_per_query": {
        "min": 22501319.333333332,
        "median": 22691430.666666668,
        "max": 22985458.333333332,
        "mean": 22700916.6,
        "population_stdev": 173173.1330183627,
        "cv": 0.0076284643510105095
      },
      "raw_sample_total_ns": [
        68956375,
        68074292,
        67503958,
        67646166,
        68332958
      ]
    },
    "candidate": {
      "samples": 5,
      "ns_per_query": {
        "min": 20820083.333333332,
        "median": 21018083.333333332,
        "max": 21224166.666666668,
        "mean": 21029536.066666666,
        "population_stdev": 133408.95264951355,
        "cv": 0.0063438847260628055
      },
      "raw_sample_total_ns": [
        63302458,
        63054250,
        62953583,
        63672500,
        62460250
      ]
    },
    "candidate_to_baseline_median_ratio": 0.9262564199712866,
    "ranges_overlap": false,
    "paired_ratios": [
      0.9180073343472595,
      0.9262564199712867,
      0.9325909897016705,
      0.9412580751435343,
      0.914057459652193
    ]
  },
  "8": {
    "baseline": {
      "samples": 5,
      "ns_per_query": {
        "min": 33265722.333333332,
        "median": 33355778.0,
        "max": 33980069.333333336,
        "mean": 33478350.0,
        "population_stdev": 260474.55306984216,
        "cv": 0.007780388014040183
      },
      "raw_sample_total_ns": [
        99939583,
        100067334,
        99797167,
        100430958,
        101940208
      ]
    },
    "candidate": {
      "samples": 5,
      "ns_per_query": {
        "min": 31346014.0,
        "median": 31675722.333333332,
        "max": 31743611.0,
        "mean": 31586130.6,
        "population_stdev": 151910.39569392364,
        "cv": 0.004809401873806082
      },
      "raw_sample_total_ns": [
        94038042,
        94413375,
        95027167,
        95082542,
        95230833
      ]
    },
    "candidate_to_baseline_median_ratio": 0.9496322446244045,
    "ranges_overlap": false,
    "paired_ratios": [
      0.9409489131048305,
      0.9434984547504783,
      0.9522030520164966,
      0.9467453451952534,
      0.9341832321943074
    ]
  }
}

```
