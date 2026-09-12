#!/usr/bin/env python3
# [OPUS-5] sparq-org/sparq#4652 — the zero-dispatch merge-group watchdog test suite.
#
# Three layers, because on THIS repo every uncaught mutant in a recent sweep lived at
# the YAML seam rather than in the Python:
#
#   1. POLICY (pure) — decide_entry / classify_dequeue_route decision tables. Every
#      arm, and for the two headline claims a paired CONTROL that must NOT fire.
#   2. BEHAVIOUR (fake gh) — drive the real Watchdog.sweep() against a scripted gh and
#      assert on the MUTATIONS ISSUED, not just the log text. A watchdog that logs
#      "RECOVER" while issuing no mutation, or that mutates on the control input, dies
#      here.
#   3. WIRING (YAML inspection) — the `if:` expressions, the step list, and the CALL
#      SITE argv of both workflows. YAML `if:` cannot be executed in a unit test, so
#      its SHAPE is pinned instead: delete the classify step, invert the fail-safe
#      `!= 'preserve'` to `== 'demote'`, drop `sweep` from the call site, or add a
#      pull_request trigger to the cron watchdog, and a test here goes red.
#
# Hermetic: no gh, no network, no git. Needs PyYAML (already a docs-quality dep).
# Run:  python3 scripts/tests/test_merge_group_watchdog.py

from __future__ import annotations

import importlib.util
import json
import re
import sys
import unittest
from datetime import datetime, timedelta, timezone
from pathlib import Path

import yaml

REPO_ROOT = Path(__file__).resolve().parent.parent.parent
SCRIPTS = REPO_ROOT / "scripts"
FEEDBACK_YML = REPO_ROOT / ".github" / "workflows" / "merge-queue-feedback.yml"
WATCHDOG_YML = REPO_ROOT / ".github" / "workflows" / "merge-group-watchdog.yml"
DOORBELL_YML = REPO_ROOT / ".github" / "workflows" / "merge-group-doorbell.yml"
DOCS_QUALITY_YML = REPO_ROOT / ".github" / "workflows" / "docs-quality.yml"


def _load(module_name: str, filename: str):
    sys.path.insert(0, str(SCRIPTS))
    spec = importlib.util.spec_from_file_location(module_name, SCRIPTS / filename)
    assert spec and spec.loader
    module = importlib.util.module_from_spec(spec)
    sys.modules[module_name] = module
    spec.loader.exec_module(module)
    return module


mgw = _load("merge_group_watchdog", "merge-group-watchdog.py")


# ── THE SUITE MUST NEVER MAKE A REAL `gh` CALL ───────────────────────────────────
# This is not belt-and-braces; it is a repair. `Watchdog.__init__` used to default
# `gh=run_gh` as a DEFAULT ARGUMENT, which binds at definition time — so patching
# `mgw.run_gh` from a test did NOT reach it, and `main(["sweep", ...])` used the REAL
# runner while its reads were faked. The result: this suite posted **567 real comments
# to production PR #4534** during a mutation sweep (175 mutants x a few invocations).
#
# Poisoning both module-level runners at import makes that class of mistake impossible
# to repeat SILENTLY: any code path that forgets to inject a fake now raises here
# instead of reaching the network. A test that needs a runner must inject one.
class RealGhCallAttempted(AssertionError):
    """A test tried to invoke the real `gh`. Inject a fake instead."""


def _forbidden_gh(argv):
    raise RealGhCallAttempted(
        "the test suite attempted a REAL gh call: gh "
        + " ".join(str(a) for a in argv[:4])
        + " — inject a fake runner (gh=/gh_read=) instead. See #4652."
    )


# The genuine functions, kept so the two tests that exercise run_gh_read's own
# exception translation can call it directly rather than the poison.
_REAL_RUN_GH_READ = mgw.run_gh_read

mgw.run_gh = _forbidden_gh
mgw.run_gh_read = _forbidden_gh


class _NoNetwork:
    """Swap in fake runners and ALWAYS restore BOTH.

    A finally that restored only `run_gh_read` leaked a stale fake into every later
    test — which is how the poison tests started failing and how a real runner could
    be reinstated unnoticed.
    """

    def __init__(self, gh):
        self.gh = gh

    def __enter__(self):
        self._saved = (mgw.run_gh, mgw.run_gh_read)
        mgw.run_gh = mgw.run_gh_read = self.gh
        return self.gh

    def __exit__(self, *exc):
        mgw.run_gh, mgw.run_gh_read = self._saved
        return False

# The real incident's identifiers (#4652), so the fixtures are not invented shapes.
BASE = "3cc1bf828c1335577069f5fa65d832c0ae1c8c38"
HEAD = "1bfb0174f5cc2da1ed9dfe7997b7ab089e7cab26"
REF = f"gh-readonly-queue/main/pr-99900001-{BASE}"
BUILT = mgw.parse_iso("2026-07-27T23:04:37Z")  # measured branch_creation timestamp
NOW = mgw.parse_iso("2026-07-27T23:20:00Z")


def entry(**overrides) -> "mgw.QueueEntry":
    fields = dict(
        pr_number=99900001,
        pr_id="PR_99900001",
        entry_id="MQE_99900001",
        position=1,
        state="AWAITING_CHECKS",
        enqueued_at=mgw.parse_iso("2026-07-27T22:58:53Z"),
        base_oid=BASE,
        head_oid=HEAD,
    )
    fields.update(overrides)
    return mgw.QueueEntry(**fields)


def decide(**overrides) -> "mgw.Decision":
    kwargs = dict(
        suites=0,
        created_at=BUILT,
        now=NOW,
        grace_seconds=mgw.DEFAULT_GRACE_SECONDS,
        markers=(),
        recoveries_in_window=0,
        max_recoveries_per_pr=mgw.DEFAULT_MAX_RECOVERIES_PER_PR,
        run_recoveries=0,
        max_recoveries_per_run=mgw.DEFAULT_MAX_RECOVERIES_PER_RUN,
    )
    subject = overrides.pop("entry", entry())
    kwargs.update(overrides)
    return mgw.decide_entry(subject, **kwargs)


def marker(**overrides) -> "mgw.Marker":
    fields = dict(
        pr=99900001,
        head=HEAD,
        base=BASE,
        ref=REF,
        suites=0,
        observed=NOW - timedelta(minutes=5),
        action="re-enqueue",
    )
    fields.update(overrides)
    return mgw.Marker(**fields)


# ── 1. policy ────────────────────────────────────────────────────────────────────


class TestZeroSuiteDetection(unittest.TestCase):
    """The headline claim, and the control that stops it firing on everything."""

    def test_zero_suites_past_grace_is_recovered(self):
        # THE RED TEST: the exact #4534 shape — group ref built, zero check-suites,
        # well past the grace period — must be detected and recovered.
        self.assertEqual(decide().verdict, mgw.RECOVER)

    def test_control_suites_present_runs_pending_is_never_touched(self):
        # THE CONTROL: a perfectly healthy group whose check-runs are simply still
        # pending has SUITES. Without this the detector could pass by firing on
        # everything. Age is irrelevant here — even an hour-old pending group holds.
        for suites in (1, 8, 40):
            for age in (0, mgw.DEFAULT_GRACE_SECONDS * 100):
                verdict = decide(suites=suites, now=BUILT + timedelta(seconds=age)).verdict
                self.assertEqual(verdict, mgw.HOLD, (suites, age))

    def test_zero_check_runs_is_not_the_predicate(self):
        # A paths-filtered workflow that matches nothing still creates a check-SUITE
        # and produces no run. The detector must key on the suite count only; a
        # suites=N/runs=0 group is a HOLD.
        self.assertEqual(decide(suites=3).verdict, mgw.HOLD)

    def test_grace_boundary(self):
        self.assertEqual(decide(now=BUILT + timedelta(seconds=0)).verdict, mgw.WAIT)
        self.assertEqual(
            decide(now=BUILT + timedelta(seconds=mgw.DEFAULT_GRACE_SECONDS - 1)).verdict,
            mgw.WAIT,
        )
        self.assertEqual(
            decide(now=BUILT + timedelta(seconds=mgw.DEFAULT_GRACE_SECONDS)).verdict,
            mgw.RECOVER,
        )

    def test_grace_is_bounded_by_the_measured_dispatch_latency(self):
        # MEASURED N=209 (2026-07-25 -> 07-28): create->first-suite latency
        # min 1 s / p50 2 s / p99 4 s / MAX 4 s, with no tail between 4 s and 3600 s.
        # Floor: >=20x the measured maximum, so nobody tunes the grace into the noise
        # band. Ceiling: <= the 300 s cron interval, so the grace never costs an extra
        # tick (a longer grace would silently halve the recovery speed).
        measured_max_seconds = 4
        self.assertGreaterEqual(mgw.DEFAULT_GRACE_SECONDS, 20 * measured_max_seconds)
        self.assertLessEqual(mgw.DEFAULT_GRACE_SECONDS, 300)

    def test_only_awaiting_checks_is_actionable(self):
        self.assertEqual(mgw.ACTIONABLE_STATES, frozenset({"AWAITING_CHECKS"}))
        for state in ("QUEUED", "MERGEABLE", "UNMERGEABLE", "LOCKED"):
            self.assertEqual(decide(entry=entry(state=state)).verdict, mgw.SKIP, state)

    def test_entry_without_a_built_group_is_skipped(self):
        self.assertEqual(decide(entry=entry(head_oid=None)).verdict, mgw.SKIP)
        self.assertEqual(decide(entry=entry(base_oid=None)).verdict, mgw.SKIP)


class TestFailSafe(unittest.TestCase):
    """Anything not POSITIVELY established must be a refusal, never a recovery."""

    def test_unknown_suite_count_refuses(self):
        self.assertEqual(decide(suites=None).verdict, mgw.REFUSE)

    def test_unreadable_marker_history_refuses(self):
        # "I could not read the history" is not "there is no history". Treating them as
        # the same would bypass the per-PR cap on every API blip.
        self.assertEqual(decide(markers_readable=False).verdict, mgw.REFUSE)
        # ...and it must outrank the caps, not be masked by them.
        self.assertEqual(
            decide(markers_readable=False, recoveries_in_window=99).verdict, mgw.REFUSE
        )

    def test_missing_branch_creation_anchor_refuses(self):
        self.assertEqual(decide(created_at=None).verdict, mgw.REFUSE)

    def test_future_creation_time_refuses(self):
        self.assertEqual(decide(now=BUILT - timedelta(minutes=1)).verdict, mgw.REFUSE)

    def test_check_suite_count_returns_none_on_every_unreadable_shape(self):
        watchdog = FakeWatchdog.build()
        for payload in (
            "not json",
            "[]",
            json.dumps({"check_suites": []}),
            json.dumps({"total_count": 0}),
            json.dumps({"total_count": "0", "check_suites": []}),
            # total_count says zero but the array is non-empty: a garbled response must
            # never be mistaken for a positively-observed zero.
            json.dumps({"total_count": 0, "check_suites": [{"id": 1}]}),
            # `isinstance(False, int)` is True in Python, so a bool total_count read as
            # a positively-observed ZERO and drove a recovery off a malformed response.
            json.dumps({"total_count": False, "check_suites": []}),
            json.dumps({"total_count": True, "check_suites": []}),
        ):
            watchdog.gh.suites_raw = payload
            self.assertIsNone(watchdog.watchdog.check_suite_count(HEAD), payload)

    def test_positive_zero_is_returned(self):
        watchdog = FakeWatchdog.build()
        watchdog.gh.suites_raw = json.dumps({"total_count": 0, "check_suites": []})
        self.assertEqual(watchdog.watchdog.check_suite_count(HEAD), 0)

    def test_the_authoritative_counter_is_total_count_not_the_page_length(self):
        # `check_suites` is capped at per_page; `total_count` is the counter GitHub
        # itself maintains. Reading the array length would silently under-report any
        # group with more than one page of suites.
        watchdog = FakeWatchdog.build()
        watchdog.gh.suites_raw = json.dumps(
            {"total_count": 137, "check_suites": [{"id": i} for i in range(100)]}
        )
        self.assertEqual(watchdog.watchdog.check_suite_count(HEAD), 137)


class TestCapAndIdempotence(unittest.TestCase):
    def test_repeat_detection_on_the_same_ref_is_a_noop(self):
        self.assertEqual(decide(markers=(marker(),)).verdict, mgw.NOOP)

    def test_a_marker_for_a_different_head_does_not_suppress_detection(self):
        self.assertEqual(
            decide(markers=(marker(head="b" * 40),), recoveries_in_window=1).verdict,
            mgw.RECOVER,
        )

    def test_per_pr_cap(self):
        self.assertEqual(decide(recoveries_in_window=1).verdict, mgw.RECOVER)
        self.assertEqual(decide(recoveries_in_window=2).verdict, mgw.CAP)
        self.assertEqual(decide(recoveries_in_window=99).verdict, mgw.CAP)
        self.assertEqual(mgw.DEFAULT_MAX_RECOVERIES_PER_PR, 2)

    def test_per_run_cap(self):
        self.assertEqual(decide(run_recoveries=1).verdict, mgw.RECOVER)
        self.assertEqual(decide(run_recoveries=2).verdict, mgw.CAP)

    def test_cap_is_a_refusal_not_a_mutation(self):
        # Exhaustion hands back to the platform CI_TIMEOUT; it must not act.
        harness = FakeWatchdog.build(comments=[_marker_comment(head="c" * 40),
                                               _marker_comment(head="d" * 40)])
        harness.run()
        self.assertEqual(harness.gh.mutations, [])
        self.assertTrue(any("decision=CAP" in row for row in harness.rows), harness.rows)


