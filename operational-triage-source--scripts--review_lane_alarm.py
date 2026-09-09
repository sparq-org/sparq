#!/usr/bin/env python3
# [OPUS-5] Review-lane BLIND-SPOT alarm. 🤖 SPARQ agent.
#
# WHAT THIS IS: the "silence must not look like green" safeguard for VERDICT PRODUCTION,
# the sibling of scripts/formal_lane_alarm.py (which watches verdict production for the
# formal lanes) and scripts/ci_selection_alarm.py.
#
# MEASURED CAUSE (2026-07-26, paginated over all 118 open PRs, total_count cross-checked):
# every review that sparq PRs receive is produced OUT OF REPO by the registry's
# review-fix.yml, scheduled by the registry's dispatch.yml. That scheduler admits a PR
# only through `enumerate_review_items` (registry scripts/dispatch-claim.py), whose first
# three gates are AUTHOR-SIDE and fail closed:
#
#     if not HEAD_REF_RE.match(ref):        continue   # ^sparq-agent/issue-([1-9][0-9]*)-
#     if head_repo != repo:                 continue
#     if not login.endswith("[bot]") ...:   continue   # must be the worker App bot
#
# followed by a mandatory REGISTRY provenance record (`provenance_admission_error`) that
# only a host-side worker run can write. A PR opened by anything other than the worker App
# — every PR an interactive agent session pushes — fails all three by construction and is
# therefore INVISIBLE to the only review producer that exists. The census that proves it:
#
#     89 open PRs by sparq-orchestrator[bot]: 100% carry a review:* loop label
#     29 open PRs by jeswr:                     0 carry review:pass/changes/needs
#
# Zero is not a coincidence, it is a predicate. Nothing in sparq ever noticed, because
# absence of a review produces no artifact — no red check, no queue entry, nothing. This
# script makes that absence LOUD.
#
# WHERE VERDICTS ACTUALLY LIVE (checked, not assumed). The authoritative verdict store is
# NOT the PR comment thread — it is the registry's `ledger` branch, at
# `orchestration/review-verdicts/<owner>--<repo>--pr<N>-round<K>.json`, head-bound through
# a `host_envelope.reviewed_sha` field. 830 records exist, 568 of them sparq. This alarm
# reads PR COMMENTS, so the obvious objection is that it measures the wrong store and will
# false-alarm on PRs that hold a ledger verdict. It does not, and the cross-check is the
# reason this predicate is trustworthy — over the 35 open non-draft sparq PRs on
# 2026-07-26 the partition is EXACT:
#
#     registry_lane_reachable == True   (10 PRs) -> 10/10 hold a ledger verdict record
#     registry_lane_reachable == False  (25 PRs) ->  0/25 hold a ledger verdict record
#
# Zero overlap in either direction, because the registry review lane is the ONLY writer of
# that store and it writes only for PRs it can enumerate — the very predicate replicated
# below. So "unreachable" and "has no verdict anywhere" coincide, and this alarm never
# needs a cross-repo ledger read (which it has no token for anyway). If that partition ever
# breaks — if some future producer writes ledger records for unreachable PRs — this alarm
# starts false-alarming, loudly and visibly, which is the correct direction for it to fail.
#
# THE ALARM: enumerate the BLIND SPOT — open, non-draft, non-human-held PRs that the
# registry lane can never reach — and RED this run (plus file one deduped issue) when any
# of them has sat without a countable verdict for longer than the threshold.
#
# WHO OWNS A "HUMAN" HOLD (sparq#4911). The human-hold exit above is terminal: a PR
# carrying `needs:user` / `review:needs-user` raises no alarm, because autonomy is
# standing down until a human decides. That exemption was keyed on the label's NAME, and
# the census in sparq#4911 shows the name does not carry the fact — of the 41 live
# human-owned holds on open sparq PRs, ZERO were applied by a human (23/23 `needs:user`
# and 18/18 `review:needs-user` by `sparq-orchestrator[bot]`), and a further population of
# machine parks was written under the maintainer's own alias. So this alarm's quietest
# exit was being taken on machine writes: the loop escalates to a human, signs the
# escalation as one, and every detector downstream reads a human decision that never
# happened. `hold_ownership` therefore resolves the hold from the `labeled` TIMELINE EVENT
# — who actually applied the label the PR still carries — and not from its name:
#   * applied by a Bot actor            -> MACHINE-owned; the PR is judged on its verdict.
#   * applied by a human actor, but an ORCHESTRATOR-authored park receipt (a
#     `class=<reason>` marker) lands within RECEIPT_WINDOW_SECONDS of the write ->
#     MACHINE-owned too. This is the credential-drift half: the predicate means to trust an
#     INTENT, and an actor identity is only a proxy for it. sparq#4911's PR #3620 is the
#     measured shape — the label written under the maintainer's alias at 08:54:39 and the
#     loop's own `class=capacity cause=partition` receipt posted by the bot 86s later,
#     mixed credentials inside one logical operation. The receipt's author must be the
#     ALLOWLISTED orchestrator App (`RECEIPT_AUTHOR_LOGINS`), not merely some machine
#     account: a `class=` marker in a human's own comment — or in an unrelated review bot's
#     — proves nothing about the loop's intent and must never be able to silence a real
#     human decision.
#   * applied by a human actor with no such receipt -> HUMAN-owned, exempt as before.
#   * no `labeled` event resolvable at all -> UNKNOWN, treated as human-owned (the quiet,
#     under-reporting direction this file takes everywhere) but COUNTED in the census as
#     `hold-owner-unknown`, because a missing edge is only visible as a population.
# This changes no credential and cannot: it makes the sparq-side predicate correct even
# while the writes stay mis-signed.
#
# THE SAME BLIND SPOT DISARMS NOTHING. The registry's force-push safety net,
# `enumerate_disarm_items` (same file as the review enumerator), retracts the auto-merge
# latch whenever an armed PR's live head diverges from its recorded reviewed-sha — a real
# and correct net. But it replicates the IDENTICAL author-side gates (head-ref regex, head
# repo, `[bot]` author, provenance record), so it too is blind to this population. On the
# sparq side auto-arm.py's `expectedHeadOid` CAS binds an arm to the head it just READ, not
# to the head that was REVIEWED, so it does not substitute. For these PRs no ledger verdict
# exists today, so there is nothing yet to go stale; the gap opens the moment a
# comment-sourced `review:pass` lands on one of them.
#
# WHAT COUNTS AS A VERDICT (deliberately strict; a wrong pass is worse than no pass):
#   1. LINE-ANCHORED. `VERDICT: pass` / `VERDICT: fail` as the LAST non-blank line of the
#      comment, matched with ^...$ under re.MULTILINE-free full-line comparison. NEVER a
#      substring search: the standing review brief quotes the string `VERDICT: pass` in its
#      own instruction text, and a substring grep has already scored that quotation as a
#      real pass in this repo.
#   2. HEAD-BOUND. The comment body must contain the CURRENT head as a standalone 40-hex
#      SHA. A force-push advances the head, so an old comment stops counting the instant
#      the tree it reviewed is superseded.
#   3. AUTHOR XOR REVIEWER. A comment posted by the SAME GitHub account that opened the PR
#      is never a review of it. Self-approval has already happened in this repo; a detector
#      that credits it would certify the exact failure it exists to find.
# A comment failing (1) is not a verdict at all. A comment satisfying (1) but failing (2)
# or (3) is a DISCREDITED verdict — reported by name in the issue body, because "reviewed
# on a stale head" and "never reviewed" need different human responses.
#
# TWO DESIGN INVARIANTS (mirrors formal_lane_alarm.py / ci_selection_alarm.py):
#   1. FAIL-LOUD: an alarm-INFRASTRUCTURE error (gh api failure, unparseable response)
#      exits NON-ZERO with a `::error::` annotation. A broken detector must never look
#      like a quiet green.
#   2. NON-SPAMMY: ONE open issue, keyed by a stable `<!-- review-lane-alarm-key: … -->`
#      body marker searched over open `review-alarm`-labelled issues before filing.
#
# WHAT THIS IS NOT. It is NOT a gate: scheduled on main, creates no check-run on any PR
# head or merge-group ref, blocks no merge. It grants NO arming authority and writes NO PR
# labels — it cannot arm, promote, or unblock anything, by construction (the workflow holds
# `issues: write` and nothing else). It does not lower any bar: it never marks a PR
# reviewed, it only refuses to let "nobody reviewed this" stay silent.
#
# Usage:
#   review_lane_alarm.py                              # real (gh; env GITHUB_REPOSITORY)
#   review_lane_alarm.py --dry-run                    # print findings; no gh writes
#   review_lane_alarm.py --prs-file prs.json \
#       --now 2026-07-26T22:00:00Z --dry-run          # hermetic (tests)
#   review_lane_alarm.py --self-test                  # hermetic fixtures
#
# Stdlib only + the `gh` CLI (present on every GitHub runner).

