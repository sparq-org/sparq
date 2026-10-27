// Zero-knowledge SPARQL answers: the query request a verifier service sends and the answer
// presentation a holder service returns, as RDF with a JSON-LD serialization. This is the data
// model only; transport is left to the carrying protocol. Two informative companions sit
// beside it: zksparql-answers-oid4vp.typ (carrying it over OpenID for Verifiable
// Presentations) and zksparql-answers-implementation.typ (how the code compares).

#import "_lib/spec.typ": spec-head, sotd, intro-section, references, dfn, note, cite

#set document(title: "Zero-Knowledge SPARQL Answers: Request and Presentation Data Model")
#set text(size: 11pt)
#set par(justify: true)
#set heading(numbering: "1.")

#spec-head()

#intro-section("abstract", "Abstract")[
  A verifier asks a SPARQL query. A holder evaluates it over their own verifiable credentials
  and returns the answer with a proof that it is correct, without handing over the
  credentials. This document defines the two resources that pass between a verifier service
  and a holder service, the #dfn[query request] and the #dfn[answer presentation], as RDF with
  a JSON-LD serialization and a companion vocabulary and context. The answer presentation is a
  verifiable presentation in the sense of the VC Data Model. The request says which issuers the
  verifier trusts, which cryptosuites and proof methods it accepts, whether the issuers'
  signatures stay hidden inside the proof or are revealed, and, optionally, which input dataset
  the answer must be about. Proof methods, which need not be zero-knowledge, plug in by IRI, as
  cryptosuites do for VC Data Integrity. The proof methods in the sparq repository are research
  prototypes and have not been externally audited.
]

#sotd()

Two informative companion documents accompany this one: #emph[Zero-Knowledge SPARQL Answers
over OpenID for Verifiable Presentations] (https://sparq.jeswr.org/specs/zksparql-answers-oid4vp)
shows how OpenID4VP can carry the request and the presentation, and #emph[Zero-Knowledge SPARQL
Answers: Implementation Status] (https://sparq.jeswr.org/specs/zksparql-answers-implementation)
lists the proof methods built and proposed and compares this document with the code in
the sparq repository.

= Introduction <sec-intro>

This section is informative.

In the usual verifiable-credential exchange #cite("VC-DATA-MODEL-2.0"), a holder shows a
verifier the credentials, or selected claims from them. Many questions need less than that. A
lender may only need to know whether any payment in a bank statement was returned. The answer
is one boolean, but showing the statement would reveal every payment.

This document lets the verifier send the question as a SPARQL query #cite("SPARQL12-QUERY")
and lets the holder send back only the answer, with a proof that the answer is correct. The
three roles are those of the VC Data Model:

- an #dfn[issuer] signs credentials about a subject;
- a #dfn[holder] keeps credentials and answers queries over them;
- a #dfn[verifier] asks a query and checks the answer.

The exchange has three steps:

+ The verifier service sends a query request (@sec-request).
+ The holder service builds an RDF dataset from some of its credentials, evaluates the query
  over it, proves the result with a proof method the request accepts, and sends an answer
  presentation (@sec-presentation).
+ The verifier service checks the presentation against the request it sent (@sec-verify).

Any SPARQL query form this document allows can be asked, including queries whose answer
depends on what is absent, such as `NOT EXISTS`, `MINUS`, `OPTIONAL` and aggregates. Such an
answer is about the input dataset the proof covers. Whether that dataset holds everything the
verifier cares about is a separate question, which @sec-input and @sec-cwa address.

== Example <sec-example-intro>

The lender asks whether any payment was returned:

```sparql
PREFIX ex: <https://bank.example/terms#>
ASK { ?payment a ex:Payment ; ex:status ex:Returned . }
```

An answer of `true` shows that a returned payment exists in the credentials the proof covers.
An answer of `false` shows that none of those credentials records a returned payment; it is
only as useful as the lender's confidence that the holder included every relevant statement.
The lender can get that confidence by agreeing the input dataset in advance (@sec-input).
@sec-example shows the full request and presentation.

= Conformance and terminology <sec-conformance>

The key words #strong[MUST], #strong[MUST NOT], #strong[REQUIRED], #strong[SHOULD],
#strong[SHOULD NOT], #strong[RECOMMENDED], #strong[MAY] and #strong[OPTIONAL] are to be
interpreted as described in #cite("RFC2119") and #cite("RFC8174") when, and only when, they
appear in all capitals. Sections marked informative, notes and examples are not normative.

There are three conformance classes: a #emph[verifier service] produces query requests and
verifies answer presentations; a #emph[holder service] consumes query requests and produces
answer presentations; a #emph[proof method] (@sec-methods) defines how one kind of proof is
made and checked.

This document uses the RDF 1.2 terms #emph[RDF dataset], #emph[graph], #emph[IRI],
#emph[literal], #emph[blank node] and #emph[triple term] #cite("RDF12-CONCEPTS"), and the SPARQL
1.2 terms #emph[query form], #emph[solution mapping] and #emph[solution sequence]
#cite("SPARQL12-QUERY"). It also uses:

/ Input dataset: The RDF dataset a holder evaluates the query over, built from credentials as
  the proof method states. The methods in this document use the RDF merge
  #cite("RDF12-SEMANTICS") of the credentials' graphs as the default graph, which keeps each
  credential's blank nodes apart. A method could instead put each credential in its own named
  graph, as a verifiable presentation does #cite("VC-DATA-MODEL-2.0"); a query written for one
  layout does not match the other, so the method states which it uses.
/ Dataset commitment: A digest that fixes the input dataset without revealing it. It includes
  a random salt chosen by the holder, so equal datasets do not give equal commitments unless
  the same salt is reused.
/ Proof method: A way of producing and checking evidence that a SPARQL result is correct,
  named by an IRI (@sec-methods).
/ Proof: The evidence a proof method produces. Depending on the method, it is a
  zero-knowledge proof, a proof that is not zero-knowledge, a signed attestation, or the
  credentials themselves.
/ Statement: The values a proof is about: the request digest, the dataset commitment, the
  result, the signature mode and, in the revealed mode, the signed messages (@sec-binding).
/ Signed message: The bytes an issuer's signature is computed over, as the cryptosuite defines
  them. For `eddsa-rdfc-2022` #cite("VC-DI-EDDSA") it is the SHA-256 hash of the canonical
  proof configuration followed by the SHA-256 hash of the canonical credential document. The
  Merkle-root cryptosuites #cite("ZK-MERKLE-CRYPTOSUITE") (`eddsa-sha256-merkle-2026`,
  `schnorr-poseidon2-merkle-2026` and `mldsa44-sha256-merkle-2026`) sign a digest of the suite
  identifier, a 32-byte salt, the number of quads, the root of a Merkle tree with one leaf per
  canonical quad of the credential, and the digest of the canonical proof configuration.

