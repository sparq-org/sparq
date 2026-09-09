// [FABLE-5] sq-rvgr2.2 — zkSPARQL: Zero-Knowledge Query Proofs over SPARQL.
//
// Authored STRICTLY from the prose estate-recon digest (research/specs/zksparql-estate-recon.md
// on the docs/fable-program-recon-records branch); no crate sources were read while drafting or
// revising. Code-level details the digest does not pin are flagged inline as Editor's notes and
// enumerated in section 2.3; transcribing them is tracked as bead sq-rvgr2.7 — the
// implementation remains the source of truth for those details until then.
//
// REVISION 2 (review round 1, PR #1333): re-scoped as an architecture overview with an
// explicitly enumerated normative kernel (section 2); added a threat model (section 5), a
// normative fragment semantics (section 7.2), related work + full provenance citations
// (section 3, references), a manifest member table + worked example (section 8), an
// unambiguous audit-gate mapping (section 10.4); dropped RFC-2119 force from the
// reverse-engineered public-input layout (section 8.4); corrected the per-obligation
// forge-test overclaim. Bibliographic entries were verified against public sources
// (zksparql.org, CEUR-WS Vol-4085, Springer/ACM indexes) on 2026-07-01.
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
// HONESTY: the entire zkSPARQL estate is research-grade and NOT externally audited (open
// external-audit gate sq-qhy4). This draft states that plainly and repeatedly; it must never be
// edited into claiming a settled production guarantee while that gate is open.
// (Provenance: dispatched as Claude Fable 5; exact model marker reconciled post-hoc by the
// orchestrator from the transcript.)

#import "_lib/spec.typ": spec-head, sotd, intro-section, references, dfn, note, cite

#set document(title: "zkSPARQL: Zero-Knowledge Query Proofs over SPARQL")
#set text(size: 11pt)
#set par(justify: true)
#set heading(numbering: "1.")

#spec-head()

#intro-section("abstract", "Abstract")[
  zkSPARQL is a proposal for proving, in zero knowledge, that the answer to a SPARQL query is
  correct with respect to one or more committed RDF graphs, without revealing the graphs
  themselves. This document describes the interfaces realised by the sparq reference
  implementation — the committed data model (RDF Dataset Canonicalization with Poseidon2
  commitments over the BN254 scalar field, and issuer attestation signatures over Baby
  Jubjub); the supported SPARQL fragment, given a normative algebraic definition, and its
  circuit family; the #dfn[proof manifest] interchange object; the verifier-nonce
  challenge–response; the verifier's fail-closed obligation set; the external trust anchors a
  relying party supplies; and the layered security-properties vocabulary with its ODRL
  admissibility profile — against an explicit threat model (section 5). The document is
  primarily an informative architecture overview: only the normative kernel enumerated in
  section 2 is candidate-normative, and the security-load-bearing encodings still pinned only
  by the implementation are named there as blocking transcription work. Parts that do not yet
  exist — a registered media type, a JSON-LD context, and a wire protocol — are named as
  explicit proposals. The entire scheme is research-grade: it has not been externally audited,
  and no production guarantee is claimed (see the Security and Privacy Considerations,
  section 17).
]

#sotd()

#intro-section("audit-status", "Implementation and audit status")[
  A reference implementation of everything marked *implemented* below exists in the sparq
  repository #cite("SPARQ") as two opt-in, unpublished crates, with an adversarial ("forge")
  test suite. However, the implementation has *not* been reviewed by an external
  cryptographer: the external audit gate (tracked in-repo as *sq-qhy4*) is open. Until it
  closes, soundness and attestation are *not production-ready*; every positive security
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
threshold FILTER, that a value was not revoked at an authoritative snapshot, or that two
hidden credentials agree on a join key.

The flow is challenge–response:

+ The verifier mints a fresh #dfn[verifier nonce] and hands it to the prover.
+ The prover evaluates the query over its committed source graphs and produces a proof
  manifest whose every sub-proof commits to that nonce.
+ The verifier checks the manifest against its own #dfn[trust anchors] (trusted issuer keys,
  an authoritative revocation snapshot, a holder registry, and its nonce store), enforcing a
  fixed, fail-closed obligation set (section 10).
+ Optionally, a policy engine decides whether the *method* used is admissible for the purpose
  at hand, by reasoning over the security-properties vocabulary (sections 12 and 13).

This document describes the interfaces of that pipeline as they exist in the sparq reference
implementation, so they can be reviewed, critiqued, and cited. Section 2 states precisely how
much of the text is normatively fixed — and how much is not. Where the implementation leaves
an interface unspecified (transport, media type, JSON-LD form), this document proposes one
and labels it a proposal. It is an Unofficial Proposal Draft; see the Status of This
Document.

= Document scope and maturity

== What this document is — and is not

This document is an *architecture and interface overview* of the zkSPARQL query-proof
pipeline, published in specification form so the design can be reviewed and cited. It is
#strong[not] yet a specification from which an independent party could build an interoperable
— let alone provably sound — prover or verifier: several security-load-bearing definitions  // privacy-claims-allow: explicit anti-overclaim — the draft is NOT a spec from which a provably-sound verifier could be built; negated usage, not a settled soundness claim (sq-qhy4)
are still pinned only by the reference implementation (section 2.3). A reader who needs a
conformance target should read section 2.2 for the exact clauses this draft does fix, and
section 4.3 for what may — and may not — be claimed against it.

