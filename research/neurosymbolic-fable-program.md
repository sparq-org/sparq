<!-- [FABLE-5] Design record for the #1111 neurosymbolic re-attempt-on-Fable program
(epics sq-2m6zm / sq-mztg8 / sq-2489d; maintainer decision 2026-07-05). 🤖 SPARQ agent.
This is the ONE design-record for the program; child beads carry the implementation. -->

# Neurosymbolic self-built KB — the Fable re-attempt program (#1111)

> 🤖 **SPARQ agent** (Claude Fable 5, front decomposition stage). Decomposition record
> for GitHub issue [#1111](https://github.com/sparq-org/sparq/issues/1111) — the
> maintainer-flagged re-attempt of the neurosymbolic knowledge-base direction on a
> stronger model, now that Fable is available. One record, N disjoint child beads;
> no implementation happens in this PR.

## 1. Mandate — the maintainer's decision and autonomy grant

Issue #1111 tagged the whole neurosymbolic-KB direction `revisit-with-fable` because
every measured verdict in it is **model-dependent** (the headline finding is literally
that the *agent's fluency* drives outcomes). On **2026-07-05T16:56:35Z** the maintainer
ruled on #1111, verbatim:

> "Please start with A. Then move towards having A + B. Then move towards C as you
> advise. Please do not wait for my sign off before starting B or C, do this as soon
> as you are ready."

This one comment decides **two ladders at once**, and under the standing
proceed-without-greenlight rule (`.claude/skills/proceed-and-document/SKILL.md`) both
readings are treated as DECIDED — recorded here, steered post-hoc via the launch
comment on #1111:

1. **The #1111 thread ladder** (the issue body's three threads):
   **A** = PKG dogfooding (epic `sq-2m6zm`) → **A+B** adds the FO/LLM-fluency bridge
   (epics `sq-mztg8`, `sq-8dkyo`) → **C** = provenance-driven GenAI ingestion
   (epic `sq-2489d`, issue #1110). Start A now; start B and C as soon as ready, with
   no sign-off waits.
2. **The `sq-tzars.5` §6.4 calibration ladder** (the option tables posted on #1111
   on 2026-07-05, immediately preceding the decision comment):
   **A** = ship hedging as *asserted assurance* (already the shipped design intent)
   → **+B** = surface an explicit "UNCALIBRATED, hand-authored" disclaimer in KB
   outputs → **C** = build a reliability-diagram calibration harness. Same
   no-sign-off-wait grant between rungs.

### 1.1 The §6.1 / §6.2 / §6.4 rulings (recorded)

From the same decision, applied to the `research/provenance-driven-genai-kb.md` §6
open questions that `sq-tzars.5` packaged for the maintainer:

- **§6.1 DQV posture — DECIDED: ratify (option A).** The merged `sq-2489d.3` DQV
  adoption stands (full adoption in `crates/sparq-kb/ingest/pkg.ttl`, byte-pinned in
  `vocab.rs`, W3C-Note caveat already recorded in the crate's `PROVENANCE.md`). No
  code change; the ratification is recorded here and written into the master record
  by bead `sq-2489d.8`.
- **§6.2 research-verdict enum — DECIDED: keep `{yes,no,partial}` + assurance.**
  No `pkg:ResearchVerdict` enum now; a convention-mapping comment
  (`holds=yes`, `refuted=no`, `uncertain=partial+secx:Conjectured`,
  `superseded=dcterms:replaces`) goes next to `pkg:verdict` (bead `sq-2489d.8`).
  A richer enum is deferred until **real** literature ingestion (the `sq-tzars.6` /
  `sq-tzars.9` pipeline) shows *measured* strain — the "strain shows at ingestion"
  claim was anticipatory, not observed.
- **§6.4 confidence calibration — DECIDED: the A → A+B → C ladder above.** Rung A is
  zero-code (already the design intent in `crates/sparq-nlq/src/qualify.rs`, which is
  explicit that the hedge is a monotone reading of hand-authored estimates, **not** a
  calibrated probability). Rung B and rung C are beads `sq-2489d.9` and `sq-2489d.10`.
  Until rung C's measurement exists and passes its pre-registered bar, **no artifact
  anywhere may claim "calibrated confidence"**.

## 2. Grounding — what actually exists (verified against the tree, not the issue)

The #1111 framing checks out against the code; nothing in it is stale, with one
refinement worth stating precisely (who the *subject model* was in each benchmark):

- **`bench/pkg-dogfood/`** — the real-transcript 3-arm token A/B is re-runnable as
  built: frozen `tasks/abm_tasks.json` (30 tasks, 4 strata), `tokens_real.py` mines
  cache-discounted effective input tokens straight from sub-agent transcript
  `message.usage`, `analyze3.py` emits the model-price-weighted verdict. The measured
  record ([RESULTS.md](../bench/pkg-dogfood/RESULTS.md)) found `pkg-query` roughly
  halves the orchestrator's effective tokens vs doc-read at equal quality, and a
  Haiku NL-tool delegation is materially cheaper again — **ADOPTED**. The *subject*
  in arms A/B was **Opus** (arm C: Haiku).
- **`bench/fo-km/`** — Metric-1 is re-runnable as built: frozen 16-task
  `tasks.jsonl`, four overlays (`no-fo` / `gufo` / `dolce-dul` / `schema-org`), one
  fresh NL-tool sub-agent per (arm, task). The measured record
  ([RESULTS.md](../bench/fo-km/RESULTS.md)) found **schema.org-as-top** clearly beats
  gUFO and DOLCE-DUL on agent KM-task accuracy, with LLM fluency (not formal
  richness) as the driver; the facade/hide follow-ups (`sq-jw312` / `sq-5rizt`)
  confirmed a rich-FO backbone behind a schema.org facade does **not** recover the
  pure-schema.org score. The *subject* was **Haiku** for every (arm, task).
- **`crates/sparq-nlq/src/qualify.rs`** — answer-qualification (hedge + abstention)
  is merged (`sq-2489d.2`) and honest in its doc comments ("**not** a calibrated
  probability — a monotone reading of asserted, hand-authored `pkg:confidence`
  estimates"), but the disclaimer is not yet surfaced in the *rendered output* —
  that is exactly §6.4 rung B.
- **The `sq-tzars` research-KB estate** (design record
  [research-kb-program.md](./research-kb-program.md)) is 7/9 complete; the
  literature **pilot loop** `sq-tzars.9` (hard-capped, dry-run-first, pre-registered
  precision bar, maintainer-armed) is now **dependency-clear** — all four of its
  blockers are done. It is the thread-C ingestion vehicle; nothing new needs
  designing there.
- **Prior art beads that must not be duplicated:** `sq-givgo` (FO round 2 — broaden
  the overlay *set* with gist/BFO/etc.), `sq-2489d.6` (end-to-end token A/B of the
  provenance-driven KB), `sq-2m6zm.7`/`sq-2m6zm.8` (nlq-endpoint productization).
  This program *re-runs the existing arms under a new subject model*; it does not
  widen the arm set (that stays `sq-givgo`).

## 3. Why "re-run first" is the highest-value move

Every load-bearing verdict in this direction was measured with **Opus or Haiku as the
subject**. The #1111 thesis is that these verdicts may shift under a stronger model:
richer foundational ontologies (gUFO / DOLCE / BFO) may become usable at Fable-level
fluency (reopening the FO choice), and the doc-read-vs-pkg-query economics may change
when the orchestrator is Fable. The harnesses were deliberately built so this is a
**re-run, not a rebuild** — frozen task sets, deterministic graders, real-transcript
token mining.

Two honesty notes shape the design:

1. **This project has reversed itself on proxy measurements three times** (the
   pkg-dogfood char-proxy inversion, the ast-grep/outline verdict, the terse token
   thesis). The re-run therefore reuses the *real-transcript* instruments unchanged,
   and a **null result (verdicts hold) is an acceptable, honest outcome** — the
   program is built to return it.
2. **Model-identity capture is mandatory.** Fable sessions have been observed to
   silently serve a different model mid-run under some conditions. A "Fable-subject"
   benchmark row is only valid if the transcript's `message.usage`/`message.model`
   confirms the serving model; non-Fable-served tasks are flagged and excluded, never
   counted as Fable.

## 4. The decomposition (A → A+B → C)

Five new child beads, parented under the **existing** epics (no new umbrella epic),
plus one existing bead sequenced. No two parallel beads share a file; the single
shared-file pair (`pkg.ttl` in `sq-2489d.8` and `sq-mztg8.5`) is serialized by a
dependency edge.

### Phase A — now (all three immediately dispatchable)

| Bead | Epic | Surface / files | Tier | Invariant (load-bearing) | Acceptance |
| --- | --- | --- | --- | --- | --- |
| `sq-2m6zm.9` **Fable benchmark re-run** (rung 1, P1) | `sq-2m6zm` | `bench/pkg-dogfood/RESULTS.md`, `bench/fo-km/RESULTS.md` + new run artifacts only | opus | Re-run-not-rebuild: frozen tasks/graders/stats byte-unchanged; results append-only; per-task serving-model id recorded | `analyze3.py` + fo-km scoring green over the Fable-run artifacts; verdicts recorded verbatim with a model-id column; `git diff` shows zero edits to tasks/graders |
| `sq-2489d.8` **Rulings bookkeeping** (P2) | `sq-2489d` | `research/provenance-driven-genai-kb.md` §6, `crates/sparq-kb/ingest/pkg.ttl` (comment-only) | haiku | Comment-only ontology change — zero triple diff, `vocab.rs` pins + SHACL untouched; no calibrated-confidence claim introduced | `cargo test -p sparq-kb --features literature` green |
| `sq-2489d.9` **UNCALIBRATED disclaimer** (§6.4 rung B, P1) | `sq-2489d` | `crates/sparq-nlq/src/qualify.rs` (+ its direct unit test) | haiku | Every confidence-bearing qualified answer surfaces the uncalibrated/hand-authored disclaimer; NlqConfig defaults otherwise unchanged | `cargo test -p sparq-nlq` green incl. the new direct disclaimer test |

### Phase A+B — verdict-gated (dep-blocked)

| Bead | Epic | Surface / files | Tier | Invariant | Acceptance | Blocked by |
| --- | --- | --- | --- | --- | --- | --- |
| `sq-mztg8.5` **Re-open the FO choice per the Fable verdict** (P2) | `sq-mztg8` | `crates/sparq-kb/ingest/pkg.ttl` (+ shapes/pins only if typing changes), `research/fo-llm-bridge.md` | sonnet | Verdict-gated: zero ontology diff unless the Fable re-run shows a richer FO at least matching schema.org-as-top on the frozen set; a Fable-only win does not auto-flip the default (the PKG is also queried by cheaper tiers — weigh per-tier fluency explicitly) | `cargo test -p sparq-kb --features literature` green; if typing changed, the fo-km per-arm `pkg-query` spot-check returns the winning arm's gold answers | `sq-2m6zm.9`, `sq-2489d.8` |

### Phase C — ingestion + calibration (sequenced per the grant)

| Bead | Epic | Surface / files | Tier | Invariant | Acceptance | Blocked by |
| --- | --- | --- | --- | --- | --- | --- |
| `sq-tzars.9` **literature pilot loop** (existing bead, now dep-clear, P1, **maintainer-arm**) | `sq-tzars` | `crates/sparq-kb/src/bin/literature_pilot.rs` (per its own spec) | opus | KB never mutated before the pre-registered bar passes; caps fail-stop; metrics verbatim append-only | per its own spec (`cargo test -p sparq-kb --features literature,literature-live`) | — (dispatchable now) |
| `sq-2489d.10` **reliability-diagram harness** (§6.4 rung C, P2) | `sq-2489d` | `bench/confidence-calibration/**` (new dir) | sonnet | Pre-registered bins/bar before data; abstains below the bar — a calibrated-confidence claim is impossible without a passing measured result; hand-authored tier alone recorded as insufficient | fixture-below-bar run abstains, fixture-above-bar run emits a verdict; both in a documented smoke test | `sq-tzars.9` |

**Dependency edges added:** `sq-2m6zm.9 → sq-mztg8.5`, `sq-2489d.8 → sq-mztg8.5`
(pkg.ttl serialization), `sq-tzars.9 → sq-2489d.10` (machine-tier findings are the
calibration input). No cycles (`bd dep cycles` clean).

**Immediately dispatchable:** `sq-2m6zm.9`, `sq-2489d.8`, `sq-2489d.9`, and the
pre-existing `sq-tzars.9` (maintainer-armed at PR time per its own spec — it writes
the first live data into the KB). The maintainer's "do not wait for my sign off
before starting B or C" applies to *starting work*; it does not convert `sq-tzars.9`
live-run PRs to fleet auto-arm.

## 5. Sequencing rationale (the "as you advise" part of the grant)

- **A first, and A's first bead is the re-run**, because every downstream choice
  (the FO reopen, how much facade machinery to keep, whether the NL-tool delegation
  pattern still pays under a Fable orchestrator) is gated on whether the
  Opus/Haiku-era verdicts survive the subject-model change. Re-running two existing
  harnesses is the cheapest way to buy that information.
- **B is verdict-gated, not time-gated.** Starting B "as soon as ready" means: the
  moment `sq-2m6zm.9` lands its fo-km-on-Fable section, `sq-mztg8.5` is ready. If the
  verdict holds, B collapses to a one-line ratification — that is the honest,
  intended cheap path.
- **C's ingestion vehicle already exists** (`sq-tzars.9`, de-risked: hard caps,
  dry-run default, pre-registered precision bar) and is dependency-clear, so C starts
  now in parallel — the grant explicitly authorizes this. The §6.4 rung-C calibration
  harness waits for the pilot's machine-tier findings because calibrating on the 11
  hand-authored findings alone (all clustered at the top of the confidence range)
  would be a near-vacuous measurement, and the harness is required to say so.
- **What this program deliberately does NOT do:** widen the FO arm set (that is
  `sq-givgo`), rebuild any harness, create a new umbrella epic, or make any
  cryptographic/privacy claim (the PKG reuses the `sig-impl` assertion vocabulary
  lineage, but nothing here touches the ZK/MPC verification surface or its pending
  external-audit status).

## 6. Risks and honest caveats

- **Cost/scale:** the re-run dispatches on the order of a hundred fresh sub-agents
  with Fable as the subject — materially more expensive per task than the original
  Haiku/Opus runs. The bead is tiered `opus` for orchestration+interpretation, but
  the *subject* arms must be Fable; the implementing agent should batch and respect
  the existing harness's counterbalancing rather than invent a cheaper shortcut.
- **Silent model substitution** (§3, note 2) can quietly turn a "Fable" arm into a
  mixed-model arm; the per-task model-id column is the guard, and the analysis must
  report the exclusion count.
- **Verdict-shift interpretation:** if the fo-km verdict *partially* shifts (e.g. a
  richer FO ties for Fable but still loses for Haiku), the right call is per-tier
  documentation, not a global flip — `sq-mztg8.5`'s invariant encodes this.
- **Work-box measurements are non-canonical** — no timing/latency figures from these
  runs get baked into docs or tests; token/accuracy verdicts live in `bench/` only.
