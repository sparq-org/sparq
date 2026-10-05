#!/usr/bin/env python3
# [OPUS-4.8] Nightly change-based-selection BUG ALARM (bead sq-va7at, epic
# sq-fmx4u). Design: research/change-based-test-selection.md §6.1.
# Authored by Opus 4.8 (running in place of the intended sonnet tier — Fable
# unavailable). Flag for re-review when Fable returns.
#
# WHAT THIS IS (and is NOT):
#   The correctness safeguard that closes the loop on change-based test selection
#   (scripts/ci_select.py). The nightly FULL-matrix backstop (ci.yml `schedule`
#   => mode=full) already RE-RUNS everything a PR may have selection-skipped; the
#   `selection-backstop` job asserts that full-run invariant. What was MISSING is
#   the correlation ALARM: when a scheduled nightly job FAILS and that same job's
#   crate was selection-SKIPPED in one or more PRs that landed since the last
#   GREEN nightly, that intersection is *prima-facie* evidence of a selection
#   soundness bug (the non-interference proof in §2 was wrong — a change that
#   "looked unaffected" actually broke the crate). This script computes that
#   intersection and files a self-identified SPARQ-agent GitHub issue naming the
#   offending job(s) + the suspect PRs.
#
#   It is a DETECTOR, never a gate: it runs AFTER a nightly completes (a
#   `workflow_run` trigger, see .github/workflows/selection-alarm.yml), files
#   issues, and never blocks a merge (design §6.1: "fail-open").
#
#   [OPUS-5] #4965 — WHAT COUNTS AS A NIGHTLY WORTH CORRELATING. A "failed nightly"
#   means "a nightly with genuinely-failed JOBS", never "a nightly GitHub concluded
#   `failure`". Those are different populations: fail-fast cancels the surviving
#   matrix legs, so the RUN concludes `cancelled` and the run-level conclusion is a
#   lossy aggregate. Admission is therefore made HERE, over the job list, by the same
#   FAILED_CONCLUSIONS predicate the analysis uses — see STRICT_EMPTY_CONCLUSIONS.
#
#   [OPUS-5] #5025 — WHAT BOUNDS THE CORRELATION WINDOW. #4965 moved ADMISSION onto
#   the job list and left the WINDOW BOUND on the same lossy run-level aggregate:
#   the base was the head_sha of the most recent run-level `success`. That is the
#   last place the alarm trusted the aggregate, and it is why a first real firing
#   carried 644 suspects — the last `success` was 28 days behind HEAD while nightly
#   after nightly aggregated to `cancelled`. The bound is now computed by
#   `find_window_bound` over the JOB list of each recent scheduled run — see
#   `classify_run` for the predicate and why it needs a POSITIVE coverage term that
#   "no job failed" alone does not give. `render_nightly_health` states — whenever
#   there is something to correlate, and on demand via `--nightly-health` — which
#   runs were examined, why each was rejected, and which jobs are red in more than
#   one of them.
#
# TWO DESIGN INVARIANTS (both load-bearing):
#   1. FAIL-SAFE / FAIL-LOUD (opposite of ci_select.py's fail-CLOSED). An
#      alarm-INFRASTRUCTURE error (cannot list the failed jobs, cannot load cargo
#      metadata, cannot replay a landed commit's selection) must NEVER be swallowed
#      into a silent exit-0 — that would MASK a real selection bug. main() files
#      every finding it COULD compute and then exits NON-ZERO with a loud
#      `::error::` annotation, so the alarm workflow run goes RED and a human
#      notices the detector itself is broken. Redding this post-hoc workflow blocks
#      nothing (it is not a required check).
#   2. NON-SPAMMY (dedupe). One issue per distinct failure<->skip mapping, keyed by
#      a stable `<!-- selection-alarm-key: ... -->` marker; before filing we search
#      OPEN labelled issues for that key and skip if one already exists (the exact
#      flow-on idempotency mechanism). And the alarm fires ONLY when selection
#      actually skipped something relevant: an empty skip-set in the window => no
#      finding, because a nightly failure with nothing skipped cannot be a
#      selection bug.
#
# WHY GIT-REPLAY (not a stored artifact) for "record per-PR selection decisions"
# (design §6.1 item 1): scripts/ci_select.py is a PURE function of (diff, cargo
# metadata, ownership map). The per-PR selection decision is therefore
# RECONSTRUCTIBLE from git history — the most durable "record" there is (git
# history outlives any artifact-retention window). For each landed commit we
# replay `select(git-diff, metadata)` and take skipped = members - affected when
# mode=='selected'. Using the CURRENT (HEAD) metadata is conservative: any member
# added/removed in the window changed the root Cargo.toml, a §4.1 full-run trigger
# => that commit replays mode=full => skipped=∅ => it cannot be a suspect anyway.
#
# WHY GITHUB ISSUES, NOT `bd create` (mirrors flow-on.py): CI has NO `bd` dolt DB
# (it lives only in the orchestrator's gitignored checkout; hand-editing `.beads/`
# is forbidden). So this script emits GitHub issues; the orchestrator reconciles
# alarm issues into P1 beads out-of-band. The bead's "check bd before filing"
# dedupe is satisfied at that reconciliation layer; in CI, dedupe is by open issue.
#
# Usage:
#   ci_selection_alarm.py --run-id <id> --head-sha <sha>     # real (gh + git + cargo)
#   ci_selection_alarm.py --run-id <id> --head-sha <sha> --dry-run   # no gh writes
#   ci_selection_alarm.py --run-id <id> --head-sha <sha> \
#       --failed-jobs-file jobs.txt --metadata-file meta.json        # hermetic inputs (tests)
#
# Stdlib only (design §7 P1): argparse/json/os/re/subprocess/importlib + the
# shared scripts/ci_select.py (imported by path). Runs under the CI setup-python
# 3.12.

from __future__ import annotations

import argparse
import importlib.util
import json
import os
import re
import subprocess
import sys
from dataclasses import dataclass, field
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent

# Labels every alarm issue carries (mirrors flow-on's `flow-on`+`auto`). The label
# names carry NO `advisory`/`informational` token — irrelevant here (issues are
# not check-runs) but kept clean for consistency.
BASE_LABELS = ["selection-alarm", "auto"]

# Idempotency marker embedded in each issue body (flow-on pattern).
KEY_PREFIX = "selection-alarm-key"

# Job conclusions that count as a "failed" nightly job.
FAILED_CONCLUSIONS = {"failure", "timed_out"}

