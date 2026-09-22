#!/usr/bin/env python3
"""CI EXECUTION-LATENCY alarm — three detected modes, one documented gap. 🤖 SPARQ agent.

Bead sq-1lc4i. Maintainer-requested (2026-07-28): "setting up alerting for when runners
are not picked up for crons or delayed for other dispatches on both repos. This is an
indicator that you are choking on runner availability and is something that we would need
to manage."

WHAT IS AND IS NOT COVERED. Three modes are detected. The fourth — queue wait, which is
runner availability proper and the maintainer's literal ask — is NOT detected, and that
gap is documented rather than filled with a check that cannot see it. Read the QUEUE WAIT
section below the constants before assuming this file covers starvation; it does not.

  M1 CRON-FIRING DEFICIT   DETECTED. A scheduled run that never fires produces NO artifact
                           at all. There is no run to inspect, no conclusion, no check-run.
                           The only way to see it is to compute what the lane's own `cron:`
                           EXPECTED and compare against what arrived.
  M3 EXECUTION OVERRUN     DETECTED. A run that is `in_progress` far past its lane's own
                           measured duration. It consumes capacity while looking perfectly
                           healthy: no red, no alert, no artifact.
  M4 CADENCE FIDELITY      DETECTED. A lane whose declared `cron:` rate is a fiction on
                           this repo — it keeps running, keeps going green, and delivers a
                           small fraction of what it declares, every day. CHRONIC, not an
                           incident: it reports into the deduped issue and deliberately
                           does NOT red the run. See the M4 section below.
  QUEUE WAIT               NOT DETECTED — deliberate, documented gap. Zero non-zero queue
                           waits across 44,190 completed runs, and the one event that
                           motivated a detector turned out to be a configured
                           `max-parallel: 8`, not withheld runners. See the long note
                           below the constants.

=============================================================================
THE MEASUREMENT TRAP THIS DETECTOR EXISTS BECAUSE OF — read before editing
=============================================================================
The obvious instrument for "are runners being picked up" is job-level
`started_at - created_at` aggregated over recent runs. It does not work, and it fails in
the SAFE-LOOKING direction: it reports health while a leak is live.

MEASURED 2026-07-28 on run 30333511110 (sparq-org/sparq, a `schedule` CI run on main that
ran 6.7 hours), RE-MEASURED over the completed run, N=84 jobs:

    pickup lag (started_at - created_at):        min 0s   MAX 10s   >60s: 0 jobs
    job-creation lag (job.created_at - run.created_at):    MAX 402 min  (6.7 hours)

Every job was picked up within ten seconds of EXISTING. A matrix leg waiting behind a
concurrency limit is not created as a job object at all, so it contributes no queue depth
and no pickup lag; `status=queued` read ZERO for the whole 6.7 hours.

CORRECTION, and it matters. An earlier revision presented this run as evidence of an
invisible CAPACITY problem. It is not. The 402 minutes is INTRA-MATRIX — `mutation ratchet`
has 51 legs spread over 401.9 min, while the `test` matrix (5 legs, no `max-parallel`) has
a spread of 0.0 min — and `mutants-nightly-advisory` declares `max-parallel: 8`. 51 legs at
8 concurrent is 6.4 waves, which IS the 402 minutes. This run shows a configured cap
working correctly, not GitHub withholding runners. The API observation below stands; the
capacity inference drawn from it did not, and was removed along with the detector built on
it.

Two consequences, both load-bearing:

  (a) A job that never finishes never appears in a completed run. Any statistic computed
      over COMPLETED work is structurally incapable of detecting a hang or a leak — the
      failing population is precisely the population it excludes. This is survivorship
      bias, and it is why M3's DETECTION VARIABLE is the live age of a `status=in_progress`
      run and NOTHING derived from finished work.

  (b) Completed-run history is still the right population for the THRESHOLD — "how long
      does this lane normally take" is a question about runs that took a length of time,
      i.e. runs that ended. The distinction is: completed runs answer HOW LONG IS NORMAL;
      only live runs answer IS SOMETHING STUCK RIGHT NOW. Never let (b) drift into (a).

  (c) FAIL-OPEN HOLE, closed deliberately: if a lane's runs never complete, it has no
      completed history, hence no derived baseline. A detector that SKIPS a lane with no
      baseline would go silent exactly when a lane is 100% hung. So a missing baseline
      falls back to EXEC_FLOOR_SECONDS instead of skipping. See find_execution_overruns.

=============================================================================
COORDINATION WITH sparq-org/sparq#4368 (`scripts/cron_lane_liveness.py`)
=============================================================================
#4368 watches CRON-ONLY lanes — its scope predicate is: `on:` has `schedule`, AND every
other `on:` key is in {schedule, workflow_dispatch}. It asks "is this lane DEAD?" over a
window of max(3 x period, 6h), keyed on run CONCLUSIONS (consecutive failures / no green).

M1 here is deliberately the EXACT SET COMPLEMENT: a workflow is in M1 scope iff it has a
`schedule:` AND at least one trigger OUTSIDE {schedule, workflow_dispatch}. The predicate
is computed, not listed, so the two populations can never overlap and can never both raise
on one lane — and if #4368's scope is edited, this one moves with it.

That complement is not a leftover. MEASURED on sparq main d2b6034a8: of 31 workflows
carrying a `schedule:`, 18 are cron-only (#4368's) and 13 are NOT — and the 13 include
every throughput-critical orchestration lane in the repo: ci.yml, auto-arm.yml (10-min),
batch-merge.yml (15-min), verdict-bridge.yml (10-min), pr-backlog.yml, refresh-start-here,
plus bench/codeql/container-scan/fuzz/scorecard/xpath-differential/zk-toolchain. #4368
cannot see any of them, and `ci.yml` — the single largest capacity consumer in the repo —
is among them.

M3 is keyed on live run STATE, which #4368 does not look at in any scope.

M4 is scoped to EVERY scheduled lane, cron-only included, and is NOT partitioned against
either of the other two. It does not need to be: M1 and #4368 both ask "is this LANE
healthy?", and two health verdicts on one lane are the duplicate that matters. M4 asks
"is this lane's DECLARATION achievable here?" — a property of the workflow file, answered
identically on a good day and a bad one — and it emits ONE repo-level finding into the
same deduped issue, never a per-lane issue. So it can share a lane with M1 the way a lint
shares a file with a test.

=============================================================================
M4 — WHY THIS IS A DECLARATION DETECTOR AND NOT AN INCIDENT DETECTOR (#4802)
=============================================================================
#4802 measured, on 2026-07-28 to ~12:30Z, five sub-hourly lanes that had each produced
5 scheduled runs since 00:00Z with largest gaps of 193-200 min, every run green, and asked
for an alarm on "sustained under-delivery" against declared cadence.

RECONCILED against the measurement already recorded in the M1 constants below — the same
repo, the same lanes, the 24h to 2026-07-28T13:15Z — those numbers are NOT an incident:

    lane                    #4802 (12.5h to 12:30Z)   M1 census (24h to 13:15Z)
    promote-on-approval                  5                        12
    verdict-bridge                       5                        12
    auto-arm                             5                        12
    rearm-sweeper                        5                        13
    retriage                             5                        12

5 fires in 12.5h is ~9.6/day against a measured achieved baseline of 12/day, i.e. ratio
~0.8 — and the MINIMUM ratio across the 24 lanes on that same healthy census was 0.75.
The #4802 day is inside the healthy band on the only baseline this repo has measured. So
the incident detector the issue asked for cannot be validated: its "known positive" and
its "known negative" are the same day, and any per-lane threshold that separates them
sits above the worst healthy lane and fires on healthy lanes forever. Two candidate
instruments were worked through and rejected on exactly that:

  * ESTATE-WIDE AGGREGATE RATIO (sum actual / sum expected over all lanes). Tightens the
    distribution, but the positive is 0.83 and the healthy aggregate implied by the same
    census is ~1.00, over one observation each. One day per side is not a validation.
  * CHANGE POINT (the rate INSIDE the #4802 window against the rate over the REST of the
    same 24h census). 5 fires in the 12.5h since 00:00Z is 0.40/h; the other 7 of that
    lane's 12 fall in the remaining ~11.5h, i.e. ~0.61/h — a ratio of ~0.66, a real drop.
    But GitHub's `schedule` delivery is documented best-effort and bursty at sub-day
    scale, and this file already rejected sub-day prorating for exactly that reason (see
    the NEW-LANE WINDOW note). There is no measured healthy distribution of this statistic
    to put a floor inside, and one observation cannot supply one.

What IS separable, cleanly and with the measurement already in hand, is the thing #4802's
headline actually names: **a lane firing at a small fraction of its DECLARED cadence.**
On the 24h to 2026-07-28T13:15Z census the achieved/declared fidelity of every measured
lane fell into two groups with an EMPTY 3x band between them:

    declaration honoured   16 daily lanes 1/1 = 1.00 ; refresh-start-here 3/4 = 0.75
    -------------------------- nothing at all between 0.25 and 0.75 ----------------
    declaration a fiction  retriage 12/48 = 0.25 ; batch-merge 12/96 = 0.125 ;
                           auto-arm 12/144 = 0.083 ; promote-on-approval 12/144 = 0.083 ;
                           verdict-bridge 12/144 = 0.083 ; rearm-sweeper 13/144 = 0.090

That is the detector this file can honestly ship, and it answers the operational half of
#4802: this estate's self-maintenance loops (arming, re-arming, promote-on-approval,
retriage, the merge-group watchdog) are WRITTEN as 10-minute loops and RUN as ~2-hour
loops, and nothing anywhere says so.

WHAT M4 DOES NOT DO, stated plainly rather than implied: it does not detect a transient
scheduler degradation. If GitHub halves its delivery for six hours, M4's 24h fidelity
barely moves and M4 stays quiet. That gap is real and remains open; closing it needs a
measured healthy distribution of a sub-day statistic, which this repo does not have.

WHY M4 DOES NOT RED THE RUN. M4 is chronic by construction: it is true on a good day, and
it stays true until a workflow file is edited. Wiring it to `exit 1` would leave this
hourly lane permanently red, and a permanently-red alarm is a muted alarm — which would
mute M1 and M3, the two INCIDENT modes that share it. That is the same argument the QUEUE
WAIT note uses to reject a job-creation-lag detector, applied to this file's own output.
M4's artifact is the deduped repo-level issue, which is the shape #4802 asked for ("One
repo-level finding, deduped, is probably right"); exit 1 stays reserved for M1/M3.

=============================================================================
THRESHOLDS — every one derived from measurement, with the N stated
=============================================================================
See the constants below. Nothing here is chosen by taste; each carries the sample it came
from and the multiple it represents. Re-derive with scripts/tests/-adjacent tooling if the
repo's shape changes; a threshold whose provenance is not stated is not maintainable.

NOT A GATE. This lane REDs when it finds a choke, and REDs fail-loud when the detector
itself breaks; neither is a property of the commit under test. It is DECLARED in
`.github/advisory-registry.json`, which since #3773 is the ONLY thing that makes a
check-run non-gating — not where its check-run lands. A scheduled run on main puts its
check-run on the same head SHA the push-triggered `ci-summary` gate polls, so the
declaration is what keeps it safe to RED. Pinned by
scripts/tests/test_alarm_lanes_non_gating.py.

Usage:
  ci_execution_latency_alarm.py                       # real (gh; env GITHUB_REPOSITORY)
  ci_execution_latency_alarm.py --dry-run             # print findings; no gh writes
  ci_execution_latency_alarm.py --state-file s.json \
      --now 2026-07-28T12:00:00Z --dry-run            # hermetic (tests)
  ci_execution_latency_alarm.py --self-test           # hermetic fixtures

Exit codes: 0 = no INCIDENT (an M4 chronic finding may still have been filed), 1 = an
incident choke was found (RED by design), 2 = the detector itself is broken (fail-loud).
Collapsing any pair would make a broken detector indistinguishable from a healthy repo.

Stdlib + PyYAML (preinstalled on GitHub-hosted runners) + the `gh` CLI.
"""

