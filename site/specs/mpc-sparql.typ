// [FABLE] sq-rvgr2.3 — MPC-SPARQL: Secure Multi-Party Federated SPARQL — Requirements and
// Reference Architecture (Unofficial Proposal Draft). Authored from the estate recon digest
// (research/specs/mpc-sparql-estate-recon.md, bead sq-rvgr2.3): every normative statement
// below traces either to MERGED, TESTED behaviour in the sparq-mpc / sparq-zk-compose /
// sparq-fedplan-mpc estate (M0–M3) or is explicitly labelled a PROPOSAL. Revised per the
// Fable soundness review of the initial draft: the document is framed as a REQUIREMENTS +
// REFERENCE-ARCHITECTURE draft (not a protocol spec — the interoperable byte formats are
// open, §1.2), every normative section carries a conformance class + a testable-now vs
// future-implementation category stamp (§2.3), and audit scopes are reconciled (sq-qhy4
// clears the single-prover ZK layer ONLY, §12.1). No performance numbers appear in this
// source (build-boundary honesty scan enforces this).

#import "_lib/spec.typ": spec-head, sotd, intro-section, references, dfn, note, cite

#set document(title: "MPC-SPARQL: Secure Multi-Party Federated SPARQL — Requirements and Reference Architecture")
#set text(size: 11pt)
#set par(justify: true)
#set heading(numbering: "1.")

// Local helpers: an implementation-status aside and an open-issue aside. Both render through
// the shared `note` helper so they carry the ReSpec .note styling in HTML and the styled
// block in PDF. Status labels are load-bearing honesty devices — see §2.3.
#let impl-status(label, body) = note[#strong[Implementation status — #label.] #body]
#let open-issue(id, body) = note[#strong[Open issue (#id).] #body]

// A per-section conformance stamp (§2.3): which conformance class(es) the section's RFC 2119
// statements bind, and whether they are checkable against the reference estate today
// ("testable-now") or bind software that is not yet built ("future-implementation"). This is
// the mechanism that lets a conformance test suite separate the two — a review requirement.
#let conf(class, cat) = [#emph[Conformance: binds #class · category #strong[#cat] (§2.3).]]

#spec-head()

#intro-section("abstract", "Abstract")[
  MPC-SPARQL is a proposal for evaluating a federated SPARQL query across mutually
  distrusting data sources using secure multi-party computation (MPC), so that each source's
  private triples would never leave its custody in the clear and so that — once the
  proposal's verification layers are complete — the relying party would receive the query
  result together with machine-checkable evidence about how it was derived. Today such
  evidence exists only at the single-prover binding layer (§9); its federated, collaborative
  form is designed but not built (§10). The proposal targets four separable guarantees:
  input confidentiality during evaluation, correctness of the disclosed result,
  attested-source derivation (the result was computed over data signed by trusted issuers),
  and resilience against actively malicious participants.

  This document is a #strong[requirements and reference-architecture draft], not yet a
  protocol specification. It states the party and trust model, a consolidated adversary
  model, the secret-share and join-key encodings, per-operator disclosure routing, the
  secure evaluation operators, a simulation-tier wire protocol, and the verifier-side
  binding rules (freshness, key-set anchoring, public-input reconstruction). The
  interoperable artifacts a protocol specification would additionally need — the canonical
  query form and digest, the deployed party-mesh wire format, the verifier–federation
  handshake bytes, and the federated public-input layout — are explicitly open (§1.2), and
  no security property in this document is production-claimable until external audits of
  both the single-prover layer and the multi-party layer complete (§12.1).
]

#sotd()

= Introduction

Federated SPARQL query processing, as standardised in #cite("SPARQL11-FED"), assumes every
endpoint is willing to disclose the solution mappings it contributes. Many real federations do
not satisfy that assumption: the parties hold data that is individually private (personal
records, commercially sensitive holdings) yet jointly useful — a threshold decision over the
sum of per-party values, an intersection over hidden keys, a path query whose intermediate
nodes must remain undisclosed. MPC-SPARQL proposes to address this setting by evaluating the
private fragments of a federated query #emph[inside] a secure multi-party computation among
the data sources themselves, and by binding the disclosed output to cryptographic evidence a
relying party can check. That binding exists today only at the single-prover layer (§9); its
federated, collaborative form — the evidence a relying party would check for a multi-holder
run — is a design-stage proposal whose joint prove/verify operations are unimplemented
stubs (§10).

== Goals: the four guarantees

MPC-SPARQL decomposes its security objective into four distinct guarantees, which this
document tracks separately because their maturity differs sharply:

#table(
  columns: 3,
  align: (left, left, left),
  table.header[Guarantee][Meaning][Status in this draft],
  [A — Input confidentiality], [No party (and no coordinator or planner) learns another
    party's private inputs beyond the agreed disclosed output and the declared leakage
    envelope.], [Specified; implemented at the semi-honest honest-majority tier (§3.2, §7).],
  [B — Output correctness], [The relying party can verify the disclosed result is a correct
    evaluation of the agreed query over the committed data.], [Specified for the single-prover
    binding layer (§9); the #emph[collaborative] correctness proof is a proposal (§10).],
  [C — Attested-source derivation], [The result derives only from graphs whose commitments are
    signed by issuers in a relying-party-trusted key set.], [Single-prover binding is
    specified and implemented (§9.3); the federated/multi-prover form is a design-stage
    proposal (§10).],
  [D — Malicious security], [Guarantees A–C survive actively cheating participants.],
    [Partially specified: authenticated (IT-MAC) sharing and MAC-checked disclosure exist;
    the default operating tier remains semi-honest (§3.3, §12).],
)

== Position and maturity of this document

This draft is written against a concrete research implementation (the sparq project's
`sparq-mpc`, `sparq-zk-compose`, and `sparq-fedplan-mpc` crates, milestones M0–M3) and is
grounded in its estate reconnaissance record. Where this document states a requirement with an
RFC 2119 keyword, the requirement traces either to implemented, tested behaviour or to an
explicitly labelled proposal; §2.3 defines the conformance classes and categories. This
document is an Unofficial Proposal Draft (see Status of This Document): it is published to
make the design reviewable, not to claim a standard or a deployed security guarantee.

#strong[Why this is a requirements and reference-architecture draft, not a protocol
specification.] Two independent parties cannot yet interoperate from this document alone,
because every load-bearing interoperable artifact a protocol specification would need to pin
at the byte level is currently open:

+ the canonical SPARQL query form and its digest (§6.4);
+ the deployed party-mesh wire format (§8.2) — only the simulation-tier harness protocol of
  §8.1 has specified bytes;
+ the verifier–federation handshake formats (§8.3); and
+ the federated public-input byte layout that statement binding depends on (§9.2) — this
  revision adds a proposed single-prover statement serialisation (§9.1), itself pending
  reconciliation with the reference estate.

Accordingly, this document claims to be the #emph[requirements baseline and reference
architecture] that a future protocol-level revision must satisfy, and it #strong[MUST NOT]
be cited as an implementable protocol specification until those artifacts land.

== Related work and positioning

This subsection is informative. MPC-SPARQL sits at the intersection of three literatures;
its claim to novelty is narrow and architectural, not cryptographic.