# How many suspect PRs a rendered issue body LISTS. [OPUS-5] sparq-org/sparq#4984.
#
# WHY A CAP AT ALL. The suspect set is every PR that selection-skipped the crate in
# the window (last green nightly, HEAD] — so it grows with the AGE OF THE BACKSTOP,
# not with the size of the bug. MEASURED 2026-07-28 against real nightly
# 30333511110: 644 / 579 / 554 suspects per finding in 18,064 / 16,329 / 15,755-char
# bodies, because the last GREEN nightly was 2026-07-01. Unbounded that reaches
# GitHub's 65,536-char issue-body limit at roughly 2,300 suspects, where
# `gh issue create` 422s and create_issue()'s AlarmError escapes main()'s try.
#
# WHAT THE CAP IS NOT. It is not a fix for the stale backstop and it does not narrow
# the correlation: `correlate` still computes the FULL suspect set, dedupe still keys
# on the same marker, and the omitted COUNT is always stated in the body. A 644-entry
# list is a TRUE statement about a 28-day gap; the defect is that it is unreadable.
# Silent truncation would turn a true-but-unreadable body into a false one.
#
# WHY 20. It is a page a human actually reads. Past ~20 candidates the response to a
# suspect list stops being "read it" and becomes "bisect", and the body says so.
MAX_RENDERED_SUSPECTS = 20

# Stable token marking the truncation disclosure. Present IFF suspects were omitted,
# so its ABSENCE is the machine-checkable form of "this list is complete".
SUSPECT_OMISSION_MARKER = "more suspect PR(s) not listed"

# RUN-level conclusions for which an EMPTY genuinely-failed-job set is ANOMALOUS
# (fail-loud, invariant 1). [OPUS-5] sparq-org/sparq#4965.
#
# WHY THIS EXISTS. The workflow used to admit ONLY `conclusion == 'failure'` runs,
# so "the run fired => at least one job failed" was a safe premise and an empty
# failed-job set could only be an API-shape surprise. That premise cost the alarm
# its whole population: on a fail-fast matrix ONE dying job CANCELS the other 30,
# and GitHub then concludes the RUN `cancelled`, not `failure`. MEASURED over all
# 45 nightlies: 35 `cancelled`, 7 `success`, 3 `failure` (last 2026-07-19) — 266 of
# 267 alarm runs job-SKIPPED, and a skipped job is never red, so the alarm went
# inert in silence. Known positive: run 30333511110 concluded `cancelled` while
# carrying FOUR genuinely `failure` jobs (sparq-mpc / sparq-fedplan / sparq-reason).
#
# The run conclusion is therefore a LOSSY aggregate and is no longer the admission
# filter (see .github/workflows/selection-alarm.yml). What survives of the old
# premise is exactly this: a run GitHub concluded `failure` or `timed_out` MUST
# contain a job in FAILED_CONCLUSIONS, so an empty set there is still anomalous and
# must never be swallowed into a silent exit 0. Every OTHER conclusion — above all
# `cancelled`, which is the COMMON case here — can legitimately carry zero genuine
# job failures, and for those an empty set is a correct, quiet no-op. That split is
# what keeps a nightly alarm from crying wolf 35 times a window; an alarm that cries
# wolf gets muted, which is the same inertness by a slower route.
#
# "" (conclusion unknown / unresolvable) is STRICT on purpose: fail loud on
# ignorance rather than assume the quiet branch.
STRICT_EMPTY_CONCLUSIONS = {"failure", "timed_out", ""}

# --- triage lanes (design §5.2 + the erratum) --------------------------------
# The change-based selector skips two lane CLASSES per PR:
#   (a) crate-NAMED lanes — feature-matrix legs `opt-in <crate> (...)`, per-crate
#       wasm/other jobs — where the crate name appears in the JOB name, so a
#       failed job maps PRECISELY to its crate by a delimited-token match; and
#   (b) the cross-crate bulk/heavy nextest SHARDS `test (load-aware shard <name>)`,
#       which are NARROWED per-PR by the nextest `filterset` (bulk) or a
#       single-crate skip guard (heavy) — their JOB names ("bulk 1/3",
#       "heavy-diskann") do NOT encode the failing crate.
# A class-(b) shard failure is a REAL selection-skip signal we must not mask, but
# we cannot name the crate from the job name alone. So a failed shard with no
# extractable crate goes to the FAIL-SAFE TRIAGE path: one issue naming the
# shard(s) + the whole skipped set in the window, for a human to narrow from the
# shard log. Everything ELSE that is unmappable (coverage/select/backstop/docs/
# supply-chain jobs — not per-crate test lanes) is NOT triage-eligible, so an
# infra-lane failure never mints a spurious selection alarm.
# scripts/tests/test_ci_selection_alarm.py pins this regex against the real
# ci.yml shard job name (`test (load-aware shard ${{ matrix.name }})`).
TRIAGE_LANE_RE = re.compile(r"^test \(load-aware shard ")

# --- the correlation window's LOWER BOUND (#5025) ----------------------------
# How many recent COMPLETED scheduled runs `find_window_bound` examines before it
# gives up and reports a stale backstop. One job-list API call per run examined, so
# this is the cost ceiling of the scan; at the nightly cadence it is ~a month of
# backstops, which is longer than the 28-day gap that motivated #5025.
BOUND_SCAN_RUNS = 30

# A job conclusion that means the lane RAN and PASSED. Anything else — `cancelled`
# (the 6h/timeout-minutes ceiling, or a concurrency cancel), `skipped`, `neutral`,
# "" — means the lane was not verified BY THIS RUN, which is a different claim from
# "the lane failed" and is why classify_run keeps the two apart.
VERIFIED_CONCLUSION = "success"

# A job that failed in at least this many of the runs examined is reported as
# PERSISTENTLY red rather than as a one-off. 2 is the smallest number that can
# distinguish the two at all; the report prints the count either way, so a reader
# never has to trust the label over the number.
PERSISTENT_RED_MIN_RUNS = 2

# Per-run job names quoted inline in the health report. Run 30333511110's census is
# 84 jobs — 31 of them cancelled — so an unbounded per-run job list would make a
# health TABLE unreadable, the #4984 defect in a new place. The omitted COUNT is
# always stated, so a bounded list is never a silent truncation.
MAX_RENDERED_JOBS = 5


class AlarmError(Exception):
    """An alarm-INFRASTRUCTURE failure. main() surfaces it LOUD (non-zero exit)."""


# --------------------------------------------------------------------------- #
# Shared selector (imported by path — scripts/ is not an importable package)
# --------------------------------------------------------------------------- #
def _load_ci_select():
    spec = importlib.util.spec_from_file_location(
        "ci_select", REPO_ROOT / "scripts" / "ci_select.py"
    )
    assert spec and spec.loader
    mod = importlib.util.module_from_spec(spec)
    # Register before exec so ci_select's dataclass string annotations
    # (from __future__ import annotations) resolve via sys.modules.
    sys.modules[spec.name] = mod
    spec.loader.exec_module(mod)
    return mod


# --------------------------------------------------------------------------- #
# Data model
# --------------------------------------------------------------------------- #
@dataclass(frozen=True)
class WindowCommit:
    """A landed commit in the (last-green-nightly, HEAD] window and the crate set
    its PR selection-SKIPPED (empty when it ran mode=full)."""

    sha: str
    pr: int | None
    skipped: frozenset[str]


