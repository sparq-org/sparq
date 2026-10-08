<!-- sq-lhwo.4 (epic sq-lhwo). 🤖 SPARQ agent. -->
# ast-grep + outline token A/B harness (sq-lhwo.4)

> 🤖 **SPARQ agent — verdict already SETTLED; this is the runnable proxy harness, not the
> canonical answer.** The load-bearing decision ("does outline/`ast-grep`-first save agent
> tokens?") was settled by the **real-transcript** A/B, whose CONCLUSION is:
> **ast-grep + outline are PRECISION / completeness tools, measured NOT to save tokens**
> (slightly more expensive end-to-end). The **canonical record** is
> [`bench/pkg-dogfood/RESULTS-astgrep.md`](../../../bench/pkg-dogfood/RESULTS-astgrep.md)
> (cited qualitatively by `AGENTS.md` §2 and the `ast-grep` SKILL §5). This directory is the
> **lower-fidelity read-payload PROXY** harness (the runnable code + the frozen PREREG/tasks);
> by construction it **cannot** reach `recommend_adopt` (see below), and its predecessor
> byte-proxy in fact **overstated** the saving — the real measurement corrected it. Keep this
> harness for reproducible proxy runs and as the pre-registration of record; for the *verdict*,
> read `RESULTS-astgrep.md`.

The FIRM, runnable A/B for the **ast-grep + outline-before-read** intervention vs the
competent `grep -rn` + full-`Read` baseline — deferred from `sq-lhwo.2` (where the
skill shipped and a directional, advisory byte-cost pilot ran). It applies the SHARED
measurement protocol in
[`research/dogfooding-sparq-knowledge-graph.md`](../../../research/dogfooding-sparq-knowledge-graph.md)
§5.1-§5.6 — the same engine the PKG track (`sq-2m6zm.4`) pre-registers — to a
different intervention, and emits the §5.6 verdict object.

## Files

| Path | What |
|---|---|
| `PREREG.md` | the FROZEN pre-registration — hypotheses, arms, metric, kill criteria, fixed BEFORE any run. Editing it to fit a result is forbidden. |
| `tasks/tasks.jsonl` | the frozen, stratified task set (N=30, 4 strata: `list`/`locate`/`shape`/`negative`). |
| `corpus-manifest.json` | the pinned-corpus sha256 manifest; `measure_ab.py` drift-checks it BEFORE measuring so a run over changed code is FLAGGED, not silently trusted. |
| `rules/*.yml` | ast-grep YAML rules for the trait-impl `list` tasks (self-contained copies of the SKILL's `impl-of-trait` rule). |
| `measure_ab.py` | the model-free read-payload measurer — EXECUTES both arms over the real repo and emits per-task paired effective-token rows. |
| `grade.py` | the deterministic, arm-blinded quality grader (fabrication on all tasks + the negative-set-empty invariant). |
| `ab_stats.py` | the intervention-agnostic statistics + §5.6 verdict-object emitter — Wilcoxon signed-rank + bootstrap 95% CI + break-even N* + the kill criteria, applied mechanically. |
| `run.sh` | one-shot driver: measure → grade → verdict. |
| `tests/` | hermetic stdlib `unittest` over the math, the kill criteria, the verdict-object gating, and the drift check. |

## Run

```sh
scripts/agent-telemetry/ast-grep-outline/run.sh           # -> /tmp/astgrep-outline-ab/verdict.json
python3 scripts/agent-telemetry/ast-grep-outline/tests/test_ab_stats.py
python3 scripts/agent-telemetry/ast-grep-outline/tests/test_measure_ab.py
```

ast-grep must be installed (the run self-checks). Install + verify per
[`.claude/skills/ast-grep/SKILL.md`](../../../.claude/skills/ast-grep/SKILL.md) §0 —
and NEVER invoke it as `sg` (that name collides with `newgrp` on this box).

## What the run measures — and the honest fidelity ceiling

`measure_ab.py` measures the **read-payload effective tokens**: the input bytes each
arm forces into an agent's context, executed over the real code, via the cache-discount
formula (`1.0*fresh + 0.1*cache_read + 1.25*cache_write`). This is a **lower-fidelity
proxy**: it captures the dominant, model-free driver of the delta but EXCLUDES the
model round-trip / repair tokens and per-turn cached-prefix dynamics that the §5 gold
standard (each arm a separate session, completions pinned, two `agent_telemetry.py`
reports diffed, an `ANTHROPIC_API_KEY` for the exact tokenizer) would capture — neither
is available to a single sub-agent here. See `PREREG.md` for the full caveat.

Consequently a proxy run carries `measurement_fidelity="read-payload-proxy"` and is
`provisional`: it **cannot** reach `recommend_adopt`. Adoption requires the full-session
A/B AND a model-graded quality pair on the `locate`/`shape` strata (the deterministic
grader covers fabrication + the negative invariant only). That full-session A/B is the
documented follow-up.

## Numbers are NON-CANONICAL

Every token / delta / p-value the harness prints is a work-box, runtime-only,
**non-canonical** number (see `MEMORY: project-ec2-execution-env`). Do **not** copy one
into committed markdown — `scripts/check-no-perf-numbers.py` scans `*.md`/`*.typ` and
would (correctly) flag it. The committed artifacts are the harness CODE, the PREREG, the
frozen tasks, the corpus manifest, and the verdict SCHEMA — never a measured number and
never a real session transcript.
