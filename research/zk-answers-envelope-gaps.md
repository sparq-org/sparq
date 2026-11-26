# Credential evidence for Solid query answers

The query-method review is based on `e626490` on `main`. The Merkle section also covers
the native SHA successor. The authenticated Noir query method implements
the three relations required by DER-13. Whether a particular answer qualifies depends on
the verifier's query and policy. Both Node proof methods are research grade and externally
unaudited. [Issue #7379](https://github.com/sparq-org/sparq/issues/7379)

## DER-13 and credential validity

The verifier should count an answer only after verifying the proof against its own stored
request for an approved Solid query. That query must constrain the issuer for each required
fact. Holder binding must cover the relevant issuers and status must be required with
missing entries disallowed. The query, public constants, policy, dataset commitment and
released result are bound together. Issuer membership paths, holder signatures and status
indices can stay private. This gate needs no new circuit input.
[Statement reconstruction](../crates/sparq-zk-query-wasm/src/verification.rs) ·
[Generated constraints](../crates/sparq-zk-query/src/codegen.rs)

| Item | Status on main and evidence |
| --- | --- |
| A1 · Issuer per fact | **Done** in the query method. Every selected credential has a request-scoped named graph. The default graph contains the merged facts and a reserved `vcq:verifiedIssuer` assertion derived from the credential's issuer, signing key and authorized method. The request binds the issuer root and depth. Joining a fact's graph variable to that assertion can require a different issuer for each fact. An unscoped query makes no such demand. [Dataset assembly](../crates/sparq-zk-query/src/credential.rs) · [Two-issuer insurance query](../crates/sparq-zk-query/tests/fixtures/solid_paths/insurance-count.rq) |
| A2 · Holder binding | **Done** when requested. Public `holder_required` and the optional issuer scope select which credentials require possession of their subject's `confidenceMethod` key. A private Schnorr signature covers the domain-separated full request digest, including challenge and audience. Omitting the issuer list selects all issuers. The relation supports distinct required keys. The Node prover takes one holder key per call. [Policy and signature relation](../zk/noir-query/core/src/envelope.nr) · [Node evidence builder](../crates/sparq-zk-query-wasm/src/envelope.rs) |
| A3 · Status | **Done** when requested. Public `status_required`, `allow_missing_status`, `snapshot_root` and `snapshot_depth` constrain checks of the signed credential's entries. Each must open a clear revocation or suspension bit in a verifier-accepted snapshot for the same list and purpose. The verifier authenticates the list and decides how fresh it must be. The generated profile admits one entry per credential and refuses overflow. [Policy derivation](../crates/sparq-zk-query/src/envelope_policy.rs) · [Generated slot relation](../zk/noir-query/core/src/ops_core_envelope.nr) |
| A4 · Expiry | **Done** when requested. `validity_required` and public `now` select the inclusive credential interval at `validityTime` (epoch milliseconds). A single bound applies on its own. With neither bound, authenticated creation supplies the start and an unknown start refuses. `earliestCreated` checks creation freshness independently. The verifier checks its own request window in seconds. [Validity and freshness checks](../zk/noir-query/core/src/envelope.nr) · [End-to-end cases](../js/zk-query/scripts/targets-live.mjs) |

The insurance and trial examples require all three DER-13 conditions. The recycled-content
examples bind issuer and status and omit holder binding. Keep that distinction in the
application gate. Test with a verified insurance answer, then run independent cases with the
fact under a different authorized issuer, holder binding omitted, and status optional or
missing allowed. Only a verified answer satisfying the required policy and issuer-scoped
query may count. Retain a valid recycled-content proof as a control for the holder condition.
[Target policies](../js/zk-query/scripts/solid-targets.mjs) ·
[Query compiler](https://github.com/sparq-org/sparq/pull/7385) ·
[Authenticated Node delivery](https://github.com/sparq-org/sparq/pull/7402)

The successful-result method has a narrower contract. Its scoped form pins **one issuer key
across every authenticated slot** and refuses distinct issuer IRIs. It checks credential
status using receiver-supplied versioned snapshots. Its reports have `subject: []` and
`complete: false`. A shared `trustMembers` roster can include unused members and its
verification-method IRIs are policy metadata. In the query method `report.issuers` is the
trusted key set and `report.subject` is the holder-policy scope. Neither report enumerates
which credential supplied each fact. Route DER-13 requests to the authenticated query method.
[Scoped-result tests](../js/zk-result/test/scope.test.js) ·
[Result contract](../js/zk-result/index.d.ts) ·
[Single-issuer scope](https://github.com/sparq-org/sparq/pull/7401) ·
[Shared roster](https://github.com/sparq-org/sparq/pull/7403)

These findings apply to the packages' typed APIs. The implementation companion describes an
earlier compiler stage, and the proposed OpenID4VP mapping selects bearer presentations.
JSON-LD exchange and binding the OpenID4VP presentation to its holder are separate transport
work. [Implementation snapshot](../site/specs/zksparql-answers-implementation.typ) ·
[Transport mapping](../site/specs/zksparql-answers-oid4vp.typ)

## Dataset agreement and repeated questions

**A5 · Signed completeness — partial.** A verifier can already store an `inputCommitment`.
The query proof covers all quads of the ordered signed credentials committed with the
presentation salt. Without an anchor the holder chooses the credentials. The insurance
count query can require a register's signed `five-year-complete` fact. There is no generic
signed agreement linking an authority's coverage claim to the chosen dataset.
[Commitment](../crates/sparq-zk-query/src/credential.rs) ·
[Agreement check](../crates/sparq-zk-query/src/driver.rs) ·
[Insurance count query](../crates/sparq-zk-query/tests/fixtures/solid_paths/insurance-count.rq)

Choose a detached, signed coverage credential from an authority the verifier accepts for
the specified registry, subject, claim types and coverage interval. The authority determines
the complete set within that scope and signs its method/profile, `inputCommitment`, scope,
as-of instant and expiry. Keeping this credential outside the selected dataset avoids a
self-referential commitment. Before issuing the request, the verifier checks the signature
and scope, and limits its request window to the attestation's validity. It stores the
credential and binds the SHA-256 digest of its RDFC-1.0 canonical signed claims, including
the authority IRI, alongside the exact anchor in the request. The public statement then
binds the coverage digest via the request digest and
the dataset via the existing commitment. Report the named authority and scope separately
from query exactness. Test a matching attestation and answer. Omitting a credential must
fail agreement even when the query result stays equal. Vary the authority, scope and expiry
independently. Each mismatch must preclude a coverage claim.
An ordinary unanchored request can still yield an exact answer over the presented roots.

**A6 · Repeated queries — partial.** The Answers draft describes cumulative disclosure.
Both APIs expose verifier-side nonce handling. Neither prover accepts a holder consent or
query-history policy. A second request with a fresh challenge is a separate release.
[Draft privacy guidance](../site/specs/zksparql-answers.typ) ·
[Query API](../js/zk-query/index.d.ts) · [Result API](../js/zk-result/index.d.ts)

Put the decision in the holder service shared by both proving paths. For a consent grant,
record the data scope, allowed query templates and release limit. Reserve a release
atomically in durable history before proving. Account across verifier identities for the
same grant, and require renewed consent outside the grant. Record the completed response
before release so a retry can return it. Refuse if the history store is unavailable.
The holder can observe past releases locally, so the public
statement needs no history field or stable holder identifier. Test related queries with
distinct nonces and audiences against a one-release grant, including concurrent requests
and a service restart. At most one distinct answer may be released.

## Merkle cryptosuites

**M1 · Date and derived-integer keys — implemented in the extended profile.**
Schnorr/Poseidon2 and the opt-in native Ed25519/SHA-256 successor give timezone-free
`xsd:date` with years from `0001` through `9999` class `0x08` and representable,
range-checked integer subtypes the decimal class `0x01`. A valid zoned date is
unrepresentable (`0x7F`), and an ill-typed literal has no key (`0x00`). Frozen
Ed25519/SHA-256 gives dates and integer subtypes the all-zero key.
The successful-result package's bundled date and unsigned-integer filters use a separate
credential format. [Key computation](../crates/sparq-merkle-suite/src/keys.rs) ·
[Frozen dispatch](../crates/sparq-merkle-suite/src/sha256.rs) ·
[Filter members](https://github.com/sparq-org/sparq/pull/7397)

The authenticated Noir query method uses the field-native profile. The native SHA
successor commits the comparison key alongside the RDF term identity, and its
suite identifier selects the key and tree rules. The tests pin leap days, timezone
forms, subtype limits, lexical aliases and frozen SHA credentials.
[Successor tests](../crates/sparq-merkle-suite/tests/sha_salted.rs) ·
[Frozen vectors](../crates/sparq-merkle-suite/tests/sha_parity.rs)

**M2 · Leaf salts — implemented in the salted members.** Poseidon2 and the SHA successor
order private quad digests and derive a private leaf salt for each index from the issuance
salt. The SHA successor derives its leaf salts with HMAC-SHA-256 and binds each salt and
quad digest in a domain-separated leaf. The original SHA edition uses unsalted leaves and
root and salts its signed message. Issuing the same quads under different salts changes
the successor's leaves and root. The presentation salt for dataset agreement is separate.
The successor is available for native issuance and whole-credential verification. Guest
adoption is tracked by the [frozen-source consolidation](https://github.com/sparq-org/sparq/issues/7391).
[Poseidon2 tree](../crates/sparq-merkle-suite/src/poseidon.rs) ·
[SHA successor tree](../crates/sparq-merkle-suite/src/sha256_salted.rs) ·
[Original SHA tree](../crates/sparq-merkle-suite/src/sha256.rs)

**M3 · Implemented members — partial.** Ed25519/SHA-256 has native issuance and verification
and a zkVM relation. Schnorr/Poseidon2 has native issuance and verification and a Noir query
opener. ML-DSA-44/SHA-256 is specified and missing from the suite implementation.
[Suite exports](../crates/sparq-merkle-suite/src/lib.rs) ·
[Native round trips](../crates/sparq-merkle-suite/tests/suites.rs) ·
[Guest parity](../crates/sparq-merkle-suite/tests/sha_parity.rs) ·
[Suite definitions](../site/specs/zk-merkle-cryptosuite.typ)

Implement the specified ML-DSA member as an opt-in native issuer and verifier first.
Reuse the original SHA tree and implement the member's own identifier in the signed
message, using pure ML-DSA-44 with empty context. This can be checked against an independent
implementation before introducing any proof-backend dependency. No query method gains an
accepted suite or changes its public statement at this stage. Test published signature
vectors, an independently computed credential message, a native round trip and refusal
under the wrong suite, key or document. Query admission requires a separately tested
method that binds this member and checks its signature in the advertised mode.