#strong[Secure collaborative query systems (relational).] SMCQL #cite("SMCQL") compiles a
SQL fragment onto secure-computation backends for a mutually distrusting two-party
federation; Conclave #cite("CONCLAVE") scales MPC queries by pushing work into annotated
cleartext pre- and post-processing; Senate #cite("SENATE") provides maliciously secure  // privacy-claims-allow: describes prior work (Senate), not a sparq claim
collaborative analytics via a decomposition that localises cross-party computation; Cerebro
#cite("CEREBRO") adds a platform layer (policy, auditing, release control) around
collaborative cryptographic computation. MPC-SPARQL adapts two load-bearing ideas from this
line — the cleartext/secure operator split (Conclave's routing appears here as the
disclosed/hidden routing of §6.2) and policy-gated admission — to the SPARQL algebra. What
that line does not address is what is RDF-specific here: encoding a schema-free term model
into a protocol field (§4.2, including the blank-node problem), routing SPARQL-specific
operators such as property paths and hidden-key `DISTINCT` (§7.4), and — the main delta —
binding the disclosed output to #emph[attested-source derivation] (guarantee C): none of the
systems above aims to convince a third-party relying party that the result derives only from
issuer-signed inputs.

#strong[Private set operations.] The hidden-value join of §7.1 is, in its equi-join case, a
private set intersection evaluated inside generic MPC. Dedicated circuit-PSI protocols
#cite("PSTY19") achieve far better asymptotics for the standalone problem; this draft
therefore specifies the join #emph[interface] (secret-shared keys in, secret-shared match
indicators out), not the join algorithm, so a PSI-based operator can replace the current
exhaustive comparison (§7.1's implementation-status note) without a spec change.

#strong[Privacy for RDF and SPARQL.] Prior work on sensitive RDF is predominantly
access-control- and encryption-based: policy-driven SPARQL federation over access-controlled
data (SAFE #cite("SAFE")), encrypted RDF storage and exchange (HDT-crypt #cite("HDTCRYPT")),
and the access-control survey of Kirrane et al. #cite("KIRRANE17"). These protect data at
rest or gate #emph[who may ask]; none evaluates a join or aggregate whose operands remain
secret from #emph[every] party, which is the setting here. On the verification side,
collaborative zk-SNARKs #cite("OB22") are the primitive the §10 proposal targets — a proof
over a witness secret-shared among mutually distrusting provers — with the security caveats
analysed in #cite("EPRINT-2025-1026"); the single-prover binding layer this document builds
on follows the commit-and-prove line (for example #cite("ARTEMIS")).

#strong[What is claimed as new.] To the knowledge of the reference estate: (i) per-operator
disclosure routing expressed over the SPARQL algebra with a ratified leakage envelope
(§6.1–§6.2); (ii) composing MPC evaluation with verifier-anchored, issuer-signed
attested-source derivation for federated SPARQL (§9.3, §10.4); and (iii) a fail-closed
security-model vocabulary reported per SPARQL operator (§3.3). Every cryptographic primitive
used is standard and cited as such (§4, §7); no new primitive is proposed.

= Terminology and conformance

== RFC 2119 keywords

The key words #strong[MUST], #strong[MUST NOT], #strong[REQUIRED], #strong[SHALL],
#strong[SHALL NOT], #strong[SHOULD], #strong[SHOULD NOT], #strong[RECOMMENDED], #strong[MAY],
and #strong[OPTIONAL] in this document are to be interpreted as described in #cite("RFC2119")
and #cite("RFC8174") when, and only when, they appear in all capitals, as shown here.

== Terms

- A #dfn[federation] is the set of parties jointly evaluating one MPC-SPARQL query: the
  holders, a coordinator role (if the transport uses one), and the verifier-facing response
  path.
- An #dfn[issuer] is a signer of graph commitments; issuers are the roots of trust for
  guarantee C.
- A #dfn[holder] (data source) is a party contributing private RDF data; holders are
  mutually distrusting and possibly malicious.
- A #dfn[verifier] (relying party) is the consumer of the disclosed result and its evidence.
- A #dfn[planner] is the component that produces the federated query plan and the
  per-operator disclosure routing; it is untrusted for soundness (§3.1).
- A #dfn[share] is one party's point on a Shamir polynomial over the protocol field (§4.1).
- #dfn[Disclosed] and #dfn[hidden] are the two routing classes of a query operator: computed
  in the clear on public values, or computed inside the MPC on secret-shared values (§6.2).
- The #dfn[leakage envelope] is the explicit, machine-readable declaration of everything a
  protocol run reveals beyond the final disclosed result.

== Conformance classes, categories, and status labels

This document defines conformance requirements for four implementation classes:
#emph[federation], #emph[holder], #emph[verifier], and #emph[planner]. Each normative
section names the class it binds.

Because this draft mixes requirements that are checkable against the reference estate today
with requirements that bind software not yet built, every normative section additionally
carries exactly one of two #emph[conformance categories], stamped in an explicit
"Conformance:" line at the top of the section:

- #strong[testable-now] — the requirement is backed by implemented, tested behaviour in the
  reference estate, and a conformance test suite could check an implementation against it
  today;
- #strong[future-implementation] — the requirement binds a future implementation of a
  designed-but-not-built feature. No conformance test can be executed against it today, and
  it #strong[MUST NOT] be represented as an existing, working capability. All of §10, the
  canonical-digest requirement of §6.4, and the canonical statement serialisation of §9.1
  are in this category.

Where a section mixes categories, its stamp names the dominant category and the text marks
the exceptions inline. A conformance test suite #strong[MUST] treat the two categories as
disjoint: only testable-now requirements may appear in a test report about existing
software.

Two kinds of labelled asides further qualify the normative text:

- #emph[Implementation status] notes state whether the requirement is backed by implemented,
  tested behaviour in the reference estate, or is a #strong[proposal] — designed but not
  built (the aside-level view of the future-implementation category).
- #emph[Open issue] notes mark points this draft deliberately leaves unspecified, with the
  tracking identifier in the reference estate.

= Roles and trust model

== Party and role model

#conf("federation", "testable-now")

A conformant MPC-SPARQL federation #strong[MUST] distinguish four roles:

+ #strong[Issuer] — an honest signer and root of trust. An issuer's public key is a member of
  the disclosed key-set K (§4.4). Issuers sign graph commitments (§4.3) but do not
  participate in query evaluation.
+ #strong[Holder] — a data source. Holders are mutually distrusting and possibly malicious.
  Each holder acts #emph[both] as an MPC compute party and, in the collaborative-proof
  proposal (§10), as a collaborative prover.
+ #strong[Verifier] — the relying party. The verifier is modelled honest-but-curious: it
  issues a fresh challenge nonce N, supplies or agrees the trusted issuer key-set K, and
  checks the returned evidence. The verifier #strong[MUST NOT] be relied upon to keep
  disclosed outputs confidential.
+ #strong[Planner] — the producer of the federated plan and operator routing. The planner is
  #strong[untrusted]: its output #strong[MUST NOT] be trusted for soundness, and every
  security-relevant property of a plan (source admission, disclosure routing, leakage
  envelope) #strong[MUST] be independently ratified before execution (§6.1).

#impl-status("implemented")[
  The role split and the untrusted-planner seam are implemented in the reference estate: the
  end-to-end federated pipeline driver composes holder → share → disclosed-key join → secure
  aggregate → threshold verdict → proof statement, and a dedicated planner-routing crate
  performs privacy-aware default-deny source selection with leakage-envelope dual
  ratification.
]

== Corruption threshold

#conf("federation", "testable-now")

Each holder #strong[MUST] be exactly one compute party of an honest-majority MPC. For any
operation that requires interaction beyond local share arithmetic — in particular secret-shared
multiplication and secure comparison — the federation #strong[MUST] satisfy `n >= 2t + 1`,
where n is the number of compute parties and t the maximum number of tolerated corruptions.

A federation #strong[MUST] refuse to run (fail closed) when the threshold condition is not
met. It #strong[MUST NOT] silently downgrade to a weaker corruption model.

#impl-status("implemented, with a normativity note")[
  The reference pipeline checks `n >= 2t + 1` fail-closed before any multiplication or
  comparison, and the security-model registry fail-closes rather than downgrading when a
  requested tier (for example dishonest majority) is unavailable. The estate recon records
  that the #emph[normative] force of the refuse-to-run rule had not previously been pinned;
  this draft pins it as the MUST above.
]

== Security-model vocabulary

#conf("federation and holder", "testable-now")

An implementation #strong[MUST] describe the guarantee of every operator it executes over
three orthogonal axes, plus one flag:

- #strong[AdversaryModel] — `SemiHonest`, `Covert` (with a declared detection parameter
  epsilon), or `Malicious`;
