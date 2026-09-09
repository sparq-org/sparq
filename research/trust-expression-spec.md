# Trust-expression specification: the verifier-to-holder contract for framework-anchored attestation queries

<!-- [OPUS-4.8] sq-5reoy (#1599): the `zk/xpath` tree was externalized to the `sparq-org/noir_XPath` (v0.2.0) face repo; its `KNOWN_FAILING` discipline (referenced below) now lives in that repo's CI. Any `zk/xpath/…` path below is a HISTORICAL in-tree reference. -->

Status: **design-for-review decomposition record** (epic `sq-6syab`, issue
[#1592](https://github.com/sparq-org/sparq/issues/1592)). This record frames the
maintainer's trust-expression directive against the built estate, makes the
architecture choices the child beads need, and cuts the program into disjoint
fragments. Nothing here is shipped; the specification itself, the vocabulary
layer, the conformance suite, and the integration paper are the child beads of
`sq-6syab` (§8). Implementation is sequenced after the #1591 fragment-extension
work (`sq-3kd2g`) per the directive.

<!-- [FABLE-5] Fable-tier FRONT decomposition record for sq-6syab. 🤖 SPARQ agent. -->

> **HONESTY FRAMING (load-bearing).** Everything in this record that touches the
> ZK/MPC estate inherits the standing discipline: the sparq ZK verifier is
> remediated and internally re-audited but has **no external accredited-
> cryptographer sign-off** (open gate `sq-qhy4`), and `sparq-mpc` is
> honest-majority **semi-honest only**. No clause of the trust-expression
> specification, and no child bead of this program, may assert a settled
> cryptographic soundness or privacy guarantee. "Framework-anchored trust" as
> designed here bottoms out in a *trust anchor* (the framework operator's
> signed trusted-list/certification attestations), not in cryptography — §7
> states the residual trust assumptions plainly.

## 0. The request (verbatim, #1592)

> Once this is done, as a separate piece of work; I would like you to develop a
> specification that accounts for trust in a way that aligns with current
> thinking around trust frameworks and governance - for instance, in eidas the
> query would effectively be "PROVE THAT X CAN BE ATTESTED BASED ON UNREVOKED
> ATTRIBUTES ISSUED BY PARTIES [X, Y and Z] _or_ BASED ON CERTIFIED ISSUERS
> WITHIN THE EIDAS FRAMEWORK or DIATF FRAMEWORK" noting that the latter also
> requires attestation that issuers have only issued information that they are
> certified / approved to issue. Ideally I would like the specification itself
> to be as _simple_ as possible in defining the contract between verifier and
> holder (where the verifier is asking that question of the holder), whilst
> affording it to work in the context of the ODRL + PROV + MPC + zkSPARQL work
> that you have done. I think that ideally there are two outputs here -- and I
> want your critical review on whether this is the case: the specification with
> support for expressions on trust. And a paper describing how this works with
> the work / framework you have built - including performance analysis and
> security analysis. [...] If reasonable, the spec that you develop really
> should just be pure SPARQL (perhaps SPARQL 1.2), using ontologies for
> describing trusted sources and reification for appropriately describing
> provenance in the response (again I have not thought this through
> particularly deeply, I am not attached to use of SPARQL 1.2 or reification -
> I just want to make sure this makes use of existing RDF technology
> appropriately).

Agreed constraints already recorded on #1592: non-revocation must be modelled
as a **positive attestation** (OWA/monotonicity); issuer-certification scope
needs a vocabulary layer; the program must **compose with** the ODRL + PROV +
MPC + zkSPARQL estate rather than duplicate it; implementation beads are
dep-linked after the #1591 fragment extension; the conformance suite is a
first-class output; the *pure* zk-architecture paper is tracked under #1591,
not here.

## 1. Premise check — what the estate already has (verified, not assumed)

This area is heavily built. The specification must be a thin normative layer
over these verified surfaces, not a re-derivation:

| Estate surface | Verified state | Evidence |
|---|---|---|
| Per-(source, statement-type) trust vocabulary (`trust:`, ten-term core + `forPredicate` sugar) | **Merged** | `crates/sparq-trust/ontologies/trust/trust.ttl`, byte-pinned to `src/vocab.rs` (`desugar_for_predicate`, sync test) |
| Security-properties ontology + assurance axis (`sec-prop:` vendored, `secx:` extension, `Claimed`/`ExternalSignOffPending`) | **Merged** | `crates/sparq-trust/ontologies/zkp-sparql/` (vendored, ISWC 2025 companion); `secprop-ext.ttl` + the IRI constants in `crates/sparq-secprop-vocab/` (#3705); design `research/security-properties-ontology-design.md` |
| Regulatory-framework instances — **eIDAS 2.0, UK DVS/DIATF, NIST PQC — already exist as `sec-req:Requirement` individuals** | **Merged (vendored)** | `crates/sparq-trust/ontologies/zkp-sparql/vocab/sec-req.yaml.ld` |
| ODRL-driven proof-method admissibility (N3 ruleset + `admissible()`, ODRL leftOperand profile) | **Merged** | `crates/sparq-trust/src/admissibility.rs`, epic `sq-0dksu` Phases 1–5 |
| Positive status attestation, clear path: signed Bitstring status lists with freshness window + RDF justification triples | **Merged** | `crates/sparq-trust/src/status_list.rs` (`SignedStatusList`, `VerifyingLiveStatusCheck`, `justify_status_decision`) |
| Non-revocation in ZK: committed-index bit-unset proof at an authoritative snapshot; hidden-index variant designed | **Merged / designed** | `zk/compose/compose_core/src/revoke.nr`, `crates/sparq-zk-compose/src/revocation.rs`; `research/zk-statuslist-hide-iri-version.md` |
| Issuer-attestation binding, multi-attestation, plus hidden-issuer **set membership over a trusted key set** | **Merged** | `crates/sparq-zk-compose/src/verifier.rs` (`bind_issuer_attestations`, `bind_hidden_issuer_attestations`); ledger in `research/mpc-zkp-federated-sparql-design.md` §3 |
| zkSPARQL spec draft (verifier↔holder challenge-response, trust anchors, manifest, fail-closed obligations) | **Drafted, held for review** | `site/specs/zksparql.typ` (PR #1509, bead `sq-vvu9d`); fragment extension program `sq-3kd2g` (#1591) |
| Specs + papers factories (Typst → PDF/HTML, honesty gates fail-closed) | **Merged** | `site/src/data/specs.ts`, `site/scripts/build-specs.mjs`; `research/specs/specs-infra-plan.md` |
| RDF 1.2 triple terms | **Parsing merged; SPARQL surface NOT built** | `crates/sparq-core/src/nt.rs` (`triple_term`, object-position, depth-bounded); no `<<( … )>>` support in `crates/sparq-parse` (grep-verified 2026-07-05); design records `research/rdf12-parser.md`, `research/rdf12-indexing.md` |

**What is genuinely missing** (the program's real delta): (i) a
verifier-to-holder *contract* — today the trust requirements are implicit in
verifier-side Rust configuration (`RevocationPolicy`, trusted key sets), not an
interchangeable RDF artifact; (ii) the **framework / certification-scope
vocabulary layer** ("certified issuers within eIDAS/DIATF", "issued only what
they are certified to issue") — `sec-req:` names the frameworks but has no
notion of an issuer being *certified under* one, nor of a certification
*scope*; (iii) a normative **provenance-in-response encoding**; (iv) a
**conformance suite**; (v) the **integration paper**.

## 2. Bounded external survey — what eIDAS 2 / EUDI ARF and DIATF actually pin

Read 2026-07-05; cited so the vocabulary layer models real framework
mechanics rather than a caricature.

- **EUDI ARF 2.4.0** ([eudi.dev/2.4.0/architecture-and-reference-framework-main](https://eudi.dev/2.4.0/architecture-and-reference-framework-main/)):
  issuer trust is established by **Trusted Lists** — PID/QEAA/PuB-EAA
  providers are registered by a **Registrar** (§3.17) and notified to the
  Commission for inclusion (§3.5); attribute schemas are published via
  **Attestation Rulebooks** (§5.4); a Relying Party registers *which
  attributes it intends to request*, and the wallet checks it does not
  request more than registered (§6.6.3.3); the RP must verify the attestation
  "is not revoked" (§6.6.3.7). Revocation status is published via **status
  lists / identifier lists** (IETF Token Status List is the mechanism tracked
  by the reference implementation; providers choose the method, wallets and
  relying parties must support both; RPs are advised to fetch each list
  version once, decoupled from any presentation — see the ARF revocation
  topic, [eudi.dev/1.4.0/arf](https://eudi.dev/1.4.0/arf/) Topic 7 and
  [eudi-doc-standards issue #11](https://github.com/eu-digital-identity-wallet/eudi-doc-standards-and-technical-specifications/issues/11)).
  Legal base: Regulation (EU) 2024/1183 (already cited by the vendored
  `sec-req:` eIDAS instance).
- **UK DIATF (gamma 0.4)** ([GOV.UK pre-release](https://www.gov.uk/government/publications/uk-digital-identity-and-attributes-trust-framework-04/uk-digital-identity-and-attributes-trust-framework-gamma-04-pre-release)):
  five certifiable roles (identity / attribute / holder / orchestration /
  component service providers); certification is by an independent
  Conformity Assessment Body, valid three years, and applies **per certified
  service, not per attribute type**; certified providers may appear on the
  public **register of digital identity and attribute services**; attribute
  service providers must reliably link attributes and assess/share attribute
  quality on request.

**Two load-bearing survey findings for the vocabulary design:**

1. **Certification scope granularity differs by framework.** eIDAS gives
   schema-level scoping hooks (Attestation Rulebooks, the attribute
   catalogue); DIATF certifies a *service*, not an attribute list. So
   `trustx:certificationScope` must range from "everything this certified service issues"
   (service-level, the DIATF reality) down to a specific attestation type or
   attribute-predicate set (the eIDAS Rulebook reality). A spec that hardwires
   attribute-level scope would misdescribe DIATF; one that omits scope cannot
   express the maintainer's "issued only what they are certified to issue".
2. **Both frameworks realize "trusted issuer" as a *published positive
   artifact*** (a Trusted List entry / a register entry), and revocation as a
   *published status artifact* (status list) — exactly the
   positive-attestation shape the OWA constraint requires. The vocabulary can
   therefore model framework membership and non-revocation the same way:
   time-windowed signed attestations by an authority, never
   evidence-of-absence.

## 3. The contract — design decisions

### 3.1 D1: the contract is ONE SPARQL query + ONE RDF trust-requirements document [FABLE-5]

The verifier sends the holder exactly three things: a **SPARQL query** `Q`
(ASK or SELECT; SPARQL 1.1-evaluable, SPARQL 1.2 surface where triple terms
are matched), a **trust-requirements document** `TR` (a small RDF graph in the
trust-expression vocabulary), and a **nonce** (reusing the zkSPARQL
challenge-response verbatim — no new freshness mechanism). The holder returns
a **response dataset** `R`: the query answer plus, for every statement that
contributed to it, machine-readable provenance (§4) sufficient for the
verifier to re-check admissibility under `TR` — and, on the ZK path, a
zkSPARQL proof manifest whose trust anchors are *derived from* `TR`.

There is **no new query syntax**. The trust conditions live in `TR`, not in
`Q`. The normative semantics of "evaluate `Q` under `TR`" is defined by a
**reference rewrite**: `Q` over the trust-scoped dataset ≜ a plain SPARQL
query `Q'` over the provenance-encoded response form, where `Q'` conjoins
`Q`'s patterns with the admissibility patterns generated from `TR` (issuer
membership, status-attestation validity at time *t*, scope conformance). This
is what makes the spec "pure SPARQL": the contract's meaning is *checkable by
any conformant SPARQL engine* running `Q'` over `R` — which is also the
conformance suite's oracle (§6). The maintainer's example question becomes,
literally, an ASK.

Rejected alternatives: new SPARQL keywords / magic `SERVICE` IRIs (not pure
SPARQL; every engine forks); embedding trust conditions inside `Q` by hand
(pushes the hard part onto every verifier and makes trust-requirements documents non-reusable);
a bespoke JSON request object (duplicates what RDF already does — the
trust-requirements document should itself be data the estate's reasoners can consume).

### 3.2 D2: two trust modes, one trust-requirements shape [FABLE-5]

Mode 1 — **enumerated parties**: `TR` lists issuer identities (DID/key
binding reuses `sq-pfae.3`'s did:key/did:web work):

```turtle
[] a trustx:TrustRequirements ;
   trustx:question   <urn:q1> ;             # names the question TR was authored for (opaque label — §7.7)
   trustx:trustsIssuer <did:web:x.example>, <did:web:y.example>, <did:web:z.example> ;
   trustx:requiresValidStatusAt "2026-07-05T00:00:00Z"^^xsd:dateTime .
```

Mode 2 — **framework-certified issuers**: `TR` names a framework and requires
scope conformance; the two modes compose with plain OR (two trust-requirements documents, or one
document with both, per the maintainer's "OR" phrasing):

```turtle
[] a trustx:TrustRequirements ;
   trustx:question <urn:q1> ;
   trustx:trustsFramework trustx:eIDAS2 ;    # rdfs:seeAlso the sec-req: individual
   trustx:requiresScopeConformance true ;
   trustx:requiresValidStatusAt "2026-07-05T00:00:00Z"^^xsd:dateTime .
```

### 3.3 D3: non-revocation is a positive, time-windowed status attestation

Per the agreed #1592 constraint, "unrevoked" is never queried as absence.
`trustx:StatusAttestation` is a signed statement by the status authority that
credential *c*'s status was `valid` in a window `[validFrom, validUntil]` —
realized on the clear path by the already-merged signed Bitstring status list
machinery (`SignedStatusList` + `VerifyingLiveStatusCheck` +
`justify_status_decision`, which already emits justification *triples*), and
on the ZK path by the already-merged committed-index bit-unset proof at an
authoritative snapshot. IETF Token Status List (the eIDAS ARF mechanism) maps
to the same shape. The reference rewrite (§3.1) asks for the **existence** of
a covering status attestation — monotone under OWA; a revoked or
stale-windowed credential simply yields no admissible binding (fail-closed by
construction, not by negation).

### 3.4 D4: the certification-scope vocabulary layer [FABLE-5]

New terms (working prefix `trustx:`, minted as an extension file in the same
placeholder namespace and directory as `trust:` —
`crates/sparq-trust/ontologies/trust/trust-framework.ttl` — with the same
"NON-STANDARD, a WG would rehome" honesty banner, byte-pinned to Rust
constants like `trust.ttl`/`vocab.rs` and `secprop-ext.ttl`/`secprop.rs`):

- `trustx:Framework` — a governance framework operating a
  trusted-list/register. Individuals `trustx:eIDAS2`, `trustx:DIATF` carry
  `rdfs:seeAlso` to the **existing vendored `sec-req:` individuals** (eIDAS
  2.0, UK DVS) rather than duplicating them; `sec-req:` stays the
  regulatory-requirement view, `trustx:` adds only the certification
  mechanics it lacks.
- `trustx:Certification` — a time-windowed, authority-signed attestation
  that an issuer is certified under a framework: `trustx:certifies` (issuer),
  `trustx:underFramework`, `trustx:certificationScope`, `trustx:validFrom`/`validUntil`,
  plus the signature/attestation binding. A Trusted-List or DIATF-register
  entry *is* a `trustx:Certification` in this model.
- `trustx:certificationScope` — what the issuer is certified to issue. Ranges over (i)
  `trustx:AnyServiceScope` (service-level — the honest DIATF granularity),
  (ii) an attestation type / Rulebook IRI, (iii) a predicate set or SHACL
  shape (reusing `trust:`'s existing `forPredicate` → `forShape` desugaring
  pattern rather than inventing a second scoping idiom). Named
  `certificationScope`, **not** `scope`: `trustx:` is a prose-only sub-prefix
  over `trust:`'s *own* base IRI, so a `trustx:scope` term would literally be
  `trust:scope` (rule applicability, `rdfs:domain trust:TrustRule`) — one
  property carrying two conflicting domains rather than two homonyms
  (issue #3801). Every `trustx:` local name must stay disjoint from
  `trust.ttl`'s for the shared-base choice to remain sound.
- `trustx:validFrom` / `trustx:validUntil` carry the validity window of a
  `trustx:Certification` **and** of a `trustx:StatusAttestation`, so their
  `rdfs:domain` is the *union* of those two classes. A bare
  `rdfs:domain trustx:Certification` would, under RDFS entailment, type every
  status attestation as a certification and oblige it to satisfy the
  certification's `certifies` / `underFramework` / `certificationScope`
  structure (issue #3801).
- "**Issuers only issued what they are certified to issue**" is then a
  *scope-conformance check*: every contributing attested statement's
  type/predicate falls under its issuer's `trustx:Certification.scope` valid
  at the evaluation time. Honest status: this is a check against the
  framework's **published certification attestations** — a trust-anchored
  delegation claim, not a cryptographic guarantee about everything the issuer
  ever did (§7.2).

### 3.5 D5: composition with the estate (reference, don't duplicate)

- **ODRL / secprop** (`sq-0dksu`): orthogonal axes — the secprop/ODRL
  admissibility pre-check decides which *proof methods* are acceptable; the
  trust requirements decide which *sources* are. `TR` may carry an optional
  `trustx:methodPolicy` pointing at an ODRL policy consumed by the existing
  `admissible()` path; the spec normatively references, and does not restate,
  that machinery.
- **PROV**: the response encoding (§4) uses PROV-O qualification terms on
  reifiers; citation chains reuse the vendored `prov-ext:` pattern.
- **MPC** (attested-source derivation): in the federated case the
  trust-requirements document is precisely the generator of the trusted key set `K` that
  `bind_issuer_attestations` / the M4-v1 verifier-side attestation gate
  (`sq-f7bu`, gated `sq-34ml`) consume — mode 1 enumerates `K`; mode 2
  derives `K` from framework certifications. No new MPC surface is designed
  here; MPC remains semi-honest, stated wherever mentioned.
- **zkSPARQL**: the ZK realization of the contract is a zkSPARQL manifest
  whose trust anchors are derived from `TR`. Already-merged pieces map
  directly: issuer binding (mode 1), hidden-issuer set membership over `K`
  (mode 2 with issuer privacy), committed-index non-revocation (§3.3). The
  genuinely NEW zk work is **certification-scope binding** (proving the
  disclosed/derived claims fall inside a certified scope without disclosing
  more) — bead `sq-6syab.5`, honestly flagged as unbuilt and
  soundness-sensitive.

## 4. Provenance-in-response: options and ONE recommendation

The response must let the verifier see, per contributing statement: who
issued it, under which certification/framework (mode 2), and which status
attestation covered it. Options considered (the maintainer is explicitly not
attached to reification):

| Option | Mechanism | For | Against |
|---|---|---|---|
| (a) **RDF 1.2 triple terms + `rdf:reifies` reifier, PROV-O on the reifier** | `_:r rdf:reifies <<( :jesse :age 25 )>> ; prov:wasAttributedTo <did:web:x…> ; trustx:coveredBy _:status .` | Standards-track (RDF 1.2 Concepts at CR; triple terms are object-position-only, the reifier is the natural PROV qualification subject — `research/rdf12-parser.md`); ONE self-contained graph (no dataset semantics needed); sparq already parses triple terms (`sparq-core::nt`); matches the vendored `sig-impl:` reified-Assertion precedent | SPARQL-1.2 triple-term *query* surface not yet in sparq (`sparq-parse` has none) or in most deployed engines; tooling maturity |
| (b) Named graphs + PROV-O qualified attribution (one graph per attestation bundle) | graph IRI as the qualification subject | Works in every SPARQL 1.1 engine today; matches sparq-solid's graph-per-resource reality | Response becomes a *dataset* (TriG, not Turtle); RDF semantics of named graphs are unspecified (each consumer re-invents them); graph-per-statement proliferates; awkward nesting for status-attestations-about-certifications |
| (c) Classic `rdf:Statement` reification | — | universal | ×4 triple blow-up; no term identity; superseded by RDF 1.2's design; rejected |
| (d) Singleton properties / n-ary restructuring | — | no new syntax | non-standard idiom, destroys vocabulary reuse (queries no longer match the plain data shape); rejected |

**Recommendation (ONE): (a) — RDF 1.2 reifiers as the normative response
encoding, with (b) specified as an informative, mechanically lossless
mapping** for SPARQL 1.1 consumers (reifier node ↔ graph IRI; the spec fixes
the bidirectional mapping so implementations can down-convert). Rationale:
the response is a single self-contained document, which is (a)'s sweet spot
and (b)'s weak spot; the reifier is exactly what PROV-O wants to qualify; it
is the RDF 1.2-native answer to the maintainer's "perhaps SPARQL 1.2 …
reification" instinct without the legacy `rdf:Statement` costs; and it makes
the §3.1 reference rewrite a *SPARQL 1.2* query, aligning the spec with where
the standards are going rather than where they have been. The honest cost is
named, not hidden: sparq's engine cannot yet *match* triple terms, so until
the SPARQL 1.2 surface lands the conformance runner checks the encoding
structurally via `sparq-core` parsing plus the (b)-mapping, and the
`Q'`-over-`R` oracle runs over the (b) form. **Steer point:** if the
maintainer prefers (b) as normative, only §response-encoding and fixtures
change — the vocabulary and contract are encoding-neutral by construction.

## 5. Critical review of the "two outputs" question

The maintainer asked whether spec + integration paper is the right output
split. Verdict (already posted in-session on #1592, reaffirmed here after
grounding): **three outputs, not two** — (1) the specification (with the
trusted-sources/framework ontology *normative inside it*, following the
secprop-extends-`sec-prop:` pattern); (2) the integration paper (perf +
security analysis under the `sq-qhy4` discipline); (3) the **conformance
suite**, mandatory per the maintainer's standing rule for specs in his
namespace (#1546: contribute the tests upstream, sparq passes all). The
*pure* zk-architecture paper is correctly NOT in this program (it belongs
to issue #1591 / `sq-3kd2g` and should land first, so the integration paper
analyzes only the trust delta). No fourth output is warranted: a separate "trust
protocol" document would duplicate the spec, and the vocabulary should not be
split from the contract it serves.

## 6. Conformance suite design

W3C-style manifest (`mf:`-vocabulary) plus fixture graphs and expected
outcomes under `crates/sparq-trust/tests/trust-expression/`, structured so
the whole directory can be lifted verbatim into the spec's own upstream
repository once the maintainer picks its home (open question §9.3). Case
classes, all derived from §3's semantics, every negative case fail-closed:

1. Mode 1 pass: all contributing statements issued by enumerated parties,
   covering status attestations valid at *t*.
2. Revoked: status bit set at the authoritative snapshot → no admissible
   binding (answer absent, never "false because revoked").
3. Stale window: status attestation exists but does not cover *t* → reject.
4. Untrusted issuer: signature valid, issuer not in `TR` → reject.
5. Mode 2 pass: issuer certified under the framework, scope-conformant.
6. Scope violation: certified issuer, contributing statement outside
   `trustx:certificationScope` → reject (the "only issued what certified to issue" case).
7. Certification expired/revoked at *t* → reject (certifications are status-
   checked exactly like credentials — same positive-attestation machinery).
8. Encoding: response provenance round-trips (a) ↔ (b) losslessly.

Zero `KNOWN_FAILING` entries without a beaded reason per case (the zk/xpath
conformance-honesty discipline).

## 7. Honest limitations (carried into the spec's Security Considerations)

1. **No externally audited cryptography.** The ZK path's soundness rests on
   internal re-audit only (`sq-qhy4` open); MPC is semi-honest. The spec's
   conformance clauses are phrased as "matches this specification /
   fail-closed", never "sound" or "private" without the pending-audit caveat.
2. **Framework trust is anchored, not proven.** Mode 2 bottoms out in the
   framework operator's signed certification/trusted-list artifacts. The
   scope-conformance check constrains what the *verifier accepts*, and — via
   the published certification — what the issuer was *authorized* to issue;
   it cannot retroactively prove an issuer never mis-issued elsewhere.
3. **Clear path trusts the holder's evaluation.** Without the ZK manifest,
   the verifier re-checks admissibility over `R` (the §3.1 rewrite) but must
   trust the underlying attestations' signatures — which it verifies — and
   the completeness of what the holder chose to disclose (inherent to OWA;
   the contract only ever asks monotone existence questions).
4. **Freshness/caching trade-off.** Status attestations have validity
   windows (the ARF itself advises fetch-once distribution); a revocation
   inside the window is invisible until the next status attestation. The
   window is a verifier-chosen parameter in `TR`, not a spec constant.
5. **SPARQL 1.2 engine gap.** The normative encoding leans on RDF 1.2
   reifiers; sparq parses triple terms but cannot query them yet. Named in
   §4; the (b)-mapping keeps every conformance case runnable today.
6. No performance numbers appear in this record or the spec; the integration
   paper takes numbers only from the canonical evidence pipeline (work-box
   timings are non-canonical).
7. **`trustx:question` is a label, not an enforced binding.** The clear path
   checks that `TR` names exactly one question IRI but defines no canonical
   resolution or digest scheme tying that IRI to a query string, so it never
   verifies that `Q` *is* the named question — a `TR` authored for question A
   paired with an unrelated supported query B parses and evaluates. The
   question↔query association belongs to whoever authenticates the request
   (e.g. a signature over `(Q, TR, nonce)` or a trusted question publication),
   exactly like the `MethodPrecheck` policy resolution. An authoritative
   in-band binding (a digest-IRI question scheme over a canonical query form)
   is future work.

## 8. Decomposition — child beads of `sq-6syab`

Disjointness: no two beads touch the same file. `site/` carries two beads
(spec, paper) and `crates/sparq-trust` carries four; every same-surface pair
is sequenced by a REAL dependency edge (marked NON-parallel below), so the
parallel frontier never co-schedules two beads on one surface. Implementation
beads are dep-linked on `sq-3kd2g` (#1591 fragment extension) per the
directive; spec/vocabulary/fixture authoring proceeds now.

| Bead | Fragment | Surface / files | Tier | Invariant | Acceptance test |
|---|---|---|---|---|---|
| `sq-6syab.1` | Spec document: `site/specs/trust-expression.typ` + `specs.ts` entry — contract, two modes, positive non-revocation, scope layer, normative (a) + informative (b) encodings, Security Considerations §7 | `site/specs/trust-expression.typ`, `site/src/data/specs.ts` | opus | privacy-claims gate passes; every estate reference cites the crate item; no unqualified soundness claim | `cd site && npm run build` (build-specs.mjs honesty gates) + `node --test scripts/specs.test.mjs` |
| `sq-6syab.2` | Vocabulary layer: `trust-framework.ttl` (`trustx:` terms §3.4) + Rust constants module + ttl↔Rust sync test | `crates/sparq-trust/ontologies/trust/trust-framework.ttl`, `crates/sparq-trust/src/framework_vocab.rs`, `src/lib.rs` (mod line) | sonnet | extends `trust:`/`sec-req:` (no fork, no duplicate IRIs); every term carries the NON-STANDARD banner; ttl byte-pinned to constants | `cargo test -p sparq-trust framework_vocab` |
| `sq-6syab.3` | Conformance suite data: manifest + fixtures + expected outcomes (§6 cases 1–8) + fixture well-formedness test | `crates/sparq-trust/tests/trust-expression/**`, `crates/sparq-trust/tests/trust_expression_fixtures.rs` | sonnet | every negative case encodes fail-closed rejection; zero unbeaded KNOWN_FAILING | `cargo test -p sparq-trust --test trust_expression_fixtures` |
| `sq-6syab.4` | Holder-side contract evaluation (clear path): trust-requirements parsing, §3.1 reference rewrite, scoped evaluation via sparq-engine, provenance-encoded response assembly | `crates/sparq-trust/src/expression.rs` (new module only) | opus | fail-closed: no admissible derivation ⇒ no binding; response provenance sufficient for independent verifier re-check (`Q'` over `R`) | `cargo test -p sparq-trust expression` |
| `sq-6syab.5` | ZK composition: derive zkSPARQL trust anchors from `TR`; certification-scope binding design+impl on the manifest/verifier path (mode 2 in zero knowledge) | `crates/sparq-zk-compose/src/**` (single-crate bead) | opus (**maintainer-arm — ZK-soundness-sensitive**) | fail-closed verifier obligations; spec-conformance phrasing only — NO soundness/privacy claim while `sq-qhy4` open | `cargo test -p sparq-zk-compose` (incl. new forge gates for scope binding) |
| `sq-6syab.6` | Conformance runner (semantics): drive `sq-6syab.4`'s API over the `sq-6syab.3` manifest; wire as the suite sparq must fully pass (maintainer-namespace rule) | `crates/sparq-trust/tests/trust_expression_conformance.rs` (single file) | sonnet | ALL manifest cases pass; zero unbeaded KNOWN_FAILING | `cargo test -p sparq-trust --test trust_expression_conformance` |
| `sq-6syab.7` | Integration paper: how the spec composes with ODRL + PROV + MPC + zkSPARQL, perf analysis (canonical evidence only) + security analysis under `sq-qhy4` discipline | `site/papers/trust-expression-integration.typ`, `site/src/data/papers.ts` | opus | paper-factory honesty gates pass; no non-canonical number; audit-pending caveat prominent | `cd site && npm run build` (build-papers.mjs honesty gate) |

Dependency edges (all REAL orderings; same-surface pairs NON-parallel):

- `sq-6syab.1` ← `sq-6syab.3`, `sq-6syab.7` (fixtures and paper follow the spec text)
- `sq-6syab.2` ← `sq-6syab.3`, `sq-6syab.4`, `sq-6syab.5` (everything consumes the vocabulary)
- `sq-6syab.3`, `sq-6syab.4` ← `sq-6syab.6` (runner needs both the data and the API)
- `sq-3kd2g` (#1591) ← `sq-6syab.4`, `sq-6syab.5`, `sq-6syab.7` (build-later per
  the directive; the paper additionally wants #1591's pure zk-architecture paper
  landed so it analyzes only the trust delta). Tooling note: `bd` rejects
  epic→task dependency edges, so this gate is carried as the
  `blocked-by-epic:sq-3kd2g` label + a bead note on all three; the orchestrator
  wires concrete edges once the #1591 architect creates `sq-3kd2g`'s children.

Immediately dispatchable: `sq-6syab.1` and `sq-6syab.2` (disjoint surfaces,
parallel). `sq-6syab.3` unlocks when both land. The judgment call that the
conformance *data* (`.3`) is a spec artifact — authored now, not gated on
`sq-3kd2g` — while the runner (`.6`) waits for the gated evaluation API, is a
deliberate reading of "design now, build later"; flagged for steer in §9.

## 9. Open questions (proceed-and-document: defaults chosen, steer welcome)

1. **Normative encoding** — proceeding with RDF 1.2 reifiers (§4). Flipping
   to named-graphs-normative changes only the encoding section + fixtures.
2. **Conformance-data timing** — proceeding with fixtures authored pre-
   `sq-3kd2g` (§8). If "build later" was meant to cover test data too,
   `sq-6syab.3` gains a dep edge and nothing else changes.
3. **Upstream home + namespace** — the spec drafts as a sparq Unofficial
   Proposal Draft (`site/specs/`) with placeholder `trustx:` IRIs beside
   `trust:`. When the maintainer mints a w3id namespace / standalone repo for
   it, the vocabulary rehomes and the conformance directory lifts upstream
   (per the #1546 standing rule). Until then the suite lives in-repo.
4. **Framework individuals** — proceeding with thin `trustx:` individuals
   that `rdfs:seeAlso` the vendored `sec-req:` ones (no duplication, no edit
   to vendored files). The alternative — extending `sec-req:` in place, as
   `secx:` did for `sec-prop:` — is workable if preferred.
