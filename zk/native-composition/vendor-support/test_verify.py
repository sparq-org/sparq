#!/usr/bin/env python3
"""[GPT-6] Discriminating checks for exact native dependency selection."""

import copy
import importlib.util
import io
import json
from pathlib import Path
import subprocess
import unittest
from contextlib import redirect_stderr
from unittest.mock import patch


SPEC = importlib.util.spec_from_file_location("native_vendor_verify", Path(__file__).with_name("verify.py"))
VERIFY = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(VERIFY)


class MetadataSelectionTests(unittest.TestCase):
    def setUp(self):
        self.metadata = {
            "packages": [
                {"name": "ark-relations", "version": "0.4.0", "source": None,
                 "manifest_path": str(VERIFY.PACKAGE / "Cargo.toml"), "id": "local-ark"},
                {"name": "tracing-subscriber", "version": "0.3.23", "source": "registry",
                 "manifest_path": "/registry/tracing-subscriber/Cargo.toml", "id": "subscriber"},
            ],
            "resolve": {"nodes": [{"id": "local-ark"}, {"id": "subscriber"}]},
        }

    def test_exact_vendor_is_accepted(self):
        VERIFY.check_metadata(self.metadata)

    def test_wrong_local_vendor_path_rejects(self):
        self.metadata["packages"][0]["manifest_path"] = str(VERIFY.NATIVE / "other/Cargo.toml")
        with self.assertRaisesRegex(ValueError, "exact vendor manifest"):
            VERIFY.check_metadata(self.metadata)

    def test_registry_substitution_rejects(self):
        self.metadata["packages"][0]["source"] = "registry+https://github.com/rust-lang/crates.io-index"
        with self.assertRaisesRegex(ValueError, "must be local"):
            VERIFY.check_metadata(self.metadata)

    def test_inactive_patch_rejects(self):
        self.metadata["resolve"]["nodes"] = [{"id": "subscriber"}]
        with self.assertRaisesRegex(ValueError, "active graph"):
            VERIFY.check_metadata(self.metadata)

    def test_old_subscriber_rejects(self):
        self.metadata["packages"][1]["version"] = "0.2.25"
        with self.assertRaisesRegex(ValueError, "maintained subscriber"):
            VERIFY.check_metadata(self.metadata)

    def test_duplicate_subscriber_rejects(self):
        old = copy.deepcopy(self.metadata["packages"][1])
        old.update(version="0.2.25", id="old-subscriber")
        self.metadata["packages"].append(old)
        self.metadata["resolve"]["nodes"].append({"id": "old-subscriber"})
        with self.assertRaisesRegex(ValueError, "maintained subscriber"):
            VERIFY.check_metadata(self.metadata)


class MetadataInvocationTests(unittest.TestCase):
    def test_success_retains_exact_locked_offline_all_features_flags(self):
        result = subprocess.CompletedProcess([], 0, stdout=json.dumps({"packages": []}), stderr="")
        with patch.object(VERIFY.subprocess, "run", return_value=result) as run:
            self.assertEqual(VERIFY.native_metadata(), {"packages": []})
        run.assert_called_once_with(
            ["cargo", "metadata", "--offline", "--locked", "--all-features", "--format-version", "1", "--manifest-path", str(VERIFY.NATIVE / "Cargo.toml")],
            capture_output=True, text=True, timeout=120,
        )

    def test_failure_retains_cargo_diagnostic_without_retry(self):
        result = subprocess.CompletedProcess(["cargo", "metadata"], 101,
                                             stdout="not a metadata success", stderr="fixture missing archive\n")
        stderr = io.StringIO()
        with patch.object(VERIFY.subprocess, "run", return_value=result) as run:
            with redirect_stderr(stderr), self.assertRaises(subprocess.CalledProcessError) as caught:
                VERIFY.native_metadata()
        self.assertEqual(caught.exception.returncode, 101)
        self.assertEqual(stderr.getvalue(), "fixture missing archive\n")
        self.assertEqual(run.call_count, 1)

    def test_ci_fetches_locked_graph_before_offline_provenance(self):
        workflow = (VERIFY.NATIVE.parents[1] / ".github/workflows/zk-native-composition.yml").read_text()
        step = workflow.split("      - name: Verify native dependency patch provenance and API compatibility\n", 1)[1].split("      - name:", 1)[0]
        fetch = "cargo fetch --locked --manifest-path zk/native-composition/Cargo.toml"
        verify = "python3 zk/native-composition/vendor-support/verify.py --smoke"
        self.assertIn(fetch, step)
        self.assertLess(step.index(fetch), step.index(verify))


if __name__ == "__main__":
    unittest.main()