- #strong[OutputGuarantee] — `Abort` (security with abort), `Fairness`, or
  `GuaranteedOutput`;
- #strong[CorruptionThreshold] — `HonestMajority` or `DishonestMajority`;
- #strong[PublicVerifiability] — whether a third party can check the run's evidence.

Guarantees #strong[MUST] be reported #emph[per operator] (see the operator classes of §7), not
as a single blanket claim for the whole query, because a plan routinely mixes disclosed
(crypto-free) and hidden (MPC) operators with different tiers.

Combinations ruled out by Cleve's impossibility result #cite("CLEVE86") — fairness or
guaranteed output delivery against a dishonest majority — #strong[MUST] be unrepresentable or
rejected by construction; the reference estate enforces this as a type-level invariant.

#impl-status("implemented")[
  The three-axis descriptor, the `PublicVerifiability` flag, the per-operator `OperatorClass`
  reporting, and the Cleve invariant are implemented in the reference backend.
]

== Consolidated adversary model and assumptions

This subsection consolidates, in one place, the adversary and environment assumptions under
which every security-relevant requirement in this document is stated. The assumptions
themselves are preconditions, not testable requirements; the one normative duty here — a
deployment operating outside any assumption below #strong[MUST] disclose that fact to its
relying parties — is testable-now and binds every conformance class.

- #strong[Corruption model.] The MPC layer assumes a #emph[static, honest-majority]
  adversary: at most t of the n compute parties are corrupted, `n >= 2t + 1` (§3.2), and the
  corrupted set is fixed before a run begins. Adaptive corruption is out of scope and
  unanalysed. The default tier assumes #emph[semi-honest] (passive) corruption (§12.2); the
  authenticated tier targets #emph[malicious] (active) corruption with abort and is only
  partially built (§7.5).
- #strong[Network and synchrony.] The protocol as drafted assumes synchronous, ordered,
  reliable message delivery over confidential, mutually authenticated point-to-point
  channels. This is an #emph[assumption], not a specified mechanism: the deployed transport
  that would realise it is not specified (§8.2), and the simulation-tier star coordinator
  (§8.1) realises the synchrony assumption trivially while violating the metadata-privacy
  expectation entirely — the coordinator observes all traffic patterns (§12.8).
- #strong[Setup and key distribution.] Issuer public keys are distributed out of band and
  their authenticity is assumed; this draft specifies no PKI, key-discovery, or
  revocation-distribution mechanism (revocation binding exists single-prover only, §9.4).
  There is no trusted dealer: each holder deals Shamir shares of its own inputs (§4.1). The
  trusted key-set K is supplied or agreed by the verifier (§4.4); it is a trust
  #emph[input] to the run, not a protocol output.
- #strong[What the verifier is trusted for.] The verifier is honest-but-curious (§3.1): it
  is trusted to supply a fresh single-use nonce and to actually run the §9 checks it
  reports; it is #emph[not] trusted with any party's private inputs (it never receives
  shares) and #emph[not] trusted to keep disclosed outputs confidential. Collusion between
  the verifier and a subset of holders is not analysed in this draft beyond the observation
  that the verifier holds no share material.
- #strong[Composition boundary (MPC layer vs single-prover ZK layer).] Guarantees B and C
  compose two distinct mechanisms: the MPC evaluation layer (§7) and the single-prover ZK
  verification layer (§9). A #dfn[sub-proof] is one proof checked by the §9 layer; a
  #dfn[composed verification] is the conjunction (logical AND) of the §9 checks over every
  sub-proof of a run, cross-bound by the shared challenge nonce N as public-input field 0 of
  every sub-proof and by the shared statement of §9.1. The composition claim made by this
  draft is exactly that conjunction under shared binding — nothing stronger. In particular,
  no universal-composability or joint-state analysis of the MPC-plus-ZK composition has been
  performed, and the collaborative proof that would tie the MPC execution itself into the
  proven statement is a proposal (§10).

#open-issue("adversary-model details to reconcile with the reference estate")[
  Whether the reference estate's share-opening and reconstruction steps assume a broadcast
  channel, and whether they tolerate a rushing adversary, is not recorded in the estate
  digest this revision was authored from (a no-code-reading authoring constraint applied); a
  future revision must pin both against the crate sources.
]

= Data model and encodings

== Protocol field and secret-share format

#conf("holder and federation", "testable-now")

The protocol field is the prime field `F_p` with `p = 2^61 - 1` (a Mersenne prime). A
#dfn[Shamir share] (#cite("SHAMIR79")) of a secret `s` is a pair `(x, y)`:

- `x` is the party index, an unsigned 64-bit integer, and #strong[MUST] be at least 1;
- `y = f(x)` is the polynomial evaluation in `F_p`, where `f` is a degree-t polynomial with
  `f(0) = s`.

The evaluation point `x = 0` is #strong[RESERVED] for the secret itself and #strong[MUST
NEVER] be handed to any party as a share.

Randomness used for share masking #strong[MUST] come from a cryptographically secure  // privacy-claims-allow: normative requirement on the RNG, not a soundness claim about sparq
pseudorandom generator; the reference estate uses ChaCha20 #cite("RFC8439") and confines any
deterministic test generator behind a test-only build feature that #strong[MUST NOT] be
enabled in production builds.

After each secret-shared multiplication, parties #strong[MUST] perform degree reduction (the
BGW protocol family, #cite("BGW88")) so the sharing degree returns to t before further
operations.

#impl-status("implemented")[
  Field, share format, reserved zero point, ChaCha20-based masking, and BGW degree reduction
  are implemented and tested in the reference estate (milestones M0–M3, merged).
]

#strong[Statistical security of the 61-bit field.] The choice of `F_p` with `p = 2^61 - 1`
is a #emph[performance] choice (native 64-bit arithmetic with cheap Mersenne reduction); it
caps, rather than follows from, the protocol's statistical-security level. This draft sets
the target statistical-security parameter at `s = 40` (a common choice in the MPC
literature) and accounts against it as follows:

