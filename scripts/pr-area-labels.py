#!/usr/bin/env python3
# [OPUS-5] 🤖 SPARQ agent — derive a PR's `area:` conflict-partition labels from its
# changed paths. Registry issue jeswr/agent-account-registry#677.
"""pr-area-labels.py — the ONE deriver used by both the per-PR workflow and the backfill.

WHY (measured, reg#677 2026-07-26): the fleet scheduler partitions work by `area:<name>`.
An in-flight PR RESERVES its areas; a ready issue defers while any of its areas is
reserved. A PR with NO `area:` label maps to the SERIALIZING `__global__` partition, so
one such PR defers EVERY issue. 84 of 87 open PRs had no `area:` label because nothing
in the pipeline ever applied one — making this layer, not `max_concurrent` (reg#689:
net +0 workers) and not the 28-slot account pool, the binding limiter on throughput.

THE CONTRACT
  * `__global__` is expressed as the ABSENCE of any `area:` label. This script only ever
    ADDS labels; there is no removal code path at all, so a human-applied `area:` label
    can never be dropped (`apply_areas` records every request; the test asserts no
    DELETE is ever issued).
  * FAIL CLOSED: it emits NO labels — leaving the PR on `__global__` — when the changed
    paths are unavailable (`no-paths`), only PARTIALLY available (`incomplete-paths`),
    when ANY path is unattributable (`unresolved`), or when more than
    `[policy] max_areas` distinct areas are implicated (`cross-cutting`). Unknown blast
    radius must serialize. See ci/area-labels.toml.
  * THE INPUT BOUNDARY IS THE PROOF OBLIGATION. Attribution is a pure function of the
    path string, so within a COMPLETE list under-reservation is impossible. It is the
    list itself that cannot be trusted: `gh pr view --json files` is GraphQL
    `files(first: 100)` and SILENTLY TRUNCATES (measured on this repo: PR #3581 reports
    `changedFiles = 646` and `--json files` returns 100). A truncated list derives a
    PROPER SUBSET of the true areas — the CORRUPTING direction, because a too-narrow
    reservation puts two workers on one crate, while a too-broad one only delays. So the
    file list is enumerated with the PAGINATED REST endpoint AND cross-checked against
    the authoritative `changedFiles` count; any disagreement is `incomplete-paths`.
    Renames are consumed via `previous_filename` (REST-only), so a cross-crate move
    implicates BOTH the source and the destination crate.
  * It creates a label in EXACTLY ONE case, on a STRUCTURAL WITNESS. GitHub's "add labels
    to an issue" REST call SILENTLY CREATES an unknown label, so a typo'd area would
    become permanent repo state; every candidate area is therefore checked against the
    repo's live label set first and dropped (loudly) if absent. The ONE exception
    (issue #4582) is a PR that ADDS `crates/<name>/Cargo.toml`: that entry, carrying a
    base-ref-absent status, is PROOF — not a guess — that this PR creates crate `<name>`,
    and such a PR is otherwise unattributable FOREVER, because the workflow evaluates it
    against the DEFAULT BRANCH, where the crate does not exist and no `area:<name>` label
    was ever made. There the label is created EXPLICITLY (`POST /repos/{repo}/labels`,
    with `new_crate_area` deciding the name) and, if creation fails, the run FAILS LOUDLY
    rather than warning and falling back to `__global__`. With no such witness the
    fail-closed behaviour is unchanged.
  * IDEMPOTENT: areas already on the PR are never re-posted; a second run over an
    already-labelled PR issues zero writes.
  * READ-ONLY BY DEFAULT (repo convention, cf. scripts/push-frontier.sh): `--dry-run` is
    the default and prints exactly what it would do. `--apply` is required to mutate.

MODES
  --pr N        derive (and with --apply, label) a single PR. Used by the workflow.
  --backfill    derive (and with --apply, label) every OPEN PR in one pass.
  --self-test   hermetic assertions; no network. Run by routing-self-tests.yml.

FORK PRs: `pull_request` (not `pull_request_target`) hands a fork PR a READ-ONLY token,
so no label write can succeed. That path is handled EXPLICITLY: pass
`--head-repo <full_name>` and the run becomes a reporting no-op with a `::notice` when
the head repo is not this repo. It is not left to fail confusingly on a 403.
"""
from __future__ import annotations

import argparse
import json
import os
import re
import subprocess
import sys
import tomllib
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
POLICY_PATH = REPO_ROOT / "ci" / "area-labels.toml"
WORKSPACE_MANIFEST = REPO_ROOT / "Cargo.toml"

GLOBAL = "__global__"          # mirrors ready-issues.py / dispatch-claim.py
AREA_PREFIX = "area:"
CRATES_DIR = "crates"
CRATE_MANIFEST = "Cargo.toml"

