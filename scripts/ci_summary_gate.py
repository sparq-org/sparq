#!/usr/bin/env python3
# ci-summary gate poll loop — the single REQUIRED branch-protection check's brain.
# [FABLE-5] Extracted from the inline bash in .github/workflows/ci-summary.yml so
# the loop is UNIT-TESTABLE (bead sq-90cv4 mandates regression tests over the gate's
# verdict semantics), and extended with the ADAPTIVE SATURATION BUDGET described
# below. The workflow header comment in ci-summary.yml remains the doctrine for WHY
# the gate exists and what it aggregates; this file is the doctrine for HOW the loop
# decides. Invoked by ci-summary.yml with env: REPO, SHA, SELF_RUN_ID (+ GH_TOKEN
# for `gh`).
#
# SEMANTICS (faithful port of the bash — sq-prg4 / sq-ipkku / sq-wjth):
#   * Discovers every check-run AND every Actions workflow-run on the head commit.
#     For each workflow, only its newest run/attempt is authoritative; every job
#     from an older run is a supersession artifact, regardless of conclusion.
#     This gate's own workflow is excluded by SELF_RUN_ID/workflow identity.
#   * pending = check-runs with status != "completed". The settle window is re-armed
#     ONLY by pending work (the sq-ipkku / #997 guard): an injection of
#     already-terminal check-runs can never starve convergence.
#   * A verdict renders only when EVERY discovered sibling is terminal, never before
#     the MIN_POLLS startup floor, and only after SETTLE_POLLS consecutive quiet
#     polls. Verdict: only DECLARED-advisory checks (see §ADVISORY MUST BE DECLARED)
#     are EXCLUDED; a gating check passes iff its conclusion is success/skipped/
#     neutral; an empty stable set passes.
#
# ADVISORY MUST BE DECLARED, NOT INFERRED FROM A NAME (#3773). [OPUS-5] Until
# 2026-07-25 this gate dropped a whole check-run from the gating set whenever its
# DISPLAY NAME matched `\b(advisory|informational)\b`. That was a correctness hole in
# the one check that authorises merges: any job whose name happened to contain those
# words was neutralised wholesale — no waiver, no registry entry, no record — so
# `gate: SUCCESS` over-promised. An adversarial audit (#3773) found FOUR genuinely
# gating checks neutralised that way (the site determinism grep-gate, the #1740
# browserName tripwire, the gui no-sleep-gate, the axe a11y ratchet), and two of
# them were documented in-repo as "HARD" gates while gating nothing.
# THE RULE NOW: a check-run is non-gating ONLY if it is EXPLICITLY DECLARED in
# `.github/advisory-registry.json` (or is on the tiny platform-managed allow-list
# below). Anything else GATES, whatever it is called. Consequences, deliberately:
#   * a name token is DIAGNOSTIC ONLY — `foo (advisory)` with no registry entry
#     GATES, and the verdict prints a loud UNDECLARED note naming it;
#   * the registry declaration is BOUND to the job's stable identity
#     (`workflow file` + `job_id`, enforced by scripts/check-advisory-registry.py
#     C4), so a RENAME can never flip gating status silently: renaming a declared
#     job makes it GATE (no entry matches the new name) and simultaneously REDs the
#     C4 registry check until the declaration is deliberately updated;
#   * a MISSING or unparseable registry is a LOUD, immediate exit-1 (fail-closed):
#     the gate refuses to evaluate rather than silently gating everything;
#   * a registry entry missing any of its five required fields — INCLUDING the
#     `workflow`/`job_id` identity pair C4 binds to — does NOT declare anything; that
#     check keeps gating (fail-closed per entry) and the load prints a warning. The
#     gate and scripts/check-advisory-registry.py require the SAME five fields; when
#     the gate required only three, a 3-field entry bought a silent exclusion that the
#     checker reported as "all clear" (#3774 review, gpt-5.6-sol finding 2(a));
#   * a registry key with NO literal anchor outside its `${{ … }}` expressions is
#     REFUSED for the same reason: it compiles to `.+` and would neutralise every
#     check-run on the commit, `gate` included (finding 2(b)).
# The gate reads the registry from its own checkout, so ci-summary.yml's sparse
# checkout MUST include `.github/advisory-registry.json` (pinned by a wiring test in
# scripts/tests/test_ci_summary_gate.py). Trust model is unchanged: on
# `pull_request` the registry, the workflow files and this script all come from the
# same merge ref that already decided the job names, so nothing widens — the
# difference is that an exclusion is now a reviewable diff in one file instead of an
# invisible consequence of wording.
#   * Exhausting the loop budget with pending == 0 renders the REAL verdict on the
#     final all-terminal set (the #997 graceful timeout), never a blind RED.
#
# ADAPTIVE SATURATION BUDGET (sq-90cv4). The gate is a WAITER occupying a runner
# slot; under org-pool saturation many concurrent gates starve the very builds they
# wait on, the base budget expires with siblings still QUEUED, and the old
# unconditional `exit 1` emitted a FALSE RED on a PR whose real legs never got to
# run (the 2026-07-02 congestion collapse; memory: reference-ci-congestion-collapse).
# Runner saturation is a THROUGHPUT signal, not a hang. So: at base-budget
# exhaustion with pending work, the gate now distinguishes:
#   * STILL SETTLING — the repo's Actions queue is deep (queued workflow-run count
#     >= SAT_QUEUE_MIN) OR sibling completions are still landing (completed count
#     rose over the last PROGRESS_WINDOW polls). Keep polling, at the slower
#     SAT_INTERVAL to cut API pressure, re-checking the signal each poll, up to the
#     ABSOLUTE cap MAX_TOTAL_POLLS.
#   * GENUINE HANG — pending work with an idle queue AND no recent completions
#     AND nothing executing (see §LIVENESS VETO). RED immediately (the old
#     behaviour, now correctly scoped to real hangs).
# The extension NEVER changes what a verdict says: exit 0 still happens ONLY via
# render_verdict over an all-terminal set (or the stable-empty set), so a genuinely
# failing leg still fails and nothing green is synthesised. The absolute cap +
# the workflow-level timeout-minutes bound the wait — no infinite gate.
#
# LIVENESS VETO (#3783). [OPUS-5] The hang heuristic above shipped with TWO
# signals — "the Actions queue is idle" AND "no completions in the last
# PROGRESS_WINDOW polls" — and both are satisfied by a PERFECTLY HEALTHY long
# bounded proof. On 2026-07-25 `gate` run 30149978128 on `main` declared a
# "genuine hang" while three `kani` legs had been `in_progress` for 11 minutes:
# the queue was idle precisely BECAUSE those jobs had dequeued and started, and a
# `kani` harness emits no completion for tens of minutes by design. The two
# signals meant to PROVE a hang are exactly what a live proof looks like.
# THE RULE NOW: an awaited sibling in `in_progress` is POSITIVE LIVENESS EVIDENCE
# and VETOES the genuine-hang verdict (live_siblings()). Queue depth may only
# count toward "hang" when the awaited siblings are `queued` or absent — i.e.
# genuinely not running. The veto is exit-1-only in effect: it can only POSTPONE
# a red to the absolute cap, never synthesise a pass, and the absolute cap plus
# ci-summary.yml's own `timeout-minutes` still bound the wait.
# The #3677 case the detector exists for is UNTOUCHED: an evaporated check-run is
# represented by _workflow_summary_check(force_pending=True), whose status is
# `queued`, so a lost leg still REDs at the base budget. That discrimination
# (in_progress => not a hang; queued/absent => still a hang) is the property the
# tests pin, and it is deliberately name-INDEPENDENT: a hard-coded slow-lane list
# (`kani`, `cargo-fuzz`, coverage shards) would re-introduce exactly the
# rename-fragility #3773 removed from the advisory rule, so liveness is read from
# the platform's own status field instead.
#
# VERDICT TAXONOMY (#3783 ask 3). [OPUS-5] "The gate could not determine an
# answer" and "a gating check failed" are different events and must not read
# identically — conflating them cost repeated wasted diagnosis (#3758/#3765
# unsatisfiable hold, #3781 ready->re-draft race, #3783 this). So the two
# ABSOLUTE-budget exits are now reported as `UNDETERMINED (not a test failure)`,
# naming WHICH could-not-determine it was (siblings still EXECUTING vs the runner
# pool never draining) and stating explicitly that nothing has been shown to be
# broken. Both still exit 1 — an unobserved leg is never assumed green — so the
# fail-closed posture is byte-for-byte unchanged; only the words changed. A
# base-budget GENUINE HANG stays a FAILURE, because nothing executing plus an idle
# queue plus no completions really is a broken pipeline.
#
# FETCH-FAILURE TOLERANCE: the bash `set -e` turned ONE transient `gh api` blip
# into a gate RED. A failed poll is now skipped (state untouched) and only
# MAX_CONSEC_FETCH_FAILURES consecutive failures fail the gate. No data => no
# verdict, so this cannot create a false pass.
#
# FAIL-FAST ON A CONCLUDED GATING FAILURE (2026-07-17 maintainer directive).
# [FABLE-5] Mid-poll, if any GATING leg has CONCLUDED failure, the gate REDs NOW
# instead of waiting for every other sibling to finish: a genuine `failure` in
# the authoritative newest workflow run/attempt is never forgiven, so every
# future render over any superset of this already-resolved sibling set
# REDs anyway — the remaining wait is pure latency on the red verdict (and on the
# fast-fix trigger it fires, ci-summary.yml `fix-ring`). Soundness guards, each
# reusing the FINAL render's own classification (failfast_failures):
#   * the candidate set is the SAME post-forgive_superseded / post-self-exclusion
#     / post-draft-gate-artifact set the final render sees — a cancelled-then-
#     rerun leg or a concurrency race-loser select can never fire it (cancelled
#     is not failure, and forgiven runs are dropped upstream);
#   * DECLARED-advisory legs never fire it (the same is_advisory predicate
#     render_verdict excludes by);
#   * a red leg with a same-tier-normalized-name run still IN PROGRESS (a rerun
#     already underway on the SHA) stands down — the gate keeps waiting on the
#     rerun exactly as before;
#   * the verdict fires only after ONE immediate grace re-poll re-observes the
#     identical concluded-failure set (name+id), dodging API read races; and
#   * it fires only while siblings are still outstanding (pending, or the
#     awaiting-full draft-tier hold) — an all-terminal set renders through the
#     normal settle path, byte-identical to before.
# Fail-fast is an exit-1-only path: it can never conclude success, so every
# exit-0 invariant below is untouched.
#
# AUTHORITATIVE WORKFLOW-RUN RESOLUTION (#3505). [GPT-5.6] A check-run name is not
# a stable attempt identity: distinct workflows may reuse one name, a re-run may
# reuse one Actions run id, and a disabled/rewritten workflow may leave its newest
# green run with no corresponding check-run on the commit. The live fetcher now
# lists Actions workflow-runs for SHA and selects exactly the newest run by
# created_at/id and its newest run_attempt for each workflow_id. Check-runs from
# every older run are NON-EVENTS even when they concluded failure/cancelled;
# completed runs and re-run attempts are read through the attempt-scoped jobs
# endpoint; and a run-level synthetic check preserves the newest run's terminal
# verdict when job check-runs evaporate.
# A newest genuine FAILURE therefore still REDs. A newest CANCELLED run is never
# rendered as failure directly: the resolver asks Actions to re-run it once when
# run_attempt == 1, using both run_attempt (durable server-side marker) and an
# in-process attempted-set (API-lag guard) to bound dispatch. A cancelled retry or
# a dispatch that never advances emits the distinct loud
# "superseded-legs, re-run required" failure. The poll/absolute time budgets are
# unchanged, and re-dispatch remains orchestration only — this waiter executes no
# test command.
#
# DRAFT-TIER INTEGRITY (bead: draft-tier CI). [FABLE-5] Draft PR heads run a REDUCED
# leg set (coverage / bench / CodeQL / heavy shards / wasm-equality skipped — see
# docs/branch-protection.md §Draft-tier CI); the ci-select job NAME carries the
# ", draft-tier" marker on draft-assembled runs, and — the STRUCTURAL mechanism —
# the ci-summary gate job's OWN check-run name is tiered the same way: a draft
# payload emits `gate, draft-tier`, never the required `gate` context, so branch
# protection ({context: "gate"}) is unsatisfiable by any draft-tier run and there
# is no supersession window at the un-draft moment (the required context is simply
# ABSENT until the full-tier run concludes). THE INVARIANT: a draft-tier gate
# result must NEVER admit a PR to the merge queue. This script adds four belts on
# top of the structural name tiering:
#   * DRAFT-GATE-ARTIFACT EXCLUSION — a `gate, draft-tier` check-run left on the
#     SHA by an earlier draft-tier gate run is an aggregator verdict over a
#     superseded assembly, not a leg: it is excluded from every sibling set (its
#     FAILURE must not permanently RED the full-tier gate on the same SHA, and its
#     SUCCESS carries nothing the live evaluation does not re-derive).
#   * SUPERSEDED-RUN FORGIVENESS — un-drafting fires ready_for_review on the SAME
#     head SHA; per-PR concurrency then CANCELS the in-flight draft-tier runs,
#     leaving conclusion=cancelled check-runs on the SHA the fresh full-tier gate
#     polls. A cancelled/stale check-run is excused iff a LATER check-run with the
#     same tier-normalized name exists (any state but cancelled/stale — this gate's
#     own fresh `gate` run supersedes its cancelled predecessor). A cancelled run
#     with NO successor still fails the gate; failure/timed_out are NEVER forgiven.
#     This matches branch protection's own semantics (it reads the LATEST run of a
#     required check name).
#   * STALE DRAFT-TIER LEG SET — a FULL-tier pull_request gate run refuses to
#     conclude success while any draft-tier-marked select check-run INSTANCE lacks
#     its OWN, distinct, strictly-later full-tier (unmarked) same-normalized-name
#     successor. Per-INSTANCE matters: ci.yml, bench.yml, feature-matrix.yml and
#     fuzz.yml all expose the IDENTICAL select check-run name, so the first
#     workflow's full-tier select must not release the hold for the other three
#     (whose full-tier runs may not have registered any check-runs yet). The loop
#     treats an unmatched instance as STILL-SETTLING (the ready_for_review re-runs
#     are seconds away); budget exhaustion in that state is a RED ("stale
#     draft-tier run, full run pending"), never a pass over draft legs.
#   * CONCLUSION-TIME DRAFT RE-CHECK — a DRAFT-tier gate run re-reads the PR's
#     CURRENT draft state from the API immediately before emitting a SUCCESS
#     verdict; if the PR is no longer a draft the gate concludes FAILURE ("stale
#     draft-tier run, full run pending") so the queue can never latch onto a
#     draft-tier green. API failure after retries fail-closes to RED (a draft PR
#     cannot merge anyway, so a false RED here is cheap; a false PASS is the
#     invariant violation).
#
# NO-LEG RUNS AND THE UNSATISFIABLE HOLD (#3781). [OPUS-5] The two rules "a worker PR
# stays DRAFT until reviewed" and "a full-tier gate refuses a draft-tier leg set" are
# each correct and composed into a deadlock with no exit. Measured 3-for-3 on
# 2026-07-25 (#3472/#3468/#3681): `sparq-orchestrator[bot]` re-drafts a freshly-readied
# worker PR ~13 min after the ready, flipping `review:needs` in the same breath; the
# label flip re-triggers ci.yml / bench.yml / feature-matrix.yml / fuzz.yml, whose
# #2546 label-trigger guard skips EVERY root job — so each run's only non-skipped job
# is the unconditional select pre-job, which (the PR now being a draft) came out named
# `…, draft-tier`. Four fresh draft-marked select instances therefore appeared on a
# head with no possible full-tier successor, and the gate burned all 155 polls before
# fail-closed-refusing a leg set with ZERO failing legs. Two additions close it:
#   * NO-LEG RUNS ARE NOT EVIDENCE — ci-select.yml names its job `…, no-leg` (never
#     `…, draft-tier`) on a guarded label-flip no-op, and the gate treats such a run as
#     NON-AUTHORITATIVE: it is excluded from newest-run candidacy and every check-run
#     it produced is a non-event (no_leg_run_ids / resolve_newest_workflow_runs). The
#     previous REAL run of that workflow therefore stays authoritative. This is
#     strictly MORE fail-closed than the pre-#3781 behaviour, where newest-run
#     resolution let a vacuous all-skipped run supersede a real one — erasing a
#     still-in-flight matrix (measured: #3472's real CI run finished at 07:34:08, six
#     minutes AFTER the flip runs completed) or even a real FAILURE.
#   * THE HOLD IS DETECTED WHEN IT IS UNSATISFIABLE — a full-tier PR gate whose
#     siblings have ALL concluded, which still holds on a draft-marked select with no
#     successor, and whose PR reads as CURRENTLY A DRAFT, is waiting for something that
#     cannot happen (only a non-draft payload produces a full-tier select). It REDs
#     immediately with that diagnosis instead of burning ~67 minutes of budget on a
#     refusal already decided. Same verdict, named honestly, ~1 minute in.
#
# Exit-0 paths, exhaustively (fail-fast adds NO exit-0 path — it only ever
# returns 1): (1) render_verdict over a stable-empty set;
# (2) render_verdict over an all-terminal set with zero non-passing GATING checks
# AND every change-based-selection pre-job check green (sq-fmx4u.3: `skipped` is
# satisfied only under a successful selection; a present-but-not-success select
# REDs outright) AND the draft-tier integrity checks above pass. There is no other
# `return 0`.
#
# SLOTLESS EVENT-DRIVEN EVALUATION — `--evaluate` (bead sq-lfmvd, design record
# research/ci-gate-slotless-aggregation.md §2). [FABLE-5] The poll loop above is a
# RESIDENT WAITER: it holds a hosted-runner slot for as long as the slowest sibling
# takes. `--evaluate` is the SAME BRAIN with a different transport — one short-lived
# evaluation, fired by .github/workflows/ci-gate-status.yml on each sibling
# `workflow_run` requested/completed event, publishing its result as a COMMIT STATUS
# on the head SHA instead of as this job's own conclusion. It never waits for a
# sibling: an unsettled set publishes `pending` and exits in seconds.
#
# It re-uses forgive_superseded / is_self / is_draft_gate_artifact / is_advisory /
# draft_selects_unsuperseded / fm_report_status / failfast_failures / render_verdict
# verbatim — the verdict semantics are the ONE implementation, so the two transports
# cannot drift. Only the *when do I look* changes.
#
# The three deliberate DIVERGENCES from the poll loop (each strictly more
# fail-closed, each pinned by scripts/tests/test_ci_summary_gate.py):
#   1. AN EMPTY SIBLING SET NEVER PASSES. The poll loop passes a stable-empty set
#      (a docs-only PR triggering no other workflow). An evaluation is fired BY a
#      sibling run on that SHA, so an empty set means the API has not caught up —
#      an API artifact, not evidence — and the evaluation publishes NOTHING
#      (leaving whatever status is already there), never a green.
#   2. NO INFORMATION => NO WRITE. A fetch failure, or a PR whose live draft state
#      cannot be read, publishes nothing rather than overwriting a verdict with a
#      guess. The next sibling completion re-evaluates; a required-but-absent
#      status blocks the merge in the meantime, which is the safe direction.
#   3. THE STARTUP-RACE FLOOR IS A CONFIRM RE-FETCH (§3.6). Instead of MIN_POLLS,
#      a would-be-terminal observation must be RE-OBSERVED terminal after a short
#      confirm window before any success/failure is published — the settle_polls==2
#      analogue, and simultaneously the fail-fast grace re-poll.
# The draft tier is derived from the PR's LIVE draft state, and a draft-tier
# evaluation publishes to a SEPARATE context (`<context>/draft-tier`), so the
# required context is never written by a reduced-matrix evaluation — the same
# structural invariant the tiered `gate` / `gate, draft-tier` job NAME encodes.
#
# STAGED MIGRATION (design §3.1): the status is published ALONGSIDE the existing
# required `ci-summary / gate` job and is NOT required by the ruleset yet. See
# docs/branch-protection.md §Slotless gate evaluation for the parity phase and the
# maintainer-only ruleset edit that ends it.
#
# Hermetic tests: scripts/tests/test_ci_summary_gate.py (stdlib-only unittest; no
# network — fetchers are injected). Run: python3 scripts/tests/test_ci_summary_gate.py