- #strong[IT-MAC forgery (§7.5).] A single information-theoretic MAC check over `F_p` is
  forgeable with probability at most `1/p ≈ 2^-61`; a run performing R MAC checks has total
  forgery probability at most `R · 2^-61` by the union bound, which stays below `2^-40` for
  any R up to `2^21` checks. The per-check margin therefore #emph[exceeds] the `s = 40`
  target, but by 21 bits only — far short of the `2^-128` a reader might assume — and it
  #strong[MUST NOT] be described as providing 128-bit security of any kind.
- #strong[Join-key collisions (§4.2).] The term encoding is birthday-bounded: q encoded
  terms collide with probability approximately `q^2 / 2^62`. Meeting a `2^-s` failure bound
  requires `q <= 2^((62-s)/2)`: roughly `2^11` (about two thousand) hidden join-key terms
  for `s = 40`, `2^16` for `s = 30`, and `2^21` (about two million) for `s = 20`. This — not
  MAC forgery — is the binding constraint of the 61-bit field, and it is why §4.2 mandates
  fail-closed collision detection.

An implementation #strong[MUST] document the statistical-security level it actually achieves
for its run sizes, and a deployment whose hidden-key count pushes the collision bound above
`2^-40` #strong[MUST] either reject the run or disclose the weakened bound to its relying
parties. Restoring headroom (a larger field, or an extension-field MAC) is an open design
choice traded against the performance that motivated `2^61 - 1`.

== Join-key encoding for private terms

#conf("holder and federation", "testable-now")

When an RDF term serves as a #emph[hidden] join key (§7.1), it #strong[MUST] be encoded into
`F_p` as:

```
encode(term) = reduce_mod_p( SHA-512( DOMAIN_TAG || ntriples(term) ) )
```

where `ntriples(term)` is the canonical N-Triples serialisation of the term, `SHA-512` is as
defined in #cite("FIPS-180-4"), and the domain-separation tag is the byte string
`DOMAIN_TAG = "sparq-mpc/term-join-key/v1\0"`. Implementations #strong[MUST NOT] reuse this
tag for any other purpose and #strong[MUST] version any change to the encoding through a new
tag.

The encoder provides a #emph[statistical] collision bound only — a birthday bound of
approximately `q^2 / 2^62` for q encoded terms — and this bound is #strong[not] a
computational-security guarantee. Implementations #strong[MUST] run collision detection over
the disclosed-side key material they can see and #strong[MUST] fail closed on a detected
collision rather than emit an incorrect join.

#impl-status("implemented; the in-circuit form is not")[
  The domain-separated encoder and the fail-closed collision-detection path are implemented.
  The #emph[in-circuit] proof that a hidden join key was encoded correctly from a committed
  term — required before guarantee B can cover hidden joins end-to-end — is designed, not
  built (it is part of the M4 verification work, §10).
]

#open-issue("blank-node join semantics")[
  The encoder operates on the term's serialisation, so blank nodes match on label only.
  Cross-graph blank-node identity is therefore undefined at the encoder. #strong[Proposal]:
  a federation MUST apply a canonical blank-node relabelling (RDF dataset canonicalisation,
  #cite("RDF-CANON")) to every graph before encoding, or exclude blank nodes from hidden join
  keys. This draft does not yet pin one of the two options.
]

== Graph commitments and issuer attestations

#conf("issuer and verifier", "testable-now (single-prover)")

Each named graph `G_i` contributed by a holder carries a commitment `C(G_i)`, and each
commitment is signed by its issuer as:

```
Sign( sk_i, { C(G_i), true-triple-count, salt } )
```

The reference signature scheme is Schnorr over the Baby-JubJub curve with a Poseidon2-based
challenge, using the domain-separation tags `ZKSIG_C3` / `ZKSIG_C4`. Implementations
#strong[MUST] preserve the domain separation of the challenge computation; a signature
computed under any other tag #strong[MUST NOT] be accepted as a graph attestation.

#impl-status("implemented at the single-prover layer")[
  Commitment signing and its verifier-side binding are implemented in the single-prover
  verification layer (§9.3). Their #emph[distributed] counterpart over a secret-shared
  witness is an unsolved research problem and is deferred (§10.5).
]

== The disclosed issuer key-set K

#conf("federation and verifier", "testable-now")

The set K of issuer public keys that a run treats as trusted #strong[MUST] be canonicalised —
sorted and deduplicated — before it is bound into any statement or digest, so that the proof
statement and public-input digest are invariant to caller ordering and multiplicity.

#impl-status("implemented")[Key-set canonicalisation is implemented in the reference
pipeline and reflected in the proof-statement structure (§9.1).]

= Protocol overview

This section is informative. A full MPC-SPARQL exchange has six steps; the normative content
of each step lives in the section referenced.

+ #strong[Commit and attest] — each issuer signs the commitment of each contributed graph
  (§4.3).
+ #strong[Challenge] — the verifier delivers the query Q, a fresh single-use nonce N, and the
  trusted key-set K to the federation (§8.3, §9.4).
+ #strong[Plan and ratify] — the untrusted planner produces source selection, operator
  routing, and a leakage envelope; the federation independently ratifies them, default-deny
  (§6).
+ #strong[Evaluate] — holders secret-share their private inputs and execute the routed plan:
  disclosed operators in the clear, hidden operators inside the MPC (§7).
+ #strong[Prove] — the holders jointly produce evidence binding the disclosed result to the
  statement of §9.1. #emph[This step is a design-stage proposal: the collaborative proof is
  not built (§10); the implemented evidence today is the assembled statement plus the
  single-prover binding layer of §9.]
+ #strong[Verify] — the verifier checks freshness, key-set anchoring, and the evidence, and
  recomputes every post-processing operator that is a deterministic function of the disclosed
  multiset (§6.3, §9).

#note[
  The reference estate implements this flow end-to-end for a concrete multi-holder scenario
  (a four-party threshold decision over a secure sum), with per-operator disclosed/hidden
  routing and a differential correctness anchor against an equivalent single-store
  evaluation. The proof step of that driver assembles the statement shape and then stops,
  honestly, at the unimplemented collaborative proof.
]

= Planning, admission, and disclosure routing

== The untrusted planner

#conf("federation and planner", "testable-now")

The planner's output #strong[MUST NOT] be trusted for soundness. A conformant federation
#strong[MUST]:

- perform #strong[privacy-aware source selection with default-deny]: a source is admitted to
  a plan only when an explicit policy admits it, never by omission;
- ratify the plan's #strong[leakage envelope] independently of the planner (dual
  ratification): the parties whose data is at stake #strong[MUST] be able to inspect and
  refuse the declared leakage before execution;
- reject any plan that routes an operator contrary to the routing rules of §6.2.

#impl-status("implemented")[
  Default-deny privacy-aware source selection, disclosed/hidden operator routing, and
  leakage-envelope dual ratification are implemented in the reference planner-seam crate.
]

Property-level admissibility policies (for example, an ontology-driven pre-check that a given
property may be disclosed to a given audience) #strong[SHOULD] gate admission of a source's
properties into the plan.

#open-issue("federated admissibility wiring")[
  The admissibility pre-check primitive exists and is tested in the reference estate
  (consuming a vendored security-properties ontology), but it is exercised only in
  single-party form today; wiring it into the federated multi-source admission path is open.
]

== Per-operator disclosure routing

#conf("federation and planner", "testable-now")

Every operator of the federated plan #strong[MUST] be explicitly tagged with exactly one
routing class:

- #strong[Disclosed] — the operator computes in the clear over public values (for example an
  equi-join on a global IRI key that all parties may see); no cryptography is applied;
