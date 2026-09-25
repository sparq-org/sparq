#!/usr/bin/env python3
"""[GPT-6] Pure Python hosted input/wiring controls; no Cargo or network calls."""
import json
from pathlib import Path
import tempfile
import tomllib
from types import SimpleNamespace
import unittest
from unittest.mock import patch

import fetch_baseline
import verify


class HostedControls(unittest.TestCase):
    def test_fetch_graph_retains_only_reviewed_registry_closure(self):
        packages = fetch_baseline.baseline_packages()
        self.assertEqual({p["name"] for p in packages},
                         {"proc-macro-error2", "proc-macro-error-attr2", "proc-macro2", "quote", "syn", "unicode-ident"})
        self.assertEqual(next(p["version"] for p in packages if p["name"] == "syn"), "2.0.119")
        removed = json.loads((verify.SUPPORT / "lock-delta.json").read_text())["removed_registry_packages"]
        self.assertTrue(all(p in packages for p in removed))

    def test_preparation_is_source_only_and_retains_exact_inputs(self):
        with tempfile.TemporaryDirectory() as temp, patch.object(fetch_baseline.subprocess, "run") as command:
            # Static provenance itself uses git apply; verify it separately in
            # test_verify.py so this guard observes any fetch/compile attempt.
            with patch.object(fetch_baseline.verify, "verify"):
                output = Path(temp) / "baseline"
                record = fetch_baseline.prepare(output)
            command.assert_not_called()
            lock = tomllib.loads((output / "Cargo.lock").read_text())["package"]
            expected = {(p["name"], p["version"], p["checksum"]) for p in record["registry_packages"]}
            self.assertEqual({(p["name"], p["version"], p["checksum"]) for p in lock if p.get("source")}, expected)
            self.assertEqual(record["lock_sha256"], verify.sha((output / "Cargo.lock").read_bytes()))
            manifest = tomllib.loads((output / "Cargo.toml").read_text())
            self.assertEqual(manifest["dependencies"], {"proc-macro-error2": "=2.0.1", "syn": "=2.0.119"})

    def test_hosted_lane_cannot_skip_fetch_controls_or_partial_evidence(self):
        root = verify.NATIVE.parents[1]
        workflow = (root / ".github/workflows/zk-native-composition.yml").read_text()
        control = workflow.split("      - name: Wasmer diagnostic, token and layout controls\n", 1)[1].split("      - name:", 1)[0]
        self.assertNotIn("continue-on-error", control)
        self.assertNotIn("|| true", control)
        self.assertIn("timeout-minutes: 15", control)
        for name in ("test_verify.py", "test_remote_protocol.py", "test_hosted.py", "fetch_baseline.py --fetch", "remote_checks.py"):
            self.assertIn("python3 zk/native-composition/vendor-support/wasmer/" + name, control)
        self.assertIn('--target-dir "$GITHUB_WORKSPACE/zk/native-composition/target"', control)
        evidence = workflow.split("      - name: Preserve Wasmer control evidence including failures\n", 1)[1].split("      - name:", 1)[0]
        self.assertIn("if: always()", evidence)
        self.assertIn("if-no-files-found: error", evidence)
        for path in ("wasmer-baseline-fetch", "wasmer-controls"):
            self.assertIn("${{ runner.temp }}/" + path, evidence)

    def test_fetch_failure_or_input_mutation_is_not_success(self):
        for code, mutate in ((1, False), (0, True)):
            with self.subTest(code=code, mutate=mutate), tempfile.TemporaryDirectory() as temp:
                output = Path(temp) / "baseline"

                def fake_fetch(command, **kwargs):
                    self.assertEqual(command[:3], ["cargo", "fetch", "--locked"])
                    if mutate:
                        with (output / "Cargo.lock").open("a") as lock:
                            lock.write("# unexpected input rewrite\n")
                    return SimpleNamespace(returncode=code)

                with patch.object(fetch_baseline.verify, "verify"), patch.object(fetch_baseline.subprocess, "run", side_effect=fake_fetch):
                    with self.assertRaises(ValueError):
                        fetch_baseline.fetch(output)
                self.assertTrue((output / "fetch.json").is_file())


if __name__ == "__main__":
    unittest.main()