from __future__ import annotations

import io
import json
import os
import re
import subprocess
import sys
import time
from contextlib import redirect_stdout
from dataclasses import dataclass, field

# [OPUS-5] #3773 — the DECLARED-advisory registry. See §ADVISORY MUST BE DECLARED.
# Path is relative to the repo checkout the gate runs from (ci-summary.yml sparse-
# checks out BOTH this script and this file).
ADVISORY_REGISTRY_PATH = ".github/advisory-registry.json"
# A registry entry declares nothing unless it carries ALL FIVE — the SAME required
# set scripts/check-advisory-registry.py C2/C4 enforce. An under-specified entry must
# not buy an exclusion.
# [OPUS-5] #3774 review (gpt-5.6-sol, finding 2(a)): this tuple used to hold only the
# three bookkeeping fields while the checker required five, so a 3-field entry with NO
# `workflow`/`job_id` neutralised any check-run it named while C4 `continue`d past it
# and the checker printed `all clear`. The identity pair is what C4 binds a
# declaration to, so an entry without it is exactly the entry C4 cannot police:
# requiring it HERE is what makes "an under-specified entry must not buy an exclusion"
# true of the GATE and not merely of the checker.
REGISTRY_REQUIRED_FIELDS = (
    "owner_bead", "promotion_criteria", "registered", "workflow", "job_id",
)
# A registry key is the job's `name:` as written in the workflow YAML, so it may embed
# `${{ matrix.x }}` expressions that only expand at runtime. Each expression matches a
# non-empty run of characters; every other character is matched LITERALLY, whole-name,
# case-insensitively. Nothing else is pattern-like: this is an exact declared-identity
# match, never a substring/word search over the display name.
_YAML_EXPR_RE = re.compile(r"\$\{\{.*?\}\}")
# [OPUS-5] #3773 — DIAGNOSTIC ONLY, never a decision input. The old (removed) rule
# excluded any name matching this; the verdict now prints an UNDECLARED note for a
# check whose name carries the token but which has NO registry declaration, so the
# formerly-silent hole is loud. Wiring this back into is_advisory() would restore the
# defect — scripts/tests/test_ci_summary_gate.py::TestDeclaredAdvisoryRule pins that.
ADVISORY_NAME_TOKEN_RE = re.compile(r"\b(advisory|informational)\b")
# [OPUS-5] PLATFORM-MANAGED advisory check-runs — an EXACT, fail-closed allow-list.
# The registry declares jobs THIS repo authors, keyed on the workflow job it belongs
# to. A GitHub-MANAGED job has no workflow file in this repo at all, so it cannot be
# declared that way — and it therefore GATES, however non-gating it actually is.
# Entries here are matched WHOLE and case-insensitively
# (never as a substring / prefix / wildcard), so an unknown or newly-introduced name
# still GATES: adding a platform surface is a deliberate, reviewed edit to this set.
#
#   • "dependabot" — the sole job of GitHub's managed `Dependabot Updates` workflow
#     (event=dynamic, path `dynamic/dependabot/dependabot-updates`). Verified against
#     the Actions Jobs API over the newest 60 runs of that workflow on this repo: the
#     ONLY job name it has ever emitted is exactly "Dependabot" (37 success /
#     23 failure), so this allow-list is complete for the surface as it stands, and no
#     repo-authored workflow declares a job of that name (a collision would need a new
#     job deliberately named "Dependabot").
#     WHY NON-GATING: this check reports DEPENDABOT'S OWN ability to act on an upstream
#     advisory, not this repo's code health. It concludes `failure` on outcomes nobody
#     in this repo can fix — notably `security_update_not_possible`, i.e. the reachable
#     update path cannot land a version that clears every advisory on the package (live
#     case: main run 30136978362, 2026-07-25T00:46Z, npm `brace-expansion` — the 1.x
#     tree is pinned by `minimatch@3`'s `^1.1.7`, so the only unaffected release, 5.0.8,
#     is unreachable). Under the stop-the-line rule that red halted `main` for a
#     condition with no in-repo remedy.
#     SECURITY POSTURE IS UNCHANGED: nothing here suppresses an alert or a scanner.
#     The Dependabot alert stays OPEN and visible, Dependabot keeps retrying weekly and
#     will open the PR the moment a reachable patch exists, and the actionable Rust
#     dependency-vulnerability GATE — `cargo deny check advisories` inside
#     "supply-chain gates (deny + vet + SBOM + VEX + OpenSSF + js-sbom)" — is untouched
#     and still REDs on a real finding. Honest caveat: that lane covers the CARGO graph;
#     this repo has no in-repo npm vulnerability gate today (the js-sbom step generates
#     CycloneDX SBOMs, it does not fail on advisories), so Dependabot alerts remain the
#     npm surveillance surface — which is exactly why this change must not, and does
#     not, touch alerting.
PLATFORM_MANAGED_ADVISORY_NAMES = frozenset({"dependabot"})
_PASSING = ("success", "skipped", "neutral")
# [FABLE-5] draft-tier CI: the marker ci-select.yml appends to the select job name
# on a draft-assembled run ("select (change-based test selection, draft-tier)").
# The tier travels in the check-run NAME — the same name-as-contract mechanism the
# advisory rule uses — so the gate can partition draft-assembled from full-assembled
# selection check-runs on a head SHA without any extra API surface.
DRAFT_TIER_MARKER = ", draft-tier"
# [OPUS-5] #3781: the marker ci-select.yml appends INSTEAD of ", draft-tier" when the
# calling run is a GUARDED LABEL-FLIP NO-OP — a labeled/unlabeled pull_request event
# whose label is none of ci-full/bench-full/fuzz-full. The #2546 label-trigger guard
# `if:`s off every root job of ci.yml / bench.yml / feature-matrix.yml / fuzz.yml on
# such an event, so the run assembles ZERO legs and that workflow's unconditional
# select pre-job is its ONLY non-skipped job. Such a run is NOT draft-tier evidence and
# NOT full-tier evidence — it is not evidence at all — so the gate treats it as
# NON-AUTHORITATIVE (no_leg_run_ids + resolve_newest_workflow_runs) and it can neither
# create nor discharge a draft-tier hold (draft_selects_unsuperseded).
#
# WHY A WHOLE-RUN EXCLUSION, NOT MERELY A TIER-NEUTRAL SELECT NAME. Measured on sparq
# #3472: the PR was readied at 07:15:37 (full-tier runs dispatched; CI's real matrix ran
# until 07:34:08), re-drafted with `review:needs` at 07:28:50, and the resulting 8
# label-flip runs completed within ~90s with every job skipped. Newest-run resolution
# (#3505) is per-workflow, so those vacuous runs BECOME the authoritative run for CI /
# Benchmarks / feature-matrix / fuzz and their all-`skipped` leg sets REPLACE the real,
# still-in-flight ones. Neutralising only the select NAME would therefore have swapped a
# 155-poll deadlock for a GREEN gate rendered over legs that had not finished. Ignoring
# the RUN instead keeps the previous real run authoritative — strictly more fail-closed:
# a mid-flight run keeps the gate polling, and a FAILED run keeps failing it instead of
# being erased by a label flip.
NO_LEG_MARKER = ", no-leg"
# [FABLE-5] Draft-tier CI: THIS aggregator's own job name is tiered the same way
# (ci-summary.yml `gate` job): a draft-payload run emits the check-run
# `gate, draft-tier` and NEVER the required `gate` context — the structural half
# of the integrity invariant (branch protection's required_status_checks entry
# {context: "gate"} cannot be satisfied by a draft-tier run at all). A
# `gate, draft-tier` check-run left on a SHA by an earlier draft-tier gate run is
# therefore a tier ARTIFACT, not a leg: is_draft_gate_artifact() excludes it from
# the sibling set (see run_gate).
GATE_CHECK_NAME = "gate"
DRAFT_TIER_GATE_NAME = GATE_CHECK_NAME + DRAFT_TIER_MARKER
# Conclusions a LATER same-workflow/name check-run may excuse after authoritative
# workflow-run resolution. Deliberately ONLY check-level supersession artifacts;
# failures from OLDER workflow runs are removed by resolve_newest_workflow_runs,
# while a failure in the newest run/attempt is never forgiven here.
_SUPERSEDABLE = ("cancelled", "stale")
# [FABLE-5] sq-fmx4u.3: the change-based test-selection pre-job (the reusable
# .github/workflows/ci-select.yml job, called from ci.yml + feature-matrix.yml).
# Its check-run name embeds this phrase; scripts/tests/test_ci_select_wiring.py
# pins the workflow job name against this regex so the two cannot drift apart.
SELECT_RE = re.compile(r"change-based test selection")

# [OPUS-5] #4614 (carry-over from the superseded #3765): the shared REMEDY tail for
# both draft-tier refusals — the #3781 unsatisfiable-hold fast fail and
# render_verdict's stale-draft-tier belt. Each is a verdict about WHICH CHECK-RUNS
# EXIST on the head SHA, and re-running the ci-summary workflow re-runs no SELECTING
# workflow, so a bare `gh run rerun` cannot make the missing full-tier select appear
# — it re-reads the same evidence. Saying so IN the message is the point: an
# automated repair lane whose reflex for any RED is "re-run it" otherwise burns
# runner time on a verdict its re-run cannot move.
UNSAT_HOLD_REMEDY = (
    "NOTE FOR AUTOMATED REPAIR LANES: re-running this `ci-summary` gate does not "
    "clear this state. The verdict is a function of which check-runs exist on this "
    "head SHA, and re-running THIS workflow re-runs no selecting workflow — so the "
    "missing full-tier select cannot appear as a result of the re-run. Only a "
    "`ready_for_review` event (`gh pr ready`) or a new head commit re-runs the "
    "selecting workflows. Do not re-run `ci-summary` for this verdict."
)

# [FABLE-5] PR #3511 review finding 1 (HIGH): STRUCTURAL AWAIT of the trusted
# feature-matrix reporter. The privileged reporter runs in the separate,
# default-branch-owned .github/workflows/feature-matrix-report.yml on a
# workflow_run event, so it posts its `feature-matrix report` summary check-run
# a little AFTER feature-matrix's group jobs finish. A CRASHED or merely DELAYED
# reporter could otherwise be missed: ci-summary could conclude green over the
# group jobs' own successes in the two-poll settle window BEFORE the reporter's
# check-run ever landed, so a reporter that FAILED to post its verdict (e.g. an
# artifact-completeness violation it detected, or a token error) would race past
# the required gate. The fix makes the reporter check STRUCTURALLY REQUIRED
# whenever the feature-matrix produced legs for this head — the robust,
# reporter-timing-INDEPENDENT signal that legs were selected is the presence of
# an `opt-in group (…)` check-run: the group jobs run on THIS (PR/merge_group)
# event under `if: rust_changed == 'true' && legs != '0'` and post their OWN
# conclusions directly on the head SHA (no privileged token, so ci-summary always
# sees them). Their presence therefore PROVES the reporter must post
# `feature-matrix report`; its absence keeps the gate polling (still-settling)
# until the loop's own timeout, then FAILS CLOSED — never a conclude-by-timing.
FM_GROUP_PREFIX = "opt-in group ("
FM_REPORT_NAME = "feature-matrix report"
# [FABLE-5] PR #3511 finding 2 (same-SHA stale-report race): an `opt-in group (…)`
# check-run is an Actions JOB check-run of the feature-matrix run, so its details_url
# is `…/actions/runs/<FM_RUN_ID>/job/<job_id>` — the SAME run id the trusted reporter
# receives as github.event.workflow_run.id and embeds as the report's external_id.
# This extracts <FM_RUN_ID> from that url so the report can be correlated to the
# CURRENT group run (feature-matrix reruns on the same head on ready_for_review /
# label events, so several group runs — hence several reports — can share a head SHA).
RUNS_URL_RE = re.compile(r"/actions/runs/(\d+)(?:/|$)")
ACTIONS_JOB_URL_RE = re.compile(r"/actions/runs/\d+/job/\d+(?:/|$)")


@dataclass
class Config:
    """Loop tunables. Prod values mirror the previous inline bash (INTERVAL /
    MIN_POLLS / SETTLE_POLLS / BASE_POLLS == the old 110-attempt cap) plus the
    sq-90cv4 adaptive-extension knobs. Tests inject tiny values."""

    self_run_id: str = ""
    interval: int = 20          # seconds between polls (base phase)
    min_polls: int = 3          # startup-race floor: no verdict before 3 polls
    settle_polls: int = 2       # all-terminal must hold this many consecutive polls
    base_polls: int = 110       # base budget: 110 x 20s ~= 37 min (the old hard cap)
    sat_interval: int = 40      # slower poll cadence during the saturation extension
    max_total_polls: int = 155  # absolute cap: 45 extension polls x 40s = +30 min
    sat_queue_min: int = 5      # queued workflow-runs in the repo => saturation
    progress_window: int = 15   # polls over which a completed-count rise = progress
    # [OPUS-5] #3781: consecutive polls the UNSATISFIABLE-HOLD state must persist before
    # the gate fails fast on it (see run_gate). Small on purpose — the state is already
    # all-terminal, so the only thing this window buys is tolerance for check-run
    # registration lag; 3 polls x 20s replaces a ~67-minute burn with ~1 minute.
    unsat_confirm_polls: int = 3
    max_consec_fetch_failures: int = 5
    summary_path: str = field(default_factory=lambda: os.environ.get("GITHUB_STEP_SUMMARY", ""))


class FetchError(RuntimeError):
    """A poll's API fetch failed (transient or otherwise)."""


class SupersededLegsError(RuntimeError):
    """A newest cancelled workflow could not be auto-re-dispatched safely."""


@dataclass
class TierContext:
    """[FABLE-5] Draft-tier CI: which tier THIS gate run is evaluating, and how to
    re-read the PR's live draft state at conclusion time. run_tier is computed from
    the trigger payload (pull_request + draft == true => "draft"; every other
    event/state => "full"). fetch_pr_draft() -> bool (current draft state), raising
    FetchError on API failure; None when the run has no PR (push/merge_group)."""

    run_tier: str = "full"  # "draft" | "full"
    event_name: str = ""
    fetch_pr_draft: object = None
    draft_check_retries: int = 3


def _as_int(value, default: int = 0) -> int:
    """GitHub JSON sometimes exposes numeric ids/attempts as strings."""
    try:
        return int(value)
    except (TypeError, ValueError):
        return default


def workflow_identity(run: dict) -> str:
    """Stable identity for newest-run selection.

    workflow_id is the authoritative Actions identity. The path/name fallbacks
    keep hermetic fixtures and a degraded API payload fail-closed instead of
    collapsing unrelated workflows into one empty key.
    """
    workflow_id = run.get("workflow_id")
    if workflow_id not in (None, ""):
        return f"id:{workflow_id}"
    if run.get("path"):
        return f"path:{run['path']}"
    return f"name:{run.get('name') or '<unnamed>'}"


