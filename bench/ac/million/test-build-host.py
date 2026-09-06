#!/usr/bin/env python3
"""[GPT-6] Local safety checks for build-only staging; never accesses AWS."""
from decimal import Decimal
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import tarfile
import tempfile
import time
import unittest
from unittest.mock import patch
from unittest.mock import Mock

HERE = Path(__file__).parent


def load(name):
    spec = importlib.util.spec_from_file_location(name.replace("-", "_"), HERE / (name + ".py"))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


jobs = load("build-jobs")
cost = load("build-cost")
launcher_tests = load("test-launcher")


class BuildHostTests(unittest.TestCase):
    def job(self):
        return dict(id="test-job", source_commit="a" * 40, archive_sha256="b" * 64,
                    timeout_seconds=60, commands=[["cargo", "test", "--locked", "-p", "sparq-core"]])

    def test_jobs_require_source_identity_bounded_time_and_build_commands(self):
        jobs.validate_job(self.job())
        jobs.validate_job(dict(self.job(), commands=[["cargo", "doc", "--locked", "--workspace", "--no-deps", "--all-features"]]))
        for value in (0, -1, 12601, True):
            with self.subTest(timeout=value), self.assertRaises(ValueError):
                jobs.validate_job(dict(self.job(), timeout_seconds=value))
        for argv in (["cargo", "bench", "--locked"], ["cargo", "run", "--locked"],
                     ["bash", "-c", "true"], ["cargo", "test"], ["python3", "../test_bad.py"]):
            with self.subTest(argv=argv), self.assertRaises(ValueError):
                jobs.validate_job(dict(self.job(), commands=[argv]))
        with self.assertRaises(ValueError):
            jobs.validate_job(dict(self.job(), id="../escape"))

    def test_low_disk_prevents_source_extraction_or_process_start(self):
        with tempfile.TemporaryDirectory() as directory, patch.object(jobs, "BASE", Path(directory)), \
             patch.object(jobs.shutil, "disk_usage", return_value=type("Disk", (), {"free": 19 * jobs.GIB})()), \
             patch.object(jobs.subprocess, "Popen") as process:
            with self.assertRaisesRegex(ValueError, "disk admission"):
                jobs.run_job(self.job(), 9999999999)
            process.assert_not_called()

    def test_failed_extraction_cleans_owned_source_and_archive(self):
        with tempfile.TemporaryDirectory() as directory, patch.object(jobs, "BASE", Path(directory)):
            root = Path(directory)
            (root / "sources" / "test-job").mkdir(parents=True)
            (root / "inbox").mkdir()
            archive = root / "inbox" / "test-job.tar.gz"
            archive.write_text("fixture")
            with patch.object(jobs, "execute_job", side_effect=ValueError("fixture failed extraction")):
                with self.assertRaises(ValueError):
                    jobs.run_job(self.job(), 9999999999)
            self.assertFalse(archive.exists())
            self.assertFalse((root / "sources" / "test-job").exists())

    def test_exited_process_leader_does_not_leave_descendants_alive(self):
        process = Mock(pid=12345)
        process.poll.return_value = 0
        with patch.object(jobs.os, "killpg") as kill:
            jobs.stop_process(process)
        self.assertEqual([call.args for call in kill.call_args_list],
                         [(12345, jobs.signal.SIGTERM), (12345, jobs.signal.SIGKILL)])

    def test_complete_small_python_job_binds_source_logs_and_cleans_staging(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            base, results, fixture = root / "host", root / "results", root / "fixture"
            (base / "inbox").mkdir(parents=True)
            results.mkdir()
            fixture.mkdir()
            (fixture / "Cargo.lock").write_text("fixture lock\n")
            (fixture / "rust-toolchain.toml").write_text("fixture toolchain\n")
            (fixture / "test_fixture.py").write_text('print("functional fixture passed")\n')
            archive = base / "inbox" / "test-job.tar.gz"
            with tarfile.open(archive, "w:gz") as stream:
                for path in fixture.iterdir():
                    stream.add(path, arcname=path.name)
            job = dict(self.job(), archive_sha256=jobs.digest(archive), commands=[["python3", "test_fixture.py"]])
            sleep = time.sleep
            with patch.object(jobs, "BASE", base), patch.object(jobs, "RESULTS", results), \
                 patch.object(jobs.shutil, "disk_usage", return_value=type("Disk", (), {"free": 100 * jobs.GIB})()), \
                 patch.object(jobs.time, "sleep", side_effect=lambda _: sleep(.01)):
                result = jobs.run_job(job, time.time() + 1800)
            self.assertEqual(result["status"], "passed")
            self.assertEqual(result["source_commit"], job["source_commit"])
            self.assertEqual(result["commands_run"][0]["sha256"], jobs.digest(results / "test-job-0.log"))
            self.assertEqual(result["build_environment"]["RUSTDOCFLAGS"], "-D warnings")
            self.assertFalse(archive.exists())
            self.assertFalse((base / "sources" / "test-job").exists())

    def test_final_marker_follows_manifest_and_all_evidence_hashes_match(self):
        with tempfile.TemporaryDirectory() as directory, patch.object(jobs, "RESULTS", Path(directory)):
            root = Path(directory)
            (root / "READY").touch()
            (root / "result.json").write_text('{"status":"fixture"}\n')
            jobs.finalize("DONE")
            self.assertTrue((root / "DONE").exists())
            self.assertFalse((root / "READY").exists())
            for line in (root / "MANIFEST.sha256").read_text().splitlines():
                checksum, filename = line.split("  ", 1)
                self.assertEqual(jobs.digest(root / filename), checksum)
            self.assertNotIn("DONE", (root / "MANIFEST.sha256").read_text())

    def test_build_constraints_fail_before_any_cloud_command(self):
        for changes in ({"SPARQ_POD_INSTANCE_TYPE": "r7g.8xlarge"},
                        {"SPARQ_POD_VOLUME_GB": "201"},
                        {"SPARQ_POD_WATCHDOG_SECONDS": "14401"}):
            environment = dict(os.environ, SPARQ_POD_PRIOR_AWS_USD="19.6681904444", **changes)
            result = subprocess.run(["/bin/bash", str(HERE / "build-host.sh")],
                                    env=environment, capture_output=True, text=True, timeout=5)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("build", result.stderr)
            self.assertNotIn("orphan check", result.stderr)

    def test_completed_bundle_must_match_captured_commit_before_paid_setup(self):
        source = (HERE / "launch-ec2.sh").read_text()
        guard = source[source.index("verify_bundle_source() {"):source.index('\ncase "${MODE}" in')]
        self.assertLess(source.index('verify_bundle_source "${BUNDLE}" "${SOURCE_COMMIT}"'),
                        source.index('aws ec2 import-key-pair'))
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            def git(*args):
                return subprocess.check_output(["git", "-c", "commit.gpgsign=false", *args], cwd=root, stderr=subprocess.DEVNULL, text=True).strip()
            git("init", "-q")
            git("config", "user.name", "Fixture")
            git("config", "user.email", "fixture@example.invalid")
            (root / "source.txt").write_text("captured\n")
            git("add", "source.txt")
            git("commit", "-q", "-m", "captured")
            captured = git("rev-parse", "HEAD")
            git("bundle", "create", "frozen.bundle", "HEAD")
            (root / "source.txt").write_text("changed during pricing\n")
            git("commit", "-q", "-am", "moved")
            git("bundle", "create", "changed.bundle", "HEAD")
            script = 'set -euo pipefail\ndie() { exit 9; }\n' + guard + '\nverify_bundle_source "$1" "$2"'
            for bundle, expected in (("frozen.bundle", 0), ("changed.bundle", 9)):
                result = subprocess.run(["/bin/bash", "-c", script, "fixture", str(root / bundle), captured],
                                        capture_output=True, text=True, timeout=5)
                self.assertEqual(result.returncode, expected, result.stderr)

    def test_fixed_cost_envelope_counts_only_new_storage_and_ipv4(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "prices.csv"
            path.write_text('"SKU","Region Code","Volume API Name","Product Family","TermType","Unit","Currency","PricePerUnit"\n'
                            '"fixture","eu-west-2","gp3","Storage","OnDemand","GB-Mo","USD","0.0928"\n')
            with patch.object(cost.price, "bulk_csv_hourly_price", return_value=(Decimal("0.6869"), {})):
                result = cost.reservation(path, Decimal("19.6681904444"))
                self.assertLess(Decimal(result["maximum_planned_run"]), Decimal("3.10"))
                self.assertEqual(Decimal(result["maximum_accumulated_study_spend"]),
                                 Decimal("19.6681904444") + Decimal(result["maximum_planned_run"]))
                self.assertLess(Decimal(result["ancillary_ebs_ipv4_reserve"]), Decimal(1))
                with self.assertRaises(cost.price.PricingError):
                    cost.reservation(path, Decimal(99))
                with self.assertRaises(ValueError):
                    cost.reservation(path, Decimal("NaN"))

    def test_cleanup_never_terminates_protected_or_wrong_tagged_instances(self):
        helper = launcher_tests.LauncherTests()
        with tempfile.TemporaryDirectory() as directory:
            result, root, _, _ = helper.execute(directory, 'INSTANCE_ID="$PROD_INSTANCE"; exit 1', cleanup=True)
            self.assertNotEqual(result.returncode, 0)
            calls = (root / "aws").read_text().splitlines()
            self.assertFalse(any("terminate-instances" in c and "i-protected" in c for c in calls))
        with tempfile.TemporaryDirectory() as directory:
            body = '''
aws() {
  printf '%s\\n' "$*" >> "$AWS_TRACE"
  if [[ "$*" == *--filters* ]]; then printf 'i-fixture\\n'
  elif [[ "$*" == *'Key==`purpose`'* ]]; then printf 'production\\n'
  elif [[ "$*" == *'Key==`study-run`'* ]]; then printf '%s\\n' "$RUN_TOKEN"
  fi
}
exit 1
'''
            result, root, _, _ = helper.execute(directory, body, cleanup=True)
            self.assertNotEqual(result.returncode, 0)
            self.assertNotIn("terminate-instances", (root / "aws").read_text())


if __name__ == "__main__":
    unittest.main()
