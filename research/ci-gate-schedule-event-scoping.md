# Should a `schedule`-triggered run's check-runs be siblings of the push gate? — decision record

> Status: **DECISION, DESIGN-ONLY.** This record answers the question raised in
> sparq-org/sparq#5352 (spun out of #3786) and specifies the change; it ships **no
> code**. #3786 deliberately left the question open because a `run.event`-based filter
> changes what the single required check aggregates and must be reviewed on its own
> merits rather than riding along with an incident fix. The implementation is a
> follow-up. Authored by Claude Opus 5 `[OPUS-5]`.
>
> Related: #3773 (advisory must be declared), #3783 (liveness veto), #3785, #3786.

## 1. The question

`ci-summary / gate` — the single required branch-protection check — evaluates a
commit by *discovering* every check-run and Actions workflow-run on that head SHA
(`.github/workflows/ci-summary.yml`, `scripts/ci_summary_gate.py`). Discovery is
keyed on the SHA alone: `repos/{repo}/actions/runs?head_sha={sha}`
(`make_fetch_workflow_runs`, `scripts/ci_summary_gate.py:2158`) plus the commit's
check-runs. Nothing in that query asks *which event produced the run*.

A `schedule`-triggered workflow runs on the default branch's head. So whenever a cron
fires while a push-to-main gate is polling that same head, the cron's check-runs land
in that gate's sibling set. Should they?

## 2. What is actually true today (verified at HEAD)

- **Discovery is event-blind.** The `--jq` projection in `make_fetch_workflow_runs`
  selects `{id, workflow_id, name, path, head_sha, status, conclusion, created_at,
  run_started_at, run_attempt, html_url}` — `event` is not even fetched
  (`scripts/ci_summary_gate.py:2169`). The gate cannot currently distinguish a cron
  sibling from a push sibling.
- **Declared-advisory legs cannot *fail* the gate, but they still *hold* it.** The
  verdict excludes declared names (`render_verdict`, `scripts/ci_summary_gate.py:1514`;
  its `is_advisory` gating filter at `:1614`), but the settle/budget counter does not:
  `pending = sum(1 for r in runs if r.get("status") != "completed")`
  (`scripts/ci_summary_gate.py:1829`) is computed over every non-self, non-draft-artifact
  check-run, advisory included. A cron leg that arrives *pending* therefore still
  re-arms the settle window and consumes the gate's wall-clock budget. This refines
  #5352's premise: declaring a lane advisory removes its power to *fail* the push gate,
  but not its power to *delay* one, so the registry workaround is weaker than the issue
  text assumes.
- **The incident is in-repo, not hypothetical.** The `#3783` liveness veto exists
  because `gate` run 30149978128 on `main` declared a "genuine hang" while three
  `kani` legs — a `schedule` + `workflow_dispatch`-only workflow — had been
  `in_progress` on that head (`scripts/ci_summary_gate.py:85-98`).
- **The advisory registry is already absorbing this category.** Four entries in
  `.github/advisory-registry.json` are declared for a reason that is *not*
  advisory-ness at all but trigger mismatch — `review-lane blind-spot alarm`,
  `CI execution-latency alarm`, `formal-lane no-verdict alarm`, `HEAVY-set drift
  alarm`, each stating some form of "it is scheduled on main, so its check-run lands
  on the same head SHA the push-triggered ci-summary gate polls", and each carrying
  `promotion_criteria: "NEVER promotable"`. A registry whose stated contract is
  "declare a *gate lane* you are deliberately not blocking on" is being used as the
  workaround for a *scoping* defect, one entry per cron lane, forever.
- **The set of exposed lanes is large and includes real correctness lanes.**
  Thirty-four workflows carry a `schedule` trigger. Beyond the four declared alarms,
  undeclared schedule-triggered lanes include `miri.yml`, `metamorph.yml`,
  `differential.yml`, `datalog-souffle.yml`, `shacl-diff-fuzz.yml`,
  `drift-scan.yml`, `dependency-monitoring.yml`,
  `merge-group-watchdog.yml`, `rearm-sweeper.yml`, `retriage.yml`,
  `triage-area.yml`, `promote-on-approval.yml`, `slsa-builder-pin-review.yml`. Each
  currently gates `main`'s push report if its cron overlaps a push gate's poll.