class TestDequeueRouting(unittest.TestCase):
    """Both arms of the CI_TIMEOUT split, keyed on the suite count."""

    @staticmethod
    def verdict(**overrides) -> "mgw.VerdictState":
        fields = dict(
            review_pass_at=NOW - timedelta(hours=2),
            head_moved_at=NOW - timedelta(hours=3),
            last_enqueued_at=NOW - timedelta(hours=1),
        )
        fields.update(overrides)
        return mgw.VerdictState(**fields)

    def route(self, **overrides) -> "mgw.Route":
        kwargs = dict(
            reason="CI_TIMEOUT",
            pr_number=99900001,
            markers=(marker(),),
            verdict=self.verdict(),
            live_suites=lambda _sha: 0,
            now=NOW + timedelta(minutes=40),
        )
        kwargs.update(overrides)
        return mgw.classify_dequeue_route(**kwargs)

    def test_arm_a_ci_timeout_with_zero_suites_preserves_the_verdict(self):
        result = self.route()
        self.assertEqual(result.route, mgw.ROUTE_PRESERVE)
        self.assertTrue(result.reenqueue)

    def test_arm_b_ci_timeout_with_suites_present_still_demotes(self):
        # The legitimate signal: checks genuinely ran and ran long. Keep demoting.
        self.assertEqual(self.route(live_suites=lambda _s: 8).route, mgw.ROUTE_DEMOTE)
        self.assertEqual(self.route(live_suites=lambda _s: 1).route, mgw.ROUTE_DEMOTE)

    def test_the_split_is_the_suite_count_not_the_reason(self):
        # Same reason, opposite outcome, purely because the suite count differs.
        zero = self.route(reason="CI_TIMEOUT", live_suites=lambda _s: 0)
        many = self.route(reason="CI_TIMEOUT", live_suites=lambda _s: 8)
        self.assertEqual(zero.route, mgw.ROUTE_PRESERVE)
        self.assertEqual(many.route, mgw.ROUTE_DEMOTE)

    def test_unreadable_suite_count_demotes(self):
        result = self.route(live_suites=lambda _s: None)
        self.assertEqual(result.route, mgw.ROUTE_DEMOTE)
        # "could not read it" and "it was 8" are the same ROUTE but different
        # operational facts, and the emitted row is the only place an operator sees
        # which one happened. Deleting the `count is None` arm would fall through to
        # `count != 0` and still demote — behaviourally equivalent, diagnostically
        # blind — so the distinction is asserted explicitly rather than left to a
        # mutant that "survives" by being invisible.
        self.assertIn("could not re-derive", result.detail)
        self.assertNotIn("check-suite(s)", result.detail)

    def test_a_marker_for_a_different_pr_is_ignored(self):
        self.assertEqual(self.route(pr_number=99900099).route, mgw.ROUTE_DEMOTE)

    def test_no_marker_demotes(self):
        self.assertEqual(self.route(markers=()).route, mgw.ROUTE_DEMOTE)

    def test_the_newest_marker_wins(self):
        # A PR can accumulate several observations across queue attempts. The route
        # must follow the most recent one; picking the oldest would judge this dequeue
        # against a group head from a previous attempt.
        stale = marker(head="e" * 40, observed=NOW - timedelta(hours=5))
        fresh = marker(head=HEAD, observed=NOW - timedelta(minutes=5))
        seen: list[str] = []

        def spy(sha: str) -> int:
            seen.append(sha)
            return 0

        for order in ((stale, fresh), (fresh, stale)):
            seen.clear()
            result = self.route(markers=order, live_suites=spy)
            self.assertEqual(result.route, mgw.ROUTE_PRESERVE)
            self.assertEqual(seen, [HEAD], order)

    def test_marker_predating_this_queue_attempt_demotes(self):
        self.assertEqual(
            self.route(verdict=self.verdict(last_enqueued_at=NOW + timedelta(minutes=1))).route,
            mgw.ROUTE_DEMOTE,
        )

    def test_missing_enqueue_event_demotes(self):
        self.assertEqual(
            self.route(verdict=self.verdict(last_enqueued_at=None)).route, mgw.ROUTE_DEMOTE
        )

    # ── the verdict may only survive a dequeue for the tree it was granted for ──
    def test_arm_1_an_infrastructure_dequeue_leaves_the_verdict_intact(self):
        # The watchdog's own recovery: MANUAL dequeue, fresh marker, head unmoved
        # since review:pass. Nothing about this PR's diff is in question.
        result = self.route(reason="MANUAL", now=NOW + timedelta(seconds=30))
        self.assertEqual(result.route, mgw.ROUTE_PRESERVE)
        self.assertFalse(result.reenqueue)

    def test_arm_2_control_a_genuine_manual_dequeue_still_strips_the_verdict(self):
        # THE CONTROL. A human withdrawing a PR SHOULD pull the verdict. Without this
        # the fix becomes a verdict-preservation hole — any dequeue keeps its approval,
        # which is far worse than the bug being fixed. A genuine human dequeue has no
        # fresh watchdog marker.
        self.assertEqual(
            self.route(reason="MANUAL", markers=(), now=NOW + timedelta(seconds=30)).route,
            mgw.ROUTE_DEMOTE,
        )
        # ...and one whose marker is old is equally not ours.
        self.assertEqual(
            self.route(reason="MANUAL", now=NOW + timedelta(hours=3)).route,
            mgw.ROUTE_DEMOTE,
        )

    def test_a_head_that_moved_after_the_verdict_forfeits_it(self):
        # NON-NEGOTIABLE. Never restore a verdict onto a tree it was not given for.
        moved = self.verdict(head_moved_at=NOW - timedelta(minutes=1))
        for reason, when in (("MANUAL", NOW + timedelta(seconds=30)),
                             ("CI_TIMEOUT", NOW + timedelta(minutes=40))):
            result = self.route(reason=reason, verdict=moved, now=when)
            self.assertEqual(result.route, mgw.ROUTE_DEMOTE, reason)
            self.assertIn("the head moved", result.detail)

    def test_a_head_that_moved_before_the_verdict_is_fine(self):
        ok = self.verdict(head_moved_at=NOW - timedelta(hours=2, seconds=1))
        self.assertEqual(self.route(verdict=ok).route, mgw.ROUTE_PRESERVE)

    def test_no_verdict_to_preserve_demotes(self):
        self.assertEqual(
            self.route(verdict=self.verdict(review_pass_at=None)).route, mgw.ROUTE_DEMOTE
        )

    def test_a_revoked_verdict_cannot_be_preserved(self):
        revoked = self.verdict(verdict_revoked_at=NOW - timedelta(hours=1, minutes=30))
        result = self.route(verdict=revoked)
        self.assertEqual(result.route, mgw.ROUTE_DEMOTE)
        # The reason must name the REVOCATION, not invent a push. Reporting "the head
        # moved" when no commit landed sends an operator hunting a non-existent force
        # push — a misleading diagnostic baked into the tool.
        self.assertIn("REVOKED", result.detail)
        self.assertNotIn("the head moved", result.detail)

    def test_a_head_move_and_a_revocation_report_different_reasons(self):
        moved = self.route(verdict=self.verdict(head_moved_at=NOW - timedelta(minutes=1)))
        revoked = self.route(
            verdict=self.verdict(verdict_revoked_at=NOW - timedelta(minutes=1))
        )
        self.assertEqual(moved.route, mgw.ROUTE_DEMOTE)
        self.assertEqual(revoked.route, mgw.ROUTE_DEMOTE)
        self.assertIn("the head moved", moved.detail)
        self.assertNotIn("REVOKED", moved.detail)
        self.assertIn("REVOKED", revoked.detail)

    def test_a_revocation_before_the_current_grant_is_harmless(self):
        # The real #4709 shape after a manual restore: demoted at T, re-granted at T+.
        restored = self.verdict(
            review_pass_at=NOW - timedelta(minutes=10),
            verdict_revoked_at=NOW - timedelta(minutes=40),
        )
        self.assertEqual(self.route(verdict=restored).route, mgw.ROUTE_PRESERVE)

    def test_unreadable_timeline_demotes(self):
        # ISOLATED: everything else about this verdict is valid, so ONLY the
        # unreadable flag can produce the demote. The previous version of this test
        # also set review_pass_at=None, which meant it passed through the "no verdict"
        # guard and would have stayed green with the readable check deleted.
        unreadable = mgw.VerdictState(
            review_pass_at=NOW - timedelta(hours=2),
            head_moved_at=NOW - timedelta(hours=3),
            last_enqueued_at=NOW - timedelta(hours=1),
            readable=False,
        )
        result = self.route(verdict=unreadable)
        self.assertEqual(result.route, mgw.ROUTE_DEMOTE)
        # "the API failed" and "this PR was never approved" are different operational
        # facts and the emitted row is the only place anyone sees which one happened.
        self.assertIn("timeline could not be read", result.detail)
        self.assertNotIn("no review:pass grant", result.detail)

    def test_no_verdict_reports_a_different_reason_than_an_unreadable_one(self):
        result = self.route(verdict=self.verdict(review_pass_at=None))
        self.assertEqual(result.route, mgw.ROUTE_DEMOTE)
        self.assertIn("no review:pass grant", result.detail)
        self.assertNotIn("timeline could not be read", result.detail)

    def test_the_LATEST_head_move_is_the_one_that_counts(self):
        # An early move (before the verdict) must not mask a later one (after it).
        # Reading min() instead of max() would let a stale verdict survive a push.
        both = self.verdict(head_moved_at=NOW - timedelta(minutes=1))
        self.assertEqual(self.route(verdict=both).route, mgw.ROUTE_DEMOTE)

    def test_a_verdict_RE_granted_after_a_push_is_preservable(self):
        # push at T, re-review, re-approve at T+: the newest grant is what matters.
        regranted = self.verdict(
            review_pass_at=NOW - timedelta(minutes=30),
            head_moved_at=NOW - timedelta(minutes=45),
        )
        self.assertEqual(self.route(verdict=regranted).route, mgw.ROUTE_PRESERVE)

    def test_stale_marker_demotes(self):
        self.assertEqual(self.route(now=NOW + timedelta(hours=7)).route, mgw.ROUTE_DEMOTE)

    def test_other_reasons_keep_todays_behaviour(self):
        # QUEUE_CLEARED / ROLL_BACK deliberately left OUT — sq-fnr85 gave them their own
        # arm, and TestQueueLevelDropRouting below owns them. Everything here is either
        # a genuine statement about the diff (CI_FAILURE, MERGE_CONFLICT) or a reason
        # with no preserve arm at all.
        for reason in ("CI_FAILURE", "MERGE_CONFLICT",
                       "BRANCH_PROTECTIONS", "ALREADY_MERGED"):
            self.assertEqual(self.route(reason=reason).route, mgw.ROUTE_DEMOTE, reason)

    def test_watchdogs_own_dequeue_preserves_without_a_second_reenqueue(self):
        result = self.route(reason="MANUAL", now=NOW + timedelta(seconds=30))
        self.assertEqual(result.route, mgw.ROUTE_PRESERVE)
        self.assertFalse(result.reenqueue)

    def test_watchdog_attribution_expires(self):
        self.assertEqual(
            self.route(reason="MANUAL", now=NOW + timedelta(hours=2)).route,
            mgw.ROUTE_DEMOTE,
        )

    def test_a_real_failure_right_after_a_recovery_is_not_attributed_to_us(self):
        for reason in ("MERGE_CONFLICT", "CI_FAILURE"):
            self.assertEqual(
                self.route(reason=reason, now=NOW + timedelta(seconds=30)).route,
                mgw.ROUTE_DEMOTE,
                reason,
            )

    def test_reason_is_normalised(self):
        self.assertEqual(self.route(reason=" ci_timeout ").route, mgw.ROUTE_PRESERVE)
        self.assertEqual(self.route(reason="").route, mgw.ROUTE_DEMOTE)
        self.assertEqual(self.route(reason=None).route, mgw.ROUTE_DEMOTE)


class TestQueueLevelDropRouting(unittest.TestCase):
    """sq-fnr85 arm (c): the queue removed a healthy entry for a queue-level reason.

    The reported shape is an entry that leaves the queue after its group CI went green,
    with no merge and the arm cleared. Before this arm the ONLY route for it was
    `demote`, so an untouched, freshly-reviewed diff lost `review:pass`, needed a manual
    re-arm and paid a full CI re-run — the same machine-manufactured re-review #4652
    fixed for zero-dispatch.

    Every test here supplies NO marker, because this population genuinely has none: the
    watchdog records observations only for the zero-suite groups it rescues.
    """

    GRANTED = NOW - timedelta(hours=2)

    def verdict(self, *, attempts: int = 1, **overrides) -> "mgw.VerdictState":
        fields = dict(
            review_pass_at=self.GRANTED,
            head_moved_at=NOW - timedelta(hours=3),
            last_enqueued_at=NOW - timedelta(hours=1),
            enqueued_at=tuple(
                self.GRANTED + timedelta(minutes=n + 1) for n in range(attempts)
            ),
        )
        fields.update(overrides)
        return mgw.VerdictState(**fields)

    def route(self, **overrides) -> "mgw.Route":
        kwargs = dict(
            reason="ROLL_BACK",
            pr_number=99900001,
            markers=(),
            verdict=self.verdict(),
            live_suites=lambda _sha: 8,  # the group DID run; this is not zero-dispatch
            now=NOW + timedelta(minutes=40),
        )
        kwargs.update(overrides)
        return mgw.classify_dequeue_route(**kwargs)

    def test_a_queue_level_drop_preserves_the_verdict_and_re_arms(self):
        for reason in sorted(mgw.GREEN_DROP_REASONS):
            result = self.route(reason=reason)
            self.assertEqual(result.route, mgw.ROUTE_PRESERVE, reason)
            # Re-arming is the whole point: the drop cleared the arm, and without
            # `reenqueue` the PR would sit reviewed-but-unarmed until a sweep noticed.
            self.assertTrue(result.reenqueue, reason)

    def test_it_needs_no_marker(self):
        # THE headline claim. Arm (a) and arm (b) both refuse without a marker, and the
        # marker guard used to sit ahead of every arm — so this route was unreachable
        # for the entire population it exists to serve.
        self.assertEqual(self.route(markers=()).route, mgw.ROUTE_PRESERVE)

    def test_a_green_group_is_preserved_not_demoted(self):
        # The exact sq-fnr85 report: the group ran, reported, and went green, and the
        # entry was dropped anyway. A non-zero suite count must not push this into the
        # zero-dispatch arm's "the checks genuinely ran" demote.
        result = self.route(live_suites=lambda _sha: 8)
        self.assertEqual(result.route, mgw.ROUTE_PRESERVE)
        self.assertNotIn("check-suite(s)", result.detail)

    def test_reasons_that_ARE_about_the_diff_are_untouched(self):
        # THE CONTROL. A failing or conflicting PR must still lose its verdict; if this
        # arm swallowed those, it would arm broken code straight back into the queue.
        for reason in ("CI_FAILURE", "MERGE_CONFLICT"):
            self.assertEqual(self.route(reason=reason).route, mgw.ROUTE_DEMOTE, reason)

    def test_a_human_withdrawal_is_still_a_withdrawal(self):
        # MANUAL/UNKNOWN are excluded ON PURPOSE: they conflate "a reviewer withdrew
        # this" with "infrastructure moved it", and only a fresh watchdog marker (arm a)
        # can separate them. With no marker, MANUAL must demote — otherwise this arm
        # would silently undo every manual dequeue in the repo.
        for reason in ("MANUAL", "UNKNOWN"):
            self.assertEqual(self.route(reason=reason).route, mgw.ROUTE_DEMOTE, reason)

    def test_the_tree_precondition_binds_this_arm_too(self):
        # A verdict describes a TREE. Preserving it across a queue-level drop is only
        # legitimate while the head has not moved since the grant.
        moved = self.verdict(head_moved_at=NOW - timedelta(minutes=1))
        result = self.route(verdict=moved)
        self.assertEqual(result.route, mgw.ROUTE_DEMOTE)
        self.assertIn("the head moved", result.detail)

    def test_a_revoked_verdict_is_not_restored_by_a_queue_level_drop(self):
        revoked = self.verdict(verdict_revoked_at=NOW - timedelta(minutes=1))
        result = self.route(verdict=revoked)
        self.assertEqual(result.route, mgw.ROUTE_DEMOTE)
        self.assertIn("REVOKED", result.detail)

    def test_no_verdict_means_nothing_to_preserve(self):
        self.assertEqual(
            self.route(verdict=self.verdict(review_pass_at=None)).route, mgw.ROUTE_DEMOTE
        )

    def test_an_unreadable_timeline_demotes(self):
        unreadable = self.verdict(readable=False)
        result = self.route(verdict=unreadable)
        self.assertEqual(result.route, mgw.ROUTE_DEMOTE)
        self.assertIn("timeline could not be read", result.detail)

    def test_the_requeue_budget_is_enforced(self):
        budget = mgw.DEFAULT_MAX_PRESERVED_REQUEUES
        self.assertEqual(
            self.route(verdict=self.verdict(attempts=budget - 1)).route,
            mgw.ROUTE_PRESERVE,
        )
        spent = self.route(verdict=self.verdict(attempts=budget))
        self.assertEqual(spent.route, mgw.ROUTE_DEMOTE)
        self.assertIn("already ridden", spent.detail)
        # ...and it stays spent past the boundary rather than wrapping.
        self.assertEqual(
            self.route(verdict=self.verdict(attempts=budget + 5)).route, mgw.ROUTE_DEMOTE
        )

    def test_the_budget_is_bounded_and_finite(self):
        # A budget of 0 makes the arm dead code; an unbounded one makes it a retry loop.
        self.assertGreater(mgw.DEFAULT_MAX_PRESERVED_REQUEUES, 0)
        self.assertLessEqual(mgw.DEFAULT_MAX_PRESERVED_REQUEUES, 10)

    def test_only_attempts_on_THIS_tree_count_against_the_budget(self):
        # Attempts made before the current grant belong to a verdict that is already
        # gone; counting them would spend the budget of a tree that never queued.
        old = self.verdict(
            enqueued_at=(
                self.GRANTED - timedelta(hours=3),
                self.GRANTED - timedelta(hours=2),
                self.GRANTED - timedelta(hours=1),
                self.GRANTED + timedelta(minutes=1),
            )
        )
        self.assertEqual(self.route(verdict=old).route, mgw.ROUTE_PRESERVE)

    def test_an_uncountable_budget_is_not_a_spent_one_but_still_refuses(self):
        # Fail-safe: with no readable enqueue history the budget cannot be evaluated,
        # so the arm declines rather than assuming "zero attempts so far".
        blind = self.verdict(enqueued_at=())
        result = self.route(verdict=blind)
        self.assertEqual(result.route, mgw.ROUTE_DEMOTE)
        self.assertIn("budget cannot be evaluated", result.detail)

    def test_a_drop_with_no_enqueue_event_at_all_demotes(self):
        result = self.route(verdict=self.verdict(last_enqueued_at=None))
        self.assertEqual(result.route, mgw.ROUTE_DEMOTE)
        self.assertIn("cannot be bound to a queue attempt", result.detail)

    def test_the_reason_is_normalised_like_every_other_arm(self):
        self.assertEqual(self.route(reason=" roll_back ").route, mgw.ROUTE_PRESERVE)
        self.assertEqual(self.route(reason="").route, mgw.ROUTE_DEMOTE)
        self.assertEqual(self.route(reason=None).route, mgw.ROUTE_DEMOTE)

    def test_the_arm_does_not_swallow_the_zero_dispatch_arm(self):
        # Regression guard for the ORDERING. Arm (c) runs first, so a CI_TIMEOUT must
        # still fall through to the suite-count split rather than being absorbed here.
        timeout = mgw.classify_dequeue_route(
            reason="CI_TIMEOUT",
            pr_number=99900001,
            markers=(marker(),),
            verdict=self.verdict(),
            live_suites=lambda _sha: 0,
            now=NOW + timedelta(minutes=40),
        )
        self.assertEqual(timeout.route, mgw.ROUTE_PRESERVE)
        self.assertIn("check-suite", timeout.detail)


class TestQueueLevelDropEndToEnd(unittest.TestCase):
    """The same arm through the REAL timeline parser, not a hand-built VerdictState.

    `enqueued_at` is populated by `verdict_state()`; a parser that filled only
    `last_enqueued_at` would leave the budget permanently uncountable and make the whole
    arm silently unreachable — green tests, dead feature.
    """

    def test_a_roll_back_preserves_end_to_end(self):
        harness = FakeWatchdog.build(
            suites=8,
            comments=[],
            timeline=[
                {"event": "labeled", "created_at": "2026-07-27T22:50:00Z",
                 "label": {"name": "review:pass"}},
                {"event": "added_to_merge_queue", "created_at": "2026-07-27T22:58:53Z"},
            ],
        )
        route = harness.watchdog.classify_dequeue(99900001, "ROLL_BACK")
        self.assertEqual(route.route, mgw.ROUTE_PRESERVE)
        self.assertTrue(route.reenqueue)

    def test_the_parser_really_collects_every_enqueue_event(self):
        harness = FakeWatchdog.build(
            suites=8,
            comments=[],
            timeline=[
                {"event": "labeled", "created_at": "2026-07-27T22:50:00Z",
                 "label": {"name": "review:pass"}},
                {"event": "added_to_merge_queue", "created_at": "2026-07-27T22:58:53Z"},
                {"event": "added_to_merge_queue", "created_at": "2026-07-27T23:20:00Z"},
                {"event": "added_to_merge_queue", "created_at": "2026-07-27T23:45:00Z"},
            ],
        )
        state = harness.watchdog.verdict_state(99900001)
        self.assertEqual(len(state.enqueued_at), 3)
        self.assertEqual(state.last_enqueued_at, max(state.enqueued_at))
        # Three attempts on one verdict spends the budget, so this one demotes.
        self.assertEqual(
            harness.watchdog.classify_dequeue(99900001, "ROLL_BACK").route,
            mgw.ROUTE_DEMOTE,
        )

    def test_a_backdated_commit_after_the_grant_still_forfeits_the_verdict(self):
        # THE ARM'S SHARPEST EDGE: this arm consults no watchdog marker and binds to no
        # group head, so the tree precondition is the ONLY thing standing between a
        # queue-level drop and `gh pr merge --auto`. A `committed` row dates itself from
        # `committer.date`/`author.date` — metadata the pusher picks — so a commit pushed
        # after `review:pass` (a cherry-pick, `git commit --date`, `GIT_COMMITTER_DATE`)
        # can claim 22:40 and slip under the 22:50 grant. Where the row LANDS is not the
        # pusher's to write, and it lands after the grant, so the verdict must fall.
        for drop_reason in ("QUEUE_CLEARED", "ROLL_BACK"):
            with self.subTest(reason=drop_reason):
                harness = FakeWatchdog.build(
                    suites=8,
                    comments=[],
                    timeline=[
                        {"event": "labeled", "created_at": "2026-07-27T22:50:00Z",
                         "label": {"name": "review:pass"}},
                        {"event": "added_to_merge_queue",
                         "created_at": "2026-07-27T22:58:53Z"},
                        # Pushed LAST, dated BEFORE the grant — both stamps, so neither
                        # the committer nor the author fallback rescues the comparison.
                        {"event": "committed",
                         "committer": {"date": "2026-07-27T22:40:00Z"},
                         "author": {"date": "2026-07-27T22:40:00Z"}},
                    ],
                )
                route = harness.watchdog.classify_dequeue(99900001, drop_reason)
                self.assertEqual(route.route, mgw.ROUTE_DEMOTE)
                self.assertIn("timeline order decides", route.detail)

    def test_a_commit_with_no_readable_date_after_the_grant_forfeits_it(self):
        # A `committed` row whose dates are missing or unparseable contributes nothing to
        # `head_moved_at`. Before the ordering signal it therefore vanished from the
        # check entirely — an undated head move was indistinguishable from no head move.
        harness = FakeWatchdog.build(
            suites=8,
            comments=[],
            timeline=[
                {"event": "labeled", "created_at": "2026-07-27T22:50:00Z",
                 "label": {"name": "review:pass"}},
                {"event": "added_to_merge_queue", "created_at": "2026-07-27T22:58:53Z"},
                {"event": "committed", "committer": {"date": "not-a-date"}},
            ],
        )
        state = harness.watchdog.verdict_state(99900001)
        self.assertIsNone(state.head_moved_at)
        self.assertTrue(state.head_moved_after_grant_in_order)
        self.assertEqual(
            harness.watchdog.classify_dequeue(99900001, "ROLL_BACK").route,
            mgw.ROUTE_DEMOTE,
        )

    def test_a_commit_before_the_newest_grant_keeps_the_verdict(self):
        # The ordering signal must not fire on the re-grant shape, or every PR whose
        # verdict was restored after a push would be demoted straight back again.
        harness = FakeWatchdog.build(
            suites=8,
            comments=[],
            timeline=[
                {"event": "labeled", "created_at": "2026-07-27T22:20:00Z",
                 "label": {"name": "review:pass"}},
                {"event": "committed", "committer": {"date": "2026-07-27T22:45:00Z"}},
                {"event": "labeled", "created_at": "2026-07-27T22:50:00Z",
                 "label": {"name": "review:pass"}},
                {"event": "added_to_merge_queue", "created_at": "2026-07-27T22:58:53Z"},
            ],
        )
        self.assertFalse(
            harness.watchdog.verdict_state(99900001).head_moved_after_grant_in_order
        )
        self.assertEqual(
            harness.watchdog.classify_dequeue(99900001, "QUEUE_CLEARED").route,
            mgw.ROUTE_PRESERVE,
        )

    def test_classify_still_never_mutates_on_this_arm(self):
        harness = FakeWatchdog.build(
            suites=8,
            comments=[],
            timeline=[
                {"event": "labeled", "created_at": "2026-07-27T22:50:00Z",
                 "label": {"name": "review:pass"}},
                {"event": "added_to_merge_queue", "created_at": "2026-07-27T22:58:53Z"},
            ],
        )
        harness.watchdog.classify_dequeue(99900001, "QUEUE_CLEARED")
        self.assertEqual(harness.gh.mutations, [])


