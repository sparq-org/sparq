#!/usr/bin/env python3
# [OPUS-5] sq-2489d.6 (epic sq-2489d). 🤖 SPARQ agent — self-test for the GenAI-KB
# Phase 7 per-capability verdict emitter (`prov_ab.py`).
#
# EVERYTHING BELOW IS SYNTHETIC. The fixtures are generated in-process, are named
# `syn-*`, and are NOT a measurement of anything: they exist to prove the ANALYZER
# behaves — that the §5.6 object has the right shape, that the honesty gate refuses
# every unmet precondition individually, that the kill criteria fire, and (crucially)
# that `honest=true AND recommend_adopt=true` is REACHABLE, so the gate is not a
# permanently-red rubber stamp. No real host, model, or transcript is involved.
#
# Run: python3 bench/pkg-dogfood/test_prov_ab.py

from __future__ import annotations

import json
import sys
import tempfile
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))

import prov_ab  # noqa: E402
from stats import MIN_N  # noqa: E402

SRC = "https://sparq.dev/ns/pkg/kb#syn-source"
ANCHOR = "https://sparq.dev/ns/pkg/kb#syn-anchor"

N_TRAP = MIN_N + 2  # comfortably over the frozen power bar, per class
N_ORDINARY = 6
N_MISS = 2  # traps the treatment arm still fails — a perfect arm would be a smell


def synth_tasks() -> tuple[list[dict], list[dict]]:
    """A synthetic task set + its outcome classes: `stale-fact-trap`s (the citations
    axis), `abstain-trap`s (the hedging axis), and plain tasks that only guard
    non-regression."""
    tasks, classes = [], []
    for i in range(N_TRAP):
        tasks.append({"id": f"syn-stale-{i:02d}", "stratum": "point-lookup", "gold_keys": ["alpha"]})
        classes.append(
            {"id": f"syn-stale-{i:02d}", "outcome_class": "stale-fact-trap", "catch_keys": ["contradicts the source"]}
        )
    for i in range(N_TRAP):
        tasks.append({"id": f"syn-abstain-{i:02d}", "stratum": "negative", "gold_keys": ["none"]})
        classes.append({"id": f"syn-abstain-{i:02d}", "outcome_class": "abstain-trap", "catch_keys": []})
    for i in range(N_ORDINARY):
        tasks.append({"id": f"syn-plain-{i:02d}", "stratum": "multi-hop", "gold_keys": ["beta"]})
        classes.append({"id": f"syn-plain-{i:02d}", "outcome_class": "ordinary", "catch_keys": []})
    return tasks, classes


def synth_tokens(tasks: list[dict], overhead: float) -> list[dict]:
    """One token row per (task, arm). The treatment arms cost `overhead` more than the
    inert baseline — citations and hedges are extra bytes by construction."""
    rows = []
    for i, t in enumerate(tasks):
        base_in, base_out = 40_000 + 100 * i, 500 + 10 * i
        rows.append(
            {"task": t["id"], "arm": "P0", "eff_input_tokens": base_in, "output_tokens": base_out}
        )
        for arm in ("P1", "P2"):
            rows.append(
                {
                    "task": t["id"],
                    "arm": arm,
                    "eff_input_tokens": round(base_in * (1 + overhead)),
                    "output_tokens": round(base_out * (1 + overhead)),
                }
            )
    return rows


