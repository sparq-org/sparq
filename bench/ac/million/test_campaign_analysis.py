"""[GPT-6] Protect capacity bounds, failure accounting, reconciliation and integrity."""
import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

from campaign_analysis import (Evidence, RequestAnalysis, capacity_bounds, paired_ratio_ci,
                               resource_summary, analyze_campaign)


MEASUREMENT = {"server_deadline_ms": 200, "measurement_seconds": 1, "minimum_offered": 4,
               "success_fraction": .99, "deadline_fraction_of_all_offered": .95,
               "queue_stability": {"allowed_growth_us_floor": 1000,
                 "allowed_growth_fraction_of_first_quarter": .1,
                 "required_timer_coverage_fraction_of_offered": .99}}


def request(sequence, **overrides):
    row = {"record_type": "request", "sequence": sequence, "scheduled_us": (sequence + 1) * 300000,
           "pod": 0, "operation": "query", "principal": "owner", "query_id": "point", "status": 200,
           "outcome": "ok", "scheduled_latency_us": 10000, "http_latency_us": 9500,
           "server_us": 9000, "queue_us": 100, "dispatch_lag_us": 100}
    row.update(overrides)
    return row


def records(requests):
    return [{"record_type": "load-start", "offered_rate": 4}, *requests,
            {"record_type": "load-complete", "offered": 4, "client_dropped": sum(r["outcome"] == "client-admission-drop" for r in requests),
             "plan_exhausted": sum(r["outcome"] == "client-plan-exhausted" for r in requests), "elapsed_us": 1300000}]


def resource(peak=1000000, oom=0):
    return {"memory.current": str(peak), "memory.peak": str(peak),
            "memory.events": f"oom {oom}\noom_kill 0\n", "cpu.stat": "usage_usec 1000\n"}


def write_rows(path, rows):
    path.write_text("".join(json.dumps(row) + "\n" for row in rows))


def seal(root):
    paths = sorted(p for p in root.iterdir() if p.name != "MANIFEST.sha256")
    (root / "MANIFEST.sha256").write_text("".join(f"{hashlib.sha256(p.read_bytes()).hexdigest()}  {p.name}\n" for p in paths))


def complete_fixture(root):
    dataset = {"id": "compact-1", "pods": 1, "models": ["wac", "acp"], "role": "populated-control"}
    campaign = {"campaign_id": "fixture", "corpora": [dataset], "seeds": [11, 22], "measurement": MEASUREMENT,
                "groups": [{"id": "fixed", "datasets": ["compact-1"], "memory_gib": [1], "cpus": [1], "rates": [4], "repeat": 2}]}
    (root / "campaign.json").write_text(json.dumps(campaign))
    (root / "source-commit.txt").write_text("a" * 40)
    (root / "input-hashes.txt").write_text("b" * 64 + "  bench/ac/million/workload.json\n")
    (root / "campaign-events.jsonl").write_text("")
    (root / "DONE").write_text("")
    for model in dataset["models"]:
        label = "compact-1-" + model
        manifest = {"pods": 1, "records": 1, "quads": 3, "source_bytes": 30, "packed_bytes": 10,
                    "maximum_pod_source_bytes": 30, "populated": True, "packed_sha256": model + "-pack", "index_sha256": model + "-index"}
        (root / (label + "-manifest.json")).write_text(json.dumps(manifest))
        write_rows(root / (label + "-pod-summaries.jsonl"), [{"pod_id": 0, "records": 1, "quads": 3, "bytes": 30,
                    "compressed_bytes": 10, "records_by_service": {"communication": 1}}])
        write_rows(root / (label + "-verify.jsonl"), [{"record_type": "verification-complete", "checks": 1}])
        (root / (label + "-disk.txt")).write_text("4096 corpus\n")
        churn, audit = [], []
        for i, (state, delta) in enumerate(zip(["grant", "revoke", "probe-revoked", "probe-revoked", "grant", "revoke", "grant"], [0, -1, None, None, 1, -1, 1])):
            if delta is not None:
                churn += [{"record_type": "negative-policy-probe", "status": 403} for _ in range(3)]
                receipt = {"id": f"churn-{i}"}
                churn.append({"record_type": "request", "pod": 0, "mutation_id": receipt["id"], "mutation_receipt": receipt,
                              "status": 200, "outcome": "ok", "policy_triple_delta": delta})
                audit.append({"record_type": "committed-mutation", "pod": 0, "receipt": receipt})
            churn += [{"record_type": "authorization-probe", "outcome": "ok", "status": 200, "count": "0", "expected_calendar_records": 0} for _ in range(3)]
            churn.append({"record_type": "policy-churn-check-complete", "state": state})
        audit.append({"record_type": "mutation-audit-complete", "committed_receipts": 5})
        write_rows(root / (label + "-dedicated-churn-requests.jsonl"), churn)
        write_rows(root / (label + "-dedicated-churn-audit.jsonl"), audit)
        (root / (label + "-dedicated-churn-reconciliation.json")).write_text('{"passed":true}')
        for replicate, seed in enumerate(campaign["seeds"]):
            name = f"compact-1-fixed-ram1-cpu1-r4-{replicate}-{model}"
            data = records([request(i) for i in range(4)])
            data[0].update(settings={"seed": str(seed)}, workload_sha256="b" * 64, corpus_manifest=manifest)
            write_rows(root / (name + "-requests.jsonl"), data)
            write_rows(root / (name + "-audit.jsonl"), [{"record_type": "mutation-audit-complete", "committed_receipts": 0}])
            for stage in ("before", "after"):
                (root / (name + f"-{stage}-resources.json")).write_text(json.dumps(resource()))
            (root / (name + "-reconciliation.json")).write_text('{"passed":true}')
            (root / (name + "-summary.json")).write_text(json.dumps({"dataset": "compact-1", "group": "fixed", "memory_gib": 1,
                "cpus": 1, "rate_override": 4, "replicate": replicate, "model": model, "seed": seed, "offered_rate": 4,
                "load_exit_code": 0, "warmup_exit_code": 0, "audit_exit_code": 0}))
    seal(root)