from __future__ import annotations

import argparse
import datetime as dt
import json
import os
import re
import subprocess
import sys

BASE_LABELS = ["review-alarm", "auto"]
KEY_PREFIX = "review-lane-alarm-key"
ALARM_KEY = "blind-spot"

# The registry review lane's own admission regex, replicated VERBATIM from
# scripts/dispatch-claim.py (registry, master @ 2026-07-26). Kept byte-identical on
# purpose: this alarm's whole claim is "the registry lane cannot see these PRs", so the
# reachability test must be the registry's test and not a paraphrase of it.
REGISTRY_HEAD_REF_RE = re.compile(r"^sparq-agent/issue-([1-9][0-9]*)-")

# A verdict line, anchored to a WHOLE line. The pattern is applied to an individually
# stripped line — never searched across a body — so no amount of surrounding prose can
# make it match.
VERDICT_LINE_RE = re.compile(r"^VERDICT:[ ]+(pass|fail)$")
# A standalone 40-hex commit SHA (word-bounded so a 41-char token never matches).
SHA_RE = re.compile(r"(?<![0-9a-fA-F])([0-9a-f]{40})(?![0-9a-fA-F])")

# Human-owned holds. Terminal: autonomy stands down until a human clears them, so an
# unreviewed PR carrying one is NOT an alarm — it is a human's open decision. That is true
# only when a human actually applied it; see `hold_ownership` (sparq#4911).
HUMAN_HOLD_LABELS = frozenset({"needs:user", "review:needs-user"})
# A machine park/hold RECEIPT — the comment the orchestrator posts alongside a machine
# park, carrying a `class=<reason>` field (sparq#4911 names `class=capacity`,
# `class=question` and `cause=partition` on real receipts). Not every machine hold carries
# one — 8 of the 23 `needs:user` holds in that census did — so this is a POSITIVE signal
# only: its presence machine-owns a write, its absence proves nothing either way. A
# comment carrying the marker, within RECEIPT_WINDOW_SECONDS of the label write and from an
# allowlisted author, is an INTENT signal that survives a mis-signed credential.
RECEIPT_CLASS_RE = re.compile(r"\bclass=[A-Za-z][A-Za-z0-9_-]*\b")
# …and the receipt only means anything as the ORCHESTRATOR'S OWN intent, so its author is
# allowlisted by LOGIN and not merely "some machine account". Comment bodies are untrusted
# and RECEIPT_CLASS_RE is deliberately broad, so blanket `[bot]` trust would let any
# unrelated review/app bot post — or merely QUOTE — a `class=` token inside the symmetric
# window and convert a genuinely human-applied hold to machine-owned, silencing the one
# exit this file treats as terminal. That is the wrong direction to fail in, and it is the
# same one-App trust the sibling writers already take (scripts/retriage.py's TRUSTED_BOT,
# scripts/bd-to-issues.py's _MIGRATION_BOTS). Logins are normalised (case-folded, an
# `app/` prefix and a `[bot]` suffix stripped) because GitHub reports the SAME App under
# all three spellings — `gh` as `app/x`, REST as `x[bot]`, GraphQL as `x` — and an
# un-normalised comparison would read as a guard while never matching. Same rule as
# scripts/release_pr_guard.py's `normalize_login`, re-stated locally rather than imported
# to keep this alarm stdlib-only and free-standing (see the header).
RECEIPT_AUTHOR_LOGINS = frozenset({"sparq-orchestrator"})
# Symmetric, because a receipt may be posted either side of the write it explains. Sized
# off the one measured interval sparq#4911 gives (86s, PR #3620) with room to spare; a
# tighter window would drop that case, a much wider one would start sweeping in unrelated
# loop chatter.
RECEIPT_WINDOW_SECONDS = 300
# The registry loop's own labels. A blind-spot PR cannot legitimately carry one (nothing
# in the blind spot is enumerable by the lane that writes them), but if a human or a
# migration puts one there we honour it rather than double-reporting.
LANE_LABELS = frozenset({"review:pass", "review:changes", "review:needs", "review:parked"})