- #strong[Hidden(OperatorClass)] — the operator computes inside the MPC over secret-shared
  values, and carries its operator class for per-operator guarantee reporting (§3.3).

The complete routing #strong[MUST] be inspectable by every holder before execution and
#strong[MUST] be surfaced to the verifier in the response, so that what was and was not
protected is never implicit.

#impl-status("implemented")[Explicit routing tags and their surfacing in the federated
response are implemented in the reference pipeline.]

== Post-processing convention: no proof of revealed properties

#conf("verifier", "testable-now")

Any operator that is a deterministic function of the #emph[disclosed] result multiset —
`DISTINCT`, `ORDER BY`, `LIMIT`, `OFFSET`, and the aggregates `COUNT`, `SUM`, `AVG`, `MIN`,
`MAX` of #cite("SPARQL11-QUERY") when applied to disclosed values — #strong[MUST] be recomputed by the verifier outside
the cryptographic core, and #strong[MUST NOT] be proven in-circuit. This keeps the proven
statement minimal and removes a class of needless circuit obligations.

(Operators over #emph[hidden] values — for example a secure sum whose operands never appear
in the clear — are not post-processing and are governed by §7.)

#impl-status("implemented (as a convention of the reference design)")[]

== Canonical query digest

#conf("federation and verifier", "future-implementation")

The statement of §9.1 binds a digest of the query. This draft #strong[REQUIRES] that the
digest be computed over a #emph[canonical] form of the federated query, so that two textually
different but identical-in-algebra queries cannot yield distinguishable statements; however,
the canonicalisation algorithm is not yet specified.

#open-issue("query canonicalisation")[
  The reference pipeline currently digests the query string verbatim and declares
  canonicalisation the caller's responsibility. A normative canonical SPARQL form (algebraic
  normalisation and serialisation) must be specified before interoperable statements are
  possible. Until then, implementations #strong[MUST] document exactly what byte string they
  digest — this interim documentation duty is the one #emph[testable-now] requirement of
  this section.
]

= Secure evaluation operators

This section binds the #emph[holder] and #emph[federation] classes. Each operator family
below reports its class for the per-operator guarantee vocabulary of §3.3.

#conf("holder and federation", "testable-now, except where an implementation-status note
says otherwise (§7.5)")

== Joins

Two join forms are defined:

- #strong[Disclosed-key equi-join] — when the join key is a global, public IRI, the join
  #strong[MAY] be computed crypto-free in the clear (routing class Disclosed).
- #strong[Hidden-value join] — when the join key is private, the join #strong[MUST] operate
  on secret-shared field encodings of the key (§4.2) and decide matches via secret-shared
  field equality, never revealing the keys.

#impl-status("implemented")[Both join forms are implemented and tested. The hidden join's
current form compares candidate pairs exhaustively; an oblivious sort-merge or circuit-PSI
join with better asymptotics is an open improvement in the reference estate, not part of this
draft's normative surface.]

== Secure comparison

A hidden comparison (greater-than, equality-to-bit) #strong[MUST] disclose #emph[only] the
boolean verdict bit and #strong[MUST NOT] reveal the operands. The reference realisation uses
a Rabbit-style full-field bit decomposition #cite("RABBIT") covering operand magnitudes up to
`2^60`.

#impl-status("implemented")[Verdict-bit-only comparison is implemented; a
MAC-checked, tamper-evident variant exists for the authenticated tier (§7.5). It is not maliciously secure, since a value tamper on the second operand of a multiplication is not caught, and it is not externally audited (sq-qhy4).]  // privacy-claims-allow: states the authenticated tier is NOT maliciously secure and not externally audited (sq-qhy4)

== Secure aggregates and threshold verdicts

Addition of secret-shared values #strong[MUST] be computed as local share addition
(zero-round). A #dfn[threshold verdict] — "is the (hidden) aggregate above a public
threshold?" — #strong[MUST] disclose only the boolean verdict bit across the API boundary and
#strong[MUST NOT] reconstruct the exact aggregate. Implementations #strong[SHOULD] enforce
this structurally (that is, the exact sum is unreachable through the public interface, not
merely unaccessed).

#impl-status("implemented")[Zero-round share addition, the verdict-bit-only threshold
comparison, and a structural assertion that the exact sum does not cross the API boundary are
implemented in the reference pipeline.]

== Oblivious reordering, DISTINCT over hidden keys, and property paths

When result order or multiplicity over #emph[hidden] values would otherwise leak information:

- reordering #strong[MUST] use an oblivious shuffle (Waksman/Beneš permutation network
  #cite("WAKSMAN68")) or an oblivious sort (Batcher odd-even mergesort #cite("BATCHER68"));
- `DISTINCT` over hidden keys #strong[MUST] be computed obliviously (hidden-key
  deduplication), not by disclosing keys;
- bounded-length property paths #strong[MAY] be evaluated either as a crypto-free unroll when
  the path endpoints and intermediates are disclosed, or as a hidden-intermediate exactly-k
  chain when intermediates are private; the planner #strong[MUST] guard that an unbounded
  path is never routed into the hidden tier.

#impl-status("implemented")[Oblivious shuffle and sort, the set-returning oblivious output
path, hidden-key DISTINCT, both property-path forms, and the planner guard are implemented
and tested.]

== Robustness and the authenticated (malicious) tier

For resilience against tampered shares and cheating parties:

- reconstruction #strong[SHOULD] use robust decoding (Reed–Solomon / Berlekamp–Welch
  #cite("BW86")) so that tampered shares are detected and, within the threshold, corrected;
- the #dfn[authenticated tier] uses information-theoretic MACs (IT-MACs) on shares; in that
  tier, a verdict bit #strong[MUST] be MAC-checked #emph[before] it is opened, at the minimal
  honest-majority threshold `n = 2t + 1`, yielding malicious security with abort.

#impl-status("partially implemented — default tier is semi-honest")[
  Robust reconstruction, the IT-MAC authenticated-sharing foundation, authenticated
  comparison, and MAC-checked-before-open disclosure are implemented. The #emph[full]
  honest-majority malicious-with-abort protocol suite is incomplete (open work items in the
  reference estate, sq-km34 series), and the default operating tier of the reference pipeline is
  #strong[semi-honest honest-majority]. Deployments MUST NOT describe the default tier as
  maliciously secure (§12).  // privacy-claims-allow: honesty caveat forbidding the overclaim; the default tier is semi-honest, not a sparq claim
]

= Transport and message formats

== Simulation-tier wire protocol

#conf("federation", "testable-now (within the simulation tier only)")

The reference estate defines a wire protocol for a #emph[star-coordinator simulation] tier,
in which each party is a separate process holding only its own share column and a coordinator
relays protocol steps. Within that tier, the following is normative:

- messages are self-describing, length-prefixed frames;
- the message set is `Shares(Vec<Share>)` (coordinator → party), `Open(Vec<Share>)` (party →
  coordinator), `Step(StepCode)`, `Swap{a, b, swap}`, and `Done`;
- `StepCode` is one of `SumAndOpen = 1`, `MulAndOpen = 2`, `SwapNetworkAndOpen = 3`;
- field elements travel as canonical unsigned 64-bit big-endian integers, 8 bytes each.

#impl-status("implemented — simulation harness only")[
  This wire protocol is implemented as a loopback network-tier harness. It exists to make the
  protocol's communication structure concrete and testable; it is #strong[not] the deployed
  protocol.
]

