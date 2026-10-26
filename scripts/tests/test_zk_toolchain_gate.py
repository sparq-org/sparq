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

Which test reds when a checker function is deleted or inverted (the functions are
exercised through the CLI, so this maps them to their covering test):

  * `lint_file`        -> AcquisitionLint.test_each_evasion_class_turns_the_check_red
  * `strip_noncode`    -> SourceReader.test_blanking_literals_never_moves_a_line
  * `fn_spans`         -> SourceReader.test_every_test_function_is_found
  * `_reaching`        -> DriverRouteSuites
                          .test_an_unnamed_error_arm_in_it_turns_the_check_red (the
                          prover is built in a local helper, so only the fixpoint
                          reaches it)
  * `_src_scope`       -> SrcTestRegions.test_a_planted_spawn_in_a_src_test_is_red and
                          .test_the_same_spawn_in_production_code_is_not
  * `check_src_gate_decl` -> SrcTestRegions
                          .test_dropping_the_shared_gate_declaration_is_red
  * `exempt`           -> AcquisitionLint
                          .test_an_annotation_does_not_reach_past_its_own_line
  * `check_helpers`    -> GateSource.test_the_two_copies_of_the_gate_are_byte_identical
                          plus the helper-drift fixture in `--self-test`
  * `check_ratchet`    -> AcquisitionLint
                          .test_an_annotation_outside_the_recorded_ratchet_is_an_error
  * `check_workflow`   -> LaneWiring.test_deleting_the_verify_step_turns_the_check_red
                          and .test_each_weakening_of_the_verify_step_turns_the_check_red
  * `inventory`        -> LedgerVerification
                          .test_a_duplicate_inventory_name_blocks_verification
  * `collect`          -> AcquisitionLint.test_a_missing_scan_directory_is_an_error and
                          .test_a_cargo_test_entry_pointing_nowhere_is_an_error
  * `verify_ledger`    -> every test in LedgerVerification
  * `report_check`     -> RealTree.test_the_real_tree_is_clean (exit code) and
                          .test_every_ledger_exclusion_states_a_reason (its output)
  * `static_findings`  -> all of the above; it is the aggregator they share

Mutations of the gate that turn this file red, each verified by running it:
dropping `#[track_caller]` from `gate` (the gate module's
own `gate_identity_tests`, which run in every binary that declares the module);
recording on the skip path (same); deleting CHECK A8, CHECK F, the `src` scan trees,
the annotation's line scope, or the call-site key (`--self-test`).

