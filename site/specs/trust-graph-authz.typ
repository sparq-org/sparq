// [FABLE-5] sq-q4jdu — Trust-Graph Authorisation for Solid/LWS: the pod-side admission
// Unofficial Proposal Draft. 🤖 SPARQ agent — written by Claude Fable 5.
//
// SCOPE BOUNDARY (research/trust-graph-authorisation-2026-07.md section 5): the companion
// Trust Expression draft (program sq-6syab) owns the VERIFIER <-> HOLDER contract — the
// trust-requirements document a verifier sends and the holder-side evaluation. THIS draft
// owns the POD-SIDE ADMISSION consumption of the same trustx: vocabulary: how a pod derives
// effective trust rules from signed certification edges ahead of its admission gate.
//
// SOURCE OF TRUTH: the normative algorithm transcribes the SHIPPED closure in
// crates/sparq-trust/src/graph.rs (bead sq-pfae.15 + its review fixes), the store
// composition (store.rs, sq-pfae.16), and the stateless decision-endpoint binding
// (crates/sparq-server/src/solid_authz.rs, sq-pfae.17). Where the design record
// (research/trust-graph-authorisation-2026-07.md section 2.2) sketches behaviour the
// shipped code does NOT implement (per-edge status-list attestation inside the closure),
// this draft says so plainly rather than specifying vapour — see section 6.
//
// HONESTY (the privacy-claims + no-perf-numbers gates scan this source at build time):
// this layer is CLEAR-PATH authorisation only. It makes no ZK, privacy, or unlinkability
// claim; signature verification rides the sparq-zk estate, which is internally re-audited
// but has NO external accredited-cryptographer sign-off (open external-audit gate sq-qhy4);
// sparq-mpc is honest-majority semi-honest only. No performance numbers appear here.

#import "_lib/spec.typ": spec-head, sotd, intro-section, references, dfn, note, cite

#set document(title: "Trust-Graph Authorisation for Solid/LWS: Pod-Side Admission of Certified Issuers")
#set text(size: 11pt)
#set par(justify: true)
#set heading(numbering: "1.")

#spec-head()

#intro-section("abstract", "Abstract")[
  This document proposes a pod-side admission model for Solid / Linked Web Storage (LWS)
  servers in which authorisation-relevant statements are admitted not only from issuers a
  pod enumerates directly, but also from issuers a locally anchored authority — such as an
  eIDAS-style trusted list or a trust-framework register — has #dfn[certified], via signed,
  time-windowed, scope-attenuating #dfn[certification edges]. It defines a normative,
  fail-closed derivation algorithm — a depth-bounded closure from Control-gated anchor
  rules over verified certification edges to an effective trust-rule set — together with
  the five invariants that make the derivation safe (attenuation-only, no ambient edges,
  meta-scope non-escalation, deny-by-absence on cycles and depth, and strict additivity),
  and a stateless HTTP binding that extends a WAC/ACP decision endpoint with a trust block.
  The model consumes the same `trustx:` certification vocabulary that the companion Trust
  Expression draft uses for the verifier–holder contract; this document specifies only the
  pod-side (relying-party) admission surface.
]

#sotd()

#intro-section("security-standing", "Security standing of this proposal")[
  This proposal specifies a #emph[clear-path] authorisation mechanism: it bottoms out in
  operator-anchored keys and signed attestations. It makes #strong[no ZK, privacy, or
  unlinkability claim] — no anonymity, no hidden-issuer admission, and no zero-knowledge
  soundness property is claimed anywhere in this document. Signature verification in the
  reference implementation rides the sparq-zk signature primitives, which are internally
  re-audited but have #strong[not] been externally audited by an accredited cryptographer:
  the external-audit gate (tracked in-repo as *sq-qhy4*) is #strong[open and pending].
  Until that gate closes, every cryptographic property this document depends on is a
  documented claim, not an audited guarantee. The related sparq-mpc estate is
  honest-majority #emph[semi-honest] only. The reference implementation is a research
  prototype behind default-OFF opt-in features.
]

= Introduction

Access control on a Solid / LWS pod is conventionally expressed per resource: an access
control resource enumerates which agents may read or write. A growing class of policies,
however, is #emph[attribute-driven]: "grant access to any agent whose age attestation a
source I trust has issued". Such policies need an #dfn[admission] stage ahead of the
WAC/ACP decision — a gate that decides which #emph[statements] (claims by issuers about
agents) the pod accepts as authorisation-relevant facts.