def _workflow_order_key(run: dict) -> tuple:
    """Newest order: creation/run id chooses the run; attempt breaks a same-run tie."""
    return (
        run.get("created_at") or "",
        _as_int(run.get("id")),
        _as_int(run.get("run_attempt"), 1),
    )


def newest_workflow_runs(workflow_runs: list[dict]) -> dict[str, dict]:
    """Return the newest Actions run for every workflow on the already-filtered SHA."""
    newest: dict[str, dict] = {}
    for run in workflow_runs:
        key = workflow_identity(run)
        if key not in newest or _workflow_order_key(run) > _workflow_order_key(newest[key]):
            newest[key] = run
    return newest


def _workflow_run_id_of_check(run: dict) -> int:
    """Actions run id embedded in a job check's details/html URL, else 0."""
    url = run.get("details_url") or run.get("html_url") or ""
    match = RUNS_URL_RE.search(url)
    return _as_int(match.group(1)) if match else 0


def _is_actions_job_check(run: dict) -> bool:
    """True for Actions-created job checks, false for manually posted run links."""
    url = run.get("details_url") or run.get("html_url") or ""
    return bool(ACTIONS_JOB_URL_RE.search(url))


def _workflow_summary_check(run: dict, *, force_pending: bool = False) -> dict:
    """Run-level evidence used when jobs are pending/evaporated or failure is hidden."""
    completed = run.get("status") == "completed" and not force_pending
    return {
        # Never copy the workflow display name here: the synthetic name must not
        # collide with any advisory-registry declaration, so a workflow's
        # authoritative run-level FAILURE can never be excluded. (Under the pre-#3773
        # name rule this also stopped a workflow merely CALLED "…advisory…" from
        # excluding itself; the declared-registry rule makes that structural.)
        "name": f"workflow-run verdict ({workflow_identity(run)})",
        "status": "completed" if completed else (
            "queued" if force_pending else (run.get("status") or "queued")
        ),
        "conclusion": run.get("conclusion") if completed else None,
        "details_url": run.get("html_url") or run.get("url") or "",
        "html_url": run.get("html_url") or "",
        "started_at": run.get("run_started_at") or run.get("created_at") or "",
        "id": _as_int(run.get("id")),
        "external_id": "",
        "_workflow_summary": True,
        "_workflow_name": run.get("name") or run.get("path") or "",
        "_workflow_id": workflow_identity(run),
    }


def _attempt_job_check(job: dict, workflow_run: dict) -> dict:
    """Normalize an attempt-scoped Actions job to the existing check-run vocabulary."""
    return {
        "name": job.get("name") or "<unnamed Actions job>",
        "status": job.get("status") or "queued",
        "conclusion": job.get("conclusion"),
        "details_url": job.get("html_url") or "",
        "html_url": job.get("html_url") or "",
        "started_at": job.get("started_at") or workflow_run.get("run_started_at")
        or workflow_run.get("created_at") or "",
        "id": _as_int(job.get("id")),
        "external_id": "",
        "_workflow_job_inventory": True,
        "_workflow_id": workflow_identity(workflow_run),
        "_workflow_run_id": _as_int(workflow_run.get("id")),
        "_workflow_run_attempt": _as_int(workflow_run.get("run_attempt"), 1),
    }


def resolve_newest_workflow_runs(
    check_runs: list[dict],
    workflow_runs: list[dict],
    self_run_id: str,
    *,
    attempt_jobs: dict[int, list[dict]] | None = None,
    redispatch_pending_ids: set[int] | None = None,
    no_leg_ids: set[int] | None = None,
) -> tuple[list[dict], int]:
    """Resolve Actions checks through newest workflow runs, preserving external checks.

    Returns ``(resolved_checks, superseded_check_count)``. Check-runs whose URL
    belongs to an older run of a known workflow are dropped regardless of their
    conclusion. A completed latest run (and every run_attempt > 1) is represented
    by its attempt-scoped Jobs API payload, so evaporated checks and old-attempt
    checks sharing the same run id cannot poison the verdict. Unknown run ids are
    preserved: manually-posted checks (notably ``feature-matrix report``) may link
    to a workflow_run whose own head SHA differs from the commit on which it posts
    the check.

    ``no_leg_ids`` ([OPUS-5] #3781, computed by no_leg_run_ids over the SAME check
    list): runs that declared themselves EVIDENCE-FREE via the ", no-leg" select
    marker. They are excluded from newest-run candidacy — so a vacuous label-flip
    run can never supersede the previous real run of its workflow — and every
    check-run of theirs is counted as superseded. Defaults to the empty set, which
    makes this function byte-identical to its pre-#3781 behaviour.
    """
    attempt_jobs = attempt_jobs or {}
    redispatch_pending_ids = redispatch_pending_ids or set()
    no_leg_ids = no_leg_ids or set()
    newest = newest_workflow_runs(
        [r for r in workflow_runs if _as_int(r.get("id")) not in no_leg_ids]
    )
    all_by_id = {_as_int(r.get("id")): r for r in workflow_runs if _as_int(r.get("id"))}
    self_id = _as_int(self_run_id)
    self_workflow = ""
    if self_id in all_by_id:
        self_workflow = workflow_identity(all_by_id[self_id])

    resolved: list[dict] = []
    superseded = 0
    for check in check_runs:
        run_id = _workflow_run_id_of_check(check)
        workflow_run = all_by_id.get(run_id)
        if workflow_run is None:
            resolved.append(check)  # external/manually-posted check
            continue
        key = workflow_identity(workflow_run)
        latest = newest.get(key)
        if key == self_workflow:
            continue
        # [OPUS-5] #3781: a run that assembled NO legs (a guarded label-flip no-op) was
        # already removed from `newest` above, so every check-run it produced — its own
        # select plus the whole skipped needs-graph behind it — falls out below as a
        # NON-EVENT: either `run_id != latest_id` (a real predecessor stayed
        # authoritative) or `latest is None` (the vacuous run was that workflow's only
        # one). Deliberately NOT an extra `run_id in no_leg_ids` branch here: it would
        # be unreachable, and a guard that no input can distinguish is a guard no test
        # can pin (it survived every mutant).
        if latest is None:
            superseded += 1
            continue
        latest_id = _as_int(latest.get("id"))
        if run_id != latest_id:
            # #3505: every conclusion from an older workflow run is a NON-EVENT.
            superseded += 1
            continue
        if latest_id in redispatch_pending_ids:
            # The once-only retry has been requested but no new attempt is visible.
            # Every check attached to this cancelled attempt is stale, including
            # manually posted summaries whose URL intentionally lacks `/job/`.
            superseded += 1
            continue
        attempt = _as_int(latest.get("run_attempt"), 1)
        if latest_id in attempt_jobs and _is_actions_job_check(check):
            # Completed runs and re-run attempts use the Jobs API as authority;
            # commit check-runs can be missing or belong to an earlier attempt.
            superseded += 1
            continue
        if attempt > 1:
            # Manually-posted checks (notably feature-matrix report/per-leg checks)
            # link to the run WITHOUT `/job/`; keep only ones posted at/after this
            # attempt's run_started_at so attempt-1 reports cannot satisfy attempt 2.
            if (
                (check.get("started_at") or "")
                < (latest.get("run_started_at") or latest.get("created_at") or "")
            ):
                superseded += 1
                continue
        enriched = dict(check)
        enriched["_workflow_id"] = key
        enriched["_workflow_run_id"] = latest_id
        enriched["_workflow_run_attempt"] = attempt
        resolved.append(enriched)

    selected = [r for key, r in newest.items() if key != self_workflow]
    for workflow_run in selected:
        run_id = _as_int(workflow_run.get("id"))
        force_pending = run_id in redispatch_pending_ids
        if run_id in attempt_jobs and not force_pending:
            resolved.extend(
                _attempt_job_check(job, workflow_run)
                for job in attempt_jobs.get(run_id, [])
            )

        visible = [r for r in resolved if r.get("_workflow_id") == workflow_identity(workflow_run)]
        status = workflow_run.get("status")
        conclusion = workflow_run.get("conclusion")
        # Run-level state closes three check-run holes:
        #   * pending run whose jobs have not registered yet (hold, never early-pass),
        #   * terminal run with no surviving job check (evaporation), and
        #   * non-success run whose failing job check vanished (fail closed).
        visible_required_failure = any(
            r.get("status") == "completed"
            and r.get("conclusion") not in _PASSING
            and not is_advisory(r.get("name", ""))
            for r in visible
        )
        visible_advisory_failure = any(
            r.get("_workflow_job_inventory") is True
            and r.get("status") == "completed"
            and r.get("conclusion") not in _PASSING
            and is_advisory(r.get("name", ""))
            for r in visible
        )
        # A complete Jobs API inventory lets the established advisory-name policy
        # remain authoritative: an advisory job may make its workflow run red, but
        # that advisory-only failure must not acquire a synthetic gating verdict.
        advisory_only_failure = (
            run_id in attempt_jobs
            and visible_advisory_failure
            and not visible_required_failure
        )
        need_summary = (
            force_pending
            or status != "completed"
            or not visible
            or (
                conclusion not in _PASSING
                and not visible_required_failure
                and not advisory_only_failure
            )
        )
        if need_summary:
            resolved.append(_workflow_summary_check(workflow_run, force_pending=force_pending))

    return resolved, superseded


class WorkflowRunResolver:
    """Live/testable newest-run resolver with once-only cancelled-run re-dispatch."""

    def __init__(
        self,
        *,
        self_run_id: str,
        fetch_checks,
        fetch_workflows,
        fetch_attempt_jobs,
        redispatch,
        redispatch_settle_polls: int = 3,
    ):
        self.self_run_id = self_run_id
        self.fetch_checks = fetch_checks
        self.fetch_workflows = fetch_workflows
        self.fetch_attempt_jobs = fetch_attempt_jobs
        self.redispatch = redispatch
        self.redispatch_settle_polls = max(1, redispatch_settle_polls)
        # Durable bound: only run_attempt==1 is eligible. This set is the second
        # bound, preventing repeated POSTs while the list API still shows attempt 1.
        self._redispatch_seen: dict[tuple[str, int, int], int] = {}
        self._terminal_jobs_cache: dict[tuple[int, int], list[dict]] = {}
        # [OPUS-5] #3781: run ids already announced as NO-LEG (log once, not per poll).
        self._no_leg_reported: set[int] = set()

    def __call__(self) -> list[dict]:
        checks = self.fetch_checks()
        workflows = self.fetch_workflows()
        # [OPUS-5] #3781: drop the EVIDENCE-FREE runs (", no-leg" select marker) from
        # newest-run candidacy BEFORE anything downstream keys off `newest` — the
        # redispatch decision, the attempt-jobs inventory and the resolver must all
        # agree on which run is authoritative, or a vacuous label-flip run could still
        # supersede a real predecessor through one of the other two paths.
        no_leg_ids = no_leg_run_ids(checks)
        authoritative = [
            r for r in workflows if _as_int(r.get("id")) not in no_leg_ids
        ]
        fresh_no_leg = sorted(no_leg_ids - self._no_leg_reported)
        if fresh_no_leg:
            # Announced ONCE per run id, not once per poll: the gate can poll 155 times
            # and this line is a standing fact about the head, not a per-poll event.
            self._no_leg_reported |= set(fresh_no_leg)
            print(
                f"  workflow-run resolver: {len(fresh_no_leg)} run(s) declared NO LEGS "
                f"(guarded label-flip no-op, #3781) — ignored, so the previous real run "
                f"of each workflow stays authoritative: "
                f"{', '.join(str(i) for i in fresh_no_leg)}"
            )
        newest = newest_workflow_runs(authoritative)
        self_id = _as_int(self.self_run_id)
        self_workflow = ""
        for run in workflows:
            if _as_int(run.get("id")) == self_id:
                self_workflow = workflow_identity(run)
                break

        redispatch_pending: set[int] = set()
        for key, run in newest.items():
            if key == self_workflow:
                continue
            if run.get("status") != "completed" or run.get("conclusion") != "cancelled":
                continue
            run_id = _as_int(run.get("id"))
            attempt = _as_int(run.get("run_attempt"), 1)
            marker = (key, run_id, attempt)
            if attempt > 1:
                raise SupersededLegsError(
                    f"superseded-legs, re-run required (#3505): newest workflow "
                    f"{run.get('name') or key!r} run {run_id} was cancelled on "
                    f"attempt {attempt}; the gate auto-re-dispatches at most once"
                )
            if marker not in self._redispatch_seen:
                try:
                    self.redispatch(run_id)
                except FetchError as exc:
                    raise SupersededLegsError(
                        f"superseded-legs, re-run required (#3505): newest workflow "
                        f"{run.get('name') or key!r} run {run_id} is cancelled and "
                        f"its once-only auto-redispatch failed: {exc}"
                    ) from exc
                self._redispatch_seen[marker] = 0
                print(
                    f"::notice::ci-summary #3505: newest workflow "
                    f"{run.get('name') or key!r} run {run_id} was cancelled; "
                    "requested its one bounded re-run (run_attempt marker=1)."
                )
            else:
                self._redispatch_seen[marker] += 1
                if self._redispatch_seen[marker] >= self.redispatch_settle_polls:
                    raise SupersededLegsError(
                        f"superseded-legs, re-run required (#3505): workflow "
                        f"{run.get('name') or key!r} run {run_id} stayed cancelled "
                        "after its once-only auto-redispatch request"
                    )
            redispatch_pending.add(run_id)

        attempt_jobs: dict[int, list[dict]] = {}
        for key, run in newest.items():
            run_id = _as_int(run.get("id"))
            if key == self_workflow or run_id in redispatch_pending:
                continue
            attempt = _as_int(run.get("run_attempt"), 1)
            if attempt > 1 or run.get("status") == "completed":
                cache_key = (run_id, attempt)
                if cache_key not in self._terminal_jobs_cache:
                    jobs = self.fetch_attempt_jobs(run_id, attempt)
                    if run.get("status") == "completed":
                        self._terminal_jobs_cache[cache_key] = jobs
                    else:
                        attempt_jobs[run_id] = jobs
                        continue
                attempt_jobs[run_id] = self._terminal_jobs_cache[cache_key]

        resolved, superseded = resolve_newest_workflow_runs(
            checks,
            workflows,
            self.self_run_id,
            attempt_jobs=attempt_jobs,
            redispatch_pending_ids=redispatch_pending,
            no_leg_ids=no_leg_ids,
        )
        if superseded:
            print(
                f"  workflow-run resolver: ignored {superseded} check-run(s) from "
                "superseded runs/attempts (#3505)."
            )
        return resolved


def normalized_name(name: str) -> str:
    """A check-run name with every TIER marker stripped — the identity under which a
    draft-assembled select, a no-leg label-flip select ([OPUS-5] #3781) and their
    full-tier successor are all the SAME leg."""
    return name.replace(DRAFT_TIER_MARKER, "").replace(NO_LEG_MARKER, "")


def _leg_identity(run: dict) -> tuple[str, str]:
    """Workflow-qualified leg identity; legacy/external checks use an empty scope."""
    return (run.get("_workflow_id") or "", normalized_name(run.get("name", "")))


def is_draft_tier(name: str) -> bool:
    """Was this check-run produced by a draft-tier-assembled run (name marker)?"""
    return DRAFT_TIER_MARKER in name


def is_no_leg_select(name: str) -> bool:
    """[OPUS-5] #3781: is this the PURE selection pre-job of a run that assembled NO
    LEGS (a guarded label-flip no-op — see NO_LEG_MARKER)? Deliberately conjoined with
    is_pure_select: only the bare reusable ci-select pre-job may declare its run
    evidence-free. A COMPOUND job that merely contains the selection phrase while
    carrying additional gating evidence (`fv-select (change-based test selection) +
    fv-manifest (proof inventory)`) can never make its run non-authoritative, so a
    marker that drifted onto an evidence-bearing job name fails CLOSED (the run stays
    authoritative and keeps gating)."""
    return NO_LEG_MARKER in name and is_pure_select(name)


def select_tier(name: str) -> str:
    """[OPUS-5] #3781: the TIER a selection check-run was assembled at — "draft",
    "no-leg" or "full". Three-valued so cross-tier supersession stays explicit:
    no-leg evidence must no more stand in for a full-tier selection than draft
    evidence may (forgive_superseded)."""
    if is_draft_tier(name):
        return "draft"
    if NO_LEG_MARKER in name:
        return "no-leg"
    return "full"


def no_leg_run_ids(check_runs: list[dict]) -> set[int]:
    """[OPUS-5] #3781: the Actions run ids that DECLARED THEMSELVES evidence-free by
    naming their pure select check-run with NO_LEG_MARKER. Such a run assembled zero
    legs (the #2546 label-trigger guard skipped every root job), so it must not become
    the authoritative newest run for its workflow and erase the previous real run's
    legs. Run ids come from the check-run's own details/html url — the same
    server-supplied locator the newest-run resolver already trusts."""
    out: set[int] = set()
    for r in check_runs:
        if not is_no_leg_select(r.get("name", "")):
            continue
        rid = _workflow_run_id_of_check(r)
        if rid:
            out.add(rid)
    return out


def is_draft_gate_artifact(name: str) -> bool:
    """Is this check-run a draft-tier gate VERDICT (`gate, draft-tier`)? Such a
    run is an aggregator artifact of a superseded draft-tier evaluation, never a
    leg: its FAILURE must not permanently RED the full-tier gate on the same SHA
    (the live run re-derives the verdict over the real legs), and its SUCCESS
    carries no information the live evaluation does not recompute. The full-tier
    `gate` name is deliberately NOT excluded here — a future sibling job
    literally named `gate` in another workflow must keep gating (the run-id
    self-exclusion comment in ci-summary.yml), and cancelled `gate` predecessors
    are handled by forgive_superseded instead."""
    return name == DRAFT_TIER_GATE_NAME