= Vocabulary and serialization <sec-vocab>

Both resources are RDF graphs. This document defines their terms in the vocabulary
`https://w3id.org/sparq/vcq#` (prefix `vcq:`, @sec-vocab-defs) and serializes them as
JSON-LD 1.1 #cite("JSON-LD11") in compacted form with the context
`https://w3id.org/sparq/vcq/v1` (@sec-vocab-defs). It reuses existing terms where they have the
meaning needed:

- `cred:` (`https://www.w3.org/2018/credentials#`) for `VerifiablePresentation`, `validFrom`
  and `validUntil` #cite("VC-DATA-MODEL-2.0");
- `sec:` (`https://w3id.org/security#`) for `proof`, `challenge`, `domain`, `proofValue`,
  `cryptosuite`, `verificationMethod`, `Multikey` and `publicKeyMultibase`
  #cite("VC-DATA-INTEGRITY");
- `rdf:JSON` for values that are JSON documents, such as SPARQL query results
  #cite("JSON-LD11"). JSON-LD 1.1 does not fix one lexical form for these literals, so this
  document does: it is the JSON value serialized under the JSON Canonicalization Scheme (JCS)
  #cite("RFC8785"), and equal JSON values give equal literals.

Rules for both resources:

+ A JSON-LD document of either resource MUST use the context of @sec-vocab-defs, after the VC
  base context `https://www.w3.org/ns/credentials/v2` where the resource is a verifiable
  presentation. Outside the value of `resultGraph`, a receiver MUST reject a document in which
  any property or type does not expand to an IRI defined by these contexts, as Data Integrity
  requires for signed documents #cite("VC-DATA-INTEGRITY"). Ignoring an unknown term could make
  a verifier accept a weaker answer than it asked for. The value of `resultGraph` is query
  data and may use any IRI.
+ In the whole document, including `resultGraph`, a receiver MUST reject a document from which
  JSON-LD expansion drops any key or value, for example a key that maps to no IRI.
+ Whether a resource has a property is decided on its RDF graph: it has the property if the
  graph holds at least one triple with that resource as subject and that property as
  predicate. A term whose value is a list MUST NOT be given as an empty array, since an empty
  array produces no triple and so is the same as an absent term; a receiver MUST reject a
  document that does so.
+ Byte strings are multibase-encoded base64url without padding (prefix `u`)
  #cite("VC-DATA-INTEGRITY"), typed `sec:multibase` in RDF.
+ Digests are SHA-256 #cite("FIPS180-4") and are 32 bytes before encoding.
+ Times are `xsd:dateTime` values, as in the VC Data Model #cite("VC-DATA-MODEL-2.0"), and
  MUST include a time zone offset.
+ The #dfn[request digest] is SHA-256 over the canonical N-Quads of the query request's RDF
  dataset under RDF Dataset Canonicalization (RDFC-1.0) #cite("RDF-CANON"). Two JSON-LD
  documents with the same RDF content have the same request digest.

This document has no version member. A change of meaning gets new terms or a new context URL;
a proof method that changes gets a new IRI (@sec-methods).

= Query request <sec-request>

A query request is a resource of type `vcq:QueryRequest`. Its properties, by JSON-LD term:

