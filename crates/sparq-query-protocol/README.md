<!-- [OPUS-5.5] zkp-16.1: first executable contract layer for vcq draft 0. -->
# sparq-query-protocol

> **Experimental, unpublished (`publish = false`), zero dependencies.** Typed capability
> negotiation and method contract for the [VC query protocol draft](../../research/vc-query-protocol.md)
> and its [method registry](../../research/vc-query-methods.json). This crate contains no proof
> backend, interoperable wire encoding or transport, RDF/SPARQL canonicalization, query parser,
> hash or other cryptography, **proves no cryptographic claim**, and is not externally audited
> (sq-qhy4). No workspace crate consumes it yet, so `sparq-core`, `sparq-engine` and the default
> wasm build are unchanged.

## Current subset

| Draft | API |
|---|---|
| §5.1 request, negotiation fields only | `RequirementsSpec` → `QueryRequirements::new` |
| §5.2 contracts and modes | `ResultContract`, `EvaluationMode` (`ResultContract::supports_mode`) |
| §5.3 scope | `ScopeAuthority` × `SourceEvidence` (orthogonal), `Completeness` |
| §6.1 descriptor, capabilities | `MethodDescriptor`, `ArtifactIdentity`, `CapabilitiesSpec` → `Capabilities::new`, `CapabilityTuple` |
| §6.2 admission and operations | `admit` → `Admission`; trait `QueryMethod`; `PreparedWitness` |
| §6.3 claim | `VerifiedClaim::new(&Admission, ClaimEvidence)`, `ClaimScope`, `ObligationOutcome` |
| §6.4 failures | `ProtocolError { class, phase, code }` |
| §7.3 enforcers | `Obligation`, `Enforcer`, `Enforcement`, `BindingRoute` |
| §5.1 stored request subset | `StoredRequestSpec` → `StoredRequest::new`, `Challenge32`, `BaseIri`, `QueryForm` |
| §6.4 challenge consumption | trait `ChallengeStore`, `consume_challenge`, `ChallengeOutcome`, `ChallengeStoreError` |
| local stand-in for §7.2 | `encode_stored_request`, `encode_method_descriptor` (profile `local-struct-v1`) |

<!-- [OPUS-5.5] zkp-16.2 request-binding prerequisite. -->
`StoredRequest` adds the exact query text (non-empty, at most `MAX_QUERY_LEN` = 1 MiB), the
original nonzero `Challenge32` (a separate type from `Digest32`), the exact audience, a
`not_before < not_after` window in Unix seconds, the declared `QueryForm`, an optional base IRI
and a DESCRIBE policy that is present exactly for DESCRIBE. The form must take the requested
result contract. These are shape checks only: the query is not parsed, and `BaseIri` is a bounded
string with no whitespace or control characters, not an RFC 3987 IRI check. Adapters owe the
parse, the form check against it and IRI validation.

Still not modelled: issuer sets, status windows, trust-policy roots, disclosure policy, the
evaluation context other than DESCRIBE, and graph-size bounds. Not every credential policy is
represented, and none is enforced here. The first planned backend targets only source evidence
`none`, status `not-requested` and bearer holders. Credential import (§4) and linking-profile
semantics (§8) are out of scope; linking profiles are compared as exact identifiers only.
`DatasetAssembly::ExactSourceCatalog` names exact source N-Quads bytes with an exact graph
catalog; it is not a credential assembly and says nothing about authenticity.

## Local encoding (`local-struct-v1`)

`encode_stored_request` and `encode_method_descriptor` return exact bytes: a per-object domain
separator ending in `0x00`, then every field as a one-byte tag and value in ascending tag order,
with u64 big-endian length prefixes, explicit option presence, fixed-width integers, raw 32-byte
digests and challenges, and fixed enum-byte tables. Lists keep stored order, including method
preference. The schema table is in the `encoding` rustdoc and the
[skill reference](../../skills/zk-query-proofs/references/query-method-contracts.md). This is a
structural encoding of typed values: not RDF or SPARQL canonicalization, not the §7.2 wire
profile, not a transport, and there is no decoder. It computes no digest; an adapter names its
own hash.

## Shared challenge store

`ChallengeStore::consume(&self, &Challenge32)` is one atomic check-and-set returning `Fresh` or
`AlreadyConsumed`, or a typed `ChallengeStoreError` (mapped to `infrastructure`, never replay or
acceptance). One verifier shares one store namespace across all methods; each verification has
exactly one owner, which consumes the ORIGINAL request challenge, never a per-method derived
nonce, and the store must be durable before it returns `Fresh`. The trait cannot enforce
atomicity or persistence, and the crate ships no production store. `QueryMethod` should use
`type Request = StoredRequest` and `type ChallengeStore = dyn ChallengeStore`.

