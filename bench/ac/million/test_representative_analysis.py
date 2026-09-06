"""[GPT-6] Main-runner format adapters; all inventories and results here are fixtures."""
import hashlib
import json
from pathlib import Path
import tempfile
import unittest

from campaign_analysis import Evidence, analyze_campaign, inventory_summary
from test_campaign_analysis import complete_fixture, representative_fixture, seal, write_rows


def volume_fixture(root):
    dataset = {"id": "history-6", "pods": 6, "role": "synthetic-test"}
    rows = [{"pod_id": pod, "records": 1, "quads": 3, "bytes": size, "compressed_bytes": 3,
             "records_by_service": {"communication": 1}, "intensity_numerator": 1 if pod < 3 else 4,
             "intensity_denominator": 1} for pod, size in enumerate([10, 30, 30, 20, 60, 60])]
    manifest = {"pods": 6, "records": 6, "quads": 18, "source_bytes": 210, "packed_bytes": 18,
                "index_bytes": 144, "maximum_pod_source_bytes": 60, "populated": True,
                "config": {"volume_classes": [{"numerator": n, "denominator": 1, "weight": 1} for n in (1, 4)]}}
    label = dataset["id"] + "-wac"
    (root / (label + "-manifest.json")).write_text(json.dumps(manifest))
    write_rows(root / (label + "-pod-summaries.jsonl"), rows)
    (root / (label + "-disk.txt")).write_text("4096 fixture\n")
    representatives = [{"pod": pod, "source_bytes": rows[pod]["bytes"], "intensity_numerator": rows[pod]["intensity_numerator"],
                        "intensity_denominator": 1} for pod in (0, 1, 3, 4)]
    events = representative_fixture(root, dataset["id"], "wac", representatives)
    return dataset, events, label


