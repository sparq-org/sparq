#!/usr/bin/env python3
# ci_mergegroup_poles — re-derive the per-workflow `merge_group` WALL-CLOCK POLE ranking
# on demand. 🤖 SPARQ agent. [OPUS-5] issue #5250.
#
# WHY THIS EXISTS. Issue #5250 reports a specific, repeating failure: issue #3005 sized the
# docs/deploy fast lane off a "CodeQL-rust ~20-40 min merge_group cost" that was already
# refuted by a measured note in `.github/workflows/codeql.yml`'s own header — the figure
# predates the `build-mode: none` buildless migration. A stale duration outlived its
# validity and kept steering effort at a lane that is not the pole.
#
# That is not a one-off. Every duration in research/ci-mergequeue-speedup-2026-07.md is a
# hand-run `gh run list` snapshot, and the record is now annotated with three different
# flavours of decay: §2.1 is banner-marked "STALE AS A BASELINE", most of its rows read
# "not re-sampled", and two lanes read "never sampled". §6 states the gap outright — the
# 2026-08-01 lane-set re-verification was STRUCTURAL, and "what this method does NOT give
# is durations". The lane INVENTORY is pinned against drift by
# scripts/tests/test_mergequeue_lane_inventory.py; the lane DURATIONS are pinned by
# nothing, so they rot silently and the next lever gets mis-sized off them again.
#
# So the fix #5250 actually needs is not another hand-measured number to paste into the
# epic — that is the thing that went stale — but a REPRODUCIBLE derivation. Numbers this
# emits are stamped with their sample window and can be re-run in one command, so a
# reader can always ask "is this still true?" instead of trusting a date.
#
# WHAT IT COMPUTES. Over the last N `merge_group` runs: per-workflow n / median / p90 /
# max entry wall, ranked by median, plus the top-3 poles. `--jobs <workflow.yml>`
# decomposes ONE workflow's merge_group runs to the job level — the scope §2.1's own
# conclusion says a re-sample belongs in ("Any re-sample should therefore be scoped to
# ci.yml's job level, not spent re-timing the surviving sub-minute lanes"), because the
# entry wall is `max` over lanes and the surviving non-poles cannot move it.
#
# FIVE THINGS IT GETS RIGHT THAT A BARE `gh run list` PASS DOES NOT. Each is a real
# mis-measurement observed in this repo's own history, and each is pinned by a named test
# in scripts/tests/test_ci_mergegroup_poles.py:
#
#   1. EVENT FILTER. Only `event == merge_group` runs count. A workflow's push/PR runs
#      have a different lane set and different cache warmth; mixing them measures
#      something that is not the queue.
#   2. SUCCESS FILTER. Only `conclusion == success` runs count. This is the big one: the
#      merge queue CANCELS waves mid-flight (the record observed push CI cancelled at
#      928 s and at 311 s), and a cancelled run's wall clock is a truncated lower bound.
#      Including them biases every median DOWNWARD, i.e. exactly toward "no pole here",
#      which is the direction that loses a pole rather than inventing one.
#   3. `max`, NOT `sum`. The entry wall is the SLOWEST lane, not the total. Summing lane
#      medians overstates the wall several-fold and makes every lane look worth cutting;
#      the ranking and the reported wall are both max-based, and `--json` exposes
#      `entry_wall_median` as such so a caller cannot re-derive a sum by accident.
#   4. STRUCTURAL vs OBSERVED RECONCILIATION. A workflow can declare `merge_group` in its
#      `on:` block and still produce NO check-run — `codeql.yml` did exactly this while
#      `disabled_manually`, and §6 flags it as a correction "a run-history pass alone
#      would miss". Such a lane is reported as UNOBSERVED, never as a fast 0 m lane. The
#      converse (runs from a workflow with no current trigger — a rename or a removed
#      trigger with runs still inside the window) is reported too, so the ranking cannot
#      silently drop or silently invent a lane.
#   5. FAIL-LOUD ON AN EMPTY POPULATION. Zero usable runs exits 2. Reporting a confident
#      pole ranking over nothing is the one failure mode this file exists to prevent, and
#      it is the same posture as the sibling detectors (ci_execution_latency_alarm.py,
#      heavy_set_alarm.py).
#
# NOT A GATE, NOT A DETECTOR, NO WRITE AUTHORITY. This is an on-demand measurement tool: no
# workflow runs it on a schedule, it files no issue, it needs only `actions: read`, and it
# has no `--fix` path. It therefore needs no `.github/advisory-registry.json` entry (it
# never produces a check-run). Its hermetic self-test IS gated, as a step in the existing
# `docs-quality quick-gates` job, so the estimator cannot rot unnoticed.
#
# THE LANE-SET DEFINITION IS NOT FORKED. `triggers_on_merge_group` below is the stdlib
# comment-aware parse used by scripts/tests/test_mergequeue_cache_posture.py, which
# test_mergequeue_lane_inventory.py already imports rather than re-implements so the
# suites "can never disagree about what triggers on merge_group means". This file cannot
# import from scripts/tests/ (a production tool must not depend on a test module), so
# instead the test asserts SET EQUALITY between this parser and the canonical one over the
# real workflow tree — same invariant, enforced rather than assumed.
#
# PyYAML is deliberately not used: it is absent from some execution environments here (the
# sibling alarm guards an optional import for this reason), and YAML 1.1 resolves a bare
# `on:` key to the boolean `True`, a trap §6 calls out explicitly. Stdlib only.
#
# Usage:
#   python3 scripts/ci_mergegroup_poles.py                       # workflow-level ranking
#   python3 scripts/ci_mergegroup_poles.py --limit 400 --json
#   python3 scripts/ci_mergegroup_poles.py --jobs ci.yml         # job-level decomposition
#   python3 scripts/ci_mergegroup_poles.py --self-test           # hermetic, no gh/network
#
# Exit codes: 0 ok; 1 usage/argument error; 2 measurement infrastructure failure
# (gh failure, unparseable payload, or an EMPTY sample — never a silent all-clear).

