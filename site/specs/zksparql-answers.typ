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
  proof much cheaper at the cost of disclosing the signatures. Proof methods, which need not be
  zero-knowledge, plug in through an identifier and a version, as cryptosuites do for VC Data
  Integrity. The proof methods in the sparq repository are research prototypes and have not
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
  over that dataset, proves the result with a proof method the request accepts, and sends an
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
answer presentations; a #emph[proof method] (section 9) defines how one kind of proof is made
and checked.

This document uses the SPARQL 1.1 terms #emph[query form], #emph[solution mapping],
#emph[solution sequence] and #emph[RDF dataset] #cite("SPARQL11-QUERY"), and writes
$[| Q |]_D$ for the multiset of solution mappings of query $Q$ over RDF dataset $D$ under the
SPARQL 1.1 semantics. It also uses:

/ Input dataset: The RDF dataset a holder evaluates the query over, built from credentials as
  the proof method states (for example, the union of the credentials' graphs in the default
  graph, with blank nodes kept apart per credential).
/ Dataset commitment: A digest that fixes the input dataset without revealing it. It includes
  a random salt chosen by the holder, so equal datasets do not give equal commitments unless
  the same salt is reused.
/ Proof method: A way of producing and checking evidence for a SPARQL result, named by an
  identifier and a version, together with the artifact that checks it (a verification key, a
  program image or an attestation root). Section 9 defines what a method states.
/ Proof: The evidence a proof method produces, carried in the presentation's `proof` member.
  Depending on the method, it is a zero-knowledge proof, a proof that is not zero-knowledge, or
  a signed attestation.
/ Statement: The public values a proof is about: the request digest, the answer kind, the
  input kind, the dataset commitment, the result, the signature mode and, in the revealed
  mode, the signed messages.
/ Signed message: The bytes an issuer's signature is computed over, as the cryptosuite defines
  them. For `eddsa-rdfc-2022` #cite("VC-DI-EDDSA") it is the SHA-256 hash of the canonical
  proof configuration followed by the SHA-256 hash of the canonical credential document. The
  Merkle-root cryptosuites #cite("ZK-MERKLE-CRYPTOSUITE") (`eddsa-sha256-merkle-2026`,
  `schnorr-poseidon2-merkle-2026` and `mldsa44-sha256-merkle-2026`) sign a salted digest of the
  root of a Merkle tree over the credential's canonical quads, the number of quads and the
  proof configuration.

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
    has `issuer` (IRI), `verificationMethod` (IRI), `cryptosuite` (string, for example
    `"eddsa-rdfc-2022"`) and `publicKeyMultibase`, the key itself as the cryptosuite encodes
    it. Keys are given, never looked up, so both sides prove and verify against the same
    keys. An empty array means the verifier
    accepts an input dataset whose credentials are not checked against any issuer key.],
  [`signatureModes`], [array of strings], [REQUIRED if `issuers` is not empty, and absent
    otherwise. The signature modes the verifier accepts: one or more of `"hidden"`, `"revealed"` and
    `"disclosed"`.
    Section 6.3.],
  [`proofMethods`], [array of objects], [REQUIRED, non-empty. The proof methods the verifier
    accepts, in order of preference. Each entry has `id`, `version`, `artifact` and
    `parameters`: the proof method's settings that the statement depends on, such as its
    capacity bounds, in the encoding the proof method publishes (section 9). An empty
    object means the proof method's published defaults. Two entries MUST NOT have the same
    `id` and `version`, so that the pair identifies one entry, artifact and parameters
    included.],
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
+ The verifier service MUST list a proof method only if it holds that system's verification
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
  [`proofMethod`], [object], [REQUIRED. `id` and `version` of one entry in the request's
    `proofMethods`.],
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
  [`proof`], [byte string], [REQUIRED. The proof, in the encoding the proof method defines.],
)

Except in the disclosed signature mode (section 6.3), the presentation carries no credential
and no per-row provenance, and in the hidden signature mode it also carries no issuer
identity. What the verifier learns is listed in section 10.2.

== Result encoding

