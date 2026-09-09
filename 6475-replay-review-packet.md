# Issue6475 replay/localization

```json
{
  "issue": 6475,
  "base": "e53464c73f31f7aca800f3867ac054c36408e346",
  "worktree": "/private/tmp/sparq-pr6049/.throughput-monitor/worktrees/issue6475",
  "branch": "codex/canon-relabel-invariance",
  "worktree_clean": true,
  "provenance": "Actual OpenAI GPT-6 Astra xhigh authored the scratch harness and evidence. Production source and cached dependencies were not modified.",
  "result": "REPRODUCED_AND_LOCALIZED_UPSTREAM. The direct rdf-canon0.15.3/oxrdf0.2.4 harness emits byte-identical left/right canonical strings to the Linux CI failure and exits1; the one-edge positive control passes. No Sparq crate, bridge, parser, proptest or engine is linked.",
  "executed": {
    "build": "cargo build --locked --offline",
    "runtime": "native aarch64-apple-darwin, pinned rustc1.97.1 (8bab26f4f68e0e26f0bb7960be334d5b520ea452)",
    "tests": "One exact saved three-quad pair plus one-edge calibration in a standalone main, not cargo test. No randomized cases or performance measurements.",
    "elapsed_total_seconds": 21.493258875008905,
    "jobs": 2,
    "free_before_bytes": 6969995264,
    "free_after_bytes": 6836699136,
    "floor_bytes": 6442450944,
    "limit_seconds": 300,
    "limits_triggered": false,
    "registry_package_drift_from_main": []
  },
  "dependency_proof": "Two direct dependencies,40 resolved packages. rdf-canon has no enabled features; oxrdf default. Locked checksum matches each cached .crate archive; all311 rdf-canon and16 oxrdf archive files equal expanded source. Full metadata/lock/feature and binary/source hashes preserved.",
  "bridge_exclusion": "The input quads are constructed directly as oxrdf0.2.4 values with checked term constructors. Their printed N-Quads exactly equal the saved source-derived CI fixtures. No oxrdf0.3\u2192text\u2192oxrdf0.2 bridge runs. This independently reproduces the failure below the bridge, so changing Sparq serialization is not the repair boundary.",
  "mechanism_evidence": "Sparq canonicalize_quads atlib.rs310\u2013312 simply bridges and invokes rdf_canon. Upstream canon.rs602\u2013662 constructs per-related-node hashes;708\u2013854 groups subject/object/graph mentions;320\u2013449 uses N-degree hashes and issuer order. The two ambiguous nodes b2/b3 exchange object versus graph-name positions across the q quads; fixed anchors b4/b0 are distinguished by the p triple. Their first-degree descriptions coincide; the related-node representation is the narrow investigation target. The replay proves an upstream path failure, but this phase did not instrument internal hashes or prove which algorithm line must change.",
  "repair_recommendation": {
    "boundary": "Persist the exact pair/seed as a permanent Sparq regression, then repair or patch-pin the upstream canonicalizer component shared by all standard canonicalize/issue/digest consumers. Keep the bridge, proptest case count and gate policy unchanged.",
    "not_yet_justified": "No safe one-line production correction is established. Sorting original labels, swallowing failure, special-casing these quads, or retrying for green would mask the defect. A speculative change to related-hash inputs may alter standardized output and needs specification/upstream comparison plus W3C regression validation.",
    "next_bounded_step": "Use the frozen minimal harness to capture upstream N-degree hash/issuer trace on this pair and compare the same pair with an independent established implementation before choosing a narrow dependency patch. Preserve exact baseline outputs and all existing conformance/poison-input limits."
  },
  "ownership": {
    "area_query": "repo:sparq-org/sparq is:pr is:open label:area:sparq-canon",
    "area_results": 0,
    "mention_query": "repo:sparq-org/sparq is:pr is:open \"sparq-canon\"",
    "mention_results": [
      5357,
      6148
    ],
    "relevant_pr": {
      "number": 5357,
      "head": "d460484e050889b931cc4b83b5cae09a09992d6f",
      "state": "OPEN DRAFT",
      "labels": [
        "review:needs-user",
        "area:docs"
      ],
      "files": [
        "research/sparq-canon-oxrdf03-bridge-removal.md"
      ],
      "action": "Preserve its hold and historical authorship; this diagnostic does not adopt its bridge-removal design or supersede its review. Its only source change is documentation, so no confirmed Rust hunk conflict."
    },
    "limits": "Focused searches and5357 file listing, not a complete path census of every open PR.6148 title/body indicates site/guide documentation; its files were not fetched."
  },
  "upstream_lookup": {
    "query": "repo:zkp-ld/rdf-canon is:issue relabel",
    "count": 0,
    "limits": "One focused lookup does not establish no upstream duplicate or fix."
  },
  "scope_limits": [
    "Native macOS reproduction, not new Linux/wasm conformance.",
    "No actual Sparq library build due disk/engine dev-dependency constraints; original CI supplies the actual Sparq-path failure.",
    "No production repair, commit, push, rerun, requeue, model invocation, registry/release/EC2 change.",
    "All6095 evidence remains unchanged; issue6475 worktree stays clean."
  ],
  "paths": {
    "target": "/private/tmp/sparq-pr6049/.throughput-monitor/direct-6475/replay/target",
    "target_allocated_kib": 94872,
    "binary": "/private/tmp/sparq-pr6049/.throughput-monitor/direct-6475/replay/binary/canon-relabel-replay",
    "harness": "/private/tmp/sparq-pr6049/.throughput-monitor/direct-6475/replay/harness",
    "input_bundle_manifest": "1ef65eb20334c6934a82876cf712658c7bb383ac4f344107e4757da45ba46239"
  }
}
```

