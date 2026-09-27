#!/usr/bin/env python3
"""Suite for scripts/ci_execution_latency_alarm.py. 🤖 SPARQ agent. Bead sq-1lc4i.

Three halves, because three different things can go wrong:

  1. THE CLASSIFIERS — every guard has a named test that reds when the guard is deleted or
     inverted. Includes the two guards that are easiest to get wrong in the SILENT
     direction: the missing-baseline fallback (a lane whose runs never complete must still
     be watchable) and the empty-scan-set fail-loud (a detector watching nothing must not
     report health). M4 (#4802) additionally carries a KNOWN POSITIVE and a KNOWN NEGATIVE
     named test built from measurements recorded in the detector's own header, a
     real-workflow-tree anti-vacuity anchor for its admission predicate (whose empty state
     is legitimate and therefore cannot be fail-loud), and the exit-code rule that keeps a
     CHRONIC finding from permanently redding — and so muting — the incident modes.

  2. THE #4368 DISJOINTNESS — M1's scope is the exact set complement of
     `cron_lane_liveness.py`'s. If either predicate drifts the two detectors could both
     raise on one lane (duplicate issues) or neither could (a hole). Both directions are
     pinned, over the REAL workflow tree rather than fixtures.

  3. THE YAML SEAM — measured across this repo, every mutant that survived a Python-only
     battery lived in the workflow. So the step, its `if:`, the call site, the ordering,
     the permissions and the action pins are all asserted by EXACT match, never substring:
     `--apply-DROPPED` and `--self-test-DISABLED` both survive a containment check.
"""

from __future__ import annotations

import datetime as dt
import importlib.util
import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

import yaml

REPO_ROOT = Path(__file__).resolve().parents[2]
SCRIPT = REPO_ROOT / "scripts" / "ci_execution_latency_alarm.py"
WORKFLOWS = REPO_ROOT / ".github" / "workflows"
ALARM_WF = WORKFLOWS / "ci-latency-alarm.yml"
DOCS_QUALITY = WORKFLOWS / "docs-quality.yml"
REGISTRY_PATH = REPO_ROOT / ".github" / "advisory-registry.json"

_spec = importlib.util.spec_from_file_location("ci_execution_latency_alarm", SCRIPT)
alarm = importlib.util.module_from_spec(_spec)
sys.modules["ci_execution_latency_alarm"] = alarm
_spec.loader.exec_module(alarm)

NOW = dt.datetime(2026, 7, 28, 12, 0, tzinfo=dt.timezone.utc)


def _lane(**kw):
    return alarm._lane(now=NOW, **kw)


def _run_obj(**kw):
    return alarm._run_obj(now=NOW, **kw)


def _job_name(workflow_file: Path) -> str:
    jobs = yaml.safe_load(workflow_file.read_text())["jobs"]
    assert len(jobs) == 1, f"{workflow_file}: expected exactly one job, got {list(jobs)}"
    return str(next(iter(jobs.values()))["name"])


# ---------------------------------------------------------------------------------
# 1. classifiers
# ---------------------------------------------------------------------------------
class TestCronExpansion(unittest.TestCase):
    """Canary-validated against hand-computed answers. An expander that silently returns
    0 would make every lane look like a 100% deficit; one that over-counts would make
    every lane look healthy. Both directions are pinned."""

    def test_known_firing_counts(self):
        cases = [
            ("*/10 * * * *", 6, 36),
            ("*/10 * * * *", 24, 144),
            ("17 3 * * *", 24, 1),
            ("4,14,24,34,44,54 * * * *", 6, 36),
            ("2,7,12,17,22,27,32,37,42,47,52,57 * * * *", 6, 72),
            ("*/30 * * * *", 24, 48),
            ("37 1,7,13,19 * * *", 24, 4),
            ("0 */4 * * *", 24, 6),
        ]
        for expr, hours, want in cases:
            with self.subTest(expr=expr):
                got = alarm.expected_firings(expr, NOW - dt.timedelta(hours=hours), NOW)
                self.assertEqual(got, want)

    def test_weekly_cron_is_absent_from_a_window_that_does_not_contain_it(self):
        # 2026-07-28 is a Tuesday; the Monday fire is >24h back but <48h back.
        self.assertEqual(
            alarm.expected_firings("0 6 * * 1", NOW - dt.timedelta(hours=24), NOW), 0)
        self.assertEqual(
            alarm.expected_firings("0 6 * * 1", NOW - dt.timedelta(hours=48), NOW), 1)

    def test_dom_and_dow_both_restricted_is_a_UNION_not_an_intersection(self):
        # POSIX: when both day fields are restricted the match is their OR. Reading it as
        # AND would UNDER-count expected firings and hide a real deficit.
        n = alarm.expected_firings(
            "0 0 1 * 3",
            dt.datetime(2026, 7, 1, tzinfo=dt.timezone.utc),
            dt.datetime(2026, 7, 31, tzinfo=dt.timezone.utc))
        self.assertGreater(n, 1, "dom+dow must be a union (all Wednesdays plus the 1st)")

    def test_malformed_cron_raises_rather_than_returning_zero(self):
        for bad in ("", "* * * *", "* * * * * *", "60 * * * *", "*/0 * * * *",
                    "5-1 * * * *", "x * * * *"):
            with self.subTest(bad=bad):
                with self.assertRaises(alarm.CronError):
                    alarm.expected_firings(bad, NOW - dt.timedelta(hours=6), NOW)