def _order_key(run: dict) -> tuple:
    """Later-run ordering: started_at (ISO-8601 Zulu strings compare correctly as
    text) tie-broken by the check-run id (monotonically allocated)."""
    return (run.get("started_at") or "", run.get("id") or 0)


def forgive_superseded(runs: list[dict]) -> tuple[list[dict], list[dict]]:
    """[FABLE-5] Draft-tier CI: drop cancelled/stale check-runs that a LATER
    check-run with the same tier-normalized name supersedes (any status, any
    conclusion EXCEPT cancelled/stale — an in-flight successor counts: the gate
    then waits on it). Returns (kept, forgiven).

    WHY: un-drafting fires ready_for_review on the SAME head SHA; the per-PR
    concurrency groups cancel the in-flight draft-tier runs, leaving
    conclusion=cancelled check-runs on the SHA that would otherwise permanently
    RED the fresh full-tier gate. Branch protection itself honours only the
    LATEST run of a check name, so excusing a superseded cancellation aligns the
    gate with the enforcement layer. SAFETY: only cancelled/stale are ever
    forgiven here (a genuine failure/timed_out in this already-newest/external
    check set always gates), and a cancellation with NO successor still fails. Call this
    over the RAW list (self-run included) so THIS gate run's own fresh `gate`
    check-run can supersede its cancelled predecessor.

    [OPUS-4.8] sq-fmx4u.3 hardening (2026-07-17 fleet jam): for the PURE
    change-based SELECTION pre-job (is_pure_select) the successor need only be a
    same-normalized-name, SAME-TIER SUCCESS ANYWHERE on the SHA, not one that is
    STRICTLY LATER. Rationale: the pure select is a deterministic function of the
    SHA's diff (given the tier), so a same-tier same-name success PROVES that
    selection was computed soundly — the per-instance timestamp ordering of its
    concurrency-cancel race-loser siblings is irrelevant. Under the draft-tier +
    review-pipeline label churn (#2537/#2546) a head SHA accretes many cancel
    rounds whose doomed select instances can out-timestamp the winning run's
    already-concluded select success; the strictly-later rule then left a residual
    cancelled select that RED the gate over a provably-sound selection, escalating
    ~20 review:pass PRs to needs:user and starving the merge train. SAFETY (per
    the cross-provider review of PR #3417): the widening is DOUBLY scoped —
    (a) is_pure_select ONLY (name ends with the selection phrase): a COMPOUND job
    that carries additional gating evidence (fv-select + fv-manifest) keeps the
    strictly-later rule, and (b) SAME TIER only: a full-tier success never erases
    a cancelled draft-tier instance (which must stay visible to the
    draft_selects_unsuperseded hold), and a draft-tier success never erases a
    cancelled full-tier instance (draft evidence must never stand in for a
    full-tier selection). Cross-tier supersession remains available ONLY via the
    original strictly-later rule (the intended un-draft flow). Only
    cancelled/stale are ever forgiven; a genuine `failure` select or a cancelled
    select with NO qualifying sibling still fails the gate. Non-select legs keep
    the strictly-later rule (a real leg's earlier stale success must never
    forgive its later cancellation)."""
    kept: list[dict] = []
    forgiven: list[dict] = []
    for r in runs:
        if r.get("conclusion") in _SUPERSEDABLE:
            r_name = r.get("name", "")
            key = _leg_identity(r)
            mine = _order_key(r)
            pure_select = is_pure_select(r_name)
            r_tier = select_tier(r_name)
            if any(
                o is not r
                and _leg_identity(o) == key
                and o.get("conclusion") not in _SUPERSEDABLE
                # [OPUS-5] #3781: a NO-LEG select proves nothing about any tier's
                # selection, so it may only ever supersede another no-leg select.
                # Without this the strictly-later disjunct below would let a vacuous
                # label-flip select forgive a cancelled REAL-tier select and thereby
                # release a draft-tier hold. Pure narrowing (fewer forgivenesses =>
                # more REDs), and unreachable in production because the no-leg run's
                # checks never survive resolve_newest_workflow_runs — defence in depth
                # for any caller that renders a verdict over a raw check list.
                and (select_tier(o.get("name", "")) == "no-leg") == (r_tier == "no-leg")
                and (
                    # pure select: any SAME-TIER same-name success proves a
                    # sound selection; everything else (non-select legs,
                    # compound select+evidence jobs, cross-tier pairs): only a
                    # STRICTLY-LATER re-run supersedes.
                    (
                        pure_select
                        and o.get("conclusion") == "success"
                        and select_tier(o.get("name", "")) == r_tier
                    )
                    or _order_key(o) > mine
                )
                for o in runs
            ):
                forgiven.append(r)
                continue
        kept.append(r)
    return kept, forgiven


def draft_selects_unsuperseded(runs: list[dict]) -> list[str]:
    """[FABLE-5] Draft-tier CI: names of draft-tier-marked selection check-run
    INSTANCES that have no OWN, distinct, strictly-later full-tier (unmarked)
    successor of the same normalized name — one entry per unsuperseded instance
    (duplicates preserved so the caller can report counts). Non-empty on a
    full-tier pull_request gate run == the leg set on this SHA is (still, at
    least partly) draft-tier-assembled: the gate must wait for the
    ready_for_review full re-runs to register, and must never conclude success
    over it.

    PER-INSTANCE, not per-name: ci.yml, bench.yml, feature-matrix.yml and
    fuzz.yml all call the same reusable ci-select job, so a head SHA carries up
    to FOUR draft-marked selects under the IDENTICAL check-run name. Matching by
    name alone would let the FIRST workflow's full-tier select supersede ALL of
    them, releasing the hold while the other workflows' full-tier runs may not
    have registered any check-runs yet — their skipped/vacuous draft-tier legs
    would then satisfy a full-tier verdict (the exact admission the invariant
    forbids; each full-tier run that registers also registers its pending legs,
    so demanding one successor per instance holds the gate until every selecting
    workflow's re-run is visible). Within each normalized-name group the marked
    and unmarked selects are therefore matched greedily in start order (the
    earliest marked instance consumes the earliest unused strictly-later
    unmarked one — a maximum matching for this single-key order structure).

    Deliberately fail-closed: repeated draft-tier rounds on ONE SHA (e.g. a
    ci-full label toggle while the PR was a draft) accumulate marked instances
    that each demand a successor; a hold that cannot be satisfied REDs at budget
    exhaustion ("stale draft-tier run, full run pending") rather than ever
    passing over draft-assembled legs. Re-running the selecting workflows (or
    pushing a new head) clears it.

    [OPUS-5] #3781: a NO-LEG select (a guarded label-flip no-op run — see
    NO_LEG_MARKER) belongs to NEITHER pool. It cannot CREATE a hold, because its run
    assembled no legs at all and a full-tier successor for it can never exist while
    the PR is a draft — that composition is the #3781 deadlock, which burned all 155
    polls on three PRs with zero failing legs. And it cannot DISCHARGE one either: an
    evidence-free selection must never stand in for the full-tier re-run a genuinely
    draft-assembled leg set is waiting on. In production such a check never reaches
    here (its whole run is dropped by resolve_newest_workflow_runs); this is the
    predicate-level belt for any path that renders a verdict over a raw check list."""
    groups: dict[tuple[str, str], tuple[list[dict], list[dict]]] = {}
    for r in runs:
        name = r.get("name", "")
        if not is_select(name) or is_no_leg_select(name):
            continue
        marked, unmarked = groups.setdefault(_leg_identity(r), ([], []))
        (marked if is_draft_tier(name) else unmarked).append(r)
    out: list[str] = []
    for marked, unmarked in groups.values():
        marked.sort(key=_order_key)
        unmarked.sort(key=_order_key)
        fi = 0
        for m in marked:
            while fi < len(unmarked) and _order_key(unmarked[fi]) <= _order_key(m):
                fi += 1
            if fi < len(unmarked):
                fi += 1  # this full-tier successor is consumed by instance m
            else:
                out.append(m.get("name", ""))
    return sorted(out)


def is_platform_managed_advisory(name: str) -> bool:
    """[OPUS-5] Is this a GitHub-MANAGED advisory check-run whose name we cannot
    tag with the advisory token? EXACT whole-name, case-insensitive membership in
    PLATFORM_MANAGED_ADVISORY_NAMES (see that constant for the per-name rationale
    and the security-posture argument). Deliberately NOT a substring/prefix/wildcard
    rule: an unknown or renamed platform check FAILS CLOSED (it keeps gating), so
    widening this exclusion is always an explicit, reviewed edit."""
    return name.strip().lower() in PLATFORM_MANAGED_ADVISORY_NAMES


class AdvisoryRegistryError(RuntimeError):
    """The declared-advisory registry could not be read (missing/unparseable)."""


def registry_key_has_literal_anchor(declared: str) -> bool:
    """[OPUS-5] #3774 review (gpt-5.6-sol, finding 2(b)) — does this registry key pin
    at least one LITERAL, non-whitespace character outside a `${{ … }}` expression?

    Each expression compiles to an unbounded `.+`, so a key that is ONLY an expression
    — the idiomatic `name: ${{ matrix.label }}` — compiles to `.+`, whole-name-matches
    EVERY check-run including `gate` itself, and thereby declares the entire run
    non-gating from a single registry line. C4 cannot catch it either: the key does
    equal the live YAML `name:`, so the binding looks correct. A key with no literal
    frame is therefore REFUSED outright rather than compiled: declare each expansion
    with a literal frame around the expression (as the shipped Tauri key does)."""
    return any(part.strip() for part in _YAML_EXPR_RE.split(declared or ""))


def _compile_declared_name(declared: str) -> re.Pattern:
    """Compile ONE registry key into a whole-name matcher (see _YAML_EXPR_RE).

    Raises AdvisoryRegistryError for an ANCHORLESS key (see
    registry_key_has_literal_anchor) — fail-closed: the gate refuses to install a
    matcher that could neutralise arbitrary check-runs."""
    if not registry_key_has_literal_anchor(declared):
        raise AdvisoryRegistryError(
            f"registry key {declared!r} has no literal anchor outside its "
            "expression(s) — it would match EVERY check-run name (including `gate`)"
        )
    literals = _YAML_EXPR_RE.split(declared.strip())
    return re.compile(
        "".join(re.escape(part) for part in literals[:1])
        + "".join(".+" + re.escape(part) for part in literals[1:]),
        re.IGNORECASE,
    )


def parse_advisory_registry(payload: object) -> tuple[list[str], list[str]]:
    """Split a loaded registry document into (declared_keys, warnings).

    An entry declares its key non-gating ONLY when it is a mapping carrying every
    REGISTRY_REQUIRED_FIELDS value AND its key carries a literal anchor
    (registry_key_has_literal_anchor); anything else is skipped with a warning so the
    check keeps GATING (fail-closed per entry) instead of buying a silent exclusion.
    """
    if not isinstance(payload, dict):
        raise AdvisoryRegistryError("registry root is not a JSON object")
    jobs = payload.get("jobs")
    if not isinstance(jobs, dict):
        raise AdvisoryRegistryError("registry has no `jobs` object")
    declared: list[str] = []
    warnings: list[str] = []
    for key, entry in jobs.items():
        if not isinstance(key, str) or not key.strip():
            warnings.append("registry entry with an empty key ignored (it declares nothing)")
            continue
        if not isinstance(entry, dict):
            warnings.append(f"registry entry {key!r} is not an object — it declares nothing (still GATES)")
            continue
        missing = [f for f in REGISTRY_REQUIRED_FIELDS if not entry.get(f)]
        if missing:
            warnings.append(
                f"registry entry {key!r} is missing {missing} — it declares nothing (still GATES)"
            )
            continue
        if not registry_key_has_literal_anchor(key):
            warnings.append(
                f"registry entry {key!r} has no literal anchor outside its "
                "expression(s) — it would match EVERY check-run name; it declares "
                "nothing (still GATES)"
            )
            continue
        declared.append(key)
    return declared, warnings


# The installed declared-advisory matchers. EMPTY BY DEFAULT: with no registry loaded
# NOTHING is advisory, so an un-wired gate fails closed (it can only over-gate, never
# under-gate). main() loads the real registry and exits 1 loudly if it cannot.
_DECLARED_ADVISORY: tuple[re.Pattern, ...] = ()


def set_declared_advisory(names) -> None:
    """Install the DECLARED-advisory check-name set (main() + hermetic tests)."""
    global _DECLARED_ADVISORY
    _DECLARED_ADVISORY = tuple(_compile_declared_name(n) for n in names)


def declared_advisory_names() -> tuple[str, ...]:
    """The installed declarations, as their compiled source (diagnostics/tests)."""
    return tuple(m.pattern for m in _DECLARED_ADVISORY)


def load_advisory_registry(path: str = ADVISORY_REGISTRY_PATH) -> list[str]:
    """Read + install the declared-advisory set from the registry file.

    Raises AdvisoryRegistryError when the file is missing or unreadable — main()
    turns that into an immediate loud exit 1 rather than evaluating a gate whose
    exclusion set it could not establish.
    """
    try:
        with open(path, encoding="utf-8") as handle:
            payload = json.load(handle)
    except FileNotFoundError as exc:
        raise AdvisoryRegistryError(f"{path} not found in this checkout") from exc
    except (OSError, json.JSONDecodeError) as exc:
        raise AdvisoryRegistryError(f"{path} could not be read: {exc}") from exc
    declared, warnings = parse_advisory_registry(payload)
    for warning in warnings:
        print(f"::warning::ci-summary: {warning}")
    set_declared_advisory(declared)
    return declared


def is_declared_advisory(name: str) -> bool:
    """[OPUS-5] #3773 — is this check-run EXPLICITLY DECLARED non-gating?

    Matched WHOLE-NAME against the installed `.github/advisory-registry.json` keys
    (case-insensitive; `${{ … }}` in a key matches its runtime expansion). No
    substring reach, no word search: an undeclared check GATES no matter what it is
    called, and a declared job that gets RENAMED stops matching — i.e. it GATES,
    fail-closed, while C4 in scripts/check-advisory-registry.py REDs on the drift."""
    candidate = (name or "").strip()
    return any(m.fullmatch(candidate) for m in _DECLARED_ADVISORY)


def is_advisory(name: str) -> bool:
    """[OPUS-5] #3773 — the SINGLE non-gating classifier: DECLARED in the advisory
    registry, or on the exact platform-managed allow-list. NOT a name rule — the
    display-name regex this used to consult was the #3773 correctness hole.

    Every consumer inherits the same answer — the verdict (render_verdict),
    fail-fast (failfast_failures) AND the resolver's run-level synthetic check
    (advisory_only_failure). Splitting them would leave the Dependabot workflow's red
    run to acquire a synthetic gating verdict and red the gate anyway."""
    return is_declared_advisory(name) or is_platform_managed_advisory(name)


def undeclared_token_names(runs: list[dict]) -> list[str]:
    """[OPUS-5] #3773 DIAGNOSTIC: check-runs whose NAME carries an advisory/
    informational token but which are NOT declared — i.e. exactly the checks the old
    name rule neutralised silently and that now GATE. Reported by render_verdict so
    the transition is visible in every gate summary; never a decision input."""
    return sorted(
        {
            r.get("name", "")
            for r in runs
            if ADVISORY_NAME_TOKEN_RE.search((r.get("name", "") or "").lower())
            and not is_advisory(r.get("name", ""))
        }
    )


def is_select(name: str) -> bool:
    """[FABLE-5] sq-fmx4u.3: is this check-run the change-based test-selection
    pre-job? Matched by the stable phrase in its job name (case-insensitive).
    If the name ever drifts, detection degrades to select_runs == [] — i.e. the
    PRE-selection semantics (skipped is unconditionally satisfied), never a
    false RED; the wiring inspection test pins the name so it does not drift."""
    return bool(SELECT_RE.search(name.lower()))


def is_pure_select(name: str) -> bool:
    """[OPUS-4.8] sq-fmx4u.3: eligibility for forgive_superseded's same-name
    same-tier any-success rule — STRICTER than is_select. Only the PURE reusable
    change-based-selection pre-job qualifies, matched by the tier-normalized name
    ENDING with the selection phrase; a COMPOUND job whose name merely CONTAINS
    the phrase but carries additional gating evidence (e.g. `fv-select
    (change-based test selection) + fv-manifest (proof inventory)`) is excluded
    and keeps the strictly-later supersession rule. is_select stays broad for
    selection-HEALTH detection (a cancelled compound select still REDs
    select_ok); this predicate only gates the forgiveness widening."""
    return normalized_name(name).lower().rstrip().endswith(
        "(change-based test selection)"
    )


def is_self(run: dict, self_run_id: str) -> bool:
    """True iff this check-run belongs to THIS gate's workflow run. Anchored to
    /runs/<id>(/|$) so a longer sibling run id sharing the prefix can't match."""
    if not self_run_id:
        return False
    url = run.get("details_url") or run.get("html_url") or ""
    return bool(re.search(rf"/runs/{re.escape(self_run_id)}(/|$)", url))