from __future__ import annotations

import argparse
import datetime as dt
import json
import re
import subprocess
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
WORKFLOWS_DIR = REPO_ROOT / ".github" / "workflows"

DEFAULT_LIMIT = 250          # matches the §6 method's window, so results are comparable
DEFAULT_JOB_RUNS = 20        # runs decomposed in --jobs mode (one API call each)
RUNS_PAGE_CAP = 20           # 100/page; a guard against an unbounded pager loop
TOP_POLES = 3                # #5250 asks for the top-3
MIN_SAMPLE_WARN = 5          # below this, a median is reported but flagged thin


class PoleError(RuntimeError):
    """Measurement infrastructure failed. Always fatal — never degraded to a result."""


# ---------------------------------------------------------------------------------
# pure helpers
# ---------------------------------------------------------------------------------
def _ts(value: str) -> dt.datetime:
    try:
        return dt.datetime.fromisoformat(str(value).replace("Z", "+00:00"))
    except ValueError as exc:
        raise PoleError(f"unparseable timestamp {value!r}: {exc}") from exc


def percentile(values: list[float], q: float) -> float:
    """Nearest-rank percentile — the same estimator as ci_execution_latency_alarm.py.

    Both the median and the p90 come from this one estimator, so every figure reported
    is an ACTUALLY OBSERVED run duration rather than an interpolation between two runs.
    That matters when a table is going to be read as "a run took this long".
    """
    if not values:
        raise PoleError("percentile over an empty sample")
    ordered = sorted(values)
    idx = min(len(ordered) - 1, int(round(q * (len(ordered) - 1))))
    return ordered[idx]


def fmt_minutes(seconds: float | None) -> str:
    return "—" if seconds is None else f"{seconds / 60.0:.1f} m"


