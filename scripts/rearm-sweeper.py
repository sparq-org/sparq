#!/usr/bin/env python3
"""Re-arm reviewed pull requests whose merge-queue arm was dropped by GitHub."""

# [GPT-5.6] Issue #3675 — restore dropped arms without mistaking queued PRs for drops.
# [FABLE-5] Issue #3759 / registry#563 item 4 — idempotent gh READS (pr list, the
# GraphQL live-state QUERY) go through gh_retry.run_gh_read (bounded, transient-only
# retry); the arm MUTATION (gh pr merge --auto) stays one-shot on the un-wrapped
# runner. Exhausted transient retries end the sweep as ::warning + exit 0 (a missed
# cycle is harmless — the cron covers it) instead of redding main's gate; any
# non-transient error still fails loudly.
# [OPUS-5] Issue #3760 — fail LOUDLY ONCE when the token cannot arm at all, and never let
#   a single per-PR arm failure abort the rest of the sweep. A capability denial is NOT a
#   transient: it never routes through the #3759 ::warning + exit-0 path.
# [OPUS-5] Issue #3766 — STICKY FAILURE PRECEDENCE. Collecting the failures (above) was
#   only half the job: the collected state lived in a LOCAL of run(), so a
#   GhTransientExhausted raised while processing a LATER candidate unwound past the
#   accumulated-failure check and main()'s lenient handler reported ::warning + exit 0.
#   Reproduced: PR #3011's arm failed, PR #3012's live-state read exhausted its retries,
#   and the run exited 0 — discarding #3011's real failure. Now the outcome lives on the
#   SWEEPER (RearmSweeper.outcome), a later candidate's exhaustion is recorded as its OWN
#   warning-class per-candidate outcome without escaping the loop, and the exit status is
#   computed at the END by sweep_exit() from that final state.
#   PRECEDENCE: collected-failure > transient-exhaustion > clean.

from __future__ import annotations

import argparse
import ast
import copy
import json
import re
import subprocess
import sys
import time
from dataclasses import dataclass, field
from datetime import datetime, timezone
from pathlib import Path
from typing import Callable


# --------------------------------------------------------------------------------------
# [OPUS-5] Issue #3776 — A MISSING RESILIENCE HELPER MUST NEVER BRICK AN ARM SWEEP.
#
# THE OUTAGE was in the SIBLING script: auto-arm.yml runs on `pull_request`, so its WORKFLOW
# FILE (and therefore its file-by-file `sparse-checkout` manifest) comes from the PR's own
# ref while the SCRIPT comes from `ref: default_branch`. #3766 added `import gh_retry` and
# the matching manifest entry in one commit — atomic on main, not across refs — so every
# stale PR ref got the new script without gh_retry.py and died with
# `ModuleNotFoundError: No module named 'gh_retry'` on a GATING check (sparq #3434, run
# 30143852994). See scripts/auto-arm.py for the full account.
#
# THIS script is NOT exposed to that ref skew today: rearm-sweeper.yml triggers only on
# `schedule` / `workflow_dispatch`, so its workflow file and its `ref: default_branch`
# checkout are always the same commit, and it is explicitly NOT A GATE. The guard is here
# for the two ways that changes: adding any per-ref trigger to rearm-sweeper.yml, or adding
# a second sibling import without remembering the manifest. Same class, same remedy — a
# missing resilience helper must degrade, not abort.
#
# THE DEGRADATION, EXPLICITLY. gh_retry is a RESILIENCE helper (bounded, transient-only
# retry for idempotent READS). Losing it must cost RETRIES — never the arm:
#   * reads become ONE-SHOT via _DegradedGhRetry.run_gh_read. The load-bearing work is
#     unchanged: candidates are still enumerated, every skip rule is still evaluated, a
#     draft is still marked ready, and the arm mutation is still issued.
#   * FAIL-CLOSED classification. With no retries there is nothing to exhaust, so a read
#     failure raises GhFatalError (loud red). It is NEVER reported as GhTransientExhausted:
#     that type is precisely what #3759 converts into ::warning + exit 0, so synthesising
#     it here would turn a 403 into a false success. Degraded mode therefore trades
#     transient tolerance for LOUDNESS, which is the safe direction — and strictly better
#     than the guaranteed red it replaces.
#   * assert_read_only stays a REAL fail-closed guard, not a no-op, so the self-test
#     assertion "every read this script issues IS a read" cannot go vacuous when degraded.
#   * exactly ONE loud ::warning at import time names the cause and the remedy.
# Deliberately DUPLICATED into the sibling arm script — each workflow sparse-checks out only
# its own script, so neither may import a shared helper (that very constraint is what caused
# this outage). scripts/tests/test_arm_capability_wiring.py pins the two copies together and
# proves the degraded path still mutates.
class _DegradedGhRetry:
    """One-shot stand-in for scripts/gh_retry.py when that file was not checked out."""

    # Mirrors gh_retry._READ_SUBCOMMANDS.
    READ_SUBCOMMANDS = frozenset(
        {
            ("pr", "list"),
            ("pr", "view"),
            ("pr", "checks"),
            ("pr", "status"),
            ("issue", "list"),
            ("issue", "view"),
            ("run", "list"),
            ("run", "view"),
            ("label", "list"),
            ("search", "prs"),
            ("search", "issues"),
        }
    )
    MUTATION_RE = re.compile(r"\b(?:mutation|subscription)\b", re.IGNORECASE)
    QUERY_FIELD_FLAGS = ("-f", "--field", "-F", "--raw-field")
    FIELD_FLAGS = ("-f", "-F", "--field", "--raw-field", "--input")

    class GhRetryUsageError(ValueError):
        """argv could not be PROVEN a read. Refused, never wrapped."""

    class GhFatalError(RuntimeError):
        """A non-retried ``gh`` failure — the caller must fail loudly."""

    class GhTransientExhausted(RuntimeError):
        """Never raised by this stand-in: with zero retries there is nothing to exhaust.

        Kept so ``except gh_retry.GhTransientExhausted`` clauses stay valid (and so the
        self-test can still raise it through an injected runner).
        """

    @classmethod
    def _field_values(cls, rest: list[str], flags: tuple[str, ...]) -> list[str]:
        values: list[str] = []
        index = 0
        while index < len(rest):
            arg = rest[index]
            if arg in flags:
                values.append(rest[index + 1] if index + 1 < len(rest) else "")
                index += 2
                continue
            for flag in flags:
                if arg.startswith(flag + "="):
                    values.append(arg.split("=", 1)[1])
                    break
            index += 1
        return values

    @classmethod
    def assert_read_only(cls, argv) -> None:
        """Fail-closed: refuse anything not AFFIRMATIVELY provable as a read."""
        rest = [str(arg) for arg in argv]
        if not rest:
            raise cls.GhRetryUsageError("empty gh argv")
        if rest[0] != "api":
            if tuple(rest[:2]) in cls.READ_SUBCOMMANDS:
                return
            raise cls.GhRetryUsageError(
                f"gh {' '.join(rest[:2])} is not an allow-listed read — "
                "mutations/arm calls stay one-shot"
            )
        tail = rest[1:]
        method = None
        for index, arg in enumerate(tail):
            if arg in ("-X", "--method"):
                method = tail[index + 1].upper() if index + 1 < len(tail) else ""
                break
            if arg.startswith("--method="):
                method = arg.split("=", 1)[1].upper()
                break
        if "graphql" in tail:
            inline = None
            for value in cls._field_values(tail, cls.QUERY_FIELD_FLAGS):
                if value.startswith("query=") and not value.startswith("query=@"):
                    inline = value.split("=", 1)[1]
            if inline is None:
                raise cls.GhRetryUsageError(
                    "refusing gh api graphql with no inline `query=` text (file-backed / "
                    "stdin bodies are opaque) — cannot prove it is a read"
                )
            if cls.MUTATION_RE.search(inline):
                raise cls.GhRetryUsageError(
                    "refusing a GraphQL mutation/subscription — arms stay one-shot"
                )
            return
        if method not in (None, "GET", "HEAD"):
            raise cls.GhRetryUsageError(
                f"refusing gh api with method {method or '<missing>'}"
            )
        if method is None and any(
            arg in cls.FIELD_FLAGS
            or any(arg.startswith(flag + "=") for flag in cls.FIELD_FLAGS[2:])
            for arg in tail
        ):
            raise cls.GhRetryUsageError(
                "refusing gh api with field params and no explicit GET "
                "(gh auto-switches to POST)"
            )

    @classmethod
    def run_gh_read(cls, argv, *, run=subprocess.run) -> str:
        """Do the read ONCE. No retry, and no transient classification (see above).

        ``run`` is injectable so the wiring test can prove a failing read raises
        GhFatalError and NEVER the lenient GhTransientExhausted.
        """
        cls.assert_read_only(argv)
        command = ["gh", *[str(arg) for arg in argv]]
        result = run(command, capture_output=True, text=True, check=False)
        if result.returncode != 0:
            detail = (
                (result.stderr or "").strip()
                or (result.stdout or "").strip()
                or "unknown gh failure"
            )
            raise cls.GhFatalError(
                f"{' '.join(command[:4])} failed (degraded: scripts/gh_retry.py was not "
                f"checked out, so this read was NOT retried): {detail}"
            )
        return result.stdout


try:
    import gh_retry

    GH_RETRY_DEGRADED = False
except ImportError as _gh_retry_missing:  # pragma: no cover - see the wiring test
    gh_retry = _DegradedGhRetry  # type: ignore[assignment]
    GH_RETRY_DEGRADED = True
    print(
        "::warning title=rearm-sweeper running WITHOUT transient-retry (scripts/gh_retry.py "
        f"not checked out)::{_gh_retry_missing} — this run's idempotent gh READS are "
        "ONE-SHOT: a GitHub 5xx blip will red it instead of being retried. Arming itself "
        "is UNAFFECTED and proceeds. CAUSE: this script came from the default branch "
        "while the `sparse-checkout` manifest came from an older workflow snapshot on the "
        "PR ref, which does not list scripts/gh_retry.py (sparq #3776 / #3766). REMEDY: "
        "rebase the PR onto the default branch, or re-run once the PR ref carries a "
        "workflow file that sparse-checks out the whole scripts/ directory."
    )


# --------------------------------------------------------------------------------------
# [OPUS-5] Issue #1135 — RELEASE-PR EXCLUSION. THE GAP THIS CLOSES: this sweep's ONLY
# exclusions were LABEL-keyed (`EXCLUDED_LABELS` + the `needs:*` namespace). Nothing was
# keyed on the head branch or the author, so the release-plz **Release PR** carrying
# `review:pass` and no hold label was re-armed like any other PR. Merging it cuts a `v*`
# tag and (once `publish = true` in release-plz.toml) `cargo publish`es 37 crates to
# crates.io — a version that can never be unpublished.
#
# A label-keyed exclusion is the wrong key: anything with `pull-requests: write` can add
# or remove a label. scripts/release_pr_guard.py keys on BRANCH / AUTHOR / TITLE instead.
#
# UNLIKE the gh_retry degradation above, a MISSING guard module must NOT degrade to
# "proceed": the stub below refuses EVERY re-arm.
try:
    import release_pr_guard

    RELEASE_GUARD_DEGRADED = False
except ImportError as _release_guard_missing:  # pragma: no cover - see the self-test

    class _FailClosedReleaseGuard:
        """Refuse EVERY re-arm when the release-PR guard could not be imported."""

        _REASON = (
            "release-pr-guard: scripts/release_pr_guard.py is NOT importable, so this "
            "sweep cannot prove any PR is not the release-plz Release PR — refusing "
            "every re-arm (fail-closed, #1135). REMEDY: add "
            "scripts/release_pr_guard.py to this workflow's sparse-checkout manifest."
        )

        @staticmethod
        def arm_block_reason(**_kwargs) -> str:
            return _FailClosedReleaseGuard._REASON

    release_pr_guard = _FailClosedReleaseGuard  # type: ignore[assignment]
    RELEASE_GUARD_DEGRADED = True
    print(
        "::error title=rearm-sweeper refusing every re-arm (scripts/release_pr_guard.py "
        f"not checked out)::{_release_guard_missing} — the release-PR exclusion (#1135) "
        "cannot be evaluated, so NOTHING is re-armed this cycle. Add "
        "scripts/release_pr_guard.py to the sparse-checkout manifest."
    )


PROGRAM = "rearm-sweeper"
REVIEW_ATTESTATION = "review:pass"
EXCLUDED_LABELS = frozenset(
    {
        "review:changes",
        "review:needs",
        "review:needs-user",
        "trust:untrusted",
    }
)
DEFAULT_MAX_REARMS = 10
# [OPUS-5] #1135: headRefName/author/title are the release-PR guard's ONLY inputs. Dropping
# one makes parse_* yield None for it and arm_block_reason fail CLOSED — the guard can
# degrade to refusing, never to admitting.
PR_LIST_FIELDS = "number,state,isDraft,baseRefName,headRefName,labels,author,title"
LIVE_QUERY = """query($owner:String!,$name:String!,$number:Int!){
  repository(owner:$owner,name:$name){
    pullRequest(number:$number){
      number
      state
      isDraft
      baseRefName
      headRefName
      title
      author{login}
      labels(first:100){nodes{name} pageInfo{hasNextPage}}
      autoMergeRequest{enabledAt}
      mergeQueueEntry{id}
    }
  }
}"""

# --------------------------------------------------------------------------------------
# [OPUS-5] #3760 — ARM CAPABILITY.
#
# LIVE ROOT CAUSE. Sweeper run 30033315483 (2026-07-23T18:21Z) failed arming PR #3454
# (OPEN, non-draft, CLEAN, review:pass) with
#     GraphQL: Resource not accessible by integration (enablePullRequestAutoMerge)
# while auto-arm run 30027010495 (2026-07-23T16:52Z, ~90 minutes earlier, SAME repo, SAME
# plain GITHUB_TOKEN, no App token) armed two PRs successfully. The ONLY difference was the
# job `permissions:` block: auto-arm.yml declared `contents: write`, rearm-sweeper.yml
# declared `contents: read`. Enabling auto-merge is a repository WRITE operation, so
# `contents: read` is denied with exactly this error. It is NOT a maintainer-only setting:
# `allow_auto_merge` is already true on the repository and the `main` ruleset carries no
# integration restriction.
#
# PROBE DESIGN — what is and is NOT a usable capability signal (all measured):
#   * Repository.autoMergeAllowed IS usable: it is the repository "Allow auto-merge"
#     setting, readable read-only, and false means no token can ever arm here.
#   * Repository.viewerPermission is NOT usable: GitHub documents it as "will return null
#     if authenticated as a GitHub App", and the Actions GITHUB_TOKEN *is* an App
#     installation token. Kept in the query as diagnostics only — never gate on it.
#   * PullRequest.viewerCanEnableAutoMerge is NOT a token-capability signal: measured
#     false, under an ADMIN user token, for already-armed (#2521), queued (#3764), draft
#     and merged PRs. It conflates PR state with permission, so gating on it would refuse
#     legitimate arms.
#   * A mutation against a bogus node id is NOT a probe: an authorized token gets
#     NOT_FOUND ("Could not resolve to a node ..."), so the denial class cannot be
#     distinguished without actually being denied.
# Hence: the startup probe fails loud on the signals it can read decisively, stays
# INCONCLUSIVE (never a false red) otherwise, and the first real denial from the mutation
# itself is promoted to a capability verdict — one ::error, sweep stopped, exit 1 once.
CAPABILITY_QUERY = """query($owner:String!,$name:String!){
  repository(owner:$owner,name:$name){
    autoMergeAllowed
    viewerPermission
  }
  viewer{login}
}"""

# Substrings that mark a token-capability denial rather than a per-PR condition. Matched
# case-insensitively against the whole gh/GraphQL failure text. Deliberately disjoint from
# the #3759 transient set: a denial is permanent and must never be swallowed as a transient.
ARM_DENIAL_MARKERS = (
    "resource not accessible by integration",
    "must have write access",
    "must have admin access",
    "not authorized to enable auto-merge",
    "auto merge is not allowed for this repository",
    "auto-merge is not allowed for this repository",
)

# One remediation string, naming every exact thing a human can flip, in fix order.
ARM_REMEDIATION = (
    "the arm token cannot call enablePullRequestAutoMerge. Fix in this order: "
    "(1) .github/workflows/rearm-sweeper.yml MUST declare `permissions:` with BOTH "
    "`contents: write` and `pull-requests: write` — enabling auto-merge is a repository "
    "WRITE operation and `contents: read` is denied with exactly this error (live proof: "
    "sweeper run 30033315483 failed with contents: read while auto-arm run 30027010495 "
    "armed successfully with contents: write on the same repo and the same GITHUB_TOKEN "
    "90 minutes earlier); "
    "(2) repository Settings -> General -> Pull Requests -> 'Allow auto-merge' must be ON "
    "(GraphQL Repository.autoMergeAllowed must be true); "
    "(3) if the repository's `main` ruleset gains a restriction on which integrations may "
    "write, github-actions[bot] must be allowed or the App path used instead; "
    "(4) to arm as the sparq-orchestrator App instead of GITHUB_TOKEN, set the "
    "repository/organization secrets ORCHESTRATOR_APP_ID and ORCHESTRATOR_APP_PRIVATE_KEY "
    "(both are currently UNSET, so the mint step is skipped and the job falls back to "
    "GITHUB_TOKEN)."
)

CAN_ARM = "can-arm"
CANNOT_ARM = "cannot-arm"
INCONCLUSIVE = "inconclusive"


def is_arm_denial(text: str) -> bool:
    """True when a failure text is a token-capability denial, not a per-PR condition."""
    lowered = str(text).lower()
    return any(marker in lowered for marker in ARM_DENIAL_MARKERS)


@dataclass(frozen=True)
class CapabilityVerdict:
    status: str
    detail: str

    @property
    def blocks_sweep(self) -> bool:
        """Only a decisive cannot-arm blocks; inconclusive must never red the run."""
        return self.status == CANNOT_ARM


@dataclass
class SweepOutcome:
    candidates: int = 0
    attempts: int = 0
    armed: int = 0
    # (number, reason) for every PR whose arm was attempted and failed.
    arm_failures: list[tuple[int, str]] = field(default_factory=list)
    # Live-state query failures: not arm failures, but still surfaced and still red
    # (this preserves the pre-#3760 `errors` contract for the diagnostic read).
    state_failures: list[tuple[int, str]] = field(default_factory=list)
    capability: str = INCONCLUSIVE
    # [OPUS-5] #3766 — warning-class per-candidate outcomes: (number, detail) for every
    # candidate whose own bounded read retries were exhausted. These NEVER downgrade a
    # collected failure; they only matter when they are the run's ONLY problem.
    transient_exhaustions: list[tuple[int, str]] = field(default_factory=list)
    # Whole-sweep transient exhaustion (the enumeration read, or a backstop catch).
    sweep_transient: str | None = None

    @property
    def capability_failed(self) -> bool:
        return self.capability == CANNOT_ARM

    @property
    def hard_failed(self) -> bool:
        """A COLLECTED failure: the verdict that outranks every later transient (#3766)."""
        return bool(self.capability_failed or self.arm_failures or self.state_failures)

    @property
    def transient_detail(self) -> str | None:
        """The first transient exhaustion, or None. Warning-class — never a verdict."""
        if self.sweep_transient:
            return self.sweep_transient
        if self.transient_exhaustions:
            number, reason = self.transient_exhaustions[0]
            return f"PR #{number}: {reason}"
        return None

    @property
    def exit_code(self) -> int:
        """Never exit 0 while an arm failed — a silent 0 is how #3760 hid for days.

        Deliberately independent of ``transient_exhaustions`` — a transient's exit is
        MODE-dependent (#3759) and is applied by :func:`sweep_exit`, whereas a collected
        failure is non-zero in every mode.
        """
        return 1 if self.hard_failed else 0


class GhError(RuntimeError):
    """A GitHub CLI command failed."""


@dataclass(frozen=True)
class PullRequest:
    number: int
    state: str
    is_draft: bool
    base_ref: str
    labels: frozenset[str]
    has_auto_merge: bool = False
    has_queue_entry: bool = False
    labels_truncated: bool = False
    # [OPUS-5] #1135 release-PR guard inputs. `None` means NOT REPORTED and is DISTINCT
    # from "" — arm_block_reason fails closed on an unknown head branch, so a missing
    # field must never be flattened into a string.
    head_ref: str | None = None
    author_login: str | None = None
    title: str | None = None


def _optional_str(raw: dict, key: str) -> str | None:
    """raw[key] as a string, or None when the key is absent/null (see PullRequest)."""
    if key not in raw:
        return None
    value = raw[key]
    return None if value is None else str(value)


def normalized_labels(raw: object) -> frozenset[str]:
    if not isinstance(raw, list):
        return frozenset()
    return frozenset(
        str(label.get("name", "")).strip().lower()
        for label in raw
        if isinstance(label, dict) and str(label.get("name", "")).strip()
    )