An admission gate over a flat list of directly enumerated issuers already exists in the
reference implementation: a pod's Control-gated trust policy lists trusted issuer keys,
per statement type, per resource scope, with a freshness bound. What a flat list cannot
express is the trusted-list relation used by real identity frameworks: #emph[the pod does
not know the issuer; it knows an authority that certifies issuers]. Under eIDAS 2.0, a
relying party trusts a national trusted list, and the list certifies qualified providers.
Under the UK DIATF, a scheme operator certifies identity-service providers. In both cases
trust is a #emph[graph]: a locally anchored authority vouches, in a signed and
time-windowed record, for issuers the relying party never enumerated.

This document specifies that graph step for pods: a normative, fail-closed derivation from
local #dfn[anchor rules] plus signed certification edges to an #dfn[effective trust-rule
set], which an unchanged flat-rule admission gate then consumes. The derivation is
deliberately small: version 1 is a depth-1 closure whose every step can only #emph[narrow]
the authority the anchor already holds.

== Relationship to the Trust Expression draft

The `trustx:` certification vocabulary this document consumes is shared with a companion
proposal, the Trust Expression draft (the sparq `sq-6syab` program), which specifies the
#emph[verifier–holder] contract: how a verifier states its trust requirements and how a
holder evaluates which of its credentials satisfy them. The two documents deliberately
partition the surface:

#table(
  columns: 2,
  align: (left, left),
  table.header[Surface][Owned by],
  [Verifier → holder: expressing trust requirements; holder-side evaluation],
  [Trust Expression draft (`sq-6syab`)],
  [Pod-side admission: deriving effective trust rules from certification edges; the
   decision-endpoint binding],
  [This document],
  [The `trustx:` vocabulary terms themselves (byte-pinned ontology)],
  [Shared; minted by the Trust Expression program and consumed here unchanged],
)

This document mints #strong[no] new vocabulary terms.

== What this document does not specify

- The flat-rule admission gate itself (issuer-signature verification, shape matching,
  scope and freshness checks over directly enumerated rules). This document treats it as
  an opaque consumer of `TrustRule` sets and constrains only what is fed into it.
- The WAC/ACP decision algorithm. Admission runs #emph[ahead of] it and never modifies it.
- Multi-hop (depth greater than 1) certification chains, threshold corroboration, and any
  zero-knowledge or hidden-issuer admission mode. These are out of scope for version 1
  (section 9).

= Terminology and conformance

The key words #strong[MUST], #strong[MUST NOT], #strong[REQUIRED], #strong[SHALL],
#strong[SHALL NOT], #strong[SHOULD], #strong[SHOULD NOT], #strong[RECOMMENDED],
#strong[MAY], and #strong[OPTIONAL] in this document are to be interpreted as described in
#cite("RFC2119") and #cite("RFC8174") when, and only when, they appear in all capitals, as
shown here.

An #dfn[anchor rule] is a trust rule (`trust:TrustRule`) parsed from a pod's Control-gated
trust policy whose trusted source is a certifying authority (asserted via
`trustx:trustsFramework`), carrying the authority's verification key, a statement-type
ceiling (`trust:forShape` / `trust:forPredicate`), a resource scope (`trust:scope`), and a
freshness bound (`trust:freshWithin`).

A #dfn[certification edge] is a signed `trustx:Certification`: an attestation by a
#dfn[certifier] (the authority) that a #dfn[certified issuer] may act as a source of
statements within a stated scope and validity window.

The #dfn[effective trust-rule set] is the output of the derivation in section 4: the
direct (anchor and non-anchor) rules plus every rule derived from a surviving
certification edge.

This document defines conformance for four artefact classes:

#table(
  columns: 2,
  align: (left, left),
  table.header[Conformance class][Artefact],
  [#emph[Admission engine]], [An implementation of the derivation algorithm (section 4)
    and its invariants (section 5).],
  [#emph[Trust policy document]], [A Control-gated document carrying anchor rules and,
    optionally, certification edges (sections 3.1, 6).],
  [#emph[Certification bundle]], [A set of signed certification edges presented for
    derivation (section 3.2).],
  [#emph[Decision endpoint]], [An HTTP WAC/ACP decision service implementing the trust
    block binding (section 7).],
)

= Data model

== Anchor rules