def is_fm_group(name: str) -> bool:
    """[FABLE-5] PR #3511 finding 1: is this an `opt-in group (…)` check-run? These
    are the feature-matrix GROUP jobs — they run on the PR/merge_group event only
    when the setup job assembled >=1 leg (rust_changed && legs != '0') and post
    their own conclusions directly on the head SHA (no privileged token), so their
    PRESENCE is the robust, reporter-timing-independent proof that legs were
    selected and the trusted `feature-matrix report` reporter must post its
    verdict for this head.

    ZERO-LEG SKELETON (production incident, PR #3524 2026-07-19): when the setup
    job selects ZERO legs the skipped matrix job still posts ONE skeleton
    check-run named with the UNEXPANDED placeholder — literally
    `opt-in group (${{ matrix.group }})`, conclusion=skipped. That is proof legs
    did NOT run; counting it as group presence made every docs/config-only PR
    await a reporter verdict the reporter correctly never posts (zero-leg no-op)
    and time out RED after the full gate budget.

    SECURITY (sol on #3525): the exclusion must NOT be name-only — matrix.group
    names come from the PR-controlled assembler, so a real successful group named
    to contain `${{` could masquerade as the skeleton and drop the reporter
    requirement (fail-open). The skeleton is identified by BOTH marks: the
    unexpanded placeholder in the name AND conclusion == skipped (server-set).
    is_real_fm_group(run) implements that; this name-only helper is for callers
    that have no conclusion in hand (run-id extraction, where a forged name only
    ADDS a candidate id — never removes the requirement)."""
    return name.startswith(FM_GROUP_PREFIX)


def is_real_fm_group(run: dict) -> bool:
    """A group check that PROVES legs ran: group-prefixed name, excluding only the
    zero-leg skeleton (unexpanded `${{` placeholder AND server-set skipped)."""
    name = run.get("name", "")
    if not name.startswith(FM_GROUP_PREFIX):
        return False
    if "${{" in name and (run.get("conclusion") or "") == "skipped":
        return False
    return True


def is_fm_report(name: str) -> bool:
    """[FABLE-5] PR #3511 finding 1: the trusted reporter's summary check-run
    (`feature-matrix report`, posted from the default-branch-owned
    feature-matrix-report.yml). Its terminal-success is the verdict ci-summary
    must structurally await whenever `opt-in group (…)` legs ran."""
    return name == FM_REPORT_NAME


def fm_run_id_of(run: dict) -> str:
    """The feature-matrix Actions run id a group check-run belongs to, parsed from
    its `/actions/runs/<id>` url (details_url, else html_url). "" when the url has no
    parseable run id (a group check that carries no locating url). Used to bucket
    group checks by the run they belong to so presence and correlation are decided
    relative to the LATEST run — including the zero-leg skeleton, which is itself a
    check-run of that run and so carries its run id."""
    url = run.get("details_url") or run.get("html_url") or ""
    m = RUNS_URL_RE.search(url)
    return m.group(1) if m else ""


def fm_group_run_id(runs: list[dict]) -> str:
    """[FABLE-5] PR #3511 finding 2: the CURRENT feature-matrix run id, extracted as
    the MAXIMUM `/actions/runs/<id>` seen across the `opt-in group (…)` check-runs'
    details_url — name-only (is_fm_group), so the zero-leg skeleton (a check-run of
    the current run) is INCLUDED. GitHub Actions run ids are monotonically ascending,
    so the max is the LATEST feature-matrix run on this head — the one whose reporter
    verdict the gate must await (or, when that run is zero-leg, the one that proves no
    reporter is expected; see fm_report_status). On a same-SHA rerun (ready_for_review
    / label) the commit carries group check-runs from BOTH the stale and the fresh run;
    the fresh run's id is the larger. Returns "" if no group check carries a parseable
    run id (then correlation degrades to the pre-finding-2 any-report behaviour — never
    a false RED, and the stale-report race is only reachable on a same-SHA rerun)."""
    best = ""
    for r in runs:
        if not is_fm_group(r.get("name", "")):
            continue
        rid = fm_run_id_of(r)
        if rid:
            # Numeric max (ids are ints of possibly-different width); compare as ints.
            if best == "" or int(rid) > int(best):
                best = rid
    return best


def fm_report_status(runs: list[dict]) -> str:
    """[FABLE-5] PR #3511 finding 1 (HIGH) + finding 2 (HIGH, correlation): the
    reporter-await status over the (already forgiveness-filtered, self-excluded)
    sibling set. Returns one of:

      * "n/a"     — the LATEST feature-matrix run on this head produced NO legs, so
                    NO reporter is expected. Two shapes: (1) no `opt-in group (…)`
                    check-run at all — a doc-only PR, a fully change-selected-out
                    matrix, or a merge_group that skipped the lane; (2) the latest
                    run posted ONLY the zero-leg skeleton (unexpanded placeholder,
                    server-set skipped) — no real leg ran, so the reporter correctly
                    posts nothing. Presence is LATEST-RUN-RELATIVE (finding, r3): an
                    OLDER real group run's leftover check on a same-SHA rerun does
                    NOT resurrect the requirement when the current run is zero-leg.
      * "ok"      — legs ran AND a terminal-SUCCESS `feature-matrix report`
                    check-run FOR THE CURRENT GROUP RUN is present: the reporter
                    posted its green verdict.
      * "failed"  — legs ran AND the CURRENT run's reporter check-run is terminal
                    but NOT success (crashed / completeness-violation / POST error).
                    The caller REDs. (Such a check also fails the normal gating-set
                    render on its own — this is belt-and-braces so the reporter
                    requirement is explicit and cannot be silently dropped.)
      * "pending" — legs ran but the CURRENT run's `feature-matrix report` check-run
                    is either ABSENT (its workflow_run has not landed it yet, or
                    crashed before posting) or present-but-not-terminal. The gate
                    must keep polling (still-settling); budget exhaustion in this
                    state FAILS CLOSED via render_verdict's reporter belt — never
                    a conclude-by-timing over the group jobs' bare successes.

    CORRELATION (finding 2 — same-SHA stale-report race): feature-matrix reruns on the
    SAME head SHA (ready_for_review / label events), so a STALE report from an earlier
    run can sit on the commit while the CURRENT run's reporter is delayed/crashed. Each
    reporter embeds its TRIGGERING feature-matrix run id as the report's external_id
    (server-supplied, unforgeable); the CURRENT group run's id is fm_group_run_id(runs)
    (max `/actions/runs/<id>` across the group checks). Only a report whose external_id
    equals that id counts — a stale report (older run id) is IGNORED, so it can never
    satisfy the hold for a fresh group run. When the current group run id is
    unresolvable (no parseable url — not expected in prod) OR no report carries an
    external_id (legacy reporter), we fall back to matching ANY report: a graceful
    degradation to the pre-finding-2 behaviour that is never a false RED and only
    weakens the stale-race defence, which is unreachable absent a same-SHA rerun.

    SAFETY: a reporter that FAILED to post (delayed / crashed) is indistinguishable
    from one that has not posted YET, and both map to "pending" — the fail-CLOSED
    direction. A same-SHA fork PR whose reporter is fork-denied (cannot POST with a
    read-only token) will hold here to timeout and RED; that is acceptable — a fork
    PR is not auto-merged into the queue and fail-closed is the safe posture the
    finding demands."""
    # PRESENCE is decided relative to the LATEST feature-matrix run (finding, r3).
    # A same-SHA rerun (ready_for_review / label) leaves an OLDER real group run's
    # check-runs on the commit alongside a NEWER zero-leg skeleton run; keying
    # presence off ANY real group (the old run's) while keying the run id off the
    # newer skeleton deadlocked the gate — it awaited a reporter the zero-leg run
    # correctly never posts. So: bucket the group checks by the run they belong to
    # and judge the presence + reporter requirement of the LATEST run only.
    group_checks = [r for r in runs if is_fm_group(r.get("name", ""))]
    if not group_checks:
        return "n/a"  # no feature-matrix legs on this head at all
    real_groups = [r for r in group_checks if is_real_fm_group(r)]
    if not real_groups:
        return "n/a"  # only zero-leg skeleton(s) anywhere — no real leg ever ran
    current_run_id = fm_group_run_id(runs)  # max run id across ALL group checks
    # Can we bucket by run id? Only if a latest run id resolved AND every REAL group
    # carries a parseable run id — otherwise a real leg of an UNKNOWN (possibly newer)
    # run may exist, so we cannot safely declare the latest run zero-leg. In that
    # fail-CLOSED case we keep the reporter requirement (real legs ran somewhere) and
    # let current_run_id (possibly "") drive the graceful any-report degradation below.
    real_unparseable = any(not fm_run_id_of(r) for r in real_groups)
    if current_run_id and not real_unparseable:
        latest_run_checks = [r for r in group_checks
                             if fm_run_id_of(r) == current_run_id]
        if not any(is_real_fm_group(r) for r in latest_run_checks):
            # The LATEST run posted only the zero-leg skeleton — no leg ran, no
            # reporter is expected. (A stale OLDER real run's leftover check does not
            # matter; its own reporter, if any, is not what this head awaits.)
            return "n/a"
        # else: the latest run DID run real legs (a forged placeholder name with a
        # server-set NON-skipped conclusion still counts as real — security, r2) —
        # require its reporter, correlated to current_run_id.
    reports = [r for r in runs if is_fm_report(r.get("name", ""))]
    if not reports:
        return "pending"  # legs ran; reporter verdict not on the head SHA yet
    # CORRELATION: bind the verdict to the CURRENT group run. Prefer the report(s)
    # whose external_id equals the current group run id; ignore stale reports.
    any_report_has_extid = any((r.get("external_id") or "") for r in reports)
    if current_run_id and any_report_has_extid:
        matched = [r for r in reports if (r.get("external_id") or "") == current_run_id]
        if not matched:
            # Every report on this SHA is for an OLDER feature-matrix run (or carries
            # no external_id). The current run's verdict has not landed — keep waiting
            # (fail-closed on budget exhaustion). A stale success can never green us.
            return "pending"
        reports = matched
    # If ANY (current-run) report check is non-terminal, keep waiting; else judge them.
    if any(r.get("status") != "completed" for r in reports):
        return "pending"
    if all(r.get("conclusion") == "success" for r in reports):
        return "ok"
    return "failed"


def _emit(line: str, summary_path: str = "") -> None:
    print(line, flush=True)
    if summary_path:
        with open(summary_path, "a", encoding="utf-8") as fh:
            fh.write(line + "\n")


def _bounded_draft_read(
    tier_ctx: TierContext | None,
) -> tuple[bool | None, Exception | None]:
    """The PR's LIVE draft state via tier_ctx.fetch_pr_draft, bounded-retried on
    transient FetchError. Returns (state, last_error); state is None when there is no
    fetcher wired or every attempt failed. Shared by the conclusion-time re-check
    (_draft_recheck) and the #3781 unsatisfiable-hold detector so both read the state
    exactly the same way — the CALLERS decide what an unreadable state means."""
    still_draft: bool | None = None
    last_err: Exception | None = None
    if tier_ctx is not None and tier_ctx.fetch_pr_draft is not None:
        for _ in range(max(1, tier_ctx.draft_check_retries)):
            try:
                still_draft = tier_ctx.fetch_pr_draft()
                break
            except FetchError as exc:  # transient API blip: bounded retry
                last_err = exc
    return still_draft, last_err


def _draft_recheck(tier_ctx: TierContext | None, summary_path: str = "") -> int:
    """[FABLE-5] Draft-tier conclusion-time re-check, applied on EVERY would-be-
    SUCCESS path (including the stable-empty set): a DRAFT-tier run confirms the
    PR is STILL a draft immediately before emitting success. Un-drafted => the
    ready_for_review full-tier run supersedes this one, so conclude FAILURE
    ("stale draft-tier run, full run pending") — the merge queue must never latch
    a draft-tier green. An unreadable draft state (API failure after bounded
    retries, or no fetcher wired) fail-closes to FAILURE: a draft PR cannot merge
    regardless, so a false RED here is cheap and a false PASS is the invariant
    violation. Returns 0 (ok to pass) or 1 (fail). Full-tier runs: always 0."""
    if not tier_ctx or tier_ctx.run_tier != "draft":
        return 0
    still_draft, last_err = _bounded_draft_read(tier_ctx)
    if still_draft is None:
        _emit(
            "### ci-summary: FAILED — draft-tier run could not confirm the PR's "
            f"current draft state at conclusion time (last error: {last_err}). "
            "Fail-closed: a draft-tier result must never admit a PR to the merge "
            "queue, so an unverifiable draft state REDs (re-run once the API "
            "recovers; a draft PR cannot merge regardless).",
            summary_path,
        )
        print("::error::ci-summary failed — draft state unverifiable on a draft-tier run.")
        return 1
    if still_draft is False:
        _emit(
            "### ci-summary: FAILED — stale draft-tier run, full run pending. This "
            "gate run evaluated the REDUCED draft-tier leg set, but the PR is no "
            "longer a draft: the ready_for_review full-tier run on this head SHA "
            "supersedes this check (docs/branch-protection.md §Draft-tier CI).",
            summary_path,
        )
        print("::error::ci-summary failed — stale draft-tier run on a now-ready PR.")
        return 1
    return 0


def render_verdict(runs: list[dict], summary_path: str = "", tier_ctx: TierContext | None = None) -> int:
    """Shared by the clean-converge, graceful-timeout, and post-extension paths, so
    every path applies IDENTICAL gating semantics. Returns the process exit code.

    DRAFT-TIER INTEGRITY ([FABLE-5], see the header): with a TierContext,
      * a FULL-tier pull_request verdict REDs while any draft-tier-marked select
        INSTANCE lacks its own later full-tier successor (stale draft-tier leg
        set — at least one selecting workflow's ready_for_review full run has
        not registered on this SHA);
      * a DRAFT-tier verdict that would otherwise be SUCCESS first re-reads the
        PR's CURRENT draft state from the API: no-longer-draft => FAILURE ("stale
        draft-tier run, full run pending"), and an unreadable state fail-closes
        to FAILURE after bounded retries. A draft-tier verdict that is already a
        FAILURE skips the re-check (a RED can never be latched by the queue).
    Without a TierContext (tests / push / merge_group) the semantics are exactly
    the pre-draft-tier ones.

    SELECTION SEMANTICS ([FABLE-5] sq-fmx4u.3, design §5.3): a `skipped`
    conclusion is satisfied ONLY when the change-based selection pre-job
    (is_select) succeeded — a skip is trustworthy iff the thing that decided to
    skip ran to a successful conclusion. Concretely:
      * every select check-run present must have conclusion == "success";
        anything else (failure, cancelled, skipped, neutral, stale) REDs the
        gate outright, even if every other sibling is green — an unobservable
        selection means the skips on this commit are unattributable (§4.3);
      * with select green (or absent — e.g. a pre-selection sibling set, where
        no skip was produced by selection), `skipped` stays non-failing exactly
        as before. Absent-select degradation is deliberately the PRE-sq-fmx4u.3
        behaviour, never a new failure mode.
    A job that FAILED still fails the gate regardless of selection — selection
    can only ever decide whether a SKIP is satisfied, never mask a failure."""
    # [FABLE-5] Draft-tier belt: a full-tier pull_request gate must never conclude
    # over a leg set whose selection was assembled draft-tier (checked FIRST — it
    # invalidates the whole set, including an otherwise-green one).
    if tier_ctx and tier_ctx.run_tier == "full" and tier_ctx.event_name == "pull_request":
        stale = draft_selects_unsuperseded(runs)
        if stale:
            counts: dict[str, int] = {}
            for n in stale:
                counts[n] = counts.get(n, 0) + 1
            detail = ", ".join(
                f"{n} ×{c}" if c > 1 else n for n, c in sorted(counts.items())
            )
            _emit(
                "### ci-summary: FAILED — stale draft-tier run, full run pending. The "
                "selection on this head SHA is (at least partly) draft-tier-assembled: "
                f"{len(stale)} draft-marked select instance(s) have no OWN later "
                f"full-tier successor ({detail}). Each selecting workflow's "
                "ready_for_review full-tier re-run must register its own successor "
                "(ci/bench/feature-matrix/fuzz share one select name — one full-tier "
                "select must never release the hold for the others). A draft-tier leg "
                "set must never admit a non-draft PR to the merge queue "
                "(docs/branch-protection.md §Draft-tier CI). " + UNSAT_HOLD_REMEDY,
                summary_path,
            )
            print("::error::ci-summary failed — stale draft-tier leg set on a non-draft head.")
            return 1
    # [FABLE-5] PR #3511 finding 1 (HIGH): STRUCTURAL AWAIT of the trusted
    # feature-matrix reporter. If `opt-in group (…)` legs ran for this head, the
    # `feature-matrix report` summary check MUST be present and terminal-SUCCESS
    # before the gate can conclude green. This is reached only on a would-CONCLUDE
    # render (clean settle or budget-exhaustion timeout), so a "pending" here at
    # RENDER time is FAIL-CLOSED, never a conclude-by-timing: the poll loop holds
    # the settle window open while the report is missing/pending (report_pending),
    # and a render still finding it unresolved means the reporter never landed
    # within the loop's own timeout. A "failed" reporter (crashed / completeness
    # violation) REDs here too (belt-and-braces: its own check-run also fails the
    # gating-set render below). Checked BEFORE the empty-set / normal-render paths
    # so a group set that is otherwise all-green cannot pass over an absent verdict.
    fm = fm_report_status(runs)
    if fm != "n/a" and fm != "ok":
        if fm == "failed":
            _emit(
                "### ci-summary: FAILED — the trusted `feature-matrix report` reporter "
                "concluded a NON-SUCCESS verdict (crashed / artifact-completeness "
                "violation / check-run POST error). The feature-matrix legs ran (an "
                "`opt-in group (…)` check-run is present on this head), so the reporter's "
                "verdict is required and it failed (fail-closed, PR #3511 finding 1).",
                summary_path,
            )
            print("::error::ci-summary failed — the feature-matrix reporter concluded non-success.")
        else:
            _emit(
                "### ci-summary: FAILED — the trusted `feature-matrix report` reporter "
                "verdict never landed on this head SHA within the gate's budget. The "
                "feature-matrix legs ran (an `opt-in group (…)` check-run is present), so "
                "its `feature-matrix report` summary check-run is STRUCTURALLY REQUIRED — "
                "a delayed or crashed reporter must never race past the gate. Fail-closed "
                "(PR #3511 finding 1): re-run the feature-matrix-report workflow (or push "
                "a new head) so the reporter posts its verdict.",
                summary_path,
            )
            print("::error::ci-summary failed — the feature-matrix reporter verdict is missing (fail-closed).")
        return 1
    total = len(runs)
    if total == 0:
        if _draft_recheck(tier_ctx, summary_path) != 0:
            return 1
        _emit("ci-summary: no sibling checks to aggregate (stable empty set) — passing.", summary_path)
        return 0
    gating = [r for r in runs if not is_advisory(r.get("name", ""))]
    excluded = total - len(gating)
    # [OPUS-5] #3773: make the formerly-silent exclusion loud in BOTH directions —
    # every check that carries an advisory name token but is NOT declared is listed
    # here and IS in `gating` above. (Diagnostic only; see undeclared_token_names.)
    for undeclared in undeclared_token_names(runs):
        _emit(
            f"note: `{undeclared}` carries an advisory/informational NAME token but has no "
            f"declaration in {ADVISORY_REGISTRY_PATH} — it GATES (#3773). Declare it there "
            f"(with an owner_bead + promotion_criteria) or drop the misleading token.",
            summary_path,
        )
    # Selection pre-job health — searched over ALL runs (not just gating) so a
    # hypothetical advisory-renamed select could still never green-light a skip.
    # NB superseded-cancelled select INSTANCES are already dropped upstream by
    # forgive_superseded (including the deterministic-select same-name SAME-TIER
    # success race-loser rule for the PURE select pre-job, sq-fmx4u.3
    # hardening), so any cancelled select that SURVIVES to here has NO
    # qualifying sibling (no strictly-later successor, and — for a pure select —
    # no same-tier same-name success either) and rightly REDs.
    select_runs = [r for r in runs if is_select(r.get("name", ""))]
    select_ok = all(r.get("conclusion") == "success" for r in select_runs)
    skipped_ct = sum(1 for r in gating if r.get("conclusion") == "skipped")
    if not select_ok:
        _emit(
            f"### ci-summary: FAILED — the change-based test-selection pre-job did not "
            f"succeed, so the {skipped_ct} skipped gating check(s) on this commit cannot "
            f"be attributed to a sound selection (fail-closed, sq-fmx4u.3 / design §4.3).",
            summary_path,
        )
        for r in select_runs:
            _emit(f"- ✗ {r.get('name')}: {r.get('conclusion') or 'incomplete'}", summary_path)
        print("::error::ci-summary failed — the selection pre-job must conclude success.")
        return 1

    def _satisfied(r: dict) -> bool:
        c = r.get("conclusion")
        if c == "skipped":
            return select_ok  # always True past the gate above; kept explicit so a
            # future refactor that moves this check cannot silently trust a skip.
        return c in _PASSING

    failed = [r for r in gating if not _satisfied(r)]
    if failed:
        _emit(
            f"### ci-summary: FAILED — {len(failed)} non-passing gating check(s) of "
            f"{len(gating)} gating ({excluded} advisory check(s) excluded — each "
            f"DECLARED in {ADVISORY_REGISTRY_PATH} or on the platform-managed "
            f"allow-list)",
            summary_path,
        )
        for r in failed:
            _emit(f"- ✗ {r.get('name')}: {r.get('conclusion') or 'incomplete'}", summary_path)
        print("::error::ci-summary failed — see the non-passing gating checks above.")
        return 1
    if _draft_recheck(tier_ctx, summary_path) != 0:
        return 1
    _emit(
        # [OPUS-5] #3774 review: the excluded set is DECLARED-in-the-registry OR on the
        # exact platform-managed allow-list (PLATFORM_MANAGED_ADVISORY_NAMES) — saying
        # "each DECLARED in the registry" understated the second, smaller source.
        f"### ci-summary: PASSED — all {len(gating)} gating check(s) green (or skipped/neutral); "
        f"{excluded} advisory check(s) excluded (each DECLARED in "
        f"{ADVISORY_REGISTRY_PATH}, or on the exact platform-managed allow-list); "
        f"set stable."
        + (
            " DRAFT-TIER verdict (reduced leg set; PR draft state re-confirmed). This "
            f"check-run is `{DRAFT_TIER_GATE_NAME}`, never the required `{GATE_CHECK_NAME}` "
            "context — it cannot satisfy branch protection; the full matrix re-runs at "
            "ready_for_review and only its full-tier gate can."
            if tier_ctx and tier_ctx.run_tier == "draft"
            else ""
        ),
        summary_path,
    )
    if select_runs:
        _emit(
            f"selection: {len(gating) - skipped_ct} of {len(gating)} gating check(s) ran, "
            f"{skipped_ct} skipped (selection and/or path-filter; selection pre-job succeeded).",
            summary_path,
        )
    return 0


