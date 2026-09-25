"""[GPT-6] Hermetic protocol mutations only; these tests generate no proofs."""
import copy
import hashlib
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

from corpus import encoded
import native_ci


class NativeCiTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.output = Path(self.tmp.name)
        self.plan = native_ci.native_plan()

    def fixture(self, kind, job=None):
        job = copy.deepcopy(job) if job is not None else copy.deepcopy(next(j for j in self.plan["jobs"] if (
            j["expected_accept"] if kind == "positive" else
            not j["expected_accept"] and bool(j["triples"]) == (kind == "weaker"))))
        result = {"schema": "sparq.proof-binding-outcome.v1", "job_id": job["id"],
                  "case_sha256": job["case_sha256"], "backend": "native_rdf", "tier": "real",
                  "observed": "accepted" if kind == "positive" else "rejected",
                  "stage": "admission" if kind == "empty" else "proof_verifier",
                  "proof_count": 0 if kind == "empty" else 1,
                  "verified_count": int(kind == "positive"),
                  "error_class": {"positive": None, "weaker": "Verification", "empty": "Capacity"}[kind],
                  "controls": copy.deepcopy([native_ci.REPLAY] if kind == "positive" else
                                             native_ci.WEAKER_CONTROLS if kind == "weaker" else []),
                  "artifacts": [], "implementation": {"input_sha256": hashlib.sha256(encoded(job)).hexdigest()}}
        if kind == "empty":
            result["unsupported_reason"] = "existing bounded nonempty signed-graph profile; admission-only refusal, no attempted proof"
            return job, result
        key = [1, 2, 3]  # Protocol mock, never represented as an actual issuer key.
        public = {"nonce": list(hashlib.sha256(b"sparq/native-rdf/binding-job/nonce/v1\0" + encoded(job)).digest()),
                  "issuer_key": key, "support": [[[0, 0]]],
                  "context": {"protocol": list(b"sparq/native-rdf/public-bgp/v1"), "query": job["query"],
                              "rows": [dict(zip(job["variables"], job["rows"][0]))],
                              "semantics": "nonempty-distinct-successful-support", "support": [[[0, 0]]],
                              "roles": [["urn:sparq:proof-binding:synthetic-issuer", key,
                                         "urn:sparq:proof-binding:synthetic-status", 1, 0,
                                         list(hashlib.blake2b(b"\0", digest_size=64).digest())]]}}
        for name, data in [("public.json", encoded(public)),
                           ("proof.bin" if kind == "positive" else "weaker-proof.bin", b"NOT-A-PROOF-UNIT-TEST" + job["id"].encode())]:
            (self.output / name).write_bytes(data)
            result["artifacts"].append({"path": name, "sha256": hashlib.sha256(data).hexdigest()})
        return job, result

    def test_fixed_domain_and_three_distinct_classes(self):
        self.assertEqual(len(self.plan["jobs"]), 576)
        self.assertEqual(len(self.plan["classifications"]), 80)
        for kind, expected in [("positive", "required_accepted"),
                               ("weaker", "weaker_verified_required_rejected"),
                               ("empty", "empty_graph_admission")]:
            with self.subTest(kind=kind):
                job, result = self.fixture(kind)
                self.assertEqual(native_ci.check_native_outcome(job, result, self.output)[0], expected)

    def test_source_domain_cannot_shrink(self):
        altered = copy.deepcopy(self.plan)
        altered["jobs"].pop()
        with patch.object(native_ci, "plan", return_value=altered):
            with self.assertRaisesRegex(ValueError, "reviewed pin"):
                native_ci.native_plan()

    def test_honest_refusal_cannot_replace_genuine_negative(self):
        job, result = self.fixture("weaker")
        result.update(stage="support", error_class="Support", proof_count=0, controls=[], artifacts=[])
        with self.assertRaisesRegex(ValueError, "exactly one actual"):
            native_ci.check_native_outcome(job, result, self.output)

    def test_weaker_and_required_controls_are_both_mandatory(self):
        for control in (1, 2):
            job, result = self.fixture("weaker")
            result["controls"].pop(control)
            with self.assertRaisesRegex(ValueError, "weaker-statement control"):
                native_ci.check_native_outcome(job, result, self.output)

    def test_accepted_weaker_claim_is_not_accepted_required_claim(self):
        job, result = self.fixture("weaker")
        result["verified_count"] = 1
        with self.assertRaisesRegex(ValueError, "exactly one actual"):
            native_ci.check_native_outcome(job, result, self.output)

    def test_replay_and_proof_artifact_are_mandatory(self):
        job, result = self.fixture("positive")
        result["controls"] = []
        with self.assertRaisesRegex(ValueError, "replay"):
            native_ci.check_native_outcome(job, result, self.output)
        job, result = self.fixture("positive")
        (self.output / "proof.bin").write_bytes(b"changed")
        with self.assertRaisesRegex(ValueError, "artifact hash"):
            native_ci.check_native_outcome(job, result, self.output)

    def test_public_context_substitution_rejected_after_rehash(self):
        for field in ("query", "nonce", "issuer_key"):
            job, result = self.fixture("weaker")
            public = native_ci.load(self.output / "public.json")
            if field == "query":
                public["context"]["query"] += " "
            else:
                public[field][0] ^= 1
            data = encoded(public)
            (self.output / "public.json").write_bytes(data)
            result["artifacts"][0]["sha256"] = hashlib.sha256(data).hexdigest()
            with self.assertRaisesRegex(ValueError, "nonce|claim|issuer/status"):
                native_ci.check_native_outcome(job, result, self.output)

    def test_empty_boundary_cannot_claim_proofs_or_unknown_error(self):
        for field, value in [("proof_count", 1), ("error_class", "Unknown"), ("stage", "support")]:
            job, result = self.fixture("empty")
            result[field] = value
            with self.assertRaises(ValueError):
                native_ci.check_native_outcome(job, result, self.output)

    def test_complete_mock_inventory_and_aggregate_tampering(self):
        # Structural protocol fixtures only: no BBS operation is invoked here.
        campaign = self.output / "campaign"
        campaign.mkdir()
        native_ci.controller.write(campaign / "plan.json", self.plan)
        records = []
        root = self.output
        for ordinal, job in enumerate(self.plan["jobs"]):
            directory = campaign / f"{ordinal:06}-{job['id'][:12]}"
            self.output = directory / "adapter"
            self.output.mkdir(parents=True)
            kind = "positive" if job["expected_accept"] else "weaker" if job["triples"] else "empty"
            _, outcome = self.fixture(kind, job)
            record = {"job_id": job["id"], "backend": "native_rdf", "expected_accept": job["expected_accept"],
                      "status": "passed", "input_sha256": hashlib.sha256(encoded(job)).hexdigest(),
                      "outcome": outcome, "inventory": native_ci.controller.check_outcome(job, outcome, self.output)}
            native_ci.controller.write(directory / "job.json", job)
            native_ci.controller.write(directory / "record.json", record)
            native_ci.controller.write(self.output / "outcome.json", outcome)
            records.append(record)
        self.output = root
        report = {"passed": True, "complete_declared_domain": True, "plan_sha256": native_ci.PLAN_SHA256,
                  "coverage_gaps": [], "all_backend_profiles_complete": False, "records": records,
                  "totals": {"native_rdf": self.plan["totals"]["native_rdf"] | {
                      "executed_jobs": 576, "passed_jobs": 576, "genuine_proofs": 540, "verified_proofs": 72,
                      "negative_stages": {"admission": 36, "proof_verifier": 468}}}}
        report_path = campaign / "report.json"
        report_path.write_bytes(encoded(report))
        self.assertEqual(native_ci.verify_campaign(root)["counts"], native_ci.EXPECTED_COUNTS)
        report["totals"]["native_rdf"]["verified_proofs"] = 540
        report_path.write_bytes(encoded(report))
        with self.assertRaisesRegex(ValueError, "aggregate native counts"):
            native_ci.verify_campaign(root)

    def test_missing_records_fail_even_if_report_says_success(self):
        campaign = self.output / "campaign"
        campaign.mkdir()
        (campaign / "plan.json").write_bytes(encoded(self.plan))
        (campaign / "report.json").write_bytes(encoded({
            "passed": True, "complete_declared_domain": True, "plan_sha256": native_ci.PLAN_SHA256,
            "coverage_gaps": [], "all_backend_profiles_complete": False, "records": []}))
        with self.assertRaisesRegex(ValueError, "job record"):
            native_ci.verify_campaign(self.output)


if __name__ == "__main__":
    unittest.main()
