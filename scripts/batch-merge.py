#!/usr/bin/env python3
# Omnibus merge-queue overflow batcher (scheduled / event-driven, NON-gating).
#
# WHAT THIS IS
# ------------
# The deterministic policy core of .github/workflows/batch-merge.yml. The merge queue on
# `main` (ALLGREEN, max_entries_to_merge=8) drains individually-armed worker PRs one queue
# window at a time. When MORE than QUEUE_WINDOW reviewed worker PRs are armed at once, the
# overflow waits multiple queue cycles. This script batches that overflow into ONE
# integration ("omnibus") PR so a single queue slot lands many reviewed constituents.
# It makes NO model calls: pure, testable policy over a GitHub/git state snapshot.
#
# POLICY (maintainer-directed, 2026-07-17)
# ----------------------------------------
#  1. ELIGIBILITY. A constituent is an OPEN PR whose head branch matches ^sparq-agent/,
#     carrying label `review:pass` AND an ACTIVE auto-merge arm enabled by
#     app/sparq-orchestrator. NEVER touched: release-plz PRs, dependabot/* branches,
#     drafts, PRs labeled needs:user or trust:*, or any non-sparq-agent branch.
#  2. THRESHOLD + CLASS PARTITION (issue #3433 throughput program). eligible <=
#     QUEUE_WINDOW (8) -> no-op (the queue handles them alone). Otherwise sort by PR
#     number ASCENDING; the first QUEUE_WINDOW stay individually queued, the REST are
#     the overflow. The overflow is PARTITIONED by change-class: SLIM = every changed
#     path is docs-/orchestration-only per ci_select.classify_change (the SAME audited
#     allowlist behind the #3428 merge-group slimming), ENGINE = everything else (an
#     unknown/unfetchable diff is engine — fail-closed into the heavy class). Per
#     class: overflow < MIN_CONSTITUENTS (2) -> no batch for that class; at most
#     MAX_CONSTITUENTS (15) per omnibus (wall-time-tuned toward the 60-merges/hour
#     target; a v2 culprit bisect at 15 stays log2(15) ~= 4 runs) — the excess waits,
#     still individually armed, for the next run. Classification is SCHEDULING-ONLY:
#     the omnibus's own CI computes its change-class from its real diff (ci-select),
#     so a misclassification here can never skip a required check — a slim batch just
#     rides the slim merge-group lane set instead of waiting on a full-matrix batch.
#  3. INTEGRATION BRANCH. sparq-omnibus/<class>-<utcstamp> fresh off origin/main;
#     sequentially `git merge --no-ff` each overflow branch (ascending PR number). A
#     conflicting constituent is SKIPPED (it stays individually armed) and recorded in
#     the PR body. If fewer than MIN_CONSTITUENTS merge cleanly the branch is deleted
#     and no PR opens.
#  4. ONE PR PER CLASS. Title `omnibus(<class>): <n> reviewed worker PRs`; body =
#     SPARQ-agent self-ID + machine marker (class + constituent PR numbers — the class
#     in branch/title/marker is what makes per-class omnibus CI wall time measurable
#     straight from PR history) + constituent table (PR, issue, crate) + one
#     `Closes #<issue>` line per constituent target issue. The PR is armed with
#     `gh pr merge --auto` (strategy is chosen by the merge queue). The sparq-omnibus/
#     prefix keeps it OUT of the worker review loop: the registry's enumerator
#     (dispatch-claim.py) admits only heads matching ^sparq-agent/issue-<n>- and keys
#     state off review:* labels, so the omnibus carries NO review:* label ever.
#  5. CONSTITUENT CLOSURE. For every MERGED omnibus PR (found via the body marker), any
#     still-open constituent whose branch now adds NOTHING vs origin/main (merge-tree of
#     origin/main + branch == origin/main's tree) is closed with a comment linking the
#     omnibus. Its issue closes via the omnibus's `Closes #` refs. Idempotent: closed
#     constituents drop out of the open set; a constituent with post-omnibus commits is
#     NOT empty and is left alone.
#  6. FAILURE PATH (v1). An OPEN omnibus PR is closed (marker comment + branch deleted)
#     when ANY of: (a) its ci-summary `gate` check CONCLUDED in failure — live because
#     the omnibus is pushed/created with the sparq-orchestrator App token, so head
#     pull_request CI fires on it like on any worker PR; (b) it is CONFLICTING vs main;
#     (c) it is OLDER than MAX_OMNIBUS_AGE_HOURS (stamp parsed from its branch name) —
#     the liveness backstop for every head-invisible failure mode: a merge-group `gate`
#     failure (which reports on the queue's synthetic ref, not the PR head), a dropped
#     auto-merge arm, or checks that never reported. Constituents remain individually
#     armed — no worse than before. Bisection is deliberately v2: the close comment
#     carries the v2 marker and NOTHING else is filed (the orchestrator tracks v2).
#     A dying omnibus does NOT suppress a new batch in the same run (close-and-recreate),
#     so one bad batch can never wedge the batcher.
#  7. RE-ARM LIVENESS. GitHub DROPS the auto-merge arm when a merge group fails. Each
#     run, an open, young (under the age bound), MERGEABLE omnibus with NO active arm is
#     re-armed idempotently (`gh pr merge --auto`) — a transient queue failure gets
#     retried; a persistent one hits the age bound and is closed.
#  8. STALE HYGIENE. Any sparq-omnibus/* remote branch with no open PR is deleted.
#
# TOKENS. The omnibus branch/PR MUST be pushed + created with the sparq-orchestrator App
# installation token: GITHUB_TOKEN-created refs/PRs get their workflow events SUPPRESSED,
# so ci-summary would never run on the omnibus head, the required `gate` context would
# never report, and the merge queue would never ADMIT the PR (admission requires the
# required checks to pass on the head; merge-group evaluation happens only after
# admission) — the PR would wedge open forever (empirically: GITHUB_TOKEN-created PR
# #1084, zero check-runs, BLOCKED since 2026-06-21). When the App credentials are absent
# the workflow runs this script with --hygiene-only: closure/failure/re-arm/stale legs
# still run, but NO new omnibus is created.
#
# DESIGN: policy is a PURE FUNCTION plan(state) -> [Action]; an Action carries the exact
# `gh`/`git` argv it maps to. `live` mode gathers the state via real gh/git then executes
# the plan; `--self-test` runs plan() over fixtures with gh AND git STUBBED so no live
# mutation can happen from a test. Fixtures cover the DO-NOTHING cases explicitly and the
# asserts compare exact argv (flip any one expectation and the suite goes red).
#
# USAGE
#   scripts/batch-merge.py --repo owner/repo                 # live run (App token in GH_TOKEN)
#   scripts/batch-merge.py --repo owner/repo --dry-run       # print plan, no mutations
#   scripts/batch-merge.py --repo owner/repo --hygiene-only  # no new omnibus (no App token)
#   scripts/batch-merge.py --self-test                       # hermetic; gh + git stubbed
#
# Exit 0 on success; non-zero only on a real error or a failed self-test.
import argparse
import json
import os
import re
import subprocess
import sys
from dataclasses import dataclass
from datetime import datetime, timezone

PROG = "batch-merge"

# Change-class source (issue #3433): reuse ci_select.classify_change — the SAME audited
# orchestration-safe/docs-only allowlist that drives merge-group CI slimming (#3428) —
# so "slim to the batcher" and "slim to the merge-group lanes" can never drift apart.
# Fail-soft: if the import ever breaks, everything classifies engine (exactly the
# pre-#3433 single-batch behaviour). Grouping is scheduling-only — the omnibus's own
# CI recomputes the class from its real diff, so this can never skip a required check.
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
try:
    from ci_select import _INERT_CLASSES, classify_change
