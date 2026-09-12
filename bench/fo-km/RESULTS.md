<!-- [OPUS-4.8] sq-mztg8 (FO-KM epic; design research/foundational-ontology-km-benchmark.md,
PR #1106; harness PR #1107). 🤖 SPARQ agent — the MEASURED Metric-1 record. This is the
SANCTIONED numeric home (bench/ is exempt from check-no-perf-numbers.py); no user-facing
markdown repeats these figures. Written while Fable unavailable; flag for re-review when
Fable returns. -->

# RESULTS — FO-KM Metric 1 (agent KM-task accuracy + cost over the PKG)

> 🤖 **SPARQ agent.** Measurement record for **Metric 1** of epic **sq-mztg8** — the
> 4-arm A/B in which the same Haiku NL-tool answers FO-exercising KM questions over the
> **same** Project-Knowledge-Graph (PKG) typed under different foundational-ontology (FO)
> overlays. `bench/` is the AGENTS.md-sanctioned home for measured figures; the numbers
> below live HERE and are not repeated in any user-facing doc.
>
> This is the **verdict** for the harness shipped in PR #1107 (`bench/fo-km/`). The
> pre-registered prior (design §7) was **NEUTRAL** — built to be able to return the null
> honestly. The measured Metric-1 result is **NOT** neutral; see the finding below.

## The question

Does any foundational ontology beat the **no-FO incumbent** — and the named **gUFO**
baseline — on the knowledge-management tasks this repo's PKG actually does, measured by
the **agent's** answer accuracy and model-price-weighted token cost? (Design §5, Metric 1.)

## The four arms

Each arm = the shipped PKG (`pkg.ttl` + `pkg-instances.ttl`) **plus** one FO overlay
loaded via `pkg-query --extra-graph <overlay> --close owl-rl`, so the overlay's
`rdfs:subClassOf` axioms entail the FO-typed facts.

| Arm | Overlay | PKG-class → FO top category |
|---|---|---|
| **no-FO** (incumbent) | `overlays/no-fo.ttl` | (none — the shipped reuse-first PKG) |
| **gUFO** (named baseline) | `overlays/gufo.ttl` | Task→`gufo:Event`; Finding→`gufo:AbstractIndividual`; Source/Technique→`gufo:Object` |
| **DOLCE-DUL** | `overlays/dolce-dul.ttl` | Task→`dul:Action`; Finding→`dul:Description`; Source→`dul:InformationObject`; Technique→`dul:Method` |
| **schema.org-as-top** | `overlays/schema-org.ttl` | Task→`schema:Action`; Finding→`schema:Claim`; Source→`schema:DigitalDocument`; Technique→`schema:HowTo` |

## Method

- **One Haiku NL-tool per (arm, task).** For each of the 4 arms × 16 FO-exercising KM
  tasks (`tasks.jsonl`: TH 7, ER 4, CC 5), a fresh Haiku sub-agent answered the natural-
  language question by driving the `crates/sparq-kb` `pkg-query` helper
  (`--extra-graph <arm overlay> --close owl-rl`) end to end: **introspect → ground → ask**.
  The agent saw only the NL question; it chose the SPARQL, ran it under closure, and read
  back the rows. (This is the same NL-tool envelope measured positive for the PKG-
  answerable class in `bench/pkg-dogfood/RESULTS.md` arm C, here re-run per FO overlay.)
- **Real cache-discounted effective input tokens** were mined straight from each fresh
  sub-agent's transcript `message.usage` block —
  `1.0·input + 0.1·cache_read + 1.25·cache_creation` (the canonical §5.1 multipliers; the
  same formula as `bench/pkg-dogfood/tokens_real.py`). **No `count_tokens` API, no char
  proxy.** Closure build CPU/wall cost is **non-canonical** and is never charged as a token
  cost (design §5.1). See `analyze.py` for the miner + grader.
- **Heuristic deterministic grading.** Accuracy is per-task gold-key coverage with no
  model in the loop: a count/list task is correct when the agent's answer resolves the
  gold count and entity local-names (count match + entity-coverage); a concept-coverage
  (CC) partition task is correct when the answer covers each gold sub-category bucket.
  Honest abstention on a task an arm genuinely cannot answer is scored as an abstain
  (counted separately), not a wrong answer.
- **Prices** (list approx, USD/Mtok): Haiku $1 in / $5 out. Totals are model-price-
  weighted $ over all 16 tasks per arm.

## The measured result

N = 16 FO-exercising tasks, single counterbalanced run, one Haiku NL-tool per (arm, task).

| Arm | accuracy | abstain | median eff. input tok | total $ (16 tasks) |
|---|---|---|---|---|
| **no-FO** (incumbent) | 0.58 | 5/16 | 57,921 | $1.21 |
| **gUFO** | 0.54 | 2/16 | 45,069 | $0.94 |
| **DOLCE-DUL** | 0.64 | 1/16 | 68,502 | $1.50 |
| **schema.org-as-top** | **0.84** | 2/16 | 51,005 | $1.38 |

By task kind (TH type-hierarchy / ER entailment / CC cross-category):

| Arm | TH | ER | CC |
|---|---|---|---|
| no-FO | 0.61 | 0.25 | 0.80 |
| gUFO | 0.37 | 0.50 | 0.80 |
| DOLCE-DUL | 0.62 | 0.50 | 0.80 |
| **schema.org-as-top** | **0.86** | **0.75** | **0.90** |

## The finding

**schema.org-as-top markedly beats gUFO (0.84 vs 0.54) for the agent's KM tasks, and
gUFO scored *below* the no-FO incumbent (0.54 vs 0.58).** The win is consistent across
all three task kinds (schema.org is the top arm on TH, ER, and CC alike).

The driver is **LLM fluency**, not formal ontological richness. The agent wields the
ubiquitous `schema:` vocabulary reliably — it grounds NL questions onto `schema:Claim` /
`schema:DigitalDocument` / `schema:HowTo` / `schema:Action` and writes correct SPARQL —
but fumbles the academic `gufo:` / `dul:` terms it has seen far less in training, choosing
the wrong category or mis-typing the query. The metaphysically richer FOs (gUFO's
endurant/perdurant axis, DOLCE's descriptive layer) bought *more* expressivity but were
*harder for the agent to use correctly*, so their realised accuracy fell. This **confirms
the LLM-fluency hypothesis of research PR #1106** (design §2 fluency stream): for an
agentic KG queried by an LLM, the FO's fluency to the model dominates its formal fit.

DOLCE-DUL edged the incumbent (0.64 vs 0.58) and abstained least (1/16) — its native
method/document/description categories gave the agent reachable targets — but it cost the
most ($1.50) and stayed well behind schema.org. gUFO is the clear loser of the four:
lower accuracy than doing nothing, at a lower token cost that does not redeem it.

## Honest caveats

- **N = 16, single run.** This is one counterbalanced pass over 16 tasks, not a powered
  multi-seed study. The point estimates carry run-to-run variance.
- **Heuristic grading.** Accuracy is a deterministic gold-key/coverage resolver, not a
  semantic judge — it can mis-grade an answer that is right in spirit but phrased so the
  resolver misses a key. **Robustness to grading noise:** the schema.org ≫ gUFO gap is
  *large* (0.30 absolute) and *consistent across all three task kinds* (TH/ER/CC), so the
  direction of the finding is not an artefact of any single task's grader. A grading-noise
  flip of the headline ordering would require errors correlated the same way across all
  three strata, which the per-kind breakdown does not show.
- **This is Metric 1 — the AGENT.** It measures how well an LLM agent *uses* each FO to
  answer KM questions. It does **not** measure a formal-reasoning quality the agent never
  touches. **Metric 2** (the KGE closure-prior MRR via `eval.rs`
  `run_ablation_multiseed_paired`) is **EC2-deferred** (bead **sq-p5ro8**): it needs a
  canonical/quiet box and is a separate phase. A metaphysically richer FO (gUFO/DOLCE)
  could rank *differently* on Metric 2 — where the prior is consumed by a learned model,
  not authored into SPARQL by an LLM — so this Metric-1 verdict is **scoped to the agent
  use-case** and must not be read as a global "schema.org is the best FO" claim.
- **Token cost is informational here, not the decision metric.** Unlike the cross-model
  cost study in `bench/pkg-dogfood/`, all four arms use the same Haiku NL-tool, so the
  $ differences reflect how much introspection each FO drove (richer overlays → more rows
  to read), not a model-tier saving. Accuracy is the decision metric for Metric 1.

## Reproduce

The 4 overlays + 16 tasks are committed (`overlays/`, `tasks.jsonl`); the per-(arm, task)
Haiku NL-tool dispatch is documented in `README.md`. The token miner + coverage grader is
`analyze.py` (run it against a directory of the run's `agent-*.jsonl` transcripts). Task
discrimination — that each FO arm answers under closure while the no-FO arm returns 0 —
is independently checked by `validate_tasks.py` (needs the `close` feature).


---

## RE-RUN — Fable subject (sq-2m6zm.9, 2026-07-05): schema.org's clear win does NOT reproduce — DOLCE-DUL ties

> 🤖 **SPARQ agent** [FABLE-5]. Append-only re-run record for bead **sq-2m6zm.9**
> (#1111 re-attempt program, thread A rung 1; design record
> `research/neurosymbolic-fable-program.md`). Everything above this line is the
> original **Haiku-subject** record, unchanged. Harness **byte-unchanged**: frozen
> `tasks.jsonl`, `overlays/`, `analyze.py` (verified `git diff` clean against
> `origin/main`). Subject substitution only: **Fable (`claude-fable-5`) replaces
> Haiku as the NL-tool subject for every (arm, task) cell.**

### Method delta (vs the original run)

- Same 16 frozen tasks × 4 overlays = 64 cells; **one fresh headless `claude -p`
  Fable session per cell** (`--allowedTools Bash Read Grep Glob Skill`; brief opens
  with the `[FOKM task= arm=]` tag and drives `pkg-query --extra-graph
  overlays/<arm>.ttl --close owl-rl` end to end), graded by the frozen `analyze.py`
  miner + coverage grader.
- **Serving-model gate (bead invariant).** Each transcript's `message.model` mined per
  assistant line (`bench/pkg-dogfood/model_ids.py`). **64/64 cells VALID for
  `claude-fable-5`, 0 excluded** — no silent model substitution observed. Per-task
  evidence table below.
- **Operational note (recorded for honesty).** The first dispatch wave
  (2026-07-05 ≈ 18:07–18:25 UTC) hit the account session limit: 63 of 64 cells
  returned 429 error results (only `th01`/no-fo, the smoke-test cell, completed
  cleanly). The 63 poisoned cells were **discarded entirely and re-run after the
  window reset** (≈ 20:10 UTC) under the same briefs — no partial or mixed transcript
  was counted.

### The measured result (verbatim `analyze.py` output)

```
tasks: 16 {'TH': 7, 'ER': 4, 'CC': 5}
transcripts: 64 | FOKM-tagged: 64

arm                  acc  abst  med_eff_tok  total_$  by-kind
no-fo               0.25     0      216,002     4.27  CC=0.2 ER=0.5 TH=0.14
gufo                0.31     0      182,498     3.85  CC=0.4 ER=0.25 TH=0.29
dolce-dul           0.56     0      176,194     3.64  CC=0.6 ER=0.25 TH=0.71
schema-org          0.56     0      187,845     3.92  CC=0.6 ER=0.25 TH=0.71

wrote /tmp/sq2m6zm9/fokm_fable.json
```

`analyze.py`'s `total_$` column prices at its frozen **Haiku $1/$5** constants — kept
verbatim per the re-run-not-rebuild invariant; with a Fable subject the actual $ is
≈ 10× the input leg + the same output rate. **Accuracy is the decision metric for
Metric 1** (design §5), so the mispriced column is informational only.

### Side-by-side with the Haiku-subject record (accuracy, frozen grader)

| Arm | Haiku subject (above) | Fable subject | abstains (Haiku → Fable) |
|---|---|---|---|
| no-FO (incumbent) | 0.58 | **0.25** | 5 → 0 |
| gUFO | 0.54 | 0.31 | 2 → 0 |
| DOLCE-DUL | 0.64 | **0.56** | 1 → 0 |
| schema.org-as-top | **0.84** | **0.56** | 2 → 0 |

Supplementary split (same frozen tasks + grader, splitting by each task's frozen
per-arm `select` answerability): on the tasks an arm's overlay **can express**,
schema.org still leads — **0.67 over its 12 expressible tasks vs DOLCE-DUL's 0.53
over 15**; the tie on the full set comes from schema.org's 4 `select=null` tasks,
where Fable attempts an answer anyway and misses 3 of 4, while DOLCE recovers its
single null task by cross-inference (1/1).

### Verdict vs the Haiku-era record — PARTIALLY SHIFTED

1. **The headline dominance does not reproduce.** schema.org's +0.20 clear win over
   DOLCE-DUL (0.84 vs 0.64) collapses to a **tie (0.56 vs 0.56)** with a Fable
   subject; schema.org keeps only an expressibility-subset edge (0.67 vs 0.53).
2. **The #1111 fluency thesis is half-confirmed.** The prediction was that richer FOs
   become usable at higher fluency: **true for DOLCE-DUL** (gap to schema.org closes
   from −0.20 to 0.00), **false for gUFO** (0.31 — still clearly behind, as in the
   Haiku era).
3. **FO typing now beats no-FO across the board.** The no-FO incumbent is the *worst*
   arm at Fable fluency (0.25); in the Haiku era gUFO scored below no-FO. Any
   overlay > none, for a strong agent.
4. **Abstention collapses (0 vs 10)** — Fable always attempts an answer. It sometimes
   recovers a formally unanswerable task by cross-inference and sometimes mis-answers
   it; either way the graded denominators differ from the Haiku run, so **absolute
   cross-run levels are not comparable** — only within-run ordering is.
5. **Why every arm's absolute score fell.** Spot-checking graded-wrong cells shows the
   drop is substantially **grader × answer-style interaction**, not a capability
   regression: the frozen deterministic grader rewards verbatim row enumeration,
   exact bucket counts, and Haiku-style abstain phrasing, while Fable summarizes
   ("86 artifacts, in these categories…"), scopes categories differently, and answers
   "0 — the data does not model this distinction" instead of the abstain phrases the
   `is_abstain` regex recognizes. The same grader grades all four arms within the
   run, so the **ordering** (schema.org = DOLCE > gUFO > no-FO) is the valid signal.

**Consequence (gated bead `sq-mztg8.5`, per its invariant):** a Fable-tier tie does
**not** auto-flip the schema.org-as-top default — the PKG is also queried by cheaper
tiers, where the Haiku-era record (0.84 vs 0.64) stands. The per-tier reading this
record supports: **schema.org remains the right default; DOLCE-DUL is no longer
dominated at the Fable tier** (and is the natural second arm to watch in `sq-givgo`
round 2).

### Per-task serving-model ids (validity evidence)

<details>
<summary>64-cell model-id table (mined from transcript <code>message.model</code>; all VALID)</summary>

| task | no-fo | gufo | dolce-dul | schema-org |
|---|---|---|---|---|
| cc01 | claude-fable-5 | claude-fable-5 | claude-fable-5 | claude-fable-5 |
| cc02 | claude-fable-5 | claude-fable-5 | claude-fable-5 | claude-fable-5 |
| cc03 | claude-fable-5 | claude-fable-5 | claude-fable-5 | claude-fable-5 |
| cc04 | claude-fable-5 | claude-fable-5 | claude-fable-5 | claude-fable-5 |
| cc05 | claude-fable-5 | claude-fable-5 | claude-fable-5 | claude-fable-5 |
| er01 | claude-fable-5 | claude-fable-5 | claude-fable-5 | claude-fable-5 |
| er02 | claude-fable-5 | claude-fable-5 | claude-fable-5 | claude-fable-5 |
| er03 | claude-fable-5 | claude-fable-5 | claude-fable-5 | claude-fable-5 |
| er04 | claude-fable-5 | claude-fable-5 | claude-fable-5 | claude-fable-5 |
| th01 | claude-fable-5 | claude-fable-5 | claude-fable-5 | claude-fable-5 |
| th02 | claude-fable-5 | claude-fable-5 | claude-fable-5 | claude-fable-5 |
| th03 | claude-fable-5 | claude-fable-5 | claude-fable-5 | claude-fable-5 |
| th04 | claude-fable-5 | claude-fable-5 | claude-fable-5 | claude-fable-5 |
| th05 | claude-fable-5 | claude-fable-5 | claude-fable-5 | claude-fable-5 |
| th06 | claude-fable-5 | claude-fable-5 | claude-fable-5 | claude-fable-5 |
| th07 | claude-fable-5 | claude-fable-5 | claude-fable-5 | claude-fable-5 |

</details>

### Honest caveats

- **N = 16, single run per subject**; point estimates carry run-to-run variance, and
  the grader is heuristic (see item 5 above — quantified here because the re-run made
  the style sensitivity visible).
- **This is Metric 1 (the agent) only.** Metric 2 (KGE closure-prior MRR) remains
  EC2-deferred (`sq-p5ro8`); nothing here reads on it.
- **All numbers runtime / NON-CANONICAL** (work-box transcripts, list-price
  approximations). Run artifacts are regenerable scratch; the committed artifact is
  this record.

---

## FIXTURE REFRESH — the gold answers above were graded against a STALER PKG (sq-mztg8, 2026-07-26)

> 🤖 **SPARQ agent** [SONNET-4.6]. Append-only integrity note. **No figure above has been
> altered** — both measurement records stand exactly as recorded. What changed is the
> fixture underneath them, and that bears on how the numbers may be re-used.

`tasks.jsonl` is a **projection of `pkg-instances.ttl`**, but `build_tasks.py` originally
hard-coded the gold answers as Python literals. The PKG has since grown, so the committed
gold answers stopped describing the graph the arms query:

| gold quantity | as committed when the runs above were graded | measured 2026-07-26 |
|---|---|---|
| `pkg:Finding` | 11 | 15 |
| `pkg:Source` | 6 | **71** |
| `pkg:Technique` | 5 | 5 |
| `pkg:Task` | 1255 | 1258 |
| information-artifact union (TH1/ER3) | 22 | 91 |
| FO top category (ER2/CC3) | 1277 | 1349 |

Run against the current graph, the pre-refresh corpus produced **24 discrimination
failures** in `validate_tasks.py`. Half the drift had been invisible to validation because
dict-shaped multi-part golds (th03's truth-bearer / information-bearer split) were never
count-checked; that gap is now closed.

**What this does and does not invalidate.**

- It does **not** retract the recorded orderings. Within each run the *same* gold answers
  graded all four arms, so the arm-vs-arm comparison — the only thing either record claims
  — is internally consistent and stands.
- It **does** mean the absolute accuracy levels above are **not comparable to any future run**
  over the refreshed corpus. A future run is a new baseline, not a third point on the same
  series.
- The committed record cannot establish *when* the graph outgrew the golds, so it cannot be
  determined from this repo alone whether either historical run was graded against a graph
  that still matched its fixture. Treat both as measured against their own snapshot.
- Gold answers are now **probed from the live PKG** at build time in `pkg:` domain terms
  (independent of the FO overlay under test), so this specific rot cannot silently recur —
  but the fixture must be **regenerated whenever the PKG instance data changes**.

**A refresh side-effect the next runner must weigh (NOT silently fixed here).** Two
enumeration tasks got materially harder, because their gold answer is now a far longer list:
`th01`'s information-artifact list went 22 → **91** local names, and `th06`'s document list
6 → **71**. `analyze.py` grades entity-coverage at a default threshold of **1.0** — *every*
gold local-name must resolve in the answer text — so these two tasks now require an agent to
enumerate 91 (resp. 71) names verbatim in order to score. That amplifies exactly the
grader × answer-style interaction the Fable re-run documented above (item 5: the frozen
grader rewards verbatim row enumeration, while a stronger model summarises). A future run
should decide **before dispatch** whether to grade these two at a coverage threshold below
1.0 or to re-scope them to a count question, and pre-register that choice. It was
deliberately **not** changed here: moving the grading threshold changes what the metric
means, which is a benchmark-design decision, not a staleness fix.

**Power, restated.** N = 16 remains below the pre-registered **KILL-A** floor of ≥30
FO-exercising tasks (design §7.1), so neither record above is an adoption verdict, only an
ordering. The epic's remaining work is unchanged: grow the TH/ER/CC strata to ≥30
(design §8 step 2), re-run Metric 1 on the refreshed corpus, then Metric 2 on EC2
(`sq-p5ro8`).