#table(
  columns: (1.6fr, 1fr, 3fr),
  align: (left, left, left),
  table.header[Term][Value][Meaning],
  [`type`], [`QueryRequest`], [REQUIRED.],
  [`id`], [IRI], [OPTIONAL. An identifier for the request.],
  [`query`], [string], [REQUIRED. The SPARQL query text (`vcq:query`). Compared and hashed
    exactly as given.],
  [`inputCommitment`], [byte string], [OPTIONAL. A dataset commitment the verifier agreed in
    advance (@sec-input). If present, the answer MUST be over the dataset it fixes.],
  [`trustedIssuers`], [list of trust requirements], [OPTIONAL; non-empty if present. Which
    issuers the verifier trusts (@sec-trust). If absent, the credentials in the input dataset
    are not checked against any issuer.],
  [`cryptosuite`], [list of strings], [REQUIRED and non-empty if `trustedIssuers` is present. The
    cryptosuites the verifier accepts for credential proofs, for example
    `"eddsa-rdfc-2022"`.],
  [`signatureMode`], [list of IRIs], [REQUIRED and non-empty if `trustedIssuers` is present.
    The signature modes the verifier accepts: one or more of `hidden`, `revealed` and
    `disclosed` (@sec-modes).],
  [`proofMethod`], [list of objects], [REQUIRED, non-empty. The proof methods the verifier
    accepts; the holder service uses any one of them. Each has `method` (the method's IRI),
    `verificationKey`
    (byte string) and `parameters` (JSON). @sec-methods.],
  [`maxPresentationBytes`], [integer], [REQUIRED. Bounds the encoded presentation.],
  [`maxResultSize`], [integer], [REQUIRED. Bounds the solutions in a SELECT result or the
    triples in a CONSTRUCT result.],
  [`challenge`], [string], [REQUIRED. At least 128 bits of randomness, fresh for this
    request (`sec:challenge`).],
  [`domain`], [string], [REQUIRED. Identifies the verifier, for example its OpenID client
    identifier (`sec:domain`).],
  [`validFrom`, `validUntil`], [times], [REQUIRED. The period in which the verifier accepts a
    presentation for this request (`cred:validFrom`, `cred:validUntil`). `validFrom` MUST be
    earlier than `validUntil`.],
)

The query MUST meet these requirements:

+ It is a SPARQL 1.2 query #cite("SPARQL12-QUERY") of form SELECT, ASK or CONSTRUCT.
+ It is self-contained. Every IRI in it is absolute, or relative and resolved by a `BASE`
  declaration in the query itself; nothing outside the query text is needed to parse it.
+ It reads only the input dataset. It uses no `FROM` or `FROM NAMED` clause, no `SERVICE`
  pattern, and none of the functions whose value depends on when or where they are evaluated
  (`NOW`, `RAND`, `UUID`, `STRUUID`, and `BNODE` with no argument).

A query request states nothing about credential status (revocation) or about whether the
presenter is the credential subject. This document checks neither (@sec-not-established).

= Answer presentation <sec-presentation>

An answer presentation is a verifiable presentation #cite("VC-DATA-MODEL-2.0") with types
`VerifiablePresentation` and `vcq:QueryAnswerPresentation`. The VC Data Model allows a
presentation to carry data derived from credentials, such as a zero-knowledge proof, instead of
the credentials themselves. Its properties, by JSON-LD term:

