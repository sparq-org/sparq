#!/usr/bin/env python3
"""Tests for `scripts/check-zk-toolchain-gate.py` and the gate it protects.

The checker makes two properties hold by construction: `tests/support/mod.rs` is the
only way a ZK test obtains `nargo` / `bb`, and the `zk-toolchain` lane runs every
package whose tests use that gate. The tests below pin both ends:

  * the checker's own negative fixtures (`--self-test`);
  * the checker against the REAL tree (it must be green, and it must not be green
    vacuously);
  * each evasion class planted into a COPY of the real tree, which must turn it red;
  * the lane wiring and the converted `poseidon2_noir_cross.rs` source.

Mutations that turn this file red: reverting `poseidon2_noir_cross.rs` to its
not-found soft pass (`GateIsTheOnlyPath`, `AcquisitionLint`); dropping `-p sparq-zk`
from the package-wide step, or renaming that step (`LaneCoverage`); letting the two
copies of the gate drift (`HelperModules`).
"""
from __future__ import annotations

import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from _yaml_seam import yaml_or_local_skip  # noqa: E402

yaml_or_local_skip()

REPO = Path(__file__).resolve().parents[2]
SCRIPT = REPO / "scripts/check-zk-toolchain-gate.py"
WORKFLOW = "/".join([".github", "workflows", "zk-toolchain.yml"])
COVERAGE_STEP = "Run the default-run live ZK tests (real nargo/bb)"
POSEIDON = "crates/sparq-zk/tests/poseidon2_noir_cross.rs"
HELPERS = (
    "crates/sparq-zk/tests/support/mod.rs",
    "crates/sparq-zk-compose/tests/support/mod.rs",
)
COPIED = (
    "crates/sparq-zk/tests",
    "crates/sparq-zk/Cargo.toml",
    "crates/sparq-zk-compose/tests",
    "crates/sparq-zk-compose/Cargo.toml",
)

# One planted file per evasion class. Each must turn the real-tree check red.
EVASIONS: dict[str, str] = {
    "a helper wrapper outside the gate module": (
        "fn my_gate() -> bool {\n"
        '    std::process::Command::new("nargo").output().is_ok()\n'
        "}\n\n#[test]\nfn case() {\n    assert!(my_gate());\n}\n"
    ),
    "a const executable": (
        'const TOOL: &str = "nargo";\n\n#[test]\nfn case() {\n'
        "    let _ = std::process::Command::new(TOOL).output();\n}\n"
    ),
    "a which probe": (
        '#[test]\nfn case() {\n    assert!(which::which("nargo").is_ok());\n}\n'
    ),
    "an env-gated driver": (
        "#[test]\nfn case() {\n"
        '    let bin = std::env::var("NARGO_BIN").unwrap_or_default();\n'
        "    let _ = std::process::Command::new(bin).output();\n}\n"
    ),
    "a one-line test fn": (
        '#[test] fn case() { let _ = std::process::Command::new("bb").output(); }\n'
    ),
    "a not-found soft pass": (
        "mod support;\n\n#[test]\nfn case() {\n"
        '    let r = support::live_tool("nargo").output();\n'
        "    if matches!(&r, Err(e) if e.kind() == std::io::ErrorKind::NotFound) {\n"
        "        return;\n    }\n    assert!(r.is_ok());\n}\n"
    ),
}


def run_script(*args: str) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        [sys.executable, str(SCRIPT), *args], capture_output=True, text=True, check=False
    )


def copy_tree(tmp: str) -> Path:
    root = Path(tmp)
    (root / WORKFLOW).parent.mkdir(parents=True, exist_ok=True)
    shutil.copy2(REPO / WORKFLOW, root / WORKFLOW)
    for rel in COPIED:
        src = REPO / rel
        dst = root / rel
        dst.parent.mkdir(parents=True, exist_ok=True)
        if src.is_dir():
            shutil.copytree(src, dst)
        else:
            shutil.copy2(src, dst)
    return root


class SelfTest(unittest.TestCase):
    def test_the_negative_fixtures_all_fire(self):
        proc = run_script("--self-test")
        self.assertEqual(
            proc.returncode, 0, f"--self-test failed:\n{proc.stdout}\n{proc.stderr}"
        )


class RealTree(unittest.TestCase):
    def test_the_real_tree_is_clean(self):
        proc = run_script("--check")
        self.assertEqual(
            proc.returncode,
            0,
            f"the checker must not ship failing:\n{proc.stdout}\n{proc.stderr}",
        )

    def test_the_inventory_is_not_empty(self):
        """A grep that matched nothing would make every check vacuously green."""
        proc = run_script("--check")
        self.assertIn("sparq-zk::poseidon2_noir_cross", proc.stdout)
        self.assertIn("sparq-zk-compose::e2e", proc.stdout)