from __future__ import annotations

import argparse
import datetime as dt
import json
import os
import subprocess
import sys
from pathlib import Path

try:
    import yaml
except ImportError as _exc:  # pragma: no cover - fail loud, never silently skip M1
    yaml = None
    _YAML_IMPORT_ERROR = _exc


class AlarmError(RuntimeError):
    """The detector itself is broken. Always exit 2 — never mask a choke."""


class CronError(ValueError):
    """An unparseable cron expression. Fail-safe QUIET (the lane is skipped, and counted
    in the census as `cron-unparseable` so the skip is visible rather than silent)."""


KEY_PREFIX = "ci-execution-latency-alarm-key"
BASE_LABELS = ["ci-latency-alarm", "auto"]
# Separator for the flat "<workflow path>|<event>" baseline keys a hermetic --state-file
# carries (JSON object keys cannot be tuples). A pipe cannot occur in either half.
BASELINE_KEY_SEP = "|"

# --- M1: cron firing deficit -------------------------------------------------------
# WINDOW: 24h. Chosen from measurement, not taste. A 6h window was tried first and
# rejected: at 6h a DAILY lane has an expectation of 0 or 1 and a single jittered fire
# swings the ratio between 0.00 and 1.00, so the mode would either alarm constantly or be
# switched off. Over 24h every daily lane on sparq delivered EXACTLY its expectation.
CRON_WINDOW_HOURS = 24.0
# THE CAP, and why nominal cron rate is NOT a usable expectation.
# MEASURED on sparq-org/sparq over the 24h to 2026-07-28T13:15Z, per-lane, event-filtered:
#   DAILY lanes (asan, bench, ci, kani, miri, fuzz, metamorph, differential, ... 16 of
#   them): delivered ratio 1.00 — every single one.
#   SUB-HOURLY lanes: delivered 0.08-0.25 of nominal. auto-arm (`*/10`, nominal 144) got
#   12. promote-on-approval (`*/10`, 144) got 12. rearm-sweeper (6/h, 144) got 13.
#   verdict-bridge (6/h, 144) got 12. batch-merge (4/h, 96) got 12. retriage (`*/30`, 48)
#   got 12.
# GitHub's `schedule` trigger is best-effort and documented as droppable under load; on a
# repo this busy it converges to ~12 scheduled runs per lane per day REGARDLESS of the
# cron. Comparing against nominal would put six lanes permanently in breach — an alarm
# that is always red is muted, and a muted alarm is how a real outage gets ignored.
# So the expectation is CAPPED at what GitHub demonstrably delivers. 0.5/h = 12 per 24h;
# the highest any sparq lane actually achieved in 24h was 13.
CRON_MAX_CREDIBLE_FIRINGS_PER_HOUR = 0.5
# Ignore firings younger than this. Without it the window edge races the scheduler: at
# 03:17:30 a `17 3 * * *` lane has an expectation of 1 whose run may not be created yet,
# giving ratio 0.00 and a phantom deficit on every single tick that lands near the cron.
CRON_GRACE_MINUTES = 15
# A lane needs at least this much capped expectation for a ratio to mean anything.
CRON_MIN_EXPECTED = 1
# Raise below this delivery ratio. VALIDATED, not chosen: over the 24 covered sparq lanes
# the MINIMUM capped delivery ratio was 0.75 (refresh-start-here.yml, 3 of 4), so 0.60
# sits 0.15 BELOW the worst healthy lane and fires on ZERO of them. It still catches the
# mode that has no artifact — a lane that stops entirely goes to 0.00 — and catches a
# halving of a sub-hourly lane's achieved rate (12 -> 6 of a capped 12 = 0.50).
CRON_DELIVERY_FLOOR = 0.60

# --- M4: declared-vs-achieved cadence fidelity (#4802) -----------------------------
# WINDOW: 24h, reusing the same fetched run times as M1. 24h is "several times the period"
# by a wide margin for every lane M4 admits (the shortest-period lane it can admit declares
# 13 fires/day), so a single busy hour cannot move it: one lost hour of a `*/10` lane costs
# 6 of 142 declared fires, 4%.
# FIXED, and deliberately NOT `--window-hours`. The floor and the admission ceiling below
# are validated against a 24h census and nothing else; at `--window-hours 6` the same 0.40
# floor would be judging six-hour delivery, which is exactly the sub-day burstiness the
# header says M4 must NOT try to see. `run()` therefore does not thread the M1 CLI window
# into M4 — see the call site.
M4_WINDOW_HOURS = CRON_WINDOW_HOURS
# ADMISSION. A lane is only asked the fidelity question if its DECLARED rate exceeds what
# this repo has been measured to deliver at all — i.e. nominal > the M1 cap of 12/24h.
# Two reasons, both load-bearing:
#   1. Below the cap the declaration is not a fiction, so a low ratio means the lane had a
#      BAD DAY — which is M1's question, on M1's capped denominator. Admitting those lanes
#      would put M1 and M4 on the same lane answering the same question, which is the
#      duplicate the #4368 partition exists to prevent.
#   2. A ratio needs a denominator. The smallest admissible nominal is 13; the achievable
#      lanes measured on 2026-07-28 declared 1 or 4, where one dropped firing swings the
#      ratio by 0.25-1.00 and no floor means anything.
# The admission rule is DERIVED from CRON_MAX_CREDIBLE_FIRINGS_PER_HOUR rather than being
# its own number, so the two cannot drift apart.
# FLOOR. VALIDATED against the 24-lane census recorded in the M1 note above, not chosen:
# the achievable lanes bottomed out at 0.75 and the fictional ones topped out at 0.25, an
# EMPTY 3x band. 0.40 sits inside it — 1.6x above the highest fictional lane and 1.9x
# below the lowest honoured one — and is the geometric-ish middle of an interval with no
# observation in it. It is deliberately set BELOW 0.50: an hourly lane (nominal 24) would
# land at exactly 0.50 IF the measured ~12/day ceiling held for it, but that ceiling was
# only measured on lanes declaring far more than 12, so firing on it would be acting on an
# inference rather than a measurement. `ci-latency-alarm.yml` itself is that lane, and M4
# is quiet on it by design. That is a KNOWN MISS, recorded here rather than papered over.
M4_FIDELITY_FLOOR = 0.40