def _author_login(raw: dict) -> str | None:
    author = raw.get("author")
    return _optional_str(author, "login") if isinstance(author, dict) else None


def parse_list_pr(raw: dict) -> PullRequest:
    return PullRequest(
        number=int(raw["number"]),
        state=str(raw.get("state", "")).upper(),
        is_draft=bool(raw.get("isDraft")),
        base_ref=str(raw.get("baseRefName", "")),
        labels=normalized_labels(raw.get("labels")),
        head_ref=_optional_str(raw, "headRefName"),
        author_login=_author_login(raw),
        title=_optional_str(raw, "title"),
    )


def parse_live_pr(number: int, raw: dict | None) -> PullRequest:
    if not isinstance(raw, dict):
        # An unreadable PR keeps head_ref=None, so #1135's guard fails CLOSED on it.
        return PullRequest(number, "UNKNOWN", False, "", frozenset())
    labels = raw.get("labels") or {}
    return PullRequest(
        number=int(raw.get("number", number)),
        state=str(raw.get("state", "")).upper(),
        is_draft=bool(raw.get("isDraft")),
        base_ref=str(raw.get("baseRefName", "")),
        labels=normalized_labels(labels.get("nodes")),
        has_auto_merge=raw.get("autoMergeRequest") is not None,
        has_queue_entry=raw.get("mergeQueueEntry") is not None,
        labels_truncated=bool((labels.get("pageInfo") or {}).get("hasNextPage")),
        head_ref=_optional_str(raw, "headRefName"),
        author_login=_author_login(raw),
        title=_optional_str(raw, "title"),
    )


def exclusion_labels(labels: frozenset[str]) -> list[str]:
    """Return every fail-closed hold label, including the whole needs:* namespace."""
    return sorted(
        label
        for label in labels
        if label.startswith("needs:") or label in EXCLUDED_LABELS
    )


def run_gh(argv: list[str]) -> str:
    result = subprocess.run(["gh", *argv], capture_output=True, text=True, check=False)
    if result.returncode != 0:
        detail = result.stderr.strip() or result.stdout.strip() or "unknown gh failure"
        raise GhError(f"gh {' '.join(argv[:3])} failed: {detail}")
    return result.stdout


def run_gh_read(argv: list[str]) -> str:
    """Idempotent READ path: bounded transient-only retries (#3759).

    Non-transient failures re-raise as :class:`GhError` so the existing per-PR
    error handling keeps working; exhausted transients propagate as
    :class:`gh_retry.GhTransientExhausted` for the ::warning + exit-0 sweep policy.
    """
    try:
        return gh_retry.run_gh_read(argv)
    except gh_retry.GhFatalError as error:
        raise GhError(str(error)) from error


class RearmSweeper:
    def __init__(
        self,
        repo: str,
        default_branch: str,
        *,
        max_rearms: int = DEFAULT_MAX_REARMS,
        gh: Callable[[list[str]], str] = run_gh,
        gh_read: Callable[[list[str]], str] | None = None,
        log: Callable[[str], None] = print,
    ) -> None:
        if not 1 <= max_rearms <= DEFAULT_MAX_REARMS:
            raise ValueError(f"max_rearms must be between 1 and {DEFAULT_MAX_REARMS}")
        self.repo = repo
        self.default_branch = default_branch
        self.max_rearms = max_rearms
        self.gh = gh
        # Idempotent READS may go through a retrying runner (#3759); the arm
        # MUTATION always uses the one-shot `gh` runner above.
        self.gh_read = gh_read if gh_read is not None else gh
        self.log = log
        # [OPUS-5] #3760 latches: the fleet must see exactly ONE ::error per run, never
        # one per PR, and the sweep must stop once the token is known to be unable to arm.
        self.capability_error_emitted = False
        self.capability_lost = False
        # [OPUS-5] #3766: the collected outcome lives HERE, not only in a local of run(),
        # so an exception escaping run() can never discard an already-earned failure.
        self.outcome = SweepOutcome()
        try:
            self.owner, self.name = repo.split("/", 1)
        except ValueError as error:
            raise ValueError("repo must be OWNER/REPOSITORY") from error
        if not self.owner or not self.name or "/" in self.name:
            raise ValueError("repo must be OWNER/REPOSITORY")

    def decision(self, pr: PullRequest, *, live: bool) -> str | None:
        # State is deliberately first: closed/merged PRs may report other fields as UNKNOWN.
        if pr.state != "OPEN":
            return f"not-open ({pr.state or 'UNKNOWN'})"
        # [OPUS-5] #1135: the release-plz Release PR is NEVER re-armed. Checked BEFORE
        # every label-derived rule and independent of them — keyed on branch/author/title,
        # so no label combination (adding review:pass, stripping every hold) can make it
        # re-armable. FAIL-CLOSED when the head branch is unknown.
        release_reason = release_pr_guard.arm_block_reason(
            head_ref=pr.head_ref,
            author_login=pr.author_login,
            title=pr.title,
        )
        if release_reason:
            return release_reason
        if pr.is_draft:
            return "draft"
        if pr.base_ref != self.default_branch:
            return f"non-default-base ({pr.base_ref or 'UNKNOWN'})"
        if live and pr.labels_truncated:
            return "live label set exceeds query page"
        if REVIEW_ATTESTATION not in pr.labels:
            return "review:pass attestation absent"
        excluded = exclusion_labels(pr.labels)
        if excluded:
            return f"hard exclusion ({', '.join(excluded)})"
        if live and pr.has_queue_entry:
            return "live mergeQueueEntry"
        if live and pr.has_auto_merge:
            return "live autoMergeRequest"
        return None

    def list_candidates(self) -> list[PullRequest]:
        raw = json.loads(
            self.gh_read(
                [
                    "pr",
                    "list",
                    "--repo",
                    self.repo,
                    "--state",
                    "open",
                    "--label",
                    REVIEW_ATTESTATION,
                    "--limit",
                    "1000",
                    "--json",
                    PR_LIST_FIELDS,
                ]
            )
        )
        if not isinstance(raw, list):
            raise GhError("gh pr list returned a non-list response")
        return [parse_list_pr(item) for item in raw]

    def live_state(self, number: int) -> PullRequest:
        response = json.loads(
            self.gh_read(
                [
                    "api",
                    "graphql",
                    "-f",
                    f"query={LIVE_QUERY}",
                    "-f",
                    f"owner={self.owner}",
                    "-f",
                    f"name={self.name}",
                    "-F",
                    f"number={number}",
                ]
            )
        )
        if response.get("errors"):
            raise GhError(f"GraphQL returned errors: {response['errors']}")
        repository = (response.get("data") or {}).get("repository") or {}
        return parse_live_pr(number, repository.get("pullRequest"))

    def arm(self, number: int) -> None:
        # No merge-method flag: the repository's merge queue chooses the strategy.
        self.gh(["pr", "merge", str(number), "--repo", self.repo, "--auto"])

    def emit(self, number: int, verdict: str, detail: str) -> None:
        self.log(f"[{PROGRAM}] PR #{number}: {verdict} — {detail}")

    def probe_arm_capability(self) -> CapabilityVerdict:
        """Read-only startup probe: can this token arm at all in this repository?

        Decisive only in the directions it can actually read (see the ARM CAPABILITY note
        above). Anything unreadable is INCONCLUSIVE and must not red the run — the real
        mutation is the backstop and its denial is promoted to a capability verdict. An
        exhausted #3759 transient is likewise inconclusive: the sweep's own retrying reads
        remain the authority on whether the cycle can proceed.
        """
        try:
            raw = self.gh_read(
                [
                    "api",
                    "graphql",
                    "-f",
                    f"query={CAPABILITY_QUERY}",
                    "-f",
                    f"owner={self.owner}",
                    "-f",
                    f"name={self.name}",
                ]
            )
        except gh_retry.GhTransientExhausted as error:
            return CapabilityVerdict(
                INCONCLUSIVE, f"capability query exhausted transient retries ({error})"
            )
        except GhError as error:
            if is_arm_denial(str(error)):
                return CapabilityVerdict(
                    CANNOT_ARM, f"the arm token was denied by GitHub ({error})"
                )
            return CapabilityVerdict(
                INCONCLUSIVE, f"capability query failed, assuming armable ({error})"
            )
        try:
            response = json.loads(raw)
        except json.JSONDecodeError as error:
            return CapabilityVerdict(
                INCONCLUSIVE, f"capability query returned non-JSON ({error})"
            )
        if response.get("errors"):
            detail = json.dumps(response["errors"], sort_keys=True)
            if is_arm_denial(detail):
                return CapabilityVerdict(
                    CANNOT_ARM, f"the arm token was denied by GitHub ({detail})"
                )
            return CapabilityVerdict(
                INCONCLUSIVE, f"capability query returned errors ({detail})"
            )
        data = response.get("data") or {}
        repository = data.get("repository")
        if not isinstance(repository, dict):
            return CapabilityVerdict(
                INCONCLUSIVE, "capability query returned no repository node"
            )
        # Diagnostics only — null for App tokens, so it can never be a gate.
        login = str((data.get("viewer") or {}).get("login") or "unknown")
        permission = repository.get("viewerPermission")
        allowed = repository.get("autoMergeAllowed")
        if allowed is False:
            return CapabilityVerdict(
                CANNOT_ARM,
                "repository setting 'Allow auto-merge' is OFF "
                "(GraphQL Repository.autoMergeAllowed=false)",
            )
        if allowed is not True:
            return CapabilityVerdict(
                INCONCLUSIVE,
                f"autoMergeAllowed not reported (token={login}, "
                f"viewerPermission={permission})",
            )
        return CapabilityVerdict(
            CAN_ARM,
            f"autoMergeAllowed=true (token={login}, viewerPermission={permission}); "
            "write scope is confirmed by the first arm",
        )

    def fail_capability(self, detail: str) -> None:
        """Emit the single ::error for this run and stop attempting further arms."""
        self.capability_lost = True
        if self.capability_error_emitted:
            return
        self.capability_error_emitted = True
        self.log(
            f"::error title={PROGRAM} cannot enable auto-merge::{detail} — "
            f"{ARM_REMEDIATION}"
        )

    def run(self) -> SweepOutcome:
        # [OPUS-5] #3766: publish the outcome on the SWEEPER before doing any work, so
        # every failure collected below survives any exception that escapes this method and
        # the exit status can be computed at the END from this state (see sweep_exit).
        outcome = self.outcome = SweepOutcome()
        verdict = self.probe_arm_capability()
        outcome.capability = verdict.status
        self.log(f"[{PROGRAM}] arm-capability probe: {verdict.status} — {verdict.detail}")
        if verdict.blocks_sweep:
            # Fail ONCE, before touching any PR: a broken token would otherwise fail
            # identically on every candidate, every ten minutes, forever. This is NOT a
            # transient, so it never reaches the #3759 ::warning + exit-0 path.
            self.fail_capability(f"startup arm-capability probe failed: {verdict.detail}")
            self.log(
                f"[{PROGRAM}] complete: candidates=0 re-arm-attempts=0 armed=0 "
                f"arm-failures=0 state-failures=0 capability={verdict.status}"
            )
            return outcome

        try:
            candidates = self.list_candidates()
        except gh_retry.GhTransientExhausted as error:
            # Whole-sweep transient: nothing has been collected yet, so this is a plain
            # missed cycle. Recorded, not raised — the exit is decided at the end.
            outcome.sweep_transient = str(error)
            self.log(
                f"[{PROGRAM}] candidate enumeration exhausted transient retries ({error})"
            )
            candidates = []
        outcome.candidates = len(candidates)
        self.log(
            f"[{PROGRAM}] found {len(candidates)} open PR(s) returned for "
            f"label {REVIEW_ATTESTATION}; re-arm limit={self.max_rearms}"
        )
        for snapshot in candidates:
            # [OPUS-5] #3766 (BLOCKING, reproduced by cross-provider review): every read in
            # this body is retriable, so a LATER candidate's exhausted transient could
            # escape the loop, unwind run(), and land in main()'s lenient
            # `except GhTransientExhausted -> ::warning + exit 0` — discarding an EARLIER
            # candidate's collected arm failure (PR #3011 arm-failed, PR #3012 live-state
            # exhausted, run reported success). The exhaustion is now this candidate's OWN
            # warning-class outcome: the sweep continues and the verdict #3011 already
            # earned is untouchable.
            try:
                if self.capability_lost:
                    self.emit(
                        snapshot.number,
                        "SKIP",
                        "arm capability lost this run (see the single ::error above)",
                    )
                    continue

                reason = self.decision(snapshot, live=False)
                if reason:
                    self.emit(snapshot.number, "SKIP", reason)
                    continue

                try:
                    current = self.live_state(snapshot.number)
                except (GhError, json.JSONDecodeError) as error:
                    self.emit(
                        snapshot.number, "SKIP", f"live-state query failed ({error})"
                    )
                    outcome.state_failures.append((snapshot.number, str(error)))
                    continue

                reason = self.decision(current, live=True)
                if reason:
                    self.emit(current.number, "SKIP", reason)
                    continue

                # Both fields are absent here: and only here is the arm considered dropped.
                if outcome.attempts >= self.max_rearms:
                    self.emit(current.number, "SKIP", "per-run re-arm limit reached")
                    continue
                outcome.attempts += 1
                try:
                    self.arm(current.number)
                except gh_retry.GhTransientExhausted as exhausted:
                    # [OPUS-5] #3766, fail CLOSED. The arm runs on the ONE-SHOT runner,
                    # which never raises this today (gh_retry refuses to wrap mutations) —
                    # but if it ever did, the mutation's outcome would be UNKNOWN, and an
                    # unknown arm must be a per-PR FAILURE, never the lenient
                    # warning-class outcome a transient normally gets.
                    detail = (
                        "re-arm failed (transient exhaustion on the arm mutation "
                        f"itself: {exhausted})"
                    )
                    self.emit(current.number, "ARM-FAILED", detail)
                    outcome.arm_failures.append((current.number, detail))
                    continue
                except GhError as error:
                    message = str(error)
                    # [OPUS-5] #3760: a capability denial is NOT a per-PR condition — every
                    # remaining candidate would fail identically. Record it, stop arming,
                    # and surface exactly ONE ::error naming what to change.
                    if is_arm_denial(message):
                        self.emit(
                            current.number,
                            "ARM-FAILED",
                            f"arm capability denied ({message})",
                        )
                        outcome.arm_failures.append(
                            (current.number, f"arm capability denied ({message})")
                        )
                        outcome.capability = CANNOT_ARM
                        self.fail_capability(
                            f"arming PR #{current.number} was denied: {message}"
                        )
                        continue
                    # The arm MUTATION failed — that failure is the primary status. The
                    # follow-up live-state read is a diagnostic to classify an API race,
                    # and it is retriable: it can raise GhError, a decode error, OR
                    # GhTransientExhausted. [FABLE-5] #3759 finding 5: the diagnostic's
                    # OWN exhaustion must never escape here — if it did, main's lenient
                    # `except GhTransientExhausted -> exit 0` would convert a genuine
                    # failed arm into a false success. Catch exhaustion too; on any
                    # inconclusive diagnostic, record the arm failure as a real error.
                    try:
                        raced = self.live_state(current.number)
                        race_reason = self.decision(raced, live=True)
                    except (
                        GhError,
                        json.JSONDecodeError,
                        gh_retry.GhTransientExhausted,
                    ):
                        race_reason = None
                    if race_reason:
                        self.emit(current.number, "SKIP", f"arm raced: {race_reason}")
                    else:
                        # Collect and CONTINUE — one bad PR must never abort the sweep.
                        self.emit(
                            current.number, "ARM-FAILED", f"re-arm failed ({message})"
                        )
                        outcome.arm_failures.append(
                            (current.number, f"re-arm failed ({message})")
                        )
                    continue
            except gh_retry.GhTransientExhausted as error:
                outcome.transient_exhaustions.append((snapshot.number, str(error)))
                self.emit(
                    snapshot.number,
                    "SKIP",
                    f"transient-exhausted ({error}); the sweep continues and this "
                    "cannot downgrade an earlier arm failure",
                )
                continue
            outcome.armed += 1
            self.emit(current.number, "ARMED", "dropped auto-merge request restored")

        for number, reason in outcome.arm_failures:
            self.log(f"[{PROGRAM}] arm-failure summary: PR #{number} — {reason}")
        for number, reason in outcome.state_failures:
            self.log(f"[{PROGRAM}] state-failure summary: PR #{number} — {reason}")
        for number, reason in outcome.transient_exhaustions:
            self.log(f"[{PROGRAM}] transient-exhaustion summary: PR #{number} — {reason}")
        self.log(
            f"[{PROGRAM}] complete: candidates={outcome.candidates} "
            f"re-arm-attempts={outcome.attempts} armed={outcome.armed} "
            f"arm-failures={len(outcome.arm_failures)} "
            f"state-failures={len(outcome.state_failures)} "
            f"transient-exhaustions={len(outcome.transient_exhaustions)} "
            f"capability={outcome.capability}"
        )
        return outcome


# ======================================================================================
# [OPUS-5] #4548 — THE STUCK-ARM SWEEP: the missing terminal edge out of "armed".
#
# The sweep above closes ONE direction: a reviewed PR whose arm GitHub dropped gets
# re-armed. Nothing closed the OTHER direction. `decision()` above returns
# "live autoMergeRequest" and SKIPS — so the moment a PR is armed it becomes invisible to
# every sweep in this repo, forever, whatever happens to it afterwards.
#
# MEASURED 2026-07-27T10:45Z on sparq-org/sparq: merge-queue depth 0, 3 PRs armed, 0
# commits merged in the preceding hour. All three were armed AND individually unmergeable:
# #4373 CONFLICTING against a sibling that had merged; #4354 armed on a `gate` that was
# `completed/failure` (a clippy error in sparq-core cascading into 10 opt-in legs); #3451
# green-gated but BLOCKED for 19h on ten unresolved CodeQL review threads under the
# ruleset's `required_review_thread_resolution`. Armed LOOKS healthy, so nothing paged;
# each PR also holds its `area:` partition against the readiness frontier, so the lane was
# not merely idle, it was self-blocking. This is the missing-terminal-edge defect fixed in
# registry #753/#754, and per-run success rates structurally cannot express it: every
# individual run of every sweep was green.
#
# The fix is to give the armed population a TOTAL classifier and give every terminal class
# a visible, counted exit. Two design rules the scars here demand:
#
#   FAIL OPEN ON ANYTHING PENDING OR UNREADABLE. `gate` legitimately sits `in_progress`
#   for 20-40 minutes on heavy lanes, and green-but-BLOCKED is usually asynchronous
#   ruleset evaluation that self-resolves in ~11 minutes. Wrongly disarming a healthy PR
#   costs a whole review round; leaving a stuck PR one more tick costs ten minutes. So
#   every indeterminate reading maps to a NO-ACTION class that is still COUNTED.
#
#   EVERY ACTION MUST LEAVE THE POPULATION. The mutating classes all end in a disarm, a
#   branch update, or a check-run re-request — each of which either drops the PR out of
#   the armed set or bumps its activity back under the grace window. That is what keeps a
#   ten-minute cron from commenting on the same PR 144 times a day; there is no marker
#   comment to latch on, because the state transition IS the latch.
# ======================================================================================

# The tier-correct required-check names. sparq's ci-summary.yml publishes `gate` on a
# ready payload and `gate, draft-tier` on a draft one, so a lookup keyed on exactly `gate`
# finds NOTHING on a draft — the defect that left the registry's repair lane unreachable
# from 2026-07-17 (registry #761). These are matched by EXACT EQUALITY, never by prefix:
# `gate` is a strict prefix of `gate, draft-tier`, so a `startswith` lookup would silently
# resolve a draft's gate for a ready PR (and a fixture that only ever contains one of the
# two names cannot tell the difference — see the prefix tripwire in the self-test).
GATE_CHECK_NAME = "gate"
DRAFT_GATE_CHECK_NAME = "gate, draft-tier"

# Gate-resolution outcomes. UNKNOWN and MISSING are deliberately DISTINCT: an unpaginated
# or short check-run read must not be reported as "there is no gate". Check-run sets on
# this repo really do exceed one page — MEASURED 2026-07-27: 155 / 680 / 800 runs on the
# three stuck heads — so a default `commits/<sha>/check-runs` read returns no `gate` row
# at all, which is indistinguishable from a genuinely absent gate unless the two are
# separate values. UNKNOWN is fail-open; MISSING is a real observation about a complete read.
GATE_SUCCESS = "success"
GATE_FAILURE = "failure"
GATE_PENDING = "pending"
GATE_CANCELLED = "cancelled"
GATE_MISSING = "missing"
GATE_UNKNOWN = "unknown"