== The normative kernel

Only the following clauses of this document are candidate-normative, and the RFC-2119
requirement keywords of section 4.1 are confined to them. Each is stable by design intent and
implementable from this text alone:

+ the committed-data-model algorithm identities — RDFC-1.0 canonicalisation and the
  per-graph Poseidon2/BN254 commitment (section 6.1) — and the cryptosuite resolution rule
  (section 6.2);
+ the fragment boundary and its algebraic semantics (sections 7.1–7.2), including the ban on
  representing a disclosed-base entailment re-check as a zero-knowledge proof (section 7.4);
+ the manifest type identifier (section 8.1);
+ the verifier-nonce discipline (section 9);
+ the verifier's fail-closed discipline: fail closed on any check that cannot be completed
  (section 4.3), the prefilter is not verification (section 10.1), a self-declared circuit
  identifier is never sufficient (sections 7.3 and 10.2), and first-failure rejection with no
  partial results and no warning downgrades (section 10.5);
+ the externality of every trust anchor, including the key-set subset rule (section 11);
+ the vocabulary scope and over-claim rules (sections 12.2–12.3), the provenance-only reading
  of `zk:sourceCryptosuite` (section 15), the relying-party audit-status obligations
  (section 17.1), and the deployment advice of sections 17.2 and 17.5;
+ the ODRL admissibility profile rules (section 13).

Everything else in this document — in particular the whole of sections 8.3–8.5 and the
obligation glosses of section 10 — is *descriptive* of the reference implementation and
carries no conformance force.

== Deferred normative cores

The following definitions are security-load-bearing but are #strong[not] defined in this
document; the reference implementation remains their only source of truth. Until a subsequent
draft transcribes them, interoperable or independently verifiable implementation of the
manifest format and verifier is *not possible from this text*:

+ the exact leaf encoding of canonical triples into field elements and the hashing
  arrangement above the leaves (section 6.1);
+ the complete `ProofManifest` member schema and the manifest canonicalisation algorithm
  (sections 8.1–8.2);
+ the exact condition of the Q6 blank-node guard (section 10.2) — its semantic minimum is
  given in section 7.2;
+ the precise clause of each of the twelve `bind_*` obligations, including the superset
  direction enforced by `bind_attributions` (section 10.3);
+ a public-input byte layout specified independently of the pinned proving toolchain
  (section 8.4).

Transcribing these five cores is tracked in-repo as bead *sq-rvgr2.7* and blocks any
candidate-normative successor to this draft.

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
  be disclosed ("clear") or hidden behind a proof-of-knowledge tier (section 10.3).

== Conformance classes

This document names three conformance classes, of which only one is fully definable today:

+ an #dfn[admissibility policy engine], which evaluates whether an annotated proof method
  satisfies an ODRL policy (sections 12–13). This class is fully definable from this
  document, and conformance to it may be claimed.
+ a #dfn[zkSPARQL verifier] (*provisional*), which checks proof manifests (sections 9–11).
  The kernel clauses of section 2.2 are *necessary* conditions on a verifier, and a verifier
  #strong[MUST] fail closed on any check it cannot complete — but they are #strong[not]
  sufficient: the exact clauses of the section-10 obligations are not transcribed in this
  draft (section 2.3), so *full verifier conformance is not yet definable, and no
  implementation may claim it against this document*.
+ a #dfn[zkSPARQL prover] (*provisional*), which produces proof manifests (sections 6–9);
  the same caveat applies via the untranscribed manifest schema (section 2.3).

#note[
  Until the deferred cores of section 2.3 are transcribed, the only claimable conformance is
  (a) kernel-clause conformance and (b) policy-engine conformance. This is a deliberate
  re-scoping: an earlier revision of this draft implied a full verifier class whose
  obligations were only informatively glossed, which was circular.
]

= Threat model and security goals

This section defines the adversary model against which the mechanisms of sections 6–11 are
the mitigation, and what each claimed security property *means* for this scheme. It is
placed before the mechanisms deliberately: every obligation in section 10 exists to counter a
capability listed here. The properties themselves are *claims* — none is audited
(section 17.1).

== Parties and trust relationships

- The #dfn[issuer] holds a signing key and attests committed graphs (section 6.2). The
  verifier trusts an issuer's *key* exactly insofar as the relying party placed it in the
  external key set K (section 11). Whether the issuer's attested *content* is true in the
  world is out of cryptographic scope: attestation transfers trust, it does not create it.