def failfast_failures(runs: list[dict]) -> list[dict]:
    """[FABLE-5] Fail-fast (2026-07-17 directive, header §FAIL-FAST): the GATING
    legs whose CONCLUDED failure already decides the verdict. `runs` must be the
    poll loop's post-forgive_superseded, post-self-exclusion, post-draft-gate-
    artifact sibling set — i.e. EXACTLY the set render_verdict would judge — so
    this helper inherits the final render's supersession/forgiveness
    classification instead of reimplementing it (a forgiven race-loser is
    already gone; a surviving cancelled/stale run is not `failure` and never
    fires this). A leg qualifies iff it is completed with conclusion=failure,
    is not advisory (the same is_advisory predicate the verdict excludes by),
    and no same-tier-normalized-name run on the SHA is still in flight (a rerun
    already underway must be allowed to finish — the gate keeps waiting on it,
    exactly as the settle loop always has). `timed_out`/`cancelled`/`stale`
    conclusions deliberately do NOT qualify: only an unambiguous `failure`
    fails fast; everything else waits for the full render."""
    inflight = {_leg_identity(r) for r in runs if r.get("status") != "completed"}
    out: list[dict] = []
    for r in runs:
        if r.get("status") != "completed" or r.get("conclusion") != "failure":
            continue
        name = r.get("name", "")
        if is_advisory(name):
            continue
        if _leg_identity(r) in inflight:
            continue
        out.append(r)
    return out


# [OPUS-5] #3783 — the LIVENESS VETO's single source of truth.
LIVE_STATUS = "in_progress"


def live_siblings(runs: list[dict]) -> list[dict]:
    """[OPUS-5] #3783: the awaited siblings that are DEMONSTRABLY EXECUTING now.

    GitHub gives a check-run exactly ONE non-terminal status once a runner has
    picked the job up: `in_progress`. Every other non-terminal status a check-run
    or an Actions job can carry — `queued`, `waiting`, `requested`, `pending` —
    means the leg has NOT started, and *that* is the state an idle Actions queue
    is evidence about.

    So `in_progress` is POSITIVE LIVENESS EVIDENCE and vetoes the genuine-hang
    verdict (header §LIVENESS VETO): the two hang signals (idle queue + no recent
    completions) are both NORMAL for a healthy long bounded proof — the queue is
    idle precisely BECAUSE the job dequeued and started, and a `kani` harness
    emits no completion for tens of minutes by design.

    The #3677 case the detector was built for is untouched: an EVAPORATED
    check-run is represented by `_workflow_summary_check(..., force_pending=True)`,
    whose status is `queued` — never `in_progress` — so a lost leg still REDs at
    the base budget exactly as before.
    """
    return [r for r in runs if r.get("status") == LIVE_STATUS]


def _name_list(runs: list[dict], limit: int = 6) -> str:
    """Comma-joined check-run names for a diagnostic line (bounded, deterministic)."""
    names = sorted({(r.get("name") or "<unnamed>") for r in runs})
    shown = ", ".join(f"`{n}`" for n in names[:limit])
    return shown + (f", +{len(names) - limit} more" if len(names) > limit else "")


def run_gate(cfg: Config, fetch_runs, fetch_queue_depth, sleep_fn=time.sleep,
             tier_ctx: TierContext | None = None) -> int:
    """The poll loop. `fetch_runs()` -> list of {name,status,conclusion,details_url,
    started_at,id} dicts (raises FetchError on API failure); `fetch_queue_depth()`
    -> int queued workflow-run count for the repo, or None when unknown (API
    failure — the gate then falls back to the progress signal alone). tier_ctx
    carries the draft-tier integrity context (None => pre-draft-tier semantics).
    Returns the exit code."""
    prev_names: list[str] | None = None
    stable = 0
    runs: list[dict] = []
    pending = 0
    completed_hist: list[int] = []
    consec_fetch_failures = 0
    extension_started = False
    # Fail-fast grace state: the (name, id) key set of concluded gating failures
    # observed on the PREVIOUS poll — the red fires only when a fresh re-poll
    # re-observes the identical set (header §FAIL-FAST).
    ff_suspect: tuple | None = None
    # [OPUS-5] #3781: consecutive polls the UNSATISFIABLE-HOLD state has persisted.
    unsat_polls = 0

    attempt = 0
    while attempt < cfg.max_total_polls:
        attempt += 1
        try:
            raw = fetch_runs()
        except SupersededLegsError as exc:
            _emit(
                f"### ci-summary: FAILED — {exc}. The gate did not treat the "
                "cancelled newest run as a test failure and did not dispatch it "
                "more than once; a fresh workflow run or new head is required.",
                cfg.summary_path,
            )
            print(f"::error::ci-summary failed — {exc}")
            return 1
        except FetchError as exc:
            consec_fetch_failures += 1
            if consec_fetch_failures >= cfg.max_consec_fetch_failures:
                print(
                    f"::error::ci-summary: {consec_fetch_failures} consecutive check-run "
                    f"fetch failures — cannot observe the sibling set. Last error: {exc}"
                )
                return 1
            print(
                f"attempt {attempt}: check-run fetch failed ({exc}) — skipping this poll "
                f"({consec_fetch_failures}/{cfg.max_consec_fetch_failures} consecutive)."
            )
            sleep_fn(cfg.sat_interval if extension_started else cfg.interval)
            continue
        consec_fetch_failures = 0

        # [FABLE-5] Draft-tier CI: forgive cancelled/stale check-runs a later
        # same-normalized-name run supersedes — over the RAW list (self included)
        # so this run's own fresh `gate` check-run supersedes a cancelled
        # predecessor left on the SHA by the ready_for_review concurrency cancel.
        # Then drop (a) this run's own check-run and (b) any `gate, draft-tier`
        # check-run — a draft-tier gate VERDICT is a tier artifact of a
        # superseded evaluation, never a leg (a completed draft-tier gate
        # FAILURE on the SHA must not permanently RED the full-tier gate, and
        # its SUCCESS never was the required context).
        kept, forgiven = forgive_superseded(raw)
        runs = [
            r for r in kept
            if not is_self(r, cfg.self_run_id)
            and not is_draft_gate_artifact(r.get("name", ""))
        ]
        total = len(runs)
        pending = sum(1 for r in runs if r.get("status") != "completed")
        # [FABLE-5] Draft-tier CI: on a FULL-tier pull_request run, a draft-tier-
        # assembled selection with no full-tier successor means the ready_for_review
        # re-run has not registered yet — treat the set as STILL-SETTLING (hold the
        # settle window open) instead of concluding over draft-tier legs. Budget
        # exhaustion in this state REDs via render_verdict's stale-draft-tier belt.
        awaiting_full = bool(
            tier_ctx
            and tier_ctx.run_tier == "full"
            and tier_ctx.event_name == "pull_request"
            and draft_selects_unsuperseded(runs)
        )
        # [FABLE-5] PR #3511 finding 1 (HIGH): STRUCTURAL AWAIT of the trusted
        # feature-matrix reporter. When `opt-in group (…)` legs ran for this head
        # but the `feature-matrix report` verdict has not yet landed as a
        # terminal-SUCCESS check-run (absent — reporter's workflow_run not in yet
        # or crashed pre-post — or present-but-not-terminal), treat the set as
        # STILL-SETTLING and hold the settle window open, exactly like a
        # draft-tier awaiting_full. A "failed" reporter does NOT hold (it is
        # terminal and must conclude RED). Budget exhaustion while still awaiting
        # FAILS CLOSED via render_verdict's reporter belt — never conclude-by-timing.
        awaiting_report = fm_report_status(runs) == "pending"
        completed_hist.append(total - pending)
        # Settle is a POST-TERMINAL window re-armed ONLY by pending work (sq-ipkku):
        # already-terminal injections must not starve convergence.
        stable = 0 if (pending or awaiting_full or awaiting_report) else stable + 1
        names = sorted({r.get("name", "") for r in runs})
        changed = " (name set changed)" if prev_names is not None and names != prev_names else ""
        prev_names = names
        extra = f", {len(forgiven)} superseded-cancelled forgiven" if forgiven else ""
        extra += ", awaiting the full-tier re-run (draft-tier selection present)" if awaiting_full else ""
        extra += ", awaiting the feature-matrix reporter verdict" if awaiting_report else ""
        print(
            f"attempt {attempt}: {total} check-run(s), {pending} running, "
            f"all-terminal stable for {stable}/{cfg.settle_polls} poll(s){changed}{extra}",
            flush=True,
        )

        # [FABLE-5] FAIL-FAST (header §FAIL-FAST): a concluded gating failure in
        # the (already forgiveness-filtered) sibling set decides the verdict now
        # — a genuine `failure` in the authoritative newest run is never forgiven,
        # so every later render REDs
        # anyway; waiting out the remaining legs only delays the red and the
        # fast-fix trigger behind it. Applies only while siblings are still
        # outstanding (pending / awaiting_full): an all-terminal set renders via
        # the normal settle path below, byte-identical to before. The first
        # observation arms a grace re-poll (immediate, no sleep — dodges an API
        # read race); the red fires when a fresh fetch re-observes the same set.
        # awaiting_report is included so a real failing leg still fails FAST while
        # the gate is (correctly) holding for the reporter verdict.
        if pending or awaiting_full or awaiting_report:
            ff = failfast_failures(runs)
            if ff:
                key = tuple(sorted((r.get("name", ""), r.get("id") or 0) for r in ff))
                if key == ff_suspect:
                    _emit(
                        f"### ci-summary: FAILED (fail-fast) — {len(ff)} gating "
                        f"check(s) concluded failure while {pending} sibling(s) "
                        f"were still running; no later state can turn this verdict "
                        f"green (a newest-run failure is never forgiven), so the gate "
                        f"REDs now instead of waiting out the remaining legs.",
                        cfg.summary_path,
                    )
                    for r in ff:
                        _emit(f"- ✗ {r.get('name')}: failure", cfg.summary_path)
                    print(
                        "::error::ci-summary failed fast — a gating check concluded "
                        "failure; the remaining siblings were not waited on."
                    )
                    return 1
                ff_suspect = key
                print(
                    f"  fail-fast: {len(ff)} concluded gating failure(s) observed — "
                    f"immediate grace re-poll to confirm before concluding."
                )
                continue  # no sleep: the grace re-poll is deliberately immediate
            ff_suspect = None

        # [OPUS-5] #3781 — UNSATISFIABLE HOLD: detect it, do not wait it out.
        # The draft-tier hold (awaiting_full) is a WAIT for the ready_for_review
        # full-tier re-runs to register. That wait is only meaningful while such a
        # re-run can still happen. When ALL THREE hold —
        #   (1) every awaited sibling has CONCLUDED (pending == 0: nothing is coming),
        #   (2) a draft-marked select instance still lacks a full-tier successor, and
        #   (3) the PR is CURRENTLY A DRAFT (live API read),
        # — the hold is unsatisfiable ON ARRIVAL: a full-tier select is produced ONLY
        # by a non-draft pull_request payload, so while the PR stays a draft no
        # successor can ever register on this SHA. Measured on #3472/#3468/#3681: the
        # gate spent all 155 polls (~67 min of wall-clock budget) in exactly this state
        # and then emitted the refusal it could have emitted at poll 3, on PRs with
        # ZERO failing legs. This is a `return 1` — the SAME refusal render_verdict's
        # stale-draft-tier belt reaches at budget exhaustion, arrived at sooner and
        # named honestly; it never turns a would-be RED green.
        # NOT fired while anything is still settling (pending / awaiting_report), before
        # the startup-race floor, or on a state that has not persisted for
        # unsat_confirm_polls — that window is the discrimination against check-run
        # registration lag. An UNREADABLE draft state does NOT fire either: the gate
        # keeps polling exactly as before (a false RED here would be a new failure mode,
        # while the pre-#3781 behaviour of burning the budget is merely slow).
        #
        # [OPUS-5] #4614 ask 2 — DECIDED, NOT AN OVERSIGHT: the idle-head case (the PR
        # reads NON-DRAFT, or the draft read fails) gets NO exit of its own; it polls to
        # the absolute budget and then REDs via render_verdict's stale-draft-tier belt.
        # The exit above is licensed by a CAUSAL fact — a full-tier select is produced
        # only by a non-draft pull_request payload, so while the PR is a draft the
        # successor provably cannot register. No such fact is available for a non-draft
        # or unreadable head: the successor may simply be late, and the only evidence on
        # offer (e.g. #3765's `_probe_head_activity` head-activity probe, counting
        # non-terminal Actions runs on the head SHA) is CIRCUMSTANTIAL — it would trade
        # a slow-but-correct verdict for a new false-RED failure mode. Burning the
        # budget is merely slow; a false RED on a PR with zero failing legs is a
        # regression. Prior art if this is ever revisited: closed branch
        # `fable/gate-unsatisfiable-hold-3758` at `dc92b4af` (see #3765).
        if (
            awaiting_full
            and pending == 0
            and not awaiting_report
            and attempt >= cfg.min_polls
        ):
            unsat_polls += 1
            if unsat_polls >= cfg.unsat_confirm_polls:
                still_draft, draft_err = _bounded_draft_read(tier_ctx)
                if still_draft is True:
                    stale = draft_selects_unsuperseded(runs)
                    _emit(
                        "### ci-summary: FAILED (fail-fast) — UNSATISFIABLE draft-tier "
                        "hold, not a slow run. Every sibling check-run on this head SHA "
                        f"has CONCLUDED, {len(stale)} draft-marked select instance(s) "
                        "still have no full-tier successor, and the PR is CURRENTLY A "
                        "DRAFT — so no full-tier select can ever register on this SHA "
                        "(only a non-draft pull_request payload produces one). The hold "
                        "is unsatisfiable on arrival, so the gate REDs NOW with the "
                        f"diagnosis instead of burning the remaining "
                        f"{cfg.max_total_polls - attempt} poll(s) on a refusal that is "
                        "already decided (#3781). REMEDY: re-ready the PR (`gh pr "
                        "ready` fires ready_for_review, which re-runs every selecting "
                        "workflow at FULL tier), or push a new head. A worker PR that "
                        "the review pipeline re-drafts mid-gate hits this whenever the "
                        "re-draft lands inside the gate's polling window. "
                        + UNSAT_HOLD_REMEDY,
                        cfg.summary_path,
                    )
                    for n in sorted(set(stale)):
                        _emit(
                            f"- ✗ {n}: draft-tier selection with no full-tier successor",
                            cfg.summary_path,
                        )
                    print(
                        "::error::ci-summary failed fast — unsatisfiable draft-tier hold "
                        "(all siblings terminal, PR still a draft, no full-tier select "
                        "possible)."
                    )
                    return 1
                # Not unsatisfiable (PR is ready, or the state is unreadable): keep
                # polling and re-arm the window, so the live draft state is re-read at
                # most once every unsat_confirm_polls polls rather than on every poll.
                unsat_polls = 0
                print(
                    "  unsatisfiable-hold check: the draft-tier hold is all-terminal but "
                    f"the PR draft state read as {still_draft!r}"
                    + (f" (last error: {draft_err})" if draft_err else "")
                    + " — not declaring it unsatisfiable; continuing to poll."
                )
        else:
            unsat_polls = 0

        # Clean convergence: everything terminal, held for the settle, past the floor.
        if attempt >= cfg.min_polls and pending == 0 and stable >= cfg.settle_polls:
            return render_verdict(runs, cfg.summary_path, tier_ctx)

        # ADAPTIVE SATURATION BUDGET (sq-90cv4): at/after the base budget with work
        # still pending, extend ONLY while the evidence says throughput-starvation
        # (deep queue) or live progress — otherwise it's a genuine hang: RED.
        if attempt >= cfg.base_polls and pending > 0:
            progressing = (
                len(completed_hist) > cfg.progress_window
                and completed_hist[-1] > completed_hist[-1 - cfg.progress_window]
            )
            # [SONNET-4.6] Guard: if fetch_queue_depth() raises (e.g. subprocess.run
            # raises inside the closure before returning None), treat depth as the
            # "unknown" sentinel — never crash the gate, and never grant a saturation
            # extension on depth alone (None → saturated = False, conservative branch).
            try:
                depth = fetch_queue_depth()
            except Exception as exc:
                print(f"  (queue-depth fetch raised {exc!r} — treating depth as unknown)")
                depth = None
            saturated = depth is not None and depth >= cfg.sat_queue_min
            # [OPUS-5] #3783 LIVENESS VETO (header §LIVENESS VETO). An awaited
            # sibling in `in_progress` is POSITIVE evidence the work is alive, and
            # it invalidates BOTH hang signals at once: the queue is idle because
            # that job already dequeued, and a long bounded proof emits no
            # completion for tens of minutes by design. Queue depth may therefore
            # only count toward "hang" when the awaited siblings are `queued` or
            # absent — i.e. genuinely NOT running.
            live = live_siblings(runs)
            if not (saturated or progressing or live):
                print(
                    f"::error::ci-summary timed out — {pending} sibling check-run(s) never "
                    f"finished within the base budget, NO awaited sibling is `in_progress` "
                    f"(nothing is executing), the Actions queue is idle "
                    f"(depth={depth if depth is not None else 'unknown'} < {cfg.sat_queue_min}) "
                    f"and no completions landed in the last {cfg.progress_window} poll(s): "
                    f"genuine hang, not a still-settling set. See the per-poll log above."
                )
                return 1
            live_note = (
                f", {len(live)} sibling(s) EXECUTING ({_name_list(live)})" if live else ""
            )
            if not extension_started:
                extension_started = True
                print(
                    f"::notice::ci-summary base budget reached with {pending} sibling(s) still "
                    f"pending, but the runner pool shows saturation/progress/liveness "
                    f"(queued runs={depth if depth is not None else 'unknown'}, "
                    f"progressing={progressing}{live_note}) — this is a throughput/liveness "
                    f"signal, not a hang. "
                    f"Extending the wait (adaptive budget, sq-90cv4) up to poll "
                    f"{cfg.max_total_polls}."
                )
            else:
                print(
                    f"  extension: queued runs={depth if depth is not None else 'unknown'}, "
                    f"progressing={progressing}{live_note} — still settling."
                )
            if attempt < cfg.max_total_polls:
                sleep_fn(cfg.sat_interval)
            continue

        if attempt < cfg.max_total_polls:
            sleep_fn(cfg.interval)

    # Absolute budget exhausted.
    if pending == 0:
        # The #997 graceful timeout: everything IS terminal, we just never got a
        # full quiet settle — render the real verdict, never a blind RED.
        print(
            "::notice::ci-summary loop budget reached with every sibling check terminal "
            "(the set kept being injected into without a full quiet settle) — rendering "
            "the verdict on the final all-terminal set."
        )
        return render_verdict(runs, cfg.summary_path, tier_ctx)
    # [OPUS-5] #3783 VERDICT TAXONOMY (header §VERDICT TAXONOMY). Exhausting the
    # ABSOLUTE budget with work still outstanding is NOT the same event as a gating
    # leg failing: nothing in the tree has been shown to be broken, the gate simply
    # ran out of wall-clock before the answer existed. Both branches below still
    # exit 1 (fail-closed — an unobserved leg can never be assumed green), but they
    # must not READ like a test failure, because reading them that way is what
    # burned repeated diagnosis on #3758/#3765/#3781/#3783.
    live = live_siblings(runs)
    if live:
        _emit(
            f"### ci-summary: UNDETERMINED (not a test failure) — the wait budget "
            f"expired while {len(live)} awaited sibling(s) were STILL EXECUTING: "
            f"{_name_list(live)}. Nothing has been shown to be broken; these legs are "
            f"alive and governed by their own workflow `timeout-minutes`. This gate "
            f"exits non-zero because an unobserved leg is never assumed green "
            f"(fail-closed), NOT because a check failed — re-run this gate once the "
            f"long-running legs conclude. ABSOLUTE budget (base + saturation "
            f"extension, sq-90cv4) reached at poll {cfg.max_total_polls}.",
            cfg.summary_path,
        )
        print(
            "::error::ci-summary UNDETERMINED — budget expired with siblings still "
            "executing (see the step summary); this is a could-not-determine, not a "
            "failing check. See the per-poll log above."
        )
        return 1
    _emit(
        f"### ci-summary: UNDETERMINED (not a test failure) — {pending} sibling "
        f"check-run(s) never finished within the ABSOLUTE budget (base + saturation "
        f"extension, sq-90cv4). The runner pool stayed saturated longer than the "
        f"extension allows, so the sibling set never resolved; no gating check has "
        f"been shown to fail. Fail-closed exit — re-run this gate once the queue "
        f"drains.",
        cfg.summary_path,
    )
    print(
        f"::error::ci-summary timed out — {pending} sibling check-run(s) never finished "
        f"within the ABSOLUTE budget (base + saturation extension, sq-90cv4). The runner "
        f"pool stayed saturated longer than the extension allows; re-run this gate once "
        f"the queue drains. See the per-poll log above."
    )
    return 1


