#!/usr/bin/env python3
"""Bridge line-anchored ``VERDICT:`` review comments to the ``review:pass`` label.

WHY THIS EXISTS
---------------
The arming chain (``scripts/auto-arm.py`` -> ``scripts/rearm-sweeper.py`` ->
``scripts/batch-merge.py``) admits a PR on exactly one predicate: the ``review:pass``
LABEL.  The review lane's actual output artefact is a line-anchored ``VERDICT: pass``
COMMENT, and the standing review brief forbids review agents from touching labels.
Nothing in the repository ever ADDS ``review:pass`` — two workflows only REMOVE it — so
the comment -> label hop was performed exclusively by an out-of-repo orchestrator
session.  A review that lands while no orchestrator is watching is simply lost, and the
PR sits green, CLEAN and unarmed forever.  A second, related hole: a green PR that has
NO verdict (or whose verdict is bound to a superseded head) carries no ``review:*``
label at all, so it is invisible to every automated lane — auto-arm needs
``review:pass``, the fast-fix ring only fires on FAILING CI, the review dispatchers work
off ``review:needs``.

This bridge closes both holes and adds NO new arming authority: it only writes labels,
and the existing, already-tested auto-arm policy still performs the arm with its own
``expectedHeadOid`` CAS and its own security-label exclusions.

FAIL-CLOSED HEAD BINDING (the load-bearing property)
----------------------------------------------------
A verdict is honoured as a PASS only when ALL of the following hold; no missing,
malformed or unreadable input can ever produce a pass — or leave an older one standing:

* the comment's last non-blank line is exactly ``VERDICT: pass`` / ``VERDICT: fail``
  (optionally bold-wrapped) — a mention of the phrase anywhere else does not count;
* the comment body contains the CURRENT head SHA in FULL 40-hex form as a standalone
  token — a short prefix, or a SHA that is merely an ancestor, does NOT bind.  This is
  an OCCURRENCE test, deliberately: a head SHA quoted inside a commit URL or a
  ``diff --git`` index line also binds.  Binding is only what PAIRS a comment to a head;
  what makes it a *verdict* is the trusted author's trailing ``VERDICT:`` line;
* the comment's ``author_association`` is OWNER / MEMBER / COLLABORATOR — an unknown or
  drive-by association is not a reviewer.

Among the head-bound verdicts, the LATEST BY ``created_at`` wins, so a retraction
("...actually, VERDICT: fail") posted after a pass at the same head defeats the pass.
Ordering deliberately ignores ``updated_at``: ordering by an edit timestamp would let
someone edit an OLD pass comment to jump it ahead of a NEWER fail and defeat the
retraction.  ``created_at`` is immutable.

AMBIGUOUS VERDICTS (strict-alone is fail-OPEN in composition)
--------------------------------------------------------------
Strictness that merely DISCARDS a malformed line is fail-closed in isolation and
fail-OPEN in composition.  If a reviewer's retraction reads ``VERDICT: FAIL`` or
``VERDICT: fail (retracting my pass)``, discarding it leaves an OLDER, strictly-formatted
pass as the newest surviving verdict — and the superseded pass promotes.  Because review
agents are forbidden from touching labels, a comment is a reviewer's ONLY retraction
channel, so this is the load-bearing path.

So a last non-blank line that is verdict-SHAPED (``^VERDICT\b``, any case, optionally
bold-wrapped) but does not parse strictly yields ``AMBIGUOUS`` rather than ``None``.  An
ambiguous verdict never promotes, defeats any earlier pass at the same head, and REMOVES
an existing ``review:pass``: guessing the polarity of an unparseable retraction is worse
than refusing to arm.  A verdict-shaped line that is quoted, fenced, blockquoted or
bulleted is NOT verdict-shaped for this purpose (it is a mention, not a declaration), so
the instruction that tells reviewers what to write still cannot influence anything.

The hole is CHANNEL-independent, so PR REVIEW bodies are read too — on the same GraphQL
page as the PR itself, at no extra API call.  They are WITHHOLD-ONLY: a review body can
retract or suppress a pass, never grant one.  Reading one as a pass would extend arming
authority into a channel the standing review brief does not mandate, so including them
can only ever SHRINK the promote-set.

AUTHOR XOR REVIEWER (the second withhold-only axis)
---------------------------------------------------
Head binding pairs a verdict to a HEAD.  It says nothing about WHO wrote it, and until
this guard existed nothing in the arming chain compared the verdict's author to the PR's
authorship at all: a PR's own author could post ``VERDICT: pass`` on their own PR and the
bridge would promote it to ``review:pass``, after which ``auto-arm`` arms it.  MEASURED on
the live repository, not hypothesised — see the ``[OPUS-5]`` note below.

The identity axis used is the PR's COMMIT-AUTHORSHIP set at the reviewed head:

* every commit author, co-author and committer GitHub resolved to an account on the
  commits that make up the head, plus
* every ``Co-authored-by:`` trailer whose address is a ``…@users.noreply.github.com``
  form, which names an account GitHub itself may have failed to resolve.

WHO OPENED THE PR IS DELIBERATELY *NOT* IN THAT SET, and that exclusion is the whole
design.  Every local agent — the one that writes the code and the one that reviews it —
acts under the SAME operator login, so that login lands on the PR-author field of work it
did not write.  Refusing on it rejects the normal, legitimate review path: sparq#4331 and
#4386 were both independent cross-agent reviews (verified out of band; on the wire,
``COMMIT authorship=['claude'], jeswr_committed=False``), and an opener-based rule called
both self-approvals.  Since 29 of 34 open non-draft PRs are unreachable by the automated
review lane, a hand-dispatched agent verdict is their ONLY route to a review, so refusing
it trades a security hole for a total arming stall.  ``pr.opener`` is therefore carried
and REPORTED (``::warning``, ``opener_verdicts=N``) but never changes a decision.

Commit authorship is kept because it is the one part that is provable on the wire and is
already repository policy: a verdict is void if its author contributed commits to the
head being reviewed.

Logins are normalised (case-folded, a trailing ``[bot]`` stripped) because GitHub reports
the SAME App inconsistently: GraphQL ``PullRequest.author.login`` says
``sparq-orchestrator`` while ``GitActor.user.login`` and the REST comment author both say
``sparq-orchestrator[bot]``.  An un-normalised comparison would silently never match a bot
self-review — a guard that reads as present and is not.

A verdict from inside that set is WITHHOLD-ONLY, exactly like a review body: its ``fail``
or AMBIGUOUS still retracts (refusing a contributor's own retraction would be fail-open,
and nobody falsely fails their own PR), but its ``pass`` can never GRANT the attestation.
If the contributor set cannot be read COMPLETELY, no verdict may grant at all.

WHAT THIS AXIS CANNOT DISTINGUISH — stated because a guard whose limits are unwritten
gets mistaken for a stronger one.  It proves the verdict's author wrote no commit at the
reviewed head.  It does NOT prove the reviewer is an independent JUDGMENT: two agent
sessions driven by the same operator share one GitHub login, and because Claude Code
commits as ``claude`` rather than as the operator, the operator's login is outside the
commit set whether it reviewed the work or wrote it.  On the wire those two cases are
IDENTICAL, so this guard cannot separate them and does not claim to.

Note what follows for any proposal to make agent PRs BOT-authored: that changes the
PR-author field, not the commenter, so it does not supply the missing signal — it removes
the last weak one and makes this guard admit every verdict unconditionally.  MEASURED
2026-07-27: that route is not even available — there is no App private key on the work
box (`gh auth status` shows only the operator's OAuth token), so subagents cannot push as
the App.  registry#759 instead adds an `orchestrator` attestation class, deliberately
INERT behind an enable interlock at the time of writing.  Plan on this axis staying
necessary-but-not-sufficient: the separation has to be created on the REVIEWER side (a
distinct reviewing principal, or a lane-written record the commenter cannot mint), and
until one exists this guard must not be weakened on the assumption that one is coming.

So this guard is NECESSARY, not SUFFICIENT, and both the refused set and the
merely-reported set are surfaced (``::warning``) rather than failing silently.

ACTIONS
-------
promote   head-bound verdict is pass, no hold label, ``review:pass`` absent -> add it.
retract   head-bound verdict is fail (or AMBIGUOUS) and ``review:pass`` is present ->
          remove it.  Requires POSITIVE evidence (a head-bound verdict-shaped line from
          a trusted reviewer).  Absence of a verdict NEVER retracts, so a label an
          orchestrator applied by hand is never fought.  Deleting the pass COMMENT is
          therefore not a retraction gesture the bridge can see — post a fail instead.
flag      green + mergeable + non-draft + no head-bound verdict + no ``review:*`` label
          -> add ``review:unreviewed``.  Purely informational: no arming predicate
          anywhere consumes this label, so it can never block or cause a merge.
unflag    ``review:unreviewed`` is stale (a verdict or another ``review:*`` label
          arrived) -> remove it.
"""

