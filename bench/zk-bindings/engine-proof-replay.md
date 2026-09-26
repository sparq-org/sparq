# Engine replay to exact V3 proof bridge

[OPUS-5.5] This experimental adapter takes one cell retained by the
[native engine/storage replay](engine-replay.md) and prepares its unchanged
`query.rq` and `data.ttl` for the exact V3 relation of the
[proved evaluator](../../zk/sparql-evaluator/README.md). In real mode it creates
one genuine local Succinct receipt with `v3::prove_with_artifact` and checks it
with `v3::verify_with_artifact` against a verifier-owned request. It is not
externally audited.

Scope is exact evaluation of the retained query over the retained dataset. The
adapter authenticates no source credential and makes no W3C VC signature claim;
source-credential authentication is a separate, upcoming method contract.
`HolderDeclared` proves computation over holder-chosen bytes only: it does not
authenticate the dataset or establish wallet completeness. `VerifierAgreed` shows
that the input opens the verifier's anchor; in this harness that anchor is
test-setup trust input, not source authentication.

## What the receipt covers

Turtle-to-N-Quads conversion and the original-file SHA-256 checks
(`record.json`, `query.rq`, `data.ttl`) are host harness checks run before
proving. The guest does not execute them, and a receipt does not attest to them. A
genuine receipt authenticates only V3 evaluation of the request over the
converted N-Quads and named-graph catalog it commits to. A verifier can bind
those converted inputs to the originals only through a `VerifierAgreed` anchor
that it prepared itself from the originals. A `HolderDeclared` proof alone
authenticates neither the original Turtle hashes nor the correctness of the
transformation. `status.json` and the other output files record experiment
provenance. They are not new public proof claims and never extend what a
receipt states.

The native replay controller, matrix and classification are unchanged. Count-only
and other non-agreement cells stay non-agreement; a V3 receipt over a cell's
originals does not reclassify its native comparison and says nothing about the
native storage mode or feature profile that produced the record.

## Inputs and roles

| Input | Owner | Contents |
| --- | --- | --- |
| `REPLAY_DIR` | retained native run | one cell's `record.json`, `query.rq`, `data.ttl` |
| `HOLDER.json` | holder | synthetic-input declaration and published fixed salt |
| `VERIFIER.json` | verifier | cell identity, original file digests, complete `v3::Request` |
| `EXPECTED.json` | independent oracle | declared provenance and expected `v3::CanonicalResult` |
| guest, pin, `r0vm` | deployment | accepted artifact, independently accepted `ArtifactPin`, local server |

`VERIFIER.json` (`sparq.engine-replay-proof.verifier.v1`) declares the cell and
lowercase SHA-256 digests of the three retained files. Its request query must be
the exact `query.rq` text. The adapter never rewrites the request: authority,
agreed commitment, policy and nonce are compared, not recomputed or replaced.

```json
{
  "schema": "sparq.engine-replay-proof.verifier.v1",
  "cell": {"profile": "shipped", "category": "bgp", "seed": 0, "storage": "dense"},
  "originals": {"record_sha256": "…", "query_sha256": "…", "data_sha256": "…"},
  "request": {
    "version": 3, "contract": "ExactDataset", "dialect": "SparqSparql11GraphResultsV3",
    "query": "…exact query.rq text…",
    "authority": "HolderDeclared",
    "policy": "…serialized v3::Policy::default() or a stricter verifier policy…",
    "nonce": "…32 verifier-chosen bytes as a JSON array, not all zero…"
  }
}
```

A verifier-agreed request uses `{"VerifierAgreed": {"commitment": […32 bytes…]}}`.
For these synthetic cells, the verifier-side setup derives that anchor from the
retained original, never from a holder witness:

```rust
use sparq_proved_evaluator_model::{replay, v3};

let policy = v3::Policy::default();
let source = replay::convert_turtle(&data_ttl, &record_catalog, &policy)?;
let anchor = v3::dataset_commitment(&source.into_dataset(published_salt), &policy)?;
```

`HOLDER.json` (`sparq.engine-replay-proof.holder.v1`) must set
`"synthetic_inputs": true` and `"salt": {"SyntheticFixed": […32 bytes…]}`. A
fixed reproducible salt is admitted only for explicitly synthetic inputs; there
is no production-salt mode and no silent substitution of one for the other.