- A SELECT result is a SPARQL 1.1 Query Results JSON object #cite("SPARQL11-RESULTS-JSON")
  with `head.vars` and `results.bindings`. An unbound variable is omitted from its binding
  object, as that format specifies.
- An ASK result is a SPARQL 1.1 Query Results JSON object with `head` and `boolean`.
- A CONSTRUCT result is `{"ntriples": <string>}`: the constructed graph in canonical N-Triples
  under RDF Dataset Canonicalization (RDFC-1.0) #cite("RDF-CANON").
- A blank node in a SELECT result is given a label whose scope is this result. Two cells with
  the same label denote the same blank node. A result label has no relationship to any blank
  node label in a credential, even if the strings are equal.
- In an Exact SELECT result, `results.bindings` keeps duplicate solutions. Its order is the
  query's order if the query has ORDER BY; otherwise it is sorted by the proof method's
  canonical encoding of each solution and the order carries no meaning.
- In a Supported SELECT result, `results.bindings` has no duplicate solutions.

The proof is over the proof method's canonical encoding of the result, not over the JSON text.
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
bound of the proof method, the holder service MUST NOT return a truncated result.

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

A proof method may compute the dataset commitment differently in each signature mode. A
verifier-agreed commitment is then valid only for the mode it was computed under, and a
request that carries it MUST list only that mode in `signatureModes`.

Input kind is separate from issuer checking. If the request's `issuers` is not empty, the
proof MUST show that every credential in $D$ carries a valid proof from one of the listed keys,
whichever input kind is used. Checking issuers never turns a holder-declared input into a
verifier-agreed one.

== Signature modes

When the request lists issuer keys, the proof must show that every credential in $D$ is signed
by one of them. There are three ways to do that, and the verifier chooses which it accepts.

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

In the #emph[disclosed] mode, used only by proof methods whose evidence is disclosed
credentials (section 9), the `proof` member carries the credentials, or presentations derived
from them by a selective-disclosure cryptosuite such as `bbs-2023` #cite("VC-DI-BBS") or
`ecdsa-sd-2023` #cite("VC-DI-ECDSA"). The `signatures` member is absent. The verifier checks
them under their cryptosuite against a key in `issuers`, builds $D$ from what they disclose,
and evaluates the query itself. The verifier sees all the disclosed data, so a verifier lists
this mode only if it may see that data.