class TestM1CronFiringDeficit(unittest.TestCase):
    CAP = alarm.capped_expectation()

    def test_a_delivering_lane_does_not_raise(self):
        f, c = alarm.find_cron_deficits([_lane(fires=self.CAP)], NOW)
        self.assertEqual(f, [])
        self.assertEqual(c.get("delivering"), 1)

    def test_a_lane_that_fired_far_below_expectation_raises(self):
        f, c = alarm.find_cron_deficits([_lane(fires=1)], NOW)
        self.assertEqual(len(f), 1)
        self.assertEqual((f[0]["expected"], f[0]["actual"]), (self.CAP, 1))
        self.assertEqual(c.get("firing-deficit"), 1)

    def test_a_cron_that_never_fired_at_all_raises(self):
        # THE MODE WITH NO ARTIFACT. There is no run to inspect, so this can only be
        # caught by comparing against a computed expectation.
        f, _ = alarm.find_cron_deficits([_lane(fires=0)], NOW)
        self.assertEqual(len(f), 1)
        self.assertEqual(f[0]["actual"], 0)

    def test_the_expectation_is_CAPPED_at_what_github_really_delivers(self):
        """The load-bearing anti-cry-wolf guard. MEASURED: sparq's `*/10` lanes have a
        nominal expectation of 144/day and GitHub delivers ~12. Comparing against nominal
        would put six lanes permanently in breach, and a permanently-red alarm is a muted
        alarm. Delete the cap and this test reds."""
        f, _ = alarm.find_cron_deficits(
            [_lane(crons=("*/10 * * * *",), fires=self.CAP)], NOW)
        self.assertEqual(f, [], "a sub-hourly lane at GitHub's real ceiling is healthy")
        nominal = alarm.expected_firings(
            "*/10 * * * *", NOW - dt.timedelta(hours=alarm.CRON_WINDOW_HOURS), NOW)
        self.assertGreater(nominal, self.CAP,
                           "fixture is vacuous unless nominal exceeds the cap")

    def test_a_firing_inside_the_grace_window_is_not_counted_as_missing(self):
        """Without the grace period the tick that lands on a lane's own cron minute sees
        an expectation whose run does not exist yet, and manufactures a deficit daily."""
        lane = _lane(fires=0)
        lane["schedule_run_times"] = [NOW - dt.timedelta(minutes=1)] * self.CAP
        f, _ = alarm.find_cron_deficits([lane], NOW)
        self.assertEqual(len(f), 1, "runs newer than the grace cut-off are not yet due")

    def test_the_floor_boundary_is_a_strict_less_than(self):
        import math
        at_floor = math.ceil(self.CAP * alarm.CRON_DELIVERY_FLOOR)
        self.assertEqual(alarm.find_cron_deficits([_lane(fires=at_floor)], NOW)[0], [])
        self.assertEqual(
            len(alarm.find_cron_deficits([_lane(fires=at_floor - 1)], NOW)[0]), 1)

    def test_the_delivery_floor_is_pinned_by_a_LITERAL_fixture(self):
        """`at_floor` above is COMPUTED FROM CRON_DELIVERY_FLOOR, so it rescales with the
        constant and cannot fail when it moves — MEASURED: `0.60 -> 0.05` survived the
        sibling deployment's entire suite for exactly that reason, and dies in this one
        only by the accident of its smaller cap.

        `0 */4 * * *` has a 24h nominal of 5, BELOW the cap, so `expected` is 5 whatever
        the cap constant is. Neither 5 nor the fire counts derive from any constant."""
        f, _ = alarm.find_cron_deficits(
            [_lane(crons=("0 */4 * * *",), fires=2)], NOW)
        self.assertEqual(len(f), 1, "2 of 5 = 0.40 must raise")
        self.assertEqual(f[0]["expected"], 5)
        self.assertEqual(f[0]["ratio"], 0.4)
        f, _ = alarm.find_cron_deficits(
            [_lane(crons=("0 */4 * * *",), fires=3)], NOW)
        self.assertEqual(f, [], "3 of 5 = 0.60 must NOT raise")

    def test_the_window_constant_is_pinned_by_a_LITERAL_expectation(self):
        """`CRON_WINDOW_HOURS 24.0 -> 6.0` survived every earlier assertion — and 6.0 is
        the value the module docstring says was TRIED AND REJECTED. At the 24h window a
        4-hourly lane's expectation is 5; at 6h it is 1, and the same fixture goes quiet."""
        self.assertEqual(alarm.CRON_WINDOW_HOURS, 24.0)
        f, _ = alarm.find_cron_deficits(
            [_lane(crons=("0 */4 * * *",), fires=2)], NOW)
        self.assertEqual(len(f), 1)
        self.assertEqual(f[0]["expected"], 5,
                         "the DEFAULT window is what makes this expectation 5")

    def test_every_shipped_constant_is_pinned_against_a_literal(self):
        """A threshold exercised only through fixtures derived from it is not tested.
        These are the independent write-downs."""
        self.assertEqual(alarm.CRON_MAX_CREDIBLE_FIRINGS_PER_HOUR, 0.5)
        self.assertEqual(alarm.CRON_GRACE_MINUTES, 15)
        self.assertEqual(alarm.EXEC_FLOOR_SECONDS, 6 * 60 * 60)
        self.assertEqual(alarm.BASELINE_MIN_N, 5)
        self.assertGreaterEqual(alarm.CRON_DELIVERY_FLOOR, 0.50)
        self.assertLessEqual(alarm.CRON_DELIVERY_FLOOR, 0.70)
        self.assertGreaterEqual(alarm.EXEC_OVERRUN_MULTIPLE, 1.25)
        self.assertLessEqual(alarm.EXEC_OVERRUN_MULTIPLE, 2.0)


    def test_an_over_delivering_lane_is_healthy(self):
        f, _ = alarm.find_cron_deficits([_lane(fires=self.CAP * 3)], NOW)
        self.assertEqual(f, [])

    def test_a_lane_below_the_expectation_floor_is_quiet_and_counted(self):
        # A weekly lane has a capped expectation of 0 inside a 24h window.
        f, c = alarm.find_cron_deficits([_lane(crons=("0 6 * * 1",), fires=0)], NOW)
        self.assertEqual(f, [])
        self.assertEqual(c.get("expectation-below-floor"), 1)

    def test_a_disabled_lane_is_quiet_and_counted(self):
        f, c = alarm.find_cron_deficits([_lane(state="disabled_manually", fires=0)], NOW)
        self.assertEqual(f, [])
        self.assertEqual(c.get("disabled"), 1)

    def test_an_unparseable_cron_is_quiet_and_VISIBLE_in_the_census(self):
        f, c = alarm.find_cron_deficits([_lane(crons=("nonsense",), fires=0)], NOW)
        self.assertEqual(f, [])
        self.assertEqual(c.get("cron-unparseable"), 1,
                         "a fail-safe skip must be counted, or it is a silent skip")


class TestM1NewLaneWindow(unittest.TestCase):
    """A newly-added lane has not had time to deliver, and M1 cannot tell "did not fire"
    from "did not exist yet" out of run times alone — so a lane added two hours ago reads
    ratio 0.08 and alarms for the next ~14 hours. The workflow's own `created_at` resolves
    it; a lane younger than the window is counted and skipped, not guessed at."""

    def test_a_lane_younger_than_the_window_is_skipped_and_counted(self):
        f, c = alarm.find_cron_deficits([_lane(fires=1, created_hours_ago=2)], NOW)
        self.assertEqual(f, [])
        self.assertEqual(c.get("lane-too-new-for-an-expectation"), 1)

    def test_the_skip_is_not_an_unconditional_mute(self):
        """ANTI-VACUITY. The identical delivery from a lane with no birth date, and from
        one born before the window, must still alarm — otherwise the new-lane exit is
        just M1 switched off."""
        self.assertEqual(len(alarm.find_cron_deficits([_lane(fires=1)], NOW)[0]), 1)
        self.assertEqual(
            len(alarm.find_cron_deficits(
                [_lane(fires=1, created_hours_ago=48)], NOW)[0]), 1)

    def test_the_birth_date_boundary_is_pinned_from_BOTH_sides(self):
        """Without both directions `if born > start` and `if born > start - 100 years`
        are the same test."""
        f, c = alarm.find_cron_deficits([_lane(fires=1, created_hours_ago=23)], NOW)
        self.assertEqual(f, [])
        self.assertEqual(c.get("lane-too-new-for-an-expectation"), 1)
        f, c = alarm.find_cron_deficits([_lane(fires=1, created_hours_ago=25)], NOW)
        self.assertEqual(len(f), 1)
        self.assertEqual(c.get("firing-deficit"), 1)

    def test_an_established_lane_is_unaffected_by_the_check(self):
        self.assertEqual(
            alarm.find_cron_deficits(
                [_lane(fires=TestM1CronFiringDeficit.CAP, created_hours_ago=48)],
                NOW)[0], [])

    def test_the_fetch_layer_supplies_the_birth_date_the_check_reads(self):
        """THE CALL SITE again: `fetch_lanes` must carry `created_at` off the workflow
        listing, or the guard above can never fire in production."""
        listing = {"workflows": [
            {"path": ".github/workflows/ci.yml", "state": "active",
             "created_at": "2026-07-01T00:00:00Z"}]}

        def fake_gh(args, *, parse=True):
            return listing if "actions/workflows?" in args[-1] else {"workflow_runs": []}

        real = alarm._gh
        alarm._gh = fake_gh
        try:
            lanes = alarm.fetch_lanes("o/r", WORKFLOWS, 24.0, NOW)
        finally:
            alarm._gh = real
        by_path = {lane["workflow"]: lane for lane in lanes}
        self.assertEqual(by_path[".github/workflows/ci.yml"]["created_at"],
                         "2026-07-01T00:00:00Z")

