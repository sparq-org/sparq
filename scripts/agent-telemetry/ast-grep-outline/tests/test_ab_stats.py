#!/usr/bin/env python3
# Hermetic tests for the A/B statistics + verdict emitter (bead sq-lhwo.4,
# epic sq-lhwo). 🤖 SPARQ agent.
#
# Fully hermetic (stdlib unittest, NO scipy/numpy, NO network, NO real transcripts):
# imports ab_stats.py and exercises the Wilcoxon signed-rank math against a textbook
# worked example, the bootstrap CI determinism (seeded), break-even N* incl. the
# infinite-denominator guard, and — load-bearing — the §5.4 KILL criteria + the
# §5.6 verdict object's honest gating (a token win alone must NOT recommend adoption;
# an unmeasured quality pair must NOT certify non-regression; a cache-only artifact
# must NOT count as a token win).
#
# Run:  python3 scripts/agent-telemetry/ast-grep-outline/tests/test_ab_stats.py

from __future__ import annotations

import importlib.util
import math
import sys
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
MODULE = HERE.parent / "ab_stats.py"

spec = importlib.util.spec_from_file_location("ab_stats", MODULE)
ab = importlib.util.module_from_spec(spec)
assert spec and spec.loader
spec.loader.exec_module(ab)


class WilcoxonTest(unittest.TestCase):
    def test_textbook_example(self):
        # Classic worked example (Wikipedia Wilcoxon signed-rank): the |diff| ranks
        # and signs give W = 18 over n=9 non-zero pairs; the normal approx is
        # non-significant at 0.05. We assert the W statistic and a sane p in [0,1].
        deltas = [125, 115, 130, 140, 140, 115, 140, 125, 140, 135]
        baseline = [110, 122, 125, 120, 140, 124, 123, 137, 135, 145]
        d = [a - b for a, b in zip(deltas, baseline)]
        res = ab.wilcoxon_signed_rank(d)
        self.assertEqual(res["n_nonzero"], 9)  # one zero pair dropped
        self.assertAlmostEqual(res["W"], 18.0, places=6)
        self.assertIsNotNone(res["p"])
        self.assertTrue(0.0 <= res["p"] <= 1.0)
        self.assertGreater(res["p"], 0.05)  # textbook: not significant

    def test_all_positive_is_significant(self):
        # A large, consistently-positive delta set must be highly significant.
        d = [float(x) for x in range(50, 50 + 30)]  # 30 positive deltas
        res = ab.wilcoxon_signed_rank(d)
        self.assertEqual(res["n_nonzero"], 30)
        self.assertLess(res["p"], 0.05)

    def test_zero_deltas_dropped(self):
        res = ab.wilcoxon_signed_rank([0, 0, 0])
        self.assertEqual(res["n_nonzero"], 0)
        self.assertIsNone(res["p"])


class BootstrapTest(unittest.TestCase):
    def test_seeded_is_deterministic(self):
        d = [10.0, 12.0, 9.0, 11.0, 13.0, 8.0, 14.0, 10.0]
        lo1, hi1 = ab.bootstrap_median_ci(d, resamples=2000, seed=42)
        lo2, hi2 = ab.bootstrap_median_ci(d, resamples=2000, seed=42)
        self.assertEqual((lo1, hi1), (lo2, hi2))
        self.assertLessEqual(lo1, hi1)

    def test_all_positive_ci_lower_above_zero(self):
        d = [5.0] * 20
        lo, hi = ab.bootstrap_median_ci(d, resamples=1000, seed=1)
        self.assertGreater(lo, 0)


class BreakEvenTest(unittest.TestCase):
    def test_finite(self):
        # ingest cost 1000, save 100/use -> N* = 10.
        r = ab.break_even_n(c_ingest=1000, c_docread=150, c_query=50)
        self.assertFalse(r["break_even_infinite"])
        self.assertEqual(r["break_even_N"], 10)

    def test_infinite_when_query_costs_more(self):
        # If querying costs as much as / more than reading, it never pays back.
        r = ab.break_even_n(c_ingest=1000, c_docread=50, c_query=60)
        self.assertTrue(r["break_even_infinite"])
        self.assertIsNone(r["break_even_N"])