DEFAULT_MAX_AGE_HOURS = 24


class AlarmError(Exception):
    """Alarm-INFRASTRUCTURE failure — surfaced LOUD (non-zero exit)."""


# --------------------------------------------------------------------------- #
# Reachability: can the registry review lane EVER enumerate this PR?
# --------------------------------------------------------------------------- #
def registry_lane_reachable(pr: dict, repo: str) -> bool:
    """True iff `pr` passes every AUTHOR-SIDE admission gate of the registry review
    lane's `enumerate_review_items`. False => no review producer in existence can ever
    see this PR, which is precisely the blind spot this alarm watches.

    Only the three structural gates are replicated. The registry additionally requires a
    provenance record, but that is a LIVE registry read this repo has no token for, and it
    can only ever narrow admission further — so treating a PR as reachable here is the
    CONSERVATIVE direction (it under-reports the blind spot, never over-reports it)."""
    if not isinstance(pr, dict):
        raise AlarmError("pull-request listing carried a non-object entry")
    head = pr.get("head") or {}
    ref = str(head.get("ref") or "")
    head_repo = (head.get("repo") or {}).get("full_name")
    login = str((pr.get("user") or {}).get("login") or "")
    if not REGISTRY_HEAD_REF_RE.match(ref):
        return False
    if head_repo != repo:
        return False
    if not login.endswith("[bot]"):
        return False
    return True


# --------------------------------------------------------------------------- #
# Verdict detection (pure)
# --------------------------------------------------------------------------- #
def verdict_polarity(body: str) -> str | None:
    """The polarity of a comment's verdict line, or None when the comment carries no
    verdict at all.

    STRICTLY the LAST non-blank line, compared whole. This is the anti-substring rule:
    the review brief's own instruction text contains the literal `VERDICT: pass`, and a
    body that merely quotes it — anywhere, including as its own line followed by more
    prose — is NOT a verdict. Only a comment that ENDS on the line is."""
    if not isinstance(body, str):
        return None
    lines = [line.rstrip() for line in body.replace("\r\n", "\n").split("\n")]
    for line in reversed(lines):
        if not line.strip():
            continue
        match = VERDICT_LINE_RE.match(line.strip())
        return match.group(1) if match else None
    return None


def comment_binds_head(body: str, head_sha: str) -> bool:
    """True iff the comment names the CURRENT head as a standalone 40-hex SHA.

    An OCCURRENCE test, exactly as the arming bridge does it: what makes the comment a
    verdict is its trailing VERDICT line, and what binds it to a tree is naming that
    tree. A comment naming only the PREVIOUS head fails here the moment a force-push
    lands, which is the required force-push invalidation."""
    if not isinstance(body, str) or not isinstance(head_sha, str):
        return False
    if not SHA_RE.fullmatch(head_sha):
        return False
    return head_sha in SHA_RE.findall(body)


def countable_verdict(pr: dict, comments: list[dict]) -> tuple[str | None, list[str]]:
    """-> (polarity, discredited_reasons).

    `polarity` is 'pass'/'fail' for the LATEST comment that is a verdict AND binds the
    current head AND was not written by the PR's own author. `discredited_reasons`
    names every comment that carried a real verdict line but failed one of the other two
    tests — the difference between "stale review" and "no review" is the difference
    between two very different human responses, so it is never collapsed."""
    head_sha = str((pr.get("head") or {}).get("sha") or "")
    pr_author = str((pr.get("user") or {}).get("login") or "")
    if not comments:
        return None, []
    ordered = sorted(
        (c for c in comments if isinstance(c, dict)),
        key=lambda c: str(c.get("created_at") or ""),
    )
    polarity: str | None = None
    discredited: list[str] = []
    for comment in ordered:
        body = comment.get("body")
        found = verdict_polarity(body if isinstance(body, str) else "")
        if found is None:
            continue
        author = str((comment.get("user") or {}).get("login") or "")
        # AUTHOR XOR REVIEWER, structurally. Checked BEFORE the head test so a
        # self-review is never reported as merely "stale" — its problem is not its age.
        if author and pr_author and author == pr_author:
            discredited.append(f"self-review by the PR author @{author} (never countable)")
            continue
        if not comment_binds_head(body if isinstance(body, str) else "", head_sha):
            discredited.append(
                f"verdict by @{author or '?'} names no current head "
                f"{head_sha[:12] or '(unknown)'} — superseded tree"
            )
            continue
        # Latest head-bound, non-self verdict wins: a later retraction defeats an
        # earlier pass.
        polarity = found
    return polarity, discredited


# --------------------------------------------------------------------------- #
# Classification (pure)
# --------------------------------------------------------------------------- #
def _labels(pr: dict) -> set[str]:
    out: set[str] = set()
    for label in pr.get("labels") or []:
        name = label.get("name") if isinstance(label, dict) else label
        if isinstance(name, str) and name:
            out.add(name)
    return out


def _is_bot_actor(who: dict) -> bool:
    """True iff a GitHub actor/user object denotes a machine account.

    Both signals, because they are populated inconsistently across the events and comments
    APIs: the `[bot]` login suffix (App installations) and the `type: Bot` field."""
    if not isinstance(who, dict):
        return False
    return str(who.get("type") or "") == "Bot" or str(who.get("login") or "").endswith("[bot]")


def _normalise_login(login: object) -> str:
    """Case-folded login with an `app/` prefix and a `[bot]` suffix stripped.

    One App is spelled three ways depending on which surface reports it:
    `app/sparq-orchestrator` (`gh`), `sparq-orchestrator[bot]` (REST),
    `sparq-orchestrator` (GraphQL)."""
    name = str(login or "").strip().lower()
    if name.startswith("app/"):
        name = name[len("app/"):]
    if name.endswith("[bot]"):
        name = name[: -len("[bot]")]
    return name


