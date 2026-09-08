#!/usr/bin/env python3
"""[GPT-6 Astra] Prove heavy nightly completion before skipping an unchanged head.

Only scheduled ci.yml runs on main are evidence. A successful workflow with skipped
heavy jobs is NOT evidence; follow it back to real work within the fixed read budget.
Incomplete work remains eligible at the next ordinary schedule; there is no rerun API
or retry loop. Unreadable evidence blocks admission. If proof ages out of the read
window, a bounded active-run census permits periodic remeasurement. Manual dispatch
remains an explicit force and makes no history requests.
"""

from __future__ import annotations

import json
import os
from pathlib import Path
import re
import subprocess
import sys
from urllib.parse import urlencode

HISTORY_LIMIT = 10
JOB_PAGE_SIZE = 100
JOB_PAGE_LIMIT = 2
MUTATION_JOBS = 51  # Pinned to the live matrix by test_ci_nightly_freshness.py.
COVERAGE = "coverage (nightly, full incl. heavy vectors)"
COVERAGE_STEP = "Measure + enforce per-crate coverage (FULL tier, max-remeasure gate)"
MUTATION = "mutation ratchet (cargo-mutants, advisory)"
MUTATION_STEP = "Nightly mutation work completed"
# [GPT-6 Astra] All non-completed statuses supported by the workflow-runs API.
ACTIVE_STATUSES = ("queued", "in_progress", "waiting", "requested", "pending")
COMPLETED_CONCLUSIONS = ("success", "failure", "cancelled", "timed_out", "startup_failure",
                         "stale", "neutral", "action_required", "skipped")


class EvidenceError(Exception):
    """No automatic admission or freshness claim is justified."""


def positive_int(value):
    return type(value) is int and value > 0


def mutation_complete(exit_code, directory):
    """Prove completion, independently of the existing advisory ratchet verdict.

    cargo-mutants v25.3.1 src/{exit_code,outcome}.rs: 0/2/3 are completed
    runs (caught/missed/mutant timeouts). Other exits do not prove execution.
    mutants.json is written before testing; outcomes counts grow incrementally.
    Their equality is necessary even for an accepted exit; partial files are not
    evidence. A new tool schema fails closed until its contract is reviewed.
    """
    if exit_code not in (0, 2, 3):
        return False
    try:
        planned = json.loads((Path(directory) / "mutants.json").read_text())
        doc = json.loads((Path(directory) / "outcomes.json").read_text())
    except (OSError, ValueError):
        return False
    if not isinstance(planned, list) or not isinstance(doc, dict):
        return False
    counts = [doc.get(key) for key in ("caught", "missed", "timeout", "unviable")]
    if any(type(n) is not int or n < 0 for n in counts):
        return False
    return (type(doc.get("total_mutants")) is int
            and doc["total_mutants"] == len(planned) == sum(counts)
            and type(doc.get("success")) is int and doc["success"] == 0)


def gh_json(endpoint):
    """One bounded GET, no retries (including rate-limit/permission failures)."""
    try:
        result = subprocess.run(
            ["gh", "api", "--method", "GET", endpoint], capture_output=True,
            text=True, timeout=30, check=False,
        )
        if result.returncode:
            raise EvidenceError(f"GitHub read failed (exit {result.returncode}); no retry")
        return json.loads(result.stdout)
    except (OSError, subprocess.TimeoutExpired, ValueError) as exc:
        raise EvidenceError("GitHub evidence unavailable or unreadable; no retry") from exc


def nightly_identity(run, repo, head):
    return (isinstance(run, dict) and positive_int(run.get("id"))
            and positive_int(run.get("run_attempt")) and run.get("event") == "schedule"
            and run.get("head_sha") == head and run.get("head_branch") == "main"
            and run.get("path") == ".github/workflows/ci.yml"
            and isinstance(run.get("repository"), dict)
            and run["repository"].get("full_name") == repo)


