#!/usr/bin/env python3
"""[GPT-6 Astra] Offline behavior, real producer and CI wiring for issue #6436."""

from __future__ import annotations

import copy
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

import yaml

ROOT = Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location("freshness", ROOT / "scripts/ci_nightly_freshness.py")
freshness = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(freshness)
REPO = "sparq-org/sparq"
HEAD = "a" * 40
CURRENT = 1000


def run(run_id=999, **changes):
    item = dict(id=run_id, run_attempt=1, event="schedule", head_sha=HEAD,
                head_branch="main", path=".github/workflows/ci.yml",
                repository={"full_name": REPO}, status="completed", conclusion="success")
    item.update(changes)
    return item


def job(name, index, step=None, **changes):
    item = dict(id=index, run_id=999, run_attempt=1, head_sha=HEAD,
                name=name, status="completed", conclusion="success", steps=[])
    if step:
        item["steps"] = [dict(name=step, status="completed", conclusion="success")]
    item.update(changes)
    return item


def completed_jobs():
    return ([job(freshness.COVERAGE, 1, freshness.COVERAGE_STEP)]
            + [job(f"{freshness.MUTATION} (shard-{i})", i + 2, freshness.MUTATION_STEP)
               for i in range(freshness.MUTATION_JOBS)])


def skipped_jobs():
    return [job(freshness.COVERAGE, 1, conclusion="skipped"),
            job(freshness.MUTATION, 2, conclusion="skipped")]


class FakeAPI:
    def __init__(self, runs=None, jobs=None, history=None):
        self.history = history if history is not None else {
            "total_count": len(runs or []), "workflow_runs": runs or []}
        self.jobs = jobs if jobs is not None else {999: completed_jobs()}
        self.calls = []

    def __call__(self, endpoint):
        self.calls.append(endpoint)
        if "/workflows/" in endpoint:
            return copy.deepcopy(self.history)
        parts = endpoint.split("/")
        run_id = int(parts[5])
        self.assert_attempt(parts, run_id)
        jobs = self.jobs[run_id]
        page = int(endpoint.rsplit("page=", 1)[1])
        return {"total_count": len(jobs), "jobs": copy.deepcopy(jobs[(page - 1) * 100:page * 100])}

    def assert_attempt(self, parts, run_id):
        expected = next(r["run_attempt"] for r in self.history["workflow_runs"] if r["id"] == run_id)
        assert parts[6:8] == ["attempts", str(expected)], parts


