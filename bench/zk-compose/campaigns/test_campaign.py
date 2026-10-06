#!/usr/bin/env python3
"""[GPT-6] Hermetic campaign protocol checks; stub outputs are never proof evidence."""
import copy
import json
from pathlib import Path
import tempfile
import os
import sys
import unittest
from unittest.mock import patch

import campaign as c


class CampaignTests(unittest.TestCase):
    def profile(self):
        return c.load(Path(__file__).with_name("profile.json"))

    def test_materialized_input_is_deterministic_complete_and_bounded(self):
        for scale in [2, 8, 32, 128]:
            organization = c.organization("0123456789abcdef", scale)
            self.assertEqual(len(organization["nquads"].splitlines()), scale * 2)
            self.assertLessEqual(len(organization["nquads"].encode()) + sum(map(len, organization["named_graphs"])), 65536)
            self.assertEqual(organization["expected"]["Select"]["rows"][-1][1], '"0"^^<http://www.w3.org/2001/XMLSchema#integer>')
        for scale in [3, 8, 32, 128]:
            self.assertEqual(len(c.wallet("0123456789abcdef", scale)["credentials"]), scale)
        profile = self.profile()
        for field in ["wallet_scales", "organization_scales"]:
            invalid = dict(profile, **{field: [129]})
            with self.assertRaises(ValueError):
                c.build_plan(invalid)
        plan, inputs = c.build_plan(profile)
        self.assertEqual((plan, inputs), c.build_plan(profile))
        self.assertNotEqual(inputs, c.build_plan(dict(profile, seed="fedcba9876543210"))[1])

    def test_interleaving_and_paired_inputs_do_not_change_contract(self):
        plan, inputs = c.build_plan(self.profile())
        jobs = plan["jobs"]
        self.assertEqual(len(jobs), 96)
        for round_index in range(4):
            pair = [j for j in jobs if j["fixture"] == "wallet-0003" and j["round"] == round_index and not j["unsupported"]]
            self.assertEqual([j["mode"] for j in pair], ["first_success", "optimize"] if round_index % 2 == 0 else ["optimize", "first_success"])
            manifests = [c.manifest(j, inputs[j["fixture"]], "0" * 32) for j in pair]
            self.assertEqual(manifests[0]["wallet"], manifests[1]["wallet"])
            self.assertEqual(manifests[0]["contract"], manifests[1]["contract"])
            self.assertNotEqual(manifests[0]["nonce_id"], manifests[1]["nonce_id"])
        measured = [j for j in jobs if j["role"] == "measurement"]
        self.assertEqual(len(measured), 72)
        nonces = []
        for job in jobs:
            if job["unsupported"]:
                continue
            m = c.manifest(job, inputs[job["fixture"]], "0" * 32)
            nonces.append((job["backend"], m.get("nonce_id", m.get("run_id"))))
        self.assertEqual(len(nonces), len(set(nonces)))

    def test_generated_files_schedule_and_unknown_config_cannot_drift(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            profile = root / "profile.json"
            c.save(profile, self.profile())
            c.generate(profile, root / "generated")
            c.read_generated(root / "generated")
            path = root / "generated/inputs/wallet-0003.json"
            path.write_bytes(path.read_bytes() + b" ")
            with self.assertRaisesRegex(ValueError, "materialized fixture changed"):
                c.read_generated(root / "generated")
        invalid = dict(self.profile(), hidden_query="ASK {}")
        with self.assertRaises(ValueError):
            c.build_plan(invalid)
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "duplicate.json"
            path.write_text('{"x":1,"x":2}')
            with self.assertRaisesRegex(ValueError, "duplicate JSON key"):
                c.load(path)
            path.write_text('{"x":1e1000}')
            with self.assertRaisesRegex(ValueError, "non-finite"):
                c.load(path)

    def test_missing_tools_are_failed_samples_and_unsupported_is_explicit(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            profile = dict(self.profile(), wallet_scales=[3], organization_scales=[], warmups=0, repetitions=1)
            c.save(root / "profile.json", profile)
            c.generate(root / "profile.json", root / "generated")
            self.assertFalse(c.run(root / "generated", root / "output", {}, "1" * 32))
            report = c.load(root / "output/report.json")
            self.assertEqual(report["counts"], {"failure": 2, "unsupported": 1})
            self.assertTrue(report["schedule_complete"])
            self.assertTrue(all(not record["subprocess_started"] for record in report["records"]))
            self.assertIsNone(report["cross_backend_speedup"])

    def test_zero_exit_with_malformed_report_does_not_erase_later_records(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            profile = dict(self.profile(), wallet_scales=[3], organization_scales=[], warmups=0, repetitions=1, include_unsupported=False)
            c.save(root / "profile.json", profile)
            c.generate(root / "profile.json", root / "generated")
            tools = {"selected": Path(sys.executable)}
            for name in ["nargo", "bb"]:
                tools[name] = root / name
                tools[name].write_text("protocol stub, never executed")
                tools[name].chmod(0o700)
            def fake_invoke(argv, cwd, stdout, stderr, timeout, env):
                # A zero exit is not evidence of a proof or even a valid report.
                output = Path(argv[-1])
                output.mkdir()
                (output / "report.json").write_text("[]")
                stdout.touch()
                stderr.touch()
                return {"status": "exited", "exit_code": 0, "inclusive_process_ns": 1}
            with patch.object(c, "invoke", fake_invoke):
                self.assertFalse(c.run(root / "generated", root / "output", tools, "1" * 32))
            report = c.load(root / "output/report.json")
            self.assertEqual(report["counts"], {"failure": 2})
            self.assertTrue(report["schedule_complete"])

    def test_timeout_and_exit_failure_are_not_successful_samples(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            timed = c.invoke([sys.executable, "-c", "import time; time.sleep(10)"], root,
                             root / "stdout", root / "stderr", 0.05, os.environ.copy())
            self.assertEqual(timed["status"], "timeout")
            self.assertNotEqual(timed["exit_code"], 0)
            failed = c.invoke([sys.executable, "-c", "raise SystemExit(7)"], root,
                              root / "stdout2", root / "stderr2", 2, os.environ.copy())
            self.assertEqual(failed["status"], "exited")
            self.assertEqual(failed["exit_code"], 7)

    def test_output_capacity_is_checked_after_successful_exit(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            def spawn(argv, **kwargs):
                class Completed:
                    def wait(self, timeout=None):
                        kwargs["stdout"].truncate(16 * 1024 * 1024 + 1)
                        kwargs["stdout"].flush()
                        return 0
                return Completed()
            # A sparse protocol fixture models output written between poll and exit.
            # No adapter or prover is executed by this deterministic test.
            with patch.object(c.subprocess, "Popen", spawn):
                observed = c.invoke(["protocol-stub"], root, root / "stdout", root / "stderr", 2, {})
            self.assertEqual(observed["status"], "output_capacity")
            self.assertEqual(observed["exit_code"], 0)

    def test_spawn_failure_is_distinct_from_io_failure_after_spawn(self):
        for error, started in [(OSError("spawn refused"), False), (c.StartedProcessError("post-spawn host I/O"), True)]:
            with tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary)
                profile = dict(self.profile(), wallet_scales=[3], organization_scales=[], warmups=0, repetitions=1, include_unsupported=False)
                c.save(root / "profile.json", profile)
                c.generate(root / "profile.json", root / "generated")
                tools = {"selected": Path(sys.executable)}
                for name in ["nargo", "bb"]:
                    tools[name] = root / name
                    tools[name].touch(mode=0o700)
                with patch.object(c, "invoke", side_effect=error):
                    self.assertFalse(c.run(root / "generated", root / "output", tools, "1" * 32))
                records = c.load(root / "output/report.json")["records"]
                self.assertEqual(len(records), 2)
                self.assertTrue(all(r["status"] == "failure" and r["subprocess_started"] is started for r in records))

    def test_exact_report_success_requires_bound_answer_authority_and_controls(self):
        fixture = c.organization("0123456789abcdef", 2)
        job = {"ordinal": 0, "backend": "exact", "mode": "holder_declared"}
        manifest = c.manifest(job, fixture, "0" * 32)
        controls = ["expected_query_bytes", "expected_nonce", "expected_authority", "expected_dataset_root", "consumed_nonce_replay", "authenticated_returned_result", "authenticated_dataset_root", "succinct_seal_bytes"]
        # Explicitly synthetic protocol record. No test returns this as measured proof evidence.
        report = {"schema": "sparq.exact-evaluator-experiment.v1", "complete": True,
                  "run_id": manifest["run_id"], "manifest_sha256": c.digest(c.encoded(manifest)),
                  "artifact_acceptance_controls_passed": ["artifact_bytes_digest", "independent_pin_digest", "independent_pin_image_id"],
                  "samples": [{"status": "proved_and_independently_verified", "receipt_kind": "Succinct",
                               "result": fixture["expected"], "semantic_contract": {"authority_mode": "holder_declared"},
                               "semantic_contract_sha256": "synthetic-protocol-only", "typed_acceptance_controls_passed": controls}]}
        c.inspect_report(report, job, manifest)
        for field, value in [("result", {"Ask": False}), ("receipt_kind", "Fake"), ("typed_acceptance_controls_passed", controls[:-1]), ("semantic_contract", {"authority_mode": "verifier_agreed"})]:
            changed = copy.deepcopy(report)
            changed["samples"][0][field] = value
            with self.assertRaises(ValueError):
                c.inspect_report(changed, job, manifest)
        changed = dict(report, manifest_sha256="0" * 64)
        with self.assertRaises(ValueError):
            c.inspect_report(changed, job, manifest)


if __name__ == "__main__":
    unittest.main()