A pod opts into graph-derived admission by authoring, in its Control-gated trust policy,
an anchor rule whose source is a certifying authority:

```turtle
@prefix trust:  <https://sparq.dev/ns/trust#> .
@prefix trustx: <https://sparq.dev/ns/trust#> .

# Pod policy — Control-gated channel; ungated input MUST be rejected at parse.
[] a trust:TrustRule ;
   trustx:trustsFramework <https://eidas.example/trusted-list> ;
   trust:issuerKey        <did:web:eidas.example#tl-key> ;
   trust:forShape         [ <shape the derived rules may never exceed> ] ;
   trust:scope            <https://pod.example/private/> ;
   trust:freshWithin      "P30D"^^xsd:duration .
```

A trust policy document #strong[MUST] be delivered over a channel gated on Control-level
access to the governed resource, and a parser #strong[MUST] reject a policy that does not
arrive through that gate. The anchor rule's `trust:forShape` is the #dfn[ceiling]: no rule
derived from this anchor may admit any statement outside it. The anchor's key is the key
the authority's certification signatures are verified against.

== Certification edges

A certification edge attests, under the certifier's signature, that an issuer is certified
— under a framework, over a scope, within a validity window:

```turtle
[] a trustx:Certification ;
   trustx:certifies      <https://gov.example/issuer> ;
   trustx:underFramework <https://eidas.example/trusted-list> ;
   trustx:certificationScope trustx:AnyServiceScope ;
   trustx:validFrom      "2026-01-01T00:00:00Z"^^xsd:dateTime ;
   trustx:validUntil     "2027-01-01T00:00:00Z"^^xsd:dateTime .
```

An edge carries, and its signature #strong[MUST] bind, all of the following (section 3.3):

#table(
  columns: 3,
  align: (left, left, left),
  table.header[Field][Vocabulary][Meaning],
  [Certifier], [`trustx:underFramework`], [The certifying authority. To confer anything it
    MUST match the source of an anchor rule, under the anchor's key.],
  [Certified issuer], [`trustx:certifies`], [The issuer being vouched for. The derived
    rule authorises this issuer.],
  [Certified issuer's key], [—], [The verification key the derived rule binds. Signed into
    the edge so a captured edge cannot be re-presented under a substituted key.],
  [Scope], [`trustx:certificationScope`], [What the issuer is certified to issue: either
    `trustx:AnyServiceScope` (service-level: no statement-type narrowing of its own) or a
    statement-type shape.],
  [Validity window], [`trustx:validFrom` / `trustx:validUntil`], [Inclusive time window.
    An inverted window is ill-formed.],
  [Signature], [—], [The certifier's signature over the message of section 3.3. An absent
    or unverifiable signature makes the edge inert.],
)

An edge is #strong[never] blanket trust and confers nothing by itself: a graph merely
#emph[claiming] `trustx:certifies` proves nothing until the closure has verified the
signature against a locally anchored key (section 4.2).

== The signed certification message

The certifier signs a domain-separated message binding the #emph[whole] edge: the
certifier identity, the certified issuer identity #strong[and its verification key], the
scope, and the validity window. A conforming admission engine #strong[MUST] verify the
edge signature over a message that binds all of these fields, and #strong[MUST] use a
domain tag distinct from every other signature domain in the system (in the reference
implementation, the tag `TRUSTCRT`, distinct from the delegation-hop, credential-
commitment, and proof-of-possession domains), so a certification signature can never be
confused with any other signed object.

Binding every field is load-bearing, not cosmetic:

- Binding the certified issuer's #emph[key] means an attacker cannot lift a genuine
  certification and substitute its own key as the certified issuer's — the tampered
  message no longer verifies.
- Binding the scope and window means a narrow certification cannot be re-presented with a
  broader scope or a longer window — the narrowing is #emph[attested by the certifier],
  never asserted by the presenter.

= The admission derivation algorithm

== Effective-rule derivation

The derivation is a pure pre-processing step: it produces the flat rule set the unchanged
admission gate consumes, and changes nothing downstream.

```text
derive_effective_rules(direct_rules, certifications, now, depth_bound) -> rules
  rules := copy of direct_rules, verbatim, in order      # strict additivity
  if depth_bound = 0 or certifications is empty:
      return rules                                       # byte-identical no-op
  # version 1: depth 1 — derived rules are NEVER re-used as anchors
  for each cert in certifications:
      if admit_edge(direct_rules, cert, now) yields a rule r:
          append r to rules
  return rules
```

