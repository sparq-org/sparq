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

Guest package invalidation explicitly selects `--release --target
riscv32im-risc0-zkvm-elf`; Cargo's default host/debug clean scope does not clear
those artifacts. A real read-only dry run checked this distinction before the
first exported campaign. The host package rebuild uses its actual default scope.

[OPUS-5.5] zkp-14.5: the campaign also covers the separately pinned V5 guest
(`methods/guest-authrdf`), which has its own Cargo metadata, locked fetch and
guest/release rebuild scope in the `sparq-authrdf-guest` target subdirectory.
It exports `authrdf-artifact/guest.bin` and `authrdf-artifact/pin.json`
separately from the exact `artifact/`. After the exact steps it runs the V5
native model and host gates and a Clippy pass with the host `authenticated-rdf`
feature, then re-exports both guests to reject drift. `evidence.json` keeps
`artifact_pin` for the exact guest and adds `guest_artifact_pins`, keyed by
`sparq-exact-guest` and `sparq-authrdf-guest`. The two pins must differ in both
digest and image ID. These zkp-14.5 steps execute no V5 guest, and the campaign
creates no V5 receipt, as `authrdf_scope` records. Until
`methods/guest-authrdf/Cargo.lock` exists, these locked steps fail and the
campaign does not complete. This change has not yet been run.

[OPUS-5.5] zkp-14.6: after `lint-authrdf`, the campaign runs the commands named in
`VCQ_COMMANDS`, each through the same fail-closed `run_logged` path, before both
guests are re-exported. None of them has run at this source.

| Command | Scope | Cargo selection (`--locked`, evaluator manifest) |
|---|---|---|
| `native-vcq` | native | `test -p sparq-proved-evaluator --features vcq --lib --test vcq_adapter` |
| `native-vcq-authenticated` | native | `test -p sparq-proved-evaluator --features vcq-authenticated --lib --test vcq_authenticated --test vcq_authenticated_genuine --test vcq_adapter` |
| `lint-vcq-authenticated` | lint | `clippy --workspace --all-targets --features sparq-proved-evaluator/vcq-authenticated -- -D warnings` |
| `actual-authrdf-direct-execution` | direct SDK execution | `test -p sparq-proved-evaluator --features authenticated-rdf --test actual_authenticated_rdf -- --ignored --nocapture --test-threads=1` |

- `native-vcq` checks the V3 adapter, whose helpers the V5 adapter now shares,
  under its own feature. `native-vcq-authenticated` runs the library unit tests
  (including the `vcq_authenticated` module's), the V5 adapter's native gates,
  the non-ignored job-parser and policy tests in `vcq_authenticated_genuine.rs`,
  and the V3 adapter tests again with both features enabled.
- `actual-authrdf-direct-execution` is the only command with `--ignored`, and it
  selects one target. Its four tests run the embedded V5 guest (and one exact-guest
  control rejecting V5 input) in the real `r0vm` executor. They create no proof,
  receipt or presentation, and a direct execution never counts as a receipt.
  `RISC0_DEV_MODE` must be unset for these tests, not merely `0`.
- The ignored genuine drivers (`authenticated_rdf_genuine`,
  `vcq_authenticated_genuine`'s prove test, `vcq_genuine`) are never selected: each
  needs independently approved pins and an explicit bounded job.
- `evidence.json` adds `authrdf_command_scopes` (every V5 and vcq command as
  `artifact`, `native`, `lint` or `direct-sdk-execution`) and `authrdf_receipts: 0`.
  The receipt collector still requires exactly the V1–V3 fixture set, so any V5
  receipt export fails the campaign. `authrdf_scope` states the same split.

Hermetic Python tests pin each command's feature, `--lib` and `--test` targets,
each target's feature gate, the single `--ignored` use and the scope table. They
do not show that any command ran; registry `adapter_available` stays false for
the V5 adapter.