def synth_answers(
    tasks: list[dict],
    classes: list[dict],
    *,
    lift: bool = True,
    wired: bool = True,
    fabricate: bool = False,
    over_abstain: int = 0,
) -> list[dict]:
    """Per-(task, arm) answer records. The baseline is the INERT PKG: correct on the
    everyday tasks, blind to the seeded traps. The treatment arms catch most traps.

    `over_abstain` = how many ANSWERABLE tasks the hedging arm abstains on, chosen so
    the baseline was already wrong there. Both arms then score 0 on those tasks, so the
    mean ordinary accuracy is unchanged — the regression is visible ONLY as a drop in
    abstention precision."""
    by_class = {c["id"]: c for c in classes}
    recs = []
    ordinary_seen = 0
    for i, t in enumerate(tasks):
        klass = by_class[t["id"]]["outcome_class"]
        catches = lift and i >= N_MISS  # the first few traps are missed even by treatment
        gold = t["gold_keys"][0]
        cite = [SRC if not fabricate else "https://sparq.dev/ns/pkg/kb#syn-not-in-graph", ANCHOR]

        if klass == "stale-fact-trap":
            base = f"The recorded value is {gold}."
            treat_p1 = base + (" This contradicts the source of record." if catches else "")
            recs += [
                {"task": t["id"], "arm": "P0", "answer": base},
                {
                    "task": t["id"],
                    "arm": "P1",
                    "answer": treat_p1,
                    **({"citations": cite} if wired else {}),
                },
                {"task": t["id"], "arm": "P2", "answer": base, **({"hedge": "is reported to"} if wired else {})},
            ]
        elif klass == "abstain-trap":
            recs += [
                # the inert baseline answers an out-of-KG question instead of abstaining
                {"task": t["id"], "arm": "P0", "answer": "The value is 42."},
                {"task": t["id"], "arm": "P1", "answer": "The value is 42.", **({"citations": cite} if wired else {})},
                {
                    "task": t["id"],
                    "arm": "P2",
                    "answer": ("No matching records." if catches else "The value is 42."),
                    **({"abstained": bool(catches), "hedge": "is reported to"} if wired else {}),
                },
            ]
        else:
            answer = f"The answer is {gold}."
            false_positive = ordinary_seen < over_abstain
            ordinary_seen += 1
            # a wrong baseline answer that does NOT read as an abstention, so the pair
            # scores 0 → 0 and only the abstention counters move
            base_answer = "The recorded figure is unclear." if false_positive else answer
            p2 = (
                {"answer": "No matching records.", "abstained": True}
                if false_positive
                else {"answer": answer, "abstained": False}
            )
            recs += [
                {"task": t["id"], "arm": "P0", "answer": base_answer},
                {"task": t["id"], "arm": "P1", "answer": answer, **({"citations": cite} if wired else {})},
                {
                    "task": t["id"],
                    "arm": "P2",
                    **(p2 if wired else {"answer": p2["answer"]}),
                    **({"hedge": "is reported to"} if wired else {}),
                },
            ]
    return recs


GOOD_RUN = {
    "canonical_host": "syn-canonical-host",
    "token_source": "real-transcript",
    "counterbalanced": True,
    "fable_rerun": True,
    "models": {"P0": "opus", "P1": "opus", "P2": "opus"},
}