class RepresentativeAnalysisTests(unittest.TestCase):
    def test_main_format_reproduces_every_observed_class_extreme_with_low_id_ties(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            dataset, events, _ = volume_fixture(root)
            result = inventory_summary(Evidence(root), dataset, "wac", events)
            self.assertTrue(result["inventory_consistent"], result["issues"])
            self.assertTrue(result["storage_inventory_consistent"])
            self.assertEqual([r["pod"] for r in result["verification"]["expected_representatives"]], [0, 1, 3, 4])
            self.assertEqual(len(result["verification"]["representatives"]), 4)

    def test_missing_or_reduced_declaration_cannot_fall_back_to_pilot_completion(self):
        for change in ("missing", "reduced", "wrong-size"):
            with self.subTest(change=change), tempfile.TemporaryDirectory() as directory:
                root = Path(directory); dataset, events, label = volume_fixture(root)
                path = root / (label + "-verification-sample.json")
                value = json.loads(path.read_text())
                if change == "missing": path.unlink()
                else:
                    if change == "reduced": value["representatives"].pop()
                    else: value["representatives"][0]["source_bytes"] += 1
                    path.write_text(json.dumps(value))
                write_rows(root / (label + "-verify.jsonl"), [{"record_type": "verification-complete", "checks": 108}])
                result = inventory_summary(Evidence(root), dataset, "wac", events)
                self.assertFalse(result["inventory_consistent"])
                self.assertIn("representative-declaration-missing-or-inventory-mismatch", result["issues"])
                self.assertTrue(result["storage_inventory_consistent"])

    def test_missing_conflicting_and_failed_process_events_reject_representatives(self):
        for change in ("missing", "duplicate", "wrong-size", "wrong-pod", "wrong-class", "exit", "budget"):
            with self.subTest(change=change), tempfile.TemporaryDirectory() as directory:
                root = Path(directory); dataset, events, _ = volume_fixture(root)
                if change == "missing": events.pop()
                elif change == "duplicate": events.append(dict(events[0]))
                else:
                    key, value = {"wrong-size": ("source_bytes", 99), "wrong-pod": ("pod", 2),
                                  "wrong-class": ("classification", "verification-error"),
                                  "exit": ("exit_code", 1), "budget": ("verification_memory_max_bytes", 1)}[change]
                    events[0][key] = value
                result = inventory_summary(Evidence(root), dataset, "wac", events)
                self.assertFalse(result["inventory_consistent"])
                self.assertFalse(result["verification"]["passed"])
                self.assertFalse(result["verification"]["correctness_failure"])

    def test_missing_malformed_and_mismatched_success_logs_are_rejected(self):
        for change in ("missing", "truncated", "wrong-pod", "checks", "sample-count", "oracle", "duplicate", "malformed"):
            with self.subTest(change=change), tempfile.TemporaryDirectory() as directory:
                root = Path(directory); dataset, events, label = volume_fixture(root)
                path = root / (label + "-verify-pod0.jsonl")
                rows = [json.loads(line) for line in path.read_text().splitlines()]
                if change == "missing": path.unlink()
                elif change == "malformed": path.write_text(path.read_text() + "invalid JSON\n")
                else:
                    if change == "truncated": rows.pop()
                    elif change == "wrong-pod": rows[0]["pod"] = 1
                    elif change == "checks": rows[-1]["checks"] = 107
                    elif change == "sample-count": rows[-1]["sampled_pods"] = 2
                    elif change == "oracle": del rows[-1]["oracle"]
                    elif change == "duplicate": rows.append(rows[-1])
                    write_rows(path, rows)
                result = inventory_summary(Evidence(root), dataset, "wac", events)
                self.assertFalse(result["inventory_consistent"])

    def test_unconfigured_intensity_and_corrupt_counts_reject_inventory(self):
        for field, value in (("intensity_numerator", 20), ("bytes", "60"), ("intensity_denominator", 0)):
            with self.subTest(field=field), tempfile.TemporaryDirectory() as directory:
                root = Path(directory); dataset, events, label = volume_fixture(root)
                path = root / (label + "-pod-summaries.jsonl")
                rows = [json.loads(line) for line in path.read_text().splitlines()]
                rows[-1][field] = value
                write_rows(path, rows)
                result = inventory_summary(Evidence(root), dataset, "wac", events)
                self.assertFalse(result["storage_inventory_consistent"])
                self.assertFalse(result["inventory_consistent"])

    def test_oom_preserves_storage_and_other_datasets_but_oracle_failure_quarantines(self):
        for classification in ("verification-oom", "oracle-mismatch"):
            with self.subTest(classification=classification), tempfile.TemporaryDirectory() as directory:
                root = Path(directory) / "artifacts"; root.mkdir(); complete_fixture(root)
                path = root / "campaign-events.jsonl"
                events = [json.loads(line) for line in path.read_text().splitlines()]
                events[0].update(classification=classification, exit_code=137 if classification == "verification-oom" else 1,
                                 systemd_result="oom-kill" if classification == "verification-oom" else "exit-code")
                write_rows(path, events)
                (root / "compact-1-wac-verify-pod0.jsonl").write_text('{"record_type":"verify-pod-start","pod":0}\nprocess failed\n')
                review = Path(directory) / "review.json"
                review.write_text(json.dumps({"source_commit": "a" * 40, "status": "passed"}))
                seal(root)
                result = analyze_campaign(root, review)
                self.assertTrue(all(item["storage_inventory_consistent"] for item in result["corpora"]))
                self.assertTrue(all(not cell["valid_for_inference"] for cell in result["cells"] if cell["model"] == "wac"))
                if classification == "verification-oom":
                    self.assertEqual(result["source_review"]["status"], "passed")
                    self.assertTrue(all(cell["valid_for_inference"] for cell in result["cells"] if cell["model"] == "acp"))
                else:
                    self.assertEqual(result["source_review"]["status"], "quarantined")
                    self.assertTrue(all(not cell["valid_for_inference"] for cell in result["cells"]))

    def test_manifest_counts_and_populated_marker_are_not_coerced(self):
        for field, value in (("pods", 6.0), ("source_bytes", 210.0), ("populated", "true")):
            with self.subTest(field=field), tempfile.TemporaryDirectory() as directory:
                root = Path(directory); dataset, events, label = volume_fixture(root)
                path = root / (label + "-manifest.json")
                manifest = json.loads(path.read_text()); manifest[field] = value
                path.write_text(json.dumps(manifest))
                result = inventory_summary(Evidence(root), dataset, "wac", events)
                self.assertFalse(result["storage_inventory_consistent"])
                self.assertFalse(result["inventory_consistent"])

    def test_gnu_dot_prefix_manifest_verifies_real_main_format_without_input_errors(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory) / "artifacts"; root.mkdir(); complete_fixture(root)
            manifest = root / "MANIFEST.sha256"
            manifest.write_text("\n".join(line.replace("  ", "  ./", 1) for line in manifest.read_text().splitlines()) + "\n")
            review = Path(directory) / "review.json"
            review.write_text(json.dumps({"source_commit": "a" * 40, "status": "passed"}))
            result = analyze_campaign(root, review)
            self.assertTrue(result["artifact_integrity"]["complete"], result["artifact_integrity"])
            self.assertTrue(all(cell["local_guard"] == "pass" for cell in result["cells"]))

    def test_manifest_normalization_rejects_duplicate_aliases_and_escaping_paths(self):
        for name in ("./sample.json", "../outside.json"):
            with self.subTest(name=name), tempfile.TemporaryDirectory() as directory:
                root = Path(directory) / "artifacts"; root.mkdir()
                (root / "sample.json").write_text('{"fixture":true}')
                digest = hashlib.sha256((root / "sample.json").read_bytes()).hexdigest()
                (root / "MANIFEST.sha256").write_text(f"{digest}  sample.json\n{digest}  {name}\n")
                evidence = Evidence(root)
                self.assertEqual(evidence.manifest_status, "invalid")
                expected = "duplicate-manifest-member" if name.startswith("./") else "unsafe-path"
                self.assertTrue(any(error["error"] == expected for error in evidence.errors))

    def test_changed_or_missing_frozen_load_settings_prevent_cell_admission(self):
        for setting in ("scenario", "selection", "mix", "timeout-ms", "max-inflight", "duration-seconds", "requests", "rate"):
            for change in ("missing", "mismatch"):
                with self.subTest(setting=setting, change=change), tempfile.TemporaryDirectory() as directory:
                    root = Path(directory) / "artifacts"; root.mkdir(); complete_fixture(root)
                    name = "compact-1-fixed-ram1-cpu1-r4-0-wac"
                    path = root / (name + "-requests.jsonl")
                    rows = [json.loads(line) for line in path.read_text().splitlines()]
                    if change == "missing": del rows[0]["settings"][setting]
                    else: rows[0]["settings"][setting] = "wrong"
                    write_rows(path, rows); seal(root)
                    review = Path(directory) / "review.json"
                    review.write_text(json.dumps({"source_commit": "a" * 40, "status": "passed"}))
                    result = analyze_campaign(root, review)
                    cell = next(cell for cell in result["cells"] if cell["label"] == name)
                    self.assertFalse(cell["valid_for_inference"])
                    self.assertIn("raw-setting-disagrees:" + setting, cell["issues"])
                    self.assertTrue(all(other["valid_for_inference"] for other in result["cells"] if other["label"] != name))


if __name__ == "__main__": unittest.main()