class TestMarkerCodec(unittest.TestCase):
    def test_round_trip(self):
        rendered = mgw.render_marker(
            pr=99900001, head=HEAD, base=BASE, ref=REF, suites=0, observed=NOW
        )
        parsed = mgw.parse_marker(f"prose\n\n{rendered}\n")
        self.assertIsNotNone(parsed)
        self.assertEqual(parsed.head, HEAD)
        self.assertEqual(parsed.base, BASE)
        self.assertEqual(parsed.ref, REF)
        self.assertEqual(parsed.suites, 0)
        self.assertEqual(parsed.observed, NOW)
        self.assertEqual(parsed.action, "re-enqueue")

    def test_a_non_hex_head_is_rejected_on_an_otherwise_valid_marker(self):
        # ISOLATED. The earlier fixture also lacked a valid `ref=`, so the newer
        # self-consistency check rejected it first and the hex guard was never reached
        # — the mutant that disabled the hex check therefore survived.
        good = mgw.render_marker(
            pr=99900001, head=HEAD, base=BASE, ref=REF, suites=0, observed=NOW
        )
        self.assertIsNotNone(mgw.parse_marker(good))
        self.assertIsNone(mgw.parse_marker(good.replace(f"head={HEAD}", "head=nothex")))
        self.assertIsNone(
            mgw.parse_marker(good.replace(f"head={HEAD}", "head=" + "g" * 40))
        )
        self.assertIsNone(
            mgw.parse_marker(good.replace(f"head={HEAD}", "head=" + HEAD[:39]))
        )

    def test_a_nonzero_suite_count_is_rejected_on_an_otherwise_valid_marker(self):
        good = mgw.render_marker(
            pr=99900001, head=HEAD, base=BASE, ref=REF, suites=0, observed=NOW
        )
        self.assertIsNotNone(mgw.parse_marker(good))
        for bad in ("suites=1", "suites=8", "suites=-1"):
            self.assertIsNone(mgw.parse_marker(good.replace("suites=0", bad)), bad)

    def test_rejects_malformed(self):
        for body in (
            "",
            None,
            "no marker",
            f"<!-- {mgw.MARKER_KEY} pr=1 head=nothex suites=0 observed=2026-01-01T00:00:00Z -->",
            f"<!-- {mgw.MARKER_KEY} pr=1 head={HEAD} suites=0 -->",
            f"<!-- {mgw.MARKER_KEY} pr=x head={HEAD} suites=0 observed=2026-01-01T00:00:00Z -->",
            f"<!-- other-tool head={HEAD} suites=0 observed=2026-01-01T00:00:00Z -->",
        ):
            self.assertIsNone(mgw.parse_marker(body), body)

    def test_queue_ref_is_base_keyed_and_matches_live_format(self):
        # Verified against live data 2026-07-28: entry pos1 pr=4644 base=c96f5abe…
        # produced ref gh-readonly-queue/main/pr-4644-c96f5abe… with head 1612bd6f…
        self.assertEqual(
            mgw.queue_ref("main", 4644, "c96f5abe3e0b7ba50c1381a1503ba961313b3da7"),
            "gh-readonly-queue/main/pr-4644-c96f5abe3e0b7ba50c1381a1503ba961313b3da7",
        )


# ── 2. behaviour (fake gh) ───────────────────────────────────────────────────────


TRUSTED_AUTHOR = "github-actions[bot]"
APP_AUTHOR = "sparq-orchestrator[bot]"
ATTACKER = "drive-by-attacker"


def _marker_comment(
    *,
    head: str = HEAD,
    observed: datetime | None = None,
    author: str = TRUSTED_AUTHOR,
    suites: int = 0,
) -> dict:
    stamp = observed or (NOW - timedelta(minutes=10))
    return {
        "user": {"login": author},
        "body": "text\n\n"
        + mgw.render_marker(
            pr=99900001, head=head, base=BASE, ref=REF, suites=suites, observed=stamp
        ),
    }


class ScriptedGh:
    """A gh runner scripted for exactly the argv shapes the watchdog issues."""

    def __init__(self, *, entries, suites_raw, activity, comments, timeline):
        self.entries = entries
        self.suites_raw = suites_raw
        self.activity = activity
        self.comments = comments
        self.timeline = timeline
        self.calls: list[list[str]] = []
        self.mutations: list[list[str]] = []
        # Substrings of the joined argv that should raise GhError instead of replying.
        self.fail_on: tuple[str, ...] = ()
        # Identity the fake runner posts under; set to an untrusted login to simulate
        # TRUSTED_MARKER_AUTHORS missing this runner.
        self.post_author: str = TRUSTED_AUTHOR
        # Comments are per-PR, as they are in the API.
        self.comments_by_pr: dict[int, list] = {}
        # Every group head the fake knows about; anything else 404s, so a lookup
        # against the wrong sha cannot silently succeed.
        self.known_shas: set[str] = {HEAD}

    def __call__(self, argv: list[str]) -> str:
        self.calls.append(argv)
        joined = " ".join(argv)
        for needle in self.fail_on:
            if needle in joined:
                raise mgw.GhError(f"simulated failure for {needle}")
        if argv[:2] == ["api", "graphql"]:
            query = next(a for a in argv if a.startswith("query="))
            # Discriminate on the OPERATION keyword, not on a substring of the
            # selection set: both mutations select `mergeQueueEntry`, so a
            # `"mergeQueue" in query` test silently swallowed them and made this fake
            # report zero mutations for a real recovery.
            if query.startswith("query=query"):
                return json.dumps(
                    {"data": {"repository": {"mergeQueue": {"entries": {"nodes": self.entries}}}}}
                )
            assert query.startswith("query=mutation"), query
            self.mutations.append(argv)
            return json.dumps({"data": {}})
        if argv[:2] == ["pr", "comment"]:
            self.mutations.append(argv)
            # Model the write: a posted comment becomes readable ON THAT PR, authored
            # by the runner's own identity. A fake that swallowed writes made the
            # readback guard untestable; a fake that pooled comments across PRs let one
            # entry's marker suppress another's recovery. Both were caught by tests
            # that had been passing.
            self.comments_by_pr.setdefault(int(argv[2]), []).append(
                {"user": {"login": self.post_author},
                 "body": argv[argv.index("--body") + 1]}
            )
            return ""
        if "/check-suites" in joined:
            # SHA-KEYED on purpose. A fake that answers every /check-suites URL with
            # the same payload cannot tell the group HEAD from its BASE (or from
            # `main`), and both of those wrong-sha mutants survived until this fake
            # started resolving the sha out of the path.
            path = next(a for a in argv if "/check-suites" in a)
            sha = path.split("/commits/", 1)[1].split("/", 1)[0]
            if sha not in self.known_shas:
                raise mgw.GhError(f"gh api: HTTP 404 — no such commit {sha}")
            return self.suites_raw
        if "/activity" in joined:
            return "\n".join(json.dumps(row) for row in self.activity)
        if "/comments" in joined:
            path = next(a for a in argv if "/comments" in a)
            pr = int(path.split("/issues/", 1)[1].split("/", 1)[0])
            rows = list(self.comments) + self.comments_by_pr.get(pr, [])
            return "\n".join(json.dumps(row) for row in rows)
        if "/timeline" in joined:
            return "\n".join(json.dumps(row) for row in self.timeline)
        raise AssertionError(f"unexpected gh call: {argv}")


class FakeWatchdog:
    def __init__(self, gh: ScriptedGh, rows: list[str], watchdog):
        self.gh = gh
        self.rows = rows
        self.watchdog = watchdog

    @classmethod
    def build(cls, *, suites: int | None = 0, entries=None, comments=None,
              activity=None, now: datetime | None = None, dry_run: bool = False,
              timeline=None):
        node = {
            "id": "MQE_99900001",
            "position": 1,
            "state": "AWAITING_CHECKS",
            "enqueuedAt": "2026-07-27T22:58:53Z",
            "baseCommit": {"oid": BASE},
            "headCommit": {"oid": HEAD},
            "pullRequest": {"number": 99900001, "id": "PR_99900001"},
        }
        if suites is None:
            suites_raw = "not json"
        else:
            suites_raw = json.dumps(
                {"total_count": suites, "check_suites": [{"id": i} for i in range(suites)]}
            )
        gh = ScriptedGh(
            entries=[node] if entries is None else entries,
            suites_raw=suites_raw,
            activity=[{"activity_type": "branch_creation", "after": HEAD,
                       "timestamp": "2026-07-27T23:04:37Z"}]
            if activity is None
            else activity,
            comments=comments or [],
            timeline=list(timeline) if timeline is not None else [
                {"event": "committed", "committer": {"date": "2026-07-27T22:40:00Z"}},
                {"event": "labeled", "created_at": "2026-07-27T22:50:00Z",
                 "label": {"name": "review:pass"}},
                {"event": "added_to_merge_queue", "created_at": "2026-07-27T22:58:53Z"},
            ],
        )
        rows: list[str] = []
        watchdog = mgw.Watchdog(
            "sparq-org/sparq",
            dry_run=dry_run,
            gh=gh,
            log=rows.append,
            now=lambda: now or NOW,
        )
        return cls(gh, rows, watchdog)

    def run(self) -> int:
        return self.watchdog.sweep()


class TestSweepBehaviour(unittest.TestCase):
    def test_zero_dispatch_entry_is_marked_then_dequeued_then_reenqueued(self):
        harness = FakeWatchdog.build(suites=0)
        self.assertEqual(harness.run(), 0)
        kinds = [m[:2] for m in harness.gh.mutations]
        self.assertEqual(
            kinds,
            [["pr", "comment"], ["api", "graphql"], ["api", "graphql"]],
            harness.gh.mutations,
        )
        # The marker comment is posted BEFORE the dequeue, so the dequeue-triggered
        # feedback workflow can positively identify our own recovery.
        self.assertEqual(harness.gh.mutations[0][:2], ["pr", "comment"])
        body = harness.gh.mutations[0][harness.gh.mutations[0].index("--body") + 1]
        self.assertIn("🤖", body)
        self.assertIsNotNone(mgw.parse_marker(body))
        # The comment goes to THIS entry's PR.
        self.assertEqual(harness.gh.mutations[0][2], "99900001")
        queries = [
            next(a for a in m if a.startswith("query="))
            for m in harness.gh.mutations[1:]
        ]
        self.assertIn("dequeuePullRequest", queries[0])
        self.assertIn("enqueuePullRequest", queries[1])
        # BOTH mutations take the PULL REQUEST node id. GitHub names the dequeue input
        # field `id` while documenting it as "the ID of the pull request", so the merge-
        # queue-entry id (MQE_…) is the natural wrong answer and would fail at runtime.
        for mutation in harness.gh.mutations[1:]:
            ident = next(a for a in mutation if a.startswith("id="))
            self.assertEqual(ident, "id=PR_99900001", mutation)
            self.assertNotIn("MQE_", ident)

    def test_unreadable_marker_history_refuses_rather_than_assuming_none(self):
        # Treating an unreadable comment history as "no prior recovery" would let the
        # per-PR cap be bypassed on every API blip.
        harness = FakeWatchdog.build(suites=0)
        harness.gh.fail_on = ("/comments",)
        self.assertGreater(harness.run(), 0)
        self.assertEqual(harness.gh.mutations, [])
        row = next(r for r in harness.rows if "decision=REFUSE" in r)
        # ONE row shape for every entry — this path used to emit a bespoke row missing
        # `state=` and `stacked=`, so an operator parsing rows saw two formats.
        for field in ("pos=", "pr=#", "state=", "ref=", "head=", "suites=", "stacked=",
                      "stacked_green=", "decision="):
            self.assertIn(field, row, (field, row))

    def test_a_failed_recovery_is_a_red_run_not_a_silent_success(self):
        harness = FakeWatchdog.build(suites=0)
        harness.gh.fail_on = ("dequeuePullRequest",)
        self.assertGreater(harness.run(), 0)
        self.assertTrue(any("RECOVERY FAILED" in row for row in harness.rows), harness.rows)
        self.assertTrue(any("::error" in row for row in harness.rows), harness.rows)

    def test_recovery_aborts_if_its_own_marker_is_not_readable_back(self):
        # If this runner's login is not in TRUSTED_MARKER_AUTHORS, every marker the
        # watchdog writes is invisible to itself: the per-head NOOP never fires and the
        # per-PR cap never binds, so it would thrash dequeue/enqueue every tick. The
        # recovery must PROVE its evidence channel works before using it. Driven by
        # making the fake runner post under an untrusted identity — the real failure.
        harness = FakeWatchdog.build(suites=0)
        harness.gh.post_author = ATTACKER
        self.assertGreater(harness.run(), 0)
        kinds = [m[:2] for m in harness.gh.mutations]
        self.assertEqual(kinds, [["pr", "comment"]], harness.gh.mutations)
        self.assertTrue(any("RECOVERY FAILED" in r for r in harness.rows), harness.rows)
        self.assertTrue(
            any("not readable back as TRUSTED" in r for r in harness.rows), harness.rows
        )

    def test_the_readback_must_find_THIS_head_not_merely_a_marker(self):
        # A PR that already carries an older trusted marker (a previous recovery, a
        # different group head) must NOT satisfy the readback when the new post fails
        # to register. Matching "any trusted marker" instead of "a marker for this
        # head" would wave through exactly the misconfiguration the guard exists for.
        harness = FakeWatchdog.build(
            suites=0, comments=[_marker_comment(head="9" * 40)]  # older, trusted, other head
        )
        harness.gh.post_author = ATTACKER      # the new marker lands untrusted
        self.assertGreater(harness.run(), 0)
        self.assertEqual([m[:2] for m in harness.gh.mutations], [["pr", "comment"]])
        self.assertTrue(
            any("not readable back as TRUSTED" in r for r in harness.rows), harness.rows
        )

    def test_recovery_proceeds_when_the_marker_reads_back(self):
        # The paired control: the readback must not block the normal path.
        for author in (TRUSTED_AUTHOR, APP_AUTHOR):
            harness = FakeWatchdog.build(suites=0)
            harness.gh.post_author = author
            self.assertEqual(harness.run(), 0, author)
            self.assertEqual(
                [m[:2] for m in harness.gh.mutations],
                [["pr", "comment"], ["api", "graphql"], ["api", "graphql"]],
                author,
            )

    def test_control_group_with_suites_issues_no_mutation_at_all(self):
        harness = FakeWatchdog.build(suites=8)
        self.assertEqual(harness.run(), 0)
        self.assertEqual(harness.gh.mutations, [])
        self.assertTrue(any("decision=HOLD" in row for row in harness.rows), harness.rows)
        self.assertTrue(any("suites=8" in row for row in harness.rows), harness.rows)

    def test_unreadable_suite_count_issues_no_mutation(self):
        harness = FakeWatchdog.build(suites=None)
        harness.run()
        self.assertEqual(harness.gh.mutations, [])
        self.assertTrue(any("decision=REFUSE" in row for row in harness.rows), harness.rows)

    def test_missing_activity_row_issues_no_mutation(self):
        harness = FakeWatchdog.build(suites=0, activity=[])
        harness.run()
        self.assertEqual(harness.gh.mutations, [])
        self.assertTrue(any("decision=REFUSE" in row for row in harness.rows), harness.rows)

    def test_a_non_creation_activity_row_is_not_our_anchor(self):
        # The URL already filters activity_type server-side; this pins the client-side
        # half so a force_push / branch_deletion row carrying the same `after` sha can
        # never be mistaken for the group's build time.
        for activity_type in ("force_push", "push", "branch_deletion", "merge_queue_merge"):
            harness = FakeWatchdog.build(
                suites=0,
                activity=[{"activity_type": activity_type, "after": HEAD,
                           "timestamp": "2026-07-27T23:04:37Z"}],
            )
            harness.run()
            self.assertEqual(harness.gh.mutations, [], activity_type)
            self.assertTrue(
                any("decision=REFUSE" in row for row in harness.rows), activity_type
            )

    def test_activity_row_for_a_different_head_is_not_our_anchor(self):
        harness = FakeWatchdog.build(
            suites=0,
            activity=[{"activity_type": "branch_creation", "after": "f" * 40,
                       "timestamp": "2026-07-27T23:04:37Z"}],
        )
        harness.run()
        self.assertEqual(harness.gh.mutations, [])

    def test_second_detection_on_the_same_ref_issues_no_mutation(self):
        harness = FakeWatchdog.build(suites=0, comments=[_marker_comment()])
        harness.run()
        self.assertEqual(harness.gh.mutations, [])
        self.assertTrue(any("decision=NOOP" in row for row in harness.rows), harness.rows)

    def test_dry_run_decides_but_never_mutates(self):
        harness = FakeWatchdog.build(suites=0, dry_run=True)
        harness.run()
        self.assertEqual(harness.gh.mutations, [])
        self.assertTrue(any("decision=RECOVER" in row for row in harness.rows), harness.rows)

    # Each queue entry has its OWN group head, chained on the previous one — the
    # original fixture reused a single sha for all three, which made one entry's
    # marker suppress the next entry's recovery once the fake modelled writes.
    HEADS = (HEAD, "1" * 40, "2" * 40)

    @classmethod
    def _three_entry_queue(cls):
        entries = []
        for index, (pr, head) in enumerate(zip((99900001, 99900002, 99900003), cls.HEADS), start=1):
            entries.append(
                {
                    "id": f"MQE_{pr}",
                    "position": index,
                    "state": "AWAITING_CHECKS",
                    "enqueuedAt": "2026-07-27T22:58:53Z",
                    "baseCommit": {"oid": BASE if index == 1 else cls.HEADS[index - 2]},
                    "headCommit": {"oid": head},
                    "pullRequest": {"number": pr, "id": f"PR_{pr}"},
                }
            )
        return entries

    @classmethod
    def _three_entry_harness(cls):
        harness = FakeWatchdog.build(suites=0, entries=cls._three_entry_queue())
        harness.gh.known_shas = set(cls.HEADS)
        harness.gh.activity = [
            {"activity_type": "branch_creation", "after": head,
             "timestamp": "2026-07-27T23:04:37Z"}
            for head in cls.HEADS
        ]
        return harness

    def test_per_run_budget_bounds_one_tick(self):
        harness = self._three_entry_harness()
        harness.run()
        dequeues = [
            m
            for m in harness.gh.mutations
            if any("dequeuePullRequest" in a for a in m)
        ]
        self.assertEqual(len(dequeues), mgw.DEFAULT_MAX_RECOVERIES_PER_RUN)
        self.assertTrue(any("decision=CAP" in row for row in harness.rows), harness.rows)

    def test_the_budget_is_spent_on_the_head_of_the_queue_first(self):
        # Only the head-of-line entry actually blocks merges, so a bounded budget must
        # be spent in queue order. Recovering positions 3 and 2 while position 1 stays
        # broken would burn the budget and leave the outage in place.
        harness = self._three_entry_harness()
        harness.run()
        recovered = [
            m[2] for m in harness.gh.mutations if m[:2] == ["pr", "comment"]
        ]
        self.assertEqual(recovered, ["99900001", "99900002"], harness.rows)


