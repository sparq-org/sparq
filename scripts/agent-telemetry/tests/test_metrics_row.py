#!/usr/bin/env python3
# Hermetic regression tests for the metrics-collection harness
# (bead sq-lhwo.1, epic sq-lhwo).
#
# FULLY HERMETIC: imports metrics_row.py and drives it through the --from-json bundle
# path + pure helpers -- NO gh / roborev / bd / network calls. Exercises:
#   * the ref-event-derived no_rework (force-push AND post-first-push commits, NOT a
#     commit-message grep -- the amend/force-push gaming hole the review flagged),
#   * the COMPOSITE first-shot sub-flag logic (incl. the unknown/None paths),
#   * advisory checks NOT failing the gate (repo gate policy),
#   * roborev severity text-parsing,
#   * the cache-discounted effective_input_tokens formula,
#   * unattributed-vs-attributed token grading,
#   * opt-in price estimation,
#   * field-quality / caveat honesty metadata,
#   * deterministic JSON row shape.
#
# Run:  python3 scripts/agent-telemetry/tests/test_metrics_row.py
# (stdlib only; mirrors test_agent_telemetry.py.)

from __future__ import annotations

import importlib.util
import io
import json
import sys
import tempfile
import unittest
from contextlib import redirect_stdout
from pathlib import Path

HERE = Path(__file__).resolve().parent
HARNESS = HERE.parent / "metrics_row.py"
FIXTURE = HERE / "fixture_metrics_bundle.json"


def _load_module():
    spec = importlib.util.spec_from_file_location("metrics_row", HARNESS)
    assert spec and spec.loader
    mod = importlib.util.module_from_spec(spec)
    sys.modules["metrics_row"] = mod
    spec.loader.exec_module(mod)
    return mod


mr = _load_module()


def _load_fixture() -> dict:
    return json.loads(FIXTURE.read_text())


class TestFixtureBundle(unittest.TestCase):
    def setUp(self):
        self.row = mr.row_from_bundle(_load_fixture(), None)

    def test_join_keys_present(self):
        self.assertEqual(self.row["pr"], 4242)
        self.assertEqual(self.row["sha"], "merge000000000000000000000000000000000000")
        self.assertEqual(self.row["bead"], "sq-test.1")
        self.assertEqual(self.row["arm"], "treatment")
        self.assertEqual(self.row["surface"], "sparq-engine")
        self.assertEqual(self.row["schema_version"], mr.SCHEMA_VERSION)

    def test_rework_is_ref_event_derived(self):
        # The fixture has one force-push and one post-first-push commit.
        self.assertEqual(self.row["force_push_count"], 1)
        self.assertEqual(self.row["post_first_push_commits"], 1)
        self.assertFalse(self.row["no_rework"])

    def test_churn_passthrough(self):
        self.assertEqual(self.row["churn_added"], 120)
        self.assertEqual(self.row["churn_deleted"], 8)
        self.assertEqual(self.row["changed_files"], 4)

    def test_advisory_check_does_not_fail_gate(self):
        # 1 blocking SUCCESS + 1 advisory FAILURE -> ci_first_green stays True.
        self.assertTrue(self.row["ci_first_green"])
        self.assertEqual(self.row["ci_checks_failing"], 0)
        self.assertEqual(self.row["ci_advisory_failing"], 1)

    def test_roborev_severity_parsed(self):
        self.assertEqual(self.row["roborev_findings"], {"high": 0, "med": 1, "low": 2})
        self.assertEqual(self.row["roborev_verdict"], "F")
        self.assertEqual(self.row["roborev_reviews_found"], 1)

    def test_first_shot_false_when_rework_and_blocking_finding(self):
        # no_rework False AND a Med roborev finding -> first_shot False.
        self.assertFalse(self.row["first_shot"])
        sub = self.row["first_shot_subflags"]
        self.assertFalse(sub["no_rework"])
        self.assertFalse(sub["roborev_blocking_zero"])  # 1 Med finding
        self.assertTrue(sub["first_ci_attempt"])

    def test_tokens_attributed_and_effective_formula(self):
        # telemetry rollup in the fixture: fresh=1000, read=5000, write=2000.
        # effective = 1.0*1000 + 0.1*5000 + 1.25*2000 = 1000+500+2500 = 4000.
        self.assertEqual(self.row["tokens_in"], 1000)
        self.assertEqual(self.row["cache_read"], 5000)
        self.assertEqual(self.row["cache_write"], 2000)
        self.assertEqual(self.row["effective_input_tokens"], 4000.0)
        self.assertEqual(self.row["_field_quality"]["tokens"], "measured")

    def test_quality_placeholders_present_but_null(self):
        for k in (
            "coverage_delta",
            "mutation_score_delta",
            "conformance_floor_moved",
            "seeded_canary_find_rate",
            "post_merge_revert",
        ):
            self.assertIn(k, self.row)
            self.assertIsNone(self.row[k])
            self.assertEqual(self.row["_field_quality"][k], "placeholder")

    def test_honesty_metadata(self):
        self.assertIn("per_pr_token_attribution", self.row["_caveats"])
        self.assertIn("non_canonical", self.row["_caveats"])
        # field-quality grades every load-bearing column.
        fq = self.row["_field_quality"]
        self.assertEqual(fq["no_rework"], "measured")
        self.assertEqual(fq["roborev_findings"], "text_parsed")

    def test_row_is_json_serialisable_and_sorted_stable(self):
        a = json.dumps(self.row, sort_keys=True)
        b = json.dumps(mr.row_from_bundle(_load_fixture(), None), sort_keys=True)
        # collected_at differs across calls; strip it before comparing.
        da = json.loads(a)
        db = json.loads(b)
        da.pop("collected_at")
        db.pop("collected_at")
        self.assertEqual(da, db)


