#!/usr/bin/env python3
"""Validate, summarize, bootstrap, and plot AC-SPARQL JSONL observations.

The implementation is standard-library-only so the same pinned script runs on a fresh
benchmark host.  It treats process/corpus blocks as clusters; repeated requests are never
mistaken for independent experiments.
"""

from __future__ import annotations

import argparse
import csv
import hashlib
import json
import math
import random
import re
import statistics
import sys
from collections import defaultdict
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Iterable, Sequence


SCHEMA_VERSION = 6
BOOTSTRAP_SEED = 20260903
MIN_CANONICAL_BOOTSTRAP_DRAWS = 10_000
H4_BOOTSTRAP_METHOD = "resample-corpus-process-blocks-then-intact-pairs"
GUARDED_OPERATION = {
    "materialized-routed": "guarded-query-as",
    "native-http-assembly": "native-http-request",
}
PLAIN_OPERATION = {
    "materialized-routed": "plain-engine-reference",
    "native-http-assembly": "plain-engine-json-reference",
}


class AnalysisError(RuntimeError):
    """A fail-closed raw-data or analysis invariant violation."""


@dataclass(frozen=True)
class Loaded:
    observations: list[dict[str, Any]]
    constructions: list[dict[str, Any]]
    correctness: list[dict[str, Any]]
    applicability: list[dict[str, Any]]
    files: tuple[Path, ...]


@dataclass(frozen=True)
class CanonicalCellSpec:
    """One independently specified secondary-campaign fixture."""

    label: str
    domain: str
    pods: int
    documents: int
    triples: int
    depth: int
    coverage: int
    public: int
    private: int
    shared: int
    principal: str
    query_ids: tuple[str, ...]


CANONICAL_BLOCK_SEEDS = {0: 17, 1: 42, 2: 101, 3: 314, 4: 2718}
CANONICAL_LANES = {"materialized-routed", "native-http-assembly"}
CANONICAL_CAMPAIGN_PROFILES = {
    "pod-scaling": "timing",
    "pod-scaling-instrumentation": "instrumentation",
    "sensitivity": "timing",
    "sensitivity-instrumentation": "instrumentation",
    "scenarios": "timing",
    "scenarios-instrumentation": "instrumentation",
}


COMMON_FIELDS = (
    "schema_version",
    "run_id",
    "run_uuid",
    "source_commit",
    "source_dirty",
    "host",
    "instance_id",
    "instance_type",
    "cloud_region",
    "os",
    "architecture",
    "rustc",
    "profile",
    "features",
    "measurement_profile",
    "campaign",
    "cell_label",
    "lane",
    "domain",
    "topology",
    "pods",
    "documents_per_pod",
    "triples_per_document",
    "container_depth",
    "own_acl_coverage_per_mille",
    "public_per_mille",
    "private_per_mille",
    "shared_per_mille",
    "principal",
    "corpus_seed",
    "corpus_hash_sha256",
    "process_block",
    "configuration_order",
    "configuration_order_seed",
    "warmups_configured",
    "repetitions_configured",
    "concurrency",
    "rayon_threads",
    "cpu_affinity",
)

REQUIRED_BY_KIND = {
    "applicability": (
        "query_id",
        "query_family",
        "query_hash_sha256",
        "applicable",
        "selected",
    ),
    "construction": (
        "corpus_generation_ns",
        "construction_allocations",
        "construction_allocated_bytes",
        "content_documents",
        "content_triples",
        "control_documents",
        "control_triples",
        "target_readable_documents",
        "evaluation_readable_documents",
        "http_store_max_total_bytes",
        "http_store_max_resource_count",
    ),
    "correctness-gate": (
        "principals_checked",
        "queries_checked",
        "exact_result_bags",
    ),
    "observation": (
        "query_id",
        "query_family",
        "query_hash_sha256",
        "operation",
        "pair_id",
        "repetition",
        "warmup",
        "order_in_pair",
        "wall_ns",
        "allocation_operations",
        "allocated_bytes",
        "result_rows",
        "result_hash_sha256",
        "correctness",
    ),
}


def arguments() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("inputs", nargs="+", type=Path, help="JSONL file or directory")
    parser.add_argument("--out", required=True, type=Path, help="derived-output directory")
    parser.add_argument("--run-prefix", help="retain only run_id values with this prefix")
    parser.add_argument("--bootstrap-draws", type=int, default=10_000)
    parser.add_argument("--bootstrap-seed", type=int, default=BOOTSTRAP_SEED)
    parser.add_argument(
        "--require-canonical",
        action="store_true",
        help="enforce the frozen five-block EC2 Pod-scaling campaign",
    )
    parser.add_argument(
        "--expected-commit",
        help="40-hex benchmark artefact commit (required with --require-canonical)",
    )
    return parser.parse_args()


def discover(inputs: Sequence[Path]) -> tuple[Path, ...]:
    files: set[Path] = set()
    for path in inputs:
        if path.is_dir():
            files.update(path.rglob("*.jsonl"))
        elif path.is_file():
            files.add(path)
        else:
            raise AnalysisError(f"input does not exist: {path}")
    selected = tuple(sorted(path for path in files if not path.name.endswith(".partial")))
    if not selected:
        raise AnalysisError("no JSONL inputs found")
    return selected


def sha256_file(path: Path) -> str:
    """Hash a file without loading a potentially large raw stream into memory."""

    digest = hashlib.sha256()
    with path.open("rb") as source:
        while chunk := source.read(1024 * 1024):
            digest.update(chunk)
    return digest.hexdigest()


def stable_seed(base_seed: int, namespace: str, key: Sequence[Any]) -> int:
    """Derive an input-order-independent RNG seed with an explicit algorithm."""

    material = json.dumps(
        [base_seed, namespace, list(key)],
        ensure_ascii=True,
        separators=(",", ":"),
    ).encode("utf-8")
    return int.from_bytes(hashlib.sha256(material).digest(), "big")


def verify_checksum(path: Path, *, required: bool = False) -> None:
    sidecar = Path(f"{path}.sha256")
    if not sidecar.exists():
        if required:
            raise AnalysisError(f"canonical input has no checksum sidecar: {path}")
        return
    expected = sidecar.read_text(encoding="utf-8").split()[0].lower()
    actual = sha256_file(path)
    if actual != expected:
        raise AnalysisError(f"checksum mismatch for {path}: {actual} != {expected}")


def load(
    inputs: Sequence[Path],
    run_prefix: str | None,
    require_canonical: bool,
    expected_commit: str | None = None,
) -> Loaded:
    observations: list[dict[str, Any]] = []
    constructions: list[dict[str, Any]] = []
    correctness: list[dict[str, Any]] = []
    applicability: list[dict[str, Any]] = []
    files = discover(inputs)
    for path in files:
        verify_checksum(path, required=require_canonical)
        with path.open(encoding="utf-8") as source:
            for line_number, line in enumerate(source, 1):
                if not line.strip():
                    continue
                try:
                    record = json.loads(line)
                except json.JSONDecodeError as error:
                    raise AnalysisError(f"{path}:{line_number}: {error}") from error
                if record.get("schema_version") != SCHEMA_VERSION:
                    raise AnalysisError(
                        f"{path}:{line_number}: unsupported schema {record.get('schema_version')}"
                    )
                validate_shape(record, path, line_number)
                if run_prefix and not str(record.get("run_id", "")).startswith(run_prefix):
                    continue
                if require_canonical:
                    validate_canonical(record, path, line_number, expected_commit)
                kind = record.get("record_type")
                if kind == "observation":
                    observations.append(record)
                elif kind == "construction":
                    constructions.append(record)
                elif kind == "correctness-gate":
                    correctness.append(record)
                elif kind == "applicability":
                    applicability.append(record)
    if not observations:
        raise AnalysisError("selection contains no observation records")
    validate_constructions(constructions)
    validate_pairs(observations)
    validate_runs(observations, constructions, correctness, applicability)
    if require_canonical:
        validate_canonical_campaign(observations, constructions)
    return Loaded(observations, constructions, correctness, applicability, files)


def validate_shape(record: dict[str, Any], path: Path, line_number: int) -> None:
    kind = record.get("record_type")
    if kind not in REQUIRED_BY_KIND:
        raise AnalysisError(f"{path}:{line_number}: unknown record_type {kind!r}")
    missing = [
        field
        for field in (*COMMON_FIELDS, "record_type", "utc_unix_ns", *REQUIRED_BY_KIND[kind])
        if field not in record
    ]
    if missing:
        raise AnalysisError(f"{path}:{line_number}: missing fields {missing}")
    if not isinstance(record["run_id"], str) or not record["run_id"]:
        raise AnalysisError(f"{path}:{line_number}: run_id must be a non-empty string")
    if not isinstance(record["run_uuid"], str) or not record["run_uuid"]:
        raise AnalysisError(f"{path}:{line_number}: run_uuid must be a non-empty string")
    if record["lane"] not in GUARDED_OPERATION:
        raise AnalysisError(f"{path}:{line_number}: unknown lane {record['lane']!r}")
    profile = record["measurement_profile"]
    if profile not in {"timing", "instrumentation"}:
        raise AnalysisError(f"{path}:{line_number}: unknown measurement profile {profile!r}")
    instrumented = profile == "instrumentation"
    feature_flag = "ac-query-scale-instrumentation" in str(record["features"])
    if feature_flag != instrumented:
        raise AnalysisError(
            f"{path}:{line_number}: measurement profile disagrees with compiled features"
        )
    if kind == "construction":
        for field in ("construction_allocations", "construction_allocated_bytes"):
            value = record[field]
            if instrumented:
                if not isinstance(value, int) or isinstance(value, bool) or value < 0:
                    raise AnalysisError(
                        f"{path}:{line_number}: instrumented {field} must be a non-negative integer"
                    )
            elif value is not None:
                raise AnalysisError(f"{path}:{line_number}: timing profile reports {field}")
    if kind == "observation":
        for field in ("allocation_operations", "allocated_bytes"):
            value = record[field]
            if instrumented:
                if not isinstance(value, int) or isinstance(value, bool) or value < 0:
                    raise AnalysisError(
                        f"{path}:{line_number}: instrumented {field} must be a non-negative integer"
                    )
            elif value is not None:
                raise AnalysisError(f"{path}:{line_number}: timing profile reports {field}")
        guarded_http = (
            record["lane"] == "native-http-assembly"
            and record["operation"] == GUARDED_OPERATION[record["lane"]]
        )
        backend_fields = (
            "backend_sparql_queries",
            "backend_sparql_updates",
            "backend_blob_gets",
            "backend_blob_puts",
            "backend_blob_other",
            "backend_total_operations",
            "backend_max_in_flight",
        )
        if instrumented and guarded_http:
            if any(record.get(field) is None for field in backend_fields):
                raise AnalysisError(
                    f"{path}:{line_number}: instrumented HTTP observation lacks backend counters"
                )
        elif any(record.get(field) is not None for field in backend_fields):
            raise AnalysisError(
                f"{path}:{line_number}: backend counters occur outside instrumented HTTP request"
            )