@dataclass
class Finding:
    """One distinct failure<->skip mapping to file as an issue."""

    kind: str  # "crate" (precise) | "triage" (unmappable shard)
    key: str  # stable dedup key (embedded in the body marker)
    crate: str | None  # precise: the offending crate; triage: None
    failed_jobs: list[str]  # failed job(s) implicating this mapping
    suspect_prs: list[WindowCommit]  # landed PRs whose skip-set implicates this


@dataclass(frozen=True)
class RunHealth:
    """What one examined scheduled run's JOB list says about the backstop (#5025)."""

    run_id: str
    head_sha: str
    conclusion: str  # the LOSSY run-level aggregate — reported, never trusted
    created_at: str  # ISO-8601 from the runs API ("" if the API omitted it)
    failed_jobs: tuple[str, ...]  # conclusion in FAILED_CONCLUSIONS
    unverified_shards: tuple[str, ...]  # full-matrix shards that did not succeed
    shard_count: int  # full-matrix shard jobs present at all
    bounds: bool  # may this run bound the correlation window?
    reason: str  # why — stated for the accepted run as well as the rejected ones


# --------------------------------------------------------------------------- #
# Pure correlation core (hermetic-testable — no gh / git / cargo)
# --------------------------------------------------------------------------- #
def job_crate_tokens(job_name: str, members: set[str]) -> set[str]:
    """Workspace-member crate names that appear as DELIMITED tokens in a job name.

    Delimited = bounded on each side by a non `[A-Za-z0-9_-]` char (or a string
    boundary), so `sparq-core` matches inside `opt-in sparq-core (mmap)` but NOT
    inside `sparq-core-foo`, and `sparq-vec` does not match inside
    `sparq-vectors`. Usually 0 or 1 member; a job that names several crates
    implicates all of them (conservative)."""
    found: set[str] = set()
    for m in members:
        if re.search(rf"(?<![A-Za-z0-9_-]){re.escape(m)}(?![A-Za-z0-9_-])", job_name):
            found.add(m)
    return found


def correlate(
    failed_jobs: list[str],
    window: list[WindowCommit],
    members: set[str],
) -> list[Finding]:
    """Intersect the failed nightly jobs with the per-PR selection skips.

    Precise: for a failed job that names crate C AND C was skipped by some landed
    PR => a `crate` finding for C (its suspect PRs = the PRs that skipped C).
    Triage: a failed TRIAGE-LANE shard with no extractable crate, when ANYTHING
    was skipped in the window => one `triage` finding naming those shards + every
    PR that skipped anything. Fires only where selection actually skipped
    something relevant (design non-spam property)."""
    # crate -> [commits that skipped it]
    skip_to_prs: dict[str, list[WindowCommit]] = {}
    for wc in window:
        for c in wc.skipped:
            skip_to_prs.setdefault(c, []).append(wc)
    skipped_union = set(skip_to_prs)

    findings: list[Finding] = []

    # --- precise per-crate findings ---
    crate_jobs: dict[str, list[str]] = {}
    unmappable_triage_jobs: list[str] = []
    for job in failed_jobs:
        tokens = job_crate_tokens(job, members)
        implicated = tokens & skipped_union
        if implicated:
            for c in implicated:
                crate_jobs.setdefault(c, []).append(job)
        elif not tokens and TRIAGE_LANE_RE.match(job):
            # A narrowed shard with no crate in its name AND not resolved to a
            # skipped crate => fail-safe triage candidate.
            unmappable_triage_jobs.append(job)
        # else: a crate-named job whose crate was NOT skipped (its own tests ran
        # per-PR — not a selection bug), or a non-test infra lane => ignored.

    for crate in sorted(crate_jobs):
        findings.append(
            Finding(
                kind="crate",
                key=f"crate:{crate}",
                crate=crate,
                failed_jobs=sorted(set(crate_jobs[crate])),
                suspect_prs=skip_to_prs[crate],
            )
        )

    # --- fail-safe triage finding ---
    if unmappable_triage_jobs and skipped_union:
        jobs_sorted = sorted(set(unmappable_triage_jobs))
        # Suspect PRs = every landed PR that skipped ANY crate (we cannot narrow
        # which crate the generic shard's failure came from).
        suspects = [wc for wc in window if wc.skipped]
        findings.append(
            Finding(
                kind="triage",
                key="triage:" + "|".join(jobs_sorted),
                crate=None,
                failed_jobs=jobs_sorted,
                suspect_prs=suspects,
            )
        )
    return findings


# --------------------------------------------------------------------------- #
# The window's lower bound, from the JOB list (#5025) — pure, hermetic-testable
# --------------------------------------------------------------------------- #
def _quote_jobs(names: tuple[str, ...] | list[str], cap: int = MAX_RENDERED_JOBS) -> str:
    """`a`, `b`, +N more — bounded, never a silent truncation."""
    shown = list(names)[:cap]
    # `|` is escaped: these strings land in the health report's markdown TABLE, and a
    # job name carrying a pipe would silently split the row it is describing.
    rendered = ", ".join("`" + n.replace("|", "\\|") + "`" for n in shown)
    omitted = len(names) - len(shown)
    return rendered + (f", +{omitted} more" if omitted else "")