## Exact harness

```rust
// [GPT-6 Astra] Issue #6475: pinned upstream replay of the saved CI counterexample.
// No sparq crate, bridge, parser, proptest, or engine is linked in this harness.
use oxrdf::{BlankNode, GraphName, NamedNode, Quad};
use std::error::Error;

fn dataset(labels: &[&str; 6]) -> Result<Vec<Quad>, Box<dyn Error>> {
    let b = labels
        .iter()
        .map(|label| BlankNode::new(*label))
        .collect::<Result<Vec<_>, _>>()?;
    let p = NamedNode::new("http://ex/p")?;
    let q = NamedNode::new("http://ex/q")?;
    Ok(vec![
        Quad::new(
            b[4].clone(),
            q.clone(),
            b[2].clone(),
            GraphName::BlankNode(b[3].clone()),
        ),
        Quad::new(b[4].clone(), p, b[0].clone(), GraphName::DefaultGraph),
        Quad::new(
            b[0].clone(),
            q,
            b[3].clone(),
            GraphName::BlankNode(b[2].clone()),
        ),
    ])
}

fn main() -> Result<(), Box<dyn Error>> {
    let original = dataset(&["b0", "b1", "b2", "b3", "b4", "b5"])?;
    let renamed = dataset(&["zz0", "zz1", "zz4", "zz3", "zz5", "zz2"])?;
    for (name, quads) in [("original", &original), ("renamed", &renamed)] {
        for quad in quads {
            println!("input_{name}: {quad} .");
        }
    }
    let control_original = rdf_canon::canonicalize_quads(&original[1..2])?;
    let control_renamed = rdf_canon::canonicalize_quads(&renamed[1..2])?;
    println!(
        "single_edge_control_equal={}",
        control_original == control_renamed
    );
    assert_eq!(
        control_original, control_renamed,
        "positive control must canonicalize"
    );
    let left = rdf_canon::canonicalize_quads(&original)?;
    let right = rdf_canon::canonicalize_quads(&renamed)?;
    println!("original_canonical={left:?}");
    println!("renamed_canonical={right:?}");
    println!("relabel_invariant={}", left == right);
    if left != right {
        return Err(
            "saved relabeling pair violates canonicalization invariance in direct rdf-canon".into(),
        );
    }
    Ok(())
}

```