# ----------------------------- live (gh-backed) wiring -----------------------------


def _gh_json_lines(args: list[str]) -> list[dict]:
    # [SONNET-4.6] Wrap subprocess.run so FileNotFoundError / TimeoutExpired / OSError
    # (e.g. `gh` not on PATH) are converted into FetchError, routing them into the
    # existing bounded-retry / skip-this-poll tolerance in run_gate exactly as a
    # non-zero exit code does — no raw crash, no false pass.
    try:
        proc = subprocess.run(["gh", "api", *args], capture_output=True, text=True)
    except (FileNotFoundError, subprocess.TimeoutExpired, OSError) as exc:
        raise FetchError(f"subprocess raised: {exc}") from exc
    if proc.returncode != 0:
        raise FetchError(proc.stderr.strip()[:300] or f"gh api exited {proc.returncode}")
    out = []
    for line in proc.stdout.splitlines():
        line = line.strip()
        if line:
            out.append(json.loads(line))
    return out


def make_fetch_check_runs(repo: str, sha: str):
    def fetch() -> list[dict]:
        # started_at + id feed the superseded-run ordering (draft-tier CI): a
        # cancelled/stale check-run is forgiven only for a strictly LATER
        # same-normalized-name successor.
        return _gh_json_lines(
            [
                f"repos/{repo}/commits/{sha}/check-runs",
                "--paginate",
                "--jq",
                # external_id carries the reporter's finding-2 correlation token (the
                # triggering feature-matrix run id) so fm_report_status can bind a
                # `feature-matrix report` verdict to the CURRENT `opt-in group (…)`
                # run and reject a stale same-SHA report from an earlier run.
                ".check_runs[] | {name, status, conclusion, details_url, html_url, started_at, id, external_id}",
            ]
        )

    return fetch


def make_fetch_workflow_runs(repo: str, sha: str, self_run_id: str = ""):
    """List Actions workflow runs on SHA; resolution happens by workflow_id."""

    self_cache: list[dict] = []

    def fetch() -> list[dict]:
        runs = _gh_json_lines(
            [
                f"repos/{repo}/actions/runs?head_sha={sha}&per_page=100",
                "--paginate",
                "--jq",
                ".workflow_runs[] | {id, workflow_id, name, path, head_sha, status, "
                "conclusion, created_at, run_started_at, run_attempt, html_url}",
            ]
        )
        # The current run endpoint is fetched once and merged defensively. It
        # guarantees self-workflow identity even during list-index lag (otherwise
        # an older ci-summary run on this SHA could acquire a synthetic pending
        # summary and make the gate wait on itself).
        if self_run_id and not self_cache:
            self_cache.extend(
                _gh_json_lines(
                    [
                        f"repos/{repo}/actions/runs/{self_run_id}",
                        "--jq",
                        "{id, workflow_id, name, path, head_sha, status, conclusion, "
                        "created_at, run_started_at, run_attempt, html_url}",
                    ]
                )
            )
        known_ids = {_as_int(run.get("id")) for run in runs}
        runs.extend(run for run in self_cache if _as_int(run.get("id")) not in known_ids)
        return runs

    return fetch


def make_fetch_attempt_jobs(repo: str):
    """Read the selected run attempt's jobs (authoritative leg inventory)."""

    def fetch(run_id: int, attempt: int) -> list[dict]:
        endpoint = (
            f"repos/{repo}/actions/runs/{run_id}/attempts/{attempt}/jobs?per_page=100"
            if attempt > 1
            else f"repos/{repo}/actions/runs/{run_id}/jobs?filter=latest&per_page=100"
        )
        return _gh_json_lines(
            [
                endpoint,
                "--paginate",
                "--jq",
                ".jobs[] | {id, name, status, conclusion, started_at, completed_at, html_url}",
            ]
        )

    return fetch


def make_redispatch_workflow(repo: str):
    """Return the once-only Actions re-run POST used for newest cancellations."""

    def redispatch(run_id: int) -> None:
        try:
            proc = subprocess.run(
                [
                    "gh",
                    "api",
                    "--method",
                    "POST",
                    f"repos/{repo}/actions/runs/{run_id}/rerun",
                ],
                capture_output=True,
                text=True,
            )
        except (FileNotFoundError, subprocess.TimeoutExpired, OSError) as exc:
            raise FetchError(f"redispatch subprocess raised: {exc}") from exc
        if proc.returncode != 0:
            raise FetchError(
                proc.stderr.strip()[:300] or f"redispatch gh api exited {proc.returncode}"
            )

    return redispatch


def make_fetch_runs(repo: str, sha: str, self_run_id: str = ""):
    """Build the authoritative check fetcher (kept under the historical name)."""
    return WorkflowRunResolver(
        self_run_id=self_run_id,
        fetch_checks=make_fetch_check_runs(repo, sha),
        fetch_workflows=make_fetch_workflow_runs(repo, sha, self_run_id),
        fetch_attempt_jobs=make_fetch_attempt_jobs(repo),
        redispatch=make_redispatch_workflow(repo),
    )


def make_fetch_pr_draft(repo: str, pr_number: str):
    """[FABLE-5] Draft-tier CI: the conclusion-time PR draft-state reader. Returns
    a () -> bool fetcher (True == still a draft) that raises FetchError on any
    API/parse failure — the caller (render_verdict via _draft_recheck) bounded-
    retries and fail-closes to RED, never to a pass."""

    def fetch() -> bool:
        try:
            proc = subprocess.run(
                ["gh", "api", f"repos/{repo}/pulls/{pr_number}", "--jq", ".draft"],
                capture_output=True,
                text=True,
            )
        except (FileNotFoundError, subprocess.TimeoutExpired, OSError) as exc:
            raise FetchError(f"subprocess raised: {exc}") from exc
        if proc.returncode != 0:
            raise FetchError(proc.stderr.strip()[:300] or f"gh api exited {proc.returncode}")
        val = proc.stdout.strip().lower()
        if val == "true":
            return True
        if val == "false":
            return False
        raise FetchError(f"unexpected .draft value {val!r}")

    return fetch


def make_fetch_queue_depth(repo: str):
    """Queued workflow-run count for the repo — the saturation signal. Returns None
    (unknown) on any failure so a permissions/API blip degrades to progress-only,
    never crashes the gate. Needs `actions: read` on the workflow token."""

    def fetch():
        proc = subprocess.run(
            [
                "gh",
                "api",
                f"repos/{repo}/actions/runs?status=queued&per_page=1",
                "--jq",
                ".total_count",
            ],
            capture_output=True,
            text=True,
        )
        if proc.returncode != 0:
            print(f"  (queue-depth fetch failed: {proc.stderr.strip()[:200]} — treating as unknown)")
            return None
        try:
            return int(proc.stdout.strip())
        except ValueError:
            return None

    return fetch


def _self_test() -> int:
    """Hermetic mutation checks for the three #3505 safety properties."""

    def require(condition: bool, message: str) -> None:
        if not condition:
            raise AssertionError(message)

    old_cancelled = {
        "id": 101,
        "workflow_id": 7,
        "name": "CI",
        "status": "completed",
        "conclusion": "cancelled",
        "created_at": "2026-07-21T13:40:00Z",
        "run_started_at": "2026-07-21T13:40:01Z",
        "run_attempt": 1,
        "html_url": "https://github.test/actions/runs/101",
    }
    newest_green = {
        **old_cancelled,
        "id": 102,
        "status": "completed",
        "conclusion": "success",
        "created_at": "2026-07-21T14:30:00Z",
        "run_started_at": "2026-07-21T14:30:01Z",
        "html_url": "https://github.test/actions/runs/102",
    }
    cancelled_check = {
        "name": "test shard",
        "status": "completed",
        "conclusion": "cancelled",
        "details_url": "https://github.test/actions/runs/101/job/1",
        "html_url": "",
        "started_at": "2026-07-21T13:40:01Z",
        "id": 1001,
        "external_id": "",
    }
    resolved, dropped = resolve_newest_workflow_runs(
        [cancelled_check], [old_cancelled, newest_green], "999"
    )
    with redirect_stdout(io.StringIO()):
        superseded_code = render_verdict(resolved)
    require(dropped == 1, "superseded cancelled fixture was not discarded")
    require(
        superseded_code == 0,
        "superseded cancelled fixture failed (mutation: treat-superseded-as-failure)",
    )

    newest_failure = {
        **newest_green,
        "id": 103,
        "conclusion": "failure",
        "created_at": "2026-07-21T14:40:00Z",
        "run_started_at": "2026-07-21T14:40:01Z",
        "html_url": "https://github.test/actions/runs/103",
    }
    failure_resolved, _ = resolve_newest_workflow_runs(
        [], [newest_green, newest_failure], "999"
    )
    with redirect_stdout(io.StringIO()):
        failure_code = render_verdict(failure_resolved)
    require(
        failure_code == 1,
        "newest-run FAILURE passed (mutation: genuine-failure detection weakened)",
    )

    cancelled_latest = {**old_cancelled, "id": 104}
    posts: list[int] = []
    resolver = WorkflowRunResolver(
        self_run_id="999",
        fetch_checks=lambda: [],
        fetch_workflows=lambda: [cancelled_latest],
        fetch_attempt_jobs=lambda run_id, attempt: [],
        redispatch=lambda run_id: posts.append(run_id),
        redispatch_settle_polls=2,
    )
    with redirect_stdout(io.StringIO()):
        resolver()
        resolver()
        bounded_error = None
        try:
            resolver()
        except SupersededLegsError as exc:
            bounded_error = exc
    require(posts == [104], "cancelled workflow redispatch was not bounded to one POST")
    require(
        bounded_error is not None,
        "cancelled workflow never failed loud after bounded redispatch grace",
    )

    print(
        "ci-summary --self-test: ALL ASSERTIONS PASSED "
        "(superseded cancellation ignored; newest failure preserved; redispatch bounded once)"
    )
    return 0