except Exception:  # defensive only; --self-test pins the working import path red/green
    _INERT_CLASSES = ()

    def classify_change(_paths):
        return "engine"

# [OPUS-5] #1135: the shared Release-PR predicate. NOT fail-soft — unlike
# classify_change above (whose worst case is a coarser batch), a missing Release-PR
# exclusion can put the Release PR into an omnibus batch. The stub refuses everything.
try:
    import release_pr_guard
except ImportError:  # pragma: no cover - see scripts/tests/test_release_publish_guard.py

    class release_pr_guard:  # type: ignore[no-redef]
        @staticmethod
        def arm_block_reason(**_kwargs) -> str:
            return (
                "release-pr-guard: scripts/release_pr_guard.py is not importable — "
                "treating every PR as un-batchable (fail-closed, #1135)"
            )

# The merge queue's max_entries_to_merge: up to this many armed PRs land in one queue
# window, so a backlog at or under it needs no batching.
QUEUE_WINDOW = 8
# An omnibus with a single constituent is pure overhead — require at least two.
MIN_CONSTITUENTS = 2
# Per-class cap on omnibus size (issue #3433): batch-15 per 15-minute run ~= the
# 60-merges/hour target from one queue slot per class, while a v2 culprit bisect at 15
# stays log2(15) ~= 4 runs. Excess overflow waits — still individually armed — for the
# next run rather than growing one unbounded (and unbisectable) batch.
MAX_CONSTITUENTS = 15
# Liveness bound: an omnibus that has not MERGED this many hours after creation has no
# route to merge (merge-group failure / dropped arm / checks never reported — all
# invisible on the PR head) and is closed so it cannot suppress batching indefinitely.
# The queue drains a window well inside this bound; the value trades retry opportunity
# (re-arm, §7) against worst-case queue churn from a persistently-failing omnibus.
MAX_OMNIBUS_AGE_HOURS = 4

OMNIBUS_PREFIX = "sparq-omnibus/"
# Omnibus change-classes (issue #3433). SLIM batches carry only constituents whose
# diffs are docs-/orchestration-only, so their merge-group run is the slim lane set
# (#3428) and never waits on a full Rust matrix; ENGINE carries everything else.
# One omnibus PER CLASS may be in flight; a legacy unclassed sparq-omnibus/<stamp>
# branch suppresses both classes (conservative) until it drains.
BATCH_CLASS_SLIM = "slim"
BATCH_CLASS_ENGINE = "engine"
BATCH_CLASSES = (BATCH_CLASS_SLIM, BATCH_CLASS_ENGINE)
WORKER_BRANCH_RE = re.compile(r"^sparq-agent/")
# Worker heads encode their target issue: sparq-agent/issue-<N>-<run_id>-<attempt>.
WORKER_ISSUE_RE = re.compile(r"^sparq-agent/issue-([1-9][0-9]*)-")
ORCHESTRATOR_APP = "sparq-orchestrator"
# Machine marker embedded in every omnibus PR body; closure keys on it. The class
# attribute is newer than v1 bodies, so it is optional here (legacy markers parse).
MARKER_RE = re.compile(
    r"<!--\s*sparq-omnibus:v1\s+(?:class=[a-z]+\s+)?constituents=([0-9,]+)\s*-->")

SELF_ID = "> 🤖 SPARQ agent"


def log(msg: str) -> None:
    print(f"[{PROG}] {msg}", file=sys.stderr)


def author_handle(login: str) -> str:
    # gh reports app authors either as "app/<name>" or bare — normalise.
    return (login or "").split("/")[-1].lower()


def is_release_plz(pr: dict) -> bool:
    """[OPUS-5] #1135: one shared predicate for every arming/merging path.

    WAS: `author == github-actions AND title startswith "chore: release"`. That
    conjunction misses the Release PR whenever EITHER signal shifts — a different bot
    identity, or a retitled PR. release_pr_guard.arm_block_reason keys on branch OR
    author OR title and fails CLOSED on an unknown head branch. Widening the exclusion
    is the safe direction here: the batcher's only response is to leave the PR alone.
    """
    return (
        release_pr_guard.arm_block_reason(
            head_ref=pr.get("head_ref"),
            author_login=pr.get("author_login"),
            title=pr.get("title"),
        )
        is not None
    )


def has_excluded_label(pr: dict) -> bool:
    for lbl in pr.get("labels", []):
        low = (lbl or "").lower()
        if low == "needs:user" or low.startswith("trust:"):
            return True
    return False


def armed_by_orchestrator(pr: dict) -> bool:
    """True iff the PR carries an ACTIVE auto-merge request enabled by the orchestrator App."""
    am = pr.get("auto_merge")
    if not isinstance(am, dict):
        return False
    return author_handle(str(am.get("enabled_by", ""))) == ORCHESTRATOR_APP


def issue_of(pr: dict):
    """The constituent's target issue number, from its worker head ref (None if unparseable)."""
    m = WORKER_ISSUE_RE.match(pr.get("head_ref", ""))
    return int(m.group(1)) if m else None


def crate_of(pr: dict) -> str:
    """Best-effort crate column: first changed crates/<name>/ path, else area:<x> label, else em-dash."""
    for path in pr.get("files", []):
        m = re.match(r"^crates/([^/]+)/", path or "")
        if m:
            return m.group(1)
    for lbl in sorted(pr.get("labels", [])):
        if lbl.lower().startswith("area:"):
            return lbl.split(":", 1)[1]
    return "—"


def constituent_class(pr: dict) -> str:
    """Which omnibus class a constituent belongs to (issue #3433).

    SLIM iff ci_select classifies its changed paths onto one of the audited-inert
    classes (`_INERT_CLASSES` — the SAME tuple the merge-group `changes` pre-jobs
    case on, imported rather than re-spelled so the batcher and the CI lanes can
    never disagree about what "inert" means); ENGINE otherwise — including `mixed`,
    `engine`, and an EMPTY/unfetched file list (fail-closed: an unknown diff rides
    the heavy class, which is exactly the pre-partition behaviour)."""
    files = [f for f in pr.get("files", []) if f]
    if not files:
        return BATCH_CLASS_ENGINE
    return (BATCH_CLASS_SLIM
            if classify_change(files) in _INERT_CLASSES
            else BATCH_CLASS_ENGINE)


def eligible_constituents(prs: list) -> list:
    """The armed, reviewed worker PRs the batcher may consider — sorted by number ASCENDING."""
    out = []
    for pr in prs:
        head = pr.get("head_ref", "")
        if not WORKER_BRANCH_RE.match(head):
            continue  # covers dependabot/*, release-plz, and every other branch family
        if pr.get("is_draft"):
            continue
        if is_release_plz(pr) or has_excluded_label(pr):
            continue
        if "review:pass" not in [(lbl or "").lower() for lbl in pr.get("labels", [])]:
            continue
        if not armed_by_orchestrator(pr):
            continue
        out.append(pr)
    return sorted(out, key=lambda p: p["number"])


def constituents_of_marker(body: str) -> list:
    """Constituent PR numbers recorded in an omnibus body marker ([] when absent)."""
    m = MARKER_RE.search(body or "")
    if not m:
        return []
    return [int(x) for x in m.group(1).split(",") if x]


