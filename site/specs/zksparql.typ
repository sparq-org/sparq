// [OPUS-4.8] sq-vvu9d — zkSPARQL: Zero-Knowledge Query Proofs over SPARQL.
//
// RESTART FROM THE CODEBASE (maintainer directive, 2026-07-04). This draft was re-authored
// from what is ACTUALLY in the sparq repository — the `sparq-zk` / `sparq-zk-compose` crates,
// the Noir circuits under `zk/`, the vendored security-properties ontologies under
// crates/sparq-trust/ontologies/zkp-sparql/ and crates/sparq-policy/ontologies/, and the
// code-synced SKILL.md / SECURITY.md surfaces. The pre-existing zksparql.org site and the
// earlier ISWC submission texts are DEPRECATED and were NOT used as a source; where those
// authors' PUBLISHED prior art is cited (the `sec-prop` security-properties vocabulary,
// ISWC 2025) it is cited as external related work, not inherited.
//
// Every candidate-normative clause names the code that realises it (crate + item), so a
// reviewer can check the text against the implementation. Details that exist in code but are
// pinned to a specific proving toolchain (the bb public-input byte layout) are marked
// descriptive / at-risk, not normative.
//
// REVISION 3 (sq-gum8.5, submission support): hardened the related-work section into
// subsections (3.1-3.5) covering PoneglyphDB, ZKGraph, VeriDKG, zk-creds/Crescent and ZKLP,
// with (a) an EXPLICIT disclaimer of any priority/compliance claim about in-circuit IEEE 754
// (ZKLP exists and claims that ground), (b) an explicit statement that the fragment of
// section 7.1 is NARROWER than the relational systems' - OPTIONAL/MINUS/NOT EXISTS/aggregation
// are OUT, so no coverage advantage is claimed, and (c) an explicit self-delta vs the
// Braun-Kaefer / Braun-Wright-Kaefer line (dataset soundness vs evaluation correctness).
// Added a reproducibility pointer (section 16) to the deterministic constraint-count pack in
// bench/zk-compose/. New bibliographic entries carry the provenance caveat noted at the head
// of the References section. NO number of any kind was added to this document.
//
// HONESTY: the entire zkSPARQL estate is a research scaffold and is NOT externally audited
// (open external-audit gate sq-qhy4). An internal, single-model re-audit found the binding
// layer "sound as landed for the threat model the prior audit assumed", but that is explicitly
// NOT a production guarantee and does not replace external sign-off (SECURITY.md). This draft
// states that plainly and repeatedly; it must never be edited into claiming a settled
// production guarantee while sq-qhy4 is open. (Provenance: dispatched as Claude Opus 4.8 while
// Fable was unavailable; flagged for ZK re-review.)
//
// MERGED-IN WORK: the candidate-normative section-7 fragment-extension amendment (sq-3kd2g.5,
// PR #3571, commit ce73abbd) landed on main against the PRE-restart base. Its content — the
// construct-disposition table, the bounded property-path semantics and its `path_reach`
// obligations, the phase-3 expression fragment with its EBV/error lane, and the extended
// algebra grammar/evaluation — is carried forward here on top of the code-grounded rewrite.
// The rewrite's stratified implemented-today grammar is retained as the implemented tier; the
// amendment's productions sit in an explicitly phase-gated extension tier.

#import "_lib/spec.typ": spec-head, sotd, intro-section, references, dfn, note, cite

#set document(title: "zkSPARQL: Zero-Knowledge Query Proofs over SPARQL")
#set text(size: 11pt)
#set par(justify: true)
#set heading(numbering: "1.")

#spec-head()

#intro-section("abstract", "Abstract")[
  zkSPARQL is a proposal for proving, in zero knowledge, that the answer to a SPARQL query is
  correct against one or more committed RDF graphs, without revealing them. This document
  describes the interfaces realised by the sparq reference
  implementation, read directly from its source: the committed data model (RDF Dataset
  Canonicalization with a Poseidon2 sponge commitment over the BN254 scalar field, and issuer
  attestation signatures over the Baby Jubjub curve); the supported SPARQL fragment, its
  circuit family, and a normative algebraic definition of the fragment; the #dfn[proof
  manifest] interchange object, whose member schema is transcribed here from the Rust type;
  the verifier-nonce challenge–response; the verifier's fail-closed obligation set and its four
  audit gates; the external trust anchors a relying party supplies; and the layered
  security-properties vocabulary with its ODRL admissibility profile. Parts that do not yet
  exist — a registered media type, a JSON-LD context, and a wire protocol — are marked as
  proposals. The entire scheme is research-grade: it has not been externally audited,
  and no production guarantee is claimed (see the Security and Privacy Considerations,
  section 17).
]

#sotd()

#intro-section("audit-status", "Implementation and audit status")[
  A reference implementation of everything marked *implemented* below exists in the sparq
  repository #cite("SPARQ") as two opt-in, unpublished crates — `sparq-zk` (commitment and
  attestation) and `sparq-zk-compose` (circuit family, manifest, verifier) — together with an
  adversarial ("forge") test suite. However, the implementation has *not* been reviewed by an
  external cryptographer: the external audit gate (tracked in-repo as *sq-qhy4*, P0) is open.
  An internal, single-model re-audit found the verifier's binding layer *sound as landed for
  the threat model its prior audit assumed*, but that finding is #strong[not] a production
  guarantee, was produced by an LLM agent, and does not replace external sign-off. Until
  sq-qhy4 closes, soundness and attestation are *not production-ready*; every positive security
  property of this scheme is at most a *claim*, and a passing verification is not a guarantee
  that the proven SPARQL statement holds against an adversarial prover. This draft never
  asserts otherwise, and section 12.3 makes the corresponding over-claim rule normative for
  annotations.
]

= Introduction

This section is informative.

Verifiable-credential ecosystems let a holder present signed RDF data to a verifier. zkSPARQL
extends that interaction from "show the data" to "prove a query over the data": a
#dfn[prover] evaluates a SPARQL query #cite("SPARQL11-QUERY") over RDF graphs that an
#dfn[issuer] has committed to and signed, and produces a #dfn[proof manifest] — a bundle of
zero-knowledge sub-proofs plus bindings — that a #dfn[verifier] checks without seeing the
underlying graphs. Example uses include proving that a credential attribute satisfies a
threshold `FILTER`, that a value was not revoked at an authoritative snapshot, or that two
hidden credentials agree on a join key.

The flow is challenge–response:

+ The verifier mints a fresh #dfn[verifier nonce] and hands it to the prover.
+ The prover evaluates the query over its committed source graphs and produces a proof
  manifest whose every sub-proof commits to that nonce.
+ The verifier checks the manifest against its own #dfn[trust anchors] (trusted issuer keys,
  an authoritative revocation snapshot, a holder registry, and its nonce store), enforcing a
  fixed, fail-closed obligation set (section 10).
+ Optionally, a policy engine decides whether the *method* used is admissible for the purpose,
  by reasoning over the security-properties vocabulary (sections 12 and 13).

This document describes the interfaces of that pipeline as realised in the sparq reference
implementation, so they can be reviewed, critiqued, and cited. It is an Unofficial Proposal
Draft; see the Status of This Document.

= Document scope and maturity

== What this document is — and is not

This document is an *architecture and interface overview* of the zkSPARQL query-proof
pipeline, transcribed from the reference implementation and published in specification form so
the design can be reviewed and cited. It is #strong[not] a specification from which an
independent party could build a provably-sound prover or verifier — soundness rests on the  // privacy-claims-allow: explicit anti-overclaim — negated usage ("NOT a spec from which a provably-sound verifier could be built"), not a settled soundness claim (sq-qhy4)
Noir circuits and the proving backend, neither of which is externally audited (section 17.1),
and several security-load-bearing encodings are pinned to a specific proving toolchain
(section 8.4). A reader needing a conformance target should read section 2.2 for the exact
clauses this draft fixes normatively, and section 4.3 for what may — and may not — be claimed
against it.

== The normative kernel

Only the following clauses of this document are candidate-normative, and the RFC-2119
requirement keywords of section 4.1 are confined to them. Each is stable by design intent and
implementable from this text together with the cited source:

+ the committed-data-model algorithm identities — RDFC-1.0 canonicalisation, the term/leaf
  encoding, and the per-graph Poseidon2/BN254 sponge commitment (section 6.1) — and the
  cryptosuite resolution rule (section 6.3);
+ the fragment boundary and its algebraic semantics (sections 7.1–7.2), including the ban on
  representing a disclosed-base entailment re-check as a zero-knowledge proof (section 7.4);
+ the manifest type identifier and the trust-status of each declared member (sections 8.1–8.2);
+ the verifier-nonce discipline (section 9);
+ the verifier's fail-closed discipline: fail closed on any check that cannot be completed
  (section 4.3), the prefilter is not verification (section 10.2), a self-declared circuit
  identifier is never sufficient (sections 7.3 and 10.3), and first-failure rejection with no
  partial results and no warning downgrades (section 10.6);
+ the externality of every trust anchor, including the key-set subset rule (section 11);
+ the vocabulary scope and over-claim rules (sections 12.2–12.3), the provenance-only reading
  of `zk:sourceCryptosuite` (section 15), and the relying-party audit-status obligations and
  deployment cautions (sections 17.1–17.2 and 17.5–17.6);
+ the ODRL admissibility profile rules (section 13).

Everything else — in particular the public-input byte layout of section 8.4 and the
one-line obligation glosses of section 10 — is *descriptive* of the reference implementation
and carries no conformance force. The implementation remains the source of truth for the
exact clause of each obligation.

== Toolchain-pinned and unaudited cores

The following are security-load-bearing but rest on artefacts this text does not, and cannot,
fix normatively:

+ the exact in-circuit constraint relation of each circuit of section 7.3 — realised in the
  Noir sources under `zk/` and proved with an external backend, neither externally audited;
+ the public-input byte layout of section 8.4, which is *empirically pinned* to one
  Barretenberg release and is therefore descriptive, not normative;
+ the manifest hash, because the canonicalisation the reference implementation applies before
  hashing (`ProofManifest::canonicalize`) is an implementation detail this draft records but
  does not standardise (section 8.1).

Until these are respecified toolchain-independently and audited, a second implementation cannot
be guaranteed byte-interoperable, and no production soundness may be claimed.

= Related work

This section is informative.

Every comparison below states *what* a system proves and *about what*. None of it is a
performance comparison: this document reports no measured comparison against another system
and reproduces no other system's reported figures. The only quantitative artefact it points
at is the deterministic constraint-count pack of section 16.2, which counts gates in this
document's own circuit family and in nothing else.

== Verifiable and zero-knowledge query evaluation over databases

IntegriDB #cite("INTEGRIDB") and vSQL #cite("VSQL") prove SQL query answers correct against a
committed, outsourced database: integrity against a cheating server, with no hiding of the
data. ZKSQL #cite("ZKSQL") extends the guarantee to zero knowledge — the answer is proven
correct while the database's records stay hidden — with an interactive, VOLE-based argument
run between prover and verifier. PoneglyphDB #cite("PONEGLYPHDB") addresses the same SQL
setting with a *non-interactive* PLONKish argument, so its proof object, like the manifest of
section 8, can be checked offline and after the fact.

zkSPARQL targets the same class of guarantee for RDF and SPARQL, with four structural
differences: the data model is graph-shaped, with blank nodes and per-graph canonicalisation
(section 6); trust is rooted in *issuer attestation signatures* over per-graph commitments
rather than in a single data owner's commitment, so proofs compose across many small signed
graphs (credentials); credential-layer statements — holder possession, revocation
non-membership, hidden-issuer attestation — are carried by the *same* manifest and the same
verifier obligation set as the query-layer statements (sections 8 and 10), rather than by a
separate credential protocol; and the admissibility of a proof *method* is itself
policy-controlled (section 13). The zkSPARQL manifest is a non-interactive object designed to
be checked offline against externally supplied trust anchors (section 11).

