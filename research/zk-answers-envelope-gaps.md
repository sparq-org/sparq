# Credential evidence for Solid query answers

Baseline at `e626490` on `main`, with the signed coverage implementation recorded below.
The authenticated Noir query method already implements
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

**A5 · Signed completeness — implemented in the authenticated query method.** A verifier
can attach a detached coverage credential to an agreed `inputCommitment`. The authority
attests coverage for the registry, subject, claim types and historical interval. The
request builder checks the selected authority and its signature, the signed scope and
collection method, the dataset commitment and the request window. It keeps the coverage
credential with the request, outside the query input.

The proof binds the SHA-256 digest of the canonical signed coverage claims via the
request digest, and binds the ordered signed source credentials via the existing
dataset commitment. Verification reports the authority and scope alongside the
basis for query exactness. Unanchored requests continue to prove exact answers over
the presented roots.
[Coverage validation](../crates/sparq-zk-query/src/coverage.rs) ·
[Request binding](../crates/sparq-zk-query/src/request.rs) ·
[Usage](../skills/zk-query-javascript/SKILL.md) ·
[Agreement tests](../js/zk-query/test/coverage.test.js)

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

**M1 · Date and derived-integer keys — partial across members.** Schnorr/Poseidon2 gives
representable timezone-free `xsd:date` class `0x08` and range-checked integer subtypes the
decimal class `0x01`. A valid zoned date is unrepresentable (`0x7F`), an ill-typed literal
has no key (`0x00`). Frozen Ed25519/SHA-256 gives dates and integer subtypes the all-zero key.
The successful-result package's bundled date and unsigned-integer filters use a separate
credential format. [Key computation](../crates/sparq-merkle-suite/src/keys.rs) ·
[Frozen dispatch](../crates/sparq-merkle-suite/src/sha256.rs) ·
[Filter members](https://github.com/sparq-org/sparq/pull/7397)

Use the field-native comparison profile for Solid today. For SHA support, introduce a
separately identified successor using the same extended key rules. Key bytes enter signed
leaves, so a verifier must select the edition through the accepted suite identifier.
Test both editions on leap days, zoned and invalid dates, integer subtype bounds and lexical
aliases. Check class and value order independently, retain RDF term identity for joins,
and verify existing SHA credentials against the frozen vectors.

**M2 · Leaf salts — partial across members.** Poseidon2 orders private quad hashes, derives
each leaf salt from the private issuance salt and sorted index, then hashes the quad hash
with that salt. SHA-256 uses unsalted leaves and root, and salts the signed message. The
presentation salt for dataset agreement is separate. [Poseidon2 tree](../crates/sparq-merkle-suite/src/poseidon.rs) ·
[SHA tree and message](../crates/sparq-merkle-suite/src/sha256.rs)

Use the same new SHA edition as M1 to introduce leaf salting in one format transition.
Order by unsalted quad hash, derive each private leaf salt with HMAC-SHA-256 keyed by the
issuance salt over a suite-specific domain and fixed-width index, then bind it and the quad
hash in a domain-separated leaf. The new suite identifier selects the signed-message and
opening rules. No salt becomes a public query input. Test the same quads issued under two
salts. Both leaves and root must differ for the successor, while the frozen edition has
equal roots and different messages. Pin issuer/verifier vectors and reject cross-edition
verification. Coordinate guest adoption with the [frozen-source consolidation](https://github.com/sparq-org/sparq/issues/7391).

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