class TestM4CadenceFidelity(unittest.TestCase):
    """M4 (#4802): a lane that keeps running, keeps going green, and delivers a small
    fraction of its DECLARED cadence.

    The instrument is validated in both directions against measurements recorded in the
    detector's own header — see `test_KNOWN_POSITIVE_*` and `test_KNOWN_NEGATIVE_*`. The
    negative is deliberately NOT "the same lanes on a healthy day", because those lanes
    deliver the same ~12/day on a healthy day; that reconciliation is the whole reason M4
    keys on the declaration rather than on the day.
    """

    # The real cron strings from this repo's workflow files paired with the achieved
    # counts measured over the 24h to 2026-07-28T13:15Z. Nothing is derived from
    # M4_FIDELITY_FLOOR, so the floor cannot rescale the fixture.
    MEASURED_2026_07_28 = [
        ("promote-on-approval.yml", "*/10 * * * *", 12),
        ("rearm-sweeper.yml", "8,18,28,38,48,58 * * * *", 13),
        ("auto-arm.yml", "4,14,24,34,44,54 * * * *", 12),
        ("verdict-bridge.yml", "1,11,21,31,41,51 * * * *", 12),
        ("batch-merge.yml", "7,22,37,52 * * * *", 12),
        ("retriage.yml", "*/30 * * * *", 12),
    ]
    # Lanes from the SAME census whose declaration is achievable here: 1/1 and 3/4.
    MEASURED_ACHIEVABLE = [
        ("ci.yml", "17 3 * * *", 1),
        ("refresh-start-here.yml", "23 5,11,17,23 * * *", 3),
    ]
    HOURLY = ("0 * * * *",)  # exactly 23 declared firings inside the graced 24h window

    def test_KNOWN_POSITIVE_the_measured_2026_07_28_lanes_are_all_divergent(self):
        lanes = [_lane(workflow=w, crons=(c,), fires=n, spacing_min=30)
                 for w, c, n in self.MEASURED_2026_07_28]
        f, c = alarm.find_cadence_fidelity_gaps(lanes, NOW)
        self.assertEqual(c.get("declaration-unachievable"),
                         len(self.MEASURED_2026_07_28))
        self.assertEqual(len(f), 1)
        named = {x["workflow"] for x in f[0]["lanes"]}
        self.assertEqual(named, {w for w, _, _ in self.MEASURED_2026_07_28})
        # The headline of #4802: `promote-on-approval` declares ~144/day and delivers 12.
        pop = {x["workflow"]: x for x in f[0]["lanes"]}["promote-on-approval.yml"]
        self.assertEqual(pop["actual"], 12)
        self.assertGreater(pop["nominal"], 100)
        self.assertLess(pop["fidelity"], 0.10)

    def test_KNOWN_NEGATIVE_lanes_with_an_achievable_declaration_are_silent(self):
        lanes = [_lane(workflow=w, crons=(c,), fires=n)
                 for w, c, n in self.MEASURED_ACHIEVABLE]
        f, c = alarm.find_cadence_fidelity_gaps(lanes, NOW)
        self.assertEqual(f, [])
        self.assertEqual(c.get("declaration-within-the-measured-ceiling"), 2)

    def test_ONE_repo_level_finding_never_one_per_lane(self):
        """13 identical issues is the shape that gets an alarm muted (#4802). The finding
        must aggregate, and its subject must not be readable as a single lane."""
        lanes = [_lane(workflow=w, crons=(c,), fires=n, spacing_min=30)
                 for w, c, n in self.MEASURED_2026_07_28]
        f, _ = alarm.find_cadence_fidelity_gaps(lanes, NOW)
        self.assertEqual(len(f), 1)
        self.assertTrue(f[0]["workflow"].startswith("(repo-level"))
        self.assertNotIn(f[0]["workflow"], {w for w, _, _ in self.MEASURED_2026_07_28})

    def test_the_denominator_is_the_DECLARATION_not_M1s_capped_expectation(self):
        """If M4 used the cap it would be M1 with a different floor, and every one of the
        measured lanes — which deliver exactly the cap — would read as healthy."""
        lanes = [_lane(workflow=w, crons=(c,), fires=n, spacing_min=30)
                 for w, c, n in self.MEASURED_2026_07_28]
        f, _ = alarm.find_cadence_fidelity_gaps(lanes, NOW)
        for entry in f[0]["lanes"]:
            with self.subTest(lane=entry["workflow"]):
                self.assertGreater(entry["nominal"], alarm.capped_expectation())

    def test_the_fidelity_floor_is_pinned_by_a_LITERAL_fixture_from_BOTH_sides(self):
        """`0 * * * *` declares exactly 23 firings in the graced window — a number that
        derives from neither M4_FIDELITY_FLOOR nor the cap, so neither can rescale it.
        9/23 = 0.391 must raise and 10/23 = 0.435 must not."""
        f, _ = alarm.find_cadence_fidelity_gaps([_lane(crons=self.HOURLY, fires=9)], NOW)
        self.assertEqual(len(f), 1)
        self.assertEqual(f[0]["lanes"][0]["nominal"], 23)
        self.assertEqual(f[0]["lanes"][0]["actual"], 9)
        f, c = alarm.find_cadence_fidelity_gaps([_lane(crons=self.HOURLY, fires=10)], NOW)
        self.assertEqual(f, [])
        self.assertEqual(c.get("declaration-honoured"), 1)

    def test_the_admission_ceiling_is_pinned_from_BOTH_sides(self):
        """A lane declaring no more than the measured ceiling is M1's question on M1's
        denominator; admitting it would put two detectors on one lane. `0 0-11 * * *`
        declares exactly 12 and `30 0-12 * * *` exactly 13, both literal."""
        f, c = alarm.find_cadence_fidelity_gaps(
            [_lane(crons=("0 0-11 * * *",), fires=0)], NOW)
        self.assertEqual(f, [], "12 declared == the ceiling: not M4's question")
        self.assertEqual(c.get("declaration-within-the-measured-ceiling"), 1)
        f, _ = alarm.find_cadence_fidelity_gaps(
            [_lane(crons=("30 0-12 * * *",), fires=0)], NOW)
        self.assertEqual(len(f), 1, "13 declared > the ceiling: admitted")
        self.assertEqual(f[0]["lanes"][0]["nominal"], 13)

    def test_the_floor_constant_is_pinned_against_the_measured_empty_band(self):
        """0.75 was the worst HONOURED declaration and 0.25 the best FICTIONAL one on the
        2026-07-28 census; the floor must sit strictly inside that empty band. It is also
        kept below 0.50 so an hourly lane is not judged on an unmeasured inference."""
        self.assertGreater(alarm.M4_FIDELITY_FLOOR, 0.25)
        self.assertLess(alarm.M4_FIDELITY_FLOOR, 0.50)

    def test_CRON_ONLY_lanes_are_in_scope_because_M1_cannot_see_them(self):
        """Three of the five lanes #4802 measured — promote-on-approval, rearm-sweeper,
        retriage — are cron-only, and M1's scope predicate excludes every one of them."""
        lane = _lane(workflow="promote-on-approval.yml", cron_only=True, in_scope=False,
                     fires=12, spacing_min=30)
        f, _ = alarm.find_cadence_fidelity_gaps([lane], NOW)
        self.assertEqual(len(f), 1)
        self.assertEqual(
            alarm.find_cron_deficits([lane], NOW)[0], [],
            "fixture is vacuous unless M1 really is blind to this lane")

    def test_a_truncated_run_sample_is_fail_safe_QUIET_and_counted(self):
        """The sample is the newest 100 runs. An undercount would manufacture a fiction
        out of a healthy lane, which is the one direction this alarm must never fail in."""
        lane = _lane(fires=0)
        lane["truncated"] = True
        f, c = alarm.find_cadence_fidelity_gaps([lane], NOW)
        self.assertEqual(f, [])
        self.assertEqual(c.get("sample-truncated"), 1)

    def test_coverage_is_judged_per_WINDOW_not_once_against_M1s(self):
        """A full 100-run page reaching back ~6.9h COVERS a 6h window and does NOT cover
        M4's 24h one. Deciding truncation once at fetch time against `--window-hours`
        therefore leaks M1's window into M4: at `--window-hours 6` this sample would be
        called complete and M4 would divide by a 24h declaration over a ~7h count."""
        lane = _lane(fires=0, crons=self.HOURLY)
        newest = NOW - dt.timedelta(minutes=alarm.CRON_GRACE_MINUTES + 1)
        lane["schedule_run_times"] = [newest - dt.timedelta(minutes=4 * i)
                                      for i in range(100)]
        self.assertTrue(alarm.sample_truncated(lane, NOW - dt.timedelta(hours=24)))
        self.assertFalse(alarm.sample_truncated(lane, NOW - dt.timedelta(hours=6)))
        f, c = alarm.find_cadence_fidelity_gaps([lane], NOW)
        self.assertEqual(f, [])
        self.assertEqual(c.get("sample-truncated"), 1)

    def test_a_page_that_came_back_SHORT_is_complete_not_truncated(self):
        """ANTI-VACUITY for the guard above: `sample_truncated` must key on the page being
        FULL, not merely on the oldest run being young, or every quiet lane would mute
        itself and M4 could never raise at all."""
        lane = _lane(fires=3, crons=self.HOURLY)
        self.assertFalse(alarm.sample_truncated(lane, NOW - dt.timedelta(hours=24)))

    def test_THE_CALL_PATH_M4_keeps_its_24h_window_when_M1s_is_overridden(self):
        """`--window-hours` is M1's. M4's floor and admission ceiling are validated
        against a 24h census only, and the header states plainly that a six-hour
        degradation must stay invisible to M4 — so handing M4 the CLI window would let
        `--window-hours 6` manufacture a chronic declaration finding out of exactly the
        sub-day burstiness M4 is documented not to see.

        The fixture separates the two windows: an hourly lane that delivered 10 firings,
        all of them between 7h and 23h ago. Over 24h that is 10/23 = 0.43, above the
        floor. Over 6h it is 0 of a declared 5, far below it.
        """
        lane = _lane(fires=0, crons=self.HOURLY)
        lane["schedule_run_times"] = [NOW - dt.timedelta(hours=h) for h in range(7, 17)]

        m4_at_24h, _ = alarm.find_cadence_fidelity_gaps([lane], NOW)
        m4_at_6h, _ = alarm.find_cadence_fidelity_gaps([lane], NOW, 6)
        self.assertEqual(m4_at_24h, [], "10 of a declared 23 is above the floor")
        self.assertEqual(len(m4_at_6h), 1,
                         "fixture is vacuous unless the two windows disagree")

        run = TestCensusAndExitCodes._run
        import contextlib
        import io
        # Assertions key on CENSUS STATES, never on the mode names: every mode name is
        # printed on every run as a census heading, so a containment check on one of those
        # is true whether the detector fired or not.
        for extra, expect_m1 in ((), False), (("--window-hours", "6"), True):
            with self.subTest(window=extra or "default"):
                buf = io.StringIO()
                with contextlib.redirect_stdout(buf):
                    rc = run(lanes=[lane], live=[], extra=extra)
                out = buf.getvalue()
                # M1 DOES honour the override — without this the M4 assertion below would
                # pass on a build that ignored `--window-hours` everywhere.
                self.assertEqual("firing-deficit: 1" in out, expect_m1)
                self.assertEqual(rc, 1 if expect_m1 else 0)
                # M4 reached `declaration-honoured`, which is only true on its own 24h
                # window; on a 6h one this same lane is `declaration-unachievable`.
                self.assertIn("declaration-honoured: 1", out)
                self.assertNotIn("declaration-unachievable", out)

    def test_a_lane_younger_than_the_window_is_skipped_and_counted(self):
        f, c = alarm.find_cadence_fidelity_gaps([_lane(fires=0, created_hours_ago=2)], NOW)
        self.assertEqual(f, [])
        self.assertEqual(c.get("lane-too-new-for-an-expectation"), 1)

    def test_the_quiet_exits_are_not_unconditional_mutes(self):
        """ANTI-VACUITY for both skips above: the identical delivery from an established,
        untruncated lane must still be divergent."""
        self.assertEqual(len(alarm.find_cadence_fidelity_gaps([_lane(fires=0)], NOW)[0]), 1)

    def test_a_disabled_or_unparseable_lane_is_quiet_and_VISIBLE(self):
        for lane, state in ((_lane(state="disabled_manually", fires=0), "disabled"),
                            (_lane(crons=("nonsense",), fires=0), "cron-unparseable"),
                            (_lane(crons=(), fires=0), "not-scheduled")):
            with self.subTest(state=state):
                f, c = alarm.find_cadence_fidelity_gaps([lane], NOW)
                self.assertEqual(f, [])
                self.assertEqual(c.get(state), 1)

    def test_the_real_workflow_tree_admits_a_non_empty_M4_population(self):
        """ANTI-VACUITY over the REAL tree. M4's population is defined by a CONDITION, so
        an empty one is its success state and cannot be fail-loud the way M1's empty scope
        is — which means a broken admission predicate would look exactly like a fixed
        repo. This pins that the predicate still admits the lanes #4802 named."""
        start = NOW - dt.timedelta(hours=alarm.M4_WINDOW_HOURS)
        end = NOW - dt.timedelta(minutes=alarm.CRON_GRACE_MINUTES)
        ceiling = alarm.capped_expectation(alarm.M4_WINDOW_HOURS)
        admitted = set()
        for path in sorted(WORKFLOWS.glob("*.yml")):
            on = alarm.workflow_triggers(path.read_text())
            crons = [s.get("cron") for s in (on.get("schedule") or [])
                     if isinstance(s, dict) and isinstance(s.get("cron"), str)]
            if crons and sum(alarm.expected_firings(c, start, end)
                             for c in crons) > ceiling:
                admitted.add(path.name)
        self.assertGreaterEqual(len(admitted), 5, f"M4 admission collapsed to {admitted}")
        for named in ("promote-on-approval.yml", "rearm-sweeper.yml", "retriage.yml"):
            with self.subTest(workflow=named):
                self.assertIn(named, admitted)

    def test_the_fetch_layer_supplies_run_times_for_CRON_ONLY_lanes_too(self):
        """THE CALL SITE. Narrowing the run-time fetch to M1's scope would leave M4
        reading `actual = 0` for every cron-only lane and calling all of them fictions."""
        listing = {"workflows": [
            {"path": f".github/workflows/{p.name}", "state": "active",
             "created_at": "2020-01-01T00:00:00Z"} for p in WORKFLOWS.glob("*.yml")]}
        fetched = []

        def fake_gh(args, *, parse=True):
            url = args[-1]
            if "actions/workflows?" in url:
                return listing
            fetched.append(url)
            return {"workflow_runs": []}

        real = alarm._gh
        alarm._gh = fake_gh
        try:
            alarm.fetch_lanes("o/r", WORKFLOWS, 24.0, NOW)
        finally:
            alarm._gh = real
        # promote-on-approval is cron-only; M1 would never have fetched it.
        self.assertTrue(any("promote-on-approval.yml/runs" in u for u in fetched),
                        "no schedule-run fetch for a cron-only lane")