def run_duration(run: dict) -> float | None:
    """Wall clock of one run, in seconds, or None if it cannot be established.

    `run_started_at` .. `updated_at` is the same span the §6 method used. `updated_at` is
    the last mutation of the run object, which for a terminal run is its completion.
    """
    started, updated = run.get("run_started_at"), run.get("updated_at")
    if not (started and updated):
        return None
    delta = (_ts(updated) - _ts(started)).total_seconds()
    return delta if delta >= 0 else None


def usable_runs(runs: list[dict]) -> list[dict]:
    """Filters 1 and 2 — the two that decide whether the answer means anything.

    A run counts iff it is a `merge_group` run AND concluded `success`. Both filters are
    applied HERE, in one place, rather than trusted to the query string: the `gh api`
    listing is filtered server-side too, but a server-side filter that silently stops
    being applied (a typo'd query param is simply ignored by the API) would poison every
    figure downstream with no error. This is the belt to that braces.
    """
    return [
        r for r in runs
        if r.get("event") == "merge_group" and r.get("conclusion") == "success"
    ]


def summarize(runs: list[dict]) -> dict[str, dict]:
    """-> {workflow_file: {name, n, median, p90, max}} over ALREADY-FILTERED runs."""
    buckets: dict[str, dict] = {}
    for run in runs:
        path = str(run.get("path") or "")
        key = path.split("/")[-1] or "<unknown>"
        seconds = run_duration(run)
        if seconds is None:
            continue
        entry = buckets.setdefault(key, {"name": run.get("name") or key, "samples": []})
        entry["samples"].append(seconds)
    out: dict[str, dict] = {}
    for key, entry in buckets.items():
        samples = entry["samples"]
        if not samples:
            continue
        out[key] = {
            "workflow": key,
            "name": entry["name"],
            "n": len(samples),
            "median": percentile(samples, 0.50),
            "p90": percentile(samples, 0.90),
            "max": max(samples),
            "thin": len(samples) < MIN_SAMPLE_WARN,
        }
    return out


def rank_poles(stats: dict[str, dict]) -> list[dict]:
    """Lanes ranked by median DESC. Ties break on p90 then name, so the order is total
    and the output is reproducible across runs of the same sample."""
    return sorted(
        stats.values(),
        key=lambda s: (-s["median"], -s["p90"], s["workflow"]),
    )


def entry_wall(stats: dict[str, dict]) -> dict[str, float | None]:
    """The queue entry wall is `max` OVER LANES, not the sum of them (property 3).

    A caller that wants "how long does an entry take" must read this, not add up the
    table. Returned as an explicit field so the sum is never the convenient default.
    """
    if not stats:
        return {"entry_wall_median": None, "entry_wall_p90": None}
    return {
        "entry_wall_median": max(s["median"] for s in stats.values()),
        "entry_wall_p90": max(s["p90"] for s in stats.values()),
    }


# ---------------------------------------------------------------------------------
# structural lane set (stdlib; mirrors test_mergequeue_cache_posture.triggers_on_merge_group)
# ---------------------------------------------------------------------------------
def _is_comment(line: str) -> bool:
    return line.lstrip().startswith("#")


def triggers_on_merge_group(text: str) -> bool:
    """True iff the workflow's top-level `on:` block declares `merge_group`.

    The load-bearing comment-awareness is in the BLOCK-END scan: a comment at column 0
    (several workflows carry a top-level prose note about the 2026-07-18 directive that
    removed `merge_group`) would otherwise read as the next top-level key and truncate the
    block, hiding a trigger declared below it.

    The `_is_comment` filter on the trigger match itself is REDUNDANT — `^\\s+merge_group:`
    cannot match a comment line, whose first non-space character is `#`. It is kept only so
    this stays textually identical to the canonical parser in
    scripts/tests/test_mergequeue_cache_posture.py; diverging to drop a harmless guard
    would cost the "cannot disagree" property for nothing.
    """
    lines = text.split("\n")
    start = next(
        (i for i, l in enumerate(lines) if re.match(r'^(on|"on"):', l)),
        None,
    )
    if start is None:
        return False
    end = len(lines)
    for i in range(start + 1, len(lines)):
        line = lines[i]
        if line and not line[0].isspace() and not _is_comment(line):
            end = i
            break
    return any(
        re.match(r"^\s+merge_group:", l)
        for l in lines[start:end]
        if not _is_comment(l)
    )