def _is_receipt_author(who: dict) -> bool:
    """True iff an actor/user object is an ALLOWLISTED park-receipt author.

    Strictly narrower than `_is_bot_actor`, which only establishes that SOME machine posted
    the comment — not enough to speak for the orchestrator (see RECEIPT_AUTHOR_LOGINS). The
    machine-account test is kept as a conjunct so a human registering a colliding login
    cannot mint receipts either."""
    if not isinstance(who, dict):
        return False
    return _is_bot_actor(who) and _normalise_login(who.get("login")) in RECEIPT_AUTHOR_LOGINS


def latest_hold_application(pr: dict, events: list[dict] | None) -> dict | None:
    """The LATEST `labeled` event that applied a hold label the PR STILL carries, or None.

    Latest, not first, deliberately: it mirrors the park policy's own "the latest park
    application is the one that decides" rule (sparq#4911), so a machine re-park over an
    older human hold reads as machine-owned. Only labels the PR CURRENTLY carries count; an
    applied-then-removed hold owns nothing."""
    held = HUMAN_HOLD_LABELS & _labels(pr)
    if not held or not events:
        return None
    latest: dict | None = None
    for event in events:
        if not isinstance(event, dict) or event.get("event") != "labeled":
            continue
        label = event.get("label")
        name = label.get("name") if isinstance(label, dict) else None
        if name not in held:
            continue
        stamp = str(event.get("created_at") or "")
        if not stamp:
            continue
        # GitHub emits uniform `...Z` UTC stamps, so lexicographic ordering is
        # chronological — the same assumption countable_verdict() already makes.
        if latest is None or stamp > str(latest.get("created_at") or ""):
            latest = event
    return latest


def hold_ownership(pr: dict, events: list[dict] | None, comments: list[dict]) -> str:
    """Who owns this PR's human-hold label: 'human' | 'machine' | 'unknown'.

    sparq#4911: the label NAME does not carry this fact — every live human-owned hold on
    open sparq PRs was machine-applied, some under the maintainer's own alias. Resolved
    from the write itself, in two steps (see the module header):
      1. a Bot actor is machine-owned outright;
      2. a HUMAN actor whose write is accompanied by an ORCHESTRATOR-authored (login in
         RECEIPT_AUTHOR_LOGINS, not merely any bot) `class=<reason>` park receipt inside
         RECEIPT_WINDOW_SECONDS is machine-owned as well — that receipt is the loop's own
         intent, and no credential drift can move it to a human.
    'unknown' when no `labeled` event resolves; callers treat that as human-owned (the
    under-reporting direction) and count it, rather than alarming on a guess."""
    latest = latest_hold_application(pr, events)
    if latest is None:
        return "unknown"
    if _is_bot_actor(latest.get("actor") or {}):
        return "machine"
    applied_at = _parse_iso(latest.get("created_at"))
    for comment in comments or []:
        if not isinstance(comment, dict):
            continue
        if not _is_receipt_author(comment.get("user") or {}):
            continue
        body = comment.get("body")
        if not isinstance(body, str) or not RECEIPT_CLASS_RE.search(body):
            continue
        created = comment.get("created_at")
        if not created:
            continue
        if abs((_parse_iso(created) - applied_at).total_seconds()) <= RECEIPT_WINDOW_SECONDS:
            return "machine"
    return "human"


def classify_pr(pr: dict, comments: list[dict], repo: str, events: list[dict] | None = None) -> str:
    """One of: registry-lane | draft | human-held | lane-labelled | has-verdict |
    blind-spot. Only `blind-spot` is alarm-worthy."""
    if pr.get("draft") is True:
        return "draft"
    if registry_lane_reachable(pr, repo):
        return "registry-lane"
    if HUMAN_HOLD_LABELS & _labels(pr):
        # A MACHINE-written hold is not a human's open decision, so it buys no exemption:
        # fall through and judge the PR on whether a verdict actually exists (sparq#4911).
        if hold_ownership(pr, events, comments) != "machine":
            return "human-held"
    if LANE_LABELS & _labels(pr):
        return "lane-labelled"
    polarity, _ = countable_verdict(pr, comments)
    if polarity is not None:
        return "has-verdict"
    return "blind-spot"


def _parse_iso(ts: str) -> dt.datetime:
    try:
        return dt.datetime.fromisoformat(str(ts).replace("Z", "+00:00"))
    except (TypeError, ValueError) as exc:
        raise AlarmError(f"unparseable timestamp {ts!r}") from exc


def find_blind_spot(
    prs: list[dict], comments_by_pr: dict, repo: str, now: dt.datetime, max_age_hours: float,
    events_by_pr: dict | None = None,
) -> tuple[list[dict], dict]:
    """-> (alarming findings, census). The census counts EVERY state exit, not just the
    alarming one: a per-state population is the only shape in which a MISSING edge is
    visible at all (a success-rate cannot express "nothing ever entered this state").

    `events_by_pr` maps PR number -> its issue-event log, needed only for PRs carrying a
    hold label; without it every hold reads as `hold-owner-unknown` and stays exempt."""
    census: dict = {}
    findings: list[dict] = []
    for pr in prs:
        number = pr.get("number")
        if not isinstance(number, int) or isinstance(number, bool) or number <= 0:
            raise AlarmError("pull-request listing carried an invalid number")
        comments = comments_by_pr.get(number) or comments_by_pr.get(str(number)) or []
        events = (events_by_pr or {}).get(number) or (events_by_pr or {}).get(str(number)) or []
        state = classify_pr(pr, comments, repo, events)
        census[state] = census.get(state, 0) + 1
        hold_notes: list[str] = []
        if HUMAN_HOLD_LABELS & _labels(pr):
            owner = hold_ownership(pr, events, comments)
            if owner != "human":
                census[f"hold-owner-{owner}"] = census.get(f"hold-owner-{owner}", 0) + 1
            if owner == "machine":
                applied = latest_hold_application(pr, events) or {}
                label = (applied.get("label") or {}).get("name", "?")
                actor = str((applied.get("actor") or {}).get("login") or "?")
                hold_notes.append(
                    f"`{label}` was applied by @{actor} as a MACHINE write — it is not a "
                    "human's open decision and buys no exemption"
                )
        if state != "blind-spot":
            continue
        age = (now - _parse_iso(pr.get("created_at"))).total_seconds() / 3600.0
        if age < max_age_hours:
            census["blind-spot-fresh"] = census.get("blind-spot-fresh", 0) + 1
            continue
        _, discredited = countable_verdict(pr, comments)
        findings.append(
            {
                "number": number,
                "title": str(pr.get("title") or "")[:120],
                "author": str((pr.get("user") or {}).get("login") or "?"),
                "head_ref": str((pr.get("head") or {}).get("ref") or "?"),
                "head_sha": str((pr.get("head") or {}).get("sha") or "?"),
                "age_hours": round(age, 1),
                "discredited": discredited,
                "hold_notes": hold_notes,
            }
        )
    findings.sort(key=lambda f: -f["age_hours"])
    return findings, census