class TestMarkerAuthorIsPartOfThePredicate(unittest.TestCase):
    """sparq is PUBLIC: anyone can comment, so the marker's AUTHOR is the control.

    A marker is unauthenticated author-controlled input and the routing split acts on
    it. Forged, it makes `review:pass` survive a dequeue and skips the arm-disable, so
    auto-arm/rearm-sweeper re-arm within ~10 min — and the CI_TIMEOUT arm reaches
    `gh pr merge --auto` directly. Both directions are pinned here: an untrusted marker
    is IGNORED, and a trusted one still WORKS.
    """

    def test_a_forged_marker_from_an_arbitrary_author_is_ignored(self):
        harness = FakeWatchdog.build(
            suites=0, comments=[_marker_comment(author=ATTACKER)]
        )
        self.assertEqual(harness.watchdog.pr_markers(99900001), [])
        route = harness.watchdog.classify_dequeue(99900001, "MANUAL")
        self.assertEqual(route.route, mgw.ROUTE_DEMOTE)

    def test_the_exact_ci_timeout_forgery_is_ignored(self):
        # A forged marker naming ANY commit that reads 0 check-suites — such a sha is
        # trivially obtainable. Without the author filter this reaches
        # `gh pr merge --auto` via route=preserve + reenqueue=true.
        harness = FakeWatchdog.build(
            suites=0, comments=[_marker_comment(author=ATTACKER)]
        )
        route = harness.watchdog.classify_dequeue(99900001, "CI_TIMEOUT")
        self.assertEqual(route.route, mgw.ROUTE_DEMOTE)
        self.assertFalse(route.reenqueue)

    def test_forged_zero_sha_head_is_ignored(self):
        harness = FakeWatchdog.build(
            suites=0, comments=[_marker_comment(author=ATTACKER, head="0" * 40)]
        )
        self.assertEqual(
            harness.watchdog.classify_dequeue(99900001, "MANUAL").route, mgw.ROUTE_DEMOTE
        )

    # THE PAIRED CONTROL: the filter must not break the real thing.
    def test_a_trusted_bot_marker_still_works(self):
        for author in (TRUSTED_AUTHOR, APP_AUTHOR):
            harness = FakeWatchdog.build(
                suites=0, comments=[_marker_comment(author=author)]
            )
            self.assertEqual(len(harness.watchdog.pr_markers(99900001)), 1, author)
            route = harness.watchdog.classify_dequeue(99900001, "MANUAL")
            self.assertEqual(route.route, mgw.ROUTE_PRESERVE, author)

    def test_both_configured_identities_are_allow_listed(self):
        # The sweep runs under the App token when ORCHESTRATOR_APP_ID is set and the
        # workflow token otherwise, so BOTH identities must be trusted or the caps stop
        # working the moment the App is configured (or unconfigured).
        self.assertEqual(
            mgw.TRUSTED_MARKER_AUTHORS,
            frozenset({"github-actions[bot]", "sparq-orchestrator[bot]"}),
        )

    def test_a_forgery_attempt_is_logged_not_silently_dropped(self):
        harness = FakeWatchdog.build(
            suites=0, comments=[_marker_comment(author=ATTACKER)]
        )
        harness.watchdog.pr_markers(99900001)
        self.assertTrue(any("untrusted marker" in r for r in harness.rows), harness.rows)
        self.assertTrue(any(ATTACKER in r for r in harness.rows), harness.rows)

    def test_mass_forgery_emits_ONE_annotation_not_one_per_marker(self):
        # MEASURED: PR #4534 already carries 96 forged markers naming the real
        # zero-suite head. A per-marker annotation produced ~110 KB of log and would
        # blow GitHub's annotation cap, burying every other signal — so the volume of
        # an attack must not become the volume of the log.
        harness = FakeWatchdog.build(
            suites=0,
            comments=[_marker_comment(author=ATTACKER, head=f"{i:040x}") for i in range(96)],
        )
        self.assertEqual(harness.watchdog.pr_markers(99900001), [])
        warnings = [r for r in harness.rows if "::warning" in r]
        self.assertEqual(len(warnings), 1, warnings)
        self.assertIn("96 watchdog marker(s)", warnings[0])
        self.assertIn(ATTACKER, warnings[0])

    def test_the_summary_names_every_distinct_untrusted_author(self):
        harness = FakeWatchdog.build(
            suites=0,
            comments=[
                _marker_comment(author="attacker-one", head="a" * 40),
                _marker_comment(author="attacker-two", head="b" * 40),
                _marker_comment(author="attacker-one", head="c" * 40),
            ],
        )
        harness.watchdog.pr_markers(99900001)
        warning = next(r for r in harness.rows if "::warning" in r)
        self.assertIn("attacker-one", warning)
        self.assertIn("attacker-two", warning)
        self.assertIn("3 watchdog marker(s)", warning)

    def test_a_forged_marker_cannot_suppress_a_real_recovery_either(self):
        # The idempotence key is read from the SAME channel, so an attacker must not be
        # able to post a marker for the live head and block the watchdog from acting.
        harness = FakeWatchdog.build(
            suites=0, comments=[_marker_comment(author=ATTACKER)]
        )
        harness.run()
        self.assertTrue(
            any("decision=RECOVER" in r for r in harness.rows), harness.rows
        )

    def test_an_untrusted_marker_does_not_consume_the_cap(self):
        harness = FakeWatchdog.build(
            suites=0,
            comments=[
                _marker_comment(author=ATTACKER, head="a" * 40),
                _marker_comment(author=ATTACKER, head="b" * 40),
                _marker_comment(author=ATTACKER, head="c" * 40),
            ],
        )
        harness.run()
        self.assertTrue(any("decision=RECOVER" in r for r in harness.rows), harness.rows)

    def test_the_marker_action_must_be_a_recovery(self):
        # An observation-shaped marker must never read as a recovery in flight.
        body = mgw.render_marker(
            pr=99900001, head=HEAD, base=BASE, ref=REF, suites=0,
            observed=NOW - timedelta(minutes=10),
        ).replace("action=re-enqueue", "action=observed")
        harness = FakeWatchdog.build(
            suites=0, comments=[{"user": {"login": TRUSTED_AUTHOR}, "body": body}]
        )
        self.assertEqual(
            harness.watchdog.classify_dequeue(99900001, "MANUAL").route, mgw.ROUTE_DEMOTE
        )
        self.assertEqual(mgw.MARKER_ACTION_REENQUEUE, "re-enqueue")

    def test_self_dequeue_arm_re_derives_the_named_heads_suite_count(self):
        # The arm used to compare the marker head to nothing at all.
        unreadable = FakeWatchdog.build(
            suites=0, comments=[_marker_comment(head="a" * 40)]
        )
        route = unreadable.watchdog.classify_dequeue(99900001, "MANUAL")
        self.assertEqual(route.route, mgw.ROUTE_DEMOTE)
        self.assertIn("cannot be read", route.detail)

        dispatched = FakeWatchdog.build(suites=8, comments=[_marker_comment()])
        route = dispatched.watchdog.classify_dequeue(99900001, "MANUAL")
        self.assertEqual(route.route, mgw.ROUTE_DEMOTE)
        self.assertIn("WAS dispatched", route.detail)

    def test_a_marker_claiming_a_nonzero_suite_count_is_not_ours(self):
        self.assertIsNone(
            mgw.parse_marker(
                mgw.render_marker(
                    pr=1, head=HEAD, base=BASE, ref=REF, suites=1, observed=NOW
                )
            )
        )


class TestParseIsBoundedAndTrustGatedFirst(unittest.TestCase):
    """A public repo means anonymous input reaches this parser on every dequeue."""

    ADVERSARIAL = "<!-- merge-group-watchdog " * 2521          # ~65 536 chars

    def test_a_huge_adversarial_body_parses_in_bounded_time(self):
        import time
        start = time.monotonic()
        self.assertIsNone(mgw.parse_marker(self.ADVERSARIAL))
        elapsed = time.monotonic() - start
        # Unbounded `[^>]*?` took 21.4 s on this input. Bounded, it is milliseconds.
        # One second is a generous ceiling that still fails loudly on a regression.
        self.assertLess(elapsed, 1.0, f"parse took {elapsed:.3f}s — quantifier unbounded?")

    def test_the_bound_still_admits_a_real_marker(self):
        # The control: bounding the quantifier must not break the genuine payload,
        # whose key/value run is 262 chars.
        rendered = mgw.render_marker(
            pr=99900001, head=HEAD, base=BASE, ref=REF, suites=0, observed=NOW
        )
        kv = rendered[rendered.index(mgw.MARKER_KEY) + len(mgw.MARKER_KEY):].strip()[:-3]
        self.assertLess(len(kv), 400, len(kv))
        self.assertIsNotNone(mgw.parse_marker(rendered))

    def test_untrusted_bodies_are_never_parsed_at_all(self):
        # ORDERING IS THE FIX. A trust check placed AFTER the parse does not protect
        # the parse: every anonymous comment was parsed before being discarded.
        parsed: list = []
        real_parse = mgw.parse_marker

        def spy(body):
            parsed.append(body)
            return real_parse(body)

        harness = FakeWatchdog.build(
            suites=0,
            comments=[_marker_comment(author=ATTACKER), _marker_comment(author=TRUSTED_AUTHOR)],
        )
        try:
            mgw.parse_marker = spy
            harness.watchdog.pr_markers(99900001)
        finally:
            mgw.parse_marker = real_parse
        self.assertEqual(len(parsed), 1, "the untrusted body was parsed")

    def test_end_to_end_with_many_adversarial_comments_is_fast(self):
        import time
        harness = FakeWatchdog.build(
            suites=0,
            comments=[{"user": {"login": ATTACKER}, "body": self.ADVERSARIAL} for _ in range(8)],
        )
        start = time.monotonic()
        self.assertEqual(harness.watchdog.pr_markers(99900001), [])
        elapsed = time.monotonic() - start
        self.assertLess(elapsed, 1.0, f"{elapsed:.3f}s for 8 adversarial comments")


class TestQuotedMarkersAreNotClaims(unittest.TestCase):
    """A trusted bot that ECHOES PR content must not launder an attacker's marker."""

    def marker_text(self) -> str:
        return mgw.render_marker(
            pr=99900001, head=HEAD, base=BASE, ref=REF, suites=0, observed=NOW
        )

    def test_quoted_contexts_are_stripped(self):
        m = self.marker_text()
        for label, body in (
            ("fenced", f"see below\n\n```\n{m}\n```\n"),
            ("fenced-tilde", f"~~~\n{m}\n~~~\n"),
            ("blockquote", f"> {m}\n"),
            ("indented", f"    {m}\n"),
            ("inline", f"`{m}`\n"),
            ("nested-html", f"<!-- quoted: {m} -->\n"),
        ):
            self.assertIsNone(mgw.parse_marker(body), label)

    def test_an_unquoted_marker_is_still_a_claim(self):
        # THE CONTROL: the watchdog's own comment must keep working. Its self-id line
        # IS a blockquote, and the marker sits unquoted on its own line below it.
        body = "> 🤖 **SPARQ agent** — merge-group watchdog.\n\nprose\n\n" + self.marker_text()
        self.assertIsNotNone(mgw.parse_marker(body))

    def test_a_trusted_bot_quoting_an_attackers_marker_is_ignored(self):
        harness = FakeWatchdog.build(
            suites=0,
            comments=[{
                "user": {"login": APP_AUTHOR},
                "body": "Quoting the PR body:\n\n```\n" + self.marker_text() + "\n```\n",
            }],
        )
        self.assertEqual(harness.watchdog.pr_markers(99900001), [])
        self.assertEqual(
            harness.watchdog.classify_dequeue(99900001, "MANUAL").route, mgw.ROUTE_DEMOTE
        )


class TestMarkerIsBoundToThisPrAndHead(unittest.TestCase):
    def test_a_marker_for_another_pr_is_not_evidence_here(self):
        other = _marker_comment()
        other["body"] = other["body"].replace("pr=99900001", "pr=99900099").replace(
            "/pr-99900001-", "/pr-9999-"
        )
        harness = FakeWatchdog.build(suites=0, comments=[other])
        self.assertEqual(
            harness.watchdog.classify_dequeue(99900001, "CI_TIMEOUT").route, mgw.ROUTE_DEMOTE
        )

    def test_pr_and_ref_must_agree(self):
        # A single mistyped identity field cannot pass: both are checked.
        body = mgw.render_marker(
            pr=99900001, head=HEAD, base=BASE, ref=REF, suites=0, observed=NOW
        ).replace("pr=99900001", "pr=99900099")
        self.assertIsNone(mgw.parse_marker(body))

    def test_arm_b_requires_the_recovery_action(self):
        body = mgw.render_marker(
            pr=99900001, head=HEAD, base=BASE, ref=REF, suites=0, observed=NOW
        ).replace("action=re-enqueue", "action=observed")
        harness = FakeWatchdog.build(
            suites=0, comments=[{"user": {"login": TRUSTED_AUTHOR}, "body": body}]
        )
        route = harness.watchdog.classify_dequeue(99900001, "CI_TIMEOUT")
        self.assertEqual(route.route, mgw.ROUTE_DEMOTE)
        self.assertIn("action", route.detail)


class TestCapExhaustionBehaviour(unittest.TestCase):
    """Pin the DOCUMENTED behaviour so the claim cannot drift away from the code again.

    Recovery is marker -> dequeue -> enqueue, so the marker is always ~30 s OLDER than
    the `added_to_merge_queue` it produces. On the next attempt the newest trusted
    marker therefore predates that attempt, the queue-attempt binding refuses it, and
    the platform CI_TIMEOUT DEMOTES. That is correct — the only trusted observation
    names a superseded head — but it is the opposite of what four separate comments
    used to claim.
    """

    def test_after_exhaustion_a_ci_timeout_demotes(self):
        harness = FakeWatchdog.build(
            suites=0,
            comments=[_marker_comment(observed=NOW - timedelta(minutes=20))],
            timeline=[
                {"event": "labeled", "created_at": "2026-07-27T22:20:00Z",
                 "label": {"name": "review:pass"}},
                # the attempt the watchdog's own re-enqueue produced, AFTER the marker
                {"event": "added_to_merge_queue", "created_at": "2026-07-27T23:15:00Z"},
            ],
        )
        route = harness.watchdog.classify_dequeue(99900001, "CI_TIMEOUT")
        self.assertEqual(route.route, mgw.ROUTE_DEMOTE)
        self.assertIn("predates this queue attempt", route.detail)

    def test_the_cap_row_does_not_claim_the_verdict_is_preserved(self):
        row = decide(recoveries_in_window=2)
        self.assertEqual(row.verdict, mgw.CAP)
        self.assertIn("WILL demote", row.detail)
        self.assertNotIn("preserves the review verdict", row.detail)

    def test_exhaustion_never_escalates_to_a_human_hold(self):
        harness = FakeWatchdog.build(
            suites=0,
            comments=[_marker_comment(head="a" * 40), _marker_comment(head="b" * 40)],
        )
        harness.run()
        joined = "\n".join(harness.rows)
        self.assertIn("decision=CAP", joined)
        self.assertNotIn("needs:user", joined)