_GATE_OK_CONCLUSIONS = frozenset({"success", "neutral", "skipped"})
_GATE_BAD_CONCLUSIONS = frozenset(
    {"failure", "timed_out", "action_required", "startup_failure"}
)
# `stale` is GitHub's term for a run superseded before it could report; like `cancelled`
# it produced no verdict, so both route to the bounded re-trigger rather than to a disarm.
_GATE_CANCELLED_CONCLUSIONS = frozenset({"cancelled", "stale"})

# Labels that mean a human or the trust plane has taken the PR off the automated path.
# Mirrors auto-arm.py's HUMAN_OR_TRUST_LABELS intent for the *hold* half only: an
# `area:sparq-zk` PR is allowed to be armed by a reviewer, but a `needs:*` hold is not.
STUCK_HOLD_LABELS = frozenset({"review:changes", "review:needs", "trust:untrusted"})

# ---- the class enum. TOTAL over the armed population, and closed: `CLASS_ACTIONS` is the
# single source of truth for both the class set and its routing, so adding a class without
# routing it is a KeyError at import time rather than a silent no-op at 03:00. ----------
ACTION_NONE = "none"
ACTION_PARK = "park"
ACTION_ROUTE_FIX = "route-fix"
ACTION_REBASE = "rebase"
ACTION_RETRIGGER = "retrigger"

CLASS_ACTIONS = {
    # --- fail-open: the PR is progressing or unreadable. Counted, never touched. -------
    "queued": ACTION_NONE,               # live mergeQueueEntry: the queue owns it
    "gate-pending": ACTION_NONE,         # 20-40 min on heavy lanes is NORMAL
    "gate-indeterminate": ACTION_NONE,   # short/malformed check-run or label read
    "gate-missing": ACTION_NONE,         # complete read, no gate row yet
    "ruleset-grace": ACTION_NONE,        # recent activity; async ruleset eval ~11 min
    "progressing": ACTION_NONE,          # green and not BLOCKED — about to merge
    "blocked-unexplained": ACTION_NONE,  # green, BLOCKED, no thread — reported, not acted
    # --- terminal: each gets a visible, counted exit. ---------------------------------
    "held": ACTION_PARK,                 # armed while a hard hold label is on it
    "conflicting": ACTION_REBASE,        # DIRTY — attempt the update, then route
    "gate-failed": ACTION_ROUTE_FIX,     # armed on red: can never merge
    "gate-cancelled": ACTION_RETRIGGER,  # no verdict was ever produced
    "blocked-threads": ACTION_PARK,      # unresolved conversations — human-owned
    "stale": ACTION_PARK,                # past the stale horizon with no other cause
}
# The classes whose action mutates the PR. Used for the per-tick bound and for the report.
TERMINAL_CLASSES = frozenset(
    name for name, action in CLASS_ACTIONS.items() if action != ACTION_NONE
)

STUCK_LIST_FIELDS = (
    "number,state,isDraft,baseRefName,headRefName,labels,author,title,"
    "mergeable,mergeStateStatus,updatedAt,headRefOid,autoMergeRequest"
)

# Live per-PR state. `mergeable`/`mergeStateStatus` and the review threads must come from
# the SAME read as the arm state, or the classifier decides on a mixture of two snapshots.
STUCK_LIVE_QUERY = """query($owner:String!,$name:String!,$number:Int!){
  repository(owner:$owner,name:$name){
    pullRequest(number:$number){
      number
      state
      isDraft
      baseRefName
      updatedAt
      headRefOid
      mergeable
      mergeStateStatus
      labels(first:100){nodes{name} pageInfo{hasNextPage}}
      autoMergeRequest{enabledAt}
      mergeQueueEntry{id}
      reviewThreads(first:100){
        totalCount
        pageInfo{hasNextPage}
        nodes{isResolved}
      }
    }
  }
}"""

OPEN_PR_COUNT_QUERY = """query($owner:String!,$name:String!){
  repository(owner:$owner,name:$name){
    pullRequests(states:OPEN){totalCount}
  }
}"""

STUCK_MARKER = "> 🤖 SPARQ agent"


@dataclass(frozen=True)
class StuckLimits:
    """Every bound the sweep obeys, in one place so the self-test can shrink them."""

    # Green-but-BLOCKED is async ruleset evaluation that self-resolves in ~11 minutes, and
    # a just-pushed head can show the PREVIOUS run's conclusion before the new run is
    # created. 20 minutes covers both with margin; below it, nothing terminal can fire.
    grace_seconds: int = 20 * 60
    # Past this with no other identified cause, the PR is parked WITH A REASON rather than
    # left to look healthy indefinitely.
    stale_seconds: int = 6 * 3600
    # The congestion bound. This repo has a measured congestion-collapse mode: over-
    # dispatching pushes saturates the runners and manufactures false gate failures. A
    # backlog that all becomes actionable on one tick must therefore drain over several
    # ticks, not fire at once. Every deferred PR is still CLASSIFIED and COUNTED.
    max_actions: int = 5


@dataclass(frozen=True)
class ArmedPull:
    """The classifier's whole input. Pure data: no gh handle, no clock, no I/O."""

    number: int
    is_draft: bool
    base_ref: str
    labels: frozenset[str]
    mergeable: str          # MERGEABLE | CONFLICTING | UNKNOWN
    merge_state: str        # BLOCKED | CLEAN | DIRTY | BEHIND | UNSTABLE | UNKNOWN | ...
    updated_at: int         # epoch seconds
    armed_at: int           # epoch seconds
    head_oid: str
    has_queue_entry: bool
    unresolved_threads: int
    state_truncated: bool = False   # a label or review-thread page overflowed


def _iso_epoch(value: object) -> int:
    """ISO-8601 Z timestamp -> epoch seconds; 0 when absent or unparseable.

    0 makes an unreadable timestamp look INFINITELY OLD, which would push a PR straight to
    `stale`. That is the wrong direction, so callers combine timestamps with `max()` and
    the classifier treats a wholly-unknown activity time as indeterminate — see `classify`.
    """
    if not isinstance(value, str) or not value:
        return 0
    try:
        return int(
            datetime.strptime(value, "%Y-%m-%dT%H:%M:%SZ")
            .replace(tzinfo=timezone.utc)
            .timestamp()
        )
    except ValueError:
        return 0


def gate_check_name(is_draft: bool) -> str:
    """The required-check name for this PR's TIER. See GATE_CHECK_NAME above."""
    return DRAFT_GATE_CHECK_NAME if is_draft else GATE_CHECK_NAME


def _run_order(run: dict) -> tuple:
    """Newest-run ordering key. `started_at` is ISO-8601 Z so it sorts lexicographically;
    the numeric check-run id breaks ties and is monotonic per repository."""
    started = run.get("started_at")
    ident = run.get("id")
    return (
        started if isinstance(started, str) else "",
        ident if isinstance(ident, int) else -1,
    )


def collect_check_runs(pages: object) -> tuple[list[dict] | None, int | None]:
    """Flatten `gh api --paginate --slurp .../check-runs` output.

    Returns ``(runs, declared_total)``. ``runs`` is None when the payload shape is not
    recognisable at all. ``declared_total`` is the API's own ``total_count`` from the first
    page, used by `resolve_gate` to prove the read was COMPLETE.
    """
    if not isinstance(pages, list):
        return None, None
    runs: list[dict] = []
    declared: int | None = None
    for page in pages:
        if not isinstance(page, dict):
            return None, None
        if declared is None and isinstance(page.get("total_count"), int):
            declared = page["total_count"]
        chunk = page.get("check_runs")
        if not isinstance(chunk, list):
            return None, None
        runs.extend(item for item in chunk if isinstance(item, dict))
    return runs, declared


def resolve_gate(pages: object, *, is_draft: bool) -> str:
    """The TIER-CORRECT, NEWEST-RUN-PER-NAME state of this head's required gate.

    Three properties, each one a scar:

    * COMPLETENESS FIRST. If the API declared more check runs than the read returned, the
      answer is UNKNOWN — never MISSING. An unpaginated read of an 800-run head returns no
      `gate` row, and reporting that as "no gate" would disarm a perfectly healthy PR.
    * EXACT NAME. `gate` and `gate, draft-tier` are matched by equality against the tier's
      name, so neither can ever stand in for the other.
    * NEWEST RUN WINS. GitHub leaves the CANCELLED twin of a superseded run on the head
      alongside the run that replaced it. Resolving by "any cancelled run" would mask a
      green newest run — MEASURED on PR #3451, where five distinct check names each carry a
      cancelled twin behind a newer success/skip.
    """
    runs, declared = collect_check_runs(pages)
    if runs is None:
        return GATE_UNKNOWN
    if declared is None or len(runs) < declared:
        return GATE_UNKNOWN
    wanted = gate_check_name(is_draft)
    rows = [run for run in runs if run.get("name") == wanted]
    if not rows:
        return GATE_MISSING
    newest = max(rows, key=_run_order)
    if newest.get("status") != "completed":
        return GATE_PENDING
    conclusion = newest.get("conclusion")
    if conclusion in _GATE_OK_CONCLUSIONS:
        return GATE_SUCCESS
    if conclusion in _GATE_BAD_CONCLUSIONS:
        return GATE_FAILURE
    if conclusion in _GATE_CANCELLED_CONCLUSIONS:
        return GATE_CANCELLED
    return GATE_UNKNOWN


def hold_labels(labels: frozenset[str]) -> list[str]:
    """The hard holds that make an ARM unsafe, not merely unproductive.

    A `needs:*` or `review:changes` PR that is still armed will merge the instant its gate
    goes green, straight past the hold. That is the one class here where INACTION is the
    dangerous direction, so it disarms.
    """
    return sorted(
        label
        for label in labels
        if label.startswith("needs:") or label in STUCK_HOLD_LABELS
    )


def classify(pull: ArmedPull, gate: str, now: int, limits: StuckLimits) -> str:
    """TOTAL map from (live PR state, gate state) to exactly one class of `CLASS_ACTIONS`.

    ORDER IS THE POLICY, and it is fail-open first:

      1-3. Anything progressing or unreadable exits here, before any bound is consulted.
      4.   The grace window. NOTHING terminal fires inside it — a green-but-BLOCKED PR is
           usually mid-ruleset-evaluation, and a just-pushed head can still be showing the
           previous run's conclusion.
      5.   Holds and CONFLICTING are decided BEFORE any gate reading — see below.
      6+.  The gate readings, fail-open first, then the stale horizon as the backstop so
           that nothing can sit armed and unexplained forever.

    WHY HOLDS AND `CONFLICTING` OUTRANK THE GATE READING. "Fail open on pending" is the
    right rule for a PR that is merely waiting, and the WRONG rule for these two, because
    for them a pending gate is not evidence of progress:

    * A `CONFLICTING` PR cannot merge on any gate result. Its gate — green, red or absent —
      was computed against a merge that no longer exists, so no CI outcome changes the
      remedy, and deferring to a pending or unreadable gate simply defers forever. This
      is #4354's exact shape: `gate` concluded SUCCESS at 10:57 and the branch went dirty
      at 11:10, so its green is an answer to a question nobody is asking any more.
      HONESTY ABOUT THE EVIDENCE: this ordering is NOT justified by the stronger claim
      that a dirty PR never gets CI. MEASURED 2026-07-27 over all 26 open CONFLICTING PRs
      in this repo: 25 carry check runs on their head (60-788 of them) and six acquired
      new runs DAYS after going dirty, so workflows plainly do dispatch onto dirty heads.
      Exactly one (#3815) has zero check runs at all — rare, but it is the case in which a
      CI-pending fail-open strands a PR permanently, and it costs nothing to exclude.
    * A HELD PR is the one class where inaction is the dangerous direction: it is armed
      under a hold, so it merges PAST that hold the instant the gate greens. A pending
      gate makes that more urgent, not less.

    Both are still inside the grace window and both still fail open on a truncated read.
    """
    if pull.has_queue_entry:
        return "queued"
    if pull.state_truncated:
        return "gate-indeterminate"
    activity = max(pull.updated_at, pull.armed_at)
    if activity <= 0:
        # No readable activity timestamp at all: age is unknowable, so no age-derived
        # class may fire. Fail open rather than treat "unparseable" as "ancient".
        return "gate-indeterminate"
    age = now - activity
    if age < limits.grace_seconds:
        return "ruleset-grace"
    # [OPUS-5] The GRACE window and the STALE horizon measure DIFFERENT things, and
    # conflating them starves the backstop. Grace asks "did something just happen?", so it
    # is right to take the most recent event of any kind. Stale asks "how long has this arm
    # been failing to merge?" — and `updatedAt` is bumped by every label flip, every bot
    # comment and every review event, so a PR under routine pipeline churn would reset the
    # horizon forever and the one class that exists to catch everything else would be the
    # easiest of all to starve. MEASURED on #3451: armed 33h, `updatedAt` 83 minutes old.
    # Falls back to the activity clock only when the arm timestamp is unreadable.
    armed_age = now - (pull.armed_at or activity)
    if hold_labels(pull.labels):
        return "held"
    if pull.mergeable == "CONFLICTING":
        return "conflicting"
    # ---- from here the gate reading decides, and it fails OPEN. -----------------------
    # The three ways a head can show "no usable gate" are DIFFERENT states with different
    # remedies, and collapsing them is how a sweeper disarms a healthy PR: `conflicting`
    # (handled above — rebase, never wait), GATE_UNKNOWN from a short read (paginate and
    # re-read; never conclude), and GATE_MISSING on a complete read of a mergeable head
    # (the only genuine "no gate" case). MEASURED on PR #4369: total_count=486, an
    # unpaginated read returns 30 rows and ZERO gate rows.
    if gate == GATE_PENDING:
        return "gate-pending"
    if gate == GATE_UNKNOWN:
        return "gate-indeterminate"
    if gate == GATE_FAILURE:
        return "gate-failed"
    if gate == GATE_CANCELLED:
        return "gate-cancelled"
    if gate == GATE_MISSING:
        return "stale" if armed_age >= limits.stale_seconds else "gate-missing"
    # The gate is green from here.
    if pull.merge_state != "BLOCKED":
        return "progressing"
    if pull.unresolved_threads > 0:
        return "blocked-threads"
    if armed_age >= limits.stale_seconds:
        return "stale"
    return "blocked-unexplained"


CLASS_REASONS = {
    "held": (
        "it is armed while carrying a hard hold label. An armed PR merges the moment its "
        "gate goes green, so the arm has been removed; the hold itself is untouched."
    ),
    "conflicting": (
        "its branch conflicts with the base. The automatic branch update could not resolve "
        "it, so it needs a manual rebase."
    ),
    "gate-failed": (
        "it was armed on a `gate` that has already concluded FAILURE. It can never merge in "
        "that state, so the arm has been removed and it is routed to the fix lane."
    ),
    "blocked-threads": (
        "its gate is green but the branch ruleset requires every review thread to be "
        "resolved, and unresolved threads remain. Nothing automated can resolve them, so "
        "the arm has been removed and the PR is parked for a human."
    ),
    "stale": (
        "it has been armed and unmergeable past the stale horizon with no other identified "
        "cause. The arm has been removed so it stops looking healthy while it is not."
    ),
}


# ---- THE MACHINE-READABLE RECEIPT ------------------------------------------------------
#
# [OPUS-5] #4548 follow-through. A park whose only artefact is prose has a HUMAN-ONLY exit,
# and a human-only exit turns a transient cause into a permanent stall: the PR sits parked
# long after the thing that parked it went away, because nothing can read "unresolved
# conversations remain" and check whether that is still true. Registry #766 found exactly
# this — four PRs human-terminal purely because a human applied the label.
#
# So every park additionally emits ONE machine-readable receipt: the class, the head it was
# bound to, the observation that justified it, and — the load-bearing field — the named
# CONDITION under which the park is provably over. `unpark_satisfied` evaluates that
# condition against a FRESH observation, and is fail-CLOSED in every direction: an
# unrecognised condition, a version it does not know, a receipt from another PR, or an
# incomplete read all keep the PR parked. Recovery must be PROVEN, never assumed.
RECEIPT_VERSION = 1
RECEIPT_OPEN = "<!-- stuck-arm-receipt:"
RECEIPT_CLOSE = "-->"

# class -> the named condition that ENDS this park. The condition is recorded IN the receipt
# rather than re-derived from the class at un-park time, so a policy change here can never
# silently re-interpret a receipt written by an older revision.
UNPARK_HOLDS_CLEARED = "holds-cleared"
UNPARK_NOT_CONFLICTING = "not-conflicting"
UNPARK_GATE_GREEN = "gate-green"
UNPARK_GATE_CONCLUDED = "gate-concluded"
UNPARK_THREADS_RESOLVED = "threads-resolved"
UNPARK_HEAD_MOVED = "head-moved"

UNPARK_CONDITIONS = {
    "held": UNPARK_HOLDS_CLEARED,
    "conflicting": UNPARK_NOT_CONFLICTING,
    "gate-failed": UNPARK_GATE_GREEN,
    "gate-cancelled": UNPARK_GATE_CONCLUDED,
    "blocked-threads": UNPARK_THREADS_RESOLVED,
    # `stale` has no single identified cause by construction, so the only honest proof that
    # it is over is that somebody moved the branch on.
    "stale": UNPARK_HEAD_MOVED,
}
# Import-time totality. A terminal class added without a machine exit is precisely the
# "holds need a machine exit" defect, so it must be impossible to add one by omission.
assert set(UNPARK_CONDITIONS) == TERMINAL_CLASSES, (
    "every terminal class needs a named un-park condition: "
    f"missing={sorted(TERMINAL_CLASSES - set(UNPARK_CONDITIONS))} "
    f"extra={sorted(set(UNPARK_CONDITIONS) - TERMINAL_CLASSES)}"
)


def stuck_receipt(pull: ArmedPull, klass: str, gate: str, now: int) -> dict:
    """The machine-readable half of a park. Pure: same inputs, same bytes.

    `head` binds the receipt to the exact commit that was observed — a receipt found on a
    PR whose head has since moved describes a state that no longer exists, which is both the
    `head-moved` proof and the reason every other condition is re-evaluated live rather than
    trusted from the receipt.
    """
    return {
        "v": RECEIPT_VERSION,
        "program": PROGRAM,
        "phase": "stuck-arm",
        "pr": pull.number,
        "class": klass,
        "action": CLASS_ACTIONS[klass],
        "head": pull.head_oid,
        "observed_at": now,
        "observed": {
            "gate": gate,
            "mergeable": pull.mergeable,
            "merge_state": pull.merge_state,
            "unresolved_threads": pull.unresolved_threads,
            "holds": hold_labels(pull.labels),
        },
        "unpark_when": UNPARK_CONDITIONS[klass],
    }


def render_receipt(receipt: dict) -> str:
    """One line, HTML-comment-wrapped so it renders as nothing and greps as everything."""
    return (
        f"{RECEIPT_OPEN} "
        f"{json.dumps(receipt, sort_keys=True, separators=(',', ':'), ensure_ascii=False)} "
        f"{RECEIPT_CLOSE}"
    )


def parse_stuck_receipt(body: object) -> dict | None:
    """Recover the receipt from a comment body, or None.

    Tolerant of surrounding prose (the receipt is appended to a human-readable comment) and
    of a body carrying several — the LAST wins, because a later sweep's observation
    supersedes an earlier one. Returns None on anything it cannot fully parse: a caller that
    cannot read the receipt must behave as if the park were opaque, never as if it were
    satisfied.
    """
    if not isinstance(body, str) or RECEIPT_OPEN not in body:
        return None
    start = body.rfind(RECEIPT_OPEN)
    end = body.find(RECEIPT_CLOSE, start + len(RECEIPT_OPEN))
    if end < 0:
        return None
    try:
        parsed = json.loads(body[start + len(RECEIPT_OPEN):end].strip())
    except (ValueError, TypeError):
        return None
    if not isinstance(parsed, dict):
        return None
    return parsed


