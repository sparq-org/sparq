// Merkle-root cryptosuites for RDF verifiable credentials. Normative computations
// mirror crates/sparq-merkle-suite and zk/noir-query/core. The frozen SHA vectors
// also remain asserted by zk/sparql-evaluator/model/tests/merkle_suite.rs.

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
  without canonicalizing it, and reuse a checked key for typed comparisons.
  The suite family has three members that share the encoding and the
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
    [Implemented in the native suite and Noir credential library; research grade and externally unaudited],
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
salt to a verifier. For the Poseidon2 member it is the canonical, unsigned
32-byte big-endian encoding of a uniformly sampled field element; encodings at
or above the field modulus MUST be rejected. The SHA-256 members use uniformly
sampled byte salts.

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
from `0x01` to `0x06`, or class `0x08`, comparing the keys as unsigned big-endian integers gives
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
  [`0x01`], [valid `xsd:integer`, its built-in derived datatypes, and `xsd:decimal`],
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
  [`0x08`], [timezone-free `xsd:date`],
    [8 bytes: proleptic Gregorian days since 1970-01-01 plus 2#super[63]],
  [`0x7F`], [valid values the key cannot represent],
    [none: decimals with more than 18 fractional digits or a magnitude of at least
     2#super[239]/10#super[18]; NaN; date-times whose year is not four digits from
     0001, that end at 24:00:00, or that have sub-millisecond precision; dates with a timezone or a year outside the date-time profile],
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

For the SHA-256 members, let `n` be the number of quads, `root` the tree root,
`salt` the 32-byte salt,
and `config` the SHA-256 digest of the RDFC-1.0 canonical proof configuration
(the proof without `proofValue`). The #dfn[signed message] is the 32 bytes

```
SHA-256(id || 0x00 || salt || n as u32 big-endian || root || config)
```

where `id` is the ASCII suite identifier.

= Algorithms

== Proof creation

The SHA-256 members use the following steps. The Poseidon2 member uses its
field-native tree and message below, samples a canonical field salt, and signs
with the existing Schnorr-over-Baby-Jubjub scheme instead.

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
suites receives the quads in the member's canonical order, the configuration and
the salt as private input. It recomputes every leaf from the terms, checks strict
leaf order for the SHA-256 members or strict quad-hash order for the Poseidon2
member, and rebuilds the signed message. The answer is then computed over exactly
those quads. Whenever a proof consumes a comparison key for value semantics, it
MUST recompute the key from a bound lexical opening; a mismatching key makes the
proof fail. A holder can hold the same credential under several salts, so a proof
that rejects duplicate credentials MUST compare their private canonical quad
sequences, or the SHA-256 member's unsalted root, rather than their salted messages. Any order the proof makes public, such as the
order of revealed messages, MUST depend only on salted values; an order by the
unsalted root would let a verifier compare a private credential with a known
one.

- In `hidden` mode the signature is also private input, and the proof verifies
  it against a key the verifier authorized.
- In `revealed` mode the proof outputs the signed message; the verification
  method is carried by the outer presentation. The verifier checks the presented
  signature over that message. The
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

`schnorr-poseidon2-merkle-2026` is implemented by the native
`sparq-merkle-suite` crate and `zk/noir-query/core`. It is research grade and
externally unaudited. It uses the existing Poseidon2 sponge over the BN254 scalar
field, with width 4, rate 3, 8 full and 56 partial rounds, and the existing Schnorr
over Baby Jubjub relation. The older commitment methods are unchanged.

== Domains and byte packing

A domain is the unsigned big-endian integer of its eight-byte ASCII label:
`MS26TERM`, `MS26QUAD`, `MS26SALT`, `MS26LEAF`, `MS26NODE`, `MS26MESG`,
`MS26TYPE`, `MS26LANG` and `MS26SUIT`. These denote term bytes, quad hashes,
leaf-salt derivation, salted leaves, tree nodes, signed messages, datatype
identifiers, language identifiers and suite identifiers respectively.

Let `B(domain, bytes)` be `P(domain, byte_length, chunks...)`. Pack each consecutive
group of at most 31 bytes as an unsigned big-endian field integer; the last group
retains its actual width, with no appended zero bytes. The byte length is always
absorbed. An empty byte string has no chunks. These integers fit strictly below
the field modulus, and the existing sponge also binds its field-input length.
A bounded lexical opening MUST constrain all bytes beyond its length to zero.
Changing a circuit's byte capacity does not change the hash.

== Terms, quads and salted trees

The field encoding is `encF(t) = P(kind(t), B(term_domain, enc'(t)), key(t))`,
where `enc'(t)` is the typed byte encoding without its comparison key and the
complete key is one field integer, zero for non-literals. Datatype and language
identifiers use `B` under their own domains. A term cell MUST bind its kind,
encoding, key class, key, datatype and language identifiers to the same opening.
Blank-node cells also carry the credential's dataset position; equality includes
that scope. Triple terms are not admitted.

A quad hash is `q = P(quad_domain, encF(s), encF(p), encF(o), encF(g))`.
The issuer orders quads by their canonical unsigned `q` values, strictly
increasing, rejecting duplicates. For the zero-based index `i`, derive
`salt_i = P(salt_domain, salt, i)` and the leaf
`P(leaf_domain, q_i, salt_i)`. The private tree salt is a fresh uniformly sampled
canonical field element. It MUST NOT appear as a public input or output.
Pad leaves with zero to the next power of two and hash inner nodes as
`P(node_domain, left, right)`. A sole leaf is its own root. The tree is the
minimal tree for the exact positive count, independent of compiled capacity.
Padding quad cells and leaf-salt witness slots MUST be canonical zero.

== Message and signature modes

The suite identifier is `B(suite_domain, "schnorr-poseidon2-merkle-2026")`.
Let `c1` and `c0` be the unsigned high and low 128-bit halves of the SHA-256
digest of the canonical proof configuration. The signed message is
`P(message_domain, suite, salt, n, root, c1, c0)`. A signature therefore binds
the suite, private salt, exact quad count, salted root and complete configuration.
Use the existing Poseidon2-challenge Schnorr signer and verifier over Baby Jubjub.

In hidden mode, the issuer-key coordinates and verifier-agreed configuration
digest halves are public inputs; the circuit checks the signature and releases
no credential output. In revealed mode, the agreed configuration digest halves
are public inputs and the sole credential output is the salted signed message.
The verifier checks its signature externally. Both modes keep term openings,
keys, quad hashes, count, root, tree salt, leaf salts and signature witnesses
private. No public credential value is an unsalted hash or ordered by one.

== Lexical comparison-key checks

Every value-consuming gadget MUST open the lexical bytes and recompute both
comparison-key class and payload. Only that constructor produces a key-checked
cell. Integer-derived built-in datatypes use the decimal class after checking
their datatype's value bounds. XML Schema supplies no other built-in non-integer
derivatives of decimal; unknown custom datatype IRIs have no key.

Timezone-free dates get class `0x08`. A valid timezoned date gets `0x7F`, because
the day ordinal cannot retain timezone offsets or the engine's partial order
when timezone presence differs; this member adds no separate zoned-date class. Classes
`0x00` and `0x7F` establish no order; equal prefixes in class `0x07` establish no
order either. Float lexical parsing computes exact round-to-nearest-even before
forming the existing IEEE key, including subnormals and overflow.

#note[The detached zkVM model still holds a second copy of the earlier SHA-256
implementation. Its guest compilation and lockfiles are frozen. Native tests
compare both copies on the existing vectors, but the expanded datatype dispatch
of the new suite does not retroactively change the frozen model's key profile.
The previous Poseidon2 draft used truncated SHA term hashes and unsalted leaves;
this revision replaces both before fixing the implemented member's vectors.]

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
  reduced by generic quantum search. Whether a hidden-mode proof stays sound is
  a property of the proof system: pairing-based proofs do not resist such an
  adversary.
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

Its Ed25519 signature under the RFC 8032 section 7.1 TEST 1 key (public key
`d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a`) is

```
82f02d82b9845d064bae00df4668950cc03007af33ca6e9970ea0f5514b62eca
4af505ea8ada76d779bcfc7fa06dcafd997be37abedb75ff6a5ea2ef0bf33501
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
