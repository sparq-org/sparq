#!/usr/bin/env python3
"""Tests for `scripts/check-zk-toolchain-gate-coverage.py`.

The gate under test derives the set of TOOLCHAIN-BOUND test binaries in
`crates/sparq-zk*/tests/` from the test sources and fails when
`.github/workflows/zk-toolchain.yml` does not name one in a `cargo test --test`
invocation. The two things worth pinning are therefore:

  * against the REAL tree the gate is clean (so it is not shipped failing), and
  * planting an unlisted toolchain-bound test into a COPY of the real tree makes it
    fail and names that binary (so it is not vacuous).

The second case is the mutation: delete the coverage comparison in `check()` and
`PlantedIntoTheRealTree` goes red.
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
SCRIPT = REPO / "scripts/check-zk-toolchain-gate-coverage.py"
WORKFLOW = "/".join([".github", "workflows", "zk-toolchain.yml"])
COPIED = [
    "crates/sparq-zk/tests",
    "crates/sparq-zk-compose/tests",
]

# A planted binary with ONE non-ignored live test behind the declared gate. Nothing
# in the workflow names it, so the gate must report it.
PLANTED_DECLARED_GATE = """\
mod support;

#[test]
fn a_planted_live_case_nobody_runs() {
    if !support::require_toolchain("a_planted_live_case_nobody_runs", &["nargo", "bb"]) {
        return;
    }
    assert!(true);
}
"""

# The same body, but `#[ignore]`d — the lane's `-- --ignored` sweep runs it, so the
# gate must NOT report it.
PLANTED_IGNORED = """\
mod support;

#[test]
#[ignore = "needs the live toolchain"]
fn a_planted_ignored_live_case() {
    assert!(support::require_toolchain("a_planted_ignored_live_case", &["nargo", "bb"]));
}
"""

# A private copy of the gate: spawns nargo itself and returns quietly when it is
# absent. The declared-gate marker alone would miss it.
PLANTED_PRIVATE_GATE = """\
#[test]
fn a_planted_private_gate_case() {
    let out = match std::process::Command::new("nargo").arg("--version").output() {
        Ok(o) => o,
        Err(_) => {
            eprintln!("nargo not on PATH; skipping");
            return;
        }
    };
    assert!(out.status.success());
}
"""


def run_script(*args: str) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        [sys.executable, str(SCRIPT), *args],
        capture_output=True,
        text=True,
        check=False,
    )


class SelfTest(unittest.TestCase):
    """The script's built-in synthetic fixtures."""

    def test_self_test_mode_passes(self):
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
            "the gate must not be shipped failing against the real tree:\n"
            f"{proc.stdout}\n{proc.stderr}",
        )

    def test_the_enumeration_is_not_empty(self):
        """A marker regex that matched nothing would make the gate vacuously green."""
        proc = run_script("--list")
        self.assertEqual(proc.returncode, 0, proc.stderr)
        # The forge/e2e estate: there are live binaries, and they are counted.
        self.assertIn("sparq-zk-compose::e2e", proc.stdout)
        self.assertNotIn("counted by: (none)", proc.stdout)


class PlantedIntoTheRealTree(unittest.TestCase):
    """Copy the real sources, plant a test, assert the gate's verdict flips."""

    def _root_from(self, tmp: str) -> Path:
        root = Path(tmp)
        (root / WORKFLOW).parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(REPO / WORKFLOW, root / WORKFLOW)
        for rel in COPIED:
            shutil.copytree(REPO / rel, root / rel)
        return root

    def test_the_copy_without_a_planted_test_is_clean(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = self._root_from(tmp)
            proc = run_script("--check", "--root", str(root))
            self.assertEqual(proc.returncode, 0, f"{proc.stdout}\n{proc.stderr}")

    def test_a_planted_unlisted_declared_gate_test_fails_the_check(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = self._root_from(tmp)
            name = "planted_unlisted_live"
            (root / "crates/sparq-zk-compose/tests" / f"{name}.rs").write_text(
                PLANTED_DECLARED_GATE, encoding="utf-8"
            )
            proc = run_script("--check", "--root", str(root))
            self.assertEqual(
                proc.returncode,
                1,
                "an unlisted toolchain-bound binary must fail the check:\n"
                f"{proc.stdout}\n{proc.stderr}",
            )
            self.assertIn(name, proc.stderr)
            self.assertIn("a_planted_live_case_nobody_runs", proc.stderr)

    def test_a_planted_private_gate_test_fails_the_check(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = self._root_from(tmp)
            name = "planted_private_gate"
            (root / "crates/sparq-zk-compose/tests" / f"{name}.rs").write_text(
                PLANTED_PRIVATE_GATE, encoding="utf-8"
            )
            proc = run_script("--check", "--root", str(root))
            self.assertEqual(
                proc.returncode,
                1,
                "a private copy of the gate must not bypass the enumeration:\n"
                f"{proc.stdout}\n{proc.stderr}",
            )
            self.assertIn(name, proc.stderr)

    def test_a_planted_ignored_live_test_is_not_reported(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = self._root_from(tmp)
            name = "planted_ignored_live"
            (root / "crates/sparq-zk-compose/tests" / f"{name}.rs").write_text(
                PLANTED_IGNORED, encoding="utf-8"
            )
            proc = run_script("--check", "--root", str(root))
            self.assertEqual(
                proc.returncode,
                0,
                "the `-- --ignored` sweep already runs an ignored live test:\n"
                f"{proc.stdout}\n{proc.stderr}",
            )

    def test_listing_the_planted_binary_clears_the_finding(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = self._root_from(tmp)
            name = "planted_unlisted_live"
            (root / "crates/sparq-zk-compose/tests" / f"{name}.rs").write_text(
                PLANTED_DECLARED_GATE, encoding="utf-8"
            )
            wf = root / WORKFLOW
            text = wf.read_text(encoding="utf-8")
            needle = "            --test capture_car_hire \\\n"
            self.assertIn(needle, text, "the remaining-live-suites step moved")
            wf.write_text(
                text.replace(needle, needle + f"            --test {name} \\\n"),
                encoding="utf-8",
            )
            proc = run_script("--check", "--root", str(root))
            self.assertEqual(
                proc.returncode,
                0,
                "naming the binary in the workflow must clear the finding:\n"
                f"{proc.stdout}\n{proc.stderr}",
            )


class WorkflowWiring(unittest.TestCase):
    """The gate is only a gate while the lane calls it, with its exit code intact."""

    def setUp(self):
        self.text = (REPO / WORKFLOW).read_text(encoding="utf-8")

    def test_the_lane_invokes_the_checker_as_a_bare_command(self):
        import re

        self.assertRegex(
            self.text,
            r"(?m)^\s*run: python3 scripts/check-zk-toolchain-gate-coverage\.py --check\s*$",
            "the enumeration must be a bare command — appending `|| true` or piping "
            "would leave the step green while the gate can no longer fail",
        )
        self.assertNotRegex(
            self.text,
            r"check-zk-toolchain-gate-coverage\.py[^\n]*(\|\||; *true|continue-on-error)",
            "the enumeration step must not swallow its exit code",
        )

    def test_the_previously_uncounted_binaries_are_named(self):
        for name in (
            "capture_car_hire",
            "differential_fuzz",
            "e2e",
            "family_members_pzet",
            "filter_signed_binding",
            "poseidon2_noir_cross",
        ):
            self.assertIn(
                f"--test {name}",
                self.text,
                f"{name} holds non-ignored live tests and must be run by this lane",
            )


if __name__ == "__main__":
    unittest.main(verbosity=2)