#table(
  columns: (1.6fr, 1fr, 3fr),
  align: (left, left, left),
  table.header[Term][Value][Meaning],
  [`type`], [list], [REQUIRED. `VerifiablePresentation` and `QueryAnswerPresentation`.],
  [`requestDigest`], [byte string], [REQUIRED. The request digest of the query request
    answered.],
  [`inputCommitment`], [byte string], [REQUIRED. The dataset commitment of the input dataset.
    Equal to the request's `inputCommitment` if the request has one.],
  [`signatureMode`], [IRI], [REQUIRED if the request has `trustedIssuers`, and absent
    otherwise. One of the request's signature modes.],
  [`revealedSignature`], [list of objects], [REQUIRED in the revealed mode when the input
    dataset has at least one credential, and absent otherwise. One per credential in the input
    dataset, each with `verificationMethod`,
    `cryptosuite`, `signedMessage` (byte string) and `signatureValue` (byte string: the
    signature alone, without any other value the cryptosuite's `proofValue` carries).
    @sec-modes.],
  [`result`], [JSON], [REQUIRED for SELECT and ASK, and absent for CONSTRUCT. @sec-result.],
  [`resultGraph`], [graph], [REQUIRED for CONSTRUCT, and absent otherwise. @sec-result.],
  [`proof`], [object], [REQUIRED. A `vcq:QueryAnswerProof` with `proofMethod` (the IRI of one
    of the request's proof methods), `challenge` and `domain` (equal to the request's), and
    `proofValue` (byte string: the evidence, encoded as the proof method defines).],
)

Except in the disclosed signature mode (@sec-modes), the presentation carries no credential
and no per-row provenance, and in the hidden signature mode it also carries no issuer
identity. What the verifier learns is listed in @sec-learns.

== Result <sec-result>

- A SELECT or ASK result is a SPARQL 1.2 Query Results JSON document #cite("SPARQL12-RESULTS-JSON"),
  carried as a JSON literal (`rdf:JSON`) in `result`. A SELECT result keeps duplicate
  solutions; its order is the order of the proved solution sequence, which follows the query's
  ORDER BY, and without ORDER BY the order carries no meaning.
- A CONSTRUCT result is an RDF graph, carried as the named graph `resultGraph`.
- A blank node in a result is scoped to that result. It has no relationship to any blank node
  in a credential, even if the labels are equal.

The proof is over the proof method's canonical encoding of the result: the RDFC-1.0 canonical
form for a CONSTRUCT result, and the method's canonical encoding of the solution sequence
otherwise. The verifier MUST derive that encoding from the presentation and MUST reject the
presentation if the result is not exactly the proved one.

#note[
  A SELECT result is carried as a SPARQL Query Results JSON literal rather than as RDF
  because SPARQL 1.2 results can bind triple terms, which JSON-LD 1.1 cannot express, and
  because the results format is the one SPARQL 1.2 maintains. The W3C result-set test
  vocabulary was considered; the SPARQL 1.2 test suite no longer uses it. For the same
  reason, a CONSTRUCT result that contains a triple term cannot be carried until a JSON-LD
  version supports RDF 1.2 (@sec-rdf12).
]

= Semantics <sec-semantics>

This section is normative. Let $Q$ be the request's query and $D$ the input dataset the proof
is about.

== Answers <sec-answers>

The result in a presentation MUST be a result that SPARQL 1.2 #cite("SPARQL12-QUERY")
permits for $Q$ over $D$, complete and untruncated: for SELECT a solution sequence with every
solution and duplicate, for ASK `true` if and only if the solution sequence after its solution
modifiers is not empty, and for CONSTRUCT the graph. Where SPARQL permits more than one result,
for example LIMIT without ORDER BY, the holder chooses one; a proof method MAY fix that choice
(@sec-methods). A result too large for `maxResultSize` or the proof method cannot be answered.
Queries whose answer depends on absence are covered by the same definition; @sec-cwa says what
a verifier needs before relying on them.

== Input dataset <sec-input>

If the request has no `inputCommitment`, the holder chooses which of its credentials make up
$D$, and could leave one out. If it has one, the verifier agreed that dataset commitment
beforehand, by means outside this document, and the presentation's `inputCommitment` MUST
equal it. Where a proof method computes the commitment differently in each signature mode, a
request with an `inputCommitment` MUST list only the mode it was computed under.

== Trust requirements <sec-trust>

`trustedIssuers` states which issuers the verifier trusts. It is a list of #dfn[trust
requirements], each a resource whose `type` says how it identifies trusted issuers. An issuer
is trusted if it meets at least one of them. If `trustedIssuers` is present, the proof MUST
show that every credential in $D$ carries a Data Integrity proof #cite("VC-DATA-INTEGRITY"),
using one of the request's cryptosuites, that verifies under a verification method of a
trusted issuer.

A trust requirement identifies issuers and their verification methods; which cryptosuites are
accepted is the separate `cryptosuite` list. Every trust requirement MUST resolve to a finite
set of verification methods with public keys, the same for holder and verifier, before the
holder service proves and when the verifier service verifies. A requirement
that names an external source, such as a trusted list, MUST say how both sides obtain that set,
and the proof method MUST bind a digest of it.

This document defines one type of trust requirement and reserves others:

#table(
  columns: (1.2fr, 3fr, 1fr),
  align: (left, left, left),
  table.header[Type][Meaning][Status],
  [`IssuerKeys`], [An issuer (`issuer`, an IRI) and its verification methods
    (`verificationMethod`, each a `Multikey` #cite("CID") with `id`, `controller` and
    `publicKeyMultibase`). The keys are given, not looked up, so both sides prove and verify
    against the same keys.], [Defined.],
  [`TrustedList`], [Issuers on an ETSI trusted list or list of trusted entities.],
    [Reserved.],
  [`FederationTrustAnchor`], [Issuers with a trust chain to an OpenID Federation trust
    anchor.], [Reserved.],
  [`RecognitionCredential`], [Issuers recognized by a verifiable recognition credential.],
    [Reserved.],
  [`TrustFramework`], [Issuers certified under a governance framework.], [Reserved.],
)

A reserved type is not yet defined well enough to be used: a verifier service MUST NOT put
one in a request, and a holder service MUST reject a request that has one or that has any
type it does not implement. The implementation status companion describes the sources the
reserved types correspond to and what defining each one needs.

== Signature modes <sec-modes>

When the request has `trustedIssuers`, the signatures on the credentials in $D$ are shown to
the verifier in one of three modes, which the request lists and a proof method supports per
cryptosuite (@sec-methods):

