# Native RDF public support

[GPT-6] The detached `zk/native-composition` workspace has an optional
`native-rdf` research API. It authenticates **successful support** for a bounded
SELECT DISTINCT basic graph pattern using native Dock BBS+ signatures. It is
experimental and has not received external cryptographic review. It does not
establish result completeness, holder identity, or arbitrary SPARQL evaluation.

## Run and call

The existing tuple executable remains the default. The RDF feature adds the
`native-rdf` executable and `sparq_native_composition_spike::rdf` module:

```sh
cargo test --locked --manifest-path zk/native-composition/Cargo.toml --features native-rdf --lib
cargo run --release --locked --manifest-path zk/native-composition/Cargo.toml --features native-rdf --bin native-rdf
```

The executable generates fresh synthetic issuer keys and a proof locally,
independently verifies it, checks typed tamper/replay outcomes, and emits JSON.
It accepts no arguments and changes no live issuer state. The dedicated native
workflow executes the actual proof tests and uploads this report. Internal
signature-only/canonicalization-only timings and memory are unknown; the report
leaves them null. Inclusive local API wall times are not canonical benchmarks
and do not justify comparisons with a different authentication/disclosure contract.

Library entry points are `Issuer::generate`, `Issuer::public`, `issue_rdf`,
`prove_public_bgp`, `verify_public_bgp`, and `public_context`. The last exports
the exact context including trusted key bytes and accepted snapshot digests;
the challenge remains a separate proof input. A `Request` carries the verifier's
exact query bytes, distinct public `Mapping` rows, trusted `RolePolicy` list and
fresh challenge. Mapping values are canonical RDF term spellings, such as
`<urn:alice>` or `"Alice"`; variable names omit the question mark. The verifier
must select issuer keys and accepted status snapshots independently. Passing
holder-supplied keys/snapshots into a `Request` does not authenticate their trust.
`TrustedIssuer::to_bytes` and `TrustedIssuer::from_bytes` transport the fixed
compressed public key across that boundary, rejecting malformed points and
infinity. Importing a well-formed key does not establish its issuer identity.

The process-local `ConsumedNonces` consumes a challenge only after successful
verification and rejects reuse or a full store. Dropping it loses replay history;
an application must retain challenges durably. This API does not fetch issuer
keys, authenticate status-list documents, discover the latest epoch, or provide
a network protocol.

## Admitted relation

Queries must parse as SELECT DISTINCT, projection, and a nonempty BGP over the
default graph. Every BGP variable must be projected and bound in every released
row. For example:

```sparql
SELECT DISTINCT ?name WHERE { <urn:alice> <urn:name> ?name }
```

The verifier reconstructs each required triple from the query constants and
released row. Subjects and predicates must be IRIs; objects can be IRIs or
RDF 1.1 literals. Blank nodes, triple terms, directional language literals,
hidden variables, dataset clauses, graph patterns outside that admitted shape,
FILTER, BIND, property paths, aggregates, and graph-output queries are rejected.
This is a public-preimage optimization, not a substitute for the exact evaluator.

Capacity is fixed and public: sixteen canonical triple slots per credential,
plus one protocol and one status message. Requests admit at most four issuer
roles, eight rows and eight BGP patterns. Query bytes, document bytes, term
bytes, input-triple count, status bytes, proof statement sizes and replay-store
size have explicit bounds in `rdf.rs`. Exceeding a bound rejects; credentials
or rows are never silently truncated. Every accepted issuer role must contribute
support. The convenience prover considers all matching credential slots and
deterministically selects an allocation covering every role when one exists.
It tracks reachable role subsets within the fixed capacities; overlapping
credentials cannot make a first-match choice exclude a required role. This
witness-selection change retains the versioned verifier relation and wire format.
The chosen public slot indices can differ from a former first-match allocation.

Credential issuance parses a bounded N-Quads document containing only default
graph triples and uses `sparq-canon::canonicalize_triples`. Canonical lines are
sorted and deduplicated: the signed object is an RDF graph set, not input order,
duplicate input occurrences, original document bytes, or a JSON object. Literal
lexical form, datatype and language remain part of the authenticated RDF term;
`"021"^^xsd:integer` is not replaced by `"21"^^xsd:integer`.

