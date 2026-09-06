# FO-KM benchmark — Metric 1 (agent KM-task accuracy + cost over the PKG)

> 🤖 **SPARQ agent** [OPUS-4.8]. The runnable harness for **Metric 1** of the design
> record `research/foundational-ontology-km-benchmark.md` (epic **sq-mztg8**; PR #1106).
> NOT a perf claim — the actual A/B numbers live here (the `bench/` tree), never frozen
> into markdown.

## What this measures

An **apples-to-apples A/B**: the same NL-tool (`crates/sparq-kb` `pkg-query`, the
introspect→ground→ask helper / `nl_tool` envelope) answers **FO-exercising
knowledge-management questions** over the **same** Project-Knowledge-Graph (PKG), typed
under different **foundational-ontology (FO) overlays**. The question under test (epic
sq-mztg8): *does any FO beat the no-FO incumbent (and gUFO) on agent KM-task accuracy +
cost?* The pre-registered prior is **NEUTRAL** (design §7) — this harness is built to be
able to return that null honestly.

## The arms (overlays/)

Each arm = the shipped PKG (`pkg.ttl` + `pkg-instances.ttl`) **plus** one overlay TTL
loaded via `pkg-query --extra-graph`, optionally closed with `--close owl-rl` so the
overlay's `rdfs:subClassOf` axioms entail the FO-typed facts (rdfs9 type propagation +
rdfs11 transitive subclass).

| Arm | Overlay | PKG-class → FO top category | FO source |
|---|---|---|---|
| **no-FO** (incumbent) | `overlays/no-fo.ttl` | (none — the shipped reuse-first PKG) | — |
| **gUFO** (named baseline) | `overlays/gufo.ttl` | Task→`gufo:Event`; Finding→`gufo:AbstractIndividual`; Source/Technique→`gufo:Object` | nemo-ufes.github.io/gufo |
| **DOLCE-DUL** | `overlays/dolce-dul.ttl` | Task→`dul:Action`; Finding→`dul:Description`; Source→`dul:InformationObject`; Technique→`dul:Method` | ontologydesignpatterns.org DUL |
| **schema.org-as-top** | `overlays/schema-org.ttl` | Task→`schema:Action`; Finding→`schema:Claim`; Source→`schema:DigitalDocument`; Technique→`schema:HowTo` | schema.org |

Each overlay inlines only the **minimal** FO taxonomy fragment needed for closure (it
does not import the whole FO) and cites its source in the file header.

## The per-arm command

```bash
# FO arm (e.g. gUFO): load the overlay, close, ask the FO-typed query
cargo run -p sparq-kb --features close --bin pkg-query -- \
  --extra-graph bench/fo-km/overlays/gufo.ttl --close owl-rl \
  --sparql 'PREFIX gufo: <http://purl.org/nemo/gufo#> SELECT (COUNT(DISTINCT ?x) AS ?n) WHERE { ?x a gufo:Event }'

# no-FO arm: the same question over the incumbent (no FO category → 0 rows / can't answer)
cargo run -p sparq-kb --features close --bin pkg-query -- \
  --extra-graph bench/fo-km/overlays/no-fo.ttl --close owl-rl \
  --sparql 'PREFIX gufo: <http://purl.org/nemo/gufo#> SELECT (COUNT(DISTINCT ?x) AS ?n) WHERE { ?x a gufo:Event }'
```

Swap `gufo.ttl` for `dolce-dul.ttl` / `schema-org.ttl` (and the matching FO query from
`tasks.jsonl`'s `select` map) for the other arms. `--json` emits the verifiable NL-tool
envelope (executed SPARQL + resolved IRIs + grounding confidence).

## The tasks (tasks.jsonl)

16 **FO-exercising** KM tasks, stratified (design §5): **TH** type-hierarchy, **ER**
entailment-dependent, **CC** cross-category. Each line:
`{id, kind, question, gold_keys, gold_count, select, no_fo, discriminates}` —
`select` is the per-arm FO query (`gufo` / `dolce-dul` / `schema-org`; `null` where an
FO honestly cannot draw that distinction); `no_fo` is what the incumbent would attempt;
`discriminates` records why the no-FO arm genuinely cannot answer.

These tasks **discriminate**: an FO arm under closure returns the gold answer; the no-FO
arm returns 0 / can only hand-enumerate (the FO-win construction). Tasks answerable by
plain `pkg:` terms are deliberately excluded — they would not differentiate the arms.

## Running the A/B (the Metric-1 method)

The measured A/B (`RESULTS.md`) is run as **one fresh Haiku NL-tool per (arm, task)** —
4 arms × 16 tasks = 64 sub-agents. Each sub-agent gets ONLY the task's natural-language
`question`, plus its arm's overlay path, and answers it end to end by driving `pkg-query`
itself — **introspect → ground → ask**:

```bash
# one (arm, task) sub-agent's tool invocation (it picks the SPARQL; this is the harness it drives)
cargo run -q -p sparq-kb --features close --bin pkg-query -- \
  --extra-graph bench/fo-km/overlays/<arm>.ttl --close owl-rl --json \
  --nl '<the task question>'
```

The orchestrator opens each sub-agent's brief with the attribution tag
`[FOKM task=<id> arm=<no-fo|gufo|dolce-dul|schema-org>]` so its transcript can be mined.
The agent never sees the gold answer or the per-arm `select` query — it must ground the NL
question onto the overlay's vocabulary on its own (this is exactly what the LLM-fluency
hypothesis is testing).