class Admission(unittest.TestCase):
    def decide(self, api, event="schedule"):
        return freshness.decide(event, REPO, HEAD, CURRENT, api)

    def blocked(self, api, pattern=None):
        with self.assertRaisesRegex(freshness.EvidenceError, pattern or "."):
            self.decide(api)

    def test_first_schedule_at_new_head_runs_once(self):
        for history in [[], [run(CURRENT, status="in_progress", conclusion=None)]]:
            with self.subTest(history=history):
                api = FakeAPI(history)
                self.assertTrue(self.decide(api)[0])
                self.assertEqual(len(api.calls), 1)
                self.assertIn("head_sha=" + HEAD, api.calls[0])
                self.assertNotIn("status=completed", api.calls[0])

    def test_verified_same_head_skips(self):
        api = FakeAPI([run()])
        self.assertFalse(self.decide(api)[0])
        self.assertEqual(len(api.calls), 2)

    def test_failed_cancelled_incomplete_work_remains_eligible_next_schedule(self):
        for conclusion in ["failure", "cancelled", "timed_out", "success"]:
            with self.subTest(conclusion=conclusion):
                jobs = completed_jobs()
                jobs[-1]["conclusion"] = "cancelled"
                api = FakeAPI([run(conclusion=conclusion)], {999: jobs})
                self.assertTrue(self.decide(api)[0])
                self.assertEqual(len(api.calls), 2)

    def test_active_or_unreadable_same_head_never_admits(self):
        for status, conclusion in [("in_progress", None), ("queued", None), ("completed", None)]:
            with self.subTest(status=status, conclusion=conclusion):
                api = FakeAPI([run(status=status, conclusion=conclusion)])
                self.blocked(api)
                self.assertEqual(len(api.calls), 1)

    def test_manual_force_and_other_events_never_read_history(self):
        for event in ["workflow_dispatch", "merge_group", "push", "pull_request", "unknown", ""]:
            with self.subTest(event=event):
                api = FakeAPI([run(conclusion="cancelled")])
                self.assertEqual(self.decide(api, event)[0], event == "workflow_dispatch")
                self.assertEqual(api.calls, [])

    def test_skipped_followup_needs_real_older_proof(self):
        old = completed_jobs()
        for item in old:
            item["run_id"] = 998
        api = FakeAPI([run(), run(998)], {999: skipped_jobs(), 998: old})
        self.assertFalse(self.decide(api)[0])
        self.assertIn("998 attempt 1", self.decide(api)[1])
        self.assertTrue(self.decide(FakeAPI([run()], {999: skipped_jobs()}))[0])

    def test_green_skipped_followup_cannot_launder_failed_cancelled_run(self):
        for conclusion in ["failure", "cancelled"]:
            with self.subTest(conclusion=conclusion):
                incomplete = completed_jobs()
                for item in incomplete:
                    item["run_id"] = 998
                incomplete[-1]["steps"] = []
                api = FakeAPI([run(), run(998, conclusion=conclusion)],
                              {999: skipped_jobs(), 998: incomplete})
                self.assertTrue(self.decide(api)[0])
                self.assertEqual(len(api.calls), 3)

    def test_missing_unreadable_or_truncated_history_blocks(self):
        for doc in [None, {}, [], {"total_count": True, "workflow_runs": []},
                    {"total_count": 1, "workflow_runs": []},
                    {"total_count": 0, "workflow_runs": [run()]}]:
            with self.subTest(doc=doc):
                self.blocked(lambda endpoint: doc)

    def test_identity_and_different_head_evidence_rejected(self):
        for changes in [{"head_sha": "b" * 40}, {"head_branch": "feature"},
                        {"event": "workflow_dispatch"}, {"path": ".github/workflows/other.yml"},
                        {"repository": {"full_name": "elsewhere/repo"}}, {"repository": None},
                        {"run_attempt": 0}, {"id": True}]:
            with self.subTest(changes=changes):
                self.blocked(FakeAPI([run(**changes)]), "identity")
        self.blocked(FakeAPI([run(), run()]), "identity")

    def test_stale_schedule_does_not_compete_with_newer_tick(self):
        self.blocked(FakeAPI([run(CURRENT + 1)]), "newer")

    def test_invalid_input_identity_never_reads_github(self):
        for repo, head, current in [("bad repo", HEAD, CURRENT), (REPO, "short", CURRENT),
                                    (REPO, HEAD, 0)]:
            with self.subTest(repo=repo, head=head, current=current):
                api = FakeAPI()
                with self.assertRaises(freshness.EvidenceError):
                    freshness.decide("schedule", repo, head, current, api)
                self.assertEqual(api.calls, [])

    def test_unreadable_step_proof_does_not_skip_heavy_work(self):
        for steps in [None, [None], [{"name": freshness.COVERAGE_STEP}]]:
            with self.subTest(steps=steps):
                jobs = completed_jobs()
                jobs[0]["steps"] = steps
                self.assertTrue(self.decide(FakeAPI([run()], {999: jobs}))[0])

    def test_attempt_scoped_job_lookup_and_mismatched_attempt_blocks(self):
        jobs = completed_jobs()
        for item in jobs:
            item["run_attempt"] = 2
        api = FakeAPI([run(run_attempt=2)], {999: jobs})
        self.assertFalse(self.decide(api)[0])
        self.assertIn("/attempts/2/jobs?", api.calls[-1])
        jobs[0]["run_attempt"] = 1
        self.blocked(FakeAPI([run(run_attempt=2)], {999: jobs}), "identity")

    def test_each_heavy_leg_and_marker_is_required(self):
        for index in range(len(completed_jobs())):
            for change in ["absent", "failure", "cancelled", "skipped", "missing_step", "failed_step"]:
                with self.subTest(index=index, change=change):
                    jobs = completed_jobs()
                    if change == "absent":
                        jobs.pop(index)
                    elif change == "missing_step":
                        jobs[index]["steps"] = []
                    elif change == "failed_step":
                        jobs[index]["steps"][0]["conclusion"] = "failure"
                    else:
                        jobs[index]["conclusion"] = change
                    self.assertTrue(self.decide(FakeAPI([run()], {999: jobs}))[0])

    def test_duplicate_wrong_head_wrong_run_jobs_cannot_fill_matrix(self):
        for field, value in [("id", 1), ("name", completed_jobs()[1]["name"]),
                             ("head_sha", "b" * 40), ("run_id", 998)]:
            with self.subTest(field=field):
                jobs = completed_jobs()
                jobs[-1][field] = value
                if field == "name":
                    self.assertTrue(self.decide(FakeAPI([run()], {999: jobs}))[0])
                else:
                    self.blocked(FakeAPI([run()], {999: jobs}))

    def test_mixed_skipped_matrix_and_missing_placeholders_block(self):
        for jobs in [skipped_jobs()[:1], skipped_jobs()[1:],
                     skipped_jobs() + [job(freshness.MUTATION + " (extra)", 3)],
                     [job(freshness.COVERAGE, 1), skipped_jobs()[1]]]:
            with self.subTest(jobs=jobs):
                self.assertTrue(self.decide(FakeAPI([run()], {999: jobs}))[0])

    def test_full_inventory_pagination_and_hard_bound(self):
        jobs = completed_jobs() + [job(f"other-{i}", 100 + i) for i in range(100)]
        api = FakeAPI([run()], {999: jobs})
        self.assertFalse(self.decide(api)[0])
        self.assertEqual(len(api.calls), 3)
        jobs += [job(f"excess-{i}", 300 + i) for i in range(100)]
        api = FakeAPI([run()], {999: jobs})
        self.blocked(api, "unbounded")
        self.assertEqual(len(api.calls), 2)

    def test_truncated_job_page_and_missing_inventory_block(self):
        for doc in [{}, {"total_count": 100, "jobs": []},
                    {"total_count": 1, "jobs": completed_jobs()},
                    {"total_count": True, "jobs": []}]:
            with self.subTest(doc=doc):
                self.blocked(lambda endpoint: ({"total_count": 1, "workflow_runs": [run()]}
                                               if "/workflows/" in endpoint else doc))

    def test_skipped_history_budget_is_bounded(self):
        runs = [run(999 - i) for i in range(freshness.HISTORY_LIMIT)]
        jobs = {}
        for item in runs:
            jobs[item["id"]] = skipped_jobs()
            for entry in jobs[item["id"]]:
                entry["run_id"] = item["id"]
        api = FakeAPI(runs, jobs)
        api.history["total_count"] = 100
        self.blocked(api, "history budget")
        self.assertEqual(len(api.calls), 1 + freshness.HISTORY_LIMIT)

    def test_api_failure_json_and_timeout_each_make_one_read(self):
        failures = [subprocess.CompletedProcess([], 1, "", "API rate limit exceeded"),
                    subprocess.CompletedProcess([], 0, "not json", ""),
                    subprocess.TimeoutExpired("gh", 30), OSError("unavailable")]
        for failure in failures:
            with self.subTest(failure=failure):
                kwargs = ({"side_effect": failure} if isinstance(failure, Exception)
                          else {"return_value": failure})
                with patch.object(freshness.subprocess, "run", **kwargs) as call:
                    with self.assertRaises(freshness.EvidenceError):
                        freshness.gh_json("repos/sparq-org/sparq/actions/workflows/ci.yml/runs")
                    self.assertEqual(call.call_count, 1)
                    self.assertEqual(call.call_args.kwargs["timeout"], 30)
                    self.assertEqual(call.call_args.args[0][1:4], ["api", "--method", "GET"])

    def test_cli_error_emits_false_and_failure(self):
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / "output"
            with patch.dict(os.environ, {"GITHUB_EVENT_NAME": "schedule", "GITHUB_RUN_ID": "bad",
                                         "GITHUB_OUTPUT": str(output)}):
                self.assertEqual(freshness.main(), 1)
            self.assertEqual(output.read_text(), "fresh=false\n")

    def test_cli_schedule_emits_actual_decision_and_never_writes_api(self):
        for history, expected in [([], "true"), ([run()], "false")]:
            with self.subTest(expected=expected), tempfile.TemporaryDirectory() as directory:
                output = Path(directory) / "output"
                api = FakeAPI(history)
                def command(argv, **kwargs):
                    self.assertEqual(argv[:4], ["gh", "api", "--method", "GET"])
                    return subprocess.CompletedProcess(argv, 0, json.dumps(api(argv[-1])), "")
                with patch.dict(os.environ, {"GITHUB_EVENT_NAME": "schedule", "GITHUB_REPOSITORY": REPO,
                                             "GITHUB_SHA": HEAD, "GITHUB_RUN_ID": str(CURRENT),
                                             "GITHUB_OUTPUT": str(output)}), \
                        patch.object(freshness.subprocess, "run", side_effect=command):
                    self.assertEqual(freshness.main(), 0)
                self.assertEqual(output.read_text(), f"fresh={expected}\n")

    def test_cli_unreadable_evidence_emits_false_and_stops(self):
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / "output"
            with patch.dict(os.environ, {"GITHUB_EVENT_NAME": "schedule", "GITHUB_REPOSITORY": REPO,
                                         "GITHUB_SHA": HEAD, "GITHUB_RUN_ID": str(CURRENT),
                                         "GITHUB_OUTPUT": str(output)}), \
                    patch.object(freshness.subprocess, "run", return_value=
                                 subprocess.CompletedProcess([], 1, "", "API rate limit exceeded")) as call:
                self.assertEqual(freshness.main(), 1)
                self.assertEqual(call.call_count, 1)
            self.assertEqual(output.read_text(), "fresh=false\n")