`EXPECTED.json` (`sparq.engine-replay-proof.expected.v1`) binds `query_sha256`
and `data_sha256`, so one expectation serves every storage/profile copy of the
same originals. `oracle.kind` is `ReviewedDerivation` or
`ReviewedIndependentImplementation`; `source` and `reviewer` are required. There
is no kind for the evaluator under test, and the native or reference replay
results are differential observations, not goldens. A reviewed conversion of the
reference observation may be declared as an independent implementation. SELECT
tables compare with one global blank-node bijection and bag multiplicity; ASK
values and RDFC-1.0 canonical graph N-Triples compare exactly.

## Conversion

`sparq_proved_evaluator_model::replay` (model feature `graph-results`, host-only)
parses the retained Turtle with the already-resolved `oxttl` parser and no
external base. IRIs, datatype IRIs and literal lexical forms are copied unchanged
(`01` stays `"01"^^xsd:integer`). Prefixed names and a document-declared `@base`
are expanded. Blank nodes become `_:b0`, `_:b1`, … by first occurrence, so
anonymous parser labels are deterministic. Statement order and duplicates are
retained. The witness is never derived from `observed_ntriples`, result rows or
any other record field. Turtle has only a default graph; the record's
`named_graph_catalog` supplies empty named graphs, with blank-node graph names
rejected and every other catalog rule left to V3.

Conversion stops once N-Quads plus catalog bytes exceed the request's V3 dataset
byte policy, so prefix expansion is bounded. Malformed Turtle and a needed but
undeclared base are distinct `SourceError` values; the base case is detected by
a discarded second parse, not by reading parser messages. Existing V3 commitment,
admission and evaluation rules then apply unchanged, including rejection of
source triple terms and directional literals.

## Invocation

Build with the [pinned evaluator toolchain](../../zk/sparql-evaluator/README.md).

```sh
cargo build --locked --release --manifest-path zk/sparql-evaluator/Cargo.toml \
  -p sparq-proved-evaluator --example engine_replay_proof

zk/sparql-evaluator/target/release/examples/engine_replay_proof prepare \
  /retained/run/shipped-bgp-0-dense holder.json verifier.json expected.json \
  /tmp/new-engine-prepare

zk/sparql-evaluator/target/release/examples/engine_replay_proof real \
  /retained/run/shipped-bgp-0-dense holder.json verifier.json expected.json \
  /reviewed/guest.bin /trusted/pin.json /installed/r0vm /tmp/new-engine-proof
```

Obtain the artifact with the existing `export_guest` example and accept its pin
through a trusted channel; never derive the pin from the offered artifact. Real
mode requires `RISC0_DEV_MODE` to be unset and `r0vm` to report 3.0.6. Every
file read has an explicit adapter bound, including the native controller's
record bound for retained files.

Exit status 0 means prepared, or proved and independently verified. Status 1
means a typed rejection recorded in `status.json`. Status 2 means a usage or
infrastructure failure; partial outputs are retained without a status record.

## Outputs

The output directory must be new and outside both the source checkout and the
replay directory.

- `status.json` (`sparq.engine-replay-proof.status.v1`): mode, status, stage,
  disposition and exact diagnostic; the cell; digests of all accepted inputs;
  oracle provenance; verifier authority, public request digest and V3 request
  digest; dataset commitment; `reuse_allowed: false`; proof and verified counts;
  harness checks and controls; tool identities in real mode.
- `public/request.json`: the verifier request as used.
- `public/presentation.json`, `public/journal.json`: real mode only.
- `private/witness-summary.json`: N-Quads, witness JSON and guest-input digests,
  statement and blank-node counts. It is created owner-only on Unix. Digests of
  low-entropy test data can be matched by guessing; never publish this file.

Dispositions are `ContractViolation`, `Unsupported`, `Capacity`, `Malformed` and
`Unexpected`. Only a V3 diagnostic registered as a profile refusal in
[rejections.json](rejections.json), or an excluded blank-node graph name, is
`Unsupported`. Unregistered diagnostics and typed deadline, cancellation or
execution causes are `Unexpected`, never an exclusion. Real-mode stages add
`ProofFailure`, `VerificationFailure` and `ControlFailure`.

## Evidence stages

1. Native replay (existing): original generator, oracle comparison, no proof.
2. `prepare`: file and record identity, verifier request, Turtle conversion,
   V3 commitment and agreed anchor, V3 admission, native V3 evaluation and the
   independent expectation. Proof count stays zero.