class TestQueueWaitIsAnHonestlyDocumentedGap(unittest.TestCase):
    """Queue wait is NOT detected. This class exists so that stays TRUE and stays VISIBLE.

    An alarm that reads a variable which never moves answers "would we know if we were
    starved?" with a confident yes. Across 44,190 completed runs not one attempt recorded
    a non-zero queue wait, and the single event that motivated a detector turned out to be
    a configured `max-parallel: 8` rather than withheld runners. So the detector was
    removed and the gap documented.

    Two failure directions are pinned: silently RE-ADDING a detector (implying coverage
    that is not evidenced), and silently DELETING the evidence that justifies its absence.
    """

    def test_no_queue_wait_detector_is_exported(self):
        """Structural, not textual. If someone re-adds one, this reds and forces the
        question 'what positive validates it?' to be answered rather than skipped."""
        for gone in ("find_queue_overruns", "QUEUE_MAX_WAIT_SECONDS",
                     "attempt_created_at", "resolve_attempt_created",
                     "ATTEMPT_CREATED_KEY"):
            with self.subTest(symbol=gone):
                self.assertFalse(hasattr(alarm, gone),
                                 f"{gone} is back — a queue-wait detector needs an "
                                 f"observed positive first; see the QUEUE WAIT note")

    def test_the_reported_modes_do_not_imply_queue_coverage(self):
        """A reader of the issue body must not infer a mode that does not exist."""
        _, c1 = alarm.find_cron_deficits([_lane(fires=0)], NOW)
        _, c3 = alarm.find_execution_overruns([_run_obj(age_min=1)], {}, NOW)
        census = {"M1-cron-firing-deficit": c1, "M3-execution-overrun": c3}
        body = alarm.render_issue_body("o/r", [], census, NOW)
        self.assertNotIn("M2", body)
        self.assertNotIn("queue", body.lower())

    def test_the_live_read_does_not_fetch_a_population_nothing_consumes(self):
        """`status=queued` is no longer fetched. Fetching a population no detector reads
        is the shape of coverage-without-detection this file exists to avoid."""
        asked = []

        def fake_gh(args, *, parse=True):
            asked.append(args[-1])
            return {"workflow_runs": []}

        real = alarm._gh
        alarm._gh = fake_gh
        try:
            alarm.fetch_live_runs("o/r")
        finally:
            alarm._gh = real
        self.assertTrue(asked)
        self.assertFalse([u for u in asked if "status=queued" in u], asked)
        self.assertTrue([u for u in asked if "status=in_progress" in u], asked)

    def test_the_evidence_for_the_gap_is_still_recorded(self):
        """Pins the CONSEQUENCE deliberately: deleting the justification reds loudly
        rather than leaving a bare unexplained absence. Each anchor is a specific
        measured fact, not prose."""
        src = SCRIPT.read_text()
        for anchor, why in (
            ("44,190", "the corpus size behind 'the variable never moves'"),
            ("max-parallel: 8", "why run 30333511110 was NOT a capacity event"),
            ("30333511110", "the run the capacity inference was drawn from"),
            ("30318886362", "the re-run that produced the fabricated known positive"),
        ):
            with self.subTest(anchor=anchor):
                self.assertIn(anchor, src, f"missing {why}")