def finding_note(finding: dict) -> str:
    """The one-line explanation printed AND rendered into the issue table. Machine-hold
    provenance leads: "this hold was never a human's" is the reason the row is here at
    all, and it needs a different human response from a stale or missing verdict."""
    parts = list(finding.get("hold_notes") or []) + list(finding.get("discredited") or [])
    return "; ".join(parts) or "no verdict comment at all"


# --------------------------------------------------------------------------- #
# Reporting
# --------------------------------------------------------------------------- #
def render_issue_body(findings: list[dict], census: dict, repo: str, max_age_hours: float) -> str:
    lines = [
        "> 🤖 SPARQ agent",
        "",
        f"`review-alarm`: **{len(findings)} open PR(s)** have gone more than "
        f"{max_age_hours:g}h with **no countable review verdict**, and no review producer "
        "can ever reach them.",
        "",
        "These PRs fail the registry review lane's author-side admission gates "
        "(`enumerate_review_items` in the registry's `scripts/dispatch-claim.py`): the head "
        "ref does not match `^sparq-agent/issue-([1-9][0-9]*)-`, and/or the author is not the "
        "worker App bot. That lane is the only verdict producer sparq has, so nothing will "
        "ever dispatch a review for them without a human or an orchestrator doing it by hand.",
        "",
        "| PR | age (h) | author | head ref | note |",
        "| --- | --- | --- | --- | --- |",
    ]
    for finding in findings[:50]:
        note = finding_note(finding)
        lines.append(
            f"| #{finding['number']} | {finding['age_hours']} | {finding['author']} | "
            f"`{finding['head_ref']}` | {note} |"
        )
    if len(findings) > 50:
        lines.append(f"| … | | | | {len(findings) - 50} more |")
    lines += [
        "",
        "**Census of every state exit** (open PRs in `" + repo + "`):",
        "",
        "```",
        *[f"{state}: {count}" for state, count in sorted(census.items())],
        "```",
        "",
        "A verdict is countable only when it is line-anchored (`VERDICT: pass|fail` as the "
        "comment's last non-blank line), names the current head as a standalone 40-hex SHA, "
        "and was **not** posted by the PR's own author.",
        "",
        "A `needs:user` / `review:needs-user` hold exempts a PR from this alarm only when a "
        "HUMAN applied it — resolved from the `labeled` timeline event, and downgraded to a "
        "machine write when an orchestrator-authored `class=<reason>` park receipt lands within "
        f"{RECEIPT_WINDOW_SECONDS}s of it (sparq#4911). `hold-owner-machine` in the census "
        "counts holds that bought no exemption; `hold-owner-unknown` counts holds whose "
        "applier could not be resolved, which stay exempt.",
        "",
        f"<!-- {KEY_PREFIX}: {ALARM_KEY} -->",
    ]
    return "\n".join(lines)


# --------------------------------------------------------------------------- #
# gh plumbing
# --------------------------------------------------------------------------- #
def _gh(args: list[str], *, parse: bool = True):
    try:
        out = subprocess.run(
            ["gh", *args], check=True, capture_output=True, text=True, timeout=180
        )
    except (subprocess.CalledProcessError, subprocess.TimeoutExpired, OSError) as exc:
        detail = getattr(exc, "stderr", "") or str(exc)
        raise AlarmError(f"gh {' '.join(args[:2])} failed: {detail}") from exc
    if not parse:
        return out.stdout
    try:
        return json.loads(out.stdout or "null")
    except json.JSONDecodeError as exc:
        raise AlarmError(f"gh {' '.join(args[:2])}: unparseable JSON: {exc}") from exc


def fetch_open_prs(repo: str) -> list[dict]:
    """Every OPEN PR, PAGINATED. `gh api --paginate` walks the Link headers; a truncated
    read would silently shrink the blind spot, so an unexpected shape is fatal."""
    data = _gh(["api", "--paginate", "--slurp", f"repos/{repo}/pulls?state=open&per_page=100"])
    if not isinstance(data, list):
        raise AlarmError("pull-request listing is not a list")
    prs: list[dict] = []
    for page in data:
        if not isinstance(page, list):
            raise AlarmError("pull-request page is not a list")
        prs.extend(page)
    return prs


def fetch_comments(repo: str, number: int) -> list[dict]:
    data = _gh(["api", "--paginate", "--slurp", f"repos/{repo}/issues/{number}/comments"])
    if not isinstance(data, list):
        raise AlarmError(f"comment listing for #{number} is not a list")
    out: list[dict] = []
    for page in data:
        if not isinstance(page, list):
            raise AlarmError(f"comment page for #{number} is not a list")
        out.extend(page)
    return out


def fetch_label_events(repo: str, number: int) -> list[dict]:
    """One PR's issue-EVENT log, paginated. The `labeled` entries carry the ACTOR that
    applied each label, which is the only place a hold's ownership is recorded — the label
    itself carries no provenance at all. Fetched only for PRs that carry a hold label."""
    data = _gh(["api", "--paginate", "--slurp", f"repos/{repo}/issues/{number}/events?per_page=100"])
    if not isinstance(data, list):
        raise AlarmError(f"event listing for #{number} is not a list")
    out: list[dict] = []
    for page in data:
        if not isinstance(page, list):
            raise AlarmError(f"event page for #{number} is not a list")
        out.extend(page)
    return out


