#!/usr/bin/env python3
"""Assemble the sparq PR previews site (https://sparq-org.github.io/sparq-previews/).

Run by the sparq-org/sparq-previews repository's publish workflow, which checks out this
file from sparq's main branch (.github/site-previews/publish.yml is that workflow's source).

Reads, through the GitHub API, the open pull requests of SOURCE_REPO whose head branch
lives in SOURCE_REPO, finds each one's newest successful run of SOURCE_WORKFLOW, and
extracts that run's `site-preview-pr-<N>` artifact into <out>/pr-<N>/.

Trust rules (fork PRs are never published, artifacts are bound to their run):
  * a pull_request run counts for PR N only if the API lists N among the run's PRs;
  * a workflow_dispatch run counts only on the default branch, matched by its run-name
    "site-preview #<N>", which the source repo's main workflow code sets;
  * the run's head repository must be SOURCE_REPO;
  * a run on the PR's current head SHA wins over older ones, so re-running an old commit
    cannot displace the current preview;
  * archives may contain only regular files and directories confined to their pr-<N>/
    directory, and each preview is size-checked before it is added.

Writes <out>/state.json (what was published) and, when nothing changed since the live
site's state.json, exits with code 3 so the workflow can skip the deploy.

Usage: site-preview-publish.py <out-dir>   (env: GH_TOKEN, SOURCE_REPO, SOURCE_WORKFLOW, SITE_URL)
"""

import io
import json
import os
import pathlib
import stat
import sys
import urllib.error
import urllib.request
import zipfile

API = "https://api.github.com"
SOURCE_REPO = os.environ.get("SOURCE_REPO", "sparq-org/sparq")
SOURCE_WORKFLOW = os.environ.get("SOURCE_WORKFLOW", "site-preview.yml")
SITE_URL = os.environ.get("SITE_URL", "https://sparq-org.github.io/sparq-previews").rstrip("/")
# GitHub Pages caps a published site at 1 GB.
BUDGET = int(os.environ.get("PREVIEW_BUDGET_BYTES", str(900 * 1024 * 1024)))
UNCHANGED = 3


def request(url, accept="application/vnd.github+json"):
    req = urllib.request.Request(url, headers={
        "Accept": accept,
        "Authorization": f"Bearer {os.environ['GH_TOKEN']}",
        "X-GitHub-Api-Version": "2022-11-28",
    })
    with urllib.request.urlopen(req, timeout=120) as resp:
        return resp.read()


def api(path):
    return json.loads(request(f"{API}/{path}"))


def paginate(path, key=None):
    page = 1
    while True:
        sep = "&" if "?" in path else "?"
        data = api(f"{path}{sep}per_page=100&page={page}")
        items = data[key] if key else data
        yield from items
        if len(items) < 100:
            return
        page += 1


def open_prs():
    for pr in paginate(f"repos/{SOURCE_REPO}/pulls?state=open"):
        head_repo = (pr.get("head") or {}).get("repo") or {}
        if head_repo.get("full_name") == SOURCE_REPO:
            yield pr["number"], pr["head"]["sha"]


def successful_runs(default_branch):
    """Newest successful source-workflow runs, keyed by the PR each one previews."""
    by_pr = {}
    runs = api(f"repos/{SOURCE_REPO}/actions/workflows/{SOURCE_WORKFLOW}/runs"
               "?status=success&per_page=100")["workflow_runs"]
    for run in runs:
        if (run.get("head_repository") or {}).get("full_name") != SOURCE_REPO:
            continue
        if run["event"] == "pull_request":
            prs = [p["number"] for p in run.get("pull_requests") or []]
        elif run["event"] == "workflow_dispatch" and run["head_branch"] == default_branch:
            title = run.get("display_title") or ""
            num = title.removeprefix("site-preview #")
            prs = [int(num)] if title.startswith("site-preview #") and num.isdigit() else []
        else:
            prs = []
        for n in prs:
            by_pr.setdefault(n, []).append(run)
    return by_pr


def pick_run(runs, head_sha):
    # API order is newest first. A pull_request run on the current head wins.
    for run in runs:
        if run["event"] == "pull_request" and run["head_sha"] == head_sha:
            return run
    return runs[0] if runs else None