# --- EXIT-0 SURFACE BOUNDARY -----------------------------------------------------
# [FABLE-5] sq-lfmvd. Everything ABOVE this line is the POLL-LOOP transport, whose
# exit code IS the gate verdict and whose exit-0 surface is therefore FIXED and
# enumerated in the module header (_draft_recheck's two, render_verdict's empty-set
# + PASS, and _self_test's — five, no more). Everything BELOW is the `--evaluate`
# transport, where the verdict is the published STATUS and the exit code only says
# whether publishing worked, so its exit-0 paths are counted separately. This exact
# line is the split point used by scripts/tests/test_ci_summary_gate.py
# ::TestUnsatisfiableHoldFastFail::test_the_fast_path_only_ever_reds — moving code
# across it changes which budget the checker applies to it, deliberately.
# ---------------------------------------------------------------------------------

# =================================================================================
# SLOTLESS EVENT-DRIVEN EVALUATION (bead sq-lfmvd; research/ci-gate-slotless-
# aggregation.md §2). [FABLE-5] One short-lived evaluation per sibling
# `workflow_run` event, publishing a COMMIT STATUS. See the module header
# §SLOTLESS EVENT-DRIVEN EVALUATION for the three deliberate divergences.
# =================================================================================

# The commit-status context the evaluator publishes. This is the context the
# branch-protection ruleset will require INSTEAD of `ci-summary / gate` once the
# staged migration completes (docs/branch-protection.md §Slotless gate evaluation).
# A DRAFT-tier evaluation publishes to STATUS_DRAFT_CONTEXT instead, so a reduced
# draft matrix can never satisfy the required context — the status-context analogue
# of the tiered `gate` / `gate, draft-tier` job name.
STATUS_CONTEXT = "ci-gate"
STATUS_DRAFT_CONTEXT = STATUS_CONTEXT + "/draft-tier"
# GitHub truncates a commit-status description past 140 characters.
STATUS_MAX_DESCRIPTION = 140
# The four evaluation outcomes. "skip" is the NO-INFORMATION outcome: publish
# nothing at all rather than overwrite a real verdict with a guess (divergence 2).
EVAL_STATES = ("pending", "success", "failure", "skip")


@dataclass
class EvalConfig:
    """One-shot evaluation tunables. Prod values are seconds, not minutes: the whole
    point is that this never becomes a waiter. `settle_seconds` doubles as the
    DEBOUNCE window (design §3.5) — ci-gate-status.yml's per-head-SHA
    `cancel-in-progress` concurrency group means a burst of sibling completions
    collapses to one surviving evaluation, and the burst is absorbed inside this
    sleep. `confirm_seconds` is the startup-race floor + fail-fast grace re-poll
    (§3.6). Tests inject zeros."""

    self_run_id: str = ""
    settle_seconds: int = 25
    confirm_seconds: int = 15
    max_fetch_attempts: int = 3
    summary_path: str = field(default_factory=lambda: os.environ.get("GITHUB_STEP_SUMMARY", ""))


@dataclass
class Observation:
    """One fetch of the head SHA's sibling set, filtered EXACTLY as the poll loop
    filters it (forgive_superseded -> drop self -> drop draft-gate artifacts), so
    `runs` is the set render_verdict would judge."""

    runs: list[dict]
    pending: int
    awaiting_full: bool
    awaiting_report: bool
    failfast_key: tuple

    @property
    def settled(self) -> bool:
        return not (self.pending or self.awaiting_full or self.awaiting_report)

    def describe(self) -> str:
        holds = []
        if self.pending:
            holds.append(f"{self.pending} of {len(self.runs)} check(s) still running")
        if self.awaiting_full:
            holds.append("awaiting the full-tier re-run (draft-tier selection present)")
        if self.awaiting_report:
            holds.append("awaiting the feature-matrix reporter verdict")
        return "; ".join(holds) or f"{len(self.runs)} check(s) observed"


def observe_once(fetch_runs, cfg: EvalConfig, tier_ctx: TierContext | None = None):
    """Fetch + filter the sibling set once. Returns an Observation, or None when the
    set could not be observed at all (bounded-retried FetchError / SupersededLegsError
    — divergence 2: no information means no write)."""
    raw: list[dict] | None = None
    for _ in range(max(1, cfg.max_fetch_attempts)):
        try:
            raw = fetch_runs()
            break
        except SupersededLegsError as exc:
            # The poll loop REDs here because it has a job conclusion to spend. An
            # evaluation has a durable status instead: a superseded newest run needs
            # a fresh run or head, which will itself fire another evaluation.
            print(f"::notice::ci-gate: superseded legs ({exc}) — publishing nothing this evaluation.")
            return None
        except FetchError as exc:
            print(f"ci-gate: check-run fetch failed ({exc}) — retrying.")
    if raw is None:
        print("::notice::ci-gate: the sibling set could not be fetched — publishing nothing.")
        return None
    kept, _forgiven = forgive_superseded(raw)
    runs = [
        r for r in kept
        if not is_self(r, cfg.self_run_id)
        and not is_draft_gate_artifact(r.get("name", ""))
    ]
    pending = sum(1 for r in runs if r.get("status") != "completed")
    awaiting_full = bool(
        tier_ctx
        and tier_ctx.run_tier == "full"
        and tier_ctx.event_name == "pull_request"
        and draft_selects_unsuperseded(runs)
    )
    awaiting_report = fm_report_status(runs) == "pending"
    ff = failfast_failures(runs) if (pending or awaiting_full or awaiting_report) else []
    ff_key = tuple(sorted((r.get("name", ""), r.get("id") or 0) for r in ff))
    return Observation(runs, pending, awaiting_full, awaiting_report, ff_key)


def _verdict_headline(runs: list[dict], summary_path: str,
                      tier_ctx: TierContext | None) -> tuple[int, str]:
    """Run the REAL render_verdict and lift its headline for the status description.
    The full render is still printed (and still appended to the step summary by
    _emit); only the one-line headline is extracted, so the description can never
    describe a verdict the brain did not actually render."""
    buf = io.StringIO()
    with redirect_stdout(buf):
        code = render_verdict(runs, summary_path, tier_ctx)
    text = buf.getvalue()
    print(text, end="", flush=True)
    headline = ""
    for line in text.splitlines():
        marker = "ci-summary:"
        if marker in line and (line.startswith("### ") or line.startswith(marker)):
            headline = line.split(marker, 1)[1].strip()
            break
    if not headline:
        headline = "verdict rendered" if code == 0 else "gate FAILED"
    return code, headline


def evaluate_once(fetch_runs, cfg: EvalConfig, sleep_fn=time.sleep,
                  tier_ctx: TierContext | None = None) -> tuple[str, str]:
    """ONE short-lived evaluation. Returns (state, description) with state in
    EVAL_STATES. Never blocks on a sibling: an unsettled set is `pending`.

    A success or failure is published ONLY after the same conclusion survives a
    CONFIRM re-fetch (the MIN_POLLS/settle_polls analogue, §3.6), which is also the
    fail-fast grace re-poll — so a mid-flight red must be re-observed with the
    IDENTICAL failing-leg key before it is published."""
    sleep_fn(cfg.settle_seconds)
    first = observe_once(fetch_runs, cfg, tier_ctx)
    if first is None:
        return ("skip", "sibling set unobservable")
    if not first.runs:
        # Divergence 1: an evaluation is fired BY a run on this SHA, so an empty set
        # is API lag, never the poll loop's genuine stable-empty set.
        print("::notice::ci-gate: zero sibling check-runs on this head (API lag) — publishing nothing.")
        return ("skip", "no sibling check-runs observed yet")
    if not (first.settled or first.failfast_key):
        return ("pending", first.describe())

    sleep_fn(cfg.confirm_seconds)
    second = observe_once(fetch_runs, cfg, tier_ctx)
    if second is None or not second.runs:
        return ("skip", "confirm re-fetch did not observe the sibling set")
    if second.settled:
        code, headline = _verdict_headline(second.runs, cfg.summary_path, tier_ctx)
        return ("success" if code == 0 else "failure", headline)
    if first.failfast_key and first.failfast_key == second.failfast_key:
        names = ", ".join(name for name, _ in second.failfast_key[:3])
        return ("failure", f"fail-fast — gating check(s) concluded failure: {names}")
    return ("pending", second.describe())


def make_publish_status(repo: str, sha: str):
    """POST a commit status on `sha`. Needs `statuses: write`. Raises FetchError on
    failure so the caller can exit loudly — an evaluation that cannot publish is the
    ONE condition that must red the evaluator job itself."""

    def publish(context: str, state: str, description: str, target_url: str = "") -> None:
        args = [
            "gh", "api", "--method", "POST", f"repos/{repo}/statuses/{sha}",
            "-f", f"state={state}",
            "-f", f"context={context}",
            "-f", f"description={description[:STATUS_MAX_DESCRIPTION]}",
        ]
        if target_url:
            args += ["-f", f"target_url={target_url}"]
        try:
            proc = subprocess.run(args, capture_output=True, text=True)
        except (FileNotFoundError, subprocess.TimeoutExpired, OSError) as exc:
            raise FetchError(f"status POST subprocess raised: {exc}") from exc
        if proc.returncode != 0:
            raise FetchError(proc.stderr.strip()[:300] or f"status POST exited {proc.returncode}")

    return publish


def resolve_eval_tier(event_name: str, fetch_pr_draft) -> str:
    """The tier THIS evaluation is allowed to publish. Derived from the PR's LIVE
    draft state rather than a trigger payload, because a `workflow_run` payload
    carries the SIBLING run's context, not the PR's. Returns "full", "draft", or
    "unknown" — and "unknown" means publish nothing (divergence 2), never a guess
    about which context to write."""
    if fetch_pr_draft is None:
        return "full"  # push / merge_group: no PR, so no draft tier exists
    probe = TierContext(run_tier="draft", event_name=event_name, fetch_pr_draft=fetch_pr_draft)
    still_draft, last_err = _bounded_draft_read(probe)
    if still_draft is None:
        print(f"::notice::ci-gate: the PR's draft state is unreadable ({last_err}) — publishing nothing.")
        return "unknown"
    return "draft" if still_draft else "full"


def run_evaluator(cfg: EvalConfig, fetch_runs, publish, tier: str,
                  event_name: str, fetch_pr_draft=None, seed: bool = False,
                  sleep_fn=time.sleep, target_url: str = "") -> int:
    """Publish ONE commit status for the head SHA. `seed=True` is the cheap
    `workflow_run: requested` path — a run has just been created on this head, so
    the set is by construction not all-terminal and the evaluation short-circuits to
    `pending` without fetching anything. Returns the process exit code, which is 0
    for EVERY verdict (the verdict is the STATUS, not this job's conclusion) and 1
    only when the status could not be published at all."""
    if tier == "unknown":
        return 0
    context = STATUS_DRAFT_CONTEXT if tier == "draft" else STATUS_CONTEXT
    if seed:
        state, description = ("pending", "a sibling workflow run was just requested on this head")
    else:
        tier_ctx = TierContext(run_tier=tier, event_name=event_name, fetch_pr_draft=fetch_pr_draft)
        state, description = evaluate_once(fetch_runs, cfg, sleep_fn=sleep_fn, tier_ctx=tier_ctx)
    if state == "skip":
        print(f"ci-gate: no status published ({description}).")
        return 0
    # GitHub truncates a description past STATUS_MAX_DESCRIPTION; do it HERE (not only
    # in the POST helper) so the length invariant holds for every publisher, and mark
    # the cut so a reader knows the TEXT is elided, not the verdict partial.
    if len(description) > STATUS_MAX_DESCRIPTION:
        description = description[: STATUS_MAX_DESCRIPTION - 1].rstrip() + "…"
    try:
        publish(context, state, description, target_url)
    except FetchError as exc:
        print(
            f"::error::ci-gate: could not publish the `{context}` commit status "
            f"({exc}). The evaluation itself concluded {state}."
        )
        return 1
    _emit(
        f"ci-gate: published `{context}` = **{state}** — {description}",
        cfg.summary_path,
    )
    return 0


def _evaluate_main() -> int:
    """`--evaluate` entry point, driven by .github/workflows/ci-gate-status.yml."""
    repo = os.environ.get("REPO", "")
    sha = os.environ.get("SHA", "")
    self_run_id = os.environ.get("SELF_RUN_ID", "")
    if not repo or not sha:
        print("::error::ci-gate: REPO and SHA must both be set.")
        return 1
    registry_path = os.environ.get("ADVISORY_REGISTRY", ADVISORY_REGISTRY_PATH)
    try:
        declared = load_advisory_registry(registry_path)
    except AdvisoryRegistryError as exc:
        # Same fail-closed posture as the poll loop: without the registry the
        # evaluator cannot know which checks are DECLARED non-gating. It publishes
        # nothing and reds ITSELF, so the required status simply stays where it was.
        print(
            f"::error::ci-gate: the advisory registry is unreadable ({exc}). No status "
            f"published. Ensure ci-gate-status.yml's sparse-checkout includes "
            f"{ADVISORY_REGISTRY_PATH}."
        )
        return 1
    trigger_event = os.environ.get("TRIGGER_EVENT", "")
    pr_number = os.environ.get("PR_NUMBER", "").strip()
    fetch_pr_draft = make_fetch_pr_draft(repo, pr_number) if pr_number else None
    # The evaluated tier's `event_name` mirrors what the poll loop would see for this
    # head: a PR head is "pull_request" (which arms render_verdict's draft-tier belt),
    # a merge-group ref is not.
    event_name = "pull_request" if pr_number else trigger_event
    tier = resolve_eval_tier(event_name, fetch_pr_draft)
    print(
        f"ci-gate: {len(declared)} declared-advisory job name(s) loaded; evaluating "
        f"{repo}@{sha[:12]} (trigger={trigger_event or '<unset>'}, tier={tier})."
    )
    return run_evaluator(
        EvalConfig(self_run_id=self_run_id),
        make_fetch_runs(repo, sha, self_run_id),
        make_publish_status(repo, sha),
        tier,
        event_name,
        fetch_pr_draft=fetch_pr_draft,
        seed=os.environ.get("TRIGGER_ACTION", "") == "requested",
        target_url=os.environ.get("TARGET_URL", ""),
    )


def main() -> int:
    if sys.argv[1:] == ["--self-test"]:
        return _self_test()
    # [FABLE-5] sq-lfmvd: the SLOTLESS transport — one short-lived evaluation that
    # publishes a commit status instead of residing on a runner slot. Same brain.
    if sys.argv[1:] == ["--evaluate"]:
        return _evaluate_main()
    if sys.argv[1:]:
        print("usage: ci_summary_gate.py [--self-test | --evaluate]", file=sys.stderr)
        return 2
    repo = os.environ.get("REPO", "")
    sha = os.environ.get("SHA", "")
    self_run_id = os.environ.get("SELF_RUN_ID", "")
    if not repo or not sha or not self_run_id:
        print("::error::ci-summary: REPO, SHA and SELF_RUN_ID must all be set.")
        return 1
    # [OPUS-5] #3773 — establish the DECLARED-advisory set BEFORE polling, and fail
    # LOUD + FAST if it cannot be read. Evaluating a merge-authorising gate without
    # knowing which checks were deliberately declared non-gating is exactly the
    # over-promise this issue is about, so an unreadable registry is exit 1 in
    # seconds, never a 37-minute wait or a silent "everything gates".
    registry_path = os.environ.get("ADVISORY_REGISTRY", ADVISORY_REGISTRY_PATH)
    try:
        declared = load_advisory_registry(registry_path)
    except AdvisoryRegistryError as exc:
        print(
            f"::error::ci-summary: the advisory registry is unreadable ({exc}). The gate "
            f"cannot decide which checks are DECLARED non-gating, so it fails closed. "
            f"Ensure ci-summary.yml's sparse-checkout includes {ADVISORY_REGISTRY_PATH}."
        )
        return 1
    print(
        f"ci-summary: {len(declared)} declared-advisory job name(s) loaded from "
        f"{registry_path}; every other check GATES (#3773)."
    )
    # [FABLE-5] Draft-tier CI: the tier THIS run evaluates is decided by its own
    # trigger payload — a pull_request event with draft == true is a DRAFT-tier
    # gate (ci-summary.yml exports the payload's draft flag + PR number). Every
    # other event/state (push / merge_group / non-draft PR / missing payload) is
    # FULL tier, which keeps push/merge_group semantics byte-identical.
    event_name = os.environ.get("EVENT_NAME", "")
    pr_draft = os.environ.get("PR_DRAFT", "").strip().lower()
    pr_number = os.environ.get("PR_NUMBER", "").strip()
    run_tier = "draft" if (event_name == "pull_request" and pr_draft == "true") else "full"
    tier_ctx = TierContext(
        run_tier=run_tier,
        event_name=event_name,
        fetch_pr_draft=make_fetch_pr_draft(repo, pr_number) if pr_number else None,
    )
    print(f"ci-summary: evaluating tier={run_tier} (event={event_name or '<unset>'}).")
    cfg = Config(self_run_id=self_run_id)
    return run_gate(cfg, make_fetch_runs(repo, sha, self_run_id), make_fetch_queue_depth(repo),
                    tier_ctx=tier_ctx)


if __name__ == "__main__":
    sys.exit(main())
