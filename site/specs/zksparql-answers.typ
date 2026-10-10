// Zero-knowledge SPARQL answers: the query request a verifier service sends and the answer
// presentation a holder service returns. This is the data model only; transport is left to
// the carrying protocol (section 12 shows how OpenID for Verifiable Presentations can carry it).
// Section 11 maps every member to the code that exists today and lists what is not built yet.

#import "_lib/spec.typ": spec-head, sotd, intro-section, references, dfn, note, cite

#set document(title: "Zero-Knowledge SPARQL Answers: Request and Presentation Data Model")
#set text(size: 11pt)
#set par(justify: true)
#set heading(numbering: "1.")

#spec-head()

#intro-section("abstract", "Abstract")[
  A verifier asks a SPARQL query. A holder evaluates it over their own verifiable credentials
  and returns the answer with a zero-knowledge proof, without handing over the credentials.
  This document defines the two JSON objects that pass between a verifier service and a holder
  service: the #dfn[query request] and the #dfn[answer presentation]. The request states the
  query and what kind of answer the verifier needs: a #dfn[Supported] answer, in which every
  returned solution is genuine, or an #dfn[Exact] answer, which is the complete result over a
  stated input. It also states who fixed that input: the holder (#dfn[holder-declared]) or the
  verifier, in advance (#dfn[verifier-agreed]). A third choice says whether the issuers'
  signatures stay hidden inside the proof or are revealed to the verifier, which makes the
  proof much cheaper at the cost of disclosing the signatures. Proof systems plug in through an
  identifier and a version. The proof systems in the sparq repository are research prototypes and have not
  been externally audited.
]

#sotd()

= Introduction

This section is informative.

In the usual verifiable-credential exchange #cite("VC-DATA-MODEL-2.0"), a holder shows a
verifier the credentials, or selected claims from them. Many questions need less than that. A
lender may only need to know whether any payment in a bank statement was returned. The answer
is one boolean, but showing the statement would reveal every payment.

This document lets the verifier send the question as a SPARQL query #cite("SPARQL11-QUERY")
and lets the holder send back only the answer, with a proof that the answer is correct. The
three roles are those of the VC Data Model:

- an #dfn[issuer] signs credentials about a subject;
- a #dfn[holder] keeps credentials and answers queries over them;
- a #dfn[verifier] asks a query and checks the answer.

The exchange has three steps:

+ The verifier service sends a query request (section 4). It stores its own copy.
+ The holder service builds an RDF dataset from some of its credentials, evaluates the query
  over that dataset, proves the result with a proof system the request accepts, and sends an
  answer presentation (section 5).
+ The verifier service checks the presentation against its stored request (section 8).

A proof is only useful if the verifier knows what it guarantees. Two choices in the request
fix that. The #emph[answer kind] says whether the result may be partial (Supported) or must be
complete (Exact). The #emph[input kind] says who chose the dataset the query ran over. An Exact
answer is complete over its input and no more: if the holder chose the input, the holder could
have left a credential out. Section 6 states both choices precisely.

== Example

The lender asks whether any payment was returned:

```sparql
PREFIX ex: <https://bank.example/terms#>
ASK { ?payment a ex:Payment ; ex:status ex:Returned . }
```

A Supported answer can only say `true`: it shows that some returned payment exists, and a
missing row proves nothing. To learn `false`, the lender needs an Exact answer. If the holder
picked the input, `false` means only "no returned payment in the credentials the holder chose
to include". If the lender first agreed the input, for example the statements its own process
obtained, `false` is a statement about that agreed input. Section 6.4 shows the full request
and presentation for this case.

= Conformance and terminology

The key words #strong[MUST], #strong[MUST NOT], #strong[REQUIRED], #strong[SHOULD],
#strong[SHOULD NOT], #strong[RECOMMENDED], #strong[MAY] and #strong[OPTIONAL] are to be
interpreted as described in #cite("RFC2119") and #cite("RFC8174") when, and only when, they
appear in all capitals. Sections marked informative, notes and examples are not normative.

There are three conformance classes: a #emph[verifier service] produces query requests and
checks answer presentations; a #emph[holder service] consumes query requests and produces
answer presentations; a #emph[proof system] (section 9) defines how one kind of proof is made
and checked.

This document uses the SPARQL 1.1 terms #emph[query form], #emph[solution mapping],
#emph[solution sequence] and #emph[RDF dataset] #cite("SPARQL11-QUERY"), and writes
$[| Q |]_D$ for the multiset of solution mappings of query $Q$ over RDF dataset $D$ under the
SPARQL 1.1 semantics. It also uses:

/ Input dataset: The RDF dataset a holder evaluates the query over, built from credentials as
  the proof system states (for example, the union of the credentials' graphs in the default
  graph, with blank nodes kept apart per credential).
/ Dataset commitment: A digest that fixes the input dataset without revealing it. It includes
  a random salt chosen by the holder, so equal datasets do not give equal commitments unless
  the same salt is reused.
/ Proof system: A method of proving a SPARQL result, named by an identifier and a version,
  together with the verification artifact (a verification key or program image) that checks
  its proofs.
/ Statement: The public values a proof is about: the request digest, the answer kind, the
  input kind, the dataset commitment, the result, the signature mode and, in the revealed
  mode, the signed messages.
/ Signed message: The bytes an issuer's signature is computed over, as the cryptosuite defines
  them. For `eddsa-rdfc-2022` #cite("VC-DI-EDDSA") it is the SHA-256 hash of the canonical
  proof configuration followed by the SHA-256 hash of the canonical credential document. A
  cryptosuite that signs a Merkle root over a credential's RDF terms has the root as its signed
  message.

= Data model conventions

Both objects are JSON objects #cite("RFC8259").

+ Byte strings are encoded as base64url without padding #cite("RFC7515").
+ Digests are SHA-256 #cite("FIPS180-4") and are 32 bytes before encoding.
+ Times are RFC 3339 timestamps in UTC #cite("RFC3339").
+ A member marked REQUIRED MUST be present. A receiver MUST reject an object that has a member
  this document does not define for its `version`, or a member with a value of the wrong type.
  This is stricter than OpenID for Verifiable Presentations, which ignores unknown parameters:
  a verifier that silently ignored an unknown member could accept a weaker answer than it
  asked for.
+ The #dfn[request digest] is SHA-256 over the JSON Canonicalization Scheme (JCS)
  #cite("RFC8785") serialization of the query request.

The only extension points are the `version` member of each object and the proof-system
identifier and version (section 9). A change to the meaning of any member needs a new
`version`.

= Query request

The verifier service creates the request and keeps it. A holder service never changes it, and
the verifier service checks every presentation against its own stored copy, not against
anything the holder returns.

#table(
  columns: 3,
  align: (left, left, left),
  table.header[Member][Value][Meaning],
  [`type`], [`"SparqlQueryRequest"`], [REQUIRED.],
  [`version`], [`1`], [REQUIRED. This document defines version 1.],
  [`query`], [string], [REQUIRED. The exact SPARQL 1.1 query text, UTF-8. It is compared and
    hashed as given, never normalized.],
  [`baseIri`], [string], [OPTIONAL. Base IRI for resolving relative IRIs in `query`. If absent,
    the query MUST NOT contain relative IRIs.],
  [`answerKind`], [`"supported"` or `"exact"`], [REQUIRED. Section 6.1.],
  [`input`], [object], [REQUIRED. Section 6.2. Either `{"kind": "holder-declared"}` or
    `{"kind": "verifier-agreed", "commitment": <digest>}`.],
  [`issuers`], [array of objects], [REQUIRED. The issuer keys the verifier accepts. Each entry
    has `issuer` (IRI), `verificationMethod` (IRI) and `cryptosuite` (string, for example
    `"eddsa-rdfc-2022"`) and MAY have `publicKeyMultibase`. An empty array means the verifier
    accepts an input dataset whose credentials are not checked against any issuer key.],
  [`signatureModes`], [array of strings], [REQUIRED if `issuers` is not empty, and absent
    otherwise. The signature modes the verifier accepts: `"hidden"`, `"revealed"` or both.
    Section 6.3.],
  [`proofSystems`], [array of objects], [REQUIRED, non-empty. The proof systems the verifier
    accepts, in order of preference. Each entry has `id`, `version` and `artifact` (section 9).],
  [`limits`], [object], [REQUIRED. `maxPresentationBytes` (integer) bounds the encoded
    presentation; `maxResultRows` (integer) bounds the solutions in a SELECT result or the
    triples in a CONSTRUCT result.],
  [`challenge`], [byte string], [REQUIRED. 32 random bytes, fresh for this request.],
  [`audience`], [string], [REQUIRED. Identifies the verifier, for example its OpenID client
    identifier.],
  [`notBefore`, `notAfter`], [timestamps], [REQUIRED. The period in which a presentation is
    accepted. `notBefore` MUST be earlier than `notAfter`.],
)