# =============================================================================
# QUEUE WAIT / RUNNER STARVATION — DELIBERATELY NOT DETECTED. Read this before
# adding a detector for it; the obvious one does not work and was removed.
# =============================================================================
# There is NO detector on this route. That is a documented gap, not an oversight, and it
# is recorded here rather than papered over with a check that cannot see the thing.
#
# An earlier revision of this file shipped "M2", a queue-wait mode keyed on
# `now - run["created_at"]` for runs in `status=queued`. It was removed for three
# measured reasons, in increasing order of importance.
#
# 1. THE ESTIMATOR WAS WRONG. The run-level `created_at` is the creation time of ATTEMPT 1
#    and does NOT reset on re-run, while `run_attempt` and `run_started_at` DO track the
#    live attempt. MEASURED on jeswr/agent-account-registry run 30318886362:
#        run-level    run_attempt=2  created_at=00:58:13Z  run_started_at=02:37:40Z
#        /attempts/1                 created_at=00:58:13Z  run_started_at=00:58:13Z   0s
#        /attempts/2                 created_at=02:37:41Z  run_started_at=02:37:40Z   0s
#    Neither attempt ever queued; the 99 minutes is idle time before a human pressed
#    re-run. Executed on that payload the detector reported waited_seconds=5967 for a
#    sub-second pickup, i.e. it fired on every re-run older than the threshold.
#
# 2. AFTER FIXING THAT, THE VARIABLE NEVER MOVES. Per-workflow scan, splitting on
#    `run_attempt` and resolving every re-run against its own `/attempts/{n}`:
#
#                                             sparq-org/sparq   agent-account-registry
#      completed runs scanned                          35,128                    9,062
#      contaminated estimator, over 15 min                  7                        6
#        ... of which are RE-RUNS                     7 (ALL)                  6 (ALL)
#      DECONTAMINATED (run_attempt == 1)               35,070                    9,053
#        over 15 min                                        0                        0
#        with ANY non-zero wait at all                      0                        0
#        maximum observed wait                           0.0s                     0.0s
#
#    Across 44,190 completed runs not ONE attempt recorded even a non-zero queue wait.
#    The distribution is not zero-INFLATED, it is exactly all-zero, so no percentile,
#    multiple or maximum exists to anchor a threshold to.
#
# 3. THE ONE EVENT THAT MOTIVATED THE MODE WAS NOT A CAPACITY EVENT. Run 30333511110 was
#    cited as a 6.7h run whose starvation was invisible to `status=queued`. Re-measured:
#      job PICKUP lag (started_at - created_at), N=84:   max 10s,  over 60s: ZERO
#      job CREATION lag (job.created_at - run.created_at):        max 402 min
#      the 402 min is INTRA-MATRIX, in `mutation ratchet` (51 legs, spread 401.9 min)
#      control: `test` (5 legs, no max-parallel) has spread 0.0 min
#    and `mutants-nightly-advisory` declares `max-parallel: 8`. 51 legs at 8 concurrent
#    is 6.4 waves; the 402 minutes IS the configured cap working correctly. It is
#    self-inflicted scheduling, not GitHub withholding runners. So this repository has NO
#    observed instance of the phenomenon a queue-wait alarm would exist to catch.
#
# WHY NOT RE-POINT IT AT JOB-CREATION LAG. That variable does move, but on THIS corpus it
# moves because of our own `max-parallel`, so an alarm keyed on it would fire on every
# nightly mutation run forever. A permanently-red alarm is a muted alarm — the same reason
# the M1 expectation is capped at what GitHub actually delivers rather than at nominal
# cron rate. Separating a configured cap from a genuine account-level ceiling needs a model
# of each lane's configured concurrency and per-leg duration, and there is no observed
# positive to validate such a model against. That is a research task, not a threshold.
#
# 4. AND THE STRONGEST REASON, FOUND LAST: THE FIELDS CANNOT EXPRESS THE QUANTITY.
#    This is not "no positive was observed" — the data source does not carry enqueue time
#    at all, so no threshold on it could ever have worked.
#      attempt-1 runs, `run_started_at - created_at`: EXACTLY 0.0 on all 44,123 of them
#        (35,070 sparq + 9,053 registry). Not approximately zero -- identically zero.
#      re-run attempts, `run_started_at - attempt.created_at`, N=67 (58 sparq + 9 registry):
#        NEGATIVE on every single one -- sparq {-1: 41, -2: 15, -4: 1, -5: 1},
#        registry {-1: 7, -2: 2}. A queue wait cannot be negative.
#    A negative value means the attempt record's `created_at` is stamped at or AFTER the
#    run starts, not when it is enqueued; and an exactly-zero run-level difference across
#    44,123 runs is what you see when the two fields are set together rather than
#    measuring an interval between two events. So BOTH the run-level pair and the
#    per-attempt pair are unable to express "how long did this wait to be picked up".
#    The all-zero corpus in point 2 is therefore not evidence that no queueing happened;
#    it is evidence that these fields do not report queueing.
#
# WHAT WOULD CHANGE THIS -- and it is NOT a better threshold. Do not re-derive a detector
# from `created_at`/`run_started_at` on either the run or the attempt: point 4 shows they
# cannot carry the signal at any threshold. It would take a DIFFERENT data source that
# actually timestamps enqueue -- a webhook capturing `workflow_job` `queued` -> `in_progress`
# transitions, or self-reported timing from inside the job -- plus at least one observed
# positive from it before a threshold means anything.

# --- M3: execution overrun ---------------------------------------------------------
# Threshold = max(EXEC_OVERRUN_MULTIPLE x p90(completed durations for this workflow+event),
#                 EXEC_FLOOR_SECONDS).
# VALIDATED by a leave-one-out sweep over the event-filtered completed-run corpus — each
# run scored against a p90 computed from the OTHER runs in its own (workflow, event) cell:
#   sparq-org/sparq                N=8,676 runs / 119 cells
#   jeswr/agent-account-registry   N=2,063 runs /  30 cells
# Result: at K = 1.25 .. 2.0 the ONLY sparq run that fires is the genuine 2026-07-27
# nightly ci.yml outlier (17.33h against an 8.35h p90 = 2.08x), and ZERO registry runs
# fire. At K = 2.5 and above that real event is MISSED. So 2.0 is the LARGEST multiple
# that still detects the known positive, and it costs nothing: every other one of the
# 10,739 sampled runs stays silent.
# Instrument validated against a known answer before the result was trusted: injecting one
# synthetic run at 2.5x a cell's p90 fires at K=2.0 on both repos, so a zero result is a
# real zero and not a broken sweep.
EXEC_OVERRUN_MULTIPLE = 2.0
# Floor. Applies when a lane has NO usable completed baseline — including the case that
# matters most: a lane whose runs never complete (see (c) in the header). 6h is GitHub's
# hosted-runner per-job ceiling, so a RUN alive past 6h has necessarily outlived any single
# job's maximum possible life and is either a long matrix or a leak; either way it is worth
# a line. Never `continue` on a missing baseline — that is the fail-open hole.
EXEC_FLOOR_SECONDS = 6 * 60 * 60
# Completed runs sampled per (workflow, event) to build the baseline.
BASELINE_SAMPLE = 100
# A baseline needs at least this many completed samples to be trusted; below it, fall back
# to the floor (never skip).
BASELINE_MIN_N = 5

# `actions/runs` hard-caps pagination at 1000 results regardless of `--paginate`
# (MEASURED: total_count=21672, --paginate returned exactly 1000). Live-state queries are
# filtered by `status=` and are far below that, but the cap is asserted rather than assumed.
RUNS_PAGE_CAP = 10

# The trigger set that makes a lane INVISIBLE to PR review / merge queues. Identical to
# `cron_lane_liveness.INVISIBLE_TRIGGERS` (#4368) on purpose — see the coordination note.
INVISIBLE_TRIGGERS = frozenset({"schedule", "workflow_dispatch"})


# ---------------------------------------------------------------------------------
# cron expansion
# ---------------------------------------------------------------------------------
def _expand_field(spec: str, lo: int, hi: int) -> set[int]:
    """Expand one cron field to the set of values it matches."""
    if not spec:
        raise CronError("empty field")
    out: set[int] = set()
    for part in spec.split(","):
        if not part:
            raise CronError(f"empty list element in {spec!r}")
        step = 1
        if "/" in part:
            part, raw = part.split("/", 1)
            if not raw.isdigit():
                raise CronError(f"bad step in {spec!r}")
            step = int(raw)
            if step <= 0:
                raise CronError(f"non-positive step in {spec!r}")
        if part in ("*", "?"):
            a, b = lo, hi
        elif "-" in part:
            lhs, rhs = part.split("-", 1)
            if not (lhs.isdigit() and rhs.isdigit()):
                raise CronError(f"bad range in {spec!r}")
            a, b = int(lhs), int(rhs)
            if a > b:
                raise CronError(f"inverted range in {spec!r}")
        else:
            if not part.isdigit():
                raise CronError(f"not a number: {part!r}")
            a = b = int(part)
        out |= set(range(a, b + 1, step))
    out = {v for v in out if lo <= v <= hi}
    if not out:
        raise CronError(f"field matches nothing: {spec!r}")
    return out


def expected_firings(expr: str, start: dt.datetime, end: dt.datetime) -> int:
    """How many times `expr` fires in (start, end]. Minute-resolution walk.

    POSIX day semantics: when BOTH day-of-month and day-of-week are restricted the match
    is their UNION, not their intersection.
    """
    if not isinstance(expr, str):
        raise CronError(f"not a string: {expr!r}")
    fields = expr.split()
    if len(fields) != 5:
        raise CronError(f"expected 5 fields, got {len(fields)}: {expr!r}")
    minute, hour, dom, month, dow = fields
    mins = _expand_field(minute, 0, 59)
    hours = _expand_field(hour, 0, 23)
    doms = _expand_field(dom, 1, 31)
    months = _expand_field(month, 1, 12)
    dows = {d % 7 for d in _expand_field(dow, 0, 7)}
    dom_wild = dom.strip() in ("*", "?")
    dow_wild = dow.strip() in ("*", "?")
    if (end - start) > dt.timedelta(days=400):
        raise CronError("window too wide to expand")

    t = start.replace(second=0, microsecond=0) + dt.timedelta(minutes=1)
    n = 0
    while t <= end:
        if t.minute in mins and t.hour in hours and t.month in months:
            weekday = (t.weekday() + 1) % 7  # cron: Sunday == 0
            if dom_wild and dow_wild:
                day_ok = True
            elif dom_wild:
                day_ok = weekday in dows
            elif dow_wild:
                day_ok = t.day in doms
            else:
                day_ok = (t.day in doms) or (weekday in dows)
            if day_ok:
                n += 1
        t += dt.timedelta(minutes=1)
    return n


# ---------------------------------------------------------------------------------
# workflow scope
# ---------------------------------------------------------------------------------
def workflow_triggers(text: str) -> dict:
    """-> the parsed `on:` mapping. `on` is YAML 1.1 `true`, hence the two-key lookup."""
    if yaml is None:  # pragma: no cover
        raise AlarmError(f"PyYAML unavailable: {_YAML_IMPORT_ERROR}")
    try:
        doc = yaml.safe_load(text)
    except yaml.YAMLError as exc:
        raise AlarmError(f"unparseable workflow YAML: {exc}") from exc
    if not isinstance(doc, dict):
        raise AlarmError("workflow YAML is not a mapping")
    on = doc.get(True, doc.get("on"))
    return on if isinstance(on, dict) else {}


def m1_scope(on: dict) -> tuple[bool, list[str], bool]:
    """-> (in_m1_scope, cron expressions, is_cron_only).

    IN SCOPE iff the workflow has a `schedule:` AND at least one trigger OUTSIDE
    {schedule, workflow_dispatch}. That is the exact set complement of #4368's
    `cron_lane_liveness.py` scope, so the two detectors partition the scheduled
    workflows and can never both raise on the same lane.
    """
    if "schedule" not in on:
        return False, [], False
    sched = on.get("schedule") or []
    crons = [s.get("cron") for s in sched
             if isinstance(s, dict) and isinstance(s.get("cron"), str)]
    cron_only = set(on) <= INVISIBLE_TRIGGERS
    return (bool(crons) and not cron_only), crons, cron_only


# ---------------------------------------------------------------------------------
# detectors — pure functions over already-fetched state
# ---------------------------------------------------------------------------------
def sample_truncated(lane: dict, start: dt.datetime) -> bool:
    """Does this lane's newest-100 run sample fail to reach back to `start`?

    COVERAGE IS WINDOW-RELATIVE, so it cannot be decided once at fetch time and then
    shared: M1 runs on `--window-hours` and M4 on its own fixed `M4_WINDOW_HOURS`, and a
    sample that reaches back over one need not reach back over the other. Deciding it once
    against M1's window let `--window-hours 6` call M4's 24h sample complete when it was
    not — an UNDERCOUNT, which on M4's ratio manufactures a fiction out of a healthy lane —
    and, in the other direction, let a window LONGER than 24h mute M4 on a sample that
    covers M4's window perfectly well. Each detector asks about its own window instead.

    An explicit `truncated` flag on the lane still wins, so a hermetic `--state-file`
    fixture can express the condition without carrying 100 timestamps.
    """
    if lane.get("truncated"):
        return True
    times = lane.get("schedule_run_times") or []
    # A page that came back SHORT is the lane's whole history: complete, however old it is.
    return len(times) >= 100 and min(times) > start