class TestM3ExecutionOverrun(unittest.TestCase):
    BASE = {(".github/workflows/a.yml", "schedule"): {"p90": 3600.0, "n": 50}}

    def test_a_run_inside_its_band_does_not_raise(self):
        f, c = alarm.find_execution_overruns([_run_obj(age_min=30)], self.BASE, NOW)
        self.assertEqual(f, [])
        self.assertEqual(c.get("in-progress-within-threshold"), 1)

    def test_a_run_past_its_band_raises(self):
        f, c = alarm.find_execution_overruns([_run_obj(age_min=8 * 60)], self.BASE, NOW)
        self.assertEqual(len(f), 1)
        self.assertEqual(c.get("execution-overrun"), 1)

    def test_m3_ignores_queued_runs(self):
        f, _ = alarm.find_execution_overruns(
            [_run_obj(status="queued", age_min=10_000)], self.BASE, NOW)
        self.assertEqual(f, [], "a queued run is M2's population, not M3's")

    def test_a_lane_with_NO_baseline_is_still_watched(self):
        """THE FAIL-OPEN HOLE. A lane whose runs never complete has no completed history,
        so it has no derived baseline. If a missing baseline caused a `continue`, the
        detector would go silent exactly when a lane is 100% hung — the alarm would be
        blind to the worst case it exists for. Ask the 100% question: if every run of a
        lane hung forever, would this still fire? It must."""
        f, _ = alarm.find_execution_overruns([_run_obj(age_min=7 * 60)], {}, NOW)
        self.assertEqual(len(f), 1)
        self.assertIn("floor", f[0]["basis"])

    def test_an_undersampled_baseline_is_not_trusted_even_when_it_is_LARGE(self):
        """The under-sampling guard only BITES when the thin baseline would WIDEN the
        threshold past the floor. A fixture with a small p90 collapses to the floor
        whether or not the guard exists, and cannot detect its removal — the earlier
        version of this test used p90=60s and both BASELINE_MIN_N mutants SURVIVED it."""
        thin_but_huge = {(".github/workflows/a.yml", "schedule"):
                         {"p90": 20 * 3600.0, "n": 1}}
        f, _ = alarm.find_execution_overruns([_run_obj(age_min=7 * 60)], thin_but_huge,
                                             NOW)
        self.assertEqual(len(f), 1,
                         "a 1-sample 20h p90 must not widen the threshold to 40h")
        self.assertEqual(f[0]["threshold_seconds"], int(alarm.EXEC_FLOOR_SECONDS))
        # Control: the SAME p90 with a real sample size legitimately widens the band.
        thick = {(".github/workflows/a.yml", "schedule"):
                 {"p90": 20 * 3600.0, "n": alarm.BASELINE_MIN_N}}
        self.assertEqual(
            alarm.find_execution_overruns([_run_obj(age_min=7 * 60)], thick, NOW)[0], [])

    def test_a_baseline_WIDER_than_the_floor_raises_the_threshold(self):
        """Quantifier direction: the floor is a MINIMUM, not the answer. A legitimately
        long lane (nightly ci.yml runs ~8.3h) must not alarm merely for exceeding 6h."""
        big = {(".github/workflows/a.yml", "schedule"): {"p90": 8 * 3600.0, "n": 40}}
        f, _ = alarm.find_execution_overruns([_run_obj(age_min=7 * 60)], big, NOW)
        self.assertEqual(f, [], "an 8.3h-p90 lane at 7h is healthy — do not cry wolf")

    def test_the_measured_2026_07_27_outlier_shape_raises(self):
        """KNOWN POSITIVE, from real data: nightly ci.yml ran 17.33h on 2026-07-27 against
        an 8.35h p90 over the preceding 13 nights (2.08x). This pins EXEC_OVERRUN_MULTIPLE
        from ABOVE — at K=2.5 the real event is missed."""
        big = {(".github/workflows/a.yml", "schedule"): {"p90": 8.35 * 3600.0, "n": 13}}
        f, _ = alarm.find_execution_overruns(
            [_run_obj(age_min=int(17.33 * 60))], big, NOW)
        self.assertEqual(len(f), 1)

    def test_the_overrun_multiple_is_pinned_from_BELOW_as_well(self):
        """Nothing pinned the multiple downwards, so `2.0 -> 1.0` survived: every fixture
        that must RAISE still raises when the threshold shrinks. 9h against an 8h p90 is
        1.13x — inside the band at K=2.0, outside it at K<=1.13."""
        big = {(".github/workflows/a.yml", "schedule"): {"p90": 8 * 3600.0, "n": 40}}
        f, _ = alarm.find_execution_overruns([_run_obj(age_min=9 * 60)], big, NOW)
        self.assertEqual(f, [], "9h against an 8h p90 is inside a 2.0x band")

    def test_the_floor_is_pinned_from_BOTH_sides_with_no_baseline(self):
        """`EXEC_FLOOR_SECONDS -> 1h` survives any suite whose no-baseline fixtures are
        all ABOVE the floor."""
        self.assertEqual(
            alarm.find_execution_overruns([_run_obj(age_min=5 * 60)], {}, NOW)[0], [])
        self.assertEqual(
            len(alarm.find_execution_overruns([_run_obj(age_min=7 * 60)], {}, NOW)[0]), 1)

    def test_the_fixture_carries_a_live_runs_real_updated_at(self):
        """`started = run.get("updated_at") or run_started_at` SURVIVED the first battery
        because the fixture was a 7-key hand-built dict with no `updated_at`. Every live
        run has one, and on a live run it tracks the PRESENT moment — so that mutant
        collapses M3's age to ~0 and the mode never fires again. The fixture now carries
        the real shape, which is what makes the sibling tests able to see it."""
        live = _run_obj(age_min=17 * 60)
        self.assertIn("updated_at", live)
        self.assertLess((NOW - alarm._ts(live["updated_at"])).total_seconds(), 60,
                        "a live run's updated_at is ~now, not its start")
        self.assertGreater(
            (NOW - alarm._ts(live["run_started_at"])).total_seconds(), 16 * 3600)
        # And the substitution really would be catastrophic — stated as a measurement,
        # not an assertion about the shipped code.
        self.assertEqual(
            alarm.find_execution_overruns(
                [dict(live, run_started_at=live["updated_at"])], {}, NOW)[0], [],
            "reading updated_at as the start silences a 17h overrun")

    def test_the_detection_variable_is_live_age_not_completed_history(self):
        """A hang is invisible to any statistic over COMPLETED runs — a job that never
        finishes never appears in one. This pins that M3 reads the LIVE run's age: a run
        with a huge live age raises even when the completed baseline is enormous, and the
        baseline alone can never produce a finding without a live run."""
        big = {(".github/workflows/a.yml", "schedule"): {"p90": 8 * 3600.0, "n": 40}}
        f, c = alarm.find_execution_overruns([], big, NOW)
        self.assertEqual((f, c), ([], {}), "no live run => nothing to say")
        f, _ = alarm.find_execution_overruns([_run_obj(age_min=40 * 60)], big, NOW)
        self.assertEqual(len(f), 1)