def ensure_no_active(get, repo, head, current_id):
    """Five GETs maximum, no pagination/retries. Check beyond the history window.

    Two rows suffice: the current run and any competing run. Larger, malformed,
    filtered-status-mismatched or incomplete inventories cannot authorize work.
    """
    for status in ACTIVE_STATUSES:
        query = urlencode({"event": "schedule", "head_sha": head,
                           "status": status, "per_page": 2})
        doc = get(f"repos/{repo}/actions/workflows/ci.yml/runs?{query}")
        if (not isinstance(doc, dict) or type(doc.get("total_count")) is not int
                or not isinstance(doc.get("workflow_runs"), list)
                or not 0 <= doc["total_count"] <= 2
                or len(doc["workflow_runs"]) != doc["total_count"]):
            raise EvidenceError("active-run census malformed or truncated")
        seen = set()
        for run in doc["workflow_runs"]:
            if (not nightly_identity(run, repo, head) or run["id"] in seen
                    or run.get("status") != status or run.get("conclusion") is not None):
                raise EvidenceError("active-run census identity or status unreadable")
            seen.add(run["id"])
            if run["id"] != current_id:
                raise EvidenceError("another same-head schedule is active")


def read_jobs(get, repo, run):
    jobs = []
    expected = None
    for page in range(1, JOB_PAGE_LIMIT + 1):
        doc = get(f"repos/{repo}/actions/runs/{run['id']}/attempts/"
                  f"{run['run_attempt']}/jobs?per_page={JOB_PAGE_SIZE}&page={page}")
        if not isinstance(doc, dict) or type(doc.get("total_count")) is not int:
            raise EvidenceError("missing job inventory")
        total, batch = doc["total_count"], doc.get("jobs")
        if (total < 0 or total > JOB_PAGE_SIZE * JOB_PAGE_LIMIT
                or not isinstance(batch, list) or len(batch) > JOB_PAGE_SIZE
                or (expected is not None and total != expected)):
            raise EvidenceError("unbounded or inconsistent job inventory")
        expected = total
        jobs.extend(batch)
        if len(jobs) == total:
            break
        if len(batch) != JOB_PAGE_SIZE or len(jobs) > total:
            raise EvidenceError("truncated job inventory")
    ids = set()
    for job in jobs:
        if (not isinstance(job, dict) or not positive_int(job.get("id"))
                or job["id"] in ids or job.get("run_id") != run["id"]
                # [GPT-6 Astra] The attempt-scoped endpoint supplies the binding
                # when this optional field is absent. A contradictory value does not.
                or ("run_attempt" in job and (not positive_int(job["run_attempt"])
                                              or job["run_attempt"] != run["run_attempt"]))
                or job.get("head_sha") != run["head_sha"]):
            raise EvidenceError("job identity mismatch")
        ids.add(job["id"])
    return jobs


def successful_step(job, name):
    # [GPT-6 Astra] The API may omit optional steps: readable absence is no proof.
    # Null/non-array/malformed values are still unreadable evidence, not admission.
    if "steps" not in job:
        return False
    steps = job.get("steps")
    if not isinstance(steps, list) or any(not isinstance(s, dict) for s in steps):
        raise EvidenceError("heavy step inventory unreadable")
    matches = [s for s in steps if s.get("name") == name]
    return (len(matches) == 1 and matches[0].get("status") == "completed"
            and matches[0].get("conclusion") == "success")


def heavy_state(jobs):
    coverage = [j for j in jobs if j.get("name") == COVERAGE]
    mutations = [j for j in jobs if isinstance(j.get("name"), str)
                 and (j["name"] == MUTATION or j["name"].startswith(MUTATION + " ("))]
    if len(coverage) != 1 or not mutations:
        return "incomplete"
    heavy = coverage + mutations
    # [GPT-6 Astra] GitHub represents a job skipped before matrix expansion with
    # one base-name placeholder. Never accept a partial/mixed matrix as a skip.
    if (len(mutations) == 1 and mutations[0]["name"] == MUTATION
            and all(j.get("status") == "completed" and j.get("conclusion") == "skipped"
                    for j in heavy)):
        return "skipped"
    if (len(mutations) != MUTATION_JOBS
            or len({j["name"] for j in mutations}) != MUTATION_JOBS
            or any(j["name"] == MUTATION for j in mutations)
            or any(j.get("status") != "completed" or j.get("conclusion") != "success"
                   for j in heavy)
            or not successful_step(coverage[0], COVERAGE_STEP)
            or not all(successful_step(j, MUTATION_STEP) for j in mutations)):
        return "incomplete"
    return "complete"


