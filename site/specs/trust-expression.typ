// [FABLE-5] sq-6syab.1 — Trust Expression: a verifier–holder contract for framework-anchored
// attestation queries. 🤖 SPARQ agent.
//
// GROUNDING. Authored from the ratified decomposition design record
// research/trust-expression-spec.md (epic sq-6syab, issue #1592), decisions D1–D5, which was
// itself grounded in a verified estate survey (crates/sparq-trust, crates/sparq-zk-compose,
// zk/compose) and a bounded external survey of EUDI ARF 2.4.0 and UK DIATF gamma 0.4 read
// 2026-07-05. Estate references in this draft cite the crate item they describe; the
// machine-readable trustx: vocabulary file is authored in parallel (bead sq-6syab.2) from the
// same record — the record is the coordination point for term names and the namespace.
//
// HONESTY. Nothing in this document asserts a settled cryptographic soundness or privacy
// guarantee: the sparq ZK verifier has no external accredited-cryptographer sign-off (open
// audit gate sq-qhy4) and sparq-mpc is honest-majority semi-honest only. Mode 2
// ("framework-certified issuers") bottoms out in a trust anchor — the framework operator's
// signed certification artifacts — not in cryptography; the Security Considerations state the
// residual trust assumptions plainly. This draft cites NO measured performance figures; the
// spec factory's build-boundary honesty scan enforces both rules over this source.
//
// SIMPLICITY is the design's stated top requirement: the whole contract is ONE SPARQL query,
// ONE RDF trust-requirements document, and the existing zkSPARQL nonce; its meaning is fixed by a
// reference rewrite into plain SPARQL that any conformant engine can run. Every normative
// assertion carries a stable [TE-…] identifier so the conformance suite (bead sq-6syab.3) can
// be generated from the assertion inventory.

#import "_lib/spec.typ": spec-head, sotd, intro-section, references, dfn, note, cite

#set document(title: "Trust Expression: A Verifier-Holder Contract for Framework-Anchored Attestation Queries")
#set text(size: 11pt)
#set par(justify: true)
#set heading(numbering: "1.")

// An "Editor's note" aside: a detail this draft does not yet pin, to be confirmed before the
// draft advances. Mirrors the sparql-vector-genai.typ helper so both targets render it.
#let ednote(body) = context {
  if target() == "html" {
    html.elem("aside", attrs: (class: "note ednote"))[
      #html.elem("span", attrs: (class: "note-title"))[Editor's note]
      #body
    ]
  } else {
    block(width: 100%, inset: 8pt, radius: 3pt, stroke: (left: 3pt + rgb("#88a")), fill: rgb("#f4f5fb"))[
      #strong[Editor's note] — #body
    ]
  }
}

// A testable-assertion identifier. Every normative assertion carries one, rendered at the
// assertion site, so conformance is checkable assertion-by-assertion (see the
// testable-assertions section). In HTML each id is an anchor (`#req-te-…`).
#let rid(id) = context {
  let label-text = "[" + id + "]"
  if target() == "html" {
    html.elem("span", attrs: (class: "req-id", id: "req-" + lower(id)))[#label-text]
  } else {
    text(size: 8pt, weight: "bold", fill: rgb("#555"))[#label-text]
  }
}

#spec-head()

#intro-section("abstract", "Abstract")[
  This document specifies a minimal contract between a #dfn[verifier] that asks a question and
  a #dfn[holder] that answers it from issuer-attested RDF data, such that the answer is
  acceptable only under trust conditions the verifier states up front — for example: #emph["prove
  that X can be attested, based on unrevoked attributes issued by parties X, Y and Z, or based
  on certified issuers within the eIDAS or DIATF frameworks, where those issuers have only
  issued what they are certified to issue"]. The contract is deliberately as small as possible:
  #strong[one SPARQL query] (the question), #strong[one RDF trust-requirements document]
  (the trust conditions, in the `trustx:` vocabulary), and #strong[one nonce] (reusing the zkSPARQL
  challenge–response unchanged). The response is a single self-contained RDF 1.2 graph in
  which every contributing statement is reified and annotated with its issuer, its covering
  status attestation, and — in the framework mode — its issuer's certification and scope. The
  meaning of the contract is fixed by a #emph[reference rewrite]: the trust-scoped answer is,
  by definition, the answer of a plain SPARQL query (the original query conjoined with
  admissibility patterns generated from the trust requirements) evaluated over the provenance-encoded
  response, so any conformant SPARQL engine can independently re-derive it. Non-revocation and
  certification are both modelled as #emph[positive, time-windowed, signed attestations] —
  never as evidence of absence — so every question the contract asks is monotone under the
  open-world assumption and every failure is fail-closed. A named-graph + PROV-O mapping is
  fixed for SPARQL 1.1 consumers. Every normative assertion carries a testable identifier;
  the conformance suite is generated from that inventory. The zero-knowledge realisation
  composes with the (research-grade, not externally audited) zkSPARQL estate and claims no
  settled cryptographic guarantee.
]

#sotd()

#intro-section("audit-status", "Implementation and audit status")[
  This contract is #strong[specified ahead of implementation]. The sparq repository
  #cite("SPARQ") already implements several ingredients this document builds on — signed
  status lists with freshness windows, issuer-attestation binding, a committed-index
  non-revocation proof, and the `trust:` per-source vocabulary — and the informative
  implementation-status section (@sec-impl-status) reports assertion-by-assertion what exists
  today. The zero-knowledge path inherits the zkSPARQL estate's standing caveat: the
  implementation has #strong[not] been reviewed by an external cryptographer (the external
  audit gate, tracked in-repo as #emph[sq-qhy4], is open), and until it closes every positive
  security property of that path is at most a #emph[claim]. The framework mode of this
  contract additionally bottoms out in a #emph[trust anchor] — the framework operator's signed
  certification artifacts — not in cryptography (@sec-security).
]

= Introduction <sec-intro>

#emph[This section is informative.]

Verifiable-credential ecosystems increasingly sit inside #emph[trust frameworks]: a governance
scheme (eIDAS 2.0 in the EU #cite("EIDAS2"), the UK's DIATF #cite("UK-DIATF")) registers,
certifies, and publishes the parties whose attestations are acceptable, and publishes status
artifacts through which revocation is discovered. A verifier's real question is therefore
rarely "is this signature valid" — it is the compound question quoted in the Abstract: is
there an admissible derivation of the claim from attributes that were #emph[issued by parties
I trust or by parties certified under a framework I trust], that were #emph[not revoked at the
time I care about], and — in the framework case — that fall inside what their issuer was
#emph[certified to issue]?

This document gives that question a minimal machine-readable form. The design constraints,
in order:

+ #strong[Simplicity.] The verifier-to-holder contract is exactly three artifacts — a SPARQL
  query #cite("SPARQL11-QUERY"), an RDF trust-requirements document, and a nonce. There is no new query
  syntax, no new protocol state machine, and no new cryptographic mechanism.