class TestReworkDerivation(unittest.TestCase):
    def _row(self, *, force_pushes, commits, created):
        github = {
            "createdAt": created,
            "commits": commits,
            "mergeCommit": {"oid": "abc"},
            "statusCheckRollup": [],
            "reviews": [],
            "_timeline": [{"event": "head_ref_force_pushed"}] * force_pushes,
        }
        bundle = {"pr": 1, "github": github}
        return mr.row_from_bundle(bundle, None)

    def test_clean_pr_no_rework(self):
        # All commits at/before PR creation, no force-push.
        row = self._row(
            force_pushes=0,
            commits=[{"committedDate": "2026-06-20T10:00:00Z"}],
            created="2026-06-20T10:05:00Z",
        )
        self.assertEqual(row["force_push_count"], 0)
        self.assertEqual(row["post_first_push_commits"], 0)
        self.assertTrue(row["no_rework"])

    def test_post_creation_commit_is_rework(self):
        row = self._row(
            force_pushes=0,
            commits=[
                {"committedDate": "2026-06-20T10:00:00Z"},
                {"committedDate": "2026-06-20T11:00:00Z"},  # after PR open
            ],
            created="2026-06-20T10:05:00Z",
        )
        self.assertEqual(row["post_first_push_commits"], 1)
        self.assertFalse(row["no_rework"])

    def test_amend_force_push_is_rework_even_without_new_commit(self):
        # The amend leaves NO extra commit (a message-grep would miss it), but the
        # force-push event is the rework signal. This is the gaming hole closed.
        row = self._row(
            force_pushes=1,
            commits=[{"committedDate": "2026-06-20T10:00:00Z"}],  # single, pre-open
            created="2026-06-20T10:05:00Z",
        )
        self.assertEqual(row["post_first_push_commits"], 0)
        self.assertEqual(row["force_push_count"], 1)
        self.assertFalse(row["no_rework"])  # force-push alone => rework


class TestFirstShotComposite(unittest.TestCase):
    def _compute(self, **over):
        base = {
            "ci_first_green": True,
            "no_rework": True,
            "review_changes_requested": 0,
            "roborev_findings": {"high": 0, "med": 0, "low": 0},
            "roborev_reviews_found": 1,
        }
        base.update(over)
        return mr.compute_first_shot(base)

    def test_all_green_first_shot_true(self):
        self.assertTrue(self._compute()["first_shot"])

    def test_any_known_false_makes_composite_false(self):
        self.assertFalse(self._compute(no_rework=False)["first_shot"])
        self.assertFalse(self._compute(ci_first_green=False)["first_shot"])
        self.assertFalse(
            self._compute(review_changes_requested=2)["first_shot"]
        )

    def test_high_or_med_roborev_blocks(self):
        self.assertFalse(
            self._compute(roborev_findings={"high": 1, "med": 0, "low": 0})["first_shot"]
        )
        self.assertFalse(
            self._compute(roborev_findings={"high": 0, "med": 1, "low": 0})["first_shot"]
        )

    def test_low_only_roborev_does_not_block(self):
        self.assertTrue(
            self._compute(roborev_findings={"high": 0, "med": 0, "low": 3})["first_shot"]
        )

    def test_unknown_roborev_yields_none_not_false(self):
        # No review found -> roborev_blocking_zero is unknown -> composite None when
        # all other known sub-flags pass (can't certify first-shot).
        out = self._compute(roborev_reviews_found=0, roborev_findings={"high": 0, "med": 0, "low": 0})
        self.assertIsNone(out["first_shot"])
        self.assertIsNone(out["first_shot_subflags"]["roborev_blocking_zero"])

    def test_known_false_dominates_unknown(self):
        # A hard CI failure shows False even if roborev is unknown.
        out = self._compute(ci_first_green=False, roborev_reviews_found=0)
        self.assertFalse(out["first_shot"])


