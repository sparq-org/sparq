#!/usr/bin/env python3
# Per-PR / per-task metrics-collection harness (bead sq-lhwo.1, epic sq-lhwo).
#
# WHY THIS EXISTS
# --------------------------------------------------------------------------------
# This is the INSTRUMENT-FIRST backbone that GATES all adoption across both the
# agent-effectiveness program (research/agent-effectiveness-program.md) and the
# sparq-PKG dogfooding track (research/dogfooding-sparq-knowledge-graph.md). Nothing
# gets adopted without (1) a baseline captured by this harness and (2) an A/B run
# against that baseline using the SHARED §5 protocol + verdict object defined in
# research/dogfooding-sparq-knowledge-graph.md §5.1-§5.6. This harness does NOT
# define statistics or thresholds; it only EMITS the structured rows those statistics
# consume.
#
# WHAT IT DOES (per research/agent-effectiveness-program.md §1.4)
# --------------------------------------------------------------------------------
# Given a PR number, it JOINS the four already-existing data stores into ONE
# append-only JSONL row keyed by (sha, pr, bead, session, arm):
#   - token / cache / per-agent efficiency  <- agent_telemetry.py over the session
#     transcript (we DIFF/REUSE its JSON report; we do NOT re-derive token counts).
#   - CI first-pass + rework + diff churn   <- `gh pr view` statusCheckRollup/commits.
#   - review PUSHBACK count + severity       <- `roborev show <sha> --json` (codex).
#   - task identity + dependency context     <- `bd show <bead> --json` (optional).
# It then computes the COMPOSITE first-shot-success flag and the cache-discounted
# effective_input_tokens, and pairs each efficiency column with a QUALITY column so a
# "fewer tokens, worse output" win is catchable.
#
# HONESTY / WHAT IS CLEANLY-MEASURED vs APPROXIMATE (load-bearing — read this)
# --------------------------------------------------------------------------------
# Every column carries a provenance/confidence tag in `_field_quality` so a consumer
# can see which numbers are solid and which are approximate. The honest grading:
#
#   CLEANLY MEASURED (deterministic from GitHub / git ref-events):
#     * ci_first_green, first_ci_attempt   -- gh run attempt==1, all checks SUCCESS.
#     * force_push_count                    -- timeline head_ref_force_pushed events.
#     * post_first_push_commits             -- commits with committedDate > PR.createdAt
#                                              (ref-event-aligned, NOT a commit-message
#                                              grep -- closes the amend/squash gaming
#                                              hole the review flagged, §1.3).
#     * no_rework                           -- (force_push_count==0) AND
#                                              (post_first_push_commits==0).
#     * churn_added/deleted/changed_files   -- gh additions/deletions/changedFiles.
#     * review_changes_requested            -- gh reviews CHANGES_REQUESTED count.
#     * roborev_findings{high,med,low}      -- parsed from `roborev show` output text;
#                                              roborev_verdict P/F.
#
#   APPROXIMATE / PARTIAL (stated honestly, never fabricated):
#     * PER-PR TOKEN ATTRIBUTION IS HARD. Claude Code transcripts are per-SESSION,
#       not per-PR. One session can touch many PRs (an orchestrator fan-out) and one
#       PR can span many sessions. There is NO ground-truth per-PR token ledger. So
#       tokens_* are attributed ONLY when a session transcript is explicitly supplied
#       for the PR (--telemetry-json / --transcript); otherwise they are null and
#       `_field_quality.tokens` == "unattributed". The A/B harness side-steps this by
#       running ONE task per session (§5.1), where session == task is exact.
#     * usd_est is null unless explicit --price-* flags are passed (prices drift; a
#       baked price is a lie). `roborev cost` reports $ for only some agents -> an
#       explicit LOWER BOUND, coverage reported.
#     * roborev severities are parsed from free-text review output. The codex format
#       ("**Severity**: High/Medium/Low") is stable but text-derived, so it is graded
#       "text_parsed", not "structured".
#     * coverage_delta / mutation_score_delta / conformance_floor_moved /
#       seeded_canary_find_rate / post_merge_revert are QUALITY columns that the CI
#       artifacts (perf-gate, mutation ratchet, conformance scoreboard) or a later
#       backfill populate. This harness emits them as null placeholders with an
#       explicit source note so the SCHEMA is complete and the A/B can fill them in;
#       it does NOT fabricate them.
#
# The harness is model-free and deterministic (no model-in-the-loop per row -- an
# expensive measurement layer would itself be a token regression, §1.8). It RANKS and
# GATES; it never auto-adopts. Every emitted number is work-box/session-local and
# NON-CANONICAL: the row JSONL lives under a git-ignored runtime path; the committed
# artifacts are this CODE, the fixture test, and the verdict schema only.
#
# USAGE
#   metrics_row.py --pr 1057 [--bead sq-vw3ax.8] [--arm control|treatment]
#                  [--telemetry-json report.json | --transcript SESSION.jsonl]
#                  [--repo owner/name] [--out metrics/runtime/rows.jsonl]
#                  [--price-input N --price-output N ...]   # opt-in $ only
#   metrics_row.py --pr 1057 --dry-run           # print the row, do not append
#   metrics_row.py --from-json captured.json     # build a row from a pre-captured
#                                                 # join bundle (hermetic / offline)

