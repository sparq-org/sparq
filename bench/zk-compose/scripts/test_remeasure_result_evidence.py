"""Isolated regression tests; no real evidence or measurement tools are used."""

import argparse
import contextlib
import importlib.util
import io
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest import mock


SCRIPT = Path(__file__).with_name("remeasure_result_evidence.py")
SPEC = importlib.util.spec_from_file_location("remeasure_result_evidence", SCRIPT)
remeasure = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(remeasure)


class ApplyTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        self.enterContext(mock.patch.object(remeasure, "ROOT", self.root))
        self.enterContext(mock.patch.object(remeasure, "measure", side_effect=AssertionError("no measurement")))
        self.evidence_dir = self.root / "bench/zk-compose"
        self.evidence_dir.mkdir(parents=True)
        self.paths = [self.evidence_dir / f"{name}.json" for name in remeasure.EVIDENCE]
        for path in self.paths:
            path.write_text(json.dumps({"members": {}, "source_files": {},
                                        "tool_versions": {"nargo": "fixture", "bb": "fixture"}}))
        (self.evidence_dir / "gate_counts_latest.json").write_text('{"benchmarks": {}}')
        self.originals = {path: path.read_bytes() for path in self.paths}
        self.git("init", "-q")
        self.git("add", "--", "bench/zk-compose")
        self.git("-c", "user.name=Test", "-c", "user.email=test@example.invalid",
                 "-c", "commit.gpgsign=false", "-c", "core.hooksPath=/dev/null",
                 "commit", "-qm", "fixture")
        run_file = self.root / "run.json"
        run_file.write_text(json.dumps({"members": {}, "source_files": {}, "tools": {
            "nargo": "fixture", "bb": "fixture", "nargo_sha256": "fixture", "bb_sha256": "fixture"}}))
        self.args = argparse.Namespace(first=run_file, second=run_file, job=run_file,
                                       commit="HEAD", observed_at="fixture", platform="fixture",
                                       expect_member=[], expect_source=[], baseline_root=self.root)
        # apply only needs a tree lookup; poison any accidental tool invocation.
        def tree_lookup(command, cwd):
            self.assertEqual(command, ["git", "rev-parse", "HEAD^{tree}"])
            self.assertEqual(cwd, self.root)
            return "fixture-tree\n"
        self.enterContext(mock.patch.object(remeasure, "run", side_effect=tree_lookup))

    def git(self, *arguments):
        return subprocess.run(["git", *arguments], cwd=self.root, check=True,
                              capture_output=True, text=True)

    def apply(self):
        with contextlib.redirect_stdout(io.StringIO()):
            remeasure.apply(self.args)

    def assert_no_temporaries(self):
        self.assertEqual(list(self.evidence_dir.glob(".*.json")), [])

    def check_failed_replacement(self, fail_at, failed_restore_indices):
        real_replace, real_write = os.replace, Path.write_bytes
        replacements, restores = [], []
        failed_paths = [self.paths[index] for index in failed_restore_indices]

        def replace(source, target):
            replacements.append(target)
            if len(replacements) == fail_at:
                raise OSError("injected replacement failure")
            real_replace(source, target)

        def restore(target, data):
            restores.append(target)
            if target in failed_paths:
                raise OSError("injected restore failure")
            return real_write(target, data)

        with mock.patch.object(remeasure.os, "replace", side_effect=replace), \
                mock.patch.object(Path, "write_bytes", autospec=True, side_effect=restore):
            with self.assertRaises(SystemExit) as raised:
                self.apply()
        self.assertNotEqual(raised.exception.code, 0)
        message = str(raised.exception)
        self.assertIn("injected replacement failure", message)
        self.assertIn("rollback failed", message)
        commands = [line.strip() for line in message.splitlines() if "git checkout --" in line]
        self.assertEqual(commands, [f"git checkout -- {path.relative_to(self.root)}" for path in failed_paths])
        self.assertEqual(restores, self.paths[:fail_at - 1])
        for path in self.paths:
            if path in failed_paths:
                self.assertNotEqual(path.read_bytes(), self.originals[path])
            else:
                self.assertEqual(path.read_bytes(), self.originals[path])
        self.assert_no_temporaries()

    def test_second_replace_and_first_restore_fail(self):
        self.check_failed_replacement(fail_at=2, failed_restore_indices=[0])

    def test_remaining_restores_attempted_and_all_failures_listed(self):
        # Three replacements are needed to prove continuation and failure collection.
        self.check_failed_replacement(fail_at=4, failed_restore_indices=[0, 2])

    def test_replacement_failure_with_successful_rollback(self):
        real_replace = os.replace

        def replace(source, target):
            if target == self.paths[1]:
                raise OSError("injected replacement failure")
            real_replace(source, target)

        with mock.patch.object(remeasure.os, "replace", side_effect=replace):
            with self.assertRaisesRegex(OSError, "injected replacement failure"):
                self.apply()
        self.assertEqual({path: path.read_bytes() for path in self.paths}, self.originals)
        self.assert_no_temporaries()

    def test_staged_edit_with_clean_worktree_is_refused(self):
        self.paths[0].write_bytes(self.originals[self.paths[0]] + b"\n")
        self.git("add", "--", str(self.paths[0]))
        self.git("diff", "--quiet", "--", "bench/zk-compose")
        with mock.patch.object(remeasure.os, "replace") as replace:
            with self.assertRaisesRegex(SystemExit, "staged or unstaged edits against HEAD"):
                self.apply()
            replace.assert_not_called()
        self.assertEqual(self.paths[0].read_bytes(), self.originals[self.paths[0]] + b"\n")
        self.assert_no_temporaries()

    def test_clean_tree_passes(self):
        self.apply()
        for path in self.paths:
            self.assertEqual(json.loads(path.read_text())["measured_source"]["source_commit"], "HEAD")
        self.assertEqual(self.git("diff", "--cached", "--name-only").stdout, "")
        self.assert_no_temporaries()

    def test_missing_git_fails_closed(self):
        with mock.patch.object(remeasure.subprocess, "run", side_effect=FileNotFoundError("git")):
            with self.assertRaisesRegex(SystemExit, "cannot check evidence cleanliness"):
                self.apply()
        self.assertEqual({path: path.read_bytes() for path in self.paths}, self.originals)
        self.assert_no_temporaries()

    def test_non_repository_fails_closed(self):
        with mock.patch.dict(os.environ, {"GIT_DIR": str(self.root / "missing.git")}):
            with self.assertRaisesRegex(SystemExit, "cannot check evidence cleanliness"):
                self.apply()
        self.assertEqual({path: path.read_bytes() for path in self.paths}, self.originals)
        self.assert_no_temporaries()

    def test_baseline_from_git_show_is_read_without_staging(self):
        baseline = self.root / "baseline"
        for path in self.paths:
            target = baseline / path.relative_to(self.root)
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_text(self.git("show", f"HEAD:{path.relative_to(self.root)}").stdout)
        # Distinguish the current destination from the saved pre-change input.
        self.paths[0].write_text(json.dumps({"marker": "destination"}))
        self.git("add", "--", str(self.paths[0]))
        self.git("-c", "user.name=Test", "-c", "user.email=test@example.invalid",
                 "-c", "commit.gpgsign=false", "-c", "core.hooksPath=/dev/null",
                 "commit", "-qm", "destination")
        self.args.baseline_root = baseline
        self.apply()
        for path in self.paths:
            self.assertEqual((baseline / path.relative_to(self.root)).read_bytes(), self.originals[path])
            self.assertIn("members", json.loads(path.read_text()))
        self.assertEqual(self.git("diff", "--cached", "--name-only").stdout, "")


if __name__ == "__main__":
    unittest.main()