class TestEmission(unittest.TestCase):
    """A silent watchdog turns a visible outage into an invisible one."""

    def test_every_entry_emits_a_row_naming_ref_suites_and_decision(self):
        for suites in (0, 8, None):
            harness = FakeWatchdog.build(suites=suites)
            harness.run()
            rows = [r for r in harness.rows if "decision=" in r]
            self.assertEqual(len(rows), 1, (suites, harness.rows))
            row = rows[0]
            self.assertIn(f"ref={REF}", row)
            self.assertIn("suites=", row)
            self.assertIn(f"head={HEAD[:8]}", row)
            self.assertIn("pr=#99900001", row)

    def test_rows_record_how_many_groups_are_stacked_above(self):
        # The watchdog's own effectiveness measure. A rebuild discards every group
        # chained on top of the dead one, so `stacked` IS the cost multiplier — and a
        # climbing `stacked` over time is the signal that the grace has drifted too
        # long. Measured 2026-07-28: a 48-minute intervention had stacked=4,
        # stacked_green=2, and all four were discarded.
        nodes = [
            {
                "id": "MQE_99900004", "position": 1, "state": "AWAITING_CHECKS",
                "enqueuedAt": "2026-07-27T22:58:53Z",
                "baseCommit": {"oid": BASE}, "headCommit": {"oid": HEAD},
                "pullRequest": {"number": 99900004, "id": "PR_99900004"},
            },
            {
                "id": "MQE_99900005", "position": 2, "state": "MERGEABLE",
                "enqueuedAt": "2026-07-27T22:59:00Z",
                "baseCommit": {"oid": HEAD}, "headCommit": {"oid": "a" * 40},
                "pullRequest": {"number": 99900005, "id": "PR_99900005"},
            },
            {
                "id": "MQE_99900006", "position": 3, "state": "MERGEABLE",
                "enqueuedAt": "2026-07-27T22:59:10Z",
                "baseCommit": {"oid": "a" * 40}, "headCommit": {"oid": "b" * 40},
                "pullRequest": {"number": 99900006, "id": "PR_99900006"},
            },
            {
                # No group built yet — nothing to discard, so it must NOT be counted.
                "id": "MQE_99900007", "position": 4, "state": "QUEUED",
                "enqueuedAt": "2026-07-27T22:59:20Z",
                "baseCommit": None, "headCommit": None,
                "pullRequest": {"number": 99900007, "id": "PR_99900007"},
            },
        ]
        harness = FakeWatchdog.build(suites=0, entries=nodes)
        harness.run()
        head_row = next(r for r in harness.rows if "pr=#99900004" in r)
        self.assertIn("stacked=2", head_row)
        self.assertIn("stacked_green=2", head_row)
        # The last entry with a group has nothing above it.
        tail_row = next(r for r in harness.rows if "pr=#99900006" in r)
        self.assertIn("stacked=0", tail_row)
        self.assertIn("stacked_green=0", tail_row)

    def test_holding_states_re_emit_every_tick(self):
        # Three consecutive ticks on an unchanged CAP/NOOP condition must each emit.
        for _ in range(3):
            harness = FakeWatchdog.build(suites=0, comments=[_marker_comment()])
            harness.run()
            self.assertTrue(any("decision=NOOP" in r for r in harness.rows))
            self.assertTrue(any("::warning" in r for r in harness.rows), harness.rows)

    def test_empty_queue_emits_an_explicit_row(self):
        harness = FakeWatchdog.build(entries=[])
        harness.run()
        self.assertTrue(any("EMPTY" in r for r in harness.rows), harness.rows)

    def test_census_row_closes_every_sweep(self):
        harness = FakeWatchdog.build(suites=8)
        harness.run()
        self.assertTrue(any("sweep complete" in r for r in harness.rows), harness.rows)

    def test_refusals_and_caps_are_warnings_not_silence(self):
        harness = FakeWatchdog.build(suites=None)
        harness.run()
        self.assertTrue(any("::warning" in r and "REFUSE" in r for r in harness.rows),
                        harness.rows)


class TestTheWatchdogIsNotSilentByDefault(unittest.TestCase):
    """A silent watchdog turns a visible outage into an invisible one — and the
    DEFAULT is what runs in CI. Both `log=print` defaults survived the sweep, which
    is exactly the census-silence class this PR exists to prevent."""

    def test_sweep_writes_to_stdout_when_no_logger_is_injected(self):
        import io
        from contextlib import redirect_stdout

        harness = FakeWatchdog.build(suites=8)
        # rebuild WITHOUT log=, so the module default is exercised
        watchdog = mgw.Watchdog(
            "sparq-org/sparq", gh=harness.gh, now=lambda: NOW
        )
        out = io.StringIO()
        with redirect_stdout(out):
            watchdog.sweep()
        text = out.getvalue()
        self.assertIn("decision=HOLD", text)
        self.assertIn("sweep complete", text)

    def test_write_outputs_writes_to_stdout_when_no_logger_is_injected(self):
        import io
        import os
        import tempfile
        from contextlib import redirect_stdout

        out_file = Path(tempfile.mkdtemp()) / "outputs.txt"
        out_file.write_text("", encoding="utf-8")
        os.environ["GITHUB_OUTPUT"] = str(out_file)
        buf = io.StringIO()
        try:
            with redirect_stdout(buf):
                mgw.write_outputs(mgw.Route(mgw.ROUTE_DEMOTE, False, "no marker"))
        finally:
            os.environ.pop("GITHUB_OUTPUT", None)
        self.assertIn("route=demote", buf.getvalue())


class TestClassifyEndToEnd(unittest.TestCase):
    def test_live_suite_count_is_re_derived_not_read_off_the_marker(self):
        # The marker says suites=0, but the LIVE count now reads 8. The route must
        # follow the live re-derivation, not the recorded value.
        harness = FakeWatchdog.build(suites=8, comments=[_marker_comment()])
        route = harness.watchdog.classify_dequeue(99900001, "CI_TIMEOUT")
        self.assertEqual(route.route, mgw.ROUTE_DEMOTE)

    def test_zero_live_count_preserves(self):
        harness = FakeWatchdog.build(suites=0, comments=[_marker_comment()])
        route = harness.watchdog.classify_dequeue(99900001, "CI_TIMEOUT")
        self.assertEqual(route.route, mgw.ROUTE_PRESERVE)

    def test_the_LATEST_enqueue_binds_the_marker_not_the_earliest(self):
        # Every other timeline fixture carries exactly ONE added_to_merge_queue, so
        # max() and min() were indistinguishable and the mutant survived. Two attempts:
        # the marker sits AFTER the first enqueue but BEFORE the second, so only
        # max() correctly rejects it as predating the CURRENT attempt.
        harness = FakeWatchdog.build(
            suites=0,
            comments=[_marker_comment(observed=mgw.parse_iso("2026-07-27T22:40:00Z"))],
            timeline=[
                {"event": "labeled", "created_at": "2026-07-27T22:20:00Z",
                 "label": {"name": "review:pass"}},
                {"event": "added_to_merge_queue", "created_at": "2026-07-27T22:30:00Z"},
                {"event": "added_to_merge_queue", "created_at": "2026-07-27T22:50:00Z"},
            ],
        )
        route = harness.watchdog.classify_dequeue(99900001, "CI_TIMEOUT")
        self.assertEqual(route.route, mgw.ROUTE_DEMOTE)
        self.assertIn("predates this queue attempt", route.detail)

    def test_a_marker_after_the_latest_enqueue_still_binds(self):
        # The control: with the marker after BOTH enqueues it must still preserve.
        harness = FakeWatchdog.build(
            suites=0,
            comments=[_marker_comment(observed=mgw.parse_iso("2026-07-27T23:00:00Z"))],
            timeline=[
                {"event": "labeled", "created_at": "2026-07-27T22:20:00Z",
                 "label": {"name": "review:pass"}},
                {"event": "added_to_merge_queue", "created_at": "2026-07-27T22:30:00Z"},
                {"event": "added_to_merge_queue", "created_at": "2026-07-27T22:50:00Z"},
            ],
        )
        self.assertEqual(
            harness.watchdog.classify_dequeue(99900001, "CI_TIMEOUT").route,
            mgw.ROUTE_PRESERVE,
        )

    def test_a_commit_after_the_verdict_forfeits_it_end_to_end(self):
        # Exercises the real timeline PARSER, not just the policy: a `committed` event
        # carries no `created_at`, so its time must be read from `committer.date`.
        harness = FakeWatchdog.build(
            suites=0,
            comments=[_marker_comment()],
            timeline=[
                {"event": "labeled", "created_at": "2026-07-27T22:50:00Z",
                 "label": {"name": "review:pass"}},
                {"event": "added_to_merge_queue", "created_at": "2026-07-27T22:58:53Z"},
                {"event": "committed", "committer": {"date": "2026-07-27T23:05:00Z"}},
            ],
        )
        route = harness.watchdog.classify_dequeue(99900001, "CI_TIMEOUT")
        self.assertEqual(route.route, mgw.ROUTE_DEMOTE)
        self.assertIn("the head moved", route.detail)

    def test_a_force_push_after_the_verdict_forfeits_it_end_to_end(self):
        harness = FakeWatchdog.build(
            suites=0,
            comments=[_marker_comment()],
            timeline=[
                {"event": "labeled", "created_at": "2026-07-27T22:50:00Z",
                 "label": {"name": "review:pass"}},
                {"event": "added_to_merge_queue", "created_at": "2026-07-27T22:58:53Z"},
                {"event": "head_ref_force_pushed", "created_at": "2026-07-27T23:05:00Z"},
            ],
        )
        self.assertEqual(
            harness.watchdog.classify_dequeue(99900001, "CI_TIMEOUT").route, mgw.ROUTE_DEMOTE
        )

    def test_a_revocation_then_regrant_preserves_end_to_end(self):
        # PR #4709 as it actually stands: bot demoted it at 06:06:55, the verdict was
        # restored at 06:36:41, no commits since. Validated against the live timeline.
        harness = FakeWatchdog.build(
            suites=0,
            comments=[_marker_comment()],
            timeline=[
                {"event": "labeled", "created_at": "2026-07-27T22:20:00Z",
                 "label": {"name": "review:pass"}},
                {"event": "unlabeled", "created_at": "2026-07-27T22:30:00Z",
                 "label": {"name": "review:pass"}},
                {"event": "labeled", "created_at": "2026-07-27T22:50:00Z",
                 "label": {"name": "review:pass"}},
                {"event": "added_to_merge_queue", "created_at": "2026-07-27T22:58:53Z"},
            ],
        )
        self.assertEqual(
            harness.watchdog.classify_dequeue(99900001, "CI_TIMEOUT").route,
            mgw.ROUTE_PRESERVE,
        )

    def test_a_revoked_verdict_forfeits_it_end_to_end(self):
        harness = FakeWatchdog.build(
            suites=0,
            comments=[_marker_comment()],
            timeline=[
                {"event": "labeled", "created_at": "2026-07-27T22:50:00Z",
                 "label": {"name": "review:pass"}},
                {"event": "unlabeled", "created_at": "2026-07-27T22:55:00Z",
                 "label": {"name": "review:pass"}},
                {"event": "added_to_merge_queue", "created_at": "2026-07-27T22:58:53Z"},
            ],
        )
        route = harness.watchdog.classify_dequeue(99900001, "CI_TIMEOUT")
        self.assertEqual(route.route, mgw.ROUTE_DEMOTE)
        # Through the real PARSER, not a hand-built VerdictState: a revocation must be
        # reported as a revocation. Folding it into head_moved_at still demotes, so the
        # route alone cannot catch it — only the reason can.
        self.assertIn("REVOKED", route.detail)
        self.assertNotIn("the head moved", route.detail)

    def test_the_LATEST_revocation_counts_end_to_end(self):
        # revoke, re-grant, revoke again. min(revoked) picks the FIRST revocation,
        # which predates the grant, and would wrongly preserve.
        harness = FakeWatchdog.build(
            suites=0,
            comments=[_marker_comment()],
            timeline=[
                {"event": "unlabeled", "created_at": "2026-07-27T22:10:00Z",
                 "label": {"name": "review:pass"}},
                {"event": "labeled", "created_at": "2026-07-27T22:20:00Z",
                 "label": {"name": "review:pass"}},
                {"event": "unlabeled", "created_at": "2026-07-27T22:30:00Z",
                 "label": {"name": "review:pass"}},
                {"event": "added_to_merge_queue", "created_at": "2026-07-27T22:58:53Z"},
            ],
        )
        route = harness.watchdog.classify_dequeue(99900001, "CI_TIMEOUT")
        self.assertEqual(route.route, mgw.ROUTE_DEMOTE)
        self.assertIn("REVOKED", route.detail)

    def test_removing_an_unrelated_label_is_not_a_revocation_end_to_end(self):
        # PRs shed `area:*`, `status:*`, `needs:*` labels constantly. Treating any
        # `unlabeled` event as a verdict revocation would make the preserve route
        # unreachable in practice, silently and greenly.
        for other in ("area:ci", "status:untriaged", "needs:user", "role:impl"):
            harness = FakeWatchdog.build(
                suites=0,
                comments=[_marker_comment()],
                timeline=[
                    {"event": "labeled", "created_at": "2026-07-27T22:20:00Z",
                     "label": {"name": "review:pass"}},
                    {"event": "unlabeled", "created_at": "2026-07-27T22:30:00Z",
                     "label": {"name": other}},
                    {"event": "added_to_merge_queue", "created_at": "2026-07-27T22:58:53Z"},
                ],
            )
            self.assertEqual(
                harness.watchdog.classify_dequeue(99900001, "CI_TIMEOUT").route,
                mgw.ROUTE_PRESERVE,
                other,
            )

    def test_adding_an_unrelated_label_is_not_a_verdict_end_to_end(self):
        harness = FakeWatchdog.build(
            suites=0,
            comments=[_marker_comment()],
            timeline=[
                {"event": "labeled", "created_at": "2026-07-27T22:20:00Z",
                 "label": {"name": "area:ci"}},
                {"event": "added_to_merge_queue", "created_at": "2026-07-27T22:58:53Z"},
            ],
        )
        route = harness.watchdog.classify_dequeue(99900001, "CI_TIMEOUT")
        self.assertEqual(route.route, mgw.ROUTE_DEMOTE)
        self.assertIn("no review:pass grant", route.detail)

    def test_a_label_other_than_review_pass_is_not_a_verdict(self):
        harness = FakeWatchdog.build(
            suites=0,
            comments=[_marker_comment()],
            timeline=[
                {"event": "labeled", "created_at": "2026-07-27T22:50:00Z",
                 "label": {"name": "review:changes"}},
                {"event": "added_to_merge_queue", "created_at": "2026-07-27T22:58:53Z"},
            ],
        )
        self.assertEqual(
            harness.watchdog.classify_dequeue(99900001, "CI_TIMEOUT").route, mgw.ROUTE_DEMOTE
        )

    def test_two_head_moves_straddling_the_verdict_end_to_end(self):
        # min(moved) would pick the 22:40 commit and wrongly preserve; max(moved)
        # picks the 23:05 commit and correctly forfeits the verdict.
        harness = FakeWatchdog.build(
            suites=0,
            comments=[_marker_comment()],
            timeline=[
                {"event": "committed", "committer": {"date": "2026-07-27T22:40:00Z"}},
                {"event": "labeled", "created_at": "2026-07-27T22:50:00Z",
                 "label": {"name": "review:pass"}},
                {"event": "added_to_merge_queue", "created_at": "2026-07-27T22:58:53Z"},
                {"event": "committed", "committer": {"date": "2026-07-27T23:05:00Z"}},
            ],
        )
        route = harness.watchdog.classify_dequeue(99900001, "CI_TIMEOUT")
        self.assertEqual(route.route, mgw.ROUTE_DEMOTE)
        self.assertIn("the head moved", route.detail)

    def test_two_verdict_grants_straddling_a_push_end_to_end(self):
        # min(granted) would pick the 22:20 grant, see the 22:45 push after it and
        # wrongly demote; max(granted) picks the 22:50 re-grant and preserves.
        harness = FakeWatchdog.build(
            suites=0,
            comments=[_marker_comment()],
            timeline=[
                {"event": "labeled", "created_at": "2026-07-27T22:20:00Z",
                 "label": {"name": "review:pass"}},
                {"event": "committed", "committer": {"date": "2026-07-27T22:45:00Z"}},
                {"event": "labeled", "created_at": "2026-07-27T22:50:00Z",
                 "label": {"name": "review:pass"}},
                {"event": "added_to_merge_queue", "created_at": "2026-07-27T22:58:53Z"},
            ],
        )
        self.assertEqual(
            harness.watchdog.classify_dequeue(99900001, "CI_TIMEOUT").route,
            mgw.ROUTE_PRESERVE,
        )

    def test_an_unreadable_timeline_demotes_end_to_end(self):
        harness = FakeWatchdog.build(suites=0, comments=[_marker_comment()])
        harness.gh.fail_on = ("/timeline",)
        route = harness.watchdog.classify_dequeue(99900001, "CI_TIMEOUT")
        self.assertEqual(route.route, mgw.ROUTE_DEMOTE)
        self.assertIn("timeline could not be read", route.detail)

    def test_classify_never_mutates(self):
        harness = FakeWatchdog.build(suites=0, comments=[_marker_comment()])
        harness.watchdog.classify_dequeue(99900001, "CI_TIMEOUT")
        self.assertEqual(harness.gh.mutations, [])


class TestEntrypointFailSafe(unittest.TestCase):
    """The OUTERMOST guard: main() must never let a failure preserve a verdict.

    These drive `main()` end to end because the exception handler and the GITHUB_OUTPUT
    writer are the two pieces the YAML actually consumes, and nothing else exercises
    them — a mutant that flipped the crash route to `preserve`, or that stopped writing
    `route=` at all, survived every other test in this file.
    """

    def _run_main(self, argv, gh_read):
        import os
        import tempfile

        out = Path(tempfile.mkdtemp()) / "outputs.txt"
        out.write_text("", encoding="utf-8")
        os.environ["GITHUB_OUTPUT"] = str(out)
        try:
            with _NoNetwork(gh_read):
                code = mgw.main(argv)
        finally:
            os.environ.pop("GITHUB_OUTPUT", None)
        return code, dict(
            line.split("=", 1) for line in out.read_text(encoding="utf-8").splitlines()
            if "=" in line
        )

    def test_classifier_crash_routes_to_demote(self):
        def boom(_argv):
            raise mgw.GhError("HTTP 500 while reading comments")

        code, outputs = self._run_main(
            ["classify-dequeue", "--repo", "sparq-org/sparq", "--pr", "99900001",
             "--reason", "CI_TIMEOUT"],
            boom,
        )
        self.assertEqual(code, 0)  # the step must not red the feedback job
        self.assertEqual(outputs["route"], mgw.ROUTE_DEMOTE)
        self.assertEqual(outputs["reenqueue"], "false")

    def test_transient_exhaustion_routes_to_demote(self):
        def exhausted(_argv):
            raise mgw.gh_retry.GhTransientExhausted("HTTP 504 (3 attempts)")

        code, outputs = self._run_main(
            ["classify-dequeue", "--repo", "sparq-org/sparq", "--pr", "99900001",
             "--reason", "CI_TIMEOUT"],
            exhausted,
        )
        self.assertEqual(code, 0)
        self.assertEqual(outputs["route"], mgw.ROUTE_DEMOTE)

    def test_outputs_carry_every_field_the_yaml_reads(self):
        rows: list[str] = []
        import os
        import tempfile

        out = Path(tempfile.mkdtemp()) / "outputs.txt"
        out.write_text("", encoding="utf-8")
        os.environ["GITHUB_OUTPUT"] = str(out)
        try:
            mgw.write_outputs(
                mgw.Route(mgw.ROUTE_PRESERVE, True, "zero suites"), log=rows.append
            )
        finally:
            os.environ.pop("GITHUB_OUTPUT", None)
        written = out.read_text(encoding="utf-8")
        # The `if:` expressions read `route`; the preserve step reads `reenqueue` and
        # `detail`. Dropping any of the three silently disables the feature.
        self.assertIn(f"route={mgw.ROUTE_PRESERVE}\n", written)
        self.assertIn("reenqueue=true\n", written)
        self.assertIn("detail<<MGW_DETAIL_EOF\nzero suites\nMGW_DETAIL_EOF\n", written)
        self.assertTrue(any("route=preserve" in row for row in rows), rows)

    def test_demote_outputs_are_lowercase_false(self):
        import os
        import tempfile

        out = Path(tempfile.mkdtemp()) / "outputs.txt"
        out.write_text("", encoding="utf-8")
        os.environ["GITHUB_OUTPUT"] = str(out)
        try:
            mgw.write_outputs(mgw.Route(mgw.ROUTE_DEMOTE, False, "no marker"),
                              log=lambda _m: None)
        finally:
            os.environ.pop("GITHUB_OUTPUT", None)
        self.assertIn("reenqueue=false\n", out.read_text(encoding="utf-8"))


