#!/usr/bin/env python3
"""Recover a merge-queue entry whose merge-group ref was never dispatched by Actions."""

# [OPUS-5] sparq-org/sparq#4652 — the zero-dispatch merge-group watchdog.
#
# THE OUTAGE THIS FIXES (measured 2026-07-27/28, #4652): PR #4534's merge-group ref
# `gh-readonly-queue/main/pr-4534-3cc1bf828c…` was created at 23:04:37Z with head
# `1bfb0174…` and deleted at 00:05:04Z — 60 min 27 s later — having produced
# **0 check-suites, 0 check-runs, 0 workflow runs**. Eight workflows carry a bare
# unfiltered `merge_group:` trigger and fired for every other group that day; for this
# one ref none of them was ever CONSIDERED. #4534 sat at position 1 in AWAITING_CHECKS;
# the queue merges strictly in order and each entry needs its OWN group's required
# check, so #4400/#4636/#4639 sat behind it with green groups that could not land.
# GitHub eventually dequeued #4534 with reason CI_TIMEOUT at 00:05:03Z (the ruleset's
# `check_response_timeout_minutes: 60`), and the chain rebuilt 3 s later, discarding
# three already-green group CI runs. Net: 73 min 41 s of zero merges (23:18:59Z →
# 00:32:40Z) and ~66 min of wasted group CI.
#
# WHY ZERO check-SUITES IS THE LOAD-BEARING SIGNAL — and why "zero check-runs" is NOT.
# A `paths`-filtered workflow that matches nothing STILL creates a check-suite (it just
# reports no runs). So a group with real workflows but nothing to do reads as
# suites>0 / runs==0. Zero *suites* means no workflow was ever considered for the ref,
# i.e. the `merge_group` event never reached the Actions dispatcher. Weakening the
# predicate to check-runs would fire on legitimately-pending and legitimately-empty
# groups. The predicate here is `total_count == 0` on
# `GET /repos/{repo}/commits/{group_head_sha}/check-suites`, and NOTHING else.
#
# ── the anchor problem, and the measurement that settles it ───────────────────────
# The grace gate needs T_create = when the merge-group ref was BUILT. Two obvious
# proxies are WRONG and were rejected by measurement:
#   * `entry.enqueuedAt` — an entry can sit outside the `max_entries_to_build` window
#     for many minutes before its group is built, so this anchor is far too early.
#   * the group head commit's `committer.date` — GitHub STAMPS the group commit rather
#     than dating it at build time. THE LOAD-BEARING EVIDENCE IS THE DISTRIBUTION, not
#     any single commit: over 40 live group heads, `branch_creation - committer.date`
#     spans 2 s to 3929 s with a median of 16 s, and only 1 of 40 falls within +/-2 s of
#     the real creation time. `1bfb0174`'s stamp sits 344 s BEFORE its ref existed.
#     (An earlier draft of this note argued from a parent-commit contradiction instead;
#     that argument is withdrawn, because the parent's own date is a stamp too and the
#     claim could not be independently re-derived. The distribution stands on its own.)
# The CORRECT, exact anchor is the repository ACTIVITY API:
#     GET /repos/{repo}/activity?activity_type=branch_creation&ref=<full ref>
# whose rows carry an exact `timestamp` plus `after` = the created head SHA. Measured
# for the incident ref: `branch_creation` at 23:04:37Z, `after=1bfb0174…`, deleted
# 00:05:04Z — a 3627 s lifetime with zero suites. It is also the ONLY enumeration
# source that can see such a ref at all: it has no workflow runs and the branch is
# deleted, so the runs API cannot find it.
#
# We match on `after == head_oid`, never on the ref NAME. Measured over the full
# retained queue population (1281 branch_creation rows, 2026-07-02 -> 07-28): 15 ref
# NAMES were created 2+ times (max 4x) because the queue rebuilds a group on the same
# base, while head SHAs had ZERO collisions. Head SHA is the only sound key, and it is
# the same key the recovery idempotence uses.
#
# ── the decision table (fail-safe: only RECOVER is a mutation) ────────────────────
#   SKIP     entry state is not AWAITING_CHECKS, or no speculative group is built yet
#   HOLD     >=1 check-suite present — dispatch happened; NOT our case (the control)
#   WAIT     zero suites but still inside the grace period
#   REFUSE   the suite count or the creation time could NOT be positively established
#   NOOP     a recovery was already attempted for this exact group head (idempotence)
#   CAP      the per-PR or per-run recovery budget is exhausted
#   RECOVER  zero suites, past grace, positively established, within budget => act
# Anything that cannot be POSITIVELY established is a REFUSE, never a recovery: a
# watchdog that re-enqueues on ambiguity thrashes the queue and is worse than the bug.
#
# ── emission ──────────────────────────────────────────────────────────────────────
# EVERY entry emits exactly one row EVERY tick — naming the ref, the head, the suite
# count and the decision — plus a closing census row, plus an explicit row when the
# queue is empty. A held CAP or REFUSE re-emits every tick for as long as it holds;
# a silent watchdog would convert a visible 73-minute outage into an invisible one.
#
# ── the dequeue-routing split (`classify-dequeue`) ────────────────────────────────
# `.github/workflows/merge-queue-feedback.yml` routes every non-merge dequeue to
# `review:changes` by design. For a zero-dispatch CI_TIMEOUT that is a
# MACHINE-MANUFACTURED re-review: #4534 lost `review:pass` for a platform event drop,
# with nothing whatsoever wrong in its diff. `classify-dequeue` splits the two cases by
# the SUITE COUNT (never by the timeout reason alone) and prints a `route=` output the
# workflow branches on. It also recognises the watchdog's OWN dequeue — our recovery is
# implemented as dequeue+enqueue, which itself fires `pull_request.dequeued`, and
# without this arm the watchdog would demote every PR it rescued.
#
# A THIRD preserve arm (sq-fnr85) covers the QUEUE-LEVEL drop: an entry that leaves the
# queue with no merge and a cleared arm because the QUEUE did something (QUEUE_CLEARED,
# ROLL_BACK), not because anything is wrong with the diff — including after its group
# went green. That arm consults no marker (this population never has one) and is bounded
# by DEFAULT_MAX_PRESERVED_REQUEUES. See GREEN_DROP_REASONS for why those two reason
# strings are admissible evidence when `MANUAL` is not.
#
# ── A DEFENSIVE CONTROL MUST NOT AMPLIFY WHAT IT REJECTS ──────────────────────────
# Rejecting an untrusted marker emits ONE annotation per PR, never one per marker.
# Measured the hard way: a per-marker warning against a PR carrying 567 markers
# produced ~110 KB of log and 567 annotations — past GitHub's annotation cap, burying
# every other signal in the run. A filter that emits per-item output hands the attacker
# a log-flood denial of service against its own operator: the cheaper the input is to
# produce, the louder the defence gets. State the count and the distinct authors; never
# iterate the payload into the log. (110 KB -> 368 bytes on that same input.)
#
# ── the verdict-stripping defect this ALSO has to fix ─────────────────────────────
# MEASURED on #4709, 2026-07-28 — the dequeue itself takes the verdict down with it:
#     05:14:56  labeled   review:pass    by sparq-orchestrator[bot]
#     06:06:38  removed_from_merge_queue (reason MANUAL)
#     06:06:55  labeled   review:changes by github-actions[bot]
#     06:06:55  unlabeled review:pass    by github-actions[bot]
# Seventeen seconds. Because our recovery IS a dequeue, a watchdog without this fix
# would strip `review:pass`, apply `review:changes`, disable the arm and admit the PR to
# the FIX pipeline — on every PR it rescued. That is a net regression: it would
# manufacture a `review:changes` backlog out of a platform event that never fired.
#
# The existing routing is CORRECT for its original purpose — a reviewer withdrawing a PR
# *should* pull the verdict. The defect is that `MANUAL` conflates "a reviewer withdrew
# this" with "infrastructure moved it", and the handler keys off THAT a dequeue happened,
# never WHY. So we discriminate on EVIDENCE, never on the event name: a fresh
# watchdog marker for this exact group head is what makes a dequeue ours.
#
# NON-NEGOTIABLE PRECONDITION on every preserve route (`verdict_is_still_for_this_tree`):
# a verdict may only survive a dequeue if the head has NOT moved since it was granted.
# A verdict attests to a specific TREE, so if a commit, a force-push, or an
# `unlabeled review:pass` lands after the grant, the label must be allowed to fall off.
# Verified by hand on #4709 before restoring it: `review:pass` at 05:14:56Z, last commit
# 04:56:39Z, zero head-moving events between. Both the latest grant and the latest move
# are used, so the realistic push-then-re-approve sequence still preserves correctly.
#
# The group head SHA cannot be recovered from the API after the ref is deleted (that is
# exactly the population with no workflow runs to read it off), so the watchdog records
# it in a machine-readable marker comment BEFORE it acts. The marker is a POINTER, not
# a cached verdict: `classify-dequeue` re-derives the live suite count for the recorded
# SHA at decision time (the commit stays readable after the branch is deleted — verified
# on `1bfb0174`, still `total_count: 0`). A missing/stale marker, an unreadable count,
# or a non-zero count all fall back to today's demote behaviour.
#
# ── the stale-run reap (#6068) ────────────────────────────────────────────────────
# A SECOND, INDEPENDENT failure mode of the same queue, sharing this file because it
# shares the authoritative read: when the queue rebuilds, the workflow runs of the
# SUPERSEDED group refs stay queued/in-progress and hold capacity the replacement groups
# need. `reap` cancels exactly those, and only those. Its decision table is the same
# shape as the sweep's — every arm that cannot POSITIVELY establish its premise keeps the
# run:
#   SKIP    not a `merge_group` run of this branch's queue, or already terminal
#   KEEP    the run's group is CURRENT, or its group ref still points at the run's head
#   WAIT    the run was registered too close to the queue snapshot to be judged by it
#   REFUSE  the queue, the run's identity, or the group ref could not be read
#   CAP     the per-pass cancellation budget is exhausted
#   CANCEL  absent from TWO queue reads AND the group ref agrees it is gone or rebuilt
# See the DEFAULT_REAP_GRACE_SECONDS block for the incident, the (ref, head_sha) key
# argument, and why the ref is consulted as a second authority.
#
# CLI:
#   python3 scripts/merge-group-watchdog.py sweep --repo O/R [--branch main] [--dry-run]
#   python3 scripts/merge-group-watchdog.py reap --repo O/R [--branch main] [--dry-run]
#   python3 scripts/merge-group-watchdog.py classify-dequeue --repo O/R --pr N --reason R
#   python3 scripts/merge-group-watchdog.py --self-test

from __future__ import annotations

import argparse
import json
import os
import re
import subprocess
import sys
from dataclasses import dataclass
from datetime import datetime, timedelta, timezone
from typing import Callable, Iterable

import gh_retry

PROGRAM = "merge-group-watchdog"

