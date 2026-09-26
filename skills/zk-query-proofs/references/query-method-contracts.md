<!-- [OPUS-5.5] zkp-16.1: usage reference for the experimental sparq-query-protocol crate. -->
# Query-method contracts (`sparq-query-protocol`)

`crates/sparq-query-protocol` is an experimental, unpublished, dependency-free leaf crate. It
holds the typed negotiation and method contract of the
[VC query protocol draft](../../../research/vc-query-protocol.md) (§5 to §7), a validated stored
request subset, a named LOCAL structural encoding and a shared challenge-store contract. It has
**no proof backend, interoperable wire encoding, transport, parser, hash or other cryptography,
proves no cryptographic claim**, and is not externally audited (sq-qhy4). Every entry in the
[method registry](../../../research/vc-query-methods.json) keeps `adapter_available: false`.
The crate registers no method. See the [crate README](../../../crates/sparq-query-protocol/README.md)
for the full rule list. [OPUS-5.5] Its only consumer is the optional, off-by-default `vcq`
feature of the detached exact-evaluator host crate; see [vcq exact adapter](vcq-exact-adapter.md).

## API

```toml
sparq-query-protocol = { path = "crates/sparq-query-protocol" }
```

- Identity: `Identifier::new(&str)` (exact `scheme:rest`, no wildcard), `Digest32::new([u8; 32])`
  (all-zero rejected), `MethodDescriptor::new(method, version, parameter_set, parameter_digest,
  ArtifactIdentity, backend)`. `ArtifactIdentity` is `VerificationKey`, `ZkvmGuest` or `Composite`.
- Axes: `ResultContract`, `EvaluationMode`, `ScopeAuthority`, `SourceEvidence`, `StatusPolicy`,
  `HolderPolicy`, `DatasetAssembly`, `Completeness`, `ComponentStatus`, `QueryProfile`.
  `DatasetAssembly::ExactSourceCatalog` is exact source N-Quads bytes plus an exact graph
  catalog, distinct from the credential assemblies and orthogonal to authenticity.
- Capabilities: `Capabilities::new(CapabilitiesSpec { descriptor, status, adapter_available,
  tuples, ceilings, challenge, disclosure })`. Each `CapabilityTuple` is one allowed
  combination, carries its own exact `query_profile`, and carries an `Enforcement` (one
  `Enforcer` per `Obligation`). There is no method-wide profile list, so a profile never
  crosses tuples.
- Requirements: `QueryRequirements::new(RequirementsSpec { .. })`. Source evidence, status and
  holder policy are `Option`s only so that an omission is `invalid`. `required_obligations()`
  lists what is owed, and a read-only getter exists for every field.
- Stored request: `StoredRequest::new(StoredRequestSpec { requirements, query, challenge,
  audience, not_before, not_after, form, base_iri, describe_policy })`, immutable through
  getters. `Challenge32::new([u8; 32])` rejects all-zero bytes and is not a `Digest32`.
  `BaseIri::new(&str)` is a shape check only (1 to `MAX_BASE_IRI_LEN` bytes, no whitespace or
  control characters, `?` allowed); it is not IRI validation. `QueryForm::supports_contract`
  pairs SELECT with the SELECT contracts, ASK with the ASK contracts and CONSTRUCT or DESCRIBE
  with `GraphRdfc10`. The query is not parsed; adapters check that its parsed form matches.
- Encoding: `encode_stored_request(&StoredRequest)` and `encode_method_descriptor(&descriptor)`
  return `Vec<u8>` under `LOCAL_ENCODING_PROFILE` (`local-struct-v1`), see the schema below.