## Scoring the run (analyze.py)

`analyze.py` is the reproducible token-miner + coverage grader that turns the run's
transcripts into the `RESULTS.md` table. From the repo root:

```bash
python3 bench/fo-km/analyze.py <transcript-dir> --tasks bench/fo-km/tasks.jsonl
# self-check (no run on hand): print the grading contract + validate the tasks file
python3 bench/fo-km/analyze.py
```

It (1) mines the **real cache-discounted effective input tokens** from each
`agent-*.jsonl` transcript's `message.usage`
(`1.0·input + 0.1·cache_read + 1.25·cache_creation`; no `count_tokens`, no char proxy),
and (2) grades each answer deterministically (no model in the loop) — **count-coverage**
(the gold integer appears), **entity-coverage** (every gold local-name resolves), or
**concept-coverage** (every partition bucket count appears); an arm that legitimately
cannot answer (its `select` is null) and honestly abstains is counted as an ABSTAIN, not
a wrong answer. The measured verdict lives in **`RESULTS.md`** (the sanctioned numeric home).

## Authoring + validation

- `build_tasks.py` regenerates `tasks.jsonl`.
- `validate_tasks.py` proves every task discriminates (each FO arm answers with the
  expected count; the no-FO arm returns 0) — run from the repo root:
  `python3 bench/fo-km/validate_tasks.py` (needs the `close` feature).

## Metric 3 — LLM ontological-commitment stability (`STABILITY.md`)

A second, separate harness in this directory implements **Metric 3** of
`research/fo-llm-bridge.md` §4.2 / §6 Phase 6 (bead `sq-mztg8.3`): the
**Köhler–Neuhaus cross-session contradiction probe**, re-run per model, asking whether the
FO choice changes the LLM's *ontological-commitment stability* rather than its exec
accuracy. It reuses this directory's overlays as its arm definitions but shares nothing
else — Metric 1's `tasks.jsonl`, `analyze.py` and `RESULTS.md` are untouched by it.

```bash
python3 bench/fo-km/build_probes.py --check              # the 12-probe battery
python3 bench/fo-km/build_probes.py --emit-prompt gufo   # the exact per-session brief
python3 bench/fo-km/stability_analyze.py                 # self-check: assert the grader
python3 bench/fo-km/stability_analyze.py bench/fo-km/metric3-sessions.jsonl
```

With no session file the grader **asserts** its own contract and exits nonzero on any
mismatch: hand-computed fixtures pin every reported quantity (including missing,
duplicate, out-of-vocabulary and `UNDECIDABLE` answers), and every published column of
the committed 45-session table is re-derived from `metric3-sessions.jsonl`.

- `stability_probes.jsonl` — 12 forced-choice probes over a closed label set
  (`OCCURRENT`/`CONTINUANT`/`ABSTRACT`/`UNDECIDABLE`), stratified **SC** (the 4 PKG classes
  the overlays type, as generic/instance pairs) and **US** (subjects no overlay types).
- `build_probes.py` — authors the battery and renders the exact per-arm session brief
  (arms: `ungrounded` / `gufo` / `schema-org`).
- `stability_analyze.py` — the deterministic grader, no model in the loop: cross-session
  contradiction rate, dissent, within-session generic/instance inconsistency, a
  decisiveness guard, and per-arm scaffold adherence.
- `metric3-sessions.jsonl` — the raw session answers the record is derived from.
- **`STABILITY.md` is the measured record.** It is a **PILOT**: the bead carries
  `needs-maintainer-steer` on the grading protocol and N, and `STABILITY.md` § Open
  questions lists what is genuinely blocked.

## Honest scope

- This is **Metric 1** (the AGENT). It is **MEASURED** — see `RESULTS.md` for the verdict
  (schema.org-as-top wins for the agent's KM tasks; gUFO scored *below* the no-FO
  incumbent; the driver is LLM fluency, not formal richness — confirming the PR #1106
  hypothesis). Metric 2 (the KGE closure-prior MRR via `eval.rs`
  `run_ablation_multiseed_paired`) needs a canonical/EC2 box and is a separate,
  **EC2-deferred** phase (bead **sq-p5ro8**); a formal FO could rank differently there
  (design §5.1). **Metric 3** (above) measures a third, independent construct —
  commitment *stability*, not task accuracy — and its pilot verdict does not read on the
  Metric-1 facade decision.
- The closure-build CPU/wall cost is **non-canonical** and is never charged as a token
  cost (design §5.1).
