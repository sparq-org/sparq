# PR6486 CI execution evidence

{
  "verdict": "Requested execution evidence established; one remote merged-tree hash read remains unverified after404. No observed source/test gate failure in current successful cohorts.",
  "pr": 6486,
  "head": "9ac4a59e96b1c69c8d617fbde9e98fe8c0436fed",
  "model": "GPT-6 Astra xhigh",
  "protected_gate": {
    "run": 34511329028,
    "attempt": 2,
    "check_job_id": 103002616428,
    "app_id": 15368,
    "conclusion": "success",
    "completed_at": "2026-09-10T18:47:51Z",
    "evidence": "root-aggregate-recovery-gate-check.json"
  },
  "core_tests": {
    "new": 4,
    "existing_insertion": 1,
    "all_observed_PASS": true,
    "mmap_WAL": "bulk1 plus mmap,dict-spill and spqcprm2,mmap matrix legs",
    "evidence": "memo-test-evidence.json"
  },
  "feature_matrix": {
    "core_test_and_clippy_legs": 8,
    "features": [
      "compact-index",
      "mmap,dict-spill",
      "jsonld",
      "native-ttl",
      "iri-fast",
      "block-bloom",
      "spqcprm2,mmap",
      "overlay-deleted-projections"
    ],
    "evidence": "core-feature-legs.json",
    "qualification": "Separate legs plus defaults, not exhaustive powerset; mapped test is cfg(mmap) and absent in nonmmap legs as expected."
  },
  "privacy": {
    "scanned_files": 784,
    "selftests_passed": 32,
    "failed": 0,
    "job": 102985912521
  },
  "rustdoc": {
    "default_and_all_features": true,
    "cargo_doc_workspace": true,
    "RUSTDOCFLAGS": "-D warnings",
    "sparq_core_documenting_seen_twice": true,
    "job": 102986509738,
    "gap": null
  },
  "wasm": {
    "actual_base_bytes": 1562729,
    "actual_head_bytes": 1562707,
    "actual_delta_bytes": -22,
    "V2": "6486.json accepted; no byte-neutrality claim",
    "size_gate": "Benchmarks deterministic-only independently passes best-ever floor with PERF_GATE_ALLOW empty",
    "floor": 1587594,
    "latency_proof": false
  },
  "source": {
    "local_head_clean": true,
    "only_files": [
      "crates/sparq-core/src/lib.rs",
      "bench/feature-off-declarations/6486.json"
    ],
    "policy_workflows_scalar_floor_changed": false,
    "CI_merge_checkout": "613cad4d82742955394658dbac03483ef3a4cf97"
  },
  "cohorts": "Use ready-for-review run34511329xxx/34511329512, recovered current-head vector34511329019 and aggregateattempt2. Earlier synchronize34511322xxx automatic cancellations and failedattempt1 are not execution substitutes. Successful feature-off artifact job originated before the failed-job-only quick-gate recovery and remained valid same-head evidence.",
  "read_scope": {
    "requests": 18,
    "initial_core_remaining": 4971,
    "one_error": "404 for abbreviated Git-data commit613cad4; no retry or further requests",
    "remote_mutations": 0,
    "builds": 0,
    "tests_executed_locally": 0
  },
  "remaining": "Root may independently verify full CI merge commit tree using exact fullSHA if needed. All named runtime obligations have actual logs; no new execution requested. Full publication/queue decision remains root-owned.",
  "pending_commands": false
}

## Memo results
[
  {
    "line": 770,
    "text": "2026-09-10T18:22:58.6601849Z         PASS [   0.018s] ( 246/3072) sparq-core tests::has_high_precision_decimal_memo_mapped_wal_precision",
    "log": "bulk1.log",
    "job_id": 102990724110
  },
  {
    "line": 742,
    "text": "2026-09-10T18:21:45.1978503Z         PASS [   0.013s] ( 218/2877) sparq-core tests::has_high_precision_decimal_memo_fork_compact_isolation",
    "log": "bulk2.log",
    "job_id": 102990724545
  },
  {
    "line": 743,
    "text": "2026-09-10T18:21:45.2069739Z         PASS [   0.008s] ( 219/2877) sparq-core tests::has_high_precision_decimal_memo_preserves_unchanged_dictionary",
    "log": "bulk2.log",
    "job_id": 102990724545
  },
  {
    "line": 727,
    "text": "2026-09-10T18:16:12.8347821Z         PASS [   0.015s] ( 203/2739) sparq-core tests::has_high_precision_decimal_memo_survives_delta_insert",
    "log": "bulk3.log",
    "job_id": 102990724053
  },
  {
    "line": 729,
    "text": "2026-09-10T18:16:12.8437871Z         PASS [   0.023s] ( 205/2739) sparq-core tests::has_high_precision_decimal_memo_invalidates_stored_growth",
    "log": "bulk3.log",
    "job_id": 102990724053
  }
]