3. `real`: one genuine Succinct receipt, independent verification with the
   verifier's request and a fresh single-use nonce store, the independent
   expectation, a differential check against native evaluation, and
   verification-only controls.

Controls change verifier expectations (nonce, query bytes, commitment,
authority), replay a consumed nonce, alter public journal bytes (result and
commitment), damage the seal and present a fake receipt. Each requires its exact
host error. The prover never receives a result, so journal edits are integrity
checks, not malicious-witness checks. The malicious-witness case is the ignored
test's omitted statement under a verifier-agreed anchor, where the guest aborts
and no receipt exists. The same omission under `HolderDeclared` is a valid
computation that verification binding accepts; only the independent oracle can
notice a changed result, and it is not a wallet-incompleteness rejection.

## Proposed harness scope

Group retained cells by `(query_sha256, data_sha256)`. Run `prepare` for every
cell, so each record's identity and conversion are checked. Run `real` for one
representative per distinct original input and authority mode only. Storage and
profile copies share input identity, but that identity neither creates a new
proof per copy nor lets a proof be reused as evidence for another cell. Keep the
native matrix denominator and the proof denominator separate.

## Tests

```sh
cargo test --locked --manifest-path zk/sparql-evaluator/Cargo.toml \
  -p sparq-proved-evaluator-model --features graph-results --test engine_replay
cargo test --locked --manifest-path zk/sparql-evaluator/Cargo.toml \
  -p sparq-proved-evaluator --example engine_replay_proof
SPARQ_ENGINE_REPLAY_PROOF_JOB=/abs/job.json RISC0_SERVER_PATH=/installed/r0vm \
  cargo test --locked --release --manifest-path zk/sparql-evaluator/Cargo.toml \
  -p sparq-proved-evaluator --test actual_engine_replay -- --ignored --nocapture
```

Native contract tests in `model/tests/engine_replay.rs`:
`original_turtle_converts_to_exact_nquads_without_lexical_normalization`,
`conversion_is_deterministic_and_independent_of_source_blank_labels`,
`malformed_unknown_base_and_blank_graph_names_are_distinct_source_errors`,
`converted_bytes_are_bounded_by_the_request_policy_before_expansion_grows`,
`both_authorities_prepare_the_original_and_match_the_hand_derived_oracle`,
`observations_and_record_fields_never_become_input_or_expectations`,
`retained_record_and_file_identity_mismatches_fail_closed`,
`verifier_request_query_and_anchor_mismatches_fail_closed`,
`independent_expected_result_mismatch_is_not_agreement`,
`blank_node_expectations_use_one_global_bijection`,
`unsupported_profile_refusal_is_distinct_from_unexpected_failure`,
`agreed_omission_fails_its_anchor_but_holder_omission_is_only_an_oracle_mismatch`
and `manifests_reject_unknown_fields_and_other_protocol_versions`. They use
synthetic records and hand-derived expectations and produce no receipts.

The ignored `real_engine_replay_cell_proves_both_authorities_and_rejects_substitutions`
in `host/tests/actual_engine_replay.rs` reads a
`sparq.engine-replay-proof.real-test.v1` job with `replay_directory`, `holder`,
`holder_declared_request`, `verifier_agreed_request`, `expected`, `guest` and
`pin` paths. It proves one retained cell under both authorities, runs every
control and checks the omitted-statement malicious witness. Its two requests
must name the same originals with distinct nonces. It is not part of the
default run, and heavy lanes are not wired into CI until source and native
review.

## Execution status

At authoring time, no test, build or proof for this adapter has been executed.
Executed real proofs: zero. Configured but unexecuted: two genuine receipts plus
one expected guest abort in the ignored test, and one receipt per real-mode CLI
invocation. Native definitions above are not execution evidence.

## Known limitations

- V3-specific diagnostics (for example the V3 EXISTS blank-node correlation
  refusal) are absent from `rejections.json`, so they report `Unexpected` until
  reviewed there.
- Sequence results that order blank nodes follow the evaluator's tie policy over
  relabeled source nodes and may differ from the native run's order.
- Language tags follow the parser's handling; other lexical forms are copied.
- Only default-graph Turtle cells and synthetic fixed salts are admitted; the
  nonce store is in-process and is not persistent replay protection.
- There is no CLI for the verifier-side anchor setup; use the Rust recipe above.