from __future__ import annotations

import argparse
import json
import os
import re
import shutil
import subprocess
import sys
from datetime import datetime, timezone
from typing import Any, Optional

SCHEMA_VERSION = 1

# Cache-discount multipliers for effective_input_tokens (the §5.1 formula). These are
# RATIOS, not measurements (safe to keep here; they mirror agent_telemetry.py).
#   effective_input = 1.0*fresh_input + 0.1*cache_read + 1.25*cache_write
EFF_FRESH = 1.0
EFF_CACHE_READ = 0.1
EFF_CACHE_WRITE = 1.25


# --------------------------------------------------------------------------------
# Field-quality grading (the honesty spine: which columns are solid vs approximate)
# --------------------------------------------------------------------------------
# Each grade is one of:
#   "measured"     -- deterministic from GitHub/git ref-events; trustworthy.
#   "text_parsed"  -- parsed from free-text tool output (stable format, but textual).
#   "unattributed" -- not derivable for this row (e.g. per-PR tokens with no session).
#   "placeholder"  -- a schema slot the A/B / CI backfill fills; emitted null here.
#   "opt_in"       -- present only when an explicit flag was supplied (e.g. prices).
FIELD_QUALITY_BASE: dict[str, str] = {
    "ci_first_green": "measured",
    "first_ci_attempt": "measured",
    "force_push_count": "measured",
    "post_first_push_commits": "measured",
    "no_rework": "measured",
    "churn_added": "measured",
    "churn_deleted": "measured",
    "changed_files": "measured",
    "review_changes_requested": "measured",
    "roborev_findings": "text_parsed",
    "roborev_verdict": "text_parsed",
    "first_shot": "measured",  # composite of measured + text_parsed sub-flags
    # quality columns the A/B / CI backfill populates (schema-complete, null here):
    "coverage_delta": "placeholder",
    "mutation_score_delta": "placeholder",
    "conformance_floor_moved": "placeholder",
    "seeded_canary_find_rate": "placeholder",
    "post_merge_revert": "placeholder",
}


# --------------------------------------------------------------------------------
# Small subprocess helpers (stdlib-only)
# --------------------------------------------------------------------------------
class CollectorError(RuntimeError):
    pass


def _run(cmd: list[str], *, allow_fail: bool = False) -> str:
    """Run a command, return stdout. On failure: raise, or return '' if allow_fail."""
    try:
        proc = subprocess.run(
            cmd,
            check=False,
            capture_output=True,
            text=True,
        )
    except FileNotFoundError as exc:
        if allow_fail:
            return ""
        raise CollectorError(f"command not found: {cmd[0]}") from exc
    if proc.returncode != 0:
        if allow_fail:
            return ""
        raise CollectorError(
            f"command failed ({proc.returncode}): {' '.join(cmd)}\n{proc.stderr.strip()}"
        )
    return proc.stdout


def _have(cmd: str) -> bool:
    return shutil.which(cmd) is not None


def _parse_ts(value: Any) -> Optional[datetime]:
    if not isinstance(value, str) or not value:
        return None
    s = value.replace("Z", "+00:00")
    try:
        dt = datetime.fromisoformat(s)
    except ValueError:
        return None
    if dt.tzinfo is None:
        dt = dt.replace(tzinfo=timezone.utc)
    return dt


