#!/usr/bin/env python3
"""Arm reviewed pull requests through GitHub's merge queue.

The policy is deliberately fail-closed. It refreshes every candidate immediately before
arming, rechecks all human/security holds, and supplies the refreshed head OID as a CAS
to GitHub's explicit ``enablePullRequestAutoMerge`` GraphQL mutation.
"""

# [GPT-5] Issue #447 — deterministic, hermetically self-tested auto-arm policy.
# [FABLE-5] Issue #3759 / registry#563 item 4 — idempotent gh READS (pr list/view) go
# through gh_retry.run_gh_read (bounded, transient-only retry); MUTATIONS (pr ready,
# the enablePullRequestAutoMerge CAS) stay one-shot on the un-wrapped runner. When a
# transient 5xx survives every bounded retry, the sweep exits ::warning + 0 instead of
# redding main's gate — a missed cycle is harmless, the cron covers it. Any
# non-transient error still fails loudly (GhFatalError -> GhError -> traceback).
# [OPUS-5] Issue #3760 — a startup arm-capability probe that fails LOUDLY ONCE, and per-PR
#   arm failures that are COLLECTED instead of aborting every later candidate. Before this
#   change a single failing mutation re-raised out of process() -> run() -> run_sweep(), so
#   every PR after it was never even considered. #3759 finding 5's invariant is preserved
#   and strengthened, not weakened: a failed arm still can NEVER read as success — it now
#   exits non-zero via ArmOutcome.exit_code instead of by unwinding the sweep. Kept
#   deliberately self-contained (auto-arm.yml sparse-checks out only this file, so it
#   cannot import shared helpers); scripts/tests/test_arm_capability_wiring.py pins the
#   load-bearing strings against scripts/rearm-sweeper.py so the copies cannot drift.
# [OPUS-5] Issue #3766 — STICKY FAILURE PRECEDENCE. Collecting the failures (above) was
#   only half the job: the collected state lived in a LOCAL of run(), so a
#   GhTransientExhausted raised while processing a LATER candidate unwound past the
#   accumulated-failure check and run_sweep's lenient handler reported ::warning + exit 0.
#   Reproduced: PR #451's arm failed, PR #452's pre-arm refresh exhausted its retries, and
#   the run exited 0 — discarding #451's real failure. Now the outcome lives on the ARMER
#   (AutoArmer.outcome), a later candidate's exhaustion is recorded as its OWN
#   warning-class per-candidate outcome without escaping the loop, and the exit status is
#   computed at the END by arm_exit_code() from that final state.
#   PRECEDENCE: collected-failure > transient-exhaustion > clean.
# [OPUS-5] Issue #3776, second half — AN EVENT-TRIGGERED RUN JUDGES THE PR THAT TRIGGERED IT.
#   `arm reviewed PRs` is a GATING check, and run() swept EVERY open review:pass PR in BOTH
#   modes. Combined with #3766's (correct) sticky exit, ONE permanently un-armable PR redded
#   the gating check on every OTHER PR's label event. Live blast radius, measured: sparq had
#   exactly two review:pass PRs — #3434 and #2521 — and run 30144214363 reported
#   `considered=2 armed=0 arm-failures=1` because #3434 cannot be armed at all until the
#   `workflows: write` question (#3777) is decided. One PR taxed the entire queue.
#   In `--mode event` the run is now scoped to the triggering PR (--pr, or derived from the
#   runner environment — see resolve_scope_pr). This is a CORRECTNESS fix, not a policy
#   change: judging unrelated PRs on someone else's label event is wrong independently of
#   whether the check gates. It NARROWS which PRs a run is ACCOUNTABLE for; it does not
#   weaken accountability — within its own scope the #3766 precedence
#   (collected-failure > transient-exhaustion > clean) is untouched, and the triggering PR's
#   own arm failure is still a hard non-zero exit that no later transient can discard.
#   `--mode sweep` (the cron) keeps whole-repo scope and #3759's lenient transient policy.
#   DELIBERATELY NOT CHANGED HERE: whether `arm reviewed PRs` should be a GATING check at
#   all. That is a policy decision about what may block a merge, not a correctness fix — the
#   argument and the costed options live in #3777.

from __future__ import annotations

import argparse
import copy
import json
import os
import re
import subprocess
import sys
from dataclasses import dataclass, field
from typing import Callable, Mapping


# --------------------------------------------------------------------------------------
# [OPUS-5] Issue #3776 — A MISSING RESILIENCE HELPER MUST NEVER BRICK THE GATING ARM CHECK.
#
# THE OUTAGE. `arm reviewed PRs` is a GATING check and auto-arm.yml runs on `pull_request`,
# so GitHub resolves the WORKFLOW FILE from the PR's own ref while that workflow's checkout
# step resolves the SCRIPT from `ref: default_branch`. #3766 (6137b7b3, merged
# 2026-07-25T01:38:35Z) added `import gh_retry` here AND added `scripts/gh_retry.py` to
# auto-arm.yml's file-by-file `sparse-checkout` list in the SAME commit — atomic on main,
# not atomic across refs. Every PR ref whose workflow snapshot predates 01:38Z still
# enumerates `scripts/auto-arm.py` alone, so the runner pairs the NEW default-branch script
# with a checkout that never materialises gh_retry.py:
#     File "/home/runner/work/sparq/sparq/scripts/auto-arm.py", line 46, in <module>
#       import gh_retry
#   ModuleNotFoundError: No module named 'gh_retry'
# (sparq #3434, run 30143852994, 2026-07-25T04:20Z — branch cut 2026-07-18.) ci-summary
# fail-fasts on a failing GATING check, so no reviewed PR on a stale ref could merge.
#
# WHY THE FIX MUST LIVE HERE, NOT IN THE WORKFLOW. The stale PR refs cannot be rewritten,
# so the manifest those runs use is already frozen: editing auto-arm.yml's sparse list heals
# only refs snapshotted AFTER the edit. The only code that reaches an already-stale PR is
# code fetched from the DEFAULT BRANCH — this file. Hence the guard here; auto-arm.yml's
# switch to whole-directory sparse-checkout is the forward-looking half, not the remedy.
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
        "::warning title=auto-arm running WITHOUT transient-retry (scripts/gh_retry.py "
        f"not checked out)::{_gh_retry_missing} — this run's idempotent gh READS are "
        "ONE-SHOT: a GitHub 5xx blip will red it instead of being retried. Arming itself "
        "is UNAFFECTED and proceeds. CAUSE: this script came from the default branch "
        "while the `sparse-checkout` manifest came from an older workflow snapshot on the "
        "PR ref, which does not list scripts/gh_retry.py (sparq #3776 / #3766). REMEDY: "
        "rebase the PR onto the default branch, or re-run once the PR ref carries a "
        "workflow file that sparse-checks out the whole scripts/ directory."
    )


# --------------------------------------------------------------------------------------
# [OPUS-5] Issue #1135 — RELEASE-PR EXCLUSION. THE GAP THIS CLOSES: this sweep armed ANY
# open PR carrying `review:pass` whose base is the default branch. It had NO release-plz
# exclusion of any kind — unlike scripts/batch-merge.py and scripts/pr-backlog.py, which
# both exclude it. So anything that put `review:pass` on the release-plz **Release PR**
# armed it, and merging that PR cuts a `v*` tag and (once `publish = true` in
# release-plz.toml) `cargo publish`es 37 crates. A crates.io version CANNOT be unpublished.
#
# The predicate lives in scripts/release_pr_guard.py and is keyed on BRANCH / AUTHOR /
# TITLE — never on a label, because a label can be added or removed by anything holding
# `pull-requests: write`, so a label-keyed exclusion is defeated by relabelling.
#
# UNLIKE the gh_retry degradation above, a MISSING guard module must NOT degrade to
# "proceed": the stub below refuses EVERY arm. A sweep that arms nothing is a missed
# cycle the cron covers; a sweep that arms the Release PR is an irreversible publish.
try:
    import release_pr_guard

    RELEASE_GUARD_DEGRADED = False
except ImportError as _release_guard_missing:  # pragma: no cover - see the self-test

    class _FailClosedReleaseGuard:
        """Refuse EVERY arm when the release-PR guard could not be imported."""

        _REASON = (
            "release-pr-guard: scripts/release_pr_guard.py is NOT importable, so this "
            "sweep cannot prove any PR is not the release-plz Release PR — refusing "
            "every arm (fail-closed, #1135). REMEDY: add scripts/release_pr_guard.py to "
            "this workflow's sparse-checkout manifest."
        )

        @staticmethod
        def arm_block_reason(**_kwargs) -> str:
            return _FailClosedReleaseGuard._REASON

    release_pr_guard = _FailClosedReleaseGuard  # type: ignore[assignment]
    RELEASE_GUARD_DEGRADED = True
    print(
        "::error title=auto-arm refusing every arm (scripts/release_pr_guard.py not "
        f"checked out)::{_release_guard_missing} — the release-PR exclusion (#1135) "
        "cannot be evaluated, so NOTHING is armed this cycle. Add "
        "scripts/release_pr_guard.py to the sparse-checkout manifest."
    )


PROGRAM = "auto-arm"
REVIEW_LABEL = "review:pass"
HUMAN_OR_TRUST_LABELS = frozenset(
    {
        "trust-surface",
        "needs:user",
        "review:needs-user",
        "area:sparq-zk",
        "area:sparq-mpc",
        "area:sparq-trust",
    }
)
# [FABLE-5] #3759 finding 1: a PR flipped to review:changes (e.g. by the merge-queue
# dequeued-feedback loop) is in the fix lane and must NEVER be armed — even if a stale
# review:pass is still present. This is belt-and-suspenders with the feedback workflow
# atomically REMOVING review:pass when it adds review:changes: the sweep treats
# review:changes (and review:needs) as a HARD exclusion independent of review:pass, so
# a race that momentarily left BOTH labels on the PR can never re-arm the dequeued PR
# before remediation.
REVIEW_CHANGES_LABELS = frozenset({"review:changes", "review:needs"})
# [OPUS-5] #1135: headRefName/author/title are the release-PR guard's ONLY inputs. If any
# of them stops being requested here, parse_pr yields None for it and arm_block_reason
# fails CLOSED (see its docstring) — the guard degrades to refusing, never to admitting.
PR_FIELDS = (
    "id,number,state,isDraft,headRefOid,headRefName,baseRefName,labels,autoMergeRequest,"
    "mergeStateStatus,author,title"
)
ENABLE_AUTO_MERGE = """mutation($id:ID!,$oid:GitObjectID!){
  enablePullRequestAutoMerge(input:{pullRequestId:$id,expectedHeadOid:$oid,mergeMethod:SQUASH}){
    pullRequest{number headRefOid autoMergeRequest{enabledAt}}
  }
}"""

