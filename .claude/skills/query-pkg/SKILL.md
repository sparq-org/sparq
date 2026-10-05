---
name: query-pkg
description: Answer a "what does the repo say about X, where was Y decided, what is the status or provenance of Z, which sources are still unexplored, what depends on bead W" question by running a SPARQL query over the ingested Project-Knowledge-Graph (PKG) instead of reading whole documents. Use when an agent working on sparq needs a sourced, answer-sized fact from AGENTS.md, the skills, or the bd backlog and would otherwise Read or Grep a large doc. The mechanism is introspect then ground then ask via the pkg-query helper in crates/sparq-kb. DEFAULT path is to DELEGATE the round-trip to a model:haiku sub-agent as a natural-language tool call (measured cheaper at equal quality, sq-zbyo7; see bench/pkg-dogfood/RESULTS.md); plain in-context pkg-query is the fallback.
---

# Query the PKG — introspect → ground → ask

Bead **sq-2m6zm.3** (epic sq-2m6zm). Design record:
`research/dogfooding-sparq-knowledge-graph.md` (§4.1 frontier queries, §6 worked
questions). 🤖 SPARQ agent — dogfooding sparq as a project knowledge graph.

The PKG is sparq's own knowledge stored as RDF triples: `pkg:Finding`s (sourced,
confidence-tagged discoveries from AGENTS.md), `pkg:Source`s (skills / docs with an
explored-status), `pkg:Technique`s (the surfaces), and `pkg:Task`s (the bd backlog
projected to RDF with `pkg:dependsOn` edges). Instead of `Read`-ing a whole document to
find one fact, run a SPARQL query and read the **answer-sized** result. The answer is
computed by sparq's own engine over the ingested graph — within the store the agent
cannot fabricate a fact the data does not contain (the engine bounds the result set), and
an empty result is the honest "I don't know / none outstanding" answer.

> **Honesty.** This skill is the *mechanism*. The cost saving versus reading the source
> document is now **MEASURED** (bead **sq-zbyo7**, the agent-flavour cheap-model NL-tool:
> ≈ 30× cheaper in $ than read-docs at equal quality on PKG-answerable tasks); the full
> table, method, and honest caveats live in `bench/pkg-dogfood/RESULTS.md` (the sanctioned
> measurement record) — this file only points at the headline. The win is scoped to
> **PKG-answerable** questions by construction. The PKG is a Phase-1 *head slice* (the
> AGENTS.md finding set + a mechanical bd→Task projection + the heaviest skills'
> front-matter + the trust-graph admission-program provenance tier of `sq-7utko` —
> sourced, caveat-carrying findings from `research/trust-graph-authorisation-2026-07.md`,
> e.g. "unlinkability is NOT a v1 guarantee, gated on `sq-qhy4`"), not the whole corpus —
> a miss means "not in the head slice yet", so fall back to Read/Grep.

## The helper

One command, in `crates/sparq-kb`, behind the default-OFF `query` feature (keeps the lean
default build a pure data crate):

```bash
cargo run -p sparq-kb --features query --bin pkg-query -- <args>
```

It loads `pkg.ttl` (the ontology) + `pkg-instances.ttl` (the ingested data) +
`trust-graph-findings.ttl` (the `sq-7utko` provenance tier) into a sparq
store, runs a **named canned query** or a **raw SPARQL string**, prints the executed
SPARQL (always — for verification) and the result rows. Add `--features close` and
`--close owl-rl` to materialise the RDFS/OWL-RL closure first (entails the `pkg:dependsOn
owl:inverseOf pkg:blockedBy` pair and `pkg:couldBeMergedWith` symmetry — the design's
"always close first" step).

`--extra-graph <path>` (repeatable) loads extra Turtle file(s) alongside the PKG before
querying — their triples become visible to the query and participate in `--close owl-rl`.
This is the seam the FO-KM benchmark (epic `sq-mztg8`, Metric 1) uses to load a
foundational-ontology overlay so its `rdfs:subClassOf` axioms entail FO-typed facts; the
arms, overlays, and tasks live in `bench/fo-km/`.