# [OPUS-5] sparq-org/sparq — maintainer observation 2026-07-26: "there seem to be a
# number of green, ready PRs that are not in the merge queue".  Measured cause: the
# review write-path (a VERDICT comment) and the arming read-predicate (the review:pass
# label) were joined only by a human.  registry#700 records the same mismatch class.
#
# [OPUS-5] 2026-07-26, AUTHOR-XOR-REVIEWER.  Census over all 119 open PRs (paginated,
# totalCount cross-checked): 16 head-bound verdict comments from a trusted association;
# 11 of the 16 posted by the login that also OPENED the PR; only 3 of the 16 by someone
# who wrote a commit at that head.  PR authorship splits 33 `jeswr` / 86
# `sparq-orchestrator`.
#
# The opener signal is NOT a self-approval signal here, and that correction is the reason
# this module refuses on commit authorship ONLY.  Out-of-band dispatch records show
# sparq#4331 and #4386 — both `jeswr`-opened with a `jeswr` head-bound pass — were
# genuine independent cross-agent reviews.  On the wire they are indistinguishable from a
# self-approval, because every local agent session (author AND reviewer) acts under the
# one operator login while Claude Code commits under `claude`.  An opener-based refusal
# would have rejected both, and with 29 of 34 open non-draft PRs unreachable by the
# automated review lane, a hand-dispatched verdict is their only route to review.
#
# Three candidate axes, all measured:
#   * `comment.user.login != pr.user.login`.  Fires on 11 of 16 but is UNCONDITIONALLY
#     TRUE for the 86 bot-opened PRs (the App never comments), AND its 11 hits are not
#     known to be self-approvals.  Rejected as a refusal; kept as an advisory report.
#   * commit authorship at the head.  Fires on 3 of 16.  Narrow, but it is the only part
#     provable on the wire and it matches existing repository policy (a verdict is void
#     if its author contributed commits to the reviewed head).  ADOPTED as the refusal.
#   * a registry ledger record as the grant predicate.  Measured against the `ledger`
#     branch: 29 of 34 open non-draft PRs have NO record (all 29 `jeswr`-opened; all 5
#     with a record bot-opened).  Requiring one is a total arming stall.  Rejected.
#     The records also carry no reviewer provenance -- `host_envelope` is exactly
#     {pr, repo, reviewed_sha, round} -- so they cannot attest WHO reviewed either.
#
# The adopted axis is necessary, NOT sufficient (see the module docstring): it cannot
# separate two agent sessions sharing one operator login when the reviewer committed
# nothing.  Closing that needs a distinct REVIEWER principal, or a lane-written record the
# commenter cannot mint.  Making agent PRs bot-authored does NOT close it -- that moves the
# author field, not the commenter, and would make this guard admit everything.

from __future__ import annotations

import argparse
import json
import re
import subprocess
import sys
from dataclasses import dataclass, field
from typing import Callable, Iterable, Sequence

import gh_retry
import review_lane_reach


PROGRAM = "verdict-bridge"
REVIEW_ATTESTATION = "review:pass"
UNREVIEWED_LABEL = "review:unreviewed"

# Mirrors auto-arm.py's HUMAN_OR_TRUST_LABELS + REVIEW_CHANGES_LABELS.  Promoting a PR
# that auto-arm would refuse anyway would only paint a misleading attestation onto it.
HOLD_LABELS = frozenset(
    {
        "trust-surface",
        "trust:untrusted",
        "needs:user",
        "review:needs-user",
        "review:changes",
        "review:needs",
        "area:sparq-zk",
        "area:sparq-mpc",
        "area:sparq-trust",
    }
)
# Every reviewer identity GitHub reports for a comment author. Anything outside this set
# (CONTRIBUTOR, FIRST_TIME_CONTRIBUTOR, NONE, MANNEQUIN, absent, misspelt) is not a
# reviewer for our purposes and its verdict is discarded.
TRUSTED_ASSOCIATIONS = frozenset({"OWNER", "MEMBER", "COLLABORATOR"})

VERDICT_RE = re.compile(r"^\s*(?:\*\*)?VERDICT:\s*(pass|fail)(?:\*\*)?\s*$")
# `Co-authored-by: Name <1234+login@users.noreply.github.com>` / `<login@users.noreply…>`.
# GitHub resolves most co-authors to an account on the GraphQL `authors` connection; this
# recovers the ones it did not, which would otherwise be missing from the contributor set.
COAUTHOR_NOREPLY_RE = re.compile(
    r"^\s*co-authored-by:.*<(?:[0-9]+\+)?([A-Za-z0-9](?:[A-Za-z0-9-]*[A-Za-z0-9])?)"
    r"@users\.noreply\.github\.com>\s*$",
    re.IGNORECASE | re.MULTILINE,
)
# Verdict-SHAPED but not strictly parseable: "VERDICT: FAIL", "VERDICT: fail (retracting
# my pass)", "VERDICT - pass".  Anchored at the START of the last non-blank line, so a
# blockquoted (`> `), fenced, bulleted or back-ticked MENTION is not verdict-shaped and
# keeps reading as "no verdict", exactly as before.
VERDICT_SHAPE_RE = re.compile(r"^\s*(?:\*\*|__)?VERDICT\b", re.IGNORECASE)
FULL_SHA_RE = re.compile(r"(?<![0-9a-fA-F])([0-9a-fA-F]{40})(?![0-9a-fA-F])")

# A trusted, head-bound, verdict-shaped line whose polarity cannot be read.  Never a
# pass, and it defeats an earlier pass — see the module docstring.
AMBIGUOUS = "ambiguous"

# Heads here reach ~1181 check-runs (12 pages).  A bound keeps a pathological or looping
# response from spinning the sweep forever; exceeding it FAILS the read, never admits it.
MAX_CHECK_RUN_PAGES = 60


class GhError(RuntimeError):
    """A GitHub CLI command failed."""


@dataclass(frozen=True)
class Verdict:
    """A line-anchored, head-bound review verdict."""

    value: str  # "pass" | "fail" | AMBIGUOUS
    author: str
    created_at: str
    comment_id: int


@dataclass(frozen=True)
class PullRequest:
    number: int
    state: str
    is_draft: bool
    base_ref: str
    head_sha: str
    mergeable: str
    gate_conclusion: str | None
    labels: frozenset[str] = field(default_factory=frozenset)
    # NORMALISED logins of everyone who wrote a COMMIT at this head.  A verdict from
    # inside this set may withhold but never grant — see the docstring.
    contributors: frozenset[str] = field(default_factory=frozenset)
    # NORMALISED login of whoever OPENED the PR.  Advisory only: never refuses a grant.
    opener: str = ""
    # False when the commit list was truncated, so the set above is a LOWER BOUND and
    # disjointness cannot be proven.  Refuses every grant; never blocks a retraction.
    contributors_complete: bool = True
    # [OPUS-5] #4677.  Whether the ONLY review producer sparq has — the registry's review
    # lane — can EVER enumerate this PR (scripts/review_lane_reach.py).  Computed once in
    # ``parse_node``; ``decide`` reads it so the pure core stays I/O-free.
    #
    # DEFAULTS FALSE, deliberately.  ``review:unreviewed`` asserts "enrolled, awaiting a
    # review", and a PR nothing can review is not awaiting one — so an unset or unreadable
    # reachability must never let the flag be written.  Under-labelling costs nothing (the
    # blind-spot alarm still censuses the PR); over-labelling is the #4677 defect itself.
    lane_reachable: bool = False
    # The one-line statement of what happens to an unreachable PR INSTEAD of a review
    # (maintainer-reviewed release / dependency PR, or simply not enrolled).  Empty for a
    # reachable PR.  Reported in the decision reason so a refusal is never silent.
    disposition: str = ""


@dataclass(frozen=True)
class Decision:
    action: str  # promote | retract | flag | unflag | none
    reason: str
    # True when a head-bound, trusted `VERDICT: pass` at this head was DISCARDED because
    # its author wrote commits at that head.  Drives the ::warning that stops a refused
    # self-approval from being a silent non-event.
    self_review: bool = False
    # True when a counted pass came from whoever OPENED the PR.  ADVISORY: reported so
    # the population stays measurable, never acted on.  See `is_opener`.
    opener_verdict: bool = False


def normalize_login(login: object) -> str:
    """Case-fold a GitHub login and strip a trailing ``[bot]``.

    Load-bearing, not cosmetic.  GitHub reports one App under TWO spellings: GraphQL
    ``PullRequest.author.login`` gives ``sparq-orchestrator`` while ``GitActor.user.login``
    and the REST issue-comment author both give ``sparq-orchestrator[bot]``.  Comparing
    the raw strings would make an App's self-review permanently invisible to this guard.
    """
    text = str(login or "").strip().lower()
    if text.endswith("[bot]"):
        text = text[: -len("[bot]")]
    return text