A proof method states which modes it supports for each cryptosuite (section 9). Other
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
    "cryptosuite": "eddsa-rdfc-2022",
    "publicKeyMultibase": "z6Mk…"
  }],
  "signatureModes": ["hidden"],
  "proofMethods": [{
    "id": "urn:sparq:vcq:method:risc0-authenticated-rdf",
    "version": 5,
    "artifact": { "imageId": "Yc9B…", "sha256": "1mE0…" },
    "parameters": {}
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
  "proofMethod": { "id": "urn:sparq:vcq:method:risc0-authenticated-rdf", "version": 5 },
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

A proof method MUST make the statement (section 2) part of what its proof proves, so that a
proof for one statement does not verify for another. In particular:

+ The request digest MUST be bound. Because the digest covers the query, `baseIri`, answer kind,
  input, issuers, accepted proof methods, limits, challenge, audience and validity period, a
  proof made for one request does not verify against another, and a proof made for one
  verifier does not verify for another.
+ The result MUST be bound. The verifier takes the result only from what the proof proves.
+ The input kind and the dataset commitment MUST be bound.
+ The signature mode MUST be bound. In the revealed mode, every signed message MUST be bound,
  and the proof MUST show that $D$ is built from exactly the data those messages cover, so that
  a verifier who checks the signatures outside the proof knows they cover $D$.
+ The proof method identifier and version MUST select the verification artifact. A verifier
  MUST NOT take a verification key or program from the presentation.

A proof method MAY bind these values directly as public inputs, or bind a single digest over
them, as long as the verifier can recompute every bound value from its stored request and the
presentation.

= Verifier processing

A verifier service processes an answer presentation in this order and rejects it at the first
check that fails. It returns no partial result and no warning instead of a rejection.

+ Before parsing, reject a presentation larger than the verifier service's own fixed ceiling,
  which is at least the largest `limits.maxPresentationBytes` among its stored requests. If the
  transport already identifies the request (as OpenID4VP does through `state` or the Digital
  Credentials API call), use that request's `limits.maxPresentationBytes` instead.
+ Parse the presentation and reject it if it does not follow sections 3 and 5.
+ Find the stored request whose request digest equals `requestDigest`. Reject if there is none,
  if the transport identified a different request, if the current time is outside
  `notBefore` to `notAfter`, or if a presentation for this request was already accepted.
  Reject if the encoded presentation is larger than that request's
  `limits.maxPresentationBytes`.
+ Reject unless `proofMethod` names an entry of the request's `proofMethods`. Load that
  entry's verification artifact from the verifier's own configuration, and reject unless it
  matches the entry's `artifact` (for example, its digest or image identifier).
+ Reject unless `answerKind` and `input.kind` equal the request's. For a verifier-agreed input,
  reject unless `input.commitment` equals the request's.
+ If the request has `signatureModes`, reject unless `signatureMode` is one of them. In the
  revealed mode, reject unless every entry of `signatures` verifies under its cryptosuite, with
  a `verificationMethod` and `cryptosuite` that match an entry of the request's `issuers`. In
  the disclosed mode, the proof method's checks in the step below verify the disclosed
  credentials in the same way.
+ Reject if the result breaks section 5.1 or 6.1: the wrong shape for the query form, a
  Supported ASK of `false`, an empty Supported SELECT, duplicate rows in a Supported SELECT, or
  more than `limits.maxResultRows` rows or triples.
+ Verify the proof with the proof method over the statement recomputed from the stored request
  and the presentation. Reject if it fails.
+ Mark the request as answered, in one atomic step that fails if it was already marked. Only
  then accept.

On acceptance the verifier service has: the query, the result, the answer kind, the input kind,
the dataset commitment and the issuer keys from its own request. A verifier service SHOULD keep
these together, so that later use of the result also records what it guarantees.

= Proof methods

A #dfn[proof method] is a way of producing and checking evidence that the statement
(section 2) holds. It plays the part for query answers that a cryptosuite plays for VC Data
Integrity #cite("VC-DATA-INTEGRITY"): the presentation names one by identifier and version,
and the verifier accepts only the methods its request lists.

A proof method need not be zero-knowledge. Besides zero-knowledge proofs, a method may produce
a proof that hides nothing, an attestation signed by a trusted execution environment (TEE)
stating that a measured program evaluated the query over the input and checked the
signatures, or the signed credentials themselves (in full or selectively disclosed) for the
verifier to check and evaluate the query over. What a verifier relies on differs between
these, so each method states it explicitly.

Each proof method MUST publish:

- its identifier (an IRI) and version (a positive integer);
- the #strong[kind of evidence]: a zero-knowledge proof, a proof that is not zero-knowledge,
  an attestation, or disclosed credentials;
- whether the evidence is #strong[transferable] (anyone holding it and the request can check
  it) or #strong[designated-verifier] (it convinces only the verifier that took part);
- #strong[what it shows]: the statement it binds (section 7), and the query forms, answer
  kinds, input kinds and SPARQL fragment it supports (a method MAY support only part of
  SPARQL 1.1);
- #strong[what the verifier must trust] for an accepted proof to mean the statement holds: for
  example the proof system's soundness and any trusted setup, or for an attestation the
  hardware vendor's attestation key, the measured program and the TEE's resistance to physical
  and side-channel attacks;
- #strong[what the verifier learns] beyond the statement: for a zero-knowledge proof, nothing
  the method's privacy argument does not allow; for other evidence, everything it reveals,
  such as a platform identity in a TEE attestation;
- the cryptosuites it can check, how it builds the input dataset from credentials, and the
  signature modes it supports for each cryptosuite;
- the form of the `artifact` member that pins its verification key, program or attestation
  root, including byte order where the identifier is a sequence of words;
- the form of its `parameters` member and its defaults;
- the canonical encoding of the statement and how the evidence binds it (section 7); for an
  attestation, the statement digest MUST be in the signed report;
- the encoding of the `proof` member and any size or capacity bounds.

A proof method MAY be interactive, with the verifier taking part in producing the evidence.
Such a method defines the channel and the messages, MUST bind the request digest into the
protocol transcript, and defines what the `proof` member then carries (for example the
transcript, or an identifier of the completed session). For such a method, the step of
section 8 that verifies the proof means completing the protocol and checking its outcome.

A new version of a proof method is a new proof method: a verifier that accepts version 5 does
not thereby accept version 6. A method is defined by what the verifier checks, not by how the
evidence is produced: the same proof produced on a CPU or on a GPU or other proving
accelerator uses the same method. A new circuit or program for the same method, for example a
circuit compiled directly to ACIR instead of from Noir, has a new verification artifact, and
the verifier pins that artifact in `artifact`.

The proof methods currently in the sparq repository are below. This table is informative and
records what the code does, not what is assured: none of them has had an external
cryptographic audit.

#table(
  columns: (1.5fr, 1fr, 1fr, 1fr),
  align: (left, left, left, left),
  table.header[Identifier and version][Answer kinds and forms][Input kinds][Issuer checking],
  [`urn:sparq:vcq:method:` \ `risc0-exact` v3 (RISC Zero program)], [Exact: SELECT, ASK,
    CONSTRUCT], [holder-declared, verifier-agreed], [none: `issuers` MUST be empty],
  [`urn:sparq:vcq:method:` \ `risc0-authenticated-rdf` v5 (RISC Zero program)], [Exact: SELECT,
    ASK, CONSTRUCT], [holder-declared, verifier-agreed], [`eddsa-rdfc-2022` against the
    request's keys, inside the proof],
  [`urn:sparq:vcq:method:` \ `noir-selected-support-unsigned` v2 (Noir circuits)], [Supported:
    SELECT (positive basic graph patterns with integer FILTERs)], [holder-declared; not usable
    with version 1 of this document yet (section 11)], [Schnorr signatures over the sparq
    commitment format, inside the proof],
)

The following methods are proposed and not built. Their identifiers use the same prefix and
version 1.

#table(
  columns: (1.5fr, 1fr, 2fr),
  align: (left, left, left),
  table.header[Identifier][Evidence kind][What it is],
  [`disclosed-reevaluation`], [disclosed credentials], [The holder sends the signed
    credentials and the salt; the verifier checks the signatures and evaluates the query
    itself. Its `input.commitment` is the dataset commitment of `risc0-authenticated-rdf`
    version 5 over the same credentials and salt, so one verifier-agreed commitment serves both
    methods. A baseline for comparison: it hides nothing.],
  [`selective-disclosure-reevaluation`], [disclosed credentials, selectively], [The holder
    discloses, with `bbs-2023` or `ecdsa-sd-2023`, the claims each returned solution uses,
    together with any claims the issuer made mandatory to disclose and the structure the
    cryptosuite needs; the verifier checks them and evaluates the query over them. The holder
    service must check everything a derived presentation discloses before sending it. Supported answers only,
    for queries whose solutions remain solutions when data is added (no negation, OPTIONAL or
    aggregation).],
  [`vole-designated-verifier`], [zero-knowledge proof, interactive, designated-verifier],
  [An interactive proof based on vector oblivious linear evaluation (VOLE), as in QuickSilver.
    It convinces only the verifier that took part.],
  [`tee-attestation`], [attestation], [A program running in a TEE evaluates the query and
    checks the signatures; the TEE's signed report contains the statement digest. The
    `parameters` member names the platform: `intel-tdx`, `amd-sev-snp`, `aws-nitro` or
    `nvidia-cc`.],
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
- #strong[More than the method's trust assumptions.] An accepted proof means the statement
  holds only if what the method says the verifier must trust (section 9) holds. The proof
  methods in section 9 are research prototypes without an external audit; an accepted proof is
  evidence produced by that code, not a guarantee.

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
by hashing it. The Merkle-root cryptosuites sign a salted digest, which avoids the second
problem but not the first. Whether signatures can be forged by an attacker with a quantum computer depends on the
cryptosuite, not on the mode: for Ed25519, the public key alone is enough.

In the disclosed signature mode the verifier learns everything the disclosed credentials or
derived presentations contain, including claims the issuer made mandatory to disclose.

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

What matches: the two RISC Zero proof methods in section 9 (`zk/sparql-evaluator`) prove the
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
+ The version 5 RISC Zero method implements only the hidden signature mode. Its revealed
  mode and the Merkle-root cryptosuites are written but not yet merged. The Noir circuits
  support both modes, but that method cannot yet produce a version 1 presentation (below).
+ The Noir Supported proof method cannot produce a version 1 presentation. It keeps graph
  roots and salts private, so it has no dataset commitment to publish, and it requires a
  credential-status root that version 1 has no member for. It needs a new version of its
  circuits that publishes and binds a dataset commitment, not only an encoder. It is also not
  reached through the `sparq-query-protocol` adapters.
+ The version 5 policy (issuer keys and capacity bounds) has no JSON encoding yet, so the
  mapping from `issuers` and `parameters` to the adapter's parameter digest is not defined.
+ The adapters accept a narrower query surface than section 4: they reject a `baseIri` and
  SELECT queries with ORDER BY. Their request stores times as Unix seconds, so the RFC 3339
  strings need a fixed conversion.
+ The RISC Zero methods' `imageId` is eight 32-bit words; the code encodes it as the words in
  order, each little-endian. A wire profile has to fix that encoding.
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
    `signatureModes`, `proofMethods`, `limits`, `notBefore` and `notAfter`, as the exact
    strings and values of the stored request.],
  [Authorization request `nonce`], [The source of `challenge`: the holder service and the
    verifier set `challenge` to SHA-256 of the `nonce` string, which gives 32 bytes from
    OpenID4VP's string nonce.],
  [Authorization request `client_id`], [`audience`. Over the Digital Credentials API, the
    origin prefixed with `origin:`, as OpenID4VP requires.],
  [`vp_formats_supported` metadata], [The proof methods each side supports, under the
    `sparql_answer` key, as a list of `{id, version}`.],
  [Response `vp_token`], [`{ "<credential query id>": [ <answer presentation> ] }`, with the
    answer presentation as a JSON object.],
  [Response mode], [Over HTTPS redirects, `direct_post` or `direct_post.jwt`; proofs can be
    large, so the fragment and query modes are unsuitable. Over the Digital Credentials API,
    `dc_api` or `dc_api.jwt`, as OpenID4VP requires there.],
)