def classify_run(
    run_id: str,
    head_sha: str,
    conclusion: str,
    created_at: str,
    jobs: list[tuple[str, str]],
) -> RunHealth:
    """May this scheduled run bound the correlation window, and why (not)?

    THE PREDICATE. A run bounds the window iff, over its REAL job list:
      (a) no job concluded in FAILED_CONCLUSIONS — nothing genuinely broke; AND
      (b) it contains at least one full-matrix test shard (TRIAGE_LANE_RE) — the
          run actually exercised the backstop; AND
      (c) EVERY such shard concluded `success` — every lane the per-PR selector is
          able to narrow was re-run and passed in this run.

    WHY (b)+(c) AND NOT JUST (a). #5025 proposes "a scheduled run in which no job
    concluded failure/timed_out", and (a) alone is that. But "nothing failed" is
    satisfied vacuously by a run that verified NOTHING — every shard `cancelled` at
    the timeout ceiling, or `skipped` behind a freshness/path gate. Accepting such a
    run as the bound would move the base FORWARD past commits no backstop ever
    re-ran, which SHRINKS the suspect set and hides real suspects. That is the
    failure mode #5025 forbids ("Do not fix this by widening the lookback or by
    suppressing the alarm") wearing the opposite sign, so the bound needs positive
    evidence of coverage, not merely the absence of a failure.

    WHY THE RUN-LEVEL CONCLUSION IS NOT CONSULTED AT ALL. It is the lossy aggregate
    #4965 measured: one cancelled leg makes a whole run `cancelled`, so a run whose
    every shard passed can never reach `success` and the OLD bound
    (`?status=success`) skipped it. It is carried on RunHealth for the report — a
    reader comparing "GitHub says cancelled" with "every shard passed" is seeing
    exactly the discrepancy that caused #5025 — but no branch here reads it.
    """
    failed = tuple(sorted({n for n, c in jobs if (c or "").strip() in FAILED_CONCLUSIONS}))
    shards = [(n, (c or "").strip()) for n, c in jobs if TRIAGE_LANE_RE.match(n)]
    unverified = tuple(sorted({n for n, c in shards if c != VERIFIED_CONCLUSION}))

    if failed:
        bounds, reason = False, (
            f"{len(failed)} job(s) concluded {'/'.join(sorted(FAILED_CONCLUSIONS))}: "
            f"{_quote_jobs(failed)}"
        )
    elif not shards:
        bounds, reason = False, (
            "no full-matrix test shard job present — this run did not exercise the "
            "backstop, so it cannot witness that anything was re-run"
        )
    elif unverified:
        bounds, reason = False, (
            f"{len(unverified)} of {len(shards)} test shard(s) did not conclude "
            f"`{VERIFIED_CONCLUSION}` (cancelled/skipped lanes were NOT verified): "
            f"{_quote_jobs(unverified)}"
        )
    else:
        bounds, reason = True, (
            f"VERIFIED — all {len(shards)} full-matrix test shard(s) succeeded and no "
            "job failed"
        )
    return RunHealth(
        run_id=run_id,
        head_sha=head_sha,
        conclusion=(conclusion or "").strip(),
        created_at=(created_at or "").strip(),
        failed_jobs=failed,
        unverified_shards=unverified,
        shard_count=len(shards),
        bounds=bounds,
        reason=reason,
    )


def persistently_red_jobs(
    scanned: list[RunHealth], min_runs: int = PERSISTENT_RED_MIN_RUNS
) -> list[tuple[str, int]]:
    """(job name, how many of the examined runs it FAILED in), most-frequent first,
    for jobs at or above `min_runs`.

    This is the discriminator #5025 asks for and that no single run can answer: "are
    the four genuinely-failed jobs of run 30333511110 a persistent red or a flaky
    mutation-ratchet lane?" is a question about a job's behaviour ACROSS runs."""
    counts: dict[str, int] = {}
    for health in scanned:
        for job in health.failed_jobs:
            counts[job] = counts.get(job, 0) + 1
    return sorted(
        ((job, n) for job, n in counts.items() if n >= min_runs),
        key=lambda item: (-item[1], item[0]),
    )


def render_nightly_health(scanned: list[RunHealth], bound: RunHealth | None) -> str:
    """The backstop's OWN health, as a markdown block for stdout + the step summary.

    #5025 acceptance item 1 ("the nightly's own health is stated: is the full matrix
    actually green, and if not, which jobs are persistently red?") is a question that
    has to be re-answered every night, so the alarm answers it whenever it has
    something to correlate — and on demand via `--nightly-health` — rather than it
    being pasted into one issue body that is stale the next night."""
    if not scanned:
        return (
            "### nightly backstop health\n\n"
            "- **No completed scheduled run found at all.** Cold start, or the runs "
            "API returned nothing — there is no backstop to bound the window with.\n"
        )

    lines = [
        "### nightly backstop health",
        "",
        f"- **Examined:** {len(scanned)} most recent completed scheduled run(s), "
        "newest first.",
    ]
    if bound is not None:
        lines.append(
            f"- **Window bound:** run `{bound.run_id}` (head `{bound.head_sha[:12]}`, "
            f"{bound.created_at or 'date unknown'}) — {bound.reason}. GitHub "
            f"aggregated that run to `{bound.conclusion or 'unknown'}`."
        )
    else:
        lines.append(
            f"- **NO examined run verified the full matrix.** The backstop is STALE: "
            f"the window is horizon-bounded at the oldest run examined "
            f"({scanned[-1].created_at or 'date unknown'}), not coverage-bounded, so "
            f"suspects that landed before it are NOT reported. Fix the nightly — "
            f"widening or narrowing this window would only make the report false."
        )

    persistent = persistently_red_jobs(scanned)
    if persistent:
        lines.append(
            f"- **Persistently red** (failed in ≥{PERSISTENT_RED_MIN_RUNS} of "
            f"{len(scanned)} examined): "
            + ", ".join(f"`{job}` ×{n}" for job, n in persistent[:MAX_RENDERED_JOBS])
            + (f", +{len(persistent) - MAX_RENDERED_JOBS} more"
               if len(persistent) > MAX_RENDERED_JOBS else "")
        )
    else:
        lines.append(
            f"- **No job failed in ≥{PERSISTENT_RED_MIN_RUNS} of the examined runs** "
            "— every failure seen is a one-off (flake, or a red that was fixed)."
        )

    lines += ["", "| run | date | GitHub conclusion | shards | bounds? | why |",
              "| --- | --- | --- | --- | --- | --- |"]
    for h in scanned:
        lines.append(
            f"| `{h.run_id}` | {h.created_at or '?'} | `{h.conclusion or 'unknown'}` "
            f"| {h.shard_count} | {'yes' if h.bounds else 'no'} | {h.reason} |"
        )
    return "\n".join(lines) + "\n"


# --------------------------------------------------------------------------- #
# Issue rendering
# --------------------------------------------------------------------------- #
def _pr_ref(wc: WindowCommit) -> str:
    return f"#{wc.pr}" if wc.pr is not None else f"`{wc.sha[:12]}`"


def key_marker(key: str) -> str:
    return f"<!-- {KEY_PREFIX}: {key} -->"


def _suspect_row(wc: WindowCommit, kind: str) -> str:
    row = f"- {_pr_ref(wc)} (`{wc.sha[:12]}`)"
    if kind == "triage":
        # Triage cannot name the failing crate from the job, so each suspect's
        # skip-set is what a human narrows from. Bounded by the WORKSPACE size, not
        # by the window, so it is not the dimension #4984 caps.
        row += f" — skipped: {', '.join(sorted(wc.skipped))}"
    return row


