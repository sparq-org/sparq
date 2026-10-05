<!-- [OPUS-5] sq-2489d.6 (epic sq-2489d). 🤖 SPARQ agent — the GenAI-KB Phase 7
PRE-REGISTRATION: does the provenance-driven KB (citations + hedging +
provenance-weighting) change agent outcomes vs the INERT-PKG baseline? Design record:
research/provenance-driven-genai-kb.md §5 Phase 7. Shared A/B contract:
research/dogfooding-sparq-knowledge-graph.md §5.1–§5.6. -->

# PRE-REGISTRATION — provenance-driven KB end-to-end A/B (`sq-2489d.6`, Phase 7)

This fixture **fixes the capabilities under test, the arms, the metrics, the bar and
the kill criteria BEFORE any measurement is run**, per the shared §5.1 rule
("declare BEFORE running"). Editing it after a run, to fit a result, is forbidden.

> 🤖 **SPARQ agent.** This is the *honest payoff test* for the provenance-driven KB.
> Phases 1–5 shipped the capabilities; **shipping a capability is not evidence that it
> pays**. The verdict this fixture produces decides, per capability, whether it is kept
> — "keep only what measurably pays". A capability that does not clear its bar is
> reported as not clearing it; that is the finding, not a failure of the harness.

It reuses `bench/pkg-dogfood/` end to end: the frozen task substrate
(`tasks/abm_tasks.json`), the real-transcript token miner (`tokens_real.py`), the
model-price weighting (`analyze3.py`), and the frozen statistics + thresholds
(`stats.py`). It introduces **no second threshold family** — see "Thresholds" below.

## Status: NOT YET RUNNABLE — three declared blockers

The verdict object cannot honestly return `honest=true` until all three clear. The
analyzer (`prov_ab.py`) enforces each mechanically, so this section cannot rot into a
stale caveat:

1. **Canonical A/B host — `needs-maintainer-steer`.** Every token figure is
   host-sensitive, and the repo's canonical-vs-work-box discipline forbids presenting a
   work-box run as a result. The maintainer designates the host; the run declares it in
   `runmeta.json` (`canonical_host`). Absent → `honest=false`.