- **Dual-trigger workflows can be *superseded* by their own cron run.** Newest-run
  resolution keys on `workflow_identity` = `id:<workflow_id>`
  (`scripts/ci_summary_gate.py:477`) and orders by `created_at`/run id
  (`_workflow_order_key`, `:492`); every older run of that workflow is a non-event
  regardless of conclusion. `ci.yml` is `push` **and** `schedule` on the same ref —
  its own header documents that "a scheduled nightly runs on `refs/heads/main`, the
  SAME ref a push-to-main run uses" (`.github/workflows/ci.yml:67-72`). When the cron
  fires *after* a push, main's head is the pushed SHA, so both runs sit on one SHA and
  the cron run is the newer one for that `workflow_id`. The push run's legs are then
  dropped as superseded and the gate renders over the **nightly tier's** legs
  instead — a genuinely different job set, since jobs select on the event in both
  directions (`if: github.event_name == 'schedule' || …` at
  `.github/workflows/ci.yml:359`; `if: github.event_name != 'schedule' && …` at
  `:3095`, and inside the multi-line `if:` conditions containing `:3232` and
  `:3486`). *Reasoned from the code paths above; unlike the
  `#3783` case this specific supersession has not been observed in a run log, and the
  window is narrow. It is stated as a consequence, not as an incident.*

## 3. The safety property that makes this decidable

**A `schedule` run can never appear on a merge-authorising ref.** Cron runs execute
on the default branch's head. A `pull_request` gate polls the PR head SHA (which is
not main's head — a PR whose head equals its base has nothing to merge), and a
`merge_group` gate polls an ephemeral queue ref that no cron targets. So excluding
schedule-triggered runs is provably a **no-op on every ref where `gate` authorises a
merge**, and its entire effect is confined to the post-merge push-to-main report,
which authorises nothing. That asymmetry is what lets this be decided without
re-litigating the gate's merge semantics.

## 4. Arguments, honestly

