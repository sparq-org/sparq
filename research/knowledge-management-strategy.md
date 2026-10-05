# Knowledge management: documents as projections of a queryable graph [OPUS-5]

**Status:** design record, for maintainer review. Requested 2026-07-26 as the long-term extension of
`research/agent-context-sharing.md`: *"design a knowledge management strategy for this codebase so that as
much of this codebase as possible can be put into a queryable knowledge graph rather than living in
disparate un-maintained documents throughout the codebase … reducing information duplication; long comments
in documents; letting agents assemble a short relevant context of the project based on the work they are
doing; avoiding having anything manually written that can be generated from benchmarks etc."*

## 0. Grounding rules (binding on every claim here)

1. **This project has measured most plausible context ideas, and rejected two of three.** `pkg-query`
   halved Opus tokens (N=30) → adopted. Outline-first cost **~21k tokens MORE** than a scoped read →
   rejected. `sparq-terse` saved **~0%** → rejected. The prior is that a KM idea which sounds good is more
   likely than not to be worthless. **Nothing here ships unmeasured.**
2. No hard-coded performance numbers in prose (repo hygiene). Figures below are dated corpus measurements,
   not benchmark results.
3. Every figure in §1 was measured against the tree on 2026-07-26.

## 1. The measured problem

**Corpus:** 640 tracked markdown files, **140,807 lines**. `research/` alone is **275 files / 85,033
lines** — 60% of it. `skills/` 37 files, crate READMEs 76 files, `docs/` 11, `AGENTS.md` 682 lines.

**Duplication is severe and safety-relevant.** The single fact *"the external accredited-cryptographer ZK
audit (`sq-qhy4`) is pending, so no ZK security property may be relied on"* is restated across **146
files**. It is a P0 gate. When it changes — one event, one day — **146 files become wrong simultaneously**,
and nothing will tell us which. The architecture principle `opt-in` appears in **299 files**; `RDFox` in 39.

**Staleness is already the norm.** Of 275 `research/` records, **128 were last touched in 2026-06** — a
month or more ago, in a repo landing ~85 commits/day. Roughly half the design corpus describes a codebase
that no longer exists, with nothing marking which half.

**The mechanism we need already exists, for exactly one class.** `check-no-perf-numbers.py` is a HARD gate
enforcing "never hand-write a number a benchmark can generate". It works. The strategy below is the
generalisation of that one proven rule to every derivable fact.

## 2. The reframe: a document is a projection, not a source

Today prose is authored, and facts are embedded inside it. Invert this:

> **Facts live in the graph. Documents are generated views of the graph.**

Two corollaries do most of the work:

- **Generation is one-way: graph → document.** We do **not** parse prose into triples. Prose-to-graph
  extraction is lossy, produces a two-way sync problem, and silently manufactures false facts — the
  failure mode of most doc-KG projects. Nothing enters the graph by being written down in a paragraph.
- **Hand-written prose that duplicates a graph fact is deleted, not synchronised.** If a doc needs the
  `sq-qhy4` gate status, it includes a *generated block*; it does not restate it. Deletion is the point —
  the 146 restatements should become 146 transclusions of one node.

## 3. The taxonomy that makes this tractable

The reason KM projects fail is attempting to graph everything. Split the corpus three ways by
**derivability**, and treat each differently:

| Class | Definition | Where it lives | Hand-writing allowed? |
|---|---|---|---|
| **Derived** | computable from code, benchmarks, CI, git, or the API | generated **only** | **Never** — CI-enforced |
| **Asserted** | must be stated, but is structured | graph triples + provenance | Only as structured input |
| **Narrative** | genuinely irreducible argument | prose, addressable + linked | Yes — this is what prose is *for* |

**Derived** — crate inventory, feature flags, public API surface, dependency edges, benchmark results,
competitor comparisons, conformance pass rates, coverage floors, CI lane inventory, workspace layout,
release versions. Anything a script can produce. This class directly answers *"avoid having anything
manually written that can be generated from benchmarks etc."*

