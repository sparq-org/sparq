"""[GPT-6] Native proof work is required unless a complete diff proves irrelevance."""
import importlib.util
from pathlib import Path
import subprocess
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location(
    "ci_native_composition_paths", ROOT / "scripts/ci_native_composition_paths.py")
selector = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(selector)
BASE, HEAD = "a" * 40, "b" * 40


class NativeSelectionTests(unittest.TestCase):
    def test_workflow_always_creates_job_but_scopes_every_heavy_step(self):
        workflow = (ROOT / ".github/workflows/zk-native-composition.yml").read_text()
        self.assertIn("  merge_group:\n    types: [checks_requested]", workflow)
        self.assertNotIn("    paths:", workflow)
        self.assertIn("name: native credential and circuit composition", workflow)
        self.assertIn("python3 scripts/ci_native_composition_paths.py", workflow)
        self.assertIn("Require explicit classification output", workflow)
        self.assertIn("true|false) ;;", workflow)
        self.assertIn('exit 1 ;;', workflow)
        tail = workflow.split("      - name: Check out pinned Circom source", 1)[1]
        # Every original native step, including checkout/build/cache/upload, must
        # remain below the explicit selector. No job-level skip can satisfy CI.
        for step in tail.split("      - name:"):
            self.assertRegex(step, r"if: (?:always\(\) && )?steps.changes.outputs.required == 'true'")
        self.assertIn("python3 bench/zk-bindings/native_ci.py", workflow)
        self.assertIn("python3 scripts/tests/test_ci_native_composition_paths.py",
                      (ROOT / ".github/workflows/docs-quality.yml").read_text())

    def test_native_shared_and_gate_inputs_require_work(self):
        paths = ["zk/native-composition/Cargo.lock", "zk/native-composition/vendor/ark-relations-0.4.0/Cargo.toml",
                 "crates/sparq-core/src/lib.rs", "crates/sparq-canon/src/lib.rs",
                 "vendor/spargebra/src/parser.rs", ".cargo/config.toml", "Cargo.toml", "rust-toolchain.toml",
                 ".github/workflows/zk-native-composition.yml", ".github/workflows/ci-summary.yml",
                 "scripts/ci_native_composition_paths.py", "scripts/ci_summary_gate.py",
                 "scripts/tests/test_ci_native_composition_paths.py", "scripts/tests/test_ci_summary_gate.py",
                 "bench/zk-bindings/corpus.py", "bench/zk-bindings/run.py", "bench/zk-bindings/protocol.json",
                 "bench/zk-bindings/native_ci.py", "bench/zk-bindings/test_native_ci.py",
                 "bench/zk-bindings/inventory.json", "bench/zk-bindings/test_harness.py"]
        for path in paths:
            with self.subTest(path=path):
                self.assertTrue(selector.relevant_path(path))
                with patch.object(selector.subprocess, "run", side_effect=[
                    subprocess.CompletedProcess([], 0), subprocess.CompletedProcess([], 0, path.encode() + b"\0")]):
                    self.assertTrue(selector.requires_execution("pull_request", BASE, HEAD))

    def test_unrelated_and_exact_only_inputs_do_not_trigger_native_proving(self):
        paths = ["docs/design.md", "site/page.tsx", "Cargo.lock", "zk/sparql-evaluator/model/src/lib.rs",
                 "scripts/ci_exact_evaluator_paths.py", "bench/zk-bindings/exact_ci.py",
                 "bench/zk-bindings/version-expectations.json"]
        for path in paths:
            self.assertFalse(selector.relevant_path(path), path)
        for event in ("pull_request", "push", "merge_group"):
            with patch.object(selector.subprocess, "run", side_effect=[
                subprocess.CompletedProcess([], 0),
                subprocess.CompletedProcess([], 0, b"\0".join(p.encode() for p in paths) + b"\0"),
            ]) as run:
                self.assertFalse(selector.requires_execution(event, BASE, HEAD))
                self.assertIn("--no-renames", run.call_args.args[0])

    def test_deleted_or_renamed_native_input_remains_relevant(self):
        with patch.object(selector.subprocess, "run", side_effect=[
            subprocess.CompletedProcess([], 0),
            subprocess.CompletedProcess([], 0, b"zk/native-composition/src/rdf.rs\0docs/moved.txt\0"),
        ]):
            self.assertTrue(selector.requires_execution("merge_group", BASE, HEAD))

    def test_failed_or_truncated_diff_requires_execution(self):
        for outcome in [subprocess.CalledProcessError(1, "git"), subprocess.TimeoutExpired("git", 60),
                        subprocess.CompletedProcess([], 0, b"docs/design.md")]:
            with patch.object(selector.subprocess, "run", side_effect=[subprocess.CompletedProcess([], 0), outcome]):
                self.assertTrue(selector.requires_execution("merge_group", BASE, HEAD))

    def test_uncertain_event_or_revision_and_fetch_error_fail_closed(self):
        with patch.object(selector.subprocess, "run", side_effect=OSError()):
            self.assertTrue(selector.requires_execution("push", BASE, HEAD))
        with patch.object(selector.subprocess, "run") as run:
            for event, base, head in [("pull_request", "", HEAD), ("push", "0" * 40, HEAD),
                                      ("merge_group", BASE, "--bad"), ("workflow_dispatch", BASE, HEAD)]:
                self.assertTrue(selector.requires_execution(event, base, head))
            run.assert_not_called()


if __name__ == "__main__":
    unittest.main()