def omnibus_class(head_ref: str):
    """The omnibus's batch class from its branch name (sparq-omnibus/<class>-<stamp>).

    None for a legacy unclassed branch (sparq-omnibus/<stamp>) — the caller treats
    None as suppressing EVERY class (conservative: we cannot tell what it carries)."""
    rest = head_ref[len(OMNIBUS_PREFIX):] if head_ref.startswith(OMNIBUS_PREFIX) else ""
    cls = rest.split("-", 1)[0]
    return cls if cls in BATCH_CLASSES else None


def omnibus_age_hours(head_ref: str, now: datetime):
    """Hours since the omnibus branch's creation stamp — sparq-omnibus/<class>-<utcstamp>
    (or the legacy classless sparq-omnibus/<utcstamp>).

    None on an unparseable stamp — the caller treats that as EXPIRED (an omnibus branch
    we cannot date must never become immortal)."""
    rest = head_ref[len(OMNIBUS_PREFIX):] if head_ref.startswith(OMNIBUS_PREFIX) else ""
    # Stamps never contain "-", so anything before the first dash is a class prefix.
    stamp = rest.split("-", 1)[1] if "-" in rest else rest
    try:
        created = datetime.strptime(stamp, "%Y%m%dT%H%M%SZ").replace(tzinfo=timezone.utc)
    except ValueError:
        return None
    return (now - created).total_seconds() / 3600.0


def gate_concluded_failure(pr: dict) -> bool:
    """True iff the authoritative ci-summary `gate` check has CONCLUDED in failure.

    Live for an omnibus because its branch/PR are pushed + created with the App token,
    so head pull_request CI (ci-summary -> `gate`) fires on it. A merge-group failure
    reports on the queue's synthetic ref instead — the age bound covers that mode.
    A missing / in-progress / queued gate is NOT a failure (never act on a non-terminal
    gate — same posture as pr-backlog.py)."""
    for c in pr.get("checks", []):
        if (c.get("name") or "").strip().lower() != "gate":
            continue
        if (c.get("status") or "").lower() == "completed" and (
            c.get("conclusion") or ""
        ).lower() in ("failure", "timed_out", "cancelled"):
            return True
    return False


# --------------------------------------------------------------------------------------
# Actions: each carries the exact argv the live runner executes. `tool` is "gh" or "git".
# --------------------------------------------------------------------------------------
@dataclass
class Action:
    kind: str
    tool: str
    argv: list
    note: str = ""
    # A merge step may fail on conflict; the runner then aborts the merge + records a skip.
    may_conflict: bool = False
    constituent: int = 0
    # The omnibus branch this action builds ("" = a hygiene action). With one batch
    # per class in a single plan, the runner keys survivors/skips/abort per group.
    group: str = ""


def omnibus_branch(now: datetime, batch_class: str) -> str:
    return f"{OMNIBUS_PREFIX}{batch_class}-{now.strftime('%Y%m%dT%H%M%SZ')}"


def omnibus_title(count: int, batch_class: str) -> str:
    return f"omnibus({batch_class}): {count} reviewed worker PRs"


def omnibus_body(constituents: list, skipped: list, batch_class: str) -> str:
    """The omnibus PR body: self-ID, machine marker, constituent table, Closes lines."""
    nums = ",".join(str(p["number"]) for p in constituents)
    lines = [
        f"{SELF_ID} — omnibus batcher (`scripts/batch-merge.py`). This PR batches the "
        f"reviewed, orchestrator-armed worker PRs beyond the merge queue's window into one "
        f"queue entry (change-class: **{batch_class}** — slim batches carry only "
        f"docs-/orchestration-only diffs so their CI never waits on the full matrix). "
        f"Every constituent already carries `review:pass`; its issue closes "
        f"below. A conflicting constituent was skipped and stays individually armed.",
        "",
        f"<!-- sparq-omnibus:v1 class={batch_class} constituents={nums} -->",
        "",
        "| PR | Issue | Crate |",
        "|---|---|---|",
    ]
    for p in constituents:
        issue = issue_of(p)
        issue_cell = f"#{issue}" if issue else "—"
        lines.append(f"| #{p['number']} | {issue_cell} | {crate_of(p)} |")
    if skipped:
        lines += ["", "Skipped (merge conflict — still individually armed): "
                  + ", ".join(f"#{n}" for n in skipped)]
    lines.append("")
    for p in constituents:
        issue = issue_of(p)
        if issue:
            lines.append(f"Closes #{issue}")
    lines += ["", "Constituent PRs: " + ", ".join(f"#{p['number']}" for p in constituents)]
    return "\n".join(lines)


def close_constituent_comment(omnibus_num: int) -> str:
    return (
        f"{SELF_ID} — this PR's changes landed on `main` via omnibus PR #{omnibus_num} "
        f"(its branch now adds nothing vs `main`). Closing; the target issue closes via "
        f"the omnibus's `Closes #` references."
    )


def close_failed_omnibus_comment(reason: str) -> str:
    return (
        f"{SELF_ID} — closing this omnibus: {reason}. Every constituent PR remains "
        f"individually armed, so nothing is lost — the queue drains them as before.\n\n"
        f"<!-- sparq-omnibus-failure:v1 bisection=v2 -->"
    )