## Cheap-model NL-tool — the DEFAULT path (delegate to Haiku)

**For a PKG-answerable question, do NOT run the round-trip yourself in the
expensive orchestrator. DELEGATE it as a natural-language tool call to a `model:haiku`
sub-agent.** The cheap model does the whole `introspect → ground → SPARQL → run → NL`
loop; the orchestrator (Opus) only emits the question and reads the answer back. It
never sees the schema card, the SPARQL, or the raw rows.

**Why this is the default.** The verbose NL→SPARQL→run→NL middle is exactly the part a
~15×-cheaper model handles fine, and pushing it off the expensive orchestrator is a
large **model-price-weighted** saving. The agent-flavour cheap-model NL-tool was
measured (bead **sq-zbyo7**, N=30 frozen PKG-answerable tasks) at **≈ 30× cheaper in $
than Opus reading the docs, and ≈ 16× cheaper than Opus running `pkg-query` itself, at
equal answer quality.** Full table + method + caveats: `bench/pkg-dogfood/RESULTS.md`.
(The decision metric is $, not raw tokens — that is why the cheap-model arm wins even
though its raw-token count is similar.)

**The brief to give the Haiku sub-agent** (it must return a self-checkable answer):

> Use the `query-pkg` skill. Answer this question over the PKG by `introspect → ground →
> ask` via `pkg-query`. Question: «…». Return (1) a concise NL answer, (2) the **exact
> executed SPARQL**, and (3) the **provenance + confidence** of each fact (the
> `dcterms:source` section anchor and `pkg:confidence` each row carries). If the query
> returns 0 rows, say so — an empty result is the honest "not in the head slice / none
> outstanding" answer; do not invent rows.

**The verification echo (the guardrail).** Because the sub-agent returns the executed
SPARQL + resolved IRIs + the per-row provenance/confidence, the caller can **verify**
the answer was computed from a real query over the data, not guessed — the soundness
echo. If the returned SPARQL does not match the question, re-ask or fall back; never
accept a bare NL answer with no query behind it.

**Abstain → fall back (the safe-default rule).** The PKG is a Phase-1 head
slice, so it cannot answer every question — and the safe behaviour is to **abstain, not
guess**. An empty / 0-row result (or an explicit `NOT_IN_PKG` from the sub-agent) is the
honest "not in the head slice / none outstanding" answer; the sub-agent brief above
already forbids inventing rows. **On an abstain, fall back to `Read`/`Grep` on the actual
docs/code for that question** — the measured win in `bench/pkg-dogfood/RESULTS.md` is
scoped to PKG-answerable questions *by construction*, so a miss is expected and is not a
failure of the flow. Never paper over an abstain with a fabricated answer.

**Tier escalation — Haiku by default, escalate only when accuracy is critical.**
The cheap-model NL-tool (Haiku, arm **C**) is the default precisely because
it is the cheapest arm at equal quality on PKG-answerable tasks. For an
**accuracy-critical** lookup — one where a cheap-model misread would be costly — escalate
a model tier: run the round-trip on a Sonnet/Opus `pkg-query` (this is arm **B**, which
is still cheaper than Opus reading the docs, just not as cheap as the Haiku arm). Use the
verification echo as the trigger: if the returned SPARQL does not match the question, or
the answer looks off, re-ask, escalate a tier, or fall back to reading — do not lower the
bar to make the cheap arm "work".

**Fallback — plain Opus `pkg-query`.** When sub-agent delegation is not available (e.g.
you are already inside a leaf sub-agent, or the dispatch path is unavailable), run
`pkg-query` yourself in-context as described below. That is arm **B** of the measurement
— still cheaper than reading the docs, just not as cheap as the Haiku NL-tool.

## (a) INTROSPECT — what can I ask about?

Before grounding a question, learn the dataset's effective schema so you know which
classes and predicates exist (cheaper than scanning sample data). The `schema-classes`
query lists every `pkg:` class actually instantiated, with a count:

```bash
cargo run -p sparq-kb --features query --bin pkg-query -- --query schema-classes
```

```text
class  |  instances
Task  |  1255
Finding  |  11
Source  |  6
Document  |  6
Technique  |  5
```

`--query schema-properties` lists the predicates in use (label, type, title, identifier,
priority, status, issueType, isPartOf, **dependsOn**, subject, source, …). List every
canned query with:

```bash
cargo run -p sparq-kb --features query --bin pkg-query -- --list
```

## (b) GROUND — translate the question to SPARQL over those terms

The canned queries are the §4.1-class, verified-expressible SPARQL frontier queries
(plain SELECT / `FILTER NOT EXISTS` / `GROUP BY`; the §4.2/§4.3 N3 rules are explicitly
Phase-2/3 and NOT used here). Pick the template that matches the question shape; some take
an argument via `--arg`.

| Question shape | canned `--query` | `--arg` |
|---|---|---|
| "What does the repo say about TOPIC?" | `findings-about` | a topic IRI |
| "Where was this decided / what is its provenance + confidence?" | `finding-provenance` | — |
| "What are the DQV quality measurements (subject + metric + value)?" | `finding-quality-dqv` | — |
| "Which sources are still UNEXPLORED (target follow-up)?" | `unexplored-sources` | — |
| "What is the explored-status of the source catalog?" | `source-status` | — |
| "What does bead W depend on, and is each dependency done?" | `task-depends-on` | a bd id |
| "What is blocked by bead W (downstream impact)?" | `task-blocks` | a bd id |
| "Which sources should I read next (priority order)?" | `high-followup-priority` | — |
| "How many beads are ready (§4.1 dependency frontier)?" | `ready-frontier` | — |

The topic IRIs in the current head slice are
`https://sparq.dev/ns/pkg/kb#topic-merge-discipline`,
`…#topic-subagent-rules`, and `…#topic-zk-discipline`.

To go beyond the templates, write a raw SPARQL SELECT/ASK against the `pkg:` terms and
pass it with `--sparql '<query>'`. Use `--sparql-only` to print the SPARQL without running
it (verify before executing).

## (c) ASK — run the helper and read the answer

Run the chosen query; read the small result table instead of the source doc. Each finding
row carries its `dcterms:source` section anchor and `pkg:confidence`, so the answer is
**sourced and confidence-tagged**, not bare prose.

---

## End-to-end examples (real questions, real returned answers)

### Example A — "What is the merge discipline?" (a findings-about GROUND query)

Instead of reading the whole `AGENTS.md` contribution-workflow + gate-matrix sections:

```bash
cargo run -p sparq-kb --features query --bin pkg-query -- \
  --query findings-about --arg https://sparq.dev/ns/pkg/kb#topic-merge-discipline
```

Returned answer (sourced + confidence-tagged):

```text
label  |  section  |  conf
A PR merges only when ci-fast is green and every review thread is resolved
    |  AGENTS.md#contribution-workflow--prs-reviews-resolved-ci-fast  |  0.98
Never merge a stacked PR whose base is not main; retarget it first
    |  AGENTS.md#contribution-workflow--prs-reviews-resolved-ci-fast  |  0.93
2 row(s).
```

(Swap the topic to `…#topic-zk-discipline` for "what's the merge discipline for a ZK PR?"
— it adds the forge_gates + gate-count snapshot row and the HARD privacy-claims gate row.)

### Example B — "Which sources are still unexplored?" (an honest empty answer)

```bash
cargo run -p sparq-kb --features query --bin pkg-query -- --query unexplored-sources
```

Returned answer:

```text
source  |  title  |  status  |  prio
0 row(s).
```

Over the Phase-1 ingest every source is `pkg:Explored`, so the targeted-follow-up list is
**empty** — the honest "none outstanding" answer, computed by the engine rather than
guessed. (As `Unexplored`/`Exploring` sources are ingested in later phases they appear
here automatically.)