- The #dfn[holder] / #dfn[prover] controls the credentials and produces manifests. The
  verifier extends it *no* integrity trust: everything the prover sends is adversarial input
  until checked. Conversely the prover extends the verifier no privacy trust: the scheme's
  hiding goals exist precisely because the verifier is assumed curious.
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
+ *Colluding holder and issuer.* Can mint attestations for any content they like. The scheme
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
    gates (section 10).], [Claim (unaudited); the hidden-holder tiers are explicitly *not*
    sound (section 17.2).],
  [Binding], [A commitment identifies at most one canonical graph; the prover cannot open it
    to different data — the standard binding notion for commitment schemes
    #cite("PEDERSEN91").], [Poseidon2/BN254 commitment over the RDFC-1.0 canonical form
    (section 6.1).], [Claim (unaudited); fails against a quantum adversary
    (section 17.3).],
  [Hiding / zero-knowledge], [The manifest reveals nothing about the committed graphs beyond
    the proven statement, the public inputs, and the leakage dimensions declared in
    section 12.], [ZK proof system (Noir/Barretenberg, section 7.3); commitment hiding.],
    [Claim (unaudited); leakage dimensions are tracked, not bounded (section 17.4).],
  [Replay resistance], [An accepting manifest is bound to a single-use verifier nonce and
    cannot be accepted twice, nor transplanted to another request.], [Nonce discipline
    (section 9); audit gate 4 (section 10.4).], [Claim (unaudited); degraded by a
    non-durable nonce store (section 17.5).],
)

== Out of scope

Issuer content veracity (see above); side channels beyond the declared leakage dimensions
(timing is not modelled); denial of service; transport security (until section 14 is
realised); the quantum adversary (settled negative); and availability. The known deviations —
hidden-holder tiers, the dual-leaf lane — are catalogued in section 17.2 rather than silently
excluded here.

= Committed data model

== Graph canonicalisation and commitment

Each source RDF graph is canonicalised and then committed:

+ The graph #strong[MUST] be canonicalised with RDF Dataset Canonicalization (RDFC-1.0)
  #cite("RDF-CANON"), so that commitment values are independent of blank-node labelling and
  triple order.
+ The commitment #strong[MUST] be computed with the Poseidon2 permutation #cite("POSEIDON2")
  over the scalar field of the BN254 pairing-friendly curve (also known as alt-bn128)
  #cite("BN06"), per graph — one commitment per source graph.

#note[
  Editor's note — the exact leaf encoding of canonical triples into field elements, and the
  hashing arrangement above the leaves, are pinned by the implementation and are not
  respecified here; they are deferred core 1 of section 2.3 (transcription tracked as
  sq-rvgr2.7). The implementation also carries an optional "dual-leaf" value lane whose
  invariants are an accepted, documented downgrade; it is opt-in and unaudited (see
  section 17.2).
]

== Issuer attestation signatures

An issuer attests a committed graph by signing its commitment:

+ The attestation signature scheme is a Schnorr signature #cite("SCHNORR91") over the Baby
  Jubjub curve #cite("EIP2494") with a Poseidon2-derived challenge, identified by the
  cryptosuite IRI `https://sparq.dev/ns/zk#poseidon2-schnorr-v1`.
+ A verifier #strong[MUST] reject an attestation whose cryptosuite identifier it cannot
  resolve. There is #strong[no] default cryptosuite: an unresolved suite is a hard failure,
  not a fallback (fail-closed).

= Query fragment and circuit family

== The supported SPARQL fragment

This subsection is candidate-normative (section 2.2). The fragment is the monotone,
federation-free subset below. "IN (today)" means implemented by the reference verifier;
"IN (phase N)" means designed but #strong[not yet implemented]; `DEFERRED` is admissible
only after its stated re-entry condition; and `OUT` is excluded. These labels are part of
the fragment boundary, not an implementation roadmap that a manifest may anticipate.

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
  [`FILTER`], [IN (today: four numeric lanes); IN (phases 2–3: section 7.2 expression fragment)], [Monotone under SPARQL error-as-unsatisfied semantics.],
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
for the correctness obligation `bind_query_correctness` (section 10.3).

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

*Grammar.* A fragment pattern P over the committed graphs G1, …, Gn is generated by:

```
P ::= BGP | Filter(C, P) | Join(P1, P2) | Project(W, P)
    | Union(P1, P2) | Values(R) | Extend(v, E, P)
    | Path(s, path, o)
```

subject to: the first line is the currently implemented grammar; the second line is the
candidate-normative extension and is admitted only as its corresponding phase becomes
implemented. A `BGP` (a finite set of triple patterns over terms and variables) is evaluated
against *exactly one* committed graph; `C` is a value constraint drawn from the
datatype-bucketed comparison forms of section 7.1; and `Join` is the equality join of
section 7.1, whose two sub-patterns are evaluated over *distinct* committed graphs and share
at least one variable. `R` is a public `VALUES` row set, `E` is an expression from the table
above, `W` is a projection list, and `path` is an admitted path form with public bound k
where required.

*Evaluation.* The evaluation eval(P) is a set of solution mappings:

- eval(BGP over G) = the set of mappings μ with dom(μ) = vars(BGP) such that replacing each
  variable v in BGP by μ(v) yields a subgraph of G;
