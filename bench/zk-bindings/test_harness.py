"""[GPT-6] Discriminating oracle, coverage and evidence-boundary controls."""
import itertools
import json
from pathlib import Path
import tempfile
import unittest

from corpus import digest, encoded, exhaustive, finite_proof_universe, import_regressions, oracle, plan, sampled, select, tiny_case
from run import check_outcome, equal_result, execute_child, validate_plan
from minimize import minimize


class CorpusTests(unittest.TestCase):
    def test_replay_rejects_changed_query_or_denominator(self):
        multiple = plan([tiny_case(6, "join")], ["noir_unsigned", "noir_signed"], "real")
        validate_plan(json.loads(encoded(multiple)))
        source = plan([tiny_case(6, "join")], ["noir_unsigned"], "real")
        validate_plan(source)
        altered = json.loads(json.dumps(source))
        altered["jobs"][0]["query"] += " "
        with self.assertRaisesRegex(ValueError, "retained corpus"):
            validate_plan(altered)
        altered = json.loads(json.dumps(source))
        altered["jobs"].pop()
        altered["totals"]["noir_unsigned"]["configured_jobs"] -= 1
        with self.assertRaisesRegex(ValueError, "denominators"):
            validate_plan(altered)

    def test_finite_real_domain_includes_every_binding_and_attack(self):
        full = plan(finite_proof_universe(), ["noir_unsigned", "noir_signed"], "real")
        self.assertEqual(len(full["jobs"]), 92)
        for backend in ("noir_unsigned", "noir_signed"):
            jobs = [j for j in full["jobs"] if j["backend"] == backend]
            self.assertEqual(sum(j["expected_accept"] for j in jobs), 4)
            self.assertEqual(sum(j["operation"] == "binding" and not j["expected_accept"] for j in jobs), 32)
            self.assertEqual(sum(j["operation"] == "attack" for j in jobs), 10)
        self.assertEqual(full["classifications"], [])

    def test_exhaustive_denominator_and_all_bindings(self):
        cases = exhaustive()
        self.assertEqual(len(cases), 16 * 7)
        full = plan(cases, ["native_rdf", "exact_v2"], "real", coverage="exhaustive_tiny_v1")
        self.assertEqual(full["totals"]["native_rdf"]["configured_jobs"], 16 * (9 + 27))
        self.assertEqual(full["totals"]["exact_v2"]["configured_jobs"], 16 * 7 * 2)
        self.assertEqual(full["totals"]["native_rdf"]["executed_jobs"], 0)
        jobs = full["jobs"]
        for case in cases:
            if case["template"] in ("scan", "join"):
                actual = [j["rows"][0] for j in jobs if j["case_id"] == case["id"] and
                          j["backend"] == "native_rdf" and j["expected_accept"]]
                self.assertEqual(sorted(actual), sorted(case["expected"]["Select"]["rows"]))
        sharded = [plan(cases, ["exact_v2"], "real", i, 8)["jobs"] for i in range(8)]
        ids = [j["id"] for shard in sharded for j in shard]
        self.assertEqual(len(ids), len(set(ids)))
        self.assertEqual(set(ids), {j["id"] for j in jobs if j["backend"] == "exact_v2"})

    def test_independent_join_and_duplicate_projection(self):
        triples = [["<urn:a>", "<urn:p>", "<urn:b>"], ["<urn:b>", "<urn:p>", "<urn:a>"],
                   ["<urn:a>", "<urn:p>", "<urn:a>"]]
        _, result = oracle(triples, "join")
        self.assertIn(["<urn:a>", "<urn:b>", "<urn:a>"], result["Select"]["rows"])
        self.assertNotIn(["<urn:a>", "<urn:b>", "<urn:b>"], result["Select"]["rows"])
        _, bag = oracle(triples + triples, "projection_bag")
        self.assertEqual(bag["Select"]["rows"].count(["<urn:a>"]), 2)

    def test_global_blank_identity_and_order(self):
        self.assertTrue(equal_result(select(["s", "o"], [["<urn:a>", "<urn:b>"]]),
                                     select(["o", "s"], [["<urn:b>", "<urn:a>"]])))
        a = select(["s", "o"], [["_:a", "_:b"], ["_:b", "_:a"], ["_:a", "_:b"]])
        b = select(["s", "o"], [["_:y", "_:x"], ["_:x", "_:y"], ["_:x", "_:y"]])
        self.assertTrue(equal_result(a, b))
        self.assertFalse(equal_result(a, select(["s", "o"], [["_:x", "_:y"], ["_:z", "_:w"], ["_:x", "_:y"]])))
        self.assertFalse(equal_result(a, select(["s", "o"], b["Select"]["rows"][:2])))
        self.assertFalse(equal_result(select(["x"], [["<urn:a>"], ["<urn:b>"]], True),
                                      select(["x"], [["<urn:b>"], ["<urn:a>"]], True)))
        with self.assertRaisesRegex(ValueError, "capacity"):
            equal_result(select(["x"], [[f"_:a{i}"] for i in range(9)]),
                         select(["x"], [[f"_:b{i}"] for i in range(9)]))

    def test_regression_import_preserves_originals(self):
        source = Path(__file__).resolve().parents[2] / "zk/sparql-evaluator/fixtures/conformance/cases.json"
        original = json.loads(source.read_text())
        imported = import_regressions(source)
        self.assertEqual([c["original_fixture"] for c in imported], original["cases"])
        self.assertEqual([c["id"] for c in imported], [c["id"] for c in original["cases"]])
        self.assertEqual([c["query"] for c in imported], [c["query"] for c in original["cases"]])
        for case in imported:
            if "dataset" in case["original_fixture"]:
                self.assertEqual(case["dataset"]["ntriples"], case["original_fixture"]["dataset"])

    def test_numeric_import_retains_goldens_and_separates_capacity(self):
        source = Path(__file__).resolve().parents[2] / "crates/sparq-engine/tests/fixtures/builtin_edges.json"
        original = json.loads(source.read_text())["cases"]
        imported = import_regressions(source, ["v"])
        self.assertEqual([c["original_fixture"] for c in imported], original)
        self.assertEqual(sum(c["rejection"] for c in imported), 2)
        with self.assertRaisesRegex(ValueError, "projection"):
            import_regressions(source)

    def test_versioned_dataset_positives_retain_original_v1_rejections(self):
        source = Path(__file__).resolve().parents[2] / "zk/sparql-evaluator/fixtures/conformance/cases.json"
        originals = {c["id"]:c for c in json.loads(source.read_text())["cases"]}
        names = {"reject-from-default", "reject-from-named", "reject-named-graph"}
        cases = [c for c in import_regressions(source) if c["id"] in names]
        manifest = plan(cases, ["exact_v1", "exact_v2"], "native")
        self.assertEqual(len(manifest["jobs"]), 12)
        self.assertEqual(manifest["classifications"], [])
        validate_plan(json.loads(encoded(manifest)))
        for case in cases:
            self.assertEqual(case["original_fixture"], originals[case["id"]])
            self.assertEqual(case["query"], originals[case["id"]]["query"])
            self.assertTrue(case["rejection"])
            self.assertIsNone(case["expected"])
        for job in manifest["jobs"]:
            self.assertEqual(job["query"], originals[job["case_id"]]["query"])
            if job["backend"] == "exact_v1":
                self.assertFalse(job["expected_accept"])
                self.assertEqual(job["operation"], "admission")
                self.assertEqual(job["expected_rejection"]["diagnostic"], originals[job["case_id"]]["expected"]["error"])
            else:
                self.assertTrue(job["expected_accept"])
                self.assertEqual(job["operation"], "result")
                self.assertNotIn("expected_rejection", job)
                self.assertEqual(job["expected_result"], select(["s"], []))
                self.assertEqual(job["expectation_oracle"]["kind"], "REC_derived_fixed_snapshot_profile")
                outcome = {"schema":"sparq.proof-binding-outcome.v1", "job_id":job["id"],
                           "case_sha256":job["case_sha256"], "backend":"exact_v2", "tier":"native",
                           "observed":"accepted", "stage":"native", "proof_count":0,
                           "verified_count":0, "error_class":None, "artifacts":[], "controls":[],
                           "result":select(["s"], [["<http://example.org/a>"]])}
                with tempfile.TemporaryDirectory() as directory, self.assertRaises(ValueError):
                    check_outcome(job, outcome, Path(directory))

    def test_versioned_golden_refuses_changed_query_or_source(self):
        source = Path(__file__).resolve().parents[2] / "zk/sparql-evaluator/fixtures/conformance/cases.json"
        original = next(c for c in json.loads(source.read_text())["cases"] if c["id"] == "reject-from-default")
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "retained.json"
            dataset = (source.parent / "core.nt").read_text()
            path.write_text(json.dumps({"cases":[original | {"query":original["query"] + " "}], "default_dataset":"core.nt"}))
            (path.parent / "core.nt").write_text(dataset)
            with self.assertRaisesRegex(ValueError, "golden input changed"):
                import_regressions(path)
            path.write_text(json.dumps({"cases":[original], "default_dataset":"core.nt"}))
            (path.parent / "core.nt").write_text("")
            with self.assertRaisesRegex(ValueError, "golden input changed"):
                import_regressions(path)

    def test_imported_negatives_require_original_category_phase_and_diagnostic(self):
        root = Path(__file__).resolve().parents[2]
        imported = import_regressions(root / "crates/sparq-engine/tests/fixtures/builtin_edges.json", ["v"])
        case = next(c for c in imported if c["id"] == "integer-cast-outside-range")
        job = plan([case], ["exact_v1"], "native")["jobs"][0]
        self.assertEqual(job["expected_rejection"], {"category":"capacity"})
        outcome = {"schema":"sparq.proof-binding-outcome.v1", "job_id":job["id"],
                   "case_sha256":job["case_sha256"], "backend":"exact_v1", "tier":"native",
                   "observed":"rejected", "stage":"native", "proof_count":0,
                   "verified_count":0, "error_class":"parse error", "artifacts":[], "controls":[]}
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory)
            # The actual review counterexample: any nonempty error used to pass.
            with self.assertRaisesRegex(ValueError, "rejection category"):
                check_outcome(job, outcome, path)
            for category, phase, diagnostic in [
                ("parse", "admission", "SPARQL parse rejected"),
                ("profile", "admission", "only SELECT and ASK are admitted"),
                ("capacity", "evaluation", "query evaluation or resource budget rejected"),
                ("capacity", "evaluation", "unknown capacity error"),
            ]:
                outcome.update(error_class=category, rejection={"category":category,"phase":phase,"diagnostic":diagnostic})
                with self.subTest(diagnostic=diagnostic), self.assertRaises(ValueError):
                    check_outcome(job, outcome, path)
            outcome.update(error_class="capacity", rejection={"category":"capacity","phase":"evaluation","diagnostic":"result row capacity"})
            check_outcome(job, outcome, path)
            original = import_regressions(root / "zk/sparql-evaluator/fixtures/conformance/cases.json")
            case = next(c for c in original if c["id"] == "reject-named-graph")
            job = plan([case], ["exact_v1"], "native")["jobs"][0]
            wanted = job["expected_rejection"]
            self.assertEqual(wanted["phase"], case["original_fixture"]["expected"]["stage"])
            self.assertEqual(wanted["diagnostic"], case["original_fixture"]["expected"]["error"])
            outcome.update(job_id=job["id"], case_sha256=job["case_sha256"], error_class="profile", rejection=wanted)
            check_outcome(job, outcome, path)
            for key, value in [("phase","dataset"),("diagnostic","only SELECT and ASK are admitted")]:
                outcome["rejection"] = wanted | {key:value}
                with self.assertRaisesRegex(ValueError, "phase or diagnostic"):
                    check_outcome(job, outcome, path)

    def test_unknown_legacy_classes_remain_unclassified(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "legacy.json"
            original = {"id":"unknown", "query":"ASK {}", "expected":{
                "kind":"rejection", "stage":"admission", "error":"unrecognized historical reason"}}
            path.write_text(json.dumps({"cases":[original]}))
            cases = import_regressions(path)
            self.assertEqual(cases[0]["original_fixture"], original)
            manifest = plan([tiny_case(0, "scan"), *cases], ["exact_v1"], "native")
            self.assertEqual(len(manifest["jobs"]), 2)
            self.assertEqual(manifest["classifications"][0]["status"], "requires_rejection_classification")

    def test_seed_replay_is_explicitly_sampled(self):
        self.assertEqual(sampled(5, 100), sampled(5, 100))
        self.assertNotEqual(sampled(5, 100), sampled(6, 100))
        with self.assertRaises(ValueError):
            plan([], ["exact_v1"], "real")
        with self.assertRaises(ValueError):
            plan([tiny_case(0, "scan")], ["exact_v1", "exact_v1"], "real")

    def test_minimization_retains_seed_and_exact_failure(self):
        original = tiny_case(15, "scan")
        required = ["<urn:a>", "<urn:p>", "<urn:b>"]
        result = minimize(original, lambda case: required in case["triples"])
        self.assertEqual(result["minimized"]["triples"], [required])
        self.assertEqual(result["original"], original)
        self.assertEqual(result["minimized"]["query"], original["query"])
        self.assertEqual(len(original["triples"]), 4)
        with self.assertRaisesRegex(ValueError, "does not reproduce"):
            minimize(original, lambda _: False)
        with self.assertRaisesRegex(ValueError, "normative"):
            minimize(original | {"oracle":{"kind":"retained_golden"}}, lambda _: True)

    def test_real_evidence_cannot_be_native_or_mock_counts(self):
        job = plan([tiny_case(1, "scan")], ["exact_v1"], "real")["jobs"][0]
        outcome = {"schema": "sparq.proof-binding-outcome.v1", "job_id": job["id"],
                   "case_sha256": job["case_sha256"], "backend": job["backend"], "tier": "real",
                   "observed": "accepted", "stage": "native", "proof_count": 0,
                   "verified_count": 0, "error_class": None, "artifacts": [], "controls": [],
                   "result": job["expected_result"]}
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory)
            with self.assertRaisesRegex(ValueError, "genuine"):
                check_outcome(job, outcome, path)
            outcome.update(stage="proof_verifier", proof_count=1, verified_count=1)
            with self.assertRaisesRegex(ValueError, "artifacts"):
                check_outcome(job, outcome, path)
            (path / "proof").write_bytes(b"test-only verifier-boundary fixture, not a real proof")
            from run import file_hash
            outcome["artifacts"] = [{"path": "proof", "sha256": file_hash(path / "proof")}]
            check_outcome(job, outcome, path)
            outcome["case_sha256"] = "0" * 64
            with self.assertRaisesRegex(ValueError, "bound"):
                check_outcome(job, outcome, path)

    def test_late_output_and_nonzero_exit_are_infrastructure_errors(self):
        import sys
        with tempfile.TemporaryDirectory() as directory:
            with self.assertRaisesRegex(RuntimeError, "output_capacity"):
                execute_child([sys.executable, "-c", "print('x'*100000)"], Path(directory), 10, 1000)
        with tempfile.TemporaryDirectory() as directory:
            with self.assertRaisesRegex(RuntimeError, "failure"):
                execute_child([sys.executable, "-c", "raise SystemExit(3)"], Path(directory), 10, 1000)


if __name__ == "__main__":
    unittest.main()