def validate_canonical(
    record: dict[str, Any],
    path: Path,
    line_number: int,
    expected_commit: str | None,
) -> None:
    problems = []
    commit = record.get("source_commit")
    if not isinstance(commit, str) or re.fullmatch(r"[0-9a-f]{40}", commit) is None:
        problems.append("source_commit is not a lowercase 40-hex Git object")
    if expected_commit is None or commit != expected_commit:
        problems.append(f"source_commit does not equal expected commit {expected_commit!r}")
    if record.get("source_dirty") is not False:
        problems.append("source_dirty is not false")
    if record.get("profile") != "release":
        problems.append("profile is not release")
    if not record.get("instance_id"):
        problems.append("instance_id is absent")
    if not record.get("instance_type"):
        problems.append("instance_type is absent")
    if not record.get("cloud_region"):
        problems.append("cloud_region is absent")
    if record.get("os") != "linux":
        problems.append("OS is not Linux")
    if record.get("configuration_order") is None:
        problems.append("configuration_order is absent")
    if record.get("configuration_order_seed") is None:
        problems.append("configuration_order_seed is absent")
    if record.get("concurrency") != 1:
        problems.append("concurrency is not 1")
    if record.get("rayon_threads") != "1":
        problems.append("RAYON_NUM_THREADS is not 1")
    if re.fullmatch(r"[0-9]+", str(record.get("cpu_affinity", ""))) is None:
        problems.append("process affinity is not exactly one logical CPU")
    profile = record.get("measurement_profile")
    expected_warmups = 10 if profile == "timing" else 1
    expected_repetitions = 30 if profile == "timing" else 1
    if record.get("warmups_configured") != expected_warmups:
        problems.append(f"configured warm-ups are not {expected_warmups}")
    if record.get("repetitions_configured") != expected_repetitions:
        problems.append(f"configured repetitions are not {expected_repetitions}")
    if record.get("record_type") == "construction":
        if record.get("resident_bytes") is None:
            problems.append("resident_bytes is absent")
        if record.get("peak_resident_bytes") is None:
            problems.append("peak_resident_bytes is absent")
    if record.get("record_type") == "observation":
        lane = record.get("lane")
        operation = record.get("operation")
        if operation not in {GUARDED_OPERATION.get(lane), PLAIN_OPERATION.get(lane)}:
            problems.append("operation is not valid for lane")
        if record.get("measurement_profile") == "timing":
            process_cpu_ns = record.get("process_cpu_ns")
            if (
                isinstance(process_cpu_ns, bool)
                or not isinstance(process_cpu_ns, int)
                or process_cpu_ns <= 0
            ):
                problems.append("process_cpu_ns is not a positive integer")
        if lane == "native-http-assembly" and operation == GUARDED_OPERATION[lane]:
            if record.get("http_status") != 200:
                problems.append("guarded HTTP status is not 200")
            if record.get("measurement_profile") == "instrumentation":
                if record.get("backend_total_operations") is None:
                    problems.append("guarded HTTP backend counter is absent")
                if record.get("backend_max_in_flight") != 1:
                    problems.append("guarded HTTP maximum in-flight backend calls is not 1")
    if problems:
        raise AnalysisError(f"{path}:{line_number}: non-canonical: {', '.join(problems)}")


def validate_canonical_campaign(
    observations: Sequence[dict[str, Any]],
    constructions: Sequence[dict[str, Any]],
) -> None:
    """Fail closed unless the prospective primary Pod-count campaign is complete."""

    validate_canonical_campaign_profiles(observations)
    expected_pods = {1, 8, 64, 512, 2048}
    expected_blocks = set(range(5))
    expected_lanes = set(GUARDED_OPERATION)
    expected_domains = {"social", "health"}
    expected_queries = {
        "q1-point",
        "q2-star",
        "q3-join",
        "q4-optional",
        "q5-not-exists",
        "q6-path",
        "q7-aggregate",
        "q8-graph-scan",
    }
    primary = pod_scaling_observations(observations)
    if not primary:
        raise AnalysisError("canonical selection contains no primary Pod-scaling observations")
    declared_primary = [
        record
        for record in observations
        if record["campaign"] == "pod-scaling"
        and record["measurement_profile"] == "timing"
        and record["operation"] == GUARDED_OPERATION[record["lane"]]
    ]
    if len(primary) != len(declared_primary):
        raise AnalysisError("canonical Pod-scaling campaign contains off-protocol observations")

    metadata_fields = (
        "source_commit",
        "host",
        "instance_id",
        "instance_type",
        "cloud_region",
        "os",
        "architecture",
        "rustc",
        "profile",
        "features",
        "rayon_threads",
        "cpu_affinity",
    )
    for field in metadata_fields:
        values = {record.get(field) for record in primary}
        if len(values) != 1:
            raise AnalysisError(f"canonical campaign mixes {field}: {sorted(map(str, values))}")

    observed_pods = {int(record["pods"]) for record in primary}
    observed_blocks = {int(record["process_block"]) for record in primary}
    observed_lanes = {str(record["lane"]) for record in primary}
    observed_domains = {str(record["domain"]) for record in primary}
    observed_queries = {str(record["query_id"]) for record in primary}
    for label, observed, expected in (
        ("Pod counts", observed_pods, expected_pods),
        ("process blocks", observed_blocks, expected_blocks),
        ("lanes", observed_lanes, expected_lanes),
        ("domains", observed_domains, expected_domains),
        ("queries", observed_queries, expected_queries),
    ):
        if observed != expected:
            raise AnalysisError(
                f"canonical campaign {label} {sorted(observed)} != {sorted(expected)}"
            )

    cell_keys = ("lane", "domain", "query_id", "pods", "process_block")
    cells = group_by(primary, cell_keys)
    expected_cell_count = (
        len(expected_lanes)
        * len(expected_domains)
        * len(expected_queries)
        * len(expected_pods)
        * len(expected_blocks)
    )
    if len(cells) != expected_cell_count:
        raise AnalysisError(
            f"canonical campaign has {len(cells)} guarded cells, expected {expected_cell_count}"
        )
    for key, records in cells.items():
        if len(records) != 30:
            raise AnalysisError(f"canonical cell {key} has {len(records)} requests, expected 30")
        if len({record["run_uuid"] for record in records}) != 1:
            raise AnalysisError(f"canonical cell {key} spans multiple process fixtures")

    runs: dict[str, dict[str, Any]] = {}
    for record in primary:
        runs.setdefault(record["run_uuid"], record)
    if len(runs) != 2 * 2 * 5 * 5:
        raise AnalysisError(f"canonical campaign has {len(runs)} fixtures, expected 100")
    for record in runs.values():
        block = int(record["process_block"])
        if int(record["corpus_seed"]) != CANONICAL_BLOCK_SEEDS[block]:
            raise AnalysisError(
                f"block {block} uses seed {record['corpus_seed']}, "
                f"expected {CANONICAL_BLOCK_SEEDS[block]}"
            )
        expected_schedule_seed = 20260903 + block
        if int(record["configuration_order_seed"]) != expected_schedule_seed:
            raise AnalysisError(
                f"block {block} schedule seed {record['configuration_order_seed']} "
                f"!= {expected_schedule_seed}"
            )
    for block in expected_blocks:
        block_runs = [record for record in runs.values() if record["process_block"] == block]
        orders = {int(record["configuration_order"]) for record in block_runs}
        if orders != set(range(20)):
            raise AnalysisError(
                f"block {block} configuration order {sorted(orders)} != 0..19"
            )

    construction_runs = {record["run_uuid"] for record in constructions}
    if set(runs) - construction_runs:
        raise AnalysisError("canonical Pod-scaling fixtures lack construction records")

    instrumented = pod_scaling_instrumentation_observations(observations)
    if not instrumented:
        raise AnalysisError("canonical selection contains no Pod-scaling instrumentation")
    declared_instrumented = [
        record
        for record in observations
        if record["campaign"] == "pod-scaling-instrumentation"
        and record["measurement_profile"] == "instrumentation"
        and record["operation"] == GUARDED_OPERATION[record["lane"]]
    ]
    if len(instrumented) != len(declared_instrumented):
        raise AnalysisError(
            "canonical Pod-scaling instrumentation contains off-protocol observations"
        )
    instrument_cells = group_by(instrumented, cell_keys)
    if len(instrument_cells) != expected_cell_count:
        raise AnalysisError(
            "canonical instrumentation has "
            f"{len(instrument_cells)} guarded cells, expected {expected_cell_count}"
        )
    for key, records in instrument_cells.items():
        if len(records) != 1:
            raise AnalysisError(
                f"canonical instrumentation cell {key} has {len(records)} requests, expected 1"
            )
        record = records[0]
        if record["lane"] == "native-http-assembly":
            if record.get("backend_total_operations") is None:
                raise AnalysisError(f"canonical instrumentation cell {key} lacks backend work")
            if record.get("backend_max_in_flight") != 1:
                raise AnalysisError(
                    f"canonical instrumentation cell {key} has non-sequential backend work"
                )
    instrument_runs: dict[str, dict[str, Any]] = {}
    for record in instrumented:
        instrument_runs.setdefault(record["run_uuid"], record)
    if len(instrument_runs) != 2 * 2 * 5 * 5:
        raise AnalysisError(
            f"canonical instrumentation has {len(instrument_runs)} fixtures, expected 100"
        )
    for record in instrument_runs.values():
        block = int(record["process_block"])
        if int(record["corpus_seed"]) != CANONICAL_BLOCK_SEEDS[block]:
            raise AnalysisError(
                f"instrumentation block {block} uses seed {record['corpus_seed']}, "
                f"expected {CANONICAL_BLOCK_SEEDS[block]}"
            )
        if int(record["configuration_order_seed"]) != 20260903 + block:
            raise AnalysisError(
                f"instrumentation block {block} has the wrong schedule seed"
            )
    for block in expected_blocks:
        block_runs = [
            record for record in instrument_runs.values() if record["process_block"] == block
        ]
        orders = {int(record["configuration_order"]) for record in block_runs}
        if orders != set(range(20)):
            raise AnalysisError(
                f"instrumentation block {block} configuration order {sorted(orders)} != 0..19"
            )
    if set(instrument_runs) - construction_runs:
        raise AnalysisError("canonical instrumentation fixtures lack construction records")

    validate_primary_causal_invariants(
        (*primary, *instrumented), constructions, expected_pods
    )
    validate_secondary_campaigns(observations, constructions)
    validate_profile_matching(observations)

    for field in (
        "source_commit",
        "host",
        "instance_id",
        "instance_type",
        "cloud_region",
        "os",
        "architecture",
        "rustc",
        "profile",
        "rayon_threads",
        "cpu_affinity",
    ):
        values = {record.get(field) for record in (*primary, *instrumented)}
        if len(values) != 1:
            raise AnalysisError(
                f"timing and instrumentation campaigns mix {field}: {sorted(map(str, values))}"
            )

    for field in (
        "source_commit",
        "host",
        "instance_id",
        "instance_type",
        "cloud_region",
        "os",
        "architecture",
        "rustc",
        "profile",
        "rayon_threads",
        "cpu_affinity",
    ):
        values = {record.get(field) for record in observations}
        if len(values) != 1:
            raise AnalysisError(
                f"canonical campaigns mix {field}: {sorted(map(str, values))}"
            )