def find_cron_deficits(lanes: list[dict], now: dt.datetime,
                       window_hours: float = CRON_WINDOW_HOURS) -> tuple[list[dict], dict]:
    """M1. `lanes` = [{workflow, crons, cron_only, in_scope, state, schedule_run_times}].

    The census counts EVERY state exit, not just the alarming one: a per-state population
    is the only shape in which a MISSING edge is visible at all.
    """
    start = now - dt.timedelta(hours=window_hours)
    # The window ENDS a grace period before `now`, so a firing whose run may not have been
    # created yet is not counted as missing. Without this the tick that lands on a lane's
    # own cron minute manufactures a deficit for that lane, every day.
    end = now - dt.timedelta(minutes=CRON_GRACE_MINUTES)
    # What GitHub will actually deliver in a window of this length. Nominal cron rate is
    # not an expectation for sub-hourly lanes — see the constant's note.
    ceiling = int(CRON_MAX_CREDIBLE_FIRINGS_PER_HOUR * window_hours)
    findings: list[dict] = []
    census: dict[str, int] = {}

    def bump(state: str) -> None:
        census[state] = census.get(state, 0) + 1

    for lane in lanes:
        if not lane.get("in_scope"):
            bump("cron-only (watched by #4368)" if lane.get("cron_only")
                 else "not-scheduled")
            continue
        if lane.get("state") != "active":
            bump("disabled")
            continue
        # NEW-LANE WINDOW. A lane that did not EXIST for the whole window looks identical
        # to a lane that stopped firing: out of run times alone the detector cannot tell
        # "did not fire" from "did not exist yet", so a `*/10` lane added two hours ago
        # reads ratio 0.08 and alarms for the next ~14 hours. The workflow's own
        # `created_at` (carried on the `actions/workflows` listing) is the signal that
        # resolves it.
        # PRORATING THE EXPECTATION WAS TRIED AND REJECTED: scaling the cap by the lane's
        # observed lifetime gives a young lane an expectation of ~2, and at a floor of
        # 0.60 a single undelivered firing (1 of 2) still alarms. A prorated expectation
        # is not a smaller measurement, it is noise — the cap it derives from is itself a
        # RATE measured over a full 24h, and GitHub's delivery is bursty at short scales.
        # (MEASURED: the prorated form still alarmed on a 2h-old lane at the registry's
        # cap of 30, and was quiet at sparq's 12, which is a threshold that depends on
        # which repo it is deployed to — not a property of the lane.)
        # So a lane younger than the window is COUNTED and skipped, never guessed at; it
        # becomes watchable one window after it appears. The gap is deliberate, bounded,
        # and visible in the census rather than silent.
        created = lane.get("created_at")
        if created:
            born = created if isinstance(created, dt.datetime) else _ts(created)
            if born > start:
                bump("lane-too-new-for-an-expectation")
                continue
        try:
            nominal = sum(expected_firings(c, start, end) for c in lane["crons"])
        except CronError:
            bump("cron-unparseable")
            continue
        expected = min(nominal, ceiling)
        if expected < CRON_MIN_EXPECTED:
            bump("expectation-below-floor")
            continue
        actual = sum(1 for t in lane["schedule_run_times"] if start < t <= end)
        # No clamp on `actual`: an over-delivering lane gives a ratio above 1.0, which is
        # >= the floor and therefore healthy either way. A `min(actual, expected)` here
        # was removed after its own mutant SURVIVED — it changed no observable output, so
        # it was dead code, and an untestable guard is worse than no guard.
        ratio = actual / expected
        if ratio < CRON_DELIVERY_FLOOR:
            bump("firing-deficit")
            findings.append({
                "mode": "M1-cron-firing-deficit",
                "workflow": lane["workflow"],
                "expected": expected,
                "nominal": nominal,
                "actual": actual,
                "ratio": round(ratio, 3),
                "window_hours": window_hours,
                "crons": lane["crons"],
            })
        else:
            bump("delivering")
    return findings, census


def find_cadence_fidelity_gaps(lanes: list[dict], now: dt.datetime,
                               window_hours: float = M4_WINDOW_HOURS
                               ) -> tuple[list[dict], dict]:
    """M4 (#4802). Lanes whose DECLARED cron rate is a fiction on this repo.

    Scope is EVERY scheduled lane, cron-only included — the population #4802 measured is
    mostly cron-only (`promote-on-approval`, `rearm-sweeper`, `retriage`), which M1
    structurally cannot see. That is safe because M4 answers a question about the workflow
    FILE, not about the lane's health, and because it emits AT MOST ONE finding for the
    whole repo: 13 identical per-lane issues is the shape that gets an alarm muted.

    Denominator is the NOMINAL (uncapped) declaration — the whole point is the distance
    between what the file says and what arrives. M1's capped expectation is the opposite
    measurement and neither substitutes for the other.
    """
    start = now - dt.timedelta(hours=window_hours)
    end = now - dt.timedelta(minutes=CRON_GRACE_MINUTES)
    ceiling = int(CRON_MAX_CREDIBLE_FIRINGS_PER_HOUR * window_hours)
    census: dict[str, int] = {}
    divergent: list[dict] = []

    def bump(state: str) -> None:
        census[state] = census.get(state, 0) + 1

    for lane in lanes:
        if not lane.get("crons"):
            bump("not-scheduled")
            continue
        if lane.get("state") != "active":
            bump("disabled")
            continue
        # The run sample is the newest 100. If it does not reach back to THIS detector's
        # window start the count is an UNDERCOUNT, which on this ratio manufactures a
        # fictional lane out of a healthy one. Fail-safe quiet, and counted so the skip is
        # visible. Evaluated against M4's own `start`, never M1's — see `sample_truncated`.
        if sample_truncated(lane, start):
            bump("sample-truncated")
            continue
        created = lane.get("created_at")
        if created:
            born = created if isinstance(created, dt.datetime) else _ts(created)
            if born > start:
                bump("lane-too-new-for-an-expectation")
                continue
        try:
            nominal = sum(expected_firings(c, start, end) for c in lane["crons"])
        except CronError:
            bump("cron-unparseable")
            continue
        if nominal <= ceiling:
            bump("declaration-within-the-measured-ceiling")
            continue
        actual = sum(1 for t in lane["schedule_run_times"] if start < t <= end)
        fidelity = actual / nominal
        if fidelity < M4_FIDELITY_FLOOR:
            bump("declaration-unachievable")
            divergent.append({
                "workflow": lane["workflow"],
                "nominal": nominal,
                "actual": actual,
                "fidelity": round(fidelity, 3),
                "crons": lane["crons"],
            })
        else:
            bump("declaration-honoured")

    findings: list[dict] = []
    if divergent:
        # ONE finding for the repo. `workflow` is deliberately not a lane: nothing
        # downstream should be able to read this as a per-lane health verdict.
        findings.append({
            "mode": "M4-declared-cadence-unachievable",
            "workflow": f"(repo-level: {len(divergent)} lane declarations)",
            "lanes": sorted(divergent, key=lambda d: (d["fidelity"], d["workflow"])),
            "window_hours": window_hours,
            "floor": M4_FIDELITY_FLOOR,
        })
    return findings, census


def find_execution_overruns(live_runs: list[dict], baselines: dict, now: dt.datetime,
                            multiple: float = EXEC_OVERRUN_MULTIPLE,
                            floor: float = EXEC_FLOOR_SECONDS
                            ) -> tuple[list[dict], dict]:
    """M3. A run `in_progress` past its own lane's measured duration.

    DETECTION VARIABLE is the live age of an in-flight run — deliberately NOT any
    statistic over completed runs, which structurally excludes the hangs this exists to
    catch (header note (a)).

    `baselines` maps (workflow_key, event) -> {"p90": seconds, "n": int}. A missing or
    under-sampled baseline falls back to `floor` and is REPORTED as such; it must never
    cause a `continue`, or a lane whose runs never complete would be unwatchable by its
    own detector (header note (c)).
    """
    findings: list[dict] = []
    census: dict[str, int] = {}

    def bump(state: str) -> None:
        census[state] = census.get(state, 0) + 1

    for run in live_runs:
        if run.get("status") != "in_progress":
            continue
        started = run.get("run_started_at") or run.get("created_at")
        if not started:
            bump("in-progress-no-timestamp")
            continue
        age = (now - _ts(started)).total_seconds()
        key = (run.get("path") or run.get("name") or "?", run.get("event"))
        base = baselines.get(key) or {}
        n = int(base.get("n") or 0)
        p90 = base.get("p90")
        if p90 is None or n < BASELINE_MIN_N:
            threshold = floor
            basis = f"floor (no usable baseline; n={n})"
        else:
            threshold = max(multiple * float(p90), floor)
            basis = f"max({multiple}x p90={int(p90)}s over n={n}, floor)"
        if age > threshold:
            bump("execution-overrun")
            findings.append({
                "mode": "M3-execution-overrun",
                "workflow": run.get("name") or run.get("path") or "?",
                "run_id": run.get("id"),
                "event": run.get("event"),
                "age_seconds": int(age),
                "threshold_seconds": int(threshold),
                "basis": basis,
                "head_branch": run.get("head_branch"),
            })
        else:
            bump("in-progress-within-threshold")
    return findings, census


def _ts(value: str) -> dt.datetime:
    try:
        return dt.datetime.fromisoformat(str(value).replace("Z", "+00:00"))
    except ValueError as exc:
        raise AlarmError(f"unparseable timestamp {value!r}: {exc}") from exc


def percentile(values: list[float], q: float) -> float:
    ordered = sorted(values)
    idx = min(len(ordered) - 1, int(round(q * (len(ordered) - 1))))
    return ordered[idx]


