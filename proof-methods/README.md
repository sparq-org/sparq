# Proof methods outside the zero-knowledge circuits

A detached Cargo workspace for proof methods that are not Noir circuits or
zkVM programs, and for the baselines the zero-knowledge methods are compared
against. The methods take their identifiers from section 9 of the answer
specification (`site/specs/zksparql-answers.typ`), but they do not yet
implement its full proof-method profile: they work on the internal v5 request
and statement, which do not bind the protocol request's audience, acceptance
window, accepted methods or signature mode, and they define no canonical
statement or proof encoding. The workspace is not a member of the root
workspace, so it does not change the engine, the default build or the wasm
bundle.

| Crate | Method | Evidence kind |
|---|---|---|
| `disclosed` | `urn:sparq:vcq:method:disclosed-reevaluation` v1 | disclosed credentials |
| `vole` (scripts, not a crate) | prototype for `urn:sparq:vcq:method:vole-designated-verifier` | zero-knowledge proof, interactive, designated verifier |
| `tee` | `urn:sparq:vcq:method:tee-attestation` v1, platform `aws-nitro` | attestation |

`disclosed` runs the relation that the `risc0-authenticated-rdf` v5 guest
proves (`zk/sparql-evaluator/model`, `authenticated_rdf::evaluate`) natively,
on credentials the holder discloses. Its dataset commitment is the one the
zkVM method computes, so one verifier-agreed commitment serves both methods.
It is the baseline for the cost of the zero-knowledge methods: the same
statement, checked without a proof. It rejects DESCRIBE and FROM / FROM NAMED
before evaluation, and checks the request's key table and the credentials'
counts and sizes before copying either. Its evidence is publicly verifiable:
anyone holding the request can check it, so it is transferable.

Where SPARQL 1.1 permits more than one result (OFFSET or LIMIT without an
ORDER BY that strictly orders the solutions, REDUCED, SAMPLE, GROUP_CONCAT,
floating-point aggregates), `disclosed` fixes the choice as follows: the holder
chooses through the set of credentials it sends (the relation orders them by
document hash, so their order does not matter), and the verifier accepts only
the result the shared evaluator computes for that set. The
method publishes no rule for which permitted result that is, so a verifier must
not rely on it.

`vole` proves the ACIR of the `zk/compose` circuits with QuickSilver instead
of UltraHonk, so both proof systems can be compared on the same statement; see
[`vole/README.md`](vole/README.md).

`tee` runs the same relation inside an AWS Nitro Enclave. The enclave asks
the Nitro Secure Module for an attestation document whose `user_data` is the
SHA-256 statement digest and whose `nonce` is the request nonce; the verifier
validates the document's certificate path to the pinned AWS Nitro Enclaves
root G1 (RFC 5280, at the document's time and at verification time), its
ES384 COSE signature, its age, the enclave image measurement (PCR0) against
the ones it accepts, and the binding of statement and nonce. A document
verifies again until it is too old, so the verifier prevents replay by
issuing a fresh request nonce and accepting each nonce once. The verifier must
trust AWS's attestation keys and hypervisor, the measured program and the
enclave's isolation; it learns the statement and the attestation document, but
not the credentials. The evidence is publicly verifiable and transferable.
Tests run the verifier against a test certificate authority of the same shape;
`tee/nitro/run.sh` builds the enclave image and runs Q1–Q5 on an instance with
Nitro Enclaves enabled. Only the hidden signature mode is supported.

Research prototypes, not externally audited.

```sh
cargo test --release --manifest-path proof-methods/Cargo.toml
cargo run --release --manifest-path proof-methods/Cargo.toml \
  -p sparq-vcq-disclosed --example measure --features fixtures -- 31
```

`measure` prints one JSON object per query and credential count: issuer,
holder and verifier times (median and quartiles) and presentation sizes,
over the Q1–Q5 set in `disclosed/src/fixtures.rs`.