def commit_contributors(node: dict) -> tuple[frozenset[str], bool]:
    """(normalised COMMIT-authorship logins, completeness) for one GraphQL PR node.

    DELIBERATELY EXCLUDES the PR's opener.  Opening a PR is not writing its code: in this
    repository a hand-dispatched review agent and the authoring agent BOTH act under the
    operator's login, and the operator's login is on the PR-author field of work it did
    not write.  Treating the opener as a contributor therefore refuses legitimate reviews
    (measured: sparq#4331 and #4386 are independent cross-agent reviews whose reviewer
    committed nothing — `COMMIT authorship=['claude'], jeswr_committed=False` — and both
    were false positives of the opener rule).  ``pr.opener`` carries it for REPORTING.

    Returns ``complete=False`` when the commit connection was truncated: the set is then
    a LOWER BOUND, so a contributor could be missing from it and disjointness cannot be
    proven.  Callers must refuse to GRANT on an incomplete set — never merely warn.
    """
    people: set[str] = set()
    commits = node.get("commits") or {}
    nodes = commits.get("nodes") or []
    for entry in nodes:
        commit = (entry or {}).get("commit") or {}
        for author in ((commit.get("authors") or {}).get("nodes") or []):
            people.add(normalize_login(((author or {}).get("user") or {}).get("login")))
        people.add(
            normalize_login(((commit.get("committer") or {}).get("user") or {}).get("login"))
        )
        for match in COAUTHOR_NOREPLY_RE.finditer(str(commit.get("message") or "")):
            people.add(normalize_login(match.group(1)))
    total = commits.get("totalCount")
    complete = isinstance(total, int) and total <= len(nodes)
    return frozenset(p for p in people if p), complete


def is_contributor(login: object, pr: PullRequest) -> bool:
    """True when this comment author may not GRANT: wrote code here, or unreadable set.

    Fail-closed on BOTH unknowns.  An empty/absent comment author is treated as a
    contributor because an unattributable comment cannot be shown to be independent, and
    an incomplete contributor set makes EVERY author unprovable.
    """
    if not pr.contributors_complete:
        return True
    who = normalize_login(login)
    return not who or who in pr.contributors


def is_opener(login: object, pr: PullRequest) -> bool:
    """True when this comment author merely OPENED the PR (advisory signal only).

    NOT a refusal.  Opening a PR under a shared operator login says nothing about who
    wrote the code or who reviewed it, and refusing on it stalls the normal review path
    for the 29-of-34 open non-draft PRs the automated lane cannot reach.  Reported so the
    population stays measurable, never acted on.
    """
    who = normalize_login(login)
    return bool(who) and bool(pr.opener) and who == pr.opener


def hold_labels(labels: Iterable[str]) -> list[str]:
    """Every fail-closed hold, including the whole ``needs:`` namespace."""
    return sorted(
        label
        for label in labels
        if label in HOLD_LABELS or label.startswith("needs:")
    )


def parse_pr_argument(raw: str | None) -> int | None:
    """The parse boundary for the PR number an EVENT payload supplies.

    ``None`` means "the payload named no PR" (a plain issue comment, a check-suite with
    an empty ``pull_requests`` array) — a benign no-op.  Anything present but not a
    positive decimal integer RAISES: the value is interpolated from webhook-carried data,
    and quietly coercing it to "no PR" would turn a malformed payload into a silent
    full-repository sweep started by whoever sent it.
    """
    text = (raw or "").strip()
    if not text:
        return None
    # ASCII-only: str.isdigit() also accepts Arabic-Indic and other Unicode digit
    # forms, which int() then happily parses into a DIFFERENT number than the one a
    # reader of the log sees.
    if not (text.isascii() and text.isdigit()):
        raise ValueError(f"--pr must be a decimal pull-request number, got {raw!r}")
    number = int(text)
    if number <= 0:
        raise ValueError(f"--pr must be positive, got {number}")
    return number


def trailing_verdict(body: str | None) -> str | None:
    """``"pass"`` / ``"fail"`` / ``AMBIGUOUS`` for the LAST non-blank line, else None.

    Line-anchored on purpose: the brief mandates the verdict be the final line, and a
    substring search for "VERDICT: pass" matches the INSTRUCTION that tells reviewers to
    write it (the exact false-pass this project has already been bitten by).

    A verdict-SHAPED final line that does not parse strictly returns ``AMBIGUOUS``, NOT
    None.  Returning None would let an older, well-formed pass survive a malformed
    retraction at the same head and promote it — strict-and-discard is fail-open in
    composition.  See the module docstring.
    """
    if not body:
        return None
    for line in reversed(body.splitlines()):
        if not line.strip():
            continue
        match = VERDICT_RE.match(line)
        if match:
            return match.group(1).lower()
        return AMBIGUOUS if VERDICT_SHAPE_RE.match(line) else None
    return None


def binds_head(body: str | None, head_sha: str) -> bool:
    """True iff the body cites the CURRENT head as a standalone full 40-hex SHA.

    An OCCURRENCE test by design (a SHA inside a commit URL or a diff header binds too):
    it PAIRS a comment with a head.  What makes the comment a verdict is the trusted
    author's trailing ``VERDICT:`` line, which this does not and need not attest.
    """
    if not body or len(head_sha) != 40:
        return False
    target = head_sha.lower()
    return any(m.lower() == target for m in FULL_SHA_RE.findall(body))


def classify_verdicts(
    comments: Sequence[dict], head_sha: str, pr: PullRequest | None = None
) -> tuple[list[Verdict], list[Verdict], list[Verdict]]:
    """``(countable, self_passes, opener_passes)`` — head-bound trusted verdicts.

    ``countable`` is sorted oldest-first by immutable ``created_at`` (then comment id) so
    that editing an older comment can never reorder it ahead of a newer retraction.

    TWO withhold-only axes remove a ``pass`` from ``countable`` while leaving a ``fail``
    or AMBIGUOUS in place.  Both are one-directional on purpose: they can only ever SHRINK
    the promote-set, and a retraction from a withheld source must still land or the guard
    would be fail-open in composition.

    * ``_channel == "review"`` — a PR REVIEW body rather than an issue comment.  Reading
      one as a pass would extend arming authority into a channel the standing review
      brief does not mandate.
    * the author is in ``pr.contributors`` (or that set is unreadable) — author XOR
      reviewer.  These land in ``self_passes`` so the caller can REPORT the refusal
      instead of letting a suppressed self-approval vanish silently.

    ``pr=None`` disables only the second axis; it exists so the head-binding, ordering and
    association guards stay directly testable in isolation.
    """
    countable: list[Verdict] = []
    self_passes: list[Verdict] = []
    opener_passes: list[Verdict] = []
    for comment in comments:
        if not isinstance(comment, dict):
            continue
        value = trailing_verdict(comment.get("body"))
        if value is None:
            continue
        association = str(comment.get("author_association") or "").upper()
        if association not in TRUSTED_ASSOCIATIONS:
            continue
        if not binds_head(comment.get("body"), head_sha):
            continue
        author = (comment.get("user") or {}).get("login")
        found = Verdict(
            value=value,
            author=str(author or "?"),
            created_at=str(comment.get("created_at") or ""),
            comment_id=int(comment.get("id") or 0),
        )
        if value == "pass":
            if comment.get("_channel") == "review":
                continue
            if pr is not None and is_contributor(author, pr):
                self_passes.append(found)
                continue
            if pr is not None and is_opener(author, pr):
                # ADVISORY ONLY — still countable.  See ``is_opener``.
                opener_passes.append(found)
        countable.append(found)
    countable.sort(key=lambda v: (v.created_at, v.comment_id))
    self_passes.sort(key=lambda v: (v.created_at, v.comment_id))
    opener_passes.sort(key=lambda v: (v.created_at, v.comment_id))
    return countable, self_passes, opener_passes


def head_bound_verdict(
    comments: Sequence[dict], head_sha: str, pr: PullRequest | None = None
) -> Verdict | None:
    """The LATEST COUNTABLE head-bound verdict, or None."""
    countable, _, _ = classify_verdicts(comments, head_sha, pr)
    return countable[-1] if countable else None


def is_green_and_ready(pr: PullRequest) -> bool:
    """Green, mergeable and open for business — the population the maintainer sees."""
    return (
        not pr.is_draft
        and pr.mergeable.upper() == "MERGEABLE"
        and (pr.gate_conclusion or "").lower() == "success"
    )