class ProductionWiring(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.workflow = yaml.safe_load((ROOT / ".github/workflows/ci.yml").read_text())
        cls.jobs = cls.workflow["jobs"]
        cls.mutation = cls.jobs["mutants-nightly-advisory"]
        cls.producer = next(s for s in cls.mutation["steps"] if s.get("id") == "run_mutants")

    def test_schedule_manual_boundary_and_unchanged_cost_limits(self):
        gate = self.jobs["nightly-gate"]
        self.assertEqual(gate["if"], "github.event_name == 'schedule' || github.event_name == 'workflow_dispatch'")
        self.assertEqual(gate["permissions"], {"actions": "read", "contents": "read"})
        checkout = gate["steps"][0]
        self.assertTrue(checkout["uses"].startswith("actions/checkout@"))
        self.assertFalse(checkout["with"]["persist-credentials"])
        self.assertEqual(gate["steps"][-1]["run"], "python3 scripts/ci_nightly_freshness.py")
        for key in ["coverage-nightly", "mutants-nightly-advisory"]:
            self.assertEqual(self.jobs[key]["needs"], "nightly-gate")
            self.assertEqual(self.jobs[key]["if"], "needs.nightly-gate.outputs.fresh == 'true'")
        self.assertEqual(self.mutation["strategy"]["max-parallel"], 8)
        self.assertFalse(self.mutation["strategy"]["fail-fast"])
        self.assertTrue(self.mutation["continue-on-error"])
        self.assertTrue(self.producer["continue-on-error"])
        self.assertEqual(self.jobs["coverage-nightly"]["timeout-minutes"], 60)
        self.assertEqual((freshness.HISTORY_LIMIT, freshness.JOB_PAGE_SIZE, freshness.JOB_PAGE_LIMIT),
                         (10, 100, 2))

    def test_expected_matrix_count_matches_live_expansion(self):
        matrix = self.mutation["strategy"]["matrix"]
        self.assertEqual(set(matrix), {"crate", "include"})
        base = [{"crate": crate} for crate in matrix["crate"]]
        extra = []
        for include in matrix["include"]:
            matches = [leg for leg in base if leg["crate"] == include["crate"]]
            if matches:
                for leg in matches:
                    leg.update(include)
            else:
                extra.append(include)
        legs = base + extra
        self.assertEqual(len(legs), freshness.MUTATION_JOBS)
        self.assertEqual(len({(x["crate"], x.get("shard", "")) for x in legs}), len(legs))
        self.assertEqual(self.mutation["name"], freshness.MUTATION)
        coverage = self.jobs["coverage-nightly"]
        self.assertEqual(coverage["name"], freshness.COVERAGE)
        self.assertEqual(sum(s.get("name") == freshness.COVERAGE_STEP for s in coverage["steps"]), 1)

    def test_marker_requires_unmasked_success_and_ci_runs_suite(self):
        marker = next(s for s in self.mutation["steps"] if s.get("name") == freshness.MUTATION_STEP)
        self.assertEqual(marker["if"], "${{ !cancelled() && steps.run_mutants.outputs.measurement_completed == 'true' }}")
        self.assertNotIn("continue-on-error", marker)
        docs = yaml.safe_load((ROOT / ".github/workflows/docs-quality.yml").read_text())
        self.assertTrue(any(s.get("run") == "python3 scripts/tests/test_ci_nightly_freshness.py"
                            for s in docs["jobs"]["quick-gates"]["steps"]))

    def producer_result(self, cargo_exit=0, ratchet_exit=0, outcomes=True, planned=3):
        # [GPT-6 Astra] Execute the actual YAML shell under GitHub's bash -e mode.
        # PATH contains only explicit stubs plus system utilities; no real cargo,
        # Python ratchet, network, build or GitHub request is reachable.
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            bin_dir = root / "bin"
            bin_dir.mkdir()
            (root / "scripts").mkdir()
            (root / "scripts/ci_nightly_freshness.py").write_text(
                (ROOT / "scripts/ci_nightly_freshness.py").read_text())
            cargo = bin_dir / "cargo"
            cargo.write_text("#!/bin/sh\nif [ \"$FIXTURE_OUTCOMES\" = true ]; then\n"
                             "mkdir -p target/mutants/test/mutants.out\n"
                             "echo '{\"caught\":1,\"missed\":1,\"timeout\":1,\"unviable\":0,\"success\":0,\"total_mutants\":3}' > target/mutants/test/mutants.out/outcomes.json\n"
                             "echo \"$FIXTURE_PLANNED\" > target/mutants/test/mutants.out/mutants.json\nfi\n"
                             "exit \"$FIXTURE_CARGO_EXIT\"\n")
            ratchet = bin_dir / "python3"
            ratchet.write_text("#!/bin/sh\nif [ \"$1\" = scripts/ci_nightly_freshness.py ]; then\n"
                               "exec \"$FIXTURE_PYTHON\" \"$@\"\nfi\nexit \"$FIXTURE_RATCHET_EXIT\"\n")
            for path in [cargo, ratchet]:
                path.chmod(0o755)
            output = root / "output"
            env = {"PATH": f"{bin_dir}:/usr/bin:/bin", "CRATE": "test", "FEATURES": "", "SHARD": "",
                   "GITHUB_OUTPUT": str(output), "GITHUB_STEP_SUMMARY": str(root / "summary"),
                   "FIXTURE_CARGO_EXIT": str(cargo_exit), "FIXTURE_RATCHET_EXIT": str(ratchet_exit),
                   "FIXTURE_OUTCOMES": str(outcomes).lower(), "FIXTURE_PYTHON": sys.executable,
                   "FIXTURE_PLANNED": json.dumps([{}] * planned)}
            result = subprocess.run(["/bin/bash", "--noprofile", "--norc", "-eo", "pipefail", "-c",
                                     self.producer["run"]], cwd=root, env=env,
                                    capture_output=True, text=True, timeout=10)
            return result.returncode, output.read_text()

    def test_actual_producer_clean_measurement_supplies_evidence(self):
        code, output = self.producer_result()
        self.assertEqual(code, 0)
        self.assertIn("measurement_completed=true\n", output)

    def test_actual_producer_masked_cargo_failure_never_supplies_evidence(self):
        for exit_code in [1, 4, 5, 6, 70, 124, 137]:
            with self.subTest(exit_code=exit_code):
                code, output = self.producer_result(cargo_exit=exit_code)
                self.assertEqual(code, 0)  # Existing advisory behavior is preserved.
                self.assertNotIn("measurement_completed=true", output)

    def test_actual_producer_missing_outcomes_never_supplies_evidence(self):
        code, output = self.producer_result(outcomes=False)
        self.assertEqual(code, 0)
        self.assertNotIn("measurement_completed=true", output)

    def test_actual_producer_completed_advisory_results_supply_evidence(self):
        for exit_code in [2, 3]:
            with self.subTest(exit_code=exit_code):
                code, output = self.producer_result(cargo_exit=exit_code)
                self.assertEqual(code, 0)
                self.assertIn("measurement_completed=true\n", output)

    def test_actual_producer_partial_outcomes_never_supply_evidence(self):
        code, output = self.producer_result(planned=4)
        self.assertEqual(code, 0)
        self.assertNotIn("measurement_completed=true", output)

    def test_actual_producer_failed_advisory_ratchet_keeps_completion_evidence(self):
        code, output = self.producer_result(ratchet_exit=1)
        self.assertEqual(code, 1)
        self.assertIn("measurement_completed=true\n", output)


class MutationCompletion(unittest.TestCase):
    def check(self, doc, planned=None, exit_code=0):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            if planned is not None:
                (root / "mutants.json").write_text(json.dumps(planned))
            (root / "outcomes.json").write_text(json.dumps(doc))
            return freshness.mutation_complete(exit_code, directory)

    def test_complete_counts_include_missed_timeout_and_unviable(self):
        doc = dict(caught=1, missed=2, timeout=3, unviable=4, success=0, total_mutants=10)
        for exit_code in [0, 2, 3]:
            with self.subTest(exit_code=exit_code):
                self.assertTrue(self.check(doc, [{}] * 10, exit_code))
        for exit_code in [1, 4, 5, 6, 70, 124, 137]:
            with self.subTest(exit_code=exit_code):
                self.assertFalse(self.check(doc, [{}] * 10, exit_code))

    def test_mutation_cli_records_false_without_changing_advisory_exit(self):
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / "output"
            with patch.object(sys, "argv", ["ci_nightly_freshness.py", "--mutation-complete", "4", directory]), \
                    patch.dict(os.environ, {"GITHUB_OUTPUT": str(output)}):
                self.assertEqual(freshness.main(), 0)
            self.assertEqual(output.read_text(), "measurement_completed=false\n")

    def test_missing_malformed_partial_or_inconsistent_counts_are_not_evidence(self):
        doc = dict(caught=1, missed=0, timeout=0, unviable=0, success=0, total_mutants=1)
        for key, value in [("caught", True), ("caught", -1), ("missed", "0"),
                           ("total_mutants", 0), ("success", 1), ("success", False)]:
            with self.subTest(key=key, value=value):
                bad = dict(doc, **{key: value})
                self.assertFalse(self.check(bad, [{}]))
        for planned in [None, {}, [], [{}, {}]]:
            self.assertFalse(self.check(doc, planned))
        for bad in [None, [], {}, {"caught": 1}]:
            self.assertFalse(self.check(bad, [{}]))

    def test_invalid_json_or_missing_outcomes_are_not_evidence(self):
        with tempfile.TemporaryDirectory() as directory:
            self.assertFalse(freshness.mutation_complete(0, directory))
            (Path(directory) / "mutants.json").write_text("not json")
            self.assertFalse(freshness.mutation_complete(0, directory))


if __name__ == "__main__":
    unittest.main()
