// Merkle-root cryptosuites for RDF verifiable credentials. Normative computations
// mirror zk/sparql-evaluator/model/src/merkle_suite.rs; the fixed vectors in
// section 10 are asserted by zk/sparql-evaluator/model/tests/merkle_suite.rs.

#import "_lib/spec.typ": spec-head, sotd, intro-section, references, dfn, note, cite

#set document(title: "Merkle-Root Cryptosuites for RDF Verifiable Credentials")
#set text(size: 11pt)
#set par(justify: true)
#set heading(numbering: "1.")
#show table: set par(justify: false)
#set table(align: left)

#spec-head()

#intro-section("abstract", "Abstract")[
  This document defines Data Integrity cryptosuites for RDF verifiable credentials
  in which the issuer signs a salted Merkle root over the credential's quads. Each
  leaf hashes a typed encoding of one quad. A literal's encoding carries a value
  comparison key: an order-preserving encoding of its value under the SPARQL
  relational operators, for numbers, date-times, booleans and `xsd:string`
  prefixes. A zero-knowledge proof can then authenticate a credential
  without canonicalizing it, and compare typed values without parsing lexical
  forms. The suite family has three members that share the encoding and the
  tree shape: `eddsa-sha256-merkle-2026` for hash-accelerated zkVMs,
  `schnorr-poseidon2-merkle-2026` for arithmetic circuits, and the post-quantum
  `mldsa44-sha256-merkle-2026`.
]

#sotd()

= Introduction

The `eddsa-rdfc-2022` cryptosuite #cite("VC-DI-EDDSA") signs hashes of the
RDFC-1.0 canonical form #cite("RDF-CANON") of a credential. A proof of a SPARQL
answer #cite("SPARQL11-QUERY") over such a credential must either repeat the
canonicalization inside the proof or trust a canonical form supplied by the
holder. It must also parse every literal it compares.

The suites in this document move canonicalization to the issuer. The issuer
signs a Merkle tree over the canonical quads, so a proof only hashes the quads
it is given and checks they form the signed tree. Each literal leaf carries a
fixed-width comparison key, derived from the lexical form, that compares in value
order as bytes or as a field element.

The suites are designed for the two signature modes of the ZK SPARQL answers
specification #cite("ZKSPARQL-ANSWERS"). In `hidden` mode the proof verifies the
signature. In `revealed` mode the holder presents the signature and the proof
outputs the signed message. Because the message is salted, it does not let a
verifier test guesses about the credential's content.

== Conformance

The key words MUST, MUST NOT, SHOULD and MAY are to be interpreted as described
in RFC 2119 #cite("RFC2119") when, and only when, they appear in capitals.

== Suite family

