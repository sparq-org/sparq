<!-- [OPUS-5.5] zkp-16.1: first executable contract layer for vcq draft 0; zkp-16.2 request binding. -->
# sparq-query-protocol

> **Experimental, unpublished (`publish = false`), zero dependencies, not externally audited
> (sq-qhy4).** Typed capability negotiation and method contract for the
> [VC query protocol draft](../../research/vc-query-protocol.md) and its
> [method registry](../../research/vc-query-methods.json). This crate **proves no cryptographic
> claim** and has **no proof backend**: no actual method, no hash or other cryptography, no query
> parser, no RDF/SPARQL canonicalization, and no interoperable wire encoding or transport. Every
> draft registry entry still has `adapter_available: false`, and this crate registers no method.
> No workspace crate consumes it yet, so `sparq-core`, `sparq-engine` and the default wasm build
> are unchanged.

## 🚀 Quickstart

The crate is not on crates.io; depend on it by path inside the workspace. A sketch of the
verifier-side flow (the complete, doctested version is the crate-level rustdoc,
`cargo doc -p sparq-query-protocol --open`):

```rust,ignore
use sparq_query_protocol::{admit, Capabilities, StoredRequest};

// Verifier configuration: the local backend's declaration and the stored request.
let capabilities = Capabilities::new(capabilities_spec)?; // exact descriptors, capability tuples
let request = StoredRequest::new(stored_request_spec)?;   // requirements, query, challenge, audience

// The presentation names a descriptor; the verifier recomputes admission from the stored
// request's own requirements. Admission is structural only and accepts no proof.
let admission = admit(request.requirements(), &presentation_descriptor, &capabilities)?;
for (obligation, enforcer) in admission.obligations() {
    // Each owed obligation, with the enforcer the adapter's verify must actually run.
}

// Only a registered, trusted `QueryMethod` (this crate ships none) can accept a proof:
// let claim = method.verify(&request, &admission, &presentation, &shared_challenge_store)?;
// That method or adapter consumes the ORIGINAL challenge exactly once at its declared
// `ChallengePolicy`; callers must not consume it separately.
```

## ✨ Features

- **Exact admission.** `admit` matches descriptors by exact id, version, parameter set and
  digest, artifact identity and backend pin — no wildcards, fallbacks or downgrades. Exactly one
  declared `CapabilityTuple` must match every requested axis (query profile included); zero or
  two matches are rejected. Every owed obligation needs an enforcer that discharges it
  (`Enforcer::BoundOnly` discharges nothing).
- **Honest outputs.** `Admission` shows only that local **declarations** passed structural
  admission; it runs no enforcer and is not an unforgeable security token. `VerifiedClaim`
  rejects evidence that omits an owed obligation and reports un-owed ones as `NotEstablished`.
  `PreparedWitness` has no `Clone` or serialization and hides its witness in `Debug`.
- **Typed failures.** `ProtocolError { class, phase, code }`; only `ProtocolError::capacity`
  reports `capacity`, and it names the exceeded bound.
- **Stored request subset (§5.1).** `StoredRequest` holds the exact query text, the original
  nonzero `Challenge32`, audience, a `not_before < not_after` window, the declared `QueryForm`,
  an optional `BaseIri` and a DESCRIBE policy. These are shape checks only: the query is not
  parsed and `BaseIri` is not an RFC 3987 check.
- **Shared original challenge (§6.4).** `ChallengeStore::consume` is specified as one atomic,
  durable check-and-set returning `Fresh` or `AlreadyConsumed`; store errors map to
  `infrastructure`. One store namespace is shared across methods and the original challenge,
  never a per-method derived nonce, is consumed. The trait cannot enforce atomicity or
  durability, and the crate ships **no production store**.
- **Local structural encoding.** `encode_stored_request` and `encode_method_descriptor` emit
  deterministic `local-struct-v1` bytes. This is **not** the §7.2 wire profile, not a transport,
  and not RDF or SPARQL canonicalization; there is no decoder and no digest is computed.

### Not modelled yet

The stored request subset **omits detailed trust, status and disclosure policy**: issuer sets,
status windows, trust-policy roots, disclosure policy, evaluation context other than DESCRIBE,
and graph-size bounds. None of these is enforced here. Credential import (§4) and linking-profile
semantics (§8) are out of scope; linking profiles compare as exact identifiers only.
`DatasetAssembly::ExactSourceCatalog` says nothing about authenticity.

### What a real adapter still owes

`QueryMethod` implementations are **trusted local code**; implementing the trait is not evidence
of correctness. Each adapter must itself bind the stored request and released result into the
proved statement under a named encoding and hash; check audience, validity window, query form
and base IRI; consume the original challenge exactly once through the shared store; check issuer
key authorization and status from verifier-pinned trust material; link hidden witnesses under the
admitted linking profile; load and check artifacts from verifier configuration; bound input size
before decoding; and report `ClaimEvidence` only for checks it actually ran.

## 📚 Learn more

- Crate rustdoc (`cargo doc -p sparq-query-protocol --open`): full API, the complete doctested
  example, and what adapters still owe.
- [Query method contracts reference](../../skills/zk-query-proofs/references/query-method-contracts.md):
  API by draft section, failure mapping (§11 vectors), the `local-struct-v1` schema table and
  what is not covered.
- [VC query protocol draft](../../research/vc-query-protocol.md) and
  [method registry](../../research/vc-query-methods.json).

## License

License: MIT.