- eval(Filter(C, P)) = the set of μ in eval(P) such that μ satisfies C under the SPARQL 1.1
  operator semantics (section 17 of #cite("SPARQL11-QUERY")), with expression errors treated
  as *not satisfied*;
- eval(Join(P1, P2)) = the set of unions μ1 ∪ μ2 where μ1 is in eval(P1), μ2 is in eval(P2),
  and μ1 and μ2 are compatible.
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
therefore semantically meaningless in this fragment: a `Join` solution in which a shared
variable is bound to a blank node in more than one committed graph is *excluded from
eval(Join(P1, P2))*. This exclusion is the semantic minimum that the implementation's Q6
guard enforces (section 10.2).

*The correctness property.* The target property of `bind_query_correctness` (section 10.3)
is *result membership*: a manifest that discloses a solution mapping μ (or claims that a
solution exists) for a fragment pattern P over committed graphs G1, …, Gn is correct if and
only if μ is a member of eval(P) — respectively eval(P) is non-empty — as defined above.

#note[
  Editor's note — three boundaries of this definition are deliberate. (1) It is a *set*
  semantics; whether the implementation preserves duplicate-solution multiplicities (the bag
  semantics of #cite("PAG09")) is not pinned by this draft's grounding material;
  transcription is tracked as bead sq-rvgr2.7. (2) Projection (`SELECT` variable lists) is
  transcribed above as restriction of each solution mapping to the selected variables; this
  does not claim bag semantics or result completeness. (3) Result *completeness* — that no
  solutions were omitted — is #strong[not] claimed by `bind_query_correctness` as glossed here;
  whether any obligation claims it is not pinned, and is likewise tracked as bead sq-rvgr2.7.
  None of these boundaries weakens the membership property, but all three
  must be settled before a candidate-normative successor.
]

== The circuit family

Each sub-proof is generated against exactly one circuit of a fixed, named family, each
realising one operator instance of the fragment of section 7.2 (or one auxiliary statement:
revocation, issuer-set membership, holder binding). Circuits are authored in Noir
#cite("NOIR") and proved with the Barretenberg backend (see section 16 on toolchain pinning).
The family is:

#table(
  columns: 2,
  align: (left, left),
  table.header[Circuit identifier][Statement proved (descriptive gloss)],
  [`Scan`], [A BGP scan matches against a committed graph.],
  [`FilterInt`], [An integer-lane FILTER constraint holds.],
  [`FilterF64`], [An `xsd:double`-lane FILTER constraint holds.],
  [`FilterSignedInt`], [A signed-integer-lane FILTER constraint holds.],
  [`FilterDecimal`], [An `xsd:decimal`-lane FILTER constraint holds.],
  [`FilterValueDl`], [A value-dictionary-lane FILTER constraint holds.],
  [`RevokeUnset`], [A revocation bit is unset in a committed status snapshot.],
  [`HiddenIssuer`], [The issuer of a hidden credential lies in an attested set.],
  [`HolderPok`], [Holder proof-of-knowledge (hidden-holder tier; see section 17.2).],
  [`HolderSet`], [Holder set membership (hidden-holder tier; see section 17.2).],
  [`JoinEq`], [Two hidden credentials agree on an equality join key.],
)

The circuit identifier bound into a manifest is re-derived by the verifier (section 10.2); a
manifest #strong[MUST NOT] be accepted on the strength of its self-declared identifier alone.

#note[
  Editor's note — the exact in-circuit statement of each circuit (its public-input schedule
  beyond field 0 and its constraint relation) is pinned by the implementation; the glosses
  above are descriptive; per-circuit statements are transcribed under bead sq-rvgr2.7.
]

== Entailment regimes

Only *simple entailment* is proved in zero knowledge. A manifest #strong[MAY] declare RDFS/OWL
derivation steps, but these are re-checked by the verifier against *disclosed* bases
(obligation `bind_entailment`, section 10.3) — they are not proven in-circuit, and the
derivation bases are revealed to the verifier. An in-circuit closure proof is explicitly
deferred. A prover #strong[MUST NOT] represent a disclosed-base re-check as a zero-knowledge
entailment proof.

= The proof manifest

== Typing and canonical serialisation

A proof manifest is a JSON object. Its type member #strong[MUST] be the value
`urn:sparq:zk:ProofManifest`.

Every hash of a manifest — for nonce binding, deduplication, or audit — is defined over the
manifest's *canonical serialised form*, and the reference implementation canonicalises before
hashing. This draft, however, does #strong[not] define the canonicalisation algorithm: it is
deferred core 2 of section 2.3. Consequently two independent implementations cannot yet be
expected to agree on a manifest hash, no RFC-2119 requirement is attached to canonicalisation
here, and manifest-hash interoperability is expressly *not* offered by this draft. Defining
the canonical form is blocking transcription work (sq-rvgr2.7).

== Member schema

The member schema below records what this draft pins, and — explicitly — what it does not.
Rows marked "not transcribed" name member *groups* known to exist in the implementation whose
names and shapes are deferred (section 2.3); an implementer cannot round-trip a real manifest
from this table alone.

#table(
  columns: 3,
  align: (left, left, left),
  table.header[Member][Content][Status in this draft],
  [`type`], [The string `urn:sparq:zk:ProofManifest`.], [Pinned (normative kernel,
    section 8.1).],
  [`key_set`], [The prover's list of issuer verification keys. Accepted only as a *subset*
    of the external trust anchor K (section 11).], [Semantics pinned; the exact key encoding
    is not transcribed.],
  [`sub_proofs`], [Array of sub-proof objects.], [Presence pinned.],
  [sub-proof `circuit`], [One identifier from the family of section 7.3.], [Identifier set
    pinned; re-derived by the verifier, never trusted (section 10.2).],
  [sub-proof `proof_hex`], [Hex-encoded blob; layout in section 8.3.], [Partially pinned
    (descriptive).],
  [binding members; attribution set; entailment declarations; revocation references;
    holder-binding material], [The inputs consumed by the obligations of section 10.3.],
    [*Not transcribed* — deferred core 2 (section 2.3, sq-rvgr2.7).],
)

