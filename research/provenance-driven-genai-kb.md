# Provenance-driven GenAI knowledge base — making PROV-O load-bearing + scaled literature trawling

<!-- [OPUS-4.8] research design record for sq-bxse0 (epic sq-2489d, issue #1110) -->

> **Status:** design-for-review (research record, no implementation). Synthesises three
> investigation streams into one design + iteration roadmap.
> **Author:** SPARQ agent 🤖 (Opus 4.8). **Bead:** sq-bxse0. **Epic:** sq-2489d.
> **Direction:** part of the **[revisit-with-fable] neurosymbolic-KB** umbrella
> ([issue #1111](https://github.com/sparq-org/sparq/issues/1111)) — every adopt/abandon
> verdict below is **model-dependent** and should be re-iterated when a stronger model
> (Fable) is available. Verdicts here are honest-best-effort on Opus 4.8, not final.
>
> **Update (2026-07-29 — issue #3246, bead sq-dbesf; see #1139):** §4.2's access table and
> the two `[needs-access]` lists were written when **OpenAlex** served an anonymous
> "polite pool" keyed only on a `mailto`. **OpenAlex retired the no-key polite pool in
> February 2026 and now requires an API key**, so the "No key" cell and the
> "OpenAlex/Crossref polite-pool `mailto`" phrasing were stale; both are corrected below.
> This changes the *credential* line only — the connector design, DOI join key, bulk-vs-API
> guidance and everything downstream are unaffected, because the `SourceStub` boundary is
> source-agnostic. Per `research/research-kb-program.md` ("Access reality") an OpenAlex key
> and a polite-pool `mailto` are now provisioned, so the OpenAlex half of the Phase-6
> credential blocker is cleared; Semantic Scholar remains unavailable. The record's own
> standing rule still applies: **re-verify every rate/auth cell at build time.**

---

## 0. What this record is, and the one honesty rule that governs it

This designs **three concrete uses of PROV-O across sparq's GenAI packages**
(quality→embedding-weighting, answer-qualification, citations), a **ranked
complementary-metadata recommendation** (testing the "PROV-O + DQV + CiTO/FaBiO +
nanopub are load-bearing; ODRL/DCAT are plumbing" hypothesis), and a **scaled,
provenance-stamped literature-trawling architecture** (Haiku over literature-DB APIs).
It closes with an **iteration roadmap** of phased future beads, each carrying a
**falsifiable metric**.

**The governing rule** (from the project's empirical-honesty mandate and the
`research/agent-effectiveness-program.md` / `bench/pkg-dogfood/` discipline): **do not
assume provenance-weighting, answer-hedging, citations, or literature ingestion improve
any outcome.** Each is shipped behind an on/off ablation and a pre-registered metric, and
the deliverable of each phase is *a measured verdict*, not "we built it." Where the
literature reports inconsistent gains, this record says so. No performance numbers are
hard-coded; work-box/EC2 timings are non-canonical.

### Correction to the brief's premise (verified against the code)

The brief and the prior streams describe the PKG instance graph as "6 Sources, all
Explored, 0 Unexplored." **That count is stale.** As landed today,
`crates/sparq-kb/ingest/pkg-instances.ttl` carries **70 `pkg:Source` instances, all
`pkg:exploredStatus pkg:Explored`, and zero `pkg:Unexplored`** — the ingest now
auto-projects every `research/` doc and skill as a Source. The *qualitative* point the
streams rely on still holds and is in fact sharper: the **unexplored frontier is empty**,
so the literature-trawling architecture (§3) is what *populates* it; it is not adding to
an existing frontier. Everything else in the three streams checked out against the actual
code (verified files listed in §6).

---

## 1. The substrate that exists today (verified)

All three uses build on machinery that is **already landed but largely inert** — the data
is recorded and queryable but is never fed back into a weight, a hedge, or a citation.

- **PKG quality + provenance vocabulary, recorded but read-only.**
  `crates/sparq-kb/ontology/pkg/pkg.ttl` defines `pkg:Finding` (generalising the vendored
  `sig-impl:Assertion`) carrying `pkg:confidence` (NET-NEW `xsd:decimal` 0..1, on a
  Finding *or* a Source — Source confidence = reliability), `pkg:assurance` →
  `secx:Proven`/`secx:Claimed`/`secx:Conjectured` (epistemic basis), and
  `pkg:discoveredFrom rdfs:subPropertyOf prov:wasDerivedFrom` + `cito:citesAsEvidence` +
  `dcterms:source` (section anchor). Predicate constants are in
  `crates/sparq-kb/src/vocab.rs` (a byte-pin test mirrors every term — any new term must
  be added there in the same PR).
- **Canned queries already SELECT the quality fields.** In
  `crates/sparq-kb/src/query/canned.rs`, `FINDINGS_ABOUT` returns `?label ?section ?conf`
  ordered by confidence; `FINDING_PROVENANCE` returns
  `?label ?source ?section ?assurance ?conf`. So provenance + confidence are *queryable*
  but never consumed downstream.
- **The cheap-model NL tool already returns its own provenance.**
  `crates/sparq-nlq/src/lib.rs` `Answer` carries `sparql`, `result`, `repairs`, and a full
  `transcript: Vec<Turn>` — but **no link from result rows back to the supporting source
  quality**, no hedge, and no citations field.
- **`sparq-prov` is the standard PROV-O lineage producer.**
  `derive_construct` (`src/lib.rs`), `derive_update` (`src/update.rs`), and
  `prov_from_proof` (`src/reason.rs`) emit per-fact `wasDerivedFrom`/`wasGeneratedBy`/
  `used`, content-addressed and stitchable — the natural lineage source for citations.
- **`sparq-vectors` (epic sq-0wo9e) has the structured-object machinery but no trainer.**
  `structure.rs` (closure-before-vectorise + type-constrained negatives, P0); `encode.rs`
  `Block`/`SchemaHeader` (per-block encoder + metric + `[offset,width)`, versioned
  sidecar); `grounding.rs` (`Modality::Subgraph`/`TypedSubVector`/`NlString`/`TypedValue`);
  `fuse.rs` (`fuse_rrf_weighted`/`fuse_scores`/`hybrid_search`, per-list weights);
  `shacl_priors.rs` (the "read a schema into a prior" pattern to mirror). **Verified
  gaps:** `Block` has exactly four fields (`encoder`, `metric`, `offset`, `width`) — **no
  fusion-weight field**; and there is **no in-tree KGE training loop** (embeddings are
  produced out-of-process via `embed.rs`/`import_npy`).
- **The SHACL write-gate + projector exist.** `crates/sparq-kb/shapes/pkg.shapes.ttl` is
  the shape set; `crates/sparq-kb/ingest/ingest_pkg.py` is a deterministic projector
  (`project_beads`, `project_skills`) that emits SHACL-conforming TTL and quarantines
  rejects to a `.stale-edges.tsv` sidecar.

---

## 2. The three PROV-O uses — concrete design + gap-from-today

### Cross-cutting primitive (build once, unlocks Uses 2 and 3)

A **result-row → supporting-triples → `prov:wasDerivedFrom`/`pkg:confidence`/
`pkg:assurance` join**. It is *trivial for the PKG* (provenance is asserted on Findings,
and the canned queries already return it) and is the general-case dependency for arbitrary
graphs (needs PROV-annotated data, or `sparq-prov` derivation capture wired into the NL
execution path). Build this primitive first; Uses 2 and 3 are renderers on top of it.

### USE 1 — quality → weighting (provenance/trust-weighted KG embeddings)

**Goal.** Use `pkg:confidence`/`pkg:assurance`/`prov:wasDerivedFrom` to weight how
strongly a relationship contributes to a node's vector.

**Design.** Derive a **per-triple weight** `w(t) ∈ (0,1]` from the PKG: combine the
source-reliability `pkg:confidence` of the triple's `prov:wasDerivedFrom` source with an
assurance multiplier (`secx:Proven` → high, `secx:Claimed` → mid, `secx:Conjectured` →
low), defaulting to `1.0` when no provenance exists (so plain graphs are unchanged). Thread
`w(t)` into the three points `sparq-vectors` actually has:
1. **Type-constrained negative sampling (`structure.rs`, P0)** — emit `w(t)` alongside
   each positive in the sampler's output stream so the out-of-process trainer can scale the
   positive-triple loss contribution (the CKRL confidence-weighted-loss move), plus a
   confidence-biased-walk weight for any RDF2Vec-style walker.
2. **Structural-sketch / characteristic-set pooling** — when pooling a multi-valued
   predicate's contributions to a node vector, pool **confidence-weighted** rather than
   uniform.
3. **A new per-`Block` default fusion weight** (`encode.rs`) — at query time scale a
   node's modality contribution by an aggregate of its incident-edge provenance, consumed
   by the existing `fuse_rrf_weighted`/`fuse_scores` path.

**Prior art (real).** CKRL (Confidence-aware KG Representation Learning, Xie et al. AAAI
2018) — per-triple confidence reweights the margin loss; the canonical formulation.
Embedding-with-triple-trustiness (PMC7514427) and entropy-weighted relations (TransCE).
Edge-weighted/trust-propagation GNNs (TrustGNN, arXiv:2205.12784; interpretable GCN on
noisy KGs, arXiv:1812.00279 — low-weight edges are more likely erroneous). **Honest note:**
most of this weights by an *intrinsic learned/extracted* triple confidence, not an
*extrinsic provenance/source-reliability* signal. sparq's PKG gives a clean extrinsic
signal (source reliability + Proven/Claimed assurance), so this is a defensible novel angle
rather than a re-implementation — but the literal/type-aware KGE line reports *inconsistent*
gains, so any accuracy lift is unproven and dataset-dependent.

**Gap from today.** No in-tree KGE trainer → confidence-weighted loss must target the
out-of-process trainer (emit weights from the sampler). `Block` has no weight field →
small additive field + a versioned-sidecar bump. No code path reads `pkg:confidence`/
`pkg:assurance` into `sparq-vectors` → needs a PKG→weight reader mirroring
`shacl_priors.rs`. Ships behind an on/off ablation; **claim nothing in advance — measure
Hits@k / MRR with weighting on vs off.**

### USE 2 — answer-qualification (calibrated/confidence-aware KG-QA)

**Goal.** Use `pkg:confidence`/`pkg:assurance` to hedge the NL tool's answers.

**Design.** Add a **hedging layer over the `Answer`** in `sparq-nlq`. After execution, run
the cross-cutting provenance join: for each answer row, pull the supporting Findings'/
Sources' `pkg:confidence` + `pkg:assurance`. Derive an answer-level qualification —
**assurance → verb hedge** (all-Proven → assertive; any Claimed → "appears to be
(claimed)"; Conjectured/missing → "may be" / abstain), **confidence → verbal band**
(high/medium/low, optional numeric), and an **abstention threshold** (below a configurable
floor, return "insufficient confidence to answer" rather than asserting). This composes
with the existing `NlqConfig` knob pattern as a new opt-in `hedge` / `min_confidence` knob
(re-record fixtures).

**Prior art (real).** Two uncertainty sources to calibrate — *evidence* uncertainty and
*reasoning* uncertainty (arXiv:2410.08985, Trustworthy KG Reasoning); sparq's
`pkg:confidence`/`pkg:assurance` is a *direct, symbolic evidence-uncertainty signal* that
most LLM work has to estimate. Abstention/confidence-aware answering ("Know Your Limits"
abstention survey; I-CALM, arXiv:2604.03904; "When Silence Is Golden", arXiv:2602.04755;
Double-Calibration, arXiv:2601.11956). KGE calibration: KGE scores are often *mis*calibrated
and need Platt/temperature scaling (arXiv:2004.01168) — relevant only if a hedge is ever
derived from a vector score rather than asserted `pkg:confidence`.

**Gap from today.** `Answer`/`Turn` have no link from result rows back to supporting source
quality → the cross-cutting join. No hedging/abstention rendering exists → net-new but
small, behaves like the existing config knobs. **Calibration is unverified:** a verbal band
is only meaningful if `pkg:confidence` is itself calibrated, and today those numbers are
hand-authored estimates in `agents-findings.ttl` — so the honest framing is "the hedge
*reflects asserted assurance*", and any "calibrated confidence" claim needs a
reliability-diagram measurement (the privacy/honesty gates forbid overclaiming).

### USE 3 — citations (citation-generating RAG/KG-QA)

**Goal.** Make NL responses emit citations to sources.

**Design.** Citations are **emitted from provenance, never generated by the LLM** — the key
honesty win, and the thing that avoids reference-hallucination. Two tiers:
1. **PKG-native (ready ~90%).** `FINDING_PROVENANCE`/`FINDINGS_ABOUT` already return
   `prov:wasDerivedFrom` source + `dcterms:source` anchor + `cito:citesAsEvidence`. A
   **citation renderer** turns each answer row into a footnote: claim → [source title,
   section anchor]. Only the renderer is missing. Lowest-risk, highest-value first slice.
2. **General KG-QA.** After the NL tool executes its SPARQL, for each row identify the
   supporting triples and — if the graph (or a companion store) carries
   `prov:wasDerivedFrom`, or `sparq-prov` tracked the derivation — emit a citation to each
   source IRI. The grounding dispatcher's `Modality::Subgraph` (smallest sub-BGP entailing
   the answer) **is** the structured citation object; rendering it + its `wasDerivedFrom`
   edges is the citation.

Output contract: extend `Answer` with `citations: Vec<Citation>` (source IRI + anchor +
supporting triples/sub-BGP), aligned to ALCE-style inline markers so a downstream LLM
verbalisation can carry `[1]`-style references back to the IRIs.

**Prior art (real).** ALCE (Gao et al. 2023/2024) — first end-to-end LLM-citation
benchmark; even the best models lack full citation support roughly half the time on ELI5.
Citation correctness ≠ faithfulness (arXiv:2412.18004; survey arXiv:2508.15396) — a
citation can be correct yet not what the model actually used. **KG-QA over an exact SPARQL
result sidesteps this:** the citation is the *provenance of the binding*, not a post-hoc
retrieval guess — a structurally stronger guarantee than text-RAG citation. Reference-
hallucination detection (arXiv:2604.03173) is the failure mode this avoids by construction.
Citekit (arXiv:2408.04662), grounded-attribution + learning-to-refuse (arXiv:2409.11242),
MIRAGE internals attribution (arXiv:2406.13663).

**Gap from today.** PKG-native tier needs only a renderer. General tier needs the same
row→supporting-triples→source join Use 2 needs, plus PROV-annotated data or `sparq-prov`
capture for non-PROV graphs. `Answer` has no `citations` field (additive). **Faithfulness
is by-construction only when real provenance exists** — over a bare graph with no
provenance the honest output is "no source recorded", **never a fabricated citation**.

### Architecture fit (all three)

All three are **opt-in/additive**, matching the standing opt-in-feature mandate: Use 1
extends `sparq-vectors` behind the existing `structure` feature + an ablation; Uses 2/3
extend `sparq-nlq` behind new `NlqConfig` knobs. No core crate (`sparq-core`/`sparq-engine`)
changes. None of the three needs a new ontology term — they make the *existing*
`pkg:confidence`/`pkg:assurance`/`prov:wasDerivedFrom`/`cito:citesAsEvidence`/`dcterms:source`
terms load-bearing.

---

## 3. Ranked complementary-metadata recommendation

**Hypothesis under test:** "PROV-O + DQV + CiTO/FaBiO + nanopub are load-bearing;
ODRL/DCAT are plumbing."

**Verdict (honest):** the hypothesis is **largely correct but mis-weighted on two
vocabularies.** PROV-O + CiTO/FaBiO + nanopub are load-bearing **and already vendored into
`pkg.ttl`** (verified: `cito:`, `fabio:`, `np:` prefixes are present). **DQV is also
load-bearing but is the ONE the codebase does not use** (verified: no `dqv:` prefix in
`pkg.ttl`) — it is the actionable gap. And **DCAT is not pure plumbing**: it is a *quiet
load-bearing reuse* already in the ontology (`pkg:Source rdfs:subClassOf
fabio:Expression , dcat:CatalogRecord`, verified). **ODRL is genuinely orthogonal**
(hypothesis confirmed). SKOS is foundational-but-unexciting (already pervasive).

| Rank | Vocab | Status | Net assessment | In-repo state (verified) |
|---|---|---|---|---|
| 1 | **PROV-O** | W3C Rec | Load-bearing baseline; the floor everything attaches to | `sparq-prov` + mandatory `prov:wasDerivedFrom` on every Finding |
| 2 | **DQV** | W3C **Note** (2016) | **Load-bearing AND the biggest gap** — the principled home for the ad-hoc `pkg:confidence`/`pkg:assurance` axes | **NOT in `pkg.ttl`** — the actionable finding |
| 3 | **CiTO + FaBiO** | community, stable | Load-bearing for citations + the algorithm-relation graph; **under-exploited** | Present (`cito:`, `fabio:`, `bibo:`) |
| 4 | **nanopublications** | community, active | Load-bearing as the *answer-/finding-export envelope* (packaging, not new semantics) | `np:` referenced in ontology, not yet *produced* |
| 5 | **SKOS** | W3C Rec | Foundational substrate (concept/topic/status schemes) | Pervasive backbone |
| 6 | **DCAT** | W3C Rec v2/v3 | **Not pure plumbing** — the right source/lit-catalog base class; already reused | `pkg:Source rdfs:subClassOf dcat:CatalogRecord` |
| 7 | **ODRL** | W3C Rec | **Orthogonal — confirmed.** Rights/policy, not quality/citation/prov | Separate epic sq-3183; not a GenAI-metadata vocab |

**The two adjustments (the only real changes):**
1. **Adopt DQV as the model for the quality axis.** `dqv:QualityMeasurement` (a reified
   `dqv:value` + `dqv:isMeasurementOf` a `dqv:Metric`, in a `dqv:Dimension`) is the
   standard home for source-reliability, answer-confidence, link-score, and
   provenance-completeness as *named, distinct* metrics instead of one overloaded
   `pkg:confidence` scalar. It carries `prov:wasGeneratedBy`/`wasAttributedTo` natively, so
   it composes with `sparq-prov` for free, and the PKG already subclasses
   `dcat:CatalogRecord` (DQV's anchor), so the coupling is already paid. **Honest caveats:**
   DQV is a W3C *Note*, not a Recommendation (lower normative weight than PROV-O/SKOS/DCAT —
   matters for the project's verified-stable-namespace discipline); and DQV expresses a
   measurement, it does **not** give confidence *propagation through joins* (the
   Hartig/tSPARQL gap is real and orthogonal). Recommendation: keep `pkg:confidence` as a
   convenience shorthand but define it as derived-from a named `dqv:QualityMeasurement`, and
   align `pkg:assurance` as a `dqv:Metric` in an epistemic-basis `dqv:Dimension`.
2. **Lean harder on CiTO typed relations for the technique-graph.** Today the PKG uses
   `dcterms:replaces` for supersedes and a **net-new `pkg:couldBeMergedWith`** (verified,
   `owl:SymmetricProperty`). The established SPAR terms `cito:extends` /
   `cito:usesMethodIn` cover much of the "novel/mergeable algorithm" link and would shrink
   the net-new surface — reserve the bespoke symmetric term only for genuine
   merge-candidacy.

**Plumbing-but-load-bearing / orthogonal (no change):** DCAT stays as the source-catalog
base, co-typed with FaBiO. SKOS stays the scheme backbone. ODRL stays *out* of the GenAI
metadata vocabulary — its one adjacent niche (literature-licensing at ingest) belongs to
the existing usage-control estate (epic sq-3183, `research/feature-research-odrl-policy.md`,
skill `usage-control-policy`); reference it via a `dcterms:license` / `odrl:hasPolicy`
pointer on `pkg:Source`, don't duplicate it.

---

## 4. Scaled, provenance-stamped literature-trawling architecture

"Deep research on steroids" — but **targeted, gated, and opt-in**, not an unbounded
crawler. The honest counter-weight, stated up front because the project's measurement
discipline demands it: `research/agent-effectiveness-program.md` and the dogfooding §5
already found a queryable PKG to be a *modest, contingent* win for agent memory, and bulk
ingestion inflates the graph with `pkg:Source`/`pkg:Finding` nodes whose token-A/B payoff
is unproven. So **the deliverable of an ingestion run is findings + a verdict, not "we
ingested N papers."**

### 4.1 Substrate fit (the ontology was built for this)

The PKG already models exactly what literature ingestion needs: `pkg:Source` (a
`fabio:Expression` + `dcat:CatalogRecord` with `bibo:doi`, `dcterms:creator`,
`dcterms:issued`), `pkg:exploredStatus` (`Unexplored|Exploring|Explored|DeadEnd`),
`pkg:followUpPriority`, and `pkg:Finding` (confidence/assurance/provenance). The schema work
is done; what is missing is connectors, a cheap-model extraction pipeline, targeting, and
dedup/quality-gating at scale.

### 4.2 Literature-DB sources + access reality (`[needs-access]` is the hard blocker)

All of these are external network resources unreachable from the sandboxed work box without
the maintainer providing access — egress, a polite-pool email, and per-source keys. Limits
per public API docs as of training cutoff; **re-verify at build time** (they change).

| Source | Coverage | Auth | Rate-limit reality | Best for |
|---|---|---|---|---|
| **OpenAlex** | ~250M works, broadest open corpus | **API key required** (the no-key polite pool was retired Feb 2026; `mailto` alone no longer suffices) | per-key quota — re-verify; monthly S3 snapshot | Primary breadth + bulk snapshot |
| **Crossref** | ~150M DOIs + reference lists | No key; polite-pool `mailto` | polite pool soft cap | DOI canonicalisation + citation edges |
| **Semantic Scholar** | ~200M papers + TLDRs + SPECTER2 vectors + influential-citation flags | Free key on request (`x-api-key`) | dedicated ~1 req/s with key; bulk/datasets API | Abstracts/TLDRs + SPECTER2 (feeds `sparq-vectors`) |
| **arXiv** | preprints, full PDF/LaTeX | No key | ~1 req/3s; Kaggle/S3 bulk | Full text for DB/ML/CL |
| **DBLP** | CS bibliography, clean author/venue | No key; XML dump | gentle | Venue authority (SIGMOD/VLDB/ICDE) |
| **ACL Anthology** | computational linguistics, near-complete | No key; BibTeX + dump | static files | Linguistics/NLP full coverage |

Cross-cutting: **the bulk/snapshot path beats the per-record API at scale** (sidesteps
rate-limit fragility); **DOI is the join key** (make the DOI — or arXiv id where none — the
content-addressed `pkg:Source` IRI so the same paper from two sources stitches to one node).
**`[needs-access]` (maintainer-only):** an S2 API key; an **OpenAlex API key** (mandatory
since the Feb-2026 polite-pool retirement — see the Update at the top) plus a Crossref
polite-pool `mailto`; outbound egress + S3/requester-pays creds for the bulk path; an
Anthropic key/budget for the Haiku batches. An agent cannot mint these — the only valid
blocker under the proceed-without-greenlight rule. (Status: the OpenAlex key and the
`mailto` are provisioned per `research/research-kb-program.md`; S2 is not.)

### 4.3 The extraction pipeline (Haiku turning papers into Findings)

Model + cost surface (verified against the `claude-api` skill): **`claude-haiku-4-5`**,
$1.00/1M in, $5.00/1M out, 200K context, 64K max output. **Batches API: 50% off**, ≤100K
requests / 256MB per batch, most complete <1h — the correct substrate (ingestion is not
latency-sensitive). **Structured outputs** supported — force every extraction into the exact
`pkg:Finding` shape (verdict, confidence ∈ [0,1], assurance ∈ {Proven,Claimed,Conjectured},
justification, cited DOIs). **Prompt-caching caveat (load-bearing):** Haiku's minimum
cacheable prefix is **4096 tokens** — the shared system prompt + ontology card + few-shot
exemplars must clear 4096 tokens to cache (they comfortably will). **Haiku does NOT support
`effort`** — keep the extraction prompt lean by construction.

The IE stack, collapsed (`[connector] → [normalise] → [extract] → [ground] → [emit TTL] →
[SHACL gate] → [dedup] → [verdict]`): connector pulls abstract/TLDR; normalise emits a
DOI-keyed `pkg:Source` stub (`pkg:Exploring`); Haiku (Batches, cached prefix) emits N
candidate Findings via a JSON schema; ground maps cited DOIs → existing Source IRIs and
`about` → existing `skos:Concept` topics via `sparq-vectors`/`sparq-sim` nearest-concept
(mint a new topic only past a distance threshold); emit full PROV-O lineage
(`wasDerivedFrom` Source, `wasGeneratedBy` the extraction Activity, `wasAttributedTo`
agent:haiku-extractor); **SHACL gate** against `pkg.shapes.ttl` (conforming committed,
violators quarantined to a sidecar, never dropped); dedup; flip `exploredStatus` to
Explored (or DeadEnd if 0 findings).

**The honest extraction-quality boundary (load-bearing, not soft-pedalled).** A cheap model
**will hallucinate claims and mis-cite.** Mitigations are real but partial: SHACL gates
*structure, not truth* (a well-formed-but-false Finding becomes durable poison); the
strongest available anchor is a **deterministic grounding-resolver** — require every
Finding's cited DOI to resolve to a `pkg:Source` actually in the batch, and (stretch)
require the justification to be an entailed span of the abstract — quarantine, don't commit,
on failure (this is the propose-then-verify pattern, OPEN as sq-0wo9e.6). **Tier by trust:**
all Haiku-extracted Findings sit on a low-trust tier (`secx:Conjectured`, a confidence
ceiling) until a sampled human/independent audit calibrates Haiku's accuracy on this corpus;
**a Haiku extraction must never outrank a hand-authored Finding** and must **never** stamp
`secx:Proven` on someone else's paper claim (enforce declaratively in a SHACL shape).

### 4.4 Targeted ingestion — the PKG is the work-queue

This is the genuinely novel part vs generic literature-RAG. **Topic-driven:** SPARQL over
existing `skos:Concept` topics + open `pkg:Task`s with status ∈ {Open,InProgress} — ingest
*for the topics the project has open work on*. **Unexplored-driven:** the canned
"what-to-read-next" query (`pkg:exploredStatus pkg:Unexplored ORDER BY pkg:followUpPriority`)
joined with the "already-explored?" membership test (`ASK { ?act prov:used <S> }`) so the
crawler never re-ingests a source already in the provenance DAG. (Because the committed
frontier is empty today — §0 correction — a connector first *seeds* `pkg:Unexplored` Source
stubs cheaply; extraction is the expensive step that flips them to Explored.)
**Citation-frontier expansion:** from a high-confidence Explored Source, walk one hop of
Crossref/OpenAlex/S2 references + citations, mint neighbours as `pkg:Unexplored` ranked by
topic overlap — bounded BFS, not unbounded crawl. **The research↔bead rule:** a novel,
un-implemented technique surfaced with no covering bead implies `pkg:needsBead`; the rule
*proposes*, `bd` *records*. **Budget discipline (mandatory):** every run carries a hard cap
— N papers OR M dollars OR a wall-clock window, with `--dry-run`. Unbounded crawling is the
anti-pattern.

### 4.5 Dedup + quality-gating (SHACL + DQV)

**Dedup, multi-layer:** (1) exact — DOI-keyed content-addressed IRI; arXiv-id fallback. (2)
near-dup Sources (preprint vs published) — `sparq-sim` or `sparq-vectors` over **the
SPECTER2 vectors S2 ships** (don't re-embed) → `skos:exactMatch`/`dcterms:isVersionOf`. (3)
near-dup Findings — `pkg:couldBeMergedWith`/`skos:related`; contradictory claims →
`cito:disagreesWith`, **kept not dropped**, with confidence carrying the weight.
**Quality-gating:** SHACL is the write-gate (have it). New literature-tier shapes worth
adding (bead them): a `cito:citesAsEvidence` must resolve to an in-graph `pkg:Source` (no
dangling citation); a Haiku-extracted Finding must carry `pkg:assurance` ≠ `secx:Proven`; a
confidence ceiling on the Haiku tier. **DQV is the one new vocabulary import this work
needs** — `dqv:QualityMeasurement`/`dqv:Metric` for *per-batch* quality
(extraction-precision-on-sample, citation-resolution rate, dedup-collision rate,
topic-coverage), making "how good is this batch" a query that feeds the §5 verdict.

### 4.6 Cost model (parameterised, non-canonical, structure-not-magnitude)

Per-paper extraction cost ≈ `input(cached prefix ×0.1 after first + abstract + schema) +
output(N findings)`, at Haiku rates, ×0.5 for Batches. **Dominant levers, highest first:**
(1) **abstract-only vs full-text** — full PDFs are far larger; default to abstract+TLDR,
escalate to full text only past an abstract-tier relevance gate (the single biggest knob).
(2) Batches ×0.5 — always on. (3) prompt caching — shared ontology-card prefix amortised
(must clear 4096 tokens). (4) structured-output schema bounds findings/paper. **Honest
verdict:** extraction is cheap; **the risk is the long tail of low-value ingestion**, not
per-paper price — ingesting 10,000 papers nobody queries is a net loss in storage, dedup
overhead, and poison risk. **Gate ingestion per-topic on the same recommend-adopt verdict
the dogfooding §5 defines:** ingest a topic only if its findings are projected to be queried
above break-even *and* a sampled extraction-accuracy audit clears a pre-registered bar. (All
numbers computed at runtime via `count_tokens`; never frozen — `check-no-perf-numbers.py`.)

### 4.7 How this extends `ingest_pkg.py`

Add a **`project_literature(...)`** projector alongside `project_beads`/`project_skills`,
fed by per-source connector adapters, emitting into the **same TTL stream through the same
SHACL gate**. The Haiku extract step is the **only LLM-in-the-loop part** — isolate it
behind a **record/replay trait** so CI runs on recorded fixtures and a real Haiku batch is
opt-in + feature-gated OFF (**CI must never make a live model call**). Reuse the
sidecar-honesty pattern (`.quarantine.ttl` for SHACL rejects, `.unresolved-citations.tsv`
for ungrounded citations). Keep the literature tier a **separate named graph** so the
high-trust hand-authored tier and the Haiku tier are queryable-apart and the A/B can isolate
them. `pkg.ttl` changes (small, bead them): import DQV; add the three literature SHACL
shapes; consider a `pkg:`-specific research-verdict enum ({holds/refuted/uncertain/
superseded}) since the generic `{yes,no,partial}` verdict may not map onto a research
verdict space — the strain shows exactly at literature ingestion. The `vocab.rs` byte-pin
test means any new term is mirrored there in the same PR.

---

## 5. Iteration roadmap (this is an ongoing task — phased future beads)

Each phase is a future bead the orchestrator can track. Every phase carries a
**falsifiable metric** and reuses the `bench/pkg-dogfood/` / `agent-effectiveness-program`
measure-first discipline (PREREG → run → grade → verdict). Phases are ordered by dependency
and by risk/value (cheapest, most-grounded first). **None ships a quality claim ahead of its
metric.** Verdicts are **model-dependent** — re-run under Fable (issue #1111) before
treating any "abandon" as final.

1. **Phase 1 — Cross-cutting provenance join + Use 3 PKG-native citation renderer.**
   Build the result-row → supporting-triples → `prov:wasDerivedFrom`/`pkg:confidence`/
   `pkg:assurance` join (trivial for the PKG), then a renderer turning
   `FINDING_PROVENANCE`/`FINDINGS_ABOUT` rows into cited footnotes; add `Answer.citations`.
   *Metric:* on a PKG-answerable fixture set, **citation-resolution rate = 1.0** (every
   emitted citation resolves to a real in-graph Source/anchor) and **zero fabricated
   citations**; over a no-provenance graph the output is "no source recorded", never a
   guess. Lowest-risk, highest-value first slice.
2. **Phase 2 — Use 2 answer-qualification (hedge + abstention) behind an `NlqConfig` knob.**
   Reuse the Phase-1 join; add assurance→verb-hedge, confidence→band, and a
   `min_confidence` abstention floor; re-record fixtures. *Metric:* on a labelled
   answerable/unanswerable fixture split, **abstention precision/recall** at the floor, and
   a **reliability check** that the verbal band tracks asserted `pkg:assurance`
   monotonically. Do **not** claim "calibrated confidence" until a reliability-diagram
   measurement exists on calibrated (not hand-authored) confidences.
3. **Phase 3 — Adopt DQV + lean on CiTO typed relations (ontology change).** Import DQV;
   define `pkg:confidence` as derived-from a named `dqv:QualityMeasurement`, `pkg:assurance`
   as a `dqv:Metric`; add `cito:extends`/`usesMethodIn` for the technique-graph; mirror new
   terms in `vocab.rs` (byte-pin) in the same PR. *Metric:* **no regression** — every
   existing canned query + SHACL shape + `vocab.rs` test stays green; the DQV-modelled
   quality axis answers at least the queries the ad-hoc `pkg:confidence` answered today, and
   the net-new term count does **not** grow (CiTO replaces some bespoke surface). Honest:
   flag DQV's W3C-Note status in `PROVENANCE.md`.
4. **Phase 4 — Use 1 provenance-weighting in `sparq-vectors` (behind `structure` +
   ablation).** Add the PKG→`w(t)` reader (mirror `shacl_priors.rs`), emit `w(t)` from the
   `structure.rs` sampler stream, add the per-`Block` weight field + sidecar bump.
   *Metric:* **Hits@k / MRR with provenance-weighting ON vs OFF** on a held-out link-
   prediction split — adopt only if the lift clears a pre-registered bar; **abandon (and
   say so) if it doesn't**, consistent with the inconsistent-gains literature.
5. **Phase 5 — Literature-ingestion scaffolding on fixtures (no live calls).** Add
   `project_literature` + the record/replay extract trait + one connector adapter +
   literature SHACL shapes + the deterministic grounding-resolver (propose-then-verify),
   all behind a feature gate, CI on recorded fixtures only. *Metric:* on a frozen fixture
   batch, **SHACL-conformance rate** and **citation-grounding rate** reported to a sidecar;
   the pipeline quarantines (never silently drops) every reject; **CI makes zero live model
   calls**.
6. **Phase 6 — Gated live ingestion pilot (needs maintainer access).** With
   `[needs-access]` credentials, run one hard-capped, `--dry-run`-first batch for a single
   open-work topic via Haiku Batches; sample-audit accuracy. *Metric:* a per-topic
   **recommend-adopt verdict** — extraction precision on a human-audited sample vs a
   pre-registered bar, and projected query-volume vs break-even; **ingest the topic only if
   both clear**. The run ends in a verdict, not a commitment.
7. **Phase 7 — Token-A/B of the provenance-driven KB end-to-end (the honest payoff test).**
   Reusing `bench/pkg-dogfood/`, measure whether citations + hedging + provenance-weighting
   change agent task outcomes/tokens vs the inert-PKG baseline. *Metric:* the
   `agent-effectiveness-program` A/B verdict (honest=true AND recommend=true) per capability
   — keep only what measurably pays. Re-run under Fable before any final verdict (#1111).

**Dependencies:** P1 → {P2, P4 (join reused by both)}; P3 is independent (ontology, can run
parallel); P5 → P6 (live needs scaffolding); P6 → P7 (end-to-end needs ingested data, or
runs on the existing hand-authored tier). Partition by crate so P1/P2 (`sparq-nlq`),
P3/P5/P6 (`sparq-kb`), and P4 (`sparq-vectors`) parallelise without merge contention.

---

## 6. Open questions — 2026-07-05 rulings

**Authority & ruling origin:** The maintainer's 2026-07-05 decision on #1111
(PR #1589) decomposed the neurosymbolic re-attempt as phased work under existing epics
with explicit grant to proceed without waiting for greenlight on phases B/C. The rulings
below close the three open design questions; the two-ladder decision (§1 + calibration)
is recorded in that PR's body (issue #1111 comment by @jeswr).

1. **DQV's W3C-Note status vs the verified-stable-namespace discipline — DECIDED: Adopt.**
   **Ruling (sq-2489d.3, merged):** DQV is the right model and is RATIFIED. The adoption
   stands: `pkg:confidence` is now `rdfs:subPropertyOf dqv:value` of a named
   `dqv:QualityMeasurement`, `pkg:assurance` is a `dqv:Metric`, and the technique-relation
   graph leans on `cito:extends`/`cito:usesMethodIn` — the §3 gap is closed. DQV's W3C-Note
   status (not a Rec) is flagged in `PROVENANCE.md` as a caveat, consistent with the
   verified-stable-namespace discipline (use the right model; acknowledge normative weight).
   No breaking change to the codebase; the reuse-first principle is preserved.

2. **Research-verdict enum — DECIDED: Keep `{yes,no,partial}` + assurance; defer richer
   enum until measured ingestion strain.** **Ruling (sq-2489d / #1111 2026-07-05):** The
   {yes,no,partial} verdict paired with the categorical assurance axis (Proven/Claimed/
   Conjectured) is sufficient for launch. The enum-mapping convention is: a Finding with
   `pkg:verdict pkg:yes` + `pkg:assurance secx:Proven` is high-confidence asserted truth;
   `pkg:verdict pkg:yes` + `pkg:assurance secx:Claimed` is a claimed fact (the claim is
   true; the assurance is lower); `pkg:verdict pkg:no` + `pkg:assurance` reverses the
   polarity (the claim is false, with assurance level modulating how sure we are the
   negation is true); `pkg:verdict pkg:partial` signals a mixed or nuanced result (e.g.,
   "this technique is faster on graphs <100K triples, slower beyond"). A research-verdict
   subclass ({holds/refuted/uncertain/superseded}) is deferred until literature ingestion
   produces measured evidence of strain on the generic enum (sq-2489d.6 / Phase 5).

3. **`[needs-access]` for the live pilot (Phase 6).** S2 API key; an **OpenAlex API key**
   (the no-key polite pool was retired Feb 2026) + a Crossref polite-pool `mailto`; egress +
   S3/requester-pays creds; Anthropic Haiku budget. An agent cannot mint these — Phases 1–5
   are fully buildable + testable on fixtures without them. (No ruling needed; this is a
   credential block, not a design choice.) **Partially cleared (2026-07-29):** the OpenAlex
   key and polite-pool `mailto` are provisioned per `research/research-kb-program.md`; S2
   remains unavailable, so the SPECTER2 near-dup layer (§4.5 layer 2) stays designed-only.

4. **Confidence calibration source — DECIDED: Ship hedging as "reflects asserted assurance"
   until rung C (sq-2489d.10).** **Ruling (sq-2489d / #1111 / sq-tzars.5 2026-07-05):**
   Today `pkg:confidence` numbers are hand-authored estimates. The calibration ladder is:
   - **Rung A (now, sq-2489d.9 in flight):** asserted-assurance hedging over hand-authored
     confidences, with an explicit **UNCALIBRATED** disclaimer in outputs (the hedge
     "reflects asserted assurance" only; any verbal band is a tool's subjective rendering).
   - **Rung B (sq-2489d.9):** explicit UNCALIBRATED disclaimer message surfaced in
     `sparq-nlq` outputs, warning users that confidences are hand-authored, not empirically
     calibrated.
   - **Rung C (blocked by sq-tzars.9, sq-2489d.10):** a reliability-diagram harness
     measuring how well the asserted confidences actually predict accuracy. Only after rung
     C passes a pre-registered bar should any "calibrated confidence" claim be shipped; until
     then, the honest framing is "asserted epistemic weight, unvalidated".

5. **Foundational-ontology choice for the PKG — DECIDED: schema.org-as-top RATIFIED
   (bead sq-mztg8.5, 2026-07-06).** The Fable-subject Metric-1 re-run (PR #1603,
   `bench/fo-km/RESULTS.md` Fable section) shifts but does not flip the incumbent
   verdict. Per-tier evidence:

   | Tier | schema.org | DOLCE-DUL | gUFO | no-FO | Signal |
   |---|---|---|---|---|---|
   | **Haiku/cheap fleet** (original run, PRs #1107/#1108) | **0.84** | 0.64 | 0.54 | 0.58 | Clear schema.org win across TH/ER/CC |
   | **Fable tier** (PR #1603) | **0.56** (tie) | **0.56** (tie) | 0.31 | 0.25 | Tie on full task set; schema.org retains expressibility-subset edge (0.67 vs 0.53 over tasks each arm can express) |

   **Decision:** schema.org-as-top remains the PKG's top FO. The Fable-tier tie does
   not auto-flip the default because (a) the cheap fleet's PKG-query traffic still
   shows the original clear win — the PKG is predominantly queried by Haiku/Sonnet
   agents; (b) on tasks each arm can actually express, schema.org retains the lead even
   at Fable tier; (c) gUFO is a clear loser at both tiers and no-FO is worst at Fable
   tier. No ontology data changes: `pkg.ttl` bridge axioms and the existing schema.org
   FO typing are unchanged. **Revisit trigger:** if the fleet's KM-query tier moves
   predominantly to Fable-class models, re-run bench/fo-km Metric-1 (the 16-task
   frozen set, `bench/fo-km/tasks.jsonl`) before re-opening this decision. Source:
   `bench/fo-km/RESULTS.md` (both sections); bead sq-mztg8.5; epic sq-mztg8.

---

## 7. Verified source files (all absolute)

- `/home/ubuntu/sparq/crates/sparq-kb/ontology/pkg/pkg.ttl` — `pkg:Finding`/`confidence`/
  `assurance`/`discoveredFrom`/`couldBeMergedWith`; reuses `prov:`/`cito:`/`fabio:`/`np:`/
  `dcat:`/`skos:`/`dqv:`. **DQV adopted by Phase 3 (sq-2489d.3)**: `pkg:confidence` is now
  `rdfs:subPropertyOf dqv:value` of a named `dqv:QualityMeasurement`, `pkg:assurance` a
  `dqv:Metric`, and the technique-graph leans on `cito:extends`/`cito:usesMethodIn` — the
  §3 gap is closed.
- `/home/ubuntu/sparq/crates/sparq-kb/ontology/pkg/PROVENANCE.md` — reuse table (DQV row +
  *DQV quality model* sub-section + W3C-Note caveat, added by sq-2489d.3)
- `/home/ubuntu/sparq/crates/sparq-kb/src/vocab.rs` — predicate constants (byte-pin test)
- `/home/ubuntu/sparq/crates/sparq-kb/src/query/canned.rs` — `FINDINGS_ABOUT` /
  `FINDING_PROVENANCE` already SELECT confidence + source + assurance
- `/home/ubuntu/sparq/crates/sparq-kb/ingest/agents-findings.ttl` — the high-trust tier
- `/home/ubuntu/sparq/crates/sparq-kb/ingest/pkg-instances.ttl` — **70 Sources, all
  Explored, 0 Unexplored** (the §0 correction)
- `/home/ubuntu/sparq/crates/sparq-kb/ingest/ingest_pkg.py` — extension point for
  `project_literature` + record/replay extract
- `/home/ubuntu/sparq/crates/sparq-kb/shapes/pkg.shapes.ttl` — the SHACL write-gate
- `/home/ubuntu/sparq/crates/sparq-nlq/src/lib.rs` — `Answer`/`Turn`/`NlqConfig` (no
  hedge/citation/row-provenance link yet)
- `/home/ubuntu/sparq/crates/sparq-prov/src/{lib,update,reason}.rs` — `derive_construct` /
  `derive_update` / `prov_from_proof`
- `/home/ubuntu/sparq/crates/sparq-vectors/src/{structure,encode,fuse,grounding,shacl_priors}.rs`
  — P0 sampler / `Block` (no weight field) / weighted fuse / `Modality::Subgraph` /
  the read-schema-into-prior pattern; **no in-tree KGE trainer**
- `/home/ubuntu/sparq/research/dogfooding-sparq-knowledge-graph.md`,
  `/home/ubuntu/sparq/research/agent-effectiveness-program.md`,
  `/home/ubuntu/sparq/research/structure-aware-vectorisation.md`,
  `/home/ubuntu/sparq/research/feature-research-odrl-policy.md`
- `/home/ubuntu/sparq/bench/pkg-dogfood/` — the measure-first PREREG/grade pattern reused
  by the roadmap

### External sources (real, traceable)

CKRL (Xie et al. AAAI 2018); Embedding-with-triple-trustiness (PMC7514427); TrustGNN
(arXiv:2205.12784); interpretable GCN on noisy KGs (arXiv:1812.00279); Trustworthy KG
Reasoning / uncertainty-aware (arXiv:2410.08985); KGE calibration (arXiv:2004.01168);
I-CALM (arXiv:2604.03904); When Silence Is Golden (arXiv:2602.04755); Double-Calibration
(arXiv:2601.11956); ALCE (Gao et al. 2023/2024); citation correctness≠faithfulness
(arXiv:2412.18004); attribution survey (arXiv:2508.15396); Citekit (arXiv:2408.04662);
grounded-attribution + refuse (arXiv:2409.11242); MIRAGE (arXiv:2406.13663); DQV (W3C WG
Note, w3.org/TR/vocab-dqv/); CiTO (sparontologies.github.io/cito); nanopub guidelines +
provenance-driven nanopubs (Springer IJDL 2025; CEUR Vol-3937); DCAT v2/v3 (W3C). Haiku
4.5 pricing/limits/caching verified via the `claude-api` skill.