The fragment comparison cuts the other way and is stated plainly here so it is not mistaken:
the relational systems above accept far more of their query language than section 7.1 accepts
of SPARQL. `OPTIONAL`, `MINUS`, `FILTER NOT EXISTS` and aggregation are #strong[OUT] of this
fragment by design, because each asserts a closed-world or completeness property that
composed membership proofs do not establish. This document therefore claims no coverage
advantage over relational zero-knowledge query systems; its fragment is monotone and
deliberately narrow.

== Verifiable and zero-knowledge queries over graphs and RDF

ZKGraph #cite("ZKGRAPH") evaluates graph queries in zero knowledge under a PLONKish argument
and is the closest graph-shaped analogue. It does not target RDF or SPARQL: there is no RDF
dataset canonicalisation, no blank-node discipline, and no notion of many independently
signed source graphs — the three things sections 6 and 7 are built around.

VeriDKG #cite("VERIDKG") verifies SPARQL query results over decentralised knowledge graphs
using an authenticated data structure. Its guarantee is *integrity against a cheating server*
and it is deliberately not hiding: the verifier sees the results and the authenticated
structure they were drawn from. That is a different point in the design space from the model
of section 5, where the verifier is not trusted with the source graphs at all. The two
guarantees are complementary rather than competing — and VeriDKG's is a settled published
result, whereas this document's is not, pending the audit gate of section 17.1.

== Anonymous credentials and selective disclosure

CL signatures #cite("CL02"), the BBS line of multi-message signatures #cite("BBS04"), and the
`bbs-2023` / `ecdsa-sd-2023` Data-Integrity cryptosuites #cite("VC-DI") let a holder reveal a
subset of signed attributes, sometimes with simple predicates. zk-creds #cite("ZKCREDS")
generalises the mechanism by putting the credential check inside a zkSNARK, so statements
about attributes of *existing* identity documents can be proven; Crescent #cite("CRESCENT")
follows the same route for existing JWT and mDL credentials with a prepare-once /
show-fast split.

What all of these prove is a statement about a *credential and its attributes*. zkSPARQL
generalises the *statement language* instead: the holder proves that a SPARQL query — joins,
typed value filters, revocation state — evaluates as claimed over the signed data
(section 7), without disclosing the data. It does not replace credential-level
selective disclosure, and it is not a competing credential format: ingest of `bbs-2023`
credentials is an explicitly deferred seam (section 15).

== In-circuit numerics

The `xsd:double` FILTER lane of section 7.1 needs IEEE 754 semantics inside a circuit. ZKLP
#cite("ZKLP") gives zero-knowledge circuits for IEEE 754 arithmetic and states that they are
the first set fully compliant with that standard; it also surveys the earlier in-circuit
floating-point line. This document accordingly makes #strong[no] priority, completeness, or
standard-compliance claim about floating point in zero knowledge. The reference
implementation's double lane rests on the editor's `noir_IEEE754` Noir library, consumed by
the circuit family as a pinned external dependency (`sparq_ieee754`); it is an engineering
dependency of the value-comparison lane, not a contribution of this document. Its own
evidence is a differential harness against the hardware floating-point oracle — a testing
artefact, not a proof of anything.

== Delta to this document's own line

Parts of the work cited in this section are prior work of this document's editor and
co-authors, so the boundary is stated explicitly rather than left to the reader.

The annotation and admissibility layer of this document (sections 12–13) directly extends the
`sec-prop` security-properties vocabulary of Wright, Shadbolt, Zhao, Zhao and Braun
#cite("SEC-PROP"), and the query-proof pipeline shares that work's goal of proving correct
SPARQL evaluation over verifiable credentials, realised here with a different commitment
scheme, circuit family, and manifest format. The research agenda is stated in
#cite("WRIGHT-DC25").

Braun and Käfer #cite("BK25"), and then Braun, Wright and Käfer #cite("BWK26"), establish
what is best called #dfn[dataset soundness]: the verifier is shown a *selectively disclosed
view* of the queried dataset together with a proof that the view is a faithful part of the
signed source, and the query is then checked against that disclosed view. The hidden object
is the undisclosed remainder of the dataset; the evaluated portion is revealed.

zkSPARQL targets #dfn[evaluation correctness] instead: no view of the source graphs is
disclosed, the algebra of section 7.2 is evaluated *inside* the circuit family over
commitments (section 7.3), and the verifier checks a manifest (section 8) whose sub-proofs
bind to those commitments and to the verifier nonce (section 9). The two mechanisms are
complementary, and this document does not claim its direction is generally preferable: for
many predicates a disclosed view is cheaper, simpler, and easier to audit, and a
disclosed-base entailment re-check is explicitly #strong[not] representable as a
zero-knowledge sub-proof here (section 7.4). What is claimed is only that the two hide
different things and therefore suit different threat models (section 5).

The sparq estate described here is an engine-integrated implementation with its own manifest
format, verifier obligation set, and admissibility layer; it is not a wire-compatible
implementation of any of the work cited above. No positive security property of it is
asserted as achieved while the external audit gate remains open (sections 1 and 17.1).

= Terminology and conformance

== Requirement keywords

The key words #strong[MUST], #strong[MUST NOT], #strong[REQUIRED], #strong[SHALL],
#strong[SHALL NOT], #strong[SHOULD], #strong[SHOULD NOT], #strong[RECOMMENDED], #strong[MAY],
and #strong[OPTIONAL] in this document are to be interpreted as described in #cite("RFC2119")
and #cite("RFC8174") when, and only when, they appear in all capitals, as shown here. Their
use is confined to the normative-kernel clauses enumerated in section 2.2; all other text is
descriptive.

== Terms

- A #dfn[committed graph] is an RDF graph #cite("RDF11-CONCEPTS") together with a
  cryptographic commitment to its canonical form (section 6.1).
- A #dfn[sub-proof] is a single zero-knowledge proof for one circuit of the family in
  section 7.3, carried inside a proof manifest.
- A #dfn[proof manifest] is the JSON interchange object of section 8 bundling sub-proofs,
  their public metadata, and bindings.
- A #dfn[trust anchor] is an input the verifier obtains out of band from the relying party —
  never from the manifest (section 11).
- A #dfn[holder] is the party that controls the credentials a proof draws on; a holder may
  be disclosed ("clear") or hidden behind a proof-of-knowledge tier (section 10.4).

== Conformance and the fail-closed rule

A #dfn[zkSPARQL verifier] checks proof manifests (sections 9–11); a #dfn[zkSPARQL prover]
produces them (sections 6–9); an #dfn[admissibility policy engine] evaluates whether an
annotated method satisfies an ODRL policy (sections 12–13). Over-arching every verifier
clause: a verifier #strong[MUST] fail closed — it #strong[MUST] reject the whole manifest on
any check it cannot complete, and #strong[MUST NOT] return a partial or best-effort result.
Because the exact clause of each obligation (section 10) lives in the implementation, this
draft states verifier obligations as *necessary* conditions; full verifier conformance is
defined against the reference implementation until a successor draft transcribes every clause.

#note[
  This document couples its normative force to the code deliberately. Every candidate-normative
  clause names the crate item that realises it, so "conformance to this draft" means "agrees
  with the cited reference behaviour". A future revision may lift individual clauses to
  implementation-independent normativity once they are audited.
]

= Threat model and security goals

This section defines the adversary model against which the mechanisms of sections 6–11 are
the mitigation, and what each claimed security property *means* for this scheme. It is placed
before the mechanisms deliberately: every obligation in section 10 exists to counter a
capability listed here. The properties themselves are *claims* — none is externally audited
(section 17.1).

== Parties and trust relationships

- The #dfn[issuer] holds a signing key and attests committed graphs (section 6.3). The
  verifier trusts an issuer's *key* exactly insofar as the relying party placed it in the
  external key set K (section 11). Whether the issuer's attested *content* is true in the
  world is out of cryptographic scope: attestation transfers trust, it does not create it.
- The #dfn[holder] / #dfn[prover] controls the credentials and produces manifests. The
  verifier extends it *no* integrity trust: everything the prover sends is adversarial input
  until checked — including the manifest's own declared `key_set`, `query`, and
  `status_snapshots` (section 8.2). Conversely the prover extends the verifier no privacy
  trust: the scheme's hiding goals exist because the verifier is assumed curious.
- The #dfn[verifier] acts for a #dfn[relying party], which supplies every trust anchor
  (section 11). The verifier is trusted by the relying party to enforce the full obligation
  set; a verifier that skips checks voids all guarantees silently, which is why the
  discipline is fail-closed.
- The #dfn[admissibility policy engine] (section 13) reasons over *declared annotations*,
  not cryptography; it is trusted only for policy evaluation (section 17.6).

== Adversary capabilities

+ *Malicious prover.* Controls manifest contents entirely and adaptively: it can submit
  arbitrary JSON, forged or mutated sub-proofs, proofs generated against substitute circuits
  or keys, manifests replayed from earlier sessions or other verifiers, its own `key_set`
  entries, non-canonical serialisations, and claims about constructs outside the fragment.
  Its goal is to make the verifier accept a false SPARQL statement (soundness break), to
  reuse a proof (replay), or to smuggle in an untrusted issuer (attestation break).
+ *Malicious or curious verifier.* Receives the manifest and chooses the nonce adversarially.
  Its goal is to learn anything about the committed graphs beyond the proven statement and
  the public inputs (hiding break), or to link two presentations by the same holder
  (unlinkability break — tracked as a vocabulary dimension, section 12.1, not a settled
  property).
+ *Colluding holder and issuer.* Can mint attestations for arbitrary content. The scheme
  does not defend the relying party against attested-but-false real-world content
  (garbage-in); it defends only the binding between what was attested and what is proven.
  Collusion confers no capability against *other* holders' privacy or other issuers' keys.
+ *Network adversary.* Transport is currently out of band and unspecified (section 14); a
  confidential, authenticated channel is assumed and the network adversary is otherwise out
  of scope until a transport binding exists.
+ *Quantum adversary.* Explicitly conceded: the post-quantum posture is a settled negative
  (section 17.3).

== Security goals

The table gives each goal's meaning *for this scheme*, the mechanism intended to enforce it,
and its honest status. "Claim" means: implemented and exercised by the reference test suite,
but not externally audited (gate sq-qhy4, section 17.1).

#table(
  columns: 4,
  align: (left, left, left, left),
  table.header[Goal][Meaning in this scheme][Primary mechanism][Status],
  [Completeness], [An honest prover holding graphs that satisfy the query, with valid
    attestations and a fresh nonce, can produce a manifest the verifier accepts.],
    [Circuit family (section 7.3); prover pipeline.], [Claim (unaudited).],
  [Soundness], [If the verifier accepts a manifest under trust anchors (K, snapshot,
    registry, nonce), then the claimed statement holds — in the sense of section 7.2 — over
    graphs whose commitments are attested by keys in K.], [The full obligation set and audit
    gates (section 10).], [Claim (unaudited); the hidden-holder tiers are explicitly *not yet*
    sound (section 17.2).],
  [Binding], [A commitment identifies at most one canonical graph; the prover cannot open it
    to different data — the standard binding notion for commitment schemes
    #cite("PEDERSEN91").], [Poseidon2/BN254 sponge commitment over the RDFC-1.0 canonical form
    (section 6.1).], [Claim (unaudited); fails against a quantum adversary (section 17.3).],
  [Hiding / zero-knowledge], [The manifest reveals nothing about the committed graphs beyond
    the proven statement, the public inputs, and the leakage dimensions declared in
    section 12.], [ZK proof system (Noir circuits, backend proving; section 7.3); commitment
    hiding.], [Claim (unaudited); leakage dimensions are tracked, not bounded (section 17.4).],
  [Replay resistance], [An accepting manifest is bound to a single-use verifier nonce and
    cannot be accepted twice, nor transplanted to another request.], [Nonce discipline
    (section 9); audit gate 4 (section 10.5).], [Claim (unaudited); degraded by a
    non-durable nonce store (section 17.5).],
)

