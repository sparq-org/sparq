#!/usr/bin/env python3
"""Assemble the sparq PR previews site (https://sparq-org.github.io/sparq-previews/).

Run by the sparq-org/sparq-previews repository's publish workflow, which checks out this
file from sparq's main branch (.github/site-previews/publish.yml is that workflow's source).

For each open pull request of SOURCE_REPO whose head branch lives in SOURCE_REPO, it takes
the successful `pull_request` run of SOURCE_WORKFLOW on the PR's current head SHA (or, while
that build is still running, the newest successful run for the PR), and extracts that run's
`site-preview-pr-<N>` artifact into <out>/pr-<N>/.

Trust rules (fork PRs are never published; artifacts are bound to their run):
  * only `pull_request` runs whose head repository is SOURCE_REPO and whose PR list, as
    reported by the API, contains N;
  * a run on the PR's current head wins, so re-running an old commit cannot displace it;
  * archives may contain only regular files and directories confined to pr-<N>/; each one
    is extracted into a temporary directory and moved into place only when complete;
  * each preview is size-checked against the remaining budget before anything is written.

Writes <out>/state.json (what was published) and exits with code 3 when that equals the
live site's state.json, so the workflow can skip the deploy.

Usage: site-preview-publish.py <out-dir>
Env: GH_TOKEN (read access to SOURCE_REPO's Actions artifacts), SOURCE_REPO,
     SOURCE_WORKFLOW, SITE_URL, PREVIEW_BUDGET_BYTES, FORCE_DEPLOY.
"""

import json
import os
import pathlib
import shutil
import stat
import sys
import tempfile
import urllib.error
import urllib.parse
import urllib.request
import zipfile

API = "https://api.github.com"
SOURCE_REPO = os.environ.get("SOURCE_REPO", "sparq-org/sparq")
SOURCE_WORKFLOW = os.environ.get("SOURCE_WORKFLOW", "site-preview.yml")
SITE_URL = os.environ.get("SITE_URL", "https://sparq-org.github.io/sparq-previews").rstrip("/")
# GitHub Pages caps a published site at 1 GB.
BUDGET = int(os.environ.get("PREVIEW_BUDGET_BYTES", str(900 * 1024 * 1024)))
UNCHANGED = 3


class Refused(Exception):
    pass


def request(url):
    req = urllib.request.Request(url, headers={
        "Accept": "application/vnd.github+json",
        "Authorization": f"Bearer {os.environ['GH_TOKEN']}",
        "X-GitHub-Api-Version": "2022-11-28",
    })
    return urllib.request.urlopen(req, timeout=120)


def api(path):
    with request(f"{API}/{path}") as resp:
        return json.loads(resp.read())


def open_prs():
    page = 1
    while True:
        prs = api(f"repos/{SOURCE_REPO}/pulls?state=open&per_page=100&page={page}")
        for pr in prs:
            head_repo = (pr.get("head") or {}).get("repo") or {}
            if head_repo.get("full_name") == SOURCE_REPO:
                yield pr["number"], pr["head"]["sha"], pr["head"]["ref"]
        if len(prs) < 100:
            return
        page += 1


def eligible(run, n):
    return (run["event"] == "pull_request"
            and (run.get("head_repository") or {}).get("full_name") == SOURCE_REPO
            and n in [p["number"] for p in run.get("pull_requests") or []])


def pick_run(n, head_sha, head_ref):
    """The PR's build to publish: its current head's, else its newest successful one."""
    base = f"repos/{SOURCE_REPO}/actions/workflows/{SOURCE_WORKFLOW}/runs?event=pull_request&status=success"
    for query in (f"&head_sha={head_sha}&per_page=20",
                  f"&branch={urllib.parse.quote(head_ref, safe='')}&per_page=50"):
        for run in api(base + query)["workflow_runs"]:   # newest first
            if eligible(run, n):
                return run
    return None


def run_artifact(run, n):
    for art in api(f"repos/{SOURCE_REPO}/actions/runs/{run['id']}/artifacts")["artifacts"]:
        if art["name"] == f"site-preview-pr-{n}" and not art["expired"]:
            return art
    return None


def check_archive(zf):
    """Validate every entry; return the total uncompressed size."""
    size = 0
    for info in zf.infolist():
        kind = stat.S_IFMT(info.external_attr >> 16)
        if kind not in (0, stat.S_IFREG, stat.S_IFDIR):
            raise Refused(f"{info.filename}: not a regular file or directory")
        parts = pathlib.PurePosixPath(info.filename.replace("\\", "/")).parts
        if info.filename.startswith(("/", "\\")) or ".." in parts or (parts and ":" in parts[0]):
            raise Refused(f"{info.filename}: escapes the preview directory")
        size += info.file_size
    return size


def extract(archive, out, n, budget_left):
    """Extract into <out>/pr-<n>/ atomically; return the bytes written."""
    dest = pathlib.Path(out).resolve() / f"pr-{n}"
    try:
        zf = zipfile.ZipFile(archive)
    except zipfile.BadZipFile as err:
        raise Refused(f"not a zip archive: {err}") from err
    with zf:
        size = check_archive(zf)
        if size > budget_left:
            raise Refused(f"{size} bytes exceeds the remaining budget of {budget_left}")
        staging = pathlib.Path(tempfile.mkdtemp(prefix=f".pr-{n}-", dir=dest.parent))
        try:
            for info in zf.infolist():
                target = (staging / info.filename).resolve()
                if staging != target and staging not in target.parents:
                    raise Refused(f"{info.filename}: escapes the preview directory")
                if info.is_dir():
                    target.mkdir(parents=True, exist_ok=True)
                    continue
                target.parent.mkdir(parents=True, exist_ok=True)
                with zf.open(info) as src, open(target, "wb") as dst:
                    shutil.copyfileobj(src, dst)
        except (zipfile.BadZipFile, OSError, EOFError, Refused) as err:
            shutil.rmtree(staging, ignore_errors=True)
            raise Refused(f"extraction failed: {err}") from err
    staging.chmod(0o755)
    if dest.exists():
        shutil.rmtree(dest)
    staging.rename(dest)
    return size


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

    wanted = []
    for n, head_sha, head_ref in sorted(open_prs(), reverse=True):
        run = pick_run(n, head_sha, head_ref)
        art = run and run_artifact(run, n)
        if art:
            wanted.append({"pr": n, "run": run["id"], "artifact": art["id"], "sha": run["head_sha"]})

    state = {"previews": wanted}
    if live_state() == state and os.environ.get("FORCE_DEPLOY") != "true":
        print("previews unchanged; nothing to deploy")
        return UNCHANGED

    published, used = [], 0
    with tempfile.TemporaryDirectory() as tmp:
        for p in wanted:
            archive = pathlib.Path(tmp) / f"pr-{p['pr']}.zip"
            with request(f"{API}/repos/{SOURCE_REPO}/actions/artifacts/{p['artifact']}/zip") as resp, \
                    open(archive, "wb") as fh:
                shutil.copyfileobj(resp, fh)
            try:
                used += extract(archive, out, p["pr"], BUDGET - used)
            except Refused as err:
                print(f"::warning::PR #{p['pr']} preview not published: {err}")
                continue
            finally:
                archive.unlink()
            published.append(p)
            print(f"published PR #{p['pr']} from run {p['run']}")

    # state.json records what was asked for, so a refused preview is not retried every pass.
    (out / "state.json").write_text(json.dumps(state, indent=2) + "\n")
    (out / ".nojekyll").write_text("")
    write_index(out, published)
    return 0


if __name__ == "__main__":
    sys.exit(main())
