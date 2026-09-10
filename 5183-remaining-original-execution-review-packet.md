# Original remaining seven UPDATE failures: fixed-list execution

Actual author: GPT-6 Astra (xhigh). No production changes.

```json
{
  "issue": 5183,
  "author": "GPT-6 Astra xhigh",
  "decision": "All seven original CI seed markers reproduced once on both recorded revisions; no new implementation or candidate-generator run.",
  "results": [
    {
      "seed": 4141222535,
      "index_zero_based": 7,
      "requests": 8,
      "parent_exit": 1,
      "main_exit": 1,
      "same_failure_and_sequence": true,
      "sequence_sha256": "08974ad3006e27ef48ded068730e1a99344b9555b16839258c24df25194e1860"
    },
    {
      "seed": 4141222571,
      "index_zero_based": 9,
      "requests": 10,
      "parent_exit": 1,
      "main_exit": 1,
      "same_failure_and_sequence": true,
      "sequence_sha256": "1a7fbbfbeddb03cd347c81578928d6a196355d4f5cbaca99a973e7f23b624298"
    },
    {
      "seed": 4141222576,
      "index_zero_based": 8,
      "requests": 9,
      "parent_exit": 1,
      "main_exit": 1,
      "same_failure_and_sequence": true,
      "sequence_sha256": "8458ff1f7ac29f3308e08d4f7eb7c8b360c31c830dc46c69f033400f4d17f0ca"
    },
    {
      "seed": 4141222599,
      "index_zero_based": 5,
      "requests": 6,
      "parent_exit": 1,
      "main_exit": 1,
      "same_failure_and_sequence": true,
      "sequence_sha256": "2db068e2087df1b60c887da420eee438b3c7c6d644e094ae80f27cb11e133186"
    },
    {
      "seed": 4141222612,
      "index_zero_based": 6,
      "requests": 8,
      "parent_exit": 1,
      "main_exit": 1,
      "same_failure_and_sequence": true,
      "sequence_sha256": "bc1f01746af39e035b1edc96392e9224f93f4cf4e1f6eb0e48b1fdafb5d579b2"
    },
    {
      "seed": 4141222617,
      "index_zero_based": 5,
      "requests": 7,
      "parent_exit": 1,
      "main_exit": 1,
      "same_failure_and_sequence": true,
      "sequence_sha256": "367128fa1d44a38a5a1916ca03e7cd37b9f98ce274d1162a8d5fbbc678477eb4"
    },
    {
      "seed": 4141222622,
      "index_zero_based": 4,
      "requests": 5,
      "parent_exit": 1,
      "main_exit": 1,
      "same_failure_and_sequence": true,
      "sequence_sha256": "4cd602a44149c850f83bdc8c354cb97d4b4ad5d4deaf88d19a931c84af955630"
    }
  ],
  "common_observation": "Every case fails comparing sparq(rebuild) to Oxigraph after INSERT { ?s <http://ex/p0> _:bt } WHERE { ?s <http://ex/p1> ?o }. Each preceding sequence explicitly inserts distinct8 and008 terms at s2/p1. Deltas show lexical008 plus one net extra bnode-bearing quad on Sparq; some canonical labels also shift.",
  "inference": "All seven local witnesses are consistent with the already-reproduced reference lexical-collapse/WHERE-cardinality mechanism of first seed4141222487. This phase does not add separate per-step WHERE cardinality instrumentation or prove every hypothetical alternate cause; strict comparator failure is preserved.",
  "lineage": "Identical original source0807388f, hash-verified d41 and781f dependencies; equal entire failure bodies and reconstructed sequences in both revisions. No evidence that6478 introduced these seven failures.",
  "ci_qualification": "Frozen raw CI only contained these seven MISMATCH seed markers. Indices, request sequences and printed LOAD mirrors here are newly deterministically reconstructed original-generator evidence, not recovered raw CI sequence text.",
  "driver_qualification": "New Rust2021 drivers match workspace edition; older diagnostic driver used2024. Unique driver/metadata/output paths and fixed-list CLI replace old hard-coded first seed, while original module and recorded dependency flags/features remain unchanged. macOS O3/unwind/noLTO/codegen16/Rayon1 differs from LinuxCI release-fast.",
  "limits": [
    "No candidate generator, seed search or outcome retries. Every seed was run once per revision; all14 exited1.",
    "Original run stops at first mismatch. Printed suffix requests were not executed; per-case count is in results.json.",
    "LOAD reference INSERT mirrors are printed and captured. Original sandbox behavior is unchanged; ephemeral LOAD files are not separately retained after its normal cleanup.",
    "These failures do not establish an engine fix, a comparator defect or passing all historical samples with candidate. Future source admission remains subject to independent review/CI."
  ],
  "resources": {
    "phase_seconds": 39.381189959000004,
    "sum_command_seconds": 34.23177266599998,
    "driver_build_seconds": {
      "parent-build": 17.56406175,
      "main-build": 9.503190499999999
    },
    "max_monitored_new_allocated_bytes": 22622208,
    "min_monitored_free_bytes": 2584088576,
    "final_free_bytes": 2584195072,
    "bounds": {
      "aggregate_seconds": 600,
      "build_seconds": 120,
      "seed_seconds": 30,
      "new_allocated": 134217728,
      "min_free": 2147483648,
      "jobs": 1
    }
  },
  "terminal": "Two builds and14 seed processes returned terminal codes; controller returned0. No managed command pending.",
  "next_small_step": "Root can fold these fixed-list observations into the existing5183 diagnosis/review. The repeated collision/template shape supports grouping the seven with the first reference-domain failure, with the stated instrumentation limit. No further replay or production edit is required by this phase."
}
```