# ---------------------------------------------------------------------------------
# gh I/O
# ---------------------------------------------------------------------------------
def _gh(args: list[str], *, parse: bool = True):
    try:
        out = subprocess.run(
            ["gh", *args], check=True, capture_output=True, text=True, timeout=180
        )
    except (subprocess.CalledProcessError, subprocess.TimeoutExpired, OSError) as exc:
        detail = getattr(exc, "stderr", "") or str(exc)
        raise AlarmError(f"gh {' '.join(args[:2])} failed: {detail}") from exc
    if not parse:
        return out.stdout
    try:
        return json.loads(out.stdout or "null")
    except json.JSONDecodeError as exc:
        raise AlarmError(f"gh {' '.join(args[:2])}: unparseable JSON: {exc}") from exc


def _paged_runs(repo: str, query: str) -> list[dict]:
    """Walk `actions/runs` pages. Terminates on a SHORT page, never on
    `len(runs) >= total_count`: a total_count read before a new run started is an
    UNDERCOUNT and stopping on it would truncate exactly when the list is growing."""
    runs: list[dict] = []
    page = 1
    while page <= RUNS_PAGE_CAP:
        payload = _gh(["api", f"repos/{repo}/actions/runs?{query}&per_page=100&page={page}"])
        if not isinstance(payload, dict):
            raise AlarmError("workflow-run listing is not an object")
        batch = payload.get("workflow_runs")
        if batch is None:
            raise AlarmError("workflow-run listing carries no workflow_runs")
        runs.extend(batch)
        if len(batch) < 100:
            return runs
        page += 1
    raise AlarmError(f"workflow-run listing exceeded {RUNS_PAGE_CAP} pages for {query!r}")


def fetch_live_runs(repo: str) -> list[dict]:
    """Every run currently `in_progress`. This is the live-state read M3 keys on; it is
    NOT derived from completed work (header note (a)).

    `status=queued` is deliberately NOT fetched — see the QUEUE WAIT note near the top.
    Nothing reads it any more, and fetching a population no detector consumes would be
    the shape of coverage this file exists to avoid.
    """
    return _paged_runs(repo, "status=in_progress")


def fetch_baseline(repo: str, workflow_path: str, event: str) -> dict:
    """p90 of completed-run DURATION for one (workflow, event).

    Completed runs are the correct population for "how long is normal" and the WRONG
    population for "is something stuck" — see header notes (a) and (b).
    """
    wf = workflow_path.split("/")[-1]
    payload = _gh(["api", f"repos/{repo}/actions/workflows/{wf}/runs"
                          f"?status=completed&event={event}&per_page={BASELINE_SAMPLE}"])
    if not isinstance(payload, dict) or payload.get("workflow_runs") is None:
        raise AlarmError(f"{wf}: baseline response carries no workflow_runs")
    durations = []
    for run in payload["workflow_runs"]:
        started, updated = run.get("run_started_at"), run.get("updated_at")
        if not (started and updated):
            continue
        delta = (_ts(updated) - _ts(started)).total_seconds()
        if delta >= 0:
            durations.append(delta)
    if not durations:
        return {"p90": None, "n": 0}
    return {"p90": percentile(durations, 0.90), "n": len(durations)}


def fetch_lanes(repo: str, workflows_dir: Path, window_hours: float,
                now: dt.datetime) -> list[dict]:
    """Build the M1 lane list from the workflow files plus each lane's schedule runs."""
    if not workflows_dir.is_dir():
        raise AlarmError(f"no workflows directory at {workflows_dir}")
    listing = _gh(["api", f"repos/{repo}/actions/workflows?per_page=100"])
    if not isinstance(listing, dict) or listing.get("workflows") is None:
        raise AlarmError("workflow listing carries no workflows")
    state_by_path = {w["path"]: w.get("state") for w in listing["workflows"]}
    # The lane's own birth date, which is the ONLY thing that separates "did not fire"
    # from "did not exist yet" — see the NEW-LANE WINDOW note in find_cron_deficits.
    created_by_path = {w["path"]: w.get("created_at") for w in listing["workflows"]}

    lanes: list[dict] = []
    start = now - dt.timedelta(hours=window_hours)
    for path in sorted(workflows_dir.glob("*.yml")) + sorted(workflows_dir.glob("*.yaml")):
        rel = f".github/workflows/{path.name}"
        on = workflow_triggers(path.read_text(encoding="utf-8"))
        in_scope, crons, cron_only = m1_scope(on)
        lane = {"workflow": rel, "crons": crons, "cron_only": cron_only,
                "in_scope": in_scope, "state": state_by_path.get(rel, "active"),
                "created_at": created_by_path.get(rel),
                "schedule_run_times": []}
        # Run times are fetched for EVERY active scheduled lane, not just M1's complement:
        # M4's population is all of them, and the lanes #4802 measured are mostly cron-only
        # (`promote-on-approval`, `rearm-sweeper`, `retriage`). Narrowing this to `in_scope`
        # would leave M4 reading `actual = 0` for 20 of the 33 scheduled lanes and calling
        # every one of them a fiction.
        if crons and lane["state"] == "active":
            payload = _gh(["api", f"repos/{repo}/actions/workflows/{path.name}/runs"
                                  f"?event=schedule&per_page=100"])
            if not isinstance(payload, dict) or payload.get("workflow_runs") is None:
                raise AlarmError(f"{path.name}: schedule-run response carries no workflow_runs")
            times = [_ts(r["created_at"]) for r in payload["workflow_runs"]
                     if r.get("created_at")]
            lane["schedule_run_times"] = times
            # COVERAGE GUARD: the sample is the newest 100 runs. If the OLDEST sampled run
            # is newer than the window start, the count is TRUNCATED and would manufacture
            # a phantom deficit. Treat as indeterminate (fail-safe quiet), never as 0.
            # Decided here for M1 ONLY, because `in_scope` is M1's admission and `start`
            # is M1's `--window-hours`. M4 re-asks `sample_truncated` against its own 24h
            # window; no truncation flag is written here, or M1's window would decide M4's.
            if sample_truncated(lane, start):
                lane["in_scope"] = False
        lanes.append(lane)
    return lanes


# ---------------------------------------------------------------------------------
# reporting
# ---------------------------------------------------------------------------------
def render_issue_body(repo: str, findings: list[dict], census: dict[str, dict],
                      now: dt.datetime) -> str:
    lines = [
        "> 🤖 **SPARQ agent** — automated CI execution-latency alarm "
        "(`scripts/ci_execution_latency_alarm.py`, bead sq-1lc4i). @jeswr runs multiple "
        "agents on this account; this issue was filed by CI automation.",
        "",
        f"CI execution latency in `{repo}` breached a measured threshold at "
        f"`{now:%Y-%m-%dT%H:%M:%SZ}`.",
        "",
    ]
    by_mode: dict[str, list[dict]] = {}
    for f in findings:
        by_mode.setdefault(f["mode"], []).append(f)
    for mode in sorted(by_mode):
        lines += [f"### {mode}", ""]
        for f in by_mode[mode]:
            if mode.startswith("M1"):
                lines.append(
                    f"- `{f['workflow']}` fired **{f['actual']}** of an expected "
                    f"**{f['expected']}** in the last {f['window_hours']:g}h "
                    f"(ratio {f['ratio']}, floor {CRON_DELIVERY_FLOOR}); "
                    f"cron `{' | '.join(f['crons'])}`")
            elif mode.startswith("M4"):
                lines += [
                    f"**{len(f['lanes'])} lane(s)** declare a cadence this repo does not "
                    f"deliver: achieved below {f['floor']} of the DECLARED firing count "
                    f"over {f['window_hours']:g}h. This is chronic, not an incident — it "
                    "does not red this run. The declaration is the defect: correct each "
                    "`cron:` to a rate GitHub actually delivers here, or every loop sized "
                    "on it is sized on a fiction.",
                    "",
                ]
                for lane in f["lanes"]:
                    lines.append(
                        f"- `{lane['workflow']}` declares **{lane['nominal']}** fires/"
                        f"{f['window_hours']:g}h, achieved **{lane['actual']}** "
                        f"(fidelity {lane['fidelity']}); "
                        f"cron `{' | '.join(lane['crons'])}`")
            else:
                lines.append(
                    f"- `{f['workflow']}` run {f['run_id']} ({f['event']}) has been "
                    f"**in progress {f['age_seconds']//60} min**, threshold "
                    f"{f['threshold_seconds']//60} min — basis: {f['basis']}")
        lines.append("")
    lines += ["**Census of every state exit** (emitted on every run, including the "
              "all-clear — a silent alarm is indistinguishable from a healthy system):", ""]
    for mode in sorted(census):
        lines += [f"`{mode}`", "", "```"]
        lines += [f"{k}: {v}" for k, v in sorted(census[mode].items())]
        lines += ["```", ""]
    lines.append(f"<!-- {KEY_PREFIX}: {repo} -->")
    return "\n".join(lines)


def file_issue(repo: str, body: str, count: int) -> None:
    """One open issue, keyed by the body marker. Exact-match the marker rather than
    using `--search`: a key containing punctuation tokenises unreliably, which could MISS
    an existing issue and mint a duplicate."""
    marker = f"<!-- {KEY_PREFIX}: {repo} -->"
    existing = _gh(["api", "--paginate", "--slurp",
                    f"repos/{repo}/issues?state=open&labels={BASE_LABELS[0]}&per_page=100"])
    if not isinstance(existing, list):
        raise AlarmError("issue listing is not a list")
    for page in existing:
        for issue in page if isinstance(page, list) else []:
            if isinstance(issue, dict) and marker in str(issue.get("body") or ""):
                _gh(["api", "-X", "PATCH", f"repos/{repo}/issues/{issue['number']}",
                     "-f", f"body={body}"])
                print(f"::notice::updated existing ci-latency-alarm issue "
                      f"#{issue['number']}")
                return
    title = f"ci-latency-alarm: {count} CI execution-latency breach(es) in {repo}"
    _gh(["api", "-X", "POST", f"repos/{repo}/issues",
         "-f", f"title={title}", "-f", f"body={body}",
         *[arg for label in BASE_LABELS for arg in ("-f", f"labels[]={label}")]])
    print("::notice::filed a new ci-latency-alarm issue")