- #emph[hidden] (`vcq:hidden`): the proof shows that every credential in $D$ carries a valid
  signature by a trusted issuer. Signatures, signed messages and keys stay hidden.
- #emph[revealed] (`vcq:revealed`): `revealedSignature` gives each credential's signature and
  signed message, the verifier checks them itself, and the proof shows that $D$ is exactly the
  data those messages cover. `signatureValue` carries the signature bytes only; other values a
  cryptosuite's `proofValue` carries, such as the salt the Merkle-root cryptosuites append
  #cite("ZK-MERKLE-CRYPTOSUITE"), MUST NOT be sent.
- #emph[disclosed] (`vcq:disclosed`): the proof carries the credentials, or presentations
  derived from them by a selective-disclosure cryptosuite such as `bbs-2023`
  #cite("VC-DI-BBS") or `ecdsa-sd-2023` #cite("VC-DI-ECDSA"); the verifier checks them, builds
  $D$ from them and evaluates the query itself.

What each mode discloses is listed in @sec-learns.

== Example <sec-example>

This example is informative. Long values are shortened with `…`. The lender's request, over an
input dataset it agreed earlier:

```json
{
  "@context": "https://w3id.org/sparq/vcq/v1",
  "type": "QueryRequest",
  "query": "PREFIX ex: <https://bank.example/terms#>\nASK { ?payment a ex:Payment ; ex:status ex:Returned . }",
  "inputCommitment": "uq0Lx…",
  "trustedIssuers": [{
    "type": "IssuerKeys",
    "issuer": "https://bank.example/",
    "verificationMethod": [{
      "id": "https://bank.example/keys#2026",
      "type": "Multikey",
      "controller": "https://bank.example/",
      "publicKeyMultibase": "z6Mk…"
    }]
  }],
  "cryptosuite": ["eddsa-rdfc-2022"],
  "signatureMode": ["hidden"],
  "proofMethod": [{
    "method": "urn:sparq:vcq:method:risc0-authenticated-rdf:v5",
    "verificationKey": "uYc9B…",
    "parameters": {}
  }],
  "maxPresentationBytes": 2000000,
  "maxResultSize": 1,
  "challenge": "3Jd8…",
  "domain": "x509_san_dns:lender.example",
  "validFrom": "2026-10-10T10:00:00Z",
  "validUntil": "2026-10-10T10:05:00Z"
}
```

The holder's presentation:

```json
{
  "@context": ["https://www.w3.org/ns/credentials/v2", "https://w3id.org/sparq/vcq/v1"],
  "type": ["VerifiablePresentation", "QueryAnswerPresentation"],
  "requestDigest": "uVt2c…",
  "inputCommitment": "uq0Lx…",
  "signatureMode": "hidden",
  "result": { "head": {}, "boolean": false },
  "proof": {
    "type": "QueryAnswerProof",
    "proofMethod": "urn:sparq:vcq:method:risc0-authenticated-rdf:v5",
    "challenge": "3Jd8…",
    "domain": "x509_san_dns:lender.example",
    "proofValue": "uAAEC…"
  }
}
```

The lender learns that the agreed statements, all signed by the bank's key, contain no
returned payment. It learns nothing else about the payments.

= Binding <sec-binding>

This section is normative.

A proof method MUST make the statement part of what its proof proves, so that a proof for one
statement does not verify for another. In particular:

+ The request digest MUST be bound. The digest covers the query, the input commitment, the
  trust requirements, the accepted cryptosuites, signature modes and proof methods, the
  bounds, the challenge, the domain and the validity period. Binding it means a proof made for
  one request does not verify for another, including one sent to another verifier, and the
  proof needs only one public value for all of them.
+ The result MUST be bound. The verifier takes the result only from what the proof proves.
+ The dataset commitment MUST be bound.
+ The signature mode MUST be bound. In the revealed mode, every signed message MUST be bound,
  and the proof MUST show that $D$ is built from exactly the data those messages cover, so that
  a verifier who checks the signatures outside the proof knows they cover $D$.
+ The proof method's IRI MUST select the verification key. A verifier MUST NOT take a
  verification key or program from the presentation.

A proof method MAY bind these values directly as public inputs, or bind a single digest over
them, as long as the verifier can recompute every bound value from the request and the
presentation. A proof method MAY also make public any value the verifier can compute from the
request and the result alone, for example the triples obtained by substituting a returned
solution into a query that is a single basic graph pattern: such a value tells the verifier
nothing the result does not.

= Verification <sec-verify>

A verifier service verifies an answer presentation against the query request it sent, in this
order, and rejects it at the first check that fails:

+ Reject a presentation larger than the request's `maxPresentationBytes`, before parsing it.
+ Reject it unless it is a JSON-LD document that follows @sec-vocab and @sec-presentation.
+ Reject it unless `requestDigest` is the request digest of the request, the current time is
  between the request's `validFrom` and `validUntil`, and the proof's `challenge` and
  `domain` equal the request's.
+ Reject it unless the proof's `proofMethod` is the `method` of one of the request's proof
  method entries, and verify with that entry's `verificationKey` and `parameters`.