### Example C — "What is blocked by bead sq-8thu?" (a task-blocks GROUND query)

Instead of running `bd dep tree` or grepping `.beads/issues.jsonl`, ask the read-model:

```bash
cargo run -p sparq-kb --features query --bin pkg-query -- --query task-blocks --arg sq-8thu
```

Returned answer (downstream dependents + each one's status; truncated):

```text
downstreamId  |  downstreamStatus  |  downstreamTitle
sq-0po6  |  Closed  |  page: /surface/inference (tier-b live)
sq-11zy  |  Closed  |  page: /surface/streaming-rsp (tier-b live)
sq-13rg  |  Closed  |  ZK: bb.js in-browser UltraHonk proving wiring (WZK)
…
22 row(s).
```

The inverse direction (`?b pkg:blockedBy sq-8thu`) is never asserted in the data; under
OWL-RL closure it is entailed, so this is equivalent — verify with:

```bash
cargo run -p sparq-kb --features close --bin pkg-query -- --close owl-rl \
  --sparql 'PREFIX pkg: <https://sparq.dev/ns/pkg#> PREFIX dcterms: <http://purl.org/dc/terms/>
            SELECT (COUNT(*) AS ?n) WHERE { ?b dcterms:identifier "sq-8thu" . ?b pkg:blockedBy ?d }'
```

## The natural-language tool (agent flavor) — `sq-ve5dy`

Instead of writing the SPARQL yourself, you can delegate the whole round-trip to a
**cheap-model (Haiku) sub-agent** so the expensive orchestrator only emits a plain-English
question and reads a short answer. The sub-agent (`.claude/agents/sparq-pkg-nl.md`,
`model: haiku`) does NL → introspect → ground → SPARQL → run → NL answer itself.

**Soundness guardrail (load-bearing):** the tool ALWAYS returns the **executed SPARQL +
resolved IRIs + grounding confidence** so the caller can verify the answer was *computed*,
not guessed — it never silently answers from a guessed query. `pkg-query --json` emits this
**NL-tool envelope** (`crates/sparq-kb/src/query/nl_tool.rs`):

```bash
cargo run -q -p sparq-kb --features query --bin pkg-query -- --query schema-classes --json
```

```json
{
  "answer": "5 row(s). Columns: class, instances.\n- Task | 1255\n- Finding | 11 …",
  "executed_sparql": "PREFIX pkg: <https://sparq.dev/ns/pkg#> SELECT ?class …",
  "resolved_iris": ["http://www.w3.org/1999/02/22-rdf-syntax-ns#type"],
  "ungrounded_iris": [],
  "hints": [],
  "row_count": 5,
  "confidence": "canned"
}
```

`confidence` is a **query-grounding** signal (NOT an answer-correctness claim): `canned` =
a curated template; `grounded` = a raw query whose every term exists in the data;
`ungrounded` = a raw query that used a term the data lacks (so it matched nothing — the
`hints` give same-namespace repair candidates; re-ground and re-run before trusting it). The
deterministic `answer` is the engine's computed rows; the cheap model rephrases it. The
**PRODUCT flavor** (sparq calling a configurable cheap-model endpoint as a real GenAI
integration) needs sparq's own API key (an external-credential dependency) and is a
follow-up — only the **agent flavor** (no API key) ships here.

> **Honesty.** Whether routing PKG access through a cheap-model sub-agent actually cuts the
> orchestrator's *model-price-weighted* cost is now **MEASURED** (beads sq-zbyo7 / sq-jgi97,
> the real-transcript N=30 3-arm A/B that superseded the sq-2m6zm.4 #1078 proxy): the Haiku
> NL-tool was ≈ 30× cheaper in $ than read-docs at equal quality. The full table, method,
> and caveats live in `bench/pkg-dogfood/RESULTS.md` (the sanctioned record); this file
> only points at it. The win is scoped to **PKG-answerable** questions by construction —
> a miss is an honest abstain, not a fabricated answer.

## Authoring new Findings — the write-path (`sq-mztg8.2`)

The `pkg:Finding` tier this skill queries is **authored**, not hand-written as
raw Turtle. To add or edit a Finding, edit the compact, IRI-free YAML-LD source
`crates/sparq-kb/ingest/agents-findings.yaml.ld` (generalising the shipped
`sec-prop.yaml.ld` pattern), then recompile:

```bash
python3 crates/sparq-kb/ingest/ingest_pkg.py \
  --beads .beads/issues.jsonl --skills-dir skills \
  --findings-yamlld crates/sparq-kb/ingest/agents-findings.yaml.ld \
  --out crates/sparq-kb/ingest/pkg-instances.ttl
```

A **deterministic** compiler (`ingest/yamlld_compile.py`) — not an LLM — expands the typed
`@graph` to schema.org-typed PKG Turtle. You write fluent author keys + concept TOKENS
(e.g. `about: merge-discipline`), never IRIs or `xsd:` datatypes. The **guarded `V()`**
resolver grounds each TOKEN against the file's `concepts:` catalog (lexical: local-name or
normalised `prefLabel`); an **ambiguous TOKEN is a HARD compile error** — the write-path
analogue of the read-path abstain, so a mis-resolution fails loud rather than emitting a
silent wrong IRI. The compiled tier is gated for **0 SHACL violations + triple-for-triple
round-trip** against the prior hand-authored tier
(`cargo test -p sparq-kb --features validate --test yamlld_write_path`; the compiler's
parser + resolver are self-tested by `scripts/tests/test_yamlld_compile.py`). See
`research/fo-llm-bridge.md` §3.4 / §6 Phase 4.