class CampaignAnalysisTests(unittest.TestCase):
    def test_http_status_retains_success_and_fast_errors_without_changing_deadlines(self):
        with tempfile.TemporaryDirectory() as temp:
            analyzer = RequestAnalysis(Path(temp) / "test.sqlite", MEASUREMENT)
            analyzer.requests(records([request(0), request(1),
                request(2, status=503, outcome="http-error", scheduled_latency_us=50, http_latency_us=40, server_us=30),
                request(3, status=401, outcome="http-error", scheduled_latency_us=60, http_latency_us=50, server_us=40)]))
            result = analyzer.result(); analyzer.close()
        self.assertEqual(result["http_status"]["counts_of_recorded"], {"200": 2, "401": 1, "503": 1, "missing": 0, "invalid": 0})
        self.assertEqual(result["http_status"]["by_outcome"], {"ok": {"200": 2}, "http-error": {"401": 1, "503": 1}})
        self.assertEqual(result["successful"], 2)
        self.assertEqual(result["success_fraction_of_offered"], .5)
        self.assertEqual(result["deadline_fraction_of_offered"], .5)
        self.assertEqual(result["latency_us"]["successful:scheduled_latency_us"], {"n": 2, "p50": 10000, "p95": 10000, "p99": 10000, "maximum": 10000})
        self.assertEqual(result["latency_us"]["all:scheduled_latency_us"]["n"], 4)
        self.assertTrue(result["complete_valid_schedule"])

    def test_http_status_does_not_infer_responses_for_transport_or_unsent_requests(self):
        missing_status = request(3, outcome="client-plan-exhausted")
        del missing_status["status"]
        with tempfile.TemporaryDirectory() as temp:
            analyzer = RequestAnalysis(Path(temp) / "test.sqlite", MEASUREMENT)
            analyzer.requests(records([request(0), request(1, status=None, outcome="transport-error", is_timeout=True),
                request(2, status=None, outcome="client-admission-drop"), missing_status]))
            result = analyzer.result(); analyzer.close()
        self.assertEqual(result["http_status"]["counts_of_recorded"], {"200": 1, "missing": 3, "invalid": 0})
        for outcome in ("transport-error", "client-admission-drop", "client-plan-exhausted"):
            self.assertEqual(result["http_status"]["by_outcome"][outcome], {"missing": 1})
        self.assertEqual(result["explicit_timeouts"], 1)
        self.assertEqual(result["success_fraction_of_offered"], .25)
        self.assertEqual(result["deadline_fraction_of_offered"], .25)
        self.assertTrue(result["client_limited"])

    def test_http_status_counts_only_present_rows_in_an_incomplete_schedule(self):
        incomplete = request(2, outcome="http-error")
        del incomplete["status"]
        with tempfile.TemporaryDirectory() as temp:
            analyzer = RequestAnalysis(Path(temp) / "test.sqlite", MEASUREMENT)
            analyzer.requests(records([request(0), request(1, status=503, outcome="http-error"), incomplete]))
            result = analyzer.result(); analyzer.close()
        self.assertEqual(result["http_status"]["counts_of_recorded"], {"200": 1, "503": 1, "missing": 1, "invalid": 0})
        self.assertEqual(sum(result["http_status"]["counts_of_recorded"].values()), result["recorded"])
        self.assertEqual((result["recorded"], result["offered"]), (3, 4))
        self.assertEqual(result["success_fraction_of_offered"], .25)
        self.assertEqual(result["deadline_fraction_of_offered"], .25)
        self.assertFalse(result["complete_valid_schedule"])
        self.assertIn("offered-sequence-coverage-incomplete", result["issues"])

    def test_http_status_rejects_invalid_values_without_coercion(self):
        for status in (True, False, "200", 200.0, 99, 600, [], {}):
            with self.subTest(status=status), tempfile.TemporaryDirectory() as temp:
                analyzer = RequestAnalysis(Path(temp) / "test.sqlite", MEASUREMENT)
                analyzer.requests(records([request(0, status=status, outcome="http-error"), *[request(i) for i in (1, 2, 3)]]))
                result = analyzer.result(); analyzer.close()
                self.assertEqual(result["http_status"]["counts_of_recorded"], {"200": 3, "missing": 0, "invalid": 1})
                self.assertEqual(result["http_status"]["by_outcome"]["http-error"], {"invalid": 1})
                self.assertEqual(result["success_fraction_of_offered"], .75)
                self.assertTrue(result["complete_valid_schedule"])

    def test_end_to_end_admission_failure_does_not_poison_verified_compact_cells(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp) / "artifacts"; root.mkdir()
            complete_fixture(root)
            review = Path(temp) / "review.json"
            review.write_text(json.dumps({"source_commit": "a" * 40, "status": "passed"}))
            result = analyze_campaign(root, review)
            self.assertTrue(all(c["local_guard"] == "pass" for c in result["cells"]))
            self.assertTrue(result["paired_comparisons"][0]["paired_p95_scheduled_response_ratio"]["available"])
            self.assertFalse(result["full_service_million_history_admitted"])
            write_rows(root / "campaign-events.jsonl", [{"record_type": "admission-quarantine", "dataset": "unreached-history", "model": "wac"}])
            (root / "DONE").unlink(); (root / "FAILED").write_text("resource stop")
            seal(root)
            limited = analyze_campaign(root, review)
            self.assertTrue(all(c["valid_for_inference"] for c in limited["cells"]))
            write_rows(root / "campaign-events.jsonl", [{"record_type": "correctness-quarantine", "label": "wrong-result"}])
            seal(root)
            quarantined = analyze_campaign(root, review)
            self.assertTrue(all(not c["valid_for_inference"] for c in quarantined["cells"]))
            self.assertFalse(quarantined["paired_comparisons"][0]["paired_p95_scheduled_response_ratio"]["available"])

    def test_equal_coarse_passing_rates_do_not_establish_equivalence(self):
        rows = [{"rate": 32, "state": "pass"}, {"rate": 128, "state": "fail"}, {"rate": 512, "state": "inconclusive"}]
        result = capacity_bounds({"wac": rows, "acp": rows})
        self.assertEqual(result["highest_tested_passing_rate_ratio_acp_over_wac"], 1)
        self.assertEqual(result["conservative_capacity_ratio_bounds"], [.25, 4])
        self.assertEqual(result["capacity_equivalence"], "unestablished")
        ci = paired_ratio_ci([(32, 32)] * 5, resamples=100)
        self.assertEqual(ci["ci95"], [1, 1])
        self.assertEqual(result["capacity_equivalence"], "unestablished")

    def test_missing_or_nonmonotone_capacity_brackets_stay_unknown(self):
        cases = [[{"rate": 32, "state": "pass"}, {"rate": 128, "state": "inconclusive"}],
                 [{"rate": 32, "state": "fail"}, {"rate": 128, "state": "pass"}]]
        for rows in cases:
            with self.subTest(rows=rows):
                result = capacity_bounds({"wac": rows, "acp": rows})
                self.assertIsNone(result["conservative_capacity_ratio_bounds"])
                self.assertEqual(result["capacity_equivalence"], "unestablished")

    def test_fine_brackets_retain_conservative_width(self):
        rows = [{"rate": 100, "state": "pass"}, {"rate": 105, "state": "fail"}]
        result = capacity_bounds({"wac": rows, "acp": rows})
        self.assertEqual(result["conservative_capacity_ratio_bounds"], [100 / 105, 105 / 100])
        self.assertEqual(result["capacity_equivalence"], "bounded-within-margin")

    def test_failed_mutation_is_resolved_using_durable_audit(self):
        receipt = {"id": "x", "inserted_triples": 1, "deleted_triples": 0, "inserted_records": 1, "deleted_records": 0}
        with tempfile.TemporaryDirectory() as temp:
            analyzer = RequestAnalysis(Path(temp) / "test.sqlite", MEASUREMENT)
            analyzer.audit([{"record_type": "committed-mutation", "pod": 0, "receipt": receipt},
                            {"record_type": "mutation-audit-complete", "committed_receipts": 1}])
            analyzer.requests(records([request(0), request(1, status=None, outcome="transport-error", is_timeout=True,
                operation="ingest", mutation_id="x", scheduled_latency_us=5000000, http_latency_us=5000000),
                request(2, status=None, outcome="client-admission-drop"), request(3)]))
            result = analyzer.result(); analyzer.close()
            self.assertEqual(result["deadline_fraction_of_offered"], .5)
            self.assertEqual(result["explicit_timeouts"], 1)
            self.assertEqual(result["mutation"]["unknown_at_client"], 1)
            self.assertEqual(result["mutation"]["unknown_resolved_committed"], 1)
            self.assertEqual(result["mutation"]["unknown_resolved_not_committed"], 0)
            self.assertTrue(result["client_limited"])
            self.assertEqual(result["latency_us"]["successful:scheduled_latency_us"]["p95"], 10000)

    def test_acknowledgement_must_match_exact_durable_receipt(self):
        receipt = {"id": "x", "inserted_triples": 1, "deleted_triples": 0}
        with tempfile.TemporaryDirectory() as temp:
            analyzer = RequestAnalysis(Path(temp) / "test.sqlite", MEASUREMENT)
            analyzer.audit([{"record_type": "committed-mutation", "pod": 0, "receipt": {**receipt, "inserted_triples": 2}},
                            {"record_type": "mutation-audit-complete", "committed_receipts": 1}])
            analyzer.requests(records([request(0, operation="ingest", mutation_id="x", mutation_receipt=receipt), *[request(i) for i in (1, 2, 3)]]))
            result = analyzer.result(); analyzer.close()
            self.assertFalse(result["mutation"]["reconciled"])
            self.assertIn("acknowledgement-not-identically-durable", result["mutation"]["issues"])

    def test_missing_before_resources_prevents_resource_admission(self):
        self.assertFalse(resource_summary(None, resource(), 1)["passed"])
        self.assertFalse(resource_summary(resource(), resource(oom=1), 1)["passed"])
        self.assertFalse(resource_summary(resource(), resource(peak=2 * 1024**3), 1)["passed"])
        self.assertTrue(resource_summary(resource(), resource(), 1)["passed"])

    @unittest.skipUnless(shutil.which("zstd"), "zstd CLI required for compressed artifact support")
    def test_compressed_inputs_are_verified_and_truncation_is_rejected(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp); raw = root / "sample.jsonl"
            raw.write_text('{"record_type":"sample","value":17}\n')
            subprocess.run(["zstd", "-q", "--rm", str(raw)], check=True)
            compressed = root / "sample.jsonl.zst"
            digest = hashlib.sha256(compressed.read_bytes()).hexdigest()
            (root / "MANIFEST.sha256").write_text(f"{digest}  sample.jsonl.zst\n")
            evidence = Evidence(root)
            self.assertEqual(list(evidence.rows("sample.jsonl"))[0]["value"], 17)
            self.assertEqual(evidence.manifest_status, "verified")
            compressed.write_bytes(compressed.read_bytes()[:-2])
            broken = Evidence(root)
            list(broken.rows("sample.jsonl"))
            self.assertEqual(broken.manifest_status, "invalid")
            self.assertTrue(any(x["error"] == "zstd-decompression-failed" for x in broken.errors))

    def test_later_source_quarantine_overrides_passed_review(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            campaign = {"campaign_id": "test", "corpora": [], "groups": [], "measurement": MEASUREMENT}
            (root / "campaign.json").write_text(json.dumps(campaign))
            (root / "source-commit.txt").write_text("exact-source\n")
            (root / "campaign-events.jsonl").write_text('{"record_type":"correctness-quarantine","label":"failed-cell"}\n')
            (root / "DONE").write_text("")
            (root / "MANIFEST.sha256").write_text("".join(f"{hashlib.sha256(p.read_bytes()).hexdigest()}  {p.name}\n" for p in sorted(root.iterdir())))
            review = root / "review.json"
            review.write_text('{"source_commit":"exact-source","status":"passed"}')
            result = analyze_campaign(root, review)
            self.assertEqual(result["source_review"]["status"], "quarantined")
            self.assertFalse(result["full_service_million_history_admitted"])


if __name__ == "__main__": unittest.main()