def unpark_satisfied(receipt: object, pull: ArmedPull, gate: str) -> bool:
    """Has the recorded cause of this park PROVABLY gone away?

    FAIL-CLOSED on every axis. The dangerous direction here is un-parking a PR whose cause
    still holds — that re-admits it to the merge lane where it will wedge again — so
    anything short of positive proof returns False:

      * a receipt this revision does not understand (version, shape, wrong PR number);
      * a condition name that is not in the closed set (a receipt from a FUTURE revision
        naming a condition this one cannot evaluate);
      * an incomplete live read (`state_truncated`), or a gate that is not a concluded
        observation, for any condition that depends on those.

    It is deliberately NOT a method: the un-park decision is a pure function of a receipt
    plus a fresh observation, and keeping it pure is what makes it testable without gh.
    """
    if not isinstance(receipt, dict):
        return False
    if receipt.get("v") != RECEIPT_VERSION:
        return False
    if receipt.get("program") != PROGRAM or receipt.get("phase") != "stuck-arm":
        return False
    if receipt.get("pr") != pull.number:
        return False
    # A truncated label or review-thread page means the fresh observation is itself partial;
    # no condition can be PROVEN against it.
    if pull.state_truncated:
        return False
    condition = receipt.get("unpark_when")
    if condition == UNPARK_HOLDS_CLEARED:
        return not hold_labels(pull.labels)
    if condition == UNPARK_NOT_CONFLICTING:
        # UNKNOWN is GitHub still computing the merge — not proof of anything.
        return pull.mergeable == "MERGEABLE"
    if condition == UNPARK_GATE_GREEN:
        return gate == GATE_SUCCESS
    if condition == UNPARK_GATE_CONCLUDED:
        return gate in (GATE_SUCCESS, GATE_FAILURE)
    if condition == UNPARK_THREADS_RESOLVED:
        return pull.unresolved_threads == 0
    if condition == UNPARK_HEAD_MOVED:
        recorded = receipt.get("head")
        return bool(pull.head_oid) and isinstance(recorded, str) and pull.head_oid != recorded
    return False


# The exact phrase that CLAIMS the park clears itself. Named once so the self-test can ask
# the rendered comment whether the claim was made, rather than eyeballing an f-string.
AUTO_UNPARK_PROMISE = "Un-parks automatically when:"


def unpark_production_callers() -> set[str]:
    """Names of the functions that actually CALL `unpark_satisfied`, minus the self-test.

    Asked of the parse tree, not of the file's text. A containment check ("does
    'unpark_satisfied' appear outside the self-test?") is vacuous for this guard class —
    the identifier necessarily appears in its own definition, its docstring and this very
    function — so it would pass while the evaluator remained dead. The AST question has
    exactly one answer: for each `Call` to that name, which `FunctionDef` most closely
    encloses it?

    Returning names (not a bool) is deliberate: the assertion message can then say WHO wired
    it, which is the difference between a failure a reader can act on and one they cannot.
    """
    tree = ast.parse(Path(__file__).resolve().read_text(encoding="utf-8"))
    parents: dict[ast.AST, ast.AST] = {}
    for node in ast.walk(tree):
        for child in ast.iter_child_nodes(node):
            parents[child] = node

    callers: set[str] = set()
    for node in ast.walk(tree):
        if not isinstance(node, ast.Call):
            continue
        func = node.func
        name = func.id if isinstance(func, ast.Name) else getattr(func, "attr", None)
        if name != "unpark_satisfied":
            continue
        enclosing, cursor = "<module>", parents.get(node)
        while cursor is not None:
            if isinstance(cursor, (ast.FunctionDef, ast.AsyncFunctionDef)):
                enclosing = cursor.name
                break
            cursor = parents.get(cursor)
        callers.add(enclosing)
    return callers - {"stuck_self_test", "unpark_production_callers"}


def stuck_comment(pull: ArmedPull, klass: str, gate: str, detail: str, now: int) -> str:
    """The one comment body: prose for the human, a receipt for the machine.

    Both, not either. The prose is what a maintainer reads; the receipt is what an un-park
    path evaluates. A park with only the first has no exit that does not require a person.
    """
    reason = CLASS_REASONS.get(klass, "it is armed but cannot merge.")
    receipt = stuck_receipt(pull, klass, gate, now)
    return (
        f"{STUCK_MARKER}\n\n"
        f"**stuck-arm sweep — `{klass}`**\n\n"
        f"This PR was armed for auto-merge but {reason}\n\n"
        f"Observed: {detail}\n\n"
        f"Un-park condition: `{receipt['unpark_when']}` — the named state that ends this "
        f"park. It is evaluated by `unpark_satisfied`, which is NOT wired into this sweep, "
        f"so clearing this park is a HUMAN action today (#5041).\n\n"
        f"_Posted by the `{PROGRAM}` stuck-arm phase because an armed PR that cannot merge "
        f"is otherwise invisible: it looks healthy, and it holds its `area:` partition "
        f"against the readiness frontier._\n\n"
        f"{render_receipt(receipt)}"
    )


class StuckArmSweeper:
    """Phase two of this lane: classify the ARMED population and give each class an exit.

    Deliberately a sibling of `RearmSweeper` rather than a separate workflow. The two are
    exact duals over one decision surface — `RearmSweeper.decision()` returns
    "live autoMergeRequest" and skips, and THAT skip is the edge this class closes — so
    they share the repo/owner parsing, the one-shot-mutation vs retrying-read split, and
    the same cron, concurrency group and token.
    """

    def __init__(
        self,
        repo: str,
        default_branch: str,
        *,
        limits: StuckLimits | None = None,
        gh: Callable[[list[str]], str] = run_gh,
        gh_read: Callable[[list[str]], str] | None = None,
        log: Callable[[str], None] = print,
        now: Callable[[], int] = lambda: int(time.time()),
        dry_run: bool = False,
    ) -> None:
        self.repo = repo
        self.default_branch = default_branch
        self.limits = limits or StuckLimits()
        # OBSERVE-ONLY. Exists so the census can be taken against the live repository
        # before any remediation is enabled, and so an operator can answer "what would this
        # tick do" without doing it. The live workflow step MUST NOT pass it — pinned by
        # test_arm_capability_wiring.py::TestStuckArmWiring, because a sweep permanently
        # stuck in dry-run is a lane that reports beautifully and repairs nothing.
        self.dry_run = dry_run
        self.gh = gh
        self.gh_read = gh_read if gh_read is not None else gh
        self.log = log
        self.now = now
        self.counts: dict[str, int] = {}
        self.errors = 0
        self.observed = 0
        self.actions = 0
        self.deferred = 0
        self.truncated_enumeration = False
        self.armed_on_red: list[int] = []
        try:
            self.owner, self.name = repo.split("/", 1)
        except ValueError as error:
            raise ValueError("repo must be OWNER/REPOSITORY") from error
        if not self.owner or not self.name or "/" in self.name:
            raise ValueError("repo must be OWNER/REPOSITORY")

    # ---- reads ------------------------------------------------------------------------

    def _read_json(self, argv: list[str]) -> object:
        return json.loads(self.gh_read(argv))

    def open_pr_total(self) -> int | None:
        """The repository's own count of open PRs, for the enumeration cross-check."""
        response = self._read_json(
            [
                "api", "graphql",
                "-f", f"query={OPEN_PR_COUNT_QUERY}",
                "-f", f"owner={self.owner}",
                "-f", f"name={self.name}",
            ]
        )
        if not isinstance(response, dict) or response.get("errors"):
            return None
        repository = (response.get("data") or {}).get("repository") or {}
        total = (repository.get("pullRequests") or {}).get("totalCount")
        return total if isinstance(total, int) else None

    def list_armed(self) -> list[dict]:
        """Every OPEN PR carrying a live autoMergeRequest, cross-checked against totalCount.

        A SHORT enumeration does not invalidate the PRs we did see — each classification is
        independent — so the sweep continues, but the run is flagged so nobody reads the
        class counts as covering the whole population.
        """
        raw = self._read_json(
            [
                "pr", "list",
                "--repo", self.repo,
                "--state", "open",
                "--limit", "1000",
                "--json", STUCK_LIST_FIELDS,
            ]
        )
        if not isinstance(raw, list):
            raise GhError("gh pr list returned a non-list response")
        total = self.open_pr_total()
        if total is not None and len(raw) < total:
            self.truncated_enumeration = True
            self.log(
                f"::warning title={PROGRAM} stuck-arm enumeration truncated::"
                f"listed {len(raw)} open PR(s) but the repository reports {total} — the "
                "class counts below cover only what was read."
            )
        return [item for item in raw if isinstance(item, dict) and item.get("autoMergeRequest")]

    def live_pull(self, number: int) -> ArmedPull | None:
        response = self._read_json(
            [
                "api", "graphql",
                "-f", f"query={STUCK_LIVE_QUERY}",
                "-f", f"owner={self.owner}",
                "-f", f"name={self.name}",
                "-F", f"number={number}",
            ]
        )
        if not isinstance(response, dict) or response.get("errors"):
            raise GhError(f"GraphQL returned errors for #{number}")
        repository = (response.get("data") or {}).get("repository") or {}
        raw = repository.get("pullRequest")
        if not isinstance(raw, dict):
            return None
        labels = raw.get("labels") or {}
        threads = raw.get("reviewThreads") or {}
        nodes = threads.get("nodes")
        unresolved = 0
        if isinstance(nodes, list):
            unresolved = sum(
                1 for node in nodes
                if isinstance(node, dict) and node.get("isResolved") is False
            )
        # A truncated LABEL page can hide a hold; a truncated THREAD page can hide the
        # unresolved thread that explains a BLOCKED state. Either way the state is only
        # partially known, so the classifier must fail open rather than guess.
        truncated = bool((labels.get("pageInfo") or {}).get("hasNextPage")) or (
            bool((threads.get("pageInfo") or {}).get("hasNextPage")) and unresolved == 0
        )
        return ArmedPull(
            number=int(raw.get("number", number)),
            is_draft=bool(raw.get("isDraft")),
            base_ref=str(raw.get("baseRefName") or ""),
            labels=normalized_labels(labels.get("nodes")),
            mergeable=str(raw.get("mergeable") or "UNKNOWN").upper(),
            merge_state=str(raw.get("mergeStateStatus") or "UNKNOWN").upper(),
            updated_at=_iso_epoch(raw.get("updatedAt")),
            armed_at=_iso_epoch((raw.get("autoMergeRequest") or {}).get("enabledAt")),
            head_oid=str(raw.get("headRefOid") or ""),
            has_queue_entry=raw.get("mergeQueueEntry") is not None,
            unresolved_threads=unresolved,
            state_truncated=truncated,
        )

    def gate_state(self, pull: ArmedPull) -> tuple[str, int | None]:
        """`resolve_gate` over a PAGINATED read, plus the newest gate run's id for re-trigger."""
        if not pull.head_oid:
            return GATE_UNKNOWN, None
        pages = self._read_json(
            [
                "api", "--paginate", "--slurp",
                f"repos/{self.repo}/commits/{pull.head_oid}/check-runs?per_page=100",
            ]
        )
        state = resolve_gate(pages, is_draft=pull.is_draft)
        runs, _declared = collect_check_runs(pages)
        run_id = None
        if runs:
            rows = [r for r in runs if r.get("name") == gate_check_name(pull.is_draft)]
            if rows:
                ident = max(rows, key=_run_order).get("id")
                run_id = ident if isinstance(ident, int) else None
        return state, run_id

    # ---- mutations. One-shot, never through the retrying read runner. -----------------

    def disarm(self, number: int) -> None:
        self.gh(["pr", "merge", str(number), "--repo", self.repo, "--disable-auto"])

    def comment(self, number: int, body: str) -> None:
        self.gh(["pr", "comment", str(number), "--repo", self.repo, "--body", body])

    def relabel(self, number: int, add: list[str], remove: list[str]) -> None:
        argv = ["pr", "edit", str(number), "--repo", self.repo]
        for label in add:
            argv += ["--add-label", label]
        for label in remove:
            argv += ["--remove-label", label]
        self.gh(argv)

    def update_branch(self, number: int, head_oid: str) -> None:
        self.gh(
            [
                "api", "-X", "PUT",
                f"repos/{self.repo}/pulls/{number}/update-branch",
                "-f", f"expected_head_sha={head_oid}",
            ]
        )

    def rerequest(self, run_id: int) -> None:
        self.gh(["api", "-X", "POST", f"repos/{self.repo}/check-runs/{run_id}/rerequest"])

    # ---- the per-class exits ----------------------------------------------------------

    def park(self, pull: ArmedPull, klass: str, gate: str, detail: str) -> None:
        """Disarm, label for a human, and SAY WHY — in prose AND in a machine receipt."""
        self.disarm(pull.number)
        self.relabel(pull.number, ["needs:user"], [])
        self.comment(pull.number, stuck_comment(pull, klass, gate, detail, self.now()))

    def route_fix(self, pull: ArmedPull, klass: str, gate: str, detail: str) -> None:
        """Disarm and hand the PR to the fix lane.

        `review:pass` is REMOVED with the same call that adds `review:changes`: the arm was
        only ever justified by that attestation, and leaving a stale `review:pass` behind
        would let the re-arm phase above immediately re-arm what this phase just disarmed.
        """
        self.disarm(pull.number)
        self.relabel(pull.number, ["review:changes"], ["review:pass"])
        self.comment(pull.number, stuck_comment(pull, klass, gate, detail, self.now()))

    def rebase(self, pull: ArmedPull, gate: str, detail: str) -> None:
        """Attempt GitHub's own branch update; fall back to the fix lane when it conflicts."""
        try:
            self.update_branch(pull.number, pull.head_oid)
        except GhError as error:
            self.route_fix(
                pull, "conflicting", gate, f"{detail}; branch update refused ({error})"
            )
            return
        self.log(f"[{PROGRAM}] stuck-arm PR #{pull.number}: REBASE — branch update requested")

    def retrigger(self, pull: ArmedPull, run_id: int | None, detail: str) -> None:
        if run_id is None:
            self.log(
                f"[{PROGRAM}] stuck-arm PR #{pull.number}: SKIP — cancelled gate has no "
                "re-requestable run id"
            )
            return
        self.rerequest(run_id)
        self.log(
            f"[{PROGRAM}] stuck-arm PR #{pull.number}: RETRIGGER — re-requested the "
            f"cancelled gate run {run_id} ({detail})"
        )

    def apply(
        self, pull: ArmedPull, klass: str, gate: str, run_id: int | None, detail: str
    ) -> None:
        action = CLASS_ACTIONS[klass]
        if self.dry_run:
            self.log(
                f"[{PROGRAM}] stuck-arm PR #{pull.number}: DRY-RUN would {action} ({detail})"
            )
            return
        if action == ACTION_PARK:
            self.park(pull, klass, gate, detail)
        elif action == ACTION_ROUTE_FIX:
            self.route_fix(pull, klass, gate, detail)
        elif action == ACTION_REBASE:
            self.rebase(pull, gate, detail)
        elif action == ACTION_RETRIGGER:
            self.retrigger(pull, run_id, detail)

    # ---- the sweep --------------------------------------------------------------------

    def run(self) -> int:
        limits = self.limits
        now = self.now()
        armed = self.list_armed()
        self.observed = len(armed)
        self.log(
            f"[{PROGRAM}] stuck-arm: {self.observed} armed PR(s) observed; "
            f"grace={limits.grace_seconds}s stale={limits.stale_seconds}s "
            f"max-actions={limits.max_actions}"
        )
        for item in armed:
            number = item.get("number")
            if not isinstance(number, int):
                continue
            try:
                pull = self.live_pull(number)
                if pull is None:
                    self.log(f"[{PROGRAM}] stuck-arm PR #{number}: SKIP — no live state")
                    self.counts["gate-indeterminate"] = (
                        self.counts.get("gate-indeterminate", 0) + 1
                    )
                    continue
                if pull.base_ref != self.default_branch:
                    # Not our lane; still counted so the totals stay closed.
                    self.counts["progressing"] = self.counts.get("progressing", 0) + 1
                    continue
                gate, run_id = self.gate_state(pull)
            except (GhError, json.JSONDecodeError) as error:
                self.log(f"[{PROGRAM}] stuck-arm PR #{number}: SKIP — read failed ({error})")
                self.counts["gate-indeterminate"] = (
                    self.counts.get("gate-indeterminate", 0) + 1
                )
                self.errors += 1
                continue

            klass = classify(pull, gate, now, limits)
            self.counts[klass] = self.counts.get(klass, 0) + 1
            detail = (
                f"gate={gate} mergeable={pull.mergeable} state={pull.merge_state} "
                f"unresolved-threads={pull.unresolved_threads} "
                f"age={now - max(pull.updated_at, pull.armed_at)}s "
                f"armed-age={now - (pull.armed_at or max(pull.updated_at, pull.armed_at))}s "
                f"head={pull.head_oid[:12]}"
            )
            if klass == "gate-failed":
                # Nothing in the arm path reads CI state — auto-arm.py arms on LABELS alone
                # — so a PR can be armed while red, not merely go red after arming. That is
                # a defect in the ARM policy, not in this PR, and it is reported separately
                # so it can be fixed at its own layer instead of being absorbed here.
                self.armed_on_red.append(pull.number)
            if CLASS_ACTIONS[klass] == ACTION_NONE:
                self.log(f"[{PROGRAM}] stuck-arm PR #{number}: {klass} — no action ({detail})")
                continue
            if self.actions >= limits.max_actions:
                self.deferred += 1
                self.log(
                    f"[{PROGRAM}] stuck-arm PR #{number}: {klass} — DEFERRED "
                    f"(per-tick action cap {limits.max_actions} reached; the next cron "
                    f"tick takes it) ({detail})"
                )
                continue
            self.actions += 1
            try:
                self.apply(pull, klass, gate, run_id, detail)
                self.log(
                    f"[{PROGRAM}] stuck-arm PR #{number}: {klass} -> "
                    f"{CLASS_ACTIONS[klass]} ({detail})"
                )
            except GhError as error:
                self.errors += 1
                self.log(
                    f"[{PROGRAM}] stuck-arm PR #{number}: {klass} -> "
                    f"{CLASS_ACTIONS[klass]} FAILED ({error})"
                )
        self.report()
        return self.errors

    def report(self) -> None:
        """The counted, visible state. The census is the point: per-run success rates
        cannot express a missing edge, but a per-CLASS population with timestamps can."""
        total = sum(self.counts.values())
        breakdown = " ".join(
            f"{name}={self.counts.get(name, 0)}" for name in sorted(CLASS_ACTIONS)
        )
        self.log(f"[{PROGRAM}] stuck-arm census: {breakdown}")
        self.log(
            f"[{PROGRAM}] stuck-arm complete: observed={self.observed} classified={total} "
            f"actions={self.actions} deferred={self.deferred} errors={self.errors} "
            f"enumeration={'TRUNCATED' if self.truncated_enumeration else 'complete'}"
        )
        if self.armed_on_red:
            # Loud, and separate from the per-PR remediation above.
            self.log(
                f"::warning title={PROGRAM} PRs were armed on a RED gate::"
                f"{', '.join('#' + str(n) for n in self.armed_on_red)} — the arm path does "
                "not consult CI state, so this is an arm-policy defect, not a PR defect."
            )


# Sentinel: "the caller did not mention this key at all" (vs. an explicit None, which
# means "the key is present and null"). #1135's fail-closed branch needs both.
_MISSING = object()


def fixture(
    number: int,
    *,
    state: str = "OPEN",
    draft: bool = False,
    base: str = "main",
    labels: tuple[str, ...] = (REVIEW_ATTESTATION,),
    armed: bool = False,
    queued: bool = False,
    # [OPUS-5] #1135 release-PR guard inputs. Defaults describe an ORDINARY worker PR so
    # every pre-existing fixture still re-arms; `_MISSING` distinguishes "omit the key"
    # (the fail-closed case) from "the key is present with this value".
    head_ref: object = _MISSING,
    author_login: object = _MISSING,
    title: object = _MISSING,
) -> dict:
    raw = {
        "number": number,
        "state": state,
        "isDraft": draft,
        "baseRefName": base,
        "labels": [{"name": label} for label in labels],
        "autoMergeRequest": {"enabledAt": "2026-07-21T00:00:00Z"} if armed else None,
        "mergeQueueEntry": {"id": f"MQE_{number}"} if queued else None,
    }
    raw["headRefName"] = (
        f"sparq-agent/issue-{number}-worker" if head_ref is _MISSING else head_ref
    )
    resolved_author = (
        "app/sparq-orchestrator" if author_login is _MISSING else author_login
    )
    raw["author"] = None if resolved_author is None else {"login": resolved_author}
    raw["title"] = (
        f"fix(engine): worker change for #{number}" if title is _MISSING else title
    )
    return raw


CAPABILITY_RESPONSE = {
    "data": {
        "repository": {"autoMergeAllowed": True, "viewerPermission": None},
        "viewer": {"login": "github-actions[bot]"},
    }
}
# The exact text GitHub returned in run 30033315483 — the #3760 regression anchor.
DENIAL_TEXT = (
    "gh pr merge 3454 failed: GraphQL: Resource not accessible by integration "
    "(enablePullRequestAutoMerge)"
)


def capability_payload(auto_merge_allowed: bool | None) -> str:
    payload = copy.deepcopy(CAPABILITY_RESPONSE)
    payload["data"]["repository"]["autoMergeAllowed"] = auto_merge_allowed
    return json.dumps(payload)


def is_capability_query(argv: list[str]) -> bool:
    return argv[:2] == ["api", "graphql"] and any(
        "autoMergeAllowed" in arg for arg in argv
    )