A conforming admission engine:

- #strong[MUST] emit the direct rules verbatim, in their input order, as a prefix of the
  output (section 5, strict additivity);
- #strong[MUST] treat a depth bound of zero, or an empty certification set, as the
  identity function on the direct rules;
- #strong[MUST NOT], at depth bound 1, use a derived rule as an anchor for a further
  derivation round (a certifier MUST be a direct anchor);
- #strong[MAY] support depth bounds greater than 1 only if it implements the meta-scope
  non-escalation invariant (section 5) for every additional hop; this document defines no
  such profile.

== Edge admission gates

Each candidate edge passes through the following gates, in order. The #strong[first]
failing gate rejects the edge; a rejected edge contributes #strong[nothing] to the
effective rule set — rejection is deny-by-absence, never an error path that grants.

#table(
  columns: 3,
  align: (left, left, left),
  table.header[Gate][Requirement][Rejection],
  [1 — Cycle], [The edge MUST NOT be self-certifying (certifier equals certified issuer),
    and the certified issuer (under the certified key) MUST NOT already be a direct anchor
    — either shape is a cycle that could launder a broadening and can add no authority the
    graph does not already hold.], [`Cyclic`],
  [2 — Anchor], [The certifier MUST match the source of at least one direct anchor rule,
    and the edge's certifier key MUST equal that anchor's key. An edge with no local
    anchor is ambient and MUST be ignored.], [`NoAnchor`],
  [3 — Signature], [The edge signature MUST verify, under the anchored certifier key, over
    the full bound message of section 3.3. Self-asserted or forged edges MUST be
    rejected.], [`SignatureInvalid`],
  [4 — Time], [The window MUST be well-formed (until #sym.gt.eq from) and MUST cover the
    evaluation instant, inclusively. Expired, not-yet-valid, and inverted windows MUST be
    rejected.], [`OutOfWindow`],
  [5 — Scope attenuation], [The derived statement-type MUST be provably contained in a
    matching anchor's ceiling (section 4.3). Where containment cannot be proven, the
    engine MUST treat it as not contained.], [`Broadening`],
  [6 — Freshness attenuation], [The derived freshness bound is the minimum of the anchor's
    freshness bound and the remaining validity window; it can only shrink (this gate
    computes, it never rejects).], [—],
)

A surviving edge yields exactly one derived rule:

#table(
  columns: 2,
  align: (left, left),
  table.header[Derived-rule field][Value],
  [Source (who)], [The certified issuer, bound to the certified key from the signed
    edge.],
  [Statement type (what)], [The attenuated shape of gate 5 — the anchor ceiling for
    `trustx:AnyServiceScope`, or the provably narrower certification shape.],
  [Resource scope (where)], [The anchor's resource scope, #strong[unchanged]. A
    certification narrows who and what type; it MUST NOT alter where.],
  [Freshness], [min(anchor freshness, remaining window), never negative.],
)

An engine #strong[SHOULD] additionally expose a per-edge diagnostic path that reports
which gate rejected an edge (the rejection labels above). If it does, the diagnostic path
#strong[MUST] route through the same gate implementation as the fast path, so the two can
never diverge on what they admit.

== Scope attenuation: shape narrowing

Deciding that a certification's statement-type shape narrows the anchor ceiling is the
subtle gate, because SHACL-style node shapes contain two kinds of statement with
#emph[opposite] variance:

- #dfn[Selection statements] (the `sh:target*` family — `sh:targetNode`,
  `sh:targetClass`, `sh:targetSubjectsOf`, `sh:targetObjectsOf`, `sh:targetWhere` — and
  implicit-class typing) pick focus nodes by the #strong[union] of targets. Adding a
  selection statement #strong[widens] the admitted set.
- #dfn[Conformance constraints] (`sh:property`, `sh:minCount`, `sh:datatype`, and
  similar) must all be satisfied by a selected node. Adding a conformance constraint
  #strong[narrows] the admitted set.

A conforming engine therefore #strong[MUST] decide narrowing contravariantly on selection
and covariantly on conformance. A certification shape narrows an anchor shape if, and only
if, both of the following are proven:

+ #strong[Selection may only shrink.] Every selection statement the certification shape
  declares at its root is also declared at the anchor's root. An extra selection statement
  the anchor lacks is a broadening and MUST reject the edge. Any selection predicate the
  engine does not model (including any unrecognised member of the `sh:target*` family)
  MUST be treated as selection and MUST fail closed — it MUST never be silently treated as
  a conformance constraint.
