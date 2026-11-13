#!/usr/bin/env python3
"""Exercise the dispatch planner and evidence boundary without running proofs."""
import copy
import hashlib
import itertools
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

from _yaml_seam import yaml_or_local_skip

yaml = yaml_or_local_skip()
ROOT = Path(__file__).resolve().parents[2]
WORKFLOW = ROOT / ".github/workflows/zk-paper-timing-hosted.yml"
DOC = yaml.safe_load(WORKFLOW.read_text())
JOBS = DOC["jobs"]


def step(job, name):
    return next(s for s in JOBS[job]["steps"] if s.get("name") == name)


class HostedTiming(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.env = dict(os.environ, RUNNER_TEMP=str(self.root), GITHUB_OUTPUT=str(self.root / "outputs"),
                        FREEZE_COMMIT="a" * 40, PROVE_RUNS="5", SUITE="merkle", MODE="revealed",
                        N="1", QUERY="q1-ask-payment-returned", REPETITION="3",
                        GITHUB_RUN_ID="fixture", GITHUB_RUN_ATTEMPT="1")

    def run_step(self, job, name, **env):
        script = step(job, name)["run"]
        # The container need not have GNU time. This stand-in only records that
        # the workflow invoked it and passes through to the actual command.
        script = script.replace("/usr/bin/time", str(self.root / "time"))
        return subprocess.run(["bash", "-euo", "pipefail", "-c", script], cwd=ROOT,
                              env=dict(self.env, **env), text=True, capture_output=True, timeout=30)

    def test_dispatch_only_read_only_and_pinned(self):
        self.assertEqual(set(DOC.get("on", DOC.get(True))), {"workflow_dispatch"})
        self.assertEqual(DOC["permissions"], {"contents": "read"})
        self.assertFalse(DOC["concurrency"]["cancel-in-progress"])
        for job in JOBS.values():
            self.assertNotIn("permissions", job)
            for item in job["steps"]:
                if "uses" in item:
                    self.assertRegex(item["uses"], r"^[\w/-]+@[0-9a-f]{40}$")
                if item.get("uses", "").startswith("actions/checkout@"):
                    self.assertFalse(item["with"]["persist-credentials"])
        self.assertEqual(JOBS["prove"]["steps"][0]["with"]["ref"], "${{ needs.prepare.outputs.commit }}")

    def test_contract_suite_is_reached_from_quality_workflow(self):
        quality = yaml.safe_load((WORKFLOW.parent / "docs-quality.yml").read_text())
        self.assertTrue(any(s.get("run") == "python3 scripts/tests/test_zk_paper_timing_hosted.py"
                            for job in quality["jobs"].values() for s in job.get("steps", [])))

    def test_installer_is_the_existing_checksum_pinned_step(self):
        genuine = yaml.safe_load((WORKFLOW.parent / "zk-authrdf-genuine.yml").read_text())
        name = "Install checksum-pinned RISC Zero binaries and guest Rust"
        expected = next(s["run"] for s in genuine["jobs"]["genuine"]["steps"] if s.get("name") == name)
        for job in JOBS:
            self.assertEqual(step(job, name)["run"], expected)

    def test_inputs_reject_empty_campaigns_and_noncommit_refs(self):
        name = "Validate dispatch inputs"
        for runs in ("0", "-1", "1.5", "7", "1; exit 0", ""):
            self.assertNotEqual(self.run_step("prepare", name, PROVE_RUNS=runs).returncode, 0, runs)
        for commit in ("main", "abc", "a" * 39, "z" * 40, "$(exit 0)"):
            self.assertNotEqual(self.run_step("prepare", name, FREEZE_COMMIT=commit).returncode, 0, commit)
        for commit in ("", "a" * 40):
            self.assertEqual(self.run_step("prepare", name, FREEZE_COMMIT=commit).returncode, 0)

    def test_planner_covers_every_configuration_query_and_repetition(self):
        queries = {p.stem for p in (ROOT / "zk/sparql-evaluator/fixtures/paper").glob("*.rq")}
        for count in (1, 5, 6):
            result = self.run_step("prepare", "Plan independent proof jobs", PROVE_RUNS=str(count))
            self.assertEqual(result.returncode, 0, result.stderr)
            lines = (self.root / "outputs").read_text().splitlines()
            matrix = json.loads(next(l[7:] for l in reversed(lines) if l.startswith("matrix=")))
            rows = matrix["include"]
            expected = set(itertools.product(["eddsa", "merkle"], ["hidden", "revealed"], [1, 4], queries, range(1, count + 1)))
            actual = {(r["suite"], r["mode"], r["n"], r["query"], r["repetition"]) for r in rows}
            self.assertEqual(actual, expected)
            self.assertEqual(len(rows), len(expected))
            self.assertLessEqual(len(rows), 256)

    def test_job_budgets_leave_the_requested_margin_and_keep_partial_evidence(self):
        job = JOBS["prove"]
        self.assertLessEqual(job["timeout-minutes"], 360)
        self.assertLessEqual(sum(s["timeout-minutes"] for s in job["steps"]), job["timeout-minutes"] * 0.6)
        self.assertFalse(job["strategy"]["fail-fast"])
        upload = step("prove", "Upload evidence, including partial output")
        self.assertEqual(upload["if"], "always()")
        self.assertEqual(upload["with"]["path"], "${{ runner.temp }}/paper-evidence")
        self.assertEqual(upload["with"]["if-no-files-found"], "error")

    def test_partial_retry_reuses_the_prepared_build_artifact(self):
        result = self.run_step("prepare", "Plan independent proof jobs")
        self.assertEqual(result.returncode, 0, result.stderr)
        output = dict(line.split("=", 1) for line in (self.root / "outputs").read_text().splitlines())
        self.assertEqual(output["build_artifact"], "paper-timing-build-fixture-1")
        # On a retry of failed proof jobs, prepare retains the original outputs.
        # The retry's run_attempt must not be used to reconstruct this name.
        self.assertEqual(JOBS["prepare"]["outputs"]["build_artifact"], "${{ steps.plan.outputs.build_artifact }}")
        upload = next(s for s in JOBS["prepare"]["steps"] if s.get("uses", "").startswith("actions/upload-artifact@"))
        download = next(s for s in JOBS["prove"]["steps"] if s.get("uses", "").startswith("actions/download-artifact@"))
        self.assertEqual(upload["with"]["name"], "${{ steps.plan.outputs.build_artifact }}")
        self.assertEqual(download["with"]["name"], "${{ needs.prepare.outputs.build_artifact }}")

    def fixture(self):
        build = self.root / "paper-build"
        build.mkdir()
        out = self.root / "paper-evidence"
        out.mkdir()
        (out / "machine.json").write_text(json.dumps({"commit": "a" * 40, "environment": "indicative"}))
        binary = build / "paper_sweep"
        binary.write_text('#!/bin/bash\nif [[ ${FAKE_EXIT:-0} != 0 ]]; then exit "$FAKE_EXIT"; fi\nprintf "%s\\n" "$FAKE_RECORD"\n')
        binary.chmod(0o755)
        timer = self.root / "time"
        timer.write_text('#!/bin/bash\n[[ $1 == -v && $2 == -o ]] || exit 2\necho "resource log" > "$3"\nshift 3\nexec "$@"\n')
        timer.chmod(0o755)
        return {"set": "main", "id": self.env["QUERY"], "suite": "eddsa-sha256-merkle-2026",
                "mode": "revealed", "n": 1, "admitted": True, "query_profile": "accepted",
                "execution": {"segments": 4}, "proofs": [{"prove_ns": 123, "receipt_kind": "succinct"}]}

    def test_valid_record_is_annotated_and_raw_output_is_retained(self):
        record = self.fixture()
        result = self.run_step("prove", "Prove one case and retain raw output", FAKE_RECORD=json.dumps(record))
        self.assertEqual(result.returncode, 0, result.stderr)
        out = self.root / "paper-evidence"
        self.assertEqual(json.loads((out / "raw.jsonl").read_text()), record)
        annotated = json.loads((out / "records.jsonl").read_text())
        self.assertEqual(annotated["environment"], "indicative")
        self.assertEqual(annotated["repetition"], 3)
        self.assertEqual(annotated["machine"]["commit"], "a" * 40)
        self.assertEqual(annotated["proofs"], record["proofs"])
        self.assertTrue((out / "time-v.txt").read_text())
        self.assertEqual((out / "exit-status.txt").read_text().strip(), "0")

    def test_missing_mismatched_and_incomplete_proofs_fail(self):
        original = self.fixture()
        variants = []
        for key, value in (("id", "unknown"), ("suite", "eddsa-rdfc-2022"), ("mode", "hidden"),
                           ("n", 4), ("admitted", False), ("query_profile", "rejected"),
                           ("proofs", []), ("proofs", original["proofs"] * 2),
                           ("proofs", [{"prove_ns": 0, "receipt_kind": "succinct"}]),
                           ("proofs", [{"prove_ns": 123, "receipt_kind": "fake"}]),
                           ("execution", {"segments": 0}), ("execution", {"segments": 4, "error": "failed"})):
            record = copy.deepcopy(original)
            record[key] = value
            variants.append(json.dumps(record))
        variants.extend(["", json.dumps(original) + "\n" + json.dumps(original)])
        for raw in variants:
            with self.subTest(raw=raw):
                result = self.run_step("prove", "Prove one case and retain raw output", FAKE_RECORD=raw)
                self.assertNotEqual(result.returncode, 0)
                self.assertTrue((self.root / "paper-evidence/time-v.txt").exists())

    def test_hidden_four_credential_cells_are_deferred_without_proving(self):
        record = self.fixture()
        for suite in ("eddsa", "merkle"):
            result = self.run_step("prove", "Prove one case and retain raw output",
                                   MODE="hidden", N="4", SUITE=suite, FAKE_EXIT="99",
                                   FAKE_RECORD=json.dumps(record))
            self.assertEqual(result.returncode, 0, result.stderr)
            out = self.root / "paper-evidence"
            coverage = json.loads((out / "coverage.jsonl").read_text())
            self.assertEqual(coverage["status"], "deferred_pending_calibration")
            self.assertEqual((coverage["suite"], coverage["mode"], coverage["n"]), (suite, "hidden", 4))
            self.assertEqual(coverage["environment"], "indicative")
            self.assertFalse((out / "raw.jsonl").exists())
            self.assertFalse((out / "time-v.txt").exists())

    def test_planned_active_cells_exclude_only_hidden_four(self):
        result = self.run_step("prepare", "Plan independent proof jobs")
        self.assertEqual(result.returncode, 0, result.stderr)
        rows = json.loads(next(line[7:] for line in (self.root / "outputs").read_text().splitlines()
                               if line.startswith("matrix=")))["include"]
        active = [r for r in rows if (r["mode"], r["n"]) != ("hidden", 4)]
        self.assertEqual(len(active), 150)
        self.assertEqual(len(rows) - len(active), 50)

    def test_failed_prover_keeps_its_status_and_resource_log(self):
        record = self.fixture()
        result = self.run_step("prove", "Prove one case and retain raw output", FAKE_RECORD=json.dumps(record), FAKE_EXIT="124")
        self.assertNotEqual(result.returncode, 0)
        out = self.root / "paper-evidence"
        self.assertEqual((out / "exit-status.txt").read_text().strip(), "124")
        self.assertTrue((out / "time-v.txt").exists())
        self.assertFalse((out / "records.jsonl").exists())

    def machine_fixture(self):
        self.fixture()
        build = self.root / "paper-build"
        (build / "commit.txt").write_text("a" * 40 + "\n")
        (build / "build-rustc.txt").write_text("build compiler\n")
        (build / "guest").mkdir()
        (build / "expected-audit.json").write_text('{"guest_image_id":"0100000002000000030000000400000005000000060000000700000008000000"}')
        self.env["GITHUB_WORKSPACE"] = str(self.root / "source")
        expected_dir = self.root / "source/research/zk-paper-evidence"
        expected_dir.mkdir(parents=True)
        (expected_dir / "paper-executor-main-audit.json").write_text((build / "expected-audit.json").read_text())
        (build / "guest/pin.json").write_text('{"image_id":[1,2,3,4,5,6,7,8]}')
        digest = hashlib.sha256((build / "paper_sweep").read_bytes()).hexdigest()
        (build / "binary.sha256").write_text(digest + "  paper_sweep\n")
        bin_dir = self.root / "bin"
        bin_dir.mkdir()
        for name in ("r0vm", "rustc"):
            program = bin_dir / name
            program.write_text('#!/bin/bash\necho "fixture version"\n')
            program.chmod(0o755)
        self.env.update(PATH=str(bin_dir) + os.pathsep + self.env["PATH"],
                        RISC0_SERVER_PATH=str(bin_dir / "r0vm"), GITHUB_SHA="b" * 40,
                        GITHUB_RUN_ID="test-run", GITHUB_RUN_ATTEMPT="test-attempt")

    def test_machine_metadata_binds_the_binary_pin_and_commit(self):
        self.machine_fixture()
        result = self.run_step("prove", "Record the machine and exact build")
        self.assertEqual(result.returncode, 0, result.stderr)
        machine = json.loads((self.root / "paper-evidence/machine.json").read_text())
        self.assertEqual(machine["environment"], "indicative")
        self.assertEqual(machine["commit"], "a" * 40)
        self.assertEqual(machine["workflow_commit"], "b" * 40)
        self.assertEqual(machine["guest_pin"]["image_id"], list(range(1, 9)))
        self.assertGreater(machine["core_count"], 0)
        self.assertTrue(machine["cpu"] and machine["memory"] and machine["rustc"] and machine["r0vm"])

    def test_changed_expected_guest_is_rejected_before_timing(self):
        self.machine_fixture()
        pin = self.root / "paper-build/guest/pin.json"
        pin.write_text('{"image_id":[9,2,3,4,5,6,7,8]}')
        result = self.run_step("prove", "Record the machine and exact build")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("guest differs", result.stderr)
        self.assertFalse((self.root / "paper-evidence/time-v.txt").exists())

    def test_changed_binary_or_source_commit_is_rejected(self):
        self.machine_fixture()
        build = self.root / "paper-build"
        result = self.run_step("prove", "Record the machine and exact build", FREEZE_COMMIT="c" * 40)
        self.assertNotEqual(result.returncode, 0)
        (build / "paper_sweep").write_text("changed binary")
        result = self.run_step("prove", "Record the machine and exact build")
        self.assertNotEqual(result.returncode, 0)


if __name__ == "__main__":
    unittest.main()
