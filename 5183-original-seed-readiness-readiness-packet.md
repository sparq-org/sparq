# Original UPDATE seed replay readiness

{
  "author": "GPT-6 Astra xhigh",
  "status": "READY_FOR_BOUNDED_EXECUTION_IF_ROOT_AUTHORIZES",
  "phase": "Readiness only: two successful offline cargo metadata commands; no build/test/run, network, production edit, commit or GitHub action.",
  "revisions": {
    "parent": "d41ec9fcb796504d85f5a5247a5fdc9eb3e65de5",
    "main": "781f667c19a8ebb779cfccb24b05ea432360b025"
  },
  "fixed_entrypoint": "A tiny standalone binary imports the unchanged update_fuzz.rs as a module and calls update_fuzz::run(4141222487, 1). No CLI parser, broad benchmark modules, seed search or comparator edit needed.",
  "direct_dependencies": [
    "sparq-core defaults + mmap + dict-spill",
    "sparq-engine defaults + algebra-rewrite",
    "sparq-canon defaults + rdf12-triple-terms",
    "oxigraph=0.5.9 default-features=false + rdf-12",
    "oxrdf=0.3.3 + rdf-12",
    "serde_json=1.0.150 from committed lock"
  ],
  "feature_proof": {
    "result": "Minimal closure103packages versus benchmark-reference111packages; every shared package has identical resolved features, and both resolved locks have zero dependency pin drift.",
    "important_unified_features": {
      "oxigraph@0.5.9": [
        "rdf-12"
      ],
      "oxrdf@0.2.4": [
        "default"
      ],
      "oxrdf@0.3.3": [
        "default",
        "oxsdatatypes",
        "rdf-12",
        "rdfc-10"
      ],
      "oxttl@0.1.8": [
        "default"
      ],
      "oxttl@0.2.3": [
        "default",
        "rdf-12"
      ],
      "rdf-canon@0.15.3": [],
      "spargebra@0.4.6": [
        "default",
        "sep-0002",
        "sep-0006",
        "sparql-12"
      ],
      "sparq-canon@0.1.1": [
        "default",
        "parallel",
        "rdf12-triple-terms"
      ],
      "sparq-core@0.1.1": [
        "default",
        "dict-spill",
        "mmap",
        "parallel"
      ],
      "sparq-engine@0.1.1": [
        "algebra-rewrite",
        "default",
        "digest",
        "parallel",
        "regex"
      ],
      "sparq-substrate@0.1.1": [
        "compare",
        "join",
        "numeric",
        "rows"
      ]
    },
    "omissions": "sparq-difftest plus seven exclusive dependency packages are unused by update_fuzz.rs. Direct libc is unused by the module and stays resolved transitively through core. No query differential or unrelated benchmark code required.",
    "source_equality": "All12 compared source/manifest/lock/allowlist/workflow files are byte-identical between parent/main. Internal source runtime differences remain and must be compiled separately; inspect parent-main-path-delta.txt."
  },
  "path_and_guard_contract": {
    "layout": "For each variant, standalone manifest at harness/crates/sparq-bench/Cargo.toml with its own [workspace]; committed allowlist copied byte-for-byte to harness/bench/differential-divergences.json. Module source referenced by #[path] to the qualified revision source.",
    "allowlist": "Unset SPARQ_FUZZ_DIVERGENCES in child env so the unmodified env!(CARGO_MANIFEST_DIR)/../../bench path is exercised. Before run, verify path resolution and file hash; require initial stdout to list the existing integer-lexical class. Missing/unreadable path would silently switch the existing helper to STRICT, so treat this as invalid replay setup, not a regression.",
    "LOAD": "Keep LoadSandbox and with_load_base unchanged. Use a task-owned TMPDIR; generator relative file://doc0.nt strings, contents and Oxigraph INSERT substitution stay unchanged. No HTTP feature/network is needed.",
    "strict_comparison": "Keep check_seed(None injection), full generator, every per-step dataset/probe guard, raw count checks, strict Sparq rebuild/in-place comparison before reference comparison, original canonicalizer and allowlist behavior.",
    "process_exit": "run() exits1 on a mismatch. The capture wrapper must preserve stdout/stderr and exit1 as observed outcome, not retry or abort evidence export. It reports all10 generated operations while stopping evaluation at the first divergence.",
    "identity": "Use unique parent/main harness package names and separate source paths. Record actual rustc -vv commands, linked rlib hashes and source-tree/file hashes; never rely on a same-package preserved-mtime cache hit. Do not edit source or reuse a falsely qualified old binary."
  },
  "source_provision": {
    "main": "Use clean existing issue5183 worktree at781f; no shared checkout switch.",
    "parent": "Propose task-owned git archive/export of exactd41 tracked tree, read-only thereafter. Full export avoids missing optional path manifests/workspace inheritance during Cargo resolution; standalone harness prevents workspace-wide builds.",
    "cost": "Exact tracked parent content79,102,861B / estimated97,091,584B file allocation; no symlinks. Actual compiled internal source content5,623,162B. Export NOT performed in readiness.",
    "parser": "Patch crates-io spargebra to each qualified revision vendor/spargebra; all relevant source/manifest bytes currently identical."
  },
  "warm_cache": {
    "target_allocated_bytes": 596918272,
    "target_files": 1706,
    "free_bytes_observed": 6479761408,
    "existing": "Fresh pinned Oxigraph witness rlib4,079,840B; parser with sep-0002/sep-0006/sparql-12 present; core mmap/dict-spill rlib4,395,240B exists. Existing engine variants roughly5.3-6.1MB.",
    "not_warm": "No engine artifact with algebra-rewrite and no sparq-canon or rdf-canon rlib in authorized target. Many dependent artifacts may rebuild from source-path/feature identity changes.",
    "incremental_closure": "41 package identities beyond the Oxigraph-only metadata graph, including already-cached engine/core dependencies; this is NOT41 guaranteed cold compiles. See incremental-package-closure.json.",
    "freshness_limit": "Artifact presence/features are only readiness hints; actual Cargo Fresh/Compiling and dependency hashes must establish reuse. No runtime or compile-cost guarantee."
  },
  "proposed_caps": {
    "jobs": 1,
    "offline": true,
    "incremental": false,
    "total_build_and_run_seconds": 1200,
    "per_build_seconds": 600,
    "per_fixed_seed_seconds": 60,
    "aggregate_new_allocated_bytes": 536870912,
    "free_floor_bytes": 2147483648,
    "scope": "One main and one parent build/run, sequential; growth includes parent source export, targets, binaries and evidence. Stop without retries, cleanup or expansion on missing dependencies/time/growth/free limit.",
    "profile": "Same diagnostic O3/unwind/no-LTO/codegen16 with installedRust1.97.1. This qualifies source/feature comparison but NOT exact LinuxCI release-fast thin-LTO/abort reproduction."
  },
  "prediction_and_limits": "The approximately97MB parent export plus mostly warm dependency graph makes a512MiB bounded attempt plausible; compiler transient/output growth and engine compile duration remain unmeasured. A hard monitored stop is required. Existing rdf-canon issue6475 remains untouched; preserve any canonicalizer failure as a distinct outcome. This does not prove the original CI seed yet.",
  "expected_capture": [
    "Exact original seed4141222487, count1 on each qualified revision",
    "Allowlist-enabled startup line and original10-operation sequence compared to captured record",
    "Actual first failing step/reason/raw mismatch and strict internal-check boundary",
    "Exit codes, complete stdout/stderr, source/lock/features/binary hashes, actual compile invocations, disk/time receipts"
  ],
  "no_commands_pending": true
}

## Proposed entrypoint (not built)

```rust
// [GPT-6 ASTRA] Future harness only; not compiled or executed in this phase.
#[path = "QUALIFIED_REVISION/crates/sparq-bench/src/update_fuzz.rs"]
mod update_fuzz;
fn main() { update_fuzz::run(4141222487, 1); }
```