- Challenges: trait `ChallengeStore: Send + Sync` with `consume(&self, &Challenge32) ->
  Result<ChallengeOutcome, ChallengeStoreError>`; `consume_challenge(&store, &stored_request)`
  consumes the request's ORIGINAL challenge and maps replay to `invalid` and store failure to
  `infrastructure`.
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
| all-zero challenge | invalid, request, `ZeroChallenge` |
| empty query, or longer than `MAX_QUERY_LEN` | invalid, request, `MalformedQuery` |
| malformed base-IRI shape | invalid, request, `MalformedBaseIri` |
| `not_before >= not_after` | invalid, request, `InvalidValidityWindow` |
| form does not take the result contract | invalid, request, `FormContractMismatch` |
| DESCRIBE without policy, or policy on another form | invalid, request, `DescribePolicyMismatch` |
| original challenge already consumed | invalid, verify, `ChallengeReplayed` |
| challenge store failed | infrastructure, verify, `ChallengeStoreFailure(code)` |

## Local encoding schema (`local-struct-v1`)

<!-- [OPUS-5.5] zkp-16.2: mirrors the `encoding` module rustdoc; change both together. -->
A structural, byte-exact encoding of this crate's typed values. It is **not** RDF or SPARQL
canonicalization, not the draft §7.2 wire profile, not a transport and has no decoder, and it
computes no digest. Each object starts with its domain separator
(`sparq:vcq:stored-request:local-struct-v1` or `sparq:vcq:method-descriptor:local-struct-v1`,
then `0x00`), then each field as a one-byte tag plus value in ascending tag order. `bytes` is a
u64 big-endian length then the bytes; `u32`/`u64` are big-endian; `fixed32` is 32 raw bytes;
`option` is `0x00`, or `0x01` then the value; `list` is a u64 big-endian count then each entry in
stored order (never sorted).

| Tag | Stored-request field | Value |
|---|---|---|
| `01` to `08` | query, base IRI, challenge, audience, not before, not after, form, DESCRIBE policy | `bytes`, `option<bytes>`, `fixed32`, `bytes`, `u64`, `u64`, `enum`, `option<bytes>` |
| `09` to `0b` | contract, mode, authority | `enum` |
| `0c` | anchor | `option<fixed32>` |
| `0d` to `10` | source evidence, status, holder, assembly | `enum` |
| `11`, `12` | dialect, fragment | `bytes` |
| `13` to `15` | accepted suites, mappings, linking | `list<bytes>` |
| `16`, `17` | released-row bound, presentation-byte bound | `u32` |
| `18` | methods, preference order | `list<bytes>`, each a full descriptor encoding |

| Tag | Descriptor field | Value |
|---|---|---|
| `01` | method id | `bytes` |
| `02` | version | `u32` |
| `03` | parameter set | `bytes` |
| `04` | parameter digest | `fixed32` |
| `05` | artifact | variant byte, then its `fixed32` fields |
| `06` | backend pin | `bytes` |

Enum bytes start at `01` in declaration order: form Select, Ask, Construct, Describe; contract
SelectDistinctSet, SelectBag, SelectSequence, AskBoolean, AskTrueOnly, GraphRdfc10; mode
SelectedSupport, ExactBounded; authority VerifierAgreedAnchor, HolderDeclared; source evidence
None, IssuerAuthenticated, ReAttested; status Required, NotRequested; holder Required,
BearerAccepted; assembly UnionDefaultGraph, CredentialNamedGraphs, ExactSourceCatalog;
artifact VerificationKey (vk), ZkvmGuest (artifact, image id), Composite (circuit, setup).
`tests/request_binding.rs` pins golden bytes for one descriptor and one stored request. Any
change to a tag, table, order or value form needs a new profile name.

## Not covered

This crate has no interoperable wire request or transport, statement encoding, digest
computation, production challenge store, query parser, IRI validation, credential import,
issuer-authorization check, status check, linking check or proof check. The stored request is a
subset of the draft: issuer sets, status windows, trust-policy roots, disclosure policy and most
of the evaluation context are not modelled. Adapters owe all of these; the README lists each
obligation. Because `QueryMethod` implementations are trusted local code, trait compliance is
not evidence of correctness.