+ #strong[Conformance may only grow.] The certification shape's conformance constraints
  structurally contain the anchor's: every anchor constraint is matched by a
  #strong[distinct] certification constraint, with blank-node constraint objects matched
  recursively by structural position under an #strong[injective] blank-node assignment,
  and ground objects matched exactly. Injectivity is the defence against reporting a shape
  with fewer constraints as containing one with more.

Treating "more statements, therefore narrower" uniformly is #strong[unsound]: an extra
`sh:targetSubjectsOf` statement is a broadening, not a narrowing — an engine that admits
it grants the certified issuer authority over a statement type the certifier never held,
which is a privilege escalation.

Where containment cannot be decided by the engine's (deliberately conservative, bounded)
matcher, the engine #strong[MUST] treat the shape as not contained and reject the edge:
undecidable containment is not contained. A false rejection merely fails to admit; a false
acceptance escalates.

The scope `trustx:AnyServiceScope` imposes no statement-type narrowing of its own: the
derived shape is the anchor ceiling, unchanged. It can never broaden, because the anchor
remains the ceiling.

== Multiple anchors for one certifier

A certifier may be anchored more than once over the same key (for instance, once per
statement type). An engine #strong[MUST] consider every matching anchor at gate 5 and
admit the edge through any one anchor whose ceiling the certification provably narrows.
The chosen anchor is genuine pod-granted authority, so this can never widen; requiring the
first-listed anchor to match would spuriously reject valid narrowings.

= Invariants

The following five properties are the load-bearing safety contract of this proposal. A
conforming admission engine #strong[MUST] maintain all of them; each is independently
testable.

+ #strong[Attenuation only.] Every derived rule's statement type, resource scope, and
  freshness are contained in the matching anchor's; an edge can only narrow, never widen,
  what its anchor already grants. Where narrowing cannot be proven, the edge contributes
  nothing.
+ #strong[No ambient edges.] Only certification edges anchored by a local, Control-gated
  anchor rule confer anything. An attacker-supplied certification with no matching anchor
  is inert — exactly as inert as an untrusted credential.
+ #strong[Meta-scope non-escalation.] An issuer certified #emph[for attributes] cannot
  thereby certify other issuers: acting as an edge source at depth k+1 would require the
  entity's own incoming certification scope to cover the issuing of certifications
  themselves, which an attribute scope never does. At depth 1 this is structural (only
  direct anchors are edge sources); it is stated normatively so a future depth extension
  cannot silently drop it.
+ #strong[Cycles and depth deny by absence.] A self-certifying edge, a cycle back into the
  anchor set, an over-depth path, or a zero depth bound each contribute nothing. None of
  them may cause divergence or an error path that grants.
+ #strong[Strict additivity, twice.] First: with zero certification edges (or none
  surviving the gates), the effective rule set is the direct rule set, byte-identical and
  in order — the graph mode degrades exactly to the flat mode. Second: with the capability
  disabled (it MUST be an opt-in, default-OFF feature), no derivation code is active and
  the host's behaviour is byte-identical to a build without it.

= Revocation, caching, and status

Certification edges enter the derivation as part of the Control-gated trust policy
document (section 3.1) or as request-supplied input to a stateless endpoint (section 7).
For the document-carried case:

- Revoking an edge is expressed by #strong[re-authoring the document without it] (or with
  a shortened window). A conforming trust store #strong[MUST] fold the certification set —
  at minimum each edge's certifier, certified issuer, and validity window — into its
  policy version, so removing or re-windowing an edge changes the version and
  #strong[MUST] invalidate every cached admission verdict computed under the old version.
- Window enforcement (gate 4) happens at every derivation, so an expired edge stops
  conferring authority at evaluation time even though wall-clock expiry alone (with no
  document edit) does not change the policy version.
- #strong[Honest residue:] where admitted facts have been materialised, revocation
  propagates by re-materialisation. The stale-authority window is #emph[bounded] by the
  derived freshness (which the closure already caps at the remaining validity window) —
  it is not closed. Deployments for which bounded staleness is unacceptable MUST
  re-derive on every decision rather than cache.

