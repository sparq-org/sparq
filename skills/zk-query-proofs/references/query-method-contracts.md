<!-- zkp-16.1: usage reference for the experimental sparq-query-protocol crate. -->
# Query-method contracts (`sparq-query-protocol`)

`crates/sparq-query-protocol` is an experimental, unpublished, dependency-free leaf crate. It
holds the typed negotiation and method contract of the
[VC query protocol draft](../../../research/vc-query-protocol.md) (§5 to §7), a validated stored
request subset, a named LOCAL structural encoding and a shared challenge-store contract. It has
**no proof backend, interoperable wire encoding, transport, parser, hash or other cryptography,
proves no cryptographic claim**, and is not externally audited (sq-qhy4). The crate registers
no method. <!-- --> In the [method registry](../../../research/vc-query-methods.json),
`adapter_available` is true only for `method:risc0-exact` version 3, and only for the six tuples
of its experimental adapter: SELECT bag, ASK and CONSTRUCT, each under holder-declared or
verifier-agreed authority, with source evidence `None`, status `NotRequested` and holder
`BearerAccepted`. It is false for every other entry and version. See the [crate README](../../../crates/sparq-query-protocol/README.md)
for the full rule list. Its consumers are the optional, off-by-default `vcq`
feature of the detached exact-evaluator host crate (see [vcq exact adapter](vcq-exact-adapter.md))
and, since zkp-14.6, its `vcq-authenticated` feature: a V5 issuer-authenticated RDF adapter
whose registry entry keeps `adapter_available` false, because an independently audited genuine
receipt covers only one of its six tuples, `select-bag-verifier-agreed`, on one public synthetic
fixture (see [vcq authenticated-RDF adapter](vcq-authenticated-rdf-adapter.md)). That adapter
is the first source to declare `SourceEvidence::IssuerAuthenticated` tuples with a suite. It
names its guest relation as the authenticity enforcer; that is a source declaration, exercised
by that one receipt and not by any receipt for the other five tuples.

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

<!-- zkp-16.2: mirrors the `encoding` module rustdoc; change both together. -->
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

The host compiler modules and presentation values build for WebAssembly; the
native `method::Compiler` alone owns subprocesses and temporary proof files.
`Request::plan()` reconstructs the trusted plan and credential limits, while
`Answer::release` converts its canonical result through the supplied vocabulary.
`method::Prepared` and `method::Presentation` remain available on both targets.

`statement::EnvelopeInputs` carries the verifier-stored typed envelope policy.
Admit it with `Request::new_authenticated_with_policy(stored, bounds, key,
envelope, quad_bounds)` and obtain it through `Request::envelope_inputs()`.
`Layout::reconstruct_authenticated_with_envelope` and
`witness::prover_toml_authenticated_with_envelope` consume the same typed group.
The existing authenticated methods retain the issuer-only profile. Enabled
holder, status, validity and freshness policies require the statement lane's
consumer; the current native driver refuses them by name until it is integrated.
Typed canonical policy inputs also require that consumer, even when their tree
depth happens to match the issuer-only relation.
`Request::new_authenticated_with_envelope(stored, bounds, key, policy,
quad_bounds)` takes `envelope_policy::PolicyInput`, derives its canonical roots
and retains it for fresh `envelope_inputs()` derivation during verification.
Validity and freshness times are exact nonnegative Unix epoch milliseconds within
the suite's signed timestamp range. A present-empty
holder issuer list is refused; omit holder binding to require no holder keys.

Expression evaluation uses the shared typed gadgets, with a separate capacity
channel. The generated-query coverage ratchet requires native and WebAssembly
ABI, bytecode and independently derived key agreement for every passing operator
program. Authenticated delivery uses the native catalogue and requires native
compile/key determinism; the Node package refuses unknown plans or changed keys.
Isolated gadget evidence stays distinct from generated-query and proof evidence.

The credential suite now authenticates proof metadata. `Credential::metadata`,
`Credential::signed_message` and `credential::dataset_commitment` return `Result`;
propagate their errors. Witness generation carries creation and verification
method as private openings and uses the authenticated opener's returned message.
`credential::credential_graph(request_digest, index)` shares the suite's
presentation-local graph-name construction. Credentials using the reserved
`vcq:verifiedIssuer` predicate are refused before dataset assembly.