def file_issue(repo: str, body: str, count: int) -> None:
    """One open issue, keyed by the body marker (the flow-on idempotency mechanism)."""
    marker = f"<!-- {KEY_PREFIX}: {ALARM_KEY} -->"
    existing = _gh(
        [
            "api",
            "--paginate",
            "--slurp",
            f"repos/{repo}/issues?state=open&labels=review-alarm&per_page=100",
        ]
    )
    if not isinstance(existing, list):
        raise AlarmError("issue listing is not a list")
    for page in existing:
        for issue in page if isinstance(page, list) else []:
            if isinstance(issue, dict) and marker in str(issue.get("body") or ""):
                _gh(
                    [
                        "api",
                        "-X",
                        "PATCH",
                        f"repos/{repo}/issues/{issue['number']}",
                        "-f",
                        f"body={body}",
                    ]
                )
                print(f"::notice::updated existing review-alarm issue #{issue['number']}")
                return
    title = f"review-alarm: {count} PR(s) unreachable by every review producer"
    _gh(
        [
            "api",
            "-X",
            "POST",
            f"repos/{repo}/issues",
            "-f",
            f"title={title}",
            "-f",
            f"body={body}",
            *[arg for label in BASE_LABELS for arg in ("-f", f"labels[]={label}")],
        ]
    )
    print("::notice::filed a new review-alarm issue")


# --------------------------------------------------------------------------- #
# Entry point
# --------------------------------------------------------------------------- #
def run(args: argparse.Namespace) -> int:
    repo = args.repo or os.environ.get("GITHUB_REPOSITORY") or ""
    if not re.fullmatch(r"[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+", repo):
        raise AlarmError("repo must be OWNER/REPOSITORY (set --repo or GITHUB_REPOSITORY)")
    now = _parse_iso(args.now) if args.now else dt.datetime.now(dt.timezone.utc)

    if args.prs_file:
        with open(args.prs_file, encoding="utf-8") as fh:
            payload = json.load(fh)
        prs = payload.get("pulls") or []
        comments_by_pr = payload.get("comments") or {}
        events_by_pr = payload.get("events") or {}
    else:
        prs = fetch_open_prs(repo)
        comments_by_pr = {}
        events_by_pr = {}
        for pr in prs:
            number = pr.get("number")
            if not isinstance(number, int) or isinstance(number, bool):
                raise AlarmError("pull-request listing carried an invalid number")
            # Only the blind-spot candidates need their comments read; a reachable or
            # drafted PR is decided without them.
            if pr.get("draft") is not True and not registry_lane_reachable(pr, repo):
                comments_by_pr[number] = fetch_comments(repo, number)
                # …and only a candidate that CLAIMS a human hold needs its event log, to
                # resolve who actually applied it (sparq#4911).
                if HUMAN_HOLD_LABELS & _labels(pr):
                    events_by_pr[number] = fetch_label_events(repo, number)

    findings, census = find_blind_spot(
        prs, comments_by_pr, repo, now, args.max_age_hours, events_by_pr
    )

    print(f"review-alarm census over {len(prs)} open PR(s) in {repo}:")
    for state, count in sorted(census.items()):
        print(f"  {state}: {count}")
    for finding in findings:
        print(f"  BLIND SPOT #{finding['number']} ({finding['age_hours']}h) {finding_note(finding)}")

    if not findings:
        print("::notice::no PR is outside every review producer's reach")
        return 0

    body = render_issue_body(findings, census, repo, args.max_age_hours)
    if args.dry_run:
        print(body)
    else:
        file_issue(repo, body, len(findings))
    print(
        f"::error::{len(findings)} open PR(s) have no countable review verdict and are "
        "unreachable by every review producer — verdict PRODUCTION, not transport, is the "
        "stalled stage"
    )
    return 1


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repo", default="")
    parser.add_argument("--now", default="")
    parser.add_argument("--prs-file", default="")
    parser.add_argument("--max-age-hours", type=float, default=DEFAULT_MAX_AGE_HOURS)
    parser.add_argument("--dry-run", action="store_true")
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args(argv)
    if args.self_test:
        return _self_test()
    try:
        return run(args)
    except AlarmError as exc:
        print(f"::error::review-lane alarm infrastructure failure: {exc}")
        return 2


# --------------------------------------------------------------------------- #
# Hermetic self-test (runs as the FIRST step of every scheduled run — a detector is
# never trusted to watch anything before it has proved itself on this tick's code).
# --------------------------------------------------------------------------- #
SHA_A = "a" * 40
SHA_B = "b" * 40
# The literal the standing review brief quotes at every reviewer. Substring-grepping for
# it has already scored a false pass in this repo; the fixture pins that it cannot again.
BRIEF_QUOTE = (
    "End with a line-anchored final line, exactly `VERDICT: pass` or `VERDICT: fail`.\n"
    "Please review the diff now.\n"
)


def _pr(number=1, *, ref="research/x", login="jeswr", sha=SHA_A, draft=False, labels=(),
        created="2026-07-01T00:00:00Z", repo="sparq-org/sparq"):
    return {
        "number": number,
        "draft": draft,
        "created_at": created,
        "title": f"pr {number}",
        "user": {"login": login},
        "labels": [{"name": name} for name in labels],
        "head": {"ref": ref, "sha": sha, "repo": {"full_name": repo}},
    }


def _comment(body, login="reviewer", created="2026-07-02T00:00:00Z", kind=None):
    user = {"login": login}
    if kind:
        user["type"] = kind
    return {"body": body, "user": user, "created_at": created}


def _labeled(label="needs:user", login="jeswr", created="2026-07-20T00:00:00Z", kind=None):
    """A `labeled` issue event, in the shape `GET /issues/{n}/events` returns."""
    actor = {"login": login}
    if kind:
        actor["type"] = kind
    return {"event": "labeled", "label": {"name": label}, "actor": actor, "created_at": created}