def validate_canonical_campaign_profiles(
    observations: Sequence[dict[str, Any]],
) -> None:
    """Reject an unknown campaign or a profile hidden under the wrong campaign name."""

    observed_campaigns = {str(record["campaign"]) for record in observations}
    expected_campaigns = set(CANONICAL_CAMPAIGN_PROFILES)
    if observed_campaigns != expected_campaigns:
        raise AnalysisError(
            "canonical campaign names "
            f"{sorted(observed_campaigns)} != {sorted(expected_campaigns)}"
        )
    for record in observations:
        campaign = str(record["campaign"])
        expected_profile = CANONICAL_CAMPAIGN_PROFILES[campaign]
        if record["measurement_profile"] != expected_profile:
            raise AnalysisError(
                f"canonical campaign {campaign!r} contains "
                f"{record['measurement_profile']!r} profile observations; "
                f"expected {expected_profile!r}"
            )


def sensitivity_specs() -> tuple[CanonicalCellSpec, ...]:
    specs = []
    for domain in ("social", "health"):
        for documents in (1, 8, 32, 128, 512):
            specs.append(
                CanonicalCellSpec(
                    f"documents-{documents}",
                    domain,
                    16,
                    documents,
                    8,
                    3,
                    250,
                    0,
                    1000,
                    0,
                    "owner",
                    ("q1-point", "q8-graph-scan"),
                )
            )
        for triples in (1, 8, 32, 128):
            specs.append(
                CanonicalCellSpec(
                    f"triples-{triples}",
                    domain,
                    16,
                    32,
                    triples,
                    3,
                    250,
                    0,
                    1000,
                    0,
                    "owner",
                    ("q1-point", "q8-graph-scan"),
                )
            )
        for coverage in (0, 100, 250, 1000):
            for depth in (1, 3, 6):
                specs.append(
                    CanonicalCellSpec(
                        f"placement-{coverage}-{depth}",
                        domain,
                        16,
                        16,
                        8,
                        depth,
                        coverage,
                        0,
                        1000,
                        0,
                        "owner",
                        ("q1-point", "q8-graph-scan"),
                    )
                )
        for public in (10, 100, 500, 1000):
            specs.append(
                CanonicalCellSpec(
                    f"visibility-{public}",
                    domain,
                    1,
                    512,
                    8,
                    3,
                    250,
                    public,
                    1000 - public,
                    0,
                    "anonymous",
                    ("q1-point", "q8-graph-scan"),
                )
            )
    return tuple(specs)


def scenario_specs() -> tuple[CanonicalCellSpec, ...]:
    return (
        CanonicalCellSpec(
            "social-count-small", "social", 100, 103, 22, 3, 250, 100, 700, 200,
            "owner", ("q1-point",),
        ),
        CanonicalCellSpec(
            "social-count-anchor", "social", 1531, 103, 22, 3, 250, 100, 700, 200,
            "owner", ("q1-point",),
        ),
        CanonicalCellSpec(
            "health-compact-small", "health", 64, 1, 32, 1, 1000, 0, 1000, 0,
            "owner", ("q1-point",),
        ),
        CanonicalCellSpec(
            "health-compact-anchor", "health", 256, 1, 128, 1, 1000, 0, 1000, 0,
            "owner", ("q1-point",),
        ),
    )


def validate_campaign_profile(
    observations: Sequence[dict[str, Any]],
    constructions: Sequence[dict[str, Any]],
    *,
    campaign: str,
    profile: str,
    specs: Sequence[CanonicalCellSpec],
    order_seed_base: int,
    repetitions: int,
    blocks: Sequence[int] = tuple(range(5)),
    lanes: set[str] = CANONICAL_LANES,
) -> None:
    """Validate every fixture and repetition in one secondary campaign profile."""

    guarded = [
        record
        for record in observations
        if record["campaign"] == campaign
        and record["measurement_profile"] == profile
        and record["operation"] == GUARDED_OPERATION[record["lane"]]
    ]
    if not guarded:
        raise AnalysisError(f"canonical selection contains no {campaign} observations")

    spec_by_key = {(spec.domain, spec.label): spec for spec in specs}
    if len(spec_by_key) != len(specs):
        raise AnalysisError(f"internal duplicate cell specification for {campaign}")
    expected_fixtures = {
        (block, lane, spec.domain, spec.label)
        for block in blocks
        for lane in lanes
        for spec in specs
    }
    cells = group_by(
        guarded,
        ("process_block", "lane", "domain", "cell_label"),
    )
    observed_fixtures = {
        (int(block), str(lane), str(domain), str(label))
        for block, lane, domain, label in cells
    }
    if observed_fixtures != expected_fixtures:
        missing = sorted(expected_fixtures - observed_fixtures)
        extra = sorted(observed_fixtures - expected_fixtures)
        raise AnalysisError(
            f"canonical {campaign} fixtures differ: missing={missing[:5]}, extra={extra[:5]}"
        )

    run_ids = set()
    orders_by_block: dict[int, set[int]] = defaultdict(set)
    parameter_fields = (
        ("pods", "pods"),
        ("documents_per_pod", "documents"),
        ("triples_per_document", "triples"),
        ("container_depth", "depth"),
        ("own_acl_coverage_per_mille", "coverage"),
        ("public_per_mille", "public"),
        ("private_per_mille", "private"),
        ("shared_per_mille", "shared"),
        ("principal", "principal"),
    )
    for (block_value, lane, domain, label), records in cells.items():
        block = int(block_value)
        spec = spec_by_key[(str(domain), str(label))]
        fixture_runs = {str(record["run_uuid"]) for record in records}
        if len(fixture_runs) != 1:
            raise AnalysisError(
                f"canonical {campaign} fixture {(block, lane, domain, label)} "
                f"spans {len(fixture_runs)} runs"
            )
        run_uuid = next(iter(fixture_runs))
        if run_uuid in run_ids:
            raise AnalysisError(f"canonical {campaign} reuses run UUID {run_uuid}")
        run_ids.add(run_uuid)
        first = records[0]
        for record_field, spec_field in parameter_fields:
            if first[record_field] != getattr(spec, spec_field):
                raise AnalysisError(
                    f"canonical {campaign} {label} has {record_field}="
                    f"{first[record_field]!r}, expected {getattr(spec, spec_field)!r}"
                )
        expected_topology = (
            "origin-per-pod" if lane == "materialized-routed" else "shared-origin"
        )
        if first["topology"] != expected_topology:
            raise AnalysisError(
                f"canonical {campaign} {label} topology {first['topology']!r} "
                f"!= {expected_topology!r}"
            )
        query_counts: dict[str, int] = defaultdict(int)
        for record in records:
            query_counts[str(record["query_id"])] += 1
        expected_query_counts = {query_id: repetitions for query_id in spec.query_ids}
        if dict(query_counts) != expected_query_counts:
            raise AnalysisError(
                f"canonical {campaign} fixture {(block, lane, domain, label)} query counts "
                f"{dict(query_counts)} != {expected_query_counts}"
            )
        if int(first["corpus_seed"]) != CANONICAL_BLOCK_SEEDS[block]:
            raise AnalysisError(f"canonical {campaign} block {block} has the wrong corpus seed")
        if int(first["configuration_order_seed"]) != order_seed_base + block:
            raise AnalysisError(f"canonical {campaign} block {block} has the wrong order seed")
        orders_by_block[block].add(int(first["configuration_order"]))

    expected_orders = set(range(len(lanes) * len(specs)))
    for block in blocks:
        if orders_by_block[block] != expected_orders:
            raise AnalysisError(
                f"canonical {campaign} block {block} configuration order "
                f"{sorted(orders_by_block[block])} != 0..{len(expected_orders) - 1}"
            )
    construction_runs = {
        str(record["run_uuid"])
        for record in constructions
        if record["campaign"] == campaign and record["measurement_profile"] == profile
    }
    if run_ids != construction_runs:
        raise AnalysisError(
            f"canonical {campaign} construction fixtures do not equal observation fixtures"
        )