class FakeGh:
    def __init__(
        self,
        snapshots: list[dict],
        live: dict[int, dict],
        *,
        auto_merge_allowed: bool | None = True,
        capability_error: str | None = None,
        arm_errors: dict[int, str] | None = None,
    ) -> None:
        self.snapshots = copy.deepcopy(snapshots)
        self.live = copy.deepcopy(live)
        self.auto_merge_allowed = auto_merge_allowed
        self.capability_error = capability_error
        self.arm_errors = dict(arm_errors or {})
        self.calls: list[list[str]] = []

    def __call__(self, argv: list[str]) -> str:
        self.calls.append(argv)
        if argv[:2] == ["pr", "list"]:
            return json.dumps(self.snapshots)
        if is_capability_query(argv):
            if self.capability_error is not None:
                raise GhError(self.capability_error)
            return capability_payload(self.auto_merge_allowed)
        if argv[:2] == ["api", "graphql"]:
            number = int(
                next(arg.split("=", 1)[1] for arg in argv if arg.startswith("number="))
            )
            pr = self.live.get(number)
            if pr is not None:
                pr = copy.deepcopy(pr)
                pr["labels"] = {
                    "nodes": pr["labels"],
                    "pageInfo": {"hasNextPage": False},
                }
            return json.dumps({"data": {"repository": {"pullRequest": pr}}})
        if argv[:2] == ["pr", "merge"]:
            failure = self.arm_errors.get(int(argv[2]))
            if failure is not None:
                raise GhError(failure)
            return ""
        raise AssertionError(f"unexpected fake gh call: {argv}")


def arm_calls(fake: FakeGh) -> list[list[str]]:
    return [call for call in fake.calls if call[:2] == ["pr", "merge"]]


def probe_query_calls(fake: FakeGh) -> list[list[str]]:
    return [call for call in fake.calls if is_capability_query(call)]


def exercise(
    *prs: dict,
    live_prs: tuple[dict, ...] | None = None,
    max_rearms: int = DEFAULT_MAX_REARMS,
    auto_merge_allowed: bool | None = True,
    capability_error: str | None = None,
    arm_errors: dict[int, str] | None = None,
):
    snapshots = [
        {
            key: copy.deepcopy(value)
            for key, value in pr.items()
            if key != "mergeQueueEntry" and key != "autoMergeRequest"
        }
        for pr in prs
    ]
    current = live_prs if live_prs is not None else prs
    fake = FakeGh(
        snapshots,
        {int(pr["number"]): pr for pr in current},
        auto_merge_allowed=auto_merge_allowed,
        capability_error=capability_error,
        arm_errors=arm_errors,
    )
    messages: list[str] = []
    outcome = RearmSweeper(
        "sparq-org/sparq", "main", max_rearms=max_rearms, gh=fake, log=messages.append
    ).run()
    return fake, messages, outcome


def check_run(name, conclusion="success", status="completed", started="2026-07-27T00:00:00Z",
              ident=1):
    return {"name": name, "status": status, "conclusion": conclusion,
            "started_at": started, "id": ident}


def check_pages(runs, *, declared=None, per_page=100):
    """Wrap check runs the way `gh api --paginate --slurp` returns them.

    `declared` defaults to len(runs) — a COMPLETE read. Passing a larger value models the
    unpaginated read the completeness guard exists for.
    """
    total = len(runs) if declared is None else declared
    pages = [runs[i:i + per_page] for i in range(0, len(runs), per_page)] or [[]]
    return [{"total_count": total, "check_runs": chunk} for chunk in pages]


def armed_pull(number=1, **kw):
    """A classifier input. `armed_at` FOLLOWS `updated_at` unless given explicitly.

    The two clocks are distinct in `classify` (grace reads the latest activity, the stale
    horizon reads the arm), so a fixture that pinned `armed_at` to a constant while
    `updated_at` moved would silently make every fixture ancient-armed and hide exactly the
    starvation this default exists to let the tests express.
    """
    base = dict(
        number=number, is_draft=False, base_ref="main", labels=frozenset(),
        mergeable="MERGEABLE", merge_state="BLOCKED", updated_at=1_000_000,
        armed_at=None, head_oid="0" * 40, has_queue_entry=False,
        unresolved_threads=0, state_truncated=False,
    )
    base.update(kw)
    if base["armed_at"] is None:
        base["armed_at"] = base["updated_at"]
    return ArmedPull(**base)


class FakeStuckGh:
    """A scriptable gh for the stuck-arm sweep. Records every argv so the tests can assert
    on the MUTATIONS actually issued rather than on a log line."""

    def __init__(self, listed, live, pages, *, open_total=None, fail=()):
        self.listed = listed
        self.live = live
        self.pages = pages
        self.open_total = len(listed) if open_total is None else open_total
        self.fail = set(fail)
        self.calls: list[list[str]] = []

    def __call__(self, argv: list[str]) -> str:
        self.calls.append(argv)
        head = tuple(argv[:2])
        if self.fail.intersection(
            {argv[0], " ".join(argv[:2]), " ".join(argv[:3])}
        ):
            raise GhError(f"simulated failure: {' '.join(argv[:3])}")
        if head == ("pr", "list"):
            return json.dumps(self.listed)
        if head == ("api", "graphql"):
            query = next((a for a in argv if a.startswith("query=")), "")
            if "pullRequests(states:OPEN)" in query:
                return json.dumps(
                    {"data": {"repository": {"pullRequests": {"totalCount": self.open_total}}}}
                )
            number = int(next(a.split("=", 1)[1] for a in argv if a.startswith("number=")))
            return json.dumps({"data": {"repository": {"pullRequest": self.live.get(number)}}})
        if head == ("api", "--paginate"):
            return json.dumps(self.pages)
        if head[0] == "api":            # -X PUT / -X POST mutations
            return "{}"
        if head == ("pr", "merge") or head == ("pr", "comment") or head == ("pr", "edit"):
            return ""
        raise AssertionError(f"unexpected fake gh call: {argv}")

    def mutations(self, *prefix):
        return [c for c in self.calls if tuple(c[:len(prefix)]) == prefix]


def live_pr(number, *, draft=False, mergeable="MERGEABLE", state="BLOCKED",
            labels=(), updated="2026-07-27T00:00:00Z", armed="2026-07-27T00:00:00Z",
            queued=False, threads=(), threads_next=False, labels_next=False,
            head="a" * 40):
    return {
        "number": number, "state": "OPEN", "isDraft": draft, "baseRefName": "main",
        "updatedAt": updated, "headRefOid": head, "mergeable": mergeable,
        "mergeStateStatus": state,
        "labels": {"nodes": [{"name": n} for n in labels],
                   "pageInfo": {"hasNextPage": labels_next}},
        "autoMergeRequest": {"enabledAt": armed} if armed else None,
        "mergeQueueEntry": {"id": "MQE"} if queued else None,
        "reviewThreads": {"totalCount": len(threads),
                          "pageInfo": {"hasNextPage": threads_next},
                          "nodes": [{"isResolved": r} for r in threads]},
    }


