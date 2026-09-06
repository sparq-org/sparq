#!/usr/bin/env python3
# [OPUS-5] issue #4559 (bead sq-jxeqz) — the NAMED wiring + invariant test for the bench
# HARD-ZONE gate (scripts/bench_hardzone.py).
#
# WHY THIS EXISTS. Two independent failures met in #4559:
#
#   1. The gate FAILED main for achieving a PERFECT result. `vectors_hnsw_recall_at10` is a
#      recall DEFICIT (bench/vector/README.md), so 0 is its optimum — and the gate routed that
#      to a fail-closed `invalid`. 15 of the last 60 Benchmarks runs on main failed, 100% on
#      that one step.
#   2. NOTHING CAUGHT IT BEFORE MERGE. `bench_hardzone.py --self-test` ran ONLY inside
#      bench.yml's "Hard zone" step, which is guarded to main pushes — so the gate's own logic
#      was never exercised on a PR. A regression in the regression gate could only be
#      discovered by reddening main after merge. The YAML seam is where vacuity lives: a check
#      that does not execute in the gating matrix is not a check.
#
# Every assertion below is a MUTATION anchor:
#
#   TestSelfTestRunsOnPullRequests — delete the docs-quality step (or its `run:` call site)
#                       => the hard-zone gate's 89 self-test checks stop gating a PR => RED.
#   TestSelfAnchoring — delete THIS test's own step => RED (the wiring test must itself be
#                       wired, or the regress just moves up one level).
#   TestBenchLaneNotSilenced — add `|| true` / `continue-on-error` / `set +e`, or drop the
#                       real gate invocation, in the bench.yml Hard zone step => RED.
#   TestPublishOrderIsTheAntiLaunderingGuard — move the gate AFTER the Store step, or make
#                       Store `if: always()` => RED. This is the guard that stops a REJECTED
#                       regression being published into the very median that judges it.
#   TestOptimumIsNeverARegression — a behaviour anchor executed against the real module, so
#                       deleting the in-script self-test section does not leave zero coverage
#                       of the defect this issue is about.
#
# Run:  python3 scripts/tests/test_bench_hardzone_wiring.py

from __future__ import annotations

import importlib.util
import re
import unittest
from pathlib import Path

import yaml

REPO_ROOT = Path(__file__).resolve().parent.parent.parent
GATE_PY = REPO_ROOT / "scripts" / "bench_hardzone.py"
BENCH_YML = REPO_ROOT / ".github" / "workflows" / "bench.yml"
DOCS_QUALITY_YML = REPO_ROOT / ".github" / "workflows" / "docs-quality.yml"

SELF_TEST_CALL = re.compile(r"python3\s+scripts/bench_hardzone\.py\s+--self-test")
WIRING_TEST_CALL = re.compile(r"python3\s+scripts/tests/test_bench_hardzone_wiring\.py")


def _load(path: Path) -> dict:
    return yaml.safe_load(path.read_text(encoding="utf-8"))


def _steps(doc: dict, job: str) -> list[dict]:
    return doc["jobs"][job].get("steps", [])


def _runs(steps: list[dict]) -> str:
    return "\n".join(s.get("run", "") for s in steps if isinstance(s.get("run"), str))


def _load_gate_module():
    spec = importlib.util.spec_from_file_location("bench_hardzone", GATE_PY)
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


class TestSelfTestRunsOnPullRequests(unittest.TestCase):
    """The gate's own logic must be exercised in the PR-gating matrix, not only on main."""

    def setUp(self):
        self.doc = _load(DOCS_QUALITY_YML)
        self.steps = _steps(self.doc, "quick-gates")

    def test_self_test_call_site_present(self):
        """MUTANT: delete the step or its `run:` line => the 89 checks stop gating a PR."""
        self.assertRegex(
            _runs(self.steps), SELF_TEST_CALL,
            "docs-quality `quick-gates` must run `python3 scripts/bench_hardzone.py "
            "--self-test`. Without it the hard-zone gate's logic is only ever executed by "
            "bench.yml's main-push-guarded step, so a regression in the regression gate can "
            "only be found by reddening main after merge (issue #4559).",
        )

    def test_step_is_not_conditioned_away(self):
        """MUTANT: add an `if:` to the step so it silently never runs => RED."""
        for step in self.steps:
            if isinstance(step.get("run"), str) and SELF_TEST_CALL.search(step["run"]):
                self.assertNotIn(
                    "if", step,
                    "the hard-zone self-test step must run unconditionally in quick-gates — "
                    "a step-level `if:` is how a gate becomes a no-op without going red",
                )

    def test_job_is_not_conditioned_away(self):
        """MUTANT: add a job-level `if:` that skips quick-gates => RED."""
        self.assertNotIn(
            "if", self.doc["jobs"]["quick-gates"],
            "quick-gates must not carry a job-level `if:` — a skipped job reports neutral, "
            "and every self-test it hosts stops gating at once",
        )

    def test_runs_in_the_required_matrix(self):
        """MUTANT: drop pull_request / merge_group => the gate stops guarding merges."""
        triggers = self.doc.get("on", self.doc.get(True))
        self.assertIsNotNone(triggers, "docs-quality must declare triggers")
        for needed in ("pull_request", "merge_group"):
            self.assertIn(
                needed, triggers,
                f"docs-quality must trigger on {needed} — `ci-summary / gate` polls the "
                "check-runs that exist on the ref, so a lane absent there is not a gate",
            )