def validate_secondary_campaigns(
    observations: Sequence[dict[str, Any]],
    constructions: Sequence[dict[str, Any]],
) -> None:
    for campaign, profile, specs, order_seed_base, repetitions in (
        ("sensitivity", "timing", sensitivity_specs(), 20260913, 30),
        (
            "sensitivity-instrumentation",
            "instrumentation",
            sensitivity_specs(),
            20260913,
            1,
        ),
        ("scenarios", "timing", scenario_specs(), 20260923, 30),
        (
            "scenarios-instrumentation",
            "instrumentation",
            scenario_specs(),
            20260923,
            1,
        ),
    ):
        validate_campaign_profile(
            observations,
            constructions,
            campaign=campaign,
            profile=profile,
            specs=specs,
            order_seed_base=order_seed_base,
            repetitions=repetitions,
        )


def validate_profile_matching(observations: Sequence[dict[str, Any]]) -> None:
    """Require timing and instrumentation builds to use identical logical fixtures."""

    guarded = [
        record
        for record in observations
        if record["operation"] == GUARDED_OPERATION[record["lane"]]
    ]
    groups: dict[tuple[Any, ...], dict[str, dict[str, Any]]] = defaultdict(dict)
    for record in guarded:
        base_campaign = str(record["campaign"]).removesuffix("-instrumentation")
        key = (
            base_campaign,
            record["process_block"],
            record["lane"],
            record["domain"],
            record["cell_label"],
            record["query_id"],
        )
        groups[key].setdefault(str(record["measurement_profile"]), record)

    match_fields = (
        "corpus_hash_sha256",
        "query_hash_sha256",
        "result_hash_sha256",
        "result_rows",
        "pods",
        "documents_per_pod",
        "triples_per_document",
        "container_depth",
        "own_acl_coverage_per_mille",
        "public_per_mille",
        "private_per_mille",
        "shared_per_mille",
        "principal",
        "topology",
        "corpus_seed",
        "configuration_order",
        "configuration_order_seed",
    )
    for key, profiles in groups.items():
        if set(profiles) != {"timing", "instrumentation"}:
            raise AnalysisError(
                f"canonical logical cell {key} has profiles {sorted(profiles)}, expected both"
            )
        timing = profiles["timing"]
        instrumentation = profiles["instrumentation"]
        mismatched = [
            field for field in match_fields if timing.get(field) != instrumentation.get(field)
        ]
        if mismatched:
            raise AnalysisError(
                f"canonical timing/instrumentation cell {key} differs in {mismatched}"
            )


def validate_primary_causal_invariants(
    observations: Sequence[dict[str, Any]],
    constructions: Sequence[dict[str, Any]],
    expected_pods: set[int],
) -> None:
    """Require the primary unrelated-Pod intervention to hold its target work fixed.

    The canonical primary cells use the Pod-0 owner with an all-private corpus. Hence
    adding Pods may change the full corpus hash and stored-state counts, but it may not
    change the query, answer multiset, or readable-document count for evaluation. Keep
    lanes, domains, profiles, queries, and seed/process blocks separate because their
    IRIs, vocabularies, execution paths, and generated target slices may differ.
    """

    query_groups = group_by(
        observations,
        ("measurement_profile", "lane", "domain", "query_id", "process_block"),
    )
    for key, records in query_groups.items():
        pods = {int(record["pods"]) for record in records}
        if pods != expected_pods:
            raise AnalysisError(
                f"primary causal query group {key} has Pod counts "
                f"{sorted(pods)}, expected {sorted(expected_pods)}"
            )
        query_hashes = {str(record["query_hash_sha256"]) for record in records}
        if len(query_hashes) != 1:
            raise AnalysisError(
                f"primary causal query group {key} changes query hash across Pod counts"
            )
        outcomes = {
            (str(record["result_hash_sha256"]), int(record["result_rows"]))
            for record in records
        }
        if len(outcomes) != 1:
            raise AnalysisError(
                f"primary causal query group {key} changes result across Pod counts"
            )

    construction_by_run = {
        str(record["run_uuid"]): record for record in constructions
    }
    run_representatives: dict[str, dict[str, Any]] = {}
    for record in observations:
        run_representatives.setdefault(str(record["run_uuid"]), record)

    primary_constructions: list[dict[str, Any]] = []
    for run_uuid, observation in run_representatives.items():
        construction = construction_by_run.get(run_uuid)
        if construction is None:
            raise AnalysisError(
                f"primary causal fixture {run_uuid} lacks a construction record"
            )
        expected_readable = int(observation["documents_per_pod"])
        readable = (
            int(construction["target_readable_documents"]),
            int(construction["evaluation_readable_documents"]),
        )
        if readable != (expected_readable, expected_readable):
            raise AnalysisError(
                f"primary all-private owner fixture {run_uuid} has readable counts "
                f"{readable}, expected {(expected_readable, expected_readable)}"
            )
        primary_constructions.append(construction)

    construction_groups = group_by(
        primary_constructions,
        ("measurement_profile", "lane", "domain", "process_block"),
    )
    for key, records in construction_groups.items():
        pods = {int(record["pods"]) for record in records}
        if pods != expected_pods:
            raise AnalysisError(
                f"primary causal construction group {key} has Pod counts "
                f"{sorted(pods)}, expected {sorted(expected_pods)}"
            )
        readable_counts = {
            (
                int(record["target_readable_documents"]),
                int(record["evaluation_readable_documents"]),
            )
            for record in records
        }
        if len(readable_counts) != 1:
            raise AnalysisError(
                f"primary causal construction group {key} changes readable counts "
                "across Pod counts"
            )


def validate_runs(
    observations: Sequence[dict[str, Any]],
    constructions: Sequence[dict[str, Any]],
    correctness: Sequence[dict[str, Any]],
    applicability: Sequence[dict[str, Any]],
) -> None:
    observations_by_run = group_by(observations, ("run_uuid",))
    constructions_by_run = group_by(constructions, ("run_uuid",))
    correctness_by_run = group_by(correctness, ("run_uuid",))
    applicability_by_run = group_by(applicability, ("run_uuid",))
    observed_runs = {record["run_uuid"] for record in observations}
    constructed_runs = {record["run_uuid"] for record in constructions}
    correctness_runs = {record["run_uuid"] for record in correctness}
    applicability_runs = {record["run_uuid"] for record in applicability}
    correct_runs = {
        record["run_uuid"]
        for record in correctness
        if record.get("exact_result_bags") is True
    }
    missing_construction = observed_runs - constructed_runs
    missing_correctness = observed_runs - correct_runs
    if missing_construction:
        raise AnalysisError(f"runs lack construction records: {sorted(missing_construction)}")
    if missing_correctness:
        raise AnalysisError(f"runs lack passing correctness gates: {sorted(missing_correctness)}")
    for label, related_runs in (
        ("construction", constructed_runs),
        ("correctness", correctness_runs),
        ("applicability", applicability_runs),
    ):
        extra = related_runs - observed_runs
        if extra:
            raise AnalysisError(f"{label} records lack observations: {sorted(extra)}")
    for run_uuid in observed_runs:
        key = (run_uuid,)
        run_observations = observations_by_run[key]
        run_constructions = constructions_by_run[key]
        run_correctness = correctness_by_run[key]
        run_applicability = applicability_by_run[key]
        if len(run_constructions) != 1:
            raise AnalysisError(
                f"run {run_uuid} has {len(run_constructions)} construction records, expected 1"
            )
        if len(run_correctness) != 1:
            raise AnalysisError(
                f"run {run_uuid} has {len(run_correctness)} correctness records, expected 1"
            )
        records = [
            *run_observations,
            *run_constructions,
            *run_correctness,
            *run_applicability,
        ]
        baseline = tuple(records[0][field] for field in COMMON_FIELDS)
        if any(tuple(record[field] for field in COMMON_FIELDS) != baseline for record in records[1:]):
            raise AnalysisError(f"common metadata changes within run {run_uuid}")
        selected = {
            record["query_id"]
            for record in run_applicability
            if record["selected"] is True and record["applicable"] is True
        }
        observed_queries = {record["query_id"] for record in run_observations}
        if selected != observed_queries:
            raise AnalysisError(
                f"run {run_uuid} selected/applicable queries {sorted(selected)} "
                f"do not equal observed queries {sorted(observed_queries)}"
            )
        applicability_hashes = {
            record["query_id"]: record["query_hash_sha256"]
            for record in run_applicability
        }
        for record in run_observations:
            expected_hash = applicability_hashes.get(record["query_id"])
            if record["query_hash_sha256"] != expected_hash:
                raise AnalysisError(
                    f"run {run_uuid} query hash disagrees with applicability metadata"
                )
        result_identities = group_by(
            run_observations,
            ("query_id",),
        )
        for query, query_records in result_identities.items():
            outcomes = {
                (record["result_hash_sha256"], record["result_rows"])
                for record in query_records
            }
            if len(outcomes) != 1:
                raise AnalysisError(
                    f"run {run_uuid} query {query[0]} changes result across repetitions"
                )
    for record in observations:
        if record.get("correctness") is not True:
            raise AnalysisError(f"incorrect observation emitted: {identity(record)}")
        if int(record.get("wall_ns", 0)) <= 0:
            raise AnalysisError(f"non-positive latency: {identity(record)}")
        if record.get("warmup") is not False:
            raise AnalysisError(f"warm-up record entered primary observations: {identity(record)}")