# The `status` values GitHub's `pulls/{n}/files` payload uses for an entry whose CURRENT
# path does NOT exist at that path on the base ref. `modified` is deliberately absent:
# editing an existing crate's manifest is not evidence that the crate is new.
BASE_ABSENT_STATUSES = frozenset({"added", "copied", "renamed"})
# A created label's name is derived from a PR-authored directory name, so it is
# constrained here rather than trusted: no whitespace, no comma (which GitHub's label
# filters treat as a separator), no `:` beyond the prefix, bounded length.
AREA_NAME_RE = re.compile(r"[A-Za-z0-9][A-Za-z0-9._-]{0,63}")
# Cosmetic; nothing structural reads either value. The description records WHY the label
# exists so a human auditing the label list can tell an auto-created one from a hand-made
# one. GitHub caps a label description at 100 characters.
AREA_LABEL_COLOR = "c5def5"
AREA_LABEL_DESCRIPTION = "Scheduler conflict partition (auto-created for a new crate)"


class PolicyError(RuntimeError):
    """A malformed ci/area-labels.toml. Fail loudly — never silently label nothing."""


# --------------------------------------------------------------------------- policy


class Policy:
    """The parsed ci/area-labels.toml: the ordered pattern rows + the guard rails."""

    def __init__(self, max_areas, rows, non_crate, members):
        self.max_areas = max_areas
        self.rows = rows                # [(pattern, area)] in FILE ORDER; first match wins
        self.non_crate = frozenset(non_crate)
        self.members = frozenset(members)   # live workspace crate dir names

    def declared_areas(self):
        return {area for _, area in self.rows}


def load_members(manifest=WORKSPACE_MANIFEST):
    """The workspace crate directory names, read from the root manifest (hermetic — no
    `cargo metadata`, no toolchain).

    NOT a gate on attribution: see `attribute` for why a PR that ADDS a crate must still
    resolve. Membership is used for the hermetic table-validity assertions and for the
    `::warning` about declared areas that have no label.
    """
    data = tomllib.loads(Path(manifest).read_text(encoding="utf-8"))
    members = ((data.get("workspace") or {}).get("members") or [])
    out = set()
    for m in members:
        if not isinstance(m, str):
            raise PolicyError(f"non-string workspace member: {m!r}")
        parts = m.strip("/").split("/")
        if len(parts) == 2 and parts[0] == CRATES_DIR:
            out.add(parts[1])
    if not out:
        raise PolicyError(f"no crates/* workspace members found in {manifest}")
    return out


def load_policy(path=POLICY_PATH, manifest=WORKSPACE_MANIFEST):
    data = tomllib.loads(Path(path).read_text(encoding="utf-8"))
    policy = data.get("policy") or {}
    max_areas = policy.get("max_areas")
    if not isinstance(max_areas, int) or isinstance(max_areas, bool) or max_areas < 1:
        raise PolicyError(f"{path}: [policy] max_areas must be a positive int")
    non_crate = (data.get("lanes") or {}).get("non_crate") or []
    if not isinstance(non_crate, list) or not all(isinstance(x, str) and x for x in non_crate):
        raise PolicyError(f"{path}: [lanes] non_crate must be a list of non-empty strings")
    rows = []
    for i, row in enumerate(data.get("map") or []):
        if not isinstance(row, dict):
            raise PolicyError(f"{path}: [[map]] entry {i} is not a table")
        pattern, area = row.get("pattern"), row.get("area")
        if not isinstance(pattern, str) or not pattern:
            raise PolicyError(f"{path}: [[map]] entry {i} has no `pattern`")
        if not isinstance(area, str) or not area:
            raise PolicyError(f"{path}: [[map]] {pattern!r} has no `area`")
        rows.append((pattern, area))
    if not rows:
        raise PolicyError(f"{path}: no [[map]] rows — every non-crate path would be unresolved")
    return Policy(max_areas, rows, non_crate, load_members(manifest))


def _segment_glob(pattern):
    """Translate a glob to a regex whose `*`/`?` do NOT cross `/`.

    The deliberate difference from ci/path-ownership.toml's `fnmatch` (documented in
    ci/area-labels.toml): with fnmatch, a `*.md` row also matches
    `skills/inference/SKILL.md`, which makes the whole map order-fragile. Here `*.md`
    means a ROOT markdown file and nothing else.
    """
    out = ["(?s:"]
    for ch in pattern:
        if ch == "*":
            out.append("[^/]*")
        elif ch == "?":
            out.append("[^/]")
        else:
            out.append(re.escape(ch))
    out.append(")\\Z")
    return re.compile("".join(out))


def matches(pattern, path):
    """Does `pattern` (see ci/area-labels.toml PATTERN GRAMMAR) match repo-relative `path`?"""
    if pattern.endswith("/**"):
        root = pattern[:-3]
        return path == root or path.startswith(root + "/")
    return bool(_segment_glob(pattern).match(path))