# ── tunables ─────────────────────────────────────────────────────────────────────
# Grace before a zero-suite group is called dropped.
#
# MEASURED, N=209 merge-group refs over 2026-07-25T00:00Z -> 2026-07-28T01:30Z, using
# the activity `branch_creation` timestamp -> earliest check-suite `created_at`:
#     min 1 s | p50 2 s | p90 3 s | p99 4 s | MAX 4 s   (whole-second quantisation)
# There is NO tail between 4 s and 3600 s. The failure mode is CATEGORICAL, not slow:
# a group either gets its suites within 1-4 s or gets none at all, ever.
#
# THREE confirmed zero-dispatch refs, one per day, all reaped by the 60-minute timeout:
#     pr-4331-f6be7767…  2026-07-26  head f44083c7  lifetime 61m07s  suites 0
#     pr-4534-3cc1bf82…  2026-07-27  head 1bfb0174  lifetime 60m27s  suites 0
#     pr-4709-63dfb7a0…  2026-07-28  head 98204656  held pos 1 48m   suites 0
# #4709 was caught LIVE at 05:55Z by this script's own `sweep --dry-run` and cleared by
# hand. Its siblings in the same chain that day discriminate the signal perfectly:
#     pr-4709 -> 0 suites / 0 runs        <-- dead
#     pr-4714 -> 8 suites / 95 runs       (ref built 05:33:08Z, first suite 05:33:10Z)
#     pr-4724 -> 8 suites / 54 runs
#     pr-4729 -> 8 suites / 69 runs
#     pr-4731 -> 8 suites / 93 runs
# Zero against eight, every time. Cost of that one dead ref: #4714 and #4729 were both
# already MERGEABLE behind it and could not land, four PRs stacked, and sparq merged
# ONE PR in sixty minutes.
#
# 120 s is 30x the measured MAXIMUM and 60x the median, and sits strictly below the
# 300 s cron interval, so it never costs an extra tick: any ref that is going to get
# suites has them long before the first tick that could look at it. Detection is
# tick-quantised, so effective recovery is 2-10 min against the 60-minute timeout.
DEFAULT_GRACE_SECONDS = 120
# At most this many watchdog recoveries for one PR inside the rolling window below.
# A re-enqueue mints a NEW queue attempt, so an attempt-scoped cap would be no cap at
# all; the budget is per PR per wall-clock window.
#
# EXHAUSTION BEHAVIOUR, stated as it actually is: the entry is handed back to the
# platform's own 60-minute CI_TIMEOUT, and that dequeue **DEMOTES to review:changes**
# like any other. It does NOT preserve the verdict. The reason is mechanical and worth
# knowing: recovery is marker -> dequeue -> enqueue, so the marker is always ~30 s OLDER
# than the `added_to_merge_queue` it produces; on the next attempt the newest trusted
# marker therefore predates that attempt and the queue-attempt binding refuses it.
#
# That is the CORRECT outcome, not merely a tolerated one. By exhaustion the PR has
# burned three consecutive zero-dispatch groups, the only trusted observation names a
# SUPERSEDED group head, and preserving a verdict on the strength of a stale head's
# suite count would be unsound. A human look is warranted at that point. What exhaustion
# does NOT do is stall: there is no `needs:user`, no permanent hold, and a CAP row is
# emitted every tick for as long as it holds.
DEFAULT_MAX_RECOVERIES_PER_PR = 2
DEFAULT_RECOVERY_WINDOW_SECONDS = 6 * 3600
# Blast-radius bound: one tick may never churn the whole queue.
DEFAULT_MAX_RECOVERIES_PER_RUN = 2
# How long after a watchdog marker a MANUAL/UNKNOWN dequeue is attributed to the
# watchdog's own recovery rather than to a human or the platform.
DEFAULT_SELF_DEQUEUE_WINDOW_SECONDS = 900
# A marker older than this is never allowed to steer a routing decision.
DEFAULT_MARKER_STALENESS_SECONDS = 6 * 3600
# Read a bounded slice of the queue; sparq's queue is single-digit deep.
QUEUE_ENTRY_PAGE = 20

# ── the STALE-RUN REAP (sparq-org/sparq#6068) ────────────────────────────────────
# THE OUTAGE THIS FIXES, observed live on the merge train and reported on #6068: a queue
# entry timed out, GitHub rebuilt the entries behind it onto NEW group refs, and the
# workflow runs belonging to the SUPERSEDED `gh-readonly-queue/main/pr-*` refs stayed
# queued / in progress. Three obsolete group refs were still holding resident
# `ci-summary` waiters plus queued `ci` and feature-matrix runs while the replacement
# group for #6049 waited for capacity. The obsolete unfinished runs named on the issue
# were 33447340157/33447340356/33447340295, 33448217700/33448217922/33448217915 and
# 33448459203/33448459374/33448459399; the replacement group's runs were created
# separately at 33451833782-33451834703, i.e. the old and the new sets were concurrent
# rather than sequential. Cancelling the nine by hand released the capacity. That manual
# step is what `reap` performs, and it is the SAME shape of problem as the zero-dispatch
# recovery above: queue work that outlives the queue state which made it relevant.
#
# WHY THE GROUP KEY IS (ref, head_sha) AND NOT EITHER ALONE. The module header above
# records that a ref NAME is reused when the queue rebuilds a group on the same base —
# measured over the full retained queue population, 15 ref names were created 2+ times
# (max 4x) while head SHAs had ZERO collisions. Keying on the name alone would therefore
# call a rebuilt group current and never reap the superseded runs, which is precisely the
# incident. Keying on the SHA alone would let a run on some other branch's queue match.
# Both together are exact, and head SHA is already the key the recovery idempotence uses.
#
# THE GRACE HERE IS NOT THE DISPATCH-LATENCY GRACE ABOVE, and it is NOT measured — it is
# a bound on how STALE the queue snapshot may be relative to the run listing, which are
# two separate API reads that cannot be taken atomically. A run registered close to (or
# after) the snapshot may belong to a group the snapshot was simply too early to see, so
# it is held rather than judged. 120 s mirrors DEFAULT_GRACE_SECONDS for one
# operator-facing reason: two graces in the same file differing by an unexplained amount
# are two numbers to maintain. It only ever DELAYS a cancellation and can never cause
# one, and the delay is absorbed by the workflow's five-pass loop, not by a cron tick.
DEFAULT_REAP_GRACE_SECONDS = 120
# Blast-radius bound, mirroring DEFAULT_MAX_RECOVERIES_PER_RUN: one pass may never cancel
# the whole estate. Sized from repo structure, not from taste — merge-group-doorbell.yml
# records that a group head here carries 8 `merge_group` workflow runs, and the #6068
# incident had 3 obsolete groups resident at once, so 24 covers the observed worst case
# exactly. Anything past it is emitted as a CAP row and reconsidered on the next pass.
DEFAULT_MAX_CANCELLATIONS_PER_PASS = 24
# The two run statuses the reap asks the API for. A `completed` run holds no capacity and
# is never a candidate; these are the two the #6068 evidence lists.
REAP_RUN_STATUSES = ("queued", "in_progress")
# ...and the statuses a listed run may still legitimately be in when we judge it. A run
# that finished between the listing and the decision is a SKIP, not a cancellation.
UNFINISHED_RUN_STATUSES = frozenset(
    {"queued", "in_progress", "waiting", "requested", "pending"}
)
# The ONLY event a reap may ever consider. A `pull_request` run's head_branch is the PR's
# own branch and a `push` run's is a real branch; neither can be spoken for by merge-queue
# state, so both are refused structurally rather than by a name heuristic.
MERGE_GROUP_EVENT = "merge_group"
GROUP_REF_PREFIX = "gh-readonly-queue/"
_SHA_RE = re.compile(r"[0-9a-f]{40}")
# `gh` surfaces the HTTP status in its stderr. A cancel of a run that has already reached
# a terminal state answers 409, which is the IDEMPOTENT case (duplicate doorbell delivery,
# or the run finishing on its own between the listing and the POST) and not a failure.
_HTTP_409_RE = re.compile(r"\bHTTP 409\b")
# A 404 from the git-ref API is the POSITIVE reading "this ref does not exist", not a
# failure to read. gh_retry classifies 404 as non-transient and raises on the first
# attempt, so this never races a retry loop.
_HTTP_404_RE = re.compile(r"\bHTTP 404\b")

# ── THE IDENTITY THIS ALL RESTS ON, and why it is not a new assumption ────────────
# The reap matches a workflow run to a queue entry by `run.head_sha == entry.head_oid`.
# That identity is ALREADY load-bearing in production in this very file: the zero-dispatch
# predicate counts check-suites on `entry.head_oid` and HOLDs when it finds them, and
# merge-group-doorbell.yml records the measurement behind it — a check-suite is created
# 1:1 per dispatched `merge_group` workflow RUN, 8 runs and exactly 8 suites on group head
# 6b291b28 and the same 8/8 on three more. A run's check-suite hangs off that run's
# `head_sha`, so those suites being ON `entry.head_oid` IS the two fields being equal.
#
# The failure directions are NOT symmetric, which is why the corroboration below exists.
# If `head_branch` were not the queue ref, the prefix check excludes everything and the
# reap does nothing — visible in the rows, and safe. If the SHAs did not correspond, every
# `merge_group` run would look unmatched and the reap would want to cancel the live queue.
# So obsolescence is never concluded from the queue snapshot alone: the group REF ITSELF
# is consulted immediately before cancelling, and a ref that still points AT the run's own
# head keeps the run whatever the queue snapshot said. Two authoritative sources
# disagreeing is not a licence to cancel.
REF_GONE = "gone"
REF_AT = "at"

# Only an entry that HAS a built group and is waiting on it can be zero-dispatch.
# QUEUED/UNMERGEABLE have no group to be dropped; MERGEABLE already reported;
# LOCKED is the queue's own transient.
ACTIONABLE_STATES = frozenset({"AWAITING_CHECKS"})

# Dequeue reasons a `dequeuePullRequest` mutation (ours) can surface as. A
# MERGE_CONFLICT / CI_FAILURE dequeue is never attributed to the watchdog.
SELF_DEQUEUE_REASONS = frozenset({"MANUAL", "UNKNOWN"})
ZERO_DISPATCH_REASON = "CI_TIMEOUT"

# ── sq-fnr85: the QUEUE-LEVEL drop ───────────────────────────────────────────────
# Reasons that describe something the QUEUE did, and that therefore carry no verdict
# whatsoever about the pull request's diff:
#   * QUEUE_CLEARED — the queue was emptied out from under this entry.
#   * ROLL_BACK     — the group was rolled back (an entry ahead changed the base).
# In both, a group can have been built and gone green and the entry still leaves with
# no merge and the arm cleared, which is exactly the sq-fnr85 report. Demoting those to
# `review:changes` manufactures a full re-review plus a full CI re-run of code nobody
# found fault with — the identical defect class #4652 fixed for zero-dispatch. This is
# not a new judgement either: merge-queue-feedback.yml has been posting "reason
# ${DEQUEUE_REASON} is queue/infra-flavoured (likely not a fault in this PR's diff)" on
# these very dequeues while demoting them anyway.
#
# WHY THE REASON IS ADMISSIBLE EVIDENCE HERE, when the header says never to route on the
# event name. That rule exists because `MANUAL` CONFLATES "a reviewer withdrew this"
# (the verdict SHOULD fall) with "infrastructure moved it" (it should not), so the name
# alone cannot separate them. QUEUE_CLEARED and ROLL_BACK conflate nothing — neither has
# a reading in which the diff is at fault — and the string arrives in the trusted
# webhook payload, not from a forgeable PR comment. `MANUAL` and `UNKNOWN` stay OUT for
# precisely the original reason; `CI_FAILURE` and `MERGE_CONFLICT` are genuine statements
# about the diff and stay out too; `CI_TIMEOUT` keeps its own suite-count split below.
GREEN_DROP_REASONS = frozenset({"QUEUE_CLEARED", "ROLL_BACK"})
# How many queue attempts ONE verdict may ride before a queue-level drop stops being
# preserved. Every automated retry loop in this file is bounded, and this one is no
# different: a tree that has been queued three times and landed none of them is worth a
# human look, so the fourth drop demotes to the fix lane. The budget resets whenever the
# head moves, because a new tree needs a new verdict anyway.
DEFAULT_MAX_PRESERVED_REQUEUES = 3
# The only action a marker may claim. Checked on the self-dequeue arm so that no other
# marker shape can ever be mistaken for a recovery in flight.
MARKER_ACTION_REENQUEUE = "re-enqueue"
# The verdict label whose survival across an infrastructure dequeue is the whole point
# of the routing split.
REVIEW_PASS = "review:pass"

# Verdicts. Named constants so a test cannot silently drift from the emitter.
SKIP = "SKIP"
HOLD = "HOLD"
WAIT = "WAIT"
REFUSE = "REFUSE"
NOOP = "NOOP"
CAP = "CAP"
RECOVER = "RECOVER"
# ...and the two the stale-run reap adds. SKIP / WAIT / REFUSE / CAP are shared with the
# recovery sweep deliberately: an operator reads one row shape across both passes, and
# each verdict means the same thing in both (SKIP = not our population, WAIT = inside a
# grace, REFUSE = could not be positively established, CAP = budget exhausted).
KEEP = "KEEP"
CANCEL = "CANCEL"

ROUTE_PRESERVE = "preserve"
ROUTE_DEMOTE = "demote"

# ── THE MARKER IS EVIDENCE, SO ITS AUTHOR IS PART OF THE PREDICATE ───────────────
# sparq is a PUBLIC repo and anyone can comment on a pull request. A marker read from an
# arbitrary comment author is UNAUTHENTICATED AUTHOR-CONTROLLED INPUT, and the routing
# split ACTS on it: a forged marker makes `review:pass` survive a dequeue and skips the
# arm-disable, so auto-arm.py / rearm-sweeper.py re-arm the PR within ~10 minutes — and
# on the CI_TIMEOUT arm it reaches `gh pr merge --auto` directly. Naming any commit that
# happens to have zero check-suites suffices, and such a sha is trivially obtainable.
#
# Markers are therefore accepted ONLY from this allow-list. This is the load-bearing
# control of the routing split, not a hardening nicety: it is the third occurrence in
# this estate of "the PR's own NAMED control was a forgeable channel", after #681
# (review evidence read from `pull["body"]`) and #937 (declarations from body/title).
# The check lives at the READ, so nothing downstream can forget to apply it.
TRUSTED_MARKER_AUTHORS = frozenset(
    {
        "github-actions[bot]",          # the workflow token's identity
        "sparq-orchestrator[bot]",      # the App used when ORCHESTRATOR_APP_ID is set
    }
)