#note[
  The design record for this layer sketches an additional per-edge status-attestation
  check inside the closure (a `trustx:StatusAttestation` covering each edge, with
  missing-or-stale attestations failing closed, mirroring how credentials are
  status-checked). The reference implementation does #strong[not] implement that check in
  the shipped closure: edge revocation is carried by document re-authoring as specified
  above. In-band status attestation for certification edges is an extension point, not a
  version-1 requirement; a future revision may make it normative. This document specifies
  what is built.
]

= HTTP decision-endpoint binding

A WAC/ACP decision endpoint (a stateless `POST /authz/decide` that evaluates an access
query over a request-supplied dataset) #strong[MAY] be extended with a #dfn[trust block]:
an optional JSON member `"trust"` carrying the full trust input in the request.

The binding #strong[MUST] be double-opt-in: the extension activates only when (a) the
server was built with the capability enabled #strong[and] (b) the deployment configuration
enables it #strong[and] (c) the request actually carries a `"trust"` member. A request
without a trust block #strong[MUST] fall through to the unextended decision path,
byte-identically.

== The trust block

```json
{
  "trust": {
    "rules": [
      { "source": "https://eidas.example/trusted-list",
        "issuerKeyHex": "…",
        "scopeIri": "https://pod.example/private/",
        "freshWithinSecs": 2592000,
        "shapePredicateIri": "https://schema.org/birthDate" }
    ],
    "certifications": [
      { "certifierIri": "https://eidas.example/trusted-list",
        "certifierKeyHex": "…",
        "certifiedIssuerIri": "https://gov.example/issuer",
        "certifiedKeyHex": "…",
        "validFromUnixSecs": 1767225600,
        "validUntilUnixSecs": 1798761600,
        "signatureHex": "…",
        "scopeKind": "anyService" }
    ],
    "credentials": [
      { "graphNquads": "…",
        "issuerSignatureHex": "…",
        "saltHex": "…",
        "issuedAtUnixSecs": 1767226000,
        "revoked": false }
    ]
  }
}
```

The endpoint derives the effective rule set from `rules` and `certifications` exactly per
section 4, admits the presented `credentials` through the unchanged admission gate over
those rules, and then runs the unchanged WAC/ACP decision over the union of the dataset
and the admitted facts.

Normative constraints:

- #strong[Fail closed, always.] Any missing or mistyped field, unparsable graph,
  unverifiable signature, unknown mode, or unknown enumeration value #strong[MUST] produce
  a deny decision (HTTP 403 with the deny body) — never a grant, and never an error
  response that a client could mistake for a grant.
- #strong[Unknown scope kinds deny.] Version 1 of this wire format carries only
  `"scopeKind": "anyService"`. A certification whose `scopeKind` is absent or unrecognised
  #strong[MUST] be rejected (fail-closed), never treated as unscoped. Statement-type-
  scoped certification shapes are expressible in the trust-policy document surface
  (section 3.2) but have no version-1 wire encoding here.
- #strong[Narrowing only.] The trust path can only add admitted facts for the decision to
  consider under its existing algebra; it #strong[MUST NOT] modify the decision algorithm
  itself.
- #strong[Justification.] A decision reached through the trust path #strong[SHOULD]
  include a minimal admission justification in the response — at minimum: the IRI of the
  rule that sourced the deciding admitted fact, the count of admitted facts, and whether
  any derived (certification-sourced) rule contributed — so a relying party can see
  #emph[why] a graph-driven decision held.

== Interaction with ODRL policies

A dataset may carry ODRL policies whose evaluation contributes deny decisions. In version
1, an endpoint that receives #strong[both] a trust block #strong[and] a dataset carrying
ODRL policies #strong[MUST] refuse the combination with a definitive deny rather than
compose the two: the composed law (an ODRL prohibition overrides a trust-admitted allow
for the same principal, mode, and target) is the design intent, but until an
implementation pins it with conformance tests, silently composing the two risks an
untested override path. A future revision is expected to replace this refusal with the
verified composition law.

= Security considerations

#note[
  #strong[Normative honesty note.] This layer is clear-path authorisation only. It makes
  #strong[no ZK, privacy, or unlinkability claim]: certification edges, presented
  credentials, and admission justifications are all visible to the pod in the clear, and
  nothing in this document provides anonymity, unlinkability, hidden-issuer admission, or
  any zero-knowledge property. Signature verification in the reference implementation
  rides the sparq-zk estate — internally re-audited, with #strong[no external
  accredited-cryptographer sign-off]; the external-audit gate #strong[sq-qhy4] is open and
  pending, and until it closes no production-grade cryptographic-soundness guarantee is
  claimed for any primitive this document depends on. The neighbouring MPC estate is
  honest-majority semi-honest only. Deployments MUST NOT represent this mechanism as
  providing audited cryptographic guarantees.
]