== Deployed party-mesh transport

#open-issue("deployed transport unspecified")[
  The deployed, full party-mesh transport — direct authenticated pairwise channels between
  holders, channel establishment, framing, and session binding — is #strong[not specified] by
  this draft. The star-coordinator protocol of §8.1 is a simulation harness and #strong[MUST
  NOT] be deployed as-is: a relaying coordinator is a single point of traffic observation and
  denial. A future revision must define the real inter-party message set.
]

== Verifier–federation handshake

#open-issue("handshake formats unspecified")[
  The message formats for the verifier ↔ federation exchange — delivery of the query Q and
  the fresh nonce N, delivery or agreement of the trusted key-set K, and the response
  envelope carrying the disclosed result, the routing report (§6.2), and the evidence — are
  not yet formalised; they exist as design prose in the reference architecture. This draft
  records their #emph[required content] (Q, N, K inbound; result, routing, statement,
  evidence outbound) but not their bytes.
]

= Verification (single-prover binding layer)

This section binds the #emph[verifier] class. It specifies the binding rules of the
#emph[implemented] single-prover verification layer, which the federated proposal of §10
inherits. #emph[Sub-proof] and #emph[composed verification] are as defined in §3.4: a
sub-proof is one proof checked by this layer, and a composed verification is the conjunction
of these checks over every sub-proof of a run under the shared nonce and statement binding.

#conf("verifier", "testable-now (single-prover), except the canonical statement
serialisation below, which is future-implementation")

== The proof statement

The public statement bound by the evidence is the tuple:

```
ProofStatement = {
  query:              canonical SPARQL digest (see §6.4),
  disclosed_key_set:  K, canonicalised per §4.4,
  challenge:          N, the verifier's fresh nonce,
  disclosed_result:   the disclosed output (result multiset or verdict bit)
}
```

#impl-status("implemented (shape)")[The statement structure is implemented and assembled by
the reference pipeline. What #emph[binds] it for the federated case — the collaborative proof
— is a proposal (§10).]

#strong[Canonical statement serialisation (category: future-implementation).] For the
binding rules of §9.2 to be independently implementable, the mapping from `ProofStatement`
to bytes must be deterministic and canonical. The following single-prover encoding is
#strong[REQUIRED] of any implementation claiming conformance to a future protocol-level
revision of this draft; the federated (multi-holder) layout remains open (§9.2's open
issue):

+ Component order is fixed: `challenge` N first — so that the nonce is public-input field 0,
  per §9.2 — then the `query` digest, then `disclosed_key_set` K, then `disclosed_result`.
+ Each component is length-prefixed with an unsigned 64-bit big-endian byte count and
  concatenated without padding; field elements are serialised as canonical unsigned 64-bit
  big-endian integers, consistent with the wire encoding of §8.1.
+ K is serialised in its canonical sorted, deduplicated order (§4.4), each key individually
  length-prefixed.
+ Where a single digest of the statement is bound instead of the full byte string, it
  #strong[MUST] be computed as `SHA-512( DOMAIN_TAG_STMT || <the concatenation above> )`
  reduced by the rule of §4.2, with
  `DOMAIN_TAG_STMT = "sparq-mpc/proof-statement/v1\0"` — a fresh tag that #strong[MUST NOT]
  be shared with the join-key encoder or any other use.

The full public-input vector that §9.2 requires the verifier to reconstruct additionally
packs the graph commitments, any disclosed FILTER operator and operands, and per-row source
attribution; their packing exists in the reference implementation but is not yet transcribed
into this draft.

#open-issue("statement-serialisation reconciliation")[
  Whether the reference estate's existing single-prover public-input packing already matches
  the encoding above is #strong[not established]: this revision was authored from the prose
  estate digest under a no-code-reading constraint, and the digest records the mapping as
  previously unspecified. Before any protocol-level revision, the estate's actual packing
  must be reconciled with — or migrated to — this encoding, and conformance test vectors
  published.
]

== Public-input binding

- Public-input field 0 of #strong[every] sub-proof #strong[MUST] be the verifier's fresh
  nonce N.
- The verifier #strong[MUST] recompute the verification key from a circuit identifier it
  re-derives itself; it #strong[MUST NOT] accept a prover-supplied verification key.
- The verifier #strong[MUST] reconstruct the expected public inputs itself — binding the
  challenge and nonce, the graph commitments, any disclosed FILTER operator and operands, and
  per-row source attribution — and compare against the proof's public inputs; it #strong[MUST
  NOT] trust a prover-supplied public-input vector.

#impl-status("implemented — single-prover / single-manifest")[
  Verifier-side public-input reconstruction with these bindings is implemented for a single
  prover and a single manifest.
]

#open-issue("federated public-input layout — tracked as sq-34ml")[
  The byte layout that packs #emph[multiple] holders' commitments, result rows, and per-row
  attribution under #strong[one] federated statement does not exist yet; it is a prerequisite
  for the verifier-side attestation gate of §10.4 (tracked as sq-f7bu).
]

== Attestation binding and the trust anchor

The trust anchor for attestation binding #strong[MUST] be the #emph[external, relying-party
supplied] key-set K. The prover's manifest #strong[MUST NOT] be able to widen the trusted
set: a prover #strong[MAY] narrow K (claim fewer trusted issuers than the verifier allows)
but #strong[MUST NOT] widen it. Any attestation that verifies only under a key outside K
#strong[MUST] be rejected.

#impl-status("implemented — single-prover")[External-anchor attestation binding with
narrow-not-widen semantics is implemented at the single-prover layer. The federated,
multi-source form is a proposal (§10.4).]

== Freshness and replay resistance

- The verifier's nonce #strong[MUST] be single-use.
- The nonce #strong[MUST] be recorded (consumed) #emph[before] the cryptographic
  verification gate runs, and #strong[MUST] be burned even when verification fails
  (burn-on-mismatch), so an attacker cannot retry against the same nonce.
- There is #strong[no opt-out]: a conformant verifier has no configuration in which replay
  checking is disabled.
- A nonce-binding mismatch #strong[MUST] be reported as a distinct, non-retryable failure.

#impl-status("implemented — single-prover")[]

#open-issue("multi-source revocation and freshness windows")[
  Revocation-status binding and its freshness window are implemented single-prover only. The
  federation-level policy — whose status lists govern which holder's credentials, and how the
  freshness windows compose across sources — is unspecified in this draft.
]

= Collaborative proof and distributed attestation (proposal)

#conf("a future collaborative-proof implementation (no existing software)",
"future-implementation, in full")

#note[
  #strong[This entire section is a proposal.] The collaborative zero-knowledge proof over a
  secret-shared witness is #strong[not implemented]: in the reference estate, the joint
  `prove` and `verify` operations are explicit not-yet-implemented stubs, the `Proof` object
  is a deliberately field-less placeholder (the proof system is unchosen), and the
  `AttestationShare` carries no fields. Nothing in this section describes working software,
  and no statement in it may be cited as an existing guarantee.
]

== Obligations of a future collaborative proof

Any future collaborative proof implementation #strong[MUST] produce a joint proof that binds,
under the statement of §9.1:

+ subset soundness — the disclosed result is a subset-correct evaluation of the agreed query
  over the committed data (the disclosed rows are among the true answers);