MARKER_KEY = "merge-group-watchdog"
# BOUNDED ON PURPOSE. `[^>]*?` is quadratic under backtracking: a 65 536-char comment
# of repeated `<!-- merge-group-watchdog ` cost 21.4 s to parse, and four such comments
# took `classify-dequeue` to 63 s — ~19 would exceed merge-queue-feedback.yml's
# 5-minute job timeout. sparq is PUBLIC and that workflow runs on every dequeue, so
# that is an anonymous denial of service against merge-queue routing. A real marker's
# key/value run is 262 chars; 400 is comfortable headroom and makes the parse linear.
_MARKER_RE = re.compile(
    r"<!--\s*" + MARKER_KEY + r"\s+(?P<kv>[^>]{0,400}?)\s*-->",
)

# Contexts in which a marker is being QUOTED rather than asserted. A trusted bot that
# echoes PR content — and `sparq-orchestrator[bot]` really does, quoting file spans on
# worker PRs — would otherwise launder an attacker's marker through the author filter.
# Stripped before matching, so a quoted marker is never seen at all.
_FENCED_RE = re.compile(r"(?ms)^\s*(`{3,}|~{3,}).*?^\s*\1\s*$")
_INDENTED_CODE_RE = re.compile(r"(?m)^(?: {4,}|\t).*$")
_BLOCKQUOTE_RE = re.compile(r"(?m)^\s*>.*$")
_INLINE_CODE_RE = re.compile(r"`[^`\n]*`")


def strip_quoted_contexts(body: str) -> str:
    """Remove fenced/indented code, blockquotes and inline spans from a comment body.

    The watchdog's own marker sits on its own unquoted line, so this never removes a
    genuine marker; it removes every context in which one is being displayed rather
    than claimed.
    """
    for pattern in (_FENCED_RE, _INDENTED_CODE_RE, _BLOCKQUOTE_RE, _INLINE_CODE_RE):
        body = pattern.sub(" ", body)
    return body
_MARKER_FIELD_RE = re.compile(r"(?P<key>[a-z_]+)=(?P<value>\S+)")

QUEUE_QUERY = """query($owner:String!,$name:String!,$branch:String!,$first:Int!){
  repository(owner:$owner,name:$name){
    mergeQueue(branch:$branch){
      entries(first:$first){
        nodes{
          id
          position
          state
          enqueuedAt
          baseCommit{oid}
          headCommit{oid}
          pullRequest{number id}
        }
      }
    }
  }
}"""

# THE REMEDIATION CALL, and the three ways to get it wrong (each cost real time):
#   * `gh pr merge <N> --disable-auto` does NOT dequeue an already-queued PR — it
#     answers "is already queued to merge" and leaves the entry in place.
#   * `DequeuePullRequestInput` names its field `id`, NOT `pullRequestId`, and that
#     field wants the PULL REQUEST node id (`PR_…`) — not the merge-queue ENTRY id
#     (`MQE_…`), which is the natural wrong answer given the field name. (Enqueue, in
#     the other direction, really is `pullRequestId`.)
#   * `MergeQueueEntry` has no `headOid` field; the group head is `headCommit{oid}`.
DEQUEUE_MUTATION = """mutation($id:ID!){
  dequeuePullRequest(input:{id:$id}){ mergeQueueEntry{ id } }
}"""

ENQUEUE_MUTATION = """mutation($id:ID!){
  enqueuePullRequest(input:{pullRequestId:$id}){ mergeQueueEntry{ id position } }
}"""


class GhError(RuntimeError):
    """A GitHub CLI command failed."""


# ── pure helpers (no I/O — the self-test drives these directly) ──────────────────


def parse_iso(value: str | None) -> datetime | None:
    """Parse a GitHub ISO-8601 UTC timestamp. Returns None on anything unparseable."""
    if not value or not isinstance(value, str):
        return None
    text = value.strip()
    if text.endswith("Z"):
        text = text[:-1] + "+00:00"
    try:
        parsed = datetime.fromisoformat(text)
    except ValueError:
        return None
    if parsed.tzinfo is None:
        parsed = parsed.replace(tzinfo=timezone.utc)
    return parsed.astimezone(timezone.utc)


