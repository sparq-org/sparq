"""The result-evidence rewrite refuses non-HEAD inputs and rolls back every file it can."""
import contextlib
import importlib.util
import io
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location(
    "remeasure_result_evidence", ROOT / "bench/zk-compose/scripts/remeasure_result_evidence.py")
remeasure = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(remeasure)


def git(repo, *args):
    subprocess.run(["git", "-c", "user.name=t", "-c", "user.email=t@t", *args], cwd=repo, check=True,
                   capture_output=True)


class RemeasureWriteTests(unittest.TestCase):
    def setUp(self):
        self.repo = Path(tempfile.mkdtemp())
        self.addCleanup(subprocess.run, ["rm", "-rf", str(self.repo)])
        git(self.repo, "init", "-q")
        self.paths = [f"bench/zk-compose/{name}.json" for name in remeasure.EVIDENCE]
        for path in self.paths:
            (self.repo / path).parent.mkdir(parents=True, exist_ok=True)
            (self.repo / path).write_text(json.dumps({"file": path}) + "\n")
        git(self.repo, "add", "-A")
        git(self.repo, "commit", "-q", "-m", "evidence")
        patcher = patch.object(remeasure, "ROOT", self.repo)
        patcher.start()
        self.addCleanup(patcher.stop)

    def test_clean_tree_passes(self):
        remeasure.require_clean(self.paths)

    def test_staged_edit_is_refused(self):
        (self.repo / self.paths[0]).write_text("{}\n")
        git(self.repo, "add", self.paths[0])
        with self.assertRaises(SystemExit):
            remeasure.require_clean(self.paths)

    def test_unstaged_edit_is_refused(self):
        (self.repo / self.paths[1]).write_text("{}\n")
        with self.assertRaises(SystemExit):
            remeasure.require_clean(self.paths)

    def test_baseline_is_read_without_touching_the_index(self):
        (self.repo / self.paths[0]).write_text(json.dumps({"file": "newer"}) + "\n")
        git(self.repo, "commit", "-q", "-am", "newer")
        self.assertEqual(remeasure.load_revision("HEAD~1")["result_v1_compatibility"],
                         {"file": self.paths[0]})
        remeasure.require_clean(self.paths)

    def test_every_replaced_file_is_restored_even_if_one_restore_fails(self):
        targets = [self.repo / path for path in self.paths]
        originals = {target: target.read_bytes() for target in targets}
        real_replace, real_write = os.replace, Path.write_bytes
        replacements = []

        def replace(source, target):
            replacements.append(target)
            if len(replacements) == 4:
                raise OSError("disk full")
            real_replace(source, target)

        def write_bytes(path, data):
            if path == targets[0]:
                raise OSError("read-only")
            return real_write(path, data)

        stderr = io.StringIO()
        with patch.object(remeasure.os, "replace", replace), patch.object(Path, "write_bytes", write_bytes), \
                contextlib.redirect_stderr(stderr), self.assertRaises(OSError):
            remeasure.replace_all({target: {"new": True} for target in targets})
        self.assertNotEqual(targets[0].read_bytes(), originals[targets[0]])
        for target in targets[1:]:
            self.assertEqual(target.read_bytes(), originals[target], target)
        self.assertIn(str(targets[0]), stderr.getvalue())
        self.assertEqual(sorted(p.name for p in targets[0].parent.iterdir()),
                         sorted(target.name for target in targets))


if __name__ == "__main__":
    unittest.main()