def structural_lanes(workflows_dir: Path) -> set[str]:
    if not workflows_dir.is_dir():
        raise PoleError(f"workflows directory not found: {workflows_dir}")
    lanes = {
        p.name for p in sorted(workflows_dir.glob("*.yml"))
        if triggers_on_merge_group(p.read_text())
    }
    if not lanes:
        raise PoleError(f"no workflow in {workflows_dir} declares a merge_group trigger")
    return lanes


def reconcile(structural: set[str], stats: dict[str, dict]) -> dict[str, list[str]]:
    """Property 4. The two set differences are reported, never silently dropped.

    `unobserved` — declares the trigger, produced no successful run in the window. This
    is the `codeql.yml`-while-`disabled_manually` shape: trigger presence is NOT a
    check-run. Such a lane must never be rendered as a fast 0 m lane, because "cheap" and
    "absent" are opposite conclusions for a fast-lane decision.

    `untriggered` — produced runs but declares no trigger today: a rename, or a trigger
    removed while its runs are still inside the sample window. Its duration is real but
    its future cost is zero, so it is listed apart from the live ranking.
    """
    observed = set(stats)
    return {
        "unobserved": sorted(structural - observed),
        "untriggered": sorted(observed - structural),
    }


# ---------------------------------------------------------------------------------
# gh I/O
# ---------------------------------------------------------------------------------
def _gh(args: list[str]):
    try:
        out = subprocess.run(
            ["gh", *args], check=True, capture_output=True, text=True, timeout=180
        )
    except (subprocess.CalledProcessError, subprocess.TimeoutExpired, OSError) as exc:
        detail = getattr(exc, "stderr", "") or str(exc)
        raise PoleError(f"gh {' '.join(args[:2])} failed: {detail}") from exc
    try:
        return json.loads(out.stdout or "null")
    except json.JSONDecodeError as exc:
        raise PoleError(f"gh {' '.join(args[:2])}: unparseable JSON: {exc}") from exc


def fetch_merge_group_runs(repo: str, limit: int) -> list[dict]:
    """The last `limit` merge_group runs. Filtered server-side for cost, then re-filtered
    locally by `usable_runs` for correctness (see that docstring)."""
    runs: list[dict] = []
    page = 1
    while page <= RUNS_PAGE_CAP and len(runs) < limit:
        payload = _gh([
            "api",
            f"repos/{repo}/actions/runs?event=merge_group&status=completed"
            f"&per_page=100&page={page}",
        ])
        if not isinstance(payload, dict):
            raise PoleError("workflow-run listing is not an object")
        batch = payload.get("workflow_runs")
        if batch is None:
            raise PoleError("workflow-run listing carries no workflow_runs")
        runs.extend(batch)
        if len(batch) < 100:
            break
        page += 1
    return runs[:limit]


def fetch_jobs(repo: str, run_id: int) -> list[dict]:
    payload = _gh(["api", f"repos/{repo}/actions/runs/{run_id}/jobs?per_page=100"])
    if not isinstance(payload, dict) or payload.get("jobs") is None:
        raise PoleError(f"run {run_id}: jobs response carries no jobs")
    return payload["jobs"]


def job_duration(job: dict) -> float | None:
    started, completed = job.get("started_at"), job.get("completed_at")
    if not (started and completed):
        return None
    delta = (_ts(completed) - _ts(started)).total_seconds()
    return delta if delta >= 0 else None


