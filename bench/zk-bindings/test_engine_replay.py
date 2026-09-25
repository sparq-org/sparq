"""[GPT-6] Pure controller/source controls; no engine, Cargo or prover processes."""
import copy
import hashlib
import json
from pathlib import Path
import tempfile
from types import SimpleNamespace
import unittest
from unittest import mock

import engine_replay as replay


class EngineReplayTests(unittest.TestCase):
    def setUp(self):
        self.matrix = json.loads((replay.HERE / "engine-matrix.json").read_text())
        self.plan = replay.jobs(self.matrix)
        self.registry = replay.ROOT / "bench/differential-divergences.json"

    def record(self, job, status="agreement"):
        return {"schema": "sparq.engine-seed-replay.v1", **{k: job[k] for k in ("seed", "category", "storage")},
            "input": {"query": "ASK {}", "dataset": "", "format": "turtle", "named_graph_catalog": []},
            "observed_ntriples": [], "native_result": {"kind": "ask", "value": True},
            "reference_result": {"kind": "ask", "value": True}, "native_error": None, "reference_error": None,
            "comparison": {"status": status}, "divergence_registry": {"source": str(self.registry)},
            "proof_count": 0, "verified_proof_count": 0,
            "proof_bridge": {"status": "not-prepared", "backend": None, "authority": None,
                "public_statement_sha256": None, "private_witness_sha256": None, "reuse_allowed": False}}

    def test_exact_denominator_and_unchanged_original_generator(self):
        self.assertEqual(len(self.plan), self.matrix["configured_native_cells"])
        source = (replay.ROOT / "crates/sparq-bench/src/fuzz.rs").read_text()
        addition = "\n// [GPT-6] Structured replay reuses this generator and these independent comparators.\npub(crate) mod replay;\n"
        self.assertEqual(source.count(addition), 1)
        self.assertEqual(hashlib.sha256(source.replace(addition, "").encode()).hexdigest(), self.matrix["original_generator_sha256"])
        self.assertEqual(replay.digest(self.registry), self.matrix["original_registry_sha256"])

    def test_absent_duplicate_and_missing_variant_fail(self):
        rows = [{"id": j["id"], "status": "agreement", "record": self.record(j)} for j in self.plan]
        self.assertTrue(replay.summarize(self.plan, rows)["complete_oracle_agreement"])
        for bad in (rows[:-1], rows + rows[:1]):
            with self.assertRaises(ValueError):
                replay.summarize(self.plan, bad)
        broken = copy.deepcopy(self.matrix)
        broken["storage"].pop()
        with self.assertRaises(ValueError):
            replay.jobs(broken)
        broken = copy.deepcopy(self.matrix)
        broken["profiles"].pop()
        with self.assertRaisesRegex(ValueError, "denominator"):
            replay.jobs(broken)

    def test_nonagreement_and_unrelated_errors_never_pass(self):
        job = self.plan[0]
        for state in replay.GAPS | {"mismatch"}:
            record = self.record(job, state)
            self.assertEqual(replay.validate_record(job, record, self.registry), state)
            self.assertFalse(replay.summarize([job], [{"id":job["id"], "status":state, "record":record}])["complete_oracle_agreement"])
        record = self.record(job)
        record["native_error"] = "parse failure is not capacity"
        self.assertEqual(replay.validate_record(job, record, self.registry), "unclassified-execution")

    def test_normalized_agreement_does_not_authorize_raw_identity_or_reuse(self):
        plan = self.plan[:2]
        rows = [{"id": j["id"], "status": "agreement", "record": self.record(j)} for j in plan]
        rows[1]["record"]["native_result"] = {"kind":"ask", "value":False}
        report = replay.summarize(plan, rows)
        self.assertFalse(report["complete_raw_identity"])
        self.assertFalse(report["proof_reuse_authorized"])
        forged = self.record(plan[0])
        forged["proof_bridge"]["reuse_allowed"] = True
        with self.assertRaisesRegex(ValueError, "proof reuse"):
            replay.validate_record(plan[0], forged, self.registry)
        forged = self.record(plan[0]); forged["proof_count"] = 1
        with self.assertRaisesRegex(ValueError, "native counted"):
            replay.validate_record(plan[0], forged, self.registry)

    def test_changed_input_is_not_variant_agreement(self):
        plan = self.plan[:2]
        rows = [{"id": j["id"], "status": "agreement", "record": self.record(j)} for j in plan]
        rows[1]["record"]["input"]["query"] = "ASK { ?s ?p ?o }"
        self.assertFalse(replay.summarize(plan, rows)["complete_oracle_agreement"])

    def test_execution_retains_partial_outcomes_without_launching_engine(self):
        matrix = copy.deepcopy(self.matrix)
        matrix.update(profiles=["shipped"], categories=["bgp"], seeds=[0], configured_native_cells=6)
        binary = Path(__file__).resolve()
        accepted = {"shipped": {"binary":str(binary), "sha256":replay.digest(binary)}}

        def fake_run(command, **kwargs):
            output = Path(command[-1]); output.mkdir()
            job = {"seed":int(command[2]), "category":command[3], "storage":command[4]}
            record = self.record(job)
            if job["storage"] == "fork":
                raise OSError("unavailable storage process")
            if job["storage"] == "compressed":
                record["comparison"] = {"status":"no-agreement", "reason":"arbitrary-window-row-choice"}
            (output / "record.json").write_text(json.dumps(record))
            (output / "query.rq").write_text(record["input"]["query"])
            (output / "data.ttl").write_text(record["input"]["dataset"])
            return SimpleNamespace(returncode=0)

        with tempfile.TemporaryDirectory() as temp, mock.patch.object(replay, "accepted_binaries", return_value=accepted), mock.patch.object(replay.subprocess, "run", side_effect=fake_run):
            output = Path(temp) / "out"
            report = replay.execute(matrix, {}, output)
            self.assertEqual(report["recorded_native_cells"], 6)
            self.assertEqual(report["comparisons"], {"agreement":4, "infrastructure":1, "no-agreement":1})
            self.assertFalse(report["complete_oracle_agreement"])
            self.assertFalse(report["complete_raw_identity"])
            self.assertEqual(len((output / "outcomes.jsonl").read_text().splitlines()), 6)


if __name__ == "__main__":
    unittest.main()