+ attested sources — every contributing graph commitment `C(G_i)` is issuer-signed under a
  key in K;
+ statement completeness — the query digest, every hidden FILTER's operator, operands, and
  verdict, per-row source-graph attribution, and the challenge nonce N.

== Witness validation before proving

A collaborative prover #strong[MUST] validate the extended witness #emph[before] proving
begins. Proving over an invalid or inconsistent secret-shared witness is not merely a
completeness failure: published analysis of collaborative zk-SNARK constructions
#cite("EPRINT-2025-1026") shows it can #strong[leak honest provers' private inputs].
Witness validation is therefore a security obligation, not an optimisation.

== Proof-system choice

The proof system (a collaborative SNARK, or an MPC-evaluated instance of an existing
single-prover system) and the byte layout of the joint proof are #strong[unspecified]. The
closest available stack known to the reference estate (a collaborative-SNARK toolchain over
an existing proving backend) is explicitly #strong[unaudited], and no candidate has been
security-reviewed for this use.

== Verifier-side authenticated-input attestation gate (v1)

As an interim design that avoids the unsolved distributed signature (§10.5), the v1 proposal
moves attestation checking to the verifier: holders' inputs enter the MPC as
#emph[authenticated inputs] (following the authenticated-input MPC pattern of
#cite("EPRINT-2022-1648")), and the verifier checks a commit-and-prove anchor (in the style
of #cite("ARTEMIS")) that ties the MPC's inputs to the issuer-signed commitments.

This v1 design #strong[knowingly gives up], and any implementation of it #strong[MUST]
disclose that it gives up:

+ source-unlinkability — the verifier learns which source contributed which attested input;
+ a single succinct signature-to-witness proof — attestation evidence is per-source, not one
  proof;
+ malicious security against `n - 1` corruptions — the gate's guarantees assume the
  honest-majority tier;
+ in-band freshness — freshness binding needs an explicit out-of-circuit mechanism (§9.4).

== Distributed in-circuit signature verification

The end-state design — verifying issuer signatures #emph[inside] the collaborative circuit
over a secret-shared witness, yielding a single source-unlinkable proof — is
#strong[unsolved in the open literature] as known to the reference estate, and is deferred
(tracked as sq-bjl).
It #strong[MUST NOT] be presented with a feasibility or performance figure, because none
exists.

= Implementation status summary

This section is informative. It summarises, per feature, what the reference estate has
actually merged and tested versus what is design-only, so that no reader mistakes this
draft's proposals for capabilities.

#table(
  columns: 2,
  align: (left, left),
  table.header[Implemented and tested (M0–M3, merged)][Designed, not built],
  [Honest-majority Shamir backend over `F_p = 2^61 - 1`; BGW degree reduction; CSPRNG
   masking], [Collaborative ZK proof over a secret-shared witness (`prove`/`verify` are
   honest not-yet-implemented stubs; proof system unchosen)],
  [Disclosed-key and hidden-value joins; verdict-bit-only secure comparison; zero-round
   secure aggregation with threshold verdicts], [Distributed issuer-signature attestation
   over a secret-shared witness (placeholder types; unsolved in the literature)],
  [Oblivious shuffle/sort, hidden-key DISTINCT, bounded property paths with planner guard],
   [Verifier-side authenticated-input attestation gate (v1) and the federated public-input
   layout it needs],
  [Robust Reed–Solomon reconstruction; IT-MAC authenticated sharing with MAC-checked
   disclosure (partial malicious tier)], [Full honest-majority malicious-with-abort suite;
   dishonest-majority backend; constant-round WAN family],
  [Domain-separated term→field join-key encoder with fail-closed collision detection],
   [In-circuit encoding-correctness proof for hidden join keys],
  [Untrusted-planner seam: default-deny source selection, routing, leakage-envelope dual
   ratification], [Normative leakage vocabulary and differential-privacy epsilon-budget
   format],
  [Single-prover verification layer: public-input reconstruction, external-anchor
   attestation binding, single-use burn-on-mismatch nonce], [Deployed party-mesh transport;
   verifier–federation handshake formats; multi-source revocation policy],
  [End-to-end federated pipeline driver with differential correctness anchor; simulation-tier
   wire protocol; deterministic communication/round counters], [Canonical query
   normalisation and digest],
)

= Security and Privacy Considerations

This section is normative for every conformance class.

#conf("every conformance class (deployment duties)", "testable-now")

== Audit status: nothing here is production-ready

#strong[No soundness, attestation, or privacy property described in this document is
production-claimable today.] All existing security review of the reference estate consists
of internal, single-model self-audits; the estate's own security policy correctly instructs
users to treat the implementation as untrusted.

Two #emph[distinct] external audit scopes gate this document's claims, and they must not be
conflated:

- The planned external audit by accredited cryptographers tracked in the reference estate as
  sq-qhy4 (open at the date of this draft) is scoped to the #emph[single-prover ZK layer] —
  the ZK verifier and its circuits. When it completes, it can clear the single-prover claims
  of §9, and #strong[only] those.
- The MPC evaluation layer (§4, §7, §8) and every §10 proposal require #emph[additional,
  separately scoped] external review — of the MPC protocol suite, of the composition
  boundary of §3.4, and of any collaborative-proof toolchain eventually chosen (§12.4) — for
  which no audit is scoped or scheduled at the date of this draft.

Implementations and deployments #strong[MUST NOT] represent MPC-SPARQL as providing audited
or production-grade guarantees until the audit covering the #emph[relevant layer] completes,
and #strong[MUST] surface this status to relying parties.

== Default tier is semi-honest

The default operating tier of the reference implementation is #strong[honest-majority
semi-honest]. The authenticated (IT-MAC) malicious-with-abort tier is only partially built
(§7.5). A deployment #strong[MUST] report the actual per-operator tier (§3.3) and #strong[MUST
NOT] describe a semi-honest run as maliciously secure. Semi-honest security assumes every  // privacy-claims-allow: this is the honesty caveat itself, warning against the overclaim
party follows the protocol; it provides #emph[no] guarantees against a participant who
deviates.

== What v1 attestation gives up

The verifier-side attestation design of §10.4, if implemented, gives up source-unlinkability,
a single succinct signature-to-witness proof, and malicious security against `n - 1`
corruptions, and requires explicit out-of-circuit freshness binding. Every deployment of it
#strong[MUST] state these four concessions to its relying parties (§10.4).

== Collaborative-proof risks