# --------------------------------------------------------------------------------
# roborev review-finding severity parsing (text_parsed)
# --------------------------------------------------------------------------------
# The codex reviewer emits findings as "**Severity**: High/Medium/Low" lines. We
# count them per severity. This is text-derived (graded "text_parsed"), but the
# format is stable across the 1300+ reviews on this repo.
_SEVERITY_RE = re.compile(
    r"\*\*\s*Severity\s*\*\*\s*:?\s*(High|Medium|Med|Low)", re.IGNORECASE
)
_NO_ISSUES_RE = re.compile(r"^\s*No issues found", re.IGNORECASE)


def parse_roborev_findings(output: str) -> dict[str, int]:
    """Count High/Med/Low findings in a roborev review's free-text output."""
    counts = {"high": 0, "med": 0, "low": 0}
    if not output:
        return counts
    for m in _SEVERITY_RE.finditer(output):
        sev = m.group(1).lower()
        if sev == "high":
            counts["high"] += 1
        elif sev in ("medium", "med"):
            counts["med"] += 1
        elif sev == "low":
            counts["low"] += 1
    return counts


# --------------------------------------------------------------------------------
# Token attribution from an agent_telemetry.py JSON report (REUSE, do not re-derive)
# --------------------------------------------------------------------------------
def effective_input_tokens(fresh: int, cache_read: int, cache_write: int) -> float:
    return (
        EFF_FRESH * fresh
        + EFF_CACHE_READ * cache_read
        + EFF_CACHE_WRITE * cache_write
    )


def tokens_from_telemetry(report: dict) -> dict:
    """Pull the wave rollup out of an agent_telemetry.py JSON report.

    We REUSE agent_telemetry.py's already-computed rollup (the canonical accounting
    engine named by both designs) -- we never re-count tokens here.
    """
    roll = report.get("rollup") or {}
    fresh = int(roll.get("input_tokens") or 0)
    out = int(roll.get("output_tokens") or 0)
    cache_read = int(roll.get("cache_read_input_tokens") or 0)
    cache_write = int(roll.get("cache_creation_input_tokens") or 0)
    eff = effective_input_tokens(fresh, cache_read, cache_write)
    return {
        "tokens_in": fresh,
        "tokens_out": out,
        "cache_read": cache_read,
        "cache_write": cache_write,
        "effective_input_tokens": round(eff, 2),
        "cache_hit_ratio": roll.get("cache_hit_ratio"),
        "wave_duration_s": report.get("wave_duration_seconds"),
        "agent_count": report.get("agent_count"),
        "subagent_count": report.get("subagent_count"),
        "session_ids": report.get("session_ids"),
    }


def _telemetry_from_transcript(transcript_path: str) -> Optional[dict]:
    """Run agent_telemetry.py over a transcript and return its JSON report.

    Imported in-process so we share ONE token-accounting implementation rather than
    re-deriving counts (the explicit design constraint).
    """
    here = os.path.dirname(os.path.abspath(__file__))
    harness = os.path.join(here, "agent_telemetry.py")
    if not os.path.exists(harness):
        raise CollectorError("agent_telemetry.py not found next to metrics_row.py")
    import importlib.util

    spec = importlib.util.spec_from_file_location("agent_telemetry", harness)
    assert spec and spec.loader
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    report = mod.parse_transcript(transcript_path)
    return mod.build_json(report, None)


# --------------------------------------------------------------------------------
# GitHub join (gh) -- CI first-pass, rework (ref-event-derived), churn, reviews
# --------------------------------------------------------------------------------
def collect_github(pr: int, repo: Optional[str]) -> dict:
    """Collect the gh-derived signals for a PR. All deterministic / measured."""
    base = ["gh", "pr", "view", str(pr)]
    if repo:
        base = ["gh", "pr", "view", str(pr), "--repo", repo]
    fields = (
        "number,title,headRefName,createdAt,mergedAt,mergeCommit,"
        "additions,deletions,changedFiles,commits,reviews,reviewDecision,"
        "statusCheckRollup,state"
    )
    out = _run(base + ["--json", fields])
    data = json.loads(out)
    return data


