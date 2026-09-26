<!-- [OPUS-5.5] zkp-14.5: usage reference for the separate V5 guest and low-level host API. -->
# Issuer-authenticated RDF: separate V5 guest and host API

**Source only; unbuilt and unexecuted at the current checkpoint.** Guest, host
and test source exist, but the V5 guest has not been built or executed, and no
V5 test run or genuine receipt has been recorded. The V5 relation is listed in
no method registry, and no protocol adapter exists. It is experimental, research
grade, not yet sound and not externally audited.
<!-- privacy-claims-allow: source only; explicitly unbuilt, unexecuted, unaudited, no receipt recorded -->

The relation is the unchanged native V5 model described in the
[native model reference](authenticated-rdf-model.md). This slice adds a guest
image that runs it and a low-level host API. It stops before any generic
`QueryMethod` adapter: `vcq.rs`, `sparq-query-protocol` and the research method
registry are unchanged.

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
the absolute `OUT_DIR` artifact path, so their text varies between builds. The
intended invariant is that the default exact guest's artifact bytes and image ID
are unchanged; it is pending a byte comparison against the previous build and is
not yet a measured fact. Distinct digests and image IDs, and each image
rejecting the other's input, are likewise asserted by the unrun tests below, not
yet observed.

`methods/build.rs` reruns when any image source input changes. For both images
that is the root manifest, `sparq-canon`, `sparq-core`, `sparq-engine`,
`sparq-substrate`, `vendor/spargebra`, the four patched SDK crates, the model
and the exact guest workspace. With the feature it also watches
`methods/guest-authrdf`. `package.metadata.risc0.methods` lists both guest
directories; only this `build.rs` builds them.

`methods/guest-authrdf/Cargo.lock` is not committed with this source. Until it
exists, a `--locked` build with the feature fails; nothing falls back to an
unlocked build.

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
`2^20` segments). Whether valid V5 witnesses fit that ceiling is unknown until
the guest is executed; exceeding it produces no presentation.

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
for a later protocol adapter. No public API exposes it.

Obtain the expected `Request`, including its authorization table and any agreed
commitment, from verifier-owned configuration. Never take them from a
presentation. For `VerifierAgreed`, the verifier computes the commitment with
`authenticated_rdf::dataset_commitment` from its own copy of the credentials and
salt.

## Tests (written, not yet run)

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
- **Genuine receipts** (`genuine_authrdf_receipts_verify_both_authorities_and_reject_controls`
  in `host/tests/authenticated_rdf_genuine.rs`, ignored). This proves six
  receipts: SELECT (bag), ASK (true under agreed, false under holder) and
  CONSTRUCT, each under both authorities. Each must be Succinct with
  `Halted(0)`, consume one nonce and equal the native model. Controls reuse
  those receipts and create no proof. Each must fail with its exact error and
  zero nonce calls:
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
cargo test --manifest-path zk/sparql-evaluator/Cargo.toml -p sparq-proved-evaluator \
  --features authenticated-rdf --test authenticated_rdf

# Direct guest execution; no receipt.
RISC0_SERVER_PATH=/abs/r0vm cargo test --manifest-path zk/sparql-evaluator/Cargo.toml \
  -p sparq-proved-evaluator --features authenticated-rdf --test actual_authenticated_rdf \
  -- --ignored --test-threads=1

# Genuine receipts; writes new evidence.
SPARQ_AUTHRDF_PROOF_JOB=/abs/authrdf-job.json RISC0_SERVER_PATH=/abs/r0vm \
  cargo test --manifest-path zk/sparql-evaluator/Cargo.toml -p sparq-proved-evaluator \
  --features authenticated-rdf --test authenticated_rdf_genuine -- --ignored --exact \
  genuine_authrdf_receipts_verify_both_authorities_and_reject_controls --nocapture
```

Add `--locked` once the coordinator has generated
`methods/guest-authrdf/Cargo.lock` and resolved the detached workspace lock.
`RISC0_DEV_MODE` must be unset. The tests read environment variables but never
set them.

Job schema `sparq.authrdf-genuine-proof.test-job.v1` (unknown fields reject):

| Field | Meaning |
|---|---|
| `schema` | Exactly `sparq.authrdf-genuine-proof.test-job.v1` |
| `guest`, `pin` | Approved V5 guest artifact and its `ArtifactPin` |
| `exact_guest`, `exact_pin` | Approved exact V1–V3 guest and pin, used only for cross-image controls; the pins must differ |
| `challenge_seed32` | 64 hex characters, nonzero; a public synthetic seed. Each nonce is SHA-256 over `sparq:authrdf-genuine-test:nonce:v1\0` ‖ seed ‖ u64-be label length ‖ case label |
| `new_output_directory` | Absolute, absent, parent exists, outside the checkout; created owner-only |

`r0vm` comes from `RISC0_SERVER_PATH`, which must be absolute; it is
canonicalized and recorded. Inputs must be absolute regular files without `.`
or `..` and are read with fixed bounds.

Evidence (all files `create_new`, owner-only on Unix, never removed):

- `metadata.json` is written first. It is not a success record. It names the
  accepted guest (`sparq-authrdf-guest`, relation version 5) and the exact guest
  (`sparq-exact-guest`, cross-image controls only), each with its pin.
- Each case directory holds `request.json` and `started.json`, written before
  proving, and `presentation.json`, written straight after proving. After every
  assertion it also holds `record.json`. The record names the guest package and
  has the exact typed request, the approved V5 pin, the artifact, request,
  receipt and journal SHA-256 digests, the complete receipt, status and
  controls. It is written only after the receipt is re-verified against that V5
  pin, never against the embedded exact-guest pin.
- `summary.json` is written last.

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
distinct. It also runs the V5 native model and host gates and a V5 lint. It
executes no V5 guest and creates no V5 receipt. Every one of these gates fails
closed while `methods/guest-authrdf/Cargo.lock` is absent.

## Not established

Everything under "Not established" in the
[native model reference](authenticated-rdf-model.md) still applies:

- no credential status, holder binding, validity period, clock or DID resolution;
- no wallet or world completeness under either authority;
- no arbitrary JSON-LD or JSON mapping;
- no general SPARQL conformance.

The verifier's key table is the only trust input. No performance or cycle
figure is claimed.