def decide(pr: PullRequest, comments: Sequence[dict]) -> Decision:
    """Pure core. No I/O — every guard below is unit-tested in isolation.

    NOTE ON SCOPE: ``promote`` deliberately does NOT consult ``gate_conclusion`` or
    ``mergeable`` — it only attests that a review happened, and ``auto-arm``'s
    ``--auto`` holds a red or conflicting PR out of the queue on its own.
    ``is_green_and_ready`` therefore guards ``flag`` (visibility) only.
    """
    if pr.state.upper() != "OPEN":
        return Decision("none", f"not-open ({pr.state or 'UNKNOWN'})")
    if not pr.base_ref:
        return Decision("none", "unknown-base")

    countable, self_passes, opener_passes = classify_verdicts(comments, pr.head_sha, pr)
    verdict = countable[-1] if countable else None

    def out(action: str, reason: str) -> Decision:
        """Stamp every exit so neither a refusal nor an advisory exits silently."""
        if self_passes:
            who = ", ".join(sorted({f"@{v.author}" for v in self_passes}))
            detail = (
                "commit-authorship set unreadable"
                if not pr.contributors_complete
                else f"{who} wrote commits at this head"
            )
            reason = f"{reason}; SELF-REVIEW pass by {who} DISCARDED ({detail})"
        if opener_passes:
            # ADVISORY. The decision above is unchanged by this branch — on the wire an
            # opener-authored verdict is indistinguishable from an independent agent
            # review under the same operator login, and refusing it stalls the only
            # review path 29 of 34 open non-draft PRs have.
            who = ", ".join(sorted({f"@{v.author}" for v in opener_passes}))
            reason = (
                f"{reason}; NOTE pass by the PR opener {who} COUNTED "
                "(opener != code author; not separable on the wire)"
            )
        return Decision(
            action,
            reason,
            self_review=bool(self_passes),
            opener_verdict=bool(opener_passes),
        )

    has_attestation = REVIEW_ATTESTATION in pr.labels
    flagged = UNREVIEWED_LABEL in pr.labels

    def unreviewable(prefix: str) -> Decision:
        """The exit for a PR no review producer can ever reach.

        [OPUS-5] #4677.  ``review:unreviewed`` is applied HERE and consumed by a reviewer
        that lives in ANOTHER repository, and the two predicates had drifted apart: this
        labeller flagged four green PRs — including the first crates.io release PR — that
        the registry lane's ``enumerate_review_items`` structurally cannot enumerate. A PR
        that gets labelled but never reviewed is WORSE than one never labelled, because
        the label makes it look enrolled: it is then neither reviewed nor noticed, and
        every success rate the lane reports is computed over a population that excludes
        it.  So the flag is never written here, and WITHDRAWN where it already was — the
        withdrawal is not a "retraction on absence" (that rule protects the ``review:pass``
        ATTESTATION; this label attests nothing and blocks nothing).

        This does not abandon the PR: ``scripts/review_lane_alarm.py`` censuses exactly
        this population every tick and files the one deduped blind-spot issue.
        """
        detail = pr.disposition or "no review producer can reach this PR"
        if flagged:
            return out("unflag", f"{prefix}; withdrawing {UNREVIEWED_LABEL} — {detail}")
        return out("none", f"{prefix} — {detail}")

    if verdict is None:
        # A discarded self-review is POSITIVE evidence, not absence.  The "never retract
        # on absence" rule below exists so a hand-applied label with no comment behind it
        # is not fought — but here there IS a comment behind it and its author wrote
        # commits at this very head, which is already repository policy for voiding a
        # verdict.  Preventing the next promotion without withdrawing an attestation that
        # rests solely on such a comment would leave the exposure in place.
        #
        # SCOPE: `self_passes` is commit-authorship only.  A pass from the PR's OPENER
        # never reaches here — it stays countable — so this branch cannot withdraw a
        # label that rests on a legitimate same-login agent review.
        if has_attestation and self_passes:
            return out(
                "retract",
                f"{REVIEW_ATTESTATION} rests only on a self-review — withdrawing it",
            )
        # NEVER retract on absence: an orchestrator-applied label has no comment behind
        # it, and removing it would fight the human lane and un-arm a real review.
        if flagged:
            if not pr.lane_reachable:
                return unreviewable("flagged but unreviewable")
            return out("none", "flagged, still unreviewed")
        other_review_label = any(
            label.startswith("review:") for label in pr.labels
        )
        if other_review_label or has_attestation:
            return out("none", "already in a review lane")
        if is_green_and_ready(pr):
            if not pr.lane_reachable:
                return unreviewable("green + mergeable but unreviewable")
            return out("flag", "green + mergeable but invisible to every lane")
        return out("none", "no head-bound verdict")

    if verdict.value == AMBIGUOUS:
        # A trusted reviewer wrote a verdict-SHAPED line at THIS head that does not
        # parse.  Reading it as "no verdict" would resurrect an older pass (fail-open);
        # guessing its polarity would arm on a coin flip.  Refuse both: never promote,
        # and drop an attestation this line may well be retracting.
        if has_attestation:
            return out(
                "retract",
                f"ambiguous verdict line by {verdict.author} supersedes "
                f"{REVIEW_ATTESTATION} — polarity unreadable, refusing to arm",
            )
        if (
            not flagged
            and is_green_and_ready(pr)
            and not any(label.startswith("review:") for label in pr.labels)
        ):
            if not pr.lane_reachable:
                return unreviewable(
                    f"ambiguous verdict line by {verdict.author}, and unreviewable"
                )
            return out(
                "flag", f"ambiguous verdict line by {verdict.author} — needs a re-review"
            )
        if flagged and not pr.lane_reachable:
            return unreviewable(
                f"ambiguous verdict line by {verdict.author}, and unreviewable"
            )
        return out(
            "none", f"ambiguous verdict line by {verdict.author} — polarity unreadable"
        )

    if flagged:
        return out("unflag", f"verdict arrived ({verdict.value})")

    if verdict.value == "fail":
        if has_attestation:
            return out(
                "retract",
                f"head-bound fail by {verdict.author} supersedes {REVIEW_ATTESTATION}",
            )
        return out("none", "head-bound fail, no attestation to retract")

    # verdict.value == "pass"
    holds = hold_labels(pr.labels)
    if holds:
        return out("none", f"hold ({', '.join(holds)})")
    if has_attestation:
        return out("none", "already attested")
    if pr.is_draft:
        # auto-arm un-drafts on the label; do not attest a draft the author still owns.
        return out("none", "draft")
    return out(
        "promote", f"head-bound pass by {verdict.author} at {pr.head_sha[:12]}"
    )


# --------------------------------------------------------------------------- I/O


def run_gh(argv: list[str]) -> str:
    result = subprocess.run(["gh", *argv], capture_output=True, text=True, check=False)
    if result.returncode != 0:
        detail = result.stderr.strip() or result.stdout.strip() or "unknown gh failure"
        raise GhError(f"gh {' '.join(argv[:3])} failed: {detail}")
    return result.stdout


def run_gh_read(argv: list[str]) -> str:
    try:
        return gh_retry.run_gh_read(argv)
    except gh_retry.GhFatalError as error:
        raise GhError(str(error)) from error


# The node selection is shared by the SWEEP query and the SINGLE-PR query on purpose:
# the event path must read EXACTLY the fields the cron path reads, or it becomes a
# different (and possibly laxer) policy wearing the same name.  Pinned by
# scripts/tests/test_verdict_bridge.py::TestScopedRunAuthority.
PR_NODE_FIELDS = """
        number state isDraft baseRefName headRefOid mergeable
        # [OPUS-5] #4677: headRefName / headRepository / title are the REVIEW-LANE
        # REACHABILITY inputs (scripts/review_lane_reach.py).  If any of them stops being
        # requested here, parse_node reads it as absent and `lane_reachable` fails CLOSED
        # — the bridge stops writing `review:unreviewed`, never starts writing it wrongly.
        headRefName title
        headRepository{nameWithOwner}
        # `__typename` is the ONLY reliable machine-account signal on GraphQL: it reports
        # an App's login WITHOUT the `[bot]` suffix the registry's own gate tests for, so
        # a copied suffix test here would read as a guard while never matching.
        author{login __typename}
        labels(first:100){nodes{name} pageInfo{hasNextPage}}
        # 100 is GitHub's DOCUMENTED per-connection maximum.  A larger `last:` is
        # currently accepted (probed: `last:250` returns without an error), but resting a
        # security guard on undocumented leniency is how it breaks silently later — and
        # if the server ever capped instead of erroring, the guard would read a PARTIAL
        # contributor set.  `totalCount` makes truncation observable either way, and
        # `contributors_complete=False` then refuses every grant.  Measured ceiling on
        # the live population: 10 commits.
        commits(last:100){
          totalCount
          nodes{commit{
            message
            authors(first:20){nodes{user{login}}}
            committer{user{login}}
          }}
        }
        reviews(last:30){nodes{
          databaseId body authorAssociation submittedAt author{login}
        }}
"""

PR_LIST_QUERY = (
    """query($owner:String!,$name:String!,$cursor:String){
  repository(owner:$owner,name:$name){
    pullRequests(states:OPEN,first:50,after:$cursor){
      pageInfo{hasNextPage endCursor}
      totalCount
      nodes{"""
    + PR_NODE_FIELDS
    + """}
    }
  }
}"""
)