**Asserted** — decisions and their rationale, verdicts, gates and blockers, invariants, constraints,
ownership, supersession, status, "do not do X because Y". These need a human or agent to state them, but
they are **facts with a shape**, not paragraphs. `sq-qhy4` is asserted: *(gate) blocks (ZK security
claims), status pending, evidence pack at …, agent-out-of-scope true*.

**Narrative** — a survey of prior art, a design argument, a proof sketch, an honest account of what was
tried and failed. **Do not graph this.** Keep it as prose; make each load-bearing *claim* addressable
(a stable id) and link it to the graph nodes it concerns, so an agent can find the argument behind a fact
without reading 300 lines. Narrative is the minority of the corpus by value density and the majority by
line count — which is precisely why agents should query rather than read it.

## 4. What to graph first — ranked by duplication × volatility

1. **Gates and blockers** (the `sq-qhy4` class). 146 files, safety-critical, invalidated by a single
   event. Highest duplication, highest cost of staleness. **Start here.**
2. **Architecture invariants** (`opt-in` and siblings, 299 files) — restated constantly, changed rarely,
   and exactly what a worker needs in its first 200 tokens.
3. **Crate inventory, feature flags, public API surface** — pure Derived; today duplicated across 76
   READMEs, `AGENTS.md`, and `SKILL.md` files, each drifting independently.
4. **Benchmarks and competitor comparisons** — already partly protected by the no-perf-numbers gate;
   promote from "forbidden in prose" to "generated into prose".
5. **Decisions and verdicts, with supersession** — the `research/` corpus's actual payload. A decision
   node with `supersededBy` makes the 128 stale records *safely* stale: still readable, no longer
   authoritative.
6. **Task/issue state** — already authoritative in GitHub. **Federate, do not copy.** This is
   Channel C from the context-sharing record: never store what one API call derives. sparq's own SERVICE
   federation is the natural mechanism.

## 5. The anti-duplication mechanism (without this, the corpus refills)

Two pieces, and the second is what makes it durable:

**Transclusion.** A document that needs a fact includes a delimited generated block:

```markdown
<!-- pkg:begin gate=sq-qhy4 -->
…generated from the graph…
<!-- pkg:end -->
```

**A drift gate.** CI regenerates every block and **fails if the checked-in text differs**, exactly as
`check-no-perf-numbers.py` fails on a hand-written number. Generalise that script into a
`check-derivable-content` gate that also fails when a doc *restates* a fact the graph owns outside a
generated block.

Without the gate, prose re-accretes within weeks — the 146 restatements were all written by people acting
reasonably in the moment. **The gate is the strategy; the graph is just where the truth sits.**

## 6. Agents assemble context by query, not by reading

This is the direct extension of `agent-context-sharing.md` §4 (Channel B, pull-not-push):

- **Task-shaped assembly.** Given `(crate, role, issue)`, return the slice that matters: the invariants for
  that crate, its gates, its public API, its recent decisions, and the *claims* (not the documents) that
  bear on it. A worker on `sparq-mpc` should receive the MPC gate and the "never label unaudited work
  sound" invariant, not 85,033 lines of `research/`.
- **Hard retrieval budget**, and when it binds the agent is *told its view is partial* — a silently
  truncated context reads as a complete one.
- **Cheap-tier retrieval.** Measured here: the Haiku NL tool does the NL→SPARQL→answer round-trip at ~30×
  lower cost than the expensive tier, and `pkg-query` halved Opus tokens at N=30.
- This is also what shortens documents: once agents query, a document no longer needs to be
  self-contained, which is the force that made them long in the first place.

## 7. Staleness — the risk that decides whether this helps or hurts

**A drifted knowledge graph is worse than a drifted document**, because it looks authoritative and is
consumed by machines that will not sanity-check it. This is not hypothetical: a wrong normative claim
written into a `SKILL.md` on 2026-07-26 was trusted downstream and outlived the PR that introduced it.