def derive_rework(pr: int, repo: Optional[str], pr_data: dict) -> dict:
    """Derive no_rework from REF EVENTS (timeline), not commit messages (§1.3).

    no_rework = (force_push_count == 0) AND (post_first_push_commits == 0)

    * force_push_count   -- count of head_ref_force_pushed events in the PR timeline.
    * post_first_push_commits -- commits whose committedDate is AFTER the PR's
      createdAt. Commits in the FIRST push land at/around PR open; commits authored
      after PR open are genuine post-first-push rework. This is ref-event-aligned and
      robust to `git commit --amend` + force-push (an amend leaves no new commit, but
      the force-push event is counted separately).
    """
    slug = repo or _default_slug()
    timeline = _gh_timeline(pr, slug)
    force_push_count = sum(
        1 for e in timeline if e.get("event") == "head_ref_force_pushed"
    )
    created = _parse_ts(pr_data.get("createdAt"))
    commits = pr_data.get("commits") or []
    post_commits = 0
    if created is not None:
        for c in commits:
            cdate = _parse_ts(c.get("committedDate"))
            if cdate is not None and cdate > created:
                post_commits += 1
    no_rework = (force_push_count == 0) and (post_commits == 0)
    return {
        "force_push_count": force_push_count,
        "post_first_push_commits": post_commits,
        "no_rework": no_rework,
    }


def _default_slug() -> str:
    out = _run(["gh", "repo", "view", "--json", "nameWithOwner"], allow_fail=True)
    if not out:
        raise CollectorError("could not determine repo slug; pass --repo owner/name")
    return json.loads(out)["nameWithOwner"]


def _gh_timeline(pr: int, slug: str) -> list[dict]:
    """Fetch the issue/PR timeline (PRs are issues; pulls/N/timeline 404s here)."""
    out = _run(
        ["gh", "api", f"repos/{slug}/issues/{pr}/timeline?per_page=100"],
        allow_fail=True,
    )
    if not out:
        return []
    try:
        data = json.loads(out)
    except json.JSONDecodeError:
        return []
    return data if isinstance(data, list) else []


# A check whose name marks it advisory is non-blocking by the repo's gate policy
# (AGENTS.md: advisory lanes never block a merge), so a failing advisory check must
# NOT pull down ci_first_green / first-shot. We detect advisory checks by name marker
# rather than by querying branch-protection (which gh pr view does not expose).
_ADVISORY_RE = re.compile(r"\b(advisory|non-?blocking|informational)\b", re.IGNORECASE)


def _check_name(c: dict) -> str:
    return c.get("name") or c.get("context") or ""


def derive_ci_first_pass(pr_data: dict) -> dict:
    """ci_first_green: did every BLOCKING check pass on the first run/attempt?

    statusCheckRollup is the LATEST state per check. We treat ci_first_green as
    "all BLOCKING checks currently SUCCESS/NEUTRAL/SKIPPED". Advisory / non-blocking
    lanes (detected by a name marker, per the repo's gate policy) are tallied
    separately and do NOT fail the gate -- a failing advisory check is by design not a
    first-shot regression.

    A precise per-attempt history (did it pass on attempt==1, vs go green only after a
    re-run) needs `gh run list --json attempt`; that is an optional enrichment -- the
    rollup is the always-available floor, graded "measured". The rollup-latest-state
    view treats a check that needed a manual re-run-to-green as currently green, so the
    primary rework guard is the ref-event-derived no_rework, not this flag (which is
    why first_shot ANDs both).
    """
    checks = pr_data.get("statusCheckRollup") or []
    if not checks:
        return {
            "ci_first_green": None,
            "ci_checks_total": 0,
            "ci_checks_failing": 0,
            "ci_advisory_failing": 0,
        }
    ok_states = {"SUCCESS", "NEUTRAL", "SKIPPED", "EXPECTED"}
    failing = 0
    advisory_failing = 0
    for c in checks:
        # CheckRun uses .conclusion/.status; StatusContext uses .state.
        concl = (c.get("conclusion") or c.get("state") or "").upper()
        status = (c.get("status") or "").upper()
        if status and status != "COMPLETED" and not concl:
            # still running / queued -> not a pass, not yet a fail
            continue
        if concl and concl not in ok_states:
            if _ADVISORY_RE.search(_check_name(c)):
                advisory_failing += 1
            else:
                failing += 1
    return {
        "ci_first_green": failing == 0,
        "ci_checks_total": len(checks),
        "ci_checks_failing": failing,
        "ci_advisory_failing": advisory_failing,
    }