## Core feature legs
[
  {
    "features": "compact-index",
    "job_id": 102987207565,
    "log": "core-g35.log",
    "build_line": 365,
    "test_command": "2026-09-10T18:15:03.5525698Z + cargo test -p sparq-core --features compact-index",
    "clippy_command": "2026-09-10T18:15:21.4144072Z + cargo clippy -p sparq-core --features compact-index --all-targets -- -D warnings",
    "unit_summary": "2026-09-10T18:15:15.4972728Z test result: ok. 149 passed; 0 failed; 4 ignored; 0 measured; 0 filtered out; finished in 7.77s"
  },
  {
    "features": "mmap,dict-spill",
    "job_id": 102987207565,
    "log": "core-g35.log",
    "build_line": 609,
    "test_command": "2026-09-10T18:15:25.8675622Z + cargo test -p sparq-core --features mmap,dict-spill",
    "clippy_command": "2026-09-10T18:17:07.4496638Z + cargo clippy -p sparq-core --features mmap,dict-spill --all-targets -- -D warnings",
    "unit_summary": "2026-09-10T18:15:39.9460020Z test result: ok. 226 passed; 0 failed; 4 ignored; 0 measured; 0 filtered out; finished in 8.52s"
  },
  {
    "features": "jsonld",
    "job_id": 102987207565,
    "log": "core-g35.log",
    "build_line": 940,
    "test_command": "2026-09-10T18:17:12.9059225Z + cargo test -p sparq-core --features jsonld",
    "clippy_command": "2026-09-10T18:17:31.5160620Z + cargo clippy -p sparq-core --features jsonld --all-targets -- -D warnings",
    "unit_summary": "2026-09-10T18:17:25.6149648Z test result: ok. 154 passed; 0 failed; 4 ignored; 0 measured; 0 filtered out; finished in 8.11s"
  },
  {
    "features": "native-ttl",
    "job_id": 102987207412,
    "log": "core-g36.log",
    "build_line": 364,
    "test_command": "2026-09-10T18:20:38.6306676Z + cargo test -p sparq-core --features native-ttl",
    "clippy_command": "2026-09-10T18:20:56.7959247Z + cargo clippy -p sparq-core --features native-ttl --all-targets -- -D warnings",
    "unit_summary": "2026-09-10T18:20:50.8600790Z test result: ok. 175 passed; 0 failed; 4 ignored; 0 measured; 0 filtered out; finished in 7.70s"
  },
  {
    "features": "iri-fast",
    "job_id": 102987207412,
    "log": "core-g36.log",
    "build_line": 634,
    "test_command": "2026-09-10T18:21:01.0684376Z + cargo test -p sparq-core --features iri-fast",
    "clippy_command": "2026-09-10T18:21:19.9190487Z + cargo clippy -p sparq-core --features iri-fast --all-targets -- -D warnings",
    "unit_summary": "2026-09-10T18:21:13.8554428Z test result: ok. 155 passed; 0 failed; 4 ignored; 0 measured; 0 filtered out; finished in 8.35s"
  },
  {
    "features": "block-bloom",
    "job_id": 102987207412,
    "log": "core-g36.log",
    "build_line": 884,
    "test_command": "2026-09-10T18:21:23.9801392Z + cargo test -p sparq-core --features block-bloom",
    "clippy_command": "2026-09-10T18:21:43.1397320Z + cargo clippy -p sparq-core --features block-bloom --all-targets -- -D warnings",
    "unit_summary": "2026-09-10T18:21:37.1291341Z test result: ok. 154 passed; 0 failed; 6 ignored; 0 measured; 0 filtered out; finished in 8.72s"
  },
  {
    "features": "spqcprm2,mmap",
    "job_id": 102987207699,
    "log": "core-g41.log",
    "build_line": 366,
    "test_command": "2026-09-10T18:26:56.8478065Z + cargo test -p sparq-core --features spqcprm2,mmap",
    "clippy_command": "2026-09-10T18:29:08.2845599Z + cargo clippy -p sparq-core --features spqcprm2,mmap --all-targets -- -D warnings",
    "unit_summary": "2026-09-10T18:27:08.0443868Z test result: ok. 221 passed; 0 failed; 5 ignored; 0 measured; 0 filtered out; finished in 6.86s"
  },
  {
    "features": "overlay-deleted-projections",
    "job_id": 102987207699,
    "log": "core-g41.log",
    "build_line": 691,
    "test_command": "2026-09-10T18:29:12.1893964Z + cargo test -p sparq-core --features overlay-deleted-projections",
    "clippy_command": "2026-09-10T18:29:26.1395362Z + cargo clippy -p sparq-core --features overlay-deleted-projections --all-targets -- -D warnings",
    "unit_summary": "2026-09-10T18:29:22.0888895Z test result: ok. 154 passed; 0 failed; 4 ignored; 0 measured; 0 filtered out; finished in 6.27s"
  }
]

