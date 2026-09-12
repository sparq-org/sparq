"""[GPT-6] Mandatory evaluator selection must never hide an uncertain diff."""

import importlib.util
from pathlib import Path
import subprocess
import unittest
from unittest.mock import patch

SPEC = importlib.util.spec_from_file_location(
    "ci_exact_evaluator_paths", Path(__file__).parents[1] / "ci_exact_evaluator_paths.py"
)
selector = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(selector)
BASE, HEAD = "a" * 40, "b" * 40


class ExactEvaluatorSelection(unittest.TestCase):
    def test_workflow_always_creates_the_gate_and_checks_classification(self):
        workflow = (Path(__file__).parents[2] / ".github/workflows/zk-exact-evaluator.yml").read_text()
        self.assertIn("  merge_group:\n    types: [checks_requested]", workflow)
        self.assertIn("name: real exact-dataset guest and proof", workflow)
        self.assertNotIn("    paths:", workflow)
        self.assertIn("Require explicit classification output", workflow)
        self.assertIn("python3 scripts/ci_exact_evaluator_paths.py", workflow)
        self.assertIn("python3 scripts/ci_exact_evaluator_evidence.py", workflow)

    def test_every_execution_input_is_relevant(self):
        for path in ["zk/sparql-evaluator/fixtures/case.json", "crates/sparq-engine/src/exec.rs",
                     "crates/sparq-core/Cargo.toml", "crates/sparq-substrate/src/lib.rs",
                     "vendor/spargebra/src/parser.rs", ".cargo/config.toml", "Cargo.lock",
                     "Cargo.toml", "rust-toolchain.toml", ".github/workflows/zk-exact-evaluator.yml",
                     "scripts/ci_exact_evaluator_paths.py", "scripts/ci_exact_evaluator_evidence.py",
                     "scripts/tests/test_ci_exact_evaluator_evidence.py", "vendor/zk-sdk/risc0-build/src/lib.rs"]:
            self.assertTrue(selector.relevant_path(path), path)

    def test_explicit_irrelevant_diff_only_skips_heavy_steps(self):
        with patch.object(selector.subprocess, "run", side_effect=[
            subprocess.CompletedProcess([], 0),
            subprocess.CompletedProcess([], 0, b"docs/design.md\0site/page.tsx\0"),
        ]) as run:
            self.assertFalse(selector.requires_execution("merge_group", BASE, HEAD))
            self.assertIn("--no-renames", run.call_args.args[0])

    def test_relevant_deletion_or_rename_requires_execution(self):
        with patch.object(selector.subprocess, "run", side_effect=[
            subprocess.CompletedProcess([], 0),
            subprocess.CompletedProcess([], 0, b"zk/sparql-evaluator/model/src/lib.rs\0docs/moved.txt\0"),
        ]):
            self.assertTrue(selector.requires_execution("pull_request", BASE, HEAD))

    def test_failed_or_truncated_diff_requires_execution(self):
        for outcome in [subprocess.CalledProcessError(1, "git"),
                        subprocess.TimeoutExpired("git", 60),
                        subprocess.CompletedProcess([], 0, b"docs/design.md")]:
            with patch.object(selector.subprocess, "run", side_effect=[
                subprocess.CompletedProcess([], 0), outcome,
            ]):
                self.assertTrue(selector.requires_execution("merge_group", BASE, HEAD))

    def test_failed_fetch_and_missing_context_require_execution(self):
        with patch.object(selector.subprocess, "run", side_effect=OSError()):
            self.assertTrue(selector.requires_execution("push", BASE, HEAD))
        with patch.object(selector.subprocess, "run") as run:
            for event, base, head in [("pull_request", "", HEAD), ("push", "0" * 40, HEAD),
                                      ("merge_group", BASE, "--bad"), ("workflow_dispatch", "", "")]:
                self.assertTrue(selector.requires_execution(event, base, head))
            run.assert_not_called()


if __name__ == "__main__":
    unittest.main()