**For keeping cron siblings (the case #3786 declined to dismiss).** A nightly `kani`
or `miri` result *is* genuinely about that SHA; the code it proved is the code that
merged. Dropping it from main's gate drops a true statement about the commit.

**Against.** Three answers, in increasing weight:

1. *The gate is not that signal's channel.* A cron lane's failure is surfaced by its
   own check-run, by the lane's notification, and — for the formal lanes specifically —
   by `formal-alarm.yml`, which exists precisely because "the kani.yml / miri.yml
   formal lanes are deliberately nightly-only … which makes their failure mode
   INVISIBLE" (`.github/workflows/formal-alarm.yml:4-5`). Routing it through the push
   gate adds no reachability, only coupling.
2. *Membership is decided by wall-clock, not by the commit.* Whether `kani` is in
   main's sibling set depends on whether its cron happened to overlap the poll. A
   required check whose membership is a function of the time of day is not a gate over
   the commit; it is a coin flip that sometimes REDs main. The gate's settle window and
   saturation budget are both built on the assumption that siblings arrive as a bounded
   burst *caused by* the event under evaluation — an unattended cron violates that
   assumption by construction.
3. *It cannot protect anything.* Per §3 the effect is confined to a post-merge report.
   The commit has already merged; no cron verdict delivered there can stop it.

The counter-argument survives only as a **visibility** requirement: whatever the rule,
a cron leg's conclusion must remain *printed* in the gate summary. That is cheap and is
part of the decision below.

## 5. Options considered

| | Rule | Verdict |
|---|---|---|
| **A** | Status quo — registry declarations only | **Rejected.** Requires one `NEVER promotable` registry entry per cron lane (four already exist for exactly this reason), does not stop settle re-arm or budget burn (§2, `:1829`), does not touch the supersession path, and mislabels "wrong event" as "advisory" — degrading the one file whose reviewability #3773 was fought for. |
| **B** | Exclude runs whose `event == "schedule"` | **ACCEPTED.** See §6. |
| **C** | General "same triggering event" filter (`run.event == github.event_name`) | **Rejected as unsound.** It drops `workflow_run`-triggered runs, and one of those posts `feature-matrix report` — a check the gate *structurally awaits* and fails closed on (`scripts/ci_summary_gate.py:1841-1849`). C re-opens the #3773 hole along a new axis: silently neutralising a real gate by a metadata rule nobody declared. |
| **D** | Causal/temporal rule (exclude runs created after the gate's own run started) | **Rejected as not implementable soundly.** The runs API exposes no parent-run id, so a legitimately-late `workflow_run` reporter is indistinguishable from an unrelated cron. It would drop the same real checks as C, and race the gate's own start time. |

Note that B is deliberately **narrower than "unrelated events"**: `workflow_dispatch`
stays in scope. A human dispatching a lane on a head SHA is a deliberate, attributable
statement about that commit; excluding it would create a manual-verification blind spot
and buys nothing for the incident class (§2's exposure is unattended crons). So for a
`schedule` + `workflow_dispatch` lane such as `kani.yml`, B keeps a *dispatched* run in
the sibling set — where the advisory registry then decides whether it gates, exactly as
today; `kani` is declared, so it does not — while a *cron* run of the same lane is out of
scope entirely, holding nothing. That split is the intended discrimination, not an
inconsistency: it is the difference between "someone asked for this on this SHA" and
"a timer fired".

## 6. Decision

**A `schedule`-triggered workflow run is not a sibling.** Concretely:

1. **Scope.** A workflow run whose `event` field is exactly `schedule` is out of scope
   for every gate evaluation, on every trigger (`pull_request` / `merge_group` / `push`).
   One unconditional rule; no per-event special-casing, because per §3 the rule is
   vacuous everywhere except the push report.
2. **Three exclusion points, not one.** Out-of-scope runs must be removed from
   (a) check-run resolution, (b) **newest-run candidacy** — modelled on the existing
   `no_leg_ids` parameter of `resolve_newest_workflow_runs`
   (`scripts/ci_summary_gate.py:568`), so a cron run cannot supersede a push run of the
   same `workflow_id` (§2, last bullet) — and (c) the `pending` count at `:1829`, which
   is what actually stops the settle re-arm and the budget burn. Excluding at (a) alone
   would leave both live defects in place.
3. **Fail-closed.** `event` must be added to the `--jq` projection at `:2169`. A run
   whose `event` is absent, empty, or unrecognised is **in scope** (it gates). This
   makes an API-shape change or a projection regression degrade to today's behaviour
   rather than to a silent mass exclusion — the failure mode #3773's registry rule
   guards against with its "no literal anchor is refused" clause.
4. **Loud, never silent.** The gate summary lists every excluded leg by name and
   conclusion under an explicit `out-of-scope (schedule-triggered)` heading, alongside
   the existing `UNDECLARED` diagnostic. A red nightly `kani` on main stays visible in
   the summary; it just stops deciding the verdict.
5. **Rollback.** Reverting the projection line restores today's behaviour exactly (by
   rule 3, no `event` ⇒ nothing excluded), so the change carries a one-line kill switch.

### Tests that must pin it

In `scripts/tests/test_ci_summary_gate.py` (and alongside
`scripts/tests/test_alarm_lanes_non_gating.py`, which covers the registry half):

- a **failing** `schedule`-run leg on the SHA does not fail the gate **and** appears in
  the printed summary;
- a **pending** `schedule`-run leg neither holds the settle window nor produces a
  budget-exhaustion RED;
- a `schedule` run of a dual-trigger workflow does **not** supersede that workflow's
  `push` run: the push run's legs stay authoritative and a red push leg still REDs;
- a run with **no** `event` key gates (fail-closed);
- `workflow_run` and `workflow_dispatch` runs are unaffected — in particular the
  `feature-matrix report` structural await still fires.

Each test must be mutation-checked: inverting the scoping predicate has to turn it red.

## 7. What this does *not* decide

- **`workflow_run`-triggered lanes on main.** `nightly selection-bug alarm`
  (`selection-alarm.yml`) is `workflow_run`, not `schedule`, so its registry entry
  remains necessary under this rule. The four `schedule`-only alarm entries become
  *redundant*, but they must **not** be deleted in the implementing PR — deleting an
  entry changes gating status, and that is a separate, individually-reviewable change.
- **Whether a push-to-main gate should exist at all.** Main's `gate` is a mixed
  post-merge report that authorises nothing; whether it should be replaced by
  lane-specific detectors is out of scope here and overlaps
  `research/ci-gate-slotless-aggregation.md`.
- **`workflow_dispatch` scoping**, deliberately left in scope per §5.

## 8. Follow-up

- Implement §6 behind the fail-closed projection, with the §6 test set — one PR,
  reviewed on its own, per #3786's instruction.
- After it has run on main for a sustained period, sweep the four redundant
  `schedule`-only alarm registry entries in a separate PR.