Both sides rebuild the query request from `meta`, the nonce and the audience, and compute its
request digest. Every member is either carried exactly or derived by a fixed rule, and JCS is
deterministic, so both reach the same digest.

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
  ("ZK-MERKLE-CRYPTOSUITE", [The sparq project. #emph[Merkle-Root Cryptosuites for RDF
    Verifiable Credentials]. Unofficial Proposal Draft, 2026. Published on this site as
    `zk-merkle-cryptosuite`.]),
  ("VC-DATA-INTEGRITY", [Sporny, M.; Longley, D.; et al. (eds). #emph[Verifiable Credential
    Data Integrity 1.0]. W3C Recommendation, 15 May 2025. https://www.w3.org/TR/vc-data-integrity/.]),
  ("VC-DI-BBS", [Sporny, M.; Longley, D.; et al. (eds). #emph[Data Integrity BBS Cryptosuites
    v1.0]. W3C. https://www.w3.org/TR/vc-di-bbs/.]),
  ("VC-DI-ECDSA", [Sporny, M.; Longley, D.; et al. (eds). #emph[Data Integrity ECDSA
    Cryptosuites v1.0]. W3C Recommendation, 15 May 2025. https://www.w3.org/TR/vc-di-ecdsa/.]),
  ("VC-DI-EDDSA", [Sporny, M.; Longley, D.; et al. (eds). #emph[Data Integrity EdDSA
    Cryptosuites v1.0]. W3C Recommendation, 15 May 2025. https://www.w3.org/TR/vc-di-eddsa/.]),
  ("OID4VP", [Terbu, O.; Lodderstedt, T.; Yasuda, K.; Fett, D.; Heenan, J. #emph[OpenID for
    Verifiable Presentations 1.0]. OpenID Foundation, Final Specification, 9 July 2025.
    https://openid.net/specs/openid-4-verifiable-presentations-1_0.html.]),
))