class TestCensusAndExitCodes(unittest.TestCase):
    def test_a_clean_run_still_emits_a_census_for_every_mode(self):
        """A silent alarm is indistinguishable from a healthy system."""
        _, c1 = alarm.find_cron_deficits([_lane(fires=36)], NOW)
        _, c3 = alarm.find_execution_overruns(
            [_run_obj(age_min=1)], TestM3ExecutionOverrun.BASE, NOW)
        _, c4 = alarm.find_cadence_fidelity_gaps(
            [_lane(crons=TestM4CadenceFidelity.HOURLY, fires=10)], NOW)
        for name, census in (("M1", c1), ("M3", c3), ("M4", c4)):
            with self.subTest(mode=name):
                self.assertTrue(census, f"{name} emitted no census on the all-clear")

    def test_the_three_exit_codes_are_distinct(self):
        # 0 = clean, 1 = a choke, 2 = the detector is broken. Collapsing any pair makes a
        # broken detector indistinguishable from a healthy repo.
        clean = self._run(lanes=[_lane(fires=36)], live=[])
        alarmed = self._run(lanes=[_lane(fires=0)], live=[])
        broken = alarm.main(["--repo", "not-a-slug", "--dry-run"])
        self.assertEqual((clean, alarmed, broken), (0, 1, 2))

    def test_an_M4_only_finding_is_REPORTED_but_does_NOT_red_the_run(self):
        """M4 is chronic: true on a good day, and true until a workflow file is edited.
        Wiring it to exit 1 would leave this hourly lane permanently red, which mutes the
        two INCIDENT modes sharing it. The artifact is the deduped issue, not the red."""
        import contextlib
        import io
        # `*/10` declaring 142 and delivering 12 is the measured #4802 shape, and it is
        # exactly AT M1's capped expectation, so M1 is silent on it.
        chronic = [_lane(crons=("*/10 * * * *",), fires=alarm.capped_expectation(),
                         spacing_min=30)]
        self.assertEqual(alarm.find_cron_deficits(chronic, NOW)[0], [],
                         "fixture is vacuous unless M1 is silent on it")
        self.assertEqual(len(alarm.find_cadence_fidelity_gaps(chronic, NOW)[0]), 1)
        buf = io.StringIO()
        with contextlib.redirect_stdout(buf):
            rc = self._run(lanes=chronic, live=[])
        self.assertEqual(rc, 0, "a chronic finding must not red the incident lane")
        out = buf.getvalue()
        self.assertIn("M4-declared-cadence-unachievable", out,
                      "the finding must still reach the issue body")
        self.assertNotIn("::error::", out)

    def test_an_incident_still_reds_even_when_M4_is_also_firing(self):
        """ANTI-VACUITY for the exit rule above: the soft-exit must be scoped to M4, not
        an unconditional `return 0`."""
        import contextlib
        import io
        both = [_lane(crons=("*/10 * * * *",), fires=0)]
        self.assertEqual(len(alarm.find_cron_deficits(both, NOW)[0]), 1)
        self.assertEqual(len(alarm.find_cadence_fidelity_gaps(both, NOW)[0]), 1)
        buf = io.StringIO()
        with contextlib.redirect_stdout(buf):
            rc = self._run(lanes=both, live=[])
        self.assertEqual(rc, 1)
        self.assertIn("::error::", buf.getvalue())

    def test_an_empty_scan_set_is_fail_LOUD(self):
        """100% question, applied to M1: if every scheduled workflow were cron-only, M1's
        population would be empty. Reporting 'clean' over an empty population is how a
        detector reports health while watching nothing."""
        self.assertEqual(self._run(lanes=[], live=[]), 2)
        only_cron_only = [_lane(cron_only=True, in_scope=False)]
        self.assertEqual(self._run(lanes=only_cron_only, live=[]), 2)

    def test_the_two_empty_scan_set_guards_report_DISTINCT_causes(self):
        """They are not redundant: 'no workflow files at all' means the sparse-checkout
        dropped `.github/workflows` (a deploy defect), while 'nothing in scope' means the
        predicate or the repo changed shape. Collapsing them loses the more actionable
        diagnosis — and without this assertion, deleting the first guard SURVIVES, because
        the second one also exits 2 on an empty list."""
        import contextlib
        import io
        for lanes, expect in (([], "no workflows discovered"),
                              ([_lane(cron_only=True, in_scope=False)], "empty M1 scan set")):
            buf = io.StringIO()
            with contextlib.redirect_stdout(buf):
                rc = self._run(lanes=lanes, live=[])
            self.assertEqual(rc, 2)
            self.assertIn(expect, buf.getvalue())

    @staticmethod
    def _run(lanes, live, baselines=None, extra=()):
        state = {
            "repo": "o/r",
            "lanes": [dict(x, schedule_run_times=[t.strftime("%Y-%m-%dT%H:%M:%SZ")
                                                 for t in x["schedule_run_times"]])
                      for x in lanes],
            "live_runs": live,
            "baselines": baselines or {},
        }
        with tempfile.NamedTemporaryFile("w", suffix=".json", delete=False) as fh:
            json.dump(state, fh)
            path = fh.name
        return alarm.main(["--state-file", path, "--now", "2026-07-28T12:00:00Z",
                           "--dry-run", *extra])