Collaborative zk-SNARK proving over invalid witnesses can leak honest provers' inputs
(#cite("EPRINT-2025-1026")); §10.2 therefore makes pre-proof witness validation a
#strong[MUST]. The collaborative path is #strong[not covered] by the single-prover audit
scope of §12.1 (sq-qhy4) nor by any existing internal review, and candidate
collaborative-proof toolchains are unaudited.

== Statistical encodings

The join-key encoding of §4.2 carries a birthday-bound collision probability
(approximately `q^2 / 2^62` for q keys); it is a statistical bound, not a cryptographic
hardness guarantee. Deployments #strong[MUST] treat encoder collisions as a live failure
mode (fail closed on detection) and #strong[MUST] size runs against the target
statistical-security parameter and the run-size caps of §4.1 (`s = 40` admits only about
`2^11` hidden join-key terms), disclosing any weakened bound per §4.1.

== Leakage beyond the result

Even a correct run reveals more than the disclosed result: operator routing, result
cardinalities and timing on the disclosed side, and whatever the declared leakage envelope
admits. The leakage envelope #strong[MUST] be declared and ratified before execution (§6.1).
A normative leakage vocabulary and a differential-privacy budget format for output
cardinalities are #strong[not yet specified] (tracked as sq-shk5); until they are, deployments #strong[MUST]
document their leakage envelope contents ad hoc and conservatively.

== Unachieved regimes fail closed

Dishonest-majority malicious correctness for SPARQL operators over wide-area networks is not
an achieved capability in the reference estate or, to its knowledge, in the open literature
with usable performance evidence. A conformant implementation asked for an unsupported
security regime #strong[MUST] refuse to run (fail closed) rather than silently downgrade
(§3.2, §3.3).

== The simulation transport is not a deployment

The star-coordinator wire protocol of §8.1 is a simulation harness. Deploying it as-is would
concentrate traffic metadata (who exchanges shares, when, how much) at the coordinator and
create a single denial-of-service point. The deployed transport (§8.2) must be specified and
reviewed before any real-network deployment.

== No performance claims

This document intentionally contains no performance figures. The only quotable cost metrics
for the reference estate are deterministic counters (circuit gate counts, MPC round and byte
counters); wall-clock measurements from development machines are non-canonical and
#strong[MUST NOT] be cited as protocol characteristics.

= References

== Normative references

#references((
  ("RFC2119", [Bradner, S. #emph[Key words for use in RFCs to Indicate Requirement Levels].
    RFC 2119, IETF, March 1997.]),
  ("RFC8174", [Leiba, B. #emph[Ambiguity of Uppercase vs Lowercase in RFC 2119 Key Words].
    RFC 8174, IETF, May 2017.]),
  ("SPARQL11-FED", [Prud'hommeaux, E., Buil-Aranda, C. #emph[SPARQL 1.1 Federated Query].
    W3C Recommendation, 21 March 2013.]),
  ("SPARQL11-QUERY", [Harris, S., Seaborne, A. #emph[SPARQL 1.1 Query Language]. W3C
    Recommendation, 21 March 2013. (Normative: the post-processing rule of §6.3 depends on
    this recommendation's operator and aggregate definitions.)]),
  ("FIPS-180-4", [NIST. #emph[Secure Hash Standard (SHS)]. FIPS PUB 180-4, August 2015.]),
  ("RFC8439", [Nir, Y., Langley, A. #emph[ChaCha20 and Poly1305 for IETF Protocols].
    RFC 8439, IETF, June 2018.]),
  ("RDF-CANON", [Longley, D., et al. #emph[RDF Dataset Canonicalization]. W3C
    Recommendation, 21 May 2024.]),
))

== Informative references

#references((
  ("SHAMIR79", [Shamir, A. #emph[How to Share a Secret]. Communications of the ACM 22(11),
    1979.]),
  ("BGW88", [Ben-Or, M., Goldwasser, S., Wigderson, A. #emph[Completeness Theorems for
    Non-Cryptographic Fault-Tolerant Distributed Computation]. Proceedings of the 20th ACM
    Symposium on Theory of Computing (STOC), 1988.]),
  ("CLEVE86", [Cleve, R. #emph[Limits on the Security of Coin Flips when Half the Processors
    are Faulty]. Proceedings of the 18th ACM Symposium on Theory of Computing (STOC),
    1986.]),
  ("RABBIT", [Makri, E., Rotaru, D., Vercauteren, F., Wood, T. #emph[Rabbit: Efficient
    Comparison for Secure Multi-Party Computation]. Financial Cryptography and Data Security
    2021.]),
  ("BATCHER68", [Batcher, K. E. #emph[Sorting Networks and their Applications]. AFIPS Spring
    Joint Computer Conference, 1968.]),
  ("WAKSMAN68", [Waksman, A. #emph[A Permutation Network]. Journal of the ACM 15(1), 1968.]),
  ("BW86", [Welch, L. R., Berlekamp, E. R. #emph[Error Correction for Algebraic Block
    Codes]. U.S. Patent 4,633,470, filed 1983, granted 30 December 1986. (The
    Berlekamp–Welch decoder; no journal version exists — the patent is the primary
    source.)]),
  ("EPRINT-2022-1648", [Dutta, M., Ganesh, C., Patranabis, S., Singh, N. #emph[Compute, but
    Verify: Efficient Multiparty Computation over Authenticated Inputs]. IACR Cryptology
    ePrint Archive, Report 2022/1648, 2022.]),
  ("ARTEMIS", [Lycklama, H., Viand, A., Avramov, N., Küchler, N., Hithnawi, A.
    #emph[Artemis: Efficient Commit-and-Prove SNARKs for zkML]. arXiv:2409.12055, 2024.]),
  ("EPRINT-2025-1026", [Garg, S., Goel, A., Jain, A., Roberts, B., Sekar, S.
    #emph[Malicious Security in Collaborative zk-SNARKs: More than Meets the Eye]. IACR
    Cryptology ePrint Archive, Report 2025/1026, 2025.]),
  ("OB22", [Ozdemir, A., Boneh, D. #emph[Experimenting with Collaborative zk-SNARKs:
    Zero-Knowledge Proofs for Distributed Secrets]. 31st USENIX Security Symposium, 2022.]),
  ("SMCQL", [Bater, J., Elliott, G., Eggen, C., Goel, S., Kho, A., Rogers, J. #emph[SMCQL:
    Secure Querying for Federated Databases]. Proceedings of the VLDB Endowment 10(6),
    2017.]),
  ("CONCLAVE", [Volgushev, N., Schwarzkopf, M., Getchell, B., Varia, M., Lapets, A.,
    Bestavros, A. #emph[Conclave: Secure Multi-Party Computation on Big Data]. Proceedings
    of the 14th EuroSys Conference, 2019.]),
  ("SENATE", [Poddar, R., Kalra, S., Yanai, A., Deng, R., Popa, R. A., Hellerstein, J. M.
    #emph[Senate: A Maliciously-Secure MPC Platform for Collaborative Analytics]. 30th  // privacy-claims-allow: prior-work reference title (Senate), not a sparq claim
    USENIX Security Symposium, 2021.]),
  ("CEREBRO", [Zheng, W., Deng, R., Chen, W., Popa, R. A., Panda, A., Stoica, I.
    #emph[Cerebro: A Platform for Multi-Party Cryptographic Collaborative Learning]. 30th
    USENIX Security Symposium, 2021.]),
  ("PSTY19", [Pinkas, B., Schneider, T., Tkachenko, O., Yanai, A. #emph[Efficient
    Circuit-Based PSI with Linear Communication]. EUROCRYPT 2019.]),
  ("SAFE", [Khan, Y., Saleem, M., Mehdi, M., Hogan, A., Mehmood, Q., Rebholz-Schuhmann, D.,
    Sahay, R. #emph[SAFE: SPARQL Federation over RDF Data Cubes with Access Control].
    Journal of Biomedical Semantics 8:5, 2017.]),
  ("HDTCRYPT", [Fernández, J. D., Kirrane, S., Polleres, A., Steyskal, S. #emph[HDT-crypt:
    Compression and Encryption of RDF Datasets]. Semantic Web 11(2), 2020.]),
  ("KIRRANE17", [Kirrane, S., Mileo, A., Decker, S. #emph[Access Control and the Resource
    Description Framework: A Survey]. Semantic Web 8, 2017.]),
))