def render_suspects(
    suspects: list[WindowCommit], kind: str, cap: int = MAX_RENDERED_SUSPECTS
) -> str:
    """The suspect-PR block, BOUNDED at `cap` rows, disclosing any omission by COUNT.

    ORDERING — landing recency, NEWEST FIRST. This is the INPUT order, not a re-sort:
    `window_commits` reads `git log <base>..<head>`, which is reverse-chronological;
    `build_window` appends in that order; and `correlate` appends into
    `skip_to_prs[crate]` (and filters `window` for triage) in that order too. So
    `Finding.suspect_prs` arrives newest-landed first. `WindowCommit` carries no
    timestamp, so the render cannot re-derive this — it is an invariant of the
    pipeline above, pinned by TestSuspectOrderIsLandingRecency.

    WHY newest-first is the defensible end to keep. Every PR that landed AFTER a
    given suspect and that AFFECTED the failing crate re-ran that crate's lanes on a
    tree already containing the suspect. So the older a suspect is, the more
    independent chances its breakage has already had to surface elsewhere, and the
    newest suspects are the ones with the least intervening coverage. That is a
    PRIOR, not an exoneration — per-PR selected lanes are narrower than the nightly
    full matrix, so an older suspect is not cleared — which is exactly why the
    omitted count is STATED rather than dropped.
    """
    cap = max(cap, 0)
    total = len(suspects)
    shown = suspects[:cap]
    lines = [_suspect_row(wc, kind) for wc in shown]
    omitted = total - len(shown)
    if omitted:
        # Never a silent truncation: the count, the total, the cap, and the ordering
        # basis all appear, so the reader can tell what was dropped and why.
        lines.append(
            f"\n> ⚠️ **{omitted} {SUSPECT_OMISSION_MARKER}.** {total} landed PR(s) "
            f"are in this finding's suspect set; the {len(shown)} above are the most "
            f"recently landed, capped at {cap} for legibility.\n"
            f"> **Ordering is landing recency, newest first.** A suspect that landed "
            f"earlier has had more subsequent PRs re-run this crate's lanes on top of "
            f"it, so it is the less likely culprit — a prior, not an exoneration. Past "
            f"~{cap} candidates, bisect the window rather than read the list.\n"
            f"> A suspect list this long means the last VERIFIED nightly backstop is far "
            f"behind HEAD — the window, not the bug, is what is large. The full set is "
            f"unchanged in the correlation and reproducible with "
            f"`scripts/ci_selection_alarm.py --run-id <id> --head-sha <sha> --dry-run`."
        )
    return "\n".join(lines)


def render_issue(f: Finding, run_id: str, head_sha: str, repo: str | None) -> tuple[str, str]:
    """Return (title, body) for a finding."""
    run_url = (
        f"https://github.com/{repo}/actions/runs/{run_id}" if repo else f"run {run_id}"
    )
    if f.kind == "crate":
        title = (
            f"[selection-alarm] crate `{f.crate}` failed the nightly full-matrix "
            f"but was selection-skipped in landed PR(s)"
        )
        lead = (
            f"The nightly FULL-matrix backstop failed a job for crate "
            f"**`{f.crate}`**, and that crate was **selection-skipped** in one or "
            f"more PRs that landed since the last VERIFIED nightly backstop. Per "
            f"the skip invariant (research/change-based-test-selection.md §2) a test is "
            f"skipped only if *provably* no change could affect it — so this "
            f"intersection is prima-facie evidence the non-interference proof was "
            f"wrong for `{f.crate}`."
        )
    else:
        title = (
            "[selection-alarm] nightly full-matrix shard failure needs crate-triage "
            "(change-based selection active in window)"
        )
        lead = (
            "The nightly FULL-matrix backstop failed a cross-crate/narrowed test "
            "shard whose job name does not encode the failing crate, and "
            "change-based selection NARROWED that shard (dropped some crates) in "
            "PRs that landed since the last VERIFIED nightly backstop. The failing "
            "crate cannot be named from the job alone — read the shard log below to "
            "identify it, "
            "then check whether it was among the skipped crates listed."
        )

    prs = render_suspects(f.suspect_prs, f.kind)
    jobs = "\n".join(f"- `{j}`" for j in f.failed_jobs)
    body = (
        f"{lead}\n\n"
        f"**Failed nightly job(s):**\n{jobs}\n\n"
        f"**Suspect landed PR(s)** — {len(f.suspect_prs)} total, selection-skipped "
        f"since the last VERIFIED nightly backstop, newest-landed first:\n"
        f"{prs}\n\n"
        f"**Nightly run:** {run_url} (head `{head_sha[:12]}`)\n\n"
        f"This is a DETECTOR signal, not a proof: a nightly failure can also be a "
        f"flake or an unrelated regression. Verify against the run log before "
        f"concluding a selection bug — but if it IS one, the selector's ownership "
        f"map / reverse-closure (scripts/ci_select.py, ci/path-ownership.toml) let "
        f"`{f.crate or 'the crate'}` be skipped when it should not have been; fix "
        f"that and add a golden test.\n\n"
        f"{key_marker(f.key)}\n\n"
        f"> \U0001f916 SPARQ agent — auto-filed by the change-based-selection "
        f"nightly alarm (scripts/ci_selection_alarm.py, bead sq-va7at). Reconcile "
        f"into a **P1** bead. [OPUS-4.8]"
    )
    return title, body


# --------------------------------------------------------------------------- #
# gh / git / cargo I/O (real, non-hermetic)
# --------------------------------------------------------------------------- #
def _run(cmd: list[str], cwd: str | None = None, check: bool = True) -> str:
    try:
        proc = subprocess.run(
            cmd, cwd=cwd, capture_output=True, text=True, check=check, timeout=300
        )
    except (subprocess.CalledProcessError, subprocess.TimeoutExpired, OSError) as exc:
        stderr = getattr(exc, "stderr", "") or ""
        raise AlarmError(f"command failed: {' '.join(cmd)}: {exc}\n{stderr}") from exc
    return proc.stdout


def fetch_run_conclusion(run_id: str, repo: str) -> str:
    """The RUN-level conclusion of `run_id`, for the paths where the caller does not
    already have it (the alarm's own workflow_dispatch). Fail-loud: an unreadable
    run conclusion leaves us unable to tell an anomaly from a quiet no-op."""
    out = _run(
        ["gh", "api", f"repos/{repo}/actions/runs/{run_id}", "--jq", ".conclusion // \"\""]
    )
    return out.strip()


def fetch_run_jobs(run_id: str, repo: str) -> list[tuple[str, str]]:
    """(job name, conclusion) for every job of `run_id`, via `gh api --paginate`.

    `--paginate`, never `--limit`: a nightly carries ~84 jobs and a truncated job list
    would under-report both the failures (masking an alarm) and the shard coverage
    (fabricating a window bound)."""
    out = _run(
        [
            "gh", "api", "--paginate",
            f"repos/{repo}/actions/runs/{run_id}/jobs",
            "--jq", ".jobs[] | [.name, (.conclusion // \"\")] | @tsv",
        ]
    )
    jobs: list[tuple[str, str]] = []
    for line in out.splitlines():
        if not line.strip():
            continue
        name, _, conclusion = line.partition("\t")
        jobs.append((name, conclusion.strip()))
    return jobs