class ProvAbCase(unittest.TestCase):
    def build(self, *, run=None, overhead=0.05, known_iris=True, tasks_classes=None, **answer_kw):
        tasks, classes = tasks_classes or synth_tasks()
        with tempfile.TemporaryDirectory() as d:
            p = Path(d)
            (p / "tasks.json").write_text(json.dumps(tasks), encoding="utf-8")
            (p / "classes.json").write_text(json.dumps(classes), encoding="utf-8")
            (p / "tok.json").write_text(json.dumps(synth_tokens(tasks, overhead)), encoding="utf-8")
            (p / "ans.json").write_text(
                json.dumps(synth_answers(tasks, classes, **answer_kw)), encoding="utf-8"
            )
            (p / "run.json").write_text(json.dumps(GOOD_RUN if run is None else run), encoding="utf-8")
            (p / "iris.txt").write_text(f"{SRC}\n{ANCHOR}\n", encoding="utf-8")
            return prov_ab.build_verdict(
                str(p / "tasks.json"),
                [str(p / "tok.json")],
                [str(p / "ans.json")],
                str(p / "run.json"),
                str(p / "iris.txt") if known_iris else None,
                str(p / "classes.json"),
            )

    # --- shape -----------------------------------------------------------------
    def test_every_capability_gets_a_5_6_verdict_object(self):
        v = self.build()
        self.assertEqual(set(v["capabilities"]), set(prov_ab.CAPABILITIES))
        for cap in v["capabilities"].values():
            for field in (
                "token_win",
                "token_delta_median_pct",
                "token_delta_ci",
                "quality_delta",
                "break_even_N",
                "break_even_infinite",
                "honest",
                "recommend_adopt",
            ):
                self.assertIn(field, cap)
            self.assertEqual(
                set(cap["quality_delta"]),
                {"exec_acc", "provenance_completeness", "hallucination_rate"},
            )

    def test_provenance_weighting_is_emitted_as_blocked_not_omitted(self):
        pw = self.build()["capabilities"]["provenance_weighting"]
        self.assertEqual(pw["status"], "blocked")
        self.assertIn("agent-facing", pw["blocked_reason"])
        self.assertFalse(pw["honest"])
        self.assertFalse(pw["recommend_adopt"])

    # --- the acceptance path: honest=true AND recommend=true is REACHABLE -------
    def test_satisfied_run_recommends_the_capabilities_that_pay(self):
        v = self.build()
        for name in ("citations", "hedging"):
            cap = v["capabilities"][name]
            self.assertTrue(cap["honest"], f"{name}: {cap['_detail']['honesty']}")
            self.assertTrue(cap["recommend_adopt"], f"{name}: {cap['_detail']['kill_criteria']}")
            self.assertTrue(cap["token_win"])  # cost non-inferiority cleared
            self.assertEqual(cap["_detail"]["kill_criteria"], [])
            self.assertGreater(cap["_detail"]["cost_per_extra_caught_defect_usd"], 0)
        self.assertEqual(v["adopt"], ["citations", "hedging"])
        self.assertEqual(v["drop"], [])
        self.assertEqual(v["inconclusive"], [])
        self.assertEqual(v["blocked"], ["provenance_weighting"])
        self.assertTrue(v["honest"])
        self.assertTrue(v["recommend_adopt"])

    def test_break_even_is_reported_as_undefined_not_fabricated(self):
        cap = self.build()["capabilities"]["citations"]
        self.assertIsNone(cap["break_even_N"])
        self.assertFalse(cap["break_even_infinite"])
        self.assertIn("undefined in the token sense", cap["_detail"]["break_even_note"])

    def test_superiority_is_tested_on_the_trap_subset(self):
        d = self.build()["capabilities"]["citations"]["_detail"]
        self.assertEqual(d["n_traps"], N_TRAP)
        self.assertEqual(d["n_paired"], 2 * N_TRAP + N_ORDINARY)
        self.assertLess(d["outcome_wilcoxon_p"], 0.05)

    # --- the honesty gate: each precondition refuses on its own -----------------
    def _honesty(self, verdict, cap="citations"):
        return verdict["capabilities"][cap]["_detail"]["honesty"]

    def test_missing_canonical_host_blocks_honest(self):
        v = self.build(run={**GOOD_RUN, "canonical_host": None})
        self.assertFalse(self._honesty(v)["canonical_host_declared"])
        self.assertFalse(v["capabilities"]["citations"]["honest"])
        self.assertFalse(v["capabilities"]["citations"]["recommend_adopt"])
        self.assertFalse(v["recommend_adopt"])
        # a run that failed the honesty gate is INCONCLUSIVE, never "measured and dropped"
        self.assertIn("citations", v["inconclusive"])
        self.assertEqual(v["drop"], [])

    def test_missing_fable_rerun_blocks_honest(self):
        v = self.build(run={**GOOD_RUN, "fable_rerun": False})
        self.assertFalse(self._honesty(v)["fable_rerun"])
        self.assertFalse(v["capabilities"]["citations"]["honest"])

    def test_proxy_tokens_block_honest(self):
        v = self.build(run={**GOOD_RUN, "token_source": "char-proxy"})
        self.assertFalse(self._honesty(v)["real_transcript_tokens"])
        self.assertFalse(v["capabilities"]["hedging"]["honest"])

    def test_before_after_ordering_blocks_honest(self):
        v = self.build(run={**GOOD_RUN, "counterbalanced": False})
        self.assertFalse(self._honesty(v)["counterbalanced"])

    def test_unverifiable_citations_block_honest(self):
        """No IRI index → citation resolution would be the model's own say-so."""
        v = self.build(known_iris=False)
        cap = v["capabilities"]["citations"]
        self.assertFalse(self._honesty(v)["citation_resolution_verified"])
        self.assertFalse(cap["quality_delta"]["provenance_completeness"]["verified"])
        self.assertFalse(cap["honest"])

    def test_placebo_treatment_arm_is_caught(self):
        """A treatment arm that emits no citations/hedges did not exercise the capability."""
        v = self.build(wired=False)
        self.assertFalse(self._honesty(v)["arm_wiring_verified"])
        self.assertFalse(self._honesty(v, "hedging")["arm_wiring_verified"])
        self.assertFalse(v["capabilities"]["citations"]["honest"])

    def test_underpowered_trap_set_blocks_honest(self):
        tasks, classes = synth_tasks()
        keep = {c["id"] for c in classes if c["outcome_class"] != "stale-fact-trap"}
        few = [c["id"] for c in classes if c["outcome_class"] == "stale-fact-trap"][: MIN_N - 1]
        keep |= set(few)
        tasks = [t for t in tasks if t["id"] in keep]
        classes = [c for c in classes if c["id"] in keep]
        v = self.build(tasks_classes=(tasks, classes))
        self.assertFalse(self._honesty(v)["n_traps_at_least_min_n"])
        self.assertFalse(v["capabilities"]["citations"]["honest"])
        # …while the fully-powered hedging arm on the same run is unaffected
        self.assertTrue(v["capabilities"]["hedging"]["honest"])

    # --- the kill criteria ------------------------------------------------------
    def test_cost_blowout_fires_kill_1(self):
        v = self.build(overhead=0.60)
        cap = v["capabilities"]["citations"]
        self.assertFalse(cap["token_win"])
        self.assertIn("KILL_1_cost", [k["kill"] for k in cap["_detail"]["kill_criteria"]])
        self.assertFalse(cap["recommend_adopt"])

    def test_no_lift_fires_kill_2_and_kill_3(self):
        v = self.build(lift=False)
        fired = [k["kill"] for k in v["capabilities"]["citations"]["_detail"]["kill_criteria"]]
        self.assertIn("KILL_2_break_even", fired)
        self.assertIn("KILL_3_quality", fired)
        self.assertFalse(v["capabilities"]["citations"]["recommend_adopt"])
        self.assertIsNone(
            v["capabilities"]["citations"]["_detail"]["cost_per_extra_caught_defect_usd"]
        )

    def test_hedging_reports_abstention_precision_and_recall_for_both_arms(self):
        """The declared axis is emitted, not merely declared."""
        ab = self.build()["capabilities"]["hedging"]["_detail"]["abstention"]
        # the `N_MISS` missed traps are the FIRST tasks in the set, which are the
        # stale-fact traps, so the hedging arm catches every abstain-trap here
        self.assertEqual(ab["treatment"]["true_positive"], N_TRAP)
        self.assertEqual(ab["treatment"]["false_negative"], 0)
        self.assertEqual(ab["treatment"]["false_positive"], 0)
        self.assertEqual(ab["treatment"]["precision"], 1.0)
        self.assertEqual(ab["treatment"]["recall"], 1.0)
        # the inert baseline never abstains: recall 0, and precision UNDEFINED (not 1.0)
        self.assertEqual(ab["baseline"]["recall"], 0.0)
        self.assertIsNone(ab["baseline"]["precision"])
        self.assertTrue(ab["precision_non_regression"])
        # …and the citations capability, which declares no abstention axis, reports none
        self.assertIsNone(self.build()["capabilities"]["citations"]["_detail"]["abstention"])

    def test_false_positive_abstentions_fire_kill_3_even_when_mean_accuracy_holds(self):
        """Trap recall improves and the mean ordinary score does NOT move — the ONLY
        signal is the treatment abstaining on answerable tasks. Mean-accuracy
        non-regression cannot see this; abstention precision must."""
        v = self.build(over_abstain=3)
        cap = v["capabilities"]["hedging"]
        d = cap["_detail"]

        # the hole: mean ordinary accuracy is unchanged, so the old guard is satisfied…
        self.assertTrue(cap["quality_delta"]["exec_acc"]["non_regression"])
        self.assertEqual(
            cap["quality_delta"]["exec_acc"]["baseline"],
            cap["quality_delta"]["exec_acc"]["treatment"],
        )
        # …and trap recall still improved, so superiority is established…
        self.assertTrue(d["quality_superior"])
        self.assertGreater(d["abstention"]["treatment"]["recall"], d["abstention"]["baseline"]["recall"])
        # …but precision regressed: 3 abstentions on answerable tasks, none for baseline
        self.assertEqual(d["abstention"]["treatment"]["false_positive"], 3)
        self.assertEqual(d["abstention"]["baseline"]["false_positive"], 0)
        self.assertLess(d["abstention"]["treatment"]["precision"], 1.0)
        self.assertFalse(d["abstention"]["precision_non_regression"])

        fired = [k for k in d["kill_criteria"] if k["kill"] == "KILL_3_quality"]
        self.assertEqual(len(fired), 1)
        self.assertIn("abstention precision regressed", fired[0]["why"])
        self.assertFalse(cap["recommend_adopt"])
        self.assertNotIn("hedging", v["adopt"])
        self.assertIn("hedging", v["drop"])

    def test_a_fabricated_citation_is_a_hard_kill_even_when_everything_else_passes(self):
        v = self.build(fabricate=True)
        cap = v["capabilities"]["citations"]
        self.assertGreater(cap["quality_delta"]["hallucination_rate"]["fabricated_citations"], 0)
        self.assertTrue(cap["token_win"])  # cost was fine
        self.assertTrue(cap["_detail"]["quality_superior"])  # the catch rate was fine
        self.assertIn("KILL_3_quality", [k["kill"] for k in cap["_detail"]["kill_criteria"]])
        self.assertFalse(cap["recommend_adopt"])  # …and it still must not be adopted

    # --- single-sourcing --------------------------------------------------------
    def test_thresholds_are_imported_from_the_original_harness(self):
        import stats

        d = self.build()["capabilities"]["citations"]["_detail"]
        self.assertEqual(d["frozen_bar"]["min_relative_reduction"], stats.MIN_RELATIVE_REDUCTION)
        self.assertEqual(d["frozen_bar"]["max_p"], stats.MAX_P)
        self.assertEqual(d["frozen_bar"]["min_n"], stats.MIN_N)
        self.assertEqual(prov_ab.COST_OVERHEAD_CEILING, stats.MIN_RELATIVE_REDUCTION)

    def test_p_arm_tags_are_minable(self):
        import tokens_real

        self.assertTrue(tokens_real.TAG.search("[ABM task=syn-stale-00 arm=P1]"))
        self.assertTrue(tokens_real.TAG.search("[ABM task=t01 arm=C]"))  # A/B/C still works
        self.assertIsNone(tokens_real.TAG.search("[ABM task=t01 arm=P9]"))


if __name__ == "__main__":
    unittest.main(verbosity=2)
