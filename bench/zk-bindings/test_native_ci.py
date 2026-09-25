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


class NativeBuildRecordTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        package = self.root / "zk/native-composition"
        source = package / "src/bin/native-bindings.rs"
        source.parent.mkdir(parents=True)
        source.write_text("// Mock compiler input; no Rust compilation in this test.\n")
        manifest = package / "Cargo.toml"
        manifest.write_text('[package]\nname="sparq-native-composition-spike"\nversion="0.0.0"\n')
        self.binary = self.root / "native-bindings"
        self.binary.write_bytes(b"MOCK-EXECUTABLE-NEVER-RUN")
        self.other = self.root / "wrong-target"
        self.other.write_bytes(b"OTHER-MOCK-EXECUTABLE-NEVER-RUN")
        package_id = "path+file://" + str(package) + "#sparq-native-composition-spike@0.0.0"
        target = {"name": "native-bindings", "kind": ["bin"], "src_path": str(source)}
        self.metadata = {"version": 1, "workspace_root": str(package), "workspace_members": [package_id],
                         "packages": [{"id": package_id, "name": "sparq-native-composition-spike",
                                       "version": "0.0.0", "manifest_path": str(manifest), "targets": [target]}]}
        self.artifact = {"reason": "compiler-artifact", "package_id": package_id,
                         "manifest_path": str(manifest), "target": copy.deepcopy(target),
                         "profile": {"test": False, "opt_level": "3"}, "features": ["native-binding"],
                         "executable": str(self.binary), "fresh": False}

    def check(self, events=None, metadata=None):
        event_path, metadata_path = self.root / "cargo.jsonl", self.root / "metadata.json"
        event_path.write_bytes(b"".join(encoded(e) for e in (
            events if events is not None else [self.artifact, {"reason": "build-finished", "success": True}])))
        metadata_path.write_bytes(encoded(self.metadata if metadata is None else metadata))
        return native_ci.check_build_artifact(self.root, self.binary, event_path, metadata_path)

    def test_exact_package_target_and_supplied_binary_match(self):
        for fresh in (False, True):
            self.artifact["fresh"] = fresh
            result = self.check()
            self.assertEqual(result["executable_sha256"], hashlib.sha256(self.binary.read_bytes()).hexdigest())
            self.assertEqual(result["cargo_fresh"], fresh)
            self.assertIs(result["independent_or_signed_build_attestation"], False)

    def test_missing_ambiguous_failed_wrong_package_target_or_binary_rejected(self):
        done = {"reason": "build-finished", "success": True}
        cases = [[], [done], [self.artifact], [self.artifact, done, done],
                 [self.artifact, self.artifact, done], [self.artifact, {"reason": "build-finished", "success": False}]]
        for field, value in [("package_id", "another-package"), ("manifest_path", str(self.other)),
                             ("target", self.artifact["target"] | {"name": "native-rdf"}),
                             ("target", self.artifact["target"] | {"kind": ["lib"]}),
                             ("target", self.artifact["target"] | {"src_path": str(self.other)}),
                             ("profile", {"test": True}), ("features", []),
                             ("executable", str(self.other)), ("executable", None)]:
            cases.append([self.artifact | {field: value}, done])
        for events in cases:
            with self.subTest(events=events), self.assertRaises(ValueError):
                self.check(events=events)

    def test_metadata_must_identify_the_current_detached_package(self):
        cases = [self.metadata | {"version": 2}, self.metadata | {"workspace_members": []},
                 self.metadata | {"workspace_root": str(self.root)}, self.metadata | {"packages": []},
                 self.metadata | {"packages": self.metadata["packages"] * 2}]
        for field, value in [("name", "another-package"), ("version", "9.9.9"),
                             ("manifest_path", str(self.other)), ("targets", [])]:
            cases.append(self.metadata | {"packages": [self.metadata["packages"][0] | {field: value}]})
        for metadata in cases:
            with self.subTest(metadata=metadata), self.assertRaises(ValueError):
                self.check(metadata=metadata)


if __name__ == "__main__":
    unittest.main()