# Single-PR read for the EVENT path.  A comment / review / CI-conclusion webhook names
# exactly one PR, so re-sweeping all ~100 open PRs (MEASURED 8m30s, run 30221614764)
# would make the event slower than the cron it is meant to pre-empt.
PR_ONE_QUERY = (
    """query($owner:String!,$name:String!,$number:Int!){
  repository(owner:$owner,name:$name){
    pullRequest(number:$number){"""
    + PR_NODE_FIELDS
    + """}
  }
}"""
)


class VerdictBridge:
    def __init__(
        self,
        repo: str,
        default_branch: str,
        *,
        gh: Callable[[list[str]], str] = run_gh,
        gh_read: Callable[[list[str]], str] | None = None,
        log: Callable[[str], None] = print,
        dry_run: bool = False,
        max_writes: int = 25,
        only_pr: int | None = None,
    ) -> None:
        if "/" not in repo or repo.count("/") != 1 or not all(repo.split("/")):
            raise ValueError("repo must be OWNER/REPOSITORY")
        if only_pr is not None and only_pr <= 0:
            raise ValueError("--pr must be a positive pull-request number")
        self.repo = repo
        self.owner, self.name = repo.split("/", 1)
        self.default_branch = default_branch
        self.gh = gh
        self.gh_read = gh_read if gh_read is not None else gh
        self.log = log
        self.dry_run = dry_run
        self.max_writes = max_writes
        self.only_pr = only_pr

    # -- reads --------------------------------------------------------------

    def list_open(self) -> list[dict]:
        """Every open PR, EXPLICITLY paged and cross-checked against totalCount.

        `gh api --paginate` concatenates one JSON object per page with no separator, so
        a single json.loads over it raises; page by cursor instead and assert the count.
        """
        nodes: list[dict] = []
        cursor: str | None = None
        total: int | None = None
        while True:
            argv = [
                "api",
                "graphql",
                "-f",
                f"query={PR_LIST_QUERY}",
                "-F",
                f"owner={self.owner}",
                "-F",
                f"name={self.name}",
            ]
            if cursor:
                argv += ["-F", f"cursor={cursor}"]
            payload = json.loads(self.gh_read(argv))
            if payload.get("errors"):
                raise GhError(f"GraphQL returned errors: {payload['errors']}")
            page = ((payload.get("data") or {}).get("repository") or {}).get(
                "pullRequests"
            )
            if not page:
                raise GhError("GraphQL response carried no pullRequests connection")
            total = page.get("totalCount") if total is None else total
            nodes.extend(page.get("nodes") or [])
            info = page.get("pageInfo") or {}
            if not info.get("hasNextPage"):
                break
            cursor = info.get("endCursor")
            if not cursor:
                raise GhError("hasNextPage without an endCursor")
        if total is not None and len(nodes) != total:
            raise GhError(f"paged {len(nodes)} PRs but totalCount was {total}")
        return nodes

    def fetch_one(self, number: int) -> list[dict]:
        """The ONE PR an event names, in the SAME node shape ``list_open`` returns.

        Returns ``[]`` — never raises — when the PR has vanished (deleted, transferred)
        or the connection is null, because an event about a PR that no longer exists is
        nothing to do.  Every OTHER failure still raises, so a broken read can never be
        mistaken for "no work".
        """
        payload = json.loads(
            self.gh_read(
                [
                    "api",
                    "graphql",
                    "-f",
                    f"query={PR_ONE_QUERY}",
                    "-F",
                    f"owner={self.owner}",
                    "-F",
                    f"name={self.name}",
                    "-F",
                    f"number={int(number)}",
                ]
            )
        )
        if payload.get("errors"):
            raise GhError(f"GraphQL returned errors: {payload['errors']}")
        repository = (payload.get("data") or {}).get("repository")
        if repository is None:
            raise GhError("GraphQL response carried no repository")
        node = repository.get("pullRequest")
        return [node] if isinstance(node, dict) and node else []

    def check_runs(self, sha: str) -> list[dict]:
        """Every check-run for ``sha``, EXPLICITLY paged and de-duplicated by id.

        PRs here carry hundreds of check-runs; an unpaginated read silently truncates
        and can hide the `gate` run entirely, which reads as "no gate" -> not green.

        The cross-check against ``total_count`` is ONE-SIDED, and that asymmetry is
        load-bearing.  A live sweep measured ``paged 479 check-runs, total_count=473`` on
        sparq#4074: runs were being CREATED between page reads, so the list GREW under
        pagination.  Reading MORE than the first page's ``total_count`` is benign — the
        risk this guard exists for is TRUNCATION, which reads FEWER.  A two-sided
        equality check turned a routine mid-sweep re-run into a skipped PR plus a red
        workflow every cycle.  Growth also repeats entries across page boundaries, hence
        the de-duplication by run id.
        """
        runs: list[dict] = []
        seen: set = set()
        total: int | None = None
        page = 1
        while True:
            payload = json.loads(
                self.gh_read(
                    [
                        "api",
                        f"/repos/{self.repo}/commits/{sha}/check-runs"
                        f"?per_page=100&page={page}",
                    ]
                )
            )
            total = payload.get("total_count") if total is None else total
            batch = payload.get("check_runs") or []
            for run in batch:
                if isinstance(run, dict) and run.get("id") not in seen:
                    seen.add(run.get("id"))
                    runs.append(run)
            # Terminate on a SHORT page, never on `len(runs) >= total_count`: a
            # total_count read before a re-run started is an UNDERCOUNT, and stopping on
            # it would truncate exactly when the list is growing — hiding the newest
            # `gate` run, which is the one that matters.
            if len(batch) < 100:
                break
            page += 1
            if page > MAX_CHECK_RUN_PAGES:
                raise GhError(f"{sha}: check-runs exceeded {MAX_CHECK_RUN_PAGES} pages")
        if total is not None and len(runs) < total:
            # TRUNCATION — the `gate` run may be in the part we never read.  Fail closed.
            raise GhError(f"{sha}: paged only {len(runs)} check-runs, total_count={total}")
        return runs

    def gate_conclusion(self, sha: str) -> str | None:
        """Newest-run-per-name resolution of the `gate` check.

        A cancelled or superseded twin shares the name; taking any-run would read a
        stale result as the live one.
        """
        newest: tuple[str, int] | None = None
        chosen: dict | None = None
        for run in self.check_runs(sha):
            if run.get("name") != "gate":
                continue
            key = (str(run.get("started_at") or ""), int(run.get("id") or 0))
            if newest is None or key > newest:
                newest, chosen = key, run
        if chosen is None or chosen.get("status") != "completed":
            return None
        return chosen.get("conclusion")

    def comments(self, number: int) -> list[dict]:
        out: list[dict] = []
        page = 1
        while True:
            batch = json.loads(
                self.gh_read(
                    [
                        "api",
                        f"/repos/{self.repo}/issues/{number}/comments"
                        f"?per_page=100&page={page}",
                    ]
                )
            )
            if not isinstance(batch, list):
                raise GhError(f"PR #{number}: comments response was not a list")
            out.extend(batch)
            if len(batch) < 100:
                break
            page += 1
        return out

    # -- writes -------------------------------------------------------------

    @staticmethod
    def review_withholdings(node: dict) -> list[dict]:
        """PR REVIEW bodies, normalised onto the issue-comment shape.

        Carried on the SAME GraphQL page as the PR itself, so this costs no extra API
        call.  Tagged ``_channel="review"``, which makes them WITHHOLD-ONLY in
        ``head_bound_verdict``: they can retract or suppress a pass, never grant one.
        A reviewer who posts a pass as a comment and then withdraws it in a review body
        would otherwise leave the superseded pass standing — the same composition hole
        as an unparseable retraction, through a different channel.
        """
        out: list[dict] = []
        for review in ((node.get("reviews") or {}).get("nodes") or []):
            if not isinstance(review, dict) or not review.get("submittedAt"):
                continue  # a PENDING review has not been submitted; it is not evidence.
            out.append(
                {
                    "_channel": "review",
                    "id": review.get("databaseId") or 0,
                    "body": review.get("body"),
                    "author_association": review.get("authorAssociation"),
                    "created_at": review.get("submittedAt"),
                    "user": review.get("author") or {},
                }
            )
        return out

    def edit_labels(self, number: int, *, add: str = "", remove: str = "") -> None:
        argv = ["pr", "edit", str(number), "--repo", self.repo]
        if add:
            argv += ["--add-label", add]
        if remove:
            argv += ["--remove-label", remove]
        self.gh(argv)

    # -- driver -------------------------------------------------------------

    def parse_node(self, node: dict, gate: str | None) -> PullRequest:
        labels = node.get("labels") or {}
        if (labels.get("pageInfo") or {}).get("hasNextPage"):
            # Fail closed: an unseen label could be a hold.
            raise GhError(f"PR #{node.get('number')}: label set exceeds one page")
        contributors, complete = commit_contributors(node)
        # [OPUS-5] #4677: resolved HERE, once, from the shared predicate both this writer
        # and scripts/review_lane_alarm.py import — so the labeller's visibility can no
        # longer drift wider than the reviewer's.
        author = node.get("author") or {}
        head_ref = str(node.get("headRefName") or "")
        reachable = review_lane_reach.lane_reachable(
            head_ref=head_ref,
            head_repo=(node.get("headRepository") or {}).get("nameWithOwner"),
            author_is_machine=review_lane_reach.machine_actor(author),
            repo=self.repo,
        )
        disposition = ""
        if not reachable:
            _, disposition = review_lane_reach.unreachable_disposition(
                head_ref=head_ref,
                author_login=author.get("login"),
                title=node.get("title"),
            )
        return PullRequest(
            number=int(node["number"]),
            state=str(node.get("state") or "").upper(),
            is_draft=bool(node.get("isDraft")),
            base_ref=str(node.get("baseRefName") or ""),
            head_sha=str(node.get("headRefOid") or ""),
            mergeable=str(node.get("mergeable") or ""),
            gate_conclusion=gate,
            labels=frozenset(
                str(item.get("name", "")).strip().lower()
                for item in (labels.get("nodes") or [])
                if isinstance(item, dict) and str(item.get("name", "")).strip()
            ),
            contributors=contributors,
            contributors_complete=complete,
            opener=normalize_login(author.get("login")),
            lane_reachable=reachable,
            disposition=disposition,
        )

    def reconfirm(
        self, pr: PullRequest, decision: Decision
    ) -> tuple[PullRequest, Decision]:
        """Re-read the ONE PR immediately before writing and re-decide.

        WHY THIS EXISTS (the event/cron double-fire).  The cron sweep reads every open
        PR first and then writes; MEASURED on run 30221614764 that read-to-write gap is
        up to 8m30s for 103 PRs.  Now that a comment / review / CI webhook ALSO starts a
        run, a stale sweep decision can land on top of a fresher event decision and
        resurrect a label the event just removed.  ``concurrency:`` cannot prevent this:
        the two runs are in DIFFERENT groups, and a concurrency group cancels, it does
        not lock.  Convergence therefore has to come from the write path.

        Re-reading collapses the staleness window to one round trip and gives the write
        a genuine compare-and-set on (head SHA, labels, newest verdict): if ANY of them
        moved, the recomputed decision differs and the write is abandoned for this cycle.

        The ``gate`` conclusion is deliberately CARRIED FORWARD rather than re-read: it
        costs up to 7 paginated pages per PR, and ``decide()`` consults it only through
        ``is_green_and_ready``, which guards the purely informational ``flag`` action.
        No arming-relevant decision (promote / retract / unflag) depends on it — pinned
        by ``test_arming_relevant_decisions_never_depend_on_the_gate_read``.
        """
        nodes = self.fetch_one(pr.number)
        if not nodes:
            return pr, Decision("none", "vanished before the write")
        node = nodes[0]
        if str(node.get("baseRefName") or "") != self.default_branch:
            return pr, Decision("none", "base branch moved before the write")
        fresh = self.parse_node(node, pr.gate_conclusion)
        evidence = self.comments(fresh.number) + self.review_withholdings(node)
        return fresh, decide(fresh, evidence)

    def run(self) -> int:
        errors = 0
        writes = 0
        self_reviews = 0
        opener_verdicts = 0
        if self.only_pr is not None:
            nodes = self.fetch_one(self.only_pr)
            self.log(
                f"[{PROGRAM}] scoped to PR #{self.only_pr}: "
                f"{len(nodes)} node(s); dry_run={self.dry_run}"
            )
        else:
            nodes = self.list_open()
            self.log(f"[{PROGRAM}] {len(nodes)} open PR(s); dry_run={self.dry_run}")
        for node in nodes:
            number = node.get("number")
            try:
                if str(node.get("baseRefName") or "") != self.default_branch:
                    continue
                head = str(node.get("headRefOid") or "")
                # Cheap pre-filter: only PRs that could possibly need a gate read.
                gate = self.gate_conclusion(head) if head else None
                pr = self.parse_node(node, gate)
                evidence = self.comments(pr.number) + self.review_withholdings(node)
                decision = decide(pr, evidence)
            except (GhError, json.JSONDecodeError, KeyError, ValueError) as error:
                self.log(f"[{PROGRAM}] PR #{number}: SKIP inspect-failed ({error})")
                errors += 1
                continue

            # VISIBLE TERMINAL STATE.  Emitted BEFORE the `none` short-circuit and
            # independently of the action, because the whole point of this guard is that
            # a refused self-approval usually produces NO label change at all — and a
            # refusal that writes nothing and says nothing is indistinguishable from
            # never having looked.  ::warning surfaces it in the Actions run summary; the
            # `flag` action puts `review:unreviewed` on the PR itself, which is what
            # review-lane-alarm censuses as a blind spot.
            if decision.self_review:
                self_reviews += 1
                self.log(
                    f"::warning title={PROGRAM} refused a self-review::PR #{pr.number}: "
                    f"{decision.reason}"
                )
            if decision.opener_verdict:
                # ADVISORY, counted separately: this one was COUNTED, not refused.
                # Conflating it with the refusals would misreport the guard's reach.
                opener_verdicts += 1
                self.log(
                    f"::notice title={PROGRAM} counted an opener verdict::"
                    f"PR #{pr.number}: {decision.reason}"
                )
            if decision.action == "none":
                continue
            self.log(
                f"[{PROGRAM}] PR #{pr.number}: {decision.action.upper()} — "
                f"{decision.reason}"
            )
            if self.dry_run:
                continue
            if writes >= self.max_writes:
                self.log(f"[{PROGRAM}] PR #{pr.number}: SKIP per-run write cap reached")
                continue
            # Compare-and-set against a FRESH read; see reconfirm()'s docstring.
            try:
                _fresh, confirmed = self.reconfirm(pr, decision)
            except (GhError, json.JSONDecodeError, KeyError, ValueError) as error:
                self.log(f"[{PROGRAM}] PR #{pr.number}: SKIP reconfirm-failed ({error})")
                errors += 1
                continue
            if confirmed.action != decision.action:
                self.log(
                    f"[{PROGRAM}] PR #{pr.number}: SKIP superseded — "
                    f"{decision.action} -> {confirmed.action} ({confirmed.reason})"
                )
                continue
            # `promote` removes NOTHING. `decide()` returns `unflag` before it can return
            # `promote` whenever UNREVIEWED_LABEL is present, so a confirmed promote can
            # never coexist with the flag: the pairing was dead code, and dead code on a
            # write path is where a stale-snapshot read hides. Pinned by
            # test_promote_and_the_unreviewed_flag_are_mutually_exclusive_by_construction.
            add, remove = {
                "promote": (REVIEW_ATTESTATION, ""),
                "retract": ("", REVIEW_ATTESTATION),
                "flag": (UNREVIEWED_LABEL, ""),
                "unflag": ("", UNREVIEWED_LABEL),
            }[decision.action]
            try:
                self.edit_labels(pr.number, add=add, remove=remove)
                writes += 1
            except GhError as error:
                self.log(f"[{PROGRAM}] PR #{pr.number}: label edit failed ({error})")
                errors += 1
        self.log(
            f"[{PROGRAM}] complete: writes={writes} errors={errors} "
            f"self_reviews_refused={self_reviews} opener_verdicts={opener_verdicts}"
        )
        return errors


