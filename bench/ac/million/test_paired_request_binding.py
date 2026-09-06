"""[GPT-6] Pair semantic request intent, independently of responses or policy syntax."""
import copy
import json
from pathlib import Path
import tempfile
import unittest

from campaign_analysis import RequestAnalysis, analyze_campaign
from test_campaign_analysis import MEASUREMENT, complete_fixture, records, request, seal, write_rows


def fingerprint(changed):
    with tempfile.TemporaryDirectory() as directory:
        analyzer = RequestAnalysis(Path(directory) / "requests.sqlite", MEASUREMENT)
        try:
            analyzer.requests(records([changed, *[request(i) for i in (1, 2, 3)]]))
            return analyzer.result()["schedule_sha256"]
        finally:
            analyzer.close()


class PairedRequestBindingTests(unittest.TestCase):
    def test_same_query_id_with_changed_query_content_changes_fingerprint(self):
        baseline = request(0, query_sha256="a" * 64)
        self.assertNotEqual(fingerprint(baseline), fingerprint({**baseline, "query_sha256": "b" * 64}))
        self.assertNotEqual(fingerprint(baseline), fingerprint({**baseline, "query_sha256": None}))

    def test_same_counts_cannot_hide_changed_mutation_targets_or_values(self):
        cases = [
            ("ingest", {"kind": "insert", "batch_id": 101, "count": 2}, "batch_id", 102),
            ("expire", {"kind": "delete", "offset": 0, "count": 2}, "offset", 2),
            ("modify", {"kind": "modify", "offset": 20, "count": 2, "revision": 1}, "offset", 21),
            ("modify", {"kind": "modify", "offset": 20, "count": 2, "revision": 1}, "revision", 2),
            ("modify", {"kind": "modify", "offset": 20, "count": 2, "revision": 1}, "count", 3),
        ]
        for operation, mutation, field, value in cases:
            with self.subTest(operation=operation, field=field):
                baseline = request(0, operation=operation, service="communication", planned_records=2,
                                   query_sha256="a" * 64, mutation_request=mutation,
                                   expected_inserted_triples=2, expected_deleted_triples=2)
                changed = copy.deepcopy(baseline); changed["mutation_request"][field] = value
                self.assertNotEqual(fingerprint(baseline), fingerprint(changed))

    def test_expected_triple_counts_are_part_of_non_policy_intent(self):
        baseline = request(0, operation="modify", service="communication", planned_records=2,
                           query_sha256="a" * 64,
                           mutation_request={"kind": "modify", "offset": 20, "count": 2, "revision": 1},
                           expected_inserted_triples=2, expected_deleted_triples=2)
        for field in ("expected_inserted_triples", "expected_deleted_triples"):
            with self.subTest(field=field):
                self.assertNotEqual(fingerprint(baseline), fingerprint({**baseline, field: 3}))
        reordered = copy.deepcopy(baseline)
        reordered["mutation_request"] = dict(reversed(list(baseline["mutation_request"].items())))
        self.assertEqual(fingerprint(baseline), fingerprint(reordered))

    def test_policy_syntax_differs_but_common_rights_must_match(self):
        wac = request(0, operation="policy-attempt", desired_grant=False, query_sha256="a" * 64, request_bytes=100)
        acp = {**wac, "query_sha256": "b" * 64, "request_bytes": 200}
        self.assertEqual(fingerprint(wac), fingerprint(acp))
        for field, value in (("desired_grant", True), ("pod", 1), ("principal", "recipient")):
            with self.subTest(field=field):
                self.assertNotEqual(fingerprint(wac), fingerprint({**acp, field: value}))

    def test_response_outcomes_and_observed_timings_do_not_change_intent(self):
        baseline = request(0, query_sha256="a" * 64)
        failed = {**baseline, "status": 503, "outcome": "http-error", "scheduled_latency_us": 10,
                  "http_latency_us": 8, "server_us": 5, "queue_us": 0, "cache_hit": 0}
        self.assertEqual(fingerprint(baseline), fingerprint(failed))

    def test_changed_query_prevents_paired_interval_without_erasing_local_results(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory) / "artifacts"; root.mkdir(); complete_fixture(root)
            review = Path(directory) / "review.json"
            review.write_text(json.dumps({"source_commit": "a" * 40, "status": "passed"}))
            before = analyze_campaign(root, review)
            self.assertTrue(before["paired_comparisons"][0]["paired_p95_scheduled_response_ratio"]["available"])
            path = root / "compact-1-fixed-ram1-cpu1-r4-0-acp-requests.jsonl"
            rows = [json.loads(line) for line in path.read_text().splitlines()]
            rows[1]["query_sha256"] = "b" * 64
            write_rows(path, rows); seal(root)
            after = analyze_campaign(root, review)
            self.assertTrue(all(cell["local_guard"] == "pass" for cell in after["cells"]))
            pair = after["paired_comparisons"][0]
            self.assertIn("paired-intended-schedules-differ", pair["issues"])
            self.assertFalse(pair["paired_p95_scheduled_response_ratio"]["available"])
            self.assertFalse(pair["same_load_observed_deadline_completion_ratio"]["available"])


if __name__ == "__main__": unittest.main()