def attribute(path, policy):
    """The single area a changed path implicates, or None when nothing claims it.

    Resolution order is normative (ci/area-labels.toml): the implicit
    `crates/<name>/...` rule first, then the first matching `[[map]]` row.
    """
    if not isinstance(path, str) or not path:
        return None
    path = path[2:] if path.startswith("./") else path
    parts = path.split("/")
    if parts[0] == CRATES_DIR:
        # A CRATE DIRECTORY is `crates/<name>/<something>` — at least three segments.
        # A file sitting directly in `crates/` (two segments, e.g. `crates/README.md`)
        # is NOT a crate and resolves to nothing => fail closed.
        if len(parts) < 3:
            return None
        # Deliberately NOT gated on workspace membership. The workflow evaluates a PR
        # against the DEFAULT BRANCH's manifest (a `pull-requests: write` job must not
        # run PR-authored code), so a PR that ADDS a crate would be permanently
        # unresolvable — and those are exactly the PRs that collide with nothing and
        # most deserve a narrow partition. The guard against inventing an area is the
        # caller's live-`area:`-label check, which a non-crate name cannot pass.
        return parts[1]
    for pattern, area in policy.rows:
        if matches(pattern, path):
            return area
    return None


def new_crate_area(path, status):
    """The crate area a changed-file ENTRY WITNESSES as created by this PR, else None.

    THE WITNESS (issue #4582). A PR that adds a crate is unattributable by every other
    rule here: `attribute` resolves `crates/<name>/...` to `<name>` happily, but the
    caller's live-`area:`-label check then drops it, because on the DEFAULT BRANCH — the
    only tree this privileged job may read — the crate does not exist and nobody ever made
    an `area:<name>` label. The measured case is PR #4578 (`crates/sparq-wrapper-shacl`),
    which the deriver reported as `KEEP __global__ [unresolved]`.

    What makes creating that label safe is that this is not a guess. The entry's CURRENT
    path is exactly `crates/<name>/Cargo.toml` — three segments, the crate root manifest —
    and its `status` says that manifest is NOT at that path on the base ref. A directory
    named `crates/<name>/` with no added root manifest is NOT a crate (it could be a typo,
    a stray file, or a subdirectory of an existing crate), and produces no witness, so it
    keeps the old fail-closed answer. `crates/README.md` has two segments and produces
    none either.

    The name is then range-checked against `AREA_NAME_RE`, because it is the one part of
    a created label that comes from PR-authored content.
    """
    if not isinstance(path, str) or not isinstance(status, str):
        return None
    if status.strip().lower() not in BASE_ABSENT_STATUSES:
        return None
    parts = (path[2:] if path.startswith("./") else path).split("/")
    if len(parts) != 3 or parts[0] != CRATES_DIR or parts[2] != CRATE_MANIFEST:
        return None
    return parts[1] if AREA_NAME_RE.fullmatch(parts[1]) else None


def creatable_areas(pr, known_areas):
    """The area names this PR record is ALLOWED to have a label created for.

    Exactly the witnessed new crates (`new_crate_area`) that have no label yet. A name
    that already exists as a label needs no creation, and a name with no witness is not
    creatable at all — so this set, not the changed-path list, is the whole authority for
    the one write that can add permanent repo state.
    """
    return frozenset(c for c in (pr.get("new_crates") or ())
                     if isinstance(c, str) and c not in known_areas
                     and AREA_NAME_RE.fullmatch(c))


def derive_areas(paths, policy, known_areas, complete=True, creatable=frozenset()):
    """(areas, reason) for a changed-path list. EMPTY areas == stay on `__global__`.

    `known_areas` is the set of area names for which an `area:<name>` label ACTUALLY
    exists in the repo; an area outside it is treated as unresolved, because applying
    it would silently create a label (see module docstring).

    `creatable` extends that set with the areas this PR is entitled to have a label
    CREATED for — the witnessed new crates from `creatable_areas`. It widens attribution
    ONLY for a name the PR's own added crate manifest proves, and the widening is what
    turns a new-crate PR from `unresolved` into `resolved`.

    `complete` is the verdict of `paths_are_complete` — whether `paths` is KNOWN to be
    the whole changed-file list. A partial list derives a proper SUBSET of the true
    areas, which is the corrupting direction, so it emits nothing.

    The reason vocabulary is exactly {"resolved", "no-paths", "incomplete-paths",
    "unresolved", "cross-cutting"}; the latter four all mean "emit nothing, stay on
    `__global__`" and are distinguished only so the log says WHY. Callers branch on
    `== "resolved"`, so a future reason is fail-closed by construction.
    """
    if not complete:
        # TRUNCATED / partial enumeration. See `paths_are_complete`.
        return frozenset(), "incomplete-paths"
    if not paths:
        return frozenset(), "no-paths"
    attributable = frozenset(known_areas) | frozenset(creatable)
    areas, unresolved = set(), []
    for path in paths:
        area = attribute(path, policy)
        if area is None or area not in attributable:
            unresolved.append(path)
            continue
        areas.add(area)
    if unresolved:
        # STRICT on purpose: one unattributable path means unknown blast radius.
        return frozenset(), "unresolved"
    if not areas:
        return frozenset(), "no-paths"
    if len(areas) > policy.max_areas:
        return frozenset(), "cross-cutting"
    return frozenset(areas), "resolved"


