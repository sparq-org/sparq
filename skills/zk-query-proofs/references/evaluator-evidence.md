# Exact-evaluator campaign evidence

[GPT-6] The mandatory `real exact-dataset guest and proof` job executes
`scripts/ci_exact_evaluator_evidence.py --output NEW_DIRECTORY`. This GitHub
Actions helper requires the actual event context and installed pinned toolchain;
it has no mock or missing-tool success path. It runs native semantic tests,
every host integration test (including actual guest rejection and genuine proof
tests), and detached-workspace Clippy. Relevant changes include the vendored SDK.

Each run exports the following as one SHA-pinned Actions upload:

- `source.json`: checked-out commit/tree and SHA-256 inventory of tracked files.
- Host and guest Cargo metadata: locked dependency resolution and enabled features.
- `artifact/guest.bin` and `artifact/pin.json`: the exact executable and its full
  byte digest plus RISC Zero image ID.
- `receipts/`: synthetic integration-test receipts, independently verified with
  the exported image after their request-binding/tamper assertions pass.
- Command logs and `evidence.json`: actual toolchain versions/binary hashes,
  command statuses and durations, fixture inventory and observed HAL targets.

The helper rebuilds every local path package in both host and guest target
namespaces, including the patched SDK. Locked registry dependencies retain their
cache. It compares source identity before/after execution, re-exports the
embedded guest after lint, and rejects artifact drift or missing receipt exports.
Receipt export also checks that the tested binary's embedded pin equals the
exported pin. An existing output directory cannot be overwritten or reused.

For a pull request, `checkout_sha` identifies the tested merge checkout while
`pr_head_sha` identifies the branch tip from the event. Merge-group head/base and
run attempt are recorded separately. These values must not be relabeled as a
different source commit. The JSON is build evidence, not an independently signed
attestation of GitHub identity or source review.

`evidence.json` with `completed: true` exists only after every command and final
integrity check succeeds. Failed runs may upload logs and `failure.json`; such
partial artifacts do not establish successful execution. Hermetic Python tests
exercise collector rejection paths with explicitly synthetic byte/envelope
fixtures; those tests do not establish any guest execution or proof validity.

Narrow RISC Zero debug targets expose kernel dimensions and executing HAL module
names during synthetic tests. The record lists actual observed module targets;
it does not infer Metal from Apple Silicon, CUDA from a GPU, or the recursion
backend from an unlogged phase. The complete filter is recorded. Broad witness
or preflight TRACE logging is not enabled. These logs describe the local prover,
not information promised private by the receipt contract.

Download the accepted executable and pin together for deployment. An independent
release/verifier decision must still approve that program. A holder-provided pin
or a newly rebuilt artifact is not interchangeable with this accepted artifact;
cross-path and cross-platform reproducibility are not established here.

V1 and V2 campaigns are separate program executions. A V2 campaign must export
all V1 fixture receipts plus its own named-graph receipts from the same V2 image.
Success of that program cannot establish execution under a separately published
V1 image. Historical committed proof records retain their original source and
artifact identities; they are not updated by inference from a later native test.

The [experimental privacy assumptions](exact-evaluator.md) and dataset authority
boundaries apply unchanged. Synthetic correctness fixtures and actual receipt
verification are neither a cryptographic audit nor full SPARQL conformance.

A local operator can use the same runner with `--local --output NEW_DIRECTORY`.
It records a local checkout identity and no hosted or PR-head attestation; this
mode is refused inside GitHub Actions. Both modes require a clean tracked checkout
and reject non-ignored untracked files before rebuilding local source packages.
