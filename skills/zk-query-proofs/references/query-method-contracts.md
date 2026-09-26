<!-- [OPUS-5.5] zkp-16.1: usage reference for the experimental sparq-query-protocol crate. -->
# Query-method contracts (`sparq-query-protocol`)

`crates/sparq-query-protocol` is an experimental, unpublished, dependency-free leaf crate. It
holds the typed negotiation and method contract of the
[VC query protocol draft](../../../research/vc-query-protocol.md) (§5 to §7). It has **no proof
backend, wire encoding or cryptography, proves no cryptographic claim**, and is not externally
audited (sq-qhy4). No crate depends on it, and every entry in the
[method registry](../../../research/vc-query-methods.json) keeps `adapter_available: false`.
The crate registers no method. See the [crate README](../../../crates/sparq-query-protocol/README.md)
for the full rule list.

## API

```toml
sparq-query-protocol = { path = "crates/sparq-query-protocol" }
```

- Identity: `Identifier::new(&str)` (exact `scheme:rest`, no wildcard), `Digest32::new([u8; 32])`
  (all-zero rejected), `MethodDescriptor::new(method, version, parameter_set, parameter_digest,
  ArtifactIdentity, backend)`. `ArtifactIdentity` is `VerificationKey`, `ZkvmGuest` or `Composite`.
- Axes: `ResultContract`, `EvaluationMode`, `ScopeAuthority`, `SourceEvidence`, `StatusPolicy`,
  `HolderPolicy`, `DatasetAssembly`, `Completeness`, `ComponentStatus`, `QueryProfile`.
- Capabilities: `Capabilities::new(CapabilitiesSpec { descriptor, status, adapter_available,
  tuples, ceilings, challenge, disclosure })`. Each `CapabilityTuple` is one allowed
  combination, carries its own exact `query_profile`, and carries an `Enforcement` (one
  `Enforcer` per `Obligation`). There is no method-wide profile list, so a profile never
  crosses tuples.
- Requirements: `QueryRequirements::new(RequirementsSpec { .. })`. Source evidence, status and
  holder policy are `Option`s only so that an omission is `invalid`. `required_obligations()`
  lists what is owed.
- Admission: `admit(&requirements, &selected_descriptor, &capabilities) -> Result<Admission, ProtocolError>`.
  `Admission` exposes the descriptor, the single matching tuple, its `query_profile()`,
  `obligations()` with their declared enforcers, resources, challenge policy, disclosure and
  completeness. `admit` checks trusted local declarations only: it runs no enforcer and does
  not show that a declared check exists or ran. `Admission` cannot be built outside the crate,
  which shows only that it passed structural admission; anyone can declare local capabilities,
  so it is not an unforgeable security token.
- Methods: trait `QueryMethod` with associated `Request`, `PrivateInputs`, `Witness`,
  `Presentation`, `Output` and `ChallengeStore`, and operations `capabilities`,
  `descriptor`, `admit`, `prepare`, `prove` and `verify`. `prepare` returns a
  `PreparedWitness<Witness>` (no `Clone`, redacted `Debug`), and `prove` consumes it through
  `into_inner_for(descriptor)`.
- Claims: `VerifiedClaim::new(&admission, ClaimEvidence { result, statement_digest, established })`.
  It fails `invalid` at verify when an owed obligation is missing, and records unowed ones as
  `NotEstablished`.
- Failures: `ProtocolError` with `class()`, `phase()` and `code()`. Capacity is reported only
  through `ProtocolError::capacity(phase, CapacityBound)`, and backend failures through
  `ProtocolError::backend(BackendFailure, phase, code)`.

## Failure mapping (draft §11 vectors)

| Situation | Class, phase, code |
|---|---|
| presentation names a descriptor the request does not list | invalid, negotiation, `DescriptorNotRequested` |
| local backend implements another version, parameter set or artifact | unsupported, negotiation, `DescriptorUnavailable` |
| adapter unavailable, whatever the component status | unsupported, request, `AdapterUnavailable` |
| missing source-evidence, status or holder policy | invalid, request, `MissingPolicy` |
| no tuple declares the requested query profile | unsupported, admit, `QueryProfileUnsupported` |
| no single declared tuple matches (weaker mode, authority upgrade, crossed tuples, profile declared only under another tuple) | unsupported, admit, `TupleUnsupported` |
| owed obligation only bound through the challenge, or absent | unsupported, admit, `UnenforcedObligation` |
| request bound above the method ceiling | capacity, admit, `CapacityExceeded(bound)` |

## Not covered

This crate has no wire request, statement encoding, digest computation, challenge store,
credential import, issuer-authorization check, status check, linking check or proof check.
Adapters owe all of them; the README lists each obligation. Because `QueryMethod`
implementations are trusted local code, trait compliance is not evidence of correctness.