== Sub-proof encoding

Each sub-proof is carried as a single hex-encoded blob with the layout
`len | proof | len | public-inputs | vk` — a length-prefixed proof, a length-prefixed
public-input segment, and the verification key.

#note[
  Editor's note — the width and endianness of the two length prefixes are not pinned by this
  draft's grounding material; transcription is tracked as bead sq-rvgr2.7.
]

== Public-input encoding (descriptive; at-risk)

This subsection is *descriptive and at-risk*; it deliberately attaches no RFC-2119
requirement (see the note below for why). With the pinned toolchain of section 16, the
observed encoding of a sub-proof's public-input segment is:

- each public-input field element is encoded as exactly 32 bytes, big-endian;
- structs and arrays are flattened in row-major order;
- booleans encode as `0` or `1`; `u32`/`u64` values encode as their integer value;
- the segment carries no header and no per-element length prefix.

#note[
  This byte layout is *empirically pinned* to a specific Barretenberg release
  (`bb 5.0.0-nightly.20260324`, driven as a subprocess alongside `nargo 1.0.0-beta.21`); it
  was determined by observation, not derived from a backend specification, and it is not
  guaranteed stable across `bb` releases. A reverse-engineered, toolchain-fragile layout is
  not a conformance requirement, so this draft states it descriptively. A MUST-level layout
  will be introduced only once it is specified independently of the `bb` toolchain — deferred
  core 5 (section 2.3, sq-rvgr2.7); until then, interoperability across backend versions is
  not promised (section 16).
]

== Worked example (illustrative)

The example below is *illustrative only*: it is complete member-for-member against the
schema of section 8.2, but the elided hex (`…`) is not real proof material, the key encoding
is unpinned, and the members marked "not transcribed" in section 8.2 are absent. It is *not*
a test vector and cannot be verified; portable conformance fixtures are open future work
(section 16).

```json
{
  "type": "urn:sparq:zk:ProofManifest",
  "key_set": [
    "1c0aa5b7…e977"
  ],
  "sub_proofs": [
    {
      "circuit": "Scan",
      "proof_hex": "…"
    }
  ]
}
```

Decoding the single sub-proof's `proof_hex` blob per section 8.3, with the public-input
segment spelled out for its first — and only pinned — field:

#table(
  columns: 3,
  align: (left, left, left),
  table.header[Segment][Illustrative content][Meaning],
  [length prefix], [(width not pinned — section 8.3)], [Length of the proof segment.],
  [proof], [`…` (elided)], [The zero-knowledge proof for the `Scan` circuit.],
  [length prefix], [(width not pinned)], [Length of the public-input segment — here 64
    bytes: two field elements.],
  [public-input field 0 (bytes 0–31)],
    [`00000000000000000000000000000000` `000000000000000000000000075bcd15` (one 32-byte
    value, wrapped)], [*The verifier nonce* (section 9): a 32-byte big-endian BN254 scalar —
    illustratively the field element 123456789. A real nonce is a fresh random field
    element.],
  [public-input field 1 (bytes 32–63)], [`2f1e…` (elided)], [Illustrative only — e.g. a
    graph commitment. The per-circuit public-input schedule beyond field 0 is not pinned by
    this draft (section 7.3 note).],
  [verification key], [`…` (elided)], [Carried in the blob but *never trusted*: the verifier
    recomputes it from the canonical circuit (audit gate 2, section 10.4).],
)

= Challenge–response: the verifier nonce

The proof request is a nonce challenge:

+ The verifier #strong[MUST] mint a fresh #dfn[verifier nonce] — a BN254 scalar field
  element, exchanged in hexadecimal form — and deliver it to the prover *before* proving
  begins. Nonces #strong[MUST NOT] be reused across requests.
+ The prover #strong[MUST] commit the nonce as *public-input field 0 of every sub-proof* in
  the manifest.
+ The verifier #strong[MUST] record the nonce as used (single-use) *before* running the
  cryptographic checks of section 10.4, so that a manifest that fails late cannot be replayed
  against the same nonce.
+ If a manifest binds a nonce other than the one issued for the request, the verifier
  #strong[MUST] reject with a nonce-binding mismatch *and* #strong[MUST] still burn the
  issued nonce.

How the nonce is delivered and how the manifest is submitted is out of band and currently
unspecified; section 14 proposes a transport binding.

= Verification

Sections 10.2–10.4 describe the reference verifier's obligation set — the candidate checklist
a future normative revision will require of every verifier once the exact clauses are
transcribed (section 2.3). The fail-closed discipline of sections 10.1 and 10.5 is
kernel-normative now (section 2.2).

== Entry points

The reference verifier exposes two entry points:

+ a *structural prefilter* covering stages 1–2 (shape and consistency checks), which is
  #strong[not] sufficient on its own and #strong[MUST NOT] be treated as verification; and
+ *full verification*, which runs the prefilter, the structural re-checks of section 10.2,
  the binding obligations of section 10.3, and the cryptographic checks of section 10.4, in a
  fail-closed pipeline.

Only full verification confers the (unaudited — section 17.1) checking this document
describes.

== Structural re-checks

Full verification re-checks, independently of the prefilter:

- the *blank-node guard*: cross-graph joins on blank nodes are rejected (the "Q6" guard),
  enforcing the semantic exclusion of section 7.2 against a malicious prover (section 5.2);
- *attribution arity*: the attribution structure is well-formed;
- *circuit-identifier re-derivation*: the circuit identifier each sub-proof claims is
  re-derived from the statement it is bound to, and the two must agree;
- *strictly-increasing commitment ordering*: the manifest's graph commitments are strictly
  increasing, giving a canonical order and excluding duplicates.

#note[
  Editor's note — the exact condition of the Q6 guard (deferred core 3, section 2.3) and the
  precise well-formedness clause of the attribution-arity check are pinned by the
  implementation; transcription is tracked as bead sq-rvgr2.7. Section 7.2 states the semantic minimum
  the Q6 guard enforces.
]

== Binding obligations

The reference verifier enforces all twelve binding obligations below; each is fail-closed.
The obligation *set* is covered by the adversarial ("forge") test suite of the reference
implementation — this draft does not assert a per-obligation test inventory (section 16).
The one-line glosses are descriptive: the implementation remains the source of truth for each
obligation's exact clause until sq-rvgr2.7 transcribes them (section 2.3), after which a
subsequent draft will require the full set of every conforming verifier.

#table(
  columns: 2,
  align: (left, left),
  table.header[Obligation][Descriptive gloss],
  [`bind_query_correctness`], [The proven circuit statements correspond to the claimed SPARQL
    query; the target property is result membership under the fragment semantics of
    section 7.2.],
  [`bind_attributions`], [The disclosed attribution set satisfies the superset rule binding
    result rows to source graphs.],
  [`bind_issuer_attestations`], [Every issuer key used is a member of the *external* trusted
    key set K (section 11) — never merely of the manifest's own key list. This obligation
    *is* audit gate 3 (section 10.4).],
  [`bind_revocation`], [Revocation sub-proofs are bound to the authoritative status-list
    snapshot supplied by the relying party.],
  [`bind_joins`], [Join sub-proofs are bound to the participating graph commitments and
    variable slots.],
  [`bind_entailment`], [The declared entailment regime is honoured and derivation steps are
    re-checked against disclosed bases (section 7.4).],
  [`bind_holder_pop`], [Holder proof-of-possession is bound to the manifest.],
  [`bind_holder_binding`], [The clear (disclosed) holder is bound per the holder-binding
    policy and registry.],
  [`bind_hidden_revocation`], [Revocation checking for hidden credentials.],
  [`bind_hidden_issuer_attestations`], [Issuer attestation checking for hidden credentials.],
  [`bind_holder_pok`], [Hidden-holder proof-of-knowledge tier — explicitly *not yet sound*;
    opt-in only (section 17.2).],
  [`bind_holder_set`], [Hidden-holder set-membership tier — explicitly *not yet sound*;
    opt-in only (section 17.2).],
)

#note[
  Editor's note — the precise clause of each obligation, including the superset *direction*
  enforced by `bind_attributions`, is deferred core 4 of section 2.3; transcription is tracked
  as bead sq-rvgr2.7.
]

== Cryptographic checks and the four audit gates

The scheme defines exactly four #dfn[audit gates] — the cross-cutting binding checks whose
failure would each void soundness on its own. Their definitions, and the checks that enforce
them, map one-to-one:

#table(
  columns: 3,
  align: (left, left, left),
  table.header[Audit gate][Definition][Enforced by],
  [1 — public-input reconstruction], [The verifier independently reconstructs the expected
    public-input bytes of every sub-proof — with the verifier nonce at field 0 — and compares
    them byte-for-byte against the manifest's public-input segment; any difference rejects.],
    [The public-input reconstruction step of this section.],
  [2 — canonical verification key], [The verification key of every sub-proof is recomputed
    from the canonical circuit; a manifest-supplied key is never trusted as-is.],
    [The key-recomputation step of this section.],
  [3 — issuer signature and key set], [Every issuer key used is bound to the external key
    set K, and the issuer's signature over the graph commitment verifies.],
    [`bind_issuer_attestations` (section 10.3), per scan.],
  [4 — nonce single-use and binding], [The verifier nonce is fresh, single-use, recorded
    before the cryptographic checks, and burnt on mismatch.], [The nonce discipline of
    section 9.],
)

In addition to the four audit gates, each sub-proof is verified by the proving backend
against the recomputed key and reconstructed inputs (*backend proof verification*). This
check carries no audit-gate number: the audit gates are the binding checks layered *around*
backend verification, and backend verification is meaningless without gates 1 and 2 pinning
what is being verified.

After the obligations of section 10.3, full verification runs, fail-closed and in order:
public-input reconstruction (gate 1); canonical-key recomputation (gate 2); backend proof
verification of every sub-proof — with nonce single-use recorded *before* these checks and
nonce binding enforced as specified in section 9 (gate 4). Gate 3 is enforced earlier, per
scan, by `bind_issuer_attestations`.

== Fail-closed error handling

Every failure mode maps to an explicit variant of a closed error taxonomy (roughly seventy
variants in the reference implementation). A conforming verifier #strong[MUST] reject the
whole manifest on the *first* failed check, #strong[MUST NOT] return partial results, and
#strong[MUST NOT] downgrade any error to a warning.

= External trust anchors

All trust roots are inputs from the relying party. A conforming verifier #strong[MUST]
obtain each of the following out of band and #strong[MUST NOT] accept any of them from the
manifest:

+ the *trusted issuer key set K* — the manifest #strong[MAY] carry its own key list, but it
  is accepted only if it is a *subset* of K;
+ the *authoritative status-list snapshot* governing revocation, per the relying party's
  revocation policy;
+ the *holder registry* and *holder-binding policy*;
+ the *fresh verifier nonce* of section 9;
+ the *seen-nonce store* enforcing single use — this store #strong[SHOULD] be durable across
  verifier restarts (the reference implementation provides both a durable file-backed store
  and an in-memory store; the in-memory store forgets burned nonces on restart).

#note[
  The subset rule for K is codified from experience: an earlier revision that trusted the
  manifest's own key list was a review-identified soundness hole. Externalising every trust
  anchor is what closed it.
]