class TestSelfAnchoring(unittest.TestCase):
    """A wiring test that is not itself wired just moves the vacuity up one level."""

    def test_this_test_runs_in_quick_gates(self):
        """MUTANT: delete THIS test's own step from docs-quality => RED."""
        self.assertRegex(
            _runs(_steps(_load(DOCS_QUALITY_YML), "quick-gates")), WIRING_TEST_CALL,
            "docs-quality `quick-gates` must also run this wiring test itself, otherwise the "
            "wiring assertions never execute and the seam is unguarded again",
        )


class TestBenchLaneNotSilenced(unittest.TestCase):
    """The main-push hard gate must stay a HARD gate."""

    def setUp(self):
        self.doc = _load(BENCH_YML)
        self.steps = _steps(self.doc, "bench")
        self.gate = next(s for s in self.steps
                         if "Hard zone" in (s.get("name") or ""))

    def test_gate_step_still_invokes_the_real_gate(self):
        """MUTANT: drop the real invocation and leave only --self-test => RED."""
        run = self.gate.get("run", "")
        self.assertRegex(run, SELF_TEST_CALL,
                         "the bench lane must still self-test before trusting the gate")
        self.assertRegex(
            run, r"scripts/bench_hardzone\.py\s*\\?\s*\n?\s*--results",
            "the bench lane must still run the gate over the real bench-results.json — a "
            "self-test-only step verifies the logic and gates nothing",
        )

    def test_no_exit_zero_swallow(self):
        """MUTANT: add `|| true` / `set +e` / `continue-on-error` => RED.

        An exit-zero swallow discards an already-earned hard failure; this repo has had three
        separate incidents of exactly that, and #4547 exists to catch the class repo-wide.
        """
        run = self.gate.get("run", "")
        self.assertIn("set -euo pipefail", run,
                      "the hard-zone step must keep `set -euo pipefail`")
        for banned in ("|| true", "set +e", "|| :"):
            self.assertNotIn(banned, run,
                             f"the hard-zone step must never swallow a failure with {banned!r}")
        self.assertNotIn(
            "continue-on-error", self.gate,
            "the hard-zone step must never be continue-on-error — a benchmark gate that "
            "cannot fail is worse than one that fails spuriously",
        )


class TestPublishOrderIsTheAntiLaunderingGuard(unittest.TestCase):
    """The gate runs BEFORE Store, and Store must stay skippable by a gate failure.

    This encodes the deliberate decision NOT to publish a failing run's measurements
    (#4559). Store auto-pushes this run's point into the benchmark-data history; if a
    failing run published, a REJECTED regression would enter the very history the gate
    computes its median from, and after a few red runs the regression would BECOME the
    baseline — silently raising the floor for everyone. The self-perpetuating red that
    motivated #4559 is instead broken at the source: the provably-non-regressing cases now
    PASS, so they publish through the ordinary path and the median rebases normally.
    """

    def setUp(self):
        self.steps = _steps(_load(BENCH_YML), "bench")
        self.names = [s.get("name") or "" for s in self.steps]

    def _index(self, needle: str) -> int:
        for i, n in enumerate(self.names):
            if needle in n:
                return i
        self.fail(f"no step named like {needle!r} in the bench job")

    def test_gate_precedes_store(self):
        """MUTANT: move the Store step above the Hard zone step => RED."""
        self.assertLess(
            self._index("Hard zone"), self._index("Store + compare against history"),
            "the hard-zone gate MUST run before the Store step: Store auto-pushes this run's "
            "point into benchmark-data, so gating afterwards would publish a rejected "
            "regression and let it rebase the median that judges it",
        )

    def test_store_does_not_run_when_the_gate_failed(self):
        """MUTANT: add `if: always()` to Store => RED (it would publish a rejected run)."""
        store = self.steps[self._index("Store + compare against history")]
        cond = str(store.get("if", ""))
        self.assertNotIn(
            "always()", cond,
            "the Store step must NOT be `if: always()` — that would publish the measurements "
            "of a run the hard zone just rejected, rebasing the median toward the regression",
        )

    def test_the_ordering_rationale_is_documented_at_the_seam(self):
        """MUTANT: delete the ORDERING comment => the next editor reorders it innocently."""
        text = BENCH_YML.read_text(encoding="utf-8")
        self.assertIn(
            "ORDERING", text,
            "the deliberate gate-before-Store ordering must stay documented in bench.yml",
        )