+ If the request has an `inputCommitment`, reject it unless the presentation's equals it.
+ If the request has `trustedIssuers`, reject it unless `signatureMode` is one of the request's
  modes. In the revealed mode, reject it unless every entry of `revealedSignature` verifies
  under its cryptosuite, the cryptosuite is one of the request's, and the verification method
  belongs to a trusted issuer. In the disclosed mode, the proof method's checks in the next
  step verify the disclosed credentials in the same way.
+ Reject it if the result has the wrong form for the query or is larger than
  `maxResultSize`.
+ Verify the proof with the proof method over the statement computed from the request and the
  presentation. Reject it if verification fails.

A verifier service MUST NOT accept more than one presentation with the same challenge, across
all of its instances and at any time, and MUST NOT accept a presentation once the request's
`validUntil` has passed, however long verification took. Implementations typically meet this
with an atomic, durable record of the challenges already used.

On acceptance the verifier has the query, the result, the dataset commitment and whether it
agreed that commitment, and the trust requirements its credentials met.

= Proof methods <sec-methods>

A #dfn[proof method] is a way of producing and checking evidence that the statement holds. It
plays the part for query answers that a cryptosuite plays for VC Data Integrity
#cite("VC-DATA-INTEGRITY"): the presentation names one by IRI, and the verifier accepts only
the methods its request lists. A new version of a method is a new method with a new IRI: a
verifier that accepts `…:risc0-authenticated-rdf:v5` does not thereby accept
`…:risc0-authenticated-rdf:v6`.

A proof method need not be zero-knowledge. Besides zero-knowledge proofs, a method may produce
a proof that hides nothing, an attestation signed by a trusted execution environment (TEE)
stating that a measured program evaluated the query over the input and checked the
signatures, or the signed credentials themselves (in full or selectively disclosed) for the
verifier to check and evaluate the query over. What a verifier relies on differs between
these, so each method states it explicitly.

Each proof method MUST publish:

- its IRI;
- the #strong[kind of evidence]: a zero-knowledge proof, a proof that is not zero-knowledge,
  an attestation, or disclosed credentials;
- whether the evidence is #strong[publicly verifiable] (anyone holding it and the request can
  check it) or #strong[designated-verifier] (it convinces only the verifier that took part);
- #strong[the queries it supports]: the query forms and the SPARQL 1.2 features it can prove,
  stated as the features it excludes. A method that does not support triple terms, a
  function, a property path form or any other feature MUST say so. A holder service MUST
  NOT answer with a method that does not support every feature the query uses, and a
  verifier service SHOULD list only methods that support its query;
- #strong[the results it can prove]: in particular, a method that proves only that the
  returned solutions are solutions, and not that none is missing, can answer only queries
  for which every such result is a permitted result. Examples are an ASK whose answer is
  `true`, and a `SELECT DISTINCT … LIMIT k` query for which it returns `k` solutions, in both
  cases with no subquery, GROUP BY, aggregate, HAVING, ORDER BY, OFFSET or REDUCED, and for
  ASK no LIMIT. With those, which results are permitted depends on solutions not returned, so
  membership alone is not enough;
- #strong[what the verifier must trust] for an accepted proof to mean the statement holds: for
  example the proof system's soundness and any trusted setup, or for an attestation the
  hardware vendor's attestation key, the measured program and the TEE's resistance to physical
  and side-channel attacks;
- #strong[what the verifier learns] beyond the statement: for a zero-knowledge proof, nothing
  the method's privacy argument does not allow; for other evidence, everything it reveals,
  such as a platform identity in a TEE attestation;
- the cryptosuites it can check, how it builds the input dataset from credentials, and the
  signature modes it supports for each cryptosuite;
- the encoding of `verificationKey`, including byte order where the key or image ID is a
  sequence of words, and of `parameters` and their defaults;
- any of the choices @sec-answers leaves to the holder that the method fixes;
- the canonical encoding of the statement and how the evidence binds it (@sec-binding); for an
  attestation, the statement digest MUST be in the signed report;
- the encoding of `proofValue` and any size or capacity bounds.

A proof method MAY be interactive, with the verifier taking part in producing the evidence.
Such a method defines the channel and the messages, MUST bind the request digest into the
protocol transcript, and defines what `proofValue` then carries (for example the transcript,
or an identifier of the completed session). For such a method, verifying the proof
(@sec-verify) means completing the protocol and checking its outcome.

A method is defined by what the verifier checks, not by how the evidence is produced: the same
proof produced on a CPU or on a GPU or other proving accelerator uses the same method. A new
circuit or program for the same method, for example a circuit compiled directly to ACIR
instead of from Noir, has a new verification key, and the verifier names that key in
`verificationKey`.

