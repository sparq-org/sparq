#!/usr/bin/env python3
"""Regression tests for the fail-closed scaling analysis pipeline."""

from __future__ import annotations

import csv
import hashlib
import importlib.util
import json
import sys
import tempfile
import unittest
from pathlib import Path


HERE = Path(__file__).resolve().parent
SPEC = importlib.util.spec_from_file_location("ac_scaling_analyze", HERE / "analyze.py")
assert SPEC and SPEC.loader
ANALYZE = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = ANALYZE
SPEC.loader.exec_module(ANALYZE)


def common(run_uuid: str, pods: int, block: int, seed: int) -> dict[str, object]:
    return {
        "schema_version": 6,
        "run_id": f"test-p{pods}-b{block}",
        "run_uuid": run_uuid,
        "source_commit": "0" * 40,
        "source_dirty": False,
        "host": "fixture",
        "instance_id": None,
        "instance_type": None,
        "cloud_region": None,
        "os": "linux",
        "architecture": "aarch64",
        "rustc": "rustc fixture",
        "profile": "release",
        "features": "sparq-lws-core/default; sparq-solid/default",
        "measurement_profile": "timing",
        "campaign": "pod-scaling",
        "cell_label": f"pods-{pods}",
        "lane": "materialized-routed",
        "domain": "social",
        "topology": "origin-per-pod",
        "pods": pods,
        "documents_per_pod": 16,
        "triples_per_document": 8,
        "container_depth": 3,
        "own_acl_coverage_per_mille": 250,
        "public_per_mille": 0,
        "private_per_mille": 1000,
        "shared_per_mille": 0,
        "principal": "owner",
        "corpus_seed": seed,
        "corpus_hash_sha256": f"{pods:064x}",
        "process_block": block,
        "configuration_order": block,
        "configuration_order_seed": 20260903 + block,
        "warmups_configured": 10,
        "repetitions_configured": 30,
        "concurrency": 1,
        "rayon_threads": "1",
        "cpu_affinity": "1",
    }


def fixture_records() -> list[dict[str, object]]:
    records: list[dict[str, object]] = []
    for block, seed in enumerate((17, 42)):
        for pods in (1, 8):
            run_uuid = f"00000000-0000-4000-8000-{block:02d}{pods:010d}"
            base = common(run_uuid, pods, block, seed)
            records.append(
                {
                    **base,
                    "record_type": "applicability",
                    "utc_unix_ns": 1,
                    "query_id": "q1-point",
                    "query_family": "graph-bound point lookup",
                    "query_hash_sha256": "a" * 64,
                    "minimum_triples_per_document": 1,
                    "applicable": True,
                    "selected": True,
                    "reason": None,
                }
            )
            records.append(
                {
                    **base,
                    "record_type": "construction",
                    "utc_unix_ns": 2,
                    "content_documents": pods * 16,
                    "content_triples": pods * 128,
                    "container_graphs": pods * 3,
                    "control_documents": pods * 4,
                    "control_triples": pods * 20,
                    "total_source_graphs": pods * 23,
                    "target_readable_documents": 16,
                    "evaluation_readable_documents": 16,
                    "corpus_generation_ns": 100,
                    "graph_load_ns": 200,
                    "wac_materialization_ns": 300,
                    "route_index_ns": 10,
                    "lws_seed_ns": None,
                    "auth_triples_total": pods * 20,
                    "http_store_max_total_bytes": None,
                    "http_store_max_resource_count": None,
                    "construction_allocations": None,
                    "construction_allocated_bytes": None,
                    "resident_bytes": 4096,
                    "peak_resident_bytes": 8192,
                }
            )
            records.append(
                {
                    **base,
                    "record_type": "correctness-gate",
                    "utc_unix_ns": 3,
                    "gate": "fixture",
                    "principals_checked": 4,
                    "queries_checked": 1,
                    "exact_result_bags": True,
                }
            )
            for repetition in range(2):
                guarded_ns = 1_000_000 + repetition * 10_000 + (pods - 1) * 7_000
                for order, (operation, wall_ns) in enumerate(
                    (
                        ("guarded-query-as", guarded_ns),
                        ("plain-engine-reference", 900_000 + repetition * 10_000),
                    )
                ):
                    records.append(
                        {
                            **base,
                            "record_type": "observation",
                            "utc_unix_ns": 4 + repetition * 2 + order,
                            "query_id": "q1-point",
                            "query_family": "graph-bound point lookup",
                            "query_hash_sha256": "a" * 64,
                            "operation": operation,
                            "pair_id": f"{run_uuid}:{repetition}",
                            "repetition": repetition,
                            "warmup": False,
                            "order_in_pair": order,
                            "wall_ns": wall_ns,
                            "process_cpu_ns": wall_ns,
                            "allocation_operations": None,
                            "allocated_bytes": None,
                            "response_bytes": None,
                            "result_rows": 1,
                            "result_hash_sha256": "b" * 64,
                            "correctness": True,
                            "http_status": None,
                            "backend_sparql_queries": None,
                            "backend_sparql_updates": None,
                            "backend_blob_gets": None,
                            "backend_blob_puts": None,
                            "backend_blob_other": None,
                            "backend_total_operations": None,
                            "backend_max_in_flight": None,
                        }
                    )
    return records