## Rustdoc proof
{
  "job_id": 102986509738,
  "source": "ci.yml1408-1415",
  "intra_doc_link_warning_enforced": true,
  "routes": [
    {
      "command": "2026-09-10T18:03:36.9928149Z cargo doc --workspace --no-deps",
      "begin_line": 1516,
      "finish_line": 1700,
      "output": [
        {
          "line": 1527,
          "text": "2026-09-10T18:03:36.9967370Z   RUSTDOCFLAGS: -D warnings"
        },
        {
          "line": 1685,
          "text": "2026-09-10T18:04:12.1953641Z  Documenting sparq-core v0.1.1 (/home/runner/work/sparq/sparq/crates/sparq-core)"
        },
        {
          "line": 1700,
          "text": "2026-09-10T18:04:16.3555044Z     Finished `dev` profile [unoptimized + debuginfo] target(s) in 39.32s"
        }
      ]
    },
    {
      "command": "2026-09-10T18:04:16.4009237Z cargo doc --workspace --no-deps --all-features",
      "begin_line": 1702,
      "finish_line": 1851,
      "output": [
        {
          "line": 1713,
          "text": "2026-09-10T18:04:16.4047937Z   RUSTDOCFLAGS: -D warnings"
        },
        {
          "line": 1847,
          "text": "2026-09-10T18:05:04.3170988Z  Documenting sparq-core v0.1.1 (/home/runner/work/sparq/sparq/crates/sparq-core)"
        },
        {
          "line": 1851,
          "text": "2026-09-10T18:05:07.5991473Z     Finished `dev` profile [unoptimized + debuginfo] target(s) in 51.16s"
        }
      ]
    }
  ],
  "not_inferred_from_doctest_job_name": true
}


## Privacy proof
{
  "job_id": 102985912521,
  "lines": [
    {
      "line": 591,
      "text": "2026-09-10T17:58:54.7541370Z ##[group]Run bash scripts/check-privacy-claims.sh"
    },
    {
      "line": 602,
      "text": "2026-09-10T17:58:57.1826362Z check-privacy-claims: OK \u2014 scanned 784 file(s); no unqualified ZK/MPC privacy/soundness claim."
    },
    {
      "line": 615,
      "text": "2026-09-10T17:58:57.5179501Z test_privacy_claims: 32 passed, 0 failed."
    },
    {
      "line": 616,
      "text": "2026-09-10T17:58:57.5180931Z test_privacy_claims: OK \u2014 both-direction soundness-claim coverage holds."
    }
  ]
}


## Independent wasm checks
{
  "intent_job_id": 102988177578,
  "intent_lines": [
    {
      "line": 1851,
      "text": "2026-09-10T17:59:09.1482229Z   echo \"[leg2] SKIP \u2014 no base SHA resolvable for event pull_request; nothing to compare.\" \\"
    },
    {
      "line": 1877,
      "text": "2026-09-10T17:59:09.8557875Z [leg2] feature-OFF wasm DIFFERS from the base tree: base=1562729 head=1562707 bytes, size delta=-22 (-0.001%). Comparison is byte-for-byte, so same-length content changes are also detected."
    },
    {
      "line": 1878,
      "text": "2026-09-10T17:59:09.8561587Z [leg2] OK \u2014 change DECLARED (mechanism V2): the head tree added bench/feature-off-declarations/['6486.json'] not present in the base tree. The feature-OFF byte change is intentional. Its SIZE is governed by the wasm_bundle_bytes floor ratchet (+/-2% band) in bench.yml, unchanged by this gate."
    }
  ],
  "feature_resolution_job": 102988175230,
  "resolution_lines": [
    {
      "line": 901,
      "text": "2026-09-10T18:04:55.7990747Z [leg1] OK \u2014 'vectorized' is absent from all resolved default feature sets (sparq-engine, sparq-wasm, package defaults). Compile-time absence confirmed."
    }
  ],
  "size_job_id": 102986490948,
  "size_lines": [
    {
      "line": 862,
      "text": "2026-09-10T18:01:56.1482464Z ##[group]Run bash scripts/ci-bench.sh --deterministic-only 200000 bench-results.json"
    },
    {
      "line": 863,
      "text": "2026-09-10T18:01:56.1483108Z bash scripts/ci-bench.sh --deterministic-only 200000 bench-results.json"
    },
    {
      "line": 872,
      "text": "2026-09-10T18:02:48.9093016Z wrote bench-results.json (--deterministic-only):"
    },
    {
      "line": 895,
      "text": "2026-09-10T18:02:48.9111117Z     \"name\": \"wasm_bundle_bytes\","
    },
    {
      "line": 952,
      "text": "2026-09-10T18:02:50.1313484Z   PERF_GATE_ALLOW: "
    },
    {
      "line": 965,
      "text": "2026-09-10T18:02:50.1677495Z   - wasm_bundle_bytes: floor=1.58759e+06 cur=1.56271e+06 -> IMPROVEMENT delta -1.57% (below floor) [ratchet] \u2014 floor may ratchet DOWN"
    },
    {
      "line": 967,
      "text": "2026-09-10T18:02:50.1678539Z perf-gate: PASS \u2014 no perf regression above the best-ever floor (deterministic metrics strict; timing metrics advisory)."
    }
  ],
  "limits": "Self-test fake31/32-byte results later in equality log are negative controls, not actual compiled artifacts. Sizes above are from genuine PR run; no latency measurement."
}


Full raw named logs, source hashes, exact checkout metadata and request receipts are frozen alongside this report. The404was an unsuccessful read, not an exhausted-budget recovery or authorization to retry.