class TestOptimumIsNeverARegression(unittest.TestCase):
    """Behaviour anchor for the #4559 defect, independent of the in-script self-test."""

    @classmethod
    def setUpClass(cls):
        cls.hz = _load_gate_module()

    def _hist(self, name, unit, values):
        return self.hz.history_values([
            {"date": i, "benches": [{"name": name, "value": v, "unit": unit}]}
            for i, v in enumerate(values)
        ])

    def test_recall_deficit_at_zero_passes(self):
        """MUTANT: route the optimum-side zero boundary back to `invalid` => RED.

        The live failure: run 30279965305 @ cb0c6739c7 reported
        `vectors_hnsw_recall_at10: zero-boundary change (current 0 vs median 2)`.
        A deficit of 0 is recall@10 = 1.000 — the best value the metric can take.
        """
        hist = self._hist("vectors_hnsw_recall_at10", "milli", [1, 1, 2, 2, 2])
        code, rep = self.hz.evaluate(
            [{"name": "vectors_hnsw_recall_at10", "value": 0, "unit": "milli"}], hist)
        self.assertEqual(code, 0, f"a perfect recall deficit must not fail: {rep['invalid']}")
        self.assertEqual(rep["invalid"], [])
        self.assertEqual([n for n, _c, _m, _u in rep["improved_to_zero"]],
                         ["vectors_hnsw_recall_at10"])

    def test_noise_floor_is_reachable_at_zero(self):
        """MUTANT: compute floor_exempt after a zero-boundary `continue` => RED."""
        hist = self._hist("vectors_hnsw_recall_at10", "milli", [1, 1, 2, 2, 2])
        _code, rep = self.hz.evaluate(
            [{"name": "vectors_hnsw_recall_at10", "value": 0, "unit": "milli"}], hist)
        self.assertEqual([r[0] for r in rep["floor_exempt"]], ["vectors_hnsw_recall_at10"],
                         "the `milli` noise floor was added for this metric and must be "
                         "reachable at its optimum, not skipped by an early `continue`")

    def test_one_quantization_tick_is_not_a_regression(self):
        """MUTANT: delete RESOLUTION_QUANTA['s'] => RED.

        `sameas_size32_closure_s` is scraped from a `{:.3}s` print, so one tick is 1 ms and
        0.001 -> 0.002 is exactly the inclusive 2.0x threshold with zero margin.
        """
        hist = self._hist("sameas_size32_closure_s", "s", [0.001] * 5)
        code, rep = self.hz.evaluate(
            [{"name": "sameas_size32_closure_s", "value": 0.002, "unit": "s"}], hist)
        self.assertEqual(code, 0)
        self.assertEqual([r[0] for r in rep["resolution_exempt"]], ["sameas_size32_closure_s"])

    def test_a_genuine_regression_still_reds(self):
        """CONTROL. MUTANT: widen the allowance until this passes => RED.

        The same metric at the value from the real 5.00x incident (run 30235240748) must
        still hard-fail — the resolution allowance is one tick wide, not a silenced metric.
        """
        hist = self._hist("sameas_size32_closure_s", "s", [0.001] * 5)
        code, rep = self.hz.evaluate(
            [{"name": "sameas_size32_closure_s", "value": 0.005, "unit": "s"}], hist)
        self.assertEqual(code, 1, "a 5x move on the same metric must still fail the gate")
        self.assertEqual([r[0] for r in rep["hard"]], ["sameas_size32_closure_s"])

    def test_throughput_collapse_to_zero_still_fails_closed(self):
        """QUANTIFIER DIRECTION. MUTANT: treat every zero as an improvement => RED."""
        hist = self._hist("rsp_x_triples_per_s", "triples_per_s", [1000.0] * 5)
        code, rep = self.hz.evaluate(
            [{"name": "rsp_x_triples_per_s", "value": 0, "unit": "triples_per_s"}], hist)
        self.assertEqual(code, 1, "a throughput collapsing to zero is the PESSIMAL side")
        self.assertEqual(len(rep["invalid"]), 1)