# ── 3. wiring (the YAML seam) ────────────────────────────────────────────────────


def _yaml(path: Path) -> dict:
    return yaml.safe_load(path.read_text(encoding="utf-8"))


def _on_block(wf: dict) -> dict:
    """Bare `on:` is the YAML boolean True, so PyYAML keys it under True."""
    return wf.get("on", wf.get(True, {})) or {}


def _steps(wf: dict, job: str) -> list[dict]:
    return wf["jobs"][job]["steps"]


def _step(wf: dict, job: str, needle: str) -> dict:
    for step in _steps(wf, job):
        if needle in (step.get("name") or "") or needle == step.get("id"):
            return step
    raise AssertionError(f"no step matching {needle!r} in job {job}")


class TestSurvivorsFromTheCoverageCensus(unittest.TestCase):
    """Every one of these lived in a function the module self-test never executed.

    The lesson recorded with them: run the coverage census FIRST and treat a 0%-covered
    entry point as blocking, because that is exactly where the surviving mutants are.
    """

    def test_sweep_exit_code_is_nonzero_when_a_recovery_fails(self):
        # THE EXIT-ZERO CLASS. `return 1 if watchdog.sweep() else 0` -> `return 0`
        # survived the whole suite: a later transient would discard an earned hard
        # failure and the run would report success. This has bitten this estate before.
        harness = FakeWatchdog.build(suites=0)
        harness.gh.fail_on = ("dequeuePullRequest",)
        with _NoNetwork(harness.gh):
            code = mgw.main(["sweep", "--repo", "sparq-org/sparq"])
        self.assertEqual(code, 1)

    def test_sweep_exit_code_is_zero_on_a_clean_sweep(self):
        # The paired control: a healthy queue must NOT red the scheduled run.
        harness = FakeWatchdog.build(suites=8)
        with _NoNetwork(harness.gh):
            code = mgw.main(["sweep", "--repo", "sparq-org/sparq"])
        self.assertEqual(code, 0)

    def test_a_failed_recovery_appears_in_the_census(self):
        harness = FakeWatchdog.build(suites=0)
        harness.gh.fail_on = ("dequeuePullRequest",)
        harness.run()
        census = next(r for r in harness.rows if "sweep complete" in r)
        self.assertIn("RECOVER_FAILED=1", census)
        self.assertIn("errors=1", census)

    def test_stacked_green_counts_only_mergeable_entries(self):
        # MIXED fixture. Every stacked entry in the original fixture was MERGEABLE, so
        # `stacked_green = list(stacked)` was value-identical and survived.
        nodes = [
            {"id": "MQE_1", "position": 1, "state": "AWAITING_CHECKS",
             "enqueuedAt": "2026-07-27T22:58:00Z",
             "baseCommit": {"oid": BASE}, "headCommit": {"oid": HEAD},
             "pullRequest": {"number": 99900004, "id": "PR_99900004"}},
            {"id": "MQE_2", "position": 2, "state": "MERGEABLE",
             "enqueuedAt": "2026-07-27T22:59:00Z",
             "baseCommit": {"oid": HEAD}, "headCommit": {"oid": "a" * 40},
             "pullRequest": {"number": 99900005, "id": "PR_99900005"}},
            {"id": "MQE_3", "position": 3, "state": "AWAITING_CHECKS",
             "enqueuedAt": "2026-07-27T23:00:00Z",
             "baseCommit": {"oid": "a" * 40}, "headCommit": {"oid": "b" * 40},
             "pullRequest": {"number": 99900006, "id": "PR_99900006"}},
            {"id": "MQE_4", "position": 4, "state": "UNMERGEABLE",
             "enqueuedAt": "2026-07-27T23:01:00Z",
             "baseCommit": {"oid": "b" * 40}, "headCommit": {"oid": "c" * 40},
             "pullRequest": {"number": 99900007, "id": "PR_99900007"}},
        ]
        harness = FakeWatchdog.build(suites=0, entries=nodes)
        harness.run()
        head_row = next(r for r in harness.rows if "pr=#99900004" in r)
        # three entries behind it have groups, but only ONE is green
        self.assertIn("stacked=3", head_row)
        self.assertIn("stacked_green=1", head_row)

    def test_the_queue_query_asks_for_the_whole_queue(self):
        # QUEUE_ENTRY_PAGE 20 -> 1 survived: nothing asserted the page size, so a
        # single-entry read would silently miss every entry behind the head.
        harness = FakeWatchdog.build(suites=8)
        harness.run()
        call = next(
            c for c in harness.gh.calls
            if c[:2] == ["api", "graphql"] and any("first=" in a for a in c)
        )
        first = next(a for a in call if a.startswith("first="))
        self.assertEqual(first, f"first={mgw.QUEUE_ENTRY_PAGE}")
        self.assertGreaterEqual(mgw.QUEUE_ENTRY_PAGE, 20)

    def test_multi_line_detail_cannot_inject_extra_outputs(self):
        import os
        import tempfile
        out = Path(tempfile.mkdtemp()) / "outputs.txt"
        out.write_text("", encoding="utf-8")
        os.environ["GITHUB_OUTPUT"] = str(out)
        try:
            mgw.write_outputs(
                mgw.Route(mgw.ROUTE_DEMOTE, False, "gh failed:\nroute=preserve\nreenqueue=true"),
                log=lambda _m: None,
            )
        finally:
            os.environ.pop("GITHUB_OUTPUT", None)
        written = out.read_text(encoding="utf-8")
        # The injected `route=preserve` must be INSIDE the delimited block, not a key.
        self.assertTrue(written.startswith("route=demote\n"), written)
        self.assertIn("detail<<MGW_DETAIL_EOF\n", written)
        body = written.split("detail<<MGW_DETAIL_EOF\n", 1)[1]
        self.assertTrue(body.endswith("MGW_DETAIL_EOF\n"), body)


class TestFixturesNameNothingReal(unittest.TestCase):
    """Fixtures must name ids that CANNOT resolve.

    Reproducing `N01` necessarily reverts the hermeticity fix — the guard and the
    mutant are the same switch — so for that one mutant the in-process poison is off by
    design and only the fixture's own id stands between the suite and a live object.
    That is how 567 comments reached PR #4534. The reserved band below cannot resolve,
    so the same mistake now writes nowhere.
    """

    RESERVED_MIN = 99900000

    def test_no_fixture_names_a_plausibly_real_pr(self):
        # Scope: ids that can become an API TARGET. Comments are stripped, and
        # `queue_ref(...)` is exempt because it is a PURE function — its argument is a
        # recorded live-verified provenance datum (the 2026-07-28 ref-format check),
        # never a request. The distinction that matters is write-reachability, not the
        # digits themselves.
        lines = [
            ln.split("#", 1)[0]
            for ln in Path(__file__).read_text(encoding="utf-8").splitlines()
            if "queue_ref(" not in ln
        ]
        found = {
            int(n)
            for n in re.findall(
                r'(?:pr=|"number": |pr_number=|classify_dequeue\(|pr_markers\()(\d{3,})',
                "\n".join(lines),
            )
        }
        real = sorted(n for n in found if n < self.RESERVED_MIN)
        self.assertEqual(real, [], f"fixtures name plausibly-real ids: {real}")

    def test_the_reserved_band_is_actually_reserved(self):
        # sparq is nowhere near 99.9M issues; if it ever were, this test fails FIRST.
        self.assertGreater(self.RESERVED_MIN, 10_000_000)


class TestTheSuiteCannotTouchTheNetwork(unittest.TestCase):
    """Guard the guard: the poison above is the only thing standing between this
    suite and production, so its absence must itself be a test failure."""

    def test_the_module_runners_are_poisoned(self):
        self.assertIs(mgw.run_gh, _forbidden_gh)
        self.assertIs(mgw.run_gh_read, _forbidden_gh)

    def test_an_uninjected_watchdog_cannot_reach_the_network(self):
        # The exact defect: a Watchdog built WITHOUT an injected runner must now fail
        # loudly rather than silently using the real one.
        watchdog = mgw.Watchdog("sparq-org/sparq")
        with self.assertRaises(RealGhCallAttempted):
            watchdog.queue_entries()

    def test_the_runner_default_is_late_bound(self):
        # If the default were bound at definition time this would still be the real
        # runner, which is precisely how 567 comments reached a live PR.
        self.assertIs(mgw.Watchdog("sparq-org/sparq").gh, _forbidden_gh)

    def test_main_sweep_cannot_reach_the_network_uninjected(self):
        with self.assertRaises(RealGhCallAttempted):
            mgw.main(["sweep", "--repo", "sparq-org/sparq"])


class TestEntrypointExitContract(unittest.TestCase):
    """Every exit path of `main`, because that is where the exit-zero class hides.

    The coverage census put `main` at 44% under the module self-test with its whole
    sweep branch unexecuted — precisely where the surviving exit-code mutants lived.
    """

    def _sweep(self, gh):
        with _NoNetwork(gh):
            return mgw.main(["sweep", "--repo", "sparq-org/sparq"])

    def test_exhausted_transient_is_a_warning_and_exit_zero(self):
        # A periodic idempotent sweep may skip a cycle on a platform 5xx — the next
        # tick covers it — but it must SAY so rather than exit quietly.
        def exhausted(_argv):
            raise mgw.gh_retry.GhTransientExhausted("HTTP 504 (3 attempts)")

        import io
        from contextlib import redirect_stdout
        out = io.StringIO()
        with redirect_stdout(out):
            code = self._sweep(exhausted)
        self.assertEqual(code, 0)
        self.assertIn("::warning", out.getvalue())
        self.assertIn("skipped a cycle", out.getvalue())

    def test_a_non_transient_gh_failure_exits_nonzero(self):
        def broken(_argv):
            raise mgw.GhError("HTTP 401 Bad credentials")

        import io
        from contextlib import redirect_stderr
        err = io.StringIO()
        with redirect_stderr(err):
            code = self._sweep(broken)
        self.assertEqual(code, 1)
        self.assertIn("fatal", err.getvalue())

    def test_a_malformed_repo_argument_exits_nonzero(self):
        import io
        from contextlib import redirect_stderr
        err = io.StringIO()
        with redirect_stderr(err):
            code = mgw.main(["sweep", "--repo", "not-a-repo"])
        self.assertEqual(code, 1)

    def test_no_subcommand_is_an_error_not_a_silent_success(self):
        with self.assertRaises(SystemExit) as caught:
            mgw.main([])
        self.assertNotEqual(caught.exception.code, 0)

    def test_grace_below_the_floor_is_refused(self):
        with self.assertRaises(ValueError):
            mgw.Watchdog("sparq-org/sparq", grace_seconds=30)


class TestTelemetryPlumbing(unittest.TestCase):
    def test_rows_are_appended_to_the_job_summary(self):
        import os
        import tempfile
        summary = Path(tempfile.mkdtemp()) / "summary.md"
        # Pre-create it, so a watchdog that writes NOTHING fails on the assertion below
        # rather than on a FileNotFoundError raised by this test's own read. A kill
        # should name the behaviour that changed, not crash in the harness.
        summary.write_text("", encoding="utf-8")
        os.environ["GITHUB_STEP_SUMMARY"] = str(summary)
        try:
            harness = FakeWatchdog.build(suites=8)
            harness.run()
        finally:
            os.environ.pop("GITHUB_STEP_SUMMARY", None)
        written = summary.read_text(encoding="utf-8")
        self.assertIn("decision=HOLD", written)
        self.assertIn("sweep complete", written)

    def test_an_unwritable_job_summary_never_breaks_the_sweep(self):
        import os
        os.environ["GITHUB_STEP_SUMMARY"] = "/proc/definitely/not/writable"
        try:
            harness = FakeWatchdog.build(suites=8)
            self.assertEqual(harness.run(), 0)
        finally:
            os.environ.pop("GITHUB_STEP_SUMMARY", None)
        self.assertTrue(any("decision=HOLD" in r for r in harness.rows))

    def test_a_garbled_page_line_is_a_hard_error_not_an_empty_list(self):
        # _ndjson must never silently yield [] on malformed input: an empty marker list
        # reads as "no prior recovery" and would bypass the cap.
        with self.assertRaises(mgw.GhError):
            mgw._ndjson("{not json}")
        self.assertEqual(mgw._ndjson(""), [])
        self.assertEqual(mgw._ndjson("  \n\n"), [])
        # non-dict JSON lines are ignored rather than crashing
        self.assertEqual(mgw._ndjson("[1,2]"), [])

    def test_a_detail_containing_the_delimiter_cannot_break_out(self):
        # The payload embeds gh stderr. If it contained the delimiter itself, the
        # heredoc would terminate early and the remainder would be parsed as keys —
        # the same output-injection the delimiter exists to prevent.
        import os
        import tempfile
        out = Path(tempfile.mkdtemp()) / "outputs.txt"
        out.write_text("", encoding="utf-8")
        os.environ["GITHUB_OUTPUT"] = str(out)
        try:
            mgw.write_outputs(
                mgw.Route(
                    mgw.ROUTE_DEMOTE,
                    False,
                    "gh said:\nMGW_DETAIL_EOF\nroute=preserve\nreenqueue=true",
                ),
                log=lambda _m: None,
            )
        finally:
            os.environ.pop("GITHUB_OUTPUT", None)
        written = out.read_text(encoding="utf-8")
        lines = written.splitlines()
        # THE SAFETY PROPERTY: the heredoc is well-formed — the delimiter appears
        # exactly twice (open and close) and no line INSIDE the block equals it. A line
        # inside the block is inert no matter what it says; a premature close is what
        # would turn `route=preserve` into a real key.
        opener = lines.index("detail<<MGW_DETAIL_EOF")
        body = lines[opener + 1:]
        self.assertEqual(body[-1], "MGW_DETAIL_EOF", written)
        self.assertNotIn("MGW_DETAIL_EOF", body[:-1], written)
        # the attacker's delimiter was neutralised rather than passed through
        self.assertIn("MGW-DETAIL-EOF", written)
        # and the real outputs were emitted before the block, unaffected
        self.assertEqual(lines[0], "route=demote", written)
        self.assertEqual(lines[1], "reenqueue=false", written)

    def test_run_gh_read_translates_a_fatal_into_the_local_error_type(self):
        # Three lines, but the translation is real logic: a GhFatalError that escaped
        # untranslated would bypass every `except GhError` handler and crash the sweep
        # instead of producing a REFUSE row.
        original = mgw.gh_retry.run_gh_read

        def fatal(_argv):
            raise mgw.gh_retry.GhFatalError("HTTP 404 Not Found")

        try:
            mgw.gh_retry.run_gh_read = fatal
            with self.assertRaises(mgw.GhError):
                _REAL_RUN_GH_READ(["api", "-X", "GET", "repos/x/y"])
        finally:
            mgw.gh_retry.run_gh_read = original

    def test_run_gh_read_lets_transient_exhaustion_through_untouched(self):
        # Exhaustion must stay its own type so the sweep entrypoint can convert exactly
        # that case into ::warning + exit 0 and nothing else.
        original = mgw.gh_retry.run_gh_read

        def exhausted(_argv):
            raise mgw.gh_retry.GhTransientExhausted("HTTP 504 (3 attempts)")

        try:
            mgw.gh_retry.run_gh_read = exhausted
            with self.assertRaises(mgw.gh_retry.GhTransientExhausted):
                _REAL_RUN_GH_READ(["api", "-X", "GET", "repos/x/y"])
        finally:
            mgw.gh_retry.run_gh_read = original

    def test_an_unreadable_activity_feed_yields_no_anchor(self):
        # group_created_at's error branch: no anchor => REFUSE, never a recovery.
        harness = FakeWatchdog.build(suites=0)
        harness.gh.fail_on = ("/activity",)
        harness.run()
        self.assertEqual(harness.gh.mutations, [])
        self.assertTrue(any("decision=REFUSE" in r for r in harness.rows), harness.rows)

    def test_a_malformed_activity_row_yields_no_anchor(self):
        harness = FakeWatchdog.build(
            suites=0,
            activity=[{"activity_type": "branch_creation", "after": HEAD,
                       "timestamp": "not-a-timestamp"}],
        )
        harness.run()
        self.assertEqual(harness.gh.mutations, [])
        self.assertTrue(any("decision=REFUSE" in r for r in harness.rows), harness.rows)

    def test_parse_iso_rejects_junk_rather_than_guessing(self):
        for junk in (None, "", "not-a-date", "2026-13-45T99:99:99Z", 17):
            self.assertIsNone(mgw.parse_iso(junk), junk)
        self.assertIsNotNone(mgw.parse_iso("2026-07-28T05:15:09Z"))
        # a naive timestamp is treated as UTC rather than crashing
        naive = mgw.parse_iso("2026-07-28T05:15:09")
        self.assertIsNotNone(naive)
        self.assertEqual(naive.tzinfo, timezone.utc)