2. **Fable re-run (#1111).** Verdicts are model-dependent; the design record requires a
   re-run under Fable before any "abandon"/"adopt" is treated as final. Absent →
   `honest=false`.
3. **Arm wiring.** The Phase-1/2 capabilities live in `sparq-nlq` behind the
   `citations` feature (`cite.rs`, `qualify.rs`); the agent-facing `pkg-query --json`
   envelope (`NlToolResult`) does **not** yet carry citations or qualification. Until it
   does, arms P1/P2 cannot be driven through the tool an agent actually calls, and a
   "treatment" arm that does not differ from the baseline is a placebo. The analyzer
   requires per-arm capability *evidence* in the answer records and reports
   `arm_wiring_verified=false` otherwise.

## What is under test

Three capabilities, each measured **independently against the same baseline** so that a
capability that pays is not carried by one that does not:

| Capability | Ships in | Phase | Treatment arm |
|---|---|---|---|
| `citations` | `sparq-nlq::cite` (PKG-native citation renderer, `Answer::citations`) | 1 | **P1** |
| `hedging` | `sparq-nlq::qualify` (assurance verb-hedge + confidence band + `min_confidence` abstention) | 2 | **P2** |
| `provenance_weighting` | `sparq-vectors` provenance-weighted retrieval | 4 | **P3** |

**Baseline arm P0 — the INERT PKG:** the identical question answered from the identical
graph through `pkg-query` with all three capabilities off — rows only, no citation
footnotes, no hedge, no abstention floor, unweighted retrieval. P0 is today's shipped
agent-facing behaviour, so the comparison is against the real status quo and not a
strawman.

Each of the `N × 4` cells is answered by a **fresh sub-agent** whose brief opens with
the attribution tag `[ABM task=<id> arm=<P0|P1|P2|P3>]` (the same tag protocol
`tokens_real.py` already mines for the A/B/C run — extended to the P-arms). Fresh
sub-agents mean no context bleeds between cells; arm order is counterbalanced across
tasks, never before/after (the §5.5 warm-cache masquerade trap).

## The direction swap (read this before reading the verdict)

The shared §5 bar was written for an **efficiency-adding** treatment: the treatment must
be *cheaper* at non-inferior quality. Phase 7's treatments are **quality-adding**:
citations and hedges cost extra tokens *by construction*. Applying the token-reduction
bar unchanged would reject every capability before it is measured, and inventing a
second threshold family is explicitly disallowed
(`agent-effectiveness-program.md` §1.5: "No competing schema and no second threshold
set"). So Phase 7 reuses the **same statistics and the same constants** with the roles
of the two axes **swapped**, and says so on the face of the verdict object:

- **quality is the superiority axis** — the treatment must *beat* the baseline;
- **cost is the non-inferiority axis** — the treatment must not cost meaningfully more,
  where "meaningfully" reuses the frozen `MIN_RELATIVE_REDUCTION = 20 %` constant from
  `stats.py` as the overhead ceiling (mirrored, not a new number);
- the `token_win` field of the §5.6 object therefore carries **"cleared the cost
  criterion"** — for Phase 7 that is cost non-inferiority. The verdict object records
  `_detail.criterion` so no reader can mistake which direction was tested.

## Metrics

**Cost** — model-price-weighted `$` per task, computed by `analyze3.py`'s price table
from the **real cache-discounted effective input tokens** mined from each cell's
transcript `message.usage` (`1.0·input + 0.1·cache_read + 1.25·cache_creation`, the
canonical §5.1 multipliers). No char proxy: the #1078 proxy *inverted* a verdict once
already (`RESULTS.md`, CRITICAL LESSON), so a proxy run is refused by the honesty gate.

**Quality** — one pre-registered per-task `outcome_score ∈ [0,1]` per capability. The
axes are *agent-outcome* axes, deliberately not "does the feature emit what it says it
emits" (that is Phase 1/2's own acceptance metric, and re-using it here would make the
baseline score 0 by construction — a rigged win):

- **`citations`** — the **stale-fact catch rate** on seeded `stale-fact-trap` tasks:
  the agent is asked a question whose PKG answer has been perturbed away from the
  source of record. With a resolvable citation the agent can check the source and flag
  the discrepancy; without one it propagates the stale fact. Non-trap tasks score
  gold-key coverage, so a capability cannot buy trap-catching with ordinary-task
  regressions. Two mandatory guards, both mechanical: **citation-resolution rate** (every
  emitted citation IRI must be present in the graph) and **fabricated-citation count**
  (must be 0 — a fabricated citation is a hard kill regardless of the catch rate).
- **`hedging`** — **abstention precision/recall** over the `abstain-trap` positive class
  (the frozen task set's `negative` stratum: deliberately out-of-KG questions), paired
  with an over-abstention penalty on the answerable tasks. Hedging that buys abstention
  recall by abstaining everywhere scores zero.
- **`provenance_weighting`** — **no agent-facing substrate exists.** Phase 4's metric is
  offline Hits@k / MRR on a held-out link-prediction split, which is *not* an
  agent-outcome measure and cannot be substituted for one. This capability is
  pre-registered as **blocked**, and the analyzer emits it as blocked rather than
  silently omitting it. Unblocking it needs a retrieval-backed agent task set (a
  separate bead, not this one).

## Seeded `stale-fact-trap` substrate (frozen construction)

Traps are generated **mechanically**, never hand-picked, so the trap set cannot be
selected to favour an arm:

1. Copy the ingested PKG; for a deterministic pseudo-random sample of Findings
   (fixed seed), perturb one asserted object literal away from its source of record.
2. The trap's `catch_keys` are derived from the *unperturbed* source text, so
   "the agent caught it" is a substring resolution against a key authored from the
   source document — not from the PKG under test (the §5.5 circular-measurement trap).
3. Both arms answer over the *same* perturbed graph. The only difference is whether the
   answer carries a resolvable citation to follow.
4. The perturbation manifest is written alongside the tokens and is part of the run
   record; a verdict computed over an unrecorded trap set is refused.

## Thresholds (frozen — imported, not restated)

`prov_ab.py` imports `MIN_RELATIVE_REDUCTION`, `MAX_P`, `MIN_N`, `BOOTSTRAP_ITERS` and
`BOOTSTRAP_SEED` from `stats.py` at runtime. There is deliberately no copy of those
numbers in this file: a copy is a second source of truth waiting to drift.

- **Quality superiority** — paired per-task delta (treatment − baseline) with a strictly
  positive median, Wilcoxon signed-rank `p < MAX_P`, and a bootstrap 95 % CI on the
  median delta whose lower bound is `> 0`. The superiority test runs on the capability's
  **trap subset** — the tasks where the capability can act at all. This is declared here
  because it cuts both ways: diluting the test with tasks the capability cannot affect
  would drive the paired median to exactly `0` and make superiority unreachable *by
  construction*, but restricting the test also means the trap subset alone must carry the
  power bar. The non-trap tasks are **not** discarded — they carry the non-regression
  guard, scored on the same capability-aware axis, so `hedging` cannot buy abstention
  recall by abstaining on answerable questions.
- **Cost non-inferiority** — median relative overhead `≤ MIN_RELATIVE_REDUCTION`, and the
  bootstrap 95 % CI on the paired cost delta (baseline − treatment) has a lower bound
  above `−MIN_RELATIVE_REDUCTION × median baseline cost`. Cost is measured over **all**
  paired tasks: the capability bills its overhead on every question, not only the traps.
- **Power** — `N ≥ MIN_N` paired tasks per capability **and** `N ≥ MIN_N` traps (the
  binding N, since the superiority test only sees the trap subset). Below it the honest
  verdict is "no evidence", never "no effect" and never "win". Note this is not satisfied
  today: the frozen substrate carries 6 `negative`-stratum tasks, so `hedging` needs its
  abstain-trap class widened before it can be measured, and `citations` needs the seeding
  step above to produce its traps.

## Kill criteria (frozen — the shared §5.4 spec, mapped to the swapped direction)

- **KILL 1 (cost)** — median overhead exceeds the ceiling, or the cost CI lower bound
  falls below the non-inferiority margin. → the capability is not free enough to keep on
  outcome grounds alone.
- **KILL 2 (break-even)** — `break_even_N` in the token sense is **not defined** for a
  quality-adding capability, so it is reported as `null` with a reason rather than
  fabricated. Its analogue, `cost_per_extra_caught_defect`, is reported when the
  treatment catches strictly more; if the treatment catches no more defects than the
  baseline, the extra spend buys nothing and this kill fires.
- **KILL 3 (quality)** — the treatment drops paired answer accuracy, raises the
  unresolvable-claim count, emits **any** fabricated citation, or (for `hedging`)
  regresses abstention precision. A cost saving or a catch-rate gain bought with a
  fabricated source fails the bar outright.

## The honesty gate (`honest` is computed, never asserted)

`honest = true` requires **all** of: real-transcript tokens (not a proxy);
counterbalanced arm order; `N ≥ MIN_N` paired tasks **and** `N ≥ MIN_N` traps;
capability evidence present in the
treatment arm (`arm_wiring_verified` — blocker 3); a declared canonical host (blocker 1);
a recorded Fable re-run (blocker 2); a mechanical arm-blinded grader; and the outcome
substrate present for that capability. Each is reported individually, so a `false` names
which condition failed.

## Verdict object (§5.6, one per capability + a roll-up)

```json
{ "token_win": "bool", "token_delta_median_pct": "float", "token_delta_ci": ["lo", "hi"],
  "quality_delta": { "exec_acc": {}, "provenance_completeness": {}, "hallucination_rate": {} },
  "break_even_N": "int|null", "break_even_infinite": "bool",
  "honest": "bool", "recommend_adopt": "bool" }
```

`recommend_adopt = honest AND token_win (cost non-inferiority) AND quality superiority
AND no kill fired`. The decision is made on the OBJECT, never on a single number. Every
number is runtime-only and **NON-CANONICAL**; the committed artifacts are this
pre-registration, the harness code, the synthetic self-test fixture, and the verdict
schema — never a measured figure.

## Reproduce

```bash
# 1. mine the real per-cell tokens from the P-arm transcript dir(s)
python3 bench/pkg-dogfood/tokens_real.py <transcript-dir> tok_prov.json

# 2. per-capability verdict (add --known-iris to grade citations; without it the
#    citations capability is refused rather than graded on self-report)
python3 bench/pkg-dogfood/prov_ab.py \
  --tasks    bench/pkg-dogfood/tasks/abm_tasks.json \
  --tokens   tok_prov.json \
  --answers  answers_prov.json \
  --runmeta  runmeta.json \
  --known-iris pkg-iris.txt \
  --capability-tasks prov_outcomes.json \
  --out prov-verdict.json

# self-test the analyzer against the SYNTHETIC fixture (no measurement involved)
python3 bench/pkg-dogfood/test_prov_ab.py
```