class VerdictTest(unittest.TestCase):
    def _big_win_pairs(self, n=30):
        # arm A ~ full reads (large), arm B ~ outlines (tiny) -> a real >20% win.
        arm_a = [10000.0 + i for i in range(n)]
        arm_b = [200.0 + i for i in range(n)]
        return arm_a, arm_b

    def test_token_win_alone_does_not_recommend_adopt(self):
        a, b = self._big_win_pairs()
        # No quality_delta supplied -> cannot certify non-regression.
        v = ab.build_verdict(a, b)
        self.assertTrue(v["token_win"])  # the token bar is cleared
        self.assertIsNone(v["quality_non_regression"])
        self.assertFalse(v["recommend_adopt"])  # but adoption is WITHHELD
        self.assertFalse(v["honest"])  # quality unmeasured -> not a clean verdict

    def test_quality_regression_blocks_adopt(self):
        a, b = self._big_win_pairs()
        quality = {
            "exec_acc": {"delta_ci_lo": -0.1, "model_graded": True},  # accuracy DROPPED
            "hallucination_rate": {"delta": 0.0},
        }
        cost = {"c_ingest": 100, "c_docread": 9000, "c_query": 200, "realistic_horizon": 1000}
        v = ab.build_verdict(
            a, b, quality_delta=quality, cost_model=cost, measurement_fidelity="full-session"
        )
        self.assertTrue(v["token_win"])
        self.assertFalse(v["quality_non_regression"])
        self.assertFalse(v["recommend_adopt"])  # KILL3 trips

    def test_clean_win_full_session_recommends_adopt(self):
        a, b = self._big_win_pairs()
        quality = {
            "exec_acc": {"delta_ci_lo": 0.0, "model_graded": True},  # model-graded, no regression
            "hallucination_rate": {"delta": 0.0},
        }
        cost = {"c_ingest": 100, "c_docread": 9000, "c_query": 200, "realistic_horizon": 1000}
        v = ab.build_verdict(
            a, b, quality_delta=quality, cost_model=cost, measurement_fidelity="full-session"
        )
        self.assertTrue(v["token_win"])
        self.assertTrue(v["quality_non_regression"])
        self.assertTrue(v["quality_fully_graded"])
        self.assertFalse(v["provisional"])
        self.assertFalse(v["break_even_infinite"])
        self.assertTrue(v["honest"])
        self.assertTrue(v["recommend_adopt"])

    def test_proxy_run_is_provisional_and_cannot_adopt(self):
        # A read-payload-proxy run with a clean DETERMINISTIC quality pair (not
        # model-graded) clears the token bar but MUST stay provisional -> no adopt.
        # This is the PREREG honest off-ramp.
        a, b = self._big_win_pairs()
        quality = {
            "exec_acc": {"delta_ci_lo": 0.0, "model_graded": False},  # deterministic only
            "hallucination_rate": {"delta": 0.0},
        }
        cost = {"c_ingest": 100, "c_docread": 9000, "c_query": 200, "realistic_horizon": 1000}
        v = ab.build_verdict(
            a, b, quality_delta=quality, cost_model=cost,
            measurement_fidelity="read-payload-proxy",
        )
        self.assertTrue(v["token_win"])
        self.assertTrue(v["quality_non_regression"])  # deterministic axis clean
        self.assertFalse(v["quality_fully_graded"])
        self.assertTrue(v["provisional"])
        self.assertFalse(v["recommend_adopt"])  # WITHHELD: provisional
        self.assertTrue(v["honest"])  # honest precisely because it does not over-claim

    def test_full_session_but_not_model_graded_stays_provisional(self):
        # Even a full-session run cannot adopt if the quality axis is not model-graded.
        a, b = self._big_win_pairs()
        quality = {
            "exec_acc": {"delta_ci_lo": 0.0, "model_graded": False},
            "hallucination_rate": {"delta": 0.0},
        }
        cost = {"c_ingest": 100, "c_docread": 9000, "c_query": 200, "realistic_horizon": 1000}
        v = ab.build_verdict(
            a, b, quality_delta=quality, cost_model=cost, measurement_fidelity="full-session"
        )
        self.assertTrue(v["provisional"])
        self.assertFalse(v["recommend_adopt"])

    def test_under_min_n_cannot_win(self):
        # < min_n tasks -> the §5.1 power floor is unmet; token_win must be False
        # regardless of the deltas.
        a, b = self._big_win_pairs(n=12)
        quality = {"exec_acc": {"delta_ci_lo": 0.0}, "hallucination_rate": {"delta": 0.0}}
        v = ab.build_verdict(a, b, quality_delta=quality)
        self.assertFalse(v["token_win"])  # underpowered by construction
        self.assertFalse(v["recommend_adopt"])

    def test_cache_artifact_only_is_not_a_win(self):
        # The cache-discounted delta is positive, but the NOMINAL fresh input did NOT
        # drop (arm A just got cache-warmth for free). §5.4: that is not a win.
        a = [1000.0] * 30
        b = [700.0] * 30  # B looks cheaper in effective tokens
        fresh_a = [500.0] * 30
        fresh_b = [500.0] * 30  # ...but nominal fresh is identical -> artifact
        v = ab.build_verdict(
            a, b, nominal_fresh_a=fresh_a, nominal_fresh_b=fresh_b,
            quality_delta={"exec_acc": {"delta_ci_lo": 0.0}, "hallucination_rate": {"delta": 0.0}},
        )
        self.assertTrue(v["cache_artifact_only"])
        self.assertFalse(v["token_win"])
        self.assertFalse(v["honest"])

    def test_paired_length_mismatch_raises(self):
        with self.assertRaises(ValueError):
            ab.build_verdict([1.0, 2.0], [1.0])

    def test_verdict_schema_complete(self):
        a, b = self._big_win_pairs()
        v = ab.build_verdict(a, b)
        # The §5.6 verdict object's load-bearing keys must all be present.
        for k in (
            "token_win", "token_delta_median_pct", "token_delta_ci",
            "quality_delta", "break_even_N", "break_even_infinite",
            "honest", "recommend_adopt", "provisional", "measurement_fidelity",
            "quality_fully_graded",
        ):
            self.assertIn(k, v)
        self.assertEqual(len(v["token_delta_ci"]), 2)


if __name__ == "__main__":
    unittest.main(verbosity=2)