The proof methods built in the sparq repository, and those proposed, are listed in the
implementation status companion
(https://sparq.jeswr.org/specs/zksparql-answers-implementation). None has had an external
cryptographic audit.

= Security and privacy considerations <sec-security>

== Open-world and closed-world queries <sec-cwa>

RDF has an open-world reading: a graph that does not state something does not deny it. A query
that only matches patterns respects this. If it has a solution over the input dataset, it has
that solution over any dataset that includes it, so a `true` ASK or a returned pattern match
stays true whatever else the holder holds.

A query whose answer depends on absence, through `NOT EXISTS`, `MINUS`, an unmatched
`OPTIONAL`, an aggregate such as `COUNT`, or an ASK that returns `false`, reads the input
dataset as complete. Its answer is correct for the input dataset, and the proof shows that.
It is the right answer to the verifier's question only if the input dataset holds every
statement relevant to it. The proof cannot show that, because it covers only the credentials
the dataset was built from:

- If the holder chose the input dataset (the request has no `inputCommitment`), the holder
  could have left out a credential that changes the answer, for example the statement that
  records a returned payment. A verifier SHOULD NOT treat such an answer as complete unless it
  trusts the holder to include every relevant credential.
- If the verifier agreed the input dataset in advance, the answer is about that dataset. The
  verifier's guarantee is then exactly as good as its reasons for agreeing the commitment, for
  example that it saw which credentials the dataset holds and that they cover the period it
  asks about.
- Trust requirements and signature checks show who made the statements, not that no other
  statement exists.

== What an accepted answer does not establish <sec-not-established>

- #strong[Credential status.] This document does not check whether a credential was revoked
  or suspended.
- #strong[Holder binding.] This document does not show that the presenter is the credential
  subject or controls a key bound to the credential. Anyone who holds the credentials can
  answer.
- #strong[Truth of claims.] A valid issuer signature shows that the issuer made the claims, not
  that they are true #cite("VC-DATA-MODEL-2.0").
- #strong[More than the method's trust assumptions.] An accepted proof means the statement
  holds only if what the method says the verifier must trust (@sec-methods) holds. The proof
  methods built so far are research prototypes without an external audit; an accepted
  proof is evidence produced by that code, not a guarantee.

== What the verifier learns <sec-learns>

The verifier learns the query (it wrote it), the result, the proof method used and the dataset
commitment. The size of the result can reveal more than its values (for example, the number of
payments that match). Requests with narrow results, such as an ASK, reveal least.

In the revealed signature mode the verifier also learns each credential's signature, signed
message and issuer key, and so the number of credentials. A signature and its signed message
are the same in every presentation of that credential, so verifiers can link presentations of
the same credential. When the signed message is an unsalted hash of the credential, as with
`eddsa-rdfc-2022`, a verifier who can guess a credential's full content can confirm the guess
by hashing it. The Merkle-root cryptosuites sign a salted digest, which avoids the second
problem while the salt stays private, but not the first. For this reason the revealed mode
sends the signature without the salt (@sec-modes). Whether signatures can be forged by an
attacker with a quantum computer depends on the cryptosuite, not on the mode: for Ed25519, the
public key alone is enough.

Where SPARQL 1.2 permits several results (@sec-answers), the holder chooses among them, and the
choice can carry information: for `SELECT ?x WHERE { VALUES ?x { 0 1 } } LIMIT 1` the
returned value can encode one bit of the holder's choosing. A zero-knowledge proof does not
prevent this, because the result is public. This matters when the holder does not fully trust
its holder service, for example when a third party operates it.

In the disclosed signature mode the verifier learns everything the disclosed credentials or
derived presentations contain, including claims the issuer made mandatory to disclose.

A holder service SHOULD use a fresh salt for each presentation over a dataset it chose. A
reused salt over the same credentials repeats the commitment, which lets verifiers link
presentations. An agreed commitment is linkable by design, to the verifier that agreed it.

== Replay <sec-replay>

The request digest covers the challenge, the domain and the validity period, and the proof
binds the request digest. A presentation therefore verifies only for the request it answers.
The verifier accepts at most one presentation per challenge (@sec-verify), so a captured
presentation cannot be replayed to the same verifier, and the domain stops it being used with
another. Without a challenge, anyone who captured a presentation could present it again within
the validity period, so `challenge` is required even though Data Integrity makes it optional.

== RDF 1.2 <sec-rdf12>

This document depends on RDF 1.2 and SPARQL 1.2, which are not yet Recommendations; it will be
revised as they change. Three related limits apply now. JSON-LD 1.1, and so the credentials it
serializes, cannot express triple terms; JSON-LD 1.2 is planned to. RDF Dataset Canonicalization
1.0 does not handle triple terms, so neither the request digest nor a canonical CONSTRUCT
result can contain one. And a proof method states which SPARQL 1.2 features it does not
support (@sec-methods); the methods in this document support none of the features new in 1.2.

= Vocabulary and context <sec-vocab-defs>

This section is normative.

The vocabulary `https://w3id.org/sparq/vcq#` defines every `vcq:` term in RDF, with its label,
meaning, domain and range. The context `https://w3id.org/sparq/vcq/v1` maps the JSON-LD terms
of @sec-request and @sec-presentation to those IRIs and to the reused terms of @sec-vocab, with
the datatypes this document requires. Both are normative. Each term SHOULD be dereferenceable
to its description in RDF.

#note[
  The namespace and context IRIs are under `w3id.org` so that they can stay stable if the
  documents move. The redirect is not yet registered; until it is, the vocabulary is at
  https://sparq.jeswr.org/ns/vcq/vcq.ttl and the context at
  https://sparq.jeswr.org/ns/vcq/v1.jsonld. A Working Group taking this design forward would
  decide the final IRIs.
]