Mutations that turn this file red: dropping the ledger append from the gate
(`GateSource`); removing the token parameter from `live_tool` (`GateSource`);
deleting or weakening the verify step (`LaneWiring`); letting the two copies of the
gate drift (`GateSource`); reverting `poseidon2_noir_cross.rs` to its not-found soft
pass (`GateIsTheOnlyPath`, `AcquisitionLint`).
"""
from __future__ import annotations

import re
import os
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
import importlib.util

sys.path.insert(0, str(Path(__file__).resolve().parent))
from _yaml_seam import yaml_or_local_skip  # noqa: E402

yaml_or_local_skip()

REPO = Path(__file__).resolve().parents[2]
SCRIPT = REPO / "scripts/check-zk-toolchain-gate.py"
_spec = importlib.util.spec_from_file_location("zk_toolchain_gate", SCRIPT)
checker = importlib.util.module_from_spec(_spec)
sys.modules[_spec.name] = checker
_spec.loader.exec_module(checker)
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
# The `src` trees are 2.6 MB of Rust and the checker reads every line of them, so a
# test that is about `tests/` copies only each crate root (enough for the src scan
# dirs to exist and for CHECK F to read its declaration) and passes full_src=True
# only where a src region is the subject.
SRC_TREES = ("crates/sparq-zk/src", "crates/sparq-zk-compose/src")
LIB_ROOTS = ("crates/sparq-zk/src/lib.rs", "crates/sparq-zk-compose/src/lib.rs")
# Gate call sites in `src` test regions, which the lib test target runs.
SRC_GATED = (
    "crates/sparq-zk-compose/src/driver.rs",
    "crates/sparq-zk-compose/src/result.rs",
    "crates/sparq-zk-compose/src/result/signed.rs",
)
COMPOSE_LIB = "crates/sparq-zk-compose/src/lib.rs"

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
    "an ungated driver route that accepts an unnamed error": (
        "use sparq_zk_compose::driver::CircuitProver;\n#[test]\nfn case() {\n"
        "    let p = CircuitProver::from_crate_root();\n"
        "    match verify(&p) {\n        Err(_) => {}\n"
        '        Ok(()) => panic!("must be rejected"),\n    }\n}\n'
    ),
    "an ungated driver route that discards the error": (
        "use sparq_zk_compose::driver::CircuitProver;\n#[test]\nfn case() {\n"
        "    let p = CircuitProver::from_crate_root();\n"
        "    assert!(verify(&p).is_err());\n}\n"
    ),
    "an ungated driver route with an accepting wildcard arm": (
        "use sparq_zk_compose::driver::CircuitProver;\n#[test]\nfn case() {\n"
        "    let p = CircuitProver::from_crate_root();\n"
        "    match verify(&p) {\n        Ok(()) => panic!(\"no\"),\n"
        "        _ => {}\n    }\n}\n"
    ),
    "a branch on a spawn failure": (
        "mod support;\n#[test]\nfn case() {\n"
        '    let Some(gate) = support::gate("planted_spawn_arm", &["nargo"]) else { return; };\n'
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
        '    let Some(gate) = support::gate(N, &["nargo"]) else { return; };\n}\n'
    ),
    "two gate calls on one line": (
        "mod support;\n#[test]\nfn a() {\n"
        '    let _ = support::gate("x", &["nargo"]); '
        'let _ = support::gate("y", &["nargo"]);\n}\n'
    ),
    "the gate in a brace import": (
        "mod support;\nuse support::{gate, live_tool};\n#[test]\nfn case() {\n"
        '    let g = gate("case", &["nargo"]).unwrap();\n'
        '    let _ = live_tool(&g, "nargo").output().expect("runs");\n}\n'
    ),
    "the gate in a glob import": (
        "mod support;\nuse support::*;\n#[test]\nfn case() {\n"
        '    let _ = gate("case", &["nargo"]);\n}\n'
    ),
    "the gate module aliased": (
        "mod support;\nuse support as tc;\n#[test]\nfn case() {\n"
        '    let _ = tc::gate("case", &["nargo"]);\n}\n'
    ),
    "the gate renamed in a use": (
        "mod support;\nuse support::gate as g;\n#[test]\nfn case() {\n"
        '    let _ = g("case", &["nargo"]);\n}\n'
    ),
    "the gate taken as a function value": (
        "mod support;\n#[test]\nfn case() {\n    let f = support::gate;\n"
        '    let _ = f("case", &["nargo"]);\n}\n'
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


def copy_tree(tmp: str, full_src: bool = False) -> Path:
    root = Path(tmp)
    (root / WORKFLOW).parent.mkdir(parents=True, exist_ok=True)
    shutil.copy2(REPO / WORKFLOW, root / WORKFLOW)
    for rel in COPIED:
        src, dst = REPO / rel, root / rel
        dst.parent.mkdir(parents=True, exist_ok=True)
        shutil.copytree(src, dst) if src.is_dir() else shutil.copy2(src, dst)
    # The declared inventory includes src tests, so a copied tree must include
    # those definitions even when a fixture only mutates an integration test.
    for rel in SRC_TREES:
        shutil.copytree(REPO / rel, root / rel)
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

    def test_an_annotation_does_not_reach_past_its_own_line(self):
        """One code line between the annotation and the finding ends its reach."""
        with tempfile.TemporaryDirectory() as tmp:
            root = copy_tree(tmp)
            (root / "crates/sparq-zk-compose/tests/planted_far.rs").write_text(
                "#[test]\nfn case() {\n"
                "    // zk-toolchain-gate: allow a stated reason\n"
                "    let n = 1;\n"
                '    let _ = std::process::Command::new("nargo")\n'
                "        .arg(n.to_string())\n        .output();\n}\n",
                encoding="utf-8",
            )
            proc = run_script("--check", "--root", str(root))
            self.assertEqual(proc.returncode, 1, proc.stdout)
            self.assertIn("planted_far.rs", proc.stderr)

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

    @staticmethod
    def a_real_key(root: Path) -> str:
        """A key that IS a real call site, read from the checker's own output.

        An unknown entry is fatal now, so a ledger seeded with an invented key
        short-circuits before the missing-list is reached. The excluded sites are
        real call sites and are not in the expected set, which makes one of them the
        seed that leaves the rest of the inventory missing.
        """
        proc = run_script("--check", "--root", str(root))
        for line in proc.stdout.splitlines():
            if line.startswith("note: excluded "):
                return line[len("note: excluded ") :].split(" — ")[0].strip()
        raise AssertionError(f"no excluded key in:\n{proc.stdout}")

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
            root = copy_tree(tmp, full_src=True)
            first = self.a_real_key(root)
            p = self._ledger(root, [f"enter {first}"])
            proc = run_script("--verify-ledger", str(p), "--root", str(root))
            self.assertEqual(proc.returncode, 1)
            self.assertIn("left no ledger entry", proc.stderr)
            self.assertIn("crates/sparq-zk-compose/tests/e2e.rs:", proc.stderr)
            self.assertIn("crates/sparq-zk-compose/src/result.rs:", proc.stderr)
            self.assertNotIn(first + " ", proc.stderr)

    def test_a_complete_ledger_is_green(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = copy_tree(tmp, full_src=True)
            # Build a complete ledger from the checker's own missing-list.
            p = self._ledger(root, [])
            proc = run_script("--verify-ledger", str(p), "--root", str(root))
            keys = [
                line.strip().split("  ")[0]
                for line in proc.stderr.splitlines()
                if line.startswith("  crates/")
            ]
            self.assertFalse(keys, "an empty ledger short-circuits before the key list")
            # Instead, take the keys from a one-entry run, which does reach the list.
            p = self._ledger(root, [f"enter {self.a_real_key(root)}"])
            proc = run_script("--verify-ledger", str(p), "--root", str(root))
            keys = [
                line.strip().split("  ")[0]
                for line in proc.stderr.splitlines()
                if line.strip().startswith("crates/")
            ]
            self.assertGreater(len(keys), 50, proc.stderr)
            p = self._ledger(root, [f"{kind} {key}" for key in keys
                                    for kind in ("enter", "complete")])
            proc = run_script("--verify-ledger", str(p), "--root", str(root))
            self.assertEqual(proc.returncode, 0, f"{proc.stdout}\n{proc.stderr}")

    def test_an_entry_matching_no_call_site_is_red(self):
        """An unpredicted key means the inventory missed a site — an import, an
        alias, or a file outside the scan trees."""
        with tempfile.TemporaryDirectory() as tmp:
            root = copy_tree(tmp, full_src=True)
            p = self._ledger(root, ["enter crates/sparq-zk/tests/plain.rs:999"])
            proc = run_script("--verify-ledger", str(p), "--root", str(root))
            self.assertEqual(proc.returncode, 1)
            self.assertIn("match no inventoried gate call site", proc.stderr)
            self.assertIn("crates/sparq-zk/tests/plain.rs:999", proc.stderr)

    def test_two_gate_calls_on_one_line_block_verification(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = copy_tree(tmp)
            (root / "crates/sparq-zk-compose/tests/planted_dup.rs").write_text(
                EVASIONS["two gate calls on one line"], encoding="utf-8"
            )
            p = self._ledger(root, ["enter crates/sparq-zk/tests/plain.rs:1"])
            proc = run_script("--verify-ledger", str(p), "--root", str(root))
            self.assertEqual(proc.returncode, 1)
            self.assertIn("does not parse", proc.stderr)


class CompletionAndInventory(unittest.TestCase):
    def setUp(self):
        from unittest.mock import patch
        self.exclusions = patch.object(checker, "LEDGER_EXCLUSIONS", {})
        self.ratchet = patch.object(checker, "ALLOW_RATCHET", {})
        self.exclusions.start()
        self.ratchet.start()
        self.addCleanup(self.exclusions.stop)
        self.addCleanup(self.ratchet.stop)

    def fixture(self):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        root = Path(tmp.name)
        checker._write_fixture(root)
        return root

    def findings(self, root):
        return checker.static_findings(root)[0]

    def test_status_assertion_must_reference_the_spawned_value(self):
        self.assertTrue(checker._asserts_status("assert!(out.status.success());", "out"))
        self.assertFalse(checker._asserts_status("assert!(other.status.success());", "out"))
        self.assertFalse(checker._asserts_status("out.unwrap();", "out"))
        self.assertTrue(checker._asserts_status(
            "assert!(String::from_utf8_lossy(&out.stderr).contains(EXPECTED));", "out"))
        self.assertFalse(checker._asserts_status(
            "assert!(String::from_utf8_lossy(&other.stderr).contains(EXPECTED));", "out"))

    def test_absent_diagnostic_is_not_a_positive_outcome_assertion(self):
        for assertion in (
            "assert!(!String::from_utf8_lossy(&out.stderr).contains(EXPECTED));",
            "assert!((!out.stderr.contains(EXPECTED)));",
            "assert!(out.stderr.contains(EXPECTED) == false);",
            "assert_ne!(out.stderr.contains(EXPECTED), true);",
        ):
            with self.subTest(assertion=assertion):
                self.assertFalse(checker._asserts_status(assertion, "out"))
                code = checker._GOOD_LIVE.replace(
                    "assert!(out.status.success());", assertion)
                self.assertIn("C3", {f.rule for f in checker.lint_file("fixture.rs", code)})
        self.assertFalse(checker._asserts_status("assert!(!out.status.success());", "out"))

    def test_diagnostic_assertion_requires_this_process_and_a_required_condition(self):
        for assertion in (
            "assert!(out.stderr.is_empty() && other.contains(EXPECTED));",
            "assert!(out.stderr.contains(EXPECTED) || true);",
            "assert!(out.status.success() || true);",
            "assert!(out.status.success() == out.status.success());",
            "assert!(!out.status.success());",
        ):
            with self.subTest(assertion=assertion):
                self.assertFalse(checker._asserts_status(assertion, "out"))
                code = checker._GOOD_LIVE.replace("assert!(out.status.success());", assertion)
                self.assertIn("C3", {f.rule for f in checker.lint_file("fixture.rs", code)})
        for assertion in (
            'assert!(String::from_utf8_lossy(&out.stderr).contains(EXPECTED), "details");',
            "assert!((out.stderr.contains(EXPECTED)));",
            "assert_eq!(out.status.code(), Some(0));",
        ):
            self.assertTrue(checker._asserts_status(assertion, "out"), assertion)

    def test_diagnostic_assertion_requires_a_nonempty_pattern(self):
        for pattern, accepted in [('""', False), ('r""', False), ('"expected"', True), ('EXPECTED', True)]:
            with self.subTest(pattern=pattern):
                assertion = f'assert!(String::from_utf8_lossy(&out.stderr).contains({pattern}));'
                self.assertEqual(checker._asserts_status(assertion, "out"), accepted)
                code = checker._GOOD_LIVE.replace("assert!(out.status.success());", assertion)
                self.assertEqual("C3" not in {f.rule for f in checker.lint_file("fixture.rs", code)}, accepted)

    def test_inline_spawn_must_bind_its_result_before_asserting(self):
        code = checker._GOOD_LIVE.replace(
            '    let out = support::live_tool(&gate, "nargo").output().expect("nargo runs");\n'
            '    assert!(out.status.success());',
            '    assert!(true || support::live_tool(&gate, "nargo").status().unwrap().success());')
        self.assertNotEqual(code, checker._GOOD_LIVE)
        self.assertIn("C3", {f.rule for f in checker.lint_file("fixture.rs", code)})

    def test_question_mark_before_either_brace_is_an_early_exit(self):
        for expression in ("let x = { f()? };", "if f()? {}"):
            with self.subTest(expression=expression):
                root = self.fixture()
                path = root / "crates/sparq-zk-compose/tests/live.rs"
                path.write_text(checker._GOOD_LIVE.replace(
                    "    let out = ", f"    {expression}\n    let out = "))
                self.assertIn("G", {f.rule for f in self.findings(root)})

    def test_completions_outside_the_last_test_statement_are_red(self):
        for source in (
            "fn helper(token: Token) {\n    support::complete(token);\n}\n",
            "const VALUE: () = { support::complete(token); };\n",
            "macro_rules! finish { () => { support::complete(token); } }\n",
            "#[test] fn plain() {\n    fn helper(token: Token) {\n        support::complete(token);\n    }\n}\n",
            "#[test] fn plain() {\n    let finish = || { support::complete(token); };\n}\n",
        ):
            with self.subTest(source=source):
                root = self.fixture()
                (root / "crates/sparq-zk-compose/tests/extra.rs").write_text(source)
                self.assertIn("G", {f.rule for f in self.findings(root)})

    def test_completion_and_declared_checks_reject_missing_obligations(self):
        root = self.fixture()
        trees = checker.collect(root)
        sites, _ = checker.inventory(trees)
        self.assertEqual(checker.check_completion(trees, sites), [])
        self.assertEqual(checker.check_declared(root, trees), [])
        path = root / "crates/sparq-zk-compose/tests/live.rs"
        path.write_text(checker._GOOD_LIVE.replace("    support::complete(gate);\n", ""))
        trees = checker.collect(root)
        sites, _ = checker.inventory(trees)
        self.assertTrue(checker.check_completion(trees, sites))
        declared = root / checker.DECLARED_REL["sparq-zk-compose"]
        declared.write_text(declared.read_text().replace("live tests/live.rs::live_case\n", ""))
        self.assertTrue(checker.check_declared(root, trees))

    def test_complete_must_follow_the_assertions_and_consume_the_token(self):
        for label, old, new in (
            ("deleted", "    support::complete(gate);\n", ""),
            ("before assertions", "    assert!(out.status.success());\n    support::complete(gate);",
             "    support::complete(gate);\n    assert!(out.status.success());"),
            ("wrong token", "support::complete(gate)", "support::complete(other)"),
            ("should_panic", "#[test]", "#[test]\n#[should_panic]"),
        ):
            with self.subTest(label):
                root = self.fixture()
                path = root / "crates/sparq-zk-compose/tests/live.rs"
                path.write_text(checker._GOOD_LIVE.replace(old, new))
                self.assertIn("G", {f.rule for f in self.findings(root)})

    def test_declared_inventory_agrees_in_both_directions(self):
        for addition in (False, True):
            with self.subTest(addition=addition):
                root = self.fixture()
                if addition:
                    (root / "crates/sparq-zk-compose/tests/extra.rs").write_text(
                        checker._GOOD_LIVE.replace("live_case", "extra_case"))
                else:
                    path = root / checker.DECLARED_REL["sparq-zk-compose"]
                    path.write_text(path.read_text().replace("live tests/live.rs::live_case\n", ""))
                self.assertIn("L", {f.rule for f in self.findings(root)})

    def test_entry_without_completion_and_completion_without_entry_are_red(self):
        for kind in ("enter", "complete"):
            with self.subTest(kind=kind):
                root = self.fixture()
                lines = checker._ledger_lines(checker._FIXTURE_KEYS)
                other = "complete" if kind == "enter" else "enter"
                lines.remove(f"{other} {checker._FIXTURE_KEYS[0]}")
                path = root / "ledger.txt"
                path.write_text("\n".join(lines) + "\n")
                self.assertEqual(checker.verify_ledger(root, path), 1)

    def test_statement_aliases_launchers_and_unrelated_assertions_are_red(self):
        for text, rule in (
            ('#[test] fn case() { Command::new("env").args(["nargo", "execute"]); }', "A1"),
            ('#[cfg(feature="x")]\n#[test] fn case() { use crate::support::{\n gate as g}; }', "A8"),
            ('pub(crate) use support::{gate as g};', "A8"),
            ('#[cfg(test)] pub(in crate) use support::{gate as g};', "A8"),
            (checker._GOOD_LIVE.replace('assert!(out.status.success())',
                                       'assert!(other.status.success())'), "C3"),
            ('#[test] fn case() { match run() { Err(DriverError::Spawn { .. })\n => {}, _ => {} } }', "C2"),
        ):
            with self.subTest(text=text):
                self.assertIn(rule, {f.rule for f in checker.lint_file("fixture.rs", text)})

    def test_inline_module_paths_ignore_multiline_noncode(self):
        root = self.fixture()
        rel = "crates/sparq-zk-compose/src/tree.rs"
        path = root / "crates/sparq-zk-compose/src/tree/outer/inner.rs"
        path.parent.mkdir(parents=True)
        path.write_text("#[test] fn hidden() {}")
        source = '/*\nmod absent;\n*/\nconst TEXT: &str = r"mod missing;";\nmod outer { mod inner; }'
        trees = checker.Trees()
        self.assertEqual(checker._mod_targets(root, rel, source, trees),
                         [str(path.relative_to(root))])
        self.assertEqual(trees.findings, [])
        scope, extra = checker._src_scope(root, "sparq-zk-compose", rel,
            "#[cfg(test)]\nmod outer { mod inner; }", trees)
        self.assertIn(2, scope)
        self.assertEqual(extra, [str(path.relative_to(root))])
        self.assertEqual(trees.findings, [])

    def test_relative_root_is_resolved_before_module_collection(self):
        import contextlib
        import io
        root = self.fixture()
        output = io.StringIO()
        with contextlib.redirect_stdout(output), contextlib.redirect_stderr(output):
            result = checker.main(["--check", "--root", os.path.relpath(root, Path.cwd())])
        self.assertEqual(result, 0, output.getvalue())

    def test_verification_rejects_static_completion_errors(self):
        root = self.fixture()
        (root / "crates/sparq-zk-compose/tests/live.rs").write_text(
            checker._GOOD_LIVE.replace("    support::complete(gate);\n", ""))
        ledger = root / "ledger.txt"
        ledger.write_text("\n".join(checker._ledger_lines(checker._FIXTURE_KEYS)) + "\n")
        self.assertEqual(checker.verify_ledger(root, ledger), 1)


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

    def test_fixture_calls_use_existing_support_functions(self):
        exported = set(re.findall(r"pub fn (\w+)\(", self.text))
        for label, (_file, fixture) in checker.EVASIONS.items():
            calls = set(re.findall(r"support::(\w+)\s*\(", fixture))
            self.assertLessEqual(calls, exported, label)

    def test_live_tool_hard_fails_on_its_own(self):
        self.assertRegex(
            self.text,
            r"pub fn live_tool\([^)]*\)[^{]*\{\s*assert!\(\s*on_path\(tool\)",
            "live_tool must re-check PATH itself, so an is_err() test on the spawn "
            "cannot become a soft pass",
        )

    def test_the_gate_records_only_on_the_allowed_path(self):
        self.assertIn(".append(true)", self.text, "the append must be O_APPEND")
        self.assertIn("write_all(line.as_bytes())", self.text, "one write per line")
        body = self.text.split("pub fn gate(", 1)[1]
        allowed, rest = body.split('record("enter", site);', 1)
        self.assertIn("missing.is_empty()", allowed, "recording happens on the run path")
        self.assertNotIn("record(", rest.split("pub fn complete", 1)[0],
                         "a skip or a panic must record nothing")

    def test_no_orphaned_doc_block(self):
        """A `///` block followed by a blank line documents nothing and trips
        clippy::empty_line_after_doc_comments under the nightly -D warnings lane."""
        lines = self.text.splitlines()
        for i, line in enumerate(lines[:-1]):
            if line.strip().startswith("///") and not lines[i + 1].strip():
                self.fail(f"orphaned doc comment at line {i + 1}: {line.strip()!r}")

    def test_no_doc_comment_on_a_macro_invocation(self):
        """`///` on a `macro!` invocation documents nothing: rustdoc cannot see
        through the expansion, so clippy's `unused_doc_comments` fires and the
        nightly workspace lane (`-D warnings`) goes red."""
        lines = self.text.splitlines()
        for i, line in enumerate(lines):
            if not re.match(r"^\s*[\w:]+!\s*[{(\[]", line):
                continue
            j = i - 1
            while j >= 0 and lines[j].strip().startswith("#["):
                j -= 1
            if j >= 0 and lines[j].strip().startswith("///"):
                self.fail(f"doc comment on the macro invocation at line {i + 1}: "
                          f"{line.strip()!r}")

    def test_the_ledger_env_var_is_private_to_the_gate_module(self):
        self.assertIn('const LEDGER_ENV: &str = "SPARQ_ZK_LIVE_LEDGER";', self.text)
        self.assertNotIn("pub const LEDGER_ENV", self.text,
                         "a test must not be able to name or redirect the ledger")

    def test_the_ledger_key_is_the_call_site(self):
        """The identity is file:line, and `#[track_caller]` is what makes it the
        CALLER's line. The behaviour itself is pinned by the gate module's own
        `gate_identity_tests`, which run in every binary that declares the module."""
        self.assertIn('format!("{}:{}", loc.file().replace(\'\\\\\', "/"), loc.line())',
                      self.text)
        for fn in ("pub fn gate(",):
            head = self.text.rsplit(fn, 1)[0]
            self.assertTrue(
                head.rstrip().endswith("#[track_caller]"),
                f"{fn} must carry #[track_caller], or the key becomes the gate's own "
                "line for every caller",
            )
        self.assertIn("mod gate_identity_tests", self.text)
        for name in (
            "fn the_recorded_key_is_the_call_site_of_the_gate_call",
            "fn a_missing_tool_records_nothing_on_either_path",
        ):
            self.assertIn(name, self.text)


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


class DriverRouteSuites(unittest.TestCase):
    """The structural driver-route suites must fail on a spawn failure by themselves.

    `holder_pop_forge.rs` builds a `CircuitProver` and calls `verify_manifest`, but it
    is STRUCTURAL: every arm it accepts names a specific `CheckError` variant and its
    wildcard arms panic, so a missing tool produces a different variant and the test
    FAILS. That is why it is not gated — gating it would move 12 trusted-holder-gap
    tests out of ci.yml's per-PR run, where they pass today in about a second without
    any toolchain, into the nightly lane only. CHECK C1 enforces the property that
    makes that safe, instead of an annotation asserting it.
    """

    HOLDER = "crates/sparq-zk-compose/tests/holder_pop_forge.rs"

    def setUp(self):
        self.text = (REPO / self.HOLDER).read_text(encoding="utf-8")

    def test_it_carries_no_allow_annotation(self):
        self.assertNotIn("zk-toolchain-gate: allow", self.text)

    def test_every_accepting_arm_names_a_specific_variant(self):
        import re

        accepting = re.findall(r"^\s*(?:\|\s*)?(.+?)\s*=>\s*\{\}\s*$", self.text, re.M)
        self.assertGreater(len(accepting), 10, "the forge arms moved")
        for arm in accepting:
            self.assertRegex(
                arm,
                r"Err\(CheckError::[A-Z]",
                f"an accepting arm that does not name a variant would let a spawn "
                f"failure pass: {arm!r}",
            )

    def test_an_unnamed_error_arm_in_it_turns_the_check_red(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = copy_tree(tmp)
            p = root / self.HOLDER
            p.write_text(
                p.read_text(encoding="utf-8").replace(
                    "Err(CheckError::HolderKeyMismatch) => {}", "Err(_) => {}", 1
                ),
                encoding="utf-8",
            )
            proc = run_script("--check", "--root", str(root))
            self.assertEqual(proc.returncode, 1, proc.stdout)
            self.assertIn("[C1]", proc.stderr)
            self.assertIn("holder_pop_forge.rs", proc.stderr)


class SourceReader(unittest.TestCase):
    """The two readers the `src` region scan rests on."""

    def setUp(self):
        sys.path.insert(0, str(REPO / "scripts"))
        import importlib.util

        spec = importlib.util.spec_from_file_location("zk_gate_checker", SCRIPT)
        self.mod = importlib.util.module_from_spec(spec)
        sys.modules["zk_gate_checker"] = self.mod
        spec.loader.exec_module(self.mod)

    def test_blanking_literals_never_moves_a_line(self):
        """A multi-line string literal with a `\\` continuation carries unbalanced
        braces; losing its newline shifts every later line number, and the brace
        matcher then walks off the end of the file."""
        for rel in ("crates/sparq-zk/src", "crates/sparq-zk-compose/src",
                    "crates/sparq-zk/tests", "crates/sparq-zk-compose/tests"):
            for path in (REPO / rel).rglob("*.rs"):
                text = path.read_text(encoding="utf-8")
                self.assertEqual(
                    len(self.mod.strip_noncode(text).splitlines()),
                    len(text.splitlines()),
                    f"line count drifted on {path}",
                )

    def test_every_test_function_is_found(self):
        """Each `#[test]` in a real file falls inside some span, or C1 and the
        helper fixpoint silently skip it."""
        for rel in (POSEIDON, "crates/sparq-zk-compose/tests/holder_pop_forge.rs",
                    SRC_GATED[2]):
            text = (REPO / rel).read_text(encoding="utf-8")
            lines = text.splitlines()
            spans = self.mod.fn_spans(text)
            self.assertTrue(spans, rel)
            marks = [i for i, line in enumerate(lines) if line.strip() == "#[test]"]
            self.assertTrue(marks, rel)
            for i in marks:
                self.assertTrue(
                    any(a <= i + 2 <= b for a, b, _ in spans),
                    f"{rel}: the #[test] at line {i + 1} is in no fn span",
                )


class SrcTestRegions(unittest.TestCase):
    """`src` `#[cfg(test)]` code is scanned; the production code above it is not."""

    def test_the_src_live_tests_go_through_the_gate(self):
        for rel in SRC_GATED:
            text = (REPO / rel).read_text(encoding="utf-8")
            self.assertIn("crate::support::", text, rel)
        text = (REPO / COMPOSE_LIB).read_text(encoding="utf-8")
        self.assertIn('#[path = "../tests/support/mod.rs"]', text)
        self.assertIn("mod support;", text)

    def test_a_planted_spawn_in_a_src_test_is_red(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = copy_tree(tmp, full_src=True)
            p = root / "crates/sparq-zk/src/field.rs"
            text = p.read_text(encoding="utf-8")
            marker = "mod tests {"
            self.assertIn(marker, text)
            p.write_text(
                text.replace(
                    marker,
                    marker + "\n    #[test]\n    fn planted() {\n"
                    '        let _ = std::process::Command::new("nargo").output();\n'
                    "    }\n",
                    1,
                ),
                encoding="utf-8",
            )
            proc = run_script("--check", "--root", str(root))
            self.assertEqual(proc.returncode, 1, proc.stdout)
            self.assertIn("src/field.rs", proc.stderr)
            self.assertIn("[A1]", proc.stderr)

    def test_the_same_spawn_in_production_code_is_not(self):
        """The driver spawns nargo/bb for real, above its test module. Linting that
        would make the whole src scan unusable."""
        with tempfile.TemporaryDirectory() as tmp:
            root = copy_tree(tmp, full_src=True)
            p = root / "crates/sparq-zk/src/field.rs"
            p.write_text(
                'pub fn tool() -> std::process::Command {\n'
                '    std::process::Command::new("nargo")\n}\n'
                + p.read_text(encoding="utf-8"),
                encoding="utf-8",
            )
            proc = run_script("--check", "--root", str(root))
            self.assertEqual(
                proc.returncode, 0, f"{proc.stdout}\n{proc.stderr}"
            )

    def test_dropping_the_shared_gate_declaration_is_red(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = copy_tree(tmp, full_src=True)
            p = root / COMPOSE_LIB
            p.write_text(
                p.read_text(encoding="utf-8").replace(
                    '#[path = "../tests/support/mod.rs"]', "#[path = \"support.rs\"]", 1
                ),
                encoding="utf-8",
            )
            proc = run_script("--check", "--root", str(root))
            self.assertEqual(proc.returncode, 1, proc.stdout)
            self.assertIn("[F]", proc.stderr)

    def test_a_src_gate_call_is_in_the_ledger_inventory(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = copy_tree(tmp, full_src=True)
            p = self._partial(root)
            proc = run_script("--verify-ledger", str(p), "--root", str(root))
            self.assertEqual(proc.returncode, 1)
            self.assertIn("crates/sparq-zk-compose/src/driver.rs:", proc.stderr)
            self.assertIn("crates/sparq-zk-compose/src/result/signed.rs:", proc.stderr)

    def test_a_gate_call_outside_a_cfg_test_region_is_inventoried(self):
        """The lint scope stops at the region; the inventory does not."""
        with tempfile.TemporaryDirectory() as tmp:
            root = copy_tree(tmp, full_src=True)
            (root / "crates/sparq-zk-compose/src/stray.rs").write_text(
                "fn helper() {\n"
                '    let _ = crate::support::gate("stray_site", &["nargo"]);\n}\n',
                encoding="utf-8",
            )
            p = self._partial(root)
            proc = run_script("--verify-ledger", str(p), "--root", str(root))
            self.assertEqual(proc.returncode, 1)
            self.assertIn("crates/sparq-zk-compose/src/stray.rs:2", proc.stderr)

    def test_a_grandchild_of_a_src_test_module_is_linted(self):
        """`#[cfg(test)] mod inner;` makes inner.rs test code in full, and its own
        plain `mod deeper;` child too — nothing there carries a cfg attribute."""
        with tempfile.TemporaryDirectory() as tmp:
            root = copy_tree(tmp, full_src=True)
            base = root / "crates/sparq-zk-compose/src"
            (base / "stray.rs").write_text(
                "#[cfg(test)]\nmod inner;\n", encoding="utf-8")
            (base / "stray" / "inner").mkdir(parents=True)
            (base / "stray" / "inner.rs").write_text(
                "mod deeper;\n", encoding="utf-8")
            (base / "stray" / "inner" / "deeper.rs").write_text(
                "#[test]\nfn case() {\n"
                '    let _ = std::process::Command::new("nargo").output();\n}\n',
                encoding="utf-8",
            )
            proc = run_script("--check", "--root", str(root))
            self.assertEqual(proc.returncode, 1, proc.stdout)
            self.assertIn("deeper.rs", proc.stderr)
            self.assertIn("[A1]", proc.stderr)

    @staticmethod
    def _partial(root: Path) -> Path:
        """A ledger holding ONE real call site, so verification reaches the
        missing-list instead of short-circuiting on an unknown entry."""
        p = root / "ledger.txt"
        p.write_text(
            "enter " + LedgerVerification.a_real_key(root) + "\n", encoding="utf-8"
        )
        return p


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

    def test_the_verify_step_carries_only_name_and_run(self):
        import yaml  # noqa: PLC0415

        doc = yaml.safe_load(self.text)
        self.assertIsNone(doc.get("defaults"), "no workflow-level defaults")
        for jid, job in (doc.get("jobs") or {}).items():
            steps = [s or {} for s in (job.get("steps") or [])]
            verify = [s for s in steps if s.get("name") == VERIFY_STEP]
            if not verify:
                continue
            self.assertEqual(set(verify[0]), {"name", "run"})
            self.assertIsNone(job.get("defaults"), f"no defaults on {jid}")
            for step in steps:
                if "shell" in step:
                    self.assertEqual(step["shell"], "bash", step.get("name"))

    def test_a_shell_or_defaults_on_the_verify_step_turns_the_check_red(self):
        for label, old, new in (
            ("a swallowing shell",
             f"      - name: {VERIFY_STEP}\n        run:",
             f'      - name: {VERIFY_STEP}\n        shell: "true {{0}}"\n        run:'),
            ("job-level defaults",
             "    runs-on: ubuntu-latest",
             '    defaults:\n      run:\n        shell: "true {0}"\n'
             "    runs-on: ubuntu-latest"),
        ):
            with self.subTest(label), tempfile.TemporaryDirectory() as tmp:
                root = copy_tree(tmp)
                wf = root / WORKFLOW
                text = wf.read_text(encoding="utf-8")
                self.assertIn(old, text, label)
                wf.write_text(text.replace(old, new, 1), encoding="utf-8")
                proc = run_script("--check", "--root", str(root))
                self.assertEqual(proc.returncode, 1, f"{label}: {proc.stdout}")
                self.assertIn("[D]", proc.stderr)

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