# ---------------------------------------------------------------------------------
# orchestration
# ---------------------------------------------------------------------------------
def run(args: argparse.Namespace) -> int:
    now = _ts(args.now) if args.now else dt.datetime.now(dt.timezone.utc)

    if args.state_file:
        state = json.loads(Path(args.state_file).read_text(encoding="utf-8"))
        repo = state.get("repo") or args.repo or "hermetic/fixture"
        lanes = state.get("lanes", [])
        for lane in lanes:
            lane["schedule_run_times"] = [_ts(t) for t in lane.get("schedule_run_times", [])]
        live = state.get("live_runs", [])
        baselines = {tuple(k.split(BASELINE_KEY_SEP, 1)): v
                     for k, v in (state.get("baselines") or {}).items()}
    else:
        repo = args.repo or os.environ.get("GITHUB_REPOSITORY") or ""
        if "/" not in repo:
            raise AlarmError(f"no usable repo slug: {repo!r}")
        root = Path(__file__).resolve().parents[1]
        lanes = fetch_lanes(repo, root / ".github" / "workflows",
                            args.window_hours, now)
        live = fetch_live_runs(repo)
        baselines = {}
        for r in live:
            if r.get("status") != "in_progress":
                continue
            key = (r.get("path") or r.get("name") or "?", r.get("event"))
            if key not in baselines and r.get("path"):
                baselines[key] = fetch_baseline(repo, r["path"], r.get("event") or "push")

    # EMPTY SCAN SET IS FAIL-LOUD. A detector watching nothing is not a healthy repo; it
    # is a broken detector, and the two must never look alike. This is the 100% question
    # applied to M1: if every workflow were cron-only, M1's population would be empty and
    # a `return 0` here would report health while watching zero lanes.
    if not lanes:
        raise AlarmError("empty scan set: no workflows discovered")
    if not any(lane.get("in_scope") for lane in lanes):
        raise AlarmError(
            "empty M1 scan set: no workflow has a `schedule:` plus a trigger outside "
            f"{sorted(INVISIBLE_TRIGGERS)} — either the repo changed shape or the scope "
            "predicate broke. Refusing to report health over an empty population.")

    m1, c1 = find_cron_deficits(lanes, now, args.window_hours)
    m3, c3 = find_execution_overruns(live, baselines, now)
    # M4 runs on M4_WINDOW_HOURS, NOT on `--window-hours`. Its floor and its admission
    # ceiling are validated against a 24h census only; handing it a sub-day window would
    # make it judge the exact short-scale burstiness the header says it must not see, and
    # a longer one would leave it reading a floor no measurement supports.
    m4, c4 = find_cadence_fidelity_gaps(lanes, now)
    # INCIDENT vs CHRONIC. Only the incident modes set the exit code; M4 is true on a good
    # day and would leave this hourly lane permanently red, muting M1 and M3 with it. See
    # the M4 section of the header.
    incidents = m1 + m3
    findings = incidents + m4
    census = {"M1-cron-firing-deficit": c1, "M3-execution-overrun": c3,
              "M4-declared-cadence-unachievable": c4}

    print(f"ci-latency census for {repo} at {now:%Y-%m-%dT%H:%M:%SZ} "
          f"({len(lanes)} workflows, {len(live)} live runs):")
    for mode in sorted(census):
        print(f"  {mode}:")
        for state, count in sorted(census[mode].items()):
            print(f"    {state}: {count}")
    for f in findings:
        print(f"  BREACH {f['mode']} {f['workflow']}")

    if not findings:
        print("::notice::CI execution latency is within every measured threshold")
        return 0

    body = render_issue_body(repo, findings, census, now)
    if args.dry_run:
        print(body)
    else:
        file_issue(repo, body, len(findings))
    if not incidents:
        print("::notice::CI execution latency is within every measured threshold; a "
              "chronic cadence-fidelity finding (M4) was filed into the deduped issue")
        return 0
    print(f"::error::{len(incidents)} CI execution-latency breach(es) in {repo} — "
          "runner pickup, cron delivery, or execution time is outside its measured band")
    return 1


# ---------------------------------------------------------------------------------
# hermetic self-test — runs as the FIRST step of every alarm run, so a broken detector
# reds before it is trusted to watch anything.
# ---------------------------------------------------------------------------------
def capped_expectation(window_hours: float = CRON_WINDOW_HOURS) -> int:
    """The most a lane can be expected to deliver in a window — exposed so tests assert
    against the DERIVED value rather than re-hard-coding it (a test that pins 12 would go
    green-but-wrong if the cap changed)."""
    return int(CRON_MAX_CREDIBLE_FIRINGS_PER_HOUR * window_hours)


def _lane(workflow="a.yml", crons=("*/10 * * * *",), cron_only=False,
          in_scope=True, state="active", fires=0, now=None, spacing_min=30,
          created_hours_ago=None):
    """`fires` runs, all placed strictly INSIDE the counted window (older than the grace
    cut-off, newer than the window start), so a fixture's count is exactly its `fires`.

    `created_hours_ago` gives the lane a birth date, for the NEW-LANE WINDOW. `None`
    means an established lane (no `created_at`), which must behave exactly as before.
    """
    now = now or dt.datetime(2026, 7, 28, 12, 0, tzinfo=dt.timezone.utc)
    first = now - dt.timedelta(minutes=CRON_GRACE_MINUTES + 1)
    times = [first - dt.timedelta(minutes=spacing_min * i) for i in range(fires)]
    lane = {"workflow": workflow, "crons": list(crons), "cron_only": cron_only,
            "in_scope": in_scope, "state": state, "schedule_run_times": times}
    if created_hours_ago is not None:
        lane["created_at"] = (now - dt.timedelta(hours=created_hours_ago)
                              ).strftime("%Y-%m-%dT%H:%M:%SZ")
    return lane


def _run_obj(status="in_progress", event="schedule", path=".github/workflows/a.yml",
             age_min=10, now=None, name="A", attempt=1, created_age_min=None):
    """The REAL shape of an `actions/runs` element, not a minimal hand-built dict.

    A narrow fixture is a vacuity generator. `started = run.get("updated_at") or
    run_started_at` SURVIVED the first mutation battery for exactly one reason: no fixture
    carried `updated_at`, while EVERY live run does — and on a live run `updated_at` is
    bumped continuously, so that mutant collapses M3's age to ~0 and the mode never fires
    again. The keys and their relationships below are taken from a real payload
    (jeswr/agent-account-registry run 30318886362).

    `created_age_min` reproduces the RE-RUN shape, where the run-level `created_at` is
    FROZEN at attempt 1 while `run_started_at` tracks the live attempt — M3 must key on
    the latter. See the QUEUE WAIT note near the top of this file.
    """
    now = now or dt.datetime(2026, 7, 28, 12, 0, tzinfo=dt.timezone.utc)

    def stamp(minutes: float) -> str:
        return (now - dt.timedelta(minutes=minutes)).strftime("%Y-%m-%dT%H:%M:%SZ")

    obj = {
        "id": 1, "name": name, "path": path, "event": event, "status": status,
        "conclusion": None, "run_attempt": attempt, "workflow_id": 99, "run_number": 7,
        "display_title": name, "head_branch": "main", "head_sha": "0" * 40,
        "created_at": stamp(created_age_min if created_age_min is not None else age_min),
        "run_started_at": stamp(age_min),
        # A LIVE run's `updated_at` tracks the PRESENT moment, which is what makes it a
        # catastrophic substitute for `run_started_at` in M3.
        "updated_at": stamp(0) if status != "completed" else stamp(age_min),
        "html_url": "https://example.invalid/run/1",
    }
    return obj


