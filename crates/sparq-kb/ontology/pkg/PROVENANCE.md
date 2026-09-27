# PROVENANCE — the PKG (`pkg:`) ontology

> 🤖 **SPARQ agent** [OPUS-4.8] — reuse + alignment-verification record for the
> Project-Knowledge-Graph ontology. sq-2m6zm.1 (epic sq-2m6zm); design record
> `research/dogfooding-sparq-knowledge-graph.md` (PR #1063). Written while Fable
> unavailable; flag for re-review when Fable returns.

## What this is

`pkg.ttl` is the machine-readable form of the **reuse-first** Project-Knowledge-Graph
vocabulary of `research/dogfooding-sparq-knowledge-graph.md` §2.3, with `pkg.shapes.ttl`
(in `../../shapes/`) the SHACL **write-time guardrails** of §2.4 + §4.4. The
design principle is copied verbatim from the maintainer's own `sec-prop` discipline
(`research/security-properties-ontology-design.md`): **mint almost no net-new
terms.** PROV-O carries who/what/when/derived-from; SKOS carries concepts/topics;
DCAT + FaBiO/FRBR + DC carry the source catalog; **DQV models the quality axis**
(confidence/assurance as named, distinct measurements/metrics/dimensions); CiTO
carries the citation **and technique-relation** edges; schema.org fills gaps;
nanopublications package an assertion + its provenance; and the vendored `zkp-sparql`
`sig-impl:Assertion` pattern is **generalised** (not forked) into `pkg:Finding`.

## Net-new vs reused

**Only four bespoke predicates/classes are genuinely net-new**, plus the single
task-dependency inverse pair and the bd/source/technique scaffolding the design names.
The DQV adoption (sq-2489d.3) adds **no** net-new bespoke predicate — its
dimension/metric individuals are INSTANCES of the standard DQV classes (see the *DQV
quality model* sub-section below), and the technique-relation graph leans on the
established CiTO terms `cito:extends` / `cito:usesMethodIn` instead of minting bespoke
method-relation predicates, so the bespoke surface **shrinks, it does not grow**:

| net-new term | why it cannot be reused |
|---|---|
| `pkg:exploredStatus` | no FAIR-friendly DCAT/DC term carries the *explored-vs-unexplored* role for project knowledge (so follow-up can target the un-explored). Aligned to `schema:ActionStatusType` via `skos:closeMatch`. |
| `pkg:followUpPriority` | no term carries a *targeted-follow-up ordering* over un-explored sources. |
| `pkg:confidence` | no 0..1 numeric *confidence/epistemic-weight* literal exists in `schema:` / DPV / DC; minted as a bounded `xsd:decimal`, aligned to `schema:Rating`. Orthogonal to `pkg:assurance` (the enum epistemic-basis). |
| `pkg:couldBeMergedWith` | a forward-looking *merge-candidacy* hint with no established precedent; `skos:related` is weaker and not symmetric-by-design. Minted as an `owl:SymmetricProperty`. |
| `pkg:dependsOn` / `pkg:blockedBy` | the **single** task-dependency `owl:inverseOf` pair. There is deliberately **NO** `pkg:blocks`: bd's `blocks` edge is the *inverse-of-`pkg:dependsOn`* direction, modelled as `pkg:blockedBy`. Every shape / query names only these two predicates so a constraint cannot target an undefined property (design §2.2, must_fix-corrected). |

The rest reuses external vocabulary:

| PKG term | reuses (verified IRI) |
|---|---|
| `pkg:Source` | `fabio:Expression` + `dcat:CatalogRecord` + `dcterms:*` + `bibo:doi` + `frbr:realizationOf` |
| `pkg:Document` | `fabio:Expression` |
| topics / concepts | `skos:Concept` + `skos:inScheme` + `dcterms:subject` (no mint) |
| `pkg:Finding` | `rdfs:subClassOf sig-impl:Assertion , skos:Concept` |
| `pkg:about` | `rdfs:subPropertyOf dcterms:subject` |
| `pkg:verdict` | `rdfs:subPropertyOf sig-impl:verdict`; values `sig-impl:yes/no/partial` |
| `pkg:confidence` (the value) | `rdfs:subPropertyOf dqv:value`; the `dqv:value` of a named `dqv:QualityMeasurement` (kept as a convenience shorthand — see *DQV quality model* below) |
| `pkg:assurance` | values `secx:Proven` ⊐ `secx:Claimed` ⊐ `secx:Conjectured` (see *dependency* below); also a `dqv:Metric` in the `pkg:EpistemicBasisDimension` |
| quality measurements | `dqv:QualityMeasurement` + `dqv:isMeasurementOf` + `dqv:computedOn` + `dqv:value` + `dqv:inDimension` + `dqv:hasQualityMeasurement` |
| supporting / refuting | `cito:supports` / `cito:citesAsEvidence` / `cito:disagreesWith` |
| technique extends / uses-method-in | `cito:extends` / `cito:usesMethodIn` (sq-2489d.3 — replaces a bespoke method-relation predicate) |
| provenance | `prov:wasDerivedFrom` / `wasGeneratedBy` / `wasAttributedTo` / `generatedAtTime` |
| `pkg:discoveredFrom` | `rdfs:subPropertyOf prov:wasDerivedFrom` |
| nanopub packaging | `np:hasAssertion` / `hasProvenance` / `hasPublicationInfo` |
| supersedes | `dcterms:replaces` / `dcterms:isReplacedBy` |
| alternative-to | `skos:related` |
| `pkg:implementedBy` | `schema:SoftwareSourceCode` (+ implementing PR as `prov:Activity`) |
| `pkg:Task` | `rdfs:subClassOf schema:Action` |
| parent-child | `dcterms:isPartOf` |

### DQV quality model (sq-2489d.3) — instances, not net-new predicates

The DQV adoption introduces four NAMED `pkg:`-namespaced individuals — but they are
**instances of the standard DQV classes**, exactly as `pkg:Open` / `pkg:Unexplored`
are `skos:Concept` instances (and they are pinned in `vocab.rs` the same way). They mint
no new bespoke predicate:

| named individual | DQV class (reused) | role |
|---|---|---|
| `pkg:EpistemicWeightDimension` | `dqv:Dimension` | the numeric 0..1 confidence axis (Finding belief / Source reliability) |
| `pkg:EpistemicBasisDimension` | `dqv:Dimension` | the categorical assurance axis (proven/claimed/conjectured) — `pkg:assurance` is a `dqv:Metric` in it |
| `pkg:ConfidenceMeasurement` | `dqv:Metric` | the metric a Finding's `pkg:confidence` measures |
| `pkg:SourceReliabilityMeasurement` | `dqv:Metric` | the metric a Source's `pkg:confidence` measures |
| `pkg:BatchQualityDimension` | `dqv:Dimension` | the RUN-level axis (sq-2489d.5): how good was one ingestion batch |
| `pkg:GroundingRateMetric` | `dqv:Metric` | a batch's citation-grounding rate (grounded / candidate Findings) |
| `pkg:SourceYieldRateMetric` | `dqv:Metric` | the fraction of a batch's sources that yielded ≥1 grounded Finding |

A subject's `pkg:confidence` is kept as a convenience shorthand (every canned query
reads it directly, no regression) and is **also** expressible as the `dqv:value` of a
reified `dqv:QualityMeasurement` that `dqv:isMeasurementOf` one of these metrics,
`dqv:computedOn` the subject (the `finding-quality-dqv` canned query surfaces them; the
example file demonstrates both forms agreeing). DQV carries
`prov:wasGeneratedBy`/`wasAttributedTo` natively, so a measurement composes with
`sparq-prov` for free, and `pkg:Source rdfs:subClassOf dcat:CatalogRecord` (DQV's
anchor) was already paid.