## Exact original module and binary provenance

Original module SHA2560807388f84792f011b868456269a36bf74e5e463c67dfb71d1be3ad9d35aecf7 is byte-identical at both d41ec9fcb796504d85f5a5247a5fdc9eb3e65de5 and781f667c19a8ebb779cfccb24b05ea432360b025. Each new driver used only its recorded direct extern artifacts, rehashed before every command. No Cargo or dependency build ran.

## Per-seed exact localized detail

### Seed4141222535 (both revisions identical)

```text
canonical dataset differs (sparq(rebuild) vs oxigraph)
only in sparq(rebuild):
  <http://ex/s2> <http://ex/p0> _:c14n3 .
  <http://ex/s2> <http://ex/p0> _:c14n4 .
  <http://ex/s2> <http://ex/p1> "008"^^<http://www.w3.org/2001/XMLSchema#integer> .
  <http://ex/s4> <http://ex/p0> _:c14n2 .
only in oxigraph:
  <http://ex/s2> <http://ex/p0> _:c14n2 .
  <http://ex/s4> <http://ex/p0> _:c14n3 .

```

### Seed4141222571 (both revisions identical)

```text
canonical dataset differs (sparq(rebuild) vs oxigraph)
only in sparq(rebuild):
  <http://ex/s2> <http://ex/p0> _:c14n3 .
  <http://ex/s2> <http://ex/p0> _:c14n4 .
  <http://ex/s2> <http://ex/p1> "008"^^<http://www.w3.org/2001/XMLSchema#integer> .
  <http://ex/s4> <http://ex/p0> _:c14n2 .
only in oxigraph:
  <http://ex/s2> <http://ex/p0> _:c14n2 .
  <http://ex/s4> <http://ex/p0> _:c14n3 .

```

### Seed4141222576 (both revisions identical)

```text
canonical dataset differs (sparq(rebuild) vs oxigraph)
only in sparq(rebuild):
  <http://ex/s2> <http://ex/p0> _:c14n3 .
  <http://ex/s2> <http://ex/p0> _:c14n4 .
  <http://ex/s2> <http://ex/p1> "008"^^<http://www.w3.org/2001/XMLSchema#integer> .
  <http://ex/s4> <http://ex/p0> _:c14n2 .
only in oxigraph:
  <http://ex/s2> <http://ex/p0> _:c14n2 .
  <http://ex/s4> <http://ex/p0> _:c14n3 .

```

### Seed4141222599 (both revisions identical)

```text
canonical dataset differs (sparq(rebuild) vs oxigraph)
only in sparq(rebuild):
  <http://ex/s2> <http://ex/p0> _:c14n3 .
  <http://ex/s2> <http://ex/p1> "008"^^<http://www.w3.org/2001/XMLSchema#integer> .
only in oxigraph:

```

### Seed4141222612 (both revisions identical)

```text
canonical dataset differs (sparq(rebuild) vs oxigraph)
only in sparq(rebuild):
  <http://ex/s2> <http://ex/p0> _:c14n3 .
  <http://ex/s2> <http://ex/p1> "008"^^<http://www.w3.org/2001/XMLSchema#integer> .
only in oxigraph:

```

### Seed4141222617 (both revisions identical)

```text
canonical dataset differs (sparq(rebuild) vs oxigraph)
only in sparq(rebuild):
  <http://ex/s2> <http://ex/p0> _:c14n3 .
  <http://ex/s2> <http://ex/p1> "008"^^<http://www.w3.org/2001/XMLSchema#integer> .
only in oxigraph:

```

### Seed4141222622 (both revisions identical)

```text
canonical dataset differs (sparq(rebuild) vs oxigraph)
only in sparq(rebuild):
  <http://ex/s2> <http://ex/p0> _:c14n3 .
  <http://ex/s2> <http://ex/p1> "008"^^<http://www.w3.org/2001/XMLSchema#integer> .
only in oxigraph:

```

## Evidence scope

The bundle retains both per-revision raw stdout/stderr, new driver binaries/source/actual compiler argv, immutable original source, per-command identities, printed sequences and LOAD mirrors, feature proofs, resources and final clean-candidate proof. This compact packet omits long request bodies; their exact hashes and complete files are in outcomes/. Original compiler environment/build diagnostics are kept locally rather than embedded here.