def paths_are_complete(enumerated, changed):
    """Is an enumeration of `enumerated` changed-file ENTRIES the WHOLE changed set?

    `changed` is GitHub's authoritative `changedFiles` count for the PR, fetched
    independently of the file list. They disagree exactly when the enumeration was
    capped — GraphQL's `files(first: 100)`, or the REST endpoint's 3000-file ceiling.

    Fail closed on anything that is not a confident, exact agreement: a missing or
    non-integer `changed` means we could not establish completeness at all, which is
    NOT the same as having established it.
    """
    if not isinstance(enumerated, int) or isinstance(enumerated, bool) or enumerated < 0:
        return False
    if not isinstance(changed, int) or isinstance(changed, bool) or changed < 0:
        return False
    return enumerated == changed


def plan_for_pr(pr, policy, known_areas):
    """The full decision for one PR dict {number, labels:[str], files:[str], complete:bool}.

    Returns a dict with the derived areas, the labels to ADD (never to remove), and the
    reason. `partition` is what the scheduler will see AFTER the add.

    `complete` must be EXACTLY `True` for any label to be emitted. A record that omits it
    — a fetcher that never established that its file list is whole — therefore fails
    CLOSED rather than deriving from a possibly-truncated list.
    """
    existing = {lb[len(AREA_PREFIX):] for lb in pr.get("labels") or []
                if isinstance(lb, str) and lb.startswith(AREA_PREFIX)}
    creatable = creatable_areas(pr, known_areas)
    areas, reason = derive_areas(pr.get("files") or [], policy, known_areas,
                                 complete=pr.get("complete") is True, creatable=creatable)
    add = sorted(areas - existing)
    after = existing | areas
    return {
        "number": pr.get("number"),
        "existing": sorted(existing),
        "derived": sorted(areas),
        "add": [AREA_PREFIX + a for a in add],
        # A SUBSET of `add`, so a label is never created unless it is also about to be
        # applied: a fail-closed plan derives no areas and therefore creates nothing.
        "create": [AREA_PREFIX + a for a in add if a in creatable],
        "reason": reason,
        # The scheduler reads absence-of-area as `__global__`; a PR that keeps a
        # human-applied area is NOT global even when derivation failed.
        "partition": sorted(after) if after else [GLOBAL],
        "noop": not add,
    }


def unresolved_paths(pr, policy, known_areas):
    """The paths that blocked narrowing — the actionable output of a `unresolved` run."""
    attributable = frozenset(known_areas) | creatable_areas(pr, known_areas)
    out = []
    for path in pr.get("files") or []:
        area = attribute(path, policy)
        if area is None or area not in attributable:
            out.append(path)
    return out


# ------------------------------------------------------------------------ github I/O


def _gh(args, runner=None, attempts=3, sleep=None):
    """Run `gh`, retrying a TRANSIENT failure a bounded number of times.

    The retry exists so a 502/secondary-rate-limit blip does not red a gating check; it
    deliberately does NOT swallow a persistent failure — after `attempts` tries the error
    is raised and the job fails. (This repo has been bitten three times by a later
    transient DISCARDING an earned hard failure; the direction here is the opposite.)
    """
    if runner is not None:
        return runner(args)
    last = ""
    for attempt in range(attempts):
        proc = subprocess.run(["gh", *args], capture_output=True, text=True, check=False)
        if proc.returncode == 0:
            return proc.stdout
        last = proc.stderr.strip()[:400]
        if attempt + 1 < attempts:
            (sleep or __import__("time").sleep)(2 * (attempt + 1))
    raise RuntimeError(f"gh {' '.join(args)} failed after {attempts} attempt(s): {last}")


def known_area_labels(repo, runner=None):
    """The area names that already exist as labels. Fail-closed: an empty/failed read
    yields an empty set, so NOTHING is labelled and every PR stays `__global__` —
    the status quo, never a wrong narrow label."""
    raw = _gh(["api", "--paginate", f"repos/{repo}/labels?per_page=100",
               "--jq", ".[].name"], runner=runner)
    return {ln[len(AREA_PREFIX):] for ln in raw.splitlines()
            if ln.startswith(AREA_PREFIX) and len(ln) > len(AREA_PREFIX)}