def run_artifact(run, n):
    for art in api(f"repos/{SOURCE_REPO}/actions/runs/{run['id']}/artifacts")["artifacts"]:
        if art["name"] == f"site-preview-pr-{n}" and not art["expired"]:
            return art
    return None


def safe_extract(blob, dest, budget_left):
    """Extract only confined regular files and directories; return bytes written.

    Raises ValueError, leaving nothing behind, when the archive breaks a rule.
    """
    zf = zipfile.ZipFile(io.BytesIO(blob))
    root = dest.resolve()
    members, total = [], 0
    for info in zf.infolist():
        mode = info.external_attr >> 16
        kind = stat.S_IFMT(mode)
        if kind not in (0, stat.S_IFREG, stat.S_IFDIR):
            raise ValueError(f"{info.filename}: not a regular file or directory")
        target = (root / info.filename).resolve()
        if root != target and root not in target.parents:
            raise ValueError(f"{info.filename}: escapes the preview directory")
        total += info.file_size
        members.append((info, target))
    if total > budget_left:
        raise ValueError(f"{total} bytes exceeds the remaining budget of {budget_left}")
    for info, target in members:
        if info.is_dir():
            target.mkdir(parents=True, exist_ok=True)
            continue
        target.parent.mkdir(parents=True, exist_ok=True)
        with zf.open(info) as src, open(target, "wb") as dst:
            dst.write(src.read())
    return total


def live_state():
    try:
        with urllib.request.urlopen(f"{SITE_URL}/state.json", timeout=30) as resp:
            return json.loads(resp.read())
    except (urllib.error.URLError, ValueError):
        return None


def write_index(out, published):
    rows = "\n".join(
        f'<li><a href="pr-{p["pr"]}/">PR #{p["pr"]}</a> '
        f'(<a href="https://github.com/{SOURCE_REPO}/pull/{p["pr"]}">pull request</a>, '
        f'{p["sha"][:7]})</li>'
        for p in published
    ) or "<li>No open previews.</li>"
    (out / "index.html").write_text(f"""<!doctype html>
<html lang="en"><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<meta name="robots" content="noindex">
<title>sparq PR previews</title></head>
<body><h1>sparq PR previews</h1>
<p>Unmerged pull requests of <a href="https://github.com/{SOURCE_REPO}">{SOURCE_REPO}</a>.
The published site is <a href="https://sparq.jeswr.org/">sparq.jeswr.org</a>.</p>
<ul>
{rows}
</ul></body></html>
""")


def main():
    out = pathlib.Path(sys.argv[1])
    out.mkdir(parents=True, exist_ok=True)
    default_branch = api(f"repos/{SOURCE_REPO}")["default_branch"]
    runs_by_pr = successful_runs(default_branch)

    wanted = []
    for n, head_sha in sorted(open_prs(), reverse=True):
        run = pick_run(runs_by_pr.get(n, []), head_sha)
        art = run and run_artifact(run, n)
        if art:
            wanted.append({"pr": n, "run": run["id"], "artifact": art["id"],
                           "sha": run["head_sha"] if run["event"] == "pull_request" else head_sha})

    state = {"previews": wanted}
    if live_state() == state and os.environ.get("FORCE_DEPLOY") != "true":
        print("previews unchanged; nothing to deploy")
        return UNCHANGED

    published, used = [], 0
    for p in wanted:
        dest = out / f"pr-{p['pr']}"
        blob = request(f"{API}/repos/{SOURCE_REPO}/actions/artifacts/{p['artifact']}/zip",
                       accept="application/vnd.github+json")
        try:
            used += safe_extract(blob, dest, BUDGET - used)
        except (ValueError, zipfile.BadZipFile) as err:
            print(f"::warning::PR #{p['pr']} preview not published: {err}")
            continue
        published.append(p)
        print(f"published PR #{p['pr']} from run {p['run']}")

    # state.json records what was asked for, so a skipped preview is not retried every pass.
    (out / "state.json").write_text(json.dumps(state, indent=2) + "\n")
    (out / ".nojekyll").write_text("")
    write_index(out, published)
    return 0


if __name__ == "__main__":
    sys.exit(main())