= Security-properties vocabulary

zkSPARQL methods are *annotated* with machine-readable security properties so that policy
engines can reason about them (section 13). The vocabulary is layered.

== Base vocabulary

The base vocabulary is the vendored `sec-prop` ontology, namespace
`https://w3id.org/zkp-sparql/sec-prop#`, defining eight property dimensions:
`UnlinkabilityStrength`, `UnlinkabilityScope`, `PostQuantumForgery`, `PostQuantumSnooping`,
`SignatureTypeLeakage`, `ProofSizeLeakage`, `CircuitAudit`, and `ValidityPeriodLeakage`
#cite("SEC-PROP"). The `sec-prop` vocabulary is prior work of this document's editor and
collaborators (Wright, Shadbolt, Zhao, Zhao, Braun #cite("SEC-PROP")): sections 12 and 13 of
this document derive from that work, which is vendored into the sparq repository under MIT
with its provenance record.

#note[
  Editor's note — the `w3id.org/zkp-sparql/` identifiers were minted as placeholders while
  the source repository was private. Before this draft advances, the permanent-identifier
  redirect must be confirmed live and stable.
]

== The secx extension

The sparq extension vocabulary (`secx`) adds the dimensions `ZeroKnowledgeType`, `Soundness`,
`Completeness`, `Hiding`, `Binding`, `Anonymity`, `Setup`, `Interactivity`,
`SelectiveDisclosure`, and `SingleUse`, plus four orthogonal axes:

- `AssuranceLevel`, ordered `Proven` > `Claimed` > `Conjectured`;
- `AuditStatus`, including the value `ExternalSignOffPending`;
- `Assumption` (e.g. the multi-party trust assumptions referenced by composed systems);
- `PropertyScope`, distinguishing `QueryProofLayer` from `SourceLayerOnly`.

A property that holds at the source layer only #strong[MUST NOT] be used to satisfy a
query-proof-layer constraint: source-layer facts do not transfer to the query-proof layer.

The dimension names shadow the security goals of section 5.3 deliberately: an annotation is a
machine-readable *claim* about a goal, and the assurance axis records how settled the claim
is.

== The over-claim rule

While the external audit gate (sq-qhy4) is open:

+ No sparq zkSPARQL method #strong[MAY] be annotated `secx:Proven` for any *positive* privacy
  or soundness property; such properties are at most `secx:Claimed` with
  `AuditStatus ExternalSignOffPending`.
+ Only *settled negative* facts — for example `PQForgeable`, `Replayable`, `SchemeRevealed`
  — #strong[MAY] carry `Proven`.

The reference implementation enforces this rule mechanically with three machine-checkable
guards over the annotation graph: an over-claim guard, a source-layer-transfer guard, and a
completeness guard.

= Policy-controlled admissibility

Relying parties express *which* proof methods they accept as ODRL 2.2 policies
#cite("ODRL22") over the vocabulary of section 12:

+ A policy using any `secx:requires` left-operand #strong[MUST] assert
  `odrl:profile` `https://sparq.dev/ns/odrl-secprop-profile#`.
+ Each such left-operand carries exactly one `secx:overDimension` fact identifying the
  property dimension it constrains.
+ Only the operator `odrl:gteq` is given a reduction; a constraint using any other operator
  #strong[MUST] be treated as *unsatisfied* — which denies.
+ A method is admissible only if it satisfies *every* constraint of the policy
  (default-deny).
+ In the fail-closed pre-check gate, the outcomes are Admitted, Denied, and ReductionError —
  and a reduction error #strong[MUST] be treated as a denial.

Base admission additionally checks the issuer's Schnorr signature over the RDFC-1.0
commitment, a SHACL #cite("SHACL") scope constraint, a reserved-predicate guard, and — for
clear holders — a WebID holder binding.

#note[
  A consequence worth stating plainly: a policy requiring
  `requiresAssurance odrl:gteq secx:Proven` on a positive property mechanically denies *every*
  current sparq zkSPARQL method while the external audit (sq-qhy4) is open. That is by design
  — it is the honest default for high-assurance relying parties.
]

= Transport, media type, and interchange

This section is entirely a *proposal*: none of it exists in the reference implementation
today. The manifest is a bare JSON object tagged with a URN, and both nonce issuance and
manifest submission are out of band.

== Media type (proposal)