+ #strong[Pure SPARQL semantics.] The contract's meaning is defined by a reference rewrite
  (@sec-semantics) into a plain SPARQL query over the provenance-encoded response — checkable
  by any conformant SPARQL engine, which is also the conformance suite's oracle
  (@sec-testing). Where the response encoding uses RDF 1.2 triple terms
  #cite("RDF12-CONCEPTS"), the rewrite is a SPARQL 1.2 query #cite("SPARQL12-QUERY"); a fixed
  mapping (@sec-ng-mapping) keeps SPARQL 1.1 consumers able to run it.
+ #strong[Open-world monotonicity.] "Unrevoked" and "certified" are never established from
  the absence of contrary data. Both are #emph[positive, time-windowed, signed attestations]
  (@sec-status, @sec-cert) — matching how real frameworks publish trust: an eIDAS Trusted
  List entry and a DIATF register entry are positive artifacts, and revocation status is
  published as a status list #cite("EUDI-ARF"). A missing or expired attestation yields no
  admissible binding; nothing is ever concluded from absence.
+ #strong[Composition, not duplication.] The contract reuses the zkSPARQL verifier nonce
  discipline verbatim, derives zkSPARQL trust anchors from the trust requirements on the
  zero-knowledge path, feeds the same trusted key sets the MPC attestation gate consumes, and
  delegates proof-#emph[method] admissibility to the existing ODRL profile
  (@sec-composition).

The document is an Unofficial Proposal Draft; see the Status of This Document. Its normative
surface is deliberately small and fully enumerated (@sec-testable).

= Terminology and conformance <sec-conformance>

== Requirement keywords

The key words #strong[MUST], #strong[MUST NOT], #strong[REQUIRED], #strong[SHALL],
#strong[SHALL NOT], #strong[SHOULD], #strong[SHOULD NOT], #strong[RECOMMENDED], #strong[MAY],
and #strong[OPTIONAL] in this document are to be interpreted as described in #cite("RFC2119")
and #cite("RFC8174") when, and only when, they appear in all capitals, as shown here.

== Terms <sec-terms>

- An #dfn[attested statement] is an RDF triple together with an issuer attribution whose
  authenticity can be established (for example, a triple from a signed verifiable credential
  #cite("VC-DATA-MODEL")). The attestation mechanism itself is out of scope
  (@sec-authenticity).
- An #dfn[issuer] is the party to which an attested statement is attributed.
- A #dfn[holder] is the party that controls attested statements and answers contracts over
  them.
- A #dfn[verifier] is the party that poses a contract and checks its response.
- The #dfn[trust-requirements document] is the RDF graph, in the `trustx:` vocabulary
  (@sec-vocab), that states the verifier's trust conditions (@sec-trust-requirements).
- The #dfn[evaluation time] #emph[t] is the `xsd:dateTime` value of the trust-requirements
  document's `trustx:requiresValidStatusAt` property.
- A #dfn[status attestation] is a positive, time-windowed, signed statement by a status
  authority that a credential's (or certification's) status was #emph[valid] throughout the
  window (@sec-status).
- A #dfn[certification] is a positive, time-windowed statement, signed by a framework's
  authority, that an issuer is certified under that framework for a stated scope
  (@sec-cert).
- A #dfn[framework] is a governance scheme that operates a register of certified parties and
  signs certifications — e.g. `trustx:eIDAS2`, `trustx:DIATF`.
- A #dfn[contributing statement] of a response is an attested statement that a solution of
  the query depends on — precisely: a statement matched by a reified triple pattern of the
  reference rewrite (@sec-semantics) in some solution.
- The #dfn[provenance-encoded response] #emph[R] is the single RDF graph the holder returns
  (@sec-encoding).

== Conformance classes <sec-classes>

Conformance is claimed per class. An implementation MAY implement either or both classes, but
MUST satisfy every normative assertion of each class it claims:

- A #dfn[trust-expression holder] accepts a contract, evaluates it over its attested
  statements, and produces a provenance-encoded response (or a refusal).
- A #dfn[trust-expression verifier] authors contracts and checks responses by the reference
  rewrite and the verification obligations of @sec-obligations.

The named-graph mapping of @sec-ng-mapping is an OPTIONAL feature of either class; an
implementation that claims it MUST satisfy the `TE-MAP` assertions.

== Testable assertions

Every normative assertion in this document carries a stable identifier of the form
`[TE-<group>-<n>]`, rendered at the assertion site. Identifiers are never reused; a withdrawn
assertion's identifier is retired, not reassigned. @sec-testable indexes the groups and maps
the conformance suite's case classes onto them.

= The contract <sec-contract>

== Request and response <sec-request>

#rid("TE-CON-1") A #dfn[trust-expression contract] consists of exactly three artifacts,
delivered together in one request: a SPARQL query #emph[Q], a trust-requirements document
#emph[TR], and a verifier nonce. A holder MUST treat the three as one unit: it MUST NOT evaluate #emph[Q]
under any trust-requirements document other than the one delivered with it.

#rid("TE-CON-2") #emph[Q] MUST be a SPARQL `ASK` or `SELECT` query whose `WHERE` clause is a
basic graph pattern, optionally with `FILTER` constraints, in which every predicate is a
constant (an IRI). Queries outside this fragment are out of scope for this revision; a holder
receiving one MUST refuse the contract rather than answer a narrower question silently.

#note[
  The fragment is the conjunctive core every SPARQL engine shares and the reference rewrite
  is total over it. It deliberately matches the scoped fragment of the zkSPARQL draft
  #cite("ZKSPARQL"), so the same contract can be answered on the clear path or the
  zero-knowledge path without renegotiation. Widening the fragment (OPTIONAL, UNION, property
  paths) is future work and requires extending the rewrite, not the trust-requirements document.
]

#rid("TE-CON-3") The holder's answer to a contract is a #emph[provenance-encoded response]
#emph[R]: a single RDF graph conforming to @sec-encoding. The #emph[authoritative] answer of
the contract is defined by evaluation of the rewritten query #emph[Q′] over #emph[R]
(@sec-semantics); if the holder additionally transmits a plain SPARQL results document, that
document is informative and a verifier MUST NOT rely on it.

