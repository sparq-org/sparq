<!-- [OPUS-4.8] sq-2m6zm.4 / sq-ve5dy / sq-zbyo7 (epic sq-2m6zm). 🤖 SPARQ agent — the
PKG-query token A/B harness. Written while Fable unavailable; flag for re-review when
Fable returns. -->
# PKG-query token A/B (`sq-2m6zm.4` → `sq-ve5dy`)

The **scientific gate** on dogfooding sparq as a project knowledge graph (epic
`sq-2m6zm`, Phases 2–4). It measures whether answering a project-knowledge lookup
via the **`pkg-query`** skill (`introspect → ground → ask`, PR #1075) costs fewer
**cache-discounted effective input tokens** than reading the source document(s) —
and whether answer **quality** holds — then emits the verdict.
Protocol + kill-criteria: `research/dogfooding-sparq-knowledge-graph.md` §5.

> **The real measurement is `RESULTS.md`.** The original harness below (`run.py` /
> `tokens.py`) was a **char/byte PROXY** PoC (N=12), which honestly flagged itself as
> lower-fidelity. It has been **superseded** by the **real-transcript 3-arm A/B**
> (`tokens_real.py` + `analyze3.py` + `tasks/abm_tasks.json`, N=30) — see
> **[`RESULTS.md`](./RESULTS.md)** for the measured table, method, and the CRITICAL
> LESSON (the proxy inverted the verdict; prefer real measurement).

## What it does

- **`PREREG.md`** — frozen pre-registration: hypotheses, arms, the metric, and the
  kill criteria, fixed BEFORE any run (editing it to fit a result is forbidden).
- **`tasks/tasks.jsonl`** — frozen, **stratified** task set (point-lookup /
  multi-hop / synthesis / negative) answerable BOTH ways, each with a gold key.
- **`corpus-manifest.json`** — sha256 of the pinned corpus; the driver aborts on drift.
- **`run.py`** — counterbalanced A/B driver. Arm A = realistic source read
  (AGENTS.md section / `bd show`, NOT the whole corpus) + a naive whole-doc variant;
  Arm B = `pkg-query`, **charged all its costs** (schema card + executed SPARQL +
  rows + deferred-tool-def tax + amortised ingestion).
- **`tokens.py`** — effective tokens, reusing the canonical
  `scripts/agent-telemetry/metrics_row.py::effective_input_tokens` (the §5.1 formula).
- **`grade.py`** — blind deterministic PAIRED grader (no LLM judges its own KG).
- **`stats.py`** — Wilcoxon signed-rank + bootstrap median CI + break-even, applies
  the frozen bar + kill criteria, emits `verdict.json`.

```bash
bench/pkg-dogfood/run.sh   # build → drive → grade → verdict (the char-proxy PoC)
```

## The real-transcript 3-arm A/B (sq-ve5dy — supersedes the proxy)

The proven measurement (`RESULTS.md`) mines the tokens the model **actually
consumed** from real sub-agent transcripts and adds a third, cheaper arm:

- **`tasks/abm_tasks.json`** — the frozen **30-task** set (`id` / `stratum` /
  `question` / `gold_keys` / `armB`), 4 strata, PKG-answerable by construction.
- **`tokens_real.py <transcript-dir> [out.json]`** — the REAL-transcript
  effective-token miner: per `agent-*.jsonl` it attributes by the `[ABM task= arm=]`
  tag and sums `1.0·input + 0.1·cache_read + 1.25·cache_creation` straight from
  `message.usage` (**no `count_tokens`, no char proxy**).
- **`analyze3.py --tasks … --tokens … --answers … [--out verdict.json]`** — the
  3-arm **$-weighted** verdict (A = Opus read-docs, B = Opus pkg-query, C = Haiku
  pkg-query NL-tool). The decision metric is **model-price-weighted cost**, since
  arm C runs the verbose middle on a model ~15× cheaper per token.

## The provenance-capability A/B (`sq-2489d.6` — GenAI-KB Phase 7)

A **second, independent** experiment on the same substrate. The A/B/C run above asks
*"is querying the PKG cheaper than reading the docs?"*; Phase 7 asks *"does making the
PKG **provenance-driven** — citations, hedging, provenance-weighted retrieval — change
agent OUTCOMES, and is it worth what it costs?"* Baseline arm **P0** is the same
`pkg-query` with every capability off (the inert PKG, i.e. today's shipped behaviour);
arms **P1/P2/P3** each switch on exactly one capability, so a capability that pays is
never carried by one that does not.

- **`PREREG-PROVENANCE.md`** — the frozen pre-registration. Read it first: it declares
  the **direction swap** (these treatments *add* quality and *cost* tokens, so quality is
  the superiority axis and cost is the non-inferiority axis, reusing the same frozen
  constants rather than a second threshold family), the seeded stale-fact-trap
  substrate, and the three blockers that keep the verdict from being final.
- **`prov_ab.py`** — emits one §5.6 verdict object **per capability** plus an
  adopt/drop/blocked roll-up. Tokens come from `tokens_real.py` (the `[ABM … arm=P0..P3]`
  tags), prices and the gold-key grader from `analyze3.py`, and the bar + statistics are
  **imported** from `stats.py`.
- **`test_prov_ab.py`** — the analyzer's self-test over a synthetic in-process fixture
  (no measurement, no real transcript). It pins that each honesty precondition refuses on
  its own, that each kill criterion fires, and that `honest=true AND recommend_adopt=true`
  is reachable — a gate that can never go green is not a gate.

```bash
python3 bench/pkg-dogfood/test_prov_ab.py   # self-test the Phase-7 analyzer
```

**Not yet runnable, by three declared blockers** (each enforced in code, so it cannot rot
into a stale caveat): the canonical A/B host is `needs-maintainer-steer`; the Fable re-run
(#1111) has not happened; and `pkg-query`'s `NlToolResult` envelope does not yet surface
citations or qualification, so arms P1/P2 would be placebos today. Until those clear the
analyzer reports `honest=false` and names which condition failed.

## Honesty (load-bearing)

- **Fidelity caveat.** This measures the **read-payload** effective tokens via a
  documented char/token PROXY — it is **not** the full §5 multi-session Claude Code
  A/B with `count_tokens` (the exact Claude tokenizer; `tiktoken` is wrong for
  Claude) over per-session transcripts. Neither an API key nor isolated sessions
  are available where this was written. The proxy captures the dominant delta
  driver and is conservative against the PKG; the full-session A/B is the follow-up
  (`bd` bead). See `PREREG.md` "measurement-fidelity caveat".
- **All numbers are runtime-only / NON-CANONICAL** (work-box + proxy). `run.sh`
  prints them; nothing is frozen into committed markdown (`check-no-perf-numbers.py`).
  `results.json` / `grades.json` / `verdict.json` are git-ignored — regenerate them.
- **Underpowered by construction.** The PoC task set is N=12 < the pre-registered
  ≥30, so the frozen bar CANNOT return `token_win=true`. It reports per-task deltas
  + spread + per-stratum descriptively and returns an honest `recommend_adopt`.
- **Non-sycophantic verdict.** If PKG-query does not beat the realistic baseline,
  the harness says so. The realistic Arm A (scoped read / `bd show`) is the binding
  baseline precisely so a strawman "read the whole corpus" arm cannot rig the win.

Licensed MIT (repo default).
