"""[GPT-6] Check fail-closed paper bindings with synthetic analysis fixtures only."""
import copy
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

from campaign_analysis import analyze_campaign, capacity_bounds
from test_campaign_analysis import complete_fixture, write_rows, seal
from test_indexed_analysis import fixture as indexed_fixture


ROOT = Path(__file__).resolve().parents[3]
TYPST = os.environ.get("TYPST_BIN") or shutil.which("typst")


@unittest.skipUnless(TYPST, "set TYPST_BIN or install Typst to check manuscript bindings")
class PaperResultBindings(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="paper-binding-test-", dir=ROOT)
        self.addCleanup(self.temp.cleanup)
        self.directory = Path(self.temp.name)
        artifacts = self.directory / "artifacts"
        artifacts.mkdir()
        complete_fixture(artifacts)
        review = self.directory / "review.json"
        review.write_text(json.dumps({"source_commit": "a" * 40, "status": "passed"}))
        self.analysis = analyze_campaign(artifacts, review)
        self.artifacts, self.review = artifacts, review

    def query(self, body, inputs=None):
        if inputs is not None:
            (self.directory / "analysis.json").write_text(json.dumps(inputs))
        source = self.directory / "binding.typ"
        source.write_text('#import "../site/papers/solid-pod-scale-results.typ": main-state, main-tables, bounds, verdict-counts, model-responses, range-label, model-resources, storage-values, storage-number, queue-only, summary-verdicts\n' + body)
        result = subprocess.run([TYPST, "query", "--root", str(ROOT), str(source), "<result>", "--field", "value"],
                                check=True, text=True, capture_output=True)
        return json.loads(result.stdout)

    def test_missing_partial_review_mismatch_and_quarantine_remain_closed(self):
        self.assertEqual(self.query('#metadata(main-state(none)) <result>'), ["absent (campaign in progress)"])
        variants = [
            ("incomplete or checksum-invalid", ("artifact_integrity", "complete"), False),
            ("source review absent or mismatched", ("source_review", "source_binding_matches"), False),
            ("source review absent or mismatched", ("source_review", "status"), "unreviewed"),
            ("source quarantined; no main inference", ("source_review", "status"), "quarantined"),
            ("source quarantined; no main inference", ("source_review", "quarantine_events"), [{"record_type": "correctness-quarantine"}]),
        ]
        for expected, path, value in variants:
            with self.subTest(expected=expected, path=path):
                data = copy.deepcopy(self.analysis)
                data[path[0]][path[1]] = value
                self.assertEqual(self.query('#metadata(main-state(json("analysis.json"))) <result>', data), [expected])

    def test_positive_schema_renders_and_coarse_capacity_bounds_remain_wide(self):
        data = copy.deepcopy(self.analysis)
        observations = {model: [{"rate": 32, "state": "pass"}, {"rate": 128, "state": "fail"}] for model in ("wac", "acp")}
        data["capacity_brackets"] = [{"dataset": "compact-1", "group": "fixed", "memory_gib": 1, "cpus": 1,
                                     **capacity_bounds(observations)}]
        # Typst query lays out the tables without producing a manuscript PDF.
        result = self.query('''#let data = json("analysis.json")
#main-tables(data)
#metadata((main-state(data), bounds(data.capacity_brackets.first().conservative_capacity_ratio_bounds))) <result>
''', data)
        self.assertEqual(result, [["finalized and reviewed", "[0.25, 4]"]])
        self.assertFalse(data["full_service_million_history_admitted"])

    def test_admitted_component_table_uses_actual_extractor_schema(self):
        events = indexed_fixture(self.artifacts)
        event_path = self.artifacts / "campaign-events.jsonl"
        existing = [json.loads(line) for line in event_path.read_text().splitlines() if line.strip()]
        write_rows(event_path, existing + events)
        seal(self.artifacts)
        data = analyze_campaign(self.artifacts, self.review)
        self.assertTrue(data["indexed_component_diagnostics"][0]["valid_for_component_inference"])
        result = self.query('#let data = json("analysis.json")\n#main-tables(data)\n#metadata(main-state(data)) <result>', data)
        self.assertEqual(result, ["finalized and reviewed"])

    def test_absolute_ranges_do_not_pool_runs_or_include_invalid_records(self):
        cells = [
            {"model": "wac", "local_guard": "pass", "valid_for_inference": True, "requests": {
                "latency_us": {"successful:scheduled_latency_us": {"p95": 10000}},
                "deadline_fraction_of_offered": .98, "success_fraction_of_offered": .999}},
            {"model": "wac", "local_guard": "fail", "valid_for_inference": True, "requests": {
                "latency_us": {"successful:scheduled_latency_us": {"p95": 300000}},
                "deadline_fraction_of_offered": .005, "success_fraction_of_offered": .01}},
            {"model": "wac", "local_guard": "inconclusive", "valid_for_inference": False, "requests": {
                "latency_us": {"successful:scheduled_latency_us": {"p95": 1}},
                "deadline_fraction_of_offered": 1, "success_fraction_of_offered": 1}},
            {"model": "wac", "local_guard": "unmeasured", "valid_for_inference": False},
        ]
        result = self.query('#metadata(model-responses(json("analysis.json").cells, "wac")) <result>', {"cells": cells})[0]
        self.assertEqual(result["planned_runs"], 4)
        self.assertEqual(result["valid_runs"], 2)
        self.assertEqual(result["successful_p95_us"], {"n": 2, "lower": 10000, "upper": 300000})
        self.assertEqual(result["timely_fraction"], {"n": 2, "lower": .005, "upper": .98})
        self.assertEqual(result["success_fraction"], {"n": 2, "lower": .01, "upper": .999})

    def test_fast_errors_do_not_become_successful_latency_or_missing_zero(self):
        cells = [
            {"model": "wac", "local_guard": "fail", "valid_for_inference": True, "requests": {
                "latency_us": {"successful:scheduled_latency_us": {"p95": None},
                               "all:scheduled_latency_us": {"p95": 1000}},
                "deadline_fraction_of_offered": 0, "success_fraction_of_offered": 0}},
            {"model": "acp", "local_guard": "inconclusive", "valid_for_inference": False, "requests": {
                "latency_us": {"successful:scheduled_latency_us": {"p95": 1000}},
                "deadline_fraction_of_offered": 1, "success_fraction_of_offered": 1}},
        ]
        result = self.query('''#let cells = json("analysis.json").cells
#let wac = model-responses(cells, "wac")
#let acp = model-responses(cells, "acp")
#metadata((range-label(wac.successful_p95_us, scale: 0.001), range-label(wac.timely_fraction, scale: 100),
           range-label(wac.success_fraction, scale: 100), range-label(acp.timely_fraction, scale: 100))) <result>
''', {"cells": cells})
        self.assertEqual(result, [["—", "0", "0", "—"]])

    def test_percent_range_rounding_does_not_hide_a_small_failure_fraction(self):
        result = self.query('#metadata(range-label((n: 1, lower: 0.99999, upper: 0.99999), scale: 100)) <result>')
        self.assertEqual(result, ["99.9–100"])

    def test_storage_boundaries_keep_source_file_lengths_and_allocation_separate(self):
        data = {"corpora": [{"dataset": "fixture", "model": "wac", "inventory_consistent": True,
            "manifest": {"records": 2, "quads": 17, "source_bytes": 100000, "packed_bytes": 700, "index_bytes": 24},
            "allocated_bytes_before_load": 12288}]}
        result = self.query('#metadata(storage-values(json("analysis.json"), "fixture", "wac")) <result>', data)[0]
        self.assertEqual(result, {"records": 2, "quads": 17, "source_bytes": 100000,
                                 "packed_index_bytes": 724, "allocated_bytes": 12288})
        del data["corpora"][0]["manifest"]["index_bytes"]
        missing = self.query('#metadata(storage-values(json("analysis.json"), "fixture", "wac")) <result>', data)[0]
        self.assertIsNone(missing["packed_index_bytes"])
        self.assertEqual(missing["allocated_bytes"], 12288)
        data["corpora"][0]["inventory_consistent"] = False
        invalid = self.query('#metadata(storage-values(json("analysis.json"), "fixture", "wac")) <result>', data)[0]
        self.assertTrue(all(value is None for value in invalid.values()))
        # New adapter separates completed storage arithmetic from heavy-Pod query admission.
        data["corpora"][0]["storage_inventory_consistent"] = True
        storage_only = self.query('#metadata(storage-values(json("analysis.json"), "fixture", "wac")) <result>', data)[0]
        self.assertEqual(storage_only["source_bytes"], 100000)
        data["corpora"][0]["inventory_consistent"] = True
        data["corpora"][0]["storage_inventory_consistent"] = False
        inconsistent = self.query('#metadata(storage-values(json("analysis.json"), "fixture", "wac")) <result>', data)[0]
        self.assertTrue(all(value is None for value in inconsistent.values()))
        self.assertEqual(self.query('#metadata((storage-number(none), storage-number(0), storage-number(1, divisor: 1000000, digits: 2), storage-number(1234567))) <result>'), [["—", "0", "<0.01", "1,234,567"]])

    def test_resources_keep_guard_failures_but_exclude_inconclusive_counters(self):
        cells = []
        for state, valid, peak, cpu in (("pass", True, 2000000, 3000000), ("fail", True, 5000000, 9000000),
                                        ("inconclusive", False, 999000000, 888000000), ("unmeasured", False, None, None)):
            cells.append({"model": "wac", "local_guard": state, "valid_for_inference": valid,
                "resources": {"samples": {"after": {"memory_peak_bytes": peak}},
                              "deltas": {"cpu_stat": {"usage_usec": cpu}}}})
        result = self.query('#metadata(model-resources(json("analysis.json").cells, "wac")) <result>', {"cells": cells})[0]
        self.assertEqual(result, {"peak_bytes": {"n": 2, "lower": 2000000, "upper": 5000000},
                                  "cpu_us": {"n": 2, "lower": 3000000, "upper": 9000000}})
        cells[0]["resources"]["samples"]["after"]["memory_peak_bytes"] = None
        cells[1]["resources"]["samples"]["after"]["memory_peak_bytes"] = None
        cells[0]["resources"]["deltas"]["cpu_stat"]["usage_usec"] = -1
        missing = self.query('#metadata(model-resources(json("analysis.json").cells, "wac")) <result>', {"cells": cells})[0]
        self.assertEqual(missing["peak_bytes"], {"n": 0, "lower": None, "upper": None})
        self.assertEqual(missing["cpu_us"], {"n": 1, "lower": 9000000, "upper": 9000000})

    def test_main_summary_distinguishes_queue_only_from_delivery_failures(self):
        data = copy.deepcopy(self.analysis)
        cells = [copy.deepcopy(data["cells"][0]) for _ in range(3)]
        for cell in cells:
            cell["model"] = "wac"
        cells[1]["local_guard"] = "fail"
        cells[1]["requests"]["queue"]["passed"] = False
        cells[2]["local_guard"] = "fail"
        cells[2]["requests"]["queue"]["passed"] = False
        cells[2]["requests"]["success_fraction_of_offered"] = 0
        cells[2]["requests"]["deadline_fraction_of_offered"] = 0
        cells[2]["requests"]["within_server_production_deadline"] = 0
        data["cells"] = cells
        result = self.query('''#let d = json("analysis.json")
#metadata(summary-verdicts(d.cells, "wac", d.campaign.measurement)) <result>
''', data)
        self.assertEqual(result, [[1, 1, 1]])
        self.query('#main-tables(json("analysis.json"), summary: true)\n#metadata(true) <result>', self.analysis)

    def test_all_planned_states_are_visible(self):
        result = self.query('''#let cells = ("pass", "fail", "inconclusive", "unmeasured").map(s => (model: "wac", local_guard: s))
#metadata(verdict-counts(cells, "wac")) <result>
''')
        self.assertEqual(result, ["1/1/1/1"])


if __name__ == "__main__":
    unittest.main()