Non-negotiable properties:

- **Derived facts regenerate in CI.** They cannot drift; if generation fails, the gate reds. Never publish
  a stale generated block silently.
- **Asserted facts carry provenance** — who/what asserted it, when, from what evidence, with what
  confidence — and a **re-confirmation horizon**. Past the horizon the fact is not deleted; it is
  **demoted to unverified** and says so at every read. An agent may hypothesise from unverified facts but
  may not ground a claim on one.
- **Retraction is first-class and cheap.** A wrong fact must be removable in one operation, traceable to
  its source, and its dependents flagged.
- **Contradiction is an event, not an overwrite.** Two conflicting assertions surface for adjudication;
  flattening them silently is how a wrong claim becomes durable.
- **Narrative gets `supersededBy`,** not deletion. The 128 month-old records keep their history and lose
  their authority.

## 8. Ontology: reuse aggressively, mint minimally

The project already measured this: **schema.org-as-top scored best for agent-KM accuracy (0.84)**, and the
driver was LLM *fluency* with the vocabulary rather than modelling elegance. So:

- Reuse schema.org, PROV-O (provenance is load-bearing here), DOAP/SPDX for software and licensing, SKOS
  for status vocabularies. Mint under `pkg:` **only** where nothing suitable exists, and record the reuse
  rationale — the existing PKG record already carries a reuse-provenance table; extend it.
- **Over-modelling is how these projects die.** Prefer a flat, fluent shape an LLM can query correctly over
  a precise one it cannot. Accuracy at retrieval beats ontological purity.

## 9. Dogfooding is a real advantage here

sparq **is** an RDF/SPARQL engine. Running the project's own knowledge graph on sparq means every KM query
is a live engine test on a real workload with real query shapes — SERVICE federation to GitHub for
Channel C, SHACL to validate that asserted facts carry their mandatory provenance, and the reasoner for
supersession closure. The KG stops being overhead and becomes a test fixture that also happens to be
useful. `crates/sparq-kb` and `pkg-instances.ttl` already exist as the seed.

## 10. Rollout — measurement-gated, highest-duplication-first

Primary metrics: **tokens per completed task**; **worker yield** (a worker given a task-shaped context slice
should fail less often for want of context); **corpus line count** (must fall); **duplication count for a
graphed fact** (must fall to 1 + N transclusions).

1. **The `sq-qhy4` gate class end-to-end** — one node, generated block, drift gate, 146 restatements
   replaced. Smallest complete vertical slice, highest duplication, safety-critical. If this does not pay,
   nothing further will.
2. **Extend the drift gate** from perf numbers to all Derived content; generate the crate/feature/API
   sections of the 76 READMEs and `SKILL.md` files.
3. **Task-shaped context assembly** for workers, behind a hard budget; A/B against worker yield.
4. **Decision/verdict nodes with supersession**, seeded from `research/`; mark the stale half unverified
   rather than rewriting it.
5. **Federate GitHub state** rather than importing it.

**Stop conditions, declared in advance:** if step 1 does not reduce both duplication and tokens-per-task,
the reframe is wrong and the effort should stop at the drift gate (which is independently valuable). If
graphed facts start going stale faster than prose did, the provenance/horizon machinery is inadequate and
the graph must shrink to Derived-only, where drift is impossible by construction.

## 11. What this deliberately does not do

- **Does not graph narrative.** Arguments stay prose. Attempting to triple-ify reasoning is the classic
  failure.
- **Does not parse prose into the graph.** One-way generation only; no two-way sync.
- **Does not replace `AGENTS.md` / `SKILL.md` / crate READMEs as human entry points** — they remain, but
  become *projections* with generated sections, which is what stops them drifting from each other.
- **Does not copy GitHub state.** Federate; issue state has one owner.
- **Does not assume this is worth doing at full scale.** Two of three measured context optimisations in
  this project were worthless. Step 1 is designed to be a cheap, complete, falsifiable test of the whole
  idea.