# --------------------------------------------------------------------------------------
# The pure policy. `state` is a plain-dict snapshot:
#   repo: "owner/name"
#   now:  aware UTC datetime
#   open_prs: [{number, head_ref, title, author_login, labels[], is_draft,
#               auto_merge{enabled_by}|None, files[], body, checks[], mergeable}]
#   merged_omnibus: [{number, body}]                    (merged sparq-omnibus/* PRs)
#   empty_vs_main: {pr_number: bool}                    (branch adds nothing vs main)
#   remote_omnibus_branches: ["sparq-omnibus/..."]      (live remote refs)
#   batch_enabled: bool                                 (False = --hygiene-only: never
#                                                        create a new omnibus)
# --------------------------------------------------------------------------------------
def plan(state: dict) -> list:
    repo = state["repo"]
    actions: list = []
    open_prs = state.get("open_prs", [])
    open_by_num = {p["number"]: p for p in open_prs}

    # ---- 5. constituent closure for MERGED omnibus PRs (first: frees the backlog) ----
    for om in state.get("merged_omnibus", []):
        for num in constituents_of_marker(om.get("body", "")):
            pr = open_by_num.get(num)
            if pr is None:
                continue  # already closed/merged — idempotent
            if not state.get("empty_vs_main", {}).get(num, False):
                continue  # has content main lacks (e.g. post-omnibus commits) — leave armed
            actions.append(Action(
                "close-constituent", "gh",
                ["pr", "close", str(num), "--repo", repo,
                 "--comment", close_constituent_comment(om["number"])],
                note=f"constituent #{num} contained in merged omnibus #{om['number']}"))

    # ---- 6+7. failure path v1 + re-arm liveness for OPEN omnibus PRs ----
    open_omnibus = [p for p in open_prs if p.get("head_ref", "").startswith(OMNIBUS_PREFIX)]
    dying_branches = set()
    for om in open_omnibus:
        age = omnibus_age_hours(om.get("head_ref", ""), state["now"])
        if gate_concluded_failure(om):
            reason = "its `gate` check concluded in failure"
        elif (om.get("mergeable") or "").upper() == "CONFLICTING":
            reason = "it conflicts with `main`"
        elif age is None or age > MAX_OMNIBUS_AGE_HOURS:
            # Liveness backstop: covers every head-invisible failure (merge-group gate
            # failure, dropped arm, checks never reported, unparseable stamp).
            reason = (f"it did not merge within {MAX_OMNIBUS_AGE_HOURS}h of creation "
                      f"(no route to merge — e.g. a merge-group failure, a dropped "
                      f"auto-merge arm, or checks that never reported)")
        else:
            # §7 re-arm: GitHub drops the auto-merge arm when a merge group fails; a
            # young, mergeable, UNARMED omnibus gets its arm restored idempotently so a
            # transient queue failure is retried instead of wedging until the age bound.
            if (om.get("mergeable") or "").upper() == "MERGEABLE" and not om.get("auto_merge"):
                actions.append(Action(
                    "arm", "gh",
                    ["pr", "merge", str(om["number"]), "--repo", repo, "--auto"],
                    note=f"re-arm open omnibus #{om['number']} (arm dropped)"))
            continue
        dying_branches.add(om["head_ref"])
        actions.append(Action(
            "close-omnibus", "gh",
            ["pr", "close", str(om["number"]), "--repo", repo,
             "--comment", close_failed_omnibus_comment(reason)],
            note=f"omnibus #{om['number']}: {reason}"))
        actions.append(Action(
            "delete-branch", "git", ["push", "origin", "--delete", om["head_ref"]],
            note=f"branch of closed omnibus #{om['number']}"))

    # ---- 8. stale hygiene: sparq-omnibus/* branches with no open PR ----
    live_branches = {p["head_ref"] for p in open_omnibus}
    for br in sorted(state.get("remote_omnibus_branches", [])):
        if br in live_branches or br in dying_branches:
            continue
        actions.append(Action("delete-branch", "git", ["push", "origin", "--delete", br],
                              note="stale omnibus branch (no open PR)"))

    # ---- 1-4. overflow batching ----
    if not state.get("batch_enabled", True):
        # --hygiene-only (no App token in GH_TOKEN): a GITHUB_TOKEN-created omnibus can
        # NEVER enter the merge queue (events suppressed -> required `gate` never
        # reports -> no queue admission), so creating one would only wedge. The
        # closure / failure / re-arm / stale legs above still ran.
        log("hygiene-only mode — skipping omnibus creation (no App token)")
        return actions
    eligible = eligible_constituents(open_prs)
    if len(eligible) <= QUEUE_WINDOW:
        log(f"eligible armed worker PRs: {len(eligible)} <= {QUEUE_WINDOW} — queue handles them (no batch)")
        return actions
    overflow = eligible[QUEUE_WINDOW:]  # ascending: the first WINDOW stay individually queued
    # Partition by change-class (issue #3433): a slim (docs/orchestration-only) batch
    # rides the slim merge-group lane set (#3428) and never waits on an engine batch's
    # full-matrix run. Order within each class stays ascending.
    by_class = {cls: [] for cls in BATCH_CLASSES}
    for p in overflow:
        by_class[constituent_class(p)].append(p)
    # One omnibus PER CLASS in flight: never stack a second of the same class while one
    # is open (the closed-above dying ones no longer count — close-and-recreate). A
    # legacy unclassed sparq-omnibus/<stamp> suppresses every class (unknown contents).
    alive = [p for p in open_omnibus if p["head_ref"] not in dying_branches]
    for cls in BATCH_CLASSES:
        batch = by_class[cls][:MAX_CONSTITUENTS]
        if len(batch) < MIN_CONSTITUENTS:
            if by_class[cls]:
                log(f"{cls} overflow of {len(by_class[cls])} < {MIN_CONSTITUENTS} — "
                    f"not worth an omnibus (stays individually armed)")
            continue
        blockers = [p["number"] for p in alive
                    if omnibus_class(p["head_ref"]) in (cls, None)]
        if blockers:
            log(f"omnibus PR(s) {blockers} still open — not opening another {cls} batch")
            continue
        if len(by_class[cls]) > MAX_CONSTITUENTS:
            log(f"{cls} overflow of {len(by_class[cls])} capped at {MAX_CONSTITUENTS} — "
                f"the excess stays individually armed for the next run")
        branch = omnibus_branch(state["now"], cls)
        actions.append(Action("fetch", "git",
                              ["fetch", "origin", "main"] + [p["head_ref"] for p in batch],
                              group=branch))
        actions.append(Action("create-branch", "git",
                              ["checkout", "-B", branch, "origin/main"], note=branch,
                              group=branch))
        for p in batch:
            actions.append(Action(
                "merge", "git",
                ["merge", "--no-ff", "--no-edit", "-m",
                 f"omnibus: merge PR #{p['number']} ({p['head_ref']})",
                 f"origin/{p['head_ref']}"],
                may_conflict=True, constituent=p["number"],
                note=f"constituent #{p['number']}", group=branch))
        # The runner resolves the surviving constituent set (conflicts skipped) and then
        # executes these tail actions with the body/count templated over the survivors.
        actions.append(Action("push-branch", "git", ["push", "origin", branch],
                              note=branch, group=branch))
        actions.append(Action(
            "open-pr", "gh",
            ["pr", "create", "--repo", repo, "--base", "main", "--head", branch,
             "--title", omnibus_title(len(batch), cls),
             "--body", omnibus_body(batch, [], cls)],
            note=f"{len(batch)} constituents ({cls})", group=branch))
        # Arm: strategy is chosen by the merge queue, so no method flag.
        actions.append(Action("arm", "gh",
                              ["pr", "merge", branch, "--repo", repo, "--auto"],
                              note=branch, group=branch))
    return actions


# --------------------------------------------------------------------------------------
# Live runners. All mutation flows through run_gh/run_git so --self-test can stub both.
# --------------------------------------------------------------------------------------
def run_gh(argv: list, capture: bool = True) -> str:
    res = subprocess.run(["gh"] + argv, check=True,
                         capture_output=capture, text=True)
    return res.stdout if capture else ""


def run_git(argv: list, check: bool = True) -> int:
    return subprocess.run(["git"] + argv, check=check).returncode