def fetch_changed_files(repo, number, runner=None):
    """(paths, entry_count, new_crates) for a PR, from the PAGINATED REST endpoint.

    `gh pr view --json files` is GraphQL `files(first: 100)` and is NOT used anywhere in
    this module: it silently truncates, and a truncated list derives a proper SUBSET of
    the true areas (module docstring). `gh api --paginate` walks every page.

    `paths` contains, for each entry, the file's CURRENT path and — for a rename — its
    `previous_filename` as well, so a move out of `crates/A/` into `crates/B/` implicates
    BOTH crates. `entry_count` counts ENTRIES, not paths, because that is the number
    `changedFiles` is comparable with.

    `new_crates` is the crate names this PR is WITNESSED to create — see `new_crate_area`.
    It needs each entry's `status`, which is why the REST payload (not the path list
    alone) is the input: `status` is what distinguishes adding a crate manifest from
    editing one, and only the former justifies creating a label.
    """
    raw = _gh(["api", "--paginate", f"repos/{repo}/pulls/{number}/files?per_page=100",
               "--jq", r'.[] | [.filename, (.previous_filename // ""), .status] | @tsv'],
              runner=runner)
    paths, entries, new_crates = [], 0, set()
    for line in raw.splitlines():
        if not line.strip():
            continue
        entries += 1
        cols = line.split("\t")
        current = cols[0].strip()
        previous = cols[1].strip() if len(cols) > 1 else ""
        status = cols[2].strip() if len(cols) > 2 else ""
        for p in (current, previous):
            if p and p not in paths:
                paths.append(p)
        witnessed = new_crate_area(current, status)
        if witnessed:
            new_crates.add(witnessed)
    return paths, entries, frozenset(new_crates)


def _with_changed_files(repo, record, changed, runner=None):
    """Attach the enumerated paths + the completeness verdict to a PR record."""
    paths, entries, new_crates = fetch_changed_files(repo, record["number"], runner=runner)
    record["files"] = paths
    record["new_crates"] = sorted(new_crates)
    record["complete"] = paths_are_complete(entries, changed)
    return record


def fetch_pr(repo, number, runner=None):
    # `changedFiles` is the authoritative count and is fetched INDEPENDENTLY of the file
    # list, so the two can be cross-checked. `files` is deliberately NOT requested here.
    raw = _gh(["pr", "view", str(number), "--repo", repo,
               "--json", "number,labels,changedFiles"], runner=runner)
    data = json.loads(raw)
    return _with_changed_files(repo, {
        "number": data.get("number", number),
        "labels": [lb.get("name") for lb in data.get("labels") or []],
    }, data.get("changedFiles"), runner=runner)


def fetch_open_prs(repo, limit=300, runner=None):
    # Same rule as fetch_pr: the list call carries metadata + the authoritative count,
    # never the capped GraphQL `files` connection. The per-PR REST enumeration costs one
    # extra call per open PR; a backfill is a hand-run, dry-run-by-default operation, and
    # a wrong narrow label is not worth saving ~90 requests.
    raw = _gh(["pr", "list", "--repo", repo, "--state", "open", "--limit", str(limit),
               "--json", "number,labels,changedFiles,isDraft,title"], runner=runner)
    out = []
    for data in json.loads(raw):
        out.append(_with_changed_files(repo, {
            "number": data.get("number"),
            "labels": [lb.get("name") for lb in data.get("labels") or []],
            "title": data.get("title") or "",
            "isDraft": bool(data.get("isDraft")),
        }, data.get("changedFiles"), runner=runner))
    return out


def ensure_area_label(repo, label, runner=None, out=None):
    """CREATE `label` in `repo`. The ONLY call in this module that adds permanent repo
    state beyond a PR's own labels, and the ONLY answer to issue #4582 that is not
    "warn and fall back".

    It is reached only for a name in `plan["create"]` — i.e. a crate whose root manifest
    THIS PR adds (`new_crate_area`), which has no label yet, and which the same run is
    about to apply. `POST /repos/{repo}/labels` is used deliberately instead of relying on
    the "add labels to an issue" endpoint's silent creation: an explicit create names the
    one label being made and records in its description why it exists, so a human auditing
    the repo's label list can tell it from a hand-made one.

    FAILS LOUDLY. A creation that cannot succeed (no `issues: write`, a rejected name)
    propagates out of `_gh` and REDS the run, because the alternative is exactly the
    latent `__global__` hazard #4582 describes. The ONE tolerated failure is the
    already-exists race between two concurrent runs adding the same crate, and that is
    RE-READ from the live label set rather than assumed from the error text.
    """
    out = out or sys.stdout
    try:
        _gh(["api", "--method", "POST", f"repos/{repo}/labels",
             "-f", f"name={label}", "-f", f"color={AREA_LABEL_COLOR}",
             "-f", f"description={AREA_LABEL_DESCRIPTION}"], runner=runner)
        outcome = "created"
    except RuntimeError:
        if label[len(AREA_PREFIX):] not in known_area_labels(repo, runner=runner):
            raise
        outcome = "already present (created concurrently)"
    print(f"::notice title=pr-area-label label created::{label} {outcome} — this PR adds "
          f"the crate's root {CRATE_MANIFEST}, so the area is witnessed, not guessed.",
          file=out)
    return outcome


