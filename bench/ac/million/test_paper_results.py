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
        source.write_text('#import "../site/papers/solid-pod-scale-results.typ": main-state, main-tables, bounds, verdict-counts\n' + body)
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
        write_rows(self.artifacts / "campaign-events.jsonl", events)
        seal(self.artifacts)
        data = analyze_campaign(self.artifacts, self.review)
        self.assertTrue(data["indexed_component_diagnostics"][0]["valid_for_component_inference"])
        result = self.query('#let data = json("analysis.json")\n#main-tables(data)\n#metadata(main-state(data)) <result>', data)
        self.assertEqual(result, ["finalized and reviewed"])

    def test_all_planned_states_are_visible(self):
        result = self.query('''#let cells = ("pass", "fail", "inconclusive", "unmeasured").map(s => (model: "wac", local_guard: s))
#metadata(verdict-counts(cells, "wac")) <result>
''')
        self.assertEqual(result, ["1/1/1/1"])


if __name__ == "__main__":
    unittest.main()
