#!/usr/bin/env python3
# [OPUS-5] issue #5250 — pins scripts/ci_mergegroup_poles.py, the reproducible
# `merge_group` wall-clock POLE ranker. 🤖 SPARQ agent.
#
# WHAT #5250 IS ABOUT, and therefore what this suite has to protect. Issue #3005 sized the
# docs/deploy fast lane off a "CodeQL-rust ~20-40 min" figure that codeql.yml's own header
# had already refuted by measurement. The defect was not the arithmetic — it was that a
# duration had no owner, no re-derivation and no test, so it rotted and kept steering work
# at a non-pole. Replacing it with a fresh hand-measured number would reproduce the defect
# one sample later. The tool exists so the ranking can be RE-DERIVED; this suite exists so
# the derivation cannot go quietly wrong, because every failure mode of a measurement tool
# is SILENT — a wrong median looks exactly like a right one.
#
# The five correctness properties are tested here at the seam that would actually break:
#   1/2. The EVENT and SUCCESS filters, tested by their BIAS DIRECTION rather than by a
#        equality on a fixture. Leaking push runs or cancelled runs does not crash; it
#        shifts a median, and the cancelled-run leak shifts it DOWNWARD, which erases a
#        pole rather than inventing one. So each is asserted as "the polluted sample must
#        not produce the polluted answer".
#   3.   `max` not `sum`, asserted as an INEQUALITY against the sum, so the test cannot be
#        satisfied by a fixture where the two happen to coincide.
#   4.   Structural-vs-observed reconciliation, including the specific `codeql.yml`
#        shape §6 calls out (declares the trigger, produces no check-run) — a lane that
#        must never be rendered as a cheap 0 m lane.
#   5.   Fail-loud on an empty sample.
#
# PLUS the invariant that cannot be tested inside the script: its structural lane parser
# must agree EXACTLY with scripts/tests/test_mergequeue_cache_posture.triggers_on_merge_group,
# the canonical definition that test_mergequeue_lane_inventory.py imports rather than
# forks. A production tool must not import from scripts/tests/, so the agreement is
# asserted here as SET EQUALITY over the REAL .github/workflows tree — if either parser
# drifts, or a workflow is added whose `on:` block the two read differently, this reds.
# That check is deliberately run over real files, not fixtures: fixtures would only prove
# the two agree on cases I thought of.
#
# Hermetic: stdlib only, no gh, no network, no cargo.
# Run:  python3 scripts/tests/test_ci_mergegroup_poles.py

from __future__ import annotations

import importlib.util
import subprocess
import sys
import unittest
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent.parent
SCRIPT = REPO_ROOT / "scripts" / "ci_mergegroup_poles.py"
WORKFLOWS = REPO_ROOT / ".github" / "workflows"
DOCS_QUALITY = REPO_ROOT / ".github" / "workflows" / "docs-quality.yml"

sys.path.insert(0, str(Path(__file__).resolve().parent))

from test_mergequeue_cache_posture import (  # noqa: E402
    triggers_on_merge_group as canonical_triggers_on_merge_group,
)


def _load():
    spec = importlib.util.spec_from_file_location("ci_mergegroup_poles", SCRIPT)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


poles = _load()