def _self_test() -> int:  # noqa: C901 - a flat table of named assertions reads best flat
    failures: list[str] = []

    def check(name: str, condition: bool) -> None:
        if not condition:
            failures.append(name)

    NOW = dt.datetime(2026, 7, 28, 12, 0, tzinfo=dt.timezone.utc)
    H6 = NOW - dt.timedelta(hours=6)

    # --- THE CONSTANTS, pinned against LITERALS that do not derive from them ----------
    # Every threshold in this file was previously exercised only through fixtures
    # COMPUTED FROM the constant, so the fixture rescaled with the mutant and the mutant
    # survived. MEASURED instance: `CRON_DELIVERY_FLOOR 0.60 -> 0.05` survived the entire
    # suite in the sibling deployment because `at_floor = ceil(CAP * FLOOR)` moved with
    # it. A constant is only pinned by a value that is written down independently.
    check("CRON_WINDOW_HOURS is 24h — the value the rejected 6h trial was replaced by",
          CRON_WINDOW_HOURS == 24.0)
    check("CRON_DELIVERY_FLOOR is inside its validated band (worst healthy lane 0.75)",
          0.50 <= CRON_DELIVERY_FLOOR <= 0.70)
    check("CRON_MAX_CREDIBLE_FIRINGS_PER_HOUR matches the measured sparq ceiling",
          CRON_MAX_CREDIBLE_FIRINGS_PER_HOUR == 0.5)
    check("CRON_GRACE_MINUTES is 15", CRON_GRACE_MINUTES == 15)
    check("EXEC_OVERRUN_MULTIPLE is inside the band that still catches the 2.08x outlier",
          1.25 <= EXEC_OVERRUN_MULTIPLE <= 2.0)
    check("EXEC_FLOOR_SECONDS is GitHub's 6h hosted-runner job ceiling",
          EXEC_FLOOR_SECONDS == 6 * 60 * 60)
    check("BASELINE_MIN_N is 5", BASELINE_MIN_N == 5)
    # M4's floor must stay strictly inside the EMPTY band measured on 2026-07-28: the
    # worst honoured declaration was 0.75, the best fictional one 0.25. Pinned against
    # those two literals, not against itself.
    check("M4_FIDELITY_FLOOR is inside the empty 0.25-0.75 band, and below 0.50",
          0.25 < M4_FIDELITY_FLOOR < 0.50)

    # --- cron expansion, canary-validated against hand-computed answers ---
    check("cron */10 over 6h == 36", expected_firings("*/10 * * * *", H6, NOW) == 36)
    check("cron */10 over 24h == 144",
          expected_firings("*/10 * * * *", NOW - dt.timedelta(hours=24), NOW) == 144)
    check("cron nightly over 24h == 1",
          expected_firings("17 3 * * *", NOW - dt.timedelta(hours=24), NOW) == 1)
    check("cron explicit-minute list over 6h == 36",
          expected_firings("4,14,24,34,44,54 * * * *", H6, NOW) == 36)
    check("cron 4-hourly over 24h == 6",
          expected_firings("0 */4 * * *", NOW - dt.timedelta(hours=24), NOW) == 6)
    check("cron weekly Monday absent from a Tuesday 24h window",
          expected_firings("0 6 * * 1", NOW - dt.timedelta(hours=24), NOW) == 0)
    check("cron weekly Monday present in a 48h window",
          expected_firings("0 6 * * 1", NOW - dt.timedelta(hours=48), NOW) == 1)
    # POSIX day-field UNION semantics (dom AND dow both restricted -> OR, not AND).
    check("cron dom+dow both restricted is a UNION",
          expected_firings("0 0 1 * 3", dt.datetime(2026, 7, 1, tzinfo=dt.timezone.utc),
                           dt.datetime(2026, 7, 31, tzinfo=dt.timezone.utc)) > 1)
    for bad in ("", "* * * *", "* * * * * *", "60 * * * *", "*/0 * * * *", "5-1 * * * *"):
        try:
            expected_firings(bad, H6, NOW)
            check(f"cron {bad!r} must raise CronError", False)
        except CronError:
            pass
        except Exception:
            check(f"cron {bad!r} raised the wrong error", False)

    # --- M1 scope is the exact complement of #4368 ---
    sched_only = {"schedule": [{"cron": "*/10 * * * *"}], "workflow_dispatch": None}
    mixed = dict(sched_only, pull_request=None)
    check("M1 excludes a cron-ONLY lane (#4368 owns it)", m1_scope(sched_only)[0] is False)
    check("M1 marks a cron-only lane as such", m1_scope(sched_only)[2] is True)
    check("M1 includes a schedule+other-trigger lane", m1_scope(mixed)[0] is True)
    check("M1 excludes an unscheduled workflow", m1_scope({"push": None})[0] is False)

    # --- M1 detection ---
    CAP = capped_expectation()
    f, c = find_cron_deficits([_lane(fires=CAP, now=NOW)], NOW)
    check("M1 quiet when the lane delivers its capped expectation",
          not f and c.get("delivering") == 1)
    f, c = find_cron_deficits([_lane(fires=1, now=NOW)], NOW)
    check("M1 raises on a firing deficit", len(f) == 1 and c.get("firing-deficit") == 1)
    check("M1 finding carries the CAPPED expectation and the nominal rate",
          f and f[0]["expected"] == CAP and f[0]["actual"] == 1
          and f[0]["nominal"] > CAP)
    f, _ = find_cron_deficits([_lane(fires=0, now=NOW)], NOW)
    check("M1 raises when a cron fired ZERO times (the no-artifact mode)", len(f) == 1)
    # The cap is what stops a `*/10` lane alarming forever: nominal 144/day, GitHub
    # delivers ~12, and 12 of a capped 12 is healthy.
    f, _ = find_cron_deficits([_lane(crons=("*/10 * * * *",), fires=CAP, now=NOW)], NOW)
    check("M1 does not alarm on a sub-hourly lane delivering GitHub's real ceiling", not f)
    # A firing NEWER than the grace cut-off must not be counted as missing.
    edge = _lane(fires=0, now=NOW)
    edge["schedule_run_times"] = [NOW - dt.timedelta(minutes=1)] * CAP
    f, _ = find_cron_deficits([edge], NOW)
    check("M1 ignores firings inside the grace window (they are not yet due)", len(f) == 1)
    f, c = find_cron_deficits([_lane(cron_only=True, in_scope=False, fires=0)], NOW)
    check("M1 never raises on a cron-only lane",
          not f and c.get("cron-only (watched by #4368)") == 1)
    f, c = find_cron_deficits([_lane(state="disabled_manually", fires=0, now=NOW)], NOW)
    check("M1 quiet on a disabled lane", not f and c.get("disabled") == 1)
    f, c = find_cron_deficits([_lane(crons=("not a cron",), fires=0, now=NOW)], NOW)
    check("M1 quiet + counted on an unparseable cron",
          not f and c.get("cron-unparseable") == 1)
    # A weekly lane has a capped expectation of 0 inside a 24h window -> below the floor.
    f, c = find_cron_deficits([_lane(crons=("0 6 * * 1",), fires=0, now=NOW)], NOW)
    check("M1 quiet when the expectation is below the floor",
          not f and c.get("expectation-below-floor") == 1)
    # Boundary direction: at/above the delivery floor must NOT raise; just below must.
    import math  # noqa: PLC0415
    at_floor = math.ceil(CAP * CRON_DELIVERY_FLOOR)
    f, _ = find_cron_deficits([_lane(fires=at_floor, now=NOW)], NOW)
    check("M1 does not raise AT the delivery floor", not f)
    f, _ = find_cron_deficits([_lane(fires=at_floor - 1, now=NOW)], NOW)
    check("M1 does raise just BELOW the delivery floor", len(f) == 1)
    # Over-delivery is healthy (the ratio simply exceeds 1.0; there is no clamp).
    f, _ = find_cron_deficits([_lane(fires=CAP * 3, now=NOW)], NOW)
    check("M1 treats an over-delivering lane as healthy", not f)

    # --- M1 threshold pins against LITERALS ------------------------------------------
    # `0 */4 * * *` has a 24h nominal of 5, which is BELOW the cap, so `expected` is 5
    # regardless of CRON_MAX_CREDIBLE_FIRINGS_PER_HOUR. Nothing in these three fixtures
    # is computed from CRON_DELIVERY_FLOOR or from the cap, so neither can rescale them.
    FOUR_HOURLY = ("0 */4 * * *",)
    f, _ = find_cron_deficits([_lane(crons=FOUR_HOURLY, fires=2, now=NOW)], NOW)
    check("M1 raises at 2 of a literal 5 (ratio 0.40) — pins the floor from BELOW",
          len(f) == 1 and f[0]["expected"] == 5 and f[0]["ratio"] == 0.4)
    f, _ = find_cron_deficits([_lane(crons=FOUR_HOURLY, fires=3, now=NOW)], NOW)
    check("M1 is quiet at 3 of a literal 5 (ratio 0.60) — pins the floor from ABOVE",
          not f)
    f, _ = find_cron_deficits([_lane(crons=FOUR_HOURLY, fires=2, now=NOW)], NOW)
    check("M1's DEFAULT window makes a 4-hourly lane's expectation exactly 5 "
          "(it is 1 at the rejected 6h window)", len(f) == 1 and f[0]["expected"] == 5)

    # --- M1 new-lane window: 'did not fire' vs 'did not exist yet' --------------------
    # A `*/10` lane added two hours ago has delivered ~1 run. Against a full-window
    # expectation of 12 that is ratio 0.08, and it alarms for the next ~14 hours.
    young = _lane(fires=1, now=NOW, created_hours_ago=2)
    f, c = find_cron_deficits([young], NOW)
    check("M1 does not alarm on a lane that did not EXIST for most of the window",
          not f)
    check("M1 counts a too-young lane as its own census state",
          c.get("lane-too-new-for-an-expectation") == 1)
    # ANTI-VACUITY: the identical fixture WITHOUT a birth date must still alarm, or the
    # new-lane exit is just an unconditional mute.
    f, _ = find_cron_deficits([_lane(fires=1, now=NOW)], NOW)
    check("M1 still alarms on the same delivery from an ESTABLISHED lane", len(f) == 1)
    # And a lane old enough to have earned an expectation is judged on delivery, not age.
    f, _ = find_cron_deficits([_lane(fires=1, now=NOW, created_hours_ago=48)], NOW)
    check("M1 judges a lane older than the window on delivery alone", len(f) == 1)
    f, _ = find_cron_deficits([_lane(fires=CAP, now=NOW, created_hours_ago=48)], NOW)
    check("M1 birth-date check does not change an established lane's verdict", not f)
    # THE BOUNDARY, pinned from both sides against the 24h window: a lane born just
    # INSIDE the window is skipped, one born just OUTSIDE it is judged. Without both
    # directions, `if born > start` and `if born > start - 100 years` are the same test.
    f, c = find_cron_deficits([_lane(fires=1, now=NOW, created_hours_ago=23)], NOW)
    check("M1 skips a lane born 23h ago (inside the 24h window)",
          not f and c.get("lane-too-new-for-an-expectation") == 1)
    f, c = find_cron_deficits([_lane(fires=1, now=NOW, created_hours_ago=25)], NOW)
    check("M1 judges a lane born 25h ago (outside the 24h window)",
          len(f) == 1 and c.get("firing-deficit") == 1)

    # --- M4: declared-vs-achieved cadence fidelity (#4802) ---------------------------
    # KNOWN POSITIVE. The real cron strings from this repo's workflow files, paired with
    # the achieved counts MEASURED over the 24h to 2026-07-28T13:15Z and recorded in the
    # M1 constants note. Nothing here is invented and nothing is computed from
    # M4_FIDELITY_FLOOR, so the floor cannot rescale the fixture.
    MEASURED_2026_07_28 = [
        ("promote-on-approval.yml", "*/10 * * * *", 12),
        ("rearm-sweeper.yml", "8,18,28,38,48,58 * * * *", 13),
        ("auto-arm.yml", "4,14,24,34,44,54 * * * *", 12),
        ("verdict-bridge.yml", "1,11,21,31,41,51 * * * *", 12),
        ("batch-merge.yml", "7,22,37,52 * * * *", 12),
        ("retriage.yml", "*/30 * * * *", 12),
    ]
    positive = [_lane(workflow=w, crons=(c,), fires=n, now=NOW, spacing_min=30)
                for w, c, n in MEASURED_2026_07_28]
    f, c = find_cadence_fidelity_gaps(positive, NOW)
    check("M4 KNOWN POSITIVE: the six measured 2026-07-28 lanes are all divergent",
          c.get("declaration-unachievable") == len(MEASURED_2026_07_28))
    check("M4 emits ONE repo-level finding, not one per lane",
          len(f) == 1 and len(f[0]["lanes"]) == len(MEASURED_2026_07_28))
    check("M4's finding subject is the repo, never a single lane",
          f and f[0]["workflow"].startswith("(repo-level"))
    check("M4 reports the DECLARED count, not M1's capped expectation",
          f and f[0]["lanes"][0]["nominal"] > capped_expectation())
    # KNOWN NEGATIVE. NOT "the same lanes on a healthy day" — the reconciliation in the
    # header shows those lanes deliver 12/day on a healthy day too, which is exactly why
    # M4 is a declaration detector. The negative is therefore a lane whose DECLARATION is
    # achievable, from the same census: ci.yml 1/1 and refresh-start-here 3/4.
    negative = [_lane(workflow="ci.yml", crons=("17 3 * * *",), fires=1, now=NOW),
                _lane(workflow="refresh-start-here.yml",
                      crons=("23 5,11,17,23 * * *",), fires=3, now=NOW)]
    f, c = find_cadence_fidelity_gaps(negative, NOW)
    check("M4 KNOWN NEGATIVE: the measured lanes with achievable declarations are silent",
          not f and c.get("declaration-within-the-measured-ceiling") == 2)
    # THRESHOLD PINS against LITERALS. `0 * * * *` declares exactly 23 firings inside the
    # graced 24h window — a number that derives from neither the floor nor the cap.
    HOURLY = ("0 * * * *",)
    f, _ = find_cadence_fidelity_gaps([_lane(crons=HOURLY, fires=9, now=NOW)], NOW)
    check("M4 raises at 9 of a literal 23 (0.391) — pins the floor from BELOW",
          len(f) == 1 and f[0]["lanes"][0]["nominal"] == 23)
    f, _ = find_cadence_fidelity_gaps([_lane(crons=HOURLY, fires=10, now=NOW)], NOW)
    check("M4 is quiet at 10 of a literal 23 (0.435) — pins the floor from ABOVE", not f)
    # ADMISSION pins, both sides of the measured ceiling of 12 declared firings.
    f, c = find_cadence_fidelity_gaps(
        [_lane(crons=("0 0-11 * * *",), fires=0, now=NOW)], NOW)
    check("M4 does not admit a lane declaring exactly the measured ceiling (12), even "
          "at zero delivery — that is M1's question on M1's denominator",
          not f and c.get("declaration-within-the-measured-ceiling") == 1)
    f, _ = find_cadence_fidelity_gaps(
        [_lane(crons=("30 0-12 * * *",), fires=0, now=NOW)], NOW)
    check("M4 admits a lane declaring one MORE than the ceiling (13)",
          len(f) == 1 and f[0]["lanes"][0]["nominal"] == 13)
    # Cron-only lanes are M4's population too — three of the five #4802 lanes are cron-only
    # and M1 structurally cannot see any of them.
    f, _ = find_cadence_fidelity_gaps(
        [_lane(workflow="promote-on-approval.yml", cron_only=True, in_scope=False,
               fires=12, now=NOW)], NOW)
    check("M4 covers a CRON-ONLY lane (M1 cannot see it at all)", len(f) == 1)
    # Fail-safe exits, each counted rather than silent.
    f, c = find_cadence_fidelity_gaps([_lane(state="disabled_manually", fires=0, now=NOW)],
                                      NOW)
    check("M4 quiet on a disabled lane", not f and c.get("disabled") == 1)
    f, c = find_cadence_fidelity_gaps([_lane(crons=("not a cron",), fires=0, now=NOW)], NOW)
    check("M4 quiet + counted on an unparseable cron",
          not f and c.get("cron-unparseable") == 1)
    f, c = find_cadence_fidelity_gaps([_lane(crons=(), fires=0, now=NOW)], NOW)
    check("M4 skips an unscheduled workflow", not f and c.get("not-scheduled") == 1)
    truncated = _lane(fires=0, now=NOW)
    truncated["truncated"] = True
    f, c = find_cadence_fidelity_gaps([truncated], NOW)
    check("M4 is fail-safe QUIET on a truncated run sample (an undercount would "
          "manufacture a fiction out of a healthy lane)",
          not f and c.get("sample-truncated") == 1)
    f, c = find_cadence_fidelity_gaps([_lane(fires=0, now=NOW, created_hours_ago=2)], NOW)
    check("M4 does not judge a lane that did not EXIST for most of the window",
          not f and c.get("lane-too-new-for-an-expectation") == 1)
    # ANTI-VACUITY for both mutes above: the identical delivery from an established,
    # untruncated lane MUST be divergent, or those exits are unconditional switches.
    f, _ = find_cadence_fidelity_gaps([_lane(fires=0, now=NOW)], NOW)
    check("M4 still raises on the same delivery from an established, untruncated lane",
          len(f) == 1)
    _, c = find_cadence_fidelity_gaps([_lane(crons=HOURLY, fires=10, now=NOW)], NOW)
    check("M4 emits a census on the all-clear too", c.get("declaration-honoured") == 1)

    # --- M3: detection variable is LIVE in_progress age ---
    base = {(".github/workflows/a.yml", "schedule"): {"p90": 3600.0, "n": 50}}
    f, c = find_execution_overruns([_run_obj(age_min=30, now=NOW)], base, NOW)
    check("M3 quiet inside the band", not f and c.get("in-progress-within-threshold") == 1)
    f, c = find_execution_overruns([_run_obj(age_min=8 * 60, now=NOW)], base, NOW)
    check("M3 raises past the band", len(f) == 1 and c.get("execution-overrun") == 1)
    f, _ = find_execution_overruns([_run_obj(status="queued", age_min=999, now=NOW)],
                                   base, NOW)
    check("M3 ignores queued runs (that is M2's job)", not f)
    # THE FAIL-OPEN HOLE: a lane whose runs never complete has no baseline. It must still
    # be watchable, or the detector goes silent exactly when a lane is 100% hung.
    f, _ = find_execution_overruns([_run_obj(age_min=7 * 60, now=NOW)], {}, NOW)
    check("M3 still raises with NO baseline at all (fail-open hole closed)", len(f) == 1)
    check("M3 names the floor as the basis when there is no baseline",
          f and "floor" in f[0]["basis"])
    thin = {(".github/workflows/a.yml", "schedule"): {"p90": 60.0, "n": 1}}
    f, _ = find_execution_overruns([_run_obj(age_min=7 * 60, now=NOW)], thin, NOW)
    check("M3 falls back to the floor on an under-sampled baseline", len(f) == 1)
    check("M3 does NOT use a 1-sample p90 as the threshold",
          f and f[0]["threshold_seconds"] == int(EXEC_FLOOR_SECONDS))
    # A big baseline must RAISE the threshold above the floor, not be ignored.
    big = {(".github/workflows/a.yml", "schedule"): {"p90": 8 * 3600.0, "n": 40}}
    f, _ = find_execution_overruns([_run_obj(age_min=7 * 60, now=NOW)], big, NOW)
    check("M3 respects a baseline WIDER than the floor", not f)
    f, _ = find_execution_overruns([_run_obj(age_min=17 * 60, now=NOW)], big, NOW)
    check("M3 raises on the measured 2026-07-27 17.3h-vs-8.3h shape", len(f) == 1)
    # --- M3 threshold pins against LITERALS ------------------------------------------
    # 9h against an 8h p90 is 1.13x — inside the band at K=2.0, outside it at K<=1.13.
    # The 17.3h check above pins the multiple from ABOVE (K=2.5 misses it); this pins it
    # from BELOW, which nothing did before.
    f, _ = find_execution_overruns([_run_obj(age_min=9 * 60, now=NOW)], big, NOW)
    check("M3 is quiet at 9h against an 8h p90 — pins the multiple from BELOW", not f)
    # The floor, from both directions, with no baseline in play.
    f, _ = find_execution_overruns([_run_obj(age_min=5 * 60, now=NOW)], {}, NOW)
    check("M3 is quiet at 5h with no baseline — pins the 6h floor from BELOW", not f)
    f, _ = find_execution_overruns([_run_obj(age_min=7 * 60, now=NOW)], {}, NOW)
    check("M3 raises at 7h with no baseline — pins the 6h floor from ABOVE", len(f) == 1)
    # THE `updated_at` SUBSTITUTION. A live run's `updated_at` tracks the present moment,
    # so `started = run.get("updated_at") or run_started_at` collapses every age to ~0 and
    # M3 never fires again. It survived the first battery only because the fixture was a
    # hand-built dict with no `updated_at`; the fixture now carries the real shape.
    live = _run_obj(age_min=17 * 60, now=NOW)
    check("the M3 fixture carries a live run's real `updated_at`",
          (NOW - _ts(live["updated_at"])).total_seconds() < 60
          and (NOW - _ts(live["run_started_at"])).total_seconds() > 16 * 3600)

    # --- census is emitted even on the all-clear ---
    _, c1 = find_cron_deficits([_lane(fires=36, now=NOW)], NOW)
    _, c3 = find_execution_overruns([_run_obj(age_min=1, now=NOW)], base, NOW)
    check("a clean run still emits a non-empty census for every mode",
          bool(c1) and bool(c3))

    # --- issue body carries the dedupe marker ---
    body = render_issue_body("o/r", [{"mode": "M3-execution-overrun", "workflow": "a.yml",
                                      "run_id": 1, "event": "push", "age_seconds": 99,
                                      "threshold_seconds": 60, "basis": "floor",
                                      "head_branch": "main"}],
                             {"M3-execution-overrun": {"execution-overrun": 1}}, NOW)
    check("issue body ends with the dedupe marker",
          body.rstrip().endswith(f"<!-- {KEY_PREFIX}: o/r -->"))
    check("issue body self-identifies as a SPARQ agent", body.startswith("> 🤖"))
    check("issue body prints the census", "Census of every state exit" in body)
    m4_body = render_issue_body(
        "o/r",
        [{"mode": "M4-declared-cadence-unachievable",
          "workflow": "(repo-level: 1 lane declarations)",
          "lanes": [{"workflow": "retriage.yml", "nominal": 47, "actual": 12,
                     "fidelity": 0.255, "crons": ["*/30 * * * *"]}],
          "window_hours": 24.0, "floor": M4_FIDELITY_FLOOR}],
        {"M4-declared-cadence-unachievable": {"declaration-unachievable": 1}}, NOW)
    check("M4 issue body names the lane, its declaration and what arrived",
          "retriage.yml" in m4_body and "47" in m4_body and "12" in m4_body)
    check("M4 issue body says it does not red the run",
          "does not red this run" in m4_body)

    # --- exit codes are distinct ---
    check("a bad repo slug is fail-loud exit 2",
          main(["--repo", "not-a-slug", "--dry-run"]) == 2)

    if failures:
        for name in failures:
            print(f"FAIL: {name}")
        print(f"::error::ci_execution_latency_alarm self-test: {len(failures)} failure(s)")
        return 1
    print("ci_execution_latency_alarm self-test: all checks passed")
    return 0


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description="CI execution-latency alarm")
    parser.add_argument("--repo", default="")
    parser.add_argument("--now", default="")
    parser.add_argument("--state-file", default="")
    parser.add_argument("--window-hours", type=float, default=CRON_WINDOW_HOURS)
    parser.add_argument("--dry-run", action="store_true")
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args(argv)
    if args.self_test:
        return _self_test()
    try:
        return run(args)
    except AlarmError as exc:
        print(f"::error::ci execution-latency alarm infrastructure failure: {exc}")
        return 2


if __name__ == "__main__":
    sys.exit(main())