def gather_state(repo: str, now: datetime, batch_enabled: bool = True) -> dict:
    """Snapshot the live GitHub/git state the pure plan() consumes.

    Deliberately TWO-PHASE: one LIGHT list over all open PRs (a single query carrying
    body+statusCheckRollup for 200 PRs overloads the GraphQL stream), then targeted
    per-PR views only where the policy needs the heavy fields (open omnibus PRs and the
    overflow candidates)."""
    fields = "number,headRefName,title,author,labels,isDraft,autoMergeRequest"
    raw = json.loads(run_gh(["pr", "list", "--repo", repo, "--state", "open",
                             "--limit", "200", "--json", fields]))
    open_prs = []
    for pr in raw:
        am = pr.get("autoMergeRequest")
        open_prs.append({
            "number": pr["number"],
            "head_ref": pr.get("headRefName", ""),
            "title": pr.get("title", ""),
            "author_login": (pr.get("author") or {}).get("login", ""),
            "labels": [lbl["name"] for lbl in pr.get("labels", [])],
            "is_draft": bool(pr.get("isDraft")),
            "auto_merge": ({"enabled_by": ((am.get("enabledBy") or {}).get("login", ""))}
                           if isinstance(am, dict) else None),
            "files": [],  # filled below only for overflow candidates (keeps API calls low)
            "body": "",
            "mergeable": "",   # filled below for omnibus PRs only
            "checks": [],      # filled below for omnibus PRs only
        })
    for p in open_prs:
        if not p["head_ref"].startswith(OMNIBUS_PREFIX):
            continue
        try:
            heavy = json.loads(run_gh(["pr", "view", str(p["number"]), "--repo", repo,
                                       "--json", "body,mergeable,statusCheckRollup"]))
        except subprocess.CalledProcessError:
            continue  # unknown never acts: leave mergeable/checks empty for this run
        p["body"] = heavy.get("body", "")
        p["mergeable"] = heavy.get("mergeable", "")
        p["checks"] = [{"name": c.get("name", ""), "status": c.get("status", ""),
                        "conclusion": c.get("conclusion", "")}
                       for c in (heavy.get("statusCheckRollup") or []) if isinstance(c, dict)]
    eligible = eligible_constituents(open_prs)
    for p in eligible[QUEUE_WINDOW:]:
        try:
            files = json.loads(run_gh(["pr", "view", str(p["number"]), "--repo", repo,
                                       "--json", "files"]))
            p["files"] = [f.get("path", "") for f in files.get("files", [])]
        except subprocess.CalledProcessError:
            p["files"] = []  # crate column degrades to label/em-dash — non-fatal

    merged = json.loads(run_gh(["pr", "list", "--repo", repo, "--state", "merged",
                                "--limit", "50", "--search", "head:sparq-omnibus/",
                                "--json", "number,headRefName,body"]))
    merged_omnibus = [{"number": m["number"], "body": m.get("body", "")}
                      for m in merged
                      if m.get("headRefName", "").startswith(OMNIBUS_PREFIX)]

    # Emptiness test for closure: merging the constituent branch into main yields main's
    # own tree <=> the branch adds nothing. (A plain two/three-dot diff is wrong here:
    # after the omnibus SQUASHES onto main the constituent's merge-base is stale, so its
    # three-dot diff stays non-empty forever even though main contains every change.)
    run_git(["fetch", "origin", "main"])
    empty_vs_main: dict = {}
    open_by_num = {p["number"]: p for p in open_prs}
    for om in merged_omnibus:
        for num in constituents_of_marker(om["body"]):
            pr = open_by_num.get(num)
            if pr is None or num in empty_vs_main:
                continue
            branch = pr["head_ref"]
            if run_git(["fetch", "origin", branch], check=False) != 0:
                empty_vs_main[num] = False  # unfetchable head: unknown never closes
                continue
            try:
                merged_tree = subprocess.run(
                    ["git", "merge-tree", "--write-tree", "origin/main",
                     f"origin/{branch}"],
                    capture_output=True, text=True).stdout.strip().splitlines()
                main_tree = subprocess.run(
                    ["git", "rev-parse", "origin/main^{tree}"],
                    check=True, capture_output=True, text=True).stdout.strip()
                empty_vs_main[num] = bool(merged_tree) and merged_tree[0] == main_tree
            except (subprocess.CalledProcessError, IndexError):
                empty_vs_main[num] = False

    ls = subprocess.run(["git", "ls-remote", "origin", f"refs/heads/{OMNIBUS_PREFIX}*"],
                        check=True, capture_output=True, text=True).stdout
    remote_omnibus_branches = [line.split("refs/heads/", 1)[1]
                               for line in ls.strip().splitlines() if "refs/heads/" in line]

    return {"repo": repo, "now": now, "open_prs": open_prs,
            "merged_omnibus": merged_omnibus, "empty_vs_main": empty_vs_main,
            "remote_omnibus_branches": remote_omnibus_branches,
            "batch_enabled": batch_enabled}


def execute(actions: list, state: dict, dry_run: bool) -> int:
    """Run the plan. The merge sequence is stateful: conflicting constituents are skipped
    and the tail (push / open-pr / arm) is re-templated over the survivors. All per-batch
    state is keyed by the Action's `group` (the omnibus branch) — a single plan may build
    one omnibus per change-class, and one class's abort must never leak into the other."""
    survivors: dict = {}   # group -> [constituent numbers that merged cleanly]
    skipped: dict = {}     # group -> [constituent numbers skipped on conflict]
    aborted: set = set()   # groups whose branch was deleted (too few clean merges)
    batch_groups = {a.group for a in actions if a.kind == "merge"}
    open_by_num = {p["number"]: p for p in state.get("open_prs", [])}
    for act in actions:
        if act.group in aborted and act.kind in ("merge", "push-branch", "open-pr", "arm"):
            continue
        argv = list(act.argv)
        if act.kind == "push-branch" and act.group in batch_groups and not dry_run \
                and len(survivors.get(act.group, [])) < MIN_CONSTITUENTS:
            # Too few clean merges: delete the local branch, never push/open/arm.
            log(f"only {len(survivors.get(act.group, []))} constituent(s) merged cleanly "
                f"— aborting omnibus {act.group}")
            run_git(["checkout", "--detach", "origin/main"])
            run_git(["branch", "-D", act.group], check=False)
            aborted.add(act.group)
            continue
        if act.kind == "open-pr" and act.group in batch_groups:
            live = [open_by_num[n] for n in sorted(survivors.get(act.group, []))]
            cls = omnibus_class(act.group) or BATCH_CLASS_ENGINE
            argv = ["pr", "create", "--repo", state["repo"], "--base", "main",
                    "--head", act.group,
                    "--title", omnibus_title(len(live), cls),
                    "--body", omnibus_body(live, skipped.get(act.group, []), cls)]
        log(("DRY-RUN " if dry_run else "") + f"{act.kind}: {act.tool} "
            + " ".join(argv[:6]) + (" …" if len(argv) > 6 else "")
            + (f"  [{act.note}]" if act.note else ""))
        if dry_run:
            if act.kind == "merge":
                survivors.setdefault(act.group, []).append(act.constituent)
            continue
        if act.tool == "git":
            if act.may_conflict:
                if run_git(argv, check=False) == 0:
                    survivors.setdefault(act.group, []).append(act.constituent)
                else:
                    run_git(["merge", "--abort"], check=False)
                    skipped.setdefault(act.group, []).append(act.constituent)
                    log(f"constituent #{act.constituent} conflicts — skipped (stays armed)")
            else:
                run_git(argv)
        else:
            try:
                run_gh(argv)
            except subprocess.CalledProcessError as e:
                if act.kind == "arm":
                    # Arm failures are loud but non-fatal: the omnibus PR exists and the
                    # NEXT run's §7 re-arm leg restores the arm idempotently (a
                    # persistently unarmed omnibus is closed at the §6 age bound).
                    log(f"WARNING: arming failed ({e}); omnibus left open UNARMED")
                else:
                    raise
    return 0


# --------------------------------------------------------------------------------------
# Self-test: hermetic fixtures, gh + git stubbed (plan() is pure — nothing to stub away —
# and the asserts pin exact argv so any policy drift flips the suite red).
# --------------------------------------------------------------------------------------
def _pr(num, head, labels=(), armed_by=ORCHESTRATOR_APP, draft=False, author="worker[bot]",
        title="feat: x", files=(), body="", checks=(), mergeable="MERGEABLE"):
    return {"number": num, "head_ref": head, "title": title, "author_login": author,
            "labels": list(labels), "is_draft": draft,
            "auto_merge": ({"enabled_by": f"app/{armed_by}"} if armed_by else None),
            "files": list(files), "body": body, "checks": list(checks),
            "mergeable": mergeable}


def _worker(num, issue, labels=("review:pass",), **kw):
    return _pr(num, f"sparq-agent/issue-{issue}-123-1", labels=labels, **kw)