def run_bridge(
    bridge: VerdictBridge, log: Callable[[str], None] = print, mode: str = "sweep"
) -> int:
    """Exhausted TRANSIENT reads end a SWEEP cycle as ::warning + 0 — the cron covers it.

    In EVENT mode the run answers for one specific webhook.  MEASURED on this repo, the
    scheduled backstop does NOT fire on its nominal cadence: over the 24h to
    2026-07-26T22:10Z the ``1,11,21,...`` cron families actually fired 11-16% of their
    scheduled ticks (inter-run gaps of 53-75 minutes against a 10-minute cron).  Treating
    a dropped event as "the cron will get it" therefore silently costs up to an hour, so
    an event run that could not complete its reads exits NON-ZERO: visibly red, and
    re-runnable.  Any NON-transient failure propagates and reds the run in both modes.
    """
    try:
        errors = bridge.run()
    except gh_retry.GhTransientExhausted as error:
        if mode == "event":
            log(
                f"::error title={PROGRAM} event run failed on transient GitHub API "
                f"failures::{error} — bounded retries exhausted. This run answered for a "
                "single webhook and has no per-PR backstop on a useful timescale."
            )
            return 1
        log(
            f"::warning title={PROGRAM} skipped a cycle on transient GitHub API "
            f"failures::{error} — bounded retries exhausted; the cron backstop covers "
            "a missed cycle, so this run reports success."
        )
        return 0
    return 1 if errors else 0