def _self_test() -> int:  # noqa: C901 - a flat table of named assertions reads best flat
    failures: list[str] = []

    def check(name: str, condition: bool) -> None:
        if not condition:
            failures.append(name)

    repo = "sparq-org/sparq"

    # --- reachability: the registry lane's author-side gates, pinned to REAL branch
    # names measured on 2026-07-26.
    check(
        "worker branch + bot author is reachable",
        registry_lane_reachable(
            _pr(ref="sparq-agent/issue-2908-30221671021-1", login="sparq-orchestrator[bot]"), repo
        ),
    )
    check(
        "local agent branch is NOT reachable",
        not registry_lane_reachable(
            _pr(ref="ci/pr-freshness-auto-update-branch", login="jeswr"), repo
        ),
    )
    check(
        "bot author on a non-worker branch is NOT reachable",
        not registry_lane_reachable(
            _pr(ref="research/x", login="sparq-orchestrator[bot]"), repo
        ),
    )
    check(
        "worker branch from a NON-bot author is NOT reachable",
        not registry_lane_reachable(
            _pr(ref="sparq-agent/issue-1-2-1", login="jeswr"), repo
        ),
    )
    check(
        "fork head is NOT reachable",
        not registry_lane_reachable(
            _pr(
                ref="sparq-agent/issue-1-2-1",
                login="sparq-orchestrator[bot]",
                repo="attacker/sparq",
            ),
            repo,
        ),
    )
    check("issue-0 is not a worker branch", not REGISTRY_HEAD_REF_RE.match("sparq-agent/issue-0-1-1"))

    # --- verdict line: anchored, never a substring.
    check("plain pass parses", verdict_polarity(f"{SHA_A}\n\nVERDICT: pass") == "pass")
    check("plain fail parses", verdict_polarity(f"{SHA_A}\n\nVERDICT: fail") == "fail")
    check("trailing blank lines tolerated", verdict_polarity("VERDICT: pass\n\n  \n") == "pass")
    check("INSTRUCTION TEXT is not a verdict", verdict_polarity(BRIEF_QUOTE) is None)
    check(
        "quoted verdict followed by prose is not a verdict",
        verdict_polarity("VERDICT: pass\nactually I am still thinking") is None,
    )
    check("backticked line is not a verdict", verdict_polarity("`VERDICT: pass`") is None)
    check("suffixed line is not a verdict", verdict_polarity("VERDICT: pass (with caveats)") is None)
    check("wrong case is not a verdict", verdict_polarity("VERDICT: PASS") is None)
    check("unknown polarity is not a verdict", verdict_polarity("VERDICT: maybe") is None)
    check("empty body is not a verdict", verdict_polarity("") is None)
    check("non-string body is not a verdict", verdict_polarity(None) is None)

    # --- head binding.
    check("names current head", comment_binds_head(f"reviewed {SHA_A}\nVERDICT: pass", SHA_A))
    check("names only the old head", not comment_binds_head(f"reviewed {SHA_B}", SHA_A))
    check("short sha does not bind", not comment_binds_head(SHA_A[:12], SHA_A))
    check("41-hex token does not bind", not comment_binds_head(SHA_A + "c", SHA_A))
    check("unknown head never binds", not comment_binds_head(SHA_A, ""))

    # --- countable_verdict: the three rules composed.
    pr = _pr(sha=SHA_A, login="jeswr")
    good = [_comment(f"reviewed {SHA_A}\n\nVERDICT: pass", login="reviewer-bot")]
    check("a clean cross-author head-bound pass counts", countable_verdict(pr, good)[0] == "pass")

    stale = [_comment(f"reviewed {SHA_B}\n\nVERDICT: pass", login="reviewer-bot")]
    polarity, why = countable_verdict(pr, stale)
    check("SUPERSEDED HEAD does not count", polarity is None)
    check("superseded head is reported, not dropped", any("superseded" in r for r in why))

    selfrev = [_comment(f"reviewed {SHA_A}\n\nVERDICT: pass", login="jeswr")]
    polarity, why = countable_verdict(pr, selfrev)
    check("AUTHOR CANNOT VERDICT THEIR OWN PR", polarity is None)
    check("self-review is reported by name", any("self-review" in r for r in why))

    quoted = [_comment(f"{SHA_A}\n{BRIEF_QUOTE}", login="reviewer-bot")]
    check("INSTRUCTION SUBSTRING IS NOT A PASS", countable_verdict(pr, quoted)[0] is None)

    retract = [
        _comment(f"{SHA_A}\n\nVERDICT: pass", login="r1", created="2026-07-02T00:00:00Z"),
        _comment(f"{SHA_A}\n\nVERDICT: fail", login="r2", created="2026-07-03T00:00:00Z"),
    ]
    check("a later fail supersedes an earlier pass", countable_verdict(pr, retract)[0] == "fail")

    # --- classification + the alarm decision.
    now = _parse_iso("2026-07-26T00:00:00Z")
    check(
        "an unreviewed local-branch PR is a BLIND SPOT",
        classify_pr(_pr(), [], repo) == "blind-spot",
    )
    check(
        "NO VERDICT IS NOT ARMABLE — it never reads as reviewed",
        classify_pr(_pr(), [], repo) != "has-verdict",
    )
    check("a worker PR is the registry lane's", classify_pr(_pr(
        ref="sparq-agent/issue-9-1-1", login="sparq-orchestrator[bot]"), [], repo) == "registry-lane")
    check("a draft is not alarmed", classify_pr(_pr(draft=True), [], repo) == "draft")
    check(
        "a human hold is terminal",
        classify_pr(_pr(labels=("needs:user",)), [], repo) == "human-held",
    )

    # --- sparq#4911: WHO applied the hold, not what it is called.
    held = _pr(labels=("needs:user",))
    bot_applied = [_labeled(login="sparq-orchestrator[bot]")]
    check(
        "a BOT-APPLIED human hold buys NO exemption",
        classify_pr(held, [], repo, bot_applied) == "blind-spot",
    )
    check(
        "the bot-applied hold is classed machine-owned",
        hold_ownership(held, bot_applied, []) == "machine",
    )
    check(
        "a `type: Bot` actor with no [bot] suffix is still machine-owned",
        hold_ownership(held, [_labeled(login="orchestrator", kind="Bot")], []) == "machine",
    )
    check(
        "a genuinely HUMAN-applied hold is still terminal",
        classify_pr(held, [], repo, [_labeled(login="jeswr")]) == "human-held",
    )
    # The ALIAS write, on the real #3620 shape: a human actor whose label write is
    # bracketed by the loop's own `class=capacity cause=partition` receipt 86s later.
    alias = [_labeled(login="jeswr", created="2026-07-28T08:54:39Z")]
    receipt = [
        _comment(
            "park: class=capacity cause=partition",
            login="sparq-orchestrator[bot]",
            created="2026-07-28T08:56:05Z",
        )
    ]
    check(
        "an ALIAS-written hold with a bot park receipt is machine-owned",
        hold_ownership(held, alias, receipt) == "machine",
    )
    check(
        "the alias-written hold buys no exemption either",
        classify_pr(held, receipt, repo, alias) == "blind-spot",
    )
    # …and the receipt window is a WINDOW: the same receipt a day later proves nothing.
    late = [
        _comment(
            "park: class=capacity cause=partition",
            login="sparq-orchestrator[bot]",
            created="2026-07-29T08:56:05Z",
        )
    ]
    check(
        "a bot receipt OUTSIDE the window does not machine-own a human write",
        hold_ownership(held, alias, late) == "human",
    )
    check(
        "a HUMAN comment carrying a class= marker does not machine-own the write",
        hold_ownership(held, alias, [_comment("class=capacity", login="jeswr",
                                              created="2026-07-28T08:55:00Z")]) == "human",
    )
    # …nor does an UNRELATED machine account. `_is_bot_actor` only proves "some bot posted
    # this"; the receipt has to be the orchestrator's own, or any app that quotes a
    # `class=` token inside the window revokes a genuine human hold.
    other_bot = [
        _comment(
            "flagging: class=capacity looks wrong here",
            login="some-review-app[bot]",
            created="2026-07-28T08:55:00Z",
        )
    ]
    check(
        "an UNRELATED BOT's class= comment does not machine-own a human write",
        hold_ownership(held, alias, other_bot) == "human",
    )
    check(
        "…and the hold therefore stays terminal",
        classify_pr(held, other_bot, repo, alias) == "human-held",
    )
    check(
        "the receipt author is matched across GitHub's login spellings",
        hold_ownership(held, alias, [_comment("park: class=capacity",
                                              login="app/Sparq-Orchestrator", kind="Bot",
                                              created="2026-07-28T08:55:00Z")]) == "machine",
    )
    check(
        "a HUMAN squatting the orchestrator login cannot mint a receipt",
        hold_ownership(held, alias, [_comment("park: class=capacity",
                                              login="sparq-orchestrator",
                                              created="2026-07-28T08:55:00Z")]) == "human",
    )
    check(
        "an unresolvable hold stays exempt (the under-reporting direction)",
        hold_ownership(held, [], []) == "unknown"
        and classify_pr(held, [], repo, []) == "human-held",
    )
    check(
        "an event for a label the PR no longer carries owns nothing",
        latest_hold_application(held, [_labeled(label="review:parked",
                                                login="sparq-orchestrator[bot]")]) is None,
    )
    check(
        "the LATEST application decides: a machine re-park over a human hold",
        hold_ownership(
            held,
            [_labeled(login="jeswr", created="2026-07-20T00:00:00Z"),
             _labeled(login="sparq-orchestrator[bot]", created="2026-07-21T00:00:00Z")],
            [],
        ) == "machine",
    )

    machine_held = _pr(number=4804, labels=("needs:user",), created="2026-07-18T00:00:00Z")
    findings_h, census_h = find_blind_spot(
        [machine_held], {}, repo, now, DEFAULT_MAX_AGE_HOURS, {4804: bot_applied})
    check("a machine-held unreviewed PR now ALARMS", [f["number"] for f in findings_h] == [4804])
    check("the machine-written hold is counted", census_h.get("hold-owner-machine") == 1)
    check(
        "the finding says WHY the hold was ignored",
        bool(findings_h) and "MACHINE write" in finding_note(findings_h[0]),
    )
    _, census_u = find_blind_spot([machine_held], {}, repo, now, DEFAULT_MAX_AGE_HOURS, {})
    check("an unresolved hold is counted, not alarmed", census_u.get("hold-owner-unknown") == 1)
    check("…and stays out of the blind spot", census_u.get("human-held") == 1)
    check(
        "a lane label is honoured",
        classify_pr(_pr(labels=("review:pass",)), [], repo) == "lane-labelled",
    )
    check("a real verdict clears the blind spot", classify_pr(_pr(), good, repo) == "has-verdict")
    check(
        "a SELF verdict does NOT clear the blind spot",
        classify_pr(_pr(), selfrev, repo) == "blind-spot",
    )

    old = _pr(number=3426, created="2026-07-18T00:00:00Z")
    fresh = _pr(number=4342, created="2026-07-25T23:00:00Z")
    findings, census = find_blind_spot([old, fresh], {}, repo, now, DEFAULT_MAX_AGE_HOURS)
    check("the aged blind-spot PR alarms", [f["number"] for f in findings] == [3426])
    check("a fresh blind-spot PR does not alarm yet", census.get("blind-spot-fresh") == 1)
    check("the census counts every state exit", census.get("blind-spot") == 2)

    findings2, _ = find_blind_spot(
        [_pr(number=7, created="2026-07-18T00:00:00Z")], {7: good}, repo, now,
        DEFAULT_MAX_AGE_HOURS)
    check("a reviewed PR raises no finding", findings2 == [])

    findings3, _ = find_blind_spot(
        [_pr(number=8, created="2026-07-18T00:00:00Z")], {8: selfrev}, repo, now,
        DEFAULT_MAX_AGE_HOURS)
    check("a SELF-reviewed PR still alarms", [f["number"] for f in findings3] == [8])
    check(
        "the self-review reason reaches the issue body",
        bool(findings3) and any("self-review" in r for r in findings3[0]["discredited"]),
    )

    body = render_issue_body(findings3, {"blind-spot": 1}, repo, DEFAULT_MAX_AGE_HOURS)
    check("the issue body self-identifies", body.startswith("> 🤖 SPARQ agent"))
    check("the issue body carries the dedupe key", f"<!-- {KEY_PREFIX}: {ALARM_KEY} -->" in body)

    # --- fail-loud on malformed input (never a quiet green).
    for name, thunk in (
        ("malformed PR entry raises", lambda: registry_lane_reachable("nope", repo)),
        (
            "invalid PR number raises",
            lambda: find_blind_spot([{"number": 0}], {}, repo, now, 1.0),
        ),
        (
            "unparseable timestamp raises",
            lambda: find_blind_spot(
                [_pr(created="not-a-date")], {}, repo, now, 1.0
            ),
        ),
    ):
        try:
            thunk()
        except AlarmError:
            pass
        else:
            failures.append(name)

    if failures:
        for name in failures:
            print(f"FAIL: {name}")
        print(f"::error::review_lane_alarm self-test: {len(failures)} failure(s)")
        return 1
    print("review_lane_alarm self-test: all checks passed")
    return 0


if __name__ == "__main__":
    sys.exit(main())