def summarize_jobs(jobs: list[dict]) -> list[dict]:
    """Per-job-NAME stats across runs. Only `success` jobs count, for the same truncation
    reason as runs; a `skipped` job has no duration and must not be scored as instant,
    because selection-skipping is precisely what these lanes do on most entries."""
    buckets: dict[str, list[float]] = {}
    for job in jobs:
        if job.get("conclusion") != "success":
            continue
        seconds = job_duration(job)
        if seconds is None:
            continue
        buckets.setdefault(str(job.get("name") or "<unnamed>"), []).append(seconds)
    out = [
        {
            "job": name,
            "n": len(samples),
            "median": percentile(samples, 0.50),
            "p90": percentile(samples, 0.90),
            "max": max(samples),
        }
        for name, samples in buckets.items()
    ]
    return sorted(out, key=lambda s: (-s["median"], -s["p90"], s["job"]))


# ---------------------------------------------------------------------------------
# rendering
# ---------------------------------------------------------------------------------
def render_workflow_report(result: dict) -> str:
    lines: list[str] = []
    lines.append(
        f"# merge_group wall-clock poles — {result['repo']} "
        f"(n={result['sample_size']} successful merge_group runs)"
    )
    lines.append("")
    lines.append(
        "Entry wall is `max` over lanes, NOT the sum of the table: "
        f"median {fmt_minutes(result['entry_wall_median'])}, "
        f"p90 {fmt_minutes(result['entry_wall_p90'])}."
    )
    lines.append("")
    lines.append("| # | workflow | n | median | p90 | max |")
    lines.append("|---|---|---|---|---|---|")
    for i, s in enumerate(result["ranking"], 1):
        flag = " ⚠ thin" if s["thin"] else ""
        lines.append(
            f"| {i} | {s['workflow']} | {s['n']}{flag} | {fmt_minutes(s['median'])} "
            f"| {fmt_minutes(s['p90'])} | {fmt_minutes(s['max'])} |"
        )
    lines.append("")
    poles = result["ranking"][:TOP_POLES]
    if poles:
        lines.append(f"**Top-{len(poles)} poles:** " + ", ".join(
            f"{s['workflow']} ({fmt_minutes(s['median'])} median)" for s in poles
        ))
        lines.append("")
    if result["unobserved"]:
        lines.append(
            "**Declares `merge_group` but produced NO successful run in this window** — "
            "trigger presence is not a check-run (a disabled or always-skipped lane looks "
            "identical here); do NOT read these as cheap: "
            + ", ".join(result["unobserved"])
        )
        lines.append("")
    if result["untriggered"]:
        lines.append(
            "**Ran in this window but declares no `merge_group` trigger today** "
            "(renamed, or trigger removed mid-window — real past cost, zero future cost): "
            + ", ".join(result["untriggered"])
        )
        lines.append("")
    lines.append(
        "Re-derive with `python3 scripts/ci_mergegroup_poles.py`. Figures are a live "
        "sample of the window above, not a stored baseline."
    )
    return "\n".join(lines)


def render_job_report(result: dict) -> str:
    lines: list[str] = []
    lines.append(
        f"# {result['workflow']} job-level decomposition — {result['repo']} "
        f"({result['runs_decomposed']} merge_group runs)"
    )
    lines.append("")
    lines.append("| # | job | n | median | p90 | max |")
    lines.append("|---|---|---|---|---|---|")
    for i, s in enumerate(result["jobs"], 1):
        lines.append(
            f"| {i} | {s['job']} | {s['n']} | {fmt_minutes(s['median'])} "
            f"| {fmt_minutes(s['p90'])} | {fmt_minutes(s['max'])} |"
        )
    lines.append("")
    lines.append(
        "Jobs run in PARALLEL unless chained by `needs:`, so this table ranks cost, not "
        "the critical path — a fast job that gates a slow one still lengthens the wall. "
        "Read it with the workflow's `needs:` graph to locate a serial chain."
    )
    return "\n".join(lines)