def _state(open_prs=(), merged=(), empty=None, branches=(), batch_enabled=True):
    return {"repo": "sparq-org/sparq",
            "now": datetime(2026, 7, 17, 12, 0, 0, tzinfo=timezone.utc),
            "open_prs": list(open_prs), "merged_omnibus": list(merged),
            "empty_vs_main": dict(empty or {}),
            "remote_omnibus_branches": list(branches),
            "batch_enabled": batch_enabled}


def self_test() -> int:
    failures = []

    def check(name, got, want):
        if got != want:
            failures.append(f"{name}:\n  got : {got!r}\n  want: {want!r}")

    stamp_branch = "sparq-omnibus/20260717T120000Z"  # legacy classless branch shape
    engine_branch = "sparq-omnibus/engine-20260717T120000Z"
    slim_branch = "sparq-omnibus/slim-20260717T120000Z"

    # 0. PARSERS: class + stamp round-trip for both branch shapes; marker with and
    #    without the class attribute (legacy bodies must keep closing).
    check("class parsed from classed branch", omnibus_class(engine_branch), "engine")
    check("legacy branch has no class", omnibus_class(stamp_branch), None)
    check("unknown class token is not a class",
          omnibus_class("sparq-omnibus/mystery-20260717T120000Z"), None)
    now = datetime(2026, 7, 17, 12, 0, 0, tzinfo=timezone.utc)
    check("age parsed through the class prefix",
          omnibus_age_hours("sparq-omnibus/slim-20260717T060000Z", now), 6.0)
    check("age parsed on a legacy branch",
          omnibus_age_hours("sparq-omnibus/20260717T060000Z", now), 6.0)
    check("legacy marker (no class) still parses",
          constituents_of_marker("<!-- sparq-omnibus:v1 constituents=1,2 -->"), [1, 2])
    check("classed marker parses",
          constituents_of_marker("<!-- sparq-omnibus:v1 class=slim constituents=3,4 -->"),
          [3, 4])

    # 0b. CLASSIFICATION (via the real ci_select.classify_change import — a broken
    #     import degrades everything to engine and flips these red):
    check("docs-only diff classifies slim",
          constituent_class(_worker(1, 1, files=("docs/a.md", "research/b.md"))), "slim")
    check("orchestration-only diff classifies slim",
          constituent_class(_worker(1, 1, files=("orchestration/routing.toml",
                                                 "scripts/batch-merge.py"))), "slim")
    # [OPUS-5] sq-g25hr: the two classes added with the deploy fast lane.
    check("deploy-only diff classifies slim",
          constituent_class(_worker(1, 1, files=("deploy/aws/sparq-server.yaml",
                                                 ".github/workflows/deploy-lint.yml"))),
          "slim")
    check("inert-mixed diff classifies slim",
          constituent_class(_worker(1, 1, files=("deploy/gcp/README.md",
                                                 "docs/branch-protection.md"))), "slim")
    check("crate diff classifies engine",
          constituent_class(_worker(1, 1, files=("crates/sparq-core/src/lib.rs",))),
          "engine")
    check("mixed diff classifies engine (fail-closed)",
          constituent_class(_worker(1, 1, files=("docs/a.md", "crates/x/src/lib.rs"))),
          "engine")
    check("unknown (empty) diff classifies engine (fail-closed)",
          constituent_class(_worker(1, 1)), "engine")

    # 1. DO-NOTHING: empty repo state -> zero actions.
    check("empty state is a no-op", plan(_state()), [])

    # 2. DO-NOTHING: exactly QUEUE_WINDOW armed worker PRs -> the queue handles them.
    at_window = [_worker(100 + i, 900 + i) for i in range(QUEUE_WINDOW)]
    check("at-window is a no-op", plan(_state(open_prs=at_window)), [])

    # 3. DO-NOTHING: 9 candidates but the 9th is inert (draft / unarmed / wrong label /
    #    excluded label / foreign branch / release-plz) -> still <= window, no batch.
    for bad in (
        _worker(109, 909, draft=True),
        _worker(109, 909, armed_by=None),
        _worker(109, 909, labels=("review:changes",)),
        _worker(109, 909, labels=("review:pass", "needs:user")),
        _worker(109, 909, labels=("review:pass", "trust:pending")),
        _pr(109, "dependabot/cargo/serde-1.0", labels=("review:pass",)),
        _pr(109, "release-plz-2026", labels=("review:pass",),
            author="app/github-actions", title="chore: release v0.2"),
        _worker(109, 909, armed_by="someother-app"),
    ):
        check(f"ineligible 9th ({bad['head_ref']}/{bad['labels']}/draft={bad['is_draft']}"
              f"/arm={bad['auto_merge']}) is a no-op",
              plan(_state(open_prs=at_window + [bad])), [])

    # 4. OVERFLOW of exactly 1 (9 eligible) -> below MIN_CONSTITUENTS, no batch.
    nine = at_window + [_worker(109, 909)]
    check("overflow of one is a no-op", plan(_state(open_prs=nine)), [])

    # 5. OVERFLOW of 2 (10 eligible) -> full batch plan; the LOWEST 8 numbers stay
    #    individually queued and #109/#110 overflow, ascending. Both constituents are
    #    engine-class (empty diff fail-closed / crate path) -> ONE engine omnibus.
    ten = nine + [_worker(110, 910, files=("crates/sparq-core/src/lib.rs",))]
    got = plan(_state(open_prs=ten))
    kinds = [a.kind for a in got]
    check("overflow-of-two plan shape", kinds,
          ["fetch", "create-branch", "merge", "merge", "push-branch", "open-pr", "arm"])
    check("branch off origin/main carries the class", got[1].argv,
          ["checkout", "-B", engine_branch, "origin/main"])
    check("first merge is lowest overflow number", got[2].argv,
          ["merge", "--no-ff", "--no-edit", "-m",
           "omnibus: merge PR #109 (sparq-agent/issue-909-123-1)",
           "origin/sparq-agent/issue-909-123-1"])
    check("merges flagged conflict-skippable", [a.may_conflict for a in got[2:4]],
          [True, True])
    check("batch actions all carry the group", {a.group for a in got}, {engine_branch})
    check("push argv", got[4].argv, ["push", "origin", engine_branch])
    check("PR title counts constituents + class",
          got[5].argv[got[5].argv.index("--title") + 1],
          "omnibus(engine): 2 reviewed worker PRs")
    body = got[5].argv[got[5].argv.index("--body") + 1]
    check("body carries the classed machine marker",
          "<!-- sparq-omnibus:v1 class=engine constituents=109,110 -->" in body, True)
    check("body self-IDs as SPARQ agent", body.startswith(SELF_ID), True)
    check("body closes both issues",
          ("Closes #909" in body, "Closes #910" in body), (True, True))
    check("body crate column from changed files", "| #110 | #910 | sparq-core |" in body, True)
    check("arm has no merge-method flag (queue chooses)", got[6].argv,
          ["pr", "merge", engine_branch, "--repo", "sparq-org/sparq", "--auto"])
    check("no review:* label is ever added to the omnibus",
          any("label" in " ".join(a.argv) for a in got if a.tool == "gh"), False)

    # 5b. CLASS PARTITION: docs/orchestration-only overflow batches SEPARATELY from
    #     engine overflow — one omnibus per class in the same plan, slim first, each
    #     ascending within its class.
    slim_over = [_worker(111, 911, files=("docs/notes.md",)),
                 _worker(113, 913, files=("orchestration/routing.toml",))]
    got = plan(_state(open_prs=ten + slim_over))
    check("two-class plan shape", [a.kind for a in got],
          ["fetch", "create-branch", "merge", "merge", "push-branch", "open-pr", "arm",
           "fetch", "create-branch", "merge", "merge", "push-branch", "open-pr", "arm"])
    check("slim batch first, engine second",
          [a.argv[2] for a in got if a.kind == "create-branch"],
          [slim_branch, engine_branch])
    check("slim batch carries the slim constituents",
          [a.constituent for a in got[:7] if a.kind == "merge"], [111, 113])
    check("engine batch carries the engine constituents",
          [a.constituent for a in got[7:] if a.kind == "merge"], [109, 110])
    slim_title = got[5].argv[got[5].argv.index("--title") + 1]
    check("slim PR title carries the class", slim_title,
          "omnibus(slim): 2 reviewed worker PRs")
    slim_body = got[5].argv[got[5].argv.index("--body") + 1]
    check("slim marker carries the class",
          "<!-- sparq-omnibus:v1 class=slim constituents=111,113 -->" in slim_body, True)

    # 5c. BATCH CAP: overflow beyond MAX_CONSTITUENTS in one class is left for the
    #     next run (still individually armed) — the batch never exceeds the cap.
    many = at_window + [_worker(200 + i, 800 + i) for i in range(MAX_CONSTITUENTS + 2)]
    got = plan(_state(open_prs=many))
    check("batch capped at MAX_CONSTITUENTS",
          len([a for a in got if a.kind == "merge"]), MAX_CONSTITUENTS)
    check("capped batch keeps the LOWEST overflow numbers",
          [a.constituent for a in got if a.kind == "merge"],
          [200 + i for i in range(MAX_CONSTITUENTS)])
    check("capped title counts the capped batch",
          got[-2].argv[got[-2].argv.index("--title") + 1],
          f"omnibus(engine): {MAX_CONSTITUENTS} reviewed worker PRs")

    # 6. An OPEN (fresh, armed, mergeable) omnibus suppresses another batch OF ITS
    #    CLASS only; a LEGACY classless omnibus suppresses every class.
    open_om = _pr(200, stamp_branch, author="app/github-actions")  # legacy, armed
    check("legacy open omnibus suppresses every class",
          plan(_state(open_prs=ten + slim_over + [open_om])), [])
    open_engine = _pr(201, engine_branch, author="app/github-actions")
    got = plan(_state(open_prs=ten + slim_over + [open_engine]))
    check("open engine omnibus suppresses engine only — slim still batches",
          [a.argv[2] for a in got if a.kind == "create-branch"], [slim_branch])
    open_slim = _pr(202, slim_branch, author="app/github-actions")
    got = plan(_state(open_prs=ten + slim_over + [open_slim]))
    check("open slim omnibus suppresses slim only — engine still batches",
          [a.argv[2] for a in got if a.kind == "create-branch"], [engine_branch])

    # 6b. --hygiene-only NEVER creates an omnibus (no App token: a GITHUB_TOKEN omnibus
    #     cannot enter the merge queue), but the closure legs still run.
    check("hygiene-only skips batching", plan(_state(open_prs=ten, batch_enabled=False)), [])
    hygiene_merged = {"number": 310, "body": "<!-- sparq-omnibus:v1 constituents=101 -->"}
    got = plan(_state(open_prs=[_worker(101, 901)], merged=[hygiene_merged],
                      empty={101: True}, batch_enabled=False))
    check("hygiene-only still closes constituents", [a.kind for a in got],
          ["close-constituent"])

    # 7. CLOSURE: merged omnibus + open constituent with empty diff -> close w/ comment;
    #    non-empty constituent untouched.
    merged_om = {"number": 300, "body": "x\n<!-- sparq-omnibus:v1 constituents=101,102 -->\ny"}
    st = _state(open_prs=[_worker(101, 901), _worker(102, 902)],
                merged=[merged_om], empty={101: True, 102: False})
    got = plan(st)
    check("closure closes only the empty constituent", [(a.kind, a.argv[:3]) for a in got],
          [("close-constituent", ["pr", "close", "101"])])
    check("closure comment links the omnibus",
          "#300" in got[0].argv[got[0].argv.index("--comment") + 1], True)
    check("closure is idempotent (constituent already closed)",
          plan(_state(merged=[merged_om], empty={101: True})), [])

    # 8. FAILURE PATH: open FRESH omnibus with a CONCLUDED gate failure -> close + delete
    #    branch; an in-progress gate (armed) is left alone. (Head-gate checks exist on an
    #    omnibus because it is pushed/created with the App token.)
    failed_om = _pr(400, "sparq-omnibus/20260717T110000Z", author="app/github-actions",
                    armed_by=None,
                    checks=[{"name": "gate", "status": "COMPLETED", "conclusion": "FAILURE"}])
    got = plan(_state(open_prs=[failed_om],
                      branches=["sparq-omnibus/20260717T110000Z"]))
    check("failed omnibus close+delete", [(a.kind, a.tool) for a in got],
          [("close-omnibus", "gh"), ("delete-branch", "git")])
    check("failure comment carries the v2 marker",
          "<!-- sparq-omnibus-failure:v1 bisection=v2 -->"
          in got[0].argv[got[0].argv.index("--comment") + 1], True)
    check("branch delete argv", got[1].argv,
          ["push", "origin", "--delete", "sparq-omnibus/20260717T110000Z"])
    running_om = _pr(401, "sparq-omnibus/20260717T113000Z", author="app/github-actions",
                     checks=[{"name": "gate", "status": "IN_PROGRESS", "conclusion": ""}])
    check("in-progress gate omnibus untouched",
          plan(_state(open_prs=[running_om],
                      branches=["sparq-omnibus/20260717T113000Z"])), [])
    conflict_om = _pr(402, "sparq-omnibus/20260717T114000Z", author="app/github-actions",
                      armed_by=None, mergeable="CONFLICTING")
    got = plan(_state(open_prs=[conflict_om]))
    check("conflicting omnibus closed", [a.kind for a in got],
          ["close-omnibus", "delete-branch"])
    unknown_om = _pr(403, "sparq-omnibus/20260717T114500Z", author="app/github-actions",
                     mergeable="UNKNOWN")
    check("mergeable UNKNOWN (fresh, armed) never acts",
          plan(_state(open_prs=[unknown_om])), [])

    # 8b. AGE BOUND (liveness backstop for head-invisible failures: merge-group gate
    #     failure / dropped arm / checks never reported): an armed, MERGEABLE omnibus
    #     with NO checks at all, older than MAX_OMNIBUS_AGE_HOURS -> close + delete;
    #     a young one (same shape) is untouched; an unparseable stamp = expired.
    aged_om = _pr(404, "sparq-omnibus/20260717T060000Z", author="app/github-actions")
    got = plan(_state(open_prs=[aged_om]))
    check("over-age omnibus closed+deleted", [a.kind for a in got],
          ["close-omnibus", "delete-branch"])
    check("age-close reason names the bound",
          f"did not merge within {MAX_OMNIBUS_AGE_HOURS}h"
          in got[0].argv[got[0].argv.index("--comment") + 1], True)
    young_om = _pr(405, "sparq-omnibus/20260717T090000Z", author="app/github-actions")
    check("young armed mergeable omnibus untouched", plan(_state(open_prs=[young_om])), [])
    bad_stamp_om = _pr(406, "sparq-omnibus/not-a-stamp", author="app/github-actions")
    check("unparseable stamp treated as expired",
          [a.kind for a in plan(_state(open_prs=[bad_stamp_om]))],
          ["close-omnibus", "delete-branch"])

    # 8c. RE-ARM: a young, MERGEABLE omnibus whose auto-merge arm was DROPPED (merge
    #     groups drop the arm on failure) is re-armed idempotently — and still
    #     suppresses a second batch.
    dropped_om = _pr(407, "sparq-omnibus/20260717T090000Z", author="app/github-actions",
                     armed_by=None)
    got = plan(_state(open_prs=[dropped_om]))
    check("dropped arm re-armed", [(a.kind, a.argv) for a in got],
          [("arm", ["pr", "merge", "407", "--repo", "sparq-org/sparq", "--auto"])])
    got = plan(_state(open_prs=ten + [dropped_om]))
    check("re-armed omnibus still suppresses a new batch", [a.kind for a in got], ["arm"])

    # 8d. CLOSE-AND-RECREATE: a dying (over-age) omnibus does NOT suppress a new batch —
    #     the same plan closes it AND opens the fresh one (bounded suppression).
    got = plan(_state(open_prs=ten + [aged_om]))
    check("dying omnibus lifts suppression (close + fresh batch in one plan)",
          [a.kind for a in got],
          ["close-omnibus", "delete-branch", "fetch", "create-branch",
           "merge", "merge", "push-branch", "open-pr", "arm"])

    # 9. STALE HYGIENE: remote omnibus branch w/o an open PR is deleted; a branch with an
    #    open PR survives.
    got = plan(_state(open_prs=[open_om],
                      branches=[stamp_branch, "sparq-omnibus/20260701T000000Z"]))
    check("stale branch deleted, live branch kept", [(a.kind, a.argv) for a in got],
          [("delete-branch", ["push", "origin", "--delete", "sparq-omnibus/20260701T000000Z"])])

    # 10. EXECUTE with STUBBED gh+git: conflict on one constituent re-templates the tail;
    #     the arm argv never gains a method flag. (No subprocess escapes the stubs.)
    eleven = ten + [_worker(111, 911)]
    st = _state(open_prs=eleven)
    acts = plan(st)
    calls = []

    conflict_target = "origin/sparq-agent/issue-910-123-1"

    def stub_git(argv, check=True):
        calls.append(("git", list(argv)))
        return 1 if (argv[:1] == ["merge"] and conflict_target in argv) else 0

    def stub_gh(argv, capture=True):
        calls.append(("gh", list(argv)))
        return ""

    global run_git, run_gh
    real_git, real_gh = run_git, run_gh
    run_git, run_gh = stub_git, stub_gh
    try:
        execute(acts, st, dry_run=False)
    finally:
        run_git, run_gh = real_git, real_gh
    merges = [c for c in calls if c[0] == "git" and c[1][:1] == ["merge"] and "--abort" not in c[1]]
    check("three merges attempted", len(merges), 3)
    check("conflict aborted", ("git", ["merge", "--abort"]) in calls, True)
    opened = [c for c in calls if c[0] == "gh" and c[1][:2] == ["pr", "create"]]
    check("one PR opened after conflict", len(opened), 1)
    tbody = opened[0][1][opened[0][1].index("--body") + 1]
    check("re-templated marker drops the conflicted constituent",
          "<!-- sparq-omnibus:v1 class=engine constituents=109,111 -->" in tbody, True)
    check("re-templated body records the skip",
          "Skipped (merge conflict — still individually armed): #110" in tbody, True)
    check("re-templated title", opened[0][1][opened[0][1].index("--title") + 1],
          "omnibus(engine): 2 reviewed worker PRs")
    armed = [c for c in calls if c[0] == "gh" and c[1][:2] == ["pr", "merge"]]
    check("armed exactly once, --auto, no method", armed,
          [("gh", ["pr", "merge", engine_branch, "--repo", "sparq-org/sparq", "--auto"])])

    # 11. EXECUTE abort: 2 overflow, BOTH conflict -> branch deleted, nothing pushed/opened.
    acts = plan(_state(open_prs=ten))
    calls.clear()

    def stub_git_all_conflict(argv, check=True):
        calls.append(("git", list(argv)))
        return 1 if argv[:1] == ["merge"] and "--abort" not in argv else 0

    run_git, run_gh = stub_git_all_conflict, stub_gh
    try:
        execute(acts, _state(open_prs=ten), dry_run=False)
    finally:
        run_git, run_gh = real_git, real_gh
    check("all-conflict: no push, no PR, no arm",
          [c for c in calls if c[0] == "gh"
           or c[1][:1] == ["push"]], [])
    check("all-conflict: local branch deleted",
          ("git", ["branch", "-D", engine_branch]) in calls, True)

    # 12. EXECUTE abort isolation: in a two-class plan, the SLIM batch all-conflicting
    #     aborts ONLY the slim group — the engine omnibus still pushes, opens and arms.
    two_class_state = _state(open_prs=ten + slim_over)
    acts = plan(two_class_state)
    calls.clear()

    def stub_git_slim_conflict(argv, check=True):
        calls.append(("git", list(argv)))
        if argv[:1] == ["merge"] and "--abort" not in argv and any(
                ("issue-911-" in a or "issue-913-" in a) for a in argv):
            return 1
        return 0

    run_git, run_gh = stub_git_slim_conflict, stub_gh
    try:
        execute(acts, two_class_state, dry_run=False)
    finally:
        run_git, run_gh = real_git, real_gh
    check("slim all-conflict: slim branch deleted",
          ("git", ["branch", "-D", slim_branch]) in calls, True)
    check("slim all-conflict: engine branch still pushed",
          ("git", ["push", "origin", engine_branch]) in calls, True)
    opened = [c for c in calls if c[0] == "gh" and c[1][:2] == ["pr", "create"]]
    check("slim all-conflict: exactly one (engine) PR opened", len(opened), 1)
    check("surviving PR is the engine omnibus",
          opened[0][1][opened[0][1].index("--title") + 1],
          "omnibus(engine): 2 reviewed worker PRs")
    armed = [c for c in calls if c[0] == "gh" and c[1][:2] == ["pr", "merge"]]
    check("slim all-conflict: only the engine omnibus armed", armed,
          [("gh", ["pr", "merge", engine_branch, "--repo", "sparq-org/sparq", "--auto"])])

    if failures:
        for f in failures:
            print(f"[self-test] FAIL {f}", file=sys.stderr)
        print(f"[self-test] {len(failures)} failure(s)", file=sys.stderr)
        return 1
    print("[self-test] all checks passed (gh + git stubbed; no live calls)")
    return 0


def main() -> int:
    ap = argparse.ArgumentParser(prog=PROG, description=__doc__)
    ap.add_argument("--repo", help="owner/name")
    ap.add_argument("--dry-run", action="store_true", help="print the plan, mutate nothing")
    ap.add_argument("--hygiene-only", action="store_true",
                    help="closure/failure/re-arm/stale legs only; never create a new "
                         "omnibus (used when no App token is available — a GITHUB_TOKEN "
                         "omnibus cannot enter the merge queue)")
    ap.add_argument("--self-test", action="store_true", help="hermetic fixtures; gh+git stubbed")
    args = ap.parse_args()
    if args.self_test:
        return self_test()
    if not args.repo:
        ap.error("--repo is required outside --self-test")
    now = datetime.now(timezone.utc)
    state = gather_state(args.repo, now, batch_enabled=not args.hygiene_only)
    actions = plan(state)
    if not actions:
        log("nothing to do")
        return 0
    return execute(actions, state, args.dry_run)


if __name__ == "__main__":
    sys.exit(main())
