<!-- [OPUS-5.5] zkp-16.2: usage reference for the optional vcq adapter over the exact V3 relation. -->
# vcq adapter for the exact RISC Zero V3 evaluator

`sparq_proved_evaluator::vcq` (detached crate `zk/sparql-evaluator/host`, cargo feature `vcq`,
**off by default**) implements the `sparq-query-protocol` `QueryMethod` trait over the existing
V3 relation. It is experimental and not externally audited (sq-qhy4). It authenticates no
source credential, checks no credential status and no holder key. <!-- [OPUS-5.5] --> The research
[method registry](../../../research/vc-query-methods.json) lists `method:risc0-exact` with
`adapter_available` true for version 3 only, and only for the six tuples below (its
`vcq_adapter` entry); versions 1 and 2 stay `false`. The registry mirrors the adapter's local
`Capabilities` declaration; the declaration is not itself a registry entry. The existing
V1/V2/V3 host APIs and the engine replay bridge are unchanged.

```sh
cargo test --manifest-path zk/sparql-evaluator/Cargo.toml -p sparq-proved-evaluator --features vcq
cargo test --manifest-path zk/sparql-evaluator/Cargo.toml -p sparq-proved-evaluator-model \
  --features graph-results --test vcq_request_shape
```

## API

| Item | Purpose |
|---|---|
| `Risc0ExactV3::new(&ArtifactPin, AcceptedGuest)` | Method from an independently approved pin and guest; rejects a guest that does not match the pin |
| `.with_r0vm(PathBuf)` | Local `r0vm` used by `prove` |
| `.with_verifier(Identifier, fn() -> u64)` | Verifier audience and clock used by the trait `verify` |
| `system_unix_seconds()` | Default clock; a pre-epoch system time reads as `u64::MAX`, so every window rejects it as expired <!-- [OPUS-5.5] --> |
| `verify_at(request, audience, now_unix, admission, presentation, store)` | Verification with explicitly supplied audience and Unix time |
| `descriptor(&ArtifactPin)` | The exact `MethodDescriptor` a verifier lists in its request |
| `derive_nonce`, `stored_request_digest`, `descriptor_digest`, `statement_digest`, `parameter_digest` | Documented digests below |
| `expected_v3_request` | The V3 request both sides derive from a `StoredRequest` |
| `VcqPresentation` | Descriptor digest plus serialized receipt bytes |
| `V3Output`, `ReleasedResult` | Claim output: result, journaled provenance, commitment, V3 request digest |

`QueryMethod` associated types: `Request = StoredRequest`, `PrivateInputs = v3::PrivateDataset`,
`ChallengeStore = dyn ChallengeStore`. The witness is opaque, not `Clone`, and redacted in `Debug`.

## Descriptor and capabilities

- Descriptor: `urn:sparq:vcq:method:risc0-exact` version 3, parameter set
  `urn:sparq:vcq:params:risc0-exact:v3-default-policy` with `parameter_digest()`,
  `ZkvmGuest { artifact_digest: pin.sha256, image_id }` (each image-id word little-endian), backend
  `urn:sparq:vcq:backend:risc0-zkvm:3.0.6:succinct`.
- Six whole tuples: `SelectBag`, `AskBoolean`, `GraphRdfc10` (CONSTRUCT only), each under
  `VerifierAgreedAnchor` and `HolderDeclared`; `ExactBounded`; profile
  `urn:sparq:vcq:dialect:sparq-sparql11-graph-results:v3` / `urn:sparq:vcq:fragment:v3-static-admission`;
  `ExactSourceCatalog`; source evidence `None`, status `NotRequested`, holder `BearerAccepted`;
  mapping `urn:sparq:vcq:map:exact-source-bytes` (V3 via the descriptor version); linking
  `urn:sparq:vcq:link:committed-dataset-evaluation`.
- Enforcers: mapping, linking and query by the guest relation; the anchor by the public host
  check `urn:sparq:vcq:host:anchor-equality`, owed only under anchor authority. Authenticity,
  status and holder binding are `Absent`, so a claim reports them `NotEstablished`.
- Challenge: owner `Method`, `ConsumeOnSuccess`.
- Fixed default V3 policy only: dataset bytes 65,536, source quads 256, rows 4,096, named graphs
  16; canonicalization quads 32,768, input and output bytes 1 MiB, HNDQ calls 64, permutation
  steps 1,048,576; query bytes 8,192. Ceilings: 4,096 released rows, 16 MiB presentation bytes.

