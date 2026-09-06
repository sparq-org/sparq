#!/usr/bin/env python3
"""[GPT-6] Exercise actual launcher functions under native Bash; no network commands."""
import os
from pathlib import Path
import shlex
import subprocess
import tempfile
import unittest


SOURCE = Path(__file__).with_name("launch-ec2.sh").read_text()
PULL = SOURCE[SOURCE.index("stage_pull() {"):SOURCE.index("\nvalid_ipv4() {")]
CLEANUP = SOURCE[SOURCE.index("cleanup() {"):SOURCE.index("\ntrap cleanup EXIT")]
FINAL = SOURCE[SOURCE.rindex("refresh_ssh_ingress\nstage_pull final"):]
BASH = os.environ.get("BASH_TEST_BIN", "/bin/bash")


class LauncherTests(unittest.TestCase):
    def execute(self, directory, body, mode="canonical", rsync_status=0, hash_status=0,
                manifest=True, cleanup=False):
        root = Path(directory)
        results = root / "results with spaces"
        results.mkdir()
        work = root / "owned-work"
        work.mkdir()
        if manifest:
            (results / "MANIFEST.sha256").write_text("fixture consumed by mocked sha256sum\n")
        values = {"MODE": mode, "RESULTS_LOCAL": str(results), "WORK": str(work),
                  "PUBLIC_IP": "192.0.2.1", "PROFILE": "fixture", "REGION": "fixture",
                  "RUN_TOKEN": "fixture-token", "KEY_NAME": "fixture-key", "SUCCESS": "1",
                  "INSTANCE_ID": "i-fixture", "PROD_INSTANCE": "i-protected-prod",
                  "DEV_INSTANCE": "i-protected-dev", "SECURITY_GROUP_ID": "",
                  "RETRIEVAL_COMPLETE": "0", "MOCK_RSYNC_STATUS": str(rsync_status),
                  "MOCK_HASH_STATUS": str(hash_status), "TRACE": str(root / "args"),
                  "AWS_TRACE": str(root / "aws")}
        script = "set -euo pipefail\n" + "\n".join(
            key + "=" + shlex.quote(value) for key, value in values.items()) + "\n"
        # Functions shadow every network command reachable from these source slices.
        script += r'''
SSH_OPTIONS=(-o BatchMode=yes -o StrictHostKeyChecking=yes)
rsync() { printf '%s\0' "$@" > "$TRACE"; return "$MOCK_RSYNC_STATUS"; }
ssh() { printf 'fixture journal\n'; }
sha256sum() { return "$MOCK_HASH_STATUS"; }
refresh_ssh_ingress() { :; }
log() { :; }
die() { printf '%s\n' "$*" >&2; exit 1; }
aws() {
  printf '%s\n' "$*" >> "$AWS_TRACE"
  if [[ "$*" == *--filters* ]]; then printf 'i-fixture\n'
  elif [[ "$*" == *'Key==`purpose`'* ]]; then printf 'sparq-bench\n'
  elif [[ "$*" == *'Key==`study-run`'* ]]; then printf '%s\n' "$RUN_TOKEN"
  fi
}
'''
        script += PULL
        if cleanup:
            script += CLEANUP + "\ntrap cleanup EXIT\n"
        result = subprocess.run([BASH, "-c", script + "\n" + body],
                                capture_output=True, text=True, timeout=5)
        return result, root, results, work

    def test_live_and_final_transfer_arguments_preserve_boundaries(self):
        for mode, argument in (("canonical", ""), ("canonical", "live"),
                               ("canonical", "final"), ("pilot", "live"), ("pilot", "final")):
            with self.subTest(mode=mode, argument=argument), tempfile.TemporaryDirectory() as directory:
                result, root, results, _ = self.execute(directory, "stage_pull " + argument, mode)
                self.assertEqual(result.returncode, 0, result.stderr)
                args = (root / "args").read_bytes().decode().split("\0")[:-1]
                expected = ["-az", "--partial"]
                if mode == "canonical" and argument != "final":
                    expected += ["--exclude=*-requests.jsonl", "--exclude=*-warmup.jsonl", "--exclude=*-audit.jsonl"]
                expected += ["-e", "ssh -o BatchMode=yes -o StrictHostKeyChecking=yes",
                             "ubuntu@192.0.2.1:/var/tmp/sparq-pod-study/", str(results) + "/"]
                self.assertEqual(args, expected)

    def test_live_transfer_failure_is_best_effort(self):
        with tempfile.TemporaryDirectory() as directory:
            result, _, _, _ = self.execute(directory, "stage_pull\nprintf continued", rsync_status=23)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(result.stdout, "continued")

    def test_actual_final_path_and_cleanup_preserve_failures(self):
        cases = [(0, 0, True, 0), (23, 0, True, 23), (0, 29, True, 29), (0, 0, False, 1)]
        for rsync_status, hash_status, manifest, expected in cases:
            with self.subTest(case=(rsync_status, hash_status, manifest)), tempfile.TemporaryDirectory() as directory:
                result, root, results, work = self.execute(directory, FINAL, rsync_status=rsync_status,
                                                          hash_status=hash_status, manifest=manifest, cleanup=True)
                self.assertEqual(result.returncode, expected, result.stderr)
                self.assertFalse(work.exists())
                self.assertEqual((results / "cleanup-status.txt").read_text(), "cleanup_complete\n")
                calls = (root / "aws").read_text()
                self.assertIn("ec2 terminate-instances", calls)
                self.assertIn("--instance-ids i-fixture", calls)
                self.assertNotIn("i-protected", calls)
                if not manifest:
                    self.assertIn("final results manifest is absent", result.stderr)

    def test_unfinished_exit_and_fatal_expansion_cannot_be_reported_successful(self):
        for body in ("exit 0", 'unset LAUNCHER_FIXTURE_UNSET; fatal() { printf "%s" "$LAUNCHER_FIXTURE_UNSET"; }; fatal'):
            with self.subTest(body=body), tempfile.TemporaryDirectory() as directory:
                result, _, results, work = self.execute(directory, body, cleanup=True)
                self.assertEqual(result.returncode, 1, result.stderr)
                self.assertFalse(work.exists())
                self.assertEqual((results / "cleanup-status.txt").read_text(), "cleanup_complete\n")


if __name__ == "__main__":
    unittest.main()
