"""Publish the reviewed, tested comparator repair once, through normal CI."""
from pathlib import Path
import datetime
import hashlib
import json
import subprocess

A = Path(__file__).parent
R = A.parent.parent
W = R / "worktrees/issue6483"
HEAD = "0b4554b924a80432cc1b572bd19f8e58cdbfb4e6"
BASE = "f50b5049627415a0f8fd1eca6dd3cb9ac5fcd464"
BRANCH = "codex/update-comparator-raw-duplicates"
RECEIPT = A / "publication-receipt.json"
assert not RECEIPT.exists(), "Reconcile an existing publication; never replay"
receipt = {"started_at": datetime.datetime.now(datetime.timezone.utc).isoformat()}


def save():
    RECEIPT.write_text(json.dumps(receipt, indent=2) + "\n")


def api(endpoint, name, payload=None):
    args = ["/opt/homebrew/bin/gh", "api", endpoint]
    if payload is not None:
        path = A / ("publication-" + name + "-input.json")
        path.write_text(json.dumps(payload) + "\n")
        args += ["-X", "POST", "--input", str(path)]
    p = subprocess.run(args, capture_output=True, text=True)
    receipt[name] = {"exit_code": p.returncode, "stderr": p.stderr}
    save()
    if p.returncode:
        raise RuntimeError("Request failed; no retry: " + name)
    data = json.loads(p.stdout) if p.stdout.strip() else None
    (A / ("publication-" + name + ".json")).write_text(json.dumps(data, indent=2) + "\n")
    return data


def git(*args):
    return subprocess.run(["git", "-C", str(W), *args], check=True, capture_output=True).stdout


try:
    review = json.loads((A / "opus-duplicate-final-result.json").read_text())
    assert review["reviewed_head"] == HEAD and review["verdict"] == "APPROVE_FOR_CI"
    assert not review["blocking_findings"]
    proof = json.loads((A / "validation-continuation-root-verification.json").read_text())
    assert proof["head"] == HEAD and proof["clean"] and proof["verified_files"] == 47
    assert git("rev-parse", "HEAD").decode().strip() == HEAD
    assert git("rev-parse", "HEAD^").decode().strip() == BASE
    assert git("branch", "--show-current").decode().strip() == BRANCH
    assert not git("status", "--porcelain")
    assert git("diff", "--name-only", BASE, HEAD).decode().splitlines() == ["crates/sparq-bench/src/update_fuzz.rs"]
    assert hashlib.sha256(git("show", HEAD + ":crates/sparq-bench/src/update_fuzz.rs")).hexdigest() == proof["source_sha256"]
    assert "sparq-org/sparq" in git("remote", "get-url", "origin").decode()
    budget = api("rate_limit", "budget")
    assert budget["resources"]["core"]["remaining"] > 100
    registry = api("repos/jeswr/agent-account-registry/actions/permissions", "registry")
    assert registry["enabled"] is False
    existing = api("repos/sparq-org/sparq/pulls?state=all&head=sparq-org:" + BRANCH + "&per_page=100", "existing-prs")
    assert existing == []
    refs = api("repos/sparq-org/sparq/git/matching-refs/heads/" + BRANCH, "existing-refs")
    assert refs == []
    issue = api("repos/sparq-org/sparq/issues/6483", "issue")
    assert issue["state"] == "open" and not issue["assignees"]
    labels = {x["name"] for x in issue["labels"]}
    assert "status:ready" in labels and not labels.intersection({"needs:user", "blocked", "hold"})
    main = api("repos/sparq-org/sparq/git/ref/heads/main", "main")
    assert main["object"]["sha"] == BASE
    receipt["push_started_at"] = datetime.datetime.now(datetime.timezone.utc).isoformat()
    save()
    pushed = subprocess.run(["git", "-C", str(W), "push", "origin", HEAD + ":refs/heads/" + BRANCH], capture_output=True, text=True)
    receipt["push"] = {"exit_code": pushed.returncode, "stdout": pushed.stdout, "stderr": pushed.stderr}
    save()
    assert pushed.returncode == 0, "Push failed; no retry"
    remote = api("repos/sparq-org/sparq/git/ref/heads/" + BRANCH, "published-head")
    assert remote["object"]["sha"] == HEAD
    current_issue = api("repos/sparq-org/sparq/issues/6483", "immediate-issue")
    assert current_issue["state"] == "open" and not current_issue["assignees"]
    pr = api("repos/sparq-org/sparq/pulls", "create-pr", {
        "title": "fix(bench): reject duplicate raw quads in UPDATE comparisons",
        "head": BRANCH, "base": "main", "draft": False,
        "body": (A / "pr-ready-body.md").read_text(),
    })
    assert pr["head"]["sha"] == HEAD and pr["state"] == "open" and not pr["draft"]
    receipt.update(number=pr["number"], url=pr["html_url"], head=HEAD, completed_at=datetime.datetime.now(datetime.timezone.utc).isoformat())
    save()
    print(json.dumps({"number": pr["number"], "url": pr["html_url"], "head": HEAD}))
except Exception as error:
    receipt["error"] = str(error)
    save()
    raise
