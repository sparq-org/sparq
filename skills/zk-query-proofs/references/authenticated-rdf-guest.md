<!-- [OPUS-5.5] zkp-14.5: usage reference for the separate V5 guest and low-level host API. -->
# Issuer-authenticated RDF: separate V5 guest and host API

**Built and directly executed; one genuine receipt, through the adapter.** At
source `42d13fed`, an independently audited scoped gate built both guests,
passed the native host tests and executed the V5 guest directly, without
proving; see [recorded evidence](#recorded-evidence). The one low-level
genuine-receipt job attempted at that source timed out before completing a
proof. Separately, the same approved V5 guest (`c35f5e4b`) produced one
independently audited genuine receipt through the vcq adapter at frozen source
`7fe88955`, for one case only; see the
[adapter reference](vcq-authenticated-rdf-adapter.md#recorded-genuine-validation).
The low-level driver has no receipt. It is experimental, research grade, not yet
sound and not externally audited (`sq-qhy4` is open).
<!-- privacy-claims-allow: direct execution plus one adapter-driven synthetic receipt; unaudited, sq-qhy4 open -->

The relation is the unchanged native V5 model described in the
[native model reference](authenticated-rdf-model.md). This slice adds a guest
image that runs it and a low-level host API; this slice itself adds no generic
`QueryMethod` adapter. The separate generic
[V5 vcq adapter](vcq-authenticated-rdf-adapter.md) (host feature
`vcq-authenticated`) builds on this API. It has passed a scoped native gate
and one genuine case, `select-bag-verifier-agreed`; its other five tuples and
its row-bound case have not run. The research registry lists
`method:risc0-authenticated-rdf` version 5 with `adapter_available: false`.

## Features and images

| Crate (detached `zk/sparql-evaluator`) | Feature | Effect |
|---|---|---|
| `sparq-proved-evaluator-methods` | `authenticated-rdf` (off) | Also builds `methods/guest-authrdf` (package `sparq-authrdf-guest`) and generates `SPARQ_AUTHRDF_GUEST_ELF` / `SPARQ_AUTHRDF_GUEST_ID` in their own `authrdf_methods.rs` |
| `sparq-proved-evaluator` (host) | `authenticated-rdf` (off) | Enables the model and methods features, the `authenticated_rdf` module, `embedded_authrdf_artifact()`, `embedded_authrdf_pin()` and the `export_authrdf_guest` example |

The V5 guest is an independent Cargo workspace with its own manifest and its
own lock. It uses the model features `graph-results` and `authenticated-rdf`,
and the same exact SDK version, vendored patches and release profile as the
exact guest. The build writes it to its own target subdirectory,
`sparq-authrdf-guest`. It keeps the exact guest's fail-closed rules: no skipped
build, a `--locked` child Cargo, source-path remapping, `location-detail=none`
and removal of the wrapper variables from the child environment.

The exact V1–V3 guest (`methods/guest`, `SPARQ_EXACT_GUEST_*` in `methods.rs`)
is always built with unchanged flags, target subdirectory, manifest, lock and
constant names in source. Enabling the feature adds a second image and a second
generated file; it never replaces or shares the first. The generated files embed
the absolute `OUT_DIR` artifact path, so their text varies between builds. At
`42d13fed` the exact guest's bytes and image ID were the same with the feature
off and on, and the V5 guest's differed from both. Built from the same absolute
path, the exact guest's bytes at `8322a6ff` (before this slice) and `6bf6314a`
were identical. An earlier comparison across differing build paths failed and
is retained, so reproducibility from an arbitrary path is not established.
Direct execution observed each image rejecting the other's input.

`methods/build.rs` reruns when any image source input changes. For both images
that is the root manifest, `sparq-canon`, `sparq-core`, `sparq-engine`,
`sparq-substrate`, `vendor/spargebra`, the four patched SDK crates, the model
and the exact guest workspace. With the feature it also watches
`methods/guest-authrdf`. `package.metadata.risc0.methods` lists both guest
directories; only this `build.rs` builds them.

`methods/guest-authrdf/Cargo.lock` is committed, taken from an independently
reviewed real Cargo resolution (SHA-256
`c40877fd9d0f354a2ebca562a393a257590bf42dea47f654989c081ec0ac2df4`). The
`--locked` child build uses it; nothing falls back to an unlocked build. The
detached workspace lock is committed the same way.

## Guest

The guest reads at most `authenticated_rdf::MAX_WITNESS_BYTES` raw bytes, a
bound derived from the model's V5 constants, before any typed deserialization.
It requires word-aligned input whose first word is `authenticated_rdf::VERSION`
(5), so V1–V3 inputs abort. It then deserializes one
`authenticated_rdf::Witness` and requires the SDK re-encoding to equal the input
exactly. Trailing words, nonzero string padding and narrowing aliases therefore
abort. It runs `authenticated_rdf::evaluate` and commits its `Journal`. Any
rejection aborts with `bounded authenticated-RDF relation rejected`, and no
journal is produced.

Credential parsing, RDFC-1.0 canonicalization, table checks, strict Ed25519
verification, the agreed-commitment comparison and V3 evaluation all run inside
the guest. The host supplies no precomputed dataset, journal or verdict.

Proving uses the exact APIs' prover-local limits (`1 << 25` session cycles,
`2^20` segments). The valid synthetic V5 witnesses of the direct execution
tests fit that ceiling. That does not show every valid witness fits; exceeding
it produces no presentation.

## Host API (`sparq_proved_evaluator::authenticated_rdf`)

| Item | Purpose |
|---|---|
| `prove_with_artifact(&Witness, r0vm, &AcceptedGuest)` | Proves with an explicitly supplied local `r0vm` and an independently accepted V5 guest; returns an already verified and request-bound `Presentation` without consuming a nonce |
| `verify_with_artifact(&Presentation, &Request, &mut impl Nonces, &AcceptedGuest)` | Verifies against the verifier's own `Request`, then consumes its nonce once |
| `embedded_authrdf_artifact()`, `embedded_authrdf_pin()` | The locally built V5 image and its pin, for release review; operators must approve the pin |

There is no embedded-default `prove` or `verify`. Load the guest with
`AcceptedGuest::from_artifact(bytes, &pin)` from a deployment-approved pin,
exported with:

```sh
cargo run --manifest-path zk/sparql-evaluator/Cargo.toml -p sparq-proved-evaluator \
  --features authenticated-rdf --example export_authrdf_guest -- /abs/NEW_DIRECTORY
```

As with `export_guest`, the directory must not exist; the example writes
`guest.bin` and `pin.json`. It refuses to export when the V5 digest or image ID
equals the exact guest's, since both exports share that file layout. Keep the
two exports in separately named directories.

`prove_with_artifact` first runs `validate_request` and `dataset_commitment`
natively, only so that invalid input fails before proving. Nothing relies on
that check. It then uses the shared Succinct-only prover with dev mode disabled
and verifies the receipt before returning.

`verify_with_artifact` performs these steps in order:

1. Validate the expected V5 request.
2. Require `InnerReceipt::Succinct` and verify it with dev mode disabled against
   the accepted image ID. The SDK's claim check requires `Halted(0)`.
3. Decode the V5 journal and run `bind_journal`. This checks the version, the
   request digest (query, V3 policy, key table, suite and mapping profile,
   authority and nonce) and the authority: the agreed commitment with
   `VerifierAgreedAuthenticated`, or `HolderSelectedAuthenticated`.
4. Only then call `Nonces::consume` once.

Every earlier failure leaves the nonce unconsumed. A store error is returned
unchanged. A crate-private checked-verification hook runs between steps 3 and 4
for the vcq adapter. No public API exposes it.

Obtain the expected `Request`, including its authorization table and any agreed
commitment, from verifier-owned configuration. Never take them from a
presentation. For `VerifierAgreed`, the verifier computes the commitment with
`authenticated_rdf::dataset_commitment` from its own copy of the credentials and
salt.

## Tests

All three files compile only with `--features authenticated-rdf`. The fixture is
the published W3C `vc-di-eddsa` `eddsa-rdfc-2022` vector: the same bytes, key
and signature as the model tests, in `host/tests/support/authenticated_rdf.rs`.
Nothing is signed, and no secret key is involved. Besides the published hashes,
the fixture checks that each hand-written expectation is read from the signed
statements. The SELECT cell is the only `schema:name` value, and the CONSTRUCT
triple is the only `alumniOf` statement. The table's issuer and method are the
signed ones, the `did:key` method decodes to the pinned Ed25519 key, and the
other issuer and method appear nowhere in the signed inputs.

- **Native host gates** (`host/tests/authenticated_rdf.rs`, run by default with
  the feature). The fixture and the native oracle must match the hand-written
  results under both authorities. The published witness encoding must start
  with the version word and stay below the guest input bound. The two embedded
  pins must be distinct, and each artifact must reject the other's pin or a
  wrong image ID. Invalid requests and credentials must be refused before any
  executor. A fake receipt and an invalid expected request must be rejected
  with zero nonce-store calls, including with a broken store.
- **Direct guest execution** (`host/tests/actual_authenticated_rdf.rs`, ignored,
  needs `RISC0_SERVER_PATH`). Witnesses go straight to the executor with the
  prover's session and segment limits, bypassing host prechecks. The published
  vector must reach `Halted(0)`, equal the native model and match the
  hand-written result for SELECT (bag), ASK and CONSTRUCT under both
  authorities. After a positive control, one SDK error-chain entry must equal
  `Guest panicked: bounded authenticated-RDF relation rejected` exactly for:
  - forged, short or spliced credentials and a changed `created`;
  - unsupported options and `proofValue`;
  - an issuer, method or key missing from the table;
  - wrong or differently salted anchors, zero salt, no credentials and a
    duplicate credential;
  - a V3 version word, trailing, truncated, padded, narrowed, unframed input,
    input one word above `MAX_WITNESS_BYTES`, and a complete V3 witness.

  The corruptions first assert the SDK layout they rely on: version word, query
  length and zero-padded packing, and the salt as the final 32 words. The exact
  guest must still execute V3 and must abort with its own exact message on V5
  input. Any other executor failure, including a session-limit error, fails the
  test.
- **Genuine receipts** (`genuine_authrdf_receipts_verify_declared_cases_and_reject_controls`
  in `host/tests/authenticated_rdf_genuine.rs`, ignored). Six cases are defined:
  SELECT (bag), ASK (true under agreed, false under holder) and CONSTRUCT, each
  under both authorities. A job proves one receipt for each case it declares,
  and only those; see [case selection](#case-selection). Each receipt must be
  Succinct with `Halted(0)`, consume one nonce and equal the native model.
  Controls reuse those receipts and create no proof. For every declared case,
  each must fail with its exact error and zero nonce calls:
  - a changed query of the same form, or a wrong nonce;
  - tightened V3 policy rows;
  - key-table substitutions in the verifier's own expectation (widened, or
    another issuer, method or key);
  - authority substitution, a wrong anchor, or an anchor under another salt;
  - an altered journal result or flipped journal byte;
  - the exact guest's image or a fake receipt.

  The V3 API on the same receipt and V5 image, with a valid V3 request, must
  also be rejected with zero nonce calls. Either `V3 journal decoding rejected`
  or `independent V3 request binding rejected` is correct, depending on whether
  the SDK decodes the V5 journal words as V3; the observed one is recorded. A
  replay on the used store must fail after two calls. A broken store's error
  must propagate after one call.

```sh
# Native host gates.
cargo test --locked --manifest-path zk/sparql-evaluator/Cargo.toml -p sparq-proved-evaluator \
  --features authenticated-rdf --test authenticated_rdf

# Native job and case-selection checks for the genuine driver; no proof.
cargo test --locked --manifest-path zk/sparql-evaluator/Cargo.toml -p sparq-proved-evaluator \
  --features authenticated-rdf --test authenticated_rdf_genuine

# Direct guest execution; no receipt.
RISC0_SERVER_PATH=/abs/r0vm cargo test --locked --manifest-path zk/sparql-evaluator/Cargo.toml \
  -p sparq-proved-evaluator --features authenticated-rdf --test actual_authenticated_rdf \
  -- --ignored --test-threads=1

# Genuine receipts for the job's declared cases; writes new evidence.
SPARQ_AUTHRDF_PROOF_JOB=/abs/authrdf-job.json RISC0_SERVER_PATH=/abs/r0vm \
  cargo test --locked --manifest-path zk/sparql-evaluator/Cargo.toml -p sparq-proved-evaluator \
  --features authenticated-rdf --test authenticated_rdf_genuine -- --ignored --exact \
  genuine_authrdf_receipts_verify_declared_cases_and_reject_controls --nocapture
```

The detached workspace lock and `methods/guest-authrdf/Cargo.lock` are both
committed, so these commands use `--locked`. The tests behind the first three
passed in the `42d13fed` scoped gate. This low-level genuine-receipt test has
not completed a job; see [recorded evidence](#recorded-evidence). `RISC0_DEV_MODE` must be unset. The tests read environment variables
but never set them.

Job schema `sparq.authrdf-genuine-proof.test-job.v2` (unknown and missing fields
reject; no v1 job was deployed):

| Field | Meaning |
|---|---|
| `schema` | Exactly `sparq.authrdf-genuine-proof.test-job.v2` |
| `guest`, `pin` | Approved V5 guest artifact and its `ArtifactPin` |
| `exact_guest`, `exact_pin` | Approved exact V1–V3 guest and pin, used only for cross-image controls; the pins must differ |
| `cases` | Required list of 1–6 distinct case IDs from the table below, proved in the listed order |
| `challenge_seed32` | 64 hex characters, nonzero; a fresh public synthetic seed per job. Each nonce is SHA-256 over `sparq:authrdf-genuine-test:nonce:v1\0` ‖ seed ‖ u64-be label length ‖ case label |
| `new_output_directory` | Absolute, absent, parent exists, outside the checkout; created owner-only |

`r0vm` comes from `RISC0_SERVER_PATH`, which must be absolute; it is
canonicalized and recorded. Inputs must be absolute regular files without `.`
or `..` and are read with fixed bounds.

### Case selection

| Case ID | Form | Authority | Expected result |
|---|---|---|---|
| `select-bag-verifier-agreed` | SELECT (bag) | verifier-agreed | two identical `schema:name` rows |
| `select-bag-holder-declared` | SELECT (bag) | holder-declared | two identical `schema:name` rows |
| `ask-true-verifier-agreed` | ASK | verifier-agreed | `true` |
| `ask-false-holder-declared` | ASK | holder-declared | `false` |
| `construct-verifier-agreed` | CONSTRUCT | verifier-agreed | one triple, from the only `alumniOf` statement |
| `construct-holder-declared` | CONSTRUCT | holder-declared | one triple, from the only `alumniOf` statement |

There is no default set. Right after the schema check, before loading any guest,
creating the output directory or proving, the job is rejected if `cases` is
empty, has more than six entries, repeats an ID or names an unknown ID (IDs are
case-sensitive). Complete coverage requires listing all six IDs.

A subset job reports only its declared cases. The case records and the
completed case IDs in `summary.json` must equal the declared list exactly, in
declared order, with one receipt per case. The expected results
(`fixture.expected` in `metadata.json`) are a map keyed by case ID, not an
ordered list; they must equal the completed records' expected results as a map.
Any mismatch fails the run before `summary.json` is written. A subset run establishes nothing about
undeclared cases, forms or authorities.

To bound a job, declare one or a few cases. If a job times out, its completed
cases keep their `record.json` files. Write a new job for only the cases without
a record, with a new output directory and a fresh seed; do not re-prove cases
already recorded. Each such job's evidence stands alone; no tool merges them.

Evidence (all files `create_new`, owner-only on Unix, never removed):

- `metadata.json` is written first. It is not a success record. It names the
  accepted guest (`sparq-authrdf-guest`, relation version 5) and the exact guest
  (`sparq-exact-guest`, cross-image controls only), each with its pin. It records
  `declared_cases`, `all_defined_cases_declared` and the expected results of the
  declared cases only, keyed by case ID.
- Each case directory holds `request.json` and `started.json`, written before
  proving, and `presentation.json`, written straight after proving. After every
  assertion it also holds `record.json`. The record names the guest package and
  has the exact typed request, the approved V5 pin, the artifact, request,
  receipt and journal SHA-256 digests, the complete receipt, status and
  controls. It is written only after the receipt is re-verified against that V5
  pin, never against the embedded exact-guest pin.
- `summary.json` is written last. It records `declared_cases`,
  `completed_cases`, `all_defined_cases_run` (true only when all six ran) and
  `genuine_receipts`, one per declared case.

This helper is `support/authenticated_rdf_evidence.rs`; the existing
`support/evidence.rs` is unchanged. Keep evidence outside the repository.

## Repository gates

The V5 guest workspace and its lock are registered in the dependency gates
(`scripts/rust-dependency-graphs.py`, a fourth graph), the registry-parser
source-lock check (`scripts/check_registry_parser.py`) and SBOM supplier
classification (`scripts/sbom-normalize.jq`). The exact-evaluator campaign
(`scripts/ci_exact_evaluator_evidence.py`) rebuilds the V5 guest's local
packages in its own target. It exports and later re-confirms the V5 artifact in
`authrdf-artifact`, separate from the exact `artifact`. It records both pins
under `guest_artifact_pins`, keyed by guest package, and requires them to be
distinct. It also runs the V5 native model and host gates and a V5 lint; those
steps execute no V5 guest. The later zkp-14.6 campaign commands add direct V5
guest execution without proving (see [campaign evidence](evaluator-evidence.md)).
The campaign creates no V5 receipt. Every one of these gates fails closed without
`methods/guest-authrdf/Cargo.lock`, which is committed. No completed campaign
run with these V5 steps is recorded; the scoped gate below is not a campaign
record.

## Recorded evidence

Unless stated otherwise, these records are for source `42d13fed`
(`42d13fedff0f42e3cde8ea63ff611408719cd6e7`). The independent audit records are
kept outside this repository and identified by SHA-256. Host timings are
non-canonical and are not reported.

| Audit record | SHA-256 |
|---|---|
| Scoped gate, `guest-gates-42d13fed/independently-verified-gates.json` | `9d492f2392aad03b03d63de75914a1c73ac3557e70df0f302bec434d907b8729` |
| Same-path exact-guest control, `8322a6ff` to `6bf6314a` | `aa50f937076c9abfbaafced6c7103840d5a466b3f33839fd8661d2f12a8f1655` |
| Genuine job timeout, `authrdf-genuine-42d-select-agreed/independently-verified-timeout.json` | `e86acfef78a3e565294e3499532e0ad171e7c3c86c323a739956ca794d41f2ed` |

The scoped gate ran on an EC2 host:

- **Native.** Seven test functions passed: the five in
  `host/tests/authenticated_rdf.rs` and the two job-parsing tests in
  `host/tests/authenticated_rdf_genuine.rs`.
- **Direct execution.** The four ignored functions in
  `host/tests/actual_authenticated_rdf.rs` passed over 45 executions: 10 V5
  executions reached `Halted(0)` and 32 aborted with the expected V5 message;
  the exact guest completed one V3 execution and aborted twice with its own
  message. Direct execution creates no receipt.
- **Lint.** Clippy over all targets with the `authenticated-rdf` feature passed.
- **Source.** 7,081 Git source blobs and 30 lock files were unchanged by the
  gate. No receipt was created.

Guest identities observed at `42d13fed` (not a reproducibility claim; operators
still approve their own pin):

| Guest | Artifact SHA-256 | Image ID (`u32` words) |
|---|---|---|
| `sparq-authrdf-guest` (V5) | `c35f5e4b74169aa51b244b8feecb0c8e746296a6aa052be8c58f0190b1b10b11` | `[2794517044, 4278390001, 1088095536, 53994879, 3259643235, 835127579, 1589380955, 3166477348]` |
| `sparq-exact-guest` (V1–V3) | `e8c9b6b6bd43c2789add2fcbd9ec18914c2c48ccdcd1149e8b9dea6f0e760623` | `[3002199066, 1615302721, 2039644206, 2826525735, 178258819, 3880799180, 3190314515, 791784891]` |

One genuine job, declaring only `select-bag-verifier-agreed`, ran at
`42d13fed` with a four-CPU allowance and timed out. It completed no proof and no control
and wrote no receipt, presentation or `summary.json`. That is incomplete
execution, not a semantic rejection, a security finding or a benchmark, and it
establishes nothing about any case. The later adapter-driven receipt for the
same case ID, at `7fe88955`, is a separate job through a different driver; it
does not complete this job or change its record.

Later guest-branch commits up to `ff3b94bf` changed only comments and
documentation.

A separate native model validation at `8322a6ff` is described in the
[native model reference](authenticated-rdf-model.md#tests), and a separate
native adapter gate and one-case genuine adapter run at `7fe88955` in the
[adapter reference](vcq-authenticated-rdf-adapter.md#recorded-native-validation);
none is part of these counts.

## Not established

Everything under "Not established" in the
[native model reference](authenticated-rdf-model.md) still applies:

- no credential status, holder binding, validity period, clock or DID resolution;
- no wallet or world completeness under either authority;
- no arbitrary JSON-LD or JSON mapping;
- no general SPARQL conformance.

The verifier's key table is the only trust input. No performance or cycle
figure is claimed.
