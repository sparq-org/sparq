#!/usr/bin/env python3
"""Issuance hardening audit tests; run directly or with unittest discovery.

Stdlib fixtures and CLI checks; PyYAML checks the always-on workflow wiring.
"""

from __future__ import annotations

import importlib.util
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

import yaml

REPO_ROOT = Path(__file__).resolve().parent.parent.parent
SCRIPT = REPO_ROOT / "scripts" / "check-raw-commitment-signers.py"
spec = importlib.util.spec_from_file_location("raw_commitment_signers", SCRIPT)
assert spec and spec.loader
inventory = importlib.util.module_from_spec(spec)
sys.modules[spec.name] = inventory
spec.loader.exec_module(inventory)


class TestTokenizer(unittest.TestCase):
    def test_original_inventory_cases(self):
        source = r'''
            // key.sign_commitment(&comment);
            const DOC: &str = r#"key.sign_commitment(&literal); }"#;
            const BYTES: &[u8] = br#"" } key.sign_commitment(&literal)"#;
            #[cfg(test)] mod tests { fn f() { key.sign_commitment(&fixture); } }
            #[cfg(all(test, feature = "x"))] mod gated { fn f() { key.sign_commitment(&fixture); } }
            #[cfg(test)] mod external_fixture;
            #[test] fn test() { key.sign_commitment(&fixture); }
            fn production() { key.sign_commitment_with_status(&c, &salt, &status); }
            #[cfg(any(test, feature = "x"))] fn shared() { SecretKey::sign_commitment(&key, &c); }
            #[cfg(all(any(test, feature = "x"), feature = "y"))]
            fn also_shared() { key.sign_commitment(&c); }
            fn holder() { key.sign_commitment_with_holder(&c, &salt, &status, &h); }
            fn salted() { key.sign_commitment_with_salt(&c, &salt); }
        '''
        scanned = inventory.scan(source)
        self.assertEqual(scanned.test_modules, ["external_fixture"])
        self.assertEqual([signer for _, signer, _ in scanned.calls], [
            "sign_commitment_with_status", "sign_commitment", "sign_commitment",
            "sign_commitment_with_holder", "sign_commitment_with_salt",
        ])

    def test_comments_and_literal_delimiters(self):
        source = r'''
            /* key.sign_commitment(&comment); /* nested } */ */
            const S: &str = "\" } // key.sign_commitment(&literal)";
            const B: &[u8] = b"} key.sign_commitment(&literal)";
            const R: &str = r###""## } key.sign_commitment(&literal)"###;
            const C: &CStr = cr#"} key.sign_commitment(&literal)"#;
            fn chars<'a>() { let a = '}'; let b = b'{'; let c = '\''; }
            #[test] fn fixture() {
                let s = "}";
                key.sign_commitment(&fixture);
            }
            fn production() { key.sign_commitment(&c); }
        '''
        self.assertEqual(inventory.scan(source).calls, [(12, "sign_commitment", "& c")])

    def test_each_test_item_attribute(self):
        for attr in ("test", "cfg(test)", 'cfg(all(test, feature = "x"))',
                     'cfg(all(feature = "x", test))'):
            with self.subTest(attr=attr):
                self.assertEqual(inventory.scan(
                    f"#[{attr}] fn fixture() {{ key.sign_commitment(&c); }}"
                ).calls, [])

    def test_shared_cfg_items_stay_in_inventory(self):
        for attr in ('cfg(any(test, feature = "x"))',
                     'cfg(all(any(test, feature = "x"), feature = "y"))',
                     "cfg(not(test))"):
            with self.subTest(attr=attr):
                self.assertEqual(inventory.scan(
                    f"#[{attr}] fn shared() {{ key.sign_commitment(&c); }}"
                ).calls, [(1, "sign_commitment", "& c")])

    def test_unclosed_call_fails(self):
        with self.assertRaisesRegex(ValueError, "unclosed"):
            inventory.scan("key.sign_commitment(&c;")