class TestConfirmationPassIsWired(unittest.TestCase):
    """The confirm-by-re-measurement pass must actually be REACHED from the gating YAML.

    The gate-side logic is fully unit-tested by `--self-test`, but a confirmation pass with no
    `--remeasure-cmd` at the call site is a no-op that still reports green: bench_hardzone.py is
    deliberately fail-closed without one, so the omission is INVISIBLE — the gate simply keeps
    reddening main on unreproduced breaches exactly as before. That is the YAML-seam vacuity this
    class exists to prevent.
    """

    def setUp(self):
        self.steps = _steps(_load(BENCH_YML), "bench")
        self.gate = next(s for s in self.steps if "Hard zone" in (s.get("name") or ""))
        self.run = self.gate.get("run", "")

    def test_remeasure_cmd_is_passed_at_the_call_site(self):
        """MUTANT: drop `--remeasure-cmd` from the step => RED.

        Without it apply_remeasurement is handed remeasure_fn=None and relieves nothing, so
        every unreproduced breach reds main again — silently, and with a green self-test.
        """
        self.assertIn(
            "--remeasure-cmd", self.run,
            "the bench lane must pass --remeasure-cmd to bench_hardzone.py; without it the "
            "confirmation pass is a no-op and a runner slow-window reds main again "
            "(measured: 29 of 153 live runs, 19%)",
        )

    def test_remeasure_actually_reruns_the_benchmark_suite(self):
        """MUTANT: point --remeasure-cmd at `true` / `echo` / a stale file => RED.

        A confirmation that does not re-execute the benchmark is not a second measurement; it
        would either relieve nothing (best case) or, if it replayed the ORIGINAL results file,
        confirm every breach forever.
        """
        m = re.search(r"--remeasure-cmd\s+'([^']*)'", self.run)
        self.assertIsNotNone(m, "--remeasure-cmd must carry a single-quoted shell command")
        cmd = m.group(1)
        self.assertIn("scripts/ci-bench.sh", cmd,
                      "the confirmation must re-run the real benchmark suite, not a stub")
        self.assertNotIn("--deterministic-only", cmd,
                         "the confirmation must re-measure TIMING metrics — the "
                         "deterministic-only form emits none of the rows that trip this gate")

    def test_remeasure_never_overwrites_the_published_results(self):
        """MUTANT: write the re-measurement to bench-results.json => RED.

        bench-results.json is what the Store step publishes. If a confirmation run overwrote
        it, the trend would silently record the RE-MEASURED value instead of the value the
        suite actually produced — laundering the reading the gate was asked to judge.
        """
        m = re.search(r"--remeasure-cmd\s+'([^']*)'", self.run)
        cmd = m.group(1) if m else ""
        self.assertNotRegex(
            cmd, r"(?<![-\w])bench-results\.json",
            "the re-measure command must write to its own path — never bench-results.json, "
            "which the Store step publishes to the benchmark-data trend",
        )
        out = re.search(r"--remeasure-out\s+(\S+)", self.run)
        self.assertIsNotNone(out, "--remeasure-out must be pinned at the call site")
        self.assertNotEqual(out.group(1), "bench-results.json")
        self.assertIn(out.group(1), cmd,
                      "--remeasure-out must name the SAME path the command writes to, or the "
                      "gate reads a file the command never produced and relieves nothing")

    def test_the_confirmation_step_keeps_the_same_main_push_guard(self):
        """MUTANT: widen/narrow the step `if:` => RED.

        The confirmation rides the existing step, so it must inherit exactly its guard: a
        widened guard would re-run the full suite on PRs (which are --deterministic-only and
        already strictly gated by perf-gate.py), and a narrowed one would silently disable the
        gate itself.
        """
        cond = str(self.gate.get("if", ""))
        self.assertIn("github.ref == 'refs/heads/main'", cond)
        self.assertIn("github.event_name != 'schedule'", cond)

    def test_gate_module_defaults_are_fail_closed_without_a_command(self):
        """MUTANT: make remeasure_fn default to something truthy => RED.

        The behaviour every OTHER lane depends on: with no --remeasure-cmd the gate must be
        byte-identical to its pre-confirmation behaviour.
        """
        hz = _load_gate_module()
        code, rep = hz.evaluate([{"name": "q_us", "value": 2400.0, "unit": "us"}],
                                {"q_us": [1000.0] * 5})
        self.assertEqual(code, 1)
        hz.apply_remeasurement(rep, None, 1)
        self.assertEqual(len(rep["hard"]), 1, "no remeasure_fn must relieve nothing")
        self.assertEqual(hz.gate_exit_code(rep), 1)


if __name__ == "__main__":
    unittest.main(verbosity=2)