## Literature-ingestion scaffolding — fixtures only (`literature` feature, `sq-2489d.5`)

The **scaffolding** for the scaled, provenance-stamped literature-trawling tier
(GenAI-KB Phase 5, design `research/provenance-driven-genai-kb.md` §4/§5) lives behind the
default-OFF `literature` feature in `crates/sparq-kb/src/literature/`. It exercises the
`[connector] → [normalise] → [extract (record/replay)] → [ground] → [emit TTL] →
[SHACL gate] → [sidecar]` pipeline over **committed fixtures** and makes **ZERO network and
ZERO live-model calls** — the only LLM-in-the-loop step (extraction) is isolated behind the
`Extractor` **record/replay trait**, and the shipped adapter (`RecordedExtractor`) replays
a frozen tape. The live pilot (a real cheap-model batch over OpenAlex/S2) is the separate,
credential-gated **Phase-6** bead (`needs-access`).

Load-bearing honesty invariants the scaffolding enforces: the **deterministic
grounding-resolver** (`literature::ground`) requires every committed Finding's
justification to be an entailed span of the source abstract AND every cited DOI to resolve
to an in-batch `pkg:Source` — failures are **quarantined to the sidecar, never silently
dropped** (`literature::pipeline::Sidecar`). Every emitted Finding sits on the low-trust
machine tier (`prov:wasAttributedTo` a `pkg:MachineAgent`, capped at `secx:Conjectured`
with a bounded confidence) enforced declaratively by `shapes/literature.shapes.ttl`. The
Phase-5 metric (the **citation-grounding rate**) is *computed at run time* from the fixture
batch — no hard-coded number. NOT appended to `pkg-instances.ttl`: bulk ingestion is gated
on the Phase-6 per-topic recommend-adopt verdict. Run:
`cargo test -p sparq-kb --features literature,validate --test literature_pipeline`.