def validate_constructions(constructions: Sequence[dict[str, Any]]) -> None:
    for record in constructions:
        target = int(record["target_readable_documents"])
        evaluation = int(record["evaluation_readable_documents"])
        total = int(record["content_documents"])
        if not 0 <= target <= evaluation <= total:
            raise AnalysisError(
                f"invalid readable-document counts for run {record['run_uuid']}: "
                f"target={target}, evaluation={evaluation}, total={total}"
            )
        byte_limit = record.get("http_store_max_total_bytes")
        resource_limit = record.get("http_store_max_resource_count")
        if record["lane"] == "native-http-assembly":
            expected_resources = int(record["total_source_graphs"]) + 1
            if resource_limit != expected_resources:
                raise AnalysisError(
                    f"HTTP store resource limit for run {record['run_uuid']} is "
                    f"{resource_limit}, expected {expected_resources}"
                )
            if not isinstance(byte_limit, int) or isinstance(byte_limit, bool) or byte_limit <= 0:
                raise AnalysisError(
                    f"HTTP store byte limit for run {record['run_uuid']} is not positive"
                )
        elif byte_limit is not None or resource_limit is not None:
            raise AnalysisError(
                f"materialized run {record['run_uuid']} unexpectedly reports HTTP store limits"
            )


def identity(record: dict[str, Any]) -> str:
    return "/".join(
        str(record.get(key, "?"))
        for key in ("run_id", "query_id", "repetition", "operation")
    )


def validate_pairs(observations: Sequence[dict[str, Any]]) -> None:
    pairs: dict[tuple[str, str], list[dict[str, Any]]] = defaultdict(list)
    seen: set[tuple[str, str, str]] = set()
    for record in observations:
        unique = (record["run_uuid"], record["pair_id"], record["operation"])
        if unique in seen:
            raise AnalysisError(f"duplicate observation: {unique}")
        seen.add(unique)
        pairs[(record["run_uuid"], record["pair_id"])].append(record)
    for pair, records in pairs.items():
        if len(records) != 2:
            raise AnalysisError(f"incomplete pair {pair}: {len(records)} records")
        lane = records[0]["lane"]
        if any(record["lane"] != lane for record in records):
            raise AnalysisError(f"cross-lane pair {pair}")
        pair_fields = ("query_id", "query_family", "query_hash_sha256", "repetition")
        baseline = tuple(records[0][field] for field in pair_fields)
        if any(tuple(record[field] for field in pair_fields) != baseline for record in records[1:]):
            raise AnalysisError(f"paired request metadata disagreement in {pair}")
        expected = {GUARDED_OPERATION[lane], PLAIN_OPERATION[lane]}
        operations = {record["operation"] for record in records}
        if operations != expected:
            raise AnalysisError(f"wrong operations in {pair}: {operations} != {expected}")
        hashes = {record["result_hash_sha256"] for record in records}
        row_counts = {record["result_rows"] for record in records}
        if len(hashes) != 1 or len(row_counts) != 1:
            raise AnalysisError(f"paired result disagreement in {pair}")
        orders = {record["order_in_pair"] for record in records}
        if orders != {0, 1}:
            raise AnalysisError(f"invalid pair order in {pair}: {orders}")
        cpu_presence = {record.get("process_cpu_ns") is not None for record in records}
        if len(cpu_presence) != 1:
            raise AnalysisError(f"paired CPU-time availability disagreement in {pair}")


def percentile(values: Sequence[float], probability: float) -> float:
    if not values:
        raise AnalysisError("percentile of empty sequence")
    ordered = sorted(values)
    if len(ordered) == 1:
        return float(ordered[0])
    position = probability * (len(ordered) - 1)
    lower = math.floor(position)
    upper = math.ceil(position)
    if lower == upper:
        return float(ordered[lower])
    weight = position - lower
    return float(ordered[lower] * (1.0 - weight) + ordered[upper] * weight)


def descriptive(values: Sequence[float]) -> dict[str, float | int]:
    if not values:
        raise AnalysisError("descriptive statistics of empty sequence")
    median = statistics.median(values)
    deviations = [abs(value - median) for value in values]
    return {
        "n": len(values),
        "median": float(median),
        "p95": percentile(values, 0.95),
        "p99": percentile(values, 0.99),
        "mad": float(statistics.median(deviations)),
        "min": float(min(values)),
        "max": float(max(values)),
    }


SUMMARY_KEYS = (
    "campaign",
    "measurement_profile",
    "cell_label",
    "lane",
    "domain",
    "topology",
    "pods",
    "documents_per_pod",
    "triples_per_document",
    "container_depth",
    "own_acl_coverage_per_mille",
    "public_per_mille",
    "private_per_mille",
    "shared_per_mille",
    "principal",
    "query_id",
    "query_family",
    "operation",
)


def group_by(
    records: Iterable[dict[str, Any]], keys: Sequence[str]
) -> dict[tuple[Any, ...], list[dict[str, Any]]]:
    groups: dict[tuple[Any, ...], list[dict[str, Any]]] = defaultdict(list)
    for record in records:
        groups[tuple(record.get(key) for key in keys)].append(record)
    return groups


def write_summary(path: Path, observations: Sequence[dict[str, Any]]) -> None:
    rows = []
    for key, records in sorted(group_by(observations, SUMMARY_KEYS).items()):
        latency = descriptive([record["wall_ns"] / 1_000_000 for record in records])
        allocation_values = [
            float(record["allocation_operations"])
            for record in records
            if record.get("allocation_operations") is not None
        ]
        allocated_byte_values = [
            float(record["allocated_bytes"])
            for record in records
            if record.get("allocated_bytes") is not None
        ]
        backend = [
            float(record["backend_total_operations"])
            for record in records
            if record.get("backend_total_operations") is not None
        ]
        process_cpu = [
            record["process_cpu_ns"] / 1_000_000
            for record in records
            if record.get("process_cpu_ns") is not None
        ]
        cpu = descriptive(process_cpu) if process_cpu else None
        row = dict(zip(SUMMARY_KEYS, key, strict=True))
        row.update(
            {
                "requests": latency["n"],
                "run_uuids": len({record["run_uuid"] for record in records}),
                "process_blocks": len(
                    {(record["corpus_seed"], record["process_block"]) for record in records}
                ),
                "latency_median_ms": latency["median"],
                "latency_p95_ms": latency["p95"],
                "latency_p99_ms": latency["p99"],
                "latency_mad_ms": latency["mad"],
                "process_cpu_median_ms": cpu["median"] if cpu else "",
                "process_cpu_p95_ms": cpu["p95"] if cpu else "",
                "process_cpu_p99_ms": cpu["p99"] if cpu else "",
                "process_cpu_mad_ms": cpu["mad"] if cpu else "",
                "allocation_operations_median": (
                    statistics.median(allocation_values) if allocation_values else ""
                ),
                "allocated_bytes_median": (
                    statistics.median(allocated_byte_values) if allocated_byte_values else ""
                ),
                "backend_operations_median": statistics.median(backend) if backend else "",
                "result_rows": records[0]["result_rows"],
            }
        )
        rows.append(row)
    write_csv(path, rows)


H4_CELL_KEYS = SUMMARY_KEYS[:-1]


def h4_paired_effects(
    observations: Sequence[dict[str, Any]],
) -> dict[tuple[Any, ...], list[dict[str, Any]]]:
    """Form one effect per intact guarded/plain pair from timing-profile records."""

    timing = [record for record in observations if record["measurement_profile"] == "timing"]
    if not timing:
        raise AnalysisError("H4 has no timing-profile observations")
    validate_pairs(timing)
    pairs = group_by(timing, ("run_uuid", "pair_id"))
    by_cell: dict[tuple[Any, ...], list[dict[str, Any]]] = defaultdict(list)
    for pair_key, records in sorted(pairs.items()):
        lane = records[0]["lane"]
        guarded = next(
            record for record in records if record["operation"] == GUARDED_OPERATION[lane]
        )
        plain = next(
            record for record in records if record["operation"] == PLAIN_OPERATION[lane]
        )
        guarded_wall = float(guarded["wall_ns"])
        plain_wall = float(plain["wall_ns"])
        if guarded_wall <= 0 or plain_wall <= 0:
            raise AnalysisError(f"H4 pair {pair_key} has non-positive wall time")
        guarded_cpu = guarded.get("process_cpu_ns")
        plain_cpu = plain.get("process_cpu_ns")
        if (guarded_cpu is None) != (plain_cpu is None):
            raise AnalysisError(f"H4 pair {pair_key} has inconsistent CPU-time availability")
        if guarded_cpu is not None and (float(guarded_cpu) <= 0 or float(plain_cpu) <= 0):
            raise AnalysisError(f"H4 pair {pair_key} has non-positive CPU time")
        effect = {
            "cluster": (int(guarded["corpus_seed"]), int(guarded["process_block"])),
            "run_uuid": str(guarded["run_uuid"]),
            "pair_id": str(guarded["pair_id"]),
            "repetition": int(guarded["repetition"]),
            "latency_ratio": guarded_wall / plain_wall,
            "latency_difference_ms": (guarded_wall - plain_wall) / 1_000_000,
            "cpu_ratio": (
                float(guarded_cpu) / float(plain_cpu) if guarded_cpu is not None else None
            ),
            "cpu_difference_ms": (
                (float(guarded_cpu) - float(plain_cpu)) / 1_000_000
                if guarded_cpu is not None
                else None
            ),
        }
        by_cell[tuple(guarded[key] for key in H4_CELL_KEYS)].append(effect)
    return by_cell


def validate_h4_clusters(
    cell_key: tuple[Any, ...], effects: Sequence[dict[str, Any]]
) -> dict[tuple[int, int], list[dict[str, Any]]]:
    """Require balanced, independently rebuilt blocks with complete repetition indices."""

    clusters: dict[tuple[int, int], list[dict[str, Any]]] = defaultdict(list)
    for effect in effects:
        clusters[effect["cluster"]].append(effect)
    counts = {len(values) for values in clusters.values()}
    if len(counts) != 1:
        raise AnalysisError(f"H4 cell {cell_key} has unbalanced pair counts by block")
    repetitions = next(iter(counts))
    expected_repetitions = set(range(repetitions))
    ordered: dict[tuple[int, int], list[dict[str, Any]]] = {}
    for cluster, values in sorted(clusters.items()):
        run_uuids = {value["run_uuid"] for value in values}
        if len(run_uuids) != 1:
            raise AnalysisError(f"H4 cell {cell_key} block {cluster} spans multiple runs")
        observed_repetitions = {value["repetition"] for value in values}
        if observed_repetitions != expected_repetitions:
            raise AnalysisError(
                f"H4 cell {cell_key} block {cluster} repetition indices "
                f"{sorted(observed_repetitions)} != 0..{repetitions - 1}"
            )
        ordered[cluster] = sorted(
            values, key=lambda value: (value["repetition"], value["pair_id"])
        )
    return ordered


