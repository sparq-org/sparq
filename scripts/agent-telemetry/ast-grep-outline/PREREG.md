<!-- sq-lhwo.4 (epic sq-lhwo). 🤖 SPARQ agent — the cache-discounted
token A/B PRE-REGISTRATION for the ast-grep+outline intervention. Design record / shared
protocol: research/dogfooding-sparq-knowledge-graph.md §5.1-§5.6. This file fixes
the hypotheses, arms, metric, and KILL criteria BEFORE any run; editing it to fit a
result is forbidden. No measured number lives here (check-no-perf-numbers.py). -->

# PRE-REGISTRATION — ast-grep+outline token A/B (sq-lhwo.4)

This fixes the hypotheses, arms, metric, and kill criteria **BEFORE any measurement**
(`research/dogfooding-sparq-knowledge-graph.md` §5.1, "declare BEFORE running"). It is
the FIRM A/B deferred from sq-lhwo.2 (where the skill shipped and a directional,
advisory byte-cost pilot ran). It reuses the SHARED §5 protocol + verdict object — the
same engine the PKG track (sq-2m6zm.4) pre-registers — applied to a different
intervention: **ast-grep + outline-before-read vs `grep -rn` + full `Read`**.

> 🤖 **SPARQ agent.** This is the scientific gate on ADOPTING ast-grep+outline as
> standing agent practice. The verdict must be non-sycophantic: if it does NOT beat a
> *competent* grep+read baseline on the §5.4 bar, that is the finding to report. The
> pilot already found the honest shape (below); this firmly tests it.

## Hypotheses

- **H1 (token):** answering a "where/how/what-is X" code question with **ast-grep +
  the outline-before-read discipline** costs FEWER cache-discounted *effective input
  tokens* than the cheapest plausible **competent** status quo (`grep -rn` for "list
  every X"; a full `Read` of the target file to locate/shape it) — measured PAIRED,
  per task.
- **H2 (quality):** the ast-grep/outline answer is at least as CORRECT/COMPLETE as
  the grep/read answer (no missed hits, no accuracy regression, no fabricated hits).

## The pilot finding this A/B confirms/beats (sq-lhwo.2, advisory)

The directional pilot found **two distinct, honest signals**, and this firm A/B is
designed to reproduce them, not assume them:

1. **Outline-vs-full-Read is the real byte lever** — reading a large file's
   signatures instead of its whole body is a large reduction *on that file*. This is
   the `locate`/`shape` strata.
2. **ast-grep-vs-*competent*-`grep -rn` is roughly byte-neutral on flat "list every
   X" questions** — ast-grep's genuine edge is precision/expressiveness (it filters a
   comment/string false positive a line-regex counts; it matches shapes a regex
   cannot), NOT a raw token cut. This is the `list` stratum, and the harness charges
   ast-grep its FULL hit-list payload at the same rate as grep so the result is
   measured, not rigged.

## Arms (counterbalanced within task; charge each arm ALL its costs — §5.1)

- **Arm A — grep + full Read (the COMPETENT baseline):**
  - `list` task: `grep -rn <regex> <paths>` (a competent grep, **not** a strawman
    "open every file in the tree"). A strawman arm A would rig the comparison.
  - `locate`/`shape` task: a **full `Read`** of the target file (you do not yet know
    the offset/limit to read just a span — so the honest baseline reads the whole
    file to answer a "where/what is X" question).
- **Arm B — ast-grep + outline:**
  - `list` task: the ast-grep YAML-rule / `--pattern` hit list (one `file:line:
    signature` line per hit), charged in full.
  - `locate`/`shape` task: the ast-grep **signature outline** (answer-sized), not the
    file body.
  - Charged ALL of arm B's costs per §5.1: the printed hit/outline payload, **plus**
    the amortised one-time tool/skill-definition tax `(tool_def)/N` (the ast-grep CLI
    has **no** per-turn prefix tax — §1.6 reject — but the SKILL description sits in
    the prompt; charge it once, amortised) and `(index_build)/N` if a persistent
    index is used. The CLI route used here builds no index, so `index_build = 0`.

## Metric — cache-discounted effective input tokens (§5.1)

```
effective_input = 1.0*fresh + 0.1*cache_read + 1.25*cache_write
```

(reusing the canonical multipliers in `metrics_row.py` / `agent_telemetry.py`). The
paired statistic is the per-task delta `A − B` (positive = ast-grep/outline cheaper).

### HONEST measurement-fidelity caveat (load-bearing — read this)