def write_jsonl(path: Path, records: list[dict[str, object]]) -> None:
    payload = "".join(json.dumps(record, sort_keys=True) + "\n" for record in records)
    path.write_text(payload, encoding="utf-8")


class AnalyzeTests(unittest.TestCase):
    def test_canonical_analysis_requires_ten_thousand_bootstrap_draws(self) -> None:
        ANALYZE.validate_bootstrap_draws(10_000, True)
        ANALYZE.validate_bootstrap_draws(100, False)
        with self.assertRaisesRegex(ANALYZE.AnalysisError, "at least 10000"):
            ANALYZE.validate_bootstrap_draws(9_999, True)

    def test_canonical_campaign_profile_labels_fail_closed(self) -> None:
        observations = [
            {"campaign": campaign, "measurement_profile": profile}
            for campaign, profile in ANALYZE.CANONICAL_CAMPAIGN_PROFILES.items()
        ]
        ANALYZE.validate_canonical_campaign_profiles(observations)
        observations[0]["measurement_profile"] = "instrumentation"
        with self.assertRaisesRegex(ANALYZE.AnalysisError, "expected 'timing'"):
            ANALYZE.validate_canonical_campaign_profiles(observations)

    def test_secondary_profile_validator_checks_exact_fixtures(self) -> None:
        spec = ANALYZE.CanonicalCellSpec(
            "tiny",
            "social",
            2,
            3,
            4,
            1,
            250,
            0,
            1000,
            0,
            "owner",
            ("q1-point",),
        )
        observations = []
        for repetition in range(2):
            observations.append(
                {
                    "campaign": "tiny-campaign",
                    "measurement_profile": "timing",
                    "operation": "guarded-query-as",
                    "lane": "materialized-routed",
                    "domain": "social",
                    "cell_label": "tiny",
                    "process_block": 0,
                    "run_uuid": "tiny-run",
                    "pods": 2,
                    "documents_per_pod": 3,
                    "triples_per_document": 4,
                    "container_depth": 1,
                    "own_acl_coverage_per_mille": 250,
                    "public_per_mille": 0,
                    "private_per_mille": 1000,
                    "shared_per_mille": 0,
                    "principal": "owner",
                    "topology": "origin-per-pod",
                    "query_id": "q1-point",
                    "corpus_seed": 17,
                    "configuration_order_seed": 91,
                    "configuration_order": 0,
                    "repetition": repetition,
                }
            )
        constructions = [
            {
                "campaign": "tiny-campaign",
                "measurement_profile": "timing",
                "run_uuid": "tiny-run",
            }
        ]
        ANALYZE.validate_campaign_profile(
            observations,
            constructions,
            campaign="tiny-campaign",
            profile="timing",
            specs=(spec,),
            order_seed_base=91,
            repetitions=2,
            blocks=(0,),
            lanes={"materialized-routed"},
        )
        with self.assertRaisesRegex(ANALYZE.AnalysisError, "query counts"):
            ANALYZE.validate_campaign_profile(
                observations[:-1],
                constructions,
                campaign="tiny-campaign",
                profile="timing",
                specs=(spec,),
                order_seed_base=91,
                repetitions=2,
                blocks=(0,),
                lanes={"materialized-routed"},
            )

    def test_profile_matching_rejects_a_different_corpus(self) -> None:
        timing = next(
            record
            for record in fixture_records()
            if record["record_type"] == "observation"
            and record["operation"] == "guarded-query-as"
        )
        instrumented = dict(timing)
        instrumented.update(
            {
                "campaign": "pod-scaling-instrumentation",
                "measurement_profile": "instrumentation",
            }
        )
        ANALYZE.validate_profile_matching([timing, instrumented])
        instrumented["corpus_hash_sha256"] = "f" * 64
        with self.assertRaisesRegex(ANALYZE.AnalysisError, "corpus_hash_sha256"):
            ANALYZE.validate_profile_matching([timing, instrumented])

    def test_profile_matching_rejects_a_different_result(self) -> None:
        timing = next(
            record
            for record in fixture_records()
            if record["record_type"] == "observation"
            and record["operation"] == "guarded-query-as"
        )
        instrumented = dict(timing)
        instrumented.update(
            {
                "campaign": "pod-scaling-instrumentation",
                "measurement_profile": "instrumentation",
                "result_hash_sha256": "c" * 64,
            }
        )
        with self.assertRaisesRegex(ANALYZE.AnalysisError, "result_hash_sha256"):
            ANALYZE.validate_profile_matching([timing, instrumented])

    def test_primary_causal_invariants_cover_query_and_readable_state(self) -> None:
        records = fixture_records()
        observations = [
            record
            for record in records
            if record["record_type"] == "observation"
            and record["operation"] == "guarded-query-as"
        ]
        constructions = [
            record for record in records if record["record_type"] == "construction"
        ]
        ANALYZE.validate_primary_causal_invariants(
            observations, constructions, {1, 8}
        )

        changed_result = [dict(record) for record in observations]
        for record in changed_result:
            if record["pods"] == 8:
                record["result_hash_sha256"] = "c" * 64
        with self.assertRaisesRegex(ANALYZE.AnalysisError, "changes result"):
            ANALYZE.validate_primary_causal_invariants(
                changed_result, constructions, {1, 8}
            )

        changed_rows = [dict(record) for record in observations]
        for record in changed_rows:
            if record["pods"] == 8:
                record["result_rows"] = 2
        with self.assertRaisesRegex(ANALYZE.AnalysisError, "changes result"):
            ANALYZE.validate_primary_causal_invariants(
                changed_rows, constructions, {1, 8}
            )

        changed_query = [dict(record) for record in observations]
        for record in changed_query:
            if record["pods"] == 8:
                record["query_hash_sha256"] = "c" * 64
        with self.assertRaisesRegex(ANALYZE.AnalysisError, "changes query hash"):
            ANALYZE.validate_primary_causal_invariants(
                changed_query, constructions, {1, 8}
            )

        missing_level = [record for record in observations if record["pods"] == 1]
        with self.assertRaisesRegex(ANALYZE.AnalysisError, "Pod counts"):
            ANALYZE.validate_primary_causal_invariants(
                missing_level, constructions, {1, 8}
            )

        changed_construction = [dict(record) for record in constructions]
        for record in changed_construction:
            if record["pods"] == 8:
                record["evaluation_readable_documents"] = 15
        with self.assertRaisesRegex(ANALYZE.AnalysisError, "readable counts"):
            ANALYZE.validate_primary_causal_invariants(
                observations, changed_construction, {1, 8}
            )

    def test_primary_causal_invariants_keep_recorded_dimensions_separate(self) -> None:
        observations = []
        constructions = []
        for profile in ("timing", "instrumentation"):
            for lane in ("materialized-routed", "native-http-assembly"):
                for domain in ("social", "health"):
                    for block in (0, 1):
                        for pods in (1, 8):
                            run_uuid = f"{profile}:{lane}:{domain}:{block}:{pods}"
                            construction = {
                                "run_uuid": run_uuid,
                                "measurement_profile": profile,
                                "lane": lane,
                                "domain": domain,
                                "process_block": block,
                                "pods": pods,
                                "target_readable_documents": 16,
                                "evaluation_readable_documents": 16,
                            }
                            constructions.append(construction)
                            for query_id in ("q1-point", "q8-graph-scan"):
                                identity = f"{profile}:{lane}:{domain}:{query_id}:{block}"
                                observations.append(
                                    {
                                        "run_uuid": run_uuid,
                                        "measurement_profile": profile,
                                        "lane": lane,
                                        "domain": domain,
                                        "query_id": query_id,
                                        "process_block": block,
                                        "pods": pods,
                                        "documents_per_pod": 16,
                                        "query_hash_sha256": f"query:{identity}",
                                        "result_hash_sha256": f"result:{identity}",
                                        "result_rows": block + 1,
                                    }
                                )

        ANALYZE.validate_primary_causal_invariants(
            observations, constructions, {1, 8}
        )

    def test_valid_fixture_produces_h2_and_outputs(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = root / "raw.jsonl"
            write_jsonl(source, fixture_records())
            source.with_suffix(".jsonl.sha256").write_text(
                f"{hashlib.sha256(source.read_bytes()).hexdigest()}  {source}\n",
                encoding="utf-8",
            )
            loaded = ANALYZE.load([source], None, False)
            self.assertEqual(len(loaded.observations), 16)
            results = ANALYZE.analyze_h2(loaded.observations, 100, 7)
            self.assertEqual(len(results), 1)
            self.assertLess(results[0]["median_latency_ratio"], 1.10)
            self.assertLess(results[0]["median_process_cpu_ratio"], 1.10)
            self.assertIsInstance(results[0]["minimal_latency_scaling"], bool)
            self.assertIsInstance(results[0]["minimal_cpu_scaling"], bool)
            h4 = ANALYZE.analyze_h4(loaded.observations, 100, 7)
            self.assertEqual(len(h4), 2)
            self.assertTrue(all(row["process_blocks"] == 2 for row in h4))

            output = root / "derived"
            ANALYZE.write_summary(output / "summary.csv", loaded.observations)
            ANALYZE.write_overhead(output / "paired-overhead.csv", h4)
            ANALYZE.write_construction(output / "construction.csv", loaded.constructions)
            ANALYZE.write_h2(output / "h2.json", results, loaded.files)
            ANALYZE.write_scaling_svg(output / "scaling.svg", loaded.observations)
            self.assertTrue((output / "summary.csv").is_file())
            self.assertTrue((output / "construction.csv").is_file())
            self.assertTrue((output / "h2.json").is_file())
            self.assertTrue((output / "scaling.svg").is_file())
            with (output / "summary.csv").open(newline="", encoding="utf-8") as source:
                summary = list(csv.DictReader(source))
            self.assertTrue(all(row["process_cpu_median_ms"] for row in summary))
            with (output / "paired-overhead.csv").open(
                newline="", encoding="utf-8"
            ) as source:
                overhead = list(csv.DictReader(source))
            self.assertTrue(all(row["latency_ratio_ci95_low"] for row in overhead))
            self.assertTrue(all(row["latency_ratio_ci95_high"] for row in overhead))

    def test_summaries_do_not_pool_distinct_campaigns(self) -> None:
        records = [
            record
            for record in fixture_records()
            if record["record_type"] in {"observation", "construction"}
        ]
        copied = [dict(record) for record in records]
        for record in copied:
            record["campaign"] = "sensitivity"
            record["cell_label"] = "documents-16"
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory)
            observations = [
                record
                for record in (*records, *copied)
                if record["record_type"] == "observation"
            ]
            constructions = [
                record
                for record in (*records, *copied)
                if record["record_type"] == "construction"
            ]
            ANALYZE.write_summary(output / "summary.csv", observations)
            ANALYZE.write_construction(output / "construction.csv", constructions)
            with (output / "summary.csv").open(newline="", encoding="utf-8") as source:
                summary_rows = list(csv.DictReader(source))
            with (output / "construction.csv").open(newline="", encoding="utf-8") as source:
                construction_rows = list(csv.DictReader(source))
            self.assertEqual({row["campaign"] for row in summary_rows}, {"pod-scaling", "sensitivity"})
            self.assertEqual(
                {row["campaign"] for row in construction_rows},
                {"pod-scaling", "sensitivity"},
            )

    def test_pair_result_disagreement_is_rejected(self) -> None:
        records = fixture_records()
        observation = next(
            record
            for record in records
            if record["record_type"] == "observation"
            and record["operation"] == "plain-engine-reference"
        )
        observation["result_hash_sha256"] = "c" * 64
        with tempfile.TemporaryDirectory() as directory:
            source = Path(directory) / "raw.jsonl"
            write_jsonl(source, records)
            with self.assertRaisesRegex(ANALYZE.AnalysisError, "paired result disagreement"):
                ANALYZE.load([source], None, False)

    def test_result_drift_across_repetitions_is_rejected(self) -> None:
        records = fixture_records()
        for record in records:
            if record.get("record_type") == "observation" and record.get("repetition") == 1:
                record["result_hash_sha256"] = "c" * 64
        with tempfile.TemporaryDirectory() as directory:
            source = Path(directory) / "raw.jsonl"
            write_jsonl(source, records)
            with self.assertRaisesRegex(ANALYZE.AnalysisError, "changes result"):
                ANALYZE.load([source], None, False)

    def test_query_hash_must_match_applicability_metadata(self) -> None:
        records = fixture_records()
        for record in records:
            if record.get("record_type") == "observation":
                record["query_hash_sha256"] = "c" * 64
        with tempfile.TemporaryDirectory() as directory:
            source = Path(directory) / "raw.jsonl"
            write_jsonl(source, records)
            with self.assertRaisesRegex(ANALYZE.AnalysisError, "query hash"):
                ANALYZE.load([source], None, False)

    def test_missing_applicability_is_rejected(self) -> None:
        records = [
            record for record in fixture_records() if record["record_type"] != "applicability"
        ]
        with tempfile.TemporaryDirectory() as directory:
            source = Path(directory) / "raw.jsonl"
            write_jsonl(source, records)
            with self.assertRaisesRegex(ANALYZE.AnalysisError, "selected/applicable queries"):
                ANALYZE.load([source], None, False)

    def test_record_only_run_is_rejected(self) -> None:
        records = fixture_records()
        extra = dict(
            next(record for record in records if record["record_type"] == "construction")
        )
        extra["run_uuid"] = "00000000-0000-4000-8000-999999999999"
        extra["run_id"] = "record-only"
        records.append(extra)
        with tempfile.TemporaryDirectory() as directory:
            source = Path(directory) / "raw.jsonl"
            write_jsonl(source, records)
            with self.assertRaisesRegex(ANALYZE.AnalysisError, "lack observations"):
                ANALYZE.load([source], None, False)

    def test_checksum_mismatch_is_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            source = Path(directory) / "raw.jsonl"
            write_jsonl(source, fixture_records())
            source.with_suffix(".jsonl.sha256").write_text("0" * 64, encoding="utf-8")
            with self.assertRaisesRegex(ANALYZE.AnalysisError, "checksum mismatch"):
                ANALYZE.load([source], None, False)

    def test_canonical_input_requires_checksum_sidecar(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            source = Path(directory) / "raw.jsonl"
            write_jsonl(source, fixture_records())
            with self.assertRaisesRegex(ANALYZE.AnalysisError, "no checksum sidecar"):
                ANALYZE.load([source], None, True, "0" * 40)

    def test_canonical_record_is_pinned_to_expected_commit(self) -> None:
        record = fixture_records()[0]
        record.update(
            {
                "instance_id": "i-fixture",
                "instance_type": "c8g.4xlarge",
                "cloud_region": "eu-west-2",
            }
        )
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "raw.jsonl"
            with self.assertRaisesRegex(ANALYZE.AnalysisError, "expected commit"):
                ANALYZE.validate_canonical(record, path, 1, "1" * 40)

    def test_canonical_record_rejects_multi_cpu_affinity(self) -> None:
        record = fixture_records()[0]
        record.update(
            {
                "instance_id": "i-fixture",
                "instance_type": "r7g.xlarge",
                "cloud_region": "eu-west-2",
                "cpu_affinity": "0-3",
            }
        )
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "raw.jsonl"
            with self.assertRaisesRegex(ANALYZE.AnalysisError, "affinity"):
                ANALYZE.validate_canonical(record, path, 1, "0" * 40)

    def test_http_store_limit_must_match_emitted_corpus_count(self) -> None:
        construction = next(
            record
            for record in fixture_records()
            if record["record_type"] == "construction"
        )
        construction["lane"] = "native-http-assembly"
        construction["http_store_max_total_bytes"] = 4096
        construction["http_store_max_resource_count"] = (
            int(construction["total_source_graphs"]) + 1
        )
        ANALYZE.validate_constructions([construction])
        construction["http_store_max_resource_count"] = 1
        with self.assertRaisesRegex(ANALYZE.AnalysisError, "resource limit"):
            ANALYZE.validate_constructions([construction])

    def test_timing_profile_rejects_instrumentation_counters(self) -> None:
        observation = next(
            record
            for record in fixture_records()
            if record["record_type"] == "observation"
        )
        observation["allocation_operations"] = 1
        with tempfile.TemporaryDirectory() as directory:
            source = Path(directory) / "raw.jsonl"
            write_jsonl(source, [observation])
            with self.assertRaisesRegex(ANALYZE.AnalysisError, "timing profile reports"):
                ANALYZE.load([source], None, False)

    def test_h4_is_deterministic_under_input_reordering(self) -> None:
        observations = [
            record
            for record in fixture_records()
            if record["record_type"] == "observation"
        ]
        forward = ANALYZE.analyze_h4(observations, 250, 19)
        reverse = ANALYZE.analyze_h4(list(reversed(observations)), 250, 19)
        self.assertEqual(forward, reverse)
        self.assertEqual(
            {row["bootstrap_method"] for row in forward},
            {ANALYZE.H4_BOOTSTRAP_METHOD},
        )

    def test_h4_rejects_unbalanced_block_repetitions(self) -> None:
        observations = [
            record
            for record in fixture_records()
            if record["record_type"] == "observation"
            and not (
                record["process_block"] == 1
                and record["pods"] == 1
                and record["repetition"] == 1
            )
        ]
        with self.assertRaisesRegex(ANALYZE.AnalysisError, "unbalanced pair counts"):
            ANALYZE.analyze_h4(observations, 100, 7)

    def test_one_block_pilot_is_descriptive_without_interval(self) -> None:
        observations = [
            record
            for record in fixture_records()
            if record["record_type"] == "observation" and record["process_block"] == 0
        ]
        rows = ANALYZE.analyze_h4(observations, 100, 7)
        self.assertTrue(
            all(
                row["inference_status"] == "descriptive-only-insufficient-blocks"
                for row in rows
            )
        )
        self.assertTrue(all(row["bootstrap_draws"] == 0 for row in rows))
        self.assertTrue(all(row["latency_ratio_ci95_low"] is None for row in rows))
        self.assertTrue(all(row["latency_ratio_ci95_high"] is None for row in rows))

    def test_common_campaign_blocks_are_required_only_for_canonical_output(self) -> None:
        observations = [
            record
            for record in fixture_records()
            if record["record_type"] == "observation"
            and not (record["process_block"] == 1 and record["pods"] == 1)
        ]
        rows = ANALYZE.analyze_h4(observations, 100, 7)
        self.assertEqual(
            {row["inference_status"] for row in rows},
            {
                "descriptive-only-insufficient-blocks",
                "hierarchical-cluster-bootstrap",
            },
        )
        with self.assertRaisesRegex(ANALYZE.AnalysisError, "complete common blocks"):
            ANALYZE.analyze_h4(
                observations,
                100,
                7,
                require_complete_blocks=True,
            )

    def test_pair_metadata_disagreement_is_rejected(self) -> None:
        observations = [
            record
            for record in fixture_records()
            if record["record_type"] == "observation"
        ]
        plain = next(
            record
            for record in observations
            if record["operation"] == "plain-engine-reference"
        )
        plain["repetition"] = 99
        with self.assertRaisesRegex(ANALYZE.AnalysisError, "metadata disagreement"):
            ANALYZE.validate_pairs(observations)

    def test_h4_ignores_instrumentation_profile_times(self) -> None:
        observations = [
            record
            for record in fixture_records()
            if record["record_type"] == "observation"
        ]
        expected = ANALYZE.analyze_h4(observations, 100, 23)
        instrumentation = []
        for record in observations:
            copied = dict(record)
            copied["measurement_profile"] = "instrumentation"
            copied["campaign"] = f"{record['campaign']}-instrumentation"
            copied["wall_ns"] = int(record["wall_ns"]) * 1000
            copied["process_cpu_ns"] = int(record["process_cpu_ns"]) * 1000
            instrumentation.append(copied)
        self.assertEqual(
            ANALYZE.analyze_h4([*observations, *instrumentation], 100, 23),
            expected,
        )

    def test_output_artifacts_register_svg_bytes_and_digest(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory)
            svg = output / "figure.svg"
            svg.write_text("<svg/>\n", encoding="utf-8")
            artifacts = ANALYZE.output_artifacts(output, [svg.name])
            self.assertEqual(
                artifacts,
                [
                    {
                        "path": "figure.svg",
                        "bytes": len(b"<svg/>\n"),
                        "sha256": hashlib.sha256(b"<svg/>\n").hexdigest(),
                    }
                ],
            )

    def test_output_artifacts_fail_closed_for_missing_declared_file(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            with self.assertRaisesRegex(ANALYZE.AnalysisError, "does not exist"):
                ANALYZE.output_artifacts(Path(directory), ["missing.svg"])

    def test_main_manifest_registers_generated_svg_digest(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = root / "raw.jsonl"
            output = root / "derived"
            write_jsonl(source, fixture_records())
            saved_argv = sys.argv
            try:
                sys.argv = [
                    "analyze.py",
                    str(source),
                    "--out",
                    str(output),
                    "--bootstrap-draws",
                    "100",
                    "--bootstrap-seed",
                    "31",
                ]
                self.assertEqual(ANALYZE.main(), 0)
            finally:
                sys.argv = saved_argv
            manifest = json.loads((output / "manifest.json").read_text(encoding="utf-8"))
            artifacts = {
                artifact["path"]: artifact for artifact in manifest["output_artifacts"]
            }
            self.assertIn("pod-scaling-latency.svg", artifacts)
            svg = output / "pod-scaling-latency.svg"
            self.assertEqual(artifacts[svg.name]["bytes"], svg.stat().st_size)
            self.assertEqual(artifacts[svg.name]["sha256"], ANALYZE.sha256_file(svg))


if __name__ == "__main__":
    unittest.main()