def stuck_self_test() -> None:
    # ---------------------------------------------------------------------------------
    # (1) GATE RESOLUTION — the three properties resolve_gate exists for.
    # ---------------------------------------------------------------------------------
    # A CANCELLED TWIN must not mask the GREEN NEWEST RUN. This is the live shape on PR
    # #3451: the superseded run stays on the head next to the run that replaced it.
    twin = [
        check_run("gate", "cancelled", started="2026-07-26T02:33:29Z", ident=1),
        check_run("gate", "success", started="2026-07-26T02:45:22Z", ident=2),
    ]
    assert resolve_gate(check_pages(twin), is_draft=False) == GATE_SUCCESS
    # ...and the ordering is not an accident of list order.
    assert resolve_gate(check_pages(list(reversed(twin))), is_draft=False) == GATE_SUCCESS
    # The converse must also hold: a newest CANCELLED behind an older success is cancelled,
    # so "take the best conclusion" cannot pass this pair.
    assert resolve_gate(
        check_pages([
            check_run("gate", "success", started="2026-07-26T02:33:29Z", ident=1),
            check_run("gate", "cancelled", started="2026-07-26T02:45:22Z", ident=2),
        ]),
        is_draft=False,
    ) == GATE_CANCELLED

    # AN UNPAGINATED READ YIELDS `unknown`, NEVER `missing`. 680 runs declared, one page of
    # 100 read, no `gate` row among them — exactly what a default check-runs read returns.
    short = [check_run(f"opt-in leg {i}", ident=i) for i in range(100)]
    assert resolve_gate(check_pages(short, declared=680), is_draft=False) == GATE_UNKNOWN
    # The SAME rows, honestly declared complete, are a real "no gate on this head".
    assert resolve_gate(check_pages(short), is_draft=False) == GATE_MISSING
    assert resolve_gate("not-a-payload", is_draft=False) == GATE_UNKNOWN
    assert resolve_gate([{"total_count": 1}], is_draft=False) == GATE_UNKNOWN

    # THE DRAFT-TIER NAME IS MATCHED DELIBERATELY, NOT BY PREFIX. `gate` is a strict prefix
    # of `gate, draft-tier`, so both directions have to be pinned or a `startswith`
    # implementation passes. A fixture carrying only ONE of the two names cannot detect
    # this, so both fixtures below carry BOTH names with DIFFERENT conclusions.
    both = [
        check_run("gate", "failure", ident=1),
        check_run("gate, draft-tier", "success", ident=2),
    ]
    assert resolve_gate(check_pages(both), is_draft=False) == GATE_FAILURE, (
        "a ready PR must read the exact `gate` row, not the draft-tier row"
    )
    assert resolve_gate(check_pages(both), is_draft=True) == GATE_SUCCESS, (
        "a draft PR must read `gate, draft-tier`"
    )
    # A DRAFT payload carrying ONLY the ready name must be MISSING, not a prefix match.
    assert resolve_gate(check_pages([check_run("gate", "failure")]), is_draft=True) == (
        GATE_MISSING
    )
    # A READY payload carrying ONLY the draft name must be MISSING — the registry #761
    # defect in the mirror. A prefix match would return SUCCESS here.
    assert resolve_gate(
        check_pages([check_run("gate, draft-tier", "success")]), is_draft=False
    ) == GATE_MISSING
    assert gate_check_name(True) == DRAFT_GATE_CHECK_NAME
    assert gate_check_name(False) == GATE_CHECK_NAME
    assert GATE_CHECK_NAME != DRAFT_GATE_CHECK_NAME
    assert DRAFT_GATE_CHECK_NAME.startswith(GATE_CHECK_NAME), (
        "the prefix relation is the whole reason exact matching is load-bearing; if this "
        "ever stops being true, revisit the tripwires above"
    )

    # PENDING and the conclusion map.
    assert resolve_gate(
        check_pages([check_run("gate", None, status="in_progress")]), is_draft=False
    ) == GATE_PENDING
    assert resolve_gate(
        check_pages([check_run("gate", None, status="queued")]), is_draft=False
    ) == GATE_PENDING
    for conclusion, expected in (
        ("success", GATE_SUCCESS), ("neutral", GATE_SUCCESS), ("skipped", GATE_SUCCESS),
        ("failure", GATE_FAILURE), ("timed_out", GATE_FAILURE),
        ("action_required", GATE_FAILURE), ("cancelled", GATE_CANCELLED),
        ("stale", GATE_CANCELLED), ("weird-new-conclusion", GATE_UNKNOWN),
    ):
        assert resolve_gate(
            check_pages([check_run("gate", conclusion)]), is_draft=False
        ) == expected, conclusion

    # A MULTI-PAGE read really is flattened (not just the first page consulted).
    many = [check_run(f"leg {i}", ident=i) for i in range(250)]
    many.append(check_run("gate", "success", ident=999))
    assert resolve_gate(check_pages(many), is_draft=False) == GATE_SUCCESS
    assert len(check_pages(many)) == 3, "fixture must actually span pages"

    # ---------------------------------------------------------------------------------
    # (2) CLASSIFICATION — one fixture per class, EXHAUSTIVE over the enum.
    # ---------------------------------------------------------------------------------
    limits = StuckLimits(grace_seconds=1200, stale_seconds=21600, max_actions=5)
    now = 2_000_000
    fresh = now - 60                 # inside grace
    settled = now - 3600            # past grace, inside the stale horizon
    ancient = now - 90000           # past the stale horizon

    class_fixtures = {
        "queued": (armed_pull(updated_at=settled, has_queue_entry=True), GATE_FAILURE),
        "gate-pending": (armed_pull(updated_at=ancient), GATE_PENDING),
        "gate-indeterminate": (armed_pull(updated_at=ancient), GATE_UNKNOWN),
        "gate-missing": (armed_pull(updated_at=settled), GATE_MISSING),
        "ruleset-grace": (armed_pull(updated_at=fresh, armed_at=fresh), GATE_FAILURE),
        "progressing": (
            armed_pull(updated_at=settled, merge_state="CLEAN"), GATE_SUCCESS),
        "blocked-unexplained": (armed_pull(updated_at=settled), GATE_SUCCESS),
        "held": (
            armed_pull(updated_at=settled, labels=frozenset({"needs:user"})), GATE_SUCCESS),
        "conflicting": (
            armed_pull(updated_at=settled, mergeable="CONFLICTING"), GATE_SUCCESS),
        "gate-failed": (armed_pull(updated_at=settled), GATE_FAILURE),
        "gate-cancelled": (armed_pull(updated_at=settled), GATE_CANCELLED),
        "blocked-threads": (
            armed_pull(updated_at=settled, unresolved_threads=10), GATE_SUCCESS),
        "stale": (armed_pull(updated_at=ancient), GATE_SUCCESS),
    }
    # THE EXHAUSTIVENESS GUARD. A new class with no fixture — the shape that let a
    # misattribution go uncaught here before — reds RIGHT HERE rather than going untested.
    assert set(class_fixtures) == set(CLASS_ACTIONS), (
        "every class must have a routing fixture; missing="
        f"{sorted(set(CLASS_ACTIONS) - set(class_fixtures))} "
        f"extra={sorted(set(class_fixtures) - set(CLASS_ACTIONS))}"
    )
    for expected_class, (pull, gate) in class_fixtures.items():
        got = classify(pull, gate, now, limits)
        assert got == expected_class, (expected_class, got, gate)
        assert got in CLASS_ACTIONS, got

    # THE STALE HORIZON READS THE ARM CLOCK, NOT THE ACTIVITY CLOCK. `updatedAt` is bumped
    # by every label flip and bot comment in this pipeline, so keying the backstop on it
    # lets routine churn reset it forever — the one class that exists to catch everything
    # else would be the easiest of all to starve. MEASURED on #3451: armed 33h ago,
    # `updatedAt` 83 minutes old. Both readings are past grace; only the arm clock is stale.
    churned = armed_pull(armed_at=ancient, updated_at=settled)
    assert now - churned.updated_at < limits.stale_seconds, "fixture must be churn-fresh"
    assert classify(churned, GATE_SUCCESS, now, limits) == "stale", classify(
        churned, GATE_SUCCESS, now, limits
    )
    assert classify(churned, GATE_MISSING, now, limits) == "stale"
    # ...but churn INSIDE the grace window still wins: something is actively happening.
    assert classify(
        armed_pull(armed_at=ancient, updated_at=fresh), GATE_SUCCESS, now, limits
    ) == "ruleset-grace"
    # ...and a RECENTLY-armed PR is never stale however old its other timestamps are.
    assert classify(
        armed_pull(armed_at=settled, updated_at=ancient), GATE_SUCCESS, now, limits
    ) == "blocked-unexplained"
    assert classify(
        armed_pull(armed_at=settled, updated_at=ancient), GATE_MISSING, now, limits
    ) == "gate-missing"

    # A PENDING GATE IS LEFT ALONE ON A MERGEABLE, UNHELD HEAD. This is the constraint that
    # makes the sweep safe to run: `gate` sits in_progress 20-40 min on heavy lanes, and
    # wrongly disarming a healthy PR is worse than leaving a stuck one.
    for extra in ({}, {"unresolved_threads": 10}, {"merge_state": "BLOCKED"},
                  {"merge_state": "UNSTABLE"}, {"labels": frozenset({"review:pass"})}):
        pull = armed_pull(updated_at=ancient, armed_at=ancient, **extra)
        klass = classify(pull, GATE_PENDING, now, limits)
        assert CLASS_ACTIONS[klass] == ACTION_NONE, (extra, klass)
        assert CLASS_ACTIONS[classify(pull, GATE_UNKNOWN, now, limits)] == ACTION_NONE, extra
    # A truncated LABEL/THREAD read is equally fail-open even with a decisive gate, and
    # even for the two classes that otherwise outrank the gate reading.
    for extra in ({}, {"mergeable": "CONFLICTING"}, {"labels": frozenset({"needs:user"})}):
        assert CLASS_ACTIONS[
            classify(armed_pull(updated_at=ancient, armed_at=ancient, state_truncated=True,
                                **extra), GATE_FAILURE, now, limits)
        ] == ACTION_NONE, extra

    # THE TWO EXCEPTIONS TO FAIL-OPEN-ON-PENDING. For these, a pending or unreadable gate
    # is not evidence of progress, so deferring to it is what creates the state with no
    # exit. Both are asserted across EVERY gate reading, including the fail-open ones.
    for gate_reading in (GATE_PENDING, GATE_UNKNOWN, GATE_MISSING, GATE_SUCCESS,
                         GATE_FAILURE, GATE_CANCELLED):
        # A dirty PR cannot merge on ANY gate result: rebase, never wait.
        assert classify(
            armed_pull(updated_at=settled, armed_at=settled, mergeable="CONFLICTING"),
            gate_reading, now, limits,
        ) == "conflicting", gate_reading
        # An armed PR under a hold merges PAST the hold the instant the gate greens, so a
        # pending gate makes disarming MORE urgent, not less. Hold outranks dirty too.
        assert classify(
            armed_pull(updated_at=settled, armed_at=settled, mergeable="CONFLICTING",
                       labels=frozenset({"needs:user"})),
            gate_reading, now, limits,
        ) == "held", gate_reading
    # ...but the grace window still protects both: something just happened, so wait a tick.
    for extra in ({"mergeable": "CONFLICTING"}, {"labels": frozenset({"needs:user"})}):
        assert classify(armed_pull(updated_at=fresh, armed_at=fresh, **extra),
                        GATE_PENDING, now, limits) == "ruleset-grace", extra

    # THE THREE WAYS A HEAD SHOWS "NO USABLE GATE" ARE THREE DIFFERENT CLASSES. Collapsing
    # any pair of them is how a sweeper either disarms a healthy PR or waits forever.
    # (a) DIRTY — CI cannot speak to a merge that does not exist. Rebase, do not wait.
    assert classify(
        armed_pull(updated_at=settled, armed_at=settled, mergeable="CONFLICTING"),
        GATE_MISSING, now, limits,
    ) == "conflicting"
    # (b) SHORT READ — `resolve_gate` must yield UNKNOWN, never MISSING, so the classifier
    #     defers instead of concluding. Driven through resolve_gate, not hand-fed, because
    #     the defect being guarded lives in resolve_gate. Shape MEASURED on PR #4369:
    #     total_count=486, an unpaginated read returns 30 rows and no `gate` row at all.
    short_read = check_pages([check_run(f"leg {i}") for i in range(30)], declared=486)
    assert resolve_gate(short_read, is_draft=False) == GATE_UNKNOWN
    assert classify(
        armed_pull(updated_at=settled, armed_at=settled),
        resolve_gate(short_read, is_draft=False), now, limits,
    ) == "gate-indeterminate"
    # (c) GENUINELY ABSENT on a COMPLETE read of a MERGEABLE head — the real "no gate".
    complete_read = check_pages([check_run(f"leg {i}") for i in range(30)])
    assert resolve_gate(complete_read, is_draft=False) == GATE_MISSING
    assert classify(
        armed_pull(updated_at=settled, armed_at=settled),
        resolve_gate(complete_read, is_draft=False), now, limits,
    ) == "gate-missing"
    # The three land in three DIFFERENT classes with three different actions.
    assert len({"conflicting", "gate-indeterminate", "gate-missing"}) == 3
    assert CLASS_ACTIONS["conflicting"] == ACTION_REBASE
    assert CLASS_ACTIONS["gate-indeterminate"] == ACTION_NONE
    assert CLASS_ACTIONS["gate-missing"] == ACTION_NONE
    # An unreadable activity timestamp must NOT read as "infinitely old".
    assert classify(
        armed_pull(updated_at=0, armed_at=0), GATE_SUCCESS, now, limits
    ) == "gate-indeterminate"

    # NOTHING TERMINAL FIRES INSIDE THE GRACE WINDOW.
    for gate in (GATE_FAILURE, GATE_CANCELLED, GATE_MISSING, GATE_SUCCESS):
        for extra in ({}, {"mergeable": "CONFLICTING"},
                      {"labels": frozenset({"needs:user"})}, {"unresolved_threads": 3}):
            pull = armed_pull(updated_at=fresh, armed_at=fresh, **extra)
            klass = classify(pull, gate, now, limits)
            assert klass == "ruleset-grace", (gate, extra, klass)
    # The grace window is measured from the LATER of the two activity stamps, so a stale
    # `updatedAt` with a fresh arm is still in grace.
    assert classify(
        armed_pull(updated_at=ancient, armed_at=fresh), GATE_FAILURE, now, limits
    ) == "ruleset-grace"

    # PRECEDENCE: a CONFLICTING PR with a red gate rebases (the red was computed against a
    # merge that no longer exists) rather than being disarmed on that stale result.
    assert classify(
        armed_pull(updated_at=settled, mergeable="CONFLICTING"), GATE_FAILURE, now, limits
    ) == "conflicting"
    # ...but a HOLD outranks even that, because a live arm under a hold is the unsafe state.
    assert classify(
        armed_pull(updated_at=settled, mergeable="CONFLICTING",
                   labels=frozenset({"needs:user"})),
        GATE_FAILURE, now, limits,
    ) == "held"
    # mergeable=UNKNOWN is GitHub still computing; it must never be read as CONFLICTING.
    assert classify(
        armed_pull(updated_at=settled, mergeable="UNKNOWN", merge_state="CLEAN"),
        GATE_SUCCESS, now, limits,
    ) == "progressing"
    for label in ("needs:user", "needs:security", "review:changes", "review:needs",
                  "trust:untrusted"):
        assert hold_labels(frozenset({label})) == [label], label
    assert hold_labels(frozenset({"review:pass", "area:sparq-core", "trust-surface"})) == []

    # ---------------------------------------------------------------------------------
    # (3) ROUTING — every terminal class issues the MUTATIONS its action names, and every
    #     no-action class issues NONE. Asserted on the recorded argv, not on a log line.
    # ---------------------------------------------------------------------------------
    def sweep(live_rows, runs, *, limits_=None, open_total=None, fail=(), now_=now):
        listed = [{"number": n, "autoMergeRequest": {"enabledAt": "x"}} for n in live_rows]
        fake = FakeStuckGh(listed, live_rows, check_pages(runs),
                           open_total=open_total, fail=fail)
        messages: list[str] = []
        sweeper = StuckArmSweeper(
            "sparq-org/sparq", "main", limits=limits_ or limits, gh=fake,
            log=messages.append, now=lambda: now_,
        )
        errors = sweeper.run()
        return fake, messages, errors, sweeper

    old = "2026-07-26T00:00:00Z"          # far outside the grace window at `now_` below
    clock = _iso_epoch("2026-07-27T00:00:00Z")

    # gate-failed -> disarm + review:changes + DROP review:pass + comment.
    fake, messages, errors, sweeper = sweep(
        {10: live_pr(10, labels=("review:pass",), updated=old, armed=old)},
        [check_run("gate", "failure")], now_=clock,
    )
    assert errors == 0, messages
    assert sweeper.counts == {"gate-failed": 1}, sweeper.counts
    assert fake.mutations("pr", "merge"), fake.calls
    assert "--disable-auto" in fake.mutations("pr", "merge")[0]
    edit = fake.mutations("pr", "edit")[0]
    assert "--add-label" in edit and "review:changes" in edit, edit
    assert "--remove-label" in edit and "review:pass" in edit, (
        "leaving review:pass on a disarmed PR lets the re-arm phase immediately re-arm it"
    )
    body = fake.mutations("pr", "comment")[0][-1]
    assert body.startswith(STUCK_MARKER), body[:40]
    assert "gate-failed" in body, body
    # ...and arming on red is reported SEPARATELY as an arm-policy defect.
    assert sweeper.armed_on_red == [10], sweeper.armed_on_red
    assert any("armed on a RED gate" in line for line in messages), messages

    # blocked-threads -> disarm + needs:user + a comment that says why. Never forced green.
    fake, messages, errors, sweeper = sweep(
        {11: live_pr(11, updated=old, armed=old, threads=(False, False, True))},
        [check_run("gate", "success")], now_=clock,
    )
    assert sweeper.counts == {"blocked-threads": 1}, sweeper.counts
    assert "--disable-auto" in fake.mutations("pr", "merge")[0]
    assert "needs:user" in fake.mutations("pr", "edit")[0]
    body = fake.mutations("pr", "comment")[0][-1]
    assert "blocked-threads" in body and "unresolved" in body.lower(), body
    # A park NEVER removes the attestation or resolves anything on the PR's behalf.
    assert "--remove-label" not in fake.mutations("pr", "edit")[0]

    # conflicting -> attempt GitHub's branch update FIRST, and no disarm when it works.
    fake, messages, errors, sweeper = sweep(
        {12: live_pr(12, mergeable="CONFLICTING", updated=old, armed=old)},
        [check_run("gate", "success")], now_=clock,
    )
    assert sweeper.counts == {"conflicting": 1}, sweeper.counts
    update = [c for c in fake.calls if c[:3] == ["api", "-X", "PUT"]]
    assert update and "update-branch" in update[0][3], fake.calls
    assert not fake.mutations("pr", "merge"), "a rebasable PR must stay armed"
    # ...and when the update is REFUSED (a real conflict), it falls back to the fix lane.
    fake, messages, errors, sweeper = sweep(
        {12: live_pr(12, mergeable="CONFLICTING", labels=("review:pass",), updated=old,
                     armed=old)},
        [check_run("gate", "success")], fail=("api -X PUT",), now_=clock,
    )
    assert sweeper.counts == {"conflicting": 1}, sweeper.counts
    assert "--disable-auto" in fake.mutations("pr", "merge")[0], fake.calls
    assert "review:changes" in fake.mutations("pr", "edit")[0]
    assert "branch update refused" in fake.mutations("pr", "comment")[0][-1]

    # gate-cancelled -> re-request the NEWEST gate run, bounded; nothing is disarmed.
    fake, messages, errors, sweeper = sweep(
        {13: live_pr(13, updated=old, armed=old)},
        [check_run("gate", "success", started="2026-07-20T00:00:00Z", ident=7),
         check_run("gate", "cancelled", started="2026-07-21T00:00:00Z", ident=8)],
        now_=clock,
    )
    assert sweeper.counts == {"gate-cancelled": 1}, sweeper.counts
    rerun = [c for c in fake.calls if c[:3] == ["api", "-X", "POST"]]
    assert rerun and "/check-runs/8/rerequest" in rerun[0][3], (
        f"must re-request the NEWEST cancelled run, got {rerun}"
    )
    assert not fake.mutations("pr", "merge"), "a cancelled gate is not a failed gate"

    # A DRAFT armed PR resolves its own tier. With only `gate, draft-tier` present a
    # ready-keyed lookup would say MISSING and (past the horizon) park a healthy draft.
    fake, messages, errors, sweeper = sweep(
        {14: live_pr(14, draft=True, state="CLEAN", updated=old, armed=old)},
        [check_run("gate, draft-tier", "success")], now_=clock,
    )
    assert sweeper.counts == {"progressing": 1}, sweeper.counts
    assert not fake.mutations("pr", "merge"), fake.calls

    # Every NO-ACTION class really issues no mutation at all.
    for rows, runs in (
        ({20: live_pr(20, queued=True, updated=old, armed=old)},
         [check_run("gate", "failure")]),
        ({21: live_pr(21, updated=old, armed=old)},
         [check_run("gate", None, status="in_progress")]),
        # gate-missing INSIDE the stale horizon (age 2h): a complete read with no gate row
        # yet is not evidence of anything, so nothing fires.
        ({22: live_pr(22, updated="2026-07-26T22:00:00Z", armed="2026-07-26T22:00:00Z")},
         [check_run("opt-in leg", "success")]),
        ({23: live_pr(23, updated="2026-07-27T00:00:00Z", armed="2026-07-27T00:00:00Z")},
         [check_run("gate", "failure")]),
    ):
        fake, messages, errors, sweeper = sweep(rows, runs, now_=clock)
        assert errors == 0, messages
        for verb in (("pr", "merge"), ("pr", "edit"), ("pr", "comment")):
            assert not fake.mutations(*verb), (rows, verb, fake.calls)

    # ---------------------------------------------------------------------------------
    # (4) THE COUNTS SUM TO THE ARMED POPULATION, and the per-tick bound holds.
    # ---------------------------------------------------------------------------------
    population = {
        30: live_pr(30, labels=("review:pass",), updated=old, armed=old),      # gate-failed
        31: live_pr(31, queued=True, updated=old, armed=old),                  # queued
        32: live_pr(32, updated="2026-07-27T00:00:00Z",
                    armed="2026-07-27T00:00:00Z"),                             # grace
        33: live_pr(33, labels=("needs:user",), updated=old, armed=old),       # held
        34: live_pr(34, mergeable="CONFLICTING", updated=old, armed=old),      # conflicting
    }
    fake, messages, errors, sweeper = sweep(
        population, [check_run("gate", "failure")], now_=clock
    )
    assert errors == 0, messages
    assert sweeper.observed == 5, sweeper.observed
    assert sum(sweeper.counts.values()) == sweeper.observed, (
        f"class counts {sweeper.counts} must partition the {sweeper.observed} armed PRs"
    )
    assert sweeper.counts == {"gate-failed": 1, "queued": 1, "ruleset-grace": 1,
                              "held": 1, "conflicting": 1}, sweeper.counts
    census = next(line for line in messages if "stuck-arm census" in line)
    for name in CLASS_ACTIONS:
        assert f"{name}=" in census, (name, census)
    assert "classified=5" in " ".join(messages), messages

    # THE PER-TICK BOUND. Three actionable PRs, a cap of one: exactly one mutation fires,
    # the other two are still CLASSIFIED and COUNTED, and they say they were deferred.
    backlog = {40 + i: live_pr(40 + i, labels=("review:pass",), updated=old, armed=old)
               for i in range(3)}
    fake, messages, errors, sweeper = sweep(
        backlog, [check_run("gate", "failure")],
        limits_=StuckLimits(grace_seconds=1200, stale_seconds=21600, max_actions=1),
        now_=clock,
    )
    assert sweeper.actions == 1, sweeper.actions
    assert sweeper.deferred == 2, sweeper.deferred
    assert len(fake.mutations("pr", "merge")) == 1, fake.calls
    assert sum(sweeper.counts.values()) == 3, sweeper.counts
    assert sum("DEFERRED" in line for line in messages) == 2, messages

    # A NO-ACTION CLASS MUST NOT CONSUME THE ACTION BUDGET. Without the early return in
    # `run`, `apply` is still a no-op for ACTION_NONE — so the mutation surface looks
    # identical and only the BUDGET tells the two apart. A tick full of queued/pending PRs
    # would silently starve the one PR that actually needed repairing.
    starved = {70: live_pr(70, queued=True, updated=old, armed=old),
               71: live_pr(71, queued=True, updated=old, armed=old),
               72: live_pr(72, labels=("review:pass",), updated=old, armed=old)}
    fake, messages, errors, sweeper = sweep(
        starved, [check_run("gate", "failure")],
        limits_=StuckLimits(grace_seconds=1200, stale_seconds=21600, max_actions=1),
        now_=clock,
    )
    assert sweeper.counts == {"queued": 2, "gate-failed": 1}, sweeper.counts
    assert sweeper.actions == 1, sweeper.actions
    assert sweeper.deferred == 0, (
        "two no-action PRs ahead of the actionable one must not have been 'deferred' — "
        "that would mean they consumed the per-tick budget"
    )
    assert len(fake.mutations("pr", "merge")) == 1, fake.calls
    assert "72" in fake.mutations("pr", "merge")[0], (
        "the budget must have been spent on the ACTIONABLE PR, not on a queued one"
    )

    # A cancelled gate with NO re-requestable run id must SKIP, not call the API with None.
    fake, messages, errors, sweeper = sweep(
        {73: live_pr(73, updated=old, armed=old)},
        [check_run("gate", "cancelled", ident=None)], now_=clock,
    )
    assert sweeper.counts == {"gate-cancelled": 1}, sweeper.counts
    assert not [c for c in fake.calls if c[:3] == ["api", "-X", "POST"]], fake.calls
    assert any("no re-requestable run id" in line for line in messages), messages
    assert errors == 0, messages

    # A SHORT ENUMERATION is flagged, not silently reported as a full census.
    fake, messages, errors, sweeper = sweep(
        {50: live_pr(50, updated=old, armed=old)}, [check_run("gate", None,
                                                              status="in_progress")],
        open_total=999, now_=clock,
    )
    assert sweeper.truncated_enumeration is True
    assert any("enumeration truncated" in line for line in messages), messages
    assert any("enumeration=TRUNCATED" in line for line in messages), messages

    # A per-PR read failure is an ERROR and a counted class, never a silent drop.
    fake, messages, errors, sweeper = sweep(
        {60: live_pr(60, updated=old, armed=old)}, [check_run("gate", "failure")],
        fail=("api --paginate --slurp",), now_=clock,
    )
    assert errors == 1, messages
    assert sweeper.counts == {"gate-indeterminate": 1}, sweeper.counts
    assert not fake.mutations("pr", "merge"), fake.calls

    # A failed MUTATION is recorded as an error rather than reported as a clean sweep.
    fake, messages, errors, sweeper = sweep(
        {61: live_pr(61, labels=("review:pass",), updated=old, armed=old)},
        [check_run("gate", "failure")], fail=("pr merge",), now_=clock,
    )
    assert errors == 1, messages
    assert any("FAILED" in line for line in messages), messages

    # DRY-RUN MUTATES NOTHING — asserted over EVERY terminal class, not one sample, so a
    # class added later without a dry-run guard reds here.
    dry_fixtures = {
        "gate-failed": (live_pr(80, labels=("review:pass",), updated=old, armed=old),
                        [check_run("gate", "failure")]),
        "held": (live_pr(81, labels=("needs:user",), updated=old, armed=old),
                 [check_run("gate", "success")]),
        "conflicting": (live_pr(82, mergeable="CONFLICTING", updated=old, armed=old),
                        [check_run("gate", "success")]),
        "gate-cancelled": (live_pr(83, updated=old, armed=old),
                           [check_run("gate", "cancelled")]),
        "blocked-threads": (live_pr(84, updated=old, armed=old, threads=(False,)),
                            [check_run("gate", "success")]),
        # no `gate` row at all on a COMPLETE read, past the stale horizon.
        "stale": (live_pr(85, updated=old, armed=old),
                  [check_run("some-other-leg", "success")]),
    }
    assert set(dry_fixtures) == TERMINAL_CLASSES, (
        f"every terminal class needs a dry-run fixture; missing="
        f"{sorted(TERMINAL_CLASSES - set(dry_fixtures))}"
    )
    for expected, (row, runs) in dry_fixtures.items():
        number = row["number"]
        listed = [{"number": number, "autoMergeRequest": {"enabledAt": "x"}}]
        fake = FakeStuckGh(listed, {number: row}, check_pages(runs))
        messages = []
        sweeper = StuckArmSweeper(
            "sparq-org/sparq", "main", limits=limits, gh=fake, log=messages.append,
            now=lambda: clock, dry_run=True,
        )
        assert sweeper.run() == 0, messages
        assert sweeper.counts == {expected: 1}, (expected, sweeper.counts)
        for verb in (("pr", "merge"), ("pr", "edit"), ("pr", "comment")):
            assert not fake.mutations(*verb), (expected, verb, fake.calls)
        assert not [c for c in fake.calls if c[:2] == ["api", "-X"]], (expected, fake.calls)
        assert any("DRY-RUN would" in line for line in messages), (expected, messages)

    # ---------------------------------------------------------------------------------
    # (5) THE ENUM IS CLOSED. Both directions, so neither a new class nor a new action can
    #     appear without a decision being made about it here.
    # ---------------------------------------------------------------------------------
    assert set(CLASS_ACTIONS.values()) <= {
        ACTION_NONE, ACTION_PARK, ACTION_ROUTE_FIX, ACTION_REBASE, ACTION_RETRIGGER
    }, CLASS_ACTIONS
    assert TERMINAL_CLASSES == {
        "held", "conflicting", "gate-failed", "gate-cancelled", "blocked-threads", "stale"
    }, TERMINAL_CLASSES
    assert set(CLASS_REASONS) <= set(CLASS_ACTIONS), set(CLASS_REASONS)
    # Every PARK/ROUTE_FIX class must have a written reason — a park with no reason is the
    # "park silently" failure this whole phase exists to remove.
    for name, action in CLASS_ACTIONS.items():
        if action in (ACTION_PARK, ACTION_ROUTE_FIX):
            assert name in CLASS_REASONS, name
            assert len(CLASS_REASONS[name]) > 40, name

    # ---------------------------------------------------------------------------------
    # (6) THE MACHINE-READABLE RECEIPT. A park whose only artefact is prose has a
    #     human-only exit; registry #766 measured four PRs terminal for exactly that
    #     reason. Every assertion below is about the MACHINE half.
    # ---------------------------------------------------------------------------------
    # Every terminal class carries a named un-park condition, and every condition named is
    # one `unpark_satisfied` can actually evaluate. Both directions: a condition constant
    # that nothing maps to is as broken as a class with no condition.
    assert set(UNPARK_CONDITIONS) == TERMINAL_CLASSES, UNPARK_CONDITIONS
    known_conditions = {
        UNPARK_HOLDS_CLEARED, UNPARK_NOT_CONFLICTING, UNPARK_GATE_GREEN,
        UNPARK_GATE_CONCLUDED, UNPARK_THREADS_RESOLVED, UNPARK_HEAD_MOVED,
    }
    assert set(UNPARK_CONDITIONS.values()) == known_conditions, UNPARK_CONDITIONS
    # The assertion above is one belt; the IMPORT-TIME assert is the other, and it is the
    # one that matters in production, where nothing runs this self-test. A belt nobody
    # tests is decorative — measured: deleting the import-time assert left this whole
    # suite green — so prove it fires by importing a copy of this module with one class's
    # machine exit removed. Hermetic: source text + exec, no subprocess and no network.
    with open(__file__, encoding="utf-8") as handle:
        module_source = handle.read()
    maimed = module_source.replace(
        f'    "blocked-threads": UNPARK_THREADS_RESOLVED,\n', "", 1
    )
    assert maimed != module_source, "the totality-assert probe stopped matching its target"
    # @dataclass resolves annotations through sys.modules[__name__], so the probe has to be
    # a real (temporary) module rather than a bare dict namespace. Its output is SWALLOWED:
    # re-executing the module top level also re-runs the gh_retry degraded-import fallback,
    # and #3776's "exactly ONE ::warning per run" pin is a real invariant this probe must
    # not perturb.
    import io
    import types
    from contextlib import redirect_stderr, redirect_stdout

    probe = types.ModuleType("_rearm_sweeper_totality_probe")
    sys.modules[probe.__name__] = probe
    try:
        with redirect_stdout(io.StringIO()), redirect_stderr(io.StringIO()):
            exec(compile(maimed, "<rearm-sweeper-no-machine-exit>", "exec"), probe.__dict__)
    except AssertionError as error:
        assert "needs a named un-park condition" in str(error), error
    else:
        raise AssertionError(
            "a TERMINAL class with no un-park condition imported cleanly — the import-time "
            "totality assert is not firing, so a park could ship with a human-only exit"
        )
    finally:
        sys.modules.pop(probe.__name__, None)

    # ROUND TRIP. The receipt survives being embedded in the prose comment a human reads.
    parked_pull = armed_pull(
        3451, head_oid="1f" * 20, unresolved_threads=10, merge_state="BLOCKED"
    )
    body = stuck_comment(parked_pull, "blocked-threads", GATE_SUCCESS, "detail", clock)
    recovered = parse_stuck_receipt(body)
    assert recovered == stuck_receipt(parked_pull, "blocked-threads", GATE_SUCCESS, clock), (
        recovered
    )
    assert recovered["unpark_when"] == UNPARK_THREADS_RESOLVED, recovered
    assert recovered["head"] == "1f" * 20, recovered
    assert recovered["observed"]["unresolved_threads"] == 10, recovered
    # The receipt renders as NOTHING to a reader: it lives inside an HTML comment.
    assert body.rstrip().endswith(RECEIPT_CLOSE), body[-80:]
    assert RECEIPT_OPEN in body, body

    # THE PARK PATH ACTUALLY EMITS IT. Asserted on the posted argv, not on the helper —
    # a receipt function nothing calls is the vacuous version of this whole section.
    fake, messages, errors, sweeper = sweep(
        {30: live_pr(30, updated=old, armed=old, threads=(False,), head="1f" * 20)},
        [check_run("gate", "success")], now_=clock,
    )
    assert sweeper.counts == {"blocked-threads": 1}, sweeper.counts
    posted = parse_stuck_receipt(fake.mutations("pr", "comment")[0][-1])
    assert posted is not None, fake.mutations("pr", "comment")
    assert posted["pr"] == 30 and posted["class"] == "blocked-threads", posted
    assert posted["unpark_when"] == UNPARK_THREADS_RESOLVED, posted
    assert posted["head"] == "1f" * 20, posted
    # ...and so does the ROUTE-FIX path, which is a park in every respect that matters here.
    fake, messages, errors, sweeper = sweep(
        {31: live_pr(31, labels=("review:pass",), updated=old, armed=old)},
        [check_run("gate", "failure")], now_=clock,
    )
    routed = parse_stuck_receipt(fake.mutations("pr", "comment")[0][-1])
    assert routed is not None and routed["class"] == "gate-failed", routed
    assert routed["unpark_when"] == UNPARK_GATE_GREEN, routed

    # THE CONDITION IS FALSE AT PARK TIME. If it were already satisfied by the very state
    # that caused the park, the receipt would name an exit that is not an exit.
    for klass, pull_ in (
        ("blocked-threads", armed_pull(1, unresolved_threads=3)),
        ("gate-failed", armed_pull(1)),
        ("conflicting", armed_pull(1, mergeable="CONFLICTING")),
        ("held", armed_pull(1, labels=frozenset({"needs:user"}))),
        ("stale", armed_pull(1, head_oid="ab" * 20)),
    ):
        gate_at_park = GATE_FAILURE if klass == "gate-failed" else GATE_SUCCESS
        rec = stuck_receipt(pull_, klass, gate_at_park, clock)
        assert not unpark_satisfied(rec, pull_, gate_at_park), (klass, rec)

    # ...AND TRUE ONLY ON REAL RECOVERY. One recovered observation per condition.
    recoveries = (
        ("blocked-threads", armed_pull(1, unresolved_threads=3),
         armed_pull(1, unresolved_threads=0), GATE_SUCCESS),
        ("gate-failed", armed_pull(1), armed_pull(1), GATE_SUCCESS),
        ("conflicting", armed_pull(1, mergeable="CONFLICTING"),
         armed_pull(1, mergeable="MERGEABLE"), GATE_SUCCESS),
        ("held", armed_pull(1, labels=frozenset({"needs:user"})),
         armed_pull(1, labels=frozenset({"review:pass"})), GATE_SUCCESS),
        ("stale", armed_pull(1, head_oid="ab" * 20),
         armed_pull(1, head_oid="cd" * 20), GATE_SUCCESS),
    )
    for klass, before, after, gate_after in recoveries:
        gate_at_park = GATE_FAILURE if klass == "gate-failed" else GATE_SUCCESS
        rec = stuck_receipt(before, klass, gate_at_park, clock)
        assert unpark_satisfied(rec, after, gate_after), (klass, rec)

    # FAIL-CLOSED, one axis at a time. Each of these is a state in which un-parking would
    # re-admit a PR whose cause still holds, so each must stay parked.
    healthy = armed_pull(1, unresolved_threads=0)
    good = stuck_receipt(armed_pull(1, unresolved_threads=4), "blocked-threads",
                         GATE_SUCCESS, clock)
    assert unpark_satisfied(good, healthy, GATE_SUCCESS), good      # the control
    for label, mutated_receipt, observed_pull in (
        ("version from another revision", {**good, "v": RECEIPT_VERSION + 1}, healthy),
        ("receipt from another program", {**good, "program": "groom"}, healthy),
        ("receipt from another phase", {**good, "phase": "rearm"}, healthy),
        ("receipt copied from another PR", {**good, "pr": 999}, healthy),
        ("condition this revision cannot evaluate",
         {**good, "unpark_when": "vibes-improved"}, healthy),
        ("condition field absent", {k: v for k, v in good.items() if k != "unpark_when"},
         healthy),
        ("not a receipt at all", "gate looks fine now", healthy),
        ("truncated live read", good, armed_pull(1, unresolved_threads=0,
                                                 state_truncated=True)),
    ):
        assert not unpark_satisfied(mutated_receipt, observed_pull, GATE_SUCCESS), label
    # A gate that was never read cannot prove a gate-green recovery.
    red = stuck_receipt(armed_pull(1), "gate-failed", GATE_FAILURE, clock)
    for gate_now in (GATE_UNKNOWN, GATE_MISSING, GATE_PENDING, GATE_CANCELLED, GATE_FAILURE):
        assert not unpark_satisfied(red, armed_pull(1), gate_now), gate_now
    assert unpark_satisfied(red, armed_pull(1), GATE_SUCCESS)
    # mergeable=UNKNOWN is GitHub still computing — never proof the conflict is gone.
    dirty = stuck_receipt(armed_pull(1, mergeable="CONFLICTING"), "conflicting",
                          GATE_SUCCESS, clock)
    assert not unpark_satisfied(dirty, armed_pull(1, mergeable="UNKNOWN"), GATE_SUCCESS)
    # head-moved needs a head to compare against, and an unreadable one proves nothing.
    moved = stuck_receipt(armed_pull(1, head_oid="ab" * 20), "stale", GATE_SUCCESS, clock)
    assert not unpark_satisfied(moved, armed_pull(1, head_oid=""), GATE_SUCCESS)
    assert not unpark_satisfied({**moved, "head": None}, armed_pull(1, head_oid="cd" * 20),
                                GATE_SUCCESS)

    # PARSING. Malformed input yields None (opaque), never a dict that reads as satisfied.
    for bad in (None, 42, "", "no receipt here", f"{RECEIPT_OPEN} not json {RECEIPT_CLOSE}",
                f"{RECEIPT_OPEN} {{\"v\":1}} no-terminator", f"{RECEIPT_OPEN} [1,2] {RECEIPT_CLOSE}"):
        assert parse_stuck_receipt(bad) is None, bad
    # Several receipts on one body: the LAST (most recent observation) wins.
    first = render_receipt(stuck_receipt(armed_pull(7), "stale", GATE_SUCCESS, clock))
    second = render_receipt(
        stuck_receipt(armed_pull(7, unresolved_threads=2), "blocked-threads",
                      GATE_SUCCESS, clock + 1)
    )
    assert parse_stuck_receipt(f"{first}\n\n{second}")["class"] == "blocked-threads"

    # [OPUS-5] #5041. PROMISE vs IMPLEMENTATION. A park that ADVERTISES an exit it does not
    # perform turns a transient cause into a permanent stall while reading as self-healing:
    # #4847 carried "Un-parks automatically when: `gate-green`" while `unpark_satisfied` had
    # no caller at all. The two must ship together, so this is an IFF in both directions —
    # re-adding the claim without wiring the evaluator fails, and wiring the evaluator
    # without telling anyone also fails.
    rendered = stuck_comment(armed_pull(1), "stale", GATE_SUCCESS, "detail", clock)
    promised = AUTO_UNPARK_PROMISE in rendered
    wired = unpark_production_callers()
    assert promised == bool(wired), (
        "the automatic-un-park PROMISE and its IMPLEMENTATION must ship together: "
        f"promise_rendered={promised} production_callers={sorted(wired) or 'NONE'}. "
        "Either wire `unpark_satisfied` into the sweep, or do not claim the park clears "
        "itself (see #5041)."
    )

    print("rearm-sweeper stuck-arm self-test: PASS")