# ---------------------------------------------------------------------------------
# drivers
# ---------------------------------------------------------------------------------
def measure_workflows(repo: str, limit: int, workflows_dir: Path) -> dict:
    raw = fetch_merge_group_runs(repo, limit)
    runs = usable_runs(raw)
    if not runs:
        raise PoleError(
            f"no successful merge_group run found in the last {len(raw)} completed runs "
            "— refusing to report a pole ranking over an empty sample"
        )
    stats = summarize(runs)
    if not stats:
        raise PoleError("no merge_group run carried a usable duration")
    result = {
        "repo": repo,
        "sample_size": len(runs),
        "runs_listed": len(raw),
        "ranking": rank_poles(stats),
        **entry_wall(stats),
        **reconcile(structural_lanes(workflows_dir), stats),
    }
    result["top_poles"] = [s["workflow"] for s in result["ranking"][:TOP_POLES]]
    return result


def measure_jobs(repo: str, workflow: str, limit: int, job_runs: int) -> dict:
    raw = fetch_merge_group_runs(repo, limit)
    runs = [r for r in usable_runs(raw) if str(r.get("path") or "").endswith(f"/{workflow}")]
    if not runs:
        raise PoleError(
            f"no successful merge_group run for {workflow} in the last {len(raw)} runs"
        )
    selected = runs[:job_runs]
    jobs: list[dict] = []
    for run in selected:
        jobs.extend(fetch_jobs(repo, run["id"]))
    summary = summarize_jobs(jobs)
    if not summary:
        raise PoleError(f"{workflow}: no successful job carried a usable duration")
    return {
        "repo": repo,
        "workflow": workflow,
        "runs_decomposed": len(selected),
        "jobs": summary,
    }


def run(args: argparse.Namespace) -> int:
    if args.jobs:
        result = measure_jobs(args.repo, args.jobs, args.limit, args.job_runs)
        print(json.dumps(result, indent=2) if args.json else render_job_report(result))
        return 0
    result = measure_workflows(args.repo, args.limit, Path(args.workflows_dir))
    print(json.dumps(result, indent=2) if args.json else render_workflow_report(result))
    return 0


# ---------------------------------------------------------------------------------
# hermetic fixtures + self-test
# ---------------------------------------------------------------------------------
def _run_obj(path: str, minutes: float, *, event: str = "merge_group",
             conclusion: str = "success", name: str | None = None,
             run_id: int = 1) -> dict:
    start = dt.datetime(2026, 8, 1, 12, 0, tzinfo=dt.timezone.utc)
    return {
        "id": run_id,
        "name": name or path.split("/")[-1],
        "path": f".github/workflows/{path}",
        "event": event,
        "conclusion": conclusion,
        "run_started_at": start.isoformat().replace("+00:00", "Z"),
        "updated_at": (start + dt.timedelta(minutes=minutes)).isoformat().replace("+00:00", "Z"),
    }


def _job_obj(name: str, minutes: float, *, conclusion: str = "success") -> dict:
    start = dt.datetime(2026, 8, 1, 12, 0, tzinfo=dt.timezone.utc)
    return {
        "name": name,
        "conclusion": conclusion,
        "started_at": start.isoformat().replace("+00:00", "Z"),
        "completed_at": (start + dt.timedelta(minutes=minutes)).isoformat().replace("+00:00", "Z"),
    }