def gather_failed_jobs(run_id: str, repo: str, run_conclusion: str = "") -> list[str]:
    """Names of the nightly run's jobs whose conclusion is a GENUINE failure (design:
    the failed-job set to correlate).

    `run_conclusion` is the RUN-level aggregate, used ONLY to decide whether an empty
    result is anomalous — see STRICT_EMPTY_CONCLUSIONS. The per-JOB filter is
    unchanged: cancelled siblings of a fail-fast matrix are not failures and never
    enter the correlation."""
    failed = [n for n, c in fetch_run_jobs(run_id, repo) if c in FAILED_CONCLUSIONS]
    # A run GitHub concluded `failure`/`timed_out` MUST contain a failed job, so an
    # empty set there is anomalous (jobs-API filter=latest re-run race, shape
    # surprise) and must be distinguished from "no correlation found" to prevent a
    # silent exit 0 masking a real failure. A `cancelled` (or `success`) run may
    # legitimately have none — that is the quiet no-op, handled by the caller.
    if not failed and run_conclusion.strip() in STRICT_EMPTY_CONCLUSIONS:
        raise AlarmError(
            "gather_failed_jobs: empty failed-job set on a "
            f"{run_conclusion.strip() or 'conclusion-unknown'}-concluded run "
            f"(run_id={run_id}); jobs-API shape surprise or filter=latest re-run "
            "race — cannot correlate, refusing to exit 0"
        )
    return failed


def list_scheduled_runs(
    repo: str, workflow_file: str, event: str, limit: int
) -> list[tuple[str, str, str, str]]:
    """(run_id, head_sha, run-level conclusion, created_at) for the most recent
    COMPLETED `event` runs of `workflow_file`, newest first, at most `limit`.

    NOTE the absent `status=success`: filtering the LIST by the run-level aggregate is
    the #5025 bug itself. A run whose every test shard passed can still aggregate to
    `cancelled` — one job cancelled at its timeout ceiling is enough, and ci.yml's
    advisory cargo-mutants sweep runs at 120/360min caps — so the old query threw
    away exactly the runs that could have bounded the window. `status=completed`
    keeps every terminal run and lets classify_run judge it on its jobs."""
    per_page = max(1, min(limit, 100))
    out = _run(
        [
            "gh", "api",
            f"repos/{repo}/actions/workflows/{workflow_file}/runs"
            f"?event={event}&status=completed&per_page={per_page}",
            "--jq",
            ".workflow_runs[] | [(.id|tostring), .head_sha, (.conclusion // \"\"), "
            "(.created_at // \"\")] | @tsv",
        ],
        check=True,
    )
    runs: list[tuple[str, str, str, str]] = []
    for line in out.splitlines():
        if not line.strip():
            continue
        parts = line.split("\t")
        if len(parts) < 4:
            # Fail-loud (invariant 1): a shape surprise here would silently shorten
            # the scan, and a shortened scan reports a staler backstop than reality.
            raise AlarmError(f"unexpected runs-API row (expected 4 fields): {line!r}")
        runs.append(tuple(p.strip() for p in parts[:4]))  # type: ignore[arg-type]
    return runs[:limit]


def find_window_bound(
    repo: str, workflow_file: str, event: str, limit: int = BOUND_SCAN_RUNS
) -> tuple[RunHealth | None, list[RunHealth]]:
    """Scan recent scheduled runs newest-first for one that may bound the correlation
    window (classify_run). Returns (bound or None, every run examined).

    Stops at the FIRST run that qualifies — that is the tightest honest bound, and it
    keeps the cost at one job-list call per rejected run. Every examined run is
    returned either way so render_nightly_health can state what was rejected and
    why; a scan that reported only its answer could not distinguish "the backstop is
    healthy and recent" from "the backstop has been red for a month"."""
    if not repo:
        raise AlarmError(
            "--repo (or $GITHUB_REPOSITORY) required to scan the scheduled runs"
        )
    scanned: list[RunHealth] = []
    for run_id, head_sha, conclusion, created_at in list_scheduled_runs(
        repo, workflow_file, event, limit
    ):
        health = classify_run(
            run_id, head_sha, conclusion, created_at, fetch_run_jobs(run_id, repo)
        )
        scanned.append(health)
        if health.bounds:
            return health, scanned
    return None, scanned


_PR_RE = re.compile(r"\(#(\d+)\)\s*$")


def window_commits(
    base_sha: str | None, head_sha: str, lookback_hours: int, repo_root: str,
    since: str | None = None,
) -> list[tuple[str, int | None]]:
    """(sha, pr_number) for each landed commit in the window, newest first.

    Window = (last VERIFIED nightly backstop, HEAD] when base_sha is known. Otherwise
    a time bound: `since` (the oldest scheduled run the bound scan examined — the
    HORIZON of what we looked at, used when no examined run verified the matrix), or
    failing that `lookback_hours` for a true cold start with no scheduled run at all.
    Both fallbacks are disclosed by the caller; neither is ever chosen over a real
    coverage bound, because a shorter window under-reports suspects."""
    if base_sha:
        rng = [f"{base_sha}..{head_sha}"]
    elif since:
        rng = [head_sha, f"--since={since}"]
    else:
        rng = [head_sha, f"--since={lookback_hours} hours ago"]
    out = _run(
        ["git", "log", "--no-merges", "--format=%H%x00%s", *rng],
        cwd=repo_root,
    )
    commits: list[tuple[str, int | None]] = []
    for line in out.splitlines():
        if not line.strip():
            continue
        sha, _, subject = line.partition("\x00")
        m = _PR_RE.search(subject)
        commits.append((sha.strip(), int(m.group(1)) if m else None))
    return commits


def commit_changed_paths(sha: str, repo_root: str) -> list[str]:
    """Paths a landed commit changed = `git diff --no-renames <sha>^ <sha>` (the
    PR's net diff on a squash-merged linear main; matches ci_select's --no-renames
    conservatism)."""
    out = _run(
        ["git", "diff", "--name-only", "--no-renames", f"{sha}^", sha],
        cwd=repo_root,
    )
    return [ln for ln in out.splitlines() if ln.strip()]


def build_window(
    commits: list[tuple[str, int | None]], meta: dict, map_entries: list[dict],
    ci_select, repo_root: str,
) -> tuple[list[WindowCommit], set[str], list[str]]:
    """Replay the selection decision for each landed commit. Returns
    (window, members, replay_errors). A replay error is collected (not swallowed)
    so main() can fail LOUD while still filing the findings it could compute."""
    ws = ci_select.parse_workspace(meta)
    members = set(ws.members)
    window: list[WindowCommit] = []
    replay_errors: list[str] = []
    for sha, pr in commits:
        try:
            changed = commit_changed_paths(sha, repo_root)
            sel = ci_select.select(changed, meta, map_entries)
            skipped = (
                frozenset(members - set(sel.affected))
                if sel.mode == "selected"
                else frozenset()
            )
        except Exception as exc:  # noqa: BLE001 — collect, don't mask (fail-loud later)
            replay_errors.append(f"{sha[:12]}: {exc}")
            continue
        window.append(WindowCommit(sha=sha, pr=pr, skipped=skipped))
    return window, members, replay_errors