#rid("TE-NON-1") The verifier MUST mint a fresh, single-use nonce per contract, following the
verifier-nonce discipline of the zkSPARQL draft (#cite("ZKSPARQL"), section 9) unchanged;
nonces MUST NOT be reused across contracts.

#rid("TE-NON-2") On the zero-knowledge path (@sec-zk), the nonce is the zkSPARQL manifest
nonce: every sub-proof MUST commit to it, exactly as specified there. This document adds no
second freshness mechanism.

#rid("TE-NON-3") On the clear path, the holder MUST echo the contract's nonce with its
response, and a verifier MUST reject a response that echoes a different nonce or a nonce it
has already accepted a response for. @sec-security states plainly that on the clear path this
echo provides request–response correlation only, not cryptographic freshness.

== The trust-requirements document <sec-trust-requirements>

The trust-requirements document is a small RDF graph. Its vocabulary is defined in
@sec-vocab; its two trust modes are defined in @sec-modes. Example — mode 1, enumerated issuers:

```turtle
PREFIX trustx: <https://sparq.dev/ns/trust#>
PREFIX xsd:    <http://www.w3.org/2001/XMLSchema#>

[] a trustx:TrustRequirements ;
   trustx:question <urn:example:q1> ;
   trustx:trustsIssuer <did:web:x.example>, <did:web:y.example>, <did:web:z.example> ;
   trustx:requiresValidStatusAt "2026-07-05T00:00:00Z"^^xsd:dateTime .
```

Example — mode 2, framework-certified issuers:

```turtle
[] a trustx:TrustRequirements ;
   trustx:question <urn:example:q1> ;
   trustx:trustsFramework trustx:eIDAS2, trustx:DIATF ;
   trustx:requiresScopeConformance true ;
   trustx:requiresValidStatusAt "2026-07-05T00:00:00Z"^^xsd:dateTime .
```

Well-formedness. In the following, "the requirements node" is the node of type
`trustx:TrustRequirements`:

#rid("TE-REQ-1") A trust-requirements graph MUST contain exactly one node with `rdf:type`
`trustx:TrustRequirements`.

#rid("TE-REQ-2") The requirements node MUST carry exactly one `trustx:question`, whose value is
the IRI the verifier assigns to #emph[Q] in this contract. The property binds the
trust-requirements document to the query delivered with it; a holder MUST NOT reuse a
trust-requirements document for any other query.

#rid("TE-REQ-3") The requirements node MUST carry at least one `trustx:trustsIssuer` (mode 1) or
at least one `trustx:trustsFramework` (mode 2). It MAY carry both (@sec-combining).

#rid("TE-REQ-4") The requirements node MUST carry exactly one `trustx:requiresValidStatusAt`,
whose value MUST be an `xsd:dateTime` with an explicit timezone. This value is the evaluation
time #emph[t] for every window check in the contract.

#rid("TE-REQ-5") A requirements node carrying `trustx:trustsFramework` MUST also carry
`trustx:requiresScopeConformance` with the value `true`. In this revision scope conformance
is inseparable from framework trust (the framework mode exists precisely to enforce "issuers
only issued what they are certified to issue"); the value `false` is reserved for a future
revision, and a consumer encountering it MUST reject the document as not conforming to this
revision.

#rid("TE-REQ-6") The requirements node MAY carry at most one `trustx:methodPolicy`, whose value
identifies an ODRL policy #cite("ODRL22") constraining acceptable proof #emph[methods]; its
evaluation is delegated to the zkSPARQL admissibility profile (@sec-odrl) and is orthogonal
to the source-trust conditions of this document.

#rid("TE-REQ-7") A consumer of a trust-requirements document (holder or verifier) MUST reject it as
malformed if any assertion `TE-REQ-1` through `TE-REQ-6` fails, or if the document uses a
term in the `trustx:` vocabulary that the consumer does not implement. A trust-requirements
document is a security artifact: ignoring an unrecognised constraint would answer a #emph[weaker] question
than the verifier asked, so unknown-term tolerance is forbidden (fail-closed).

#rid("TE-REQ-8") A holder that rejects a contract (malformed trust-requirements document, out-of-fragment query,
or any other cause) MUST return a refusal rather than a partial or unconstrained answer.

= Trust modes <sec-modes>

One trust-requirements shape carries both trust modes; they differ only in how #emph[issuer
admissibility] is established. Throughout, statement-level admissibility additionally
requires status coverage (@sec-status).

== Mode 1 — enumerated issuers <sec-mode1>

#rid("TE-MODE-1") Under a trust-requirements document with `trustx:trustsIssuer` values
#emph[I₁ … Iₖ], an issuer #emph[I] is #dfn[issuer-admissible] if and only if #emph[I] is
equal (RDF term equality) to some #emph[Iⱼ]. Issuer identifiers are IRIs; DID-based
identifiers are compared as IRIs without resolution.

== Mode 2 — framework-certified issuers <sec-mode2>

#rid("TE-MODE-2") Under a trust-requirements document with `trustx:trustsFramework` values
#emph[F₁ … Fₘ], an issuer #emph[I] is issuer-admissible for a contributing statement
#emph[s] if and only if the response contains a certification #emph[C] such that:

+ #emph[C] has `rdf:type` `trustx:Certification`, `trustx:certifies` #emph[I], and
  `trustx:underFramework` #emph[Fⱼ] for some #emph[j];
+ #emph[C] is valid at the evaluation time #emph[t] (`TE-CERT-2`); and
+ #emph[s] falls under a scope of #emph[C] (`TE-CERT-5`).

== Combining modes <sec-combining>

#rid("TE-MODE-3") When a trust-requirements document carries both `trustx:trustsIssuer` and
`trustx:trustsFramework`, the modes combine by disjunction: a contributing statement's issuer
is issuer-admissible if it is admissible under either mode. A verifier that wants the modes
kept separate poses two contracts.

#rid("TE-MODE-4") A contributing statement whose issuer is not issuer-admissible yields no
admissible binding: solutions depending on it are absent from the contract's answer. Neither
holder nor verifier may substitute a weaker issuer condition (fail-closed).

= Status attestations: non-revocation as positive evidence <sec-status>

The contract never asks "is there evidence of revocation?"; it asks "is there positive,
authentic evidence of #emph[validity] covering #emph[t]?". This direction is what keeps every
question monotone under the open-world assumption: adding data can only add admissible
bindings, never invalidate the reasoning that produced one.

#rid("TE-STAT-1") A #dfn[status attestation] is a node with `rdf:type`
`trustx:StatusAttestation` carrying exactly one `trustx:validFrom` and exactly one
`trustx:validUntil` (both `xsd:dateTime` with explicit timezone), whose authenticity as a
statement by the relevant status authority can be established by the verifier
(@sec-authenticity).

#rid("TE-STAT-2") A status attestation #emph[covers] the evaluation time #emph[t] if and only
if `validFrom` ≤ #emph[t] ≤ `validUntil` under `xsd:dateTime` ordering.

#rid("TE-STAT-3") A contributing statement is #dfn[status-admissible] if and only if its
reifier carries `trustx:coveredBy` linking it to a status attestation that covers #emph[t]
and whose authenticity the verifier has established. A revoked credential, an expired window,
and a missing attestation are indistinguishable in effect: no covering attestation, no
admissible binding.

#rid("TE-STAT-4") A consumer MUST NOT infer validity from the absence of revocation data, and
MUST NOT interpret the absence of an admissible binding as an assertion that the queried
statement is false. A negative `ASK` answer under this contract means "not established under
these trust requirements", never "established false".

#note[
  Realisations. On the clear path, the sparq estate realises status attestations as signed
  Bitstring status lists #cite("BITSTRING-STATUS") with a freshness window — the
  `SignedStatusList` and `VerifyingLiveStatusCheck` types and the `justify_status_decision`
  function in `crates/sparq-trust/src/status_list.rs`, the last of which already emits the
  justification triples this encoding carries. On the zero-knowledge path, the estate
  realises the same shape as a committed-index bit-unset proof at an authoritative snapshot
  (`zk/compose/compose_core/src/revoke.nr`; `crates/sparq-zk-compose/src/revocation.rs`).
  The IETF Token Status List mechanism tracked by the EUDI ARF #cite("TOKEN-STATUS")
  #cite("EUDI-ARF") maps to the same positive time-windowed shape. All three are encodings
  of one abstract artifact; this specification pins the artifact, not the transport.
]