The three **batch-quality** individuals (sq-2489d.5, design §4.5) reuse the same shape one
level up: the subject is the ingestion *run* (a `prov:Activity`) rather than a Finding, so
"how good was this batch?" is a query (`batch-quality`) that can feed a per-topic
recommend-adopt decision instead of living only in a run log. Only metrics the pipeline
**actually computes** are declared — the design also names extraction-precision-on-sample
(needs a human-audited sample, Phase 6), dedup-collision rate and topic-coverage, and
those are deliberately absent rather than declared-and-unpopulated, because an unmeasured
metric IRI in the vocabulary reads as a capability the code does not have. Both declared
metrics are **structural** (anchoring + targeting); like the SHACL gate, neither measures
whether a machine-extracted Finding is *true*.

**Honest caveats (the design's §3 caveats, recorded here):** DQV is a W3C **Working
Group Note** (2016), *not* a Recommendation — lower normative weight than
PROV-O/SKOS/DCAT, which matters for the project's verified-stable-namespace discipline
(open question for the maintainer in the design record §6.1: adopt as the model, or only
`skos:closeMatch`-align?). And DQV expresses a *measurement*; it does **not** give
confidence **propagation through joins** — the Hartig / tSPARQL gap is real and
orthogonal, and is NOT claimed here.

## Live-ontology alignment verification (design §2.5 requirement)

The design §2.5 flags that the `skos:closeMatch` alignments were cited from
knowledge of the SPAR/W3C-community vocabularies and **must be checked against the
live published ontology before shipping**. Each was verified against its live source
on **2026-06-21**, and the DQV/CiTO terms of sq-2489d.3 on **2026-06-22**:

| alignment used in `pkg.ttl` | live-ontology check | result |
|---|---|---|
| `schema:PotentialActionStatus`, `schema:ActiveActionStatus`, `schema:CompletedActionStatus`, `schema:FailedActionStatus` | `https://schema.org/ActionStatusType` | **confirmed** — all four are `ActionStatusType` enumeration members. |
| `cito:supports`, `cito:citesAsEvidence`, `cito:disagreesWith` | `http://purl.org/spar/cito/` (SPAR CiTO) | **confirmed** at `http://purl.org/spar/cito/{supports,citesAsEvidence,disagreesWith}`. |
| `cito:extends`, `cito:usesMethodIn` (sq-2489d.3) | `http://purl.org/spar/cito/` (SPAR CiTO) | **confirmed** (2026-06-22) at `http://purl.org/spar/cito/{extends,usesMethodIn}` — the citing entity extends ideas in / employs a method documented in the cited entity. |
| `dqv:QualityMeasurement`, `dqv:Metric`, `dqv:Dimension`, `dqv:value`, `dqv:isMeasurementOf`, `dqv:inDimension`, `dqv:hasQualityMeasurement`, `dqv:computedOn` (sq-2489d.3) | `http://www.w3.org/ns/dqv#` (W3C DQV) | **confirmed** (2026-06-22) — namespace + all eight local names. **DQV is a W3C Working Group Note (2016-12-15), NOT a Recommendation** (recorded as an honest caveat above + design §3/§6.1). |
| `fabio:Expression`, `fabio:ConferencePaper` | `http://purl.org/spar/fabio/` (SPAR FaBiO) | **confirmed**; FaBiO is FRBR-structured. |
| `np:Nanopublication`, `np:hasAssertion`, `np:hasProvenance`, `np:hasPublicationInfo` | `http://www.nanopub.org/nschema#` | **confirmed** (namespace + local names). |
| `schema:Rating` | `https://schema.org/Rating` | **confirmed** (used only as a `skos:closeMatch` soft pointer for `pkg:confidence`). |

### Honest note — `schema.org` HTTP vs HTTPS

schema.org's **canonical** namespace is `https://schema.org/` (HTTPS). This ontology
uses the **`http://schema.org/`** form, following (a) the design's stated convention
and (b) the existing sparq repo convention — `crates/sparq-trust` (`vocab.rs`,
`wire.rs`, `admit.rs`, the e2e tests) and the wider tree use `http://schema.org/`
(68 occurrences vs 25 for the HTTPS form). Because the schema.org references here are
only soft `skos:closeMatch` pointers (not `owl:equivalentClass`/hard imports), the
`http://` vs `https://` choice does not affect validation; it is a stylistic
alignment-with-the-repo decision, recorded here for auditability. A future
consistency pass could canonicalise the whole repo to `https://schema.org/`.

### Honest note — the `secx:` assurance axis is a forward dependency

`pkg:assurance` reuses `secx:Proven` / `secx:Claimed` / `secx:Conjectured`
(namespace `https://w3id.org/zkp-sparql/sec-prop#`; the design's `secx:` prose-prefix
= the sec-prop *extension* axis). **These three IRIs are defined in the DESIGN record
`research/security-properties-ontology-design.md` §4.2.2 (epic `sq-0dksu`) and are
NOT yet shipped as a committed `.ttl`/`.yaml.ld`** — the vendored
`crates/sparq-trust/ontologies/zkp-sparql/vocab/sec-prop.yaml.ld` defines the eight
*security properties* but not this assurance axis. They are referenced here by their
stable `w3id.org` namespace so they unify automatically when `sq-0dksu` ships the
extension. SHACL `sh:in` checks IRI identity only, so the guardrail fires correctly
today regardless; but a downstream consumer that *dereferences* `secx:Proven` will
not resolve it until `sq-0dksu` lands. A bead should track shipping the `secx:`
assurance individuals (captured as discovered work).

## Namespace

The `pkg:` namespace (`https://sparq.dev/ns/pkg#`) is a **sparq-local** namespace,
consistent with `trust:` (`https://sparq.dev/ns/trust#`) and `zk:`
(`https://sparq.dev/ns/zk#`). It is NOT minted/resolvable today; a future
standardisation pass would rehome the net-new terms. Every `pkg:` IRI is mirrored as
a Rust constant in `../../src/vocab.rs` and byte-pinned against `pkg.ttl` by the
`ttl_pins_match_rust_constants` sync test (the `sparq-trust` discipline).

## Consumers within sparq

- `crates/sparq-kb` (this crate) — ships the ontology + shapes + example as data, and
  the `validate` feature drives `sparq-shacl` over them (the dogfooding test).
- Phase-2 ingestion PoC (`sq-2m6zm.2`) — projects `.beads/issues.jsonl` + the
  AGENTS.md gate matrix + Skills frontmatter into `pkg:`-typed triples, gated on
  these shapes.

## References

- Design record: `research/dogfooding-sparq-knowledge-graph.md` (PR #1063).
- Precedent: `research/security-properties-ontology-design.md` (epic `sq-0dksu`);
  `crates/sparq-trust/ontologies/zkp-sparql/` + `crates/sparq-trust/src/vocab.rs`.
- Beads: epic `sq-2m6zm`; this task `sq-2m6zm.1`; blocks ingestion `sq-2m6zm.2`.