def bootstrap_paired_medians(
    clusters: dict[tuple[int, int], list[dict[str, Any]]],
    metrics: Sequence[str],
    draws: int,
    rng: random.Random,
) -> dict[str, tuple[float, float]]:
    """Resample each paired-effect vector once and return marginal median intervals."""

    if draws < 100:
        raise AnalysisError("H4 bootstrap requires at least 100 draws")
    cluster_keys = sorted(clusters)
    if len(cluster_keys) < 2:
        raise AnalysisError("H4 bootstrap requires at least two corpus/process blocks")
    estimates: dict[str, list[float]] = {metric: [] for metric in metrics}
    for _ in range(draws):
        sample: list[dict[str, Any]] = []
        for selected_key in rng.choices(cluster_keys, k=len(cluster_keys)):
            selected = clusters[selected_key]
            sample.extend(rng.choices(selected, k=len(selected)))
        for metric in metrics:
            estimates[metric].append(
                float(statistics.median(float(effect[metric]) for effect in sample))
            )
    return {
        metric: (percentile(values, 0.025), percentile(values, 0.975))
        for metric, values in estimates.items()
    }


def summarize_h4_metric(
    clusters: dict[tuple[int, int], list[dict[str, Any]]],
    metric: str,
    interval: tuple[float, float] | None,
) -> dict[str, float | None]:
    numeric_values = [
        float(effect[metric]) for effects in clusters.values() for effect in effects
    ]
    return {
        "median": float(statistics.median(numeric_values)),
        "p95": percentile(numeric_values, 0.95),
        "ci95_low": interval[0] if interval else None,
        "ci95_high": interval[1] if interval else None,
    }


def analyze_h4(
    observations: Sequence[dict[str, Any]],
    draws: int,
    seed: int,
    *,
    require_complete_blocks: bool = False,
) -> list[dict[str, Any]]:
    """Estimate H4 paired medians and hierarchical percentile intervals by timing cell."""

    cells = h4_paired_effects(observations)
    rows: list[dict[str, Any]] = []
    campaign_clusters: dict[str, set[tuple[int, int]]] = {}
    for key, effects in sorted(cells.items()):
        clusters = validate_h4_clusters(key, effects)
        campaign = str(key[0])
        observed_clusters = set(clusters)
        if (
            require_complete_blocks
            and campaign in campaign_clusters
            and campaign_clusters[campaign] != observed_clusters
        ):
            raise AnalysisError(f"H4 campaign {campaign} does not have complete common blocks")
        campaign_clusters.setdefault(campaign, observed_clusters)
        cpu_presence = {
            effect["cpu_ratio"] is not None
            for effects in clusters.values()
            for effect in effects
        }
        if len(cpu_presence) != 1:
            raise AnalysisError(f"H4 CPU time is missing for only part of cell {key}")
        metrics = ["latency_ratio", "latency_difference_ms"]
        if cpu_presence == {True}:
            metrics.extend(("cpu_ratio", "cpu_difference_ms"))
        inferential = len(clusters) >= 2
        intervals: dict[str, tuple[float, float] | None]
        if inferential:
            intervals = bootstrap_paired_medians(
                clusters,
                metrics,
                draws,
                random.Random(stable_seed(seed, "h4:paired-effect-vector", key)),
            )
        else:
            intervals = {metric: None for metric in metrics}
        latency_ratio = summarize_h4_metric(
            clusters, "latency_ratio", intervals["latency_ratio"]
        )
        latency_difference = summarize_h4_metric(
            clusters, "latency_difference_ms", intervals["latency_difference_ms"]
        )
        cpu_ratio = (
            summarize_h4_metric(clusters, "cpu_ratio", intervals["cpu_ratio"])
            if "cpu_ratio" in intervals
            else None
        )
        cpu_difference = (
            summarize_h4_metric(
                clusters, "cpu_difference_ms", intervals["cpu_difference_ms"]
            )
            if "cpu_difference_ms" in intervals
            else None
        )
        pairs_per_block = len(next(iter(clusters.values())))
        row = dict(zip(H4_CELL_KEYS, key, strict=True))
        row.update(
            {
                "pairs": len(effects),
                "process_blocks": len(clusters),
                "pairs_per_process_block": pairs_per_block,
                "inference_status": (
                    "hierarchical-cluster-bootstrap"
                    if inferential
                    else "descriptive-only-insufficient-blocks"
                ),
                "bootstrap_draws": draws if inferential else 0,
                "bootstrap_seed": seed if inferential else "",
                "bootstrap_method": H4_BOOTSTRAP_METHOD if inferential else "",
                "latency_ratio_median": latency_ratio["median"],
                "latency_ratio_p95": latency_ratio["p95"],
                "latency_ratio_ci95_low": latency_ratio["ci95_low"],
                "latency_ratio_ci95_high": latency_ratio["ci95_high"],
                "latency_difference_median_ms": latency_difference["median"],
                "latency_difference_p95_ms": latency_difference["p95"],
                "latency_difference_ci95_low_ms": latency_difference["ci95_low"],
                "latency_difference_ci95_high_ms": latency_difference["ci95_high"],
                "cpu_ratio_median": cpu_ratio["median"] if cpu_ratio else "",
                "cpu_ratio_ci95_low": cpu_ratio["ci95_low"] if cpu_ratio else "",
                "cpu_ratio_ci95_high": cpu_ratio["ci95_high"] if cpu_ratio else "",
                "cpu_difference_median_ms": (
                    cpu_difference["median"] if cpu_difference else ""
                ),
                "cpu_difference_ci95_low_ms": (
                    cpu_difference["ci95_low"] if cpu_difference else ""
                ),
                "cpu_difference_ci95_high_ms": (
                    cpu_difference["ci95_high"] if cpu_difference else ""
                ),
            }
        )
        rows.append(row)
    return rows


def write_overhead(path: Path, rows: Sequence[dict[str, Any]]) -> None:
    write_csv(path, rows)


CONSTRUCTION_KEYS = (
    "campaign",
    "measurement_profile",
    "cell_label",
    "lane",
    "domain",
    "topology",
    "pods",
    "documents_per_pod",
    "triples_per_document",
    "container_depth",
    "own_acl_coverage_per_mille",
    "public_per_mille",
    "private_per_mille",
    "shared_per_mille",
    "principal",
)


def write_construction(path: Path, records: Sequence[dict[str, Any]]) -> None:
    rows = []
    for key, cell in sorted(group_by(records, CONSTRUCTION_KEYS).items()):
        row = dict(zip(CONSTRUCTION_KEYS, key, strict=True))
        for field in (
            "content_documents",
            "content_triples",
            "container_graphs",
            "control_documents",
            "control_triples",
            "total_source_graphs",
            "target_readable_documents",
            "evaluation_readable_documents",
            "corpus_generation_ns",
            "graph_load_ns",
            "wac_materialization_ns",
            "route_index_ns",
            "lws_seed_ns",
            "auth_triples_total",
            "http_store_max_total_bytes",
            "http_store_max_resource_count",
            "resident_bytes",
            "peak_resident_bytes",
            "construction_allocations",
            "construction_allocated_bytes",
        ):
            values = [float(record[field]) for record in cell if record.get(field) is not None]
            row[f"{field}_median"] = statistics.median(values) if values else ""
        row["process_blocks"] = len(
            {(record["corpus_seed"], record["process_block"]) for record in cell}
        )
        rows.append(row)
    write_csv(path, rows)