## Actual output

```text
input_original: _:b4 <http://ex/q> _:b2 _:b3 .
input_original: _:b4 <http://ex/p> _:b0 .
input_original: _:b0 <http://ex/q> _:b3 _:b2 .
input_renamed: _:zz5 <http://ex/q> _:zz4 _:zz3 .
input_renamed: _:zz5 <http://ex/p> _:zz0 .
input_renamed: _:zz0 <http://ex/q> _:zz3 _:zz4 .
single_edge_control_equal=true
original_canonical="_:c14n0 <http://ex/q> _:c14n3 _:c14n2 .\n_:c14n1 <http://ex/p> _:c14n0 .\n_:c14n1 <http://ex/q> _:c14n2 _:c14n3 .\n"
renamed_canonical="_:c14n0 <http://ex/q> _:c14n2 _:c14n3 .\n_:c14n1 <http://ex/p> _:c14n0 .\n_:c14n1 <http://ex/q> _:c14n3 _:c14n2 .\n"
relabel_invariant=false
Error: "saved relabeling pair violates canonicalization invariance in direct rdf-canon"

```

## Commands

```json
[
  {
    "name": "toolchain",
    "argv": [
      "/Users/jesght/.rustup/toolchains/1.97.1-aarch64-apple-darwin/bin/rustc",
      "-Vv"
    ],
    "cwd": "/private/tmp/sparq-pr6049/.throughput-monitor/direct-6475/replay/harness",
    "exit": 0,
    "expected_exit": 0,
    "free_before": 6969995264,
    "free_after": 6969982976,
    "elapsed_total_seconds": 0.25610883301123977,
    "limit": null
  },
  {
    "name": "format",
    "argv": [
      "/Users/jesght/.rustup/toolchains/1.97.1-aarch64-apple-darwin/bin/rustfmt",
      "--edition",
      "2024",
      "/private/tmp/sparq-pr6049/.throughput-monitor/direct-6475/replay/harness/src/main.rs"
    ],
    "cwd": "/private/tmp/sparq-pr6049/.throughput-monitor/direct-6475/replay/harness",
    "exit": 0,
    "expected_exit": 0,
    "free_before": 6969974784,
    "free_after": 6969933824,
    "elapsed_total_seconds": 0.5215050830156542,
    "limit": null
  },
  {
    "name": "resolve",
    "argv": [
      "/Users/jesght/.rustup/toolchains/1.97.1-aarch64-apple-darwin/bin/cargo",
      "metadata",
      "--offline",
      "--format-version",
      "1"
    ],
    "cwd": "/private/tmp/sparq-pr6049/.throughput-monitor/direct-6475/replay/harness",
    "exit": 0,
    "expected_exit": 0,
    "free_before": 6969913344,
    "free_after": 6969913344,
    "elapsed_total_seconds": 1.832613417005632,
    "limit": null
  },
  {
    "name": "build",
    "argv": [
      "/Users/jesght/.rustup/toolchains/1.97.1-aarch64-apple-darwin/bin/cargo",
      "build",
      "--locked",
      "--offline"
    ],
    "cwd": "/private/tmp/sparq-pr6049/.throughput-monitor/direct-6475/replay/harness",
    "exit": 0,
    "expected_exit": 0,
    "free_before": 6969905152,
    "free_after": 6836940800,
    "elapsed_total_seconds": 20.95031620800728,
    "limit": null
  },
  {
    "name": "replay",
    "argv": [
      "/private/tmp/sparq-pr6049/.throughput-monitor/direct-6475/replay/target/debug/canon-relabel-replay"
    ],
    "cwd": "/private/tmp/sparq-pr6049/.throughput-monitor/direct-6475/replay/harness",
    "exit": 1,
    "expected_exit": 1,
    "free_before": 6836936704,
    "free_after": 6836699136,
    "elapsed_total_seconds": 21.493258875008905,
    "limit": null
  }
]

```