class TestIssueBody(unittest.TestCase):
    def test_the_body_carries_the_dedupe_marker_and_self_identifies(self):
        body = alarm.render_issue_body(
            "o/r",
            [{"mode": "M3-execution-overrun", "workflow": "a.yml", "run_id": 1,
              "event": "push", "age_seconds": 99, "threshold_seconds": 60,
              "basis": "floor", "head_branch": "main"}],
            {"M3-execution-overrun": {"execution-overrun": 1}}, NOW)
        self.assertTrue(body.rstrip().endswith(
            f"<!-- {alarm.KEY_PREFIX}: o/r -->"))
        self.assertTrue(body.startswith("> 🤖"))
        self.assertIn("Census of every state exit", body)


# ---------------------------------------------------------------------------------
# 2. disjointness with #4368
# ---------------------------------------------------------------------------------
class TestScopeIsTheComplementOfCronLaneLiveness(unittest.TestCase):
    """M1's scope must be the exact set complement of `cron_lane_liveness.py`'s, so the
    two detectors partition the scheduled workflows: never both on one lane (duplicate
    issues), never neither (a hole)."""

    @staticmethod
    def _scheduled_workflows():
        out = {}
        for path in sorted(WORKFLOWS.glob("*.yml")):
            on = alarm.workflow_triggers(path.read_text())
            if "schedule" in on:
                out[path.name] = on
        return out

    def test_no_workflow_is_in_both_scopes(self):
        for name, on in self._scheduled_workflows().items():
            in_scope, _, cron_only = alarm.m1_scope(on)
            with self.subTest(workflow=name):
                self.assertFalse(in_scope and cron_only,
                                 f"{name} would be claimed by BOTH detectors")

    def test_every_scheduled_workflow_is_in_exactly_one_scope(self):
        for name, on in self._scheduled_workflows().items():
            in_scope, crons, cron_only = alarm.m1_scope(on)
            if not crons:
                continue  # a `schedule:` with no cron expression is neither's problem
            with self.subTest(workflow=name):
                self.assertTrue(in_scope or cron_only,
                                f"{name} would be claimed by NEITHER detector")

    def test_the_m1_population_over_the_real_tree_is_not_empty(self):
        """Anti-vacuity: if the predicate broke such that nothing was ever in scope, every
        assertion above would still pass. The measured shape is 13 in-scope lanes."""
        in_scope = [n for n, on in self._scheduled_workflows().items()
                    if alarm.m1_scope(on)[0]]
        self.assertGreaterEqual(len(in_scope), 5,
                                f"M1 scope collapsed to {in_scope}")

    def test_ci_yml_is_in_m1_scope(self):
        """ci.yml is the repo's largest capacity consumer AND carries a `schedule:`, but
        it has push/pull_request/merge_group triggers so #4368 structurally cannot see it.
        If this ever stops being true, the coordination note in both headers is wrong."""
        on = alarm.workflow_triggers((WORKFLOWS / "ci.yml").read_text())
        self.assertTrue(alarm.m1_scope(on)[0])
        self.assertFalse(alarm.m1_scope(on)[2])

    def test_a_cron_only_lane_from_the_real_tree_is_excluded(self):
        on = alarm.workflow_triggers((WORKFLOWS / "formal-alarm.yml").read_text())
        self.assertFalse(alarm.m1_scope(on)[0])
        self.assertTrue(alarm.m1_scope(on)[2])