def write_csv(path: Path, rows: Sequence[dict[str, Any]]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    if not rows:
        path.write_text("", encoding="utf-8")
        return
    with path.open("w", newline="", encoding="utf-8") as output:
        writer = csv.DictWriter(output, fieldnames=list(rows[0]))
        writer.writeheader()
        writer.writerows(rows)


def pod_scaling_observations(
    observations: Sequence[dict[str, Any]],
) -> list[dict[str, Any]]:
    return [
        record
        for record in observations
        if record["campaign"] == "pod-scaling"
        and record["measurement_profile"] == "timing"
        and record["operation"] == GUARDED_OPERATION[record["lane"]]
        and record["principal"] == "owner"
        and record["documents_per_pod"] == 16
        and record["triples_per_document"] == 8
        and record["public_per_mille"] == 0
        and record["private_per_mille"] == 1000
        and record["shared_per_mille"] == 0
        and record["container_depth"] == 3
        and record["own_acl_coverage_per_mille"] == 250
        and (
            (record["lane"] == "materialized-routed" and record["topology"] == "origin-per-pod")
            or (record["lane"] == "native-http-assembly" and record["topology"] == "shared-origin")
        )
    ]


def pod_scaling_instrumentation_observations(
    observations: Sequence[dict[str, Any]],
) -> list[dict[str, Any]]:
    return [
        record
        for record in observations
        if record["campaign"] == "pod-scaling-instrumentation"
        and record["measurement_profile"] == "instrumentation"
        and record["operation"] == GUARDED_OPERATION[record["lane"]]
        and record["principal"] == "owner"
        and record["documents_per_pod"] == 16
        and record["triples_per_document"] == 8
        and record["public_per_mille"] == 0
        and record["private_per_mille"] == 1000
        and record["shared_per_mille"] == 0
        and record["container_depth"] == 3
        and record["own_acl_coverage_per_mille"] == 250
        and (
            (record["lane"] == "materialized-routed" and record["topology"] == "origin-per-pod")
            or (record["lane"] == "native-http-assembly" and record["topology"] == "shared-origin")
        )
    ]


def cluster_data(
    records: Sequence[dict[str, Any]], metric: str,
) -> dict[tuple[int, int], dict[int, list[float]]]:
    clusters: dict[tuple[int, int], dict[int, list[float]]] = defaultdict(
        lambda: defaultdict(list)
    )
    for record in records:
        cluster = (int(record["corpus_seed"]), int(record["process_block"]))
        value = record.get(metric)
        if value is None or float(value) <= 0:
            raise AnalysisError(f"H2 metric {metric} must be positive in every selected record")
        clusters[cluster][int(record["pods"])].append(float(value))
    return clusters


def simple_slope(xs: Sequence[float], ys: Sequence[float]) -> float:
    if len(xs) != len(ys) or len(xs) < 2:
        raise AnalysisError("slope requires at least two aligned points")
    x_mean = statistics.mean(xs)
    y_mean = statistics.mean(ys)
    denominator = sum((x - x_mean) ** 2 for x in xs)
    if denominator == 0:
        raise AnalysisError("slope has no x variation")
    return sum((x - x_mean) * (y - y_mean) for x, y in zip(xs, ys)) / denominator


def elasticity(clusters: dict[tuple[int, int], dict[int, list[float]]], pods: Sequence[int]) -> float:
    xs = [math.log(value) for value in pods]
    slopes = []
    for cell in clusters.values():
        if all(cell.get(pod) for pod in pods):
            ys = [math.log(statistics.median(cell[pod])) for pod in pods]
            slopes.append(simple_slope(xs, ys))
    if not slopes:
        raise AnalysisError("no complete process block for elasticity")
    return statistics.mean(slopes)


def bootstrap_h2(
    clusters: dict[tuple[int, int], dict[int, list[float]]],
    pods: Sequence[int],
    draws: int,
    rng: random.Random,
) -> tuple[list[float], list[float]]:
    complete = [cell for cell in clusters.values() if all(cell.get(pod) for pod in pods)]
    if not complete:
        raise AnalysisError("no complete corpus/process blocks for H2")
    ratios: list[float] = []
    elasticities: list[float] = []
    xs = [math.log(value) for value in pods]
    for _ in range(draws):
        sampled_at_p: dict[int, list[float]] = defaultdict(list)
        slopes = []
        for _block_slot in complete:
            cell = rng.choice(complete)
            medians = []
            for pod in pods:
                source = cell[pod]
                sample = [rng.choice(source) for _ in source]
                sampled_at_p[pod].extend(sample)
                medians.append(statistics.median(sample))
            slopes.append(simple_slope(xs, [math.log(value) for value in medians]))
        low = statistics.median(sampled_at_p[pods[0]])
        high = statistics.median(sampled_at_p[pods[-1]])
        ratios.append(high / low)
        elasticities.append(statistics.mean(slopes))
    return ratios, elasticities


def h2_metric(
    records: Sequence[dict[str, Any]],
    pods: Sequence[int],
    metric: str,
    draws: int,
    rng: random.Random,
) -> dict[str, float | list[float]]:
    """Estimate a Pmax/P1 ratio and block-median log-log elasticity for one metric."""

    clusters = cluster_data(records, metric)
    complete = {
        cluster: cell
        for cluster, cell in clusters.items()
        if all(cell.get(pod) for pod in pods)
    }
    if not complete:
        raise AnalysisError(f"no complete process block for H2 metric {metric}")
    low = [float(record[metric]) for record in records if record["pods"] == pods[0]]
    high = [float(record[metric]) for record in records if record["pods"] == pods[-1]]
    ratio = statistics.median(high) / statistics.median(low)
    beta = elasticity(complete, pods)
    ratio_draws, beta_draws = bootstrap_h2(complete, pods, draws, rng)
    return {
        "ratio": ratio,
        "ratio_ci95": [percentile(ratio_draws, 0.025), percentile(ratio_draws, 0.975)],
        "elasticity": beta,
        "elasticity_ci95": [percentile(beta_draws, 0.025), percentile(beta_draws, 0.975)],
    }


def analyze_h2(
    observations: Sequence[dict[str, Any]], draws: int, seed: int
) -> list[dict[str, Any]]:
    primary = pod_scaling_observations(observations)
    keys = ("lane", "domain", "query_id", "query_family")
    results = []
    for key, records in sorted(group_by(primary, keys).items()):
        pods = sorted({int(record["pods"]) for record in records})
        if len(pods) < 2:
            continue
        latency = h2_metric(
            records,
            pods,
            "wall_ns",
            draws,
            random.Random(f"{seed}:wall:{':'.join(map(str, key))}"),
        )
        latency_clusters = cluster_data(records, "wall_ns")
        complete_cluster_keys = {
            cluster
            for cluster, cell in latency_clusters.items()
            if all(cell.get(pod) for pod in pods)
        }
        complete_blocks = len(complete_cluster_keys)
        cpu_available = all(record.get("process_cpu_ns") is not None for record in records)
        cpu = (
            h2_metric(
                records,
                pods,
                "process_cpu_ns",
                draws,
                random.Random(f"{seed}:cpu:{':'.join(map(str, key))}"),
            )
            if cpu_available
            else None
        )
        latency_minimal = (
            latency["ratio_ci95"][1] <= 1.10
            and latency["elasticity_ci95"][1] <= 0.10
        )
        cpu_minimal = (
            cpu["ratio_ci95"][1] <= 1.10 and cpu["elasticity_ci95"][1] <= 0.10
            if cpu is not None
            else None
        )
        result = dict(zip(keys, key, strict=True))
        result.update(
            {
                "pods": pods,
                "p_min": pods[0],
                "p_max": pods[-1],
                "complete_process_blocks": complete_blocks,
                "corpus_seeds": sorted({cluster[0] for cluster in complete_cluster_keys}),
                "requests": len(records),
                "median_latency_ratio": latency["ratio"],
                "ratio_ci95": latency["ratio_ci95"],
                "pod_elasticity": latency["elasticity"],
                "elasticity_ci95": latency["elasticity_ci95"],
                "median_process_cpu_ratio": cpu["ratio"] if cpu else None,
                "process_cpu_ratio_ci95": cpu["ratio_ci95"] if cpu else None,
                "process_cpu_pod_elasticity": cpu["elasticity"] if cpu else None,
                "process_cpu_elasticity_ci95": (
                    cpu["elasticity_ci95"] if cpu else None
                ),
                "ratio_margin": 1.10,
                "elasticity_margin": 0.10,
                "minimal_latency_scaling": latency_minimal,
                "minimal_cpu_scaling": cpu_minimal,
                "minimal_overhead": (
                    latency_minimal and cpu_minimal if cpu_minimal is not None else None
                ),
                "bootstrap_draws": draws,
                "bootstrap_seed": seed,
            }
        )
        results.append(result)
    return results


def write_h2(path: Path, results: Sequence[dict[str, Any]], files: Sequence[Path]) -> None:
    document = {
        "schema_version": SCHEMA_VERSION,
        "analysis": "prospective H2 hierarchical cluster bootstrap",
        "inputs": [str(path) for path in files],
        "results": list(results),
    }
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(document, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def xml_escape(value: Any) -> str:
    return (
        str(value)
        .replace("&", "&amp;")
        .replace("<", "&lt;")
        .replace(">", "&gt;")
        .replace('"', "&quot;")
    )


def scaling_series(
    observations: Sequence[dict[str, Any]], query_ids: set[str]
) -> dict[tuple[str, str, str], list[tuple[int, float]]]:
    primary = [
        record
        for record in pod_scaling_observations(observations)
        if record["query_id"] in query_ids
    ]
    cells = group_by(primary, ("lane", "domain", "query_id", "pods"))
    series: dict[tuple[str, str, str], list[tuple[int, float]]] = defaultdict(list)
    for (lane, domain, query, pods), records in cells.items():
        median_ms = statistics.median(record["wall_ns"] for record in records) / 1_000_000
        series[(lane, domain, query)].append((int(pods), median_ms))
    for values in series.values():
        values.sort()
    return series


def write_scaling_svg(path: Path, observations: Sequence[dict[str, Any]]) -> bool:
    series = scaling_series(observations, {"q1-point", "q8-graph-scan"})
    if not series:
        return False
    width, height = 980, 460
    margin_left, margin_top, panel_width, panel_height = 75, 55, 395, 320
    gap = 80
    colors = {"q1-point": "#1769aa", "q8-graph-scan": "#c65d09"}
    dashes = {"social": "", "health": "6 4"}
    lanes = ["materialized-routed", "native-http-assembly"]
    all_pods = sorted({pod for values in series.values() for pod, _ in values})
    all_ms = [milliseconds for values in series.values() for _, milliseconds in values]
    if len(all_pods) < 2 or not all_ms:
        return False
    x_min, x_max = math.log2(min(all_pods)), math.log2(max(all_pods))
    positive = [value for value in all_ms if value > 0]
    y_min = math.floor(math.log10(min(positive)))
    y_max = math.ceil(math.log10(max(positive)))
    if y_min == y_max:
        y_max += 1

    def x_position(panel: int, pod: int) -> float:
        start = margin_left + panel * (panel_width + gap)
        return start + (math.log2(pod) - x_min) / (x_max - x_min) * panel_width

    def y_position(milliseconds: float) -> float:
        return margin_top + panel_height - (
            (math.log10(milliseconds) - y_min) / (y_max - y_min) * panel_height
        )

    svg = [
        f'<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{height}" viewBox="0 0 {width} {height}">',
        '<rect width="100%" height="100%" fill="white"/>',
        '<style>text{font-family:system-ui,sans-serif;fill:#202124}.axis{stroke:#555;stroke-width:1}.grid{stroke:#ddd;stroke-width:1}.line{fill:none;stroke-width:2.4}.point{stroke:white;stroke-width:1}</style>',
        '<text x="490" y="24" text-anchor="middle" font-size="17" font-weight="600">Warm access-controlled query latency versus resident Pod count</text>',
    ]
    for panel, lane in enumerate(lanes):
        start = margin_left + panel * (panel_width + gap)
        svg.append(
            f'<text x="{start + panel_width / 2}" y="43" text-anchor="middle" font-size="14">{xml_escape(lane)}</text>'
        )
        for exponent in range(y_min, y_max + 1):
            value = 10**exponent
            y = y_position(value)
            svg.append(f'<line class="grid" x1="{start}" y1="{y}" x2="{start + panel_width}" y2="{y}"/>')
            if panel == 0:
                svg.append(f'<text x="{start - 9}" y="{y + 4}" text-anchor="end" font-size="11">{value:g}</text>')
        svg.append(f'<line class="axis" x1="{start}" y1="{margin_top}" x2="{start}" y2="{margin_top + panel_height}"/>')
        svg.append(f'<line class="axis" x1="{start}" y1="{margin_top + panel_height}" x2="{start + panel_width}" y2="{margin_top + panel_height}"/>')
        for pod in all_pods:
            x = x_position(panel, pod)
            svg.append(f'<text x="{x}" y="{margin_top + panel_height + 19}" text-anchor="middle" font-size="10">{pod}</text>')
        for (series_lane, domain, query), values in sorted(series.items()):
            if series_lane != lane:
                continue
            points = " ".join(f"{x_position(panel, pod):.2f},{y_position(ms):.2f}" for pod, ms in values if ms > 0)
            dash = f' stroke-dasharray="{dashes[domain]}"' if dashes[domain] else ""
            svg.append(f'<polyline class="line" stroke="{colors[query]}"{dash} points="{points}"/>')
            for pod, ms in values:
                if ms > 0:
                    svg.append(f'<circle class="point" fill="{colors[query]}" cx="{x_position(panel, pod):.2f}" cy="{y_position(ms):.2f}" r="3.5"/>')
    svg.extend(
        [
            '<text x="20" y="215" transform="rotate(-90 20 215)" text-anchor="middle" font-size="12">median latency (ms, log scale)</text>',
            '<text x="490" y="430" text-anchor="middle" font-size="12">resident Pods (log2 spacing; Pod 0 working set fixed)</text>',
            '<line x1="305" y1="447" x2="331" y2="447" stroke="#1769aa" stroke-width="2.4"/><text x="337" y="451" font-size="11">point</text>',
            '<line x1="400" y1="447" x2="426" y2="447" stroke="#c65d09" stroke-width="2.4"/><text x="432" y="451" font-size="11">GRAPH scan</text>',
            '<line x1="535" y1="447" x2="561" y2="447" stroke="#444" stroke-width="2.4"/><text x="567" y="451" font-size="11">social</text>',
            '<line x1="635" y1="447" x2="661" y2="447" stroke="#444" stroke-width="2.4" stroke-dasharray="6 4"/><text x="667" y="451" font-size="11">health</text>',
            '</svg>',
        ]
    )
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text("\n".join(svg) + "\n", encoding="utf-8")
    return True


def write_backend_svg(path: Path, observations: Sequence[dict[str, Any]]) -> bool:
    records = [
        record
        for record in pod_scaling_instrumentation_observations(observations)
        if record["lane"] == "native-http-assembly"
        and record["query_id"] == "q1-point"
        and record.get("backend_total_operations") is not None
    ]
    cells = group_by(records, ("domain", "pods"))
    series: dict[str, list[tuple[int, float]]] = defaultdict(list)
    for (domain, pods), values in cells.items():
        series[domain].append(
            (int(pods), statistics.median(record["backend_total_operations"] for record in values))
        )
    for values in series.values():
        values.sort()
    if not series:
        return False
    width, height = 620, 410
    left, top, plot_width, plot_height = 75, 45, 500, 285
    pods = sorted({pod for values in series.values() for pod, _ in values})
    operations = [value for values in series.values() for _, value in values]
    if len(pods) < 2:
        return False
    x0, x1 = math.log2(min(pods)), math.log2(max(pods))
    y0, y1 = math.floor(math.log10(min(operations))), math.ceil(math.log10(max(operations)))
    if y0 == y1:
        y1 += 1

    def x(pod: int) -> float:
        return left + (math.log2(pod) - x0) / (x1 - x0) * plot_width

    def y(value: float) -> float:
        return top + plot_height - (math.log10(value) - y0) / (y1 - y0) * plot_height

    colors = {"social": "#1769aa", "health": "#9c2f74"}
    svg = [
        f'<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{height}" viewBox="0 0 {width} {height}">',
        '<rect width="100%" height="100%" fill="white"/>',
        '<style>text{font-family:system-ui,sans-serif;fill:#202124}.axis{stroke:#555}.grid{stroke:#ddd}.line{fill:none;stroke-width:2.5}</style>',
        '<text x="310" y="23" text-anchor="middle" font-size="16" font-weight="600">Native endpoint backend work for a graph-bound query</text>',
    ]
    for exponent in range(y0, y1 + 1):
        value = 10**exponent
        yy = y(value)
        svg.append(f'<line class="grid" x1="{left}" y1="{yy}" x2="{left + plot_width}" y2="{yy}"/>')
        svg.append(f'<text x="{left - 8}" y="{yy + 4}" text-anchor="end" font-size="11">10^{exponent}</text>')
    svg.append(f'<line class="axis" x1="{left}" y1="{top}" x2="{left}" y2="{top + plot_height}"/>')
    svg.append(f'<line class="axis" x1="{left}" y1="{top + plot_height}" x2="{left + plot_width}" y2="{top + plot_height}"/>')
    for pod in pods:
        svg.append(f'<text x="{x(pod)}" y="{top + plot_height + 19}" text-anchor="middle" font-size="10">{pod}</text>')
    for domain, values in sorted(series.items()):
        points = " ".join(f"{x(pod):.2f},{y(value):.2f}" for pod, value in values)
        svg.append(f'<polyline class="line" stroke="{colors.get(domain, "#555")}" points="{points}"/>')
        for pod, value in values:
            svg.append(f'<circle fill="{colors.get(domain, "#555")}" cx="{x(pod):.2f}" cy="{y(value):.2f}" r="3.5"/>')
    svg.extend(
        [
            '<text x="19" y="190" transform="rotate(-90 19 190)" text-anchor="middle" font-size="12">backend operations/request (log scale)</text>',
            '<text x="325" y="372" text-anchor="middle" font-size="12">resident Pods (log2 spacing)</text>',
            '<circle fill="#1769aa" cx="225" cy="394" r="4"/><text x="235" y="398" font-size="11">social</text>',
            '<circle fill="#9c2f74" cx="330" cy="394" r="4"/><text x="340" y="398" font-size="11">health</text>',
            '</svg>',
        ]
    )
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text("\n".join(svg) + "\n", encoding="utf-8")
    return True


def output_artifacts(output_dir: Path, names: Sequence[str]) -> list[dict[str, Any]]:
    """Register byte length and SHA-256 for every output produced by this invocation."""

    artifacts = []
    for name in names:
        path = output_dir / name
        if not path.is_file():
            raise AnalysisError(f"declared analysis output does not exist: {path}")
        artifacts.append(
            {
                "path": name,
                "bytes": path.stat().st_size,
                "sha256": sha256_file(path),
            }
        )
    return artifacts


def validate_bootstrap_draws(draws: int, require_canonical: bool) -> None:
    if draws < 100:
        raise AnalysisError("bootstrap-draws must be at least 100")
    if require_canonical and draws < MIN_CANONICAL_BOOTSTRAP_DRAWS:
        raise AnalysisError(
            "canonical bootstrap-draws must be at least "
            f"{MIN_CANONICAL_BOOTSTRAP_DRAWS}"
        )


def main() -> int:
    args = arguments()
    validate_bootstrap_draws(args.bootstrap_draws, args.require_canonical)
    if args.require_canonical:
        if args.expected_commit is None:
            raise AnalysisError("--expected-commit is required with --require-canonical")
        if re.fullmatch(r"[0-9a-f]{40}", args.expected_commit) is None:
            raise AnalysisError("--expected-commit must be a lowercase 40-hex Git object")
    loaded = load(
        args.inputs,
        args.run_prefix,
        args.require_canonical,
        args.expected_commit,
    )
    args.out.mkdir(parents=True, exist_ok=True)
    write_summary(args.out / "summary.csv", loaded.observations)
    h4 = analyze_h4(
        loaded.observations,
        args.bootstrap_draws,
        args.bootstrap_seed,
        require_complete_blocks=args.require_canonical,
    )
    write_overhead(args.out / "paired-overhead.csv", h4)
    write_construction(args.out / "construction.csv", loaded.constructions)
    h2 = analyze_h2(loaded.observations, args.bootstrap_draws, args.bootstrap_seed)
    write_h2(args.out / "h2.json", h2, loaded.files)
    scaling_written = write_scaling_svg(
        args.out / "pod-scaling-latency.svg", loaded.observations
    )
    backend_written = write_backend_svg(
        args.out / "http-backend-operations.svg", loaded.observations
    )
    output_names = [
        "summary.csv",
        "paired-overhead.csv",
        "construction.csv",
        "h2.json",
    ]
    if scaling_written:
        output_names.append("pod-scaling-latency.svg")
    if backend_written:
        output_names.append("http-backend-operations.svg")
    if args.require_canonical and not (scaling_written and backend_written):
        raise AnalysisError("canonical analysis did not produce both declared SVG figures")
    artifacts = output_artifacts(args.out, output_names)
    manifest = {
        "schema_version": SCHEMA_VERSION,
        "input_files": [
            {
                "path": str(path),
                "sha256": sha256_file(path),
            }
            for path in loaded.files
        ],
        "analysis_script": {
            "path": str(Path(__file__).resolve()),
            "sha256": sha256_file(Path(__file__)),
        },
        "canonical_required": args.require_canonical,
        "expected_commit": args.expected_commit,
        "observations": len(loaded.observations),
        "construction_records": len(loaded.constructions),
        "correctness_records": len(loaded.correctness),
        "applicability_records": len(loaded.applicability),
        "bootstrap_draws": args.bootstrap_draws,
        "bootstrap_seed": args.bootstrap_seed,
        "outputs": output_names,
        "output_artifacts": artifacts,
    }
    (args.out / "manifest.json").write_text(
        json.dumps(manifest, indent=2, sort_keys=True) + "\n", encoding="utf-8"
    )
    print(
        f"validated {len(loaded.observations)} observations from {len(loaded.files)} files; "
        f"wrote {args.out}"
    )
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except AnalysisError as error:
        print(f"analysis refused input: {error}", file=sys.stderr)
        raise SystemExit(2) from error