class TestFeedbackWiring(unittest.TestCase):
    """Mutate the `if:`, the step, or the call site and one of these goes red."""

    def setUp(self):
        self.wf = _yaml(FEEDBACK_YML)
        self.steps = _steps(self.wf, "feedback")
        self.names = [s.get("name") or s.get("uses") for s in self.steps]

    def test_classify_step_exists_with_the_id_the_guards_reference(self):
        ids = [s.get("id") for s in self.steps]
        self.assertIn("classify", ids, self.names)

    def test_classify_call_site_invokes_the_classifier_with_pr_and_reason(self):
        run = _step(self.wf, "feedback", "classify")["run"]
        self.assertIn("scripts/merge-group-watchdog.py", run)
        self.assertIn("classify-dequeue", run)
        self.assertIn("--pr", run)
        self.assertIn("--reason", run)
        self.assertIn("$PR_NUMBER", run)
        self.assertIn("$DEQUEUE_REASON", run)

    # EXACT-MATCH, not substring. A substring assertion cannot see a guard that is
    # still present but CONDITIONALLY INERT: `… route != 'preserve' && false` contains
    # every token a substring test looks for while making the step dead, so the fix
    # lane would silently stop being fed. Both guards are therefore pinned whole; a
    # deliberate change must update this line.
    DEMOTE_GUARD = (
        "!cancelled() && github.event.pull_request.merged != true "
        "&& steps.classify.outputs.route != 'preserve'"
    )
    PRESERVE_GUARD = (
        "github.event.pull_request.merged != true "
        "&& steps.classify.outputs.route == 'preserve'"
    )

    def test_the_demote_guard_is_fail_closed(self):
        # `!= 'preserve'` covers an EMPTY/MISSING output. `!cancelled()` covers the
        # OTHER failure mode the string test could not see: a step `if:` with no status
        # function defaults to `success()`, so a FAILED or TIMED-OUT classify step would
        # SKIP the demote — keeping review:pass and leaving the arm enabled.
        guard = _step(self.wf, "feedback", "Route genuine dequeue")["if"]
        self.assertEqual(guard, self.DEMOTE_GUARD)
        self.assertIn("!cancelled()", guard)

    def test_every_guard_that_must_survive_a_failed_step_says_so(self):
        # Structural, not string-matching: any step whose condition consumes a previous
        # step's OUTPUT and is meant to run regardless must carry a status function,
        # because the implicit default is success().
        demote = _step(self.wf, "feedback", "Route genuine dequeue")["if"]
        self.assertRegex(demote, r"!cancelled\(\)|always\(\)")
        # ...and the preserve route must NOT, since it may only act on a positive
        # verdict from a step that actually succeeded.
        preserve = _step(self.wf, "feedback", "Preserve the verdict")["if"]
        self.assertNotRegex(preserve, r"!cancelled\(\)|always\(\)")

    def test_the_preserve_guard_is_fail_open_in_the_safe_direction(self):
        guard = _step(self.wf, "feedback", "Preserve the verdict")["if"]
        self.assertEqual(guard, self.PRESERVE_GUARD)

    def test_no_step_guard_carries_a_constant_disjunct(self):
        # The conditionally-inert class in general: a literal `true`/`false` operand
        # anywhere in a step condition makes that step unconditionally on or off while
        # still reading as a guard.
        for step in self.steps:
            guard = step.get("if")
            if not guard:
                continue
            # `merged != true` is a legitimate COMPARISON against a literal; strip
            # every such comparison first, then any surviving bare literal is a
            # constant operand of && / || — i.e. an always-on or always-off step.
            residue = re.sub(r"[=!]=\s*(?:true|false)\b", "", guard)
            self.assertIsNone(
                re.search(r"\b(?:true|false)\b", residue),
                (step.get("name"), guard),
            )

    def test_the_route_literal_is_the_same_string_on_both_sides(self):
        # CROSS-FILE CONTRACT. The Python constant and the YAML `if:` literal are two
        # halves of one comparison; renaming or case-drifting either side silently
        # makes the preserve route unreachable (and, worse, fails SILENTLY GREEN
        # because the fail-safe `!= 'preserve'` then always demotes).
        raw = FEEDBACK_YML.read_text(encoding="utf-8")
        self.assertIn(f"steps.classify.outputs.route == '{mgw.ROUTE_PRESERVE}'", raw)
        self.assertIn(f"steps.classify.outputs.route != '{mgw.ROUTE_PRESERVE}'", raw)

    def test_the_two_routes_are_mutually_exclusive(self):
        demote = _step(self.wf, "feedback", "Route genuine dequeue")["if"]
        preserve = _step(self.wf, "feedback", "Preserve the verdict")["if"]
        self.assertNotEqual(demote, preserve)
        self.assertIn("route != 'preserve'", demote)
        self.assertIn("route == 'preserve'", preserve)

    def test_preserve_route_never_demotes_the_label(self):
        # Preserving the verdict IS leaving the labels alone: the route must issue no
        # label mutation and must not kill the arm. (`review:changes` appears in this
        # step's explanatory COMMENT prose, so match on the gh flags, not the token.)
        run = _step(self.wf, "feedback", "Preserve the verdict")["run"]
        self.assertNotIn("--add-label", run)
        self.assertNotIn("--remove-label", run)
        self.assertNotIn("--disable-auto", run)
        self.assertNotIn("gh pr edit", run)

    def test_preserve_route_re_arms_only_when_the_classifier_says_so(self):
        # When the watchdog itself is mid-recovery (reenqueue=false) a second arm here
        # would race its enqueue; when the platform timed the entry out (reenqueue=true)
        # something must put it back. Both directions must be wired.
        run = _step(self.wf, "feedback", "Preserve the verdict")["run"]
        self.assertIn('"$CLASSIFY_REENQUEUE" = "true"', run)
        self.assertIn("gh pr merge", run)
        self.assertIn("--auto", run)
        env = _step(self.wf, "feedback", "Preserve the verdict")["env"]
        self.assertEqual(env["CLASSIFY_REENQUEUE"], "${{ steps.classify.outputs.reenqueue }}")

    def test_preserve_route_checks_the_pr_is_still_open(self):
        run = _step(self.wf, "feedback", "Preserve the verdict")["run"]
        self.assertIn("gh pr view", run)
        self.assertIn("--json state", run)
        # SHAPE, not substring presence: the guard must compare against OPEN and the
        # non-open branch must EXIT, before anything mutating runs. A mutant that left
        # the strings in place while changing what happens survived a presence test.
        lines = [ln.strip() for ln in run.splitlines()]
        idx = next(i for i, ln in enumerate(lines) if '"$STATE" != "OPEN"' in ln)
        branch = lines[idx : idx + 4]
        self.assertTrue(any(ln == "exit 0" for ln in branch), branch)
        # ...and it happens before the re-arm / comment, never after.
        first_mutation = next(
            i for i, ln in enumerate(lines) if ln.startswith("gh pr merge") or ln.startswith("gh pr comment")
        )
        self.assertLess(idx, first_mutation, lines)

    def test_every_post_skip_step_carries_the_merged_guard(self):
        # The first step short-circuits a successful-merge dequeue; every later step
        # must independently re-assert it, because a step `if:` is not inherited.
        for step in self.steps:
            name = step.get("name") or step.get("uses") or ""
            if "Skip successful-merge" in name:
                continue
            self.assertIn(
                "github.event.pull_request.merged != true", step.get("if", ""), name
            )

    def test_demote_route_still_swaps_the_labels(self):
        # The pre-#4652 behaviour must survive intact on the other arm.
        run = _step(self.wf, "feedback", "Route genuine dequeue")["run"]
        self.assertIn('--add-label "review:changes"', run)
        self.assertIn('--remove-label "review:pass"', run)
        self.assertIn("--disable-auto", run)

    def test_checkout_is_pinned_to_the_default_branch_never_the_pr_head(self):
        step = _step(self.wf, "feedback", "Checkout the dequeue classifier")
        self.assertTrue(str(step["uses"]).startswith("actions/checkout@"))
        self.assertRegex(str(step["uses"]), r"@[0-9a-f]{40}$")
        self.assertEqual(step["with"]["ref"], "${{ github.event.repository.default_branch }}")
        self.assertIs(step["with"]["persist-credentials"], False)
        # The pull_request_target trap: never the PR head.
        rendered = yaml.safe_dump(step)
        for forbidden in ("pull_request.head", "github.head_ref", "merge_commit_sha"):
            self.assertNotIn(forbidden, rendered)

    def test_permissions_add_reads_only(self):
        perms = self.wf["permissions"]
        self.assertEqual(perms.get("checks"), "read")
        self.assertEqual(perms.get("contents"), "read")
        self.assertEqual(perms.get("pull-requests"), "write")

    def test_trigger_is_unchanged(self):
        on = _on_block(self.wf)
        self.assertEqual(list(on), ["pull_request_target"])
        self.assertEqual(on["pull_request_target"]["types"], ["dequeued"])

    def test_step_order_classify_before_both_routes(self):
        order = {name: i for i, name in enumerate(self.names)}
        classify = next(i for n, i in order.items() if n and "Classify the dequeue" in n)
        preserve = next(i for n, i in order.items() if n and "Preserve the verdict" in n)
        demote = next(i for n, i in order.items() if n and "Route genuine dequeue" in n)
        checkout = next(i for n, i in order.items() if n and "Checkout the dequeue" in n)
        self.assertLess(checkout, classify)
        self.assertLess(classify, preserve)
        self.assertLess(classify, demote)


class TestWatchdogWorkflowWiring(unittest.TestCase):
    def setUp(self):
        self.wf = _yaml(WATCHDOG_YML)

    def test_is_schedule_only_and_can_never_gate(self):
        on = _on_block(self.wf)
        self.assertIn("schedule", on)
        self.assertIn("workflow_dispatch", on)
        # A PR-head trigger would make this sweep a required check on every PR.
        for gating in ("pull_request", "pull_request_target", "merge_group", "push"):
            self.assertNotIn(gating, on, gating)

    def test_cron_is_five_minutely(self):
        minutes = self.wf["on" if "on" in self.wf else True]["schedule"][0]["cron"].split()[0]
        values = sorted(int(m) for m in minutes.split(","))
        self.assertEqual(len(values), 12, values)
        gaps = {b - a for a, b in zip(values, values[1:])}
        self.assertEqual(gaps, {5}, values)

    def test_sweep_call_site_invokes_the_sweep_subcommand(self):
        run = _step(self.wf, "watch", "Sweep the merge queue")["run"]
        self.assertIn("scripts/merge-group-watchdog.py", run)
        self.assertIn("sweep", run)
        self.assertIn("--repo", run)
        self.assertIn("--branch", run)

    def test_self_test_runs_before_the_live_sweep(self):
        names = [s.get("name") or s.get("uses") for s in _steps(self.wf, "watch")]
        self.assertLess(
            next(i for i, n in enumerate(names) if n and "Self-test" in n),
            next(i for i, n in enumerate(names) if n and "Sweep the merge queue" in n),
        )
        run = _step(self.wf, "watch", "Self-test policy")["run"]
        self.assertIn("merge-group-watchdog.py --self-test", run)
        self.assertIn("gh_retry.py --self-test", run)

    def test_checkout_is_default_branch_and_sparse_covers_both_scripts(self):
        step = _step(self.wf, "watch", "Checkout watchdog policy")
        self.assertEqual(step["with"]["ref"], "${{ github.event.repository.default_branch }}")
        self.assertIs(step["with"]["persist-credentials"], False)
        sparse = step["with"]["sparse-checkout"]
        self.assertIn("scripts/merge-group-watchdog.py", sparse)
        self.assertIn("scripts/gh_retry.py", sparse)

    def test_permissions_are_least_privilege(self):
        perms = self.wf["permissions"]
        self.assertEqual(perms.get("contents"), "read")
        self.assertEqual(perms.get("checks"), "read")
        self.assertEqual(perms.get("pull-requests"), "write")

    def test_actions_are_sha_pinned(self):
        # PyYAML drops `#` comments, so the 40-hex pin is asserted on the parsed value
        # and the readable `# vX.Y.Z` tag on the raw text.
        raw = WATCHDOG_YML.read_text(encoding="utf-8")
        seen = 0
        for step in _steps(self.wf, "watch"):
            uses = step.get("uses")
            if not uses:
                continue
            seen += 1
            self.assertRegex(uses, r"^[\w.-]+/[\w.-]+@[0-9a-f]{40}$", uses)
            self.assertRegex(raw, re.escape(f"uses: {uses} # v"), uses)
        self.assertGreater(seen, 0)

    def test_serialised_so_two_ticks_cannot_double_recover(self):
        concurrency = self.wf["concurrency"]
        self.assertEqual(concurrency["group"], "merge-group-watchdog")
        self.assertIs(concurrency["cancel-in-progress"], False)

    def test_every_watchdog_job_is_declared_non_gating(self):
        # A scheduled/manual run lands on the main head SHA that the push-triggered
        # ci-summary polls. The registry — not the absence of an advisory name token —
        # is the only mechanism that prevents queue-infrastructure failures from
        # gating an unrelated commit (#6269).
        registry = json.loads(
            (REPO_ROOT / ".github" / "advisory-registry.json").read_text(encoding="utf-8")
        )["jobs"]
        declared = {
            (entry["workflow"], entry["job_id"]): name
            for name, entry in registry.items()
        }
        for job_id, job in self.wf["jobs"].items():
            name = job["name"]
            self.assertIsNone(re.search(r"\b(advisory|informational)\b", name), name)
            self.assertEqual(
                declared.get(("merge-group-watchdog.yml", job_id)),
                name,
                f"watchdog job {job_id!r} can silently gate main unless declared",
            )


class TestDoorbellIsNotSubscribedToTheEventItInspects(unittest.TestCase):
    """THE HEADLINE GUARD. The watchdog's entire predicate is `zero check-suites on the
    group head`. On this repo a check-suite is created 1:1 per dispatched `merge_group`
    workflow RUN (MEASURED: group head 6b291b28 -> 8 runs / 8 suites; same 8/8 on
    37aac5a4, e5a9d403, bef48bee). So ANY workflow that participates in the zero-dispatch
    sweep and ALSO carries a `merge_group:` trigger would put an extra suite on every
    group head and make `total_count == 0` unsatisfiable — the detector would be masked
    completely, silently, and forever.

    This is asserted over the whole workflow directory rather than over the two files
    this PR happens to add, because the failure mode is someone LATER adding
    `merge_group:` to the doorbell "so it fires on the group too" — which reads like an
    improvement and is the exact thing that destroys the signal."""

    #: A step PARTICIPATES in the live sweep if it either performs it or rings it.
    #  Deliberately narrow. Two neighbours must NOT be swept up:
    #    * docs-quality.yml runs `merge-group-watchdog.py --self-test`, a hermetic
    #      decision-table test that touches no queue and reads no group head; and
    #    * merge-queue-feedback.yml runs `classify-dequeue`, which reads a suite count
    #      for an already-recorded SHA on the PR-dequeue path.
    #  Both are fine on a `merge_group` trigger, because their suite exists only when
    #  dispatch WORKED — that is the baseline the predicate measures against, not
    #  interference with it. The masking bug is specific to the DETECTOR being
    #  dispatched by the very event whose non-delivery it detects.
    SCRIPT_RE = re.compile(r"(?<![\w-])merge-group-watchdog\.py(?![\w.-])")
    SUBCOMMAND_RE = re.compile(r"(?<![\w-])sweep(?![\w-])")
    RINGS_RE = re.compile(r"gh\s+workflow\s+run\s+merge-group-watchdog\.yml")

    @classmethod
    def _performs(cls, body: str) -> bool:
        """The script AND the `sweep` subcommand, anywhere in the SAME step body.

        Not a lookahead: the real call site builds `sweep_args=(sweep --repo ...)`
        BEFORE invoking the script, so a forward-only match would silently classify the
        live sweep as a non-participant — the vacuity this class exists to prevent.
        """
        return bool(cls.SCRIPT_RE.search(body) and cls.SUBCOMMAND_RE.search(body))

    def _participants(self):
        found = {}
        for path in sorted((REPO_ROOT / ".github" / "workflows").glob("*.yml")):
            wf = yaml.safe_load(path.read_text(encoding="utf-8"))
            if not isinstance(wf, dict):
                continue
            # Per STEP, never per file: joining a whole workflow's steps would let an
            # unrelated step contribute the word `sweep` to a self-test invocation.
            # Only an EXECUTABLE reference counts, so a workflow that merely names the
            # watchdog in a comment is not a participant.
            for job in (wf.get("jobs") or {}).values():
                if not isinstance(job, dict):
                    continue
                for step in (job.get("steps") or []):
                    if not isinstance(step, dict):
                        continue
                    body = str(step.get("run", ""))
                    if self._performs(body) or self.RINGS_RE.search(body):
                        found[path.name] = wf
        return found

    def test_the_participant_set_is_the_one_we_think_it_is(self):
        # Guards the guard: if this set silently empties or narrows, every assertion
        # below passes vacuously. An instrument that can only report "nothing to see"
        # is no instrument — so the population is pinned exactly.
        self.assertEqual(
            set(self._participants()),
            {"merge-group-watchdog.yml", "merge-group-doorbell.yml"},
        )

    def test_the_matcher_separates_a_live_sweep_from_a_hermetic_self_test(self):
        # Validates the instrument against a KNOWN POSITIVE and a KNOWN NEGATIVE, so a
        # regex that silently stopped matching anything cannot look like a clean run.
        self.assertTrue(self._performs(
            'sweep_args=(sweep --repo "$REPO")\n'
            'python3 scripts/merge-group-watchdog.py "${sweep_args[@]}"'))
        self.assertFalse(self._performs(
            "python3 scripts/merge-group-watchdog.py --self-test"))
        self.assertFalse(self._performs(
            "python3 scripts/tests/test_merge_group_watchdog.py"))
        # ...and the ORDER-INDEPENDENCE that the first lookahead attempt got wrong.
        self.assertTrue(self._performs(
            'python3 scripts/merge-group-watchdog.py sweep --repo "$REPO"'))
        self.assertTrue(self.RINGS_RE.search(
            'gh workflow run merge-group-watchdog.yml --repo "$REPO"'))

    def test_no_sweep_participant_subscribes_to_merge_group(self):
        for name, wf in self._participants().items():
            self.assertNotIn(
                "merge_group", _on_block(wf),
                f"{name} subscribes to `merge_group`. A workflow that takes part in the "
                f"zero-dispatch sweep must never create a check-suite on the group head "
                f"it inspects — doing so masks the `total_count == 0` predicate on EVERY "
                f"group, not just its own.",
            )

    def test_the_doorbell_producer_is_the_pr_level_queue_transition_pair(self):
        # EXACT list, not a containment check: adding `merge_group` here is the masking
        # bug, and adding `labeled` (223 events in 4 measured hours, versus 19 queue
        # transitions) is the cost bug. Either must fail.
        on = _on_block(_yaml(DOORBELL_YML))
        self.assertEqual(on["pull_request_target"]["types"], ["enqueued", "dequeued"])

    def test_the_doorbell_creates_no_check_suite_on_a_group_head(self):
        # The structural reason constraint (a) holds: every trigger this workflow
        # carries attaches its check-run to a PR head or to nothing at all. `push` is
        # included in the ban because a merged group head BECOMES the main head.
        on = _on_block(_yaml(DOORBELL_YML))
        self.assertEqual(set(on), {"pull_request_target", "workflow_dispatch"})

    def test_the_watchdog_itself_still_has_no_pr_or_group_trigger(self):
        # The two-stage split exists so the sweep keeps running unassociated with any
        # PR. Collapsing the doorbell INTO the watchdog would give the sweep a PR-head
        # check-run and make a 4-minute queue watchdog visible on every enqueue.
        on = _on_block(_yaml(WATCHDOG_YML))
        self.assertEqual(set(on), {"schedule", "workflow_dispatch"})