# --------------------------------------------------------------------------------------
# [OPUS-5] #3760 — ARM CAPABILITY. See scripts/rearm-sweeper.py for the full measured
# rationale. Short version: enabling auto-merge is a repository WRITE operation, so a job
# whose `permissions:` block says `contents: read` is denied with
# "Resource not accessible by integration (enablePullRequestAutoMerge)" — proven live by
# sweeper run 30033315483 (contents: read, denied) vs this workflow's run 30027010495
# (contents: write, armed two PRs with the same plain GITHUB_TOKEN 90 minutes earlier).
# Repository.viewerPermission is null for App tokens and PullRequest.viewerCanEnableAutoMerge
# is false for already-armed/queued/draft PRs even under ADMIN, so neither is a capability
# signal; autoMergeAllowed is, and the mutation's own denial is the backstop.
CAPABILITY_QUERY = """query($owner:String!,$name:String!){
  repository(owner:$owner,name:$name){
    autoMergeAllowed
    viewerPermission
  }
  viewer{login}
}"""

# Deliberately disjoint from #3759's TRANSIENT set: a capability denial is permanent and
# must never be swallowed by the lenient ::warning + exit-0 sweep policy.
ARM_DENIAL_MARKERS = (
    "resource not accessible by integration",
    "must have write access",
    "must have admin access",
    "not authorized to enable auto-merge",
    "auto merge is not allowed for this repository",
    "auto-merge is not allowed for this repository",
)

ARM_REMEDIATION = (
    "the arm token cannot call enablePullRequestAutoMerge. Fix in this order: "
    "(1) .github/workflows/auto-arm.yml MUST declare `permissions:` with BOTH "
    "`contents: write` and `pull-requests: write` — enabling auto-merge is a repository "
    "WRITE operation and `contents: read` is denied with exactly this error (live proof: "
    "sweeper run 30033315483 failed with contents: read while auto-arm run 30027010495 "
    "armed successfully with contents: write on the same repo and the same GITHUB_TOKEN "
    "90 minutes earlier); "
    "(2) repository Settings -> General -> Pull Requests -> 'Allow auto-merge' must be ON "
    "(GraphQL Repository.autoMergeAllowed must be true); "
    "(3) if the repository's `main` ruleset gains a restriction on which integrations may "
    "write, github-actions[bot] must be allowed or an App token used instead; "
    "(4) the App path needs the repository/organization secrets ORCHESTRATOR_APP_ID and "
    "ORCHESTRATOR_APP_PRIVATE_KEY (both are currently UNSET); "
    "(5) on a `pull_request` event raised from a FORK, GITHUB_TOKEN is read-only no matter "
    "what the permissions block says — such a PR can only be armed by the scheduled run."
)

CAN_ARM = "can-arm"
CANNOT_ARM = "cannot-arm"
INCONCLUSIVE = "inconclusive"

