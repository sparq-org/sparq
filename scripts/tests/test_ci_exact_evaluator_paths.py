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
EXACT_INPUTS = ["zk/sparql-evaluator/fixtures/case.json", "crates/sparq-engine/src/exec.rs",
                "crates/sparq-core/Cargo.toml", "crates/sparq-substrate/src/lib.rs",
                "crates/sparq-canon/src/lib.rs", "crates/sparq-canon/Cargo.toml",
                "vendor/spargebra/src/parser.rs", ".cargo/config.toml", "Cargo.lock",
                "Cargo.toml", "rust-toolchain.toml", ".github/workflows/zk-exact-evaluator.yml",
                "scripts/ci_exact_evaluator_paths.py", "scripts/ci_exact_evaluator_evidence.py",
                "scripts/tests/test_ci_exact_evaluator_evidence.py", "vendor/zk-sdk/risc0-build/src/lib.rs",
                "bench/zk-bindings/exact_ci.py", "bench/zk-bindings/corpus.py",
                "bench/zk-bindings/exact-originals.json", "crates/sparq-conformance/examples/proof_corpus.rs"]
# [OPUS-5.5] Registry-parser additions: gate files and migrated caller crates.
REGISTRY_ONLY_INPUTS = ["scripts/check_registry_parser.py", "scripts/tests/test_registry_parser.py",
                        "crates/sparq-text/src/rewrite.rs", "crates/sparq-shacl/src/sparql.rs",
                        "crates/sparq-shacl/Cargo.toml", "crates/sparq-vectors/src/rewrite.rs",
                        "crates/sparq-server/src/exec.rs", "crates/sparq-solid/src/rewrite.rs",
                        "crates/sparq-py/src/lib.rs", "crates/sparq-lws-core/src/sparql_endpoint.rs",
                        "crates/sparq-introspect/Cargo.toml"]


def completed(stdout=b""):
    return subprocess.CompletedProcess([], 0, stdout)


class ExactEvaluatorSelection(unittest.TestCase):
    def test_workflow_always_creates_the_gate_and_checks_classification(self):
        workflow = (Path(__file__).parents[2] / ".github/workflows/zk-exact-evaluator.yml").read_text()
        self.assertIn("  merge_group:\n    types: [checks_requested]", workflow)
        self.assertIn("name: real exact-dataset guest and proof", workflow)
        self.assertNotIn("    paths:", workflow)
        self.assertIn("Require explicit classification output", workflow)
        self.assertIn("python3 scripts/ci_exact_evaluator_paths.py", workflow)
        self.assertIn("python3 scripts/ci_exact_evaluator_paths.py --scope registry-parser", workflow)
        self.assertIn("python3 scripts/ci_exact_evaluator_evidence.py", workflow)

    def test_every_execution_input_is_relevant(self):
        for path in EXACT_INPUTS:
            self.assertTrue(selector.relevant_path(path), path)
            # [OPUS-5.5] The registry gate reruns whenever the proof job would.
            self.assertTrue(selector.relevant_path(path, "registry-parser"), path)

    def test_registry_scope_adds_gate_files_and_caller_crates_only(self):
        for path in REGISTRY_ONLY_INPUTS:
            self.assertTrue(selector.relevant_path(path, "registry-parser"), path)
            # Caller-crate edits alone must not start the 120-minute proof job.
            self.assertFalse(selector.relevant_path(path), path)
        for path in ["docs/design.md", "crates/sparq-geo/src/lib.rs", "skills/sparql-query/SKILL.md",
                     "crates/sparq-textual/src/lib.rs", "scripts/check_registry_parser.py.orig"]:
            self.assertFalse(selector.relevant_path(path, "registry-parser"), path)

    def test_caller_change_selects_the_registry_gate_but_not_the_proof(self):
        for scope, expected in [("registry-parser", True), ("exact-evaluator", False)]:
            with patch.object(selector.subprocess, "run", side_effect=[
                completed(), completed(b"crates/sparq-server/src/exec.rs\0docs/notes.md\0"),
            ]):
                self.assertEqual(selector.requires_execution("pull_request", BASE, HEAD, scope), expected)

    def test_registry_scope_keeps_fail_closed_selection(self):
        with patch.object(selector.subprocess, "run", side_effect=[completed(), completed(b"docs/design.md")]):
            self.assertTrue(selector.requires_execution("merge_group", BASE, HEAD, "registry-parser"))
        with patch.object(selector.subprocess, "run", side_effect=OSError()):
            self.assertTrue(selector.requires_execution("push", BASE, HEAD, "registry-parser"))
        with patch.object(selector.subprocess, "run") as run:
            self.assertTrue(selector.requires_execution("workflow_dispatch", "", "", "registry-parser"))
            run.assert_not_called()

    def test_unknown_scope_is_an_error_not_a_skip(self):
        # An empty diff never consults relevant_path, so the scope is checked first.
        with patch.object(selector.subprocess, "run", side_effect=[completed(), completed()]):
            with self.assertRaisesRegex(ValueError, "unknown selection scope"):
                selector.requires_execution("merge_group", BASE, HEAD, "registry")
        with self.assertRaisesRegex(ValueError, "unknown selection scope"):
            selector.relevant_path("Cargo.toml", "proof")

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