Rejected: SELECT sequences (outer ORDER BY), DESCRIBE, an explicit base IRI, set/true-only
contracts, and any authenticated, status or holder-binding request.

## Encodings

All digests are SHA-256; `local-struct-v1` is the protocol crate's local encoding.

- `parameter_digest`: `sparq:vcq:risc0-exact:parameters:v3\0`, then big-endian `u32` V3 version,
  the four dataset bounds, the five canonicalization bounds (declaration order), one DESCRIBE byte
  `0x01`, and the query bound as big-endian `u64`.
- `derive_nonce`: `sparq:vcq:risc0-exact:v3-nonce:local-struct-v1\0` ‖
  SHA-256(stored-request bytes) ‖ SHA-256(descriptor bytes). Every stored-request field,
  including the original challenge, audience and window, reaches the journaled V3 request digest.
- `statement_digest`: `sparq:vcq:risc0-exact:statement:v3\0` ‖ stored-request digest ‖
  descriptor digest ‖ SHA-256(exact verified receipt journal bytes).
- The V3 request digest keeps its existing pinned JSON format. Receipt JSON inside
  `VcqPresentation` is transport only and is never hashed as protocol data.

## Verification order

`prepare` and `verify` both recompute admission (a caller admission that differs is rejected)
and check the parsed query with the host-only `v3::query_shape` helper, so a DESCRIBE query
labelled CONSTRUCT, a SELECT labelled ASK or an ordered SELECT fails before any proof work or
challenge use. `verify` then checks audience, `not_before <= now < not_after`, the descriptor
digest, and the presentation-byte bound before decoding the receipt. The existing checked V3
path verifies the Succinct receipt with dev mode disabled, decodes the journal and binds the
request. The adapter then checks the anchor or provenance and the exact result form (bag rows of
unique variables and uniform arity, sorted, within the row bound; ASK boolean; canonical
N-Triples lines within the row bound) and builds the claim. Only after all of that does the V3
nonce call reach a private bridge. The bridge accepts one call with the expected derived nonce
and consumes the ORIGINAL challenge once through the shared store. Replay and store failure are
classified from the typed store outcome.

## Evidence status

Native tests (`host/tests/vcq_adapter.rs`, unit tests in `host/src/vcq.rs`,
`model/tests/vcq_request_shape.rs`) create no proof. The only receipt they use is a fake one,
which must be rejected without touching the store.

<!-- [OPUS-5.5] Records an independently certified run; this page re-executed nothing. -->
The genuine-receipt tests below were run once and independently certified at source
`872c219ca18c6cc978d2f5c705f020e8c69748a6`. The record:

- Seven genuine receipts, each `InnerReceipt::Succinct` with `ExitCode::Halted(0)`.
- Six were protocol-accepted: SELECT bag, ASK and CONSTRUCT, each under holder-declared and
  verifier-agreed authority.
- The seventh (`select-bag-row-bound`) was a valid V3 result of two rows against
  `released_rows = 1`. It was rejected as `capacity`
  `CapacityExceeded(Backend { "vcq-released-rows", .. })` before challenge consumption.
- 81 controls ran across the six accepted cases.
- Mutation check: the verify-only test failed against the consume-first mutant, as intended.
  The root's wrapper around it still reported overall failure, because its expected failure
  text was multiline. Treat the wrapper's verdict as not a pass.
- Source was then restored (506 files and 29 lock files checked, Git clean). A separate
  verify-only continuation and an all-targets Clippy run passed, with no new proof.
- Native gate at the same source: model, doc, `vcq`-feature and default test functions, three
  guard mutants and Clippy in both configurations. Eight older genuine-proof test functions
  were excluded, so do not cite this as a full gate.
- Evidence digests: run `d523b5405540c127c529749b5d0a571d308d348ed25d17ee27066c4457e9331b`,
  post-restore `5f8385d87f5d85dd49c043a31c1a28995218d95dcf679cdf6a8110989cb83eac`. The evidence
  itself stays outside the repository.

This is execution evidence for the adapter on a public synthetic fixture only. It shows no
issuer-authenticated credential, no status, no holder identity (bearer only), and no
completeness beyond the exact agreed bytes (no federation or nondeterministic dataset). It is
no benchmark. Any new claim needs its own run with the evidence listed below.