def _self_test() -> int:
    failures: list[str] = []

    def check(name: str, condition: bool) -> None:
        if not condition:
            failures.append(name)

    # --- property 1: the event filter -------------------------------------------------
    mixed = [
        _run_obj("ci.yml", 20.0, event="push"),
        _run_obj("ci.yml", 5.0),
    ]
    kept = usable_runs(mixed)
    check("event-filter drops non-merge_group runs", len(kept) == 1)
    check("event-filter keeps the merge_group run",
          run_duration(kept[0]) == 300.0)
    check("event-filter: a 20 m push run cannot become the pole",
          summarize(kept)["ci.yml"]["max"] == 300.0)

    # --- property 2: the success filter -----------------------------------------------
    cancelled = [
        _run_obj("ci.yml", 2.0, conclusion="cancelled"),   # truncated wave cancellation
        _run_obj("ci.yml", 15.0),
    ]
    kept2 = usable_runs(cancelled)
    check("success-filter drops cancelled runs", len(kept2) == 1)
    check("success-filter: median is not dragged down by a truncated run",
          summarize(kept2)["ci.yml"]["median"] == 900.0)

    # --- property 3: max, not sum -----------------------------------------------------
    stats = summarize(usable_runs([
        _run_obj("ci.yml", 10.0), _run_obj("ci.yml", 10.0),
        _run_obj("docs-quality.yml", 1.0), _run_obj("docs-quality.yml", 1.0),
        _run_obj("feature-matrix.yml", 6.0), _run_obj("feature-matrix.yml", 6.0),
    ]))
    wall = entry_wall(stats)
    check("entry wall is max over lanes", wall["entry_wall_median"] == 600.0)
    check("entry wall is NOT the sum of lane medians",
          wall["entry_wall_median"] != sum(s["median"] for s in stats.values()))
    ranking = rank_poles(stats)
    check("ranking is by median descending",
          [s["workflow"] for s in ranking]
          == ["ci.yml", "feature-matrix.yml", "docs-quality.yml"])
    check("top-3 slice is the three slowest lanes", len(ranking[:TOP_POLES]) == 3)

    # ranking must be a TOTAL order, or report text reorders between identical samples
    tied = summarize(usable_runs([_run_obj("b.yml", 5.0), _run_obj("a.yml", 5.0)]))
    check("tie-break is deterministic",
          [s["workflow"] for s in rank_poles(tied)] == ["a.yml", "b.yml"])

    # --- property 4: structural vs observed reconciliation ----------------------------
    rec = reconcile({"ci.yml", "codeql.yml", "docs-quality.yml"}, stats)
    check("a triggering lane with no runs is reported UNOBSERVED",
          rec["unobserved"] == ["codeql.yml"])
    check("UNOBSERVED lane is absent from the ranking (not a 0 m lane)",
          "codeql.yml" not in {s["workflow"] for s in ranking})
    check("a lane with runs but no trigger today is reported UNTRIGGERED",
          rec["untriggered"] == ["feature-matrix.yml"])

    # --- property 5: fail-loud on an empty population ---------------------------------
    try:
        percentile([], 0.5)
        check("percentile over an empty sample must raise", False)
    except PoleError:
        pass
    check("usable_runs over an all-cancelled sample is empty",
          usable_runs([_run_obj("ci.yml", 3.0, conclusion="cancelled")]) == [])

    # --- estimator ---------------------------------------------------------------------
    check("median is an observed value (nearest-rank)",
          percentile([1.0, 2.0, 3.0, 4.0], 0.50) in (2.0, 3.0))
    # Nearest-rank means p90 returns an OBSERVED sample, so on n=10 it lands at index
    # round(0.9*9)=8 — NOT the single max. Asserted on n=11 (index round(0.9*10)=9) so the
    # check pins the ranking property (p90 sits in the high tail, strictly above the
    # median) rather than the "p90 == max" that nearest-rank deliberately does not give.
    sample = [float(i) for i in range(1, 12)]
    check("p90 sits in the high tail", percentile(sample, 0.90) == 10.0)
    check("p90 is strictly above the median",
          percentile(sample, 0.90) > percentile(sample, 0.50))
    check("single-sample percentile is that sample", percentile([7.0], 0.90) == 7.0)
    check("thin samples are flagged",
          summarize(usable_runs([_run_obj("ci.yml", 4.0)]))["ci.yml"]["thin"] is True)

    # --- duration edge cases ------------------------------------------------------------
    check("a run missing timestamps yields no duration",
          run_duration({"run_started_at": None, "updated_at": None}) is None)
    check("a negative span is rejected, not reported as negative time",
          run_duration({"run_started_at": "2026-08-01T12:10:00Z",
                        "updated_at": "2026-08-01T12:00:00Z"}) is None)
    check("a run with no usable duration is excluded from the summary",
          summarize([{"path": ".github/workflows/x.yml", "event": "merge_group",
                      "conclusion": "success"}]) == {})

    # --- the structural parser ----------------------------------------------------------
    check("bare merge_group trigger is detected",
          triggers_on_merge_group("name: x\non:\n  merge_group:\njobs: {}\n"))
    check("a COMMENTED-OUT merge_group is not a trigger",
          not triggers_on_merge_group(
              "name: x\non:\n  push:\n  # merge_group: removed 2026-07-18\njobs: {}\n"))
    check("merge_group under a LATER top-level key is not a trigger",
          not triggers_on_merge_group(
              "name: x\non:\n  push:\njobs:\n  merge_group:\n    runs-on: x\n"))
    check("quoted \"on\" key is handled",
          triggers_on_merge_group('name: x\n"on":\n  merge_group:\njobs: {}\n'))
    check("a workflow with no on: block has no trigger",
          not triggers_on_merge_group("name: x\njobs: {}\n"))

    # --- job-level decomposition ---------------------------------------------------------
    job_summary = summarize_jobs([
        _job_obj("build", 8.0), _job_obj("build", 10.0),
        _job_obj("quick-gates", 1.0),
        _job_obj("skipped-shard", 0.0, conclusion="skipped"),
    ])
    check("job ranking is by median descending",
          [j["job"] for j in job_summary] == ["build", "quick-gates"])
    check("a skipped job is not scored as instant",
          "skipped-shard" not in {j["job"] for j in job_summary})

    # --- rendering ------------------------------------------------------------------------
    report = render_workflow_report({
        "repo": "o/r", "sample_size": 6, "runs_listed": 6,
        "ranking": ranking, **wall,
        "unobserved": ["codeql.yml"], "untriggered": [],
        "top_poles": ["ci.yml"],
    })
    check("report names the top pole", "ci.yml" in report)
    check("report surfaces the unobserved lane", "codeql.yml" in report)
    check("report states the max-not-sum rule", "max` over lanes" in report)
    check("report carries the re-derivation command",
          "ci_mergegroup_poles.py" in report)

    for name in failures:
        print(f"FAIL: {name}", file=sys.stderr)
    total = 31
    print(f"ci_mergegroup_poles self-test: {total - len(failures)}/{total} checks passed"
          if not failures else f"ci_mergegroup_poles self-test: {len(failures)} FAILED")
    return 1 if failures else 0


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(
        description="Rank per-workflow merge_group wall-clock poles (issue #5250)."
    )
    parser.add_argument("--repo", default="sparq-org/sparq")
    parser.add_argument("--limit", type=int, default=DEFAULT_LIMIT,
                        help=f"completed merge_group runs to sample (default {DEFAULT_LIMIT})")
    parser.add_argument("--jobs", metavar="WORKFLOW.yml",
                        help="decompose one workflow's merge_group runs to the job level")
    parser.add_argument("--job-runs", type=int, default=DEFAULT_JOB_RUNS,
                        help=f"runs to decompose in --jobs mode (default {DEFAULT_JOB_RUNS})")
    parser.add_argument("--workflows-dir", default=str(WORKFLOWS_DIR))
    parser.add_argument("--json", action="store_true")
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args(argv)

    if args.self_test:
        return _self_test()
    if args.limit < 1:
        print("--limit must be >= 1", file=sys.stderr)
        return 1
    if args.job_runs < 1:
        print("--job-runs must be >= 1", file=sys.stderr)
        return 1
    try:
        return run(args)
    except PoleError as exc:
        print(f"ci_mergegroup_poles: {exc}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    sys.exit(main())