class TestInventory(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.write("fixture/src/lib.rs", "")
        self.write("sparq-trust/examples/trust_graph_closure.rs",
                   "fn f() { key.sign_commitment(&certification_message(&cert)); }")
        self.write("sparq-trust/examples/admission_cost.rs",
                   "fn f() { key.sign_commitment(&certification_message(&cert)); "
                   "key.sign_commitment(&certification_message(&cert)); }")

    def write(self, path, source):
        file = self.root / "crates" / path
        file.parent.mkdir(parents=True, exist_ok=True)
        file.write_text(source, encoding="utf-8")

    def run_script(self):
        return subprocess.run([sys.executable, str(SCRIPT), "--root", str(self.root)],
                              capture_output=True, text=True, check=False)

    def test_synthetic_tree_passes(self):
        result = self.run_script()
        self.assertEqual(result.returncode, 0, result.stderr)

    def test_unlisted_call_fails_with_path_line_and_typed_api(self):
        self.write("fixture/src/lib.rs", "\nfn f() { key.sign_commitment_with_status(&c, &s, &r); }")
        result = self.run_script()
        self.assertEqual(result.returncode, 1, result.stderr)
        self.assertIn("crates/fixture/src/lib.rs:2: sign_commitment_with_status", result.stderr)
        self.assertIn("SecretKey::attest", result.stderr)

    def test_changed_counts_fail_in_both_directions(self):
        for count in (0, 2):
            with self.subTest(count=count):
                self.write("sparq-trust/examples/trust_graph_closure.rs",
                           "fn f() {" + "key.sign_commitment(&certification_message(&cert));" * count + "}")
                result = self.run_script()
                self.assertEqual(result.returncode, 1, result.stderr)
                self.assertIn(f"expected 1, found {count}", result.stderr)
                self.assertIn("SecretKey::attest", result.stderr)

    def test_allowlist_pins_arguments(self):
        self.write("sparq-trust/examples/trust_graph_closure.rs",
                   "fn f() { key.sign_commitment(&other_message(&cert)); }")
        self.assertIn("requires an explicit allow-list entry", self.run_script().stderr)

    def test_test_directories_and_out_of_line_modules_are_excluded(self):
        self.write("fixture/src/lib.rs", "#[cfg(test)] mod flat;\n#[cfg(all(test, unix))] mod nested;")
        for path in ("fixture/tests/integration.rs", "fixture/target/generated.rs",
                     "fixture/src/flat.rs", "fixture/src/nested/mod.rs"):
            self.write(path, "fn fixture() { key.sign_commitment(&c); }")
        result = self.run_script()
        self.assertEqual(result.returncode, 0, result.stderr)

    def test_real_tree_passes(self):
        result = subprocess.run([sys.executable, str(SCRIPT)], capture_output=True,
                                text=True, check=False, cwd=self.root)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("PASS", result.stdout)


class TestWorkflow(unittest.TestCase):
    def test_audit_and_self_test_run_on_every_pr(self):
        workflow = yaml.safe_load((REPO_ROOT / ".github/workflows/docs-quality.yml").read_text())
        trigger = workflow.get("on", workflow.get(True))
        self.assertEqual(trigger["pull_request"], {
            "types": ["opened", "synchronize", "reopened", "ready_for_review"],
        })
        job = workflow["jobs"]["quick-gates"]
        self.assertNotIn("if", job)
        self.assertNotIn("continue-on-error", job)
        for command in ("python3 scripts/check-raw-commitment-signers.py",
                        "python3 scripts/tests/test_raw_commitment_signers.py"):
            steps = [s for s in job["steps"] if s.get("run") == command]
            self.assertEqual(len(steps), 1, command)
            self.assertNotIn("if", steps[0])
            self.assertNotIn("continue-on-error", steps[0])


if __name__ == "__main__":
    unittest.main(verbosity=2)