## Genuine receipt tests (`host/tests/vcq_genuine.rs`)

<!-- [OPUS-5.5] Test definitions; the one certified run is recorded under Evidence status. -->
Two ignored tests, compiled only with `--features vcq`, driven by one explicit job file named by
`SPARQ_VCQ_PROOF_JOB`. The tests read environment variables but never set them.
`RISC0_DEV_MODE` must be unset. A missing job, tool or input fails the test; nothing is skipped
or counted as a run.

```sh
# Prove mode: seven genuine Succinct receipts, then controls; writes new evidence.
SPARQ_VCQ_PROOF_JOB=/abs/vcq-prove-job.json RISC0_BUILD_LOCKED=1 cargo test --locked \
  --manifest-path zk/sparql-evaluator/Cargo.toml -p sparq-proved-evaluator --features vcq \
  --test vcq_genuine -- --ignored --exact \
  genuine_vcq_receipts_verify_every_tuple_and_reject_controls --nocapture --test-threads=1

# Verify-only mode: re-checks one retained row-bound receipt; proves and writes nothing.
SPARQ_VCQ_PROOF_JOB=/abs/vcq-verify-job.json RISC0_BUILD_LOCKED=1 cargo test --locked \
  --manifest-path zk/sparql-evaluator/Cargo.toml -p sparq-proved-evaluator --features vcq \
  --test vcq_genuine -- --ignored --exact \
  retained_row_bound_receipt_rejects_after_proof_checks_before_consumption --nocapture
```

Job schema `sparq.vcq-genuine-proof.test-job.v1` (unknown fields rejected):

| Field | Mode | Meaning |
|---|---|---|
| `schema` | both | Exactly `sparq.vcq-genuine-proof.test-job.v1` |
| `guest`, `pin` | both | Independently approved guest artifact and deployment-approved `ArtifactPin`, loaded with `AcceptedGuest::from_artifact`. The embedded guest is never used implicitly. |
| `challenge_seed32` | both | 64 hex characters, nonzero; a fresh PUBLIC synthetic seed, not a secret. Verify-only must reuse the seed of the run that produced the retained receipt. |
| `r0vm` | prove only | Real local `r0vm` executable (canonicalized, recorded) |
| `new_output_directory` | prove only | Absolute, must not exist, parent must exist, outside the checkout; created owner-only |
| `verify_only_row_bound` | verify-only only | The retained `select-bag-row-bound/presentation.json` |

A job is prove mode (`r0vm` + `new_output_directory`, no `verify_only_row_bound`) or verify-only
mode (`verify_only_row_bound` alone); anything else fails. Inputs must be absolute paths without
`.`/`..` that name regular files, not symlinks or directories. Read caps: job 64 KiB, pin 4 KiB,
guest 32 MiB, retained presentation 72 MiB (JSON writes each receipt byte as a number; the
adapter still applies its 16 MiB receipt bound).

Fixed PUBLIC SYNTHETIC fixture: default graph
`<http://ex/a> <http://ex/p> "1" .` and `<http://ex/b> <http://ex/p> "1" .`, no named graphs,
salt `0x5a` x 32. The agreed anchor is `v3::dataset_commitment` of that fixture under
`v3::Policy::default()`; holder-declared requests carry none. Verifier audience
`urn:example:sparq-vcq-genuine-test:verifier` and the window `not_before = 1800000000`,
`not_after = 1800000600`, `now = 1800000100` are fixed synthetic values passed to `verify_at`,
not the ambient clock or credentials. Each case's original challenge is SHA-256 over
`sparq:vcq-genuine-test:original-challenge:v1\0` ‖ seed ‖ u64-be label length ‖ case label.

Cases, each through `admit`, `prepare`, `prove` (configured `r0vm`) and `verify_at`. Expected
results are hand-written from the fixture, never taken from adapter or evaluator output:

| Case | Query | Expected |
|---|---|---|
| `select-bag-{verifier-agreed,holder-declared}` | `SELECT ?o WHERE { ?s <http://ex/p> ?o }` | columns `[o]`, two identical rows `"1"` (duplicate kept) |
| `ask-true-verifier-agreed` | `ASK { <http://ex/a> <http://ex/p> ?o }` | `true` |
| `ask-false-holder-declared` | `ASK { <http://ex/c> <http://ex/p> ?o }` | `false` |
| `construct-{verifier-agreed,holder-declared}` | `CONSTRUCT { ?s <http://ex/q> ?o } WHERE { ?s <http://ex/p> ?o }` | the two `<http://ex/q>` triples, sorted canonical N-Triples |
| `select-bag-row-bound` | bag query, `released_rows = 1` | genuine receipt; protocol rejection |