def self_test() -> None:
    expected_exact_exclusions = {
        "review:changes",
        "review:needs",
        "review:needs-user",
        "trust:untrusted",
    }
    assert EXCLUDED_LABELS == expected_exact_exclusions, EXCLUDED_LABELS

    # Mutation tripwire (a): autoMergeRequest is null while a queue entry is live.
    queued = fixture(3678, queued=True)
    fake, messages, outcome = exercise(fixture(3678), live_prs=(queued,))
    assert outcome.exit_code == 0, messages
    assert not arm_calls(fake), fake.calls
    assert any("SKIP" in line and "mergeQueueEntry" in line for line in messages), (
        messages
    )
    query_call = next(
        call
        for call in fake.calls
        if call[:2] == ["api", "graphql"] and not is_capability_query(call)
    )
    query_text = next(arg for arg in query_call if arg.startswith("query="))
    assert "autoMergeRequest{" in query_text, query_text
    assert "mergeQueueEntry{" in query_text, query_text

    # Mutation tripwire (b): needs:* is a hard exclusion, independent of queue state.
    held = fixture(3682, labels=(REVIEW_ATTESTATION, "needs:user"))
    fake, messages, outcome = exercise(fixture(3682), live_prs=(held,))
    assert outcome.exit_code == 0, messages
    assert not arm_calls(fake), fake.calls
    assert any("SKIP" in line and "needs:user" in line for line in messages), messages

    # Mutation tripwire (c): a reviewed PR with neither live field must be re-armed.
    dropped = fixture(3675)
    fake, messages, outcome = exercise(dropped)
    assert outcome.exit_code == 0, messages
    assert outcome.armed == 1, outcome
    assert len(arm_calls(fake)) == 1, fake.calls
    assert arm_calls(fake)[0] == [
        "pr",
        "merge",
        "3675",
        "--repo",
        "sparq-org/sparq",
        "--auto",
    ], arm_calls(fake)
    assert any("ARMED" in line for line in messages), messages

    # ---------------------------------------------------------------- #1135 Release PR
    # THE GAP THIS CLOSES: this sweep's exclusions were LABEL-keyed only, so the Release
    # PR carrying review:pass and no hold label was re-armed. Merging it tags + (once
    # `publish = true`) publishes 37 crates to crates.io, irreversibly.
    #
    # Tripwire (c) directly above PROVES a `review:pass` PR IS re-armed by this harness,
    # so "no arm call" below is a discriminating outcome and not the fixture's default.
    release_kwargs = {
        "head_ref": "release-plz-main",
        "author_login": "app/github-actions",
        "title": "chore: release v0.2.0",
    }
    release_pr = fixture(900, **release_kwargs)
    fake, messages, outcome = exercise(release_pr)
    assert not arm_calls(fake), fake.calls
    assert any("release-pr-guard" in line for line in messages), messages

    # THE EXACT ATTACK: relabelling the Release PR — adding review:pass, stripping every
    # hold label — cannot make it re-armable, because labels are not an input to the rule.
    for labels in ((REVIEW_ATTESTATION,), (REVIEW_ATTESTATION, "area:ci"), ()):
        fake, messages, outcome = exercise(fixture(901, labels=labels, **release_kwargs))
        assert not arm_calls(fake), (labels, fake.calls)

    # Each signal alone suffices (branch / author / title are OR-ed, not AND-ed).
    for label, kwargs in (
        ("branch only", {"head_ref": "release-plz-main"}),
        ("author only", {"author_login": "release-plz[bot]"}),
        ("title only", {"title": "chore: release v0.2.0"}),
    ):
        fake, messages, outcome = exercise(fixture(902, **kwargs))
        assert not arm_calls(fake), (label, fake.calls)
        assert any("release-pr-guard" in line for line in messages), (label, messages)

    # FAIL-CLOSED: an unknown head branch must REFUSE, never admit.
    for label, kwargs in (
        ("headRefName null", {"head_ref": None}),
        ("headRefName empty", {"head_ref": ""}),
    ):
        fake, messages, outcome = exercise(fixture(903, **kwargs))
        assert not arm_calls(fake), (label, fake.calls)
        assert any("indeterminate head branch" in line for line in messages), (
            label,
            messages,
        )

    # The guard runs on the LIVE state too, not only the list snapshot: an ordinary PR
    # retargeted onto the release branch between the two reads is not re-armed.
    fake, messages, outcome = exercise(fixture(904), live_prs=(fixture(904, **release_kwargs),))
    assert not arm_calls(fake), fake.calls
    assert any("release-pr-guard" in line for line in messages), messages

    # Both queries must actually REQUEST the guard's three inputs.
    for field in ("headRefName", "author", "title"):
        assert field in PR_LIST_FIELDS, (field, PR_LIST_FIELDS)
    for field in ("headRefName", "author{login}", "title"):
        assert field in LIVE_QUERY, (field, LIVE_QUERY)

    # CI state alone is never an arm verdict: live removal of review:pass must stop it.
    unattested = fixture(105, labels=())
    fake, messages, outcome = exercise(fixture(105), live_prs=(unattested,))
    assert outcome.exit_code == 0, messages
    assert not arm_calls(fake), fake.calls
    assert any("review:pass attestation absent" in line for line in messages), messages
    list_call = next(call for call in fake.calls if call[:2] == ["pr", "list"])
    label_arg = list_call.index("--label")
    assert list_call[label_arg + 1] == REVIEW_ATTESTATION, list_call

    # Every named exclusion is fail-closed, including one added after enumeration.
    for label in (*sorted(EXCLUDED_LABELS), "needs:security"):
        held = fixture(100, labels=(REVIEW_ATTESTATION, label))
        fake, messages, _ = exercise(
            fixture(100),
            live_prs=(held,),
        )
        assert not arm_calls(fake), (label, fake.calls)
        assert any("hard exclusion" in line for line in messages), (label, messages)
    for pr, expected in (
        (fixture(101, armed=True), "autoMergeRequest"),
        (fixture(102, state="CLOSED", base="", queued=True), "not-open"),
        (fixture(103, draft=True), "draft"),
        (fixture(104, base="stacked-base"), "non-default-base"),
    ):
        fake, messages, _ = exercise(
            fixture(pr["number"]),
            live_prs=(pr,),
        )
        assert not arm_calls(fake), fake.calls
        assert any(expected in line for line in messages), messages

    # The hard bound limits commands, while every excess candidate still gets a decision.
    fake, messages, outcome = exercise(fixture(201), fixture(202), max_rearms=1)
    assert outcome.exit_code == 0, messages
    assert len(arm_calls(fake)) == 1, fake.calls
    assert any("PR #202: SKIP" in line and "limit" in line for line in messages), (
        messages
    )

    # [FABLE-5] #3759: READS route through gh_read (and really are reads — the retry
    # helper's fail-closed guard accepts them); the arm MUTATION stays on the
    # one-shot runner, so a retry wrapper can never double-fire it.
    dropped = fixture(3759)
    fake = FakeGh(
        [{k: v for k, v in dropped.items() if k not in ("mergeQueueEntry", "autoMergeRequest")}],
        {3759: dropped},
    )
    read_calls: list[list[str]] = []

    def spy_read(argv: list[str]) -> str:
        read_calls.append(argv)
        gh_retry.assert_read_only(argv)
        return fake(argv)

    RearmSweeper(
        "sparq-org/sparq", "main", gh=fake, gh_read=spy_read, log=lambda _m: None
    ).run()
    # [OPUS-5] #3760: the capability probe is itself an idempotent READ, so it routes
    # through gh_read (the retrying runner) and precedes the enumeration.
    assert [c[:2] for c in read_calls] == [
        ["api", "graphql"],
        ["pr", "list"],
        ["api", "graphql"],
    ], read_calls
    assert is_capability_query(read_calls[0]), read_calls[0]
    assert [c[:2] for c in arm_calls(fake)] == [["pr", "merge"]], fake.calls
    assert all(c[:2] != ["pr", "merge"] for c in read_calls), read_calls

    # [FABLE-5] #3759 finding 5: an arm MUTATION failure must be recorded as a real
    # error even when the follow-up race-diagnostic READ (live_state) exhausts its
    # transient retries. If the diagnostic's GhTransientExhausted escaped run(), main's
    # lenient handler would turn a genuinely failed arm into a false success. Here the
    # arm raises GhError; the post-arm live_state raises GhTransientExhausted.
    dropped = fixture(4001)
    snapshot = {
        k: v for k, v in dropped.items() if k not in ("mergeQueueEntry", "autoMergeRequest")
    }

    class _ArmFailsDiagExhausts:
        def __init__(self) -> None:
            self.armed = False
            self.calls: list[list[str]] = []

        def __call__(self, argv: list[str]) -> str:
            self.calls.append(argv)
            if argv[:2] == ["pr", "list"]:
                return json.dumps([snapshot])
            if is_capability_query(argv):
                return capability_payload(True)
            if argv[:2] == ["api", "graphql"]:
                if self.armed:
                    # The post-arm diagnostic read exhausts transient retries.
                    raise gh_retry.GhTransientExhausted(
                        "gh api graphql: HTTP 504 (3 attempts)"
                    )
                pr = copy.deepcopy(dropped)
                pr["labels"] = {
                    "nodes": pr["labels"],
                    "pageInfo": {"hasNextPage": False},
                }
                return json.dumps({"data": {"repository": {"pullRequest": pr}}})
            if argv[:2] == ["pr", "merge"]:
                self.armed = True
                raise GhError("simulated arm failure")
            raise AssertionError(f"unexpected fake gh call: {argv}")

    fake_ad = _ArmFailsDiagExhausts()
    messages = []
    outcome = RearmSweeper(
        "sparq-org/sparq", "main", gh=fake_ad, gh_read=fake_ad, log=messages.append
    ).run()
    assert outcome.exit_code == 1, (outcome, messages)
    assert len(outcome.arm_failures) == 1, outcome
    assert any("re-arm failed" in line for line in messages), messages
    # The diagnostic exhaustion did NOT escape run() (no exception propagated here).

    # [FABLE-5] #3759 finding 5: EVENT-mode exhausted transient on the enumeration read
    # exits NON-zero (no per-PR backstop); SWEEP-mode is the lenient ::warning + 0.
    import io
    from contextlib import redirect_stderr, redirect_stdout

    def _exhausted_read(_argv: list[str]) -> str:
        raise gh_retry.GhTransientExhausted("gh pr list: HTTP 504 (3 attempts)")

    original_run_gh_read = globals()["run_gh_read"]

    class _ModeHarness:
        """Drive main() with a stubbed enumeration read that exhausts transients."""

        def __init__(self, mode: str) -> None:
            self.mode = mode

        def __enter__(self):
            globals()["run_gh_read"] = _exhausted_read
            self._argv = sys.argv
            sys.argv = [
                "rearm-sweeper.py", "--repo", "sparq-org/sparq", "--mode", self.mode,
            ]
            return self

        def __exit__(self, *exc):
            globals()["run_gh_read"] = original_run_gh_read
            sys.argv = self._argv
            return False

    out, err = io.StringIO(), io.StringIO()
    with _ModeHarness("event"), redirect_stdout(out), redirect_stderr(err):
        event_code = main()
    assert event_code == 1, event_code
    assert "event-mode" in err.getvalue(), err.getvalue()

    out, err = io.StringIO(), io.StringIO()
    with _ModeHarness("sweep"), redirect_stdout(out), redirect_stderr(err):
        sweep_code = main()
    assert sweep_code == 0, sweep_code
    assert "::warning" in out.getvalue(), out.getvalue()

    # ---------------------------------------------------------------------------------
    # [OPUS-5] #3760 — ARM CAPABILITY + PER-PR FAILURE ISOLATION.
    # ---------------------------------------------------------------------------------

    # The live #3760 error text must classify as a CAPABILITY denial, and near-misses
    # must NOT — a race/CAS/transient stays per-PR, and the denial set must stay disjoint
    # from #3759's transient set (a denial must never be swallowed as ::warning + 0).
    assert is_arm_denial(DENIAL_TEXT), DENIAL_TEXT
    assert is_arm_denial(DENIAL_TEXT.upper()), "denial matching must be case-insensitive"
    for benign in (
        "gh pr merge 1 failed: GraphQL: Head branch was modified. Review and try again",
        "gh pr merge 1 failed: HTTP 502: 502 Bad Gateway",
        "gh pr merge 1 failed: HTTP 504: Gateway Timeout",
        "gh pr merge 1 failed: Pull request is in unstable status",
    ):
        assert not is_arm_denial(benign), benign

    # (1) PROBE — can-arm: the sweep proceeds, the verdict is logged, exit 0, and the
    # probe really runs BEFORE any arm so a broken token never touches a PR.
    fake, messages, outcome = exercise(fixture(3001))
    assert outcome.capability == CAN_ARM, outcome
    assert outcome.exit_code == 0, messages
    assert len(arm_calls(fake)) == 1, fake.calls
    assert any("arm-capability probe: can-arm" in line for line in messages), messages
    assert len(probe_query_calls(fake)) == 1, fake.calls
    assert fake.calls.index(probe_query_calls(fake)[0]) < min(
        index for index, call in enumerate(fake.calls) if call[:2] == ["pr", "merge"]
    ), fake.calls

    # (2) PROBE — cannot-arm (repository setting OFF): ONE ::error naming the exact
    # setting, ZERO PRs touched (not even enumerated), exit 1.
    fake, messages, outcome = exercise(fixture(3002), auto_merge_allowed=False)
    assert outcome.capability == CANNOT_ARM, outcome
    assert outcome.exit_code == 1, messages
    assert not arm_calls(fake), fake.calls
    assert not [call for call in fake.calls if call[:2] == ["pr", "list"]], fake.calls
    emitted = [line for line in messages if line.startswith("::error")]
    assert len(emitted) == 1, emitted
    assert "Allow auto-merge" in emitted[0], emitted
    assert "contents: write" in emitted[0], emitted
    assert "ORCHESTRATOR_APP_ID" in emitted[0], emitted

    # (3) PROBE — cannot-arm (the token itself is denied): same single loud error.
    fake, messages, outcome = exercise(
        fixture(3003),
        capability_error="gh api graphql failed: Resource not accessible by integration",
    )
    assert outcome.capability == CANNOT_ARM, outcome
    assert outcome.exit_code == 1, messages
    assert not arm_calls(fake), fake.calls
    assert len([line for line in messages if line.startswith("::error")]) == 1, messages

    # (4) PROBE — inconclusive must NEVER red the run or stop the sweep: a transient 502,
    # a repository that does not report the field, and an exhausted #3759 retry all pass.
    for kwargs in (
        {"capability_error": "gh api graphql failed: HTTP 502: 502 Bad Gateway"},
        {"auto_merge_allowed": None},
    ):
        fake, messages, outcome = exercise(fixture(3004), **kwargs)
        assert outcome.capability == INCONCLUSIVE, (kwargs, outcome)
        assert outcome.exit_code == 0, (kwargs, messages)
        assert len(arm_calls(fake)) == 1, (kwargs, fake.calls)
        assert not [line for line in messages if line.startswith("::error")], messages

    exhausting = FakeGh([], {})

    def _probe_exhausts(argv: list[str]) -> str:
        if is_capability_query(argv):
            raise gh_retry.GhTransientExhausted("gh api graphql: HTTP 504 (3 attempts)")
        return exhausting(argv)

    probe_verdict = RearmSweeper(
        "sparq-org/sparq", "main", gh=exhausting, gh_read=_probe_exhausts,
        log=lambda _line: None,
    ).probe_arm_capability()
    assert probe_verdict.status == INCONCLUSIVE, probe_verdict
    assert "transient" in probe_verdict.detail, probe_verdict

    # (5) PER-PR ARM FAILURE — a non-capability failure on the FIRST PR must not abort
    # the sweep: the second PR is still armed, the failure is summarised, exit is 1.
    fake, messages, outcome = exercise(
        fixture(3011),
        fixture(3012),
        arm_errors={3011: "gh pr merge 3011 failed: Pull request is in unstable status"},
    )
    assert [call[2] for call in arm_calls(fake)] == ["3011", "3012"], fake.calls
    assert outcome.armed == 1, outcome
    assert [number for number, _ in outcome.arm_failures] == [3011], outcome
    assert outcome.exit_code == 1, messages
    assert any("PR #3011: ARM-FAILED" in line for line in messages), messages
    assert any("PR #3012: ARMED" in line for line in messages), messages
    assert any("arm-failure summary: PR #3011" in line for line in messages), messages
    assert any(
        "arm-failures=1" in line and "armed=1" in line for line in messages
    ), messages
    # A per-PR failure is NOT a capability failure, so it emits no ::error.
    assert not [line for line in messages if line.startswith("::error")], messages

    # (6) MID-SWEEP CAPABILITY DENIAL — the #3760 error on the first PR must stop the
    # sweep after ONE ::error (never one per PR) and still exit non-zero.
    fake, messages, outcome = exercise(
        fixture(3021),
        fixture(3022),
        fixture(3023),
        arm_errors={number: DENIAL_TEXT for number in (3021, 3022, 3023)},
    )
    assert [call[2] for call in arm_calls(fake)] == ["3021"], fake.calls
    assert outcome.capability == CANNOT_ARM, outcome
    assert outcome.armed == 0, outcome
    assert [number for number, _ in outcome.arm_failures] == [3021], outcome
    assert outcome.exit_code == 1, messages
    emitted = [line for line in messages if line.startswith("::error")]
    assert len(emitted) == 1, emitted
    assert "contents: write" in emitted[0], emitted
    assert sum("arm capability lost" in line for line in messages) == 2, messages

    # (7) EXIT SEMANTICS — an arm failure must never exit 0, and the pre-#3760
    # live-state `errors` contract still reds the run.
    assert SweepOutcome().exit_code == 0
    assert SweepOutcome(arm_failures=[(1, "x")]).exit_code == 1
    assert SweepOutcome(state_failures=[(1, "x")]).exit_code == 1
    assert SweepOutcome(capability=CANNOT_ARM).exit_code == 1
    assert SweepOutcome(capability=INCONCLUSIVE).exit_code == 0

    # (8) The standalone probe entry point must agree with the in-sweep verdict, in both
    # directions, and a cannot-arm probe must exit non-zero exactly once.
    blocked = RearmSweeper(
        "sparq-org/sparq",
        "main",
        gh=FakeGh([], {}, auto_merge_allowed=False),
        log=lambda _line: None,
    )
    assert blocked.probe_arm_capability().status == CANNOT_ARM
    assert probe_arm_capability_exit(blocked) == 1
    healthy = RearmSweeper(
        "sparq-org/sparq", "main", gh=FakeGh([], {}), log=lambda _line: None
    )
    assert healthy.probe_arm_capability().status == CAN_ARM
    assert probe_arm_capability_exit(healthy) == 0

    # A capability failure is NOT a transient: --mode sweep must still exit 1 (it must
    # never be converted into the #3759 ::warning + exit-0 missed cycle).
    class _CannotArmHarness:
        def __enter__(self):
            self._argv = sys.argv
            globals()["run_gh_read"] = lambda argv: capability_payload(False)
            sys.argv = ["rearm-sweeper.py", "--repo", "sparq-org/sparq", "--mode", "sweep"]
            return self

        def __exit__(self, *exc):
            globals()["run_gh_read"] = original_run_gh_read
            sys.argv = self._argv
            return False

    out, err = io.StringIO(), io.StringIO()
    with _CannotArmHarness(), redirect_stdout(out), redirect_stderr(err):
        cannot_arm_code = main()
    assert cannot_arm_code == 1, cannot_arm_code
    assert "::error" in out.getvalue(), out.getvalue()
    assert "::warning" not in out.getvalue(), out.getvalue()

    # ---------------------------------------------------------------------------------
    # [OPUS-5] #3766 — STICKY FAILURE PRECEDENCE:
    #     collected-failure > transient-exhaustion > clean.
    # The false pass this fixes (reproduced by the cross-provider review): PR #3011's arm
    # FAILED and was collected, then PR #3012's live-state read exhausted its bounded
    # retries; that exhaustion unwound run() and main()'s lenient handler reported
    # ::warning + exit 0 — discarding a real arm failure. The collected verdict must be
    # untouchable by anything that happens for a LATER candidate.
    # ---------------------------------------------------------------------------------

    def strip_live(pr: dict) -> dict:
        return {
            key: value
            for key, value in pr.items()
            if key not in ("mergeQueueEntry", "autoMergeRequest")
        }

    def exhausting_read(base: FakeGh, number: int) -> Callable[[list[str]], str]:
        """A gh_read whose live-state query for ONE candidate exhausts its retries."""

        def read(argv: list[str]) -> str:
            if (
                argv[:2] == ["api", "graphql"]
                and not is_capability_query(argv)
                and f"number={number}" in argv
            ):
                raise gh_retry.GhTransientExhausted(
                    "gh api graphql: HTTP 504 (3 attempts)"
                )
            return base(argv)

        return read

    def sequence(*, arm_fails: bool):
        """PR #3011 (optionally arm-failing), then PR #3012 whose live read exhausts."""
        first_pr, second_pr = fixture(3011), fixture(3012)
        base = FakeGh(
            [strip_live(first_pr), strip_live(second_pr)],
            {3011: first_pr, 3012: second_pr},
            arm_errors=(
                {3011: "gh pr merge 3011 failed: Pull request is in unstable status"}
                if arm_fails
                else None
            ),
        )
        lines: list[str] = []
        sweeper = RearmSweeper(
            "sparq-org/sparq",
            "main",
            gh=base,
            gh_read=exhausting_read(base, 3012),
            log=lines.append,
        )
        # run() must RETURN here: an ESCAPING exhaustion is exactly the #3766 false pass.
        return lines, sweeper.run()

    # (9) THE #3766 SEQUENCE — arm failure on A, exhausted transient on a LATER B.
    lines, outcome = sequence(arm_fails=True)
    assert [number for number, _ in outcome.arm_failures] == [3011], outcome
    assert [number for number, _ in outcome.transient_exhaustions] == [3012], outcome
    assert outcome.hard_failed and outcome.exit_code == 1, outcome
    for mode in ("sweep", "event"):
        assert sweep_exit(outcome, mode, lines.append, lines.append) == 1, (mode, lines)
    assert any("PR #3011: ARM-FAILED" in line for line in lines), lines
    assert any("arm-failure summary: PR #3011" in line for line in lines), lines
    assert any(
        "transient-exhausted" in line and "3012" in line for line in lines
    ), lines
    assert any("precedence: collected-failure" in line for line in lines), lines
    # The lenient ::warning must NOT be emitted while a real failure stands.
    assert not [line for line in lines if line.startswith("::warning")], lines

    # (10) CONTROL — the lenient policy is intact where it IS correct: a sweep whose ONLY
    # problem is an exhausted transient still exits 0 with exactly one ::warning, and the
    # candidates around it are still armed.
    lines, outcome = sequence(arm_fails=False)
    assert outcome.armed == 1 and not outcome.hard_failed, outcome
    assert [number for number, _ in outcome.transient_exhaustions] == [3012], outcome
    warned: list[str] = []
    assert sweep_exit(outcome, "sweep", warned.append, warned.append) == 0, warned
    assert len(warned) == 1 and warned[0].startswith("::warning"), warned
    assert "3012" in warned[0], warned
    assert any("PR #3011: ARMED" in line for line in lines), lines

    # (11) CONTROL — event mode is unchanged: no per-PR cron backstop, so the same
    # transient-only outcome fails loudly on the stderr channel.
    loud: list[str] = []
    assert sweep_exit(outcome, "event", loud.append, loud.append) == 1, loud
    assert any("event-mode" in line for line in loud), loud

    # (12) The precedence rule itself, at the seam.
    assert SweepOutcome().transient_detail is None
    assert SweepOutcome(sweep_transient="HTTP 504").transient_detail == "HTTP 504"
    assert (
        SweepOutcome(transient_exhaustions=[(1, "HTTP 504")]).transient_detail
        == "PR #1: HTTP 504"
    )
    assert SweepOutcome(transient_exhaustions=[(1, "HTTP 504")]).exit_code == 0
    dominated = SweepOutcome(
        arm_failures=[(1, "unstable")], transient_exhaustions=[(2, "HTTP 504")]
    )
    assert dominated.hard_failed and dominated.exit_code == 1, dominated
    for mode in ("sweep", "event"):
        assert sweep_exit(dominated, mode, lambda _l: None, lambda _l: None) == 1, mode

    # (13) FAIL CLOSED — an exhausted transient raised by the ARM MUTATION itself leaves the
    # arm's outcome UNKNOWN, so it must be a per-PR failure (exit 1), never a lenient
    # warning-class missed cycle. The one-shot runner cannot raise this today; the handler
    # exists so a future refactor cannot open a second false-pass route.
    only = fixture(3031)
    mutation_base = FakeGh([strip_live(only)], {3031: only})

    def gh_arm_exhausts(argv: list[str]) -> str:
        if argv[:2] == ["pr", "merge"]:
            raise gh_retry.GhTransientExhausted("gh pr merge: HTTP 504 (3 attempts)")
        return mutation_base(argv)

    lines = []
    outcome = RearmSweeper(
        "sparq-org/sparq", "main", gh=gh_arm_exhausts, gh_read=mutation_base,
        log=lines.append,
    ).run()
    assert [number for number, _ in outcome.arm_failures] == [3031], outcome
    assert not outcome.transient_exhaustions, outcome
    assert sweep_exit(outcome, "sweep", lines.append, lines.append) == 1, lines
    assert not [line for line in lines if line.startswith("::warning")], lines
    assert any(
        "transient exhaustion on the arm mutation" in line for line in lines
    ), lines

    # (14) END TO END through main() — the precedence must hold for the real entry point,
    # not only for the seam the assertions above call directly.
    original_run_gh = globals()["run_gh"]

    class _StickyPrecedenceHarness:
        """A collected arm failure on PR #3011 must dominate PR #3012's exhaustion."""

        def __enter__(self):
            first_pr, second_pr = fixture(3011), fixture(3012)
            fake = FakeGh(
                [strip_live(first_pr), strip_live(second_pr)],
                {3011: first_pr, 3012: second_pr},
                arm_errors={
                    3011: "gh pr merge 3011 failed: Pull request is in unstable status"
                },
            )
            self._argv = sys.argv
            globals()["run_gh"] = fake
            globals()["run_gh_read"] = exhausting_read(fake, 3012)
            sys.argv = [
                "rearm-sweeper.py", "--repo", "sparq-org/sparq", "--mode", "sweep",
            ]
            return self

        def __exit__(self, *exc):
            globals()["run_gh"] = original_run_gh
            globals()["run_gh_read"] = original_run_gh_read
            sys.argv = self._argv
            return False

    out, err = io.StringIO(), io.StringIO()
    with _StickyPrecedenceHarness(), redirect_stdout(out), redirect_stderr(err):
        sticky_code = main()
    assert sticky_code == 1, (sticky_code, out.getvalue(), err.getvalue())
    assert "arm-failure summary: PR #3011" in out.getvalue(), out.getvalue()
    assert "::warning" not in out.getvalue(), out.getvalue()

    stuck_self_test()

    print("rearm-sweeper self-test: PASS")