Rules for the verifier service:

+ The query MUST be a SPARQL 1.1 query of form SELECT, ASK or CONSTRUCT. SPARQL Update and
  DESCRIBE are not part of version 1.
+ The query MUST NOT use a FROM or FROM NAMED clause, a SERVICE pattern, or a function whose
  value depends on when or where it is evaluated (`NOW`, `RAND`, `UUID`, `STRUUID`,
  `BNODE()` with no argument): the input dataset is the only data the query may read.
+ With `"answerKind": "supported"`, the form MUST be SELECT or ASK.
+ The verifier service MUST list a proof system only if it holds that system's verification
  artifact in its own configuration.
+ The verifier service MUST store the request and its request digest, and MUST NOT accept a
  presentation for it after `notAfter` or after one presentation has been accepted.

A query request states nothing about credential status (revocation) or about whether the
presenter is the credential subject. Version 1 checks neither (section 10.1).

= Answer presentation

The holder service returns one answer presentation for one query request.

#table(
  columns: 3,
  align: (left, left, left),
  table.header[Member][Value][Meaning],
  [`type`], [`"SparqlAnswerPresentation"`], [REQUIRED.],
  [`version`], [`1`], [REQUIRED.],
  [`requestDigest`], [digest], [REQUIRED. The request digest of the query request answered.],
  [`proofSystem`], [object], [REQUIRED. `id` and `version` of one entry in the request's
    `proofSystems`.],
  [`answerKind`], [string], [REQUIRED. Equal to the request's `answerKind`.],
  [`input`], [object], [REQUIRED. `kind`, equal to the request's input kind, and
    `commitment`, the dataset commitment of the input dataset.],
  [`signatureMode`], [string], [REQUIRED if the request has `signatureModes`, and absent
    otherwise. One of the request's `signatureModes`.],
  [`signatures`], [array of objects], [REQUIRED in the revealed mode, and absent otherwise.
    One entry per credential in the input dataset, each with `verificationMethod`,
    `cryptosuite`, `signedMessage` (byte string) and `proofValue` (the signature, as the
    cryptosuite encodes it). Section 6.3.],
  [`result`], [object], [REQUIRED. The query result, encoded as in section 5.1.],
  [`proof`], [byte string], [REQUIRED. The proof, in the encoding the proof system defines.],
)

The presentation carries no credential and no per-row provenance. In the hidden signature
mode it also carries no issuer identity. What
the verifier learns is listed in section 10.2.

== Result encoding

- A SELECT result is a SPARQL 1.1 Query Results JSON object #cite("SPARQL11-RESULTS-JSON")
  with `head.vars` and `results.bindings`. An unbound variable is omitted from its binding
  object, as that format specifies.
- An ASK result is a SPARQL 1.1 Query Results JSON object with `head` and `boolean`.
- A CONSTRUCT result is `{"ntriples": <string>}`: the constructed graph in canonical N-Triples
  under RDF Dataset Canonicalization (RDFC-1.0) #cite("RDF-CANON").
- A blank node in a SELECT result is given a label local to this result. Two cells with the
  same label denote the same blank node; no label is the label used in any credential.
- In an Exact SELECT result, `results.bindings` keeps duplicate solutions. Its order is the
  query's order if the query has ORDER BY; otherwise it is sorted by the proof system's
  canonical encoding of each solution and the order carries no meaning.
- In a Supported SELECT result, `results.bindings` has no duplicate solutions.

The proof is over the proof system's canonical encoding of the result, not over the JSON text.
The verifier MUST derive that encoding from the `result` member and MUST reject the
presentation if the result is not exactly the proved one.

= Answer kinds and input kinds

This section is normative. Let $Q$ be the request's query and $D$ the input dataset the proof
is about.

== Answer kinds

A #emph[Supported] answer states that every returned solution is a solution:
for SELECT, every row of `result` is in $[| Q |]_D$, and for ASK, `result` is `true` and
$[| Q |]_D$ is not empty. It says nothing about solutions it does not return. A Supported ASK
answer cannot be `false`, and a Supported SELECT answer with no rows MUST be rejected.

An #emph[Exact] answer states that `result` is the complete result over $D$: for SELECT, the
rows are exactly the multiset $[| Q |]_D$ (after LIMIT and OFFSET, if the query has them); for
ASK, `result` is `true` if and only if $[| Q |]_D$ is not empty; for CONSTRUCT, `result` is the
graph SPARQL 1.1 defines for $Q$ over $D$. If the result would exceed `limits.maxResultRows` or a
bound of the proof system, the holder service MUST NOT return a truncated result.

An Exact answer never satisfies a request for a Supported answer, and the reverse; the kinds
MUST be equal.

== Input kinds

With a #emph[holder-declared] input, the holder chose which credentials make up $D$ when it
answered. The commitment fixes $D$ but the verifier did not choose it. A holder can always
leave a credential out, so an Exact holder-declared answer of `false`, or with no rows, is a
statement about the credentials the holder included and nothing more.

With a #emph[verifier-agreed] input, the verifier accepted a dataset commitment before sending
the request, and put it in `input.commitment`. The presentation's commitment MUST equal it. An
Exact answer is then complete over the dataset the verifier agreed to. How the verifier comes
to accept a commitment is outside this document; for example, it may receive the commitment in
an earlier exchange in which it also learned which credentials the dataset holds.

Input kind is separate from issuer checking. If the request's `issuers` is not empty, the
proof MUST show that every credential in $D$ carries a valid proof from one of the listed keys,
whichever input kind is used. Checking issuers never turns a holder-declared input into a
verifier-agreed one.

== Signature modes

When the request lists issuer keys, the proof must show that every credential in $D$ is signed
by one of them. There are two ways to do that, and the verifier chooses which it accepts.

In the #emph[hidden] mode, the proof shows that the holder knows a valid signature from one of
the listed keys on every credential in $D$. The signatures, the signed messages and which key
signed which credential stay hidden. Checking a signature inside the proof is usually the most
expensive part of proving.

In the #emph[revealed] mode, the presentation's `signatures` member gives each credential's
signature and signed message. The verifier checks each signature itself, outside the proof,
against a key in `issuers`. The proof then only has to show that $D$ is exactly the data those
signed messages cover, and that the result is correct over $D$. This is cheaper to prove, but
discloses the signatures, the signed messages, the issuer keys used and the number of
credentials (section 10.2).

A proof system states which modes it supports for each cryptosuite (section 9). Other
trade-offs between what is hidden and what is revealed, such as revealing only which issuers
signed, can be added as further mode values in a later version of this document.

== Worked example

This example is informative. Long values are shortened with `…`. The lender's request, for an
Exact answer over an input it agreed earlier:

```json
{
  "type": "SparqlQueryRequest",
  "version": 1,
  "query": "PREFIX ex: <https://bank.example/terms#>\nASK { ?payment a ex:Payment ; ex:status ex:Returned . }",
  "answerKind": "exact",
  "input": { "kind": "verifier-agreed", "commitment": "q0Lx…" },
  "issuers": [{
    "issuer": "https://bank.example/",
    "verificationMethod": "https://bank.example/keys#2026",
    "cryptosuite": "eddsa-rdfc-2022"
  }],
  "signatureModes": ["hidden"],
  "proofSystems": [{
    "id": "urn:sparq:vcq:method:risc0-authenticated-rdf",
    "version": 5,
    "artifact": { "imageId": "Yc9B…", "sha256": "1mE0…" }
  }],
  "limits": { "maxPresentationBytes": 2000000, "maxResultRows": 1 },
  "challenge": "3Jd8…",
  "audience": "x509_san_dns:lender.example",
  "notBefore": "2026-10-10T10:00:00Z",
  "notAfter": "2026-10-10T10:05:00Z"
}
```

The holder's presentation:

```json
{
  "type": "SparqlAnswerPresentation",
  "version": 1,
  "requestDigest": "Vt2c…",
  "proofSystem": { "id": "urn:sparq:vcq:method:risc0-authenticated-rdf", "version": 5 },
  "answerKind": "exact",
  "input": { "kind": "verifier-agreed", "commitment": "q0Lx…" },
  "signatureMode": "hidden",
  "result": { "head": {}, "boolean": false },
  "proof": "AAEC…"
}
```

The lender learns that the agreed statements, all signed by the bank's key, contain no
returned payment. It learns nothing else about the payments.

= Binding

This section is normative.

A proof system MUST make the statement (section 2) part of what its proof proves, so that a
proof for one statement does not verify for another. In particular:

+ The request digest MUST be bound. Because the digest covers the query, `baseIri`, answer kind,
  input, issuers, accepted proof systems, limits, challenge, audience and validity period, a
  proof made for one request does not verify against another, and a proof made for one
  verifier does not verify for another.
+ The result MUST be bound. The verifier takes the result only from what the proof proves.
+ The input kind and the dataset commitment MUST be bound.
+ The signature mode MUST be bound. In the revealed mode, every signed message MUST be bound,
  and the proof MUST show that $D$ is built from exactly the data those messages cover, so that
  a verifier who checks the signatures outside the proof knows they cover $D$.
+ The proof system identifier and version MUST select the verification artifact. A verifier
  MUST NOT take a verification key or program from the presentation.

A proof system MAY bind these values directly as public inputs, or bind a single digest over
them, as long as the verifier can recompute every bound value from its stored request and the
presentation.

= Verifier processing

A verifier service processes an answer presentation in this order and rejects it at the first
check that fails. It returns no partial result and no warning instead of a rejection.

+ Reject if the encoded presentation is larger than `limits.maxPresentationBytes`. This check
  comes before parsing.
+ Parse the presentation and reject it if it does not follow sections 3 and 5.
+ Find the stored request whose request digest equals `requestDigest`. Reject if there is none,
  if the current time is outside `notBefore` to `notAfter`, or if a presentation for this
  request was already accepted.
+ Reject unless `proofSystem` names an entry of the request's `proofSystems`, and load that
  entry's verification artifact from the verifier's own configuration.
+ Reject unless `answerKind` and `input.kind` equal the request's. For a verifier-agreed input,
  reject unless `input.commitment` equals the request's.
+ If the request has `signatureModes`, reject unless `signatureMode` is one of them. In the
  revealed mode, reject unless every entry of `signatures` verifies under its cryptosuite, with
  a `verificationMethod` and `cryptosuite` that match an entry of the request's `issuers`.
+ Reject if the result breaks section 5.1 or 6.1: the wrong shape for the query form, a
  Supported ASK of `false`, an empty Supported SELECT, duplicate rows in a Supported SELECT, or
  more than `limits.maxResultRows` rows or triples.
+ Verify the proof with the proof system over the statement recomputed from the stored request
  and the presentation. Reject if it fails.
+ Mark the request as answered, in one atomic step that fails if it was already marked. Only
  then accept.

On acceptance the verifier service has: the query, the result, the answer kind, the input kind,
the dataset commitment and the issuer keys from its own request. A verifier service SHOULD keep
these together, so that later use of the result also records what it guarantees.

= Proof systems

The proof-system identifier and version are the way to add new proof systems. Each proof
system MUST publish:

- its identifier (an IRI) and version (a positive integer);
- the form of the `artifact` member that pins its verification key or program;
- the query forms, answer kinds and input kinds it supports, and the SPARQL fragment it accepts
  (a proof system MAY accept only part of SPARQL 1.1);
- how it builds the input dataset from credentials, and which cryptosuites it can check;
- the signature modes it supports for each cryptosuite;
- the canonical encoding of the statement and how the proof binds it (section 7);
- its proof encoding and any size or capacity bounds.

A new version of a proof system is a new proof system: a verifier that accepts version 5 does
not thereby accept version 6.

The proof systems currently in the sparq repository are below. This table is informative and
records what the code does, not what is assured: none of them has had an external
cryptographic audit.

#table(
  columns: 4,
  align: (left, left, left, left),
  table.header[Identifier and version][Answer kinds and forms][Input kinds][Issuer checking],
  [`urn:sparq:vcq:method:risc0-exact` v3 (RISC Zero program)], [Exact: SELECT, ASK,
    CONSTRUCT], [holder-declared, verifier-agreed], [none: `issuers` MUST be empty],
  [`urn:sparq:vcq:method:risc0-authenticated-rdf` v5 (RISC Zero program)], [Exact: SELECT,
    ASK, CONSTRUCT], [holder-declared, verifier-agreed], [`eddsa-rdfc-2022` against the
    request's keys, inside the proof],
  [`urn:sparq:vcq:method:noir-selected-support-unsigned` v2 (Noir circuits)], [Supported:
    SELECT (positive basic graph patterns with integer FILTERs)], [holder-declared], [Schnorr
    signatures over the sparq commitment format, inside the proof],
)

= Security and privacy considerations

== What an accepted answer does not establish

- #strong[Credential status.] Version 1 does not check whether a credential was revoked or
  suspended.
- #strong[Holder binding.] Version 1 does not show that the presenter is the credential subject
  or controls a key bound to the credential. Anyone who holds the credentials can answer.
- #strong[Truth of claims.] A valid issuer signature shows that the issuer made the claims, not
  that they are true #cite("VC-DATA-MODEL-2.0").
- #strong[Completeness beyond the input.] An Exact answer is complete over $D$ only. With a
  holder-declared input, no answer shows that the holder has no other relevant credential.
- #strong[Soundness of the prototype.] The proof systems in section 9 are research
  prototypes without an external audit. An accepted proof is evidence produced by that code,
  not a guarantee.

== What the verifier learns

The verifier learns the query (it wrote it), the result, the answer and input kinds, the proof
system used and the dataset commitment. A Supported answer also reveals that the returned rows
exist, and the size of the result can reveal more than its values (for example, the number of
payments that match). Requests with narrow results, such as an ASK, reveal least.

In the revealed signature mode the verifier also learns each credential's signature, signed
message and issuer key, and so the number of credentials. A signature and its signed message
are the same in every presentation of that credential, so verifiers can link presentations of
the same credential. When the signed message is an unsalted hash of the credential, as with
`eddsa-rdfc-2022`, a verifier who can guess a credential's full content can confirm the guess
by hashing it. A cryptosuite that salts what it signs avoids the second problem but not the
first. Whether signatures can be forged by an attacker with a quantum computer depends on the
cryptosuite, not on the mode: for Ed25519, the public key alone is enough.

A holder service SHOULD use a fresh salt for each holder-declared presentation. A reused
salt over the same credentials repeats the commitment, which lets verifiers link
presentations. A verifier-agreed commitment is linkable by design, to the verifier that agreed
it.

== Replay

A presentation is bound to one request digest, and through it to one challenge, audience and
validity period. The verifier accepts at most one presentation per request (section 8, final
step), so replaying a presentation, or sending it to another verifier, fails.

= Implementation status

This section is informative. It compares this document with the code in the sparq repository
on 2026-10-10.

What matches: the two RISC Zero proof systems in section 9 (`zk/sparql-evaluator`) prove the
statement of section 7 for Exact answers. Their request records hold the query, the input
kind and agreed commitment, the issuer key table (version 5) and a nonce; their public output
(the #emph[journal]) holds a request digest, the dataset commitment, the input kind and the
canonical result. The `sparq-query-protocol` crate holds the verifier-side request, the
admission check against declared proof-system capabilities and the one-time challenge store.
Through the adapters in `zk/sparql-evaluator/host` (features `vcq` and `vcq-authenticated`),
the challenge, audience and validity period reach the proof through a nonce derived from the
stored request.

What differs or is missing:

+ No code produces or parses the JSON objects of sections 4 and 5. The adapters pass Rust
  values, and the presentation is a descriptor digest plus the serialized receipt.
+ The request digest is computed over a project-specific binary encoding of the Rust values,
  not over JCS. Two implementations could not yet agree on a digest.
+ Results are in the proof's own canonical form (N-Triples term strings), not SPARQL Query
  Results JSON; the conversion of section 5.1 is not written.
+ Every proof system implements only the hidden signature mode. The revealed mode, and a
  cryptosuite that signs a Merkle root over a credential's RDF terms, are not built yet.
+ The Noir Supported proof system has its own request and result types and is not reached
  through the `sparq-query-protocol` adapters.
+ Only one end-to-end proof of the version 5 adapter has been made and independently checked
  (a verifier-agreed SELECT); its other five combinations have run only in tests without
  proving.

The older zkSPARQL proposal on this site describes the Noir proof manifest in detail. This
document replaces neither it nor the code; it fixes the data model that both sides should
converge on.

= Use with OpenID for Verifiable Presentations

This section is informative. It checks the data model against OpenID for Verifiable
Presentations 1.0 #cite("OID4VP") (OpenID4VP), the protocol most wallets and verifiers use to
request and return credentials. The conclusion is that OpenID4VP can carry a query request and
an answer presentation without changing either, by defining a new credential format. Section
12.2 lists the gaps.

== Mapping

OpenID4VP lets a deployment define a new credential format identifier, with its own `meta`
parameters in a DCQL credential query and its own presentation encoding. This document would
define one, here called
`sparql_answer`:

#table(
  columns: 2,
  align: (left, left),
  table.header[OpenID4VP element][Carries],
  [Authorization request `dcql_query`], [One credential query with `"format": "sparql_answer"`,
    `"multiple": false` and `"require_cryptographic_holder_binding": false`. Its `meta` object
    holds the query request members `query`, `baseIri`, `answerKind`, `input`, `issuers`,
    `signatureModes`, `proofSystems` and `limits`.],
  [Authorization request `nonce`], [The source of `challenge`: the holder service and the
    verifier set `challenge` to SHA-256 of the `nonce` string, which gives 32 bytes from
    OpenID4VP's string nonce.],
  [Authorization request `client_id`], [`audience`. Over the Digital Credentials API, the
    origin prefixed with `origin:`, as OpenID4VP requires.],
  [Request lifetime], [`notBefore` and `notAfter`, set by the verifier when it creates the
    request.],
  [`vp_formats_supported` metadata], [The proof systems each side supports, under the
    `sparql_answer` key, as a list of `{id, version}`.],
  [Response `vp_token`], [`{ "<credential query id>": [ <answer presentation> ] }`, with the
    answer presentation as a JSON object.],
  [Response mode], [`direct_post` or `direct_post.jwt`. Proofs can be large, so the fragment
    and query response modes are unsuitable.],
)

Both sides rebuild the query request from the authorization request and compute its request
digest. Because JCS is deterministic, they reach the same digest without the request being
sent twice.

A wallet that does not support `sparql_answer` finds no credential of that format and returns
an error (`access_denied` or `vp_formats_not_supported`). It cannot fall back to sending whole
credentials, because no other credential query was made. This matters because DCQL tells
implementations to ignore unknown properties: putting the query in an extra property of an
ordinary credential query would let an unaware wallet return the full credential.

== Gaps

+ #strong[A presentation from several credentials.] OpenID4VP defines a presentation as
  "derived from a Credential" and matches each one to a single credential query. An answer
  presentation is derived from a set of credentials. Carrying it as one `sparql_answer`
  presentation works, but stretches that definition. Wallet user interfaces that list "the
  credential being shared" would need to show the query and the answer instead.
+ #strong[Agreeing the input in advance.] A verifier-agreed input needs a step before the
  request in which the verifier accepts a dataset commitment. OpenID4VP has no such step. It
  could be a separate earlier exchange, but that is not specified anywhere.
+ #strong[Holder binding.] OpenID4VP requires cryptographic holder binding by default and binds
  replay protection to it. Version 1 has no holder binding, so the credential query sets
  `require_cryptographic_holder_binding` to false. Replay protection still holds because the
  proof binds the challenge and audience, but OpenID4VP then requires the request to carry
  `state` unless the Digital Credentials API is used.
+ #strong[Issuer trust.] DCQL's `trusted_authorities` only helps the wallet choose credentials;
  the verifier must check issuers itself. Here the issuer keys are in the request and the proof
  checks them, which fits, but there is no mapping from `trusted_authorities` (trust lists,
  OpenID Federation) to concrete keys.
+ #strong[Selective disclosure rules.] DCQL `claims` and `claim_sets` select claims to reveal.
  They have no meaning for an answer presentation and are omitted. The consent rules written
  for them do not cover a query, so wallets need their own way to show the user what the query
  reveals.
+ #strong[Registration.] `sparql_answer` is not a registered format identifier. Until it is,
  it works only between parties that agree on it, as OpenID4VP allows for deployment-defined
  formats.

None of these gaps needs a change to sections 4 and 5. They concern how the carrying protocol
presents and agrees inputs, not what the holder sends.

= References

#references((
  ("RFC2119", [Bradner, S. #emph[Key words for use in RFCs to Indicate Requirement Levels].
    RFC 2119, IETF, March 1997.]),
  ("RFC8174", [Leiba, B. #emph[Ambiguity of Uppercase vs Lowercase in RFC 2119 Key Words].
    RFC 8174, IETF, May 2017.]),
  ("RFC8259", [Bray, T. (ed). #emph[The JavaScript Object Notation (JSON) Data Interchange
    Format]. RFC 8259, IETF, December 2017.]),
  ("RFC3339", [Klyne, G.; Newman, C. #emph[Date and Time on the Internet: Timestamps].
    RFC 3339, IETF, July 2002.]),
  ("RFC7515", [Jones, M.; Bradley, J.; Sakimura, N. #emph[JSON Web Signature (JWS)].
    RFC 7515, IETF, May 2015. (Section 2 defines base64url without padding.)]),
  ("RFC8785", [Rundgren, A.; Jordan, B.; Erdtman, S. #emph[JSON Canonicalization Scheme
    (JCS)]. RFC 8785, IETF, June 2020.]),
  ("FIPS180-4", [NIST. #emph[Secure Hash Standard (SHS)]. FIPS PUB 180-4, August 2015.]),
  ("SPARQL11-QUERY", [Harris, S.; Seaborne, A. (eds). #emph[SPARQL 1.1 Query Language].
    W3C Recommendation, 21 March 2013. https://www.w3.org/TR/sparql11-query/.]),
  ("SPARQL11-RESULTS-JSON", [Seaborne, A. (ed). #emph[SPARQL 1.1 Query Results JSON Format].
    W3C Recommendation, 21 March 2013. https://www.w3.org/TR/sparql11-results-json/.]),
  ("RDF-CANON", [Longley, D.; Kellogg, G.; Yamamoto, D.; Sporny, M. (eds). #emph[RDF Dataset
    Canonicalization]. W3C Recommendation, 21 May 2024. https://www.w3.org/TR/rdf-canon/.]),
  ("VC-DATA-MODEL-2.0", [Sporny, M.; et al. (eds). #emph[Verifiable Credentials Data Model
    v2.0]. W3C Recommendation, 15 May 2025. https://www.w3.org/TR/vc-data-model-2.0/.]),
  ("VC-DI-EDDSA", [Sporny, M.; Longley, D.; et al. (eds). #emph[Data Integrity EdDSA
    Cryptosuites v1.0]. W3C Recommendation, 15 May 2025. https://www.w3.org/TR/vc-di-eddsa/.]),
  ("OID4VP", [Terbu, O.; Lodderstedt, T.; Yasuda, K.; Fett, D.; Heenan, J. #emph[OpenID for
    Verifiable Presentations 1.0]. OpenID Foundation, Final Specification, 9 July 2025.
    https://openid.net/specs/openid-4-verifiable-presentations-1_0.html.]),
))