== Out of scope

Issuer content veracity (see above); side channels beyond the declared leakage dimensions
(timing is not modelled); denial of service; transport security (until section 14 is
realised); the quantum adversary (settled negative); and availability. The known deviations —
the hidden-holder tiers, the optional dual-leaf value lane — are catalogued in section 17.2
rather than silently excluded here.

= Committed data model

== Canonicalisation, term encoding, and the graph commitment

Each source RDF graph is canonicalised and then committed. Read from `sparq-zk` (`encode` and
`commit` modules):

+ The graph #strong[MUST] be canonicalised with RDF Dataset Canonicalization (RDFC-1.0)
  #cite("RDF-CANON"), so that commitment values are independent of blank-node labelling and
  triple order. Leaf order is the canonical N-Quads (code-point-sorted) order.
+ Each canonical term #strong[MUST] be encoded to one field element as
  $"Enc"_t("term") = h_2("type_code", h_s("value"))$. Here $h_n$ denotes the Poseidon2 sponge
  hash of an $n$-element input over the BN254 scalar field #cite("POSEIDON2") #cite("BN06") —
  a fixed-width $t = 4$ permutation used at rate 3, with the capacity initialised to
  $n dot 2^64$ (the noir-lang/poseidon `Poseidon2::hash`) — so $h_2$ and $h_3$ are
  length-domain-separated by construction; $h_s$ is a Blake3 hash folded into a field element,
  and `type_code` is `1` for an IRI, `2`
  for a literal, and `3` for a blank node. For an IRI, $h_s$ ranges over the IRI string; for a
  literal, over its canonical N-Triples token (lexical form, language tag, and datatype); for a
  blank node, the encoding is $h_2("blank_code", h_2("salt"_G, "blake3"("canonical_label")))$,
  so blank-node identity is salted per graph (closing the cross-graph blank-node correlation
  channel of section 7.2).
+ Each canonical *triple* #strong[MUST] be encoded to one *leaf*
  $"leaf" = h_3("Enc"_t(s), "Enc"_t(p), "Enc"_t(o))$ — a single Poseidon2 sponge hash of the
  three term encodings.
+ The graph commitment `C(G)` #strong[MUST] be a single Poseidon2 sponge over the
  leaf sequence in canonical order — the same $h_n$ construction defined above, with $n$ the
  leaf count, so its length-bearing capacity gives domain
  separation per leaf count — one commitment per source graph. This is a sequential sponge, not
  a Merkle tree (a Merkle arrangement for very large graphs is a deferred deliverable).

#note[
  Editor's note — the encoding above is the default `string-canonical` method
  (`zk:poseidon2-rdfc10-v1`, section 6.2). The exact field-folding of a Blake3 digest and the
  Poseidon2 round parameters are pinned by the `sparq-zk` `poseidon2` and `encode` modules;
  this section fixes the structure (type-code layering, per-graph salt for blank nodes, the
  three-term leaf, the sponge over canonical-order leaves), which is what a reviewer needs to
  check the commitment is order- and label-independent.
]

== Commitment methods (configuration axis)

The committed-graph *method* — which leaf shape a graph was committed under, and therefore
which circuit family may verify it — is a closed, fail-closed configuration enum
(`sparq-zk` `commit::CommitmentMethod`). The methods are `string-canonical`
(`zk:poseidon2-rdfc10-v1`, the default and back-compatibility anchor), `dual-leaf`
(`zk:poseidon2-dualleaf-v1`, opt-in), and `value-only` (`zk:poseidon2-valuehook-v1`, an
off-by-default research/benchmark dial that is #strong[never] a production default).
`from_scheme_iri` returns nothing for an unknown IRI (no default). The `dual-leaf` and
`value-only` leaf encodings are only partly built and carry a documented value↔lexical
downgrade (INV-VL / gap CR-G8, section 17.2); they are opt-in and unaudited.

== Issuer attestation signatures

An issuer attests a committed graph by signing its commitment (`sparq-zk` `sig` module):

+ The attestation signature scheme is a Schnorr signature #cite("SCHNORR91") over the Baby
  Jubjub curve #cite("EIP2494") with a Poseidon2-derived challenge, identified by the
  cryptosuite IRI `https://sparq.dev/ns/zk#poseidon2-schnorr-v1`
  (`SignatureScheme::POSEIDON2_SCHNORR_V1_IRI`). The signed message binds the commitment and,
  as progressively bound variants, the per-graph salt, the status reference, and a holder
  binding.
+ A verifier #strong[MUST] reject an attestation whose cryptosuite identifier it cannot
  resolve. There is #strong[no] default cryptosuite: `from_cryptosuite_iri` returns nothing
  for an unknown IRI, so an unresolved suite is a hard failure, not a fallback (fail-closed).

= Query fragment and circuit family

== The supported SPARQL fragment

This subsection is candidate-normative (section 2.2). The fragment is the monotone,
federation-free subset tabulated below. "IN (today)" means implemented by the reference
verifier; "IN (phase N)" means designed but #strong[not yet implemented]; `DEFERRED` is
admissible only after its stated re-entry condition; and `OUT` is excluded. These labels are
part of the fragment boundary, not an implementation roadmap that a manifest may anticipate.

What "IN (today)" denotes concretely — the fragment realised by the `sparq-zk-compose`
`build` module — is deliberately small and bucketed:

- basic graph pattern (BGP) scans over committed graphs, with per-graph commitment recompute,
  row soundness, and scan completeness proved in-circuit;
- value `FILTER` constraints, bucketed by datatype lane: non-negative integer (`filter_int`),
  the integer-valued `xsd:double` fragment (`filter_f64`), signed integer
  (`filter_signed_int`), fixed-point `xsd:decimal` (`filter_decimal`), and, behind the
  off-by-default `dual-leaf` feature, the value-dictionary lanes (`filter_value_dl*`); the
  general fractional/scientific `xsd:double` filter is deferred;
- a single-prover equality `JOIN` across hidden credentials (`join_eq`), where the join term
  stays private.

#table(
  columns: (1.2fr, 1fr, 3.8fr),
  align: (left, left, left),
  table.header[Construct][Disposition][Reason],
  [`SELECT`], [IN (today)], [Membership is defined over solution mappings.],
  [`ASK`], [IN (today)], [Non-emptiness of eval(P) is monotone.],
  [`CONSTRUCT`], [OUT], [Graph-template instantiation is outside the membership property; a consumer can instantiate a template from a disclosed mapping.],
  [`DESCRIBE`], [OUT], [Its result is implementation-defined.],
  [BGP], [IN (today)], [Scan circuits check row membership and per-scan completeness.],
  [`Join`], [IN (today)], [Hidden equality join, retaining the cross-graph blank-node exclusion below.],
  [`FILTER`], [IN (today: four numeric lanes, plus the opt-in `dual-leaf` value-dictionary lanes); IN (phases 2–3: section 7.2 expression fragment)], [Monotone under SPARQL error-as-unsatisfied semantics.],
  [`UNION`], [IN (phase 2)], [Set union is monotone. Each disclosed solution identifies its branch; the verifier re-derives that branch from the query.],
  [`OPTIONAL` / `LeftJoin`], [OUT], [An unbound optional side asserts that no compatible extension exists, a non-monotone closed-world claim.],
  [`MINUS`], [OUT], [Closed-world set difference is non-monotone.],
  [`FILTER NOT EXISTS`], [OUT], [Closed-world negation is non-monotone.],
  [`FILTER EXISTS`], [DEFERRED], [Positive existence is monotone, but re-entry requires phase 2 and semantics pinned to SPARQL 1.2.],
  [`GRAPH`], [OUT], [Named-graph attribution contradicts the graph-set privacy model.],
  [`SERVICE`], [OUT], [Federation is outside the fragment.],
  [`VALUES`], [IN (phase 2)], [Public inline rows are monotone; `UNDEF` cells are wildcards.],
  [`BIND` / `Extend`], [IN (phase 3)], [A deterministic in-fragment expression adds a derived binding; non-deterministic built-ins remain out.],
  [Nested `SELECT`], [IN (phase 3)], [An in-fragment subquery is monotone; subqueries containing aggregates remain out.],
  [Property paths], [IN (phases 1–2, bounded semantics)], [Governed by the first-class bounded semantics below.],
  [Aggregation (`GROUP BY`, `HAVING`, `COUNT`, `SUM`, `AVG`, `MIN`, `MAX`, `GROUP_CONCAT`, `SAMPLE`)], [OUT], [An aggregate claims completeness of the whole pattern, which composed proofs do not establish. Re-entry requires a composed-completeness obligation.],
  [`ORDER BY`], [OUT; possible accept-and-strip re-entry], [Ordering is membership-indifferent, but accepting it could imply an unverified top-result claim. Re-entry requires an explicit "order not proved" manifest flag.],
  [`DISTINCT`, `REDUCED`, `LIMIT`, `OFFSET`, projection], [IN (today)], [These modifiers are membership-indifferent.],
  [SPARQL 1.2 triple terms / reification], [OUT (encoding gap)], [The committed leaf encoding has no triple-term lane.],
  [SPARQL 1.2 `LANGDIR`, `hasLANG`, `hasLANGDIR`, `STRLANGDIR`, `isTRIPLE`, `TRIPLE`, `SUBJECT`, `PREDICATE`, `OBJECT`], [OUT (encoding gap)], [These require term-encoding lanes first.],
  [SPARQL 1.2 `EXISTS` clarifications], [Adopted where relevant], [They govern eventual positive-`EXISTS` re-entry.],
)

A prover #strong[MUST NOT] emit a manifest claiming coverage of a construct whose
disposition is not "IN (today)", unless the verifier and circuit family implement the named
phase and identify that extension explicitly. A verifier #strong[MUST] reject a claimed
construct that it does not implement, any `DEFERRED` or `OUT` construct, and any expression
or path form outside the tables below. Thus candidate-normative design text does not enlarge
the reference implementation's claim surface ahead of implementation.

== Formal semantics of the fragment

This subsection is candidate-normative (section 2.2): it defines the fragment by mapping it
onto the SPARQL algebra of Pérez, Arenas and Gutiérrez #cite("PAG09"), as adopted by the
SPARQL 1.1 recommendation #cite("SPARQL11-QUERY"), over the RDF 1.1 graph model
#cite("RDF11-CONCEPTS") under simple entailment #cite("RDF11-MT"). It is the semantic anchor
for the correctness obligation `bind_query_correctness` (section 10.4).

*Property-path extension (candidate-normative; phases 1–2).* The following dispositions
are designed extensions and remain unavailable until their named phase is implemented:

#table(
  columns: (1fr, 1fr, 4fr),
  align: (left, left, left),
  table.header[Path form][Disposition][Evaluation / circuit semantics],
  [`iri`], [IN (phase 1)], [Identical to a triple pattern.],
  [`^p`], [IN (phase 1)], [Swap subject and object; composition is preserved.],
  [`p1/p2`], [IN (phase 1)], [Rewrite to a BGP with a fresh, non-projected intermediate variable.],
  [`p1|p2`], [IN (phase 2)], [Rewrite to `UNION` with per-solution branch attribution.],
  [`p?`], [IN (phase 2)], [The union of the occurrence-witnessed zero-length case and one step.],
  [`p+`], [IN (phase 2, bounded)], [`path_reach` with one through k steps.],
  [`p*`], [IN (phase 2, bounded)], [`path_reach` with zero through k steps.],
  [`!(p1|…|pn)` including inverse members], [DEFERRED], [Monotone, but deferred until after `path_reach`; predicate inequality over salted term encodings also requires the re-audited-pending-external argument to be specified.],
)

For `p+` and `p*`, the circuit #strong[MUST] prove, and the manifest #strong[MUST] be read
as claiming, exactly:

#quote(block: true)[
  There exists a chain of committed triples `(t_1, …, t_ℓ)` with `1 ≤ ℓ ≤ k`
  (`0 ≤ ℓ ≤ k` for `*`), each `t_i` a member of a committed graph in the disclosed
  attribution set with predicate `p`, chained object-to-subject, connecting `μ(s)` to
  `μ(o)` — where #strong[`k` is a public input disclosed in the manifest].
]

The following requirements are first-class verification obligations, subject to the
external-audit caveat of section 17.1:

+ *Public bound.* Proofs at different k are different statements. The verifier
  #strong[MUST] expose k to the consumer and #strong[MUST] reject a claimed depth greater
  than the selected circuit member's bound.
+ *Existence only.* A path proof #strong[MUST NOT] assert that longer paths do not exist or
  that the reachable set is complete. Failure to produce a proof at k proves nothing.
+ *One-directional equivalence.* For the bounded evaluation, $op("eval")_k(P) subset.eq op("eval")(P)$:
  every bounded witness is a SPARQL `p+` or `p*` solution, while completeness is only up to
  k. If a walk exists, a simple path of length at most the committed union's node count
  exists; choosing at least that count restores per-pair completeness, but a verifier
  #strong[MUST NOT] assume that choice was made.
+ *Padding.* Every unused step when ℓ < k #strong[MUST] contribute nothing: it
  #strong[MUST] either be a proven committed-row membership or a constrained pass-through
  preserving the chain endpoint.
+ *Zero length.* For `p*` and `p?`, a zero-length result #strong[MUST] establish both
  `μ(s) = μ(o)` and that the term occurs in the committed union. Bare equality is
  insufficient; an occurrence witness is required.
+ *Cycles.* Evaluation is existence-based set semantics. A witness chain need not be
  simple, and duplicate walks do not create additional solutions.

The intended circuit family is `path_reach_d{k}`, unrolled to k steps. It is designed but
not implemented at this draft's publication; the requirements above specify what an
implementation must bind, not a present cryptographic guarantee.

*Expression extension (candidate-normative; phase 3).* The following table is the complete
designed expression fragment. Except for the four numeric comparison lanes marked today,
these entries do not describe current verifier coverage.

#table(
  columns: (1.6fr, 1fr, 3.4fr),
  align: (left, left, left),
  table.header[Expression class][Disposition][Verification boundary],
  [Logical `&&`, `||`, `!`], [IN (phase 3)], [Requires the EBV/error lane below.],
  [Numeric comparisons], [IN (today: four lanes); phase 3 in expression positions], [`=`, `!=`, `<`, `<=`, `>`, `>=`; integer, double, signed-integer, and decimal lanes.],
  [String comparison / equality], [IN (phase 3)], [Codepoint order only; locale collation is OUT.],
  [`dateTime`, `date`, `time`, duration comparison/arithmetic], [IN (phase 3)], [Datatype-bucketed expression-node circuits.],
  [`sameTerm`], [IN (phase 3)], [Committed-leaf equality.],
  [RDF-term `=`], [IN (phase 3)], [Leaf plus literal-value equality; retains the dual-leaf caveat of section 17.2.],
  [`IN` / `NOT IN` constant lists], [IN (phase 3)], [Public-constant (in)equality; `NOT IN` is value inequality, not closed-world negation.],
  [`BOUND`], [IN (phase 3)], [With `OPTIONAL` out, boundness is static for BGP-derived variables; an `Extend`-introduced variable remains dynamically unbound when its expression errors.],
  [`IF` / `COALESCE`], [IN (phase 3)], [Requires the EBV/error lane.],
  [`isIRI`, `isBlank`, `isLiteral`, `isNumeric`, `datatype`, `lang`, `str`], [IN (phase 3 after encoding dependency)], [Requires type, datatype, and language lanes.],
  [`IRI`, `STRDT`, `STRLANG`], [DEFERRED], [Requires encoding-side term construction.],
  [`BNODE`, `UUID`, `STRUUID`, `RAND`, `NOW`], [OUT], [Non-deterministic; an as-of value may instead be verifier-supplied public input.],
  [String functions: `STRLEN`, `SUBSTR`, `UCASE`, `LCASE`, `STRSTARTS`, `STRENDS`, `CONTAINS`, `STRBEFORE`, `STRAFTER`, `ENCODE_FOR_URI`, `CONCAT`], [IN (phase 3)], [Bounded byte-array representation; `SUBSTR` retains its byte-position caveat.],
  [`REGEX` / `REPLACE`], [IN (phase 3, bounded subset only)], [Literal, anchored, and character-class subset; full `fn:matches` is OUT.],
  [`langMatches`], [IN (phase 3 after estate gap-fill)], [Requires its missing circuit implementation.],
  [`abs`, `round`, `ceil`, `floor`], [IN (phase 3)], [Integer and floating-point lanes.],
  [Arithmetic `+`, `-`, `*`, `/`], [IN (phase 3)], [Division's decimal-as-double approximation #strong[MUST] be surfaced.],
  [Date components `YEAR` through `TZ`], [IN (phase 3; `TZ` after gap-fill)], [`TZ` requires its missing implementation.],
  [`MD5`, `SHA1`, `SHA256`, `SHA384`, `SHA512`], [IN (phase 3 after estate gap-fill)], [Requires digest and hexadecimal-output circuits.],
  [Aggregate functions], [OUT], [Aggregation is outside the fragment.],
)

Phase 3 uses composable, datatype-bucketed expression-node circuits, not a generic
expression VM. Each node sub-proof discloses operand and result commitments; binding edges
#strong[MUST] connect node results leaf-to-root and root every leaf in a scan-row slot. The
verifier #strong[MUST] re-derive the expression tree from the query text and #strong[MUST]
reject a manifest whose declared tree differs.

Every expression node #strong[MUST] carry `(value, is_error)`. Comparisons and functions
#strong[MUST] propagate `is_error` according to SPARQL/XPath rules; `&&`, `||`, `IF`, and
`COALESCE` #strong[MUST] implement the three-valued effective-boolean-value table; and a
`FILTER` root #strong[MUST] accept only `true` with `is_error = false`. These are verification
obligations for the designed extension and are not claims that the phase-3 circuits exist.

*Data model.* Let I, B, and L be the pairwise-disjoint sets of IRIs, blank nodes, and
literals, and V a set of variables disjoint from all three. An RDF graph G is a finite set
of triples in (I ∪ B) × I × (I ∪ B ∪ L). Each committed graph (section 6.1) is one such
graph, fixed by its RDFC-1.0 canonical form.

*Solution mappings.* A solution mapping μ is a partial function from V to I ∪ B ∪ L, with
domain dom(μ). Two mappings μ1 and μ2 are *compatible* when μ1(v) = μ2(v) for every variable
v in dom(μ1) ∩ dom(μ2); their union μ1 ∪ μ2 is then itself a mapping.

*Grammar.* A fragment pattern P over the committed graphs G1, …, Gn is generated by the
stratified grammar:

```
# implemented today (stratified, matching the realised circuit family)
S ::= BGP | Filter(C, S)
P ::= S | Join(S1, S2) | Project(W, P)

# candidate-normative extension, admitted per phase (section 7.1)
P ::= … | Union(P1, P2) | Values(R) | Extend(v, E, P) | Path(s, path, o)
```

subject to: the first block is the currently implemented grammar; the second block is the
candidate-normative extension and is admitted only as its corresponding phase becomes
implemented. A `BGP` (a finite set of triple patterns over terms and variables) is evaluated
against *exactly one* committed graph; `C` is a value constraint drawn from the
datatype-bucketed comparison forms of section 7.1; and `Join` is the equality join of
section 7.1, whose two sub-patterns S1 and S2 are evaluated over *distinct* committed graphs
and share at least one variable. The stratification of the implemented block is deliberate and
matches the realised circuit family: join nesting (`Join` over a `Join` result) and filters
over join results are *outside* the fragment — every `FILTER` binds to a single scan's slot via
a binding edge, and each equality join spans exactly two scans. A manifest #strong[MAY] carry
several `Join(Si, Sj)` obligations over pairwise-distinct scan pairs (the `join_edges` vector).
`Project` is membership-indifferent (section 7.1) and imposes no circuit obligation. In the
extension block, `R` is a public `VALUES` row set, `E` is an expression from the table above,
`W` is a projection list, and `path` is an admitted path form with public bound k where
required; the extension productions relax the stratification only for the phases that
implement them.

*Evaluation.* The evaluation eval(P) is a set of solution mappings:

- eval(BGP over G) = the set of mappings μ with dom(μ) = vars(BGP) such that replacing each
  variable v in BGP by μ(v) yields a subgraph of G;