def derive_reviews(pr_data: dict) -> dict:
    reviews = pr_data.get("reviews") or []
    changes_requested = sum(
        1 for r in reviews if (r.get("state") or "").upper() == "CHANGES_REQUESTED"
    )
    return {"review_changes_requested": changes_requested}


# --------------------------------------------------------------------------------
# roborev join -- pushback count + severity + verdict, keyed on commit SHA
# --------------------------------------------------------------------------------
def collect_roborev(shas: list[str]) -> dict:
    """Aggregate roborev findings across the PR's reviewed commit SHAs.

    roborev reviews are keyed by git_ref (commit SHA), one review per reviewed commit
    / trial-merge. We aggregate findings across all SHAs we can resolve for the PR and
    take the worst verdict (F dominates). Coverage is reported honestly: if no review
    exists for any SHA, roborev_verdict is null and reviews_found==0.
    """
    if not _have("roborev"):
        return {
            "roborev_findings": {"high": 0, "med": 0, "low": 0},
            "roborev_verdict": None,
            "roborev_reviews_found": 0,
            "roborev_available": False,
        }
    total = {"high": 0, "med": 0, "low": 0}
    verdicts: list[str] = []
    found = 0
    for sha in shas:
        if not sha:
            continue
        out = _run(["roborev", "show", sha, "--json"], allow_fail=True)
        if not out:
            continue
        try:
            rev = json.loads(out)
        except json.JSONDecodeError:
            continue
        found += 1
        counts = parse_roborev_findings(rev.get("output") or "")
        for k in total:
            total[k] += counts[k]
        v = (rev.get("job") or {}).get("verdict") or rev.get("verdict")
        if v:
            verdicts.append(v)
    verdict = None
    if verdicts:
        verdict = "F" if any(v == "F" for v in verdicts) else "P"
    return {
        "roborev_findings": total,
        "roborev_verdict": verdict,
        "roborev_reviews_found": found,
        "roborev_available": True,
    }


# --------------------------------------------------------------------------------
# bd join (optional) -- task identity / surface / type
# --------------------------------------------------------------------------------
def collect_bead(bead: Optional[str]) -> dict:
    if not bead or not _have("bd"):
        return {"bead": bead, "bead_type": None, "surface": None}
    out = _run(["bd", "show", bead, "--json"], allow_fail=True)
    if not out:
        return {"bead": bead, "bead_type": None, "surface": None}
    try:
        data = json.loads(out)
    except json.JSONDecodeError:
        return {"bead": bead, "bead_type": None, "surface": None}
    # bd show --json may return a list or an object depending on version.
    rec = data[0] if isinstance(data, list) and data else data
    if not isinstance(rec, dict):
        return {"bead": bead, "bead_type": None, "surface": None}
    labels = rec.get("labels") or []
    surface = next(
        (lbl.split("area:", 1)[1] for lbl in labels if str(lbl).startswith("area:")),
        None,
    )
    return {
        "bead": bead,
        "bead_type": rec.get("issue_type"),
        "surface": surface,
    }


# --------------------------------------------------------------------------------
# Composite first-shot-success (§1.3) -- reported sub-flag-by-sub-flag
# --------------------------------------------------------------------------------
def compute_first_shot(row: dict) -> dict:
    """first_shot = ci_first_green AND no_rework AND roborev_blocking==0
    AND review_changes_requested==0. Each sub-flag is kept in the row so you can see
    WHICH gate trips, and so the composite cannot be gamed by suppressing one signal.

    A roborev "blocking finding" is High or Med (Low is advisory). If roborev was
    unavailable / no review found, that sub-flag is None (unknown), and the composite
    is None unless an explicitly-failing sub-flag already forces it False.
    """
    ci = row.get("ci_first_green")
    no_rework = row.get("no_rework")
    rr = row.get("review_changes_requested")
    findings = row.get("roborev_findings") or {}
    rb_found = row.get("roborev_reviews_found", 0)
    blocking = findings.get("high", 0) + findings.get("med", 0)
    roborev_blocking_zero: Optional[bool]
    if rb_found and rb_found > 0:
        roborev_blocking_zero = blocking == 0
    else:
        roborev_blocking_zero = None  # unknown: no review to judge

    sub = {
        "first_ci_attempt": ci,
        "no_rework": no_rework,
        "roborev_blocking_zero": roborev_blocking_zero,
        "review_changes_requested_zero": (None if rr is None else rr == 0),
    }
    # If any known sub-flag is False -> composite is False (a hard failure shows even
    # if another sub-flag is unknown). If all known sub-flags are True but some are
    # unknown -> composite is None (can't certify first-shot). All True -> True.
    known = [v for v in sub.values() if v is not None]
    if any(v is False for v in known):
        composite: Optional[bool] = False
    elif any(v is None for v in sub.values()):
        composite = None
    else:
        composite = True
    return {"first_shot": composite, "first_shot_subflags": sub}