# ---------------------------------------------------------------------------------
# 3. the YAML seam
# ---------------------------------------------------------------------------------
class TestWorkflowSeam(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.wf = yaml.safe_load(ALARM_WF.read_text())
        cls.text = ALARM_WF.read_text()
        cls.job = next(iter(cls.wf["jobs"].values()))
        cls.steps = cls.job["steps"]

    def test_it_is_triggered_by_schedule(self):
        on = self.wf.get(True, self.wf.get("on"))
        self.assertIn("schedule", on)
        self.assertTrue([s["cron"] for s in on["schedule"]])

    def test_the_selftest_step_runs_BEFORE_the_detector(self):
        """A detector is never trusted to watch anything before it has proved itself on
        this tick's code. Ordering, not mere presence."""
        idx = {}
        for i, step in enumerate(self.steps):
            run = str(step.get("run", ""))
            if "--self-test" in run:
                idx["selftest"] = i
            elif "ci_execution_latency_alarm.py" in run:
                idx.setdefault("detector", i)
        self.assertIn("selftest", idx)
        self.assertIn("detector", idx)
        self.assertLess(idx["selftest"], idx["detector"])

    def test_the_selftest_invocation_is_EXACTLY_the_expected_command(self):
        """EXACT match, not substring: `--self-test-DISABLED` contains `--self-test` and
        would survive a containment check while doing nothing."""
        runs = [str(s.get("run", "")).strip() for s in self.steps]
        self.assertIn("python3 scripts/ci_execution_latency_alarm.py --self-test", runs)

    def test_the_detector_invocation_is_EXACTLY_the_expected_command(self):
        runs = [str(s.get("run", "")).strip() for s in self.steps]
        self.assertIn("python3 scripts/ci_execution_latency_alarm.py", runs)

    def test_no_step_is_conditional_or_continue_on_error(self):
        """A step `if:` with no status function defaults to `success()`, so a failed
        earlier step silently skips it — and `continue-on-error` turns a fail-loud
        detector fail-open."""
        for step in self.steps:
            with self.subTest(step=step.get("name")):
                self.assertNotIn("if", step)
                self.assertNotIn("continue-on-error", step)
        self.assertNotIn("continue-on-error", self.job)

    def test_the_job_holds_no_arming_authority(self):
        """Structural, not procedural: the detector must be unable to label, promote or
        push even if its logic were subverted."""
        perms = self.job["permissions"]
        self.assertEqual(perms.get("contents"), "read")
        self.assertEqual(perms.get("issues"), "write")
        self.assertEqual(perms.get("actions"), "read")
        for forbidden in ("pull-requests", "checks", "id-token", "packages"):
            self.assertNotIn(forbidden, perms)
        self.assertEqual(self.wf.get("permissions"), {"contents": "read"})

    def test_every_action_use_is_sha_pinned(self):
        import re
        for step in self.steps:
            uses = step.get("uses")
            if uses:
                with self.subTest(uses=uses):
                    self.assertRegex(uses, r"@[0-9a-f]{40}$")

    def test_the_checkout_does_not_persist_credentials(self):
        for step in self.steps:
            if str(step.get("uses", "")).startswith("actions/checkout@"):
                self.assertIs(step["with"]["persist-credentials"], False)

    def test_the_checkout_fetches_the_workflows_dir_M1_reads(self):
        """M1 derives its expectation from the workflow files themselves. A sparse
        checkout that omits them would make every lane read as `not-scheduled` — a silent
        total blind spot rather than an error."""
        for step in self.steps:
            if str(step.get("uses", "")).startswith("actions/checkout@"):
                sparse = str(step["with"].get("sparse-checkout", ""))
                self.assertIn(".github/workflows", sparse)
                self.assertIn("scripts/ci_execution_latency_alarm.py", sparse)


class TestSuiteIsWiredIntoCI(unittest.TestCase):
    """Anti-vacuity anchor: without this the whole file could leave CI unnoticed."""

    def test_docs_quality_runs_this_suite(self):
        runs = []
        for job in yaml.safe_load(DOCS_QUALITY.read_text())["jobs"].values():
            for step in job.get("steps") or []:
                runs.append(str(step.get("run", "")).strip())
        self.assertIn(
            "python3 scripts/tests/test_ci_execution_latency_alarm.py", runs,
            "docs-quality.yml no longer invokes this suite — it would stop running")

    def test_that_call_site_is_unconditional(self):
        for job in yaml.safe_load(DOCS_QUALITY.read_text())["jobs"].values():
            for step in job.get("steps") or []:
                if "test_ci_execution_latency_alarm.py" in str(step.get("run", "")):
                    self.assertNotIn("if", step)
                    self.assertNotIn("continue-on-error", step)


class TestDeclaredNonGating(unittest.TestCase):
    """This lane REDs on a finding. That is only safe because it is DECLARED advisory."""

    def test_the_lane_is_declared_in_the_advisory_registry(self):
        registry = json.loads(REGISTRY_PATH.read_text())["jobs"]
        name = _job_name(ALARM_WF)
        self.assertIn(name, registry,
                      "an undeclared alarm lane GATES every merge on repo-wide CI health")
        entry = registry[name]
        for field in ("owner_bead", "promotion_criteria", "registered", "workflow",
                      "job_id"):
            self.assertIn(field, entry)
        self.assertEqual(entry["workflow"], ALARM_WF.name)

    def test_the_real_gate_treats_this_lane_as_advisory(self):
        sys.path.insert(0, str(REPO_ROOT / "scripts"))
        import ci_summary_gate as gate  # noqa: PLC0415
        gate.load_advisory_registry(str(REGISTRY_PATH))
        self.assertTrue(gate.is_advisory(_job_name(ALARM_WF)))
        # Anti-vacuity control: a name that is NOT declared must gate.
        self.assertFalse(gate.is_advisory("some undeclared lane that must gate"))


class TestSelfTestIsRunnable(unittest.TestCase):
    def test_the_embedded_self_test_passes(self):
        proc = subprocess.run(
            [sys.executable, "-B", str(SCRIPT), "--self-test"],
            capture_output=True, text=True, timeout=120,
            env={"PATH": "/usr/bin:/bin", "PYTHONDONTWRITEBYTECODE": "1"})
        self.assertEqual(proc.returncode, 0, proc.stdout + proc.stderr)


if __name__ == "__main__":
    unittest.main(verbosity=2)