= Certifications and scope <sec-cert>

Mode 2's certification is the same positive-attestation machinery applied one level up: the
framework's register entry for an issuer #emph[is] a certification, and it is validity-checked
exactly like a credential.

#rid("TE-CERT-1") A #dfn[certification] is a node with `rdf:type` `trustx:Certification`
carrying exactly one `trustx:certifies` (the certified issuer), exactly one
`trustx:underFramework` (a `trustx:Framework`), at least one `trustx:scope`, exactly one
`trustx:validFrom` and exactly one `trustx:validUntil` (both `xsd:dateTime` with explicit
timezone), whose authenticity as a statement by the framework's authority can be established
by the verifier (@sec-authenticity).

#rid("TE-CERT-2") A certification is #dfn[valid at] #emph[t] if and only if (i) its
`validFrom`/`validUntil` window covers #emph[t] under `xsd:dateTime` ordering, #strong[and]
(ii) it carries `trustx:coveredBy` linking it to a status attestation that covers #emph[t]
and whose authenticity the verifier has established. Certification revocation is thereby
positive-attestation-checked exactly like credential revocation; there is no second
mechanism. (Status attestations themselves are not further status-checked: their freshness
#emph[is] their window, so the recursion terminates.)

#rid("TE-CERT-3") A `trustx:scope` value MUST be one of:

+ the IRI `trustx:AnyServiceScope` — everything the certified service issues is in
  scope (the honest granularity of service-level certification regimes such as DIATF
  #cite("UK-DIATF"), where certification attaches to a service, not an attribute list);
+ an IRI naming an #emph[attestation type] (for example an eIDAS Attestation Rulebook
  identifier #cite("EUDI-ARF")); or
+ a SHACL shape #cite("SHACL"), with `trust:forPredicate` admitted as sugar that desugars
  into a single-predicate `trust:forShape` shape exactly as pinned by
  `desugar_for_predicate` in `crates/sparq-trust/src/vocab.rs` (this contract reuses the
  `trust:` vocabulary's one statement-type idiom rather than inventing a second).

#rid("TE-CERT-4") A contributing statement #emph[(s, p, o)] #dfn[falls under] a scope value
#emph[σ] if and only if:

+ #emph[σ] `=` `trustx:AnyServiceScope`; or
+ #emph[σ] is an attestation-type IRI #emph[T], and the statement's reifier carries
  `prov:wasDerivedFrom` linking it to an attestation node whose `rdf:type` includes
  #emph[T] and whose authenticity attribution matches the statement's issuer; or
+ #emph[σ] is a SHACL shape, and #emph[s] is in #emph[σ]'s target set and conforms to
  #emph[σ] when validating the graph of contributing statements attributed to the same
  issuer. For the `trust:forPredicate` desugaring this reduces to the predicate test
  #emph[p] `=` #emph[P].

#rid("TE-CERT-5") Under `trustx:requiresScopeConformance true`, #strong[every] contributing
statement admitted through mode 2 MUST fall under at least one scope of a certification that
is valid at #emph[t], certifies the statement's issuer, and is under a trusted framework.
This is the machine-checkable rendering of "issuers have only issued information that they
are certified to issue" — precisely: #emph[the verifier accepts a statement only if its
issuer was certified, at t, to issue statements of its kind]. What this check can and cannot
establish is stated in @sec-security.

#rid("TE-CERT-6") The set of framework authority keys against which certification and status
authenticity is established is a verifier trust anchor: a verifier MUST obtain it out of
band and MUST NOT accept it from the response. (This is the zkSPARQL trust-anchor
externality rule, applied to the framework layer.)

= Response encoding <sec-encoding>

== Normative encoding: RDF 1.2 reifiers <sec-reifiers>

The response #emph[R] is one self-contained RDF 1.2 graph #cite("RDF12-CONCEPTS"). Each
contributing statement appears as a triple term under `rdf:reifies`; the reifier node is the
subject of all provenance about that statement — exactly the qualification subject PROV-O
#cite("PROV-O") wants.

#rid("TE-ENC-1") For every contributing statement #emph[(s, p, o)], #emph[R] MUST contain a
#dfn[reifier] node #emph[r] with: `r rdf:reifies <<( s p o )>>`, exactly one
`r prov:wasAttributedTo issuer`, and at least one `r trustx:coveredBy statusAttestation`.

#rid("TE-ENC-2") When the contract is answered under mode 2, #emph[R] MUST additionally
contain, for every certification relied on: the certification node with all `TE-CERT-1`
properties, its covering status attestation, and — when an attestation-type scope is relied
on — the `prov:wasDerivedFrom` link and attestation-node typing of `TE-CERT-4`.

#rid("TE-ENC-3") #emph[R] MUST contain enough material for the verifier to establish the
authenticity of every attribution, status attestation, and certification it relies on
(@sec-authenticity), except for material the verifier holds as a trust anchor.

#rid("TE-ENC-4") Contributing statements are conveyed #emph[only] as triple terms under
`rdf:reifies`; asserting them additionally as plain triples of #emph[R] is OPTIONAL and
carries no conformance weight — the reference rewrite never consults asserted base triples,
so an asserted triple without a conformant reifier contributes nothing.

Example — a response fragment for the worked example of @sec-example:

```turtle
PREFIX rdf:    <http://www.w3.org/1999/02/22-rdf-syntax-ns#>
PREFIX trustx: <https://sparq.dev/ns/trust#>
PREFIX prov:   <http://www.w3.org/ns/prov#>
PREFIX xsd:    <http://www.w3.org/2001/XMLSchema#>
PREFIX ex:     <https://example.org/>

_:r1 rdf:reifies <<( ex:holder ex:over18 true )>> ;
     prov:wasAttributedTo <did:web:x.example> ;
     trustx:coveredBy _:st1 .

_:st1 a trustx:StatusAttestation ;
      trustx:validFrom  "2026-07-01T00:00:00Z"^^xsd:dateTime ;
      trustx:validUntil "2026-07-08T00:00:00Z"^^xsd:dateTime .
```

== Named-graph mapping for SPARQL 1.1 consumers <sec-ng-mapping>

#emph[The choice to use this mapping is OPTIONAL (@sec-classes); the mapping itself is fixed
normatively so that implementations that down-convert do so identically.] The mapping exists
because RDF 1.2 triple-term matching is not yet available in most deployed engines — including
sparq's own SPARQL surface today (@sec-impl-status); the conformance suite's oracle runs over
this form until it is (@sec-testing).

#rid("TE-MAP-1") Forward mapping (reifiers → dataset). For each reifier #emph[r] with
`r rdf:reifies <<( s p o )>>`: emit the triple #emph[(s, p, o)] into a named graph whose name
is #emph[r] (skolemising #emph[r] to an IRI if it is a blank node, by a skolemisation the
implementation keeps stable within one response); emit every other triple whose subject or
object is #emph[r] into the default graph with #emph[r] replaced by the graph name; emit all
remaining triples into the default graph unchanged.