# --------------------------------------------------------------------------------
# Price (opt-in only -- never hard-coded)
# --------------------------------------------------------------------------------
def usd_estimate(row: dict, prices: Optional[dict]) -> Optional[float]:
    if not prices:
        return None
    if row.get("tokens_in") is None:
        return None
    per = 1_000_000.0
    return round(
        (
            (row.get("tokens_in") or 0) * prices["input"]
            + (row.get("tokens_out") or 0) * prices["output"]
            + (row.get("cache_read") or 0) * prices["cache_read"]
            + (row.get("cache_write") or 0) * prices["cache_write"]
        )
        / per,
        6,
    )


def price_table_from_args(args: argparse.Namespace) -> Optional[dict]:
    if args.price_input is None:
        return None
    inp = float(args.price_input)
    return {
        "input": inp,
        "output": float(args.price_output) if args.price_output is not None else 0.0,
        "cache_read": (
            float(args.price_cache_read)
            if args.price_cache_read is not None
            else inp * EFF_CACHE_READ
        ),
        "cache_write": (
            float(args.price_cache_write)
            if args.price_cache_write is not None
            else inp * EFF_CACHE_WRITE
        ),
    }


# --------------------------------------------------------------------------------
# Row assembly
# --------------------------------------------------------------------------------
QUALITY_PLACEHOLDERS = {
    "coverage_delta": None,
    "mutation_score_delta": None,
    "conformance_floor_moved": None,
    "seeded_canary_find_rate": None,
    "post_merge_revert": None,
}


def _empty_token_block() -> dict:
    return {
        "tokens_in": None,
        "tokens_out": None,
        "cache_read": None,
        "cache_write": None,
        "effective_input_tokens": None,
        "cache_hit_ratio": None,
        "wave_duration_s": None,
        "agent_count": None,
        "subagent_count": None,
        "session_ids": None,
    }