Assertions per accepted case: one store call; the claim's result, provenance, commitment
(the anchor), descriptor, contract, `ExactBounded` mode and statement digest; scope
`{authority, anchor (agreed only), SourceEvidence::None, RelativeToScope}`; tuple status
`NotRequested` and holder `BearerAccepted`; authenticity, status and holder binding
`NotEstablished`; the anchor established only under agreed authority. The receipt is
re-verified through `v3::verify_with_artifact` with a separate test-only nonce set. Only after
that check does the test read status: `InnerReceipt::Succinct`, nonempty seal and
`ExitCode::Halted(0)`.

Controls reuse that receipt and create no proof. Each asserts class, phase and exact code, and
the fresh control store must see zero calls: a changed query of the same form; a wrong challenge;
a changed stored audience with a matching supplied audience; a changed window that still
contains `now` (these four fail as `vcq-proof-rejected`, the cryptographic binding); a wrong
verifier audience; expired and not-yet-valid instants; a wrong descriptor digest; an altered
journal result; the scope substitution (agreed to holder, or holder to agreed); and a wrong
agreed anchor (agreed cases). Then: a second verify on the same store is `ChallengeReplayed`;
a broken store is `infrastructure` `ChallengeStoreFailure("test-broken")`; two concurrent
verifies on a fresh mutex store give exactly one success and one replay. Every store is an
in-memory test double, never a production store. Policy and capability admission negatives
stay in the native suite.

Row bound: `prove` succeeds because the released-row ceiling is checked in the post-proof
verify callback. The adapter must return `capacity` `CapacityExceeded(Backend {
"vcq-released-rows", requested: 2, ceiling: 1 })` with **zero** original-challenge store
calls. The same receipt then passes `v3::verify_with_artifact` against the independently derived
request, with test nonces. This establishes that the rejection comes after the cryptographic
checks and before consumption. The native bridge test `check_failure_before_consumption_keeps_the_challenge` does
not exercise that hook order; this test does.

Mutation check (root-run): edit `verify_checked_program` in `host/src/v3.rs` so the nonce call
runs before the caller check. Then run the verify-only command against the retained receipt. It
must FAIL on the capacity/zero-call assertion; the bridge then reports
`vcq-nonce-bridge-violation` after one store call. Restore the file (`git checkout --
zk/sparql-evaluator/host/src/v3.rs`) and rerun; it must pass. Verify-only mode proves nothing and
writes nothing, so it never marks a fresh proof or overwrites run evidence.

Evidence (prove mode, all new files, owner-only on Unix, never removed):
`metadata.json` is written first, with the job, pin, guest and descriptor hashes, the seed, the
fixture and hand-defined expectation hashes, the anchor, audience and clock. It is not a success
record. Each case directory holds `started.json` (written before proving),
`presentation.json` (`VcqPresentation`), `receipt.json` (host receipt), `v3-request.json`,
`stored-request.local-struct-v1.bin`, `descriptor.local-struct-v1.bin`,
`expected-result.json` and `journal.json` (verified). Accepted cases add `verified.json` after
verification and `record.json` after controls. The row-bound directory keeps its material and a
`record.json` marked `protocol_accepted: false`. `summary.json` is written only after every
assertion. It counts 6 protocol-accepted genuine receipts plus 1 genuine receipt rejected by the
row bound, and it counts controls separately. Limits: public synthetic data only; no source
credential, status or holder key is authenticated; not externally audited. <!-- [OPUS-5.5] -->
The certified run at `872c219ca` wrote a v1 summary, and that summary is frozen as written. Its
`registry_adapter_available` field is a `false` hardcoded in that test source. It predates the
registry's version 3 entry and says nothing about it. Newer source writes schema
`sparq.vcq-genuine-proof.test-summary.v2` (`SUMMARY_SCHEMA`). That schema replaces the field with
the string `registry_adapter_availability`: "not tested; this test reads no registry entry".
The certified run did not exercise the v2 source, and no v2 summary is part of its evidence.
Keep raw receipts and evidence outside the tracked repository.