The fixed message vector signs a domain-separated protocol value, the exact
status reference, and domain-separated hashes of canonical triple preimages.
Unused slots use a separate padding domain. Hash-to-field uses the pinned Dock
utility with Blake2b512 and BLS12-381 Fr. This is a new experimental encoding,
not W3C bbs-2023 or authentication of an existing VC through reattestation.

## Verifier and disclosure contract

A presentation contains only bounded native BBS+ proof bytes and row/pattern
slot references. The verifier derives the required disclosed message hashes,
builds its own Dock `ProofSpec`, and selects its own trusted public keys.
The versioned envelope admits only BBS+ statement payloads, checks every frame
length, and rejects trailing bytes; it cannot select another generic proof-system
variant or an aggregation decoder.

The proof context binds exact query bytes, released mappings, ordered issuer
roles and keys, support indices, and accepted status-list policy. The verifier's
challenge is a separate proof input. The status message binds list IRI, epoch
and bit index. The verifier checks the index against its complete accepted
snapshot, checks that the bit is unset, and binds the snapshot's digest in the
context. A holder cannot substitute another list, epoch, index or snapshot as
trusted input.

The epoch is an issuance-time label inside the issuer-signed status message,
not a freshness discovery mechanism. A different verifier-accepted epoch requires
a credential signed for that exact label; an existing presentation fails
verification against the changed label. The caller can select updated snapshot
bytes under the same signed label, with the exact accepted snapshot digest bound
into each newly constructed proof context.

Status input is a raw byte vector: index `i` selects byte `i / 8` and mask
`1 << (i % 8)`, least significant bit first within each byte. This API does not
decode a [W3C Bitstring Status List encodedList](https://www.w3.org/TR/vc-bitstring-status-list/#bitstringstatuslistcredential)
or authenticate a status-list credential; an application must supply the stated
native representation and independent trust policy.

Public transcript fields include query, mappings, issuer roles, fixed capacity,
status list/epoch/index, and **disclosed signed-slot indices**. Those indices
reveal position/rank in the canonical message vector. This disclosure differs
from the Noir route with a hidden status reference; a benchmark must retain that
distinction. An accepted nonempty row has signed supporting triples; unreturned
solutions may still exist. Output order is context-bound even though the admitted
query has set semantics and provides no ORDER BY guarantee.

No residual circuit is needed for the public-preimage relation because the
verifier sees and recomputes every required preimage. Hidden canonical-triple
hashes cannot be equated with private numeric values merely by sharing a nonce.
The separate tuple executable uses actual Dock `EqualWitnesses` within one
BLS12-381 proof; its signed numeric tuple relation remains distinct. There is
no implemented Noir/BN254 same-witness bridge in this RDF route.

## Validation scope

The native library tests construct real single- and two-issuer BBS+ proofs and
check altered query, subject/predicate/object, lexical/datatype identity,
issuer, challenge, public slot references, status policy, revocation, malformed
wire input and replay. A deliberately weaker proof is first verified under its
own weaker specification with the same context and nonce, then rejected by the
independently constructed required verifier specification. Structural input
tests are separate from proof executions; test-function counts are not receipt
counts. The [local evidence](../../../zk/native-composition/evidence/native-rdf-local-smoke.json)
pins the executed source, original report, executable, typed outcomes and guard
mutations. Its local debug timings are validation evidence only.

The [dependency gate record](../../../zk/native-composition/evidence/native-rdf-dependency-gates.json)
preserves failed license, advisory and source-audit coverage checks. Running the
same accepted policy on the exact foundation baseline produced the same failures;
that comparison does not make the dependency gates pass. Dependency remediation
and independent review remain required before publication is ready to land.

All dependencies remain in the detached opt-in workspace. Its existing build
script still requires Circom even for this RDF feature, and the pinned
`proof_system` dependency still activates the LegoGroth16/Circom/Wasmer graph.
The RDF proof itself uses no residual circuit. No dependency fork hides that
build cost or changes the original tuple executable's supported behavior.

The native-only [Ark tracing patch](../../../zk/native-composition/vendor-support/README.md)
has independent provenance and compatibility checks. Its callback rename does
not change constraint arithmetic; the historical unpatched evidence above
retains its original source identity and dependency failures.

Primary references: [SPARQL BGP matching](https://www.w3.org/TR/sparql11-query/#BasicGraphPatternMatching),
[RDF dataset canonicalization](https://www.w3.org/TR/rdf-canon/), and
[Dock composite proof statements](https://github.com/docknetwork/crypto/tree/main/proof_system).
