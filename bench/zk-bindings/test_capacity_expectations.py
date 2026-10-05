"""[GPT-6] Each unchanged capacity fixture must fail for its intended reason."""
import hashlib
import json
from pathlib import Path
import tempfile
import unittest

from corpus import capacity_expectation, digest, import_regressions, jobs_for
from run import check_outcome

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]


class CapacityExpectations(unittest.TestCase):
    def test_all_43_sources_and_exact_causes_are_bound(self):
        registry = json.loads((HERE / "capacity-expectations.json").read_text())
        classes = json.loads((HERE / "rejections.json").read_text())
        counts = {}
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory)
            for source in registry["sources"]:
                raw = (ROOT / source["path"]).read_bytes()
                self.assertEqual(hashlib.sha256(raw).hexdigest(), source["source_sha256"])
                originals = {c["id"]: c for c in json.loads(raw)[source["field"]]}
                for entry in source["cases"]:
                    original = originals[entry["id"]]
                    self.assertEqual(digest(original), entry["original_fixture_sha256"])
                    required = entry["expected_rejection"]
                    self.assertEqual(capacity_expectation(original), required)
                    key = required.get("cause", required.get("diagnostic"))
                    counts[key] = counts.get(key, 0) + 1
                    actual = (classes["typed_causes"][key] | {"cause": key}
                              if "cause" in required else required)
                    job = {"id":entry["id"], "case_sha256":entry["original_fixture_sha256"],
                           "backend":"exact_v3", "tier":"native", "operation":"admission",
                           "expected_accept":False, "expected_rejection":required}
                    outcome = {"schema":"sparq.proof-binding-outcome.v1", "job_id":job["id"],
                               "case_sha256":job["case_sha256"], "backend":"exact_v3", "tier":"native",
                               "observed":"rejected", "stage":"native", "proof_count":0,
                               "verified_count":0, "artifacts":[], "controls":[],
                               "error_class":"capacity", "rejection":actual}
                    check_outcome(job, outcome, output)
                    for cause, known in classes["typed_causes"].items():
                        if cause == required.get("cause"):
                            continue
                        wrong = known | {"cause": cause}
                        with self.subTest(case=job["id"], wrong=cause), self.assertRaises(ValueError):
                            check_outcome(job, outcome | {"error_class":wrong["category"], "rejection":wrong}, output)
                    with self.assertRaisesRegex(ValueError, "exact cause"):
                        check_outcome(job | {"expected_rejection":{"category":"capacity"}}, outcome, output)
        self.assertEqual(counts, {"numeric_representation":26, "temporal_year":9,
                                 "budget_rows":1, "temporal year capacity":7})

    def test_unknown_or_changed_capacity_fixture_is_not_guessed(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "capacity.json"
            path.write_text(json.dumps({"expectation_kind":"implementation_capacity", "cases":[
                {"id":"integer-cast-outside-range", "query":"ASK {}"}]}))
            imported = import_regressions(path)
            self.assertIsNone(capacity_expectation(imported[0]["original_fixture"]))
            jobs, classification = jobs_for(imported[0], "exact_v3", "native")
            self.assertEqual(jobs, [])
            self.assertEqual(classification["status"], "requires_rejection_classification")


if __name__ == "__main__":
    unittest.main()