- eval(Filter(C, S)) = the set of μ in eval(S) such that μ satisfies C under the SPARQL 1.1
  operator semantics (section 17 of #cite("SPARQL11-QUERY")), with expression errors treated
  as *not satisfied*;
- eval(Join(S1, S2)) = the set of unions μ1 ∪ μ2 where μ1 is in eval(S1), μ2 is in eval(S2),
  and μ1 and μ2 are compatible;
- eval(Union(P1, P2)) = eval(P1) ∪ eval(P2), with the witnessed branch disclosed;
- eval(Values(R)) is the public set of mappings encoded by R, where `UNDEF` omits that
  variable from the row mapping;
- eval(Extend(v, E, P)) evaluates E under each μ in eval(P), adds v ↦ value when E succeeds,
  and retains μ without a v binding when E raises an expression error;
- eval(Project(W, P)) restricts each mapping in eval(P) to W;
- eval(Path(s, path, o)) is SPARQL path evaluation for non-recursive rewrites and eval_k for
  bounded `p+` / `p*`, exactly as constrained above.

*Blank nodes across graphs.* Blank-node identity is scoped to a single graph
#cite("RDF11-CONCEPTS"), and per-graph canonicalisation (section 6.1) does not — and cannot —
align blank-node labels *across* committed graphs. Cross-graph equality of blank nodes is
therefore semantically meaningless in this fragment: a union μ1 ∪ μ2 in which some variable
v ∈ dom(μ1) ∩ dom(μ2) is bound to a blank node is *excluded from eval(Join(S1, S2))* — since
S1 and S2 range over distinct committed graphs, such a binding would assert a cross-graph
blank-node identity. This exclusion is the semantic minimum that the implementation's "Q6"
guard (`verify::recheck`, section 10.3) enforces.

*The correctness property.* The target property of `bind_query_correctness` (section 10.4)
is *result membership*: a manifest that discloses a solution mapping μ (or claims that a
solution exists) for a fragment pattern P over committed graphs G1, …, Gn is correct if and
only if μ is a member of eval(P) — respectively eval(P) is non-empty — as defined above.

#note[
  Editor's note — three boundaries of this definition are deliberate. (1) It is a *set*
  semantics; whether the implementation preserves duplicate-solution multiplicities (the bag
  semantics of #cite("PAG09")) is scoped out here. (2) Projection (`SELECT` variable lists) is
  transcribed above as restriction of each solution mapping to the selected variables, which is
  membership-indifferent: it claims neither bag semantics nor result completeness. (3) Result
  *completeness* — that no solutions were omitted — is proved for the in-circuit BGP scan but
  #strong[not] for the whole pattern, and is #strong[not] claimed by `bind_query_correctness`
  as glossed here. None of these boundaries weakens the membership property, but all three must
  be settled before an implementation-independent, candidate-normative successor; the remaining
  transcription work is tracked as bead sq-rvgr2.7.
]

== The circuit family

Each sub-proof is generated against exactly one circuit of a fixed, named family
(`sparq-zk-compose` `manifest::CircuitId`), each realising one operator instance of the
fragment (or one auxiliary statement: revocation, issuer-set membership, holder binding). The
circuits are authored in Noir #cite("NOIR") across three source estates under `zk/` — an
in-tree IEEE-754 library (`zk/ieee754`; canonical here, with a published face maintained at
`sparq-org/noir_IEEE754`), an XPath 2.0 function library (`zk/xpath`), and the
compiled per-property family workspace (`zk/compose`, one compiled binary per shape bucket) —
and proved with the Barretenberg backend (see section 16 on toolchain pinning). The family
(with the fixed shape parameters each variant carries) is:

#table(
  columns: 2,
  align: (left, left),
  table.header[Circuit identifier][Statement proved (descriptive gloss)],
  [`Scan { k, n, r }`], [A BGP scan over `k` committed graph(s) matches; `n`, `r` are the
    compiled slot/row capacity buckets (in-circuit commitment recompute + scan completeness).],
  [`FilterInt { d }`], [A non-negative integer-lane `FILTER` holds; `d` is the digit bucket.],
  [`FilterF64 { d }`], [An integer-valued `xsd:double`-lane `FILTER` holds.],
  [`FilterSignedInt { md }`], [A signed-integer-lane `FILTER` holds.],
  [`FilterDecimal { id, fd }`], [A fixed-point `xsd:decimal`-lane `FILTER` holds.],
  [`FilterValueDl` / `FilterValueDlF64` / `FilterValueDlDecimal`], [Value-dictionary-lane
    `FILTER` for integer / `xsd:double` / `xsd:decimal` (opt-in `dual-leaf`; section 17.2).],
  [`RevokeUnset { depth }`], [The revocation bit at a hidden index is unset in a committed
    status snapshot of the given Merkle depth.],
  [`HiddenIssuer { depth }`], [A committed graph was signed by *some* key in an attested key
    set, without disclosing which issuer.],
  [`HolderPok`], [Holder proof-of-knowledge (hidden-holder tier; see section 17.2).],
  [`HolderSet { depth }`], [Holder set membership (hidden-holder tier; see section 17.2).],
  [`JoinEq { n_a, n_b }`], [Two hidden credentials agree on an equality join key without
    disclosing it.],
)

The prover derives the compiled shape bucket from the data; the verifier *re-derives* the
circuit identifier from the statement each sub-proof is bound to (section 10.3), and a manifest
#strong[MUST NOT] be accepted on its self-declared identifier alone. An
out-of-bucket shape is a clean rejection, never a silently unprovable member.

== Entailment regimes

Only *simple entailment* is proved in zero knowledge (`manifest::EntailmentRegime::Simple`;
`Rdfs`/`Owl` are placeholders). A manifest #strong[MAY] declare `Rdfs`/`Owl` derivation steps
(`derivation_steps`), but these are re-checked by the verifier against *disclosed* bases
(`bind_entailment`, section 10.4): every step must be a well-formed, regime-admitted rule
instance whose antecedents are grounded in an earlier step or a disclosed scan row. A
non-`Simple` regime with no grounded steps is rejected (fail-closed). The derivation bases are
revealed to the verifier; the in-circuit closure proof is deferred. A prover #strong[MUST NOT]
represent a disclosed-base re-check as a zero-knowledge entailment proof.

= The proof manifest

== Typing and canonical serialisation

A proof manifest is a JSON object (`sparq-zk-compose` `manifest::ProofManifest`,
`to_json`/`from_json` via serde). Its `type` member #strong[MUST] be the value
`urn:sparq:zk:ProofManifest` (the field defaults to this constant when absent).

Every hash of a manifest — for nonce binding, deduplication, or audit — is defined over the
manifest's *canonical serialised form* (`ProofManifest::canonicalize`, which sorts the
self-contained `binding_edges` and `join_edges` into their derived total order before
serialising). This draft records that the reference implementation canonicalises before hashing,
but does #strong[not] standardise the
canonicalisation algorithm; two independent implementations cannot yet be expected to agree on
a manifest hash, and manifest-hash interoperability is expressly *not* offered by this draft
(section 2.3).

== Member schema

The following members are transcribed from the `ProofManifest` Rust struct. The *trust status*
column is candidate-normative: it records whether each member is a trust anchor, a mere
narrowing claim, or informational — the load-bearing distinction the codex #1 soundness fix
codified (section 11).