# --------------------------------------------------------------------------- #
# Issue minting (mirror flow-on's CI guard + idempotency + label upsert)
# --------------------------------------------------------------------------- #
CI_ENV_VARS = ("GITHUB_ACTIONS", "CI")
ALLOW_LOCAL_ENV_VAR = "SELECTION_ALARM_ALLOW_LOCAL"


def _is_truthy(val: str | None) -> bool:
    return val is not None and val.strip().lower() in {"1", "true", "yes", "on"}


def require_ci(env: dict[str, str] | None = None) -> None:
    e = os.environ if env is None else env
    if any(_is_truthy(e.get(v)) for v in CI_ENV_VARS) or _is_truthy(e.get(ALLOW_LOCAL_ENV_VAR)):
        return
    raise AlarmError(
        "refusing to file GitHub issues outside CI (no GITHUB_ACTIONS / CI env "
        f"set). Use --dry-run to preview, or set {ALLOW_LOCAL_ENV_VAR}=1 to force."
    )


_ENSURED_LABELS: set[str] = set()


def ensure_label(label: str) -> None:
    if label in _ENSURED_LABELS:
        return
    try:
        _run(["gh", "label", "create", label, "--force"], check=True)
    except AlarmError:
        pass  # best-effort upsert; create_issue surfaces a genuine failure
    _ENSURED_LABELS.add(label)


def open_issue_exists(key: str, repo: str) -> bool:
    # Idempotency: list ALL open selection-alarm issues and EXACT-match the body
    # marker. We deliberately do NOT pass `--search` — a triage key contains
    # punctuation (parens/slashes) that gh's search tokeniser handles unreliably,
    # which could MISS an existing issue and break dedupe (mint a duplicate). The
    # deduped label keeps the open set tiny, so listing 100 is cheap.
    marker = key_marker(key)
    out = _run(
        [
            "gh", "issue", "list", "--repo", repo, "--state", "open",
            "--label", "selection-alarm", "--json", "number,body", "--limit", "100",
        ],
        check=False,
    )
    try:
        issues = json.loads(out or "[]")
    except json.JSONDecodeError:
        return False
    return any(marker in (i.get("body") or "") for i in issues)


def create_issue(title: str, body: str, labels: list[str], repo: str) -> str:
    args = ["gh", "issue", "create", "--repo", repo, "--title", title, "--body", body]
    for lbl in labels:
        ensure_label(lbl)
        args += ["--label", lbl]
    return _run(args).strip()


# --------------------------------------------------------------------------- #
# CLI
# --------------------------------------------------------------------------- #
def _parse_args(argv: list[str] | None) -> argparse.Namespace:
    p = argparse.ArgumentParser(
        description="Nightly change-based-selection bug alarm (design §6.1)."
    )
    p.add_argument("--run-id", help="the failed nightly CI run id "
                                    "(required unless --nightly-health)")
    p.add_argument("--head-sha", help="the nightly run's head SHA "
                                      "(required unless --nightly-health)")
    p.add_argument("--run-conclusion", default="",
                   help="the nightly run's RUN-level conclusion. NOT an admission "
                        "filter (#4965) — it only decides whether an EMPTY "
                        "genuinely-failed-job set is anomalous (STRICT_EMPTY_"
                        "CONCLUSIONS) or a quiet no-op. Empty => looked up via gh on "
                        "the non-hermetic path, and STRICT on the hermetic one.")
    p.add_argument("--repo", default=os.environ.get("GITHUB_REPOSITORY"),
                   help="owner/name (default $GITHUB_REPOSITORY)")
    p.add_argument("--event", default="schedule",
                   help="the nightly trigger event whose runs bound the window")
    p.add_argument("--workflow-file", default="ci.yml",
                   help="workflow file whose runs bound the window")
    p.add_argument("--lookback-hours", type=int, default=25,
                   help="cold-start fallback window when NO completed scheduled run "
                        "exists at all (never used to shorten a real window)")
    p.add_argument("--bound-scan-runs", type=int, default=BOUND_SCAN_RUNS,
                   help="how many recent completed scheduled runs to examine for a "
                        "window bound / for the nightly health report (#5025)")
    p.add_argument("--nightly-health", action="store_true",
                   help="report the backstop's own health (which recent scheduled "
                        "runs verified the full matrix, which jobs are persistently "
                        "red) and exit; no correlation, no issues")
    p.add_argument("--repo-root", help="repo root (default: git toplevel)")
    p.add_argument("--dry-run", action="store_true", help="print findings; no gh writes")
    # Hermetic inputs (tests / offline):
    p.add_argument("--failed-jobs-file", help="hermetic: one failed job name per line")
    p.add_argument("--metadata-file", help="hermetic: cargo metadata JSON snapshot")
    args = p.parse_args(argv)
    if not args.nightly_health and (not args.run_id or not args.head_sha):
        p.error("--run-id and --head-sha are required unless --nightly-health is given")
    return args


def _resolve_repo_root(explicit: str | None) -> str:
    if explicit:
        return explicit
    try:
        return _run(["git", "rev-parse", "--show-toplevel"]).strip()
    except AlarmError:
        return str(REPO_ROOT)


def append_step_summary(text: str) -> None:
    """Best-effort append to $GITHUB_STEP_SUMMARY (absent locally; never fatal)."""
    summary = os.environ.get("GITHUB_STEP_SUMMARY")
    if not summary:
        return
    try:
        with open(summary, "a") as fh:
            fh.write(text if text.endswith("\n") else text + "\n")
    except OSError:
        pass


def resolve_window_bound(
    args: argparse.Namespace, repo: str
) -> tuple[str | None, str | None]:
    """Scan the recent scheduled runs, STATE the backstop's health, and return the
    (base_sha, since) pair that bounds the correlation window.

    Three outcomes, all disclosed — never a silent choice:
      * a run VERIFIED the full matrix  => (its head_sha, None): a real coverage bound;
      * runs exist, none verified       => (None, oldest examined run's created_at) and
        a ::warning::. The backstop is stale; the window is the horizon we examined,
        which is a TRUE report of that staleness. #5025's ⚠️ forbids the alternative
        (shorten it and the report becomes a lie);
      * no completed scheduled run      => (None, None) and the cold-start ::notice::,
        the pre-existing --lookback-hours fallback.
    """
    if not repo:
        raise AlarmError("--repo (or $GITHUB_REPOSITORY) required to bound the window")
    bound, scanned = find_window_bound(
        repo, args.workflow_file, args.event, args.bound_scan_runs
    )
    report = render_nightly_health(scanned, bound)
    print(report)
    append_step_summary(report)

    if bound is not None:
        return bound.head_sha, None
    if scanned:
        oldest = scanned[-1].created_at or None
        print(
            f"::warning::selection-alarm: none of the {len(scanned)} scheduled "
            f"{args.workflow_file} run(s) examined verified the full matrix — the "
            "backstop is STALE. The correlation window is HORIZON-bounded at "
            f"{oldest or f'{args.lookback_hours}h ago'}, not coverage-bounded: "
            "suspects that landed earlier are NOT reported, and no suspect list here "
            "should be read as complete. Fix the nightly (see the health table above)."
        )
        return None, oldest
    print("::notice::selection-alarm: no completed scheduled run found; using "
          f"{args.lookback_hours}h cold-start fallback window")
    return None, None


