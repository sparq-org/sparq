# Issue5183 replay readiness

{
  "task": "Existing issue5183 source triage and replay readiness",
  "model": "GPT-6 Astra xhigh",
  "created_utc": "2026-09-10T10:24:35.222772+00:00",
  "main": "781f667c19a8ebb779cfccb24b05ea432360b025",
  "parent": "d41ec9fcb796504d85f5a5247a5fdc9eb3e65de5",
  "worktree": "/private/tmp/sparq-pr6049/.throughput-monitor/worktrees/issue5183",
  "branch": "codex/update-differential-replay",
  "runtime_executed": false,
  "builds_executed": false,
  "source_edits": false,
  "remote_reads_or_mutations": false,
  "ci_evidence": {
    "run": 34458877088,
    "job": 102811674711,
    "head": "781f667c19a8ebb779cfccb24b05ea432360b025",
    "first_seed": 4141222487,
    "zero_based_step": 8,
    "sequence_length": 10,
    "mismatch_seeds": [
      4141222487,
      4141222535,
      4141222571,
      4141222576,
      4141222599,
      4141222612,
      4141222617,
      4141222622
    ],
    "log_sha256": "648510046732f8c41893f56fefe4e776f39a0632181fa25349243b91aad29392"
  },
  "finding": {
    "classification": "Source-supported lossy-reference history hypothesis; deterministic runtime replay still required.",
    "cause_candidate": "Operation 4 inserts lexically distinct integer 8 and 008 at identical s2/p1. Oxigraph 0.5.9 encodes valid integers by value. Operation 8 creates one fresh blank node per WHERE solution, so the earlier cardinality difference can become a persistent extra p0 blank-node triple.",
    "strict_internal_guard": "The actual failing message is reached only after strict rebuild versus in-place comparison passes at that step (update_fuzz.rs:1136-1156).",
    "diagnostic_caveat": "compare reports the original canonical ca/cb difference, not the failed normalized pair. The displayed 008 line is not itself proof that integer normalization failed.",
    "canonicalization_scope": "An independently observed different raw term/blank-node count would establish a non-isomorphism independently of canonical labels. No canon defect is established by this log.",
    "main_attribution": "Comparator, canon source, workflow and Cargo.lock unchanged across6478; update.rs changes only budget scope installation. Existing issue predates6478. Exact same-seed parent replay has not run, so no causal exclusion claimed.",
    "semantic_basis": "Current shared instantiate_templates loops over every materialized solution with a fresh map per row; existing tests pin lexical term distinctness and per-solution blank nodes. Independent normative verification was not newly fetched in this no-network phase."
  },
  "recommended_next_step": {
    "first": "Authorize a small direct-rustc public-API harness linked separately to the hash-verified default engine rlibs for parent/main runtime-equivalent sources; execute proposed two-operation reduction plus two controls through rebuild and in-place, export raw terms and bnode counts.",
    "source_identity": "Parent cached ed66 engine/core/lock are byte-identical to d41. Main cached d07 differs from781f only in a README and cfg(test) code; exact diff saved. Both engine hashes match frozen compiler provenance.",
    "limits": "Default cached engine lacks CI algebra-rewrite; core cache does not establish exact mmap/dict-spill feature parity. This cheap step confirms public runtime semantics, not exact CI replay.",
    "second": "For exact fixed-seed replay, build only a small entry point importing unchanged update_fuzz.rs with its exact dependencies/features and allowlist working directory, then run seed4141222487,count1 on each source. Full sparq-bench pulls unrelated surfaces; do not build it speculatively.",
    "ci_equivalent_command": "./target/release-fast/sparq-bench update-fuzz --seed-start 4141222487 --seed-count 1",
    "resource_proposal": "Direct linkage: two small binaries plus source/logs; propose aggregate64MiB hard output cap, two-minute link/run phase, jobs1, no dependency compilation. Size/time are operational caps, not demonstrated estimates. Exact-feature fuzzer needs cold Oxigraph/canon and extra core/engine features: footprint not established; return separate plan before compilation.",
    "controls": "No seed search, no comparator alteration, no count/guard weakening. Keep original full10-operation sequence as distinct exact witness.",
    "remaining": "Pinned Oxigraph direct runtime and exact CI-feature/source replay both unexecuted; eight-seed class equivalence not established."
  },
  "limits": [
    "No network, builds, Rust edits, tests or SPARQL execution in this phase.",
    "No existing warm complete benchmark/fuzzer binary or Oxigraph/canon rlib found in authorized target.",
    "System python3 lacks tomllib; failed read-only inventory attempt was corrected with a bounded lockfile field parser, no installation.",
    "Existing root issue/CI evidence reused; no new issue/dedupe/API action.",
    "No claim that6478 caused or fixed these mismatches."
  ]
}

## Source anchors

- crates/sparq-bench/src/update_fuzz.rs:39-49: lexical identity and known Oxigraph limitation.
- update_fuzz.rs:771-908: canonical comparison then final-state normalization.
- update_fuzz.rs:1100-1172: apply all engines, strict internal comparison first.
- crates/sparq-engine/src/update.rs:203-220,288-333: fresh blank node per solution.
- Pinned Oxigraph0.5.9 numeric_encoder.rs:583-595,870-872,1140: integer value encoding/decoding.

No runtime result is presented; proposed reduced input is not yet a demonstrated shrink.