def decide(event, repo, head, current_id, get=gh_json):
    """Return (run_heavy, reason); only this ordinary schedule may admit new work."""
    if event == "workflow_dispatch":
        return True, "manual dispatch explicitly requests heavy work"
    if event != "schedule":
        return False, "heavy nightly work is isolated from this event"
    if (not re.fullmatch(r"[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+", repo)
            or not re.fullmatch(r"[0-9a-f]{40}", head) or not positive_int(current_id)):
        raise EvidenceError("invalid scheduled run identity")
    query = urlencode({"event": "schedule", "head_sha": head, "per_page": HISTORY_LIMIT})
    doc = get(f"repos/{repo}/actions/workflows/ci.yml/runs?{query}")
    if (not isinstance(doc, dict) or type(doc.get("total_count")) is not int
            or doc["total_count"] < 0 or not isinstance(doc.get("workflow_runs"), list)):
        raise EvidenceError("missing scheduled history")
    runs = doc["workflow_runs"]
    if len(runs) != min(doc["total_count"], HISTORY_LIMIT):
        raise EvidenceError("truncated scheduled history")
    seen = set()
    for run in runs:
        if not nightly_identity(run, repo, head) or run["id"] in seen:
            raise EvidenceError("scheduled history identity mismatch")
        seen.add(run["id"])
    # [GPT-6 Astra] Run IDs order creation, not the update time of an old rerun.
    # A newer same-head schedule means this tick is stale; do not start more work.
    if any(run["id"] > current_id for run in runs):
        raise EvidenceError("newer same-head schedule exists")
    prior = sorted((r for r in runs if r["id"] != current_id),
                   key=lambda r: r["id"], reverse=True)
    if any(r.get("status") != "completed" for r in prior):
        raise EvidenceError("same-head schedule still active or status unreadable")
    for run in prior:
        if run.get("conclusion") not in COMPLETED_CONCLUSIONS:
            raise EvidenceError("scheduled conclusion unreadable or unsupported")
        state = heavy_state(read_jobs(get, repo, run))
        if state == "complete":
            return False, f"heavy completion verified in run {run['id']} attempt {run['run_attempt']}"
        if state == "incomplete":
            if doc["total_count"] > HISTORY_LIMIT:
                ensure_no_active(get, repo, head, current_id)
            return True, f"heavy work incomplete in run {run['id']}; admit this ordinary schedule"
    if doc["total_count"] > HISTORY_LIMIT:
        ensure_no_active(get, repo, head, current_id)
        return True, "completion proof aged out; admit periodic measurement on this ordinary schedule"
    if prior:
        return True, "only skipped follow-ups exist; this schedule must perform the heavy work"
    return True, "no prior same-head schedule; admit its first heavy attempt"


def main():
    if len(sys.argv) == 4 and sys.argv[1] == "--mutation-complete":
        complete = mutation_complete(int(sys.argv[2]), sys.argv[3])
        with open(os.environ["GITHUB_OUTPUT"], "a", encoding="utf-8") as output:
            output.write(f"measurement_completed={str(complete).lower()}\n")
        print(f"Mutation completion evidence: {str(complete).lower()}")
        return 0  # Evidence only; preserve the existing advisory command/ratchet policy.
    fresh = False
    try:
        fresh, reason = decide(os.environ.get("GITHUB_EVENT_NAME", ""),
                               os.environ.get("GITHUB_REPOSITORY", ""),
                               os.environ.get("GITHUB_SHA", ""),
                               int(os.environ.get("GITHUB_RUN_ID", "0")))
        code = 0
    except (EvidenceError, ValueError, TypeError) as exc:
        reason, code = f"nightly admission blocked: {exc}", 1
    with open(os.environ["GITHUB_OUTPUT"], "a", encoding="utf-8") as output:
        output.write(f"fresh={str(fresh).lower()}\n")
    print(reason)
    return code


if __name__ == "__main__":
    sys.exit(main())