# --------------------------------------------------------------------------- tests


def comment(
    body: str,
    *,
    association: str = "OWNER",
    created_at: str = "2026-07-26T10:00:00Z",
    cid: int = 1,
    user: str = "reviewer",
) -> dict:
    return {
        "id": cid,
        "body": body,
        "author_association": association,
        "created_at": created_at,
        "updated_at": created_at,
        "user": {"login": user},
    }


HEAD = "a" * 40
OTHER = "b" * 40


def pr_fixture(**overrides) -> PullRequest:
    base = {
        "number": 4200,
        "state": "OPEN",
        "is_draft": False,
        "base_ref": "main",
        "head_sha": HEAD,
        "mergeable": "MERGEABLE",
        "gate_conclusion": "success",
        "labels": frozenset(),
        # The DEFAULT fixture is a worker PR the registry lane can enumerate — the shape
        # 89 of the 118 open PRs measured on 2026-07-26 actually have.  Unreachable PRs
        # are opted into explicitly (`lane_reachable=False`), so a test asserting `flag`
        # is asserting it about a PR that really can be reviewed.
        "lane_reachable": True,
    }
    base.update(overrides)
    base["labels"] = frozenset(base["labels"])
    return PullRequest(**base)


def self_test() -> None:
    passing = comment(f"Reviewed {HEAD}.\n\nVERDICT: pass")
    failing = comment(f"Reviewed {HEAD}.\n\nVERDICT: fail", cid=2)

    # A head-bound pass on a clean PR promotes.
    assert decide(pr_fixture(), [passing]).action == "promote"

    # Head binding: a verdict citing a DIFFERENT sha does not bind, so the PR reads as
    # unreviewed (and, being green, gets flagged) rather than promoted.
    stale = comment(f"Reviewed {OTHER}.\n\nVERDICT: pass")
    assert decide(pr_fixture(), [stale]).action == "flag", "stale sha must not promote"
    # A short prefix of the head is NOT a binding.
    short = comment(f"Reviewed {HEAD[:12]}.\n\nVERDICT: pass")
    assert decide(pr_fixture(), [short]).action == "flag", "prefix must not bind"
    # A 40-hex token embedded in a longer hex run is not a standalone SHA.
    glued = comment(f"Reviewed {HEAD}cafe.\n\nVERDICT: pass")
    assert decide(pr_fixture(), [glued]).action == "flag", "glued sha must not bind"

    # Line anchoring: the phrase inside a quoted INSTRUCTION is not a verdict.
    quoted = comment(
        f"Reviewed {HEAD}. The brief says to end with `VERDICT: pass` or "
        "`VERDICT: fail`.\n\nStill working on it."
    )
    assert decide(pr_fixture(), [quoted]).action == "flag", "substring must not pass"
    assert trailing_verdict("VERDICT: pass\n\ntrailing prose") is None
    # A MENTION (quoted / blockquoted / fenced / bulleted) is not verdict-shaped.
    for mention in ("> VERDICT: pass", "- VERDICT: pass", "`VERDICT: pass`", "```"):
        assert trailing_verdict(mention) is None, mention

    # AMBIGUITY: a verdict-SHAPED final line that does not parse must NOT read as "no
    # verdict" — that would leave an older, well-formed pass as the newest verdict.
    for shaped in (
        "VERDICT: FAIL",
        "VERDICT: fail (retracting my pass)",
        "**VERDICT: Fail**",
        "VERDICT - pass",
        "verdict: unclear",
    ):
        assert trailing_verdict(shaped) == AMBIGUOUS, shaped
    stale_pass = comment(
        f"{HEAD}\n\nVERDICT: pass", created_at="2026-01-01T00:00:00Z", cid=1
    )
    for shaped in ("VERDICT: FAIL", "VERDICT: fail (retracting my pass)"):
        retraction = comment(
            f"{HEAD}\n\n{shaped}", created_at="2026-02-01T00:00:00Z", cid=2
        )
        combo = [stale_pass, retraction]
        assert head_bound_verdict(combo, HEAD).value == AMBIGUOUS, shaped
        assert decide(pr_fixture(), combo).action != "promote", shaped
        # ... and it REMOVES an attestation it may be retracting.
        held = decide(pr_fixture(labels={REVIEW_ATTESTATION}), combo)
        assert held.action == "retract", (shaped, held)
    # An ambiguous line from an UNTRUSTED author is still ignored entirely (no DoS on
    # arming by a drive-by "VERDICT: FAIL").
    drive_by_shaped = comment(
        f"{HEAD}\n\nVERDICT: FAIL", association="NONE", created_at="2026-02-01T00:00:00Z", cid=3
    )
    assert decide(pr_fixture(), [stale_pass, drive_by_shaped]).action == "promote"

    # Provenance: an untrusted association is not a reviewer.
    for association in ("CONTRIBUTOR", "FIRST_TIME_CONTRIBUTOR", "NONE", "", "owner "):
        drive_by = comment(f"Reviewed {HEAD}.\n\nVERDICT: pass", association=association)
        assert decide(pr_fixture(), [drive_by]).action == "flag", association

    # RETRACTION: a later head-bound fail defeats an earlier pass at the same head.
    early = comment(f"{HEAD}\n\nVERDICT: pass", created_at="2026-07-26T10:00:00Z", cid=1)
    later = comment(f"{HEAD}\n\nVERDICT: fail", created_at="2026-07-26T11:00:00Z", cid=2)
    assert decide(pr_fixture(), [early, later]).action != "promote"
    assert decide(pr_fixture(), [later, early]).action != "promote", "order-independent"
    # ... and it REMOVES an attestation that is already on the PR.
    retract = decide(pr_fixture(labels={REVIEW_ATTESTATION}), [early, later])
    assert retract.action == "retract", retract
    # Editing the OLD pass must not reorder it ahead of the newer fail.
    edited = dict(early, updated_at="2026-07-26T23:00:00Z")
    assert decide(pr_fixture(), [edited, later]).action != "promote", "edit reorders"
    # A newer PASS after a fail does re-attest.
    newer_pass = comment(
        f"{HEAD}\n\nVERDICT: pass", created_at="2026-07-26T12:00:00Z", cid=3
    )
    assert decide(pr_fixture(), [later, newer_pass]).action == "promote"

    # Holds are fail-closed, matching auto-arm's exclusion set.
    for hold in sorted(HOLD_LABELS) + ["needs:design", "needs:anything"]:
        held = decide(pr_fixture(labels={hold}), [passing])
        assert held.action == "none", (hold, held)
        assert "hold" in held.reason, (hold, held)

    # Never retract on ABSENCE of evidence — a hand-applied label survives.
    kept = decide(pr_fixture(labels={REVIEW_ATTESTATION}), [])
    assert kept.action == "none", kept
    kept_stale = decide(pr_fixture(labels={REVIEW_ATTESTATION}), [stale])
    assert kept_stale.action == "none", kept_stale

    # Idempotence: an already-attested PR is not re-promoted.
    assert decide(pr_fixture(labels={REVIEW_ATTESTATION}), [passing]).action == "none"

    # Drafts and non-open PRs are never attested.
    assert decide(pr_fixture(is_draft=True), [passing]).action == "none"
    assert decide(pr_fixture(state="MERGED"), [passing]).action == "none"

    # FLAG: the invisible population — green, mergeable, non-draft, no review label.
    assert decide(pr_fixture(), []).action == "flag"
    #   ... but not a draft, not a conflicting PR, not a red/absent gate,
    #   and not one already in a review lane.
    assert decide(pr_fixture(is_draft=True), []).action == "none"
    assert decide(pr_fixture(mergeable="CONFLICTING"), []).action == "none"
    assert decide(pr_fixture(gate_conclusion="failure"), []).action == "none"
    assert decide(pr_fixture(gate_conclusion=None), []).action == "none"
    for lane in ("review:needs", "review:changes", "review:needs-user", "review:pass"):
        assert decide(pr_fixture(labels={lane}), []).action == "none", lane

    # UNFLAG: the informational label is dropped as soon as a verdict binds.
    assert decide(pr_fixture(labels={UNREVIEWED_LABEL}), [passing]).action == "unflag"
    assert decide(pr_fixture(labels={UNREVIEWED_LABEL}), [failing]).action == "unflag"
    assert decide(pr_fixture(labels={UNREVIEWED_LABEL}), []).action == "none"

    # [OPUS-5] #4677. THE LABELLER'S VISIBILITY MUST NOT EXCEED THE REVIEWER'S.
    # A PR the registry review lane cannot enumerate is never flagged — the label would
    # claim an enrolment nothing can honour — and an existing flag is WITHDRAWN.
    unreachable = pr_fixture(lane_reachable=False, disposition="not enrolled")
    flagged_unreachable = pr_fixture(
        lane_reachable=False, disposition="not enrolled", labels={UNREVIEWED_LABEL}
    )
    assert decide(unreachable, []).action == "none", "must not flag what nothing reviews"
    assert decide(flagged_unreachable, []).action == "unflag", "a stale flag is withdrawn"
    assert "not enrolled" in decide(unreachable, []).reason, "the refusal states WHY"
    # The control arm: the ONLY difference is reachability, so this pair reds if the
    # guard is deleted or inverted rather than passing vacuously.
    assert decide(pr_fixture(lane_reachable=True), []).action == "flag"
    # The same guard covers the AMBIGUOUS-verdict flag site, which is a second writer.
    ambiguous = comment(f"{HEAD}\n\nVERDICT: FAIL")
    assert decide(pr_fixture(lane_reachable=True), [ambiguous]).action == "flag"
    assert decide(unreachable, [ambiguous]).action == "none"
    assert decide(flagged_unreachable, [ambiguous]).action == "unflag"
    # An unreachable PR is still promoted on a real hand-review: the guard scopes the
    # informational flag only, and must never suppress an attestation someone earned.
    assert decide(unreachable, [passing]).action == "promote"
    # UNREADABLE reachability fails CLOSED — the dataclass default writes no label.
    assert PullRequest.__dataclass_fields__["lane_reachable"].default is False

    # SCOPING (the event path). The number an event payload names is parsed strictly;
    # anything present but non-numeric must RAISE, never degrade to a full sweep.
    assert parse_pr_argument("") is None
    assert parse_pr_argument(None) is None
    assert parse_pr_argument("  ") is None
    assert parse_pr_argument("4324") == 4324
    for bad in ("0", "-1", "12x", "4324; rm -rf /", "1e3", "٤٣", "null"):
        try:
            parse_pr_argument(bad)
        except ValueError:
            pass
        else:
            raise AssertionError(f"parse_pr_argument accepted {bad!r}")

    # The informational label must never be a hold (it would deadlock the arm lane).
    assert UNREVIEWED_LABEL not in HOLD_LABELS
    assert not hold_labels({UNREVIEWED_LABEL})

    # Malformed comment payloads are ignored, never treated as a pass.
    assert head_bound_verdict([None, 7, {}, {"body": None}], HEAD) is None  # type: ignore[list-item]

    # A PR REVIEW body may WITHHOLD but never GRANT.
    review_fail = dict(
        comment(f"{HEAD}\n\nVERDICT: fail", created_at="2026-02-01T00:00:00Z", cid=9),
        _channel="review",
    )
    review_pass = dict(
        comment(f"{HEAD}\n\nVERDICT: pass", created_at="2026-02-01T00:00:00Z", cid=9),
        _channel="review",
    )
    old_pass = comment(f"{HEAD}\n\nVERDICT: pass", created_at="2026-01-01T00:00:00Z")
    assert decide(pr_fixture(), [old_pass, review_fail]).action != "promote"
    assert (
        decide(pr_fixture(labels={REVIEW_ATTESTATION}), [old_pass, review_fail]).action
        == "retract"
    )
    # ... and a review body ALONE never grants the attestation.
    assert decide(pr_fixture(), [review_pass]).action == "flag"
    # A review fail cannot be overridden by an even newer review pass either.
    newest_review_pass = dict(
        comment(f"{HEAD}\n\nVERDICT: pass", created_at="2026-03-01T00:00:00Z", cid=10),
        _channel="review",
    )
    assert decide(pr_fixture(), [old_pass, review_fail, newest_review_pass]).action != "promote"

    # AUTHOR XOR REVIEWER.  A verdict from someone who wrote COMMITS at this head may
    # WITHHOLD but never GRANT.  The control arm is load-bearing: with the guard deleted
    # BOTH arms promote and the second assertion fires, so this is non-vacuous for the
    # same-login case that is the whole reason the guard exists.
    #
    # And the OPENER is NOT refused: `jeswr` opening a PR whose commits are `claude`'s is
    # the normal shape of a legitimate cross-agent review here (sparq#4331, #4386).
    opener_only = pr_fixture(opener="jeswr", contributors=frozenset({"claude"}))
    opener_pass = comment(f"{HEAD}\n\nVERDICT: pass", user="jeswr")
    opened = decide(opener_only, [opener_pass])
    assert opened.action == "promote", opened
    assert opened.opener_verdict and not opened.self_review, opened
    assert "COUNTED" in opened.reason, opened
    # ... and `commit_contributors` must not smuggle the opener back in.
    who, complete = commit_contributors(
        {"author": {"login": "jeswr"},
         "commits": {"totalCount": 1, "nodes": [{"commit": {
             "message": "x",
             "authors": {"nodes": [{"user": {"login": "claude"}}]},
             "committer": {"user": {"login": "web-flow"}}}}]}}
    )
    assert complete and "jeswr" not in who, who
    contrib = pr_fixture(contributors=frozenset({"jeswr", "claude"}))
    self_pass = comment(f"{HEAD}\n\nVERDICT: pass", user="jeswr")
    indep_pass = comment(f"{HEAD}\n\nVERDICT: pass", user="reviewer")
    assert decide(contrib, [indep_pass]).action == "promote", "control arm must promote"
    refused = decide(contrib, [self_pass])
    assert refused.action != "promote", refused
    assert refused.self_review and "SELF-REVIEW" in refused.reason, refused
    # ... and the bot-suffix spelling difference does not launder it.
    app_contrib = pr_fixture(contributors=frozenset({"sparq-orchestrator"}))
    app_self = comment(f"{HEAD}\n\nVERDICT: pass", user="sparq-orchestrator[bot]")
    assert decide(app_contrib, [app_self]).action != "promote"
    # ... a contributor's FAIL still retracts (withhold-only, never a DoS on retraction).
    own_fail = comment(f"{HEAD}\n\nVERDICT: fail", user="jeswr")
    held = pr_fixture(
        labels={REVIEW_ATTESTATION}, contributors=frozenset({"jeswr", "claude"})
    )
    assert decide(held, [own_fail]).action == "retract"
    # ... an unreadable contributor set refuses every grant, but not a retraction.
    torn = pr_fixture(contributors_complete=False)
    assert decide(torn, [indep_pass]).action != "promote"
    assert (
        decide(
            pr_fixture(labels={REVIEW_ATTESTATION}, contributors_complete=False),
            [comment(f"{HEAD}\n\nVERDICT: fail", user="reviewer")],
        ).action
        == "retract"
    )
    # ... and a clean PR does not report a phantom self-review.
    assert not decide(contrib, [indep_pass]).self_review
    # ... an attestation resting ONLY on a self-review is WITHDRAWN (positive evidence),
    #     while absence of any verdict still never retracts.
    self_only = pr_fixture(
        labels={REVIEW_ATTESTATION}, contributors=frozenset({"jeswr", "claude"})
    )
    assert decide(self_only, [self_pass]).action == "retract"
    assert decide(self_only, []).action == "none"
    assert decide(self_only, [indep_pass, self_pass]).action == "none"

    print(f"{PROGRAM} self-test: PASS")