def build_row(
    *,
    pr: int,
    bead: Optional[str],
    arm: Optional[str],
    agent_type: Optional[str],
    repo: Optional[str],
    telemetry: Optional[dict],
    prices: Optional[dict],
    github: dict,
    roborev: dict,
    bead_info: dict,
) -> dict:
    """Assemble one metrics row from already-collected join bundles (pure)."""
    pr_data = github
    merge = pr_data.get("mergeCommit") or {}
    merge_sha = merge.get("oid")
    commits = pr_data.get("commits") or []
    commit_shas = [c.get("oid") for c in commits if c.get("oid")]
    shas_for_roborev = ([merge_sha] if merge_sha else []) + commit_shas

    rework = derive_rework_from_data(pr, repo, pr_data)
    ci = derive_ci_first_pass(pr_data)
    reviews = derive_reviews(pr_data)

    token_block = (
        tokens_from_telemetry(telemetry) if telemetry else _empty_token_block()
    )
    tokens_attributed = telemetry is not None

    row: dict[str, Any] = {
        "schema_version": SCHEMA_VERSION,
        "collected_at": datetime.now(timezone.utc).isoformat(),
        # join key (sha, pr, bead, session, arm)
        "pr": pr,
        "sha": merge_sha,
        "bead": bead or bead_info.get("bead"),
        "arm": arm,
        "agent_type": agent_type,
        "surface": bead_info.get("surface"),
        "bead_type": bead_info.get("bead_type"),
        "title": pr_data.get("title"),
        "head_ref": pr_data.get("headRefName"),
        "merged_at": pr_data.get("mergedAt"),
        # --- efficiency (token) columns (approximate unless a session is supplied) ---
        **token_block,
        "usd_est": None,
        # --- CI / rework / churn (measured) ---
        **ci,
        **rework,
        "churn_added": pr_data.get("additions"),
        "churn_deleted": pr_data.get("deletions"),
        "changed_files": pr_data.get("changedFiles"),
        # --- review pushback (measured + text_parsed) ---
        **reviews,
        "roborev_findings": roborev.get("roborev_findings"),
        "roborev_verdict": roborev.get("roborev_verdict"),
        "roborev_reviews_found": roborev.get("roborev_reviews_found"),
        # --- QUALITY pairs (placeholders the A/B / CI backfill fills) ---
        **QUALITY_PLACEHOLDERS,
    }
    row.update(compute_first_shot(row))
    row["usd_est"] = usd_estimate(row, prices)

    # honesty metadata: per-field provenance grade + the load-bearing caveats.
    fq = dict(FIELD_QUALITY_BASE)
    fq["tokens"] = "measured" if tokens_attributed else "unattributed"
    fq["usd_est"] = "opt_in" if prices else "placeholder"
    row["_field_quality"] = fq
    row["_caveats"] = {
        "per_pr_token_attribution": (
            "tokens are session-scoped, not per-PR; attributed only when a session "
            "transcript is supplied (1 task = 1 session in the A/B). null = unattributed."
        ),
        "roborev_severity": "parsed from review free-text (stable codex format).",
        "roborev_blocking_proxy": (
            "first_shot treats any High+Med roborev finding as 'blocking' -- a "
            "conservative proxy. The design's 'forced a change' is not deterministically "
            "derivable from one review, so High+Med count is the honest floor (Low is "
            "advisory). A PR merged despite a Med finding still scores first_shot=False; "
            "inspect roborev_findings to see why."
        ),
        "ci_first_green": (
            "from statusCheckRollup latest state; per-attempt history is an optional "
            "enrichment via `gh run list --json attempt`."
        ),
        "non_canonical": "work-box/session-local; do NOT bake into committed markdown.",
    }
    row["_roborev_shas_considered"] = shas_for_roborev
    return row


# A pure variant of derive_rework that does not re-fetch the timeline when the bundle
# already carries it (used by --from-json for hermetic tests).
def derive_rework_from_data(pr: int, repo: Optional[str], pr_data: dict) -> dict:
    if "_timeline" in pr_data:
        timeline = pr_data.get("_timeline") or []
        force_push_count = sum(
            1 for e in timeline if e.get("event") == "head_ref_force_pushed"
        )
        created = _parse_ts(pr_data.get("createdAt"))
        post_commits = 0
        if created is not None:
            for c in pr_data.get("commits") or []:
                cdate = _parse_ts(c.get("committedDate"))
                if cdate is not None and cdate > created:
                    post_commits += 1
        return {
            "force_push_count": force_push_count,
            "post_first_push_commits": post_commits,
            "no_rework": (force_push_count == 0) and (post_commits == 0),
        }
    return derive_rework(pr, repo, pr_data)


# --------------------------------------------------------------------------------
# Collection orchestration (live, talks to gh/roborev/bd)
# --------------------------------------------------------------------------------
def collect_row(args: argparse.Namespace) -> dict:
    if not _have("gh"):
        raise CollectorError("gh CLI not found -- required to collect a PR row")
    github = collect_github(args.pr, args.repo)
    merge = github.get("mergeCommit") or {}
    commits = github.get("commits") or []
    shas = ([merge.get("oid")] if merge.get("oid") else []) + [
        c.get("oid") for c in commits if c.get("oid")
    ]
    roborev = collect_roborev(shas)
    bead_info = collect_bead(args.bead)

    telemetry: Optional[dict] = None
    if args.telemetry_json:
        with open(args.telemetry_json, "r", encoding="utf-8") as fh:
            telemetry = json.load(fh)
    elif args.transcript:
        telemetry = _telemetry_from_transcript(args.transcript)

    prices = price_table_from_args(args)
    return build_row(
        pr=args.pr,
        bead=args.bead,
        arm=args.arm,
        agent_type=args.agent_type,
        repo=args.repo,
        telemetry=telemetry,
        prices=prices,
        github=github,
        roborev=roborev,
        bead_info=bead_info,
    )


