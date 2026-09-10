"""One aggregate-only recovery after the current PR cohort completes."""
from pathlib import Path
import datetime
import json
import subprocess

A = Path(__file__).parent
HEAD = "9ac4a59e96b1c69c8d617fbde9e98fe8c0436fed"
RUN = 34511329028
JOB = 102985912054
RECEIPT = A / "recover6486-aggregate-receipt.json"
assert not RECEIPT.exists(), "Reconcile the existing attempt; never replay"
receipt = {
    "started_at": datetime.datetime.now(datetime.timezone.utc).isoformat(),
    "scope": "One failed ci-summary job only, after current peers succeed",
    "source_recovery": "recover-paths-filter-receipt.json",
}


def save():
    RECEIPT.write_text(json.dumps(receipt, indent=2) + "\n")


def api(endpoint, name, method="GET"):
    p = subprocess.run(
        ["/opt/homebrew/bin/gh", "api", endpoint, "-X", method],
        capture_output=True, text=True,
    )
    receipt[name] = {"exit_code": p.returncode, "stderr": p.stderr}
    save()
    if p.returncode:
        raise RuntimeError("GitHub request failed; no retry: " + name)
    data = json.loads(p.stdout) if p.stdout.strip() else None
    (A / ("recover6486-aggregate-" + name + ".json")).write_text(
        json.dumps(data, indent=2) + "\n"
    )
    return data


try:
    budget = api("rate_limit", "budget")
    assert budget["resources"]["core"]["remaining"] > 100
    runs = api(
        "repos/sparq-org/sparq/actions/runs?head_sha=" + HEAD + "&per_page=100",
        "runs",
    )
    assert len(runs["workflow_runs"]) == runs["total_count"]
    latest = {}
    by_id = {}
    for run in runs["workflow_runs"]:
        assert run["head_sha"] == HEAD
        by_id[run["id"]] = run
        key = (run["path"], run["event"])
        if key not in latest or run["id"] > latest[key]["id"]:
            latest[key] = run
    assert by_id[34511329235]["conclusion"] == "success"
    assert by_id[34511329512]["conclusion"] == "success"
    source = by_id[34511329019]
    assert source["conclusion"] == "success" and source["run_attempt"] == 2
    assert latest[(".github/workflows/ci-summary.yml", "pull_request")]["id"] == RUN
    for run in latest.values():
        assert run["status"] == "completed", (run["id"], run["status"])
        if run["id"] == RUN:
            assert run["conclusion"] == "failure" and run["run_attempt"] == 1
        else:
            assert run["conclusion"] in ("success", "skipped"), (
                run["id"], run["conclusion"]
            )
    registry = api("repos/jeswr/agent-account-registry/actions/permissions", "registry")
    assert registry["enabled"] is False
    jobs = api(f"repos/sparq-org/sparq/actions/runs/{RUN}/jobs?per_page=100", "jobs")
    assert jobs["total_count"] == 1 and len(jobs["jobs"]) == 1
    job = jobs["jobs"][0]
    assert job["id"] == JOB and job["name"] == "gate"
    assert job["status"] == "completed" and job["conclusion"] == "failure"
    current = api(f"repos/sparq-org/sparq/actions/runs/{RUN}", "immediate-run")
    assert current["head_sha"] == HEAD and current["run_attempt"] == 1
    assert current["path"] == ".github/workflows/ci-summary.yml"
    assert current["event"] == "pull_request"
    assert current["status"] == "completed" and current["conclusion"] == "failure"
    pr = api("repos/sparq-org/sparq/pulls/6486", "immediate-pr")
    assert pr["state"] == "open" and not pr["draft"] and pr["head"]["sha"] == HEAD
    assert pr["base"]["ref"] == "main"
    receipt["mutation_started_at"] = datetime.datetime.now(datetime.timezone.utc).isoformat()
    save()
    api(f"repos/sparq-org/sparq/actions/jobs/{JOB}/rerun", "single-rerun", "POST")
    receipt["completed_at"] = datetime.datetime.now(datetime.timezone.utc).isoformat()
    receipt["result"] = "One aggregate-only rerun accepted; result still pending"
    save()
    print(json.dumps({"run": RUN, "job": JOB, "result": receipt["result"]}))
except Exception as error:
    receipt["error"] = str(error)
    save()
    raise
