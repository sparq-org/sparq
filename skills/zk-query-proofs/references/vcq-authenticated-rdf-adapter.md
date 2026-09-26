<!-- [OPUS-5.5] zkp-14.6: usage reference for the optional vcq adapter over the V5 authenticated-RDF relation. -->
# vcq adapter for issuer-authenticated RDF (V5)

**One genuine case; five tuples and the row-bound case not run.** At frozen adapter source
`7fe88955`, one independently audited genuine job proved one public synthetic case,
`select-bag-verifier-agreed`, with the approved V5 guest `c35f5e4b`, and the protocol accepted it;
see [recorded genuine validation](#recorded-genuine-validation). A separate independently verified
native gate at the same source passed the adapter's native tests and Clippy without proving; see
[recorded native validation](#recorded-native-validation). The other five declared result and
authority tuples and the genuine row-bound rejection case have not run, so the registry keeps
`adapter_available: false`. It is experimental and not externally audited (sq-qhy4). Do not
treat it as a validated, available method. <!-- privacy-claims-allow: one genuine synthetic case plus native tests; not audited -->

`sparq_proved_evaluator::vcq_authenticated` (detached crate `zk/sparql-evaluator/host`, cargo
feature `vcq-authenticated`, **off by default**) implements the `sparq-query-protocol`
`QueryMethod` trait over the V5 relation. See the [native model reference](authenticated-rdf-model.md)
and the [V5 guest reference](authenticated-rdf-guest.md) for the relation and guest. The feature is
exactly `["vcq", "authenticated-rdf"]`; it adds no dependency or lock change. The V3 adapter
([vcq exact adapter](vcq-exact-adapter.md)), the low-level V5 host API, the model and both guests
are unchanged. The only other changes are crate-private: shared V3 adapter helpers and the V5
`verify_checked` hook are now `pub(crate)`, with no behavior change.

```sh
# Native adapter gates, unit tests and the V3 adapter regression; no proof.
cargo test --locked --manifest-path zk/sparql-evaluator/Cargo.toml -p sparq-proved-evaluator \
  --features vcq-authenticated --lib --test vcq_authenticated --test vcq_authenticated_genuine \
  --test vcq_adapter
```

## CI campaign scope (authored; full campaign not yet run)

The exact-evaluator campaign (`scripts/ci_exact_evaluator_evidence.py`) names four commands for
this feature and its dependencies; see [campaign evidence](evaluator-evidence.md) for the table.
They were authored at `e4fb7dff`, where the campaign's 17 hermetic Python tests and the scoped
documentation gates passed. The full named campaign has not run at that revision. The manual
native gate below is not that campaign.

- `native-vcq`: the V3 adapter tests under `vcq` alone, covering the shared helpers.
- `native-vcq-authenticated`: the command above; it runs the genuine driver file's native
  job-parser tests but never its ignored prove test.
- `lint-vcq-authenticated`: all-target workspace Clippy with `-D warnings`.
- `actual-authrdf-direct-execution`: the ignored direct V5 executor tests in
  `actual_authenticated_rdf.rs` only. That is SDK execution, not proving: it creates no receipt,
  and the campaign records zero V5 receipts.

A passing campaign would be native, lint and direct-execution evidence only. It would not be an
adapter receipt and would not change the registry's `adapter_available: false`.

## Availability: implementation is not validation

`Capabilities::is_executable()` is true in this build because the adapter code exists; without
that local declaration, `admit` could not select it. That is an implementation declaration only.
The research [method registry](../../../research/vc-query-methods.json) lists
`method:risc0-authenticated-rdf` version 5 with `adapter_available: false`. One genuine adapter
receipt is now independently audited, for `select-bag-verifier-agreed` only. That is partial
coverage of one of six declared tuples, so the value stays false for the six-tuple declaration.
Six source capabilities are not six proven profiles. The adapter never reads the registry.

## API

| Item | Purpose |
|---|---|
| `Risc0AuthenticatedRdfV5::new(&ArtifactPin, AcceptedGuest, Policy)` | Method from an approved V5 pin, the guest accepted under it, and the verifier's policy; rejects a guest that does not match the pin, an invalid policy and an all-zero pin |
| `.policy()` | The immutable verifier policy, table sorted by verification method |
| `.with_r0vm(PathBuf)`, `.with_verifier(Identifier, fn() -> u64)` | Local `r0vm` for `prove`; verifier audience and clock for the trait `verify` |
| `verify_at(request, audience, now_unix, admission, presentation, store)` | Verification with an explicitly supplied audience and Unix time |
| `descriptor(&ArtifactPin, &Policy)` | The exact `MethodDescriptor` a verifier lists in its request |
| `policy_digest`, `parameter_digest`, `derive_nonce`, `statement_digest` | Documented digests below |
| `expected_v5_request(request, descriptor, &Policy, phase)` | The V5 request both sides derive; refuses a policy that does not produce the descriptor |
| `vcq::VcqPresentation` | Reused unchanged: descriptor digest plus serialized receipt bytes |
| `AuthenticatedOutput` | Claim output: `ReleasedResult`, journaled V5 `Provenance`, authenticated commitment, V5 request digest |

`QueryMethod` associated types: `Request = StoredRequest`,
`PrivateInputs = authenticated_rdf::PrivateCredentials`, `Witness = AuthenticatedWitness`,
`Presentation = VcqPresentation`, `ChallengeStore = dyn ChallengeStore`. `PrivateCredentials`
redacts its `Debug` output, but the model type is `Clone`, and this task did not change the
model. The adapter's `AuthenticatedWitness` is opaque: it is not `Clone` or serializable, and
its `Debug` output is redacted.

## What the relation authenticates

Per credential, inside the pinned V5 guest and within fixed bounds, the relation does this:

- It parses the document and proof configuration as N-Quads (default graph only) and
  canonicalizes each with bounded RDFC-1.0/SHA-256.
- It reads every check from those canonical bytes: one `DataIntegrityProof` node, typed
  `cryptosuite` `eddsa-rdfc-2022`, one IRI `verificationMethod`, `proofPurpose assertionMethod`,
  and at most one `created` in a restricted whole-second UTC profile, never compared with a clock.
- The method must be in the verifier's table, and the document's single issuer must equal that
  entry's pinned issuer.
- It verifies strict Ed25519 over `SHA-256(canonical config) || SHA-256(canonical document)`.

It then unions the hashed canonical documents into one default graph with per-credential
blank-node scopes and evaluates the V3 query over that union.

This is a bounded canonical RDF profile, **not** a Data Integrity processor. It does no
JSON-LD expansion or context handling, no `proofValue` multibase decoding and no DID or
controller resolution. It checks no validity period or `created` time, no credential status and
no holder binding. The verifier's table is the only trust input. The table has no controller
field: an entry's `issuer` is the only issuer-to-key authorization.

## Descriptor and capabilities

- Descriptor: `urn:sparq:vcq:method:risc0-authenticated-rdf` version 5, parameter set
  `urn:sparq:vcq:params:risc0-authenticated-rdf:v5-verifier-policy` with `parameter_digest(policy)`,
  `ZkvmGuest { artifact_digest: pin.sha256, image_id }` (each image-id word little-endian), backend
  `urn:sparq:vcq:backend:risc0-zkvm:3.0.6:succinct`. The backend string equals the V3 adapter's,
  because both guests use the same SDK and receipt kind. Method, version, parameter set, parameter
  digest and artifact all differ from the V3 descriptor.
- Six whole tuples: `SelectBag`, `AskBoolean` and `GraphRdfc10` (CONSTRUCT only), each under
  `VerifierAgreedAnchor` and `HolderDeclared`. All are `ExactBounded`, with profile
  `urn:sparq:vcq:dialect:sparq-sparql11-graph-results:v3` / `urn:sparq:vcq:fragment:v3-static-admission`
  (the V3 language, unchanged). Assembly is `UnionDefaultGraph` and source evidence is
  `IssuerAuthenticated`, with suite `urn:sparq:vcq:suite:di-eddsa-rdfc-2022:v5-bounded-canonical-rdf`.
  Status is `NotRequested` and the holder `BearerAccepted`. Mapping is
  `urn:sparq:vcq:map:v5-scoped-canonical-union` and linking is
  `urn:sparq:vcq:link:authenticated-dataset-evaluation`.
- Enforcers: authenticity, mapping, linking and query are all declared as the relation
  `urn:sparq:vcq:relation:risc0-authenticated-rdf-v5-guest`. The anchor is the public host check
  `urn:sparq:vcq:host:authenticated-anchor-equality`, owed only under anchor authority; the guest
  also rejects an unequal agreed commitment. Status and holder binding are `Absent`, so a claim
  reports them `NotEstablished`.
- Challenge: owner `Method`, `ConsumeOnSuccess`. Ceilings: 4,096 released rows, 16 MiB
  presentation bytes. Disclosure: `urn:sparq:vcq:disclosure:risc0-authenticated-rdf-v5-journal`
  (V5 request digest, authenticated commitment, provenance, result).

These requests are rejected at admission (`TupleUnsupported`, `QueryProfileUnsupported`,
`DescriptorUnavailable` or `capacity`), before any proof or challenge use:

- source evidence `None` or `ReAttested`, or an accepted-suite set without this suite;
- another mapping or linking profile;
- required status or holder binding;
- `CredentialNamedGraphs` or `ExactSourceCatalog` assembly;
- SELECT sequence or set contracts;
- another dialect;
- a V3 or other-policy descriptor.

The typed query-shape check then rejects a form mismatch, an ordered SELECT, DESCRIBE, an
explicit base IRI, a parse failure, an unadmitted query and an oversized query.

## Policy binding

- `policy_digest`: SHA-256 over `sparq:vcq:risc0-authenticated-rdf:policy:v5\0` ‖ the V5 model's
  own `authenticated_rdf::request_digest` of a fixed sentinel request. The sentinel has version 5,
  query `ASK {}`, `HolderDeclared` authority, nonce `sparq/vcq/authrdf/policy-binding` (32 ASCII
  bytes) and the policy. The sentinel is never evaluated.
- Why not a second encoder: the model's policy encoder is private, and the model is out of scope.
  With every other sentinel field fixed, the model request digest depends only on its framed
  policy digest. That covers the suite and mapping profile tags, every fixed model bound, the
  serialized V3 evaluation policy, and each entry's issuer, verification method and 32-byte key,
  sorted by method. So every authorized-key, method, issuer and evaluation-policy field changes
  the digest, while table order does not.
- The model policy has no `created` or controller field. `created` is a signed proof option
  inside each credential. The signature and the commitment's proof-configuration hash cover it.
- The suite, mapping and DESCRIBE policy each have one variant. An exhaustive match maps the
  suite and mapping to this adapter's identifiers, so a new model variant fails to compile until
  it has its own identifiers.
- Invalid policies reject as `invalid` `vcq-authrdf-policy-invalid` at `request`:
  - an empty table, or more than 16 keys;
  - a relative or oversized IRI;
  - an invalid or small-order key;
  - a repeated verification method;
  - V3 evaluation capacities outside the program ceilings.
- `parameter_digest`: SHA-256 over these fields, in order:
  1. `sparq:vcq:risc0-authenticated-rdf:parameters:v5\0` and big-endian `u32` 5;
  2. `policy_digest`;
  3. the suite, mapping, linking and relation identifiers, each a big-endian `u64` length and
     its bytes;
  4. as big-endian `u64`: query bytes (8,192), then `MAX_CREDENTIALS` 4, `MAX_AUTHORIZED_KEYS`
     16, `MAX_IRI_BYTES` 512, `MAX_DOCUMENT_BYTES` 8,192, `MAX_PROOF_CONFIG_BYTES` 2,048,
     `MAX_TOTAL_BYTES` 32,768, `MAX_DOCUMENT_QUADS` 128, `MAX_PROOF_CONFIG_QUADS` 8,
     `MAX_TOTAL_QUADS` 256 and `MAX_WITNESS_BYTES` 64,512;
  5. as big-endian `u32`: the released-row (4,096) and presentation-byte (16 MiB) ceilings.

`Risc0AuthenticatedRdfV5::new` stores the policy with its table sorted by verification method,
and the policy cannot be changed afterwards. Build one adapter per verifier policy. A different
policy is a different descriptor, and admission refuses a request that names it.

## Request binding and verification order

- `derive_nonce`: SHA-256 over `sparq:vcq:risc0-authenticated-rdf:v5-nonce:local-struct-v1\0` ‖
  SHA-256(stored-request bytes) ‖ SHA-256(descriptor bytes), under `local-struct-v1`. It differs
  from the V3 adapter's nonce for the same stored request.
- `expected_v5_request` carries the exact query, the authority and anchor, the verifier's own
  policy and that nonce. The anchor is an authenticated V5 commitment from
  `authenticated_rdf::dataset_commitment`, not a V3 anchor. The guest journals the model request
  digest. That binds the original challenge, audience, window, accepted suites, row and byte
  bounds, method list, descriptor (and through it the policy), authority, anchor and query. The
  verifier never infers a policy from a presentation.
- `statement_digest`: SHA-256 over `sparq:vcq:risc0-authenticated-rdf:statement:v5\0` ‖
  stored-request digest ‖ descriptor digest ‖ SHA-256(exact verified journal bytes).

`prepare` and `verify` both recompute admission and run the typed shape check. `prepare` then
authenticates natively; that check only fails early, because the guest repeats every check.
Every model rejection is one `invalid` code, `vcq-authrdf-credentials-rejected`. The model's
diagnostic text is never classified, so this code does not distinguish capacity, forgery or
authorization. `prepare` then requires the credentials to open an agreed anchor (otherwise
`unsatisfiable` `vcq-anchor-not-opened`).

`verify` checks these in order:

1. audience, `not_before <= now < not_after` and the challenge policy;
2. the descriptor digest;
3. the presentation-byte bound, before decoding;
4. receipt decoding;
5. the V5 request rebuilt from the stored request and the adapter's own policy;
6. the Succinct receipt, with dev mode off, against the accepted V5 image;
7. journal decoding and `bind_journal`;
8. anchor or provenance: `VerifierAgreedAuthenticated` plus an equal commitment, or
   `HolderSelectedAuthenticated`;
9. the result contract and released-row bound;
10. the claim.

Only then does the reused crate-private bridge consume the ORIGINAL challenge once through the
shared store. Replay and store failure are typed. Nothing before that point calls the store.

## Claim

A claim has scope `{authority, anchor (agreed only), IssuerAuthenticated, RelativeToScope}` and
tuple suite `Some(...)`. Authenticity, mapping, linking and query are `Established(Relation)`.
The anchor is `Established(HostPublic)` only under agreed authority. Status and holder binding
are `NotEstablished`. `AuthenticatedOutput.provenance` is the journaled V5 value.

Relative completeness means only this: complete over the agreed commitment, or over the holder's
chosen authenticated credentials. It never means wallet or world completeness. Salt reuse makes
commitments publicly linkable.

## Tests

The unit tests, native gates and the genuine driver's non-ignored job-parser and policy tests
passed in the [recorded native gate](#recorded-native-validation). The ignored genuine driver was
not run there. It ran separately for one declared case; see
[recorded genuine validation](#recorded-genuine-validation).

- **Unit tests** (`host/src/vcq_authenticated.rs`). They check claim conversion on hand-built
  journals: agreed and holder scope and obligations, and output fields. They also check that
  provenance upgrade, wrong anchor, wrong version, wrong result kind and row excess reject. The
  expected request must refuse a mismatched or invalid policy. No proof.
- **Native gates** (`host/tests/vcq_authenticated.rs`, published W3C vector, no proof):
  - the descriptor and six tuples, and separation from V3;
  - the hand-computed `policy_digest` and `parameter_digest` composition;
  - that each of 17 policy mutations changes the policy, parameter and descriptor digests, the
    nonce and the V5 request digest, while table order does not;
  - canonical stored policy order;
  - nine invalid policies;
  - nonce, statement and expected-request composition;
  - representative stored-request fields;
  - admission rejections and typed shape rejections, at prepare and verify;
  - admission mismatch;
  - native authentication and anchor opening, with redacted `Debug`;
  - verify gates before decoding;
  - fake receipts claiming the V5 or exact image, and a V3-descriptor presentation;
  - a guest/pin mismatch;
  - native oracle results.

  Every rejection case asserts zero challenge-store calls. A native journal is never a proof.
- **Genuine receipts** (`genuine_vcq_authrdf_receipts_verify_declared_cases_and_reject_controls`
  in `host/tests/vcq_authenticated_genuine.rs`, ignored). This reuses the explicit-case job
  design of `authenticated_rdf_genuine.rs`.

```sh
SPARQ_VCQ_AUTHRDF_PROOF_JOB=/abs/job.json RISC0_SERVER_PATH=/abs/r0vm \
  cargo test --locked --manifest-path zk/sparql-evaluator/Cargo.toml -p sparq-proved-evaluator \
  --features vcq-authenticated --test vcq_authenticated_genuine -- --ignored --exact \
  genuine_vcq_authrdf_receipts_verify_declared_cases_and_reject_controls --nocapture
```

Job schema `sparq.vcq-authrdf-genuine-proof.test-job.v1` (unknown and missing fields reject):
`schema`, V5 `guest` and `pin`, and exact `exact_guest` and `exact_pin` (cross-image control
only; the pins must differ). `cases` lists 1–7 distinct known IDs, in run order, with no
default. `challenge_seed32` is 64 hex characters, nonzero and public. Each original challenge
is SHA-256 over `sparq:vcq-authrdf-genuine-test:original-challenge:v1\0` ‖ seed ‖ u64-be label
length ‖ case label. `new_output_directory` must be absolute, absent and outside the checkout.
`RISC0_SERVER_PATH` must be absolute, and `RISC0_DEV_MODE` must be unset. Adapters and the
checker are built only from the job's approved pins, never from embedded guests.

| Case ID | Contract | Authority | Expected |
|---|---|---|---|
| `select-bag-verifier-agreed`, `select-bag-holder-declared` | SELECT bag | agreed / holder | two identical `"Alumni Credential"` rows |
| `ask-true-verifier-agreed` | ASK | agreed | `true` |
| `ask-false-holder-declared` | ASK | holder | `false` |
| `construct-verifier-agreed`, `construct-holder-declared` | CONSTRUCT | agreed / holder | the one `<http://ex/alumniOf>` triple |
| `select-bag-row-bound` | SELECT bag, `released_rows = 1` | agreed | genuine receipt, protocol rejection |

Expectations are hand-defined from the published document and compared with the native model.

Each accepted case must do the following:

- consume the original challenge exactly once;
- match its expected result, provenance, commitment, V5 request digest, descriptor, statement
  digest, scope, suite and obligations;
- pass SDK-checked V5 verification with test nonces, as `Succinct` with `Halted(0)`;
- equal the native journal.

Controls reuse the receipt and create no proof. Each must fail with its exact code and zero
store calls:

- changed query, challenge, stored audience or window;
- wrong audience;
- expired or not-yet-valid instants;
- wrong descriptor digest;
- altered journal result, flipped journal byte or a fake receipt;
- scope substitution;
- wrong anchor or an anchor under another salt (agreed cases);
- five verifier-policy substitutions (other key, issuer or method; widened table; tightened rows),
  each as presented (`vcq-descriptor-digest-mismatch`) and with the descriptor digest spliced
  (`vcq-proof-rejected`);
- the exact guest's pin with the descriptor spliced (`vcq-proof-rejected`).

Then a replay on the used store must be `ChallengeReplayed` after two calls, a broken store
`infrastructure` after one call, and two concurrent verifications must give one accept and one
replay. The row-bound case must reject as `capacity` `vcq-released-rows` (requested 2, ceiling 1)
with zero store calls, and its receipt must still verify through the low-level V5 API with test
nonces.

A subset job proves and reports only its declared cases. `summary.json` is written last. It
records `declared_cases`, `completed_cases`, `all_defined_cases_run`, the receipt and control
counts and `registry_adapter_availability: "not tested; this test reads no registry entry"`.
Evidence reuses `support/authenticated_rdf_evidence.rs`. Before any verification it holds
`metadata.json`, and per case `request.json`, `started.json` and `presentation.json`, plus
`vcq-presentation.json`, the `local-struct-v1` stored-request and descriptor bytes and
`expected-result.json`. It adds `record.json` after the receipt is re-verified against the
approved V5 pin. Keep evidence outside the repository.

## Recorded native validation

Adapter source `7fe88955` (`7fe889557afcf1497769c1360ef53348980a7bae`), one manually scoped
native gate on an EC2 host, independently verified. The audit record is kept outside this
repository. Host timings are non-canonical and are not reported.

| Audit record | SHA-256 |
|---|---|
| `vcq-authenticated-native-7fe88955/independently-verified-gates.json` | `6a8fc2300f0463290859b0f8f4c0b9ef790382b0ef9401ec2a60a07645eab385` |

- **Native.** 44 distinct test functions passed: 4 new `vcq_authenticated` module unit tests,
  15 new `host/tests/vcq_authenticated.rs` tests, 3 new non-ignored job-parser and valid-policy
  tests in `vcq_authenticated_genuine.rs`, 5 existing low-level V5 tests and 17 existing V3
  `vcq` adapter tests. The 17 V3 functions ran again under `vcq` alone, so there were 61
  executions of 44 distinct functions. The one ignored genuine driver was not run.
- **Lint.** All-target Clippy passed in two feature scopes.
- **No proof or direct guest execution.** The gate created no proof and ran no direct guest execution.
- **Source.** 7,085 Git blobs and 30 lock files were unchanged by the gate.
- **Guest identities.** The V5 artifact SHA-256 was
  `c35f5e4b74169aa51b244b8feecb0c8e746296a6aa052be8c58f0190b1b10b11`, and the exact guest's was
  `e8c9b6b6bd43c2789add2fcbd9ec18914c2c48ccdcd1149e8b9dea6f0e760623` in both the feature-off and
  feature-on builds. Both match the [V5 guest gate](authenticated-rdf-guest.md#recorded-evidence), built at
  the same fixed path; this is not an arbitrary-path reproducibility claim.

This is native adapter evidence only. It is not the CI campaign, not direct guest execution,
not a genuine receipt and not a full workspace gate, and it leaves the registry's
`adapter_available` false. A separate independent source review found no actionable production
or test defect; that review is not a security audit.

## Recorded genuine validation

Frozen adapter source `7fe88955` (`7fe889557afcf1497769c1360ef53348980a7bae`). One genuine job
declared only `select-bag-verifier-agreed` over the public synthetic W3C `eddsa-rdfc-2022`
vector, under an eight-CPU allowance. The independent audit found the source identical before
and after the run and equal to a local archive of that commit (7,085 source files, 30 lock
files). The run was not repeated at later commits. The audit record and evidence archive are
kept outside this repository. Host timings and memory are non-canonical and are not reported.

| Record | SHA-256 |
|---|---|
| Independent audit (`sparq.independent-genuine-evidence-audit.v1`) | `7eddc0999228f9db3a2ddcd8b9415d36272dc63e0bb1c7727675e071701eea49` |
| Evidence archive, `vcq-authrdf-genuine-7fe-select-agreed/terminal-evidence.tar.gz` (26 files verified) | `4c123dc5c5b4dc94483d77af905940918aa6ad7b527b0943018c4d0a81508b8c` |
| Source audit | `4796827c3602ef6e4cfaa3b8aa06ffdb1b0850d68914326619186a56ca6ecbe1` |
| Proof job; runner | `8e3dab7f927c43508378418091811c2562380f21047bf69ad23850940e338cfd`; `89a899a1798c51f9f75eaa99d88b78a66f7bb861442370724e49e2ad9b99787a` |
| `vcq_authenticated_genuine` test executable | `693c8e36fb69788b01f79a67aef0e187f213423bacf1dbe33c37cdb71d10be07` |
| `r0vm` 3.0.6 | `751b9b188d341e8bec5e02060086b7b1dc3f7289e90f726c38589bb6735dd6d7` |
| `summary.json`; case `record.json`; run log | `f2bf9ac622453adbed64cfaca47e9f501caffc53086af7726346e0243fceeea0`; `a237a5a09d5210aad7ac76b56cb76dd54e3cdac3aea600058d4ed72cbe79993a`; `45db3b8af8194ec608b66b3e3f94b01e46d8d42e6bbd3c5fc735562e5bd7c397` |

- **Receipt.** One genuine receipt, protocol-accepted: `Succinct`, `Halted(0)`, dev mode off,
  seal 55,667 words. Receipt SHA-256
  `1e24b3af5dfd186054040f694135c235def3f28183b8a459df51c8208be50810`, journal SHA-256
  `d68246c1c14c18927bf09e00db62e1c45921ba07463f165de255acd7d39a94b1`. Presentation file SHA-256
  `7736b137b085b33aa7eacc0211ac595ff5690b6024ad8fa300fff52a168e01b4`, vcq transport SHA-256
  `0ff8dc6073eb40add2bd2b8ee40ff18125ffed6bd0fdc2f1ac14b3779145f19b`.
- **Pin.** The approved V5 guest, artifact SHA-256
  `c35f5e4b74169aa51b244b8feecb0c8e746296a6aa052be8c58f0190b1b10b11`, with the image ID recorded
  by the [V5 guest gate](authenticated-rdf-guest.md#recorded-evidence) at `42d13fed`. The
  cross-image control used the exact guest's pin (`e8c9b6b6…`, same reference).
- **Result.** The expected bag SELECT result, two identical `"Alumni Credential"` rows for
  `?name`, matched the native journal. The test completed SDK verification with dev mode off
  under the V5 pin, the native and hand-defined result comparison and the generic protocol
  checks. The audit checked the retained bytes, source and execution provenance and the test's
  assertions; it launched no extra verification process.
- **Controls.** 28, all reusing the receipt. 25 binding controls failed as `invalid` with their
  exact codes and zero store calls: 16 `vcq-proof-rejected`, 6 `vcq-descriptor-digest-mismatch`
  and one each of `vcq-audience-mismatch`, `vcq-request-expired` and `vcq-request-not-yet-valid`.
  A replay gave `ChallengeReplayed` after two store calls, a broken store `infrastructure` after
  one, and two concurrent verifications one accept and one replay. The stores are in-memory test
  doubles, not production durability or concurrency evidence.
- **Scope.** `all_defined_cases_run` is false. The driver emitted no guest cycle metric.

This is one tested fixture for one of six tuples. The other five tuples
(`select-bag-holder-declared`, `ask-true-verifier-agreed`, `ask-false-holder-declared`,
`construct-verifier-agreed`, `construct-holder-declared`) and the genuine `select-bag-row-bound`
rejection case have not run, and the registry keeps `adapter_available` false. It is not a broad
semantic or security result, not a benchmark and not an external audit. The earlier low-level V5
job at `42d13fed` that [timed out](authenticated-rdf-guest.md#recorded-evidence) is separate
evidence, as are the native gate above and the V5 guest gate's direct executions; no count here
combines with theirs.

## Not established

- no credential status, holder binding, validity period, clock or DID or controller resolution;
- no JSON-LD, full Data Integrity or other-suite processing;
- no wallet or world completeness under either authority;
- no genuine receipt for five of the six tuples or for the row-bound case, and no result beyond
  one public synthetic fixture;
- no general SPARQL conformance, benchmark or performance figure;
- no external audit, and no soundness or privacy claim.
