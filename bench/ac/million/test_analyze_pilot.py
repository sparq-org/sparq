"""[GPT-6] Check failure denominators, incomplete evidence and integrity rejection."""
import hashlib
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest

spec = importlib.util.spec_from_file_location("pilot_analysis", Path(__file__).with_name("analyze-pilot.py"))
pilot = importlib.util.module_from_spec(spec)
spec.loader.exec_module(pilot)


class PilotAnalysisTests(unittest.TestCase):
    def test_failures_and_unsent_remain_in_denominator(self):
        rows = [
            {"record_type": "load-start", "settings": {"requests": "4", "timeout-ms": "5000"}},
            {"record_type": "request", "sequence": 0, "status": 200, "outcome": "ok",
             "scheduled_latency_us": 200000, "http_latency_us": 190000, "server_us": 180000},
            {"record_type": "request", "sequence": 1, "status": None, "outcome": "transport-error",
             "scheduled_latency_us": 5000001, "http_latency_us": 5000000},
            {"record_type": "request", "sequence": 2, "status": None, "outcome": "client-admission-drop",
             "scheduled_latency_us": 9000000},
            {"record_type": "load-complete", "offered": 4, "client_dropped": 1, "elapsed_us": 10000000},
        ]
        result = pilot.summarize_requests(rows)
        self.assertFalse(result["complete_record_sequence"])
        self.assertEqual(result["within_scheduled_deadline_fraction_of_offered"], 0.25)
        self.assertEqual(result["unsuccessful_or_unrecorded"], 3)
        self.assertEqual(result["latency_us"]["successful_complete_body_from_scheduled_arrival"]["p95"], 200000)
        self.assertEqual(result["explicit_timeout_count"], 0)
        self.assertEqual(result["failure_durations_at_least_configured_timeout"], 1)
        self.assertEqual(result["observed_successful_completions_per_second"], 0.1)

    def test_successful_noop_does_not_prove_revocation(self):
        rows = [{"record_type": "load-start", "settings": {"requests": "3"}},
                *[{"record_type": "request", "sequence": i, "outcome": "ok", "status": 200,
                   "operation": "policy-write", **delta} for i, delta in enumerate(
                    [{"policy_triple_delta": 0}, {"policy_triple_delta": 2}, {}])],
                {"record_type": "load-complete", "offered": 3, "client_dropped": 0, "elapsed_us": 3000}]
        coverage = pilot.summarize_requests(rows)["policy_write_coverage"]
        self.assertEqual([coverage[x] for x in ("zero_triple_delta", "nonzero_triple_delta", "unknown_delta")], [1, 1, 1])
        self.assertFalse(coverage["effective_rights_change_verified_by_timing_rows"])

    def test_empty_checkpoint_is_unknown_not_zero_latency(self):
        result = pilot.summarize_requests([])
        self.assertIsNone(result["offered"])
        self.assertIsNone(result["success_fraction"])
        self.assertIsNone(result["latency_us"]["successful_complete_body_from_scheduled_arrival"]["p95"])
        self.assertFalse(result["complete_record_sequence"])

    def test_checksum_detects_corruption_and_blocks_external_paths(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            (root / "sample.txt").write_text("changed")
            expected = hashlib.sha256(b"original").hexdigest()
            (root / "MANIFEST.sha256").write_text(f"{expected}  sample.txt\n{expected}  ../outside.txt\n")
            result = pilot.Inputs(root).verify_manifest()
            self.assertEqual(result["status"], "invalid")
            self.assertEqual({x["error"] for x in result["errors"]},
                             {"checksum-mismatch", "unsafe-or-duplicate-manifest-path"})

    def test_checksummed_bundle_is_still_not_capacity_evidence(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            (root / "DONE").write_text("")
            (root / "source-commit.txt").write_text("source\n")
            (root / "MANIFEST.sha256").write_text("".join(
                f"{hashlib.sha256((root / n).read_bytes()).hexdigest()}  {n}\n"
                for n in ("DONE", "source-commit.txt")))
            result = pilot.analyze(root)
            self.assertEqual(result["artifact_status"], "complete-checksummed")
            self.assertFalse(result["canonical"])
            self.assertFalse(result["protocol_capacity_admitted"])
            self.assertFalse(result["equivalence_admitted"])


if __name__ == "__main__":
    unittest.main()