def apply_areas(repo, number, labels, runner=None):
    """ADD labels to a PR. There is deliberately no removal counterpart in this module."""
    if not labels:
        return
    args = ["pr", "edit", str(number), "--repo", repo]
    for lb in labels:
        args += ["--add-label", lb]
    _gh(args, runner=runner)


# ----------------------------------------------------------------------------- CLI


def _emit(plan, pr, policy, known_areas, apply, out):
    n = plan["number"]
    if plan["reason"] != "resolved":
        blockers = unresolved_paths(pr, policy, known_areas)
        detail = ""
        if plan["reason"] == "unresolved":
            detail = "  unattributable: " + ", ".join(sorted(blockers)[:8])
            if len(blockers) > 8:
                detail += f" (+{len(blockers) - 8} more)"
        elif plan["reason"] == "cross-cutting":
            detail = f"  spans >{policy.max_areas} areas"
        elif plan["reason"] == "incomplete-paths":
            detail = ("  changed-file enumeration disagreed with `changedFiles` — the "
                      "list is partial, so any narrowing would be a proper SUBSET")
        print(f"#{n} KEEP {GLOBAL} [{plan['reason']}]{detail}", file=out)
        return 0
    if plan["noop"]:
        print(f"#{n} no-change  already {' '.join(plan['existing']) or '(none)'}", file=out)
        return 0
    verb = "LABEL" if apply else "would label"
    made = ""
    if plan["create"]:
        made = (f"  ({'CREATE' if apply else 'would create'} "
                f"{' '.join(plan['create'])} — new crate manifest added by this PR)")
    print(f"#{n} {verb} {' '.join(plan['add'])}{made}"
          f"   partition -> {' '.join(plan['partition'])}", file=out)
    return 1


def main(argv=None, runner=None, out=None):
    out = out or sys.stdout
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--repo", default=os.environ.get("SPARQ_REPO", "sparq-org/sparq"))
    ap.add_argument("--pr", type=int, help="derive for a single PR number")
    ap.add_argument("--backfill", action="store_true", help="derive for every OPEN PR")
    ap.add_argument("--apply", action="store_true",
                    help="actually add the labels (default: dry run, touch nothing)")
    ap.add_argument("--dry-run", action="store_true",
                    help="explicit no-op default; mutually exclusive with --apply")
    ap.add_argument("--limit", type=int, default=300, help="backfill PR page ceiling")
    ap.add_argument("--pace", type=float, default=0.0,
                    help="seconds to wait between label writes during a backfill. Adding a "
                         "label fires a `pull_request: labeled` event, and ci/bench/fuzz/"
                         "feature-matrix all trigger on it (as guarded no-op runs). A "
                         "one-pass backfill of ~85 PRs therefore queues hundreds of short "
                         "jobs at once; pacing keeps the runners from congesting, which this "
                         "repo has previously seen turn into FALSE gate failures.")
    ap.add_argument("--head-repo", default=None,
                    help="the PR head repo full_name. When it is not --repo the PR is a "
                         "FORK PR whose `pull_request` token is READ-ONLY: report and exit 0.")
    ap.add_argument("--self-test", action="store_true")
    args = ap.parse_args(argv)

    if args.self_test:
        return _self_test()
    if args.apply and args.dry_run:
        print("--apply and --dry-run are mutually exclusive", file=sys.stderr)
        return 2
    # NO-PULL-REQUEST-CONTEXT PATH. The workflow passes `--pr 0` on a merge_group ref
    # (the payload has no pull request). Decided HERE, in Python, rather than with a
    # workflow `if:` — an `if:` mutant survives, a Python one dies against a named test.
    if args.pr == 0 and not args.backfill:
        print("::notice title=pr-area-label skipped::no pull-request context "
              "(`--pr 0`); nothing to label on this ref.", file=out)
        return 0
    if bool(args.pr) == bool(args.backfill):
        print("exactly one of --pr N / --backfill is required", file=sys.stderr)
        return 2

    # FORK PATH, handled explicitly rather than as a confusing 403 (see docstring).
    # An EMPTY --head-repo is treated the same way: the payload did not tell us the head
    # repo, so we cannot establish that the token is writable — fail closed, stay global.
    if args.head_repo is not None and args.head_repo != args.repo:
        where = f"'{args.head_repo}'" if args.head_repo else "(not reported)"
        print(f"::notice title=pr-area-label skipped::PR head repo {where} is not "
              f"'{args.repo}'; a fork `pull_request` run holds a READ-ONLY token, so no "
              f"`area:` label can be applied. The PR stays on the fail-closed "
              f"{GLOBAL} partition; label it by hand or from a trusted context.", file=out)
        return 0

    policy = load_policy()
    known = known_area_labels(args.repo, runner=runner)
    if not known:
        print(f"::warning title=pr-area-label inert::no `{AREA_PREFIX}*` labels readable in "
              f"{args.repo}; nothing to apply (every PR stays {GLOBAL}).", file=out)
        return 0

    prs = ([fetch_pr(args.repo, args.pr, runner=runner)] if args.pr
           else fetch_open_prs(args.repo, limit=args.limit, runner=runner))
    changed = kept = 0
    for pr in prs:
        plan = plan_for_pr(pr, policy, known)
        changed += _emit(plan, pr, policy, known, args.apply, out)
        if plan["reason"] != "resolved":
            kept += 1
        if args.apply and plan["add"]:
            for label in plan["create"]:
                ensure_area_label(args.repo, label, runner=runner, out=out)
                # Keep the live set current so a BACKFILL over two PRs adding the same
                # crate creates the label once and applies it twice.
                known.add(label[len(AREA_PREFIX):])
            apply_areas(args.repo, plan["number"], plan["add"], runner=runner)
            if args.pace > 0:
                import time
                time.sleep(args.pace)
    mode = "APPLIED" if args.apply else "DRY RUN (nothing written)"
    print(f"-- {mode}: {len(prs)} PR(s); {changed} relabelled; "
          f"{kept} left on {GLOBAL} (fail-closed)", file=out)
    # Report, in the workflow log, which areas were dropped for want of a label — the
    # declared [[map]] areas AND every workspace member, since a crate with no
    # `area:` label silently sends every PR that touches it back to __global__.
    missing = sorted((policy.declared_areas() | policy.members) - known)
    if missing:
        print(f"::warning title=pr-area-label missing labels::declared in "
              f"ci/area-labels.toml but no such label exists: "
              f"{', '.join(AREA_PREFIX + m for m in missing)}", file=out)
    return 0