The §5 GOLD standard runs each arm as a **separate Claude Code session**, pins model
completions via record/replay, and diffs two `agent_telemetry.py` transcript reports
— capturing per-turn cached-prefix dynamics and model round-trip / repair tokens.
That needs ≥30×2 isolated sessions and an `ANTHROPIC_API_KEY` for the only exact
Claude tokenizer (`count_tokens`; `tiktoken` is wrong for Claude). **Neither is
available to a single sub-agent in this work-box.** So `measure_ab.py` measures the
**READ-PAYLOAD effective tokens**: the input bytes each arm forces into context,
EXECUTED over the real repo code, converted to tokens by a documented `~3.5 char/tok`
proxy, run through the canonical effective-token formula.

This captures the DOMINANT, model-free driver of the delta (the saving scales with
`file_size / answer_size`) but it is NOT the full-session A/B. It deliberately
EXCLUDES the model round-trip / repair-loop output tokens and the per-turn cached-
prefix dynamics — those need the multi-session harness. Numbers are therefore a
**lower-fidelity proxy**, runtime-only, NON-CANONICAL, never frozen into committed
markdown (`check-no-perf-numbers.py`). Verdict objects from the proxy carry
`measurement="read-payload-proxy"` and **cannot certify `recommend_adopt`** alone:
they have no model-in-the-loop quality pair, so KILL 3 is unsatisfied by construction
until the quality grader is wired (see `grade.py`). The full-session A/B + the
model-graded quality pair is the FOLLOW-UP (tracked as a bead). The committed
artifacts are this protocol + the harness code + the frozen tasks + the verdict
schema.

The proxy is **conservative on the contested arm**: for a flat "list every X" task it
charges ast-grep its full hit-list payload at the same per-byte rate as grep, so the
pilot's byte-neutral `list` finding is reproduced, not assumed; and it amortises arm
B's one-time costs over the small frozen N (a larger real task corpus amortises them
further, helping B). Because both arms use the same proxy rate, the **paired delta is
invariant to the exact char/token constant** — only the absolute magnitudes shift.

## Pre-registered significance bar (frozen — §5.1)

A token WIN (`token_win = true`) requires the paired-median effective-input reduction
of `A − B` to exceed **BOTH**:

1. **≥ 20 %** relative reduction of the paired median (below ~20 % is inside the noise
   of the one independent 15–28 % memory-saving benchmark — not worth the
   maintenance), **AND**
2. **p < 0.05** via the Wilcoxon signed-rank test over **≥ 30 tasks**, with a
   bootstrap 95 % CI on the median delta whose **lower bound is also > 0**,

AND the saving must **not be entirely the cache-discount component** (§5.4: if the
nominal fresh input did not drop, it is a cache-warmth artifact, not a win). In the
read-payload proxy the payload is pure fresh input, so the cache-artifact guard is
trivially satisfied; the guard becomes load-bearing in the full-session harness.

## Stratification (§5.5 anti-selection-bias) — N = 30, ≥4 strata

A point-lookup-heavy set would trivially favour one tool; the frozen `tasks.jsonl`
spans **4 strata** (each reported separately):

- `list` (n=8) — flat "list every X" workspace enumeration (impls of a trait, call
  sites, a code shape). The arm where the pilot expects **byte-neutral** vs grep.
- `locate` (n=9) — "where is the fn that does X" in a big file → outline first, then
  read the span. The arm where the **outline lever** is expected to win big.
- `shape` (n=9) — "what is the public surface / type skeleton of this big file" →
  signatures only. Also an **outline-lever** win.
- `negative` (n=4) — a deliberately out-of-codebase existence check (a fn/trait/macro
  that does not exist). Both arms must return empty cheaply; tests honest abstention
  and that ast-grep does not over-report.

## Kill criteria (frozen — §5.4, applied mechanically by `ab_stats.py`)

- **KILL 1 (token):** paired-median effective reduction `< 20 %`, OR Wilcoxon
  `p ≥ 0.05`, OR the bootstrap median-delta CI includes 0, OR the saving is entirely
  the cache-discount component. → do not adopt for token reasons.
- **KILL 2 (break-even):** `N*` infinite (`C_grepread − C_astgrep ≤ 0`), OR `N*`
  exceeds the realistic recurrence of a question class before the code changes. → net
  loss; kill. (For the CLI route `C_ingest` is just the amortised tool/skill-def, so
  `N*` is small when there is any per-use saving.)
- **KILL 3 (quality):** ast-grep/outline arm MISSES hits the baseline finds, OR adds
  unresolvable/fabricated hits, OR drops paired answer correctness below the baseline.
  → a token saving bought with worse/incomplete answers fails the bar; kill.

## Verdict object (§5.6)

`recommend_adopt = true` requires `token_win` **AND** quality non-regression
(`exec_acc` lower CI ≥ 0 AND hallucination not increased) **AND** a finite
`break_even_N` within a realistic horizon. The decision is made on the verdict
OBJECT, never on a single number (arm-on-verdict discipline). A proxy-only verdict
(no model-graded quality pair) **cannot** reach `recommend_adopt`; it reports the
token signal and an explicit "needs the full-session + quality-graded A/B" off-ramp.