#rid("TE-MAP-2") Reverse mapping (dataset → reifiers). For each named graph #emph[g]
containing triples #emph[(s, p, o)]: emit `g rdf:reifies <<( s p o )>>` for each, and emit
all default-graph triples unchanged. The reverse mapping MUST reproduce the original graph
up to blank-node skolemisation of reifiers.

#rid("TE-MAP-3") The two mappings MUST round-trip losslessly: forward followed by reverse
yields an RDF graph isomorphic (modulo reifier skolemisation) to the input, and reverse
followed by forward yields an isomorphic dataset. This is conformance case class 8
(@sec-testing).

#note[
  The named-graph form is a TriG #emph[dataset], not a single graph, and RDF assigns named
  graphs no fixed semantics — two of the reasons the reifier form is the normative one for a
  self-contained response document. PROV-O qualification applies unchanged: the graph name
  plays the reifier's role as qualification subject.
]

= Evaluation semantics: the reference rewrite <sec-semantics>

The normative meaning of "evaluate #emph[Q] under #emph[TR]" is a rewrite into plain SPARQL
over #emph[R]. This is the load-bearing simplicity move: the contract needs no bespoke
evaluator — #emph[any] conformant SPARQL engine can re-derive the answer, and the conformance
suite uses exactly that as its oracle.

== The rewrite <sec-rewrite>

#rid("TE-SEM-1") Given #emph[Q] (fragment of `TE-CON-2`) with basic graph pattern
#emph[tp₁ … tpₙ] and filters #emph[F], and a trust-requirements document #emph[TR] with evaluation time
#emph[t], the #dfn[reference rewrite] #emph[Q′] is the query with the same query form and
projection as #emph[Q] whose `WHERE` clause is the conjunction of, for each
#emph[tpᵢ = (sᵢ, pᵢ, oᵢ)] (with fresh variables `?r_i`, `?issuer_i`, `?st_i`, `?f_i`,
`?u_i` per pattern):

```sparql
  ?r_i rdf:reifies <<( ?s_i p_i ?o_i )>> ;      # s_i/o_i kept verbatim if constant
       prov:wasAttributedTo ?issuer_i ;
       trustx:coveredBy ?st_i .
  ?st_i a trustx:StatusAttestation ;
        trustx:validFrom ?f_i ;
        trustx:validUntil ?u_i .
  FILTER (?f_i <= t && t <= ?u_i)
  # issuer admissibility: mode 1, mode 2, or their UNION per TE-MODE-3
```

together with #emph[Q]'s own filters #emph[F]. The mode-1 issuer-admissibility constraint is
`FILTER (?issuer_i IN (I1, …, Ik))`. The mode-2 constraint is the graph pattern (fresh
variables per pattern):

```sparql
  ?c_i a trustx:Certification ;
       trustx:certifies ?issuer_i ;
       trustx:underFramework ?fw_i ;
       trustx:scope ?sc_i ;
       trustx:validFrom ?cf_i ;
       trustx:validUntil ?cu_i ;
       trustx:coveredBy ?cst_i .
  ?cst_i a trustx:StatusAttestation ;
         trustx:validFrom ?csf_i ;
         trustx:validUntil ?csu_i .
  FILTER (?fw_i IN (F1, …, Fm))
  FILTER (?cf_i <= t && t <= ?cu_i)
  FILTER (?csf_i <= t && t <= ?csu_i)
  # scope conformance for tp_i, per TE-SEM-2
```

#rid("TE-SEM-2") Within the mode-2 constraint for #emph[tpᵢ], scope conformance is rendered
as the `UNION` of the scope forms admitted by the frameworks of #emph[TR]:
(i)~`?sc_i` equal to `trustx:AnyServiceScope`;
(ii)~attestation-type scope — `?r_i prov:wasDerivedFrom ?a_i . ?a_i rdf:type ?sc_i .`
(iii)~single-predicate shape scope — `?sc_i sh:targetSubjectsOf p_i .` matching the
`trust:forPredicate` desugaring.
General SHACL shape scopes (beyond the single-predicate desugaring) are #strong[not]
expressible inside #emph[Q′]; a verifier relying on one MUST additionally validate the
statement against the shape with a SHACL processor (`TE-CERT-4`), and the conformance suite
carries such cases as paired SPARQL + SHACL checks.

#rid("TE-SEM-3") The #dfn[trust-scoped answer] of a contract is, by definition: for `ASK`,
`true` if and only if #emph[Q′] has at least one solution over #emph[R] #emph[after] the
verification obligations of @sec-obligations have discharged; for `SELECT`, the solutions of
#emph[Q′] over #emph[R] projected to #emph[Q]'s projection variables. This definition is the
conformance oracle.

== Verification obligations <sec-obligations>

#rid("TE-SEM-4") Before evaluating #emph[Q′], a verifier MUST establish the authenticity of
every attribution, status attestation, and certification in #emph[R] that #emph[Q′] would
consult, against its out-of-band trust anchors, and MUST remove (or refuse the response over)
any it cannot establish. Verification precedes query evaluation; unverifiable provenance is
treated as absent (fail-closed).

#rid("TE-SEM-5") A verifier MUST fail closed on any check it cannot complete — unreachable
trust anchor, unparseable response, unsupported scope form — rejecting the response rather
than downgrading the failed check to a warning or answering from the remainder.

#rid("TE-SEM-6") A holder SHOULD evaluate the same rewrite over its own data before
responding, and MUST NOT include in #emph[R] provenance it knows to be outside the conditions of
#emph[TR] in order to widen the answer; a verifier's re-evaluation (`TE-SEM-3`) is
authoritative regardless.

== Authenticity establishment <sec-authenticity>

#emph[This subsection is informative.] How signatures bind issuers to statements, status
authorities to status attestations, and framework authorities to certifications is
deliberately out of this document's scope: it is the province of the credential layer (W3C
Verifiable Credentials with Data Integrity proofs #cite("VC-DATA-MODEL") #cite("VC-DI"), the
sparq estate's issuer-attestation binding, or a successor). This document requires only the
#emph[obligation] shape: each such binding can be checked by the verifier against material
obtained out of band (`TE-CERT-6`, `TE-SEM-4`), and every check that cannot be completed
fails closed (`TE-SEM-5`). The conformance suite exercises the obligation with fixtures whose
authenticity bit is simply given as valid or invalid.

== Worked example <sec-example>

#emph[This subsection is informative.] The motivating question — #emph["prove that
#raw("ex:holder") is over 18, based on unrevoked attributes issued by X, Y or Z, or based on
certified issuers within eIDAS or DIATF"] — is the contract: nonce; query

```sparql
ASK { ex:holder ex:over18 true }
```

and a trust-requirements document carrying both modes:

```turtle
[] a trustx:TrustRequirements ;
   trustx:question <urn:example:q1> ;
   trustx:trustsIssuer <did:web:x.example>, <did:web:y.example>, <did:web:z.example> ;
   trustx:trustsFramework trustx:eIDAS2, trustx:DIATF ;
   trustx:requiresScopeConformance true ;
   trustx:requiresValidStatusAt "2026-07-05T00:00:00Z"^^xsd:dateTime .
```

