"""Hermetic provenance rejection tests; no simulated proof is evidence."""

import importlib.util
import json
from pathlib import Path
import re
import subprocess
import tempfile
import tomllib
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

    def test_each_version_retains_earlier_receipt_gates_and_native_features(self):
        self.assertEqual(evidence.active_profile(self.root), ("evaluate", evidence.V1_RECEIPTS))
        modules = self.root / "zk/sparql-evaluator/host/src"
        modules.mkdir(parents=True)
        (modules / "v2.rs").touch()
        self.assertEqual(evidence.active_profile(self.root),
                         ("evaluate", evidence.V1_RECEIPTS | evidence.V2_RECEIPTS))
        (modules / "v3.rs").touch()
        feature, expected = evidence.active_profile(self.root)
        self.assertEqual(feature, "graph-results")
        self.assertEqual(expected, evidence.V1_RECEIPTS | evidence.V2_RECEIPTS | evidence.V3_RECEIPTS)
        self.assertEqual(len(expected), 8)
        (self.root / "receipts").mkdir()
        with self.assertRaisesRegex(ValueError, "missing or unexpected"):
            evidence.receipts(self.root, self.pin, expected)

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

    def test_guest_invalidation_targets_actual_release_artifacts(self):
        # Real dry-run evidence found zero files with Cargo's default host/debug
        # scope and seven guest/release files with these explicit selectors.
        guest = evidence.clean_command("guest", "guest/Cargo.toml", self.root, ["sparq-core"])
        self.assertIn("--release", guest)
        self.assertEqual(guest[guest.index("--target") + 1], "riscv32im-risc0-zkvm-elf")
        self.assertEqual(guest[-2:], ["-p", "sparq-core"])
        host = evidence.clean_command("host", "Cargo.toml", self.root, ["sparq-core"])
        self.assertNotIn("--release", host)
        with self.assertRaisesRegex(ValueError, "unknown package"):
            evidence.clean_command("ambiguous", "Cargo.toml", self.root, [])
        # zkp-14.5: the V5 guest invalidates its own guest/release artifacts.
        authrdf = evidence.clean_command("authrdf-guest", evidence.AUTHRDF_GUEST_MANIFEST,
                                         self.root, ["sparq-canon"])
        self.assertEqual(authrdf[authrdf.index("--target") + 1], "riscv32im-risc0-zkvm-elf")
        self.assertIn("--release", authrdf)

    def test_each_guest_workspace_has_its_own_rebuild_scope_and_target(self):
        scopes = {label: (manifest, subdirectory) for label, manifest, subdirectory in evidence.REBUILD_SCOPES}
        self.assertEqual(scopes, {
            "host": ("zk/sparql-evaluator/Cargo.toml", None),
            "guest": ("zk/sparql-evaluator/methods/guest/Cargo.toml", "sparq-exact-guest"),
            "authrdf-guest": ("zk/sparql-evaluator/methods/guest-authrdf/Cargo.toml", "sparq-authrdf-guest"),
        })
        self.assertEqual(set(scopes) - {"host"}, evidence.GUEST_LABELS)
        # The target subdirectories are the package names methods/build.rs builds into.
        root = Path(__file__).parents[2]
        build = (root / "zk/sparql-evaluator/methods/build.rs").read_text()
        for label in evidence.GUEST_LABELS:
            manifest, package = scopes[label]
            self.assertIn(f'package: "{package}"', build)
            self.assertIn(f'dir: "{Path(manifest).parent.name}"', build)
            self.assertEqual(tomllib.loads((root / manifest).read_text())["package"]["name"], package)

    def test_guest_pins_are_named_per_guest_and_must_be_distinct(self):
        other = {"sha256": [7] * 32, "image_id": [9] * 8}
        self.assertEqual(evidence.guest_pins(self.pin, other),
                         {"sparq-exact-guest": self.pin, "sparq-authrdf-guest": other})
        for same in [self.pin, other | {"sha256": self.pin["sha256"]}, other | {"image_id": self.pin["image_id"]}]:
            with self.subTest(same=same):
                with self.assertRaisesRegex(ValueError, "not distinct"):
                    evidence.guest_pins(self.pin, same)

    def test_campaign_exports_and_confirms_both_guests_without_v5_receipts(self):
        runner = (Path(__file__).parents[2] / "scripts/ci_exact_evaluator_evidence.py").read_text()
        for step in ["export-authrdf-guest", "confirm-authrdf-guest", "native-authrdf",
                     "native-authrdf-host", "lint-authrdf"]:
            self.assertIn(f'"{step}"', runner)
        self.assertIn('"--example", "export_authrdf_guest"', runner)
        self.assertIn("creates no V5 receipt", evidence.AUTHRDF_SCOPE)

    # zkp-14.6: feature and target selection of the added commands.
    @staticmethod
    def targets(argv):
        return [argv[index + 1] for index, arg in enumerate(argv) if arg == "--test"]

    def test_vcq_native_commands_select_every_feature_gated_target(self):
        commands = dict(evidence.VCQ_COMMANDS)
        self.assertEqual(len(commands), len(evidence.VCQ_COMMANDS), "unique command names")
        expected = {
            "native-vcq": ("vcq", ["vcq_adapter"]),
            "native-vcq-authenticated": ("vcq-authenticated", [
                "vcq_authenticated", "vcq_authenticated_genuine", "vcq_adapter"]),
        }
        root = Path(__file__).parents[2]
        host = root / "zk/sparql-evaluator/host"
        features = tomllib.loads((host / "Cargo.toml").read_text())["features"]
        # Default-off; the adapter feature combines exactly the two existing ones.
        self.assertEqual(features["default"], [])
        self.assertEqual(features["vcq-authenticated"], ["vcq", "authenticated-rdf"])
        for name, (feature, targets) in expected.items():
            with self.subTest(name=name):
                argv = commands[name]
                self.assertEqual(argv[:4], ["-p", "sparq-proved-evaluator", "--features", feature])
                self.assertIn("--lib", argv)  # module unit tests, including vcq_authenticated
                self.assertEqual(self.targets(argv), targets)
                # No filter, skip or ignored selection on a native command.
                self.assertNotIn("--", argv)
                self.assertNotIn("--ignored", argv)
                self.assertNotIn("--skip", argv)
                for target in targets:
                    # A file gated on another feature would compile to zero tests.
                    source = (host / "tests" / f"{target}.rs").read_text()
                    gate = re.search(r'#!\[cfg\(feature = "([^"]+)"\)\]', source).group(1)
                    self.assertIn(gate, {feature} | set(features[feature]), target)
        self.assertIn("mod vcq_authenticated;", (host / "src/lib.rs").read_text())
        genuine = (host / "tests/vcq_authenticated_genuine.rs").read_text()
        for test in ["genuine_job_rejects_unknown_fields_and_invalid_seeds",
                     "genuine_job_case_selection_is_bounded_known_and_distinct"]:
            self.assertIn(f"#[test]\nfn {test}()", genuine, "non-ignored parser test")

    def test_vcq_authenticated_lint_covers_all_targets_with_warnings_denied(self):
        argv = dict(evidence.VCQ_COMMANDS)["lint-vcq-authenticated"]
        self.assertEqual(argv, ["--workspace", "--all-targets", "--features",
                                "sparq-proved-evaluator/vcq-authenticated", "--", "-D", "warnings"])
        runner = (Path(__file__).parents[2] / "scripts/ci_exact_evaluator_evidence.py").read_text()
        self.assertIn('if name.startswith("lint-") else test', runner)
        self.assertEqual([name for name, _ in evidence.VCQ_COMMANDS if name.startswith("lint-")],
                         ["lint-vcq-authenticated"])

    def test_only_the_direct_v5_executor_target_runs_ignored_tests(self):
        commands = dict(evidence.VCQ_COMMANDS)
        ignored = [name for name, argv in evidence.VCQ_COMMANDS if "--ignored" in argv]
        self.assertEqual(ignored, ["actual-authrdf-direct-execution"])
        argv = commands["actual-authrdf-direct-execution"]
        self.assertEqual(argv[:4], ["-p", "sparq-proved-evaluator", "--features", "authenticated-rdf"])
        self.assertEqual(self.targets(argv), ["actual_authenticated_rdf"])
        self.assertEqual(argv[argv.index("--") + 1:], ["--ignored", "--nocapture", "--test-threads=1"])
        source = (Path(__file__).parents[2]
                  / "zk/sparql-evaluator/host/tests/actual_authenticated_rdf.rs").read_text()
        reasons = re.findall(r'#\[ignore = "([^"]+)"\]', source)
        self.assertEqual(len(reasons), 4)
        for reason in reasons:
            self.assertIn("creates no receipt", reason)
        # Executor only: no prover, presentation, proof job or receipt export.
        for forbidden in [".prove(", "prove_with_artifact", "Presentation", "PROOF_JOB",
                          "SPARQ_EVALUATOR_EVIDENCE_DIR", "support/evidence.rs"]:
            self.assertNotIn(forbidden, source)
        runner = (Path(__file__).parents[2] / "scripts/ci_exact_evaluator_evidence.py").read_text()
        self.assertEqual(runner.count('"--ignored"'), 1)
        # Genuine proof drivers are never selected; the V5 adapter driver's file
        # runs only its non-ignored native tests under native-vcq-authenticated.
        for driver in ["authenticated_rdf_genuine", "vcq_genuine", "actual_engine_replay"]:
            self.assertNotIn(f'"{driver}"', runner)

    def test_v5_command_scopes_distinguish_native_direct_execution_and_zero_receipts(self):
        scopes = evidence.AUTHRDF_COMMAND_SCOPES
        runner = (Path(__file__).parents[2] / "scripts/ci_exact_evaluator_evidence.py").read_text()
        zkp_14_5 = {"export-authrdf-guest", "confirm-authrdf-guest", "native-authrdf",
                     "native-authrdf-host", "lint-authrdf"}
        self.assertEqual(set(scopes), zkp_14_5 | {name for name, _ in evidence.VCQ_COMMANDS})
        for name in scopes:
            # Named both in the scope table and where the command runs.
            self.assertGreaterEqual(runner.count(f'"{name}"'), 2, name)
        self.assertEqual([name for name, scope in scopes.items() if scope == "direct-sdk-execution"],
                         ["actual-authrdf-direct-execution"])
        for phrase in ["Native:", "Direct SDK execution:", "without proving", "zero V5 receipts",
                       "counts no direct execution as a receipt", "runs no genuine V5 proof driver"]:
            self.assertIn(phrase, evidence.AUTHRDF_SCOPE)
        self.assertIn('"authrdf_command_scopes": AUTHRDF_COMMAND_SCOPES, "authrdf_receipts": 0', runner)
        # The receipt collector admits exactly the V1-V3 fixtures; a V5 file is rejected.
        _, expected = evidence.active_profile(Path(__file__).parents[2])
        self.assertFalse({name for name in expected if not name.startswith(("v1-", "v2-", "v3-"))})
        (self.root / "receipts").mkdir()
        for name in expected | {"v5-direct-execution"}:
            (self.root / "receipts" / f"{name}.json").write_text("{}")
        with self.assertRaisesRegex(ValueError, "missing or unexpected"):
            evidence.receipts(self.root, self.pin, expected)

    def test_workflow_requires_export_and_upload_of_actual_evidence(self):
        root = Path(__file__).parents[2]
        workflow = (root / ".github/workflows/zk-exact-evaluator.yml").read_text()
        self.assertIn("python3 scripts/ci_exact_evaluator_evidence.py", workflow)
        self.assertIn("if-no-files-found: error", workflow)
        self.assertIn("actions/upload-artifact@043fb46d1a93c77aae656e7c1c64a875d1fc6a0a", workflow)
        runner = (root / "scripts/ci_exact_evaluator_evidence.py").read_text()
        self.assertIn('"--nocapture", "--test-threads=1"', runner)
        # zkp-14.6: once, for the direct V5 executor target only (see above).
        self.assertEqual(runner.count('"--ignored"'), 1)


if __name__ == "__main__":
    unittest.main()
