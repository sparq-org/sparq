#!/usr/bin/env python3
"""Tests for `scripts/check-zk-toolchain-gate.py` and the gate it protects.

The gate module is the only way a ZK test may obtain `nargo` / `bb`, and a RUNTIME
LEDGER is the proof that the lane reached the gate in every live test. The tests
below pin both ends:

  * the checker's own negative fixtures (`--self-test`);
  * the checker against the REAL tree (green, and not vacuously so);
  * each acquisition and driver-route evasion class planted into a COPY of the real
    tree, which must turn it red;
  * ledger verification: a missing name, an empty ledger, a missing file, an unknown
    entry, a duplicate inventory name;
  * the gate's source (token, ledger append, byte-identical copies) and the lane
    wiring (job-level ledger env, the verify step's shape, the default-run step with
    no `if:`).

Mutations that turn this file red: dropping the ledger append from the gate
(`GateSource`); removing the token parameter from `live_tool` (`GateSource`);
deleting or weakening the verify step (`LaneWiring`); letting the two copies of the
gate drift (`GateSource`); reverting `poseidon2_noir_cross.rs` to its not-found soft
pass (`GateIsTheOnlyPath`, `AcquisitionLint`).
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
VERIFY_STEP = "Verify every live ZK test ran"
LEDGER_ENV = "SPARQ_ZK_LIVE_LEDGER"
LEDGER_PATH = (
    "${{ github.workspace }}/.zk-live-ledger-"
    "${{ github.run_id }}-${{ github.run_attempt }}.txt"
)
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
        'fn g() -> bool { std::process::Command::new("nargo").output().is_ok() }\n'
        "\n#[test]\nfn case() { assert!(g()); }\n"
    ),
    "a const executable": (
        'const T: &str = "nargo";\n#[test]\nfn case() {\n'
        "    let _ = std::process::Command::new(T).output();\n}\n"
    ),
    "a which probe": '#[test]\nfn case() { assert!(which::which("nargo").is_ok()); }\n',
    "an env-gated driver": (
        "#[test]\nfn case() {\n"
        '    let b = std::env::var("NARGO_BIN").unwrap_or_default();\n'
        "    let _ = std::process::Command::new(b).output();\n}\n"
    ),
    "a one-line test fn": (
        '#[test] fn case() { let _ = std::process::Command::new("bb").output(); }\n'
    ),
    "a not-found soft pass": (
        "#[test]\nfn case() {\n"
        "    if matches!(std::io::ErrorKind::NotFound, std::io::ErrorKind::Other) "
        "{ return; }\n}\n"
    ),
    "a path-literal executable": (
        "#[test]\nfn case() {\n"
        '    let _ = std::process::Command::new("/root/.nargo/bin/nargo").output();\n}\n'
    ),
    "a windows-suffixed executable": (
        '#[test]\nfn case() {\n'
        '    let _ = std::process::Command::new("bb.exe").output();\n}\n'
    ),
    "a whitespace-spelled new": (
        "#[test]\nfn case() {\n"
        '    let _ = std::process::Command :: new ("nargo").output();\n}\n'
    ),
    "a shell laundering the tool name": (
        "#[test]\nfn case() {\n"
        '    let _ = std::process::Command::new("sh").arg("-c").arg("nargo x").output();\n'
        "}\n"
    ),
    "a Command alias": (
        "use std::process::Command as Runner;\n#[test]\nfn case() {\n"
        '    let _ = Runner::new("nargo").output();\n}\n'
    ),
    "a cmd! macro": '#[test]\nfn case() { let _ = cmd!("nargo", "execute"); }\n',
    "a test redirecting the ledger": (
        "#[test]\nfn case() {\n"
        '    std::env::set_var("SPARQ_ZK_LIVE_LEDGER", "/dev/null");\n}\n'
    ),
    "an ungated driver route": (
        "use sparq_zk_compose::driver::CircuitProver;\n#[test]\nfn case() {\n"
        "    let p = CircuitProver::from_crate_root();\n    let _ = p;\n}\n"
    ),
    "a branch on a spawn failure": (
        "mod support;\n#[test]\nfn case() {\n"
        '    if !support::require_toolchain("planted_spawn_arm", &["nargo"]) { return; }\n'
        "    match run() {\n        Err(DriverError::Spawn { .. }) => return,\n"
        "        _ => {}\n    }\n}\n"
    ),
    "a discarded live_tool spawn result": (
        "mod support;\n#[test]\nfn case() {\n"
        '    let g = match support::gate("planted_discard", &["nargo"]) {\n'
        "        Some(g) => g,\n        None => return,\n    };\n"
        '    let _ = support::live_tool(&g, "nargo").output().ok();\n}\n'
    ),
    "a non-literal gate name": (
        'mod support;\nconst N: &str = "planted_dyn";\n#[test]\nfn case() {\n'
        '    if !support::require_toolchain(N, &["nargo"]) { return; }\n}\n'
    ),
    "a duplicate inventory name": (
        "mod support;\n#[test]\nfn a() {\n"
        '    if !support::require_toolchain("planted_dup", &["nargo"]) { return; }\n}\n'
        "\n#[test]\nfn b() {\n"
        '    if !support::require_toolchain("planted_dup", &["nargo"]) { return; }\n}\n'
    ),
    "a quoted exemption marker": (
        "#[test]\nfn case() {\n"
        '    let s = "// zk-toolchain-gate: allow not a real comment";\n'
        '    let _ = std::process::Command::new("nargo").output();\n'
        "    let _ = s;\n}\n"
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
        src, dst = REPO / rel, root / rel
        dst.parent.mkdir(parents=True, exist_ok=True)
        shutil.copytree(src, dst) if src.is_dir() else shutil.copy2(src, dst)
    return root


class SelfTest(unittest.TestCase):
    def test_the_negative_fixtures_all_fire(self):
        proc = run_script("--self-test")
        self.assertEqual(
            proc.returncode, 0, f"--self-test failed:\n{proc.stdout}\n{proc.stderr}"
        )

    def test_the_self_test_counts_its_fixtures(self):
        """The count comes from expect_red calls, not a hard-coded total."""
        proc = run_script("--self-test")
        self.assertRegex(proc.stdout, r"all \d\d+ planted fixtures are reported")


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
        self.assertRegex(proc.stdout, r"note: \d\d+ gate call site\(s\) inventoried")

    def test_every_ledger_exclusion_states_a_reason(self):
        proc = run_script("--check")
        for line in proc.stdout.splitlines():
            if line.startswith("note: excluded "):
                self.assertIn(" — ", line, f"exclusion with no reason: {line}")
                self.assertGreater(len(line.split(" — ", 1)[1].strip()), 20, line)


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

    def test_an_annotation_outside_the_recorded_ratchet_is_an_error(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = copy_tree(tmp)
            (root / "crates/sparq-zk-compose/tests/planted_annot.rs").write_text(
                "#[test]\nfn case() {\n"
                "    // zk-toolchain-gate: allow a stated reason\n"
                '    let _ = std::process::Command::new("nargo").output();\n}\n',
                encoding="utf-8",
            )
            proc = run_script("--check", "--root", str(root))
            self.assertEqual(proc.returncode, 1, proc.stdout)
            self.assertIn("[E]", proc.stderr)

    def test_a_missing_scan_directory_is_an_error(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = copy_tree(tmp)
            shutil.rmtree(root / "crates/sparq-zk/tests/support")
            proc = run_script("--check", "--root", str(root))
            self.assertEqual(proc.returncode, 1, proc.stdout)
            self.assertIn("SCAN", proc.stderr)

    def test_a_cargo_test_entry_pointing_nowhere_is_an_error(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = copy_tree(tmp)
            p = root / "crates/sparq-zk-compose/Cargo.toml"
            p.write_text(
                p.read_text(encoding="utf-8")
                + '\n[[test]]\nname = "ghost"\npath = "tests/ghost/main.rs"\n',
                encoding="utf-8",
            )
            proc = run_script("--check", "--root", str(root))
            self.assertEqual(proc.returncode, 1, proc.stdout)
            self.assertIn("SCAN", proc.stderr)


class LedgerVerification(unittest.TestCase):
    """--verify-ledger against the real inventory."""

    def _ledger(self, root: Path, lines: list[str]) -> Path:
        p = root / "ledger.txt"
        p.write_text("".join(f"{line}\n" for line in lines), encoding="utf-8")
        return p

    def test_a_missing_file_is_red(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = copy_tree(tmp)
            proc = run_script(
                "--verify-ledger", str(root / "nope.txt"), "--root", str(root)
            )
            self.assertEqual(proc.returncode, 1)
            self.assertIn("does not exist", proc.stderr)

    def test_an_empty_ledger_is_red(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = copy_tree(tmp)
            p = self._ledger(root, [])
            proc = run_script("--verify-ledger", str(p), "--root", str(root))
            self.assertEqual(proc.returncode, 1)
            self.assertIn("empty", proc.stderr)

    def test_a_partial_ledger_names_the_missing_call_sites(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = copy_tree(tmp)
            p = self._ledger(root, ["sparq-zk/poseidon2_noir_cross::nargo_live_cross_check"])
            proc = run_script("--verify-ledger", str(p), "--root", str(root))
            self.assertEqual(proc.returncode, 1)
            self.assertIn("left no ledger entry", proc.stderr)
            self.assertIn("sparq-zk-compose/e2e::", proc.stderr)
            self.assertNotIn(
                "sparq-zk/poseidon2_noir_cross::nargo_live_cross_check", proc.stderr
            )

    def test_a_complete_ledger_is_green_and_an_unknown_entry_is_only_reported(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = copy_tree(tmp)
            # Build a complete ledger from the checker's own missing-list.
            p = self._ledger(root, [])
            proc = run_script("--verify-ledger", str(p), "--root", str(root))
            keys = [
                line.strip().split("  ")[0]
                for line in proc.stderr.splitlines()
                if line.startswith("  sparq-")
            ]
            self.assertFalse(keys, "an empty ledger short-circuits before the key list")
            # Instead, take the keys from a one-entry run, which does reach the list.
            p = self._ledger(root, ["sparq-zk/plain::nothing"])
            proc = run_script("--verify-ledger", str(p), "--root", str(root))
            keys = [
                line.strip().split("  ")[0]
                for line in proc.stderr.splitlines()
                if line.strip().startswith("sparq-")
            ]
            self.assertGreater(len(keys), 50, proc.stderr)
            p = self._ledger(root, keys + ["sparq-zk/plain::who_is_this"])
            proc = run_script("--verify-ledger", str(p), "--root", str(root))
            self.assertEqual(proc.returncode, 0, f"{proc.stdout}\n{proc.stderr}")
            self.assertIn("no matching call site", proc.stdout)

    def test_a_duplicate_inventory_name_blocks_verification(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = copy_tree(tmp)
            (root / "crates/sparq-zk-compose/tests/planted_dup.rs").write_text(
                EVASIONS["a duplicate inventory name"], encoding="utf-8"
            )
            p = self._ledger(root, ["sparq-zk/plain::nothing"])
            proc = run_script("--verify-ledger", str(p), "--root", str(root))
            self.assertEqual(proc.returncode, 1)
            self.assertIn("does not parse", proc.stderr)


class GateSource(unittest.TestCase):
    def setUp(self):
        self.text = (REPO / HELPERS[0]).read_text(encoding="utf-8")

    def test_the_two_copies_of_the_gate_are_byte_identical(self):
        self.assertEqual(
            (REPO / HELPERS[0]).read_bytes(),
            (REPO / HELPERS[1]).read_bytes(),
            "sparq-zk cannot depend on sparq-zk-compose's test tree, so the gate is "
            "duplicated; the copies must not drift",
        )

    def test_the_gate_mints_a_token_that_live_tool_requires(self):
        self.assertIn("pub struct LiveToolchain", self.text)
        self.assertIn("pub fn gate(test: &str, tools: &[&str]) -> Option<LiveToolchain>",
                      self.text)
        self.assertIn("pub fn live_tool(_gate: &LiveToolchain, tool: &str)", self.text)

    def test_live_tool_hard_fails_on_its_own(self):
        self.assertRegex(
            self.text,
            r"pub fn live_tool\([^)]*\)[^{]*\{\s*assert!\(\s*on_path\(tool\)",
            "live_tool must re-check PATH itself, so an is_err() test on the spawn "
            "cannot become a soft pass",
        )

    def test_the_gate_records_only_on_the_allowed_path(self):
        self.assertIn('pub const LEDGER_ENV: &str = "SPARQ_ZK_LIVE_LEDGER";', self.text)
        self.assertIn("fn record(test: &str)", self.text)
        self.assertIn(".append(true)", self.text, "the append must be O_APPEND")
        self.assertIn("write_all(line.as_bytes())", self.text, "one write per line")
        body = self.text.split("pub fn gate(", 1)[1]
        allowed, rest = body.split("record(test);", 1)
        self.assertIn("missing.is_empty()", allowed, "recording happens on the run path")
        self.assertNotIn("record(", rest.split("pub fn require_toolchain", 1)[0],
                         "a skip or a panic must record nothing")

    def test_the_ledger_key_is_package_binary_and_test(self):
        self.assertIn('env!("CARGO_CRATE_NAME")', self.text)
        self.assertIn('env!("CARGO_PKG_NAME")', self.text)
        self.assertIn('format!("{}/{}::{}"', self.text)


class GateIsTheOnlyPath(unittest.TestCase):
    """The converted source: poseidon2_noir_cross no longer soft-passes."""

    def setUp(self):
        self.text = (REPO / POSEIDON).read_text(encoding="utf-8")

    def test_the_live_case_goes_through_the_gate_and_the_token(self):
        self.assertIn("mod support;", self.text)
        self.assertIn('support::gate("nargo_live_cross_check", &["nargo"])', self.text)
        self.assertIn('support::live_tool(&gate, "nargo")', self.text)

    def test_the_not_found_soft_pass_is_gone(self):
        self.assertNotIn("NotFound", self.text)
        self.assertNotIn("Command::new(", self.text)


class LaneWiring(unittest.TestCase):
    def setUp(self):
        self.text = (REPO / WORKFLOW).read_text(encoding="utf-8")

    def test_the_ledger_path_is_set_at_job_level(self):
        self.assertIn(
            f"      {LEDGER_ENV}: " + "${{ github.workspace }}/.zk-live-ledger-${{ github.run_id }}-${{ github.run_attempt }}.txt", self.text
        )

    def test_the_verify_step_is_a_bare_command_with_a_literal_path(self):
        self.assertRegex(
            self.text,
            r"(?m)^      - name: Verify every live ZK test ran\n"
            r"        run: python3 scripts/check-zk-toolchain-gate\.py --verify-ledger "
            r'"\$\{\{ github\.workspace \}\}/\.zk-live-ledger-'
            r'\$\{\{ github\.run_id \}\}-\$\{\{ github\.run_attempt \}\}\.txt"\s*$',
            "the verify step must be exactly that one command, with the path as a "
            "literal rather than $SPARQ_ZK_LIVE_LEDGER, which a test could rewrite",
        )

    def test_the_verify_step_is_last_and_unconditional(self):
        self.assertTrue(
            self.text.rstrip().endswith('"${{ github.workspace }}/.zk-live-ledger-${{ github.run_id }}-${{ github.run_attempt }}.txt"'),
            "the verify step must be the job's last step",
        )
        tail = self.text[self.text.index("- name: Verify every live ZK test ran") :]
        for key in ("if:", "continue-on-error:", "env:", "working-directory:"):
            self.assertNotIn(key, tail, f"the verify step must not carry {key}")

    def test_the_default_run_step_has_no_if(self):
        start = self.text.index("- name: Run the default-run live ZK tests (real nargo/bb)")
        rest = self.text[start + len("- name: Run the default-run live ZK tests (real nargo/bb)") :]
        block = rest.split("- name:", 1)[0]
        self.assertNotIn("if:", block)
        self.assertIn("cargo test -p sparq-zk-compose -p sparq-zk -- --test-threads=1",
                      block)

    def test_pyyaml_is_installed_with_python3_dash_m_pip(self):
        self.assertIn(
            "python3 -m pip install --require-hashes -r "
            ".github/requirements/docs-quality.txt",
            self.text,
        )

    def test_deleting_the_verify_step_turns_the_check_red(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = copy_tree(tmp)
            wf = root / WORKFLOW
            wf.write_text(
                wf.read_text(encoding="utf-8").replace(
                    f"- name: {VERIFY_STEP}", "- name: Something else", 1
                ),
                encoding="utf-8",
            )
            proc = run_script("--check", "--root", str(root))
            self.assertEqual(proc.returncode, 1, proc.stdout)
            self.assertIn("exactly one step named", proc.stderr)

    def test_each_weakening_of_the_verify_step_turns_the_check_red(self):
        run_line = (
            "        run: python3 scripts/check-zk-toolchain-gate.py --verify-ledger "
            '"${{ github.workspace }}/.zk-live-ledger-${{ github.run_id }}-${{ github.run_attempt }}.txt"'
        )
        for label, old, new in (
            ("an if:", run_line, "        if: always()\n" + run_line),
            ("continue-on-error", run_line, "        continue-on-error: true\n" + run_line),
            ("an env", run_line, "        env:\n          X: '1'\n" + run_line),
            ("a working-directory", run_line, "        working-directory: scripts\n" + run_line),
            (
                "the path from the environment",
                '--verify-ledger "${{ github.workspace }}/.zk-live-ledger-${{ github.run_id }}-${{ github.run_attempt }}.txt"',
                '--verify-ledger "$SPARQ_ZK_LIVE_LEDGER"',
            ),
            (
                "the job-level ledger env",
                "      SPARQ_ZK_LIVE_LEDGER: ${{ github.workspace }}/.zk-live-ledger-${{ github.run_id }}-${{ github.run_attempt }}.txt\n",
                "",
            ),
            (
                "the opt-out set in the lane",
                "    env:\n",
                "    env:\n      SPARQ_ZK_ALLOW_MISSING_TOOLCHAIN: '1'\n",
            ),
        ):
            with self.subTest(label), tempfile.TemporaryDirectory() as tmp:
                root = copy_tree(tmp)
                wf = root / WORKFLOW
                text = wf.read_text(encoding="utf-8")
                self.assertIn(old, text, f"{label}: anchor moved")
                wf.write_text(text.replace(old, new, 1), encoding="utf-8")
                proc = run_script("--check", "--root", str(root))
                self.assertEqual(
                    proc.returncode,
                    1,
                    f"{label} must be reported:\n{proc.stdout}\n{proc.stderr}",
                )
                self.assertIn("[D]", proc.stderr)


if __name__ == "__main__":
    unittest.main(verbosity=2)
