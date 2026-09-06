<!-- [OPUS-4.8] sq-8dkyo (epic sq-mztg8): synthesis of four research streams (teach / facade /
hide / coverage) into one design-for-review record. Authored by a SPARQ agent 🤖. -->

# Bridging the foundational-ontology ↔ LLM-fluency gap for the PKG

> 🤖 **SPARQ agent note.** This is a *design-for-review* research record (no implementation).
> It synthesises four investigation streams under epic **sq-mztg8 / bead sq-8dkyo**. Every
> empirical number traces to a `bench/` record (cited, not inlined). Where the brief's premise
> diverged from the verified repo state, the divergence is called out in **§0 Premise check**.

## 0. Premise check — what is actually true on `main` today

The brief assumes some artifacts are already merged. Verified against `origin/main`
(2026-06-21); the corrections matter because they change which evidence is *load-bearing*
vs *pending*:

| Claim in the brief / streams | Verified state on `main` | Consequence |
|---|---|---|
| The FO-KM **Metric-1 result** (schema.org 0.84 ≫ gUFO 0.54 < no-FO 0.58) | **MERGED.** Design doc `research/foundational-ontology-km-benchmark.md` (PR #1106) + harness `bench/fo-km/` + Haiku-subject `RESULTS.md` (PRs #1107/#1108) are all on `main`. A Fable-subject re-run (PR #1603) found schema.org ties DOLCE-DUL 0.56/0.56 at Fable tier (schema.org retains an expressibility-subset edge); the Haiku-tier result stands. FO choice ratified as schema.org-as-top (bead sq-mztg8.5; `research/provenance-driven-genai-kb.md` §6 item 5). | The headline result is canonical and load-bearing. Cite `bench/fo-km/RESULTS.md` directly. |
| `crates/sparq-terse` (the `V()` transpiler facade) is design-only (PR #1074) | **MERGED** as a skeleton (PR #1088, sq-leg8n): `terse_to_sparql(src) -> Expansion { canonical_sparql, resolutions, warnings }`, `TerseError::CanaryFailed`, lexical-first + vector-fallback `V()`. The *design* (#1074) is still open. | Direction 2's query-side facade is partly **built**, not just proposed. Phases past the skeleton are future beads. |
| Full JSON-LD 1.1 compaction needed for a clean author `@context` is "sq-ixc3.4, not yet built" | **BUILT.** sq-ixc3.4 is CLOSED — full W3C JSON-LD 1.1 Compaction shipped dependency-free in `crate::serialize::compact` behind `serialize-rdf` (PR #950). | The write-path `@context` story (§2.4) is *less blocked* than the stream claimed: compaction exists; what is missing is a *consumer* + the PKG-authoring `@context` itself. |
| Metric-2 (gUFO closure-prior) "could rank a rich FO higher" | True but weaker than implied. sq-p5ro8 records the lift is **direction-unstable / sign-flips across synthetic slices**; variance reduction (#1094 paired-delta) did **not** firm it. | The rich-FO backbone's payoff is *not merely unmeasured — it is currently unsupported by the synthetic evidence*. The benchmark plan (§3) must treat the backbone's value as a falsifiable hypothesis with a real-data resolution, not a near-certainty. |
| `sec-prop.yaml.ld` YAML-LD authoring precedent exists in-tree | **CONFIRMED.** `crates/sparq-trust/ontologies/zkp-sparql/vocab/*.yaml.ld` (typed `@context` + `@graph`, render scripts, SHACL-validated). | The write-path design (§2.4) generalises a real, shipped pattern — not a green-field invention. |
| The PKG already ships facade→backbone bridge axioms | **CONFIRMED** in `crates/sparq-kb/ontology/pkg/pkg.ttl`: `rdfs:subClassOf` / `rdfs:subPropertyOf` / `owl:inverseOf` / `skos:closeMatch` bridges onto `schema:` / `prov:` / `dcterms:` / `fabio:`. | The facade pattern is largely *implemented* for the schema side; the recommendation is additive, low-risk. |

**Net premise correction.** The brief frames this as "design three new bridges." In reality
**two of the three directions are already partly built and the rich-FO backbone's value is
currently un-evidenced (sign-unstable), not merely unmeasured.** That sharpens, rather than
weakens, the recommendation below.

## 1. Problem framing

The maintainer's **FO-KM Metric 1** (agent KM-task accuracy + cost over the dogfooded PKG,
four FO overlays, Haiku NL-tool, N=16 FO-exercising tasks) found — see the `bench/fo-km/`
record (PRs #1107/#1108) — that the **fluent web vocabulary `schema.org` markedly beat the
metaphysically rich `gUFO`, and `gUFO` scored *below* doing nothing.** The measured driver is
**LLM fluency of the vocabulary, not formal richness**: an agentic LLM grounds ubiquitous
`schema:` terms reliably and fumbles rare academic `gufo:` / `dul:` terms. This is corroborated
by the prevalence law (LLMs4OL; ID-prevalence→accuracy) and explained mechanistically by
**Köhler & Neuhaus** ("The Mercurial Top-Level Ontology of LLMs", arXiv:2405.01581 / FOIS 2024):
ontology terms have fixed compositional semantics; LLMs emit tokens stochastically by context,
so asking a small model to internalise and apply a formal FO's categorial distinctions in-context
fights the model's nature — the instability is **structural, not a tuning problem**, and small
models have the least capacity to suppress it.

**The tension to resolve.** A rich FO buys *structural* benefits the no-FO incumbent lacks —
disjointness/rigidity axioms, principled subsumption for type-hierarchy (TH) / entity-resolution
(ER) / cross-cutting (CC) queries, an interop pivot for heterogeneous sources, and a closure
target for the reasoner. Those benefits accrue to **deterministic symbolic layers** (reasoner,
validator, alignment hub) and to **humans/interop** — *not* to the LLM read-path, where the same
rich FO is a net negative. The goal of this record: **get the FO's structural benefits without
paying the fluency penalty on the agent's read-path.**

## 2. The spectrum: how much ontology to expose

The maintainer's three directions are not alternatives — they are **points on one axis: how much
of the rich FO the *agent* is asked to see and wield.** Reframing them as a spectrum makes the
honest verdict visible.

```text
  MORE ontology exposed to the LLM  ───────────────────────────►  LESS
  ┌─────────────────────┐   ┌──────────────────────┐   ┌──────────────────────┐
  │ (1) TEACH           │   │ (2) FACADE           │   │ (3) HIDE             │
  │ FO in the prompt /  │   │ fluent surface for   │   │ no URIs/ontology in  │
  │ schema-card / few-  │   │ the agent; rich FO    │   │ the loop at all;     │
  │ shot priming        │   │ confined to the       │   │ vector concept-      │
  │                     │   │ reasoner/validator    │   │ resolution + NL-out  │
  └─────────────────────┘   └──────────────────────┘   └──────────────────────┘
        ▲ fluency penalty           ▲ value-loci split           ▲ grounding-only
        is PAID per query           agent↔reasoner               surface to LLM
```

### 2.1 Direction (1) — Teach the small model the FO

**Verdict: REJECT the formal-FO variant as primary; KEEP a reframed, fluent variant as a
default add-on.** Split the question into two layers that behave *oppositely*:

- **Formal-FO primer (gUFO/BFO/DOLCE in the prompt): net-negative for a small model.** It pays
  tokens on every query, is grounded unreliably by cheap models, is destabilised by the
  Köhler–Neuhaus effect, and in the FO-KM measurement *lowered* accuracy below the no-FO
  incumbent. The verbosity tipping point is real and measured elsewhere: KGQA prompt-engineering
  studies find accuracy falls past ~5 few-shot exemplars and that *only the relevant* schema
  segment helps — a verbose complete schema hurts (PMC11770024; arXiv:2507.22619;
  arXiv:2410.09244 "reveal just enough").
- **Fluent, data-derived domain schema-card + dynamic few-shot exemplars: a real, large,
  repeatable lever** — but it is *domain-vocabulary* priming, not *foundational-ontology*
  priming. Removing the domain ontology dropped KGQA accuracy 51%→13% (−26pp, PMC11770024);
  ontology-as-validator drove 16.7%→54.2%→72.6% (Allemang & Sequeda, arXiv:2405.11706, where the
  ontology is a *post-hoc check*, not prompt-stuffing). This is exactly the
  `schema_summary_for(seeds)` retrieval-bounded card already designed in
  `research/genai-ontology-introspection.md`.

The single highest-ROI *validity* guarantee is not a primer at all: **post-hoc `spargebra`
validation** (free in sparq — the engine already embeds it) and, on local backends only,
grammar-constrained decoding give 100% syntactic validity regardless of model size. (Grammar
constraint is **not** available on the Anthropic Messages API — no logit/grammar access — so
for the hosted Haiku tool the lever is post-parse validation + the `constrain.rs` did-you-mean
repair loop, not decoder-level constraint.)

This verdict is consistent with the FO-KM design doc's own §6.3 conclusion: *put FO value in the
post-hoc reasoner/validator, not the LLM prompt.*

### 2.2 Direction (2) — Fluent facade over a rich FO backbone

**Verdict: LEADING candidate, and it is the natural home for the FO-KM result.** The measured
bifurcation — fluent surface wins for the LLM (Metric 1), rich-FO value (if any) accrues to the
reasoner (Metric 2) — *is* the facade/backbone thesis. The two need not be the same ontology.
sparq already ships every layer (§2.3). Honest counter-pressure: the backbone half of the thesis
(that a rich FO pays off in the reasoner) is currently **un-evidenced** — sq-p5ro8's synthetic
lift is sign-unstable. So Direction (2) is well-founded on its *facade* half and *speculative*
on its *backbone* half until Metric 2 runs on real data.

### 2.3 Direction (3) — Hide the ontology + URIs

**Verdict: STRONG complement to (2), ~70% built on the read-path, and the cleanest realisation
of "fluency penalty → zero" because the agent sees no FO vocabulary at all.** The read-path
composes existing seams (`sparq-nlq::link` → `sparq-vectors` `vec:nearest` → `grounding.rs`
`OutputType::Text/Value` → `verbalize`) behind the #1074 echo/confidence envelope; the write-path
generalises the shipped `sec-prop.yaml.ld` YAML-LD pattern into a PKG authoring form that
*deterministically* compiles to FO triples. Honest limit: the LLM can still hallucinate the
*query shape* (wrong predicate) even with URIs hidden, and real-model NL→SPARQL accuracy in this
loop is **unmeasured** (only record/replay fixtures exist).

### 2.4 The honest combined verdict

> **The evidence favours (2)+(3) combined over (1)-as-primary.**

- **Against (1)-as-primary:** the FO-KM result (gUFO 0.54 < no-FO 0.58 < schema.org 0.84) +
  Köhler–Neuhaus (the formal FO is structurally unstable in-context for a cheap model). Putting
  a formal FO in the prompt is the one move the measurement directly punishes. Cite:
  `bench/fo-km/RESULTS.md` (PRs #1107/#1108); arXiv:2405.01581.
- **For (2)+(3):** the facade keeps the agent on fluent terms (where schema.org already wins),
  the backbone keeps formal power for the *deterministic* layers (reasoner/validator/alignment
  hub) the LLM never touches, and hiding URIs drives the agent-facing FO-fluency penalty toward
  zero while the write-path compiler keeps the stored graph FO-conformant. (1)'s *surviving*
  good idea — a fluent retrieval-bounded schema-card + dynamic few-shot — is *subsumed* into the
  (2) facade read-path; only its *formal-FO* form is rejected.

## 3. Recommended sparq architecture

A four-layer split that confines the rich FO to deterministic/interop layers and presents only
fluent surfaces to the agent. **All layers are opt-in crates/features** (consistent with the
repo's opt-in-feature architecture); none touches `sparq-core`/`spargebra` or the parser.

```text
                         ┌────────────────────────────────────────────┐
   AGENT (LLM)  ◄──NL/────│  READ-PATH FACADE  (fluent, URI-optional)   │
                NL+typed  │  • fluent vocab card (schema.org/gist)      │
                values    │  • V() concept-resolution  (sparq-terse)    │
                          │  • grounding → NlString/TypedValue (hide)   │
                          └───────────────┬────────────────────────────┘
                                          │  bridge axioms (subClassOf /
                                          │  subPropertyOf / equivalentClass)
                          ┌───────────────▼────────────────────────────┐
   REASONER/VALIDATOR ────│  RICH-FO BACKBONE  (deterministic, hidden)  │
   (no LLM judgement)     │  • FO overlay loaded ONLY under --close     │
                          │  • sparq-reason OWL-2-RL materialisation     │
                          │  • SHACL validation (sparq-shacl)           │
                          └───────────────┬────────────────────────────┘
                                          │  YAML-LD → Turtle compile
                          ┌───────────────▼────────────────────────────┐
   AUTHOR / write-time ───│  WRITE-PATH  (structured-markdown authoring) │
   (human or schema-      │  • typed YAML-LD block, no IRIs/xsd          │
   constrained LLM)       │  • V()-resolved concept TOKENS              │
                          │  • deterministic compiler → FO triples      │
                          │  • SHACL as the compile gate                │
                          └─────────────────────────────────────────────┘
```

### 3.1 Rich FO confined to the reasoner/validator (the backbone)

The rich FO lives in an **FO overlay loaded only under `--close owl-rl`** (the
`bench/fo-km/overlays/*.ttl` mechanism, PR #1107), never in the agent's prompt. `sparq-reason`'s
forward-chaining OWL-2-RL materialisation (`materialize_owl_rl`, with the `cax-eqc*` /
`scm-*` / `prp-inv*` bridge rules and `inconsistencies()`, all verified in
`crates/sparq-reason/src/owl.rs`) entails the FO-typed facts so structural (TH/ER/CC) queries
resolve under closure. SHACL (`sparq-shacl`) validates conformance. **No LLM judgement enters
this layer** — it is the deterministic answer to the Köhler–Neuhaus instability: push formal
structure into run-independent layers.

### 3.2 Fluent facade for the agent read-path

The agent grounds onto and queries **fluent `schema:` / `pkg:` terms only**. The PKG already
*is* a reuse-first fluent facade — `crates/sparq-kb/ontology/pkg/pkg.ttl` bridges fluent terms
onto richer patterns via standard W3C constructs (`pkg:Task ⊑ schema:Action`,
`pkg:about ⊑ dcterms:subject`, `pkg:discoveredFrom ⊑ prov:wasDerivedFrom`, status enums
`skos:closeMatch schema:*ActionStatus`, `pkg:dependsOn owl:inverseOf pkg:blockedBy`). The
retrieval-bounded schema-card (`schema_summary_for(seeds)` from
`research/genai-ontology-introspection.md`) is the *only* ontology context the agent ever
sees, and it carries fluent vocabulary, not the FO. **gist is an unmeasured facade candidate**
(see §4): if the PKG ever wants a coherent ~135-class enterprise facade (Person/Organization/
Agreement/Event/Commitment — deliberately metaphysics-free, the same fluency property
schema.org won on) rather than reusing scattered vocabularies, gist is the named alternative
to "schema.org-as-top".

### 3.3 Vector concept-resolution to hide URIs

`V("phrase")` resolves a fluent NL phrase to a backbone IRI **lexical-first** (deterministic
`sparq-nlq::link` over `rdfs:label`/`skos:prefLabel`/`schema:name`/…), **vector-fallback**
(`sparq-vectors` `vec:nearest`, staleness-guarded by the graph-fingerprint contract in
`rewrite.rs`). The IRI stays an opaque dict-id; the agent sees only the label it typed plus a
confidence. On the answer side, results route through `grounding.rs` —
`OutputType::Text → Modality::NlString` (token-budgeted `verbalize`) or
`OutputType::Value → Modality::TypedValue` — so `local_name(iri)` / enum-label strip every IRI
to a human label before it reaches the LLM. **Ambiguity defaults to the exact re-validated
subgraph** (`OutputType::Ambiguous → Subgraph`), never an approximate signal.

**The governing soundness envelope (from sparq-terse §6 / #1074), enforced both read and write:**

1. **Always echo the resolution** — resolved label + similarity + runner-up + confidence +
   method, even though the IRI stays hidden, so a silent mis-resolution is *visible*.
2. **Lexical-first, vector-fallback** — exact label match outranks any cosine hit; approximate
   never auto-accepts.
3. **Confidence-gate / abstain** — below threshold (near-tied candidates / low cosine) return the
   candidate list and abstain (empty-result ≠ wrong).
4. **Staleness guard** — fingerprint-keyed vector store; a stale index is caught, never served;
   reopen via `Graph::open` (frozen id order), never re-parse.
5. **Silent-rewrite canary** (`TerseError::CanaryFailed`) — re-parse the canonical expansion
   under `spargebra`; loud-fail rather than silently rewrite intent.

*"A convenience that shows its work, never an oracle that hides it."*

### 3.4 Structured-markdown authoring (write-path): a YAML-LD-like spec sketch

The write-path generalises the shipped `sec-prop.yaml.ld` pattern (typed `@context` + `@graph`,
render scripts, SHACL-validated) into a **PKG authoring form** so neither author nor LLM ever
writes raw RDF or IRIs. A deterministic compiler — *not* an LLM — turns the typed block into
FO-conformant triples; the SHACL shape (`crates/sparq-kb/shapes/pkg.shapes.ttl`) is the only
admission control. Sketch:

```yaml
# pkg-findings.yaml.ld  — authored against the PKG @context; no IRIs, no xsd:
"@context":
  "@vocab": "https://w3id.org/sparq/pkg#"
  finding:    { "@id": "rdfs:label",          "@language": "en-GB" }
  about:      { "@id": "pkg:about",           "@type": "@id" }   # concept TOKEN
  source:     { "@id": "pkg:discoveredFrom",  "@type": "@id" }   # concept TOKEN
  verdict:    { "@id": "sigimpl:verdict" }
  confidence: { "@id": "pkg:confidence",      "@type": "xsd:decimal" }
  assurance:  { "@id": "pkg:assurance" }
"@graph":
  - "@type": pkg:Finding
    finding: "Characteristic sets serve planner + LLM grounding"
    about: characteristic-sets        # a TOKEN; V() resolves to the skos:Concept IRI
    source: genai-ontology-introspection
    verdict: "yes"
    confidence: 0.9
    assurance: claimed
```

**Compile contract (deterministic, auditable):**

1. **Parse** the typed block (a new reader alongside `ingest_pkg.py`'s existing
   `parse_frontmatter`/`project_skills` mechanical projectors).
2. **Resolve concept TOKENS via the same guarded `V()` resolver** as the read-path
   (`about: characteristic-sets` → nearest `skos:Concept` IRI). An ambiguous token is a
   **hard compile error** (the write-path analogue of the read-path abstain) — never a silent
   wrong-IRI. The same echo/confidence envelope guards write-time mis-resolution.
3. **Compile to Turtle** deterministically using the `@context` (which maps the fluent author
   keys to FO predicates) — full JSON-LD 1.1 compaction now exists in-tree
   (`crate::serialize::compact`, behind `serialize-rdf`, sq-ixc3.4 CLOSED via #950), so a clean
   author-facing `@context` is no longer blocked; what remains is the PKG `@context` artifact +
   wiring `ingest_pkg.py` to call the compiler instead of appending raw Turtle.
4. **SHACL-validate the compiled output** against `pkg.shapes.ttl`; reject on violation. The
   existing ingest already runs to 0 violations and side-cars stale edges — keep that gate.
5. **Round-trip property:** compile → re-expand to RDF reconstructs the same triples (the
   inverse-of-toRdf property the JSON-LD writer is already tested on).

This generalises the script's existing "EXTRACTION METHOD = STRUCTURED-PARSE (deterministic,
not LLM)" posture: the *author or a schema-constrained LLM* produces the typed block; a
**deterministic compiler** turns it into FO triples, so ingestion provenance stays auditable and
the rich FO never has to be authored by hand. It would **replace the hand-authored
`agents-findings.ttl` raw-Turtle tier** with an authored YAML-LD source the script compiles.

## 4. Broadened-coverage benchmark plan (Metric 1 here; Metric 2 EC2-deferred)

The current benchmark samples only the *middle* of the size axis: gUFO (51) + DOLCE-DUL +
schema.org (a web vocab, not a true FO) + a no-FO control. It has **no realist pole, no
lean-FO lower anchor, and no pragmatic-enterprise facade.** This plan (bead sq-givgo, round 2)
broadens coverage across the **size axis** and the **realism↔descriptivism fork**.

### 4.1 The size axis and which FO each direction wants

Verified/corrected counts (the survey stream's figures had drift):

| FO | Size (verified) | Pole | Recommended role |
|---|---|---|---|
| **BFO** (ISO/IEC 21838-2) | **36 classes** (survey said ~35) | Realist, 3D/4D | **Lean-FO lower anchor + realist pole + cheapest backbone.** The minimal-commitment control for the vectorisation prior — if a 36-class rigid/disjoint spine moves the reasoner metric, the prior is real; if only gUFO's 51-class machinery does, the cost story changes. Most likely real FO to appear in *wild* OBO/biomed data. |
| **gUFO** | **51 classes** (survey "~51–67"; 51 confirmed) | Conceptual | The existing named baseline; keep. Best formal fit of the rich options, worst LLM-fluency (measured). |
| **DOLCE-DUL** | DUL ~80 (survey "~100" high for *core* DOLCE) | Descriptive | Keep; the descriptivist counterpoint. Edged the incumbent in Metric 1. |
| **gist** | **~135 classes** (survey "~200" too high) | Pragmatic-enterprise | **The missing pragmatic-middle + fluent facade candidate.** OWL-2-DL native (no SUO-KIF/CycL conversion loss), everyday names, matches the PKG's own task/agreement/provenance shape. *Honest caveat: single-vendor (Semantic Arts), shallow metaphysics.* |
| **SUMO** | ~25k terms / ~80k axioms (SUO-KIF, lossy to OWL-DL) | Broad | **Acknowledged-too-big reference marker, not a run target** (higher-order arity drops in the OWL approximation). |
| **Cyc / OpenCyc** | millions of axioms; OpenCyc retired | Commonsense KB | **Documented-impractical** (licensing + scale). |

**Per direction:** Direction A/facade wants **BFO + gUFO + DOLCE-DUL** (rich+formal enough to
constrain, small enough for a token-budgeted card) plus **gist** as the fluent-facade candidate;
**reject SUMO/Cyc as facade** (too big, lossy). Direction B/vectorisation-prior wants
**gUFO + BFO** (BFO as the minimal-commitment control). Direction C/alignment-hub wants
**gist as the fluent surface anchored to a BFO or DOLCE realist/descriptivist hub** — and note
the matching literature is conditional: upper ontologies improve matching F-measure *only when
the bridge ontology is large* (Mascardi et al.), so a *small verified hub + documented-lossy
alignments* beats a giant FO here.

**Resulting span (four runnable OWL artifacts + reference markers):**
`BFO (36, realist) → gUFO (51, conceptual) → DUL (~80, descriptive) → gist (~135, pragmatic)`,
with `[SUMO ~25k]` and `[Cyc]` as acknowledged-too-big markers. This covers the size axis and
both poles of the realism/descriptivism fork.

### 4.2 Metrics

- **Metric 1 — agent KM A/B (runnable here, on this work-box).** Re-run the
  `bench/fo-km/` Haiku NL-tool A/B (PR #1107 harness) with the broadened arm set over the same
  PKG + overlay + `--close owl-rl`. Add a **gist arm** and a **BFO arm** as overlays under
  `bench/fo-km/overlays/`. Report the per-task-kind (TH/ER/CC) breakdown and effective token
  cost. *Work-box timings/costs are NON-canonical — record them in the bench record, never inline
  in docs.*
- **Metric 2 — reasoning/KGE closure-prior (EC2-deferred, sq-p5ro8).** The `gufo_prior` ablation
  axis in `crates/sparq-vectors/src/eval.rs` (paired-delta machinery, #1094) measures formal-
  reasoning quality the agent never touches. **This is the half that would justify keeping a rich
  FO at all — and it is currently un-evidenced: the synthetic lift sign-flips across slices
  (sq-p5ro8).** It must be settled on a **real schema-bearing KG** (`SPARQ_KGE_DATASET`) on a
  **canonical/quiet machine**, not on this work-box. Add BFO as a minimal-commitment arm here.
- **(Optional) Metric 3 — LLM ontological-commitment stability.** Per Köhler & Neuhaus
  (scoped to GPT-3.5; re-run per model, do not assume frontier models behave identically),
  instrument whether grounding against an explicit FO *reduces* cross-session ontological
  contradiction vs the ungrounded LLM. This directly tests the "FO-as-external-scaffold" thesis
  rather than only exec-accuracy. Lower priority; flag as a stretch arm.

### 4.3 Pre-registered kill-criteria (falsifiable — register BEFORE running)

The whole design is a hypothesis; pre-register the conditions under which each part is
**rejected**, so the result cannot be rationalised after the fact:

- **K1 (facade > incumbent).** If, on the broadened Metric-1 run, **no fluent facade arm
  (schema.org or gist) beats the no-FO incumbent by ≥ a pre-set margin** (margin + N + grading
  protocol fixed in the bench record before running), then *Direction 2's facade premise fails*
  → the recommendation collapses to "no FO; keep the no-FO incumbent + post-hoc validation."
- **K2 (gist vs schema.org).** If gist does **not** match-or-beat schema.org, gist-as-named-facade
  is dropped and schema.org-as-top stays the facade.
- **K3 (rich-FO backbone).** If Metric 2 on a **real** schema-bearing KG shows the rich-FO
  closure-prior lift is **null or negative** (consistent with the current sign-unstable synthetic
  signal), then *the rich-FO backbone is rejected entirely* — there is no reasoner payoff to
  justify it, and the architecture simplifies to facade + hide over a *thin* type spine (or none).
- **K4 (hide read-path).** If a real-model NL→NL A/B over the hidden-URI loop does **not**
  match-or-beat the URI-visible NL→SPARQL path on answer accuracy, *URI-hiding is a no-op
  complexity cost* and is dropped (the read-path keeps grounding only).
- **K5 (write-path).** If the deterministic YAML-LD compiler cannot hold SHACL conformance at
  parity with the current hand-authored tier (0 violations) on a representative corpus, the
  write-path generalisation is deferred.

**No criterion is satisfied by the work-box alone:** K1/K2/K4/K5 run here (Metric 1 / fixtures /
compile-gate); K3 is **EC2/canonical-box only** and is the gate on the rich-FO backbone.

## 5. Honest stance — the design is falsifiable, not assumed-better

- **Do not assume any of this beats the no-FO / schema.org incumbent.** The *only* arm that has
  beaten the incumbent in a real measurement is **schema.org-as-top (a fluent facade)**, and that
  result is N=16, single run, heuristic grading, and **not yet on `main`** (PRs #1107/#1108). The
  gap is large (0.30) and consistent across TH/ER/CC, so it is robust to grading noise — but it is
  one measurement, not a law.
- **The rich-FO backbone is the weakest link.** Its justification (Metric 2 reasoner payoff) is
  *currently un-evidenced* — sq-p5ro8's synthetic lift sign-flips. K3 must pass on real data or
  the backbone is rejected, in which case the architecture is *facade + hide over a thin/no type
  spine* — still a coherent, honest design, just without the rich FO.
- **No privacy/soundness claim is made.** Nothing here is a ZK/MPC property; the design is RDF
  authoring + reasoning + vector grounding only.
- **No hard-coded performance numbers** appear in this doc; all figures cite the `bench/fo-km/`
  record and the named beads. Work-box timings are non-canonical.

## 6. Phased plan (each phase = a future bead)

1. **Broaden Metric-1 coverage (sq-givgo).** Add **gist** and **BFO** overlays under
   `bench/fo-km/overlays/`; re-run the Haiku A/B over the broadened arm set; report per-task-kind
   + cost into the bench record. *Pre-register K1/K2 first.* (Depends on PR #1107 harness landing.)
2. **Settle Metric-2 on a real KG (sq-p5ro8).** Run the `eval.rs` closure-prior ablation
   (gUFO + BFO arms) over `SPARQ_KGE_DATASET` on a canonical box; resolve K3. **This is the gate
   on whether the rich-FO backbone survives at all.**
3. **Read-path URI-hiding compose-and-A/B (new bead).** Wire `link → vec:nearest →
   grounding(Text/Value) → verbalize` behind the #1074 echo/confidence envelope into a closed
   NL→NL loop; A/B it against the URI-visible NL→SPARQL path on real-model answer accuracy;
   resolve K4. (Depends on a real NL→SPARQL eval harness — flag the unmeasured-accuracy gap.)
4. **Write-path YAML-LD authoring form (new bead).** Define the PKG authoring `@context`; add a
   typed-block reader + deterministic compiler to `ingest_pkg.py` (replacing the raw-Turtle
   `agents-findings.ttl` tier); resolve concept TOKENS via the guarded `V()` (ambiguous = hard
   compile error); keep `pkg.shapes.ttl` as the compile gate; resolve K5. (Generalises
   `sec-prop.yaml.ld`; JSON-LD compaction already shipped.)
5. **Facade selection (depends on Phase 1).** If gist wins K2, promote a coherent gist (or
   schema.org) facade for the PKG read-path; otherwise keep schema.org-as-top. Update
   `pkg.ttl` bridge axioms + the schema-card generator accordingly.

   > **RESOLVED — the "otherwise" branch (bead `sq-mztg8.4`).** The facade **stays
   > schema.org-as-top**, taken on the evidence rather than by preference. K2 has never
   > been able to fire: Phase 1 (`sq-givgo`) has **not landed** — `bench/fo-km/overlays/`
   > carries only `no-fo` / `gufo` / `dolce-dul` / `schema-org`, with **no gist (or BFO)
   > overlay** — so **gist has never been measured** against this PKG at any model tier,
   > and cannot have matched-or-beaten schema.org. Metric 2 (`sq-p5ro8`) remains
   > EC2-deferred and sign-unstable, and in any case reads on the rich-FO **backbone**
   > (K3), not on the facade. This is consistent with the independent
   > schema.org-as-top ratification of `sq-mztg8.5`
   > (`research/provenance-driven-genai-kb.md` §6 item 5).
   >
   > **What changed, concretely.** `pkg.ttl`'s bridge axioms are *unchanged* (they already
   > assert the schema.org facade). The real gap was the second half of the acceptance
   > clause: the schema-card generator showed the agent **none** of the facade. The PKG's
   > introspect card is the canned `schema-classes` / `schema-properties` pair, and
   > `schema-classes` filters to `STRSTARTS(STR(?class), STR(pkg:))` — so the ratified
   > `schema:` vocabulary, which Metric 1 says the agent grounds *most* reliably, was
   > invisible at introspect time. A new `facade-terms` canned query
   > (`crates/sparq-kb/src/query/canned.rs`) now surfaces the asserted
   > `rdfs:subClassOf` / `rdfs:subPropertyOf` / `skos:closeMatch` bridges from `pkg:` onto
   > `schema:`. It is **additive** — `schema-classes` is untouched, because its
   > `pkg:`-only filter is load-bearing (§7 of the FO-KM design record notes the class
   > card gets *noisier* under closure), so there is no canned-query or SHACL regression.
   >
   > **Reopen trigger:** `sq-givgo` lands a measured gist arm. Note that promoting gist is
   > *also* a facade-identity **values call** (§7 open question 3) needing maintainer
   > steer — so even a K2 win would not be self-executing. [OPUS-5]
6. **(Stretch) LLM ontological-commitment stability probe (new bead).** Instrument Metric 3
   (Köhler–Neuhaus-style cross-session contradiction), re-run per model.

## 7. Open questions that genuinely need the maintainer

1. **Kill-margins.** What absolute margin (over the no-FO incumbent) and what N / grading
   protocol should K1/K2 pre-register? The current N=16 heuristic-graded run is suggestive, not
   decisive; a semantic-grader + larger N may be warranted before promoting a facade.
2. **Backbone-survival default.** If K3 (real-data Metric 2) is null/negative, do you want the
   architecture to keep a *thin* type spine (e.g. schema.org-as-top under closure for TH/ER/CC
   structural queries) or drop the FO backbone entirely? The streams assume a rich backbone; the
   evidence may not support one.
3. **Facade identity.** Reuse-first scattered vocabularies (today's `pkg.ttl`) vs a single
   coherent named facade (gist / schema.org-as-top)? This trades interop breadth against author
   cognitive load and is a values call, not purely empirical.
4. **Authoring surface.** Should the write-path YAML-LD be a *standalone* `*.yaml.ld` source (the
   `sec-prop` shape) or *typed frontmatter on existing markdown* docs (so a research doc carries
   its own findings block)? The latter is more dogfood-natural but couples authoring to doc churn.

## Sources

**Local (verified):**

- `bench/fo-km/RESULTS.md` + `bench/fo-km/overlays/*.ttl` — MEASURED Metric-1 A/B
  (PRs #1107/#1108, **not yet on `main`**).
- `research/foundational-ontology-km-benchmark.md` (PR #1106, on `main`) — design; §6.3 puts FO
  value in the reasoner/validator not the prompt; §7 LLM-fluency-tracks-prevalence.
- `crates/sparq-kb/ontology/pkg/pkg.ttl` — the shipped reuse-first facade + bridge axioms.
- `crates/sparq-terse/src/{lib,transpile,resolve,error}.rs` — the verifiable `V()` transpiler
  facade (PR #1088, on `main`); `research/llm-ergonomic-sparql-surface.md` (PR #1074, design).
- `crates/sparq-reason/src/owl.rs` — `materialize_owl_rl` (cax-eqc/scm-*/prp-inv bridge rules).
- `crates/sparq-vectors/src/{grounding,rewrite,labels,verbalize}.rs` — modality dispatcher,
  `vec:nearest` + staleness guard, label index; `crates/sparq-vectors/src/eval.rs` `gufo_prior`
  ablation axis.
- `crates/sparq-kb/ingest/ingest_pkg.py`, `crates/sparq-kb/shapes/pkg.shapes.ttl`,
  `crates/sparq-trust/ontologies/zkp-sparql/vocab/sec-prop.yaml.ld` — the write-path: structured-
  parse ingest, SHACL gate, and the YAML-LD authoring precedent.
- `research/genai-ontology-introspection.md` (retrieval-bounded schema-card),
  `research/genai-nl-to-sparql.md` (post-hoc `spargebra` validity), `research/jsonld-support-roadmap.md`.
- Beads: sq-mztg8 (epic), sq-givgo (round 2), sq-p5ro8 (Metric 2, sign-unstable),
  sq-ixc3.4 (JSON-LD compaction — CLOSED/shipped, #950).

**External:**

- Köhler & Neuhaus, *The Mercurial Top-Level Ontology of LLMs*, FOIS 2024 —
  <https://arxiv.org/abs/2405.01581> · <https://doi.org/10.1177/15705838251336685>
- *Increasing LLM Accuracy for QA: Ontologies to the Rescue!* (Allemang & Sequeda) —
  <https://arxiv.org/html/2405.11706v1>
- *Evaluating prompt engineering for KGQA* — <https://pmc.ncbi.nlm.nih.gov/articles/PMC11770024/>
- *Progressively revealing ontologies* — <https://arxiv.org/abs/2410.09244> · *Context-aware
  prompting (manufacturing KGQA)* — <https://arxiv.org/html/2507.22619v1>
- *Assessing SPARQL capabilities of LLMs* — <https://arxiv.org/html/2409.05925v2>
- LLMs4OL — <https://arxiv.org/html/2409.10146v1> · ID-prevalence→accuracy —
  <https://arxiv.org/pdf/2409.13746>
- gist (Semantic Arts) — <https://www.semanticarts.com/gist/> · BFO/ISO 21838-2 —
  <https://iso.org/standard/74572.html>
- BFO↔DOLCE mapping (Grenon/Smith) — <https://pubmed.ncbi.nlm.nih.gov/20841847/> ·
  *Feasibility of Automated Foundational Ontology Interchangeability* —
  <https://semantic-web-journal.net/system/files/swj723.pdf>
- Mascardi et al., *Automatic Ontology Matching via Upper Ontologies* (IEEE).