**CORE API v3 live connector** (`sq-tzars.1`, `connector_core`): a second connector
(`parse_core_batch` → the same DOI-keyed `SourceStub`s as `parse_openalex_batch`) plus a
live HTTP client behind the default-OFF **`literature-live`** feature (implies `literature`,
pulls only `ureq`). The pure parse + paging/retry discipline (`fetch_paginated`,
`backoff_delay`, the `Transport` trait) is under `literature` and runs over a **fake
transport** + the committed `fixtures/literature/core-batch.json` (a REAL, SANITIZED CORE
response — no key, no full-text) so CI stays **network-free**; only `CoreClient` (from the
`CORE_API_KEY` env var, **never** committed/logged) touches sockets. `SourceStub` now carries
`license: Option<String>`, captured **fail-closed** by both connectors — `None` (unknown) is
treated as NON-REDISTRIBUTABLE by the downstream dump tiering (`sq-tzars.7`). Rate-limit
discipline honours `429`/`Retry-After` with bounded backoff + a HARD per-run request cap.
Run: `cargo test -p sparq-kb --features literature,literature-live --test core_connector`.

## When NOT to use this

- The fact is **not in the head slice** (PKG is a Phase-1 subset) → an empty/partial
  result means fall back to `Read`/`Grep`. An empty result is honest, not a failure.
- The document is **small** and you would read it once → loading it directly is fewer
  tokens and fewer round-trips (the design's §1.1 honest boundary).
- For the **live** ready-frontier (git/gh/nproc-aware) use `scripts/push-frontier.sh`;
  `ready-frontier` here is only the §4.1 dependency half over the projected backlog.
- bd remains the **source-of-record** for tasks; the PKG `pkg:Task`s are a read-model
  mirror, not a place to write task state.

## Where this lives

- Helper binary: `crates/sparq-kb/src/bin/pkg_query.rs` (`--bin pkg-query`; `--json`
  emits the NL-tool envelope).
- Canned queries (the library the binary + the test share):
  `crates/sparq-kb/src/query/canned.rs`.
- NL-tool envelope (`sq-ve5dy`): `crates/sparq-kb/src/query/nl_tool.rs`
  (`run_canned` / `run_raw` → `NlToolResult`).
- Agent-flavor sub-agent (`model: haiku`): `.claude/agents/sparq-pkg-nl.md`.
- Loader + optional closure: `crates/sparq-kb/src/lib.rs` (`query` / `query::close` modules).
- Rot-guard tests: `crates/sparq-kb/tests/query_canned.rs` and
  `crates/sparq-kb/tests/nl_tool.rs`
  (`cargo test -p sparq-kb --features query`; add `--features close` for the closure test).
- Ontology + data: `crates/sparq-kb/ontology/pkg/pkg.ttl`,
  `crates/sparq-kb/ingest/pkg-instances.ttl`.
- Write-path (`sq-mztg8.2`): authoring source
  `crates/sparq-kb/ingest/agents-findings.yaml.ld`, deterministic compiler
  `crates/sparq-kb/ingest/yamlld_compile.py`, gates
  `crates/sparq-kb/tests/yamlld_write_path.rs` + `scripts/tests/test_yamlld_compile.py`.
- Literature-ingestion scaffolding (`literature` feature, `sq-2489d.5`):
  `crates/sparq-kb/src/literature/` (`connector` / `extract` / `ground` / `pipeline`),
  shapes `crates/sparq-kb/shapes/literature.shapes.ttl`, fixtures
  `crates/sparq-kb/fixtures/literature/`, gate
  `crates/sparq-kb/tests/literature_pipeline.rs`.
- CORE v3 live connector (`literature`/`literature-live`, `sq-tzars.1`):
  `crates/sparq-kb/src/literature/connector_core.rs`, fixture
  `crates/sparq-kb/fixtures/literature/core-batch.json`, gate
  `crates/sparq-kb/tests/core_connector.rs`.
- **Citation renderer** (`query` feature, `sq-2489d.11`):
  `crates/sparq-kb/src/query/citations.rs` — `render_citations(&graph, &rows)` renders
  `prov:wasDerivedFrom` sources from `FINDING_PROVENANCE` rows as `[source: label]`
  citations; returns `CitationReport` with `CitationMetrics` (resolution-rate + fabricated-count).
  Gate: `crates/sparq-kb/tests/citations.rs`.