#table(
  columns: (auto, auto, auto, 1fr),
  [*Identifier*], [*Tree hash*], [*Signature*], [*Implementation status*],
  [`eddsa-sha256-merkle-2026`], [SHA-256], [Ed25519 #cite("RFC8032")],
    [Implemented in the sparq zkVM relation and its issuer function],
  [`schnorr-poseidon2-merkle-2026`], [Poseidon2 over BN254], [Schnorr over Baby Jubjub],
    [Specified here; not yet implemented],
  [`mldsa44-sha256-merkle-2026`], [SHA-256], [ML-DSA-44 #cite("FIPS204")],
    [Specified here; not yet implemented],
)

= Data model

A credential secured with one of these suites carries a `DataIntegrityProof`
#cite("VC-DATA-INTEGRITY") with:

- `cryptosuite`: one identifier from the table above;
- `verificationMethod`: the issuer's key;
- `proofPurpose`: `assertionMethod`;
- optionally `created`;
- `proofValue`: the base58-btc multibase encoding (prefix `z`) of the signature
  bytes followed by the 32-byte tree salt.

The salt is secret to the issuer and the holder. A holder MUST NOT disclose the
salt to a verifier.

= Encoding

== Canonical quads

The issuer canonicalizes the unsecured credential with RDFC-1.0 using SHA-256.
The result is a set of quads with canonical blank node labels. Blank nodes keep
those labels in their leaves. Quads containing an RDF 1.2 triple term are not
supported.

== Term encoding

All lengths are 32-bit unsigned big-endian integers, and strings are UTF-8.
The #dfn[term encoding] `enc(t)` of a term `t` is:

- an IRI: byte `0x01`, then the length and bytes of the IRI;
- a blank node: byte `0x02`, then the length and bytes of its label without `_:`;
- a literal: byte `0x03`, then the length and bytes of the lexical form, of the
  datatype IRI, and of the language tag in lower case (empty when there is
  none), then the 31-byte comparison key;
- the default graph, as a graph name: byte `0x00`.

== Comparison keys

A literal's #dfn[comparison key] is one class byte and 30 payload bytes,
computed from its datatype and lexical form only. It is an order-preserving
encoding of the literal's value: for two literals whose keys have the same class
from `0x01` to `0x06`, comparing the keys as unsigned big-endian integers gives
the result of the SPARQL operators `=`, `!=`, `<`, `>`, `<=` and `>=` on the
literals #cite("SPARQL11-QUERY"). Each class holds only datatypes that SPARQL
compares with one another.

- Keys of different classes MUST NOT be compared.
- A class `0x07` key decides an order only when the two prefixes differ.
- Keys of class `0x00` or `0x7F` decide nothing.
- Equal keys do not make two literals the same RDF term. `sameTerm`, joins and
  `DISTINCT` use the whole term encoding.
- A function such as `STRSTARTS` that has its own argument compatibility rules
  MUST apply them before it uses a key.

In every other case, a proof uses the lexical form.

#table(
  columns: (auto, 1fr, 1.6fr),
  [*Class*], [*Literals*], [*Payload*],
  [`0x01`], [valid `xsd:integer` and `xsd:decimal`],
    [the value times 10#super[18] plus 2#super[239], as 30 bytes; integers and
     decimals with equal values share a key],
  [`0x02`], [valid `xsd:double` and `xsd:float`],
    [8 bytes: the IEEE 754 binary64 value's bits, with every bit inverted when the
     sign bit is set and only the sign bit inverted otherwise. A float is widened
     to binary64 first, and negative zero is encoded as zero.],
  [`0x03`], [`xsd:dateTime` with a timezone],
    [8 bytes: milliseconds since 1970-01-01T00:00:00Z plus 2#super[63]],
  [`0x04`], [`xsd:dateTime` without a timezone], [as class `0x03`, read as UTC],
  [`0x05`], [`xsd:boolean`], [one byte: 0 for `false` or `0`, 1 for `true` or `1`],
  [`0x06`], [`xsd:string` of at most 30 bytes and no U+0000],
    [the lexical bytes],
  [`0x07`], [other `xsd:string`], [the first 30 lexical bytes],
  [`0x7F`], [valid values the key cannot represent],
    [none: decimals with more than 18 fractional digits or a magnitude of at least
     2#super[239]/10#super[18]; NaN; date-times whose year is not four digits from
     0001, that end at 24:00:00, or that have sub-millisecond precision],
  [`0x00`], [every other literal, including `rdf:langString` and ill-typed
    literals], [none],
)

Payload bytes not listed are zero. For class `0x06`, byte order is code point
order because UTF-8 preserves it. Language-tagged strings get no key because
SPARQL defines no order on them. A date-time's validity, including 29 February,
follows the proleptic Gregorian calendar of its actual year, however many
digits it has.

== Leaves and tree

For `eddsa-sha256-merkle-2026` and `mldsa44-sha256-merkle-2026`:

+ The leaf of a quad `(s, p, o, g)` is
  `SHA-256(0x00 || enc(s) || enc(p) || enc(o) || enc(g))`.
+ The leaves are sorted as unsigned 32-byte big-endian integers. Equal leaves
  MUST NOT occur.
+ The row of leaves is padded with all-zero 32-byte leaves to the next power of
  two.
+ Each inner node is `SHA-256(0x01 || left || right)`. The root is the single
  remaining node. A tree with one leaf has that leaf as its root.

The `0x00` and `0x01` prefixes separate leaves from inner nodes, as in
Certificate Transparency #cite("RFC6962").

== Signed message

Let `n` be the number of quads, `root` the tree root, `salt` the 32-byte salt,
and `config` the SHA-256 digest of the RDFC-1.0 canonical proof configuration
(the proof without `proofValue`). The #dfn[signed message] is the 32 bytes

```
SHA-256(id || 0x00 || salt || n as u32 big-endian || root || config)
```

where `id` is the ASCII suite identifier.

= Algorithms

== Proof creation

+ Canonicalize the credential and the proof configuration with RDFC-1.0.
+ Draw a fresh, uniformly random 32-byte salt.
+ Compute the leaves, the root and the signed message.
+ Sign the message: with Ed25519 for `eddsa-sha256-merkle-2026`, or with
  ML-DSA-44 (pure, empty context) for `mldsa44-sha256-merkle-2026`.
+ Set `proofValue` to the multibase encoding of the signature followed by the
  salt.

== Proof verification

A verifier that holds the whole credential, including the salt, recomputes the
signed message from the canonical quads and configuration and verifies the
signature. Ed25519 verification MUST reject non-canonical encodings and
small-order keys.

== Use in proofs of query answers

A proof system that evaluates a SPARQL query over credentials secured with these
suites receives the quads in leaf order, the configuration and the salt as
private input. It recomputes every leaf from the terms, checks that the leaves
are strictly increasing, and rebuilds the signed message. The answer is then
computed over exactly those quads. Because every comparison key is recomputed
from its lexical form, a malformed key in a credential makes the proof fail. A
holder can hold the same credential under several salts, so a proof that rejects
duplicate credentials MUST identify a credential by its quad count and unsalted
root, not by its signed message.

- In `hidden` mode the signature is also private input, and the proof verifies
  it against a key the verifier authorized.
- In `revealed` mode the proof outputs the signed message and the verification
  method. The verifier checks the presented signature over that message. The
  holder reveals only the signature bytes, never the full `proofValue`: the tree
  salt MUST stay private to the holder, because a verifier that learns it can
  confirm guesses of the credential's content. The
  verifier learns a salted digest, so it cannot confirm guesses about the
  credential, but presentations of the same credential are linkable through the
  message and the signature.

A circuit that compares keys without recomputing them from lexical forms relies
on the issuer to have computed them correctly. Such a circuit SHOULD recompute
the keys it compares.

= The Poseidon2 member

`schnorr-poseidon2-merkle-2026` uses field elements of the BN254 scalar field
and the Poseidon2 sponge `P` with width 4, rate 3, 8 full and 56 partial rounds,
as defined by the zkSPARQL draft #cite("ZKSPARQL") and Noir's standard library.

- `h(x)` is SHA-256 of the bytes `x`, keeping its low 31 bytes as an integer.
- The field term encoding is `encF(t) = P(k, h(enc'(t)), key(t))`, where `k` is
  the term's tag byte, `enc'(t)` is `enc(t)` without the comparison key, and
  `key(t)` is the 31-byte comparison key as an integer (zero for non-literals).
- A leaf is `P(1, encF(s), encF(p), encF(o), encF(g))`, and an inner node is
  `P(2, left, right)`. The padding leaf is zero, and leaves are sorted as
  integers.
- The signed message is `P(3, salt, n, root, c1, c0)`, where `salt` is the
  32-byte salt reduced modulo the field order and `c1`, `c0` are the high and
  low 128 bits of the configuration digest.
- The signature is a Schnorr signature over Baby Jubjub with a Poseidon2
  challenge, as defined by the zkSPARQL draft's issuer attestation.

A circuit compares two class-`0x01` keys with one range check on their
difference, instead of opening a hashed lexical form.

#note[This member has no implementation in this revision. Its constants and
message layout may change when it is implemented and given fixed vectors.]

= Security and privacy considerations

#table(
  columns: (auto, 1fr, 1fr, 1fr),
  [*Property*], [*eddsa-sha256*], [*schnorr-poseidon2*], [*mldsa44-sha256*],
  [Unforgeability, classical], [Ed25519], [Discrete log on Baby Jubjub], [ML-DSA-44],
  [Unforgeability, post-quantum], [None], [None], [ML-DSA-44],
  [Tree binding], [SHA-256 collision resistance], [Poseidon2 collision resistance],
    [SHA-256 collision resistance],
  [Revealed message], [Salted SHA-256], [Salted Poseidon2], [Salted SHA-256],
)

- *Hidden mode.* What the verifier learns beyond the answer depends only on the
  proof system's zero-knowledge property, not on the suite.
- *Revealed mode.* The verifier learns the signed message, the signature and the
  verification method. These are the same in every presentation of a credential,
  so such presentations are linkable. The salt prevents the verifier from
  confirming guessed content. An `eddsa-rdfc-2022` presentation in revealed mode
  exposes unsalted document hashes, so it does allow such guesses.
- *Salt.* A reused or low-entropy salt removes that protection. Issuers MUST use
  a fresh salt for each credential.
- *Quantum adversaries.* A quantum adversary able to compute discrete logarithms
  can forge Ed25519 and Baby Jubjub signatures. Only the ML-DSA-44 member resists
  forgery by such an adversary. The hash-based tree and the salted message keep
  their binding and hiding against quantum adversaries, with security margins
  reduced by generic quantum search.
- *Comparison keys.* Keys add no information beyond the lexical form. They are
  disclosed only where a proof discloses a leaf.
- *Completeness.* A signature covers one credential. A holder can still choose
  which credentials to present unless the verifier fixes them in advance.

= Test vectors

The quad
`_:c14n0 <http://ex/balance> "1250.50"^^<http://www.w3.org/2001/XMLSchema#decimal> .`
has the leaf

```
8c4a491021d82e07c77c748df8c123d9721163cb8da614c7d21a4aef901c1146
```

Together with `<urn:vc:1> <http://ex/name> "Alice"@en .`, the root is

```
311072a0f565868c243e392ea085e4c2fe752b39751e0707bcd7e42c78683682
```

With 32 bytes of `0x07` as the salt, a quad count of 2 and 32 bytes of `0x09` as
the configuration digest, the `eddsa-sha256-merkle-2026` signed message is

```
d79eb8afe1cbb6c7c146089a2522aa54a6a875176585d135a39d129d50be81a8
```

= References

#references((
  ("FIPS204", [Module-Lattice-Based Digital Signature Standard. NIST FIPS 204, 2024.]),
  ("RDF-CANON", [RDF Dataset Canonicalization. W3C Recommendation, 2024.]),
  ("RFC2119", [Key words for use in RFCs to Indicate Requirement Levels. IETF RFC 2119.]),
  ("RFC6962", [Certificate Transparency. IETF RFC 6962.]),
  ("RFC8032", [Edwards-Curve Digital Signature Algorithm (EdDSA). IETF RFC 8032.]),
  ("SPARQL11-QUERY", [SPARQL 1.1 Query Language. W3C Recommendation, 2013.]),
  ("VC-DATA-INTEGRITY", [Verifiable Credential Data Integrity 1.0. W3C Recommendation, 2025.]),
  ("VC-DI-EDDSA", [Data Integrity EdDSA Cryptosuites v1.0. W3C Recommendation, 2025.]),
  ("ZKSPARQL", [zkSPARQL: Zero-Knowledge Query Proofs over SPARQL. sparq Unofficial Proposal Draft.]),
  ("ZKSPARQL-ANSWERS", [Zero-knowledge SPARQL answers: query requests and answer presentations. sparq Unofficial Proposal Draft.]),
))