class TestEventAndSuccessFilters(unittest.TestCase):
    """Properties 1 and 2 — the filters that decide whether the answer means anything."""

    def test_push_runs_cannot_enter_the_ranking(self):
        # A slow push run of the SAME workflow must not become its merge_group pole.
        runs = [
            poles._run_obj("ci.yml", 40.0, event="push"),
            poles._run_obj("ci.yml", 6.0),
        ]
        stats = poles.summarize(poles.usable_runs(runs))
        self.assertEqual(stats["ci.yml"]["n"], 1)
        self.assertEqual(stats["ci.yml"]["median"], 360.0)
        self.assertLess(stats["ci.yml"]["max"], 40.0 * 60)

    def test_schedule_and_pull_request_events_are_excluded(self):
        for event in ("schedule", "pull_request", "workflow_dispatch", "push"):
            with self.subTest(event=event):
                self.assertEqual(
                    poles.usable_runs([poles._run_obj("ci.yml", 9.0, event=event)]), []
                )

    def test_cancelled_runs_do_not_drag_the_median_down(self):
        # The queue cancels waves mid-flight; those runs are truncated lower bounds.
        # Bias direction matters: a leak here makes a real pole look fast.
        truncated = [poles._run_obj("ci.yml", 1.0, conclusion="cancelled")] * 5
        real = [poles._run_obj("ci.yml", 18.0) for _ in range(3)]
        stats = poles.summarize(poles.usable_runs(truncated + real))
        self.assertEqual(stats["ci.yml"]["n"], 3)
        self.assertEqual(stats["ci.yml"]["median"], 18.0 * 60)

    def test_failed_and_skipped_runs_are_excluded(self):
        for conclusion in ("failure", "cancelled", "skipped", "timed_out", None):
            with self.subTest(conclusion=conclusion):
                self.assertEqual(
                    poles.usable_runs(
                        [poles._run_obj("ci.yml", 9.0, conclusion=conclusion)]
                    ),
                    [],
                )


class TestMaxNotSum(unittest.TestCase):
    """Property 3 — the entry wall is the slowest lane, not the total."""

    def setUp(self):
        self.stats = poles.summarize(poles.usable_runs([
            poles._run_obj("ci.yml", 12.0), poles._run_obj("ci.yml", 12.0),
            poles._run_obj("feature-matrix.yml", 5.0),
            poles._run_obj("feature-matrix.yml", 5.0),
            poles._run_obj("docs-quality.yml", 0.5),
            poles._run_obj("docs-quality.yml", 0.5),
        ]))

    def test_entry_wall_is_the_slowest_lane(self):
        self.assertEqual(poles.entry_wall(self.stats)["entry_wall_median"], 12.0 * 60)

    def test_entry_wall_is_strictly_below_the_sum(self):
        # Asserted as an inequality so a fixture where max == sum cannot satisfy it.
        wall = poles.entry_wall(self.stats)["entry_wall_median"]
        self.assertLess(wall, sum(s["median"] for s in self.stats.values()))

    def test_ranking_orders_by_median_descending(self):
        self.assertEqual(
            [s["workflow"] for s in poles.rank_poles(self.stats)],
            ["ci.yml", "feature-matrix.yml", "docs-quality.yml"],
        )

    def test_top_poles_are_the_slowest_lanes(self):
        ranked = poles.rank_poles(self.stats)[: poles.TOP_POLES]
        self.assertEqual(ranked[0]["workflow"], "ci.yml")
        self.assertEqual(len(ranked), 3)

    def test_ranking_is_a_total_order(self):
        # Equal medians must still order deterministically, or the report text churns
        # between two runs over the same sample.
        tied = poles.summarize(poles.usable_runs([
            poles._run_obj("z.yml", 4.0), poles._run_obj("a.yml", 4.0),
        ]))
        self.assertEqual(
            [s["workflow"] for s in poles.rank_poles(tied)], ["a.yml", "z.yml"]
        )