class TestDoorbellWiring(unittest.TestCase):
    def setUp(self):
        self.wf = _yaml(DOORBELL_YML)
        self.job = self.wf["jobs"]["doorbell"]
        self.steps = self.job["steps"]

    def test_the_cron_backstop_is_retained(self):
        # Two independent paths is the point. A future edit that deletes the schedule
        # "because the doorbell covers it" removes the only cover for a queue rebase
        # that produces no PR-level queue event (a direct push to main).
        on = _on_block(_yaml(WATCHDOG_YML))
        self.assertIn("schedule", on)
        minutes = on["schedule"][0]["cron"].split()[0]
        self.assertEqual(len(minutes.split(",")), 12)

    def test_the_ring_step_dispatches_the_watchdog_on_the_default_branch(self):
        self.assertEqual(len(self.steps), 1, "the doorbell must stay a one-step job")
        step = self.steps[0]
        run = step["run"]
        self.assertIn("gh workflow run merge-group-watchdog.yml", run)
        self.assertIn('--ref "$WATCHDOG_REF"', run)
        self.assertEqual(
            step["env"]["WATCHDOG_REF"],
            "${{ github.event.repository.default_branch }}",
            "the dispatched policy must be the reviewed copy on the default branch, "
            "never a PR's version of the watchdog",
        )

    def test_the_watchdog_is_actually_dispatchable(self):
        # Cross-file contract: `gh workflow run` fails outright unless the target
        # declares workflow_dispatch. Deleting it there would break the doorbell here.
        self.assertIn("workflow_dispatch", _on_block(_yaml(WATCHDOG_YML)))

    def test_no_checkout_and_no_repository_script_is_executed(self):
        # pull_request_target runs with the base repo's token. The mitigation is not
        # "check out carefully" — it is checking out nothing at all.
        for step in self.steps:
            self.assertNotIn("checkout", str(step.get("uses", "")).lower())
            self.assertNotRegex(str(step.get("run", "")), r"\bpython3?\b|\bbash\b|scripts/")

    def test_no_event_field_is_interpolated_into_an_executable_line(self):
        for step in self.steps:
            self.assertNotIn("${{", str(step.get("run", "")))

    def test_permissions_are_exactly_actions_write(self):
        # Not a superset check. `contents`/`pull-requests`/`checks` would let the
        # doorbell mutate the very things it exists only to ring about.
        self.assertEqual(self.wf["permissions"], {"actions": "write"})
        self.assertNotIn("permissions", self.job)

    def test_the_job_is_declared_advisory_in_the_registry(self):
        # Since #3773 the registry is the ONLY thing that makes a check-run non-gating,
        # and this leg lands on a PR head. Undeclared, a doorbell hiccup would red the
        # gate on a commit whose merge-worthiness it says nothing about.
        name = self.job["name"]
        self.assertRegex(name, r"\b(advisory|informational)\b")
        registry = json.loads(
            (REPO_ROOT / ".github" / "advisory-registry.json").read_text(encoding="utf-8")
        )
        self.assertIn(name, registry["jobs"])
        entry = registry["jobs"][name]
        self.assertEqual(entry["workflow"], "merge-group-doorbell.yml")
        self.assertEqual(entry["job_id"], "doorbell")

    def test_the_job_is_bounded(self):
        self.assertLessEqual(self.job["timeout-minutes"], 5)

    def test_every_ring_fires(self):
        # cancel-in-progress would let a burst of queue transitions drop rings.
        self.assertIs(self.wf["concurrency"]["cancel-in-progress"], False)


class TestTheYamlSeamCannotGoInert(unittest.TestCase):
    """THE SEAM. Measured on this estate: structure-preserving mutations at the YAML
    seam — `if:`, `continue-on-error`, a changed call-site argument — are where every
    uncaught mutant lives, because the step, its name and its `run:` body all survive
    intact and every behavioural test keeps passing while the mechanism is dead.

    Eight such mutants survived the first revision of this suite, including `if: false`
    on either job and on either step. These assertions are deliberately EXACT-MATCH and
    whole-mapping, not substring or containment.
    """

    #: The ONLY guard either workflow is allowed to carry, pinned whole. It is the
    #: fail-soft App-token mint: without the App configured the step must skip and the
    #: sweep falls back to `github.token`. Any OTHER `if:` is a way to make the
    #: mechanism inert while it still reads as wired.
    ALLOWED_STEP_GUARD = "env.ORCHESTRATOR_APP_ID != ''"
    ALLOWED_GUARD_STEP = "Mint the sparq-orchestrator App token"

    def _assert_no_inert_seam(self, path, job_id):
        wf = _yaml(path)
        job = wf["jobs"][job_id]
        self.assertNotIn("if", job, f"{path.name}: job `{job_id}` carries an `if:`. "
                         "`if: false` there makes the entire mechanism inert with every "
                         "step, name and call site intact.")
        self.assertNotIn("continue-on-error", job, f"{path.name}: job `{job_id}`")
        for step in job["steps"]:
            label = step.get("name") or step.get("uses") or "<unnamed>"
            guard = step.get("if")
            if guard is not None:
                self.assertIn(self.ALLOWED_GUARD_STEP, label,
                              f"{path.name}: unexpected `if:` on step {label!r}")
                self.assertEqual(guard, self.ALLOWED_STEP_GUARD,
                                 f"{path.name}: the one permitted guard drifted")
            self.assertNotIn("continue-on-error", step, f"{path.name}: step {label!r}")

    def test_the_watchdog_sweep_cannot_be_switched_off_at_the_seam(self):
        self._assert_no_inert_seam(WATCHDOG_YML, "watch")

    def test_the_doorbell_ring_cannot_be_switched_off_at_the_seam(self):
        self._assert_no_inert_seam(DOORBELL_YML, "doorbell")

    def test_the_doorbell_carries_no_guard_at_all(self):
        # Stronger than the shared helper: the doorbell has no App-token step, so it has
        # no legitimate `if:` anywhere. Stated separately so the shared allowance can
        # never silently license one here.
        wf = _yaml(DOORBELL_YML)
        self.assertNotIn("if", wf["jobs"]["doorbell"])
        for step in wf["jobs"]["doorbell"]["steps"]:
            self.assertNotIn("if", step, step.get("name"))

    def test_no_guard_anywhere_carries_a_constant_operand(self):
        # The conditionally-inert class in general: a literal true/false operand makes a
        # step unconditionally on or off while still reading as a guard.
        for path in (WATCHDOG_YML, DOORBELL_YML):
            wf = _yaml(path)
            for job_id, job in wf["jobs"].items():
                for guard in [job.get("if")] + [s.get("if") for s in job["steps"]]:
                    if not guard:
                        continue
                    residue = re.sub(r"[=!]=\s*(?:true|false)\b", "", str(guard))
                    self.assertIsNone(re.search(r"\b(?:true|false)\b", residue),
                                      f"{path.name}:{job_id}: {guard!r}")

    def test_the_ring_targets_THIS_repository(self):
        # `--ref` was pinned exactly while `--repo` was not asserted at all, so swapping
        # in a foreign repository left every other assertion green while the doorbell
        # rang somebody else's watchdog and this queue was never swept.
        wf = _yaml(DOORBELL_YML)
        step = wf["jobs"]["doorbell"]["steps"][0]
        self.assertIn('--repo "$REPO"', step["run"])
        self.assertEqual(step["env"]["REPO"], "${{ github.repository }}")
        # No literal owner/name may appear in the executable line.
        self.assertNotRegex(step["run"], r"[\w.-]+/[\w.-]+\.yml\s+.*\b\w+/\w+\b(?!\")")

    def test_the_sweep_call_site_arguments_are_all_pinned(self):
        # Same class on the other side: every argument the sweep depends on is asserted,
        # so dropping or retargeting one cannot pass.
        wf = _yaml(WATCHDOG_YML)
        step = _step(wf, "watch", "Sweep the merge queue")
        run = step["run"]
        for fragment in ('--repo "$REPO"', '--branch "$DEFAULT_BRANCH"',
                         'scripts/merge-group-watchdog.py'):
            self.assertIn(fragment, run, fragment)
        self.assertEqual(step["env"]["REPO"], "${{ github.repository }}")
        self.assertEqual(step["env"]["DEFAULT_BRANCH"],
                         "${{ github.event.repository.default_branch }}")


class TestTheResweepLoopIsExecutedNotJustRead(unittest.TestCase):
    """The loop is the timing fix, so it is verified by RUNNING it.

    Every assertion here survives a purely structural inspection of the YAML: a mutant
    that changes `rc=1` to `rc=0`, moves `exit "$rc"` inside the loop, appends
    `|| true`, or drops the loop to a single sweep leaves the step present, named,
    and still containing `sweep --repo --branch`. Only execution separates them.
    """

    def _run_block(self) -> str:
        return _step(_yaml(WATCHDOG_YML), "watch", "Sweep the merge queue")["run"]

    def _execute(self, exit_codes: list[int]):
        """Run the real step body with `python3` stubbed to a scripted exit sequence."""
        import subprocess
        import tempfile

        with tempfile.TemporaryDirectory() as tmp:
            tmp = Path(tmp)
            (tmp / "calls.log").write_text("", encoding="utf-8")
            codes = " ".join(str(c) for c in exit_codes)
            (tmp / "python3").write_text(
                "#!/usr/bin/env bash\n"
                f'codes=({codes})\n'
                f'log="{tmp}/calls.log"\n'
                'n=$(wc -l < "$log")\n'
                # argv COUNT is logged alongside the text because an extra EMPTY element
                # is invisible in "$*". It is NOT something the pre-loop call site could
                # produce — see test_the_dry_run_flag_is_passed_as_one_argument_when_set
                # for the measurement — but it IS what a plausible edit to the array
                # form produces, so the count is asserted rather than the joined text.
                'printf "%s|%s\\n" "$#" "$*" >> "$log"\n'
                'code=${codes[$n]:-0}\n'
                'exit "$code"\n',
                encoding="utf-8",
            )
            (tmp / "python3").chmod(0o755)
            proc = subprocess.run(
                ["bash", "-c", self._run_block()],
                env={
                    "PATH": f"{tmp}:/usr/bin:/bin",
                    "REPO": "sparq-org/sparq",
                    "DEFAULT_BRANCH": "main",
                    "DRY_RUN": "",
                    "RESWEEPS": "4",
                    "MAX_RECOVERIES_PER_SWEEP": "1",
                    # the real 60 s would make this suite unrunnable; the interval is
                    # not what is under test, the sequencing and the exit code are.
                    "RESWEEP_INTERVAL_SECONDS": "0",
                },
                capture_output=True,
                text=True,
            )
            calls = [
                line for line in (tmp / "calls.log").read_text(encoding="utf-8").splitlines()
                if line.strip()
            ]
            return proc, calls

    def test_a_clean_run_sweeps_reswept_plus_one_times_and_exits_zero(self):
        proc, calls = self._execute([0, 0, 0, 0, 0])
        self.assertEqual(proc.returncode, 0, proc.stderr)
        self.assertEqual(len(calls), 5, calls)
        for call in calls:
            argc, _, text = call.partition("|")
            self.assertIn("scripts/merge-group-watchdog.py sweep", text)
            self.assertIn("--repo sparq-org/sparq", text)
            self.assertIn("--branch main", text)
            self.assertNotIn("--dry-run", text)
            # EXACT argc: script + sweep + --repo + val + --branch + val
            # + --max-recoveries-per-run + val == 8, i.e. NO
            # empty seventh element. The hazard belongs to the ARRAY form this replaced
            # the folded one-liner with: `sweep_args+=("$DRY_RUN")` applied
            # UNCONDITIONALLY appends an empty element when the flag is unset, and
            # argparse rejects it (`error: unrecognized arguments:`, exit 2) — which
            # would fail every sweep in the DEFAULT non-dry-run configuration. Hence the
            # `if [ -n "$DRY_RUN" ]` guard, which this count is what pins.
            self.assertEqual(argc, "8", call)

    def test_a_failure_on_the_FIRST_sweep_survives_four_later_successes(self):
        """THE STICKY-STATE ASSERTION. This estate shipped the opposite defect three
        times in one day: a later transient discarding an earned hard failure. The
        interleaved sequence is the only one that can tell `rc` from a plain
        last-command exit status."""
        proc, calls = self._execute([1, 0, 0, 0, 0])
        self.assertEqual(len(calls), 5, "a failed sweep must not abort the loop")
        self.assertNotEqual(
            proc.returncode, 0,
            "sweep 1 failed and sweeps 2-5 succeeded; the run reported success, so the "
            "failure was discarded by a later success",
        )

    def test_a_failure_on_the_LAST_sweep_also_fails_the_run(self):
        proc, _ = self._execute([0, 0, 0, 0, 1])
        self.assertNotEqual(proc.returncode, 0)

    def test_a_middle_failure_fails_the_run(self):
        proc, calls = self._execute([0, 0, 1, 0, 0])
        self.assertEqual(len(calls), 5)
        self.assertNotEqual(proc.returncode, 0)

    def test_the_dry_run_flag_is_passed_as_one_argument_when_set(self):
        import subprocess
        import tempfile

        with tempfile.TemporaryDirectory() as tmp:
            tmp = Path(tmp)
            (tmp / "calls.log").write_text("", encoding="utf-8")
            (tmp / "python3").write_text(
                "#!/usr/bin/env bash\n"
                f'printf "%s\\n" "$#" >> "{tmp}/calls.log"\n'
                f'printf "%s\\n" "$*" >> "{tmp}/calls.log"\n',
                encoding="utf-8",
            )
            (tmp / "python3").chmod(0o755)
            proc = subprocess.run(
                ["bash", "-c", self._run_block()],
                env={
                    "PATH": f"{tmp}:/usr/bin:/bin",
                    "REPO": "sparq-org/sparq",
                    "DEFAULT_BRANCH": "main",
                    "DRY_RUN": "--dry-run",
                    "RESWEEPS": "0",
                    "MAX_RECOVERIES_PER_SWEEP": "1",
                    "RESWEEP_INTERVAL_SECONDS": "0",
                },
                capture_output=True,
                text=True,
            )
            lines = (tmp / "calls.log").read_text(encoding="utf-8").splitlines()
        self.assertEqual(proc.returncode, 0, proc.stderr)
        # script + sweep + --repo + val + --branch + val
        # + --max-recoveries-per-run + val + --dry-run == 9 argv elements.
        # The COUNT is the assertion: `--dry-run` must arrive as exactly ONE element.
        #
        # HISTORICAL CORRECTION, recorded because the wrong version was published: the
        # folded one-liner this replaced used an UNQUOTED `$DRY_RUN`, and an unquoted
        # expansion of an empty variable yields ZERO words, so the merge-base call site
        # produced a clean argc=6 and was NOT buggy. Injecting an empty element requires
        # a QUOTED expansion. Measured under GitHub's default
        # `bash --noprofile --norc -eo pipefail` on the MERGE-BASE body: unquoted +
        # empty => argc=6 (no empty word); quoted + empty => argc=7 with a trailing ``. The array rewrite is a
        # shellcheck-cleanliness change, NOT a bug fix, and the hazard it introduces is
        # its own (an unconditional `+=("$DRY_RUN")`), which is what these counts pin.
        self.assertEqual(lines[0], "9", lines)
        self.assertIn("--dry-run", lines[1])

    def test_the_loop_actually_repeats(self):
        # Pins the resweep COUNT to the env declaration rather than to a literal, so a
        # silent drop to a single sweep (the pre-fix behaviour) is caught.
        step = _step(_yaml(WATCHDOG_YML), "watch", "Sweep the merge queue")
        declared = int(step["env"]["RESWEEPS"])
        self.assertGreaterEqual(declared, 3)
        _, calls = self._execute([0] * (declared + 1))
        self.assertEqual(len(calls), declared + 1)

    #: The per-FIRE recovery ceiling this workflow is allowed to reach. Pinned so that
    #: raising RESWEEPS or the per-sweep budget cannot silently widen the blast radius
    #: again — the loop already multiplied the script's process-local bound once.
    MAX_RECOVERIES_PER_FIRE = 5

    def test_the_per_fire_blast_radius_is_bounded(self):
        step = _step(_yaml(WATCHDOG_YML), "watch", "Sweep the merge queue")
        per_sweep = int(step["env"]["MAX_RECOVERIES_PER_SWEEP"])
        sweeps = int(step["env"]["RESWEEPS"]) + 1
        self.assertLessEqual(
            per_sweep * sweeps, self.MAX_RECOVERIES_PER_FIRE,
            "`--max-recoveries-per-run` is PROCESS-local, so the per-fire ceiling is "
            "sweeps x per-sweep budget. Raising either without raising this constant "
            "widens what a single fire may dequeue.",
        )
        self.assertLessEqual(per_sweep, mgw.DEFAULT_MAX_RECOVERIES_PER_RUN,
                             "the per-sweep budget may only ever TIGHTEN the script "
                             "default, never widen it")

    def test_the_budget_flag_actually_reaches_the_sweep(self):
        # The env value is inert unless it is on the command line — the call-site half
        # of the bound above.
        _, calls = self._execute([0, 0, 0, 0, 0])
        for call in calls:
            self.assertIn("--max-recoveries-per-run 1", call.partition("|")[2], call)

    def test_the_loop_covers_the_grace_period(self):
        # The whole reason the loop exists: one fire must outlive
        # (group build latency + grace) or the doorbell ring is wasted.
        step = _step(_yaml(WATCHDOG_YML), "watch", "Sweep the merge queue")
        span = int(step["env"]["RESWEEPS"]) * int(step["env"]["RESWEEP_INTERVAL_SECONDS"])
        self.assertGreater(
            span, mgw.DEFAULT_GRACE_SECONDS,
            "the resweep window must outlast the grace period, or every sweep in the "
            "run decides WAIT on the group it was rung for",
        )

    def test_the_step_body_is_pure_shell(self):
        # An `${{ }}` expression inside the run block would make it unexecutable here —
        # i.e. would silently turn every test in this class vacuous.
        self.assertNotIn("${{", self._run_block())

    def test_no_failure_swallowing_idiom_in_the_shell(self):
        self.assertNotIn("|| true", self._run_block())

    def test_no_failure_swallowing_at_the_YAML_SEAM(self):
        """`continue-on-error` is a YAML KEY, so looking for it in the `run:` STRING can
        never fail for the defect it names — the string is a shell body and the key is a
        sibling mapping entry. That vacuous shape shipped in the first revision of this
        file and is why the assertion now reads the PARSED step and job.

        `continue-on-error: true` on this step makes the whole sticky-`rc` design inert:
        the step still runs all five sweeps and still exits non-zero, and the JOB still
        reports success. Every behavioural test in this class passes on that mutant."""
        wf = _yaml(WATCHDOG_YML)
        job = wf["jobs"]["watch"]
        step = _step(wf, "watch", "Sweep the merge queue")
        self.assertNotIn("continue-on-error", step, "the sweep step may not swallow its "
                         "own failure — the sticky exit code would become decorative")
        self.assertNotIn("continue-on-error", job)
        for other in _steps(wf, "watch"):
            self.assertNotIn("continue-on-error", other,
                             other.get("name") or other.get("uses"))


class TestSuiteIsWiredIntoCI(unittest.TestCase):
    """A test suite nobody runs is not a test suite (the effect-evidence rule)."""

    def test_docs_quality_runs_this_file_and_the_script_self_test(self):
        text = DOCS_QUALITY_YML.read_text(encoding="utf-8")
        self.assertIn("scripts/tests/test_merge_group_watchdog.py", text)
        self.assertIn("scripts/merge-group-watchdog.py --self-test", text)

    def test_the_running_job_is_not_advisory(self):
        wf = _yaml(DOCS_QUALITY_YML)
        for job_id, job in wf["jobs"].items():
            steps = job.get("steps") or []
            if any(
                "test_merge_group_watchdog.py" in str(step.get("run", ""))
                for step in steps
            ):
                self.assertIsNone(
                    re.search(r"\b(advisory|informational)\b", job.get("name", job_id)),
                    job.get("name", job_id),
                )
                return
        self.fail("no docs-quality job runs the watchdog suite")


if __name__ == "__main__":
    unittest.main(verbosity=2)
