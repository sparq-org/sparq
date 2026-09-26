"""[GPT-6] Pure-Python inventory checks; no compiler or evaluator invocation."""
import copy
from pathlib import Path
import unittest

from corpus import digest, load
from exact_ci import check_report, original_plan
from run import file_hash


class OriginalReplayTests(unittest.TestCase):
    def test_all_original_objects_and_both_authorities_are_configured(self):
        manifest = original_plan()
        self.assertEqual(manifest["case_count"], 319)
        self.assertEqual(len(manifest["jobs"]), 1914)
        self.assertEqual(manifest["classifications"], [])
        self.assertEqual({j["authority"] for j in manifest["jobs"]},
                         {"verifier_agreed", "holder_declared"})
        ids = {c["id"] for c in manifest["cases"]}
        # Former rejection IDs remain as their reviewed positive original cases.
        self.assertIn("exists-minus-disjoint-domain-after-substitution", ids)
        self.assertIn("nullable-path-alternative-absent-constant", ids)
        for case in manifest["cases"]:
            self.assertEqual(case["query"], case["original_fixture"]["query"])

    def test_projection_registry_is_hash_pinned_and_applied_exactly(self):
        # [OPUS-5.5] Inventory pins the reviewed projection registry bytes.
        root = Path(__file__).resolve().parents[2]
        pin = load(root / "bench/zk-bindings/exact-originals.json")["projection_expectations"]
        self.assertEqual(pin["path"], "bench/zk-bindings/projection-expectations.json")
        self.assertEqual(pin["sha256"], file_hash(root / pin["path"]))
        manifest = original_plan()
        applied = {c["id"]:c for c in manifest["cases"]
                   if c["oracle"].get("projection", {}).get("kind") == "reviewed_projection_override"}
        self.assertEqual(set(applied), {"date-end-of-year", "date-end-of-leap-month", "date-end-before-leap-day"})
        for case in applied.values():
            self.assertEqual(case["oracle"]["projection"]["registry_sha256"], pin["sha256"])
            self.assertEqual(case["expected"]["Select"]["variables"], ["y", "m", "day", "h"])
            self.assertEqual(case["expected"]["Select"]["rows"], case["original_fixture"]["expected_rows"])
            self.assertEqual(case["query"], case["original_fixture"]["query"])

    def test_missing_duplicate_wrong_counts_and_invented_proofs_fail(self):
        manifest = original_plan()
        report = {"passed":True, "complete_declared_domain":True, "coverage_gaps":[],
                  "plan_sha256":digest(manifest), "totals":{},
                  "records":[{"job_id":j["id"], "status":"passed"} for j in manifest["jobs"]]}
        for backend, expected in manifest["totals"].items():
            report["totals"][backend] = {
                key:expected["configured_jobs"] for key in
                ("configured_jobs", "shard_jobs", "executed_jobs", "passed_jobs")}
            report["totals"][backend].update(genuine_proofs=0, verified_proofs=0,
                negative_stages={"native":expected["negative_bindings_or_admissions"]})
        check_report(report, manifest)  # Synthetic controller test, not evaluation.
        for change in (lambda r:r["records"].pop(),
                       lambda r:r["records"].append(r["records"][0]),
                       lambda r:r["totals"]["exact_v3"].update(genuine_proofs=1),
                       lambda r:r["totals"]["exact_v1"].update(passed_jobs=637),
                       lambda r:r.update(coverage_gaps=[{"status":"unknown"}])):
            invalid = copy.deepcopy(report)
            change(invalid)
            with self.assertRaises(ValueError):
                check_report(invalid, manifest)

    def test_required_workflow_keeps_real_campaign_and_original_replay(self):
        root = Path(__file__).resolve().parents[2]
        text = (root / ".github/workflows/zk-exact-evaluator.yml").read_text()
        self.assertIn("python3 bench/zk-bindings/exact_ci.py", text)
        self.assertIn("python3 scripts/ci_exact_evaluator_evidence.py", text)
        self.assertIn("${{ runner.temp }}/sparq-exact-originals", text)
        self.assertNotIn("continue-on-error", text)


if __name__ == "__main__":
    unittest.main()