Constant membership lists use a fixed parameter block and a public active-length
index, so changing their contents or admitted length preserves plan hash and
program source. Oversized lists receive the named `IN list` capacity refusal.
The existing parameter enum and envelope group order are unchanged.

For the closed-world integration, `credential::assemble_dataset` takes complete
opened credential cells, envelope-authenticated issuer cells and the request
digest. Its `interp::Dataset` exposes `quads()` and `credential_counts()`;
`interp::evaluate_dataset` evaluates the default merge while retaining named
copies for the graph dispatcher. The method-generated issuer assertions occur
only in the default view. These helpers await production wiring to the envelope
consumer and graph dispatcher.

`credential::assemble_issuer_dataset` accepts complete signed credentials, typed
trust members and the request digest. It derives each assertion's issuer from
the credential's unique root and issuer property, authenticated verification
method and signing key. Trust roots use `envelope_policy`'s canonical set
construction. This host helper prepares the issuer dataset; the generated
consumer remains required to enforce its relation within a proof.

The Noir seam is `ops_core_suite::authenticate` followed by the envelope checks
and `from_authenticated`; inactive slots use `inactive_credential`. This reuses
the authenticated suite object and its metadata getters. `Dataset` retains
triple rows and active flags, with a parallel graph-name array.
`append_verified_issuer` receives an issuer term already checked by the envelope
and adds its default-only assertion. It does not supply that trust check itself.

`Answer::release` includes canonical boolean terms for computed results, even if
they were absent from the source vocabulary. Other result terms use the supplied
vocabulary or the plan's public constants.

The circuit library's `ops_core_profile` contains raw witness structs and
resource normalization only. Generated credential openings call
`ops_core_suite::open_profile_one`, delegating the complete signed opening to
`suite::open_bounded_credential_hidden` once per credential. Resource datatype,
language and key bytes are zero; the default graph and active flags are derived.
Scans, joins, GRAPH and DISTINCT compare authenticated term identity, retaining
blank-node scope. Value consumers call `term::open_key_checked` with fixed datatype
identifiers. Raw lexical backing is separately bounded for resources, objects
and scalars. Shape encoding includes all these capacities.
The verifier selects `capacity::OpeningProfile::{Solid, LongObjects, Extended}`. Its
`limits()` reports the compiled resource, object, scalar, datatype, language,
creation and verification-method bounds. `Solid` is the default. Choose
`LongObjects` for holder Multikey lexical forms with compact resource/metadata
bounds, or `Extended` for longer identifiers;
the compiler never resizes a profile from private data. Admission through
`Credential::cells_with_profile` and witness rendering refuse excess bytes.
Use `Plan::with_opening_profile` for portable generation, or
`Compiler::prepare_with_opening_profile(query, bounds, quad_bounds, profile)`
for native preparation. The matching request constructor is
`Request::new_authenticated_with_opening_profile(stored, bounds, key,
issuer_root, quad_bounds, profile)`; typed envelope policies use
`new_authenticated_with_envelope_and_profile(stored, bounds, key, policy,
quad_bounds, profile)`. Existing constructors select `Solid`.
`Request::plan()` restores the selected profile and `opening_profile()` exposes
it. Different profiles produce different plan hashes and keys.
The additive envelope seam is `authenticate_bounded` followed by its consumer
and `from_authenticated_bounded`. Existing unbounded fixture entry points remain
available, with the same suite authentication relation.

String equality keeps type errors distinct from false. Term-identity tokens are
local to same-datatype string equality; mixed string types and value families
pass through the shared comparison's type classes. These tokens must never be
used as lexical input by string functions.

The query proof tests default to `SPARQ_NOIR_QUERY_PROOF_PROFILE=full`.
`smoke` selects a small authenticated ASK sample for hosted CI. Unknown values
are refused; the workflow dispatch input `full_query_proofs` runs the complete
acceptance sweep. Each selected proof still checks its engine result and refuses
an altered answer. Local completion requires the full profile.

The shared Noir bridge exposes `ops_core_expr::temporal_payload(key: Field) ->
Field` for checked suite keys. It extracts the leading biased time word and
constrains the remaining payload bytes to zero. Graph and expression dispatchers
pass the full packed key and retain its checked class separately when constructing
`TemporalLane`; the lane's key is the scalar word.

