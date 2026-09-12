"""[GPT-6] Hermetic provenance rejection tests; no simulated proof is evidence."""

import importlib.util
import json
from pathlib import Path
import subprocess
import tempfile
import unittest

SPEC = importlib.util.spec_from_file_location(
    "exact_evidence", Path(__file__).parents[1] / "ci_exact_evaluator_evidence.py"
)
evidence = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(evidence)


class EvidenceGuards(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.artifact = self.root / "artifact"
        self.artifact.mkdir()
        (self.artifact / "guest.bin").write_bytes(b"synthetic byte fixture; not a guest")
        self.pin = {"sha256": list(bytes.fromhex(evidence.digest((self.artifact / "guest.bin").read_bytes()))),
                    "image_id": list(range(8))}
        (self.artifact / "pin.json").write_text(json.dumps(self.pin))

    def test_artifact_bytes_and_pin_are_both_required(self):
        self.assertEqual(evidence.artifact_pin(self.artifact), self.pin)
        (self.artifact / "guest.bin").write_bytes(b"different bytes")
        with self.assertRaisesRegex(ValueError, "hash differs"):
            evidence.artifact_pin(self.artifact)
        (self.artifact / "guest.bin").unlink()
        with self.assertRaises(FileNotFoundError):
            evidence.artifact_pin(self.artifact)

    def test_malformed_pin_and_empty_program_fail(self):
        for key, value in [("image_id", [True] * 8), ("sha256", [-1] * 32),
                           ("image_id", [2**32] * 8), ("sha256", []), ("image_id", None)]:
            with self.subTest(key=key, value=value):
                (self.artifact / "pin.json").write_text(json.dumps(self.pin | {key: value}))
                with self.assertRaisesRegex(ValueError, "invalid exported"):
                    evidence.artifact_pin(self.artifact)
        (self.artifact / "guest.bin").write_bytes(b"")
        with self.assertRaisesRegex(ValueError, "missing or oversized"):
            evidence.artifact_pin(self.artifact)

    def test_receipt_inventory_and_common_artifact_are_required(self):
        receipt_dir = self.root / "receipts"
        receipt_dir.mkdir()
        with self.assertRaisesRegex(ValueError, "missing or unexpected"):
            evidence.receipts(self.root, self.pin, {"test-fixture"})
        record = {"schema": "sparq-synthetic-receipt-evidence-v1", "fixture": "test-fixture",
                  "synthetic_inputs": True, "pin": self.pin, "request": {"test": True},
                  "receipt": {"test_envelope_only": True}}
        file = receipt_dir / "test-fixture.json"
        file.write_text(json.dumps(record))
        self.assertEqual(set(evidence.receipts(self.root, self.pin, {"test-fixture"})), {"test-fixture"})
        # These only test the envelope collector. Rust verifies the actual receipts.
        for delta in [{"pin": self.pin | {"image_id": [9] * 8}}, {"receipt": None},
                      {"synthetic_inputs": False}, {"fixture": "wrong"}]:
            file.write_text(json.dumps(record | delta))
            with self.assertRaisesRegex(ValueError, "invalid or mismatched"):
                evidence.receipts(self.root, self.pin, {"test-fixture"})

    def test_pr_head_is_not_mislabeled_as_checkout_merge(self):
        event = self.root / "event.json"
        event.write_text(json.dumps({"number": 12, "pull_request": {
            "head": {"sha": "b" * 40}, "base": {"sha": "c" * 40}}}))
        env = {"GITHUB_SHA": "a" * 40, "GITHUB_EVENT_PATH": str(event),
               "GITHUB_EVENT_NAME": "pull_request", "GITHUB_RUN_ID": "123",
               "GITHUB_RUN_ATTEMPT": "2", "GITHUB_REPOSITORY": "fixture/repository"}
        result = evidence.event_identity(env, "a" * 40)
        self.assertEqual(result["checkout_sha"], "a" * 40)
        self.assertEqual(result["pr_head_sha"], "b" * 40)
        with self.assertRaisesRegex(ValueError, "checkout SHA"):
            evidence.event_identity(env, "d" * 40)
        event.write_text(json.dumps({"merge_group": {"head_sha": "b" * 40, "base_sha": "c" * 40}}))
        env["GITHUB_EVENT_NAME"] = "merge_group"
        with self.assertRaisesRegex(ValueError, "merge-group head"):
            evidence.event_identity(env, "a" * 40)

    def test_local_identity_cannot_impersonate_hosted_evidence(self):
        identity = evidence.campaign_identity({}, "a" * 40, True)
        self.assertEqual(identity["event"], "local")
        self.assertIsNone(identity["pr_head_sha"])
        for env in [{"GITHUB_ACTIONS": "true"}, {"GITHUB_EVENT_NAME": "pull_request"}]:
            with self.assertRaisesRegex(ValueError, "cannot replace hosted"):
                evidence.campaign_identity(env, "a" * 40, True)

    def test_snapshot_rejects_dirty_source_and_records_content_changes(self):
        def git(*args):
            return subprocess.check_output(["git", *args], cwd=self.root, stderr=subprocess.DEVNULL)
        git("init")
        source = self.root / "source.rs"
        source.write_text("first source")
        (self.root / ".gitignore").write_text("artifact/\n")
        git("add", "source.rs", ".gitignore")
        git("-c", "user.name=Fixture", "-c", "user.email=fixture@example.invalid",
            "-c", "commit.gpgsign=false", "commit", "-m", "fixture")
        before = evidence.snapshot(self.root)
        extra = self.root / "untracked.rs"
        extra.write_text("untracked source")
        with self.assertRaisesRegex(ValueError, "untracked checkout inputs"):
            evidence.snapshot(self.root)
        extra.unlink()
        source.write_text("changed source")
        with self.assertRaisesRegex(ValueError, "tracked checkout changes"):
            evidence.snapshot(self.root)
        git("add", "source.rs")
        git("-c", "user.name=Fixture", "-c", "user.email=fixture@example.invalid",
            "-c", "commit.gpgsign=false", "commit", "-m", "changed fixture")
        after = evidence.snapshot(self.root)
        self.assertNotEqual(before["checkout_sha"], after["checkout_sha"])
        self.assertNotEqual(before["tracked_sha256"], after["tracked_sha256"])

    def test_hal_requires_actual_module_events_without_hardware_inference(self):
        self.assertEqual(evidence.observed_hal("DEBUG risc0_circuit_rv32im::prove::hal::cpu: witgen: 32"),
                         ["risc0_circuit_rv32im::prove::hal::cpu"])
        self.assertEqual(evidence.observed_hal("DEBUG risc0_zkp::hal::cpu: output: 32"),
                         ["risc0_zkp::hal::cpu"])
        for unrelated in ["", "Apple Silicon Metal", "NVIDIA CUDA detected", "all proofs passed", "warning: risc0_zkp::hal::cpu source path"]:
            with self.assertRaisesRegex(ValueError, "HAL execution diagnostics"):
                evidence.observed_hal(unrelated)

    def test_workflow_requires_export_and_upload_of_actual_evidence(self):
        root = Path(__file__).parents[2]
        workflow = (root / ".github/workflows/zk-exact-evaluator.yml").read_text()
        self.assertIn("python3 scripts/ci_exact_evaluator_evidence.py", workflow)
        self.assertIn("if-no-files-found: error", workflow)
        self.assertIn("actions/upload-artifact@043fb46d1a93c77aae656e7c1c64a875d1fc6a0a", workflow)
        runner = (root / "scripts/ci_exact_evaluator_evidence.py").read_text()
        self.assertIn('"--nocapture", "--test-threads=1"', runner)
        self.assertNotIn('"--ignored"', runner)


if __name__ == "__main__":
    unittest.main()