## Rules the code enforces

- Descriptors match by exact equality of id, version, parameter set and digest, artifact
  identity and backend pin. There are no wildcards, fallbacks or downgrades.
- A method declares whole `CapabilityTuple`s (query profile, contract, mode, authority, source
  evidence, status, holder, assembly, suite, mapping, linking, enforcers). Admission needs
  **exactly one** tuple to match every requested axis. A method advertising *holder-declared +
  issuer-authenticated* and *verifier-agreed + none* therefore rejects *verifier-agreed +
  issuer-authenticated*. Two matching tuples are ambiguous and also rejected.
- The query profile (dialect and fragment) is a per-tuple axis, not a method-wide list. A profile
  no tuple declares is `QueryProfileUnsupported`; a profile declared only under another tuple is
  `TupleUnsupported`. `Admission::query_profile()` returns the matched tuple's exact profile.
- Every owed obligation needs an enforcer that discharges it. `Enforcer::BoundOnly` (for example,
  a policy hashed into the challenge) discharges nothing, and a public host check discharges only
  the anchor comparison.
- `Capabilities::is_executable` requires `adapter_available` **and** an implemented component;
  a component status alone is never enough.
- Selected-support presentations cannot serve exact requests, and re-attested evidence cannot
  serve issuer-authenticated requests. The same holds for any other axis mismatch.
- Missing source-evidence, status or holder policy is `invalid` at request validation. Zero
  versions, all-zero digests, zero bounds and malformed or wildcarded identifiers are rejected.
- A bound over a method ceiling is `capacity` and names the bound. Only
  `ProtocolError::capacity` can report `capacity`, and a backend must name its bound.
- `admit` validates trusted local capability **declarations** against the requirements. It runs
  no enforcer and does not show that a declared check exists or ran.
- `Admission` has private fields and is only built by `admit`. That shows only that the value
  passed structural admission; anyone can declare local capabilities, so an `Admission` is not an
  unforgeable security token. `PreparedWitness` has no `Clone` or serialization, its `Debug`
  output hides the witness, and `prove` consumes it.
- A `VerifiedClaim` rejects evidence that omits an owed obligation. Obligations the request did
  not owe are `NotEstablished`, whatever a backend reports.

## Example

The crate-level rustdoc holds the complete, doctested version of this sketch:

```rust,ignore
use sparq_query_protocol::{admit, Capabilities, QueryRequirements, Obligation};

// Verifier configuration: exact descriptors, capability tuples and requirements.
let capabilities = Capabilities::new(capabilities_spec)?;   // the local backend's declaration
let requirements = QueryRequirements::new(requirements_spec)?; // verifier-owned, stored

// The presentation names a descriptor. The verifier recomputes admission itself.
let admission = admit(&requirements, &presentation_descriptor, &capabilities)?;
let profile = admission.query_profile(); // the matched tuple's exact dialect and fragment
for (obligation, enforcer) in admission.obligations() {
    // Each owed obligation, with the enforcer the adapter's verify must actually run.
}
// method.verify(&stored_request, &admission, &presentation, &challenge_store)?
```

## What a real adapter still owes

Admission and a `VerifiedClaim` certify no cryptographic fact. `QueryMethod` implementations
are **trusted local code**; implementing the trait is not evidence that they are correct. Each
adapter must itself:

1. bind the stored request and the released result into the proved statement (proved,
   journaled or derived route) and recompute the statement digest under a named encoding and a
   named hash (§7.1, §7.2). It must not hash `Debug` output or an ad hoc JSON rendering;
2. check audience and validity window against the `StoredRequest`, parse the query, confirm
   its declared form, and validate any base IRI. Every current path leaves audience and
   validity unbound (§9 gap 4);
3. consume the original challenge once through the verifier's shared `ChallengeStore`, per its
   declared owner and `ChallengeConsumption`, and never consume it again when the wrapped method
   owns it (§6.4);
4. check issuer key authorization from verifier-pinned trust material (§4.2 rule 9) before
   reporting `Authenticity`, and check status within the verifier window before reporting `Status`;
5. link hidden witnesses across obligations with the admitted linking profile (§8). A shared
   nonce or equal results are not linkage;
6. load artifacts only from verifier configuration and check them against the descriptor's
   `ArtifactIdentity`;
7. bound presentation size and row count before decoding, and report `ClaimEvidence` only for
   checks it actually ran.

A request or challenge digest supplied by an adapter does not show that any hidden predicate
was checked. Every draft registry entry still has `adapter_available: false`, and this crate
registers no method.