def iso(moment: datetime) -> str:
    return moment.astimezone(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


def queue_ref(branch: str, pr_number: int, base_oid: str) -> str:
    """The merge-group ref GitHub builds for an entry.

    VERIFIED against live data: `gh-readonly-queue/<base_branch>/pr-<N>-<BASE oid>`,
    where <N> is THIS entry's PR and the trailing sha is the group's BASE (the previous
    entry's head), NOT its head. Misreading that suffix as the head hid the #4652 cause
    for an hour, so the head is always carried separately.
    """
    return f"gh-readonly-queue/{branch}/pr-{pr_number}-{base_oid}"


def render_marker(
    *, pr: int, head: str, base: str, ref: str, suites: int, observed: datetime
) -> str:
    """The machine-readable marker line embedded in the watchdog's PR comment."""
    return (
        f"<!-- {MARKER_KEY} pr={pr} head={head} base={base} ref={ref} "
        f"suites={suites} observed={iso(observed)} action=re-enqueue -->"
    )


@dataclass(frozen=True)
class Marker:
    pr: int
    head: str
    base: str
    ref: str
    suites: int
    observed: datetime
    action: str


def parse_marker(body: str | None) -> Marker | None:
    """Parse a watchdog marker out of a comment body; None when absent/malformed."""
    if not body:
        return None
    body = strip_quoted_contexts(body)
    match = _MARKER_RE.search(body)
    if not match:
        return None
    # NESTING DEPTH, not a substring test. `<!-- quoted: <!-- merge-group-watchdog … -->`
    # matches on the INNER opener, whose payload is perfectly clean — so inspecting the
    # match tells you nothing. What matters is whether an enclosing comment was still
    # open at the match position: more `<!--` than `-->` before it means this marker is
    # being displayed inside another comment, not asserted.
    prefix = body[: match.start()]
    if prefix.count("<!--") > prefix.count("-->"):
        return None
    fields = {
        m.group("key"): m.group("value")
        for m in _MARKER_FIELD_RE.finditer(match.group("kv"))
    }
    observed = parse_iso(fields.get("observed"))
    if observed is None:
        return None
    try:
        pr = int(fields["pr"])
        suites = int(fields["suites"])
    except (KeyError, ValueError):
        return None
    head = fields.get("head", "")
    if not re.fullmatch(r"[0-9a-f]{40}", head):
        return None
    # The watchdog only ever records a ZERO observation, so anything else was not
    # written by this tool. Validated so the field is load-bearing rather than dead
    # data — a mutant flipping it to 1 otherwise changed nothing observable.
    if suites != 0:
        return None
    # Self-consistency: `ref` must name the same PR as `pr`. A marker whose two
    # identity fields disagree was not written by this tool.
    ref = fields.get("ref", "")
    if f"/pr-{pr}-" not in ref:
        return None
    return Marker(
        pr=pr,
        head=head,
        base=fields.get("base", ""),
        ref=fields.get("ref", ""),
        suites=suites,
        observed=observed,
        action=fields.get("action", ""),
    )


@dataclass(frozen=True)
class QueueEntry:
    pr_number: int
    pr_id: str
    entry_id: str
    position: int
    state: str
    enqueued_at: datetime | None
    base_oid: str | None
    head_oid: str | None


@dataclass(frozen=True)
class Decision:
    verdict: str
    detail: str


@dataclass(frozen=True)
class Route:
    route: str
    reenqueue: bool
    detail: str


@dataclass(frozen=True)
class VerdictState:
    """What the PR timeline says about the verdict and the head it was granted for."""

    review_pass_at: datetime | None
    head_moved_at: datetime | None
    last_enqueued_at: datetime | None
    readable: bool = True
    # Tracked SEPARATELY from head_moved_at purely so the emitted reason is accurate:
    # folding a label revocation into "the head moved" tells an operator a commit
    # landed when none did, and sends them looking for a push that does not exist.
    verdict_revoked_at: datetime | None = None
    # EVERY `added_to_merge_queue` stamp, not just the newest, because the queue-level
    # drop arm has to COUNT the attempts one verdict has ridden. Defaults to empty, and
    # an empty tuple is read as "the budget could not be evaluated" (=> demote), never
    # as "zero attempts so far" — an unreadable history is not evidence of no history.
    enqueued_at: tuple[datetime, ...] = ()
    # TRUE when a head-moving row (`committed` / `head_ref_force_pushed`) sits AFTER the
    # newest `review:pass` grant in TIMELINE ORDER. Tracked separately from
    # `head_moved_at` because a `committed` row carries NO event timestamp — only the
    # commit's own `committer.date`/`author.date`, which is metadata the pusher chooses
    # (`git commit --date`, `GIT_COMMITTER_DATE`, a cherry-pick that preserves dates).
    # A commit pushed AFTER the grant can therefore claim a date BEFORE it and slide
    # under the `head_moved_at > review_pass_at` comparison, carrying a verdict across a
    # tree nobody reviewed. Where the row LANDS in the timeline GitHub returns is not the
    # pusher's to write, so the two signals are read as a UNION: either one saying the
    # head moved is enough to refuse. That union only ever ADDS refusals — this field
    # defaults to False ("no ordering evidence"), which leaves the date comparison
    # exactly as it stands today and can never relax it.
    head_moved_after_grant_in_order: bool = False


def decide_entry(
    entry: QueueEntry,
    *,
    suites: int | None,
    created_at: datetime | None,
    now: datetime,
    grace_seconds: int,
    markers: Iterable[Marker],
    markers_readable: bool = True,
    recoveries_in_window: int,
    max_recoveries_per_pr: int,
    run_recoveries: int,
    max_recoveries_per_run: int,
) -> Decision:
    """The whole watchdog policy, as a pure function of observed state.

    `suites is None` and `created_at is None` both mean "could NOT be positively
    established" and must never reach a mutation — see the REFUSE arms below.
    """
    if entry.state not in ACTIONABLE_STATES:
        return Decision(SKIP, f"entry state {entry.state} is not AWAITING_CHECKS")
    if not entry.head_oid or not entry.base_oid:
        return Decision(SKIP, "no speculative group built for this entry yet")
    if suites is None:
        return Decision(
            REFUSE,
            "check-suite count could not be positively established — taking no action",
        )
    if suites > 0:
        return Decision(
            HOLD,
            f"{suites} check-suite(s) present — the merge_group event was dispatched",
        )
    if not markers_readable:
        return Decision(
            REFUSE,
            "this PR's marker history could not be read — an unreadable history is "
            "NOT evidence of no prior recovery, so the caps cannot be evaluated",
        )
    if created_at is None:
        return Decision(
            REFUSE,
            "no branch_creation activity row matches this group head — "
            "the group's build time could not be established, taking no action",
        )
    age = (now - created_at).total_seconds()
    if age < 0:
        return Decision(
            REFUSE,
            f"group ref creation {iso(created_at)} is in the future — clock skew, "
            "taking no action",
        )
    if age < grace_seconds:
        return Decision(
            WAIT,
            f"zero check-suites {int(age)}s after the ref was built; "
            f"inside the {grace_seconds}s grace",
        )
    if any(marker.head == entry.head_oid for marker in markers):
        return Decision(
            NOOP,
            "a recovery was already attempted for this exact group head — "
            "repeat detection on the same ref is a no-op",
        )
    if recoveries_in_window >= max_recoveries_per_pr:
        return Decision(
            CAP,
            f"per-PR recovery budget exhausted ({recoveries_in_window}/"
            f"{max_recoveries_per_pr}) — handing back to the platform CI_TIMEOUT, which "
            "WILL demote to review:changes (the only trusted observation names a "
            "superseded group head, so the verdict cannot be preserved soundly)",
        )
    if run_recoveries >= max_recoveries_per_run:
        return Decision(
            CAP,
            f"per-run recovery budget exhausted ({run_recoveries}/"
            f"{max_recoveries_per_run}) — the next tick re-evaluates",
        )
    return Decision(
        RECOVER,
        f"zero check-suites {int(age)}s after the ref was built "
        f"(grace {grace_seconds}s) — dequeue + re-enqueue",
    )


def verdict_is_still_for_this_tree(state: VerdictState) -> tuple[bool, str]:
    """May the review verdict survive this dequeue?

    NON-NEGOTIABLE PRECONDITION on every preserve route. A verdict attests to a
    specific TREE; if the head has moved since `review:pass` was granted, the label
    describes code nobody reviewed and it must be allowed to fall off. Measured on
    #4709: `review:pass` at 05:14:56Z for head `c84c0810d681`, last commit 04:56:39Z,
    zero head-moving events between — which is exactly the check that made restoring it
    legitimate. Anything unprovable is a refusal to preserve.
    """
    if not state.readable:
        return False, "the PR timeline could not be read"
    if state.review_pass_at is None:
        return False, "no review:pass grant found on the timeline — no verdict to keep"
    if (
        state.verdict_revoked_at is not None
        and state.verdict_revoked_at > state.review_pass_at
    ):
        return False, (
            f"review:pass was REVOKED at {iso(state.verdict_revoked_at)}, after the "
            f"grant at {iso(state.review_pass_at)} — there is no live verdict to keep"
        )
    if state.head_moved_at is not None and state.head_moved_at > state.review_pass_at:
        return False, (
            f"the head moved at {iso(state.head_moved_at)}, after review:pass was "
            f"granted at {iso(state.review_pass_at)} — the verdict is not for this tree"
        )
    # Checked AFTER the date comparison so the more precise "moved at <time>" reason wins
    # whenever the dates are honest. This arm is what catches the dishonest ones: a
    # `committed` row dates ITSELF, so a commit pushed after the grant can claim any
    # earlier date it likes and pass the comparison above. Where the row LANDS is not
    # the pusher's to write, so a head-moving row recorded after the newest grant
    # forfeits the verdict on ordering alone, whatever the commit says its date was.
    if state.head_moved_after_grant_in_order:
        return False, (
            "a head-moving timeline event is recorded AFTER the review:pass grant at "
            f"{iso(state.review_pass_at)} — the verdict is not for this tree (the "
            "commit's self-reported date claims otherwise; timeline order decides)"
        )
    return True, f"review:pass granted {iso(state.review_pass_at)} and the head has not moved since"


def classify_dequeue_route(
    *,
    reason: str,
    pr_number: int,
    markers: Iterable[Marker],
    verdict: VerdictState,
    live_suites: Callable[[str], int | None],
    now: datetime,
    self_dequeue_window_seconds: int = DEFAULT_SELF_DEQUEUE_WINDOW_SECONDS,
    marker_staleness_seconds: int = DEFAULT_MARKER_STALENESS_SECONDS,
    max_preserved_requeues: int = DEFAULT_MAX_PRESERVED_REQUEUES,
) -> Route:
    """Split a dequeue into the preserve-the-verdict route and today's demote route.

    FAIL-SAFE: every arm that cannot POSITIVELY establish its own premise returns
    `demote`, which is the pre-existing behaviour.
    """
    # BIND TO THIS PR. A marker is evidence about ONE pull request; without this a
    # marker written for a different PR (or naming a foreign zero-suite head) carries a
    # verdict through this PR's dequeue. Both the `pr=` field and the `ref=` segment
    # must agree with the PR being routed, so a single mistyped field cannot pass.
    own = [
        marker
        for marker in markers
        if marker.pr == pr_number and f"/pr-{pr_number}-" in marker.ref
    ]
    ordered = sorted(own, key=lambda m: m.observed, reverse=True)
    newest = ordered[0] if ordered else None
    normalised = (reason or "UNKNOWN").strip().upper()
    last_enqueued_at = verdict.last_enqueued_at

    # Applies to EVERY preserve arm, before any of them is considered. It is deliberately
    # ahead of the marker guard below: arm (c) needs no marker, so a precondition placed
    # after that guard would not cover it, and a non-negotiable control that only some
    # arms run is not a control at all.
    keepable, why = verdict_is_still_for_this_tree(verdict)
    if not keepable:
        return Route(ROUTE_DEMOTE, False, f"verdict may not survive this dequeue: {why}")

    # (c) the QUEUE-LEVEL drop (sq-fnr85). No marker is consulted, and none exists for
    # this population: the watchdog only records observations for zero-suite groups it
    # rescues, whereas these entries had a perfectly healthy group and were removed for
    # a reason belonging to the QUEUE, not to the diff (see GREEN_DROP_REASONS).
    if normalised in GREEN_DROP_REASONS:
        if last_enqueued_at is None:
            return Route(
                ROUTE_DEMOTE,
                False,
                f"{normalised} drop, but no added_to_merge_queue timeline event was "
                "found — this dequeue cannot be bound to a queue attempt",
            )
        # Non-None here: `verdict_is_still_for_this_tree` above refuses without a grant.
        granted = verdict.review_pass_at
        # Only attempts made on THIS verdict count. A queue attempt that predates the
        # grant belongs to a tree whose verdict is already gone, and charging it here
        # would spend the budget of a tree that has never been queued at all.
        ridden = [stamp for stamp in verdict.enqueued_at if stamp >= granted]
        if not ridden:
            return Route(
                ROUTE_DEMOTE,
                False,
                f"{normalised} drop, but no enqueue event is readable at or after the "
                f"review:pass grant ({iso(granted)}) — the re-queue budget cannot be "
                "evaluated, and an uncountable budget is not a spent one",
            )
        if len(ridden) >= max_preserved_requeues:
            return Route(
                ROUTE_DEMOTE,
                False,
                f"{normalised} drop, but this verdict has already ridden {len(ridden)} "
                f"queue attempt(s) without landing (budget {max_preserved_requeues}) — "
                "handing it to the fix lane so a human looks at why it never lands",
            )
        return Route(
            ROUTE_PRESERVE,
            True,
            f"{normalised} is a QUEUE-LEVEL removal and carries no verdict about this "
            f"diff (attempt {len(ridden)}/{max_preserved_requeues} on this tree) — "
            "verdict preserved and the PR re-armed",
        )

    if newest is None:
        return Route(
            ROUTE_DEMOTE,
            False,
            "no merge-group-watchdog observation recorded for this PR",
        )

    marker_age = (now - newest.observed).total_seconds()

    # (a) our OWN recovery. The watchdog's dequeue+enqueue fires pull_request.dequeued;
    # without this arm the watchdog would demote every PR it just rescued.
    if (
        normalised in SELF_DEQUEUE_REASONS
        and newest.action == MARKER_ACTION_REENQUEUE
        and 0 <= marker_age <= self_dequeue_window_seconds
    ):
        # DEFENCE IN DEPTH behind the author allow-list. This arm previously preserved on
        # the marker alone and compared its head to NOTHING, so "names that exact group
        # head" was not what the code implemented. Re-derive the count for the head the
        # marker names: a head that does not exist reads None, and one that was really
        # dispatched reads non-zero. Neither may carry a verdict through a dequeue.
        own = live_suites(newest.head)
        if own is None:
            return Route(
                ROUTE_DEMOTE,
                False,
                f"marker names head {newest.head[:8]} whose check-suite count cannot be "
                "read — not provably the watchdog's own recovery",
            )
        if own != 0:
            return Route(
                ROUTE_DEMOTE,
                False,
                f"marker names head {newest.head[:8]}, which has {own} check-suite(s) — "
                "that group WAS dispatched, so this is not a zero-dispatch recovery",
            )
        return Route(
            ROUTE_PRESERVE,
            False,
            f"dequeue reason {normalised} within {self_dequeue_window_seconds}s of the "
            f"watchdog's own recovery of head {newest.head[:8]} ({int(marker_age)}s ago, "
            "0 check-suites re-derived live) — the watchdog re-enqueues; verdict preserved",
        )

    # (b) the platform timeout. Split by SUITE COUNT, never by the reason alone.
    if normalised != ZERO_DISPATCH_REASON:
        return Route(
            ROUTE_DEMOTE,
            False,
            f"dequeue reason {normalised} is not {ZERO_DISPATCH_REASON}",
        )
    if last_enqueued_at is None:
        return Route(
            ROUTE_DEMOTE,
            False,
            "no added_to_merge_queue timeline event found — the watchdog observation "
            "cannot be bound to this queue attempt",
        )
    if newest.observed < last_enqueued_at:
        return Route(
            ROUTE_DEMOTE,
            False,
            f"newest watchdog observation {iso(newest.observed)} predates this queue "
            f"attempt (enqueued {iso(last_enqueued_at)})",
        )
    if marker_age < 0 or marker_age > marker_staleness_seconds:
        return Route(
            ROUTE_DEMOTE,
            False,
            f"watchdog observation is {int(marker_age)}s old — outside the "
            f"{marker_staleness_seconds}s freshness bound",
        )
    if newest.action != MARKER_ACTION_REENQUEUE:
        return Route(
            ROUTE_DEMOTE,
            False,
            f"marker action {newest.action!r} is not {MARKER_ACTION_REENQUEUE!r}",
        )
    count = live_suites(newest.head)
    if count is None:
        return Route(
            ROUTE_DEMOTE,
            False,
            f"could not re-derive the check-suite count for head {newest.head[:8]}",
        )
    if count != 0:
        return Route(
            ROUTE_DEMOTE,
            False,
            f"head {newest.head[:8]} has {count} check-suite(s) — this timeout "
            "followed checks that genuinely ran",
        )
    return Route(
        ROUTE_PRESERVE,
        True,
        f"{ZERO_DISPATCH_REASON} with 0 check-suites re-derived live on group head "
        f"{newest.head[:8]} — the merge_group event was never dispatched, so the diff "
        "is not at fault; verdict preserved and the PR re-armed",
    )


@dataclass(frozen=True)
class WorkflowRun:
    """One row of `GET /repos/{repo}/actions/runs`, reduced to what the reap decides on."""

    run_id: int
    name: str
    event: str
    status: str
    head_branch: str
    head_sha: str
    created_at: datetime | None


def group_key(ref: str, head_sha: str) -> tuple[str, str]:
    """The identity of a merge-group build. See the (ref, head_sha) note in the tunables."""
    return (ref, head_sha)


def decide_run(
    run: WorkflowRun,
    *,
    current_groups: frozenset[tuple[str, str]] | None,
    branch: str,
    snapshot_at: datetime,
    grace_seconds: int,
) -> Decision:
    """Is this workflow run POSITIVELY obsolete? Pure function of observed state.

    `current_groups is None` means the authoritative queue state could not be read, and
    is a REFUSE — never a cancellation. The whole point of this function is that the only
    path to CANCEL requires a positive reading of BOTH sides: an authoritative queue
    snapshot that does NOT contain this run's group, and a run whose own identity
    (id, event, ref, sha, registration time) is fully established.

    `snapshot_at` is the instant the authoritative queue state was read, NOT "now": the
    grace has to be measured against the snapshot the decision is made from, because that
    snapshot is what may be too early to have seen the run's group.
    """
    # STRUCTURAL EXCLUSIONS FIRST. The event and ref-prefix checks are what stop the reap
    # ever reaching a PR-head run, a push run, or another branch's queue, and neither is a
    # name heuristic. The status check then drops a run that has already finished.
    if run.event != MERGE_GROUP_EVENT:
        return Decision(
            SKIP,
            f"event {run.event or '(none)'} is not {MERGE_GROUP_EVENT} — merge-queue "
            "state says nothing about this run",
        )
    prefix = f"{GROUP_REF_PREFIX}{branch}/"
    if not run.head_branch.startswith(prefix):
        return Decision(
            SKIP,
            f"head_branch {run.head_branch or '(none)'} is not under {prefix} — the "
            f"{branch} queue is not authoritative for it",
        )
    if run.status not in UNFINISHED_RUN_STATUSES:
        return Decision(
            SKIP, f"run status {run.status or '(none)'} is not unfinished — no capacity held"
        )
    if run.run_id <= 0:
        return Decision(REFUSE, "the run carries no usable id — it cannot be identified")
    if current_groups is None:
        return Decision(
            REFUSE,
            "the merge-queue state could not be read — an unreadable queue is NOT an "
            "empty queue, so no run can be called obsolete",
        )
    if not _SHA_RE.fullmatch(run.head_sha):
        return Decision(
            REFUSE,
            f"head_sha {run.head_sha or '(none)'} is not a full commit sha — the run's "
            "group cannot be identified",
        )
    if group_key(run.head_branch, run.head_sha) in current_groups:
        return Decision(
            KEEP,
            f"{run.head_branch}@{run.head_sha[:8]} is a CURRENT merge-queue group",
        )
    if run.created_at is None:
        return Decision(
            REFUSE,
            "the run's creation time could not be established — the registration grace "
            "cannot be evaluated",
        )
    age = (snapshot_at - run.created_at).total_seconds()
    if age < 0:
        return Decision(
            WAIT,
            f"the run was registered {int(-age)}s AFTER the queue snapshot — that "
            "snapshot cannot speak for it",
        )
    if age < grace_seconds:
        return Decision(
            WAIT,
            f"registered {int(age)}s before the queue snapshot; inside the "
            f"{grace_seconds}s registration grace",
        )
    return Decision(
        CANCEL,
        f"group {run.head_branch}@{run.head_sha[:8]} is not in the merge queue and the "
        f"run predates the queue snapshot by {int(age)}s — obsolete",
    )


# ── I/O layer ────────────────────────────────────────────────────────────────────


def run_gh(argv: list[str]) -> str:
    result = subprocess.run(["gh", *argv], capture_output=True, text=True, check=False)
    if result.returncode != 0:
        detail = result.stderr.strip() or result.stdout.strip() or "unknown gh failure"
        raise GhError(f"gh {' '.join(argv[:3])} failed: {detail}")
    return result.stdout


def run_gh_read(argv: list[str]) -> str:
    """Idempotent READ path: bounded transient-only retries (#3759 helper)."""
    try:
        return gh_retry.run_gh_read(argv)
    except gh_retry.GhFatalError as error:
        raise GhError(str(error)) from error


def _ndjson(text: str) -> list[dict]:
    out: list[dict] = []
    for line in text.splitlines():
        line = line.strip()
        if not line:
            continue
        try:
            parsed = json.loads(line)
        except json.JSONDecodeError as error:
            raise GhError(f"gh api returned a non-JSON page line: {error}") from error
        if isinstance(parsed, dict):
            out.append(parsed)
    return out


class Watchdog:
    def __init__(
        self,
        repo: str,
        *,
        branch: str = "main",
        grace_seconds: int = DEFAULT_GRACE_SECONDS,
        max_recoveries_per_pr: int = DEFAULT_MAX_RECOVERIES_PER_PR,
        recovery_window_seconds: int = DEFAULT_RECOVERY_WINDOW_SECONDS,
        max_recoveries_per_run: int = DEFAULT_MAX_RECOVERIES_PER_RUN,
        reap_grace_seconds: int = DEFAULT_REAP_GRACE_SECONDS,
        max_cancellations: int = DEFAULT_MAX_CANCELLATIONS_PER_PASS,
        dry_run: bool = False,
        # LATE-BOUND ON PURPOSE. `gh: Callable = run_gh` binds the module function at
        # DEFINITION time, so a test that patches `merge_group_watchdog.run_gh` does not
        # change it and the real runner is used anyway. That defect made this suite post
        # 567 real comments to production PR #4534 during a mutation sweep: main() passes
        # gh_read= explicitly (late-bound, patched) but let `gh` fall through to the
        # stale default, so every simulated sweep issued a REAL `gh pr comment`.
        # None means "resolve at call time", which is what a test patch can reach.
        gh: Callable[[list[str]], str] | None = None,
        gh_read: Callable[[list[str]], str] | None = None,
        log: Callable[[str], None] = print,
        now: Callable[[], datetime] | None = None,
    ) -> None:
        try:
            self.owner, self.name = repo.split("/", 1)
        except ValueError as error:
            raise ValueError("repo must be OWNER/REPOSITORY") from error
        if not self.owner or not self.name or "/" in self.name:
            raise ValueError("repo must be OWNER/REPOSITORY")
        if grace_seconds < 60:
            raise ValueError("grace_seconds must be at least 60")
        # Same floor, same reason: a grace short enough to be inside the read-to-read
        # skew of two GitHub API calls is not a grace at all, and this one guards a
        # CANCELLATION. Rejected at construction so no call site can tune it to zero.
        if reap_grace_seconds < 60:
            raise ValueError("reap_grace_seconds must be at least 60")
        if max_cancellations < 0:
            raise ValueError("max_cancellations must not be negative")
        self.repo = repo
        self.branch = branch
        self.grace_seconds = grace_seconds
        self.max_recoveries_per_pr = max_recoveries_per_pr
        self.recovery_window_seconds = recovery_window_seconds
        self.max_recoveries_per_run = max_recoveries_per_run
        self.reap_grace_seconds = reap_grace_seconds
        self.max_cancellations = max_cancellations
        self.dry_run = dry_run
        self.gh = gh if gh is not None else run_gh
        self.gh_read = gh_read if gh_read is not None else self.gh
        self.log = log
        self._now = now or (lambda: datetime.now(timezone.utc))

    # -- reads ---------------------------------------------------------------

    def _graphql(self, query: str, **variables: object) -> dict:
        argv = ["api", "graphql", "-f", f"query={query}"]
        for key, value in variables.items():
            flag = "-F" if isinstance(value, (int, bool)) else "-f"
            argv += [flag, f"{key}={value}"]
        response = json.loads(self.gh_read(argv))
        if response.get("errors"):
            raise GhError(f"GraphQL returned errors: {response['errors']}")
        return response.get("data") or {}

    def queue_entries(self) -> list[QueueEntry]:
        data = self._graphql(
            QUEUE_QUERY,
            owner=self.owner,
            name=self.name,
            branch=self.branch,
            first=QUEUE_ENTRY_PAGE,
        )
        queue = ((data.get("repository") or {}).get("mergeQueue")) or {}
        nodes = ((queue.get("entries") or {}).get("nodes")) or []
        entries: list[QueueEntry] = []
        for node in nodes:
            if not isinstance(node, dict):
                continue
            pull = node.get("pullRequest") or {}
            entries.append(
                QueueEntry(
                    pr_number=int(pull.get("number", 0)),
                    pr_id=str(pull.get("id", "")),
                    entry_id=str(node.get("id", "")),
                    position=int(node.get("position", 0)),
                    state=str(node.get("state", "")).upper(),
                    enqueued_at=parse_iso(node.get("enqueuedAt")),
                    base_oid=(node.get("baseCommit") or {}).get("oid"),
                    head_oid=(node.get("headCommit") or {}).get("oid"),
                )
            )
        return entries

    def current_group_keys(self) -> frozenset[tuple[str, str]]:
        """(ref, head_sha) for every entry whose speculative group is BUILT.

        Derived from the SAME authoritative `mergeQueue` GraphQL read the recovery sweep
        decides on, so the reap and the sweep can never disagree about what the queue
        contains. An entry with no built group contributes no key and needs none: there
        is no group ref for it yet, so no run can exist for it either.

        Propagates GhError rather than returning a sentinel — a reap that cannot read the
        queue must abort loudly, and an empty set would be indistinguishable from an
        empty QUEUE, which is the one confusion that would cancel everything.
        """
        keys = set()
        for entry in self.queue_entries():
            if entry.base_oid and entry.head_oid:
                keys.add(
                    group_key(
                        queue_ref(self.branch, entry.pr_number, entry.base_oid),
                        entry.head_oid,
                    )
                )
        return frozenset(keys)

    def unfinished_merge_group_runs(self, status: str) -> list[WorkflowRun] | None:
        """One page of unfinished `merge_group` runs, or None if not establishable.

        BOUNDED ON PURPOSE — page 1 only, no `--paginate`. The population is the queue
        depth times the number of `merge_group` subscribers, i.e. tens; if it is ever
        larger than a page the excess is announced and picked up by the next pass, which
        is strictly better than an unbounded pagination loop inside a 10-minute job.
        `total_count` is cross-checked against the array length exactly as
        `check_suite_count` does, so a truncated or garbled response is never mistaken
        for a positively-read listing.
        """
        try:
            raw = self.gh_read(
                [
                    "api",
                    "-X",
                    "GET",
                    f"repos/{self.repo}/actions/runs"
                    f"?event={MERGE_GROUP_EVENT}&status={status}"
                    "&per_page=100&exclude_pull_requests=true",
                ]
            )
            payload = json.loads(raw)
        except (GhError, json.JSONDecodeError):
            return None
        if not isinstance(payload, dict):
            return None
        total = payload.get("total_count")
        rows = payload.get("workflow_runs")
        # `isinstance(False, int)` is True — reject a bool explicitly, as above.
        if isinstance(total, bool) or not isinstance(total, int):
            return None
        if not isinstance(rows, list):
            return None
        if total <= 100 and len(rows) != total:
            return None
        if total > len(rows):
            self.log(
                f"::warning title={PROGRAM} run listing truncated::status={status}: "
                f"{total} unfinished {MERGE_GROUP_EVENT} runs, {len(rows)} readable on "
                "page 1 — the remainder is left for the next pass"
            )
        runs: list[WorkflowRun] = []
        for row in rows:
            if not isinstance(row, dict):
                return None
            try:
                run_id = int(row.get("id") or 0)
            except (TypeError, ValueError):
                run_id = 0
            runs.append(
                WorkflowRun(
                    run_id=run_id,
                    name=str(row.get("name") or ""),
                    event=str(row.get("event") or ""),
                    status=str(row.get("status") or ""),
                    head_branch=str(row.get("head_branch") or ""),
                    head_sha=str(row.get("head_sha") or ""),
                    created_at=parse_iso(row.get("created_at")),
                )
            )
        return runs

    def group_ref_state(self, ref: str) -> tuple[str, str] | None:
        """Where a merge-group ref points RIGHT NOW, or None if not establishable.

        `(REF_GONE, "")` — the ref does not exist. This is the unambiguous reading of
        "obsolete": a group being tested HAS a live ref (the #4652 incident ref was
        created at 23:04:37Z and deleted at 00:05:04Z), and the queue deletes it when the
        entry merges or leaves.

        `(REF_AT, sha)` — the ref exists and points at `sha`. A tip that differs from the
        run's head means the group was rebuilt on the same base, which is the recurring-
        ref-name case the module header measures. A tip EQUAL to the run's head means the
        group is still live under this exact commit, whatever the queue snapshot said.

        Anything else is None, and None keeps the run.
        """
        try:
            raw = self.gh_read(["api", "-X", "GET", f"repos/{self.repo}/git/ref/heads/{ref}"])
        except GhError as error:
            if _HTTP_404_RE.search(str(error)):
                return (REF_GONE, "")
            return None
        try:
            payload = json.loads(raw)
        except json.JSONDecodeError:
            return None
        if not isinstance(payload, dict):
            return None
        obj = payload.get("object")
        if not isinstance(obj, dict):
            return None
        sha = obj.get("sha")
        if not isinstance(sha, str) or not _SHA_RE.fullmatch(sha):
            return None
        return (REF_AT, sha)

    def check_suite_count(self, sha: str) -> int | None:
        """`total_count` of check-suites on a commit, or None if not establishable.

        The authoritative counter is `total_count`; page 1's array length is read as a
        cross-check so a truncated/garbled response can never be mistaken for a
        positively-observed zero.
        """
        try:
            raw = self.gh_read(
                [
                    "api",
                    "-X",
                    "GET",
                    f"repos/{self.repo}/commits/{sha}/check-suites?per_page=100",
                ]
            )
            payload = json.loads(raw)
        except (GhError, json.JSONDecodeError):
            return None
        if not isinstance(payload, dict):
            return None
        total = payload.get("total_count")
        suites = payload.get("check_suites")
        # `isinstance(False, int)` is True, so a bool must be rejected EXPLICITLY:
        # `{"total_count": false}` otherwise reads as a positively-observed ZERO and
        # drives a recovery + marker write + preserve off a malformed response.
        if isinstance(total, bool) or not isinstance(total, int):
            return None
        if not isinstance(suites, list):
            return None
        if total == 0 and suites:
            return None
        return total

    def group_created_at(self, ref: str, head_oid: str) -> datetime | None:
        """Exact ref-build time from the repository activity API, matched on head SHA."""
        try:
            raw = self.gh_read(
                [
                    "api",
                    "-X",
                    "GET",
                    "--paginate",
                    "-q",
                    ".[]|@json",
                    f"repos/{self.repo}/activity"
                    f"?activity_type=branch_creation&per_page=100"
                    f"&ref=refs/heads/{ref}",
                ]
            )
            rows = _ndjson(raw)
        except GhError:
            return None
        stamps = [
            parse_iso(row.get("timestamp"))
            for row in rows
            if row.get("activity_type") == "branch_creation"
            and str(row.get("after", "")) == head_oid
        ]
        stamps = [stamp for stamp in stamps if stamp is not None]
        return max(stamps) if stamps else None

    def pr_markers(self, pr_number: int) -> list[Marker]:
        """Trusted markers only — see TRUSTED_MARKER_AUTHORS.

        A well-formed marker from an untrusted author is DROPPED and COUNTED, so a
        forgery attempt is visible in the log rather than silently discarded.
        """
        raw = self.gh_read(
            [
                "api",
                "-X",
                "GET",
                "--paginate",
                "-q",
                ".[]|@json",
                f"repos/{self.repo}/issues/{pr_number}/comments?per_page=100",
            ]
        )
        trusted: list[Marker] = []
        rejected = 0
        rejected_authors: set[str] = set()
        for row in _ndjson(raw):
            # AUTHOR FIRST, ALWAYS. The trust check used to run AFTER parse_marker,
            # so every anonymous comment was parsed before being discarded — a filter
            # placed after the expensive work does not protect the expensive work.
            # Cheap string compare first: untrusted bodies are never parsed at all.
            author = str((row.get("user") or {}).get("login", ""))
            if author not in TRUSTED_MARKER_AUTHORS:
                if MARKER_KEY in str(row.get("body") or ""):
                    rejected += 1
                    rejected_authors.add(author)
                continue
            marker = parse_marker(row.get("body"))
            if marker is not None:
                trusted.append(marker)
            continue
        if rejected:
            # ONE annotation per PR, not one per marker. MEASURED on #4534, which
            # already carries 96 forged markers naming the real zero-suite head: a
            # per-marker warning emitted 96 annotations and ~110 KB of log, which
            # would blow GitHub's annotation cap and bury every other signal. The
            # count and the distinct authors are what an operator needs.
            self.log(
                f"::warning title={PROGRAM} untrusted markers ignored::PR #{pr_number}: "
                f"{rejected} watchdog marker(s) from untrusted author(s) "
                f"{sorted(rejected_authors)} IGNORED ({len(trusted)} trusted accepted) — "
                f"markers are honoured only from {sorted(TRUSTED_MARKER_AUTHORS)}"
            )
        return trusted

    def verdict_state(self, pr_number: int) -> VerdictState:
        """One timeline read for all three facts the routing decision needs.

        A `committed` timeline event carries no `created_at` — its time is the
        commit's own `committer.date`. `head_ref_force_pushed` is a normal event with
        `created_at`. Both move the head, so both count.

        A commit's dates are written by whoever made it, so they are NOT evidence of when
        it reached the pull request. The row's INDEX in the timeline is, so the position
        of the last head-moving row relative to the newest `review:pass` grant is
        recorded alongside the dates and both are handed to
        `verdict_is_still_for_this_tree`.
        """
        try:
            raw = self.gh_read(
                [
                    "api",
                    "-X",
                    "GET",
                    "--paginate",
                    "-q",
                    ".[]|@json",
                    f"repos/{self.repo}/issues/{pr_number}/timeline?per_page=100",
                ]
            )
            rows = _ndjson(raw)
        except GhError:
            return VerdictState(None, None, None, readable=False)

        enqueued: list[datetime] = []
        granted: list[datetime] = []
        moved: list[datetime] = []
        revoked: list[datetime] = []
        # -1 = "never seen". A head-moving row is counted here even when its dates are
        # missing or unparseable, so a commit that reports no date at all still forfeits
        # the verdict instead of vanishing from the check entirely.
        last_grant_index = -1
        last_move_index = -1
        for index, row in enumerate(rows):
            event = row.get("event")
            stamp = parse_iso(row.get("created_at"))
            if event == "added_to_merge_queue" and stamp:
                enqueued.append(stamp)
            elif event == "labeled" and stamp:
                if str((row.get("label") or {}).get("name", "")).lower() == REVIEW_PASS:
                    granted.append(stamp)
                    last_grant_index = index
            elif event == "unlabeled" and stamp:
                # A verdict that was REMOVED is not a verdict, and it is recorded as a
                # revocation rather than a head move so the reason stays truthful.
                if str((row.get("label") or {}).get("name", "")).lower() == REVIEW_PASS:
                    revoked.append(stamp)
            elif event == "head_ref_force_pushed" and stamp:
                moved.append(stamp)
                last_move_index = index
            elif event == "committed":
                last_move_index = index
                committed = parse_iso(
                    (row.get("committer") or {}).get("date")
                ) or parse_iso((row.get("author") or {}).get("date"))
                if committed:
                    moved.append(committed)
        return VerdictState(
            review_pass_at=max(granted) if granted else None,
            head_moved_at=max(moved) if moved else None,
            last_enqueued_at=max(enqueued) if enqueued else None,
            enqueued_at=tuple(sorted(enqueued)),
            verdict_revoked_at=max(revoked) if revoked else None,
            head_moved_after_grant_in_order=(
                last_grant_index >= 0 and last_move_index > last_grant_index
            ),
        )

    # -- mutations -----------------------------------------------------------

    def post_marker_comment(self, entry: QueueEntry, ref: str, observed: datetime) -> None:
        marker = render_marker(
            pr=entry.pr_number,
            head=entry.head_oid or "",
            base=entry.base_oid or "",
            ref=ref,
            suites=0,
            observed=observed,
        )
        body = "\n".join(
            [
                "> 🤖 **SPARQ agent** — merge-group watchdog.",
                "",
                f"This PR's merge-group ref `{ref}` (head `{(entry.head_oid or '')[:8]}`)"
                " has **zero check-suites** — GitHub Actions never dispatched the"
                " `merge_group` event for it, so no required check can ever report and"
                " the entry would hold the head of the queue until the ruleset's"
                " 60-minute `CI_TIMEOUT` fires (sparq-org/sparq#4652).",
                "",
                "Recovering by dequeue + re-enqueue so the group is rebuilt. **Nothing"
                " is wrong with this PR's diff**; its review verdict is preserved.",
                "",
                marker,
            ]
        )
        self.gh(["pr", "comment", str(entry.pr_number), "--repo", self.repo, "--body", body])

    def recover(self, entry: QueueEntry) -> None:
        self.gh(
            [
                "api",
                "graphql",
                "-f",
                f"query={DEQUEUE_MUTATION}",
                "-f",
                f"id={entry.pr_id}",
            ]
        )
        self.gh(
            [
                "api",
                "graphql",
                "-f",
                f"query={ENQUEUE_MUTATION}",
                "-f",
                f"id={entry.pr_id}",
            ]
        )

    def cancel_run(self, run: WorkflowRun) -> str:
        """Cancel one workflow run. Returns "cancelled" or "already-terminal".

        The ONLY write the reap performs, and it is scoped to a RUN ID taken from a
        listing: a run id names one immutable dispatch, so a cancellation can never
        reach the replacement runs a rebuild creates (they carry new ids).

        A 409 is the idempotent case — the run reached a terminal state between the
        listing and this POST, or a duplicate doorbell delivery already cancelled it. It
        is reported, not raised: re-running the reap over the same state must converge,
        not fail.
        """
        try:
            self.gh(["api", "-X", "POST", f"repos/{self.repo}/actions/runs/{run.run_id}/cancel"])
        except GhError as error:
            if _HTTP_409_RE.search(str(error)):
                return "already-terminal"
            raise
        return "cancelled"

    # -- the sweep -----------------------------------------------------------

    def emit(self, row: str) -> None:
        self.log(f"[{PROGRAM}] {row}")
        summary = os.environ.get("GITHUB_STEP_SUMMARY")
        if summary:
            try:
                with open(summary, "a", encoding="utf-8") as handle:
                    handle.write(f"- `{row}`\n")
            except OSError:
                pass

    def sweep(self) -> int:
        now = self._now()
        entries = self.queue_entries()
        if not entries:
            self.emit(
                f"branch={self.branch} queue is EMPTY — 0 entries to watch "
                f"(grace={self.grace_seconds}s)"
            )
            return 0

        counts: dict[str, int] = {}
        errors = 0
        run_recoveries = 0
        ordered = sorted(entries, key=lambda e: e.position)
        for entry in ordered:
            # THE EFFECTIVENESS TERM. Every entry behind this one is chained ON TOP of
            # its group head, so a rebuild discards all of their CI too. The real cost
            # of a dead ref is therefore not the ref — it is how much work stacks above
            # it before anyone notices:
            #     cost ~= (groups stacked above the dead ref at detection) x (their CI)
            # Measured 2026-07-28: intervening at 48 min meant that term was 4 (two of
            # them already MERGEABLE, i.e. fully green, and all four were discarded on
            # the rebuild). Firing inside the grace should make it 0. Recording it on
            # every row makes the watchdog self-measuring: if `stacked` starts climbing
            # the grace has drifted too long, and the rows say so without anyone having
            # to notice an outage by hand.
            stacked = [
                other
                for other in ordered
                if other.position > entry.position and other.head_oid
            ]
            stacked_green = [o for o in stacked if o.state == "MERGEABLE"]
            ref = (
                queue_ref(self.branch, entry.pr_number, entry.base_oid)
                if entry.base_oid
                else "(no group)"
            )
            suites: int | None = None
            created_at: datetime | None = None
            markers: list[Marker] = []
            markers_readable = True
            recoveries_in_window = 0

            if entry.state in ACTIONABLE_STATES and entry.head_oid and entry.base_oid:
                suites = self.check_suite_count(entry.head_oid)
                if suites == 0:
                    created_at = self.group_created_at(ref, entry.head_oid)
                    try:
                        markers = self.pr_markers(entry.pr_number)
                    except (GhError, json.JSONDecodeError) as error:
                        # An unreadable history is NOT "no prior recovery" — assuming
                        # that would bypass the cap on every API blip. It falls through
                        # to the ONE decide/emit path below as a REFUSE, so there is a
                        # single row shape for an operator to parse.
                        markers_readable = False
                        errors += 1
                        self.log(
                            f"::warning title={PROGRAM} marker history unreadable::"
                            f"PR #{entry.pr_number}: {error}"
                        )
                    else:
                        cutoff = now - timedelta(seconds=self.recovery_window_seconds)
                        recoveries_in_window = sum(
                            1 for marker in markers if marker.observed >= cutoff
                        )

            decision = decide_entry(
                entry,
                suites=suites,
                created_at=created_at,
                now=now,
                grace_seconds=self.grace_seconds,
                markers=markers,
                markers_readable=markers_readable,
                recoveries_in_window=recoveries_in_window,
                max_recoveries_per_pr=self.max_recoveries_per_pr,
                run_recoveries=run_recoveries,
                max_recoveries_per_run=self.max_recoveries_per_run,
            )

            suite_text = "unknown" if suites is None else str(suites)
            row = (
                f"pos={entry.position} pr=#{entry.pr_number} state={entry.state} "
                f"ref={ref} head={(entry.head_oid or '-')[:8]} suites={suite_text} "
                f"stacked={len(stacked)} stacked_green={len(stacked_green)} "
                f"decision={decision.verdict} — {decision.detail}"
            )

            if decision.verdict == RECOVER and not self.dry_run:
                try:
                    self.post_marker_comment(entry, ref, now)
                    # PROVE THE EVIDENCE CHANNEL BEFORE RELYING ON IT. The caps and the
                    # idempotence key are all read back through TRUSTED_MARKER_AUTHORS.
                    # If this runner's identity is not on that list, every marker we
                    # write is invisible to us: the per-head NOOP never fires, the
                    # per-PR cap never binds, and the watchdog degrades into a
                    # dequeue/enqueue thrash loop bounded only by the per-run budget.
                    # So read the marker back and refuse to act if it is not there.
                    if not any(
                        marker.head == entry.head_oid
                        for marker in self.pr_markers(entry.pr_number)
                    ):
                        raise GhError(
                            "the marker just posted is not readable back as TRUSTED — "
                            "refusing to dequeue. TRUSTED_MARKER_AUTHORS is probably "
                            "missing this runner's identity, and acting without a "
                            "readable marker would uncap the recovery loop"
                        )
                    self.recover(entry)
                except GhError as error:
                    self.emit(f"{row} :: RECOVERY FAILED ({error})")
                    self.log(
                        f"::error title={PROGRAM} recovery failed::PR #{entry.pr_number} "
                        f"ref {ref}: {error}"
                    )
                    counts["RECOVER_FAILED"] = counts.get("RECOVER_FAILED", 0) + 1
                    errors += 1
                    continue
                run_recoveries += 1

            self.emit(row + (" [dry-run]" if self.dry_run and decision.verdict == RECOVER else ""))
            if decision.verdict in (CAP, REFUSE, NOOP):
                self.log(
                    f"::warning title={PROGRAM} {decision.verdict}::"
                    f"PR #{entry.pr_number} ref {ref}: {decision.detail}"
                )
            counts[decision.verdict] = counts.get(decision.verdict, 0) + 1

        census = " ".join(f"{key}={value}" for key, value in sorted(counts.items()))
        self.emit(f"sweep complete: entries={len(entries)} {census} errors={errors}")
        return errors

    # -- the stale-run reap (#6068) ------------------------------------------

    def reap(self) -> int:
        """Cancel the unfinished `merge_group` runs of superseded queue groups.

        Returns the error count, so the caller fails the run on anything it could not
        establish. IDEMPOTENT: a second pass over the same state finds the cancelled runs
        terminal (or already absent from the unfinished listing) and cancels nothing.

        A cancellation needs THREE positive readings, not one: the group is absent from
        the queue snapshot, still absent from a fresh queue read taken immediately before
        the POST, and the group ref itself agrees it is gone or has been rebuilt. Any one
        of them failing to read keeps the run.
        """
        snapshot_at = self._now()
        try:
            current = self.current_group_keys()
        except (GhError, ValueError, json.JSONDecodeError) as error:
            # FAIL LOUD, KEEP EVERYTHING. This is the one read the whole decision rests
            # on; without it every run is unknown, and unknown is never obsolete.
            self.emit(
                f"reap branch={self.branch}: the merge-queue state could NOT be read "
                f"({error}) — 0 runs cancelled, every unfinished run KEPT"
            )
            self.log(
                f"::error title={PROGRAM} reap aborted::the merge-queue state could not "
                f"be read ({error}); no run can be called obsolete without it"
            )
            return 1

        # Name the snapshot in the log. The reap's whole verdict is "absent from THIS
        # set", so a row that does not say what the set was is unauditable after the fact.
        labels = sorted(ref.rsplit("/", 1)[-1] + "@" + sha[:8] for ref, sha in current)
        self.emit(
            f"reap branch={self.branch} snapshot={iso(snapshot_at)} "
            f"current_groups={len(current)} [{', '.join(labels)}]"
        )

        errors = 0
        # DUPLICATE DELIVERY / DOUBLE LISTING: keyed by run id, so a run that appears
        # under both status queries (or a doorbell that rings twice) is judged ONCE.
        candidates: dict[int, WorkflowRun] = {}
        for status in REAP_RUN_STATUSES:
            page = self.unfinished_merge_group_runs(status)
            if page is None:
                # A missing page only means fewer CANDIDATES — it can never make a run
                # look obsolete — so the pass continues on what it did read. It is still
                # an error, because a reap that silently saw half the estate is a reap
                # that silently did half its job.
                errors += 1
                self.log(
                    f"::error title={PROGRAM} run listing unreadable::status={status}: "
                    "no run in that state is judged this pass"
                )
                continue
            for run in page:
                candidates.setdefault(run.run_id, run)

        if not candidates:
            self.emit(
                f"reap complete: 0 unfinished {MERGE_GROUP_EVENT} runs readable — "
                f"nothing to reap errors={errors}"
            )
            return errors

        decisions: list[tuple[WorkflowRun, Decision]] = []
        for run in sorted(candidates.values(), key=lambda r: r.run_id):
            decisions.append(
                (
                    run,
                    decide_run(
                        run,
                        current_groups=current,
                        branch=self.branch,
                        snapshot_at=snapshot_at,
                        grace_seconds=self.reap_grace_seconds,
                    ),
                )
            )

        # ── TOCTOU REVALIDATION ────────────────────────────────────────────────
        # The queue can rebuild between the snapshot and the POST. One FRESH authoritative
        # read is taken immediately before the first cancellation, and a run is cancelled
        # only if its group is absent from BOTH reads — a group present in either one is
        # not positively obsolete. If the revalidation read fails, NOTHING is cancelled
        # and the pass fails loud; a cancellation is not the kind of action that may
        # proceed on a stale premise.
        #
        # Performed in dry-run too, so `--dry-run` exercises the real decision path
        # instead of a shortened one.
        confirmed: list[tuple[WorkflowRun, Decision]] = []
        wanted = [(run, d) for run, d in decisions if d.verdict == CANCEL]
        revalidated: frozenset[tuple[str, str]] | None = None
        if wanted:
            try:
                revalidated = self.current_group_keys()
            except (GhError, ValueError, json.JSONDecodeError) as error:
                errors += 1
                self.log(
                    f"::error title={PROGRAM} revalidation failed::the merge-queue state "
                    f"could not be re-read immediately before cancelling ({error}); "
                    f"{len(wanted)} run(s) KEPT"
                )
                revalidated = None

        counts: dict[str, int] = {}
        cancelled = 0
        for run, decision in decisions:
            if decision.verdict == CANCEL:
                if revalidated is None:
                    decision = Decision(
                        REFUSE,
                        "the merge-queue state could not be re-read immediately before "
                        "cancelling — refusing to act on a stale snapshot",
                    )
                elif group_key(run.head_branch, run.head_sha) in revalidated:
                    decision = Decision(
                        KEEP,
                        f"{run.head_branch}@{run.head_sha[:8]} REAPPEARED in the "
                        "merge queue between the snapshot and the cancellation",
                    )
                elif cancelled >= self.max_cancellations:
                    decision = Decision(
                        CAP,
                        f"per-pass cancellation budget exhausted "
                        f"({cancelled}/{self.max_cancellations}) — the next pass "
                        "re-evaluates this run",
                    )
                else:
                    # ── SECOND AUTHORITATIVE SOURCE, read immediately before the POST.
                    # The queue snapshot says this group is gone; the ref itself is asked
                    # to agree. See the REF_GONE/REF_AT note in the tunables for why the
                    # asymmetry of the failure directions makes this load-bearing rather
                    # than decorative.
                    state = self.group_ref_state(run.head_branch)
                    if state is None:
                        decision = Decision(
                            REFUSE,
                            f"the group ref {run.head_branch} could not be read — "
                            "obsolescence is not corroborated, so the run is kept",
                        )
                    elif state[0] == REF_AT and state[1] == run.head_sha:
                        decision = Decision(
                            KEEP,
                            f"the group ref {run.head_branch} still points AT "
                            f"{run.head_sha[:8]} — it is live under this exact commit, "
                            "whatever the queue snapshot said",
                        )
                    else:
                        detail = (
                            "the ref no longer exists"
                            if state[0] == REF_GONE
                            else f"the ref now points at {state[1][:8]}"
                        )
                        decision = Decision(CANCEL, f"{decision.detail}; {detail}")
                        confirmed.append((run, decision))
                        cancelled += 1

            row = (
                f"run={run.run_id} workflow={run.name!r} event={run.event} "
                f"status={run.status} ref={run.head_branch or '-'} "
                f"head={(run.head_sha or '-')[:8]} decision={decision.verdict} "
                f"— {decision.detail}"
            )
            if decision.verdict == CANCEL and self.dry_run:
                row += " [dry-run]"
            self.emit(row)
            if decision.verdict in (REFUSE, CAP):
                self.log(
                    f"::warning title={PROGRAM} reap {decision.verdict}::"
                    f"run {run.run_id}: {decision.detail}"
                )
            counts[decision.verdict] = counts.get(decision.verdict, 0) + 1

        for run, _decision in confirmed:
            if self.dry_run:
                continue
            try:
                outcome = self.cancel_run(run)
            except GhError as error:
                errors += 1
                counts["CANCEL_FAILED"] = counts.get("CANCEL_FAILED", 0) + 1
                self.emit(f"run={run.run_id} :: CANCELLATION FAILED ({error})")
                self.log(
                    f"::error title={PROGRAM} cancellation failed::run {run.run_id} "
                    f"({run.head_branch}): {error}"
                )
                continue
            self.emit(f"run={run.run_id} :: {outcome}")
            counts[outcome] = counts.get(outcome, 0) + 1

        census = " ".join(f"{key}={value}" for key, value in sorted(counts.items()))
        self.emit(
            f"reap complete: runs={len(decisions)} {census} errors={errors}"
            + (" [dry-run]" if self.dry_run else "")
        )
        return errors

    # -- the dequeue routing split -------------------------------------------

    def classify_dequeue(self, pr_number: int, reason: str) -> Route:
        now = self._now()
        try:
            markers = self.pr_markers(pr_number)
        except (GhError, json.JSONDecodeError) as error:
            return Route(
                ROUTE_DEMOTE, False, f"marker history unreadable ({error})"
            )
        return classify_dequeue_route(
            reason=reason,
            pr_number=pr_number,
            markers=markers,
            verdict=self.verdict_state(pr_number),
            live_suites=self.check_suite_count,
            now=now,
        )


def write_outputs(route: Route, log: Callable[[str], None] = print) -> None:
    """Emit the routing verdict as a row AND as GitHub step outputs."""
    log(
        f"[{PROGRAM}] dequeue route={route.route} reenqueue="
        f"{'true' if route.reenqueue else 'false'} — {route.detail}"
    )
    path = os.environ.get("GITHUB_OUTPUT")
    if not path:
        return
    with open(path, "a", encoding="utf-8") as handle:
        handle.write(f"route={route.route}\n")
        handle.write(f"reenqueue={'true' if route.reenqueue else 'false'}\n")
        # `detail` can embed gh stderr, which is multi-line. A bare `k=v` line would
        # spill the remainder into the outputs file as further keys, so use the
        # heredoc form with a delimiter that cannot survive in the payload.
        delimiter = "MGW_DETAIL_EOF"
        detail = route.detail.replace(delimiter, "MGW-DETAIL-EOF")
        handle.write(f"detail<<{delimiter}\n{detail}\n{delimiter}\n")


# ── self-test ────────────────────────────────────────────────────────────────────


def _entry(
    *,
    pr: int = 4534,
    position: int = 1,
    state: str = "AWAITING_CHECKS",
    base: str | None = "3cc1bf828c1335577069f5fa65d832c0ae1c8c38",
    head: str | None = "1bfb0174f5cc2da1ed9dfe7997b7ab089e7cab26",
) -> QueueEntry:
    return QueueEntry(
        pr_number=pr,
        pr_id=f"PR_{pr}",
        entry_id=f"MQE_{pr}",
        position=position,
        state=state,
        enqueued_at=parse_iso("2026-07-27T22:58:53Z"),
        base_oid=base,
        head_oid=head,
    )


def self_test() -> None:
    now = parse_iso("2026-07-27T23:20:00Z")
    assert now is not None
    built = parse_iso("2026-07-27T23:04:37Z")  # the real #4534 branch_creation time

    def call(**overrides):
        kwargs = dict(
            suites=0,
            created_at=built,
            now=now,
            grace_seconds=DEFAULT_GRACE_SECONDS,
            markers=(),
            recoveries_in_window=0,
            max_recoveries_per_pr=DEFAULT_MAX_RECOVERIES_PER_PR,
            run_recoveries=0,
            max_recoveries_per_run=DEFAULT_MAX_RECOVERIES_PER_RUN,
        )
        entry = overrides.pop("entry", _entry())
        kwargs.update(overrides)
        return decide_entry(entry, **kwargs)

    # The incident: zero suites, well past grace => RECOVER.
    assert call().verdict == RECOVER, call()
    # THE CONTROL: suites present but runs still pending => never touched.
    assert call(suites=8).verdict == HOLD, call(suites=8)
    # Fail safe: an unknown suite count is never a recovery.
    assert call(suites=None).verdict == REFUSE, call(suites=None)
    # Fail safe: no branch_creation row => no anchor => no action.
    assert call(created_at=None).verdict == REFUSE, call(created_at=None)
    # Grace gate, expressed against the constant so a retune cannot leave it stale.
    grace = DEFAULT_GRACE_SECONDS
    assert call(now=built + timedelta(seconds=grace - 1)).verdict == WAIT
    assert call(now=built + timedelta(seconds=grace)).verdict == RECOVER
    # ...and pinned against the MEASURED dispatch latency (N=209, max 4 s).
    assert grace >= 20 * 4 and grace <= 300, grace
    # Idempotence: a repeat detection on the SAME head is a no-op.
    same = Marker(
        pr=4534,
        head="1bfb0174f5cc2da1ed9dfe7997b7ab089e7cab26",
        base="",
        ref="",
        suites=0,
        observed=now - timedelta(minutes=5),
        action="re-enqueue",
    )
    assert call(markers=(same,)).verdict == NOOP, call(markers=(same,))
    # A marker for a DIFFERENT head does not suppress a fresh detection.
    other = Marker(**{**same.__dict__, "head": "a" * 40})
    assert call(markers=(other,), recoveries_in_window=1).verdict == RECOVER
    # Caps.
    assert call(recoveries_in_window=2).verdict == CAP
    assert call(run_recoveries=2).verdict == CAP
    # Non-actionable states.
    for state in ("QUEUED", "MERGEABLE", "UNMERGEABLE", "LOCKED"):
        assert call(entry=_entry(state=state)).verdict == SKIP, state
    assert call(entry=_entry(head=None)).verdict == SKIP

    # Ref reconstruction, pinned against live-verified data.
    assert (
        queue_ref("main", 4644, "c96f5abe3e0b7ba50c1381a1503ba961313b3da7")
        == "gh-readonly-queue/main/pr-4644-c96f5abe3e0b7ba50c1381a1503ba961313b3da7"
    )

    # Marker round-trip.
    rendered = render_marker(
        pr=4534,
        head="1bfb0174f5cc2da1ed9dfe7997b7ab089e7cab26",
        base="3cc1bf828c1335577069f5fa65d832c0ae1c8c38",
        ref="gh-readonly-queue/main/pr-4534-3cc1bf828c1335577069f5fa65d832c0ae1c8c38",
        suites=0,
        observed=now,
    )
    parsed = parse_marker("body text\n\n" + rendered)
    assert parsed is not None and parsed.head.startswith("1bfb0174"), rendered
    assert parsed.suites == 0 and parsed.action == "re-enqueue"
    assert parse_marker("no marker here") is None
    assert parse_marker(f"<!-- {MARKER_KEY} pr=1 head=nothex suites=0 observed=x -->") is None

    # Routing split — both arms.
    def route(**overrides):
        kwargs = dict(
            reason=ZERO_DISPATCH_REASON,
            pr_number=4534,
            markers=(parsed,),
            verdict=VerdictState(
                review_pass_at=now - timedelta(hours=2),
                head_moved_at=now - timedelta(hours=3),
                last_enqueued_at=now - timedelta(hours=1),
            ),
            live_suites=lambda _sha: 0,
            now=now + timedelta(minutes=40),
        )
        kwargs.update(overrides)
        return classify_dequeue_route(**kwargs)

    assert route().route == ROUTE_PRESERVE, route()
    assert route().reenqueue is True
    # The other arm: a CI_TIMEOUT that followed REAL checks still demotes.
    assert route(live_suites=lambda _s: 8).route == ROUTE_DEMOTE, route(live_suites=lambda _s: 8)
    # Unreadable count => demote (fail safe).
    assert route(live_suites=lambda _s: None).route == ROUTE_DEMOTE
    # No marker => demote.
    assert route(markers=()).route == ROUTE_DEMOTE
    # A marker predating the current attempt cannot steer the route.
    assert route(verdict=VerdictState(now - timedelta(hours=2), None, now + timedelta(minutes=1))).route == ROUTE_DEMOTE
    # No added_to_merge_queue event => demote.
    assert route(verdict=VerdictState(now - timedelta(hours=2), None, None)).route == ROUTE_DEMOTE
    # A head that moved AFTER the verdict was granted => the verdict may not survive.
    assert route(verdict=VerdictState(now - timedelta(hours=2), now - timedelta(minutes=1), now - timedelta(hours=1))).route == ROUTE_DEMOTE
    # No review:pass at all => nothing to preserve.
    assert route(verdict=VerdictState(None, None, now - timedelta(hours=1))).route == ROUTE_DEMOTE
    # An unreadable timeline => demote.
    assert route(verdict=VerdictState(None, None, None, readable=False)).route == ROUTE_DEMOTE
    # A stale marker => demote.
    assert route(now=now + timedelta(hours=7)).route == ROUTE_DEMOTE
    # A marker for a DIFFERENT PR is not evidence about this one.
    assert route(pr_number=9999).route == ROUTE_DEMOTE, route(pr_number=9999)
    # Reasons that ARE a statement about the diff keep today's behaviour.
    for other_reason in ("CI_FAILURE", "MERGE_CONFLICT"):
        assert route(reason=other_reason).route == ROUTE_DEMOTE, other_reason

    # sq-fnr85 arm (c): a QUEUE-LEVEL drop preserves the verdict and re-arms.
    def queue_level(**overrides):
        attempts = overrides.pop("attempts", 1)
        granted = now - timedelta(hours=2)
        kwargs = dict(
            reason="ROLL_BACK",
            markers=(),  # this population never has a watchdog observation
            verdict=VerdictState(
                review_pass_at=granted,
                head_moved_at=now - timedelta(hours=3),
                last_enqueued_at=now - timedelta(hours=1),
                enqueued_at=tuple(
                    granted + timedelta(minutes=n + 1) for n in range(attempts)
                ),
            ),
        )
        kwargs.update(overrides)
        return route(**kwargs)

    for drop_reason in ("QUEUE_CLEARED", "ROLL_BACK"):
        dropped = queue_level(reason=drop_reason)
        assert dropped.route == ROUTE_PRESERVE, (drop_reason, dropped)
        assert dropped.reenqueue is True, (drop_reason, dropped)
    # The budget is real: one verdict may not ride the queue forever.
    assert queue_level(attempts=DEFAULT_MAX_PRESERVED_REQUEUES).route == ROUTE_DEMOTE
    assert queue_level(attempts=DEFAULT_MAX_PRESERVED_REQUEUES - 1).route == ROUTE_PRESERVE
    # An uncountable budget is not a spent one — it is a refusal to preserve.
    assert queue_level(attempts=0).route == ROUTE_DEMOTE
    # The tree precondition binds this arm too, exactly as it binds the other two.
    moved = VerdictState(
        review_pass_at=now - timedelta(hours=2),
        head_moved_at=now - timedelta(minutes=1),
        last_enqueued_at=now - timedelta(hours=1),
        enqueued_at=(now - timedelta(hours=1),),
    )
    assert queue_level(verdict=moved).route == ROUTE_DEMOTE
    # ...including when the commit's SELF-REPORTED date predates the grant and only the
    # timeline ORDER gives the push away.
    backdated = VerdictState(
        review_pass_at=now - timedelta(hours=2),
        head_moved_at=now - timedelta(hours=3),
        last_enqueued_at=now - timedelta(hours=1),
        enqueued_at=(now - timedelta(hours=1),),
        head_moved_after_grant_in_order=True,
    )
    assert queue_level(verdict=backdated).route == ROUTE_DEMOTE
    # ...and a MANUAL dequeue with no watchdog marker is still a human withdrawal.
    assert queue_level(reason="MANUAL").route == ROUTE_DEMOTE
    # The watchdog's OWN dequeue preserves without a second re-enqueue.
    own = route(reason="MANUAL", now=now + timedelta(seconds=30))
    assert own.route == ROUTE_PRESERVE and own.reenqueue is False, own
    # ...but only inside the self-dequeue window.
    assert route(reason="MANUAL", now=now + timedelta(hours=2)).route == ROUTE_DEMOTE
    # A conflict dequeue right after a recovery is NOT attributed to the watchdog.
    assert route(reason="MERGE_CONFLICT", now=now + timedelta(seconds=30)).route == ROUTE_DEMOTE

    # ── the stale-run reap (#6068) ────────────────────────────────────────────────
    stale_ref = "gh-readonly-queue/main/pr-6049-" + "b" * 40
    live_ref = "gh-readonly-queue/main/pr-6049-" + "c" * 40
    stale_head = "d" * 40
    live_head = "e" * 40
    built = now - timedelta(minutes=30)

    def a_run(**overrides) -> WorkflowRun:
        fields = dict(
            run_id=33447340157,
            name="ci-summary",
            event=MERGE_GROUP_EVENT,
            status="in_progress",
            head_branch=stale_ref,
            head_sha=stale_head,
            created_at=built,
        )
        fields.update(overrides)
        return WorkflowRun(**fields)

    def reap_call(**overrides) -> Decision:
        kwargs = dict(
            current_groups=frozenset({group_key(live_ref, live_head)}),
            branch="main",
            snapshot_at=now,
            grace_seconds=DEFAULT_REAP_GRACE_SECONDS,
        )
        subject = overrides.pop("run", a_run())
        kwargs.update(overrides)
        return decide_run(subject, **kwargs)

    # THE INCIDENT: a rebuild left this run on a group the queue no longer contains.
    assert reap_call().verdict == CANCEL, reap_call()
    # THE CONTROL: the run of a CURRENT group is never touched, at any age.
    current = a_run(head_branch=live_ref, head_sha=live_head)
    assert reap_call(run=current).verdict == KEEP, reap_call(run=current)
    # A DEQUEUE that empties the queue makes every built group obsolete...
    assert reap_call(current_groups=frozenset()).verdict == CANCEL
    # ...but an UNREADABLE queue is not an empty one.
    assert reap_call(current_groups=None).verdict == REFUSE
    # Never a PR-head run, never a push run — whatever ref they claim.
    for event in ("pull_request", "pull_request_target", "push", "schedule", ""):
        assert reap_call(run=a_run(event=event)).verdict == SKIP, event
    # Never another branch's queue, and never a plain branch.
    for ref in ("gh-readonly-queue/release/pr-1-" + "f" * 40, "main", "feature/x", ""):
        assert reap_call(run=a_run(head_branch=ref)).verdict == SKIP, ref
    # A run that is already terminal holds no capacity.
    assert reap_call(run=a_run(status="completed")).verdict == SKIP
    # Registration grace, expressed against the constant so a retune cannot leave it stale.
    reap_grace = DEFAULT_REAP_GRACE_SECONDS
    assert reap_call(snapshot_at=built + timedelta(seconds=reap_grace - 1)).verdict == WAIT
    assert reap_call(snapshot_at=built + timedelta(seconds=reap_grace)).verdict == CANCEL
    # A run registered AFTER the snapshot is held, never cancelled.
    assert reap_call(snapshot_at=built - timedelta(seconds=1)).verdict == WAIT
    # Unestablishable identity is always a refusal.
    assert reap_call(run=a_run(head_sha="not-a-sha")).verdict == REFUSE
    assert reap_call(run=a_run(created_at=None)).verdict == REFUSE
    assert reap_call(run=a_run(run_id=0)).verdict == REFUSE
    # The ref NAME alone is not the key: the queue rebuilds a group on the same base, so
    # the superseded head's runs must still reap while the name is current.
    same_name = a_run(head_branch=live_ref, head_sha=stale_head)
    assert reap_call(run=same_name).verdict == CANCEL, reap_call(run=same_name)

    print(f"{PROGRAM} self-test: PASS")


# ── entrypoint ───────────────────────────────────────────────────────────────────


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--self-test", action="store_true")
    sub = parser.add_subparsers(dest="command")

    sweep_parser = sub.add_parser("sweep", help="scan the merge queue for zero-dispatch")
    sweep_parser.add_argument("--repo", required=True)
    sweep_parser.add_argument("--branch", default="main")
    sweep_parser.add_argument("--grace-seconds", type=int, default=DEFAULT_GRACE_SECONDS)
    sweep_parser.add_argument(
        "--max-recoveries-per-pr", type=int, default=DEFAULT_MAX_RECOVERIES_PER_PR
    )
    sweep_parser.add_argument(
        "--recovery-window-seconds", type=int, default=DEFAULT_RECOVERY_WINDOW_SECONDS
    )
    sweep_parser.add_argument(
        "--max-recoveries-per-run", type=int, default=DEFAULT_MAX_RECOVERIES_PER_RUN
    )
    sweep_parser.add_argument("--dry-run", action="store_true")

    reap_parser = sub.add_parser(
        "reap", help="cancel unfinished merge_group runs of superseded queue groups"
    )
    reap_parser.add_argument("--repo", required=True)
    reap_parser.add_argument("--branch", default="main")
    reap_parser.add_argument(
        "--grace-seconds", type=int, default=DEFAULT_REAP_GRACE_SECONDS
    )
    reap_parser.add_argument(
        "--max-cancellations", type=int, default=DEFAULT_MAX_CANCELLATIONS_PER_PASS
    )
    reap_parser.add_argument("--dry-run", action="store_true")

    classify_parser = sub.add_parser(
        "classify-dequeue", help="route a pull_request.dequeued event"
    )
    classify_parser.add_argument("--repo", required=True)
    classify_parser.add_argument("--pr", type=int, required=True)
    classify_parser.add_argument("--reason", default="UNKNOWN")

    args = parser.parse_args(argv)

    if args.self_test:
        self_test()
        return 0

    if args.command == "classify-dequeue":
        # FAIL SAFE: any failure at all routes to `demote`, which is the pre-existing
        # behaviour. A classifier crash must never silently preserve a verdict.
        try:
            watchdog = Watchdog(args.repo, gh_read=run_gh_read)
            route = watchdog.classify_dequeue(args.pr, args.reason)
        except (GhError, ValueError, json.JSONDecodeError, gh_retry.GhTransientExhausted) as error:
            route = Route(ROUTE_DEMOTE, False, f"classification failed ({error})")
        write_outputs(route)
        return 0

    if args.command == "reap":
        try:
            watchdog = Watchdog(
                args.repo,
                branch=args.branch,
                reap_grace_seconds=args.grace_seconds,
                max_cancellations=args.max_cancellations,
                dry_run=args.dry_run,
                gh_read=run_gh_read,
            )
            return 1 if watchdog.reap() else 0
        except gh_retry.GhTransientExhausted as error:
            # Periodic + idempotent, exactly like `sweep`: a missed pass costs one tick
            # of stale capacity, never a wrong cancellation, so a transient API outage
            # skips rather than fails.
            print(
                f"::warning title={PROGRAM} reap skipped a cycle on transient GitHub "
                f"API failures::{error}"
            )
            return 0
        except (GhError, ValueError, json.JSONDecodeError) as error:
            print(f"[{PROGRAM}] fatal: {error}", file=sys.stderr)
            return 1

    if args.command == "sweep":
        try:
            watchdog = Watchdog(
                args.repo,
                branch=args.branch,
                grace_seconds=args.grace_seconds,
                max_recoveries_per_pr=args.max_recoveries_per_pr,
                recovery_window_seconds=args.recovery_window_seconds,
                max_recoveries_per_run=args.max_recoveries_per_run,
                dry_run=args.dry_run,
                gh_read=run_gh_read,
            )
            return 1 if watchdog.sweep() else 0
        except gh_retry.GhTransientExhausted as error:
            # Periodic + idempotent: a missed cycle is harmless, the next tick covers
            # it. Still emitted loudly so the miss is never invisible.
            print(
                f"::warning title={PROGRAM} skipped a cycle on transient GitHub API "
                f"failures::{error}"
            )
            return 0
        except (GhError, ValueError, json.JSONDecodeError) as error:
            print(f"[{PROGRAM}] fatal: {error}", file=sys.stderr)
            return 1

    parser.error("a subcommand is required unless --self-test is used")
    return 2


if __name__ == "__main__":
    sys.exit(main())