A registered media type is proposed for the proof manifest — candidate
`application/vc+zksparql+json`, with a `+ld` variant once a JSON-LD context exists — and a
companion type for the nonce challenge. Until registration, implementations exchanging
manifests over HTTP have no content-type contract at all.

== JSON-LD context (proposal)

A JSON-LD context for the manifest is proposed so that a manifest can round-trip as a W3C
Verifiable Presentation #cite("VC-DATA-MODEL") and be consumed by generic data-integrity
processors. No such context exists today; the manifest does not currently round-trip.

== Wire protocol (proposal)

A challenge–response HTTP binding is proposed: an endpoint issuing single-use nonces and an
endpoint accepting manifest submissions bound to them. A live-service and asynchronous-proving
posture has been *designed only* (no server endpoint, job model, or async proving exists in
the implementation); any binding written here would be speculative and is deferred to a
subsequent draft.

= Relationship to W3C Verifiable Credentials

This section is informative.

- A *VC cryptosuite bridge* — off-circuit Data-Integrity verification of `eddsa-rdfc-2022`
  and `ecdsa-rdfc-2019` (P-256) source credentials #cite("VC-DI") at ingest — is implemented
  but *unmerged* at the time of writing (it exists only on a feature branch, as an opt-in
  feature). Any claim of VC ingest must be caveated accordingly.
- The P-384 profile of `ecdsa-rdfc-2019` is *not* implemented and fails closed as an
  unsupported key curve.
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
  this draft does not assert an itemised forge test per individual obligation of
  section 10.3 (its grounding material records the suite collectively). A conformance suite
  of portable fixtures (manifests that must verify, and mutated manifests that must fail with
  a specific error class) is required future work.
+ *Toolchain pinning.* The circuit family is pinned to an external toolchain
  (`nargo 1.0.0-beta.21`, `bb 5.0.0-nightly.20260324`) driven by subprocess, and the
  public-input byte layout of section 8.4 is empirically determined and therefore
  descriptive, not normative. Until the layout is specified toolchain-independently
  (section 2.3), cross-version interoperability is out of reach and even reference-level
  compatibility can only be claimed against the pinned toolchain.

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
external cryptographer audit is an open gate (sq-qhy4). Accordingly:

+ A relying party #strong[MUST NOT] treat a passing verification as a settled guarantee that
  the proven SPARQL statement holds against an adversarial prover.
+ Soundness and attestation are *not production-ready*; deployments that need a production
  guarantee are out of scope for this draft until the audit closes.
+ The over-claim rule of section 12.3 applies to every annotation surface: positive
  properties are at most `Claimed`, with audit status `ExternalSignOffPending`.

== Known-unsound and downgraded components

- The hidden-holder tiers (`bind_holder_pok`, `bind_holder_set`) are explicitly labelled *not
  yet sound* in the implementation and its documentation; remediation is tracked internally.
  They are opt-in only, and verifiers #strong[SHOULD] leave them disabled unless the residual
  risk is understood and accepted.
- The optional dual-leaf value lane carries an accepted, documented invariant downgrade and
  is likewise unaudited; it is opt-in.
- Only simple entailment is proved in zero knowledge; RDFS/OWL derivations are disclosed-base
  re-checks (section 7.4), which both weakens the zero-knowledge property for the disclosed
  bases and limits the entailment coverage.

== Post-quantum posture

The post-quantum posture is a *settled negative*. All issuer signature suites in scope
(Schnorr over Baby Jubjub, EdDSA, BBS+) rest on discrete-log hardness and fall to a
Shor-capable adversary; commitment binding likewise breaks under a cryptographically relevant
quantum computer, so *retrospective* soundness of previously accepted proofs fails as well.
The vocabulary records this honestly as negative `PostQuantumForgery` / `PostQuantumSnooping`
facts — these negatives are among the few annotations permitted to carry `Proven`
(section 12.3).

== Leakage and unlinkability

`SignatureTypeLeakage`, `ProofSizeLeakage`, `ValidityPeriodLeakage`, and the unlinkability
dimensions are tracked as vocabulary dimensions so that policies can constrain them; their
values for sparq methods are at most `Claimed` and are *not* settled guarantees. Verifiers and
relying parties should assume that proof size, timing, and suite choice may leak information
about the underlying credentials until an audit says otherwise.

== Replay and nonce hygiene

Replay resistance rests entirely on the nonce discipline of section 9: single-use recording
*before* the cryptographic checks, burn-on-mismatch, and a durable seen-nonce store. A
verifier using a non-durable store forgets burned nonces on restart and #strong[SHOULD NOT]
be exposed where replay across restarts matters.

== Admissibility reasons over annotations, not cryptography

The admissibility engine of section 13 reasons over *declared annotations*, not over the
cryptography itself. An "Admitted" outcome means the method's declared properties satisfy the
policy — it is not, and must not be presented as, an independent cryptographic finding.

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
    Proof of Correct SPARQL Evaluation over Verifiable Credentials]. Paper in submission at
    the time of writing (https://zksparql.org/); vocabulary source repository
    https://github.com/jeswr/sparql-zkp-ontologies, namespace `https://w3id.org/zkp-sparql/`.
    The `sec-prop` sub-vocabulary is vendored, with the sparq `secx` extension, in the sparq
    repository (MIT). Prior work of this document's editor — declared for citation integrity;
    sections 12–13 of this document derive from it.]),
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