The rewrite of the single pattern `(ex:holder, ex:over18, true)` is (mode-1 branch shown;
the full #emph[Q′] is this `UNION`ed with the mode-2 pattern of @sec-rewrite):

```sparql
PREFIX rdf:    <http://www.w3.org/1999/02/22-rdf-syntax-ns#>
PREFIX trustx: <https://sparq.dev/ns/trust#>
PREFIX prov:   <http://www.w3.org/ns/prov#>
PREFIX xsd:    <http://www.w3.org/2001/XMLSchema#>
PREFIX ex:     <https://example.org/>

ASK {
  ?r1 rdf:reifies <<( ex:holder ex:over18 true )>> ;
      prov:wasAttributedTo ?issuer1 ;
      trustx:coveredBy ?st1 .
  ?st1 a trustx:StatusAttestation ;
       trustx:validFrom ?f1 ;
       trustx:validUntil ?u1 .
  FILTER (?issuer1 IN (<did:web:x.example>, <did:web:y.example>, <did:web:z.example>))
  FILTER (?f1 <= "2026-07-05T00:00:00Z"^^xsd:dateTime &&
          "2026-07-05T00:00:00Z"^^xsd:dateTime <= ?u1)
}
```

Run over the response fragment of @sec-reifiers, this `ASK` answers `true` — and it answers
`false` (meaning: #emph[not established]) the moment the status window lapses, the issuer
falls outside the enumerated set, or the reifier chain is incomplete. No trust logic lives
anywhere except the trust-requirements document and the rewrite.

= The trustx: vocabulary <sec-vocab>

#rid("TE-VOC-1") The `trustx:` terms of this specification are the following, and conformant
implementations MUST use exactly these names. The prefix `trustx:` binds to
`https://sparq.dev/ns/trust#` — the #strong[same placeholder namespace] as the existing
`trust:` vocabulary, with the distinct prefix marking the framework-extension stratum in
prose, following the `secx:`/`sec-prop:` same-namespace precedent in the sparq estate. All
`trustx:` IRIs are #strong[non-standard, invented for this proposal]; the namespace is a
placeholder — not minted, not resolvable, and not a claim of standardisation. A working group
taking this design forward would rename and rehome every term.

#table(
  columns: 3,
  align: (left, left, left),
  table.header[Term][Kind][Meaning],
  [`trustx:TrustRequirements`], [class], [the verifier's trust conditions (@sec-trust-requirements)],
  [`trustx:question`], [property], [binds a trust-requirements document to the query it
    governs (`TE-REQ-2`)],
  [`trustx:trustsIssuer`], [property], [mode-1 enumerated issuer (@sec-mode1)],
  [`trustx:trustsFramework`], [property], [mode-2 trusted framework (@sec-mode2)],
  [`trustx:requiresScopeConformance`], [property], [`xsd:boolean`; `true` in this revision
    (`TE-REQ-5`)],
  [`trustx:requiresValidStatusAt`], [property], [the evaluation time #emph[t] (`TE-REQ-4`)],
  [`trustx:methodPolicy`], [property], [OPTIONAL ODRL proof-method policy (`TE-REQ-6`)],
  [`trustx:Framework`], [class], [a governance framework operating a register
    (@sec-cert)],
  [`trustx:eIDAS2`], [individual], [the eIDAS 2.0 framework #cite("EIDAS2")],
  [`trustx:DIATF`], [individual], [the UK DIATF framework #cite("UK-DIATF")],
  [`trustx:Certification`], [class], [a framework's certification of an issuer
    (`TE-CERT-1`)],
  [`trustx:certifies`], [property], [certification → certified issuer],
  [`trustx:underFramework`], [property], [certification → its framework],
  [`trustx:scope`], [property], [certification → what the issuer may issue (`TE-CERT-3`)],
  [`trustx:AnyServiceScope`], [scope value], [service-level scope: everything the certified
    service issues],
  [`trustx:validFrom`], [property], [window start (`xsd:dateTime`)],
  [`trustx:validUntil`], [property], [window end (`xsd:dateTime`)],
  [`trustx:StatusAttestation`], [class], [positive time-windowed validity statement
    (@sec-status)],
  [`trustx:coveredBy`], [property], [reifier or certification → its covering status
    attestation],
)

#rid("TE-VOC-2") The framework individuals `trustx:eIDAS2` and `trustx:DIATF` carry
`rdfs:seeAlso` links to the corresponding regulatory-requirement individuals already vendored
in the sparq estate from #cite("SEC-PROP") (`sec-req:Eidas20` and `sec-req:UkDvs`, in
`crates/sparq-trust/ontologies/zkp-sparql/vocab/sec-req.yaml.ld`); the `sec-req:` view (what
a regulation requires) and the `trustx:` view (how certification under it is checked) MUST
remain distinct vocabularies — neither duplicates the other's terms.

#ednote[
  The machine-readable rendering of this vocabulary —
  `crates/sparq-trust/ontologies/trust/trust-framework.ttl`, byte-pinned to Rust constants by
  a sync test like `trust.ttl`/`vocab.rs` — is authored in parallel (bead #emph[sq-6syab.2])
  from the same design record this draft follows. Term names and the namespace above follow
  that record exactly; if the two artifacts drift, the record is the arbiter and whichever
  lands second reconciles.
]

= Composition with the sparq estate <sec-composition>

#emph[This section is informative] (its one normative hook is `TE-REQ-6`). The contract is a
thin layer over surfaces that already exist; it duplicates none of them.

== Proof-method admissibility (ODRL) <sec-odrl>

The trust-requirements document decides which #emph[sources] are acceptable; the zkSPARQL security-properties
vocabulary and its ODRL admissibility profile (#cite("ZKSPARQL"), sections 12–13; the
`admissible()` entry point in `crates/sparq-trust/src/admissibility.rs`) decide which proof
#emph[methods] are. The axes are orthogonal by design. A trust-requirements document's `trustx:methodPolicy`
(`TE-REQ-6`) is a pointer into that existing machinery, not a restatement of it.

== Provenance (PROV) <sec-prov>

The reifier is the PROV-O qualification subject: `prov:wasAttributedTo` carries issuer
attribution (`TE-ENC-1`) and `prov:wasDerivedFrom` carries attestation-of-origin links
(`TE-CERT-4`). Citation chains reuse the `prov-ext:` pattern vendored in
`crates/sparq-trust/ontologies/zkp-sparql/vocab/prov-ext.yaml.ld`. No new provenance terms
are minted.

== Multi-party computation <sec-mpc>

In the federated case #cite("MPC-SPARQL"), the trust-requirements document is the #emph[generator] of the
trusted key set #emph[K] the MPC attestation gate consumes (`bind_issuer_attestations` in
`crates/sparq-zk-compose/src/verifier.rs` and the planned M4 verifier-side attestation gate):
mode 1 enumerates #emph[K]; mode 2 derives #emph[K] from the framework's valid
certifications at #emph[t]. This document designs no new MPC surface, and the standing caveat
applies wherever MPC is mentioned: `sparq-mpc` is honest-majority #strong[semi-honest] only.

== Zero-knowledge realisation (zkSPARQL) <sec-zk>

The zero-knowledge realisation of a contract is a zkSPARQL proof manifest
#cite("ZKSPARQL") whose trust anchors are #emph[derived from the trust requirements] rather than
configured ad hoc: mode 1 maps to issuer-attestation binding; mode 2 with issuer privacy
maps to the estate's hidden-issuer set membership over #emph[K]
(`bind_hidden_issuer_attestations`); non-revocation maps to the committed-index proof of
@sec-status. The genuinely new zero-knowledge obligation is #strong[certification-scope
binding] — establishing, without disclosing more than the answer, that the proven statements
fall inside a certified scope. That component is #strong[unbuilt] (tracked as bead
#emph[sq-6syab.5]), is soundness-sensitive, and — like the whole zkSPARQL estate — sits
behind the open external-audit gate #emph[sq-qhy4]: no clause of this document asserts a
settled zero-knowledge guarantee for it.

= Conformance testing <sec-testing>

== Assertion index <sec-testable>

#table(
  columns: 3,
  align: (left, left, left),
  table.header[Group][Section][Scope],
  [`TE-CON`], [@sec-request], [contract shape and response authority],
  [`TE-NON`], [@sec-request], [nonce discipline],
  [`TE-REQ`], [@sec-trust-requirements], [trust-requirements well-formedness, fail-closed rejection],
  [`TE-MODE`], [@sec-modes], [issuer admissibility, mode combination],
  [`TE-STAT`], [@sec-status], [positive time-windowed status coverage],
  [`TE-CERT`], [@sec-cert], [certification validity, scope, framework anchors],
  [`TE-ENC`], [@sec-reifiers], [reifier response encoding],
  [`TE-MAP`], [@sec-ng-mapping], [named-graph mapping (OPTIONAL feature)],
  [`TE-SEM`], [@sec-semantics], [reference rewrite, verification obligations],
  [`TE-VOC`], [@sec-vocab], [vocabulary names and framework individuals],
  [`TE-SEC`], [@sec-security], [relying-party obligations],
)

== Case classes <sec-cases>

The conformance suite (a W3C-style manifest with fixture graphs and expected outcomes,
maintained in the sparq repository under `crates/sparq-trust/tests/trust-expression/` and
designed to lift verbatim into this specification's eventual upstream home) is generated
from the assertion inventory. Its eight case classes, every negative case fail-closed:

#table(
  columns: 3,
  align: (left, left, left),
  table.header[Case class][Expected outcome][Primary assertions],
  [1. Mode 1 pass], [answer established], [`TE-MODE-1`, `TE-STAT-3`, `TE-SEM-3`],
  [2. Revoked at #emph[t]], [no admissible binding], [`TE-STAT-3`, `TE-STAT-4`],
  [3. Stale status window], [no admissible binding], [`TE-STAT-2`, `TE-STAT-3`],
  [4. Untrusted issuer], [no admissible binding], [`TE-MODE-1`, `TE-MODE-4`],
  [5. Mode 2 pass], [answer established], [`TE-MODE-2`, `TE-CERT-2`, `TE-CERT-5`],
  [6. Scope violation], [no admissible binding], [`TE-CERT-4`, `TE-CERT-5`],
  [7. Certification expired or revoked at #emph[t]], [no admissible binding],
    [`TE-CERT-2`],
  [8. Encoding round-trip], [lossless], [`TE-MAP-1`, `TE-MAP-2`, `TE-MAP-3`],
)

== Oracle and current engine gap <sec-oracle>

The oracle for case classes 1–7 is `TE-SEM-3` itself: evaluate #emph[Q′] over the fixture
response with a conformant SPARQL engine and compare with the expected outcome. Because
SPARQL 1.2 triple-term matching is not yet available in the sparq engine (or most deployed
engines), the suite's runnable oracle evaluates #emph[Q′] over the named-graph form of
@sec-ng-mapping and checks the reifier form structurally via the estate's RDF 1.2 triple-term
parser (`crates/sparq-core/src/nt.rs`); the two are equivalent by `TE-MAP-3`. Scope cases
that use a general SHACL shape pair the SPARQL oracle with a SHACL validation (`TE-SEM-2`).
An assertion that cannot be paired with at least one test before this document advances
beyond Unofficial Proposal Draft will be reworded until testable, or demoted to informative;
the suite carries no unexplained known-failing entries.

= sparq implementation status <sec-impl-status>

#emph[This section is informative.] It reports, per assertion group, what the sparq estate
provides today, per the design record's verified estate survey
(`research/trust-expression-spec.md`, read against the crates on 2026-07-05).

#table(
  columns: 3,
  align: (left, left, left),
  table.header[Assertions][sparq status][Notes],
  [`TE-CON-*`, `TE-REQ-*`, `TE-SEM-*`], [Not built], [holder-side contract evaluation —
    trust-requirements parsing, the reference rewrite, response assembly — is future work
    (bead sq-6syab.4), sequenced after the zkSPARQL fragment-extension program],
  [`TE-NON-*`], [Built (ZK path)], [the zkSPARQL verifier-nonce discipline, reused verbatim],
  [`TE-MODE-1`], [Built (ZK path)], [issuer-attestation binding in
    `crates/sparq-zk-compose/src/verifier.rs`; hidden-issuer set membership for the
    issuer-hiding variant],
  [`TE-MODE-2`, `TE-CERT-*`], [Not built], [the certification/scope layer is new; its
    vocabulary file is authored in parallel (bead sq-6syab.2); zero-knowledge scope binding
    is unbuilt and audit-gated (bead sq-6syab.5, gate sq-qhy4)],
  [`TE-STAT-*`], [Built (both paths)], [signed Bitstring status lists with freshness windows
    (`crates/sparq-trust/src/status_list.rs`) and the committed-index non-revocation proof
    (`zk/compose/compose_core/src/revoke.nr`)],
  [`TE-ENC-*`], [Partially built], [RDF 1.2 triple terms parse in `crates/sparq-core/src/nt.rs`
    (object position, depth-bounded); no SPARQL 1.2 triple-term query surface exists in
    `crates/sparq-parse`, so #emph[Q′] cannot yet run natively over the reifier form],
  [`TE-MAP-*`], [Not built], [the mapping is specified here; the conformance suite
    (bead sq-6syab.3) exercises it],
  [`TE-VOC-*`], [In flight], [`trust-framework.ttl` + byte-pinned Rust constants,
    bead sq-6syab.2],
  [`TE-SEC-*`], [Deployment obligations], [not machine-checkable per se; the audit-status
    reporting obligation mirrors the zkSPARQL draft's],
)

= Security Considerations <sec-security>

Everything in this section is load-bearing; none of it is boilerplate.

== No externally audited cryptography

The zero-knowledge path's security rests on an implementation that has been internally
re-audited but has #strong[no external accredited-cryptographer sign-off] (open gate
#emph[sq-qhy4]), and the MPC estate is honest-majority semi-honest only. #rid("TE-SEC-1") A
relying party deploying the zero-knowledge realisation MUST treat every positive security
property of that path as a claim pending external audit, and MUST surface that status to its
own users where the zkSPARQL draft's audit-status obligations require it. The conformance
clauses of this document are phrased as "matches this specification" and "fail-closed" —
never as unqualified soundness or privacy properties.

== Framework trust is anchored, not proven

Mode 2 bottoms out in the framework operator's signed certification and status artifacts.
The scope-conformance check (`TE-CERT-5`) constrains what the #emph[verifier accepts], and —
via the published certification — what the issuer was #emph[authorised] to issue; it cannot
retroactively establish that an issuer never mis-issued elsewhere, and it inherits the
integrity of the framework's registration and certification processes wholesale. A framework
authority key compromise defeats mode 2 entirely; `TE-CERT-6` exists to keep that anchor
under the verifier's control.

== The clear path trusts the holder's disclosure

Without a zero-knowledge manifest, the verifier re-checks admissibility over #emph[R]
(`TE-SEM-3`, `TE-SEM-4`) and verifies the underlying attestations' authenticity — but the
completeness of what the holder chose to disclose is inherently unverifiable, which is why
the contract only ever asks monotone existence questions (`TE-STAT-4`): a holder can withhold
an answer, but cannot manufacture one that #emph[Q′] plus authentic attestations do not
support. The clear-path nonce echo (`TE-NON-3`) provides correlation, not cryptographic
freshness: replay resistance on the clear path comes from the status-attestation windows and
the verifier-chosen #emph[t], and a verifier needing more MUST use the zero-knowledge path's
nonce commitment (`TE-NON-2`).

== Freshness is a verifier-chosen window

Status attestations have validity windows, and framework practice distributes status lists
decoupled from any single presentation #cite("EUDI-ARF"); a revocation that occurs inside a
window is invisible until the next attestation. #rid("TE-SEC-2") The window tolerance is the
verifier's risk decision: a verifier MUST choose `trustx:requiresValidStatusAt` (and accept
or reject offered window widths) according to its own freshness policy, and MUST NOT treat
this specification as fixing a safe default window.

== SPARQL 1.2 engine gap

The normative response encoding leans on RDF 1.2 reifiers while triple-term #emph[query]
support is not yet deployed (@sec-oracle). Until it is, consumers run #emph[Q′] over the
fixed named-graph mapping; the mapping is normative (`TE-MAP-*`) precisely so this interim
path cannot fork semantics.

== No performance claims

This document contains no measured performance figures by design; performance analysis
belongs to the companion integration paper, which takes numbers only from the project's
canonical evidence pipeline.

= References

#references((
  ("RFC2119", [Bradner, S. #emph[Key words for use in RFCs to Indicate Requirement Levels].
    RFC 2119, IETF, March 1997.]),
  ("RFC8174", [Leiba, B. #emph[Ambiguity of Uppercase vs Lowercase in RFC 2119 Key Words].
    RFC 8174, IETF, May 2017.]),
  ("SPARQL11-QUERY", [Harris, S.; Seaborne, A. (eds). #emph[SPARQL 1.1 Query Language].
    W3C Recommendation, 21 March 2013. https://www.w3.org/TR/sparql11-query/.]),
  ("SPARQL12-QUERY", [Seaborne, A.; et al. (eds). #emph[SPARQL 1.2 Query Language].
    W3C Working Draft (work in progress). https://www.w3.org/TR/sparql12-query/.]),
  ("RDF12-CONCEPTS", [Hartig, O.; Champin, P.-A.; Kellogg, G.; Seaborne, A. (eds).
    #emph[RDF 1.2 Concepts and Abstract Syntax]. W3C Candidate Recommendation (work in
    progress). https://www.w3.org/TR/rdf12-concepts/.]),
  ("PROV-O", [Lebo, T.; Sahoo, S.; McGuinness, D. (eds). #emph[PROV-O: The PROV Ontology].
    W3C Recommendation, 30 April 2013. https://www.w3.org/TR/prov-o/.]),
  ("SHACL", [Knublauch, H.; Kontokostas, D. (eds). #emph[Shapes Constraint Language (SHACL)].
    W3C Recommendation, 20 July 2017. https://www.w3.org/TR/shacl/.]),
  ("ODRL22", [Iannella, R.; Villata, S. (eds). #emph[ODRL Information Model 2.2].
    W3C Recommendation, 15 February 2018. https://www.w3.org/TR/odrl-model/.]),
  ("VC-DATA-MODEL", [Sporny, M.; et al. (eds). #emph[Verifiable Credentials Data Model v2.0].
    W3C Recommendation, 2025. https://www.w3.org/TR/vc-data-model-2.0/.]),
  ("VC-DI", [Sporny, M.; Longley, D.; et al. (eds). #emph[Verifiable Credential Data
    Integrity 1.0]. W3C Recommendation, 2025. https://www.w3.org/TR/vc-data-integrity/.]),
  ("BITSTRING-STATUS", [Sporny, M.; Longley, D.; et al. (eds). #emph[Bitstring Status List
    v1.0]. W3C Recommendation, 2025. https://www.w3.org/TR/vc-bitstring-status-list/.]),
  ("TOKEN-STATUS", [Looker, T.; Bastian, P.; Bormann, C. #emph[Token Status List].
    IETF OAuth Working Group, Internet-Draft (work in progress).
    https://datatracker.ietf.org/doc/draft-ietf-oauth-status-list/.]),
  ("EIDAS2", [#emph[Regulation (EU) 2024/1183 of the European Parliament and of the Council
    amending Regulation (EU) No 910/2014 as regards establishing the European Digital
    Identity Framework]. Official Journal of the European Union, 30 April 2024.]),
  ("EUDI-ARF", [European Commission. #emph[European Digital Identity Wallet Architecture and
    Reference Framework], version 2.4.0.
    https://eudi.dev/2.4.0/architecture-and-reference-framework-main/. (Trusted Lists,
    Registrars, Attestation Rulebooks, and the status-list revocation topic; read
    2026-07-05.)]),
  ("UK-DIATF", [UK Department for Science, Innovation and Technology. #emph[UK Digital
    Identity and Attributes Trust Framework], gamma (0.4) pre-release. GOV.UK. (Five
    certifiable roles; certification per certified service by an independent Conformity
    Assessment Body; the public register of digital identity and attribute services; read
    2026-07-05.)]),
  ("ZKSPARQL", [The sparq project. #emph[zkSPARQL: Zero-Knowledge Query Proofs over SPARQL].
    sparq Unofficial Proposal Draft (sibling document in this series). Research-grade; not
    externally audited (open gate sq-qhy4).]),
  ("MPC-SPARQL", [The sparq project. #emph[MPC-SPARQL: Secure Multi-Party Federated SPARQL —
    Requirements and Reference Architecture]. sparq Unofficial Proposal Draft (sibling
    document in this series).]),
  ("SEC-PROP", [Wright, J.; Shadbolt, N.; Zhao, Jun; Zhao, Rui; Braun, C. #emph[Zero-Knowledge
    Proof of Correct SPARQL Evaluation over Verifiable Credentials]. Paper in submission at
    the time of writing (https://zksparql.org/); the `sec-req:` regulatory-requirement
    sub-vocabulary referenced by `TE-VOC-2` is vendored from it in the sparq repository
    (MIT). Prior work of this document's editor — declared for citation integrity.]),
  ("SPARQ", [The sparq project. #emph[sparq: an RDF + SPARQL engine with a zero-knowledge
    query-proof estate (reference implementation)]. https://github.com/jeswr/sparq.]),
))