class TestReconciliation(unittest.TestCase):
    """Property 4 — trigger presence is not a check-run."""

    def setUp(self):
        self.stats = poles.summarize(poles.usable_runs([poles._run_obj("ci.yml", 10.0)]))

    def test_triggering_lane_with_no_runs_is_unobserved_not_fast(self):
        # The codeql.yml-while-disabled_manually shape. "Cheap" and "absent" are opposite
        # conclusions for a fast-lane decision, so this must never render as 0 m.
        rec = poles.reconcile({"ci.yml", "codeql.yml"}, self.stats)
        self.assertEqual(rec["unobserved"], ["codeql.yml"])
        self.assertNotIn(
            "codeql.yml", {s["workflow"] for s in poles.rank_poles(self.stats)}
        )

    def test_runs_from_an_untriggered_workflow_are_reported_separately(self):
        rec = poles.reconcile({"ci.yml"}, poles.summarize(poles.usable_runs([
            poles._run_obj("ci.yml", 10.0), poles._run_obj("bench.yml", 4.0),
        ])))
        self.assertEqual(rec["untriggered"], ["bench.yml"])

    def test_a_fully_reconciled_sample_reports_neither(self):
        rec = poles.reconcile({"ci.yml"}, self.stats)
        self.assertEqual(rec["unobserved"], [])
        self.assertEqual(rec["untriggered"], [])


class TestFailLoud(unittest.TestCase):
    """Property 5 — never report a confident ranking over nothing."""

    def test_percentile_over_empty_sample_raises(self):
        with self.assertRaises(poles.PoleError):
            poles.percentile([], 0.5)

    def test_structural_lanes_raises_on_missing_directory(self):
        with self.assertRaises(poles.PoleError):
            poles.structural_lanes(REPO_ROOT / "no" / "such" / "dir")

    def test_structural_lanes_raises_when_no_lane_triggers(self):
        # An empty lane set means the parser broke, not that the queue is empty.
        with self.assertRaises(poles.PoleError):
            poles.structural_lanes(REPO_ROOT / "scripts" / "tests" / "fixtures")

    def test_bad_timestamp_raises_rather_than_scoring_zero(self):
        with self.assertRaises(poles.PoleError):
            poles._ts("not-a-timestamp")

    def test_negative_and_missing_spans_are_dropped_not_scored(self):
        self.assertIsNone(poles.run_duration({}))
        self.assertIsNone(poles.run_duration(
            {"run_started_at": "2026-08-01T12:10:00Z",
             "updated_at": "2026-08-01T12:00:00Z"}
        ))


class TestJobDecomposition(unittest.TestCase):
    def test_skipped_jobs_are_not_scored_as_instant(self):
        # Selection-skipping is what these lanes DO on most entries; scoring a skipped
        # job at 0 s would report the heaviest workflow as the cheapest.
        summary = poles.summarize_jobs([
            poles._job_obj("build", 9.0),
            poles._job_obj("test-shard", 0.0, conclusion="skipped"),
        ])
        self.assertEqual([j["job"] for j in summary], ["build"])

    def test_jobs_rank_by_median_descending(self):
        summary = poles.summarize_jobs([
            poles._job_obj("quick-gates", 1.0),
            poles._job_obj("build", 8.0),
            poles._job_obj("build", 10.0),
        ])
        self.assertEqual([j["job"] for j in summary], ["build", "quick-gates"])
        self.assertEqual(summary[0]["n"], 2)


