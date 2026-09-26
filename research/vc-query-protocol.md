<!-- [OPUS-5.5] Experimental Sparq protocol design for review, authored by Claude Opus 5.5. No implementation lands with this record. -->
# Verifiable-credential query proofs with pluggable proof methods (vcq draft 0)

> **Status: experimental Sparq protocol research, for review.** A proposed protocol shape, not
> a W3C or other standard, an implementation claim or an interoperability profile. Nothing it
> names has had an external cryptographic audit (sq-qhy4), and an accepted presentation is
> research evidence for that method's stated relation only.
> No method below is claimed to be sound or to keep any value hidden from an adversarial party. <!-- privacy-claims-allow: negated claim; asserts no soundness or privacy property; external audit sq-qhy4 open -->

The companion [`vc-query-methods.json`](vc-query-methods.json) is a bounded proposed registry
of suites, mapping profiles, methods, linking profiles, allowed combinations and conformance
vectors. A registry label is a name, never evidence that anything it names is supported.
[OPUS-5.5] Only `method:risc0-exact` version 3 has a vcq adapter (`adapter_available: true` for
exactly six tuples, §6.1, §9); every other entry and version keeps `adapter_available: false`.

## 1. Conventions and snapshot

- **Proposed requirements** use MUST/SHOULD/MAY and apply only to this Sparq draft ("vcq
  draft 0"). **Current evidence** is confined to §9 and to cells explicitly marked current.
- `code` names in *Existing API* columns exist in source. `urn:sparq:vcq:…` identifiers and
  italic interface names (*QueryMethod*, *VerifiedClaim*) are renameable proposals.
- Snapshot: `codex/zk-vc-query-spec` at `3b7a06c0c`. Native RDF integration from PRs 6504 and
  6577 is absent and not described; the native prototype read here may be older. `sparq-zk`
  import APIs are cited from the [zk-query-proofs skill](../skills/zk-query-proofs/SKILL.md), not re-read.
- Every serialization shown is **illustrative**. Interoperability requires the wire profile
  of §7.2; until one exists, no cross-implementation claim is made.

## 2. Relationship to published specifications (informative, read 2026-09-26)

[VC Data Model 2.0](https://www.w3.org/TR/2025/REC-vc-data-model-2.0-20250515/) separates
issuer, holder and verifier roles and leaves securing mechanisms to other specifications. A
verified credential authenticates that its issuer made the contained assertions. It does not
establish that they are true or that the presenter is the subject. This draft keeps that
split: suites (§4) authenticate assertions, query methods (§6) compute over them, and status
and holder binding remain separate obligations (§4.3).

[VC Data Integrity](https://www.w3.org/TR/vc-data-integrity/) lets a named cryptosuite define
its transformation, hashing and proof operations, carried by a `DataIntegrityProof` with
`cryptosuite` and `proofValue`. vcq borrows the extension pattern, an identifier that pins
exact algorithms, for query methods. Data Integrity does not define SPARQL evaluation proofs,
and vcq methods are not cryptosuites.

[Controlled Identifiers 1.0](https://www.w3.org/TR/cid/#verification-relationships) has the
verifier confirm that a verification method is authorized for the intended verification
relationship, which for asserting VC claims is `assertionMethod`. A signature that verifies
under some key is therefore not yet an issuer assertion (§4.2 rule 9).

[Data Integrity EdDSA](https://www.w3.org/TR/vc-di-eddsa/) computes the signed hash over the
canonical proof configuration together with the transformed document, so proof options such
as the verification method and proof purpose are authenticated with the credential (§4.1).

[VC JOSE/COSE](https://www.w3.org/TR/vc-jose-cose/) secures credentials with JOSE or COSE,
whose signatures cover payload bytes rather than an RDF canonical form. A query over such a
credential must bind those exact authenticated bytes and a pinned mapping from them (§4.2).

[OpenID4VP](https://openid.net/specs/openid-4-verifiable-presentations-1_0.html) DCQL lets a
verifier request credential presentations and claims. That is a transport and selection
layer; it does not prove the result of an arbitrary SPARQL computation. A deployment could
carry a vcq request beside such an exchange. No OpenID4VP compliance is claimed.

## 3. Layers, obligations and roles

| Layer | Question it answers | Identified by |
|---|---|---|
| Authenticity suite | Did issuer key K sign representation bytes B? | suite id and version |
| Mapping profile | Which RDF terms do the authenticated bytes B denote? | mapping id and pinned resources |
| Status and freshness | Is the credential unrevoked at a status version the verifier accepts? | status policy |
| Holder binding | Does the presenter control a key the issuer bound to this credential? | holder policy |
| Query method | Does the released result meet the result contract for query Q over scope D? | method descriptor (§6.1) |
| Linking profile | Are the hidden values used by the layers above the *same* values? | linking id (§8) |

The request states which obligations it requires (§5.1): source evidence (§5.3), a status
policy (`required` with a version window, or `not-requested`) and a holder policy
(`required` or `bearer-accepted`). For each credential *c* in scope a presentation owes
mapping M(c) and each *required* obligation among authenticity A(c), status T(c) and holder
binding H(c); the request owes one query obligation Q. Every hidden value shared between two
obligations is an edge that needs a named linking mechanism. An obligation the request does
not require is reported as `not-established`, never as positive evidence.

Each owed obligation and edge MUST name an **enforcer** (§7.3): a relation that checks it
over the hidden witness, or a host check whose hidden inputs reach it through an
authenticated opening or link. A required obligation or edge with no enforcer is
`unsupported`; it is never skipped, and a claim that omits a required check is `invalid`.
The verifier owns the request, trust policy, method and artifact pins and the challenge
store; the holder owns credentials and witnesses. An importer that verifies one suite and
re-signs under another is a re-issuer whose key the verifier must trust in its own right.

## 4. Credential import, verification and mapping

### 4.1 Signed representations

| Class | Authenticated input | Current status (§9) |
|---|---|---|
| `rdf-canonical` | suite hash data over RDFC-1.0 canonical proof configuration and document (Data Integrity `*-rdfc-*` suites) | import-time verification only |
| `sparq-commitment` | Poseidon2 commitment over RDFC-canonical triples, salt and status reference | used inside Noir relations |
| `json-jcs` | suite hash data over RFC 8785 JCS proof configuration and document | planned, not implemented |
| `jose-payload` | JWS or COSE signing structure: protected header plus payload octets | planned, not implemented |

The authenticated representation is the whole suite-prescribed signing input, including the
proof configuration or protected header and the suite's hash construction, not the document
bytes alone. Canonicalization and verification MUST NOT drop or substitute those fields.
Mapping (§4.2) reads only the authenticated credential payload; proof-configuration and
protected-header fields feed the authorization check of §4.2 rule 9, not the dataset.

### 4.2 Import contract (proposed)

*Import*(bytes, suite, mapping, pinned resources, trust policy) yields an *ImportedCredential*
(dataset, source digest, suite and issuer key identity, status references, holder-binding
material, blank-node scope id) or a typed failure (§6.4).

1. The suite MUST verify over the exact authenticated input before mapping. The mapping
   consumes those bytes, never a re-serialization of a parsed object.
2. The mapping MUST be a deterministic function of (authenticated bytes, mapping id and
   version, pinned resources). Its identity and resource digests enter the statement (§7).
3. JSON-LD contexts MUST come from a verifier-pinned map from context IRI to content digest,
   with no network retrieval. A known IRI with different content is rejected (*context
   substitution*). The profile fixes the expansion base IRI; relative IRIs whose meaning
   depends on retrieval location are rejected.
4. Bytes outside the authenticated input (JOSE unprotected headers, transport wrappers,
   detached metadata) MUST NOT enter the dataset. A JCS signature covers `@context` IRIs, not
   their content, so rule 3 is mandatory for `json-jcs`.
5. Blank nodes are scoped per imported credential. Equal labels in different credentials are
   different terms, and no method may join across scopes on labels.
6. Term identity is exact: IRI characters; literal lexical form, datatype IRI, language tag
   and, where the dialect admits it, base direction. Mapping never rewrites lexical forms
   (`"018"` stays `"018"`), so `sameTerm`, DISTINCT and released terms use the signed form.
   Value comparison (`=`, `<`) follows SPARQL operator semantics, under which
   `"018"^^xsd:integer = 18` holds. A method that compares values MUST bind each value to
   its signed lexical form; a signed form outside its value domain is `unsupported` at
   `prepare`, never `unsatisfiable`.
7. Each quad keeps its source (credential scope, suite, issuer key identity, source digest).
   The request's disclosure policy states which provenance may become public. A method MUST
   NOT publish more, for example per-row credential attribution.
8. *Dataset assembly* is named by the request: `union-default-graph` (all triples in the
   default graph, blank nodes kept apart) or `credential-named-graphs` (one named graph per
   credential under an opaque per-presentation name). Credential-internal named graphs are
   rejected in draft 0. Only the first form resembles current Noir behaviour (§9).
9. *Issuer key authorization.* A signature that verifies under a supplied key shows only that
   the key signed. `issuer-authenticated` (§5.3) additionally MUST resolve, from
   verifier-authenticated or verifier-pinned trust material, an authorization binding
   (issuer or controller identity, key identity, exact suite and curve or algorithm, and an
   explicit authorization of that key to make issuer assertions), and MUST check it through
   the suite's authorization profile. A profile checks only fields its suite defines; none
   requires envelope fields the suite lacks.
   - *Data Integrity envelope*: the relationship is `assertionMethod`; the signed issuer and
     the signed proof configuration (`verificationMethod`, `proofPurpose`) MUST name the
     binding.
   - *JOSE or COSE envelope*: the protected header's key identifier and algorithm MUST match
     the pinned key and suite, and the authenticated issuer claim (the credential `issuer`,
     and `iss` when present) MUST name the pinned issuer.
   - *Direct pinned key* (a custom suite with no envelope, such as current
     `sparq-poseidon2-schnorr-v1` commitment signatures): the verifier's policy names the
     pinned issuer and key and authorizes it for assertion; the signature MUST verify under
     that key, and a signed issuer claim, where the suite carries one, MUST name that issuer.

   In every profile the checked binding covers the verified credential bytes, and key
   authorization comes only from verifier trust material, never from holder or presentation
   metadata. When the request requires it, the credential's own validity constraints
   (`validFrom`, `validUntil`) are checked from the same authenticated bytes. A bare
   caller-supplied key, or a key found through a presentation-chosen locator
   (`verificationMethod`, `jku`, `x5u`), is not issuer authentication. Import performs
   no ambient retrieval (no DID or HTTP resolution). If the authorization cannot be checked
   from pinned material, import is `unsupported`; a wrong issuer, key, suite, curve,
   algorithm or purpose is `policy-rejected`. Current `vc_bridge_json` returns
   `verification_method` but leaves binding it to the caller, so current import suites are
   import primitives, not end-to-end issuer authentication (§9). No vcq adapter exists for
   any profile; the one vcq adapter (§9) requires source evidence `none` and authenticates
   nothing.

### 4.3 Status, freshness and holder binding

- **Status** is an issuer-signed (list, index, version) reference evaluated against
  verifier-authenticated snapshots inside a verifier-chosen version window, never against a
  prover's copy. Evidence from a version outside the window is a *status rollback*.
- **Holder binding** is an issuer-attested holder key (or digest) plus proof of possession
  over the request challenge. A subject IRI equal to a presenter identifier is not holder
  binding.
- Skipping either check is an explicit request value (`not-requested`, `bearer-accepted`);
  the claim then reports it `not-established`. Neither is query semantics: status and binding
  facts are not RDF triples visible to Q unless a declared mapping adds them, and no query
  result implies them.

## 5. Verifier-owned query request

### 5.1 Fields

| Field | Content |
|---|---|
| `protocol`, `challenge` | `vcq-draft-0`; 32 verifier-random bytes, single use |
| `audience`, `not_before`, `not_after` | verifier domain and validity window |
| `query`, `base_iri` | exact UTF-8 query bytes, never normalized; an explicit base IRI or explicit none |
| `dialect`, `query_profile` | pinned syntax, semantics and engine snapshot; admitted fragment id |
| `form`, `result_contract`, `mode` | query form; result semantics (§5.2); `selected-support` or `exact-bounded` |
| `scope` | authority and anchor, source evidence (§5.3), dataset assembly (§4.2 rule 8), completeness |
| `evaluation_context` | entailment regime (simple only), dataset-clause policy, DESCRIBE and canonicalization policy, numeric and temporal profile; volatile functions rejected |
| `methods` | exact acceptable descriptors (§6.1), preference-ordered, no wildcards |
| `trust` | issuer set, accepted suites and mappings, status policy, holder policy, linking profiles, disclosure policy |
| `resources` | verifier bounds on encoded presentation bytes (checked before decoding) and on released rows and graph size (semantic bounds on the result, §6.2) |

1. The verifier constructs and stores the request; a presentation never supplies or
   overrides a field. A field no proof binds (§7.3) is enforced from that stored copy.
2. `scope.source_evidence`, `trust.status` and `trust.holder` have no defaults; a request
   omitting one is `invalid` at request validation.
3. `query` MUST parse as SELECT, ASK, CONSTRUCT or DESCRIBE under `dialect`. SPARQL Update is
   `unsupported` at request validation and is never evaluated.
4. A presentation naming a descriptor outside `methods` is `invalid`; a `methods` entry that
   is not implemented is `unsupported`; there is no fallback. Requests outside their
   validity window reject before any proof check.
5. A weaker result never satisfies a stronger request: selected support never satisfies
   exact evaluation; `holder-declared` never satisfies `verifier-agreed-anchor`; source
   evidence `none` never satisfies an authenticated request, and `re-attested` never
   satisfies `issuer-authenticated`; a distinct set never satisfies a bag.

### 5.2 Result contracts

| Form and semantics | `selected-support` | `exact-bounded` |
|---|---|---|
| SELECT distinct set | each released mapping is a solution; completeness `none`; empty release rejected | equals the full DISTINCT result |
| SELECT bag | unsupported: multiplicity is a completeness fact | multiset; duplicates kept; unbound is distinct from every term |
| SELECT sequence | unsupported | outer order under the dialect's tie policy, LIMIT/OFFSET applied |
| ASK | `true` only | `true` or `false` |
| CONSTRUCT | unsupported in draft 0 | RDFC-1.0 canonical graph; fresh blank nodes per solution |
| DESCRIBE | unsupported in draft 0 | only with an explicit DESCRIBE policy; SPARQL defines no unique graph |

Released blank nodes use result-local canonical labels, never source labels; a method that
cannot produce them rejects projected blank nodes. Exact evaluation that exhausts a declared
bound yields `capacity`, never a truncated result.

### 5.3 Scope: authority and source evidence

Scope has two orthogonal axes. **Authority** records who chose the dataset:

- `verifier-agreed-anchor`: the verifier accepted a dataset commitment before issuing the
  request. Exactness is relative to that anchored dataset.
- `holder-declared`: the holder chooses the inputs at presentation time, and the claim
  carries holder-declared provenance.

**Source evidence** records what authenticates the inputs: `none`; `issuer-authenticated` (a
trusted issuer's suite is verified over each credential under a key authorized per §4.2
rule 9 and linked to the evaluated values, §8); or `re-attested` (a trusted importer's signature; the claim names the importer, not the
source issuer). Either authority may carry any source evidence. A holder-chosen subset of
issuer-authenticated credentials remains `holder-declared`: authentication never upgrades
authority. An anchor over authenticated credentials is expressible, but the authentication
must be linked to the anchored bytes, not checked beside them; no current method offers it
(§9). Completeness, status policy and holder policy are separate fields implied by neither axis.

No scope establishes whole-wallet completeness. A holder commitment to "all my credentials"
is holder-declared however it is computed. Absence claims (false ASK, an empty result, NOT
EXISTS, MINUS) exist only under `exact-bounded`, and only relative to the stated scope.

## 6. Query method contract

### 6.1 Descriptor, capabilities and version pinning

A descriptor is the tuple (method id, method version, parameter set, parameter digest,
artifact identity, backend pin). Artifact identity is a verification-key digest for a
circuit, an (artifact digest, image id) pair for a zkVM guest, or circuit and setup digests
for a composite proof. Verifiers load artifacts only from their own configuration, never
from a presentation, URL or dynamically loaded code, and never replace an unavailable or
failing descriptor with another version or parameter set.

Capabilities include an `enforces` map from each obligation the method can discharge to its
enforcer (§7.3), the challenge owner and policy (§6.4), and `adapter_available`. A registry
`status` such as `implemented-experimental` says only that component source exists; a
consumer MUST NOT register a method as executable through vcq unless `adapter_available` is
true. [OPUS-5.5] In the registry it is true only for `method:risc0-exact` version 3, and only
for the six tuples its `vcq_adapter` lists (§9); it is false for every other entry and version.

### 6.2 Operations (proposed interface *QueryMethod*)

| Operation | Public inputs | Private inputs | Output | Failure classes |
|---|---|---|---|---|
| `descriptor` | none | none | descriptor | none |
| `capabilities` | none | none | forms, semantics, modes, authorities, source evidence, `enforces`, suites, linking profiles, query profiles, resource ceilings, challenge owner, disclosure inventory, `adapter_available` | none |
| `admit` | request, descriptor | none | *Admission*: derived public layout, obligations with enforcers, resource shape | unsupported, capacity |
| `prepare` | request, admission | credentials, witnesses | opaque *PreparedWitness* | unsupported, unsatisfiable, capacity, policy-rejected, infrastructure |
| `prove` | none | *PreparedWitness*, consumed | presentation: descriptor reference, result, proof bytes | capacity, infrastructure |
| `verify` | stored request, recomputed admission, presentation, challenge store | none | *VerifiedClaim* | invalid, unsupported, capacity, infrastructure |

`admit` is deterministic over public data and sees no result. The verifier recomputes it
and never uses a holder's copy. `admit` returns `unsupported` when a required obligation has
no enforcer and `capacity` when a public request bound (such as `resources` released rows)
exceeds a method ceiling. [OPUS-5.5] `verify` MUST check two distinct bounds. The encoded
presentation byte size is checked before anything is decoded. The released-row capacity is
a semantic bound on the result. When rows are read only from the proof's result (§9.1), it is
checked after the result is decoded and the proof is verified, and BEFORE the challenge is
atomically consumed. A method whose presentation carries rows outside the proof MAY also
count them before decoding. Either excess is `capacity` at verify and never yields a claim.

### 6.3 Verified claims and witness lifetime

`verify` returns a typed *VerifiedClaim*: statement digest; descriptor; mode; result contract
and typed result; scope (authority, anchor, source evidence, completeness `none` or
`relative-to-scope`); per-slot authenticity (suite, issuer key or public slot, enforcer and
linking route from §8, or `not-established`); status (enforcer, accepted window, policy root,
or `not-established`); holder binding (clear key, hidden key, or `not-established`); the
linking profile and edges discharged; and the disclosure inventory. The protocol verifier
recomputes the statement digest from its stored request and the claimed result and compares
every field with the request. Any difference, or a required obligation reported
`not-established`, is `invalid`. An adapter that can only report that a proof verified MUST
NOT be registered.

*PreparedWitness* is method-owned and opaque. It has no clone, debug formatting or
serialization, is consumed by `prove`, and its scratch files belong to the method and are
removed after success or error. The protocol layer never inspects or logs it, and planning
diagnostics never enter presentations.

### 6.4 Failure classes and challenge consumption

| Class | Meaning |
|---|---|
| `unsupported` | outside declared capabilities: form, feature, unknown or unimplemented method, unlisted combination, unenforced obligation, Update; decided from public inputs, except a hidden value outside the method's value domain, found at `prepare` |
| `capacity` | admissible but beyond a declared bound; names the bound (search budget, encoded presentation bytes, released rows, numeric range, circuit bucket) |
| `unsatisfiable` | prover side only: under the query's SPARQL semantics no witness supports the requested or released result |
| `policy-rejected` | trust policy excludes an input: untrusted issuer, status outside the window, missing holder binding |
| `invalid` | a request or presentation fails a check: missing request policy, statement or result mismatch, proof, replay, expiry, audience, oversize or malformed encoding |
| `infrastructure` | environment failure: missing or wrong toolchain, I/O, challenge-store failure |

Every outcome other than a *VerifiedClaim* rejects. Each failure carries (class, phase, code).
Classes come from typed errors or an unambiguous call phase, never from message text. An
ambiguous error is reported as the least specific applicable class and cannot certify
`capacity`. `infrastructure` never maps to acceptance or to `invalid`.

Each descriptor names exactly one challenge **owner** (`method` or `adapter`), which performs
at most one atomic check-and-consume per verification attempt. Under `burn-on-attempt` the
consume precedes proof checking, so a failed attempt also spends the challenge. Under
`consume-on-success` it follows every other check, so a failed attempt leaves the challenge
unconsumed and retryable until expiry; descriptors MUST declare this. Under either policy,
of concurrent verifications of valid presentations over one challenge at most one succeeds
and the rest are `invalid` (replay). The protocol default, `burn-on-attempt` with the adapter
as owner, applies only to methods that consume nothing; when the wrapped method owns the
challenge, the adapter MUST NOT consume it again.

## 7. Statement construction and encoding

### 7.1 Abstract statement

The statement is a typed record: protocol id, wire-profile id, request digest, descriptor
digest, mode, result contract, result, scope descriptor, trust-policy digest, linking-profile
id, challenge, audience and validity window. The verifier builds it from its stored request
plus the presentation's result; nothing else in a presentation contributes.

### 7.2 Wire-profile obligations

This draft defines structure, not bytes. A wire profile MUST specify: an injective, typed,
length-prefixed encoding; a domain-separation tag per object (`sparq:vcq:<object>:<profile>`
followed by a zero byte); RDF terms as a kind tag plus separate lexical, datatype, language
and direction fields; distinct sets sorted by encoded row, bags sorted with every duplicate,
sequences with explicit indices, unbound as its own tag, and graphs as RDFC-1.0 canonical
N-Quads; fixed-width integers; no floating point, locale or Unicode normalization; and the
hash function. Changing any of these requires a new profile id. `JSON.stringify` output, a
struct serializer's field order and pretty-printed JSON are not canonical encodings, and
unsigned presentation metadata is never authoritative.

### 7.3 Binding routes and enforcers

A descriptor declares how each statement field reaches the proof: `proved` (a public input),
`journaled` (inside a proved output), `derived` (the verifier selects key or program from
it), `challenge` (through the request-derived challenge below) or `host` (checked only
against the stored request). Result, challenge, descriptor, scope anchor and any policy value
that shapes public inputs MUST use one of the first four routes.

**Binding is not enforcement.** A route makes a field part of the proved statement; it checks
no predicate on a hidden witness. Each owed obligation (§3) therefore also names an enforcer:
`relation:<id>`, checked inside the named relation over the witness, or `host:<check>` over
public values, where every hidden value the check depends on reaches it through an
authenticated opening or linking edge (§8). Audience, validity and policy roots are public
values that a host check compares with the stored request. That a hidden credential was
signed by a key in the issuer set, is unrevoked within the window or maps under the pinned
profile is enforceable only by a relation or through an opening. Hashing a policy into the
challenge or request digest never makes an unimplemented issuer, status or mapping
constraint supported; such a request is `unsupported` at `admit`.

Request-derived challenge: `c = H_field("sparq:vcq:challenge:draft-0" || 0x00 ||
request_digest || verifier_random)`. A method that proves over `c` binds the request's
identity, provided the verifier recomputes `c` from its own stored request and consumes it
once. This lets existing single-challenge methods bind audience, validity, method list and
trust-policy digest into the statement without circuit changes. It enforces none of them
over the witness and does not bind the result; the method must do that.

Confusion, replay and substitution: the descriptor digest is inside the request digest and
selects the verification key or program, and each method carries its version in public
inputs or journal. Result substitution fails because the claim uses only the proved result.
Replay fails on the single-use challenge and validity window. A new method version uses new
domain tags, so an old proof cannot meet a new request even with identical released results.

## 8. Composition and linking profiles

| Mechanism | What it establishes | Current status (§9) |
|---|---|---|
| `same-relation` | one relation checks signature, membership, status and query over one witness | implemented (Noir selected support) |
| `native-witness-equality` | one composite proof equates hidden messages and circuit inputs in one scalar field | standalone prototype, not RDF |
| `trusted-reattestation` | an importer verifies a source suite and re-signs a sparq commitment; only the re-signing key is checked in-proof | import seam only |
| `committed-dataset-evaluation` | the proof evaluates exactly the bytes whose commitment it journals | implemented (exact evaluator) |
| `public-authenticated-value` | a released value, authenticated in-relation, is checked by the verifier | implemented for public FILTERs |
| `cross-field-bridge` | equality between BLS12-381 and BN254 witnesses | not implemented; fails closed |

`committed-dataset-evaluation` links a commitment to its evaluation whoever endorsed that
commitment. Whether it equals a verifier anchor is an authority check (§5.3), and it
authenticates nothing: a holder-declared commitment stays holder-declared with source
evidence `none`.

A shared nonce, equal displayed results, equal labels or equal subject IRIs across
separately verified proofs are not linkage: each proof can verify over different hidden
values. A linking profile names each edge's endpoints (credential message index, circuit
input or committed leaf) and the term-encoding profile on both sides; identical encodings
are part of the link, as the fail-closed `(commitment method, circuit)` matrix in
`sparq_zk_compose::dispatch::resolve_circuit` already illustrates for one axis.

Computation may move out of a circuit when its inputs are public in the result or query,
provided their authentication and membership obligations stay in-relation and no additional
provenance becomes public. Only (suite, mapping, method, linking, authority, source
evidence) tuples listed in the registry's `combinations` are valid; every other tuple is
`unsupported`, not a cross product.

## 9. Current implementation evidence (snapshot, read from source, not re-executed)

| Proposed id | Existing API | Contract | Authenticity, status, holder | Main limits |
|---|---|---|---|---|
| `noir-selected-support-unsigned` v1, v2 | `sparq_zk_compose::result::{prepare_result_with_options, PreparedResult::prove, verify_result}` (`successful-results` feature) | selected support; SELECT DISTINCT; nonempty positive BGP with canonical `u64` integer FILTERs | Schnorr over commitment, salt and status reference plus status against the verifier's accepted root, in one circuit; no holder binding | 1 to 2 credentials of 16 triples, 3 patterns, 4 rows; string-canonical commitments only |
| `noir-selected-support-signed` v3 | `result::signed::{prepare_signed_result_with_options, verify_signed_result}` | as above with canonical signed `i64` comparisons | as above | separate version and members; fixed signed capacity |
| `risc0-exact` v1, v2, v3 | `sparq_proved_evaluator::{prove_with_artifact, verify_with_artifact}` and the `v2`, `v3` modules | exact bounded; V1 default graph SELECT/ASK; V2 named-graph catalog; V3 adds CONSTRUCT/DESCRIBE and blank-node identity | none: `DatasetAuthority::{VerifierAgreed, HolderDeclared}` becomes `Provenance`; no signature, status or holder check | commits exact source bytes, not an RDF canonical form; upstream privacy-argument caveat |
| native prototype | executable in `zk/native-composition`, no library API | fixed relation `income - 12 * rent >= threshold` over two tuples | Dock BBS+ on BLS12-381 with `EqualWitnesses`; no status or validity | not RDF, not W3C `bbs-2023`; `noir-bn254` rejected at backend selection |
| legacy manifest | `verifier::verify_manifest` over `manifest::ProofManifest` | per-graph scan, FILTER and JOIN members | Schnorr attestation, revocation, holder proof-of-possession tiers | existing path outside this protocol; not mapped here |
| import suites | `sparq_zk::vc_bridge`, `vc_bridge_json`, `vc_bridge_sd` (skill-documented, `vc-bridge` feature) | none (import only) | `eddsa-rdfc-2022` and `ecdsa-rdfc-2019` verified off-circuit, then re-committed; `bbs-2023`, `ecdsa-sd-2023` delegated and unavailable by default | JSON-LD only via a caller-supplied context allowlist; verification under caller-supplied key bytes, with `verification_method` returned for the caller to bind (no §4.2 rule 9 check); JCS and JOSE modes not implemented |
<!-- privacy-claims-allow: the table cell cites an upstream privacy-argument caveat recorded in references/exact-evaluator.md; it asserts no privacy property -->

| Statement field | Noir selected support (current) | RISC Zero exact (current) | Native prototype (current) |
|---|---|---|---|
| result | proved: encoded released terms and count | journaled | proved predicate only |
| query | proved derived layout; exact string checked on host | journaled inside the request digest | none: fixed relation |
| challenge | proved; must equal the verifier nonce | journaled | Dock proof nonce |
| version and parameters | version proved; member derived from issuer slots, private filters, integer capacity and status depth | journaled request | verifier-built `ProofSpec` |
| program or key | derived: canonical key regenerated locally under the pinned nargo/bb toolchain | derived: `ArtifactPin { sha256, image_id }` via `AcceptedGuest` | local setup; source digests are provenance only |
| scope | `holder-declared`, source evidence `issuer-authenticated`: issuer slots and accepted status root proved | journaled commitment (`committed-dataset-evaluation`); host compares it with the anchor under `verifier-agreed-anchor`; source evidence `none` | verifier-supplied issuer keys and schema tags |
| audience, validity | unbound | unbound | fixed context bytes only |
| challenge consumption | owner method, `burn-on-attempt`: `SeenNonces::record_fresh` precedes the proof check | owner method, `consume-on-success`: `Nonces::consume` after journal binding; `Nonces` is a relying-party trait whose documented contract is atomic persistent acceptance, and the host supplies no production store | not applicable (single run) |

Gaps between this source and the draft, for the planned adapter work (items 1, 4 and 6 are
narrowed for exact V3 by the adapter described after this list):

1. At this snapshot no shared request, descriptor, capability or claim types existed.
   `verify_result` returns `VerifiedResult { query, rows }` and exact hosts return a
   `Journal` (which does carry `Provenance`); neither reports enforcers, source evidence and
   completeness as a typed claim. The later `sparq-query-protocol` crate supplies those types;
   only the exact V3 adapter uses them.
2. `ResultError::Rejected(String)` and the host `Error(&'static str)` conflate unsupported,
   capacity, policy, invalid and toolchain cases. Only `ResultError::{SearchExhausted,
   Driver}` and the model's `EvaluationError::{Budget, Capacity, Execution}` are distinct.
3. Exact `request_digest` hashes `serde_json` bytes of the Rust request under a versioned
   domain tag: deterministic for one build, not a wire profile.
4. Audience and validity are unbound on every path; §7.3's derived challenge is the proposed
   binding remedy, with enforcement by host checks. The Noir method binds the parsed query
   layout, not the query bytes.
5. Exact methods and the native prototype enforce no credential obligation, so a request
   requiring authenticity or status from them is `unsupported`. Import suites reach Noir
   methods only by trusted re-attestation; no end-to-end path from `vc_bridge` to
   `verify_result` was checked. `vc_bridge` implements no §4.2 rule 9 authorization: the
   caller supplies resolved key bytes and must itself check that `verification_method`
   names them, so it is an import primitive, not issuer authentication.
6. The planner admits true ASK but `verify_result` accepts only SELECT DISTINCT. Noir keys
   are regenerated locally, not pinned by digest. No cross-field bridge exists. Both
   existing methods consume their challenge internally, so their adapters own none. The
   exact host's single-success guarantee depends on the relying party's `Nonces`
   implementation (`zk/sparql-evaluator/host/src/lib.rs:42`), which an adapter must require.

### 9.1 Exact V3 vcq adapter (later source, independently certified run)

[OPUS-5.5] `sparq_proved_evaluator::vcq::Risc0ExactV3` (detached `zk/sparql-evaluator/host`,
feature `vcq`, off by default) implements *QueryMethod* over the V3 relation for descriptor
`urn:sparq:vcq:method:risc0-exact` version 3 only. It declares exactly six capability tuples:
`SelectBag`, `AskBoolean` and `GraphRdfc10` (CONSTRUCT only), each under
`VerifierAgreedAnchor` and `HolderDeclared`. Every tuple is `ExactBounded` over
`ExactSourceCatalog`, with source evidence `None`, status `NotRequested` and holder
`BearerAccepted`. Mapping, linking and query are enforced by the guest relation; the anchor
is a public host check owed only under verifier-agreed authority. Authenticity, status and
holder binding have no enforcer. For those tuples it closes gap 1 (typed claim) and gap 4
(audience and validity reach the journaled request digest through a derived nonce and are
checked on the host). For gap 6, the method owns the challenge and consumes the ORIGINAL
stored-request challenge once through the shared store, after every other check. V1, V2 and
the direct V3 host APIs are unchanged and have no adapter. The registry records the tuples
under `method:risc0-exact` `vcq_adapter`.

A genuine run at source `872c219ca18c6cc978d2f5c705f020e8c69748a6` was independently
certified; it is recorded here, not re-executed. It produced seven Succinct receipts, each
`Halted(0)`. Six were accepted by the protocol: SELECT bag, ASK and CONSTRUCT, each under
holder-declared and verifier-agreed authority. The seventh carried a valid V3 result of two
rows against a released-row bound of one. It was rejected as `capacity`
(`vcq-released-rows`) after the proof checks and before challenge consumption. The six
accepted cases carried 81 controls. The verify-only test failed against the consume-first
mutant, as intended. The root's wrapper around that check still reported overall failure,
because its expected failure text was multiline, so the wrapper's verdict is not a pass.
After the source was restored, a separate verify-only continuation and all-targets Clippy
passed with no new proof. Eight older genuine-proof test functions were excluded from the
native gate, so no full-gate claim is made. Evidence digests are SHA-256 of
`independently-verified-terminal-evidence.json`. The run's file hashes to
`d523b5405540c127c529749b5d0a571d308d348ed25d17ee27066c4457e9331b`. The post-restore
digest `5f8385d87f5d85dd49c043a31c1a28995218d95dcf679cdf6a8110989cb83eac` is of the file
with the same name from the separate post-restore run.

The run used a public synthetic fixture. It shows no issuer authentication of any
credential, no status, no holder identity, and no completeness beyond the exact agreed
bytes (no federation or nondeterministic dataset). The adapter is experimental and not
externally audited (sq-qhy4). [OPUS-5.5] The adapter applies the two §6.2 bounds
separately. It checks the encoded presentation bytes before decoding the receipt
(`vcq-presentation-bytes`). It checks the released rows, which it reads from the verified
journal, after the proof checks and before challenge consumption (`vcq-released-rows`).
SELECT rows and CONSTRUCT N-Triples lines count; ASK has no row bound. The proposed vector
`neg-excess-rows` (§11) now expects the same outcome as the seventh receipt. That vector
has not been executed.

## 10. Worked examples (illustrative notation, not a wire encoding)

Example A: selected support, modelled on the unit fixture in `result.rs` (Alice aged 42 and
Bob aged 12 in one credential). It assumes the §7.3 derived challenge; current source leaves
audience and validity unbound.

```text
request:
  protocol: vcq-draft-0   challenge: <32 random bytes>   audience: https://rp.example/age   validity: 10:00Z..10:05Z
  query: "SELECT DISTINCT ?name WHERE { ?p <urn:name> ?name . ?p <urn:age> ?age . FILTER(?age >= 18) }"
  base_iri: none            dialect: urn:sparq:vcq:dialect:sparq-positive-bgp:1
  form: SELECT              result_contract: select-distinct-set    mode: selected-support
  scope: {authority: holder-declared, source_evidence: issuer-authenticated,
          assembly: union-default-graph, completeness: none}
  methods: [{id: urn:sparq:vcq:method:noir-selected-support-unsigned, version: 2, artifact: {vk_digest: <pinned>},
             params: {credential_capacity: smallest, integer_capacity: hide-in-u64}}]
  trust: {issuers: [<pk1>], suites: [sparq-poseidon2-schnorr-v1], status: {required, urn:status:people, 7..7},
          holder: bearer-accepted, linking: [same-relation, public-authenticated-value]}
  resources: {released_rows: 4}
presentation: {method: noir-selected-support-unsigned@2, rows: [{name: "Alice"}], proof: <bytes>}
claim:
  statement: <digest recomputed by the verifier>   result: {{name: "Alice"}}   completeness: none
  scope: {authority: holder-declared, source_evidence: issuer-authenticated}
  authenticity: [{slot: <pk1>, suite: sparq-poseidon2-schnorr-v1, enforcer: relation, route: same-relation}]
  status: {enforcer: relation, window: 7..7, root: <policy root>}   holder_binding: not-established
  disclosed: query, released terms, issuer slot, capacity buckets, result size, policy root
```

Bob's absence is not a claim, and `not-established` holder binding is the requested bearer
policy, not evidence. Alice's age is not presented, but the released row and public bound
reveal that it is at least 18; the default `smallest` integer capacity would also reveal a
range bucket. <!-- privacy-claims-allow: states residual disclosure; asserts no privacy property -->

Example B: exact evaluation against a verifier-agreed anchor whose default graph holds
`<urn:alice> <urn:knows> <urn:bob>` and `<urn:alice> <urn:knows> <urn:carol>`.

```text
request:
  query: "SELECT ?p WHERE { ?p <urn:knows> ?q }"     result_contract: select-bag   mode: exact-bounded
  scope: {authority: verifier-agreed-anchor, anchor: <commitment>, source_evidence: none}
  methods: [{id: urn:sparq:vcq:method:risc0-exact, version: 1, params: <policy digest>,
             artifact: {sha256: <pin>, image_id: <pin>}}]
  trust: {suites: [], status: not-requested, holder: bearer-accepted, linking: [committed-dataset-evaluation]}
claim:
  result: bag [(p: <urn:alice>), (p: <urn:alice>)]   completeness: relative-to-scope
  scope: {authority: verifier-agreed-anchor, source_evidence: none}
  authenticity: not-established   status: not-established   holder_binding: not-established
```

Selected support cannot carry this result: the multiplicity asserts that no third solution
exists. `ASK { <urn:alice> <urn:knows> <urn:dave> }` under the same request shape returns
`false` relative to the anchor. A holder-declared journal fails that request, and the same
method under a holder-declared request uses the same linking profile without gaining authority.

## 11. Conformance vectors

Expected outcomes name a class and phase (§6.4); vectors that do not apply to a method are
`unsupported` at negotiation. Machine-readable copies are in the registry, where `applies`
names registry method, suite, mapping or linking entries. Every vector is an illustrative
proposal and none has been executed as a vector. [OPUS-5.5] The §9.1 run executed the exact V3
adapter's own test controls, which are not a vector run. The `neg-excess-rows` expectation
was aligned with that adapter's source and its seventh receipt, not executed as a vector.

| Vector | Setup | Expected |
|---|---|---|
| `pos-selected-distinct` | Example A | accept; completeness `none`; holder binding `not-established` |
| `pos-selected-public-filter` | FILTER on a projected integer | accept; verifier checks FILTER, membership stays in-relation |
| `pos-exact-bag-duplicates`, `pos-exact-ask-false` | Example B bag; its false ASK | accept; duplicates kept; `false` relative to scope |
| `pos-exact-graph-relabel` | CONSTRUCT over two source blank-node relabelings | accept; identical canonical graph |
| `sel-missing-row-undetected` | selected support omits a valid answer | accept; the claim implies no absence |
| `neg-wrong-method` | presentation descriptor absent from request | invalid, negotiation |
| `neg-unimplemented-method` | request lists a planned or proposed entry | unsupported, request |
| `neg-missing-policy` | request omits source evidence, status or holder policy | invalid, request |
| `neg-unenforced-obligation` | request requires issuer authentication or status from a method whose only route for it is the derived challenge | unsupported, admit |
| `neg-weaker-contract` | selected-support proof for an exact request | invalid, negotiation |
| `neg-holder-declared-as-agreed` | holder-declared journal for an anchor request | invalid, verify |
| `neg-authority-upgrade` | issuer-authenticated, holder-selected method named for an anchor request | unsupported, admit |
| `neg-wrong-scope` | other dataset anchor, or issuer slot outside trust | invalid, verify |
| `neg-wrong-nonce`, `neg-replay` | proof over another challenge; reused challenge | invalid, verify |
| `neg-concurrent-replay` | two valid presentations over one challenge verified concurrently | exactly one accept; the other invalid, verify |
| `neg-expired`, `neg-wrong-audience` | validity passed; audience differs under derived challenge | invalid, verify |
| `neg-wrong-result`, `neg-duplicate-distinct` | one released term altered; repeated DISTINCT row | invalid, verify |
| `neg-missing-row-exact`, `neg-bag-collapsed` | exact bag loses a row; duplicates merged | invalid, verify |
| `neg-extra-row` | released row without witness | unsatisfiable, prepare; invalid, verify if forced |
| `neg-excess-rows` | a valid result carries more released rows than `resources` allows | capacity, verify, after result decoding and proof verification, before challenge consumption; never accept |
| `neg-json-mapping-substitution` | same authenticated bytes, other mapping version | invalid, verify |
| `neg-context-substitution` | pinned context IRI with different content | policy-rejected, import |
| `neg-unauthenticated-field` | JOSE unprotected header mapped into the dataset | invalid, import |
| `neg-issuer-key-mismatch` | signature verifies under a key pinned for issuer B, or for another suite or curve, while the signed `issuer` is A | policy-rejected, import |
| `unsup-unpinned-key-locator` | trust policy has no pinned authorization; only a bare caller key or the presentation's own key locator (`verificationMethod`, `jku`, `x5u`) is available | unsupported, import; never `issuer-authenticated` |
| `neg-wrong-proof-purpose` | pinned key is authorized for issuer A only under `authentication`, or the signed `proofPurpose` is not `assertionMethod` | policy-rejected, import |
| `neg-lexical-substitution` | signed `"018"^^xsd:integer` released or joined as the term `"18"^^xsd:integer` | unsatisfiable, prepare; invalid, verify if forced |
| `unsup-noncanonical-value` | `FILTER(?age >= 18)` whose only witness is signed `"018"^^xsd:integer` (value 18 in SPARQL) | unsupported, prepare; never unsatisfiable |
| `neg-cross-scope-blank-node` | join on equal blank-node labels from two credentials | unsatisfiable, prepare |
| `neg-spliced-hidden-values` | authenticity over credential X, query over credential Y, same nonce and result | invalid, verify |
| `neg-cross-field-link` | BLS12-381 authenticity with a BN254 query proof | unsupported, negotiation |
| `neg-status-rollback` | credential live only below the version window | policy-rejected, prepare; invalid, verify if forced |
| `neg-algorithm-confusion` | signed v3 proof relabeled as v2; bundled key or image id | invalid, verify |
| `neg-update-request`, `neg-unsupported-feature` | `INSERT DATA { … }` as the query; OPTIONAL under the positive-BGP profile | unsupported, request; unsupported, admit |
| `neg-capacity` | request `resources` asks for 5 released rows where the member ceiling is 4 | capacity, admit |
| `neg-infrastructure` | missing toolchain or challenge-store failure | infrastructure, verify; never accept |

Analogous existing controls, reported by references and not re-run here: [signed/unsigned
relabeling](../skills/zk-query-proofs/references/signed-results.md), [V3 request, nonce,
authority, policy, journal and cross-version substitution](../skills/zk-query-proofs/references/graph-results-v3.md),
and [native changed context, spliced amounts and a links-omitted proof](../zk/native-composition/README.md).

## 12. Extension author checklist

1. Registry entry with an honest component status and `adapter_available`; a name adds no
   capability.
2. Descriptor with exact version, parameters, artifact identity and backend pin, all loaded
   from verifier configuration; capabilities with resource ceilings and disclosure inventory.
3. Supported (form, result contract, mode, authority, source evidence) tuples; everything
   else `unsupported`.
4. A binding route (§7.3) for every statement field and an enforcer for every obligation it
   discharges; a cited wire profile or an explicit label that the encoding is local.
5. Typed failures with class, phase and code; opaque witnesses consumed by `prove` and
   cleaned on every exit path; one declared challenge owner and consumption policy.
6. Each (suite, mapping, linking) combination listed; missing edges and field bridges fail
   closed; term identity, blank-node scope, value-to-lexical binding and value domain stated.
7. Every applicable §11 vector, including substitution controls against sibling methods and
   versions; stated maturity and upstream caveats, with no external-audit claim.

## 13. Non-claims

This record makes no security, privacy, conformance or performance claim. It does not claim
that any W3C, IETF or OpenID specification defines this protocol, that registry identifiers
are stable, or that any listed combination beyond those marked implemented exists in source. <!-- privacy-claims-allow: negated claim; asserts no privacy or soundness property -->
