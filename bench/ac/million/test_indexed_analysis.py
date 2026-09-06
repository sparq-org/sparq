"""[GPT-6] Indexed component completeness and cache missing-value accounting."""
from pathlib import Path
import tempfile
import json
import unittest

from campaign_analysis import Evidence, RequestAnalysis, stats, analyze_campaign
from indexed_analysis import indexed_component_summary
from test_campaign_analysis import MEASUREMENT, records, request, seal, write_rows, complete_fixture


def fixture(root, wrong=False):
    rows = [{"record_type": "configuration", "schema_version": 1, "model": "wac", "pods": 1},
            {"record_type": "prepare", "pod": 0, "generate_ns": 10, "parse_and_index_ns": 20},
            {"record_type": "materialize", "pod": 0, "storage": "memory", "constructor_ns": 1,
             "materialize_ns": 2, "constructor_to_authorized_ready_ns": 3},
            {"record_type": "query", "pod": 0, "role": "owner", "query": "point", "storage": "memory", "reference": True, "rows": 1}]
    files = []
    for storage in ("raw", "compressed"):
        footprint = {"files": 1, "directories": 1, "logical_bytes": 10, "allocated_bytes": 4096}
        rows += [{"record_type": "save", "pod": 0, "storage": storage, "save_ns": 5, "files": footprint},
                 {"record_type": "open", "pod": 0, "storage": storage, "open_and_validation_ns": 100,
                  "constructor_ns": 1, "materialize_ns": 2, "open_to_authorized_ready_ns": 103, "files": footprint},
                 {"record_type": "query", "pod": 0, "role": "owner", "query": "point", "storage": storage,
                  "exact_result_match": not wrong, "rows": 0 if wrong else 1}]
        files.append({"path": f"pod-0-{storage}/index.bin", "bytes": 10, "sha256": "b" * 64})
    rows += [{"record_type": "pod-complete", "pod": 0, "exact_result_comparisons": 2},
             {"record_type": "diagnostic-complete", "pods": 1, "exact_result_comparisons": 2}]
    write_rows(root / "indexed-history8-wac.jsonl", rows)
    write_rows(root / "indexed-history8-wac-files.jsonl", files)
    seal(root)
    return [{"record_type": "indexed-component-diagnostic", "model": "wac", "classification": "complete", "exit_code": 0}]


class IndexedAnalysisTests(unittest.TestCase):
    def test_complete_component_retains_preparation_open_and_allocated_sizes(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            events = fixture(root)
            evidence = Evidence(root)
            result = indexed_component_summary(evidence, "wac", events, stats)
            self.assertTrue(result["complete"], result["issues"])
            self.assertFalse(result["valid_for_component_inference"])
            self.assertEqual(result["exact_comparisons"]["confirmed_matches"], 2)
            self.assertEqual(result["timing_ns"]["parse_plus_memory_authorized_ready_ns"]["p50"], 23)
            self.assertEqual(result["timing_ns"]["open:raw:open_to_authorized_ready_ns"]["p50"], 103)
            self.assertEqual(result["footprint"]["raw"]["snapshots"]["open"]["allocated_bytes"]["total"], 4096)
            self.assertEqual(evidence.errors, [])

    def test_wrong_results_differ_from_resource_or_incomplete_diagnostic(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            events = fixture(root, wrong=True)
            wrong = indexed_component_summary(Evidence(root), "wac", events, stats)
            self.assertTrue(wrong["correctness_failure"])
            events = fixture(root)
            events[0].update(classification="resource-or-diagnostic-error", exit_code=137)
            failed = indexed_component_summary(Evidence(root), "wac", events, stats)
            self.assertFalse(failed["complete"])
            self.assertFalse(failed["correctness_failure"])

    def test_explicit_indexed_mismatch_quarantines_source_despite_passed_review(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory) / "artifacts"; root.mkdir()
            complete_fixture(root)
            events = fixture(root, wrong=True)
            write_rows(root / "campaign-events.jsonl", events)
            seal(root)
            review = Path(directory) / "review.json"
            review.write_text(json.dumps({"source_commit": "a" * 40, "status": "passed"}))
            result = analyze_campaign(root, review)
            self.assertEqual(result["source_review"]["status"], "quarantined")
            self.assertTrue(all(not cell["valid_for_inference"] for cell in result["cells"]))
            self.assertTrue(all(not item["valid_for_component_inference"] for item in result["indexed_component_diagnostics"]))

    def test_missing_cache_headers_are_not_misses_and_failed_outcomes_remain(self):
        with tempfile.TemporaryDirectory() as directory:
            analyzer = RequestAnalysis(Path(directory) / "requests.sqlite", MEASUREMENT)
            try:
                analyzer.requests(records([request(0, cache_hit=1), request(1, cache_hit=0),
                                           request(2, outcome="transport-error", status=None), request(3, cache_hit=True)]))
                result = analyzer.result()
                self.assertEqual(result["cache"]["counts_of_recorded"], {"hit": 1, "miss": 1, "missing": 1, "invalid": 1})
                self.assertEqual(result["cache"]["by_outcome"]["transport-error"], {"missing": 1})
                self.assertEqual(result["cache"]["hit_fraction_of_classified"], .5)
                self.assertEqual(result["cache"]["classification_coverage_of_offered"], .5)
                self.assertEqual(result["offered"], 4)
            finally:
                analyzer.close()


if __name__ == "__main__":
    unittest.main()