# process() verdicts.
ARMED = "armed"
SKIPPED = "skipped"
FAILED = "failed"


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
class ArmOutcome:
    considered: int = 0
    armed: int = 0
    arm_failures: list[tuple[int, str]] = field(default_factory=list)
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
        return self.capability_failed or bool(self.arm_failures)

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

        This is how #3759 finding 5's invariant survives the #3760 refactor: the failed
        arm no longer unwinds the sweep, so it must be the run's EXIT STATUS instead.

        Deliberately independent of ``transient_exhaustions`` — a transient's exit is
        MODE-dependent (#3759) and is applied by :func:`arm_exit_code`, whereas a
        collected failure is non-zero in every mode.
        """
        return 1 if self.hard_failed else 0


class GhError(RuntimeError):
    """A gh command failed."""


@dataclass(frozen=True)
class PullRequest:
    node_id: str
    number: int
    state: str
    is_draft: bool
    head_oid: str
    base_ref: str
    labels: frozenset[str]
    armed: bool
    merge_state: str
    # [OPUS-5] #1135 release-PR guard inputs. `None` means NOT REPORTED (the field was
    # absent from the gh response) and is DISTINCT from "" — arm_block_reason fails closed
    # on an unknown head branch, so `None` must never be flattened into a string here.
    head_ref: str | None = None
    author_login: str | None = None
    title: str | None = None


def _optional_str(raw: dict, key: str) -> str | None:
    """Return raw[key] as a string, or None when the key is absent/null.

    Deliberately NOT `str(raw.get(key, ""))`: flattening a missing field to "" would let
    an unknown head branch read as a known-empty one, and the #1135 guard's fail-closed
    branch would never fire.
    """
    if key not in raw:
        return None
    value = raw[key]
    return None if value is None else str(value)


def parse_pr(raw: dict) -> PullRequest:
    labels = frozenset(
        str(label.get("name", "")).strip().lower()
        for label in raw.get("labels", [])
        if isinstance(label, dict)
    )
    author = raw.get("author")
    author_login = (
        _optional_str(author, "login") if isinstance(author, dict) else None
    )
    return PullRequest(
        node_id=str(raw.get("id", "")),
        number=int(raw["number"]),
        state=str(raw.get("state", "")).upper(),
        is_draft=bool(raw.get("isDraft")),
        head_oid=str(raw.get("headRefOid", "")),
        base_ref=str(raw.get("baseRefName", "")),
        labels=labels,
        armed=raw.get("autoMergeRequest") is not None,
        merge_state=str(raw.get("mergeStateStatus", "")).upper(),
        head_ref=_optional_str(raw, "headRefName"),
        author_login=author_login,
        title=_optional_str(raw, "title"),
    )


def run_gh(argv: list[str]) -> str:
    result = subprocess.run(
        ["gh", *argv], capture_output=True, text=True, check=False
    )
    if result.returncode != 0:
        detail = result.stderr.strip() or result.stdout.strip() or "unknown gh failure"
        raise GhError(f"gh {' '.join(argv[:3])} failed: {detail}")
    return result.stdout


def run_gh_read(argv: list[str]) -> str:
    """Idempotent READ path: bounded transient-only retries (#3759).

    Non-transient failures re-raise as :class:`GhError` so every existing per-PR
    handler keeps working; exhausted transients propagate as
    :class:`gh_retry.GhTransientExhausted` for the ::warning + exit-0 sweep policy.
    """
    try:
        return gh_retry.run_gh_read(argv)
    except gh_retry.GhFatalError as error:
        raise GhError(str(error)) from error


class AutoArmer:
    def __init__(
        self,
        repo: str,
        default_branch: str,
        gh: Callable[[list[str]], str] = run_gh,
        log: Callable[[str], None] = print,
        *,
        gh_read: Callable[[list[str]], str] | None = None,
        scope_pr: int | None = None,
    ) -> None:
        self.repo = repo
        self.default_branch = default_branch
        self.gh = gh
        # [OPUS-5] #3776: WHICH PRs this run is accountable for. None = the whole repository
        # (the cron sweep). A number = EXACTLY that PR and nothing else (an event-triggered
        # run must judge the PR whose label event triggered it, never a bystander).
        # Deliberately orthogonal to `mode`, which decides only the EXIT POLICY for an
        # exhausted transient (#3759 finding 5) — scope answers "whose failure is this run's
        # business", mode answers "how loud is a missed cycle".
        self.scope_pr = scope_pr
        # Idempotent READS may go through a retrying runner (#3759); mutations
        # (pr ready, the arm CAS) always use the one-shot `gh` runner above.
        self.gh_read = gh_read if gh_read is not None else gh
        self.log = log
        # [OPUS-5] #3760 latches: the fleet must see exactly ONE ::error per run, never
        # one per PR, and arming must stop once the token is known to be unable to arm.
        self.capability_error_emitted = False
        self.capability_lost = False
        # [OPUS-5] #3766: the collected outcome lives HERE, not only in a local of run(),
        # so an exception escaping run() can never discard an already-earned failure.
        self.outcome = ArmOutcome()
        owner, _, name = repo.partition("/")
        self.owner = owner
        self.name = name

    def list_open(self) -> list[PullRequest]:
        raw = json.loads(
            self.gh_read(
                [
                    "pr",
                    "list",
                    "--repo",
                    self.repo,
                    "--state",
                    "open",
                    "--limit",
                    "1000",
                    "--json",
                    PR_FIELDS,
                ]
            )
        )
        return [parse_pr(item) for item in raw]

    @property
    def scope_description(self) -> str:
        """Human-readable scope, logged before any work so a run states its own remit."""
        if self.scope_pr is None:
            return "EVERY open review:pass PR in the repository (whole-repo sweep)"
        return (
            f"PR #{self.scope_pr} ONLY — the PR whose label event triggered this run; "
            "another PR's arm failure is NOT this run's business (#3776)"
        )

    def candidates(self) -> list[PullRequest]:
        """The PRs this run is ACCOUNTABLE for — the #3776 scoping seam.

        SCOPED (``scope_pr`` set, i.e. ``--mode event``): exactly the triggering PR. The
        unrelated PRs are not merely excused, they are never even READ, so nothing about
        them can reach this run's outcome or its exit status.

        UNSCOPED (``scope_pr is None``, i.e. ``--mode sweep``): every open PR, unchanged.
        The cron is the whole-repo backstop and must stay one — narrowing it would turn
        this fix into "event mode does nothing and nothing else covers the repo".
        """
        if self.scope_pr is not None:
            return [self.refresh(self.scope_pr)]
        return self.list_open()

    def refresh(self, number: int) -> PullRequest:
        raw = json.loads(
            self.gh_read(
                [
                    "pr",
                    "view",
                    str(number),
                    "--repo",
                    self.repo,
                    "--json",
                    PR_FIELDS,
                ]
            )
        )
        return parse_pr(raw)

    @staticmethod
    def security_labels(pr: PullRequest) -> list[str]:
        return sorted(pr.labels & HUMAN_OR_TRUST_LABELS)

    def skip_reason(self, pr: PullRequest, *, require_review: bool = True) -> str | None:
        if pr.state != "OPEN":
            return f"not-open ({pr.state or 'unknown'})"
        # [OPUS-5] #1135: the release-plz Release PR is NEVER armed by an automated path.
        # Checked BEFORE every label-derived rule and independent of them: merging that PR
        # cuts a `v*` tag and (with `publish = true`) publishes 37 crates to crates.io,
        # which cannot be undone. Keyed on branch/author/title, so relabelling the PR — in
        # either direction — cannot change this answer. FAIL-CLOSED on an unknown branch.
        release_reason = release_pr_guard.arm_block_reason(
            head_ref=pr.head_ref,
            author_login=pr.author_login,
            title=pr.title,
        )
        if release_reason:
            return release_reason
        blocked = self.security_labels(pr)
        if blocked:
            return f"security-surface ({', '.join(blocked)})"
        # [FABLE-5] #3759 finding 1: review:changes is a HARD exclusion checked BEFORE
        # review:pass, so a PR in the fix lane is never armed even with a stale
        # review:pass still on it (a re-arm before remediation is the exact bug).
        changes = sorted(pr.labels & REVIEW_CHANGES_LABELS)
        if changes:
            return f"changes-requested ({', '.join(changes)})"
        if pr.base_ref != self.default_branch:
            return f"non-default-base ({pr.base_ref or 'unknown'})"
        if require_review and REVIEW_LABEL not in pr.labels:
            return "review-label-removed"
        if pr.armed:
            return "already-armed"
        return None

    def mark_ready(self, pr: PullRequest) -> bool:
        try:
            self.gh(["pr", "ready", str(pr.number), "--repo", self.repo])
        except GhError as error:
            self.log(f"PR #{pr.number}: skipped draft (gh pr ready failed: {error})")
            return False
        self.log(f"PR #{pr.number}: draft -> marked ready")
        return True

    def arm(self, pr: PullRequest) -> None:
        self.gh(
            [
                "api",
                "graphql",
                "-f",
                f"query={ENABLE_AUTO_MERGE}",
                "-f",
                f"id={pr.node_id}",
                "-f",
                f"oid={pr.head_oid}",
            ]
        )

    def classify_mutation_race(self, attempted: PullRequest) -> str | None:
        """Classify a failed CAS without ever retrying the mutation in this sweep."""
        current = self.refresh(attempted.number)
        reason = self.skip_reason(current)
        if reason:
            return reason
        if current.is_draft:
            return "draft"
        if current.head_oid != attempted.head_oid:
            return f"head-moved ({attempted.head_oid[:12]} -> {current.head_oid[:12]})"
        return None

    def probe_arm_capability(self) -> CapabilityVerdict:
        """Read-only startup probe: can this token arm at all in this repository?

        Decisive only where it can read decisively; anything else is INCONCLUSIVE and must
        never red the run (the mutation's own denial is the backstop). An exhausted #3759
        transient is inconclusive too — the sweep's retrying reads stay the authority on
        whether this cycle can proceed at all.
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

    def process(self, initial: PullRequest) -> tuple[str, str | None]:
        reason = self.skip_reason(initial)
        if reason:
            self.log(f"PR #{initial.number}: skipped {reason}")
            return SKIPPED, None

        if initial.is_draft and not self.mark_ready(initial):
            return SKIPPED, None

        try:
            current = self.refresh(initial.number)
        except GhError as error:
            self.log(f"PR #{initial.number}: skipped state-refresh-failed ({error})")
            return SKIPPED, None

        reason = self.skip_reason(current)
        if reason:
            self.log(f"PR #{initial.number}: skipped {reason}")
            return SKIPPED, None
        if current.is_draft:
            self.log(f"PR #{initial.number}: skipped draft")
            return SKIPPED, None
        if not current.node_id or not current.head_oid:
            self.log(f"PR #{initial.number}: skipped incomplete-live-state")
            return SKIPPED, None
        if current.head_oid != initial.head_oid:
            self.log(
                f"PR #{initial.number}: skipped head-moved "
                f"({initial.head_oid[:12]} -> {current.head_oid[:12]})"
            )
            return SKIPPED, None

        try:
            self.arm(current)
        except gh_retry.GhTransientExhausted as mutation_exhausted:
            # [OPUS-5] #3766, fail CLOSED. The arm runs on the ONE-SHOT runner, which never
            # raises this today (gh_retry refuses to wrap mutations) — but if it ever did,
            # the mutation's outcome would be UNKNOWN, and an unknown arm must be a per-PR
            # FAILURE, never the lenient warning-class outcome a transient normally gets.
            self.log(
                f"PR #{initial.number}: arm-failed (transient exhaustion on the arm "
                f"mutation itself: {mutation_exhausted})"
            )
            return FAILED, f"transient exhaustion on the arm mutation ({mutation_exhausted})"
        except GhError as mutation_error:
            message = str(mutation_error)
            # [OPUS-5] #3760: a capability denial is NOT a per-PR condition — every
            # remaining candidate would fail identically. Record it, stop arming, and
            # surface exactly ONE ::error naming what to change.
            if is_arm_denial(message):
                self.log(
                    f"PR #{initial.number}: arm-failed arm capability denied ({message})"
                )
                self.fail_capability(
                    f"arming PR #{initial.number} was denied: {message}"
                )
                return FAILED, f"arm capability denied ({message})"
            # The arm mutation FAILED — this error is the primary run status and must
            # survive whatever the follow-up diagnostic read does. The diagnostic
            # read (classify_mutation_race -> refresh) is itself retriable, so it can
            # raise GhError *or* GhTransientExhausted; if it exhausts transient
            # retries we must NOT let that exhaustion propagate, or run_sweep's
            # lenient GhTransientExhausted handler would convert a genuine arm failure
            # into a false success (::warning + exit 0). Suppress the diagnostic's own
            # failure and keep the ORIGINAL mutation error as this PR's verdict.
            # (#3759 finding 5 — its invariant now shows up as a non-zero EXIT rather
            # than as an exception unwinding the whole sweep, per #3760.)
            try:
                race = self.classify_mutation_race(current)
            except (GhError, gh_retry.GhTransientExhausted) as diag_error:
                self.log(
                    f"PR #{initial.number}: arm-failed and the race diagnostic could "
                    f"not resolve ({diag_error}); the original arm error stands ({message})"
                )
                return FAILED, message
            if race:
                self.log(f"PR #{initial.number}: skipped {race}")
                return SKIPPED, None
            # Collect and CONTINUE — before #3760 this re-raised, so ONE bad PR stopped
            # every later candidate from ever being armed.
            self.log(f"PR #{initial.number}: arm-failed ({message})")
            return FAILED, message
        self.log(f"PR #{initial.number}: armed head {current.head_oid[:12]}")
        return ARMED, None

    def run(self) -> ArmOutcome:
        # [OPUS-5] #3766: publish the outcome on the ARMER before doing any work, so every
        # failure collected below survives any exception that escapes this method and the
        # exit status can be computed at the END from this state (see arm_exit_code).
        outcome = self.outcome = ArmOutcome()
        self.log(f"[{PROGRAM}] scope: {self.scope_description}")
        verdict = self.probe_arm_capability()
        outcome.capability = verdict.status
        self.log(f"[{PROGRAM}] arm-capability probe: {verdict.status} — {verdict.detail}")
        if verdict.blocks_sweep:
            # Fail ONCE, before touching any PR: a broken token would otherwise fail
            # identically on every candidate, on every label event, forever. This is not
            # a transient, so it never reaches the #3759 ::warning + exit-0 path.
            self.fail_capability(f"startup arm-capability probe failed: {verdict.detail}")
            self.log(
                f"[{PROGRAM}] complete: considered=0 armed=0 arm-failures=0 "
                f"capability={verdict.status}"
            )
            return outcome

        try:
            candidates = self.candidates()
        except gh_retry.GhTransientExhausted as error:
            # Whole-sweep transient: nothing has been collected yet, so this is a plain
            # missed cycle. Recorded, not raised — the exit is decided at the end.
            outcome.sweep_transient = str(error)
            self.log(
                f"[{PROGRAM}] candidate enumeration exhausted transient retries ({error})"
            )
            candidates = []

        for pr in candidates:
            if REVIEW_LABEL not in pr.labels:
                continue
            outcome.considered += 1
            if self.capability_lost:
                self.log(
                    f"PR #{pr.number}: skipped arm capability lost this run "
                    "(see the single ::error above)"
                )
                continue
            try:
                status, failure = self.process(pr)
            except gh_retry.GhTransientExhausted as error:
                # [OPUS-5] #3766 (BLOCKING, reproduced by cross-provider review): a LATER
                # candidate's exhausted transient is its OWN warning-class per-candidate
                # outcome. It must NOT escape this loop — an escaping exception carried the
                # run past the accumulated-failure check, so PR #451's real arm failure was
                # reported as ::warning + exit 0 once PR #452's refresh exhausted. The
                # sweep continues; the verdict #451 already earned is untouchable.
                outcome.transient_exhaustions.append((pr.number, str(error)))
                self.log(
                    f"PR #{pr.number}: skipped transient-exhausted ({error}); the sweep "
                    "continues and this cannot downgrade an earlier arm failure"
                )
                continue
            if status == ARMED:
                outcome.armed += 1
            elif status == FAILED:
                outcome.arm_failures.append((pr.number, failure or "unknown"))
                if self.capability_lost:
                    outcome.capability = CANNOT_ARM

        for number, reason in outcome.arm_failures:
            self.log(f"[{PROGRAM}] arm-failure summary: PR #{number} — {reason}")
        for number, reason in outcome.transient_exhaustions:
            self.log(f"[{PROGRAM}] transient-exhaustion summary: PR #{number} — {reason}")
        self.log(
            f"[{PROGRAM}] complete: considered={outcome.considered} "
            f"armed={outcome.armed} arm-failures={len(outcome.arm_failures)} "
            f"transient-exhaustions={len(outcome.transient_exhaustions)} "
            f"capability={outcome.capability} "
            f"scope={'all' if self.scope_pr is None else f'pr-{self.scope_pr}'}"
        )
        return outcome


def arm_exit_code(
    outcome: ArmOutcome, *, mode: str, log: Callable[[str], None] = print
) -> int:
    """Decide the run's exit status from the FINAL collected state (#3766).

    PRECEDENCE — ``collected-failure > transient-exhaustion > clean``:

    1. A COLLECTED arm/capability failure DOMINATES, in either mode. Nothing that happens
       while processing a LATER candidate — an exhausted transient above all — can
       downgrade a verdict an EARLIER candidate already earned. This check is reached from
       the accumulated state, never short-circuited by an exception.
    2. TRANSIENT EXHAUSTION alone (no collected failure anywhere) keeps #3759's intended
       lenient policy: ``sweep`` reports ``::warning`` + exit 0, because a missed cycle is
       harmless and the cron re-covers it. ``event`` has no per-PR backstop, so it fails
       loudly instead.
    3. CLEAN is 0.
    """
    if outcome.hard_failed:
        if outcome.transient_detail:
            log(
                f"[{PROGRAM}] a transient exhaustion also occurred "
                f"({outcome.transient_detail}) but a collected failure outranks it "
                "(precedence: collected-failure > transient-exhaustion > clean): exit 1"
            )
        return outcome.exit_code
    if outcome.transient_detail:
        if mode == "event":
            # Event-driven arm: no per-PR cron backstop — fail loudly so it is retried.
            raise GhError(
                "event-mode auto-arm exhausted transient retries: "
                f"{outcome.transient_detail}"
            )
        log(
            "::warning title=auto-arm skipped a cycle on transient GitHub API "
            f"failures::{outcome.transient_detail} — bounded retries exhausted; a missed "
            "sweep is harmless (the cron backstop covers it), so this run reports success."
        )
        return 0
    return 0


def run_sweep(
    armer: AutoArmer, log: Callable[[str], None] = print, *, mode: str = "sweep"
) -> int:
    """Run the armer once. Exit policy depends on the INVOCATION MODE (#3759 finding 5).

    SCOPE and MODE are separate axes (#3776). ``AutoArmer.scope_pr`` decides WHICH PRs the
    run is accountable for; ``mode`` decides only how loud an exhausted transient is. An
    event-triggered run is scoped to its triggering PR, so the failures this function can
    ever see are that PR's own.

    ``mode="sweep"`` (the periodic cron) — exhausted TRANSIENT retries on the READ
    path => ``::warning`` + exit 0: the sweep is idempotent and cron-covered, so a
    missed cycle must not red main's gate.

    ``mode="event"`` (the label-triggered auto-arm on a single PR) — there is NO cron
    backstop for *this* PR right now, so the lenient exit does NOT apply: an exhausted
    transient (or any other failure) exits NON-zero so the arm attempt is visibly red
    and gets retried. Only the periodic sweep may swallow a transient.

    A failed arm MUTATION always reds the run in either mode (the diagnostic read's own
    exhaustion can never overwrite it — see ``process``). Since #3760 it no longer does so
    by unwinding the sweep: it is COLLECTED so every later candidate is still considered,
    and it surfaces as ``ArmOutcome.exit_code == 1``. A provable arm-capability failure is
    likewise never a transient — it reds both modes with one ``::error``.

    Since #3766 the exit is computed at the END from :class:`ArmOutcome`, whose precedence
    is ``collected-failure > transient-exhaustion > clean`` — see :func:`arm_exit_code`."""
    if mode not in ("sweep", "event"):
        raise ValueError(f"mode must be 'sweep' or 'event', got {mode!r}")
    try:
        outcome = armer.run()
    except gh_retry.GhTransientExhausted as error:
        # [OPUS-5] #3766 STICKY BACKSTOP. run() already records a per-candidate exhaustion
        # without unwinding; this is defence in depth for any read that is ever added
        # OUTSIDE that guarded loop. The outcome is read back OFF THE ARMER, so everything
        # already collected still reaches arm_exit_code — an exception can no longer
        # short-circuit past the accumulated-failure check and report a false success.
        outcome = armer.outcome
        outcome.sweep_transient = outcome.sweep_transient or str(error)
    return arm_exit_code(outcome, mode=mode, log=log)


# [OPUS-5] #3776: emitted when an EVENT-mode run cannot work out which PR triggered it.
# It falls back to whole-repo scope — the pre-#3776 behaviour — rather than exiting
# non-zero, because a hard failure here would brick the GATING check for exactly the
# stale-ref population this issue is about. Loud, not fatal.
SCOPE_UNRESOLVED_WARNING = (
    "::warning title=auto-arm could not identify the triggering PR::--mode event was "
    "requested but neither --pr nor the runner environment (GITHUB_EVENT_PATH's "
    "pull_request.number, GITHUB_REF's refs/pull/<n>/…) yielded a PR number, so this run "
    "falls back to WHOLE-REPO scope. Consequence: an unrelated un-armable PR can red this "
    "run (sparq #3776). REMEDY: pass --pr <number> from the workflow "
    "(`${{ github.event.pull_request.number }}`)."
)

# refs/pull/<n>/merge (or /head) — what GITHUB_REF holds on a pull_request event.
PULL_REF = re.compile(r"^refs/pull/(\d+)/")


def resolve_scope_pr(
    explicit: str | int | None = None,
    env: Mapping[str, str] | None = None,
) -> int | None:
    """Identify the PR whose event triggered this run, or ``None`` if undeterminable.

    Three sources, most explicit first:

    1. ``--pr`` — what the workflow passes.
    2. ``GITHUB_EVENT_PATH`` → the event payload's ``pull_request.number``.
    3. ``GITHUB_REF`` → ``refs/pull/<n>/merge``.

    The two environment sources are LOAD-BEARING, not belt-and-braces. ``--pr`` reaches
    this script from the WORKFLOW FILE, which on a `pull_request` event GitHub resolves
    from the PR's own — possibly months-old — ref, while this script always comes from the
    default branch: that skew IS #3776. Deriving the number from the RUNNER's environment
    instead means every already-stale PR ref gets per-PR scoping the moment this lands,
    with no per-ref workflow update and no rebase.

    Raises ``ValueError`` only for an explicitly supplied non-numeric ``--pr`` — a typo in
    the wiring must be loud, never silently a whole-repo sweep.
    """
    if explicit is not None and str(explicit).strip():
        text = str(explicit).strip()
        if not text.isdigit():
            # Positive integers only: a sign, a ref name or an empty-ish token must be a
            # loud wiring error, never `gh pr view -5` or a silent whole-repo sweep.
            raise ValueError(f"--pr must be a PR number, got {explicit!r}")
        return int(text)
    environment = os.environ if env is None else env
    event_path = (environment.get("GITHUB_EVENT_PATH") or "").strip()
    if event_path:
        try:
            with open(event_path, encoding="utf-8") as handle:
                payload = json.load(handle)
            number = (payload.get("pull_request") or {}).get("number")
            if number is not None:
                return int(number)
        except (OSError, ValueError, TypeError, AttributeError):
            # A malformed/absent payload is not this script's problem to die on; fall
            # through to the ref, then to the documented whole-repo fallback.
            pass
    match = PULL_REF.match((environment.get("GITHUB_REF") or "").strip())
    return int(match.group(1)) if match else None


# Sentinel: "the caller did not mention this key at all" (vs. passing an explicit None,
# which means "the key is present and null"). #1135's fail-closed branch needs both.
_MISSING = object()


def fixture(
    *,
    number: int = 447,
    labels: tuple[str, ...] = (REVIEW_LABEL,),
    draft: bool = False,
    armed: bool = False,
    oid: str = "a" * 40,
    base: str = "main",
    merge_state: str = "CLEAN",
    # [OPUS-5] #1135 release-PR guard inputs. Defaults describe an ORDINARY worker PR so
    # every pre-existing fixture keeps arming; `_MISSING` distinguishes "omit the key
    # entirely" (the fail-closed case) from "the key is present with this value".
    head_ref: object = _MISSING,
    author_login: object = _MISSING,
    title: object = _MISSING,
) -> dict:
    raw = {
        "id": f"PR_kwDO{number}",
        "number": number,
        "state": "OPEN",
        "isDraft": draft,
        "headRefOid": oid,
        "baseRefName": base,
        "labels": [{"name": label} for label in labels],
        "autoMergeRequest": {"enabledAt": "2026-07-19T00:00:00Z"} if armed else None,
        "mergeStateStatus": merge_state,
    }
    if head_ref is not _MISSING:
        raw["headRefName"] = head_ref
    else:
        raw["headRefName"] = f"sparq-agent/issue-{number}-worker"
    if author_login is not _MISSING:
        raw["author"] = None if author_login is None else {"login": author_login}
    else:
        raw["author"] = {"login": "app/sparq-orchestrator"}
    if title is not _MISSING:
        raw["title"] = title
    else:
        raw["title"] = f"fix(engine): worker change for #{number}"
    return raw


CAPABILITY_RESPONSE = {
    "data": {
        "repository": {"autoMergeAllowed": True, "viewerPermission": None},
        "viewer": {"login": "github-actions[bot]"},
    }
}
# The exact text GitHub returned in run 30033315483 — the #3760 regression anchor.
DENIAL_TEXT = (
    "gh api graphql failed: GraphQL: Resource not accessible by integration "
    "(enablePullRequestAutoMerge)"
)
# The exact text GitHub returned for PR #3434 in run 30144214363 — the PERMANENTLY
# un-armable PR that poisoned the gating arm check for the whole repository (#3776/#3777).
# It is deliberately NOT an ARM_DENIAL_MARKER: it is a per-PR condition (that PR modifies a
# workflow file), so it is collected per-PR rather than latching capability_lost.
WORKFLOWS_DENIAL = (
    "gh api graphql -f failed: gh: Pull request refusing to allow a GitHub App to create "
    "or update workflow `.github/workflows/zk-toolchain.yml` without `workflows` permission"
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
        initial: dict | list[dict],
        *,
        views: list[dict] | None = None,
        mutation_error: bool = False,
        mutation_errors: dict[int, str] | None = None,
        auto_merge_allowed: bool | None = True,
        capability_error: str | None = None,
    ) -> None:
        listing = initial if isinstance(initial, list) else [initial]
        self.listing = [copy.deepcopy(item) for item in listing]
        self.initial = self.listing[0] if self.listing else {}
        self.views = [copy.deepcopy(view) for view in (views or self.listing)]
        self.mutation_error = mutation_error
        self.mutation_errors = dict(mutation_errors or {})
        self.auto_merge_allowed = auto_merge_allowed
        self.capability_error = capability_error
        self.calls: list[list[str]] = []

    def __call__(self, argv: list[str]) -> str:
        self.calls.append(argv)
        if argv[:2] == ["pr", "list"]:
            return json.dumps(self.listing)
        if argv[:2] == ["pr", "ready"]:
            return ""
        if argv[:2] == ["pr", "view"]:
            return json.dumps(self._view(int(argv[2])))
        if is_capability_query(argv):
            if self.capability_error is not None:
                raise GhError(self.capability_error)
            return capability_payload(self.auto_merge_allowed)
        if argv[:2] == ["api", "graphql"]:
            if self.mutation_error:
                raise GhError("simulated expectedHeadOid CAS rejection")
            node = next(arg for arg in argv if arg.startswith("id="))
            failure = next(
                (
                    text
                    for number, text in self.mutation_errors.items()
                    if node.endswith(str(number))
                ),
                None,
            )
            if failure is not None:
                raise GhError(failure)
            return json.dumps({"data": {"enablePullRequestAutoMerge": {}}})
        raise AssertionError(f"unexpected fake gh call: {argv}")

    def _view(self, number: int) -> dict:
        """Successive views for one PR are consumed in order; the last one sticks."""
        matching = [view for view in self.views if int(view["number"]) == number]
        if len(matching) > 1:
            self.views.remove(matching[0])
            return matching[0]
        if matching:
            return matching[0]
        return self.views.pop(0) if len(self.views) > 1 else self.views[0]


def exercise(
    initial: dict | list[dict], *, scope_pr: int | None = None, **fake_kwargs
) -> tuple[FakeGh, list[str], ArmOutcome]:
    fake = FakeGh(initial, **fake_kwargs)
    messages: list[str] = []
    outcome = AutoArmer(
        "sparq-org/sparq", "main", fake, messages.append, scope_pr=scope_pr
    ).run()
    return fake, messages, outcome


def graphql_calls(fake: FakeGh) -> list[list[str]]:
    """Only the ARM mutations — never the read-only capability probe."""
    return [
        call
        for call in fake.calls
        if call[:2] == ["api", "graphql"] and not is_capability_query(call)
    ]


def probe_query_calls(fake: FakeGh) -> list[list[str]]:
    return [call for call in fake.calls if is_capability_query(call)]


def self_test() -> None:
    expected_guards = {
        "trust-surface",
        "needs:user",
        "review:needs-user",
        "area:sparq-zk",
        "area:sparq-mpc",
        "area:sparq-trust",
    }
    assert HUMAN_OR_TRUST_LABELS == expected_guards, HUMAN_OR_TRUST_LABELS

    # Reviewed, live, non-draft: arm once with the explicit method and head-OID CAS.
    clean = fixture()
    assert parse_pr(clean).merge_state == "CLEAN", clean
    fake, messages, outcome = exercise(clean)
    mutations = graphql_calls(fake)
    assert len(mutations) == 1, mutations
    mutation = mutations[0]
    assert "expectedHeadOid:$oid" in " ".join(mutation), mutation
    assert "mergeMethod:SQUASH" in " ".join(mutation), mutation
    assert f"oid={clean['headRefOid']}" in mutation, mutation
    assert any("armed head" in message for message in messages), messages

    # ---------------------------------------------------------------- #1135 Release PR
    # THE GAP THIS CLOSES: before #1135 this sweep had NO release-plz exclusion, so a
    # `review:pass` on the Release PR armed it — and merging it tags + (once
    # `publish = true`) publishes 37 crates to crates.io, irreversibly.
    #
    # Every case below asserts NO GraphQL mutation was issued. The `clean` fixture above
    # DOES arm, so "no mutation" is a discriminating outcome, not the fixture's default.
    release_pr = fixture(
        number=900,
        head_ref="release-plz-main",
        author_login="app/github-actions",
        title="chore: release v0.2.0",
    )
    fake, messages, outcome = exercise(release_pr)
    assert not graphql_calls(fake), fake.calls
    assert any("release-pr-guard" in message for message in messages), messages

    # THE EXACT ATTACK THE GUARD MUST DEFEAT: someone adds `review:pass` (and even strips
    # every hold label) to the Release PR. The exclusion is keyed on branch/author/title,
    # so no label combination can make it armable.
    for labels in (
        (REVIEW_LABEL,),
        (REVIEW_LABEL, "review:pass"),
        (REVIEW_LABEL, "area:ci"),
        (),
    ):
        relabelled = fixture(
            number=901,
            labels=labels,
            head_ref="release-plz-main",
            author_login="app/github-actions",
            title="chore: release v0.2.0",
        )
        fake, messages, outcome = exercise(relabelled)
        assert not graphql_calls(fake), (labels, fake.calls)

    # Each signal alone suffices — branch, author, and title are OR-ed, not AND-ed.
    for label, kwargs in (
        ("branch only", {"head_ref": "release-plz-main"}),
        ("author only", {"author_login": "release-plz[bot]"}),
        ("title only", {"title": "chore: release v0.2.0"}),
    ):
        fake, messages, outcome = exercise(fixture(number=902, **kwargs))
        assert not graphql_calls(fake), (label, fake.calls)
        assert any("release-pr-guard" in message for message in messages), (
            label,
            messages,
        )

    # FAIL-CLOSED: an unknown head branch (the gh response dropped headRefName, or
    # returned null) must REFUSE, never admit.
    for label, kwargs in (
        ("headRefName null", {"head_ref": None}),
        ("headRefName empty", {"head_ref": ""}),
    ):
        fake, messages, outcome = exercise(fixture(number=903, **kwargs))
        assert not graphql_calls(fake), (label, fake.calls)
        assert any("indeterminate head branch" in message for message in messages), (
            label,
            messages,
        )

    # The guard is evaluated on the LIVE REFRESH too, not only the list snapshot: a PR
    # that is retargeted onto the release branch between the two reads is not armed.
    fake, messages, outcome = exercise(
        clean,
        views=[
            fixture(head_ref="release-plz-main", title="chore: release v0.2.0"),
        ],
    )
    assert not graphql_calls(fake), fake.calls
    assert any("release-pr-guard" in message for message in messages), messages

    # PR_FIELDS must actually REQUEST the guard's three inputs. Deleting one of them from
    # the query does not silently disable the guard (parse_pr yields None -> fail-closed),
    # but it WOULD wedge the whole sweep, so pin them explicitly here.
    for field in ("headRefName", "author", "title"):
        assert field in PR_FIELDS, (field, PR_FIELDS)

    # Every named human/trust area is fail-closed before any mutation.
    for label in sorted(expected_guards):
        fake, messages, outcome = exercise(fixture(labels=(REVIEW_LABEL, label)))
        assert not graphql_calls(fake), (label, fake.calls)
        assert any("security-surface" in message for message in messages), (label, messages)

    # [FABLE-5] #3759 finding 1: review:changes is a HARD exclusion. A dequeued PR that
    # still carries a STALE review:pass (a race left both on) must NEVER be armed — the
    # fix-lane label wins. Both review-hold labels are checked independent of review:pass.
    for hold in ("review:changes", "review:needs"):
        fake, messages, outcome = exercise(fixture(labels=(REVIEW_LABEL, hold)))
        assert not graphql_calls(fake), (hold, fake.calls)
        assert any("changes-requested" in message for message in messages), (hold, messages)
    # It also stops an arm when review:changes appears only on the live refresh.
    fake, messages, outcome = exercise(clean, views=[fixture(labels=(REVIEW_LABEL, "review:changes"))])
    assert not graphql_calls(fake), fake.calls
    assert any("changes-requested" in message for message in messages), messages

    # Existing auto-merge arms are idempotent no-ops.
    fake, messages, outcome = exercise(fixture(armed=True))
    assert not graphql_calls(fake), fake.calls
    assert any("already-armed" in message for message in messages), messages

    # A hold, review removal, or competing arm that appears on refresh stays fail-closed.
    transitions = (
        (fixture(labels=(REVIEW_LABEL, "trust-surface")), "security-surface"),
        (fixture(labels=()), "review-label-removed"),
        (fixture(armed=True), "already-armed"),
    )
    for refreshed, expected_reason in transitions:
        fake, messages, outcome = exercise(clean, views=[refreshed])
        assert not graphql_calls(fake), (expected_reason, fake.calls)
        assert any(expected_reason in message for message in messages), (
            expected_reason,
            messages,
        )

    # Draft workers become ready, are refreshed, then are armed exactly once.
    draft = fixture(draft=True)
    ready = fixture(draft=False)
    fake, messages, outcome = exercise(draft, views=[ready])
    assert any(call[:2] == ["pr", "ready"] for call in fake.calls), fake.calls
    assert len(graphql_calls(fake)) == 1, fake.calls
    assert any("draft -> marked ready" in message for message in messages), messages

    # A push between the list snapshot and the live refresh is skipped before mutation.
    moved = fixture(oid="b" * 40)
    fake, messages, outcome = exercise(clean, views=[moved])
    assert not graphql_calls(fake), fake.calls
    assert any("head-moved" in message for message in messages), messages

    # A moved head makes the GraphQL CAS fail; classify it and never retry this sweep.
    fake, messages, outcome = exercise(
        clean, views=[clean, moved], mutation_error=True
    )
    assert len(graphql_calls(fake)) == 1, fake.calls
    assert any("head-moved" in message for message in messages), messages

    # Stacked PRs must never be armed into their non-default base.
    fake, messages, outcome = exercise(fixture(base="feature-base"))
    assert not graphql_calls(fake), fake.calls
    assert any("non-default-base" in message for message in messages), messages

    # [FABLE-5] #3759: READS route through gh_read; MUTATIONS stay on the one-shot
    # runner (the retry helper itself refuses them — see gh_retry.assert_read_only).
    fake = FakeGh(clean)
    read_calls: list[list[str]] = []

    def spy_read(argv: list[str]) -> str:
        read_calls.append(argv)
        gh_retry.assert_read_only(argv)  # every read the armer issues IS a read
        return fake(argv)

    AutoArmer(
        "sparq-org/sparq", "main", fake, lambda _m: None, gh_read=spy_read
    ).run()
    # [OPUS-5] #3760: the capability probe is itself an idempotent READ (the retry
    # helper's fail-closed guard accepts it — no mutation in the query), so it routes
    # through gh_read and precedes the enumeration.
    assert [c[:2] for c in read_calls] == [
        ["api", "graphql"],
        ["pr", "list"],
        ["pr", "view"],
    ], read_calls
    assert is_capability_query(read_calls[0]), read_calls[0]
    direct = [c for c in fake.calls if c not in read_calls]
    assert [c[:2] for c in direct] == [["api", "graphql"]], fake.calls
    assert not is_capability_query(direct[0]), direct

    # Exhausted transient retries abort the sweep as ::warning + exit 0 (a missed
    # cycle is harmless); the arm mutation is never attempted on partial state.
    def exhausted(_argv: list[str]) -> str:
        raise gh_retry.GhTransientExhausted("gh pr list: HTTP 504 (3 attempts)")

    fake = FakeGh(clean)
    warnings: list[str] = []
    code = run_sweep(
        AutoArmer("sparq-org/sparq", "main", fake, lambda _m: None, gh_read=exhausted),
        log=warnings.append,
    )
    assert code == 0, code
    assert not fake.calls, fake.calls
    assert warnings and warnings[0].startswith("::warning"), warnings

    # A NON-transient failure still fails loudly: GhError escapes run_sweep untouched.
    def fatal(_argv: list[str]) -> str:
        raise GhError("gh pr list failed: HTTP 403 forbidden")

    try:
        run_sweep(
            AutoArmer("sparq-org/sparq", "main", fake, lambda _m: None, gh_read=fatal),
            log=warnings.append,
        )
    except GhError:
        pass
    else:
        raise AssertionError("non-transient errors must not be swallowed")

    # [FABLE-5] #3759 finding 5 (BLOCKING) — invariant preserved, mechanism changed by
    # [OPUS-5] #3760: a real arm MUTATION failure must be the run's status even when the
    # follow-up race-diagnostic READ exhausts transient retries. It used to prove this by
    # PROPAGATING the mutation error out of run_sweep, which also aborted every later
    # candidate (#3760). It now proves the same thing the way the fleet actually needs it:
    # the failure is recorded and the run exits NON-ZERO. It must never be rewritten into
    # a lenient exit-0 success.
    fake = FakeGh(clean, mutation_error=True)

    def read_lists_then_exhausts(argv: list[str]) -> str:
        # The pre-arm reads (the capability probe, pr list, the pre-arm pr view) succeed;
        # the diagnostic refresh AFTER the failed arm is the one that exhausts transients.
        if argv[:2] == ["pr", "view"] and getattr(read_lists_then_exhausts, "armed", False):
            raise gh_retry.GhTransientExhausted("gh pr view: HTTP 504 (3 attempts)")
        return fake(argv)

    def gh_marks_arm(argv: list[str]) -> str:
        # Only the ARM mutation flips the marker — never the read-only probe.
        if argv[:2] == ["api", "graphql"] and not is_capability_query(argv):
            read_lists_then_exhausts.armed = True  # type: ignore[attr-defined]
        return fake(argv)

    read_lists_then_exhausts.armed = False  # type: ignore[attr-defined]
    messages: list[str] = []
    code = run_sweep(
        AutoArmer(
            "sparq-org/sparq",
            "main",
            gh_marks_arm,
            messages.append,
            gh_read=read_lists_then_exhausts,
        ),
        log=lambda _m: None,
    )
    assert code == 1, (
        "a failed arm mutation must NOT be masked as success when the follow-up "
        f"diagnostic read exhausts its transient retries (code={code}, {messages})"
    )
    assert any("CAS rejection" in message for message in messages), messages
    assert any(
        "arm-failure summary" in message for message in messages
    ), messages
    # It is a per-PR failure, not a capability failure: no ::error, no capability latch.
    assert not [m for m in messages if m.startswith("::error")], messages

    # [FABLE-5] #3759 finding 5: EVENT-mode (label-triggered) arm must NOT use the
    # sweep's lenient exit — an exhausted transient exits NON-zero (there is no
    # per-PR cron backstop), so the arm is visibly red and retried.
    def exhausted_event(_argv: list[str]) -> str:
        raise gh_retry.GhTransientExhausted("gh pr list: HTTP 504 (3 attempts)")

    fake = FakeGh(clean)
    try:
        run_sweep(
            AutoArmer(
                "sparq-org/sparq", "main", fake, lambda _m: None, gh_read=exhausted_event
            ),
            log=lambda _m: None,
            mode="event",
        )
    except GhError as error:
        assert "event-mode" in str(error), error
    else:
        raise AssertionError("event-mode exhausted transient must exit non-zero")

    # The SAME exhausted transient in SWEEP mode is still the lenient ::warning + 0.
    fake = FakeGh(clean)
    warnings = []
    code = run_sweep(
        AutoArmer(
            "sparq-org/sparq", "main", fake, lambda _m: None, gh_read=exhausted_event
        ),
        log=warnings.append,
        mode="sweep",
    )
    assert code == 0, code
    assert warnings and warnings[0].startswith("::warning"), warnings

    # ---------------------------------------------------------------------------------
    # [OPUS-5] #3760 — ARM CAPABILITY + PER-PR FAILURE ISOLATION.
    # ---------------------------------------------------------------------------------

    # The live #3760 error text classifies as a CAPABILITY denial; a CAS/race/transient
    # does not (the denial set must stay disjoint from #3759's transient set, or a
    # permanent denial could be swallowed as ::warning + exit 0).
    assert is_arm_denial(DENIAL_TEXT), DENIAL_TEXT
    assert is_arm_denial(DENIAL_TEXT.upper()), "denial matching must be case-insensitive"
    for benign in (
        "simulated expectedHeadOid CAS rejection",
        "gh api graphql failed: HTTP 502: 502 Bad Gateway",
        "gh api graphql failed: HTTP 504: Gateway Timeout",
        "gh api graphql failed: Head branch was modified. Review and try again",
    ):
        assert not is_arm_denial(benign), benign

    # (1) PROBE — can-arm: the probe runs BEFORE any mutation, the sweep proceeds, exit 0.
    fake, messages, outcome = exercise(clean)
    assert outcome.capability == CAN_ARM, outcome
    assert outcome.armed == 1 and outcome.exit_code == 0, (outcome, messages)
    assert len(probe_query_calls(fake)) == 1, fake.calls
    assert fake.calls.index(probe_query_calls(fake)[0]) < fake.calls.index(
        graphql_calls(fake)[0]
    ), fake.calls
    assert any("arm-capability probe: can-arm" in m for m in messages), messages

    # (2) PROBE — cannot-arm (repository setting OFF): ONE ::error naming the exact
    # setting, ZERO PRs touched (the listing never even runs), exit 1.
    fake, messages, outcome = exercise(clean, auto_merge_allowed=False)
    assert outcome.capability == CANNOT_ARM and outcome.exit_code == 1, outcome
    assert not graphql_calls(fake), fake.calls
    assert not [call for call in fake.calls if call[:2] == ["pr", "list"]], fake.calls
    emitted = [message for message in messages if message.startswith("::error")]
    assert len(emitted) == 1, emitted
    assert "Allow auto-merge" in emitted[0], emitted
    assert "contents: write" in emitted[0], emitted
    assert "ORCHESTRATOR_APP_ID" in emitted[0], emitted

    # (3) PROBE — cannot-arm (the token itself is denied by GitHub): same single error.
    fake, messages, outcome = exercise(clean, capability_error=DENIAL_TEXT)
    assert outcome.capability == CANNOT_ARM and outcome.exit_code == 1, outcome
    assert not graphql_calls(fake), fake.calls
    assert len([m for m in messages if m.startswith("::error")]) == 1, messages

    # (4) PROBE — inconclusive must NEVER red the run or stop the sweep, including an
    # exhausted #3759 transient on the probe read itself.
    for kwargs in (
        {"capability_error": "gh api graphql failed: HTTP 504: Gateway Timeout"},
        {"auto_merge_allowed": None},
    ):
        fake, messages, outcome = exercise(clean, **kwargs)
        assert outcome.capability == INCONCLUSIVE, (kwargs, outcome)
        assert outcome.armed == 1 and outcome.exit_code == 0, (kwargs, outcome)
        assert not [m for m in messages if m.startswith("::error")], messages

    fake = FakeGh(clean)

    def probe_exhausts(argv: list[str]) -> str:
        if is_capability_query(argv):
            raise gh_retry.GhTransientExhausted("gh api graphql: HTTP 504 (3 attempts)")
        return fake(argv)

    probe_verdict = AutoArmer(
        "sparq-org/sparq", "main", fake, lambda _m: None, gh_read=probe_exhausts
    ).probe_arm_capability()
    assert probe_verdict.status == INCONCLUSIVE, probe_verdict
    assert "transient" in probe_verdict.detail, probe_verdict

    # (5) PER-PR ARM FAILURE — the pre-#3760 bug: a non-capability mutation failure on the
    # FIRST PR re-raised out of run(), so PR #452 below was never armed. Now the failure is
    # collected, the sweep continues, and the run still exits non-zero.
    first = fixture(number=451)
    second = fixture(number=452)
    fake, messages, outcome = exercise(
        [first, second],
        mutation_errors={451: "gh api graphql failed: Pull request is in unstable status"},
    )
    assert outcome.considered == 2, outcome
    assert outcome.armed == 1, (outcome, messages)
    assert [number for number, _ in outcome.arm_failures] == [451], outcome
    assert outcome.exit_code == 1, messages
    assert any("PR #451: arm-failed" in m for m in messages), messages
    assert any("PR #452: armed head" in m for m in messages), messages
    assert any("arm-failure summary: PR #451" in m for m in messages), messages
    assert not [m for m in messages if m.startswith("::error")], messages
    assert outcome.capability == CAN_ARM, outcome

    # (6) MID-SWEEP CAPABILITY DENIAL — stop after ONE ::error (never one per PR), leave
    # the remaining candidates untouched, and still exit non-zero.
    third = fixture(number=453)
    fake, messages, outcome = exercise(
        [first, second, third],
        mutation_errors={number: DENIAL_TEXT for number in (451, 452, 453)},
    )
    assert len(graphql_calls(fake)) == 1, fake.calls
    assert outcome.capability == CANNOT_ARM, outcome
    assert outcome.armed == 0, outcome
    assert [number for number, _ in outcome.arm_failures] == [451], outcome
    assert outcome.exit_code == 1, messages
    emitted = [message for message in messages if message.startswith("::error")]
    assert len(emitted) == 1, emitted
    assert "contents: write" in emitted[0], emitted
    assert sum("arm capability lost" in m for m in messages) == 2, messages

    # (7) EXIT SEMANTICS — an arm failure must never exit 0, in EITHER mode. A capability
    # failure is not a transient, so --mode sweep must not convert it into ::warning + 0.
    assert ArmOutcome().exit_code == 0
    assert ArmOutcome(arm_failures=[(1, "x")]).exit_code == 1
    assert ArmOutcome(capability=CANNOT_ARM).exit_code == 1
    assert ArmOutcome(capability=INCONCLUSIVE).exit_code == 0
    for mode in ("sweep", "event"):
        blocked_fake = FakeGh(clean, auto_merge_allowed=False)
        emitted = []
        code = run_sweep(
            AutoArmer("sparq-org/sparq", "main", blocked_fake, emitted.append),
            log=emitted.append,
            mode=mode,
        )
        assert code == 1, (mode, code, emitted)
        assert len([m for m in emitted if m.startswith("::error")]) == 1, emitted
        assert not [m for m in emitted if m.startswith("::warning")], emitted

    # (8) The standalone probe entry point agrees with the in-sweep verdict.
    blocked = AutoArmer(
        "sparq-org/sparq",
        "main",
        FakeGh(clean, auto_merge_allowed=False),
        lambda _message: None,
    )
    assert blocked.probe_arm_capability().status == CANNOT_ARM
    assert probe_arm_capability_exit(blocked) == 1
    healthy = AutoArmer(
        "sparq-org/sparq", "main", FakeGh(clean), lambda _message: None
    )
    assert healthy.probe_arm_capability().status == CAN_ARM
    assert probe_arm_capability_exit(healthy) == 0

    # ---------------------------------------------------------------------------------
    # [OPUS-5] #3766 — STICKY FAILURE PRECEDENCE:
    #     collected-failure > transient-exhaustion > clean.
    # The false pass this fixes (reproduced by the cross-provider review): PR #451's arm
    # FAILED and was collected, then PR #452's pre-arm refresh exhausted its bounded
    # retries; that exhaustion unwound run() and run_sweep's lenient handler reported
    # ::warning + exit 0 — discarding a real arm failure. The collected verdict must be
    # untouchable by anything that happens for a LATER candidate.
    # ---------------------------------------------------------------------------------

    def sequence(*, arm_fails: bool, mode: str):
        """PR #451 (optionally arm-failing), then PR #452 whose refresh exhausts retries."""
        sequence_fake = FakeGh(
            [fixture(number=451), fixture(number=452)],
            mutation_errors=(
                {451: "gh api graphql failed: Pull request is in unstable status"}
                if arm_fails
                else None
            ),
        )

        def read_452_exhausts(argv: list[str]) -> str:
            if argv[:2] == ["pr", "view"] and argv[2] == "452":
                raise gh_retry.GhTransientExhausted("gh pr view: HTTP 504 (3 attempts)")
            return sequence_fake(argv)

        lines: list[str] = []
        armer = AutoArmer(
            "sparq-org/sparq",
            "main",
            sequence_fake,
            lines.append,
            gh_read=read_452_exhausts,
        )
        try:
            return lines, run_sweep(armer, log=lines.append, mode=mode)
        except GhError as error:
            return lines, error

    # (9) THE #3766 SEQUENCE — arm failure on A, exhausted transient on a LATER B.
    lines, code = sequence(arm_fails=True, mode="sweep")
    assert code == 1, (
        "a COLLECTED arm failure must dominate a LATER candidate's exhausted transient "
        f"(code={code}, {lines})"
    )
    assert any("PR #451: arm-failed" in line for line in lines), lines
    assert any("arm-failure summary: PR #451" in line for line in lines), lines
    assert any("PR #452: skipped transient-exhausted" in line for line in lines), lines
    assert any("precedence: collected-failure" in line for line in lines), lines
    # The lenient ::warning must NOT be emitted while a real failure stands.
    assert not [line for line in lines if line.startswith("::warning")], lines

    # (10) CONTROL — the lenient policy is intact where it IS correct: a sweep whose ONLY
    # problem is an exhausted transient still exits 0 with exactly one ::warning, and the
    # candidates around it are still armed.
    lines, code = sequence(arm_fails=False, mode="sweep")
    assert code == 0, (code, lines)
    warned = [line for line in lines if line.startswith("::warning")]
    assert len(warned) == 1, warned
    assert "452" in warned[0], warned
    assert any("PR #451: armed head" in line for line in lines), lines

    # (11) CONTROL — event mode is unchanged: a collected arm failure is non-zero, and an
    # exhausted transient alone still fails loudly (no per-PR cron backstop).
    lines, code = sequence(arm_fails=True, mode="event")
    assert code == 1, (code, lines)
    lines, result = sequence(arm_fails=False, mode="event")
    assert isinstance(result, GhError) and "event-mode" in str(result), (result, lines)

    # (12) The precedence rule itself, at the seam: a collected failure outranks a
    # transient in BOTH modes; a transient alone is mode-dependent; clean is 0.
    both = ArmOutcome(
        arm_failures=[(451, "unstable")], transient_exhaustions=[(452, "HTTP 504")]
    )
    assert both.hard_failed and both.exit_code == 1, both
    for mode in ("sweep", "event"):
        assert arm_exit_code(both, mode=mode, log=lambda _m: None) == 1, mode
    transient_only = ArmOutcome(transient_exhaustions=[(452, "HTTP 504")])
    assert not transient_only.hard_failed and transient_only.exit_code == 0
    assert arm_exit_code(transient_only, mode="sweep", log=lambda _m: None) == 0
    try:
        arm_exit_code(transient_only, mode="event", log=lambda _m: None)
    except GhError as error:
        assert "event-mode" in str(error), error
    else:
        raise AssertionError("event-mode transient exhaustion must not exit 0")
    assert ArmOutcome().transient_detail is None
    assert ArmOutcome(sweep_transient="HTTP 504").transient_detail == "HTTP 504"

    # (13) FAIL CLOSED — an exhausted transient raised by the ARM MUTATION itself leaves the
    # arm's outcome UNKNOWN, so it must be a per-PR failure (exit 1), never a lenient
    # warning-class missed cycle. The one-shot runner cannot raise this today; the handler
    # exists so a future refactor cannot open a second false-pass route.
    mutation_fake = FakeGh(clean)

    def gh_arm_exhausts(argv: list[str]) -> str:
        if argv[:2] == ["api", "graphql"] and not is_capability_query(argv):
            raise gh_retry.GhTransientExhausted("gh api graphql: HTTP 504 (3 attempts)")
        return mutation_fake(argv)

    lines = []
    code = run_sweep(
        AutoArmer(
            "sparq-org/sparq", "main", gh_arm_exhausts, lines.append,
            gh_read=mutation_fake,
        ),
        log=lines.append,
        mode="sweep",
    )
    assert code == 1, (code, lines)
    assert not [line for line in lines if line.startswith("::warning")], lines
    assert any("transient exhaustion on the arm mutation" in line for line in lines), lines

    # (14) END TO END through main() — the precedence must hold for the real entry point,
    # not only for the seam the assertions above call directly.
    import io
    from contextlib import redirect_stdout

    e2e = FakeGh(
        [fixture(number=451), fixture(number=452)],
        mutation_errors={451: "gh api graphql failed: Pull request is in unstable status"},
    )

    def e2e_read(argv: list[str]) -> str:
        if argv[:2] == ["pr", "view"] and argv[2] == "452":
            raise gh_retry.GhTransientExhausted("gh pr view: HTTP 504 (3 attempts)")
        return e2e(argv)

    saved_argv = sys.argv
    saved_read = globals()["run_gh_read"]
    saved_gh = globals()["run_gh"]
    captured = io.StringIO()
    try:
        globals()["run_gh_read"] = e2e_read
        globals()["run_gh"] = e2e
        sys.argv = ["auto-arm.py", "--repo", "sparq-org/sparq", "--mode", "sweep"]
        with redirect_stdout(captured):
            e2e_code = main()
    finally:
        sys.argv = saved_argv
        globals()["run_gh_read"] = saved_read
        globals()["run_gh"] = saved_gh
    assert e2e_code == 1, (e2e_code, captured.getvalue())
    assert "arm-failure summary: PR #451" in captured.getvalue(), captured.getvalue()
    assert "::warning" not in captured.getvalue(), captured.getvalue()

    # ---------------------------------------------------------------------------------
    # [OPUS-5] #3776 — AN EVENT-TRIGGERED RUN JUDGES THE PR THAT TRIGGERED IT.
    # The live shape, replayed: two review:pass PRs, one of them (#3434) permanently
    # un-armable because it modifies a workflow file and the arm token has no
    # `workflows: write` (#3777). Pre-#3776 an event run swept BOTH, so #3434's failure
    # redded the GATING `arm reviewed PRs` check on #2521's label event — and on every
    # other PR's. The DISCRIMINATION is the test: an UNRELATED PR's failure must be
    # invisible, while the TRIGGERING PR's own failure must still be fatal.
    # ---------------------------------------------------------------------------------
    assert not is_arm_denial(WORKFLOWS_DENIAL), (
        "the workflows-permission refusal is a PER-PR condition, not a token-capability "
        "denial: misclassifying it would latch capability_lost and skip every other PR"
    )

    def scoped(*, scope_pr: int | None, mode: str, failing: int | None = 3434):
        """Two review:pass PRs — #2521 arms cleanly, #3434 can never be armed."""
        scoped_fake = FakeGh(
            [fixture(number=2521), fixture(number=3434)],
            mutation_errors={failing: WORKFLOWS_DENIAL} if failing else None,
        )
        lines: list[str] = []
        armer = AutoArmer(
            "sparq-org/sparq", "main", scoped_fake, lines.append, scope_pr=scope_pr
        )
        try:
            return scoped_fake, lines, run_sweep(armer, log=lines.append, mode=mode)
        except GhError as error:
            return scoped_fake, lines, error

    # (15) HALF ONE — an un-armable UNRELATED PR must NOT fail an event-mode run for #2521.
    scoped_fake, lines, code = scoped(scope_pr=2521, mode="event")
    assert code == 0, (
        "an event-mode run for PR #2521 must not fail because a DIFFERENT PR (#3434) "
        f"cannot be armed (code={code}, {lines})"
    )
    assert any("PR #2521: armed head" in line for line in lines), lines
    assert any("PR #2521 ONLY" in line for line in lines), lines
    assert any("considered=1" in line and "scope=pr-2521" in line for line in lines), lines
    # Not merely excused — never even READ, so nothing about it can reach this outcome.
    assert not [line for line in lines if "3434" in line], lines
    assert not [call for call in scoped_fake.calls if "3434" in " ".join(call)], (
        scoped_fake.calls
    )

    # (16) HALF TWO, THE DISCRIMINATION — the TRIGGERING PR's own arm failure is still a
    # hard non-zero exit. Narrowing WHICH PRs a run answers for must not weaken
    # accountability for the one it does.
    scoped_fake, lines, code = scoped(scope_pr=3434, mode="event")
    assert code == 1, (
        f"the triggering PR's OWN arm failure must still red the run (code={code}, {lines})"
    )
    assert any("PR #3434: arm-failed" in line for line in lines), lines
    assert any("arm-failure summary: PR #3434" in line for line in lines), lines
    assert not [line for line in lines if line.startswith("::warning")], lines
    assert not [call for call in scoped_fake.calls if "2521" in " ".join(call)], (
        scoped_fake.calls
    )

    # (17) SWEEP MODE IS STILL WHOLE-REPO. Without this the scoping fix could silently
    # become "event mode does nothing and nothing else covers the repository".
    scoped_fake, lines, code = scoped(scope_pr=None, mode="sweep", failing=None)
    assert code == 0, (code, lines)
    assert sum("armed head" in line for line in lines) == 2, lines
    assert any("considered=2" in line and "scope=all" in line for line in lines), lines
    assert any("whole-repo sweep" in line for line in lines), lines
    # ... and the whole-repo sweep stays ACCOUNTABLE for every PR in it: #3434's failure
    # reds the sweep even though it arms #2521 fine.
    scoped_fake, lines, code = scoped(scope_pr=None, mode="sweep")
    assert code == 1, (code, lines)
    assert any("PR #2521: armed head" in line for line in lines), lines
    assert any("PR #3434: arm-failed" in line for line in lines), lines

    # (18) #3766 STICKY FAILURE, NOW WITHIN SCOPE. The triggering PR's arm fails and then
    # its OWN race diagnostic exhausts its bounded retries: the earned failure must stand
    # (exit 1, no lenient ::warning). Scope narrows accountability; it never softens it.
    sticky_fake = FakeGh(
        [fixture(number=3434)], mutation_errors={3434: WORKFLOWS_DENIAL}
    )
    sticky_views: list[list[str]] = []

    def diagnostic_exhausts(argv: list[str]) -> str:
        if argv[:2] == ["pr", "view"] and argv[2] == "3434":
            sticky_views.append(argv)
            # 1: candidates(), 2: the pre-arm refresh, 3: the post-failure diagnostic.
            if len(sticky_views) >= 3:
                raise gh_retry.GhTransientExhausted("gh pr view: HTTP 504 (3 attempts)")
        return sticky_fake(argv)

    lines = []
    code = run_sweep(
        AutoArmer(
            "sparq-org/sparq",
            "main",
            sticky_fake,
            lines.append,
            gh_read=diagnostic_exhausts,
            scope_pr=3434,
        ),
        log=lines.append,
        mode="event",
    )
    assert len(sticky_views) == 3, sticky_views
    assert code == 1, (code, lines)
    assert not [line for line in lines if line.startswith("::warning")], lines
    assert any("race diagnostic could not resolve" in line for line in lines), lines
    assert any("arm-failure summary: PR #3434" in line for line in lines), lines

    # (19) THE CALL SITE, END TO END THROUGH main(). The scoping is only real if the entry
    # point applies it in event mode and does NOT apply one in sweep mode — exactly the
    # production-call-site class a unit test on candidates() would miss.
    def through_main(*argv_tail: str, environ: dict[str, str] | None = None):
        call_fake = FakeGh(
            [fixture(number=2521), fixture(number=3434)],
            mutation_errors={3434: WORKFLOWS_DENIAL},
        )
        saved = (sys.argv, globals()["run_gh_read"], globals()["run_gh"], dict(os.environ))
        buffer = io.StringIO()
        try:
            globals()["run_gh_read"] = call_fake
            globals()["run_gh"] = call_fake
            if environ is not None:
                os.environ.clear()
                os.environ.update(environ)
            sys.argv = ["auto-arm.py", "--repo", "sparq-org/sparq", *argv_tail]
            with redirect_stdout(buffer):
                return main(), buffer.getvalue()
        finally:
            sys.argv, globals()["run_gh_read"], globals()["run_gh"] = saved[:3]
            os.environ.clear()
            os.environ.update(saved[3])

    scoped_code, scoped_out = through_main(
        "--mode", "event", "--pr", "2521", environ={}
    )
    assert scoped_code == 0, (scoped_code, scoped_out)
    assert "scope=pr-2521" in scoped_out and "3434" not in scoped_out, scoped_out
    assert "::warning" not in scoped_out, scoped_out
    # The stale-ref path: NO --pr, but the runner environment names the PR. This is what
    # scopes PR refs whose frozen workflow snapshot cannot pass --pr at all.
    stale_code, stale_out = through_main(
        "--mode", "event", environ={"GITHUB_REF": "refs/pull/2521/merge"}
    )
    assert stale_code == 0, (stale_code, stale_out)
    assert "scope=pr-2521" in stale_out and "3434" not in stale_out, stale_out
    assert "could not identify the triggering PR" not in stale_out, stale_out
    # Undeterminable => loud ::warning + the documented whole-repo fallback, never a hard
    # failure (that would brick the gating check for the stale refs this issue is about).
    fallback_code, fallback_out = through_main("--mode", "event", environ={})
    assert fallback_code == 1, (fallback_code, fallback_out)
    assert "could not identify the triggering PR" in fallback_out, fallback_out
    assert "scope=all" in fallback_out, fallback_out
    # SWEEP through main() stays whole-repo even when the environment names a PR.
    sweep_code, sweep_out = through_main(
        "--mode", "sweep", environ={"GITHUB_REF": "refs/pull/2521/merge"}
    )
    assert sweep_code == 1, (sweep_code, sweep_out)
    assert "scope=all" in sweep_out and "considered=2" in sweep_out, sweep_out
    assert "arm-failure summary: PR #3434" in sweep_out, sweep_out

    # (20) SCOPE RESOLUTION, source by source. Explicit beats payload beats ref; a typo in
    # the wiring is loud rather than a silent whole-repo sweep.
    import tempfile

    assert resolve_scope_pr("2521", {}) == 2521
    assert resolve_scope_pr(" 2521 ", {}) == 2521
    assert resolve_scope_pr(2521, {}) == 2521
    assert resolve_scope_pr("", {}) is None
    assert resolve_scope_pr(None, {}) is None
    assert resolve_scope_pr("", {"GITHUB_REF": "refs/pull/3434/merge"}) == 3434
    assert resolve_scope_pr("", {"GITHUB_REF": "refs/pull/3434/head"}) == 3434
    assert resolve_scope_pr("", {"GITHUB_REF": "refs/heads/main"}) is None
    assert resolve_scope_pr("42", {"GITHUB_REF": "refs/pull/3434/merge"}) == 42
    for rejected in ("main", "-5", "3434.0", "refs/pull/7/merge", "  "):
        try:
            resolve_scope_pr(rejected, {})
        except ValueError:
            pass
        else:
            if str(rejected).strip():
                raise AssertionError(f"--pr {rejected!r} must be rejected loudly")
    with tempfile.TemporaryDirectory() as scope_tmp:
        payload_path = f"{scope_tmp}/event.json"
        with open(payload_path, "w", encoding="utf-8") as handle:
            json.dump({"pull_request": {"number": 777}}, handle)
        assert (
            resolve_scope_pr(
                "",
                {"GITHUB_EVENT_PATH": payload_path, "GITHUB_REF": "refs/pull/3434/merge"},
            )
            == 777
        ), "the event payload is more specific than the ref"
        assert resolve_scope_pr("9", {"GITHUB_EVENT_PATH": payload_path}) == 9
        with open(payload_path, "w", encoding="utf-8") as handle:
            json.dump({"schedule": "4,14,24,34,44,54 * * * *"}, handle)
        assert resolve_scope_pr("", {"GITHUB_EVENT_PATH": payload_path}) is None
        with open(payload_path, "w", encoding="utf-8") as handle:
            handle.write("{not json")
        assert resolve_scope_pr("", {"GITHUB_EVENT_PATH": payload_path}) is None
        assert (
            resolve_scope_pr(
                "", {"GITHUB_EVENT_PATH": f"{scope_tmp}/missing.json"}
            )
            is None
        )
    try:
        resolve_scope_pr("main", {})
    except ValueError as error:
        assert "--pr must be a PR number" in str(error), error
    else:
        raise AssertionError("a non-numeric --pr must be loud, never a whole-repo sweep")

    print("auto-arm self-test: PASS")


def probe_arm_capability_exit(armer: AutoArmer) -> int:
    """Run ONLY the startup probe: 0 = armable (or inconclusive), 1 = provably not.

    Wired as its own workflow step so the job reds at second three with one actionable
    ::error instead of failing per-PR on every label event, forever.
    """
    verdict = armer.probe_arm_capability()
    armer.log(f"[{PROGRAM}] arm-capability probe: {verdict.status} — {verdict.detail}")
    if verdict.blocks_sweep:
        armer.fail_capability(f"startup arm-capability probe failed: {verdict.detail}")
        return 1
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repo", help="owner/repository to sweep")
    parser.add_argument("--default-branch", default="main")
    parser.add_argument(
        "--mode",
        choices=("sweep", "event"),
        default="sweep",
        help=(
            "sweep: periodic cron over the WHOLE repository (exhausted transient reads "
            "=> ::warning + exit 0, the cron backstop covers a missed cycle). event: "
            "label-triggered arm scoped to the TRIGGERING PR ALONE (--pr / #3776), with "
            "no per-PR backstop — so any failure of THAT PR, incl. exhausted transients, "
            "exits non-zero so the arm is visibly red and retried."
        ),
    )
    parser.add_argument(
        "--pr",
        default="",
        help=(
            "the number of the PR whose label event triggered this run. Scopes an "
            "event-mode run to that PR ALONE (#3776) — an unrelated PR that cannot be "
            "armed must never red a GATING check on this one. Empty in sweep mode; if it "
            "is empty in event mode the number is derived from the runner environment "
            "(see resolve_scope_pr), which is what scopes already-stale PR refs too."
        ),
    )
    parser.add_argument("--self-test", action="store_true")
    parser.add_argument(
        "--probe-arm-capability",
        action="store_true",
        help="only verify the token can enable auto-merge here; arm nothing",
    )
    args = parser.parse_args()

    if args.self_test:
        self_test()
        return 0
    if not args.repo:
        parser.error("--repo is required unless --self-test is used")
    # [OPUS-5] #3776 THE CALL SITE. Scope is applied in EVENT mode only: the periodic sweep
    # is the whole-repo backstop and must never be narrowed, or per-PR scoping would leave
    # nothing covering the repository at all.
    scope_pr: int | None = None
    if args.mode == "event":
        try:
            scope_pr = resolve_scope_pr(args.pr, os.environ)
        except ValueError as error:
            parser.error(str(error))
        if scope_pr is None:
            print(SCOPE_UNRESOLVED_WARNING)
    elif str(args.pr).strip():
        print(
            f"[{PROGRAM}] ignoring --pr {args.pr} in sweep mode: the periodic sweep is "
            "the whole-repo backstop by design (#3776)"
        )
    # `run_gh` is passed EXPLICITLY (not left to the default argument) so the self-test can
    # substitute both runners at the module level and drive main() end to end (#3766).
    armer = AutoArmer(
        args.repo,
        args.default_branch,
        run_gh,
        gh_read=run_gh_read,
        scope_pr=scope_pr,
    )
    if args.probe_arm_capability:
        return probe_arm_capability_exit(armer)
    return run_sweep(armer, mode=args.mode)


if __name__ == "__main__":
    sys.exit(main())