#table(
  columns: 3,
  align: (left, left, left),
  table.header[Member][Content][Trust status],
  [`type`], [The string `urn:sparq:zk:ProofManifest`.], [Schema marker.],
  [`query`], [The SPARQL query text the proof attests a result for.], [*Re-parsed, never
    trusted*: the verifier re-parses it (`verify::recheck`).],
  [`issuers`], [`did:key` references for the committed graphs.], [Informational provenance
    only.],
  [`key_set`], [The prover's declared issuer verification keys (hex Baby-JubJub points).],
    [*Narrowing claim only*: accepted only as a *subset* of the external anchor K
    (section 11); never the trust anchor (codex #1).],
  [`commitment_attestations`], [One issuer attestation per distinct scan commitment.],
    [Checked against the external K (audit gate 3).],
  [`attributions`], [Per-pattern graph-attribution sets. The indices are *scan-local*:
    `attributions[pattern]` indexes the answering scan's own `commitments` vector, and the
    verifier maps them to committed-graph *identity* (`global_attributions`) before deriving
    cross-graph obligations.], [Fed to the Q6 cross-graph blank-node-join guard; enforced as a
    superset by `bind_attributions`.],
  [`join_obligations`], [Declared non-blank-node join obligations `(variable, i, j)`.],
    [Manifest side of the join gate.],
  [`entailment_regime`], [`Simple` \| `Rdfs` \| `Owl`.], [Enforced by `bind_entailment`.],
  [`derivation_steps`], [Inference steps justifying derived triples (empty for `Simple`).],
    [Re-checked against disclosed, grounded bases (section 7.4).],
  [`binding`], [The binding mode — `Challenge { challenge }` (the v1 default), or
    `HolderPop { challenge, holder, pop, cryptosuite }` (clear-key holder
    proof-of-possession, checked by `bind_holder_pop`); both carry the verifier nonce as
    `challenge`.],
    [Bound to the verifier's own nonce (section 9).],
  [`revocation`], [Optional status reference `(status_list, index, version)`.],
    [Issuer-bound; a status-bound credential with this omitted is *rejected* (`bind_revocation`,
    section 10.4).],
  [`status_snapshots`], [Disclosed status-list bitstrings.], [*Prover copy is a tripwire
    only*; the bit decision reads the relying party's authoritative snapshot (section 11).],
  [`sub_proofs`], [Array of sub-proof objects (`{ inputs, proof_hex }`).], [Each verified
    against a recomputed key + reconstructed inputs (section 10.5).],
  [`binding_edges`], [Binding-consistency edges between sub-proofs.], [Enforce operand
    identity across sub-proofs (scan row/slot == consuming filter operand).],
  [`join_edges` / `hidden_revocation` / `hidden_issuer_attestations` / `holder_pok_proofs` /
    `holder_set_proofs`], [Optional privacy-upgrade layers (hidden join / hidden-index
    revocation / hidden issuer / hidden-key holder proof-of-possession / hidden-holder set).],
    [Additive; the clear-path checks always run (section 17.2).],
)

Three JSON value-level conventions apply throughout (descriptive, from the implementation).
Every field element (`FieldHex` — commitments, term encodings, the challenge) is rendered as a
`0x`-prefixed, 64-nibble, lowercase big-endian hexadecimal string. Pattern indices follow the
order in which the BGP triple patterns appear in the re-parsed query text
(`attributions[i]` describes the query's i-th pattern). In a binding edge, `from_slot` selects
the operand column of a disclosed scan row as `0` = subject, `1` = predicate, `2` = object.

== Sub-proof encoding

Each sub-proof carries its statement in `inputs` (a typed `ProofInputs` variant matching the
circuit) and its proof in `proof_hex`: a single hex-encoded blob produced by `sparq-zk-compose`
`verifier::encode_artifacts` with the layout

```
proof_hex = hex( LP(proof) ‖ LP(public_inputs) ‖ vk )
LP(x)     = ( len(x) as u32, big-endian, 4 bytes ) ‖ x
```

— a 4-byte big-endian length-prefixed proof, a 4-byte big-endian length-prefixed public-input
segment, and the verification key as the trailing remainder (not length-prefixed). The
verification key carried here is the prover's and is #strong[never] trusted: the verifier
recomputes the canonical key (audit gate 2, section 10.5).

== Public-input encoding (descriptive; at-risk)

This subsection is *descriptive and at-risk*; it deliberately attaches no RFC-2119
requirement. With the pinned toolchain of section 16, the encoding of a sub-proof's
public-input segment (which `verifier::reconstruct_public_inputs` byte-compares against, audit
gate 1) is:

- each public-input field element is encoded as exactly 32 bytes, big-endian;
- structs and arrays are flattened in row-major order (declaration order of the circuit's
  `main`);
- booleans encode as `0` or `1`; `u32`/`u64` values encode as their integer value;
- the segment carries no header and no per-element length prefix;
- public-input field 0 is the verifier nonce (section 9).

#note[
  This byte layout is *empirically pinned* to a specific Barretenberg release
  (`bb 5.0.0-nightly.20260324`, driven as a subprocess alongside `nargo 1.0.0-beta.21`, bb
  target `noir-recursive`); it was determined by observation against real `bb` output, not
  derived from a backend specification, and it is not guaranteed stable across `bb` releases. A
  reverse-engineered, toolchain-fragile layout is not a conformance requirement, so this draft
  states it descriptively. A MUST-level layout will be introduced only once it is specified
  independently of the `bb` toolchain (section 2.3); until then, interoperability across
  backend versions is not promised (section 16).
]

== Worked example (illustrative)

The example below is *illustrative only*: the elided hex (`…`) is not real proof material,
the scan's trailing zero-padded rows (up to `r`) are elided, and the optional
privacy-upgrade members are absent. The member names and serde tagging shapes (`"circuit"`,
`"kind"`, `"mode"`) mirror the implementation's serialisation exactly. It is *not* a test
vector and cannot be verified; portable conformance fixtures are open future work
(section 16).

```json
{
  "type": "urn:sparq:zk:ProofManifest",
  "query": "SELECT ?s ?o WHERE { ?s <http://ex/age> ?o FILTER(?o >= 18) }",
  "issuers": ["did:key:zIssuer"],
  "key_set": ["1c0aa5b7…e977"],
  "commitment_attestations": [
    { "commitment": "0x2f1e…", "issuer_public_key": "1c0aa5b7…e977",
      "signature": "…", "cryptosuite": "https://sparq.dev/ns/zk#poseidon2-schnorr-v1" }
  ],
  "attributions": [[0]],
  "entailment_regime": "simple",
  "binding": { "mode": "challenge", "challenge": "0x00…2a" },
  "sub_proofs": [
    { "inputs": { "circuit": "scan",
        "id": { "kind": "scan", "k": 1, "n": 16, "r": 4 },
        "commitments": ["0x2f1e…"],
        "pattern_is_const": [false, true, false],
        "pattern_const_enc": ["0x00…00", "0x17c4…", "0x00…00"],
        "rows": [["0x08a1…", "0x17c4…", "0x2b9d…"], …],
        "row_count": 1,
        "attribution": [true] },
      "proof_hex": "…" },
    { "inputs": { "circuit": "filter_int",
        "id": { "kind": "filter_int", "d": 2 },
        "operand_enc": "0x2b9d…",
        "op": "ge", "bound": 18, "expected": true },
      "proof_hex": "…" }
  ],
  "binding_edges": [{ "from_proof": 0, "from_row": 0, "from_slot": 2, "to_proof": 1 }]
}
```

= Challenge–response: the verifier nonce

The proof request is a nonce challenge (`sparq-zk-compose` `verifier::VerifierNonce`,
`SeenNonces`):

+ The verifier #strong[MUST] mint a fresh #dfn[verifier nonce] — a BN254 scalar field
  element, exchanged in hexadecimal form — and deliver it to the prover *before* proving
  begins. Nonces #strong[MUST NOT] be reused across requests.
+ The prover #strong[MUST] commit the nonce as *public-input field 0 of every sub-proof* in
  the manifest, and carry it in `binding` — as the `challenge` member of either binding mode,
  `Challenge` or `HolderPop` (section 8.2).
+ The verifier #strong[MUST] record the nonce as used (single-use, `SeenNonces::record_fresh`)
  *before* running the cryptographic checks of section 10.5, so that a manifest that fails late
  cannot be replayed against the same nonce.
+ If a manifest binds a nonce other than the one issued for the request, the verifier
  #strong[MUST] reject with a nonce-binding mismatch (`NonceBindingMismatch`) *and*
  #strong[MUST] still burn the issued nonce — once the nonce is recorded, no subsequent
  rejection is a free retry (rejections raised by the structural prefilter or the entailment
  re-check, which run before the nonce is recorded, do not consume it). The nonce (not
  the manifest's declared `binding`) is fed as public-input field 0 during reconstruction, so a
  proof committed under any other challenge fails the byte-compare of audit gate 1.

How the nonce is delivered and how the manifest is submitted is out of band and currently
unspecified; section 14 proposes a transport binding.

= Verification

== Overview

The reference verifier exposes the full-binding entry point `verifier::verify_manifest`, which
takes the manifest, the circuit prover, a work directory, and the relying party's trust anchors
— the trusted `KeySet`, the `RevocationPolicy`, the `HolderRegistry`, the
`HolderBindingPolicy`, the entailment policy, the fresh `VerifierNonce`, and the `SeenNonces`
store — and returns `Result<(), CheckError>`. Sections 10.2–10.5 describe its obligation set;
the fail-closed discipline of sections 10.2 and 10.6 is kernel-normative (section 2.2). Failure
yields a `CheckError` variant pinpointing the failed gate.

== Entry points

The reference verifier exposes two entry points:

+ a *structural prefilter* (`prefilter_manifest_structure`) covering shape and consistency
  checks, which is #strong[not] sufficient on its own — it runs no backend, binds nothing to a
  proof, and enforces no freshness — and #strong[MUST NOT] be treated as verification; and
+ *full verification* (`verify_manifest`), which runs the prefilter (whose stages include the
  structural re-checks of section 10.3), the binding obligations of section 10.4, and the
  cryptographic checks of section 10.5, in a fail-closed pipeline.

Only full verification performs the (unaudited — section 17.1) checking this document describes.

== Structural re-checks

Full verification performs the following structural re-checks as the first, mandatory stage of
its pipeline — they are stages of `prefilter_manifest_structure`, so they also run when the
prefilter is invoked alone. The blank-node guard and the attribution-arity check live in
`sparq-zk` (`verify::recheck`); the circuit-identifier re-derivation and the
strictly-increasing commitment ordering live in `sparq-zk-compose`
(`prefilter_manifest_structure` itself):

- the *blank-node guard* (the "Q6" guard): a cross-graph join on a blank node is rejected,
  enforcing the semantic exclusion of section 7.2 against a malicious prover — keyed on
  committed-graph identity, not the scan-local index, so a genuine cross-scan join over two
  distinct committed graphs must declare its non-blank-node `join_obligations`;
- *attribution arity*: the attribution structure is well-formed;
- *circuit-identifier re-derivation*: the identifier each sub-proof claims is re-derived from
  the statement it is bound to, and the two must agree;
- *strictly-increasing commitment ordering*: the manifest's graph commitments are strictly
  increasing, giving a canonical order and excluding duplicates.

== Binding obligations

The reference verifier enforces the twelve binding obligations below (`sparq-zk-compose`
`verifier::bind_*`); each is fail-closed. The obligation *set* is covered by the adversarial
("forge") test suite of the reference implementation. The one-line glosses are descriptive:
the implementation remains the source of truth for each obligation's exact clause (section 2.2).

#table(
  columns: 2,
  align: (left, left),
  table.header[Obligation][Descriptive gloss],
  [`bind_query_correctness`], [Every query BGP pattern has a scan binding its constant slots,
    and every `FILTER` has a slot-bound, true-verdict filter sub-proof reachable via a binding
    edge; target property is result membership under section 7.2.],
  [`bind_attributions`], [`manifest.attributions[pattern]` is a *superset* of each answering
    scan's proof-bound in-circuit attribution bits (closes the attribution-collapse forge).],
  [`bind_issuer_attestations`], [Every issuer key used is a member of the *external* trusted
    key set K (section 11) — never merely of the manifest's own key list — and its Schnorr
    signature over the commitment verifies. This obligation *is* audit gate 3 (section 10.5).],
  [`bind_revocation`], [The disclosed status reference is issuer-bound, and the liveness bit is
    read from the relying party's *authoritative* snapshot (never the prover's), within the
    freshness window.],
  [`bind_joins`], [Each `JoinEdge`'s `join_eq` proof binds its public commitments byte-for-byte
    to the two scans' graph commitments, and its slots to the query-derived slots.],
  [`bind_entailment`], [The declared entailment regime is honoured and derivation steps are
    re-checked against disclosed, grounded bases (section 7.4).],
  [`bind_holder_pop`], [Holder proof-of-possession (clear-key tier) is a valid Schnorr over the
    challenge, the holder is in the external `HolderRegistry`, and (under `require_binding`) the
    presented key matches the issuer-attested holder digest.],
  [`bind_holder_binding`], [The clear (disclosed) holder key's digest equals the attestation's
    `holder_pk_digest`, and the disclosed key matches the presented key.],
  [`bind_hidden_revocation`], [Hidden-index revocation: the proof's public Merkle root equals
    the root derived from the relying party's authoritative snapshot.],
  [`bind_hidden_issuer_attestations`], [Hidden issuer: the proof's public key-set root equals
    the root derived from the authoritative external K, without disclosing which issuer.],
  [`bind_holder_pok`], [Hidden-holder proof-of-knowledge tier — explicitly *not yet* sound;
    opt-in only (section 17.2).],
  [`bind_holder_set`], [Hidden-holder set-membership tier — explicitly *not yet* sound;
    opt-in only (section 17.2).],
)

== Cryptographic checks and the four audit gates

The scheme defines exactly four #dfn[audit gates] — the cross-cutting binding checks whose
failure would each void soundness on its own:

#table(
  columns: 3,
  align: (left, left, left),
  table.header[Audit gate][Definition][Enforced by],
  [1 — public-input reconstruction], [The verifier independently reconstructs the expected
    public-input bytes of every sub-proof — with the verifier nonce at field 0 — and compares
    them byte-for-byte against the manifest's segment; any difference rejects.],
    [`reconstruct_public_inputs` (`PublicInputMismatch`).],
  [2 — canonical verification key], [The verification key of every sub-proof is recomputed
    from the canonical circuit named by the re-derived `CircuitId`; the prover's key is never
    trusted.], [`canonical_vk` recomputation.],
  [3 — issuer signature and key set], [Every issuer key used is bound to the external key
    set K, and the issuer's signature over the graph commitment verifies.],
    [`bind_issuer_attestations` (section 10.4), per scan.],
  [4 — nonce single-use and binding], [The verifier nonce is fresh, single-use, recorded
    before the cryptographic checks, and burnt on mismatch.], [The nonce discipline of
    section 9 (`SeenNonces`, `NonceBindingMismatch`).],
)

In addition to the four audit gates, each sub-proof is verified by the proving backend
against the recomputed key and reconstructed inputs (*backend proof verification*). This
check carries no audit-gate number: the audit gates are the binding checks layered *around*
backend verification, which is meaningless without gates 1 and 2 pinning what is verified.
Full verification runs, fail-closed and in order: nonce single-use recorded and binding checked
(gate 4); then, per sub-proof, gate 1; gate 2; backend proof verification. Gate 3 is enforced
per scan by `bind_issuer_attestations`.

== Fail-closed error handling

Every failure mode maps to an explicit variant of a closed error taxonomy
(`verifier::CheckError`, on the order of eighty variants). A conforming verifier
#strong[MUST] reject the whole manifest on the *first* failed check, #strong[MUST NOT] return
partial results, and #strong[MUST NOT] downgrade any error to a warning.

= External trust anchors

All trust anchors are inputs from the relying party, passed to `verify_manifest`. A conforming
verifier #strong[MUST] obtain each of the following out of band and #strong[MUST NOT] accept
any of them from the manifest:

+ the *trusted issuer key set K* (`KeySet`) — the manifest #strong[MAY] carry its own
  `key_set`, but it is accepted only if it is a *subset* of K; an empty K trusts no issuer, so
  any scan carrying commitments is rejected;
+ the *authoritative status-list snapshot* governing revocation (`RevocationPolicy`,
  `StatusListSnapshot`) — the prover's `status_snapshots` copy is only a tamper tripwire;
+ the *holder registry* and *holder-binding policy* (`HolderRegistry`, `HolderBindingPolicy`);
+ the *fresh verifier nonce* of section 9 (`VerifierNonce`);
+ the *seen-nonce store* enforcing single use (`SeenNonces`) — this store #strong[SHOULD] be
  durable across verifier restarts (the reference implementation provides a durable file-backed
  store `FileSeenNonces` (flock + fsync, single-host) and a test-only `InMemorySeenNonces` that
  forgets burned nonces on restart).

#note[
  The subset rule for K is codified from experience: an earlier revision that trusted the
  manifest's own key list was a review-identified soundness hole (the "codex #1" fix, recorded
  in the `key_set` doc comment). Externalising every trust anchor is what closed it.
]

= Security-properties vocabulary

zkSPARQL methods are *annotated* with machine-readable security properties so that policy
engines can reason about them (section 13). The vocabulary is layered and vendored into the
repository.

== Base vocabulary

The base vocabulary is the vendored `sec-prop` ontology, namespace
`https://w3id.org/zkp-sparql/sec-prop#`, whose eight base security-property classes are
`Unlinkability`, `SourceCredentialDisclosure`, `PostQuantumForgery`, `PostQuantumSnooping`,
`SignatureTypeLeakage`, `ProofSizeLeakage`, `CircuitAudit`, and `ValidityPeriodLeakage`
#cite("SEC-PROP"). The `sec-prop` vocabulary is prior published work of this document's editor
and collaborators (Wright, Shadbolt, Zhao, Zhao, Braun #cite("SEC-PROP")); sections 12 and 13
derive from it, and it is vendored into sparq under MIT with its provenance record (co-authored
for the ISWC 2025 work, MIT-licensed by the 2026-06-21 decision).

#note[
  Editor's note — the `w3id.org/zkp-sparql/` identifiers were minted as placeholders while the
  source repository was private. Before this draft advances, the permanent-identifier redirect
  must be confirmed live and stable.
]

== The secx extension

The sparq extension (`secx`, declared in `secprop-ext.ttl` under the *same*
`https://w3id.org/zkp-sparql/sec-prop#` namespace — the `secx`/`sec-prop` distinction is
prose-only, and the extension `owl:imports` the base, it does not fork it) adds the orthogonal
proof-system dimensions `ZeroKnowledgeType`, `Soundness`, `Completeness`, `Hiding`, `Binding`,
`Anonymity`, `Setup`, `Interactivity`, `SelectiveDisclosure`, and `SingleUse`, plus four
orthogonal axes:

- `AssuranceLevel`, ordered `Proven` > `Claimed` > `Conjectured` (the sparq ZK default is
  `Claimed`);
- `AuditStatus`, ordered `ExternallyAudited` > `InternallyReviewed` > `Unreviewed`, plus the
  distinguished value `ExternalSignOffPending` (the live sq-qhy4 state);
- `Assumption` (e.g. `IssuerHonesty` — carried by the dual-leaf lane — `DiscreteLog`,
  `RandomOracle`, `HonestMajority`, `SemiHonest`);
- `PropertyScope`, distinguishing `QueryProofLayer` (default) from `SourceLayerOnly`.

A property that holds at the source layer only #strong[MUST NOT] be used to satisfy a
query-proof-layer constraint: source-layer facts do not transfer to the query-proof layer.

The dimension names shadow the security goals of section 5.3 deliberately: an annotation is a
machine-readable *claim* about a goal, and the assurance axis records how settled the claim is.

== The over-claim rule

While the external audit gate (sq-qhy4) is open (`sparq-zk` `secprop` module, behind the
`secprop-annotations` feature):

+ No sparq zkSPARQL method #strong[MAY] be annotated `secx:Proven` for any *positive* privacy
  or soundness property; such properties are at most `secx:Claimed` with
  `AuditStatus ExternalSignOffPending`.
+ Only *settled negative* facts — for example `PQForgeable`, `Replayable`, `SchemeRevealed`
  — #strong[MAY] carry `Proven`.

The reference implementation enforces this rule mechanically with three machine-checkable
guards over the annotation graph (`ontologies/secprop-methods.ttl`):
`audit_overclaim_violations` (no `Proven` on a positive property while the gate is open),
`completeness_violations` (every production-selectable method is annotated), and
`source_layer_transfer_violations` (a `SourceLayerOnly` property never satisfies a query-proof
constraint).

= Policy-controlled admissibility

Relying parties express *which* proof methods they accept as ODRL 2.2 policies #cite("ODRL22")
over the vocabulary of section 12, using the sparq security-property profile
(`odrl-secprop-profile.ttl`, `sparq-policy`; profile IRI
`https://sparq.dev/ns/odrl-secprop-profile#`, which declares fifteen `secx:requires…`
leftOperands; reduced by `sparq-trust` `admissibility` / `admit`):

+ A policy using any `secx:requires…` left-operand #strong[MUST] assert
  `odrl:profile <https://sparq.dev/ns/odrl-secprop-profile#>`.
+ Each such left-operand carries exactly one `secx:overDimension` fact identifying the
  property dimension it constrains.
+ Only the operator `odrl:gteq` is given a reduction; a constraint using any other operator
  #strong[MUST] be treated as *unsatisfied* — which denies.
+ A method is admissible only if it satisfies *every* constraint of the policy
  (default-deny).
+ In the fail-closed pre-check gate (`admit_with_precheck`), the outcomes are `Admitted`,
  `Denied`, `UnknownMethod`, `MalformedConstraint`, and `ReductionError` — and a reduction
  error or a malformed constraint #strong[MUST] be treated as a denial.

Base admission additionally checks the issuer's Schnorr signature over the RDFC-1.0
commitment, a SHACL #cite("SHACL") statement-type scope constraint, a reserved-predicate guard,
and — for clear holders — a WebID holder binding (the credential subject equals the session
agent).

#note[
  A consequence worth stating plainly: a policy requiring
  `requiresAssurance odrl:gteq secx:Proven` on a positive property mechanically denies *every*
  current sparq zkSPARQL method while the external audit (sq-qhy4) is open. That is by design —
  it is the honest default for high-assurance relying parties.
]

= Transport, media type, and interchange

This section is entirely a *proposal*: none of it exists in the reference implementation
today. The manifest is a bare JSON object tagged with a URN, and both nonce issuance and
manifest submission are out of band.

== Media type (proposal)

A registered media type is proposed for the proof manifest — candidate
`application/zksparql+json`, with an `application/zksparql+ld+json` variant once a JSON-LD
context exists — and a companion type for the nonce challenge. Until registration,
implementations exchanging manifests over HTTP have no content-type contract.

== JSON-LD context (proposal)

A JSON-LD context for the manifest is proposed so that a manifest can round-trip as a W3C
Verifiable Presentation #cite("VC-DATA-MODEL") and be consumed by generic data-integrity
processors. No such context exists today; the manifest does not currently round-trip.

== Wire protocol (proposal)

A challenge–response HTTP binding is proposed: an endpoint issuing single-use nonces and an
endpoint accepting manifest submissions bound to them. No server endpoint, job model, or
asynchronous proving exists in the implementation, so any binding written here would be
speculative and is deferred to a subsequent draft.

= Relationship to W3C Verifiable Credentials

This section is informative.

- A *VC cryptosuite bridge* — off-circuit Data-Integrity verification of `eddsa-rdfc-2022`
  and `ecdsa-rdfc-2019` (P-256) source credentials #cite("VC-DI") at ingest — is designed as
  an opt-in `vc-bridge` feature but is *not merged to the main line* at the time of writing
  (it lives on a feature branch and plugs into the `IssuerSignatureScheme` seam). Any claim of
  VC ingest must be caveated accordingly.
- In that bridge design, the P-384 profile of `ecdsa-rdfc-2019` is *not* implemented and
  fails closed as an unsupported key curve; like the bridge itself, this behaviour is not on
  the main line — on main there is no VC-ingest path at all.
- Ingest of `bbs-2023` / `ecdsa-sd-2023` selective-disclosure credentials is an explicitly
  *deferred* seam: there is no in-repo BBS verifier.
- In-circuit re-verification of the source credential's proof is deliberately *out of scope*:
  the query proof does not re-verify the source VC signature inside the circuit. The
  `zk:sourceCryptosuite` annotation is provenance only, and #strong[MUST NOT] be read as
  evidence that the source proof was verified in zero knowledge.

= Conformance testing and toolchain pinning

== Open conformance gaps

Two conformance gaps are open:

+ *No portable test vectors.* An adversarial forge-test suite exists covering the manifest
  format and the verifier obligation set, but it is internal to the Rust implementation, and
  the cryptographic-chain forge tests and real `bb` prove/verify cases are `#[ignore]`d in
  default CI (they require the nargo/bb toolchain). A conformance suite of portable fixtures
  (manifests that must verify, and mutated manifests that must fail with a specific error
  class) is required future work.
+ *Toolchain pinning.* The circuit family is pinned to an external toolchain
  (`nargo 1.0.0-beta.21`, `bb 5.0.0-nightly.20260324`, bb target `noir-recursive`, with the
  in-circuit Poseidon fixed to the `noir-lang/poseidon` `v0.3.0` tag) driven by subprocess, and
  the public-input byte layout of section 8.4 is empirically determined and therefore
  descriptive, not normative. A toolchain change could silently shift the serialisation with no
  failing test. Until the layout is specified toolchain-independently (section 2.3),
  cross-version interoperability is out of reach and even reference-level compatibility can
  only be claimed against the pinned toolchain.

== Reproducible constraint counts

The one reproducible quantitative artefact of the reference implementation is a
#dfn[constraint-count pack]: the per-member gate count of every compiled circuit-family
member, grouped by family and reported alongside the family parameters it varies over. It
lives in the sparq repository #cite("SPARQ") under `bench/zk-compose/`, is regenerated by a
script that reads a regression-gated snapshot rather than invoking the prover, and is
therefore byte-identical on re-run and independent of the machine that runs it. This document
states no figure from it; it points at it so a reader can obtain the figures without trusting
prose.

Three honesty constraints govern what that artefact may be read to mean, and they are
repeated here because they are easy to lose in a table:

+ A gate count is a *size* of a compiled circuit under the toolchain pinned in section 16.1.
  It is not a running time, and no wall-clock figure — prove, verify, or end-to-end — is
  reported by this document or by the pack. Timings gathered on a development machine are not
  comparable across machines and are excluded deliberately.
+ A gate count says nothing about whether the circuit proves the right statement. The
  coverage status of each SPARQL construct is section 7.1's table, not a circuit size; in
  particular the bounded property-path members prove a strictly weaker, bounded-existence
  statement (section 7.1), and a large or small number next to them does not change that.
+ The pack reproduces no other system's reported figures. Constraint counts are not
  comparable across proof systems, arithmetizations, or circuit granularities, so a ratio
  between this family and a differently-arithmetized published system would not be a
  measurement of anything. The related work of section 3 is cited, never re-measured.

= Security and Privacy Considerations

The threat model and the meaning of each security goal are given in section 5; this section
records the honest status of those goals and the known deviations.

== Audit status

The entire zkSPARQL estate is research-grade and has *not* been externally audited; the
external cryptographer audit is an open gate (sq-qhy4). An internal, single-model re-audit
found the verifier's binding layer *sound as landed for the threat model its prior audit
assumed*, but that finding was produced by an LLM agent, rests partly on code-reading rather
than on tests running in CI, and does #strong[not] replace external sign-off. Accordingly:

+ A relying party #strong[MUST NOT] treat a passing verification as a settled guarantee that
  the proven SPARQL statement holds against an adversarial prover.
+ Soundness and attestation are *not production-ready*; deployments that need a production
  guarantee are out of scope for this draft until the audit closes.
+ The over-claim rule of section 12.3 applies to every annotation surface: positive
  properties are at most `Claimed`, with audit status `ExternalSignOffPending`.

== Known-unsound and downgraded components

- The hidden-holder tiers (`bind_holder_pok`, `bind_holder_set`) are explicitly labelled *not
  yet* sound in the implementation and its documentation; remediation is tracked internally
  (epic sq-1s2). They are opt-in only, and verifiers #strong[SHOULD] leave them disabled
  unless the residual risk is understood and accepted.
- The optional dual-leaf value lane carries an accepted, documented value↔lexical invariant
  downgrade (INV-VL, gap CR-G8, #769): value–lexical agreement on the value-`FILTER` lane
  rests on trusted-issuer honesty (recorded as a `secx:IssuerHonesty` assumption) and is not
  machine-enforced. It is opt-in, partial, and unaudited.
- Only simple entailment is proved in zero knowledge; `Rdfs`/`Owl` derivations are
  disclosed-base re-checks (section 7.4), which reveal the derivation bases to the verifier
  and limit entailment coverage.

== Post-quantum posture

The post-quantum posture is a *settled negative*. The issuer signature suite in scope
(Schnorr over Baby Jubjub) and the related credential suites (EdDSA, BBS+) rest on
discrete-log hardness and fall to a Shor-capable adversary; commitment binding likewise breaks
under a cryptographically relevant quantum computer, so *retrospective* soundness of previously
accepted proofs fails as well. The vocabulary records this honestly as negative
`PostQuantumForgery` / `PostQuantumSnooping` facts — these negatives are among the few
annotations permitted to carry `Proven` (section 12.3). The scheme makes *no* FIPS or CMVP
claim; it is deliberately built on ZK-friendly, non-FIPS-approved primitives (BN254,
Poseidon2, Baby Jubjub), and the signing path is not constant-time (a documented residual).

== Leakage and unlinkability

`SignatureTypeLeakage`, `ProofSizeLeakage`, `ValidityPeriodLeakage`, and the unlinkability
dimensions are tracked as vocabulary dimensions so that policies can constrain them; their
values for sparq methods are at most `Claimed` and are *not* settled guarantees. By default,
issuer attestation is checked in the clear (revealing which issuer signed) and a clear-path
`revocation.index` is disclosed (a linkability channel) unless the hidden-issuer / hidden-index
circuits are enabled. Verifiers and relying parties should assume that proof size, timing, and
suite choice may leak information about the underlying credentials until an audit says
otherwise.

== Replay and nonce hygiene

Replay resistance rests entirely on the nonce discipline of section 9: single-use recording
*before* the cryptographic checks, burn-on-mismatch, and a durable seen-nonce store. A
verifier using a non-durable store (`InMemorySeenNonces`) forgets burned nonces on restart and
#strong[SHOULD NOT] be exposed where replay across restarts matters; a multi-host deployment
#strong[SHOULD] back `SeenNonces` with a database uniqueness / compare-and-set store.

== Admissibility reasons over annotations, not cryptography

The admissibility engine of section 13 reasons over *declared annotations*, not over the
cryptography itself. An "Admitted" outcome means the method's declared properties satisfy the
policy — it is not, and #strong[MUST NOT] be presented as, an independent cryptographic
finding.

= References

#note[
  Editor's note (revision 3). The entries `PONEGLYPHDB`, `ZKGRAPH`, `VERIDKG`, `ZKLP`,
  `ZKCREDS`, `CRESCENT` and `BK25` were added in revision 3 from the project's own
  search-verified related-work records. Venue, year, and the DOI / arXiv identifier are
  reproduced from those records; author initials and exact titles have #strong[not] been
  independently re-verified against the publishers' pages in this revision, and two entries
  (`ZKGRAPH`, `BK25`) deliberately carry an identifier and a description rather than a title
  that could not be confirmed. They must be checked before any camera-ready use.
]

#references((
  ("RFC2119", [Bradner, S. #emph[Key words for use in RFCs to Indicate Requirement Levels].
    RFC 2119, IETF, March 1997.]),
  ("RFC8174", [Leiba, B. #emph[Ambiguity of Uppercase vs Lowercase in RFC 2119 Key Words].
    RFC 8174, IETF, May 2017.]),
  ("SPARQL11-QUERY", [Harris, S.; Seaborne, A. (eds). #emph[SPARQL 1.1 Query Language].
    W3C Recommendation, 21 March 2013. https://www.w3.org/TR/sparql11-query/.]),
  ("PAG09", [Pérez, J.; Arenas, M.; Gutierrez, C. #emph[Semantics and Complexity of SPARQL].
    ACM Transactions on Database Systems 34(3), article 16, 2009.]),
  ("RDF11-CONCEPTS", [Cyganiak, R.; Wood, D.; Lanthaler, M. (eds). #emph[RDF 1.1 Concepts and
    Abstract Syntax]. W3C Recommendation, 25 February 2014.
    https://www.w3.org/TR/rdf11-concepts/.]),
  ("RDF11-MT", [Hayes, P.; Patel-Schneider, P. (eds). #emph[RDF 1.1 Semantics].
    W3C Recommendation, 25 February 2014. https://www.w3.org/TR/rdf11-mt/.]),
  ("RDF-CANON", [Longley, D.; Kellogg, G.; et al. (eds). #emph[RDF Dataset Canonicalization
    (RDFC-1.0)]. W3C Recommendation, 2024. https://www.w3.org/TR/rdf-canon/.]),
  ("POSEIDON2", [Grassi, L.; Khovratovich, D.; Schofnegger, M. #emph[Poseidon2: A Faster
    Version of the Poseidon Hash Function]. AFRICACRYPT 2023; IACR ePrint 2023/323.]),
  ("PEDERSEN91", [Pedersen, T. P. #emph[Non-Interactive and Information-Theoretic Secure
    Verifiable Secret Sharing]. CRYPTO '91, LNCS 576, Springer, 1992. (Source of the standard
    hiding/binding commitment notions used in section 5.3.)]),
  ("SCHNORR91", [Schnorr, C. P. #emph[Efficient Signature Generation by Smart Cards].
    Journal of Cryptology 4(3), 1991.]),
  ("BN06", [Barreto, P. S. L. M.; Naehrig, M. #emph[Pairing-Friendly Elliptic Curves of Prime
    Order]. SAC 2005, LNCS 3897, Springer, 2006. (BN254 / alt-bn128 is the 254-bit instance
    standardised for Ethereum in EIP-196/EIP-197.)]),
  ("EIP2494", [Bellés-Muñoz, M.; Baylina, J. #emph[EIP-2494: Baby Jubjub Elliptic Curve].
    Ethereum Improvement Proposals, 2020.]),
  ("NOIR", [Aztec Labs. #emph[The Noir Programming Language]. https://noir-lang.org/.]),
  ("VC-DATA-MODEL", [Sporny, M.; et al. (eds). #emph[Verifiable Credentials Data Model v2.0].
    W3C Recommendation, 2025. https://www.w3.org/TR/vc-data-model-2.0/.]),
  ("VC-DI", [Sporny, M.; Longley, D.; et al. (eds). #emph[Verifiable Credential Data
    Integrity 1.0] and its cryptosuites (eddsa-rdfc-2022, ecdsa-rdfc-2019, bbs-2023,
    ecdsa-sd-2023). W3C Recommendations, 2025. https://www.w3.org/TR/vc-data-integrity/.]),
  ("ODRL22", [Iannella, R.; Villata, S. (eds). #emph[ODRL Information Model 2.2].
    W3C Recommendation, 15 February 2018. https://www.w3.org/TR/odrl-model/.]),
  ("SHACL", [Knublauch, H.; Kontokostas, D. (eds). #emph[Shapes Constraint Language (SHACL)].
    W3C Recommendation, 20 July 2017. https://www.w3.org/TR/shacl/.]),
  ("SEC-PROP", [Wright, J.; Shadbolt, N.; Zhao, Jun; Zhao, Rui; Braun, C. #emph[Zero-Knowledge
    Proof of Correct SPARQL Evaluation over Verifiable Credentials]. Prior work of this
    document's editor and collaborators; vocabulary source repository
    https://github.com/jeswr/sparql-zkp-ontologies, namespace `https://w3id.org/zkp-sparql/`.
    The `sec-prop` sub-vocabulary is vendored, with the sparq `secx` extension, in the sparq
    repository (MIT). Declared for citation integrity; sections 12–13 derive from it.]),
  ("WRIGHT-DC25", [Wright, J. #emph[Towards Provable Provenance and Privacy-Preserving Queries  // privacy-claims-allow: prior-work reference title (Wright, ISWC 2025 DC), not a sparq claim
    in Decentralised Data Architectures]. ISWC 2025 Companion Volume (Doctoral Consortium),
    CEUR-WS Vol-4085, paper 19, Nara, Japan, November 2025.
    https://ceur-ws.org/Vol-4085/paper19.pdf.]),
  ("BK25", [Braun, C.; Käfer, T. In: The Semantic Web (ESWC 2025), Springer, 2025.
    DOI 10.1007/978-3-031-94575-5_21 — RDF-level selective disclosure combined with
    zero-knowledge proofs; the immediate predecessor of the entry below.]),
  ("BWK26", [Braun, C.; Wright, J.; Käfer, T. #emph[Proving Soundness of SPARQL Query Results
    Using Selective Disclosure of RDF Datasets and Zero-Knowledge Proofs]. In: The Semantic
    Web, Springer, 2026. DOI 10.1007/978-3-032-25156-5_16.]),
  ("INTEGRIDB", [Zhang, Y.; Katz, J.; Papamanthou, C. #emph[IntegriDB: Verifiable SQL for
    Outsourced Databases]. ACM CCS 2015.]),
  ("VSQL", [Zhang, Y.; Genkin, D.; Katz, J.; Papadopoulos, D.; Papamanthou, C. #emph[vSQL:
    Verifying Arbitrary SQL Queries over Dynamic Outsourced Databases]. IEEE Symposium on
    Security and Privacy, 2017.]),
  ("ZKSQL", [Li, X.; Weng, C.; Xu, Y.; Wang, X.; Rogers, J. #emph[ZKSQL: Verifiable and
    Efficient Query Evaluation with Zero-Knowledge Proofs]. Proceedings of the VLDB Endowment
    16(8), 1804–1816, 2023.]),
  ("PONEGLYPHDB", [Gu; Fang; Nawab. #emph[PoneglyphDB: Efficient Non-Interactive
    Zero-Knowledge Proofs for Private Database Queries]. ACM SIGMOD / Proceedings of the ACM
    on Management of Data, 2025. arXiv:2411.15031.]),
  ("ZKGRAPH", [ZKGraph — zero-knowledge evaluation of graph queries under a PLONKish
    argument; property-graph model, no RDF or SPARQL surface. arXiv:2507.00427, July 2025.]),
  ("VERIDKG", [Zhou; et al. #emph[VeriDKG: A Verifiable SPARQL Query Engine for Decentralized
    Knowledge Graphs]. Proceedings of the VLDB Endowment 17(5), 2024.
    https://www.vldb.org/pvldb/vol17/p912-zhou.pdf. (Authenticated data structure; integrity
    against a cheating server, not hiding.)]),
  ("ZKLP", [Ernstberger, J.; et al. #emph[Zero-Knowledge Location Privacy via Accurate
    Floating-Point SNARKs]. IEEE Symposium on Security and Privacy, 2025. (States the first
    set of zero-knowledge circuits fully compliant with IEEE 754; cited here to disclaim any
    priority or compliance claim of this document's own double lane — section 3.4.)]),
  ("ZKCREDS", [Rosenberg, M.; White, J.; Garman, C.; Miers, I. #emph[zk-creds: Flexible
    Anonymous Credentials from zkSNARKs and Existing Identity Infrastructure]. IEEE Symposium
    on Security and Privacy, 2023.]),
  ("CRESCENT", [Microsoft Research. #emph[Crescent] — unlinkable presentation of existing JWT
    and mDL credentials with zkSNARKs, split into a prepare-once and a show-fast phase.
    Project, no figure from it is reproduced here.]),
  ("CL02", [Camenisch, J.; Lysyanskaya, A. #emph[A Signature Scheme with Efficient
    Protocols]. SCN 2002, LNCS 2576, Springer, 2003.]),
  ("BBS04", [Boneh, D.; Boyen, X.; Shacham, H. #emph[Short Group Signatures]. CRYPTO 2004,
    LNCS 3152, Springer, 2004. (Origin of the BBS/BBS+ multi-message signature line used for
    selective disclosure.)]),
  ("SPARQ", [The sparq project. #emph[sparq: an RDF + SPARQL engine with a zero-knowledge
    query-proof estate (reference implementation)]. https://github.com/sparq-org/sparq.]),
))