The generated authenticated ABI also carries `revealed_count: u32`,
`revealed_messages: [Field; C]` and `revealed_keys: [Field; C]` after the ordered quad
bounds and before the statement binding. Hidden mode requires zero words there.
Revealed mode binds each active credential's message and suite Multikey encoding
and requires inactive slots to be zero. `statement::RevealedInputs::verify` takes
ordered `RevealedSignature {key, message, signature}` values and checks each shared
suite signature before admitting it; its stored messages and keys are private.
`Layout::reconstruct_revealed` accepts that admitted value and a mode-zero statement.
`witness::prover_toml_revealed` renders a witness from complete credentials using the
same verification. The native `Compiler` retains its hidden-only contract; the
statement adapter must enforce its request's accepted mode before using these APIs.
Both modes share one suite opening per active credential, with the shared Schnorr
verifier called for hidden mode. No presentation-supplied layout is accepted.

Authenticated query programs open complete credential identities once. Value-consuming
FILTER inputs validate each consumed literal on every active input row, including
rejected rows. Cells identical to verifier-reconstructed public parameters reuse
those canonical keys; other literal cells carry separate lexical openings. Each checked cell must equal the row's
authenticated encoding and metadata; no private source index selects the binding.
Select intermediate row bounds for the full scan output, independently of result
limits. An overflowing intermediate table refuses the proof.

`interp::Cell::enc` is the suite's key-independent RDF identity for private tables
and public request parameters. The suite's keyed encoding remains for credential
commitments, revealed verification keys and envelope trust-member derivation.
Plain-string equality and membership use identity; typed-literal equality and
membership remain value consumers. The generated read adapter checks every row
entering those consumers, including later rejected rows.

Released SELECT cells preserve identity, kind, datatype and language but set
`key_class` and `key` to zero. `Answer::encode`, `Answer::release`, the interpreter
and SELECT gadget share this conversion; `Layout` refuses released value metadata.
Result-local blank-node labels retain their existing canonical form. Public request
parameter cells keep their canonical value keys.

For toolchain CI, `python3 scripts/zk_query_ci.py hosted` loads and tests the
committed Noir manifests unchanged, runs the operator contracts, portable host
checks and small live controls, and checks agreement for every passing operator
plan. `full` runs the complete core, corpus, expressions and proof sweep and is
selected by explicit workflow dispatch. Both require the real pinned tools.
`--list` prints commands and `--root` selects a frozen checkout. Separate runtime
ledger profiles require every declared live entry and completion; a named XPath
failure records unavailable comparison rather than claiming byte or key identity.

Private scalar read admission applies the selected parser byte bound and supported
value datatypes at the consumer. An identity-only source term does not inherit a
scalar parser bound. Unsupported private value datatypes and oversized scalar
lexicals return named refusals before native execution; exact public parameter
matches retain their verifier-reconstructed key shortcut.

`table::assert_canonicalized(input, out)` verifies a stable canonical permutation
with exact multiplicity and complete metadata. The occurrence ordinals are
computed from the rows; there is no private permutation index. Scan checks its
canonical output directly against every source match and retains the first
matching metadata payload. Projection and SELECT use the permutation verifier.
Padding, output order and overflow remain part of each relation.

`ops_core_expr::temporal_payload_for_class(key: Field, class: u8) -> Field`
extracts the scalar word from a checked packed temporal key and checked class.
Its bounded reconstruction enforces the header and zero tail. The original
`temporal_payload(key)` byte-based helper remains available. Both keep the same
suite representation; the class-aware bridge is also pinned to native keys.

`table::assert_selection(input, selected, out)` verifies a complete stable
subsequence with computed destination ranks and full payload equality. FILTER
derives the selected flags from true, non-error predicates; inactive input rows
remain excluded, and an undersized output refuses.

A leading-column projection preserves the input’s canonical row order. It uses
`table::assert_ordered` to bind each row and its complete payload directly. Other
projections retain the stable canonical-permutation relation. Both paths check
cardinality, padding and capacity.

JOIN matches every compatible source pair to the first unused output occurrence
of its binding, retaining source-pair order for equal identities. The full payload
is bound at that occurrence. Complete pair coverage and complete output use
preserve bag multiplicity; an undersized output refuses. All occurrence choices
are computed inside the gadget.

Plain-string equality classifies its operands from the authenticated datatype
before value-key guards; identity-only comparison does not inspect unused keys.