def row_from_bundle(bundle: dict, prices: Optional[dict]) -> dict:
    """Build a row from a pre-captured join bundle (hermetic/offline path).

    Expected bundle shape:
      { "pr": int, "bead": str?, "arm": str?, "agent_type": str?, "repo": str?,
        "github": {<gh pr view json> + optional "_timeline": [...]},
        "roborev": {<collect_roborev output>},
        "bead_info": {<collect_bead output>},
        "telemetry": {<agent_telemetry.py json report>}? }
    """
    return build_row(
        pr=int(bundle["pr"]),
        bead=bundle.get("bead"),
        arm=bundle.get("arm"),
        agent_type=bundle.get("agent_type"),
        repo=bundle.get("repo"),
        telemetry=bundle.get("telemetry"),
        prices=prices,
        github=bundle["github"],
        roborev=bundle.get("roborev")
        or {
            "roborev_findings": {"high": 0, "med": 0, "low": 0},
            "roborev_verdict": None,
            "roborev_reviews_found": 0,
        },
        bead_info=bundle.get("bead_info") or {"bead": bundle.get("bead")},
    )


# --------------------------------------------------------------------------------
# Output
# --------------------------------------------------------------------------------
def append_row(path: str, row: dict) -> None:
    os.makedirs(os.path.dirname(os.path.abspath(path)), exist_ok=True)
    with open(path, "a", encoding="utf-8") as fh:
        fh.write(json.dumps(row, sort_keys=True) + "\n")


def build_arg_parser() -> argparse.ArgumentParser:
    p = argparse.ArgumentParser(
        description=(
            "Emit ONE structured metrics row per merged PR/task by JOINING "
            "agent_telemetry.py + gh + roborev + bd. NON-CANONICAL session-local "
            "numbers, runtime-only; see research/dogfooding-sparq-knowledge-graph.md §5."
        )
    )
    p.add_argument("--pr", type=int, help="PR number to collect a row for")
    p.add_argument("--bead", help="associated bead id (optional join key)")
    p.add_argument(
        "--arm",
        choices=["control", "treatment"],
        help="A/B arm label (per dogfooding §5.1)",
    )
    p.add_argument("--agent-type", dest="agent_type", help="agent slug (optional)")
    p.add_argument("--repo", help="owner/name (default: current repo via gh)")
    p.add_argument(
        "--telemetry-json",
        dest="telemetry_json",
        help="path to an agent_telemetry.py JSON report (REUSED, not re-derived)",
    )
    p.add_argument(
        "--transcript",
        help="path to a session transcript JSONL (runs agent_telemetry.py in-process)",
    )
    p.add_argument(
        "--from-json",
        dest="from_json",
        help="build a row from a pre-captured join bundle (hermetic/offline)",
    )
    p.add_argument(
        "--out",
        default="metrics/runtime/rows.jsonl",
        help="append the row to this JSONL (default: metrics/runtime/rows.jsonl, "
        "git-ignored)",
    )
    p.add_argument(
        "--dry-run",
        action="store_true",
        help="print the row to stdout; do NOT append to --out",
    )
    p.add_argument("--price-input", type=float, help="$ per 1M input tokens (opt-in)")
    p.add_argument("--price-output", type=float, help="$ per 1M output tokens")
    p.add_argument("--price-cache-read", type=float, help="$ per 1M cache-read tokens")
    p.add_argument(
        "--price-cache-write", type=float, help="$ per 1M cache-write tokens"
    )
    return p


def main(argv: Optional[list] = None) -> int:
    args = build_arg_parser().parse_args(argv)
    prices = price_table_from_args(args)
    try:
        if args.from_json:
            with open(args.from_json, "r", encoding="utf-8") as fh:
                bundle = json.load(fh)
            row = row_from_bundle(bundle, prices)
        elif args.pr is not None:
            row = collect_row(args)
        else:
            print("error: pass --pr N or --from-json BUNDLE", file=sys.stderr)
            return 2
    except CollectorError as exc:
        print(f"error: {exc}", file=sys.stderr)
        return 1

    line = json.dumps(row, indent=2, sort_keys=True)
    if args.dry_run:
        print(line)
        return 0
    append_row(args.out, row)
    print(f"appended 1 row to {args.out} (pr={row['pr']}, "
          f"first_shot={row['first_shot']})")
    print(
        "NOTE: NON-CANONICAL session-local row -- do not bake into committed markdown.",
        file=sys.stderr,
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