class AcquisitionLint(unittest.TestCase):
    def test_each_evasion_class_turns_the_check_red(self):
        for label, body in EVASIONS.items():
            with self.subTest(label), tempfile.TemporaryDirectory() as tmp:
                root = copy_tree(tmp)
                (root / "crates/sparq-zk-compose/tests/planted_evasion.rs").write_text(
                    body, encoding="utf-8"
                )
                proc = run_script("--check", "--root", str(root))
                self.assertEqual(
                    proc.returncode,
                    1,
                    f"{label} must be reported:\n{proc.stdout}\n{proc.stderr}",
                )
                self.assertIn("planted_evasion.rs", proc.stderr)

    def test_a_declared_exemption_clears_it_and_a_bare_one_does_not(self):
        body = (
            'const TOOL: &str = "nargo";\n\n#[test]\nfn case() {\n'
            "    %s\n    let _ = std::process::Command::new(TOOL).output();\n}\n"
        )
        for comment, expected in (
            ("// zk-toolchain-gate: allow a stated reason", 0),
            ("// zk-toolchain-gate: allow", 1),
        ):
            with self.subTest(comment), tempfile.TemporaryDirectory() as tmp:
                root = copy_tree(tmp)
                (root / "crates/sparq-zk-compose/tests/planted_exempt.rs").write_text(
                    body % comment, encoding="utf-8"
                )
                proc = run_script("--check", "--root", str(root))
                self.assertEqual(
                    proc.returncode,
                    expected,
                    f"{comment!r}:\n{proc.stdout}\n{proc.stderr}",
                )

    def test_a_missing_scan_directory_is_an_error(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = copy_tree(tmp)
            shutil.rmtree(root / "crates/sparq-zk/tests/support")
            proc = run_script("--check", "--root", str(root))
            self.assertEqual(proc.returncode, 1, proc.stdout)
            self.assertIn("SCAN", proc.stderr)


class HelperModules(unittest.TestCase):
    def test_the_two_copies_of_the_gate_are_byte_identical(self):
        a, b = (REPO / HELPERS[0]).read_bytes(), (REPO / HELPERS[1]).read_bytes()
        self.assertEqual(
            a,
            b,
            "sparq-zk cannot depend on sparq-zk-compose's test tree, so the gate is "
            "duplicated; the copies must not drift",
        )

    def test_drift_turns_the_check_red(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = copy_tree(tmp)
            p = root / HELPERS[0]
            p.write_text(p.read_text(encoding="utf-8") + "// drift\n", encoding="utf-8")
            proc = run_script("--check", "--root", str(root))
            self.assertEqual(proc.returncode, 1, proc.stdout)
            self.assertIn("[B]", proc.stderr)

    def test_the_gate_exposes_the_hard_failure_and_the_declared_opt_out(self):
        text = (REPO / HELPERS[0]).read_text(encoding="utf-8")
        self.assertIn("pub fn require_toolchain", text)
        self.assertIn("pub fn live_tool", text)
        self.assertIn("panic!(", text, "a missing, undeclared tool must fail hard")
        self.assertIn("SPARQ_ZK_ALLOW_MISSING_TOOLCHAIN", text)


class GateIsTheOnlyPath(unittest.TestCase):
    """The converted source: poseidon2_noir_cross no longer soft-passes."""

    def setUp(self):
        self.text = (REPO / POSEIDON).read_text(encoding="utf-8")

    def test_the_live_case_goes_through_the_gate(self):
        self.assertIn("mod support;", self.text)
        self.assertIn(
            'support::require_toolchain("nargo_live_cross_check", &["nargo"])',
            self.text,
        )
        self.assertIn('support::live_tool("nargo")', self.text)

    def test_the_not_found_soft_pass_is_gone(self):
        self.assertNotIn("ErrorKind::NotFound", self.text)
        self.assertNotIn("Command::new(", self.text)


class LaneCoverage(unittest.TestCase):
    def setUp(self):
        self.text = (REPO / WORKFLOW).read_text(encoding="utf-8")

    def test_the_checker_is_invoked_as_a_bare_command(self):
        self.assertRegex(
            self.text,
            r"(?m)^\s*run: python3 scripts/check-zk-toolchain-gate\.py --check\s*$",
            "appending `|| true` or piping would leave the step green while the check "
            "can no longer fail",
        )
        self.assertNotRegex(
            self.text,
            r"check-zk-toolchain-gate\.py[^\n]*(\|\||; *true|continue-on-error)",
            "the check must not swallow its exit code",
        )

    def test_pyyaml_is_installed_with_python3_dash_m_pip(self):
        self.assertIn(
            "python3 -m pip install --require-hashes -r "
            ".github/requirements/docs-quality.txt",
            self.text,
        )

    def test_the_named_step_runs_both_packages_package_wide(self):
        self.assertIn(f"- name: {COVERAGE_STEP}", self.text)
        self.assertIn(
            "cargo test -p sparq-zk-compose -p sparq-zk -- --test-threads=1", self.text
        )

    def test_dropping_a_package_from_the_named_step_turns_the_check_red(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = copy_tree(tmp)
            wf = root / WORKFLOW
            wf.write_text(
                wf.read_text(encoding="utf-8").replace(
                    "cargo test -p sparq-zk-compose -p sparq-zk -- --test-threads=1",
                    "cargo test -p sparq-zk-compose -- --test-threads=1",
                    1,
                ),
                encoding="utf-8",
            )
            proc = run_script("--check", "--root", str(root))
            self.assertEqual(proc.returncode, 1, proc.stdout)
            self.assertIn("-p sparq-zk", proc.stderr)

    def test_renaming_the_named_step_turns_the_check_red(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = copy_tree(tmp)
            wf = root / WORKFLOW
            wf.write_text(
                wf.read_text(encoding="utf-8").replace(
                    f"- name: {COVERAGE_STEP}", "- name: Run some live ZK tests", 1
                ),
                encoding="utf-8",
            )
            proc = run_script("--check", "--root", str(root))
            self.assertEqual(proc.returncode, 1, proc.stdout)
            self.assertIn("exactly one step named", proc.stderr)

    def test_each_weakening_of_the_named_step_turns_the_check_red(self):
        plain = "cargo test -p sparq-zk-compose -p sparq-zk -- --test-threads=1"
        for label, replacement in (
            ("--no-run", "cargo test --no-run -p sparq-zk-compose -p sparq-zk -- --test-threads=1"),
            ("a test-name filter", f"{plain.replace('-- --test-threads=1', '-- e2e --test-threads=1')}"),
            ("-- --ignored", "cargo test -p sparq-zk-compose -p sparq-zk -- --ignored --test-threads=1"),
            ("|| true", f"{plain} || true"),
            ("a target selector", "cargo test -p sparq-zk-compose -p sparq-zk --test e2e -- --test-threads=1"),
        ):
            with self.subTest(label), tempfile.TemporaryDirectory() as tmp:
                root = copy_tree(tmp)
                wf = root / WORKFLOW
                text = wf.read_text(encoding="utf-8")
                self.assertIn(plain, text)
                wf.write_text(text.replace(plain, replacement, 1), encoding="utf-8")
                proc = run_script("--check", "--root", str(root))
                self.assertEqual(
                    proc.returncode,
                    1,
                    f"{label} must be reported:\n{proc.stdout}\n{proc.stderr}",
                )

    def test_a_skippable_if_on_the_named_step_turns_the_check_red(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = copy_tree(tmp)
            wf = root / WORKFLOW
            wf.write_text(
                wf.read_text(encoding="utf-8").replace(
                    f"- name: {COVERAGE_STEP}\n        if: steps.changes.outputs.zk == 'true'",
                    f"- name: {COVERAGE_STEP}\n        if: github.event_name == 'schedule'",
                    1,
                ),
                encoding="utf-8",
            )
            proc = run_script("--check", "--root", str(root))
            self.assertEqual(proc.returncode, 1, proc.stdout)
            self.assertIn("provably always true", proc.stderr)

    def test_an_ignored_live_test_with_no_sweep_turns_the_check_red(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = copy_tree(tmp)
            wf = root / WORKFLOW
            wf.write_text(
                wf.read_text(encoding="utf-8").replace(
                    "- name: Run the ignored ZK forge + anchor + e2e suite (real bb)",
                    "- name: Run something else",
                    1,
                ),
                encoding="utf-8",
            )
            proc = run_script("--check", "--root", str(root))
            self.assertEqual(proc.returncode, 1, proc.stdout)
            self.assertIn("[E]", proc.stderr)

    def test_a_feature_gated_live_test_without_its_feature_turns_the_check_red(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = copy_tree(tmp)
            wf = root / WORKFLOW
            text = wf.read_text(encoding="utf-8")
            needle = "--features sparq-zk-compose/dual-leaf \\\n"
            self.assertIn(needle, text, "the dual-leaf step moved")
            wf.write_text(text.replace(needle, "", 1), encoding="utf-8")
            proc = run_script("--check", "--root", str(root))
            self.assertEqual(proc.returncode, 1, proc.stdout)
            self.assertIn("[D]", proc.stderr)


if __name__ == "__main__":
    unittest.main(verbosity=2)