# ----------------------------------------------------------------------- self-test


def _self_test():
    """Hermetic assertions over the REAL ci/area-labels.toml. Full coverage (including
    the workflow-YAML seam and the fake-gh idempotence/no-delete proofs) lives in
    scripts/tests/test_pr_area_labels.py; this is the fast in-script smoke the
    orchestration lane runs alongside the other `--self-test` entry points."""
    policy = load_policy()
    known = policy.members | policy.non_crate

    def areas(paths):
        return derive_areas(paths, policy, known)

    # A crates/x-only PR narrows to exactly x — the reg#677 #4143 case.
    assert areas(["crates/sparq-reason/src/lib.rs"]) == (frozenset({"sparq-reason"}), "resolved")
    # ... and the skill that documents it joins the SAME partition.
    assert areas(["crates/sparq-reason/src/lib.rs", "skills/inference/SKILL.md"]) == (
        frozenset({"sparq-reason"}), "resolved")
    # An unattributable path fails CLOSED, and poisons the whole PR.
    assert areas(["no/such/dir/x.txt"]) == (frozenset(), "unresolved")
    assert areas(["crates/sparq-core/src/lib.rs", "no/such/dir/x.txt"]) == (
        frozenset(), "unresolved")
    # An unknown crate directory has no label, so it is unresolved; a file sitting
    # directly in crates/ is not a crate at all.
    assert areas(["crates/not-a-crate/src/lib.rs"]) == (frozenset(), "unresolved")
    assert attribute("crates/README.md", policy) is None
    # Genuinely workspace-spanning work stays global.
    wide = [f"crates/{c}/src/lib.rs" for c in sorted(policy.members)[:policy.max_areas + 1]]
    assert areas(wide) == (frozenset(), "cross-cutting")
    # ... but max_areas crates is still narrow.
    ok = [f"crates/{c}/src/lib.rs" for c in sorted(policy.members)[:policy.max_areas]]
    assert areas(ok)[1] == "resolved" and len(areas(ok)[0]) == policy.max_areas
    # No paths at all => global.
    assert areas([]) == (frozenset(), "no-paths")
    # A PARTIAL changed-file list => global. This is the truncation boundary: the same
    # paths derive a real (but SUBSET) answer when the list is known whole.
    partial = ["crates/sparq-core/src/lib.rs"]
    assert derive_areas(partial, policy, known, complete=False) == (
        frozenset(), "incomplete-paths")
    assert derive_areas(partial, policy, known, complete=True)[1] == "resolved"
    assert paths_are_complete(100, 646) is False
    assert paths_are_complete(646, 646) is True
    assert paths_are_complete(0, None) is False
    # An area with no live label is unresolved, never auto-created ON PATH SHAPE ALONE.
    assert derive_areas(["crates/sparq-core/src/lib.rs"], policy, set()) == (
        frozenset(), "unresolved")
    # ... but an ADDED crate root manifest WITNESSES the crate (#4582), so a PR creating
    # `crates/<new>/` resolves and the label is queued for creation.
    new = "sparq-brand-new"
    assert new not in policy.members
    assert new_crate_area(f"crates/{new}/{CRATE_MANIFEST}", "added") == new
    # ... and nothing weaker is a witness: a source file, a nested manifest, an EDIT to an
    # existing manifest, or a name that cannot be a label.
    assert new_crate_area(f"crates/{new}/src/lib.rs", "added") is None
    assert new_crate_area(f"crates/{new}/sub/{CRATE_MANIFEST}", "added") is None
    assert new_crate_area(f"crates/{new}/{CRATE_MANIFEST}", "modified") is None
    assert new_crate_area(f"crates/{CRATE_MANIFEST}", "added") is None
    assert new_crate_area(f"crates/bad name/{CRATE_MANIFEST}", "added") is None
    add_crate = {"number": 5, "labels": [], "complete": True,
                 "files": [f"crates/{new}/{CRATE_MANIFEST}", f"crates/{new}/src/lib.rs"],
                 "new_crates": [new]}
    plan_new = plan_for_pr(add_crate, policy, known)
    assert plan_new["reason"] == "resolved", plan_new
    assert plan_new["add"] == [AREA_PREFIX + new], plan_new
    assert plan_new["create"] == [AREA_PREFIX + new], plan_new
    # Without the witness the SAME paths stay fail-closed on `__global__`.
    no_witness = dict(add_crate, new_crates=[])
    assert plan_for_pr(no_witness, policy, known)["reason"] == "unresolved"
    # A witness for a crate that ALREADY has a label creates nothing.
    assert plan_for_pr({"number": 6, "labels": [], "complete": True,
                        "files": [f"crates/sparq-core/{CRATE_MANIFEST}"],
                        "new_crates": ["sparq-core"]}, policy, known)["create"] == []
    # A witness never rescues an otherwise-unattributable PR: `create` is a subset of
    # `add`, and a fail-closed plan adds nothing.
    poisoned = plan_for_pr(dict(add_crate, files=add_crate["files"] + ["no/such/x.txt"]),
                           policy, known)
    assert poisoned["reason"] == "unresolved" and poisoned["create"] == [], poisoned
    # `*` never crosses `/`: the root-markdown row must not swallow a skill or a crate doc.
    assert attribute("README.md", policy) == "docs"
    assert attribute("skills/inference/SKILL.md", policy) == "sparq-reason"
    assert attribute("crates/sparq-core/README.md", policy) == "sparq-core"
    # The shared-lane rows the frontier depends on.
    assert attribute("Cargo.lock", policy) == "deps"
    # sparq#5128: the root manifest must stay ATTRIBUTABLE. `derive_areas` is strict — one
    # unresolved path and the PR derives no labels at all — so were this row to stop resolving,
    # every PR touching the root manifest would declare nothing. (It is NOT a gate that a new crate
    # registers itself in `[workspace] members`; see `ready-issues.py::partition_path`.)
    assert attribute("Cargo.toml", policy) == "workspace"
    assert attribute(".github/workflows/ci.yml", policy) == "ci"
    assert attribute("scripts/pr-area-labels.py", policy) == "ci"
    # Additive only: derivation never proposes removing a human label.
    plan = plan_for_pr({"number": 1, "labels": ["area:sparq-core", "review:parked"],
                        "files": ["crates/sparq-engine/src/lib.rs"],
                        "complete": True}, policy, known)
    assert plan["add"] == ["area:sparq-engine"], plan
    assert plan["partition"] == ["sparq-core", "sparq-engine"], plan
    # Idempotent: nothing to add when the derived area is already there.
    assert plan_for_pr({"number": 2, "labels": ["area:sparq-engine"],
                        "files": ["crates/sparq-engine/src/lib.rs"], "complete": True},
                       policy, known)["noop"] is True
    # A fail-closed PR that already carries a HUMAN area keeps it (not global).
    keep = plan_for_pr({"number": 3, "labels": ["area:sparq-zk"],
                        "files": ["no/such/dir/x.txt"], "complete": True}, policy, known)
    assert keep["add"] == [] and keep["partition"] == ["sparq-zk"], keep
    # A record with NO completeness verdict fails CLOSED — a fetcher that never
    # established that its list is whole must not narrow anything.
    blind = plan_for_pr({"number": 4, "labels": [],
                         "files": ["crates/sparq-engine/src/lib.rs"]}, policy, known)
    assert blind["add"] == [] and blind["reason"] == "incomplete-paths", blind
    print("pr-area-labels self-test OK")
    return 0


if __name__ == "__main__":
    sys.exit(main())