def run_alarm(args: argparse.Namespace) -> tuple[list[Finding], list[str], str]:
    """Gather inputs, replay the window, correlate. Returns
    (findings, replay_errors, repo). Raises AlarmError on a TOTAL blocker."""
    ci_select = _load_ci_select()
    repo = args.repo or ""
    repo_root = _resolve_repo_root(args.repo_root)

    # The RUN-level aggregate. It is NOT the admission filter (#4965: it is lossy —
    # one fail-fast cancellation turns a run full of real failures into `cancelled`).
    # It decides ONE thing: whether an EMPTY genuinely-failed-job set is anomalous.
    run_conclusion = (args.run_conclusion or "").strip()
    if not run_conclusion and not args.failed_jobs_file and repo:
        # The alarm's own workflow_dispatch path has no event payload to read it from.
        run_conclusion = fetch_run_conclusion(args.run_id, repo)

    # Failed jobs — hermetic file or gh.
    if args.failed_jobs_file:
        failed_jobs = [
            ln.strip() for ln in Path(args.failed_jobs_file).read_text().splitlines()
            if ln.strip()
        ]
    else:
        if not repo:
            raise AlarmError("--repo (or $GITHUB_REPOSITORY) required for the gh path")
        failed_jobs = gather_failed_jobs(args.run_id, repo, run_conclusion)
    if not failed_jobs:
        # THE PRECISE ADMISSION (#4965). Analysis and admission now agree: a job is
        # a failure iff its conclusion is in FAILED_CONCLUSIONS, at BOTH ends.
        if run_conclusion in STRICT_EMPTY_CONCLUSIONS:
            # A failure/timed_out-concluded run with no failed job is anomalous —
            # an empty correlation must stay distinguishable from anomalous-empty
            # so a silent exit 0 can never mask a real failure.
            raise AlarmError(
                "run_alarm: empty failed-jobs set on a "
                f"{run_conclusion or 'conclusion-unknown'}-concluded run — anomalous "
                "(jobs-API shape surprise, re-run race, or empty --failed-jobs-file). "
                "Refusing to exit 0."
            )
        # The NOISE half: a `cancelled` run whose jobs contain no genuine failure is
        # the COMMON case (fail-fast cancels the matrix). Nothing failed, so there is
        # no correlation input and no finding to make — exit quiet, not loud and not
        # green-with-a-fake-finding. Returning here also skips the cargo-metadata load
        # and the per-commit git replay, which have nothing to work on.
        print(
            f"selection-alarm: run {args.run_id} concluded '{run_conclusion}' and no "
            f"job concluded {sorted(FAILED_CONCLUSIONS)} — nothing genuinely failed, "
            "so there is no correlation input. No alarm."
        )
        return [], [], repo

    # cargo metadata (member set + reverse closure for the replay).
    meta = ci_select.load_metadata(args.metadata_file, repo_root)
    map_file = os.path.join(repo_root, "ci", "path-ownership.toml")
    map_entries = ci_select.load_ownership_map(
        map_file if os.path.exists(map_file) else None
    )

    # Window of landed commits since the last VERIFIED nightly backstop (#5025), or a
    # disclosed time fallback. The hermetic path has no runs API to scan.
    base: str | None = None
    since: str | None = None
    if not args.failed_jobs_file:
        base, since = resolve_window_bound(args, repo)
    commits = window_commits(base, args.head_sha, args.lookback_hours, repo_root,
                             since=since)
    window, members, replay_errors = build_window(
        commits, meta, map_entries, ci_select, repo_root
    )

    findings = correlate(failed_jobs, window, members)
    return findings, replay_errors, repo


def main(argv: list[str] | None = None) -> int:
    args = _parse_args(argv)
    if args.nightly_health:
        # REPORT-ONLY (#5025 acceptance item 1). Answers "is the full matrix actually
        # green, and if not which jobs are persistently red?" without correlating or
        # filing anything, so the question can be re-asked on demand instead of being
        # answered once in an issue body that goes stale the next night.
        try:
            bound, scanned = find_window_bound(
                args.repo or "", args.workflow_file, args.event, args.bound_scan_runs
            )
        except AlarmError as exc:
            print(f"::error::selection-alarm infrastructure error: {exc}", file=sys.stderr)
            return 1
        report = render_nightly_health(scanned, bound)
        print(report)
        append_step_summary(report)
        return 0
    try:
        findings, replay_errors, repo = run_alarm(args)
    except AlarmError as exc:
        # FAIL-LOUD (invariant 1): a total blocker must red the run, never exit 0.
        print(f"::error::selection-alarm infrastructure error: {exc}", file=sys.stderr)
        return 1

    if not findings:
        print("selection-alarm: no nightly failure correlates with a selection "
              "skip in the window — no alarm.")
    else:
        if not args.dry_run:
            require_ci_error = None
            try:
                require_ci()
            except AlarmError as exc:
                require_ci_error = exc
            if require_ci_error is not None:
                print(f"::error::{require_ci_error}", file=sys.stderr)
                return 1
        filed = skipped = 0
        for f in findings:
            title, body = render_issue(f, args.run_id, args.head_sha, repo or None)
            if args.dry_run:
                print(f"[dry-run] WOULD file (kind={f.kind}, key={f.key}):")
                print(f"          title: {title}")
                print(f"          jobs : {', '.join(f.failed_jobs)}")
                print(f"          PRs  : {', '.join(_pr_ref(wc) for wc in f.suspect_prs)}")
                continue
            if open_issue_exists(f.key, repo):
                print(f"selection-alarm: skip (open issue exists) key={f.key}")
                skipped += 1
                continue
            url = create_issue(title, body, list(dict.fromkeys(BASE_LABELS)), repo)
            print(f"selection-alarm: filed {url} (kind={f.kind}, key={f.key})")
            filed += 1
        if not args.dry_run:
            print(f"selection-alarm: done — {filed} filed, {skipped} already open.")
            append_step_summary(f"### selection-alarm\n\n- filed: {filed}\n"
                                f"- skipped (already open): {skipped}\n")

    # FAIL-LOUD (invariant 1): findings we COULD compute are filed above; if any
    # landed commit could not be replayed, red the run so the gap is investigated.
    if replay_errors:
        print("::error::selection-alarm: could not replay selection for "
              f"{len(replay_errors)} landed commit(s) — the correlation window is "
              "INCOMPLETE and a real selection bug may be unreported:",
              file=sys.stderr)
        for e in replay_errors:
            print(f"::error::  {e}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