= References <sec-refs>

#references((
  ("RFC8785", [Rundgren, A.; Jordan, B.; Erdtman, S. #emph[JSON Canonicalization Scheme
    (JCS)]. RFC 8785, June 2020. https://www.rfc-editor.org/rfc/rfc8785.]),
  ("RFC2119", [Bradner, S. #emph[Key words for use in RFCs to Indicate Requirement Levels].
    RFC 2119, IETF, March 1997.]),
  ("RFC8174", [Leiba, B. #emph[Ambiguity of Uppercase vs Lowercase in RFC 2119 Key Words].
    RFC 8174, IETF, May 2017.]),
  ("FIPS180-4", [NIST. #emph[Secure Hash Standard (SHS)]. FIPS PUB 180-4, August 2015.]),
  ("SPARQL12-QUERY", [Hartig, O.; Seaborne, A.; Taelman, R.; Williams, G.; Pellissier Tanon, T.
    (eds). #emph[SPARQL 1.2 Query Language]. W3C Working Draft, 8 October 2026.
    https://www.w3.org/TR/sparql12-query/.]),
  ("SPARQL12-RESULTS-JSON", [Seaborne, A.; Taelman, R.; Williams, G.; Pellissier Tanon, T.
    (eds). #emph[SPARQL 1.2 Query Results JSON Format]. W3C Working Draft, 13 August 2026.
    https://www.w3.org/TR/sparql12-results-json/.]),
  ("RDF12-CONCEPTS", [Kellogg, G.; Hartig, O.; Champin, P.-A.; Seaborne, A. (eds).
    #emph[RDF 1.2 Concepts and Abstract Data Model]. W3C Candidate Recommendation Snapshot,
    7 April 2026. https://www.w3.org/TR/rdf12-concepts/.]),
  ("RDF12-SEMANTICS", [Patel-Schneider, P.; Arndt, D.; Franconi, E. (eds). #emph[RDF 1.2
    Semantics]. W3C Candidate Recommendation Draft, 24 September 2026.
    https://www.w3.org/TR/rdf12-semantics/.]),
  ("JSON-LD11", [Kellogg, G.; Champin, P.-A.; Longley, D. (eds). #emph[JSON-LD 1.1]. W3C
    Recommendation, 16 July 2020. https://www.w3.org/TR/json-ld11/.]),
  ("RDF-CANON", [Longley, D.; Kellogg, G.; Yamamoto, D. (eds). #emph[RDF Dataset
    Canonicalization]. W3C Recommendation, 21 May 2024. https://www.w3.org/TR/rdf-canon/.]),
  ("VC-DATA-MODEL-2.0", [Sporny, M.; Thibodeau Jr, T.; Herman, I.; Cohen, G.; Jones, M. B.
    (eds). #emph[Verifiable Credentials Data Model v2.0]. W3C Recommendation, 15 May 2025.
    https://www.w3.org/TR/vc-data-model-2.0/.]),
  ("VC-DATA-INTEGRITY", [Sporny, M.; Thibodeau Jr, T.; Herman, I.; Longley, D.; Bernstein, G.
    (eds). #emph[Verifiable Credential Data Integrity 1.0]. W3C Recommendation, 15 May 2025.
    https://www.w3.org/TR/vc-data-integrity/.]),
  ("CID", [Sporny, M.; et al. (eds). #emph[Controlled Identifiers v1.0]. W3C Recommendation,
    15 May 2025. https://www.w3.org/TR/cid-1.0/.]),
  ("VC-DI-BBS", [Sporny, M.; Longley, D.; et al. (eds). #emph[Data Integrity BBS Cryptosuites
    v1.0]. W3C. https://www.w3.org/TR/vc-di-bbs/.]),
  ("VC-DI-ECDSA", [Sporny, M.; Longley, D.; et al. (eds). #emph[Data Integrity ECDSA
    Cryptosuites v1.0]. W3C Recommendation, 15 May 2025. https://www.w3.org/TR/vc-di-ecdsa/.]),
  ("VC-DI-EDDSA", [Sporny, M.; Longley, D.; et al. (eds). #emph[Data Integrity EdDSA
    Cryptosuites v1.0]. W3C Recommendation, 15 May 2025. https://www.w3.org/TR/vc-di-eddsa/.]),
  ("ZK-MERKLE-CRYPTOSUITE", [The sparq project. #emph[Merkle-Root Cryptosuites for RDF
    Verifiable Credentials]. Unofficial Proposal Draft, 2026. Published on this site as
    `zk-merkle-cryptosuite`.]),
))