Threats this design addresses, given honest anchor keys:

- #strong[Forged or replayed edges.] Gate 3 rejects any edge whose signature does not
  verify under a locally anchored certifier key over the fully bound message; key
  substitution, scope inflation, and window extension all break the signature
  (section 3.3).
- #strong[Broadening through shapes.] Gate 5's variance-aware containment (section 4.3)
  rejects the selection-statement escalation that a naive "superset means narrower" check
  admits.
- #strong[Ambient authority.] Invariant 2 makes unanchored certifications inert; the
  Control gate keeps the anchor set under the pod operator's exclusive control.
- #strong[Cycle laundering and self-certification.] Gate 1 and invariant 4.

Residual risks, stated plainly:

- #strong[Anchor-key trust.] The certifier's own key is operator- or DID-asserted. This
  layer defeats broadening and forged-edge escalation #emph[given] honest anchor keys; it
  does not close the upstream key-establishment gap (a compromised or mis-pinned anchor
  key confers everything that anchor could confer).
- #strong[Bounded stale authority.] Section 6: revocation after materialisation
  propagates by re-materialisation, bounded by the derived freshness — bounded, not
  closed.
- #strong[Unaudited primitives.] As above: gate sq-qhy4 is open; every signature-scheme
  property is a claim pending external audit.
- #strong[Availability as denial.] Fail-closed gates convert malformed input into deny.
  An attacker able to corrupt a pod's trust document can deny service (never escalate);
  Control-gating is the mitigation.

= Out of scope and future work

- #strong[Multi-hop chains (depth above 1).] The closure and the meta-scope invariant are
  specified so a depth extension cannot silently drop non-escalation, but no depth-above-1
  profile is defined; it requires a certification-authority scope shape and a cost bound
  first.
- #strong[Threshold corroboration] ("admit only if at least k independent certified
  issuers attest"). Monotone and implementable, but not required by any current use case;
  deliberately unspecified.
- #strong[Zero-knowledge or hidden-issuer admission.] Explicitly out of scope and
  hard-gated on the external audit (sq-qhy4). Nothing in this document is a step toward an
  anonymity property.
- #strong[In-band certification status attestation.] Section 6's extension point.
- #strong[The verified trust–ODRL composition law.] Section 7.2's refusal is expected to
  be replaced once the override law is pinned by conformance tests.

= References

#references((
  ("RFC2119", [Bradner, S. #emph[Key words for use in RFCs to Indicate Requirement
    Levels]. RFC 2119, IETF, March 1997.]),
  ("RFC8174", [Leiba, B. #emph[Ambiguity of Uppercase vs Lowercase in RFC 2119 Key
    Words]. RFC 8174, IETF, May 2017.]),
  ("SOLID", [Capadisli, S., et al. #emph[Solid Protocol]. W3C Solid Community Group,
    https://solidproject.org/TR/protocol.]),
  ("WAC", [Capadisli, S. #emph[Web Access Control]. W3C Solid Community Group,
    https://solidproject.org/TR/wac.]),
  ("ACP", [Bosquet, M. #emph[Access Control Policy (ACP)]. W3C Solid Community Group,
    https://solidproject.org/TR/acp.]),
  ("SHACL", [Knublauch, H., Kontokostas, D. #emph[Shapes Constraint Language (SHACL)].
    W3C Recommendation, 20 July 2017.]),
  ("ODRL", [Iannella, R., Villata, S. #emph[ODRL Information Model 2.2]. W3C
    Recommendation, 15 February 2018.]),
  ("EIDAS2", [European Union. #emph[Regulation (EU) 2024/1183 amending Regulation (EU)
    No 910/2014 as regards establishing the European Digital Identity Framework]
    (eIDAS 2.0). Official Journal of the EU, 2024.]),
  ("DIATF", [UK Department for Science, Innovation and Technology. #emph[UK digital
    identity and attributes trust framework]. 2024.]),
  ("VC-DATA-MODEL", [Sporny, M., et al. #emph[Verifiable Credentials Data Model v2.0].
    W3C Recommendation, 15 May 2025.]),
))