def sweep_exit(
    outcome: SweepOutcome,
    mode: str,
    log: Callable[[str], None] = print,
    fail: Callable[[str], None] | None = None,
) -> int:
    """Decide the run's exit status from the FINAL collected state (#3766).

    PRECEDENCE — ``collected-failure > transient-exhaustion > clean``:

    1. A COLLECTED arm/state/capability failure DOMINATES, in either mode. Nothing that
       happens while processing a LATER candidate — an exhausted transient above all — can
       downgrade a verdict an EARLIER candidate already earned. This check is reached from
       the accumulated state, never short-circuited by an exception.
    2. TRANSIENT EXHAUSTION alone (no collected failure anywhere) keeps #3759's intended
       lenient policy: ``sweep`` reports ``::warning`` + exit 0, because a missed cycle is
       harmless and the next cron run re-covers it. ``event`` has no per-PR backstop, so it
       fails loudly instead.
    3. CLEAN is 0.
    """
    if fail is None:

        def fail(message: str) -> None:
            print(message, file=sys.stderr)

    if outcome.hard_failed:
        if outcome.transient_detail:
            log(
                f"[{PROGRAM}] a transient exhaustion also occurred "
                f"({outcome.transient_detail}) but a collected failure outranks it "
                "(precedence: collected-failure > transient-exhaustion > clean): exit 1"
            )
        return outcome.exit_code
    if outcome.transient_detail:
        # [FABLE-5] #3759: only the periodic + idempotent SWEEP may swallow a missed cycle
        # on a transient platform 5xx (the cron backstop covers it). An EVENT-driven
        # invocation has no per-PR backstop, so it fails loudly (finding 5).
        if mode == "event":
            fail(
                f"[{PROGRAM}] fatal: event-mode exhausted transient retries: "
                f"{outcome.transient_detail}"
            )
            return 1
        log(
            f"::warning title={PROGRAM} skipped a cycle on transient GitHub API "
            f"failures::{outcome.transient_detail} — bounded retries exhausted; the next "
            "cron run covers this sweep, so this run reports success."
        )
        return 0
    return 0


def probe_arm_capability_exit(sweeper: RearmSweeper) -> int:
    """Run ONLY the startup probe: 0 = armable (or inconclusive), 1 = provably not.

    Wired as its own workflow step so the job reds at second three with one actionable
    ::error instead of burning a sweep and reporting a per-PR SKIP every ten minutes.
    """
    verdict = sweeper.probe_arm_capability()
    sweeper.log(f"[{PROGRAM}] arm-capability probe: {verdict.status} — {verdict.detail}")
    if verdict.blocks_sweep:
        sweeper.fail_capability(f"startup arm-capability probe failed: {verdict.detail}")
        return 1
    return 0


def stuck_arm_exit(args) -> int:
    """Run the stuck-arm phase and map its outcome onto this lane's exit contract.

    Same policy as `sweep_exit`: a collected per-PR failure is REAL and reds the run, while
    an exhausted transient on the periodic sweep is a missed cycle the next cron covers.
    Nothing here can turn a recorded failure into a success — the error count is read off
    the sweeper AFTER the exception handler, never discarded by it (#3766).
    """
    sweeper = StuckArmSweeper(
        args.repo,
        args.default_branch,
        limits=StuckLimits(max_actions=args.max_stuck_actions),
        gh=run_gh,
        gh_read=run_gh_read,
        dry_run=args.dry_run,
    )
    transient: str | None = None
    try:
        sweeper.run()
    except gh_retry.GhTransientExhausted as error:
        transient = str(error)
    except (GhError, ValueError, json.JSONDecodeError) as error:
        print(f"[{PROGRAM}] stuck-arm fatal: {error}", file=sys.stderr)
        return 1
    if sweeper.errors:
        print(
            f"::error title={PROGRAM} stuck-arm recorded {sweeper.errors} failure(s)::"
            "see the per-PR lines above"
        )
        return 1
    if transient is not None:
        if args.mode == "event":
            print(
                f"[{PROGRAM}] fatal: event-mode exhausted transient retries: {transient}",
                file=sys.stderr,
            )
            return 1
        print(
            f"::warning title={PROGRAM} stuck-arm skipped a cycle on transient GitHub API "
            f"failures::{transient} — the next cron run covers this sweep."
        )
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repo", help="OWNER/REPOSITORY to sweep")
    parser.add_argument("--default-branch", default="main")
    parser.add_argument("--max-rearms", type=int, default=DEFAULT_MAX_REARMS)
    parser.add_argument(
        "--mode",
        choices=("sweep", "event"),
        default="sweep",
        help=(
            "sweep: periodic cron (exhausted transient READS on the enumeration path "
            "=> ::warning + exit 0, the cron backstop covers a missed cycle). event: a "
            "single-PR event-driven invocation with no per-PR backstop — an exhausted "
            "transient exits non-zero so the run is visibly red. This workflow runs "
            "sweep-only today; the flag keeps the exit contract explicit and testable."
        ),
    )
    parser.add_argument("--self-test", action="store_true")
    parser.add_argument(
        "--probe-arm-capability",
        action="store_true",
        help="only verify the token can enable auto-merge here; arm nothing",
    )
    parser.add_argument(
        "--phase",
        choices=("rearm", "stuck-arm"),
        default="rearm",
        help=(
            "rearm: restore auto-merge arms GitHub dropped (the original sweep). "
            "stuck-arm: classify the ARMED population and give every armed-but-"
            "unmergeable class a visible, counted terminal exit (#4548). The two are "
            "duals over one decision surface and share this lane's cron, concurrency "
            "group and token; `rearm` skips exactly the PRs `stuck-arm` owns."
        ),
    )
    parser.add_argument(
        "--dry-run",
        action="store_true",
        help="stuck-arm only: classify and report, mutate nothing",
    )
    parser.add_argument(
        "--max-stuck-actions",
        type=int,
        default=StuckLimits().max_actions,
        help="stuck-arm only: hard cap on PRs MUTATED per tick (congestion bound)",
    )
    args = parser.parse_args()

    if args.self_test:
        self_test()
        return 0
    if not args.repo:
        parser.error("--repo is required unless --self-test is used")
    if args.phase == "stuck-arm":
        return stuck_arm_exit(args)
    sweeper: RearmSweeper | None = None
    try:
        sweeper = RearmSweeper(
            args.repo, args.default_branch, max_rearms=args.max_rearms,
            gh=run_gh, gh_read=run_gh_read,
        )
        if args.probe_arm_capability:
            return probe_arm_capability_exit(sweeper)
        outcome = sweeper.run()
    except gh_retry.GhTransientExhausted as error:
        # [OPUS-5] #3766 STICKY BACKSTOP. run() already records a per-candidate exhaustion
        # without unwinding; this is defence in depth for any read that is ever added
        # OUTSIDE that guarded loop. The outcome is read back OFF THE SWEEPER, so everything
        # already collected still reaches sweep_exit below — an exception can no longer
        # short-circuit past the accumulated-failure check and report a false success.
        outcome = sweeper.outcome if sweeper is not None else SweepOutcome()
        outcome.sweep_transient = outcome.sweep_transient or str(error)
    except (GhError, ValueError, json.JSONDecodeError) as error:
        print(f"[{PROGRAM}] fatal: {error}", file=sys.stderr)
        return 1
    # The exit is computed HERE, at the end, from the final collected state:
    # collected-failure > transient-exhaustion > clean.
    return sweep_exit(outcome, args.mode)


if __name__ == "__main__":
    sys.exit(main())