class TestLaneParserAgreesWithCanonicalDefinition(unittest.TestCase):
    """The invariant the script cannot test on itself.

    test_mergequeue_lane_inventory.py imports the canonical parser rather than forking it
    "so the two suites can never disagree about what triggers on merge_group means". A
    production tool cannot import from scripts/tests/, so the same guarantee is enforced
    here — over the REAL workflow tree, so a newly-added workflow the two parsers read
    differently reds on arrival rather than at the next hand audit.
    """

    def test_set_equality_over_the_real_workflow_tree(self):
        mine = poles.structural_lanes(WORKFLOWS)
        canonical = {
            p.name for p in sorted(WORKFLOWS.glob("*.yml"))
            if canonical_triggers_on_merge_group(p)
        }
        self.assertEqual(mine, canonical)

    def test_the_real_tree_is_a_non_trivial_population(self):
        # Guards the check above from going vacuous if both parsers returned nothing.
        self.assertGreater(len(list(WORKFLOWS.glob("*.yml"))), 20)
        self.assertGreaterEqual(len(poles.structural_lanes(WORKFLOWS)), 3)

    def test_commented_out_trigger_is_not_counted(self):
        self.assertFalse(poles.triggers_on_merge_group(
            "name: x\non:\n  push:\n  # merge_group: removed 2026-07-18\njobs: {}\n"
        ))

    def test_top_level_comment_does_not_truncate_the_on_block(self):
        # The comment-awareness that is actually LOAD-BEARING. A comment at column 0
        # inside the `on:` block would otherwise read as the next top-level key and end
        # the block early, hiding a trigger declared below it — silently dropping a real
        # gating lane from the ranking. (The `_is_comment` filter on the trigger match is
        # redundant by construction: `^\s+merge_group:` cannot match a `#` line.)
        self.assertTrue(poles.triggers_on_merge_group(
            "name: x\non:\n  push:\n"
            "# 2026-07-18 directive: several lanes dropped merge_group\n"
            "  merge_group:\njobs: {}\n"
        ))

    def test_merge_group_as_a_job_name_is_not_a_trigger(self):
        self.assertFalse(poles.triggers_on_merge_group(
            "name: x\non:\n  push:\njobs:\n  merge_group:\n    runs-on: ubuntu-latest\n"
        ))


class TestReportSurfaces(unittest.TestCase):
    def setUp(self):
        stats = poles.summarize(poles.usable_runs([
            poles._run_obj("ci.yml", 11.0), poles._run_obj("ci.yml", 13.0),
        ]))
        self.report = poles.render_workflow_report({
            "repo": "o/r", "sample_size": 2, "runs_listed": 2,
            "ranking": poles.rank_poles(stats), **poles.entry_wall(stats),
            "unobserved": ["codeql.yml"], "untriggered": [],
            "top_poles": ["ci.yml"],
        })

    def test_report_states_the_max_not_sum_rule(self):
        self.assertIn("max` over lanes", self.report)

    def test_report_warns_against_reading_unobserved_lanes_as_cheap(self):
        self.assertIn("codeql.yml", self.report)
        self.assertIn("do NOT read these as cheap", self.report)

    def test_report_carries_its_own_re_derivation_command(self):
        # The whole point of #5250: a number a reader can re-check, not a dated figure.
        self.assertIn("python3 scripts/ci_mergegroup_poles.py", self.report)

    def test_thin_samples_are_flagged_in_the_table(self):
        self.assertIn("thin", self.report)


class TestWiring(unittest.TestCase):
    def test_script_self_test_passes(self):
        proc = subprocess.run(
            [sys.executable, str(SCRIPT), "--self-test"],
            capture_output=True, text=True, timeout=120,
        )
        self.assertEqual(proc.returncode, 0, proc.stdout + proc.stderr)

    def test_self_test_is_wired_into_a_gating_job(self):
        # The estimator is only protected if this suite actually runs in CI.
        self.assertIn(
            "scripts/tests/test_ci_mergegroup_poles.py", DOCS_QUALITY.read_text()
        )

    def test_bad_arguments_are_rejected(self):
        for bad in (["--limit", "0"], ["--job-runs", "0"]):
            with self.subTest(bad=bad):
                proc = subprocess.run(
                    [sys.executable, str(SCRIPT), *bad],
                    capture_output=True, text=True, timeout=120,
                )
                self.assertEqual(proc.returncode, 1, proc.stdout + proc.stderr)

    def test_tool_makes_no_network_call_in_self_test_mode(self):
        # A measurement tool that reached the network from --self-test could not be run
        # as a gate. Asserted structurally: the gh entrypoint is never called on this path.
        source = SCRIPT.read_text()
        self_test_body = source.split("def _self_test(")[1].split("\ndef main(")[0]
        self.assertNotIn("_gh(", self_test_body)
        self.assertNotIn("fetch_", self_test_body)


if __name__ == "__main__":
    unittest.main(verbosity=2)