class TestRoborevParsing(unittest.TestCase):
    def test_counts_high_med_low(self):
        text = (
            "## Review Findings\n"
            "- **Severity**: High\n---\n"
            "- **Severity**: Medium\n---\n"
            "- **Severity**: Med\n---\n"
            "- **Severity**: Low\n"
        )
        self.assertEqual(mr.parse_roborev_findings(text), {"high": 1, "med": 2, "low": 1})

    def test_no_issues(self):
        self.assertEqual(
            mr.parse_roborev_findings("No issues found.\nSummary: clean."),
            {"high": 0, "med": 0, "low": 0},
        )

    def test_empty(self):
        self.assertEqual(mr.parse_roborev_findings(""), {"high": 0, "med": 0, "low": 0})


class TestEffectiveTokens(unittest.TestCase):
    def test_formula(self):
        # 1.0*100 + 0.1*1000 + 1.25*400 = 100 + 100 + 500 = 700
        self.assertEqual(mr.effective_input_tokens(100, 1000, 400), 700.0)

    def test_tokens_from_telemetry_reuses_rollup(self):
        report = {
            "rollup": {
                "input_tokens": 200,
                "output_tokens": 50,
                "cache_read_input_tokens": 1000,
                "cache_creation_input_tokens": 400,
                "cache_hit_ratio": 0.71,
            },
            "wave_duration_seconds": 12.5,
            "agent_count": 3,
            "subagent_count": 2,
            "session_ids": ["s1"],
        }
        out = mr.tokens_from_telemetry(report)
        self.assertEqual(out["tokens_in"], 200)
        self.assertEqual(out["cache_hit_ratio"], 0.71)
        # 1.0*200 + 0.1*1000 + 1.25*400 = 200 + 100 + 500 = 800
        self.assertEqual(out["effective_input_tokens"], 800.0)


class TestUnattributedTokens(unittest.TestCase):
    def test_no_telemetry_means_unattributed_nulls(self):
        bundle = {
            "pr": 7,
            "github": {
                "createdAt": "2026-06-20T10:00:00Z",
                "commits": [],
                "mergeCommit": {"oid": "z"},
                "statusCheckRollup": [],
                "reviews": [],
                "_timeline": [],
            },
        }
        row = mr.row_from_bundle(bundle, None)
        self.assertIsNone(row["tokens_in"])
        self.assertIsNone(row["effective_input_tokens"])
        self.assertEqual(row["_field_quality"]["tokens"], "unattributed")


class TestPricing(unittest.TestCase):
    def test_opt_in_price_estimate(self):
        bundle = _load_fixture()
        prices = {"input": 15.0, "output": 75.0, "cache_read": 1.5, "cache_write": 18.75}
        row = mr.row_from_bundle(bundle, prices)
        # tokens_in=1000, out=300, read=5000, write=2000 (per fixture telemetry).
        # (1000*15 + 300*75 + 5000*1.5 + 2000*18.75)/1e6
        expect = (1000 * 15 + 300 * 75 + 5000 * 1.5 + 2000 * 18.75) / 1_000_000.0
        self.assertAlmostEqual(row["usd_est"], round(expect, 6))
        self.assertEqual(row["_field_quality"]["usd_est"], "opt_in")

    def test_no_price_means_null_usd(self):
        row = mr.row_from_bundle(_load_fixture(), None)
        self.assertIsNone(row["usd_est"])
        self.assertEqual(row["_field_quality"]["usd_est"], "placeholder")


class TestCli(unittest.TestCase):
    def test_from_json_dry_run_prints_row(self):
        with tempfile.TemporaryDirectory() as d:
            bp = Path(d) / "bundle.json"
            bp.write_text(json.dumps(_load_fixture()))
            buf = io.StringIO()
            with redirect_stdout(buf):
                rc = mr.main(["--from-json", str(bp), "--dry-run"])
            self.assertEqual(rc, 0)
            out = json.loads(buf.getvalue())
            self.assertEqual(out["pr"], 4242)

    def test_from_json_appends_to_out(self):
        with tempfile.TemporaryDirectory() as d:
            bp = Path(d) / "bundle.json"
            bp.write_text(json.dumps(_load_fixture()))
            outp = Path(d) / "sub" / "rows.jsonl"
            buf = io.StringIO()
            with redirect_stdout(buf):
                rc = mr.main(["--from-json", str(bp), "--out", str(outp)])
            self.assertEqual(rc, 0)
            self.assertTrue(outp.exists())
            line = outp.read_text().strip()
            self.assertEqual(json.loads(line)["pr"], 4242)

    def test_no_args_errors(self):
        rc = mr.main([])
        self.assertEqual(rc, 2)


if __name__ == "__main__":
    unittest.main(verbosity=2)