def main() -> int:
    parser = argparse.ArgumentParser(description="Bridge VERDICT comments to labels.")
    parser.add_argument("--repo", help="owner/repository to sweep")
    parser.add_argument("--default-branch", default="main")
    parser.add_argument("--dry-run", action="store_true", help="report, write nothing")
    parser.add_argument("--max-writes", type=int, default=25)
    parser.add_argument(
        "--pr",
        default="",
        help="restrict the run to ONE pull request (the event path); empty = full sweep",
    )
    parser.add_argument(
        "--mode",
        choices=("sweep", "event"),
        default="sweep",
        help="event: a webhook run with no useful cron backstop — fail LOUD on exhausted "
        "transients. sweep: the scheduled reconciliation pass — fail soft.",
    )
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()

    if args.self_test:
        self_test()
        return 0
    if not args.repo:
        parser.error("--repo is required unless --self-test is used")
    only_pr = parse_pr_argument(args.pr)
    if only_pr is None and args.mode == "event":
        # An event trigger whose payload carried no PR number (an issue comment, a
        # check-suite with an empty pull_requests array).  Falling through to a FULL
        # SWEEP here would let any commenter on any issue start a ~9-minute all-PR run.
        print(f"[{PROGRAM}] event mode with no PR number in the payload — nothing to do")
        return 0
    return run_bridge(
        VerdictBridge(
            args.repo,
            args.default_branch,
            gh_read=run_gh_read,
            dry_run=args.dry_run,
            max_writes=args.max_writes,
            only_pr=only_pr,
        ),
        mode=args.mode,
    )


if __name__ == "__main__":
    sys.exit(main())
