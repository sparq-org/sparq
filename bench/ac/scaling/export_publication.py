#!/usr/bin/env python3
"""Fail-closed publication exporter for the AC-SPARQL canonical study.

This standard-library-only program consumes *validated derived artifacts*, never raw
timing values directly.  It verifies the analyzer manifest and the archived run
manifest, checks the complete frozen experimental matrix, transforms list-shaped CSV
and H2 records into stable object-keyed results, derives only preregistered Boolean and
count rollups, and emits digest-bound publication SVGs plus a paper-factory envelope.

The exporter is intentionally opinionated:

* H2 is the only performance hypothesis with a prospective numerical decision rule.
  H1 is the exact-result correctness gate.  H3--H5 are exploratory/descriptive and
  this program does not manufacture pass/fail prose for them.
* Every result scalar has a source-artifact/hash/locator binding.  The compact
  paper-facing subset additionally has a JSON-pointer declaration for the factory.
* New figures show all cells in their declared slice; there is no outcome-dependent
  query or domain selection.
* Publication output is deterministic for byte-identical inputs and metadata.
"""

from __future__ import annotations

import argparse
import csv
import hashlib
import html
import json
import math
import os
import re
import shutil
import subprocess
import sys
import tarfile
import tempfile
from dataclasses import dataclass
from datetime import datetime, timezone
from decimal import Decimal, InvalidOperation
from pathlib import Path, PurePosixPath
from typing import Any, Iterable, Mapping, Sequence


SCHEMA_VERSION = 1
ANALYZER_SCHEMA_VERSION = 6
STUDY_ID = "access-controlled-sparql-pod-scaling"
PAPER_SLUG = "access-controlled-sparql-pod-scale"
POD_COUNTS = (1, 8, 64, 512, 2048)
BLOCK_SEEDS = (17, 42, 101, 314, 2718)
LANES = ("materialized-routed", "native-http-assembly")
DOMAINS = ("health", "social")
QUERIES = (
    "q1-point",
    "q2-star",
    "q3-join",
    "q4-optional",
    "q5-not-exists",
    "q6-path",
    "q7-aggregate",
    "q8-graph-scan",
)
QUERY_SHORT = {query: query.split("-", 1)[0] for query in QUERIES}
LANE_SHORT = {
    "materialized-routed": "materialized",
    "native-http-assembly": "http",
}
TOPOLOGY = {
    "materialized-routed": "origin-per-pod",
    "native-http-assembly": "shared-origin",
}
OPERATIONS = {
    "materialized-routed": ("guarded-query-as", "plain-engine-reference"),
    "native-http-assembly": ("native-http-request", "plain-engine-json-reference"),
}
GUARDED = {lane: operations[0] for lane, operations in OPERATIONS.items()}
CORRECTNESS_PRINCIPALS = (
    "owner",
    "named-recipient",
    "authenticated-stranger",
    "anonymous",
)
CORRECTNESS_PRINCIPAL_CLI_LABEL = {
    "owner": "owner",
    "named-recipient": "recipient",
    "authenticated-stranger": "stranger",
    "anonymous": "anonymous",
}
CORRECTNESS_QUERY_METADATA = {
    "q1-point": ("graph-bound point lookup", 1),
    "q2-star": ("selective star pattern", 3),
    "q3-join": ("multiway join", 8),
    "q4-optional": ("OPTIONAL", 5),
    "q5-not-exists": ("NOT EXISTS", 5),
    "q6-path": ("bounded property path", 8),
    "q7-aggregate": ("aggregate/grouping", 2),
    "q8-graph-scan": ("unbound GRAPH scan", 1),
}
CORRECTNESS_COMMON_FIELDS = frozenset({
    "schema_version", "run_id", "run_uuid", "source_commit", "source_dirty",
    "host", "instance_id", "instance_type", "cloud_region", "os",
    "architecture", "rustc", "profile", "features", "measurement_profile",
    "campaign", "cell_label", "lane", "domain", "topology", "pods",
    "documents_per_pod", "triples_per_document", "container_depth",
    "own_acl_coverage_per_mille", "public_per_mille", "private_per_mille",
    "shared_per_mille", "principal", "corpus_seed", "corpus_hash_sha256",
    "process_block", "configuration_order", "configuration_order_seed",
    "warmups_configured", "repetitions_configured", "concurrency",
    "rayon_threads", "cpu_affinity",
})
CORRECTNESS_FIELDS_BY_KIND = {
    "applicability": frozenset({
        "record_type", "utc_unix_ns", "query_id", "query_family",
        "query_hash_sha256", "minimum_triples_per_document", "applicable",
        "selected", "reason",
    }),
    "correctness-gate": frozenset({
        "record_type", "utc_unix_ns", "gate", "principals_checked",
        "queries_checked", "exact_result_bags",
    }),
    "construction": frozenset({
        "record_type", "utc_unix_ns", "content_documents", "content_triples",
        "container_graphs", "control_documents", "control_triples",
        "total_source_graphs", "target_readable_documents",
        "evaluation_readable_documents", "corpus_generation_ns", "graph_load_ns",
        "wac_materialization_ns", "route_index_ns", "lws_seed_ns",
        "auth_triples_total", "http_store_max_total_bytes",
        "http_store_max_resource_count", "construction_allocations",
        "construction_allocated_bytes", "resident_bytes", "peak_resident_bytes",
    }),
    "observation": frozenset({
        "record_type", "utc_unix_ns", "query_id", "query_family",
        "query_hash_sha256", "operation", "pair_id", "repetition", "warmup",
        "order_in_pair", "wall_ns", "process_cpu_ns", "allocation_operations",
        "allocated_bytes", "response_bytes", "result_rows", "result_hash_sha256",
        "correctness", "http_status", "backend_sparql_queries",
        "backend_sparql_updates", "backend_blob_gets", "backend_blob_puts",
        "backend_blob_other", "backend_total_operations", "backend_max_in_flight",
    }),
}
CORRECTNESS_RECORD_COUNTS = {
    "applicability": 8,
    "correctness-gate": 1,
    "construction": 1,
    "observation": 16,
}
PROFILES = {
    "pod-scaling": "timing",
    "pod-scaling-instrumentation": "instrumentation",
    "sensitivity": "timing",
    "sensitivity-instrumentation": "instrumentation",
    "scenarios": "timing",
    "scenarios-instrumentation": "instrumentation",
}
REQUIRED_ANALYZER_OUTPUTS = {
    "summary.csv",
    "paired-overhead.csv",
    "construction.csv",
    "h2.json",
    "pod-scaling-latency.svg",
    "http-backend-operations.svg",
}
H4_METHOD = "resample-corpus-process-blocks-then-intact-pairs"
HEX40 = re.compile(r"^[0-9a-f]{40}$")
HEX64 = re.compile(r"^[0-9a-f]{64}$")
KEY_RE = re.compile(r"^[a-z0-9][a-z0-9_.-]+$")
RUN_ID_RE = re.compile(r"^[a-z0-9][a-z0-9._-]*$")
ZSTD_MAGIC = b"\x28\xb5\x2f\xfd"
SENSITIVE_MEMBER_COMPONENTS = re.compile(
    r"(?:^|[-_.])(?:credentials?|secrets?|tokens?|id_rsa|authorized_keys)(?:$|[-_.])|\.pem$",
    re.IGNORECASE,
)
SENSITIVE_CONTENT_PATTERNS: tuple[tuple[str, re.Pattern[bytes]], ...] = (
    ("AWS access-key identifier", re.compile(rb"(?:AKIA|ASIA)[0-9A-Z]{16}")),
    ("AWS secret/session credential field", re.compile(
        rb"(?i)(?:aws_secret_access_key|aws_session_token|x-amz-security-token)\s*[\"'=:\s]+[^\s\",}]{8,}"
    )),
    ("HTTP bearer credential", re.compile(rb"(?i)authorization\s*[\"'=:\s]+bearer\s+[^\s\",}]+")),
    ("private key", re.compile(rb"-----BEGIN (?:[A-Z0-9 ]+ )?PRIVATE KEY-----")),
    ("JWT-like token", re.compile(rb"eyJ[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]{8,}")),
    ("AWS account ID in ARN", re.compile(rb"arn:(?:aws|aws-us-gov|aws-cn):[^:\s]*:[^:\s]*:\d{12}:")),
    ("labelled AWS account ID", re.compile(
        rb"(?i)(?:aws[_ -]?account[_ -]?id|account[_ -]?id|owner[_ -]?id|[\"']account[\"'])\s*[\"'=:\s]+\d{12}\b"
    )),
)


class PublicationError(RuntimeError):
    """The publication bundle cannot be constructed without weakening provenance."""


@dataclass(frozen=True)
class Config:
    campaign: str
    profile: str
    cell_label: str
    lane: str
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

    @property
    def topology(self) -> str:
        return TOPOLOGY[self.lane]

    @property
    def repetitions(self) -> int:
        return 30 if self.profile == "timing" else 1

    @property
    def requests_per_summary_row(self) -> int:
        return len(BLOCK_SEEDS) * self.repetitions


@dataclass(frozen=True)
class Artifact:
    name: str
    path: Path
    sha256: str
    bytes: int


def fail(message: str) -> None:
    raise PublicationError(message)


def sha256_bytes(payload: bytes) -> str:
    return hashlib.sha256(payload).hexdigest()


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        while chunk := source.read(1024 * 1024):
            digest.update(chunk)
    return digest.hexdigest()


def read_json(path: Path, label: str) -> dict[str, Any]:
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except OSError as error:
        raise PublicationError(f"cannot read {label}: {path}") from error
    except json.JSONDecodeError as error:
        raise PublicationError(f"invalid JSON in {label}: {path}: {error}") from error
    if not isinstance(value, dict):
        fail(f"{label} must be a JSON object")
    return value


def read_csv(path: Path, label: str, expected_fields: set[str]) -> list[dict[str, str]]:
    try:
        with path.open(newline="", encoding="utf-8") as source:
            reader = csv.DictReader(source)
            fieldnames = reader.fieldnames or []
            fields = set(fieldnames)
            duplicates = sorted({field for field in fieldnames if fieldnames.count(field) > 1})
            if duplicates:
                fail(f"{label} contains duplicate columns: {duplicates}")
            if fields != expected_fields:
                fail(
                    f"{label} columns changed: missing={sorted(expected_fields - fields)}, "
                    f"extra={sorted(fields - expected_fields)}"
                )
            rows = list(reader)
    except OSError as error:
        raise PublicationError(f"cannot read {label}: {path}") from error
    if not rows:
        fail(f"{label} is empty")
    for index, row in enumerate(rows, start=2):
        if None in row or any(value is None for value in row.values()):
            fail(f"{label} row {index} does not match its header width")
    return rows


def require_object(value: Any, label: str) -> dict[str, Any]:
    if not isinstance(value, dict):
        fail(f"{label} must be an object")
    return value


def require_list(value: Any, label: str) -> list[Any]:
    if not isinstance(value, list):
        fail(f"{label} must be an array")
    return value


def require_string(value: Any, label: str) -> str:
    if not isinstance(value, str) or not value:
        fail(f"{label} must be a non-empty string")
    return value


def require_bool(value: Any, label: str) -> bool:
    if not isinstance(value, bool):
        fail(f"{label} must be Boolean")
    return value


def require_int(value: Any, label: str, minimum: int | None = None) -> int:
    if isinstance(value, bool) or not isinstance(value, int):
        fail(f"{label} must be an integer")
    if minimum is not None and value < minimum:
        fail(f"{label} must be >= {minimum}")
    return value


def require_number(value: Any, label: str) -> float | int:
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        fail(f"{label} must be numeric")
    if not math.isfinite(float(value)):
        fail(f"{label} must be finite")
    return value


def csv_int(row: Mapping[str, str], field: str, label: str, minimum: int = 0) -> int:
    value = row.get(field, "")
    if not re.fullmatch(r"-?[0-9]+", value):
        fail(f"{label}.{field} must be an integer, got {value!r}")
    number = int(value)
    if number < minimum:
        fail(f"{label}.{field} must be >= {minimum}")
    return number


def csv_float(
    row: Mapping[str, str], field: str, label: str, *, allow_blank: bool = False
) -> float | None:
    value = row.get(field, "")
    if value == "" and allow_blank:
        return None
    try:
        number = float(value)
    except (TypeError, ValueError) as error:
        raise PublicationError(f"{label}.{field} must be finite, got {value!r}") from error
    if not math.isfinite(number):
        fail(f"{label}.{field} must be finite")
    return number


def decimal_value(value: Any, label: str) -> Decimal:
    if not isinstance(value, str):
        fail(f"{label} must be a decimal string")
    try:
        result = Decimal(value)
    except InvalidOperation as error:
        raise PublicationError(f"{label} is not a decimal") from error
    if not result.is_finite() or result < 0:
        fail(f"{label} must be a finite non-negative decimal")
    return result


def resolve_from(base: Path, value: Any, label: str) -> Path:
    rel = require_string(value, label)
    path = Path(rel)
    return path if path.is_absolute() else (base / path).resolve()


def ensure_inside(root: Path, path: Path, label: str) -> str:
    root = root.resolve()
    path = path.resolve()
    try:
        rel = path.relative_to(root).as_posix()
    except ValueError as error:
        raise PublicationError(f"{label} escapes repository root: {path}") from error
    return rel


def verify_declared_file(base: Path, descriptor: Any, label: str) -> Artifact:
    obj = require_object(descriptor, label)
    path = resolve_from(base, obj.get("path"), f"{label}.path")
    declared_hash = require_string(obj.get("sha256"), f"{label}.sha256")
    if not HEX64.fullmatch(declared_hash):
        fail(f"{label}.sha256 must be lowercase 64-hex")
    if not path.is_file() or path.is_symlink():
        fail(f"{label}.path must be a regular non-symlink file: {path}")
    actual_hash = sha256_file(path)
    actual_bytes = path.stat().st_size
    if actual_hash != declared_hash:
        fail(f"{label} digest mismatch: {actual_hash} != {declared_hash}")
    if "bytes" in obj and require_int(obj["bytes"], f"{label}.bytes", 1) != actual_bytes:
        fail(f"{label} byte count mismatch")
    return Artifact(path.name, path, actual_hash, actual_bytes)


def parse_utc(value: Any, label: str) -> datetime:
    text = require_string(value, label)
    if not text.endswith("Z"):
        fail(f"{label} must end in Z")
    try:
        moment = datetime.fromisoformat(text[:-1] + "+00:00")
    except ValueError as error:
        raise PublicationError(f"{label} must be ISO-8601 UTC") from error
    if moment.tzinfo != timezone.utc:
        fail(f"{label} must be UTC")
    return moment


def parse_environment_file(path: Path) -> dict[str, str]:
    values: dict[str, str] = {}
    for line_number, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
        if "=" not in line:
            fail(f"environment file line {line_number} has no '='")
        key, value = line.split("=", 1)
        if not key or key in values:
            fail(f"environment file has invalid or duplicate key {key!r}")
        values[key] = value
    return values


def parse_sha256_manifest(path: Path, run_root: Path) -> dict[str, str]:
    entries: dict[str, str] = {}
    line_re = re.compile(r"^([0-9a-f]{64}) [ *](.+)$")
    for line_number, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
        match = line_re.fullmatch(line)
        if not match:
            fail(f"raw manifest line {line_number} is not sha256sum format")
        digest, raw_name = match.groups()
        name = raw_name.removeprefix("./")
        if name in entries:
            fail(f"raw manifest repeats {name}")
        target = (run_root / name).resolve()
        try:
            target.relative_to(run_root.resolve())
        except ValueError as error:
            raise PublicationError(f"raw manifest path escapes run root: {name}") from error
        if not target.is_file() or target.is_symlink():
            fail(f"raw manifest member is missing or not a regular file: {name}")
        actual = sha256_file(target)
        if actual != digest:
            fail(f"raw manifest member drift: {name}: {actual} != {digest}")
        entries[name] = digest
    if not entries:
        fail("raw manifest is empty")
    return entries


def protocol_version(path: Path) -> str:
    match = re.search(
        r"^\*\*Protocol version:\*\*\s*([^\s]+)\s*$",
        path.read_text(encoding="utf-8"),
        re.MULTILINE,
    )
    if not match:
        fail(f"cannot locate protocol version in {path}")
    return match.group(1)


def all_configs() -> tuple[Config, ...]:
    configs: list[Config] = []
    for campaign, profile in PROFILES.items():
        base_campaign = campaign.removesuffix("-instrumentation")
        if base_campaign == "pod-scaling":
            specs = [
                (f"pods-{pods}", domain, pods, 16, 8, 3, 250, 0, 1000, 0, "owner", QUERIES)
                for domain in DOMAINS
                for pods in POD_COUNTS
            ]
        elif base_campaign == "sensitivity":
            specs = []
            for domain in DOMAINS:
                for documents in (1, 8, 32, 128, 512):
                    specs.append((
                        f"documents-{documents}", domain, 16, documents, 8, 3, 250,
                        0, 1000, 0, "owner", ("q1-point", "q8-graph-scan"),
                    ))
                for triples in (1, 8, 32, 128):
                    specs.append((
                        f"triples-{triples}", domain, 16, 32, triples, 3, 250,
                        0, 1000, 0, "owner", ("q1-point", "q8-graph-scan"),
                    ))
                for coverage in (0, 100, 250, 1000):
                    for depth in (1, 3, 6):
                        specs.append((
                            f"placement-{coverage}-{depth}", domain, 16, 16, 8, depth,
                            coverage, 0, 1000, 0, "owner",
                            ("q1-point", "q8-graph-scan"),
                        ))
                for public in (10, 100, 500, 1000):
                    specs.append((
                        f"visibility-{public}", domain, 1, 512, 8, 3, 250,
                        public, 1000 - public, 0, "anonymous",
                        ("q1-point", "q8-graph-scan"),
                    ))
        elif base_campaign == "scenarios":
            specs = [
                ("health-compact-anchor", "health", 256, 1, 128, 1, 1000, 0, 1000, 0, "owner", ("q1-point",)),
                ("health-compact-small", "health", 64, 1, 32, 1, 1000, 0, 1000, 0, "owner", ("q1-point",)),
                ("social-count-anchor", "social", 1531, 103, 22, 3, 250, 100, 700, 200, "owner", ("q1-point",)),
                ("social-count-small", "social", 100, 103, 22, 3, 250, 100, 700, 200, "owner", ("q1-point",)),
            ]
        else:  # pragma: no cover - frozen table makes this unreachable
            fail(f"unknown campaign {campaign}")
        for spec in specs:
            label, domain, pods, documents, triples, depth, coverage, public, private, shared, principal, queries = spec
            for lane in LANES:
                configs.append(Config(
                    campaign, profile, label, lane, domain, pods, documents, triples,
                    depth, coverage, public, private, shared, principal, tuple(queries),
                ))
    return tuple(sorted(configs, key=lambda item: (
        item.campaign, item.cell_label, item.lane, item.domain,
    )))


SUMMARY_FIELDS = {
    "campaign", "measurement_profile", "cell_label", "lane", "domain", "topology",
    "pods", "documents_per_pod", "triples_per_document", "container_depth",
    "own_acl_coverage_per_mille", "public_per_mille", "private_per_mille",
    "shared_per_mille", "principal", "query_id", "query_family", "operation",
    "requests", "run_uuids", "process_blocks", "latency_median_ms", "latency_p95_ms",
    "latency_p99_ms", "latency_mad_ms", "process_cpu_median_ms", "process_cpu_p95_ms",
    "process_cpu_p99_ms", "process_cpu_mad_ms", "allocation_operations_median",
    "allocated_bytes_median", "backend_operations_median", "result_rows",
}

OVERHEAD_FIELDS = {
    "campaign", "measurement_profile", "cell_label", "lane", "domain", "topology",
    "pods", "documents_per_pod", "triples_per_document", "container_depth",
    "own_acl_coverage_per_mille", "public_per_mille", "private_per_mille",
    "shared_per_mille", "principal", "query_id", "query_family", "pairs",
    "process_blocks", "pairs_per_process_block", "inference_status",
    "bootstrap_draws", "bootstrap_seed", "bootstrap_method",
    "latency_ratio_median", "latency_ratio_p95",
    "latency_ratio_ci95_low", "latency_ratio_ci95_high",
    "latency_difference_median_ms", "latency_difference_p95_ms",
    "latency_difference_ci95_low_ms", "latency_difference_ci95_high_ms",
    "cpu_ratio_median", "cpu_ratio_ci95_low", "cpu_ratio_ci95_high",
    "cpu_difference_median_ms", "cpu_difference_ci95_low_ms",
    "cpu_difference_ci95_high_ms",
}

CONSTRUCTION_BASE_FIELDS = {
    "campaign", "measurement_profile", "cell_label", "lane", "domain", "topology",
    "pods", "documents_per_pod", "triples_per_document", "container_depth",
    "own_acl_coverage_per_mille", "public_per_mille", "private_per_mille",
    "shared_per_mille", "principal", "process_blocks",
}
CONSTRUCTION_METRICS = (
    "content_documents", "content_triples", "container_graphs", "control_documents",
    "control_triples", "total_source_graphs", "target_readable_documents",
    "evaluation_readable_documents", "corpus_generation_ns", "graph_load_ns",
    "wac_materialization_ns", "route_index_ns", "lws_seed_ns", "auth_triples_total",
    "http_store_max_total_bytes", "http_store_max_resource_count", "resident_bytes",
    "peak_resident_bytes", "construction_allocations", "construction_allocated_bytes",
)
CONSTRUCTION_FIELDS = CONSTRUCTION_BASE_FIELDS | {
    f"{field}_median" for field in CONSTRUCTION_METRICS
}

H2_TOP_FIELDS = {"analysis", "inputs", "results", "schema_version"}
H2_RESULT_FIELDS = {
    "bootstrap_draws", "bootstrap_seed", "complete_process_blocks", "corpus_seeds",
    "domain", "elasticity_ci95", "elasticity_margin", "lane",
    "median_latency_ratio", "median_process_cpu_ratio", "minimal_cpu_scaling",
    "minimal_latency_scaling", "minimal_overhead", "p_max", "p_min",
    "pod_elasticity", "pods", "process_cpu_elasticity_ci95",
    "process_cpu_pod_elasticity", "process_cpu_ratio_ci95", "query_family",
    "query_id", "ratio_ci95", "ratio_margin", "requests",
}


def config_identity(config: Config) -> tuple[str, str, str, str]:
    return config.campaign, config.cell_label, config.lane, config.domain


def validate_config_columns(row: Mapping[str, str], config: Config, label: str) -> None:
    expected: dict[str, Any] = {
        "campaign": config.campaign,
        "measurement_profile": config.profile,
        "cell_label": config.cell_label,
        "lane": config.lane,
        "domain": config.domain,
        "topology": config.topology,
        "pods": config.pods,
        "documents_per_pod": config.documents,
        "triples_per_document": config.triples,
        "container_depth": config.depth,
        "own_acl_coverage_per_mille": config.coverage,
        "public_per_mille": config.public,
        "private_per_mille": config.private,
        "shared_per_mille": config.shared,
        "principal": config.principal,
    }
    for field, value in expected.items():
        if row.get(field) != str(value):
            fail(f"{label}.{field}={row.get(field)!r}, expected {value!r}")


def index_unique(
    rows: Iterable[dict[str, str]], keys: Sequence[str], label: str
) -> dict[tuple[str, ...], dict[str, str]]:
    result: dict[tuple[str, ...], dict[str, str]] = {}
    for row in rows:
        key = tuple(row.get(field, "") for field in keys)
        if key in result:
            fail(f"{label} duplicates key {key}")
        result[key] = row
    return result


def validate_summary(rows: list[dict[str, str]], configs: Sequence[Config]) -> dict[tuple[str, ...], dict[str, str]]:
    keys = ("campaign", "cell_label", "lane", "domain", "query_id", "operation")
    indexed = index_unique(rows, keys, "summary")
    expected: dict[tuple[str, ...], Config] = {}
    for config in configs:
        for query in config.query_ids:
            for operation in OPERATIONS[config.lane]:
                key = (*config_identity(config), query, operation)
                expected[key] = config
    if set(indexed) != set(expected):
        missing = sorted(set(expected) - set(indexed))[:8]
        extra = sorted(set(indexed) - set(expected))[:8]
        fail(f"summary matrix incomplete: missing={missing}, extra={extra}")
    query_families: dict[str, set[str]] = {query: set() for query in QUERIES}
    for key in sorted(indexed):
        row = indexed[key]
        config = expected[key]
        label = f"summary[{key}]"
        validate_config_columns(row, config, label)
        if row["query_id"] not in config.query_ids or not row["query_family"]:
            fail(f"{label} has an invalid query")
        query_families[row["query_id"]].add(row["query_family"])
        if csv_int(row, "requests", label, 1) != config.requests_per_summary_row:
            fail(f"{label} request count is incomplete")
        if csv_int(row, "run_uuids", label, 1) != 5 or csv_int(row, "process_blocks", label, 1) != 5:
            fail(f"{label} does not span exactly five fixtures/blocks")
        for field in ("latency_median_ms", "latency_p95_ms", "latency_p99_ms", "latency_mad_ms"):
            value = csv_float(row, field, label)
            if value is None or value < 0:
                fail(f"{label}.{field} must be non-negative")
        if config.profile == "timing":
            for field in ("process_cpu_median_ms", "process_cpu_p95_ms", "process_cpu_p99_ms", "process_cpu_mad_ms"):
                value = csv_float(row, field, label)
                if value is None or value < 0:
                    fail(f"{label}.{field} must be non-negative")
            for field in ("allocation_operations_median", "allocated_bytes_median", "backend_operations_median"):
                if row[field] != "":
                    fail(f"{label}.{field} contaminates timing-profile output")
        else:
            for field in ("allocation_operations_median", "allocated_bytes_median"):
                value = csv_float(row, field, label)
                if value is None or value < 0:
                    fail(f"{label}.{field} is missing from instrumentation")
            needs_backend = config.lane == "native-http-assembly" and row["operation"] == GUARDED[config.lane]
            if needs_backend:
                if (csv_float(row, "backend_operations_median", label) or 0) <= 0:
                    fail(f"{label} lacks positive native guarded backend work")
            elif row["backend_operations_median"] != "":
                fail(f"{label} has backend counts outside native guarded instrumentation")
        csv_int(row, "result_rows", label, 0)
    inconsistent = {query: sorted(values) for query, values in query_families.items() if len(values) != 1}
    if inconsistent:
        fail(f"query-family labels are not invariant: {inconsistent}")
    return indexed


def validate_overhead(rows: list[dict[str, str]], configs: Sequence[Config], draws: int, seed: int) -> dict[tuple[str, ...], dict[str, str]]:
    timing = [config for config in configs if config.profile == "timing"]
    keys = ("campaign", "cell_label", "lane", "domain", "query_id")
    indexed = index_unique(rows, keys, "paired overhead")
    expected: dict[tuple[str, ...], Config] = {}
    for config in timing:
        for query in config.query_ids:
            expected[(*config_identity(config), query)] = config
    if set(indexed) != set(expected):
        missing = sorted(set(expected) - set(indexed))[:8]
        extra = sorted(set(indexed) - set(expected))[:8]
        fail(f"paired-overhead matrix incomplete: missing={missing}, extra={extra}")
    ratio_fields = (
        "latency_ratio_median", "latency_ratio_p95", "latency_ratio_ci95_low",
        "latency_ratio_ci95_high", "cpu_ratio_median", "cpu_ratio_ci95_low",
        "cpu_ratio_ci95_high",
    )
    difference_fields = (
        "latency_difference_median_ms", "latency_difference_p95_ms",
        "latency_difference_ci95_low_ms", "latency_difference_ci95_high_ms",
        "cpu_difference_median_ms", "cpu_difference_ci95_low_ms",
        "cpu_difference_ci95_high_ms",
    )
    for key in sorted(indexed):
        row, config = indexed[key], expected[key]
        label = f"paired-overhead[{key}]"
        validate_config_columns(row, config, label)
        if row["measurement_profile"] != "timing" or not row["query_family"]:
            fail(f"{label} is not a timing/query cell")
        if csv_int(row, "pairs", label, 1) != 150:
            fail(f"{label} must contain 150 pairs")
        if csv_int(row, "process_blocks", label, 1) != 5 or csv_int(row, "pairs_per_process_block", label, 1) != 30:
            fail(f"{label} must contain five balanced 30-pair blocks")
        if row["inference_status"] != "hierarchical-cluster-bootstrap":
            fail(f"{label} inference status is not canonical hierarchical-cluster-bootstrap")
        if csv_int(row, "bootstrap_draws", label, 10000) != draws or csv_int(row, "bootstrap_seed", label, 0) != seed:
            fail(f"{label} bootstrap controls disagree with analysis manifest")
        if row["bootstrap_method"] != H4_METHOD:
            fail(f"{label} bootstrap method is not the blinded amendment")
        values = {field: csv_float(row, field, label) for field in (*ratio_fields, *difference_fields)}
        if any((values[field] or 0) <= 0 for field in ratio_fields):
            fail(f"{label} contains a non-positive ratio")
        for prefix in ("latency_ratio", "cpu_ratio"):
            if values[f"{prefix}_ci95_low"] > values[f"{prefix}_ci95_high"]:  # type: ignore[operator]
                fail(f"{label} has a reversed {prefix} interval")
        for prefix in ("latency_difference", "cpu_difference"):
            if values[f"{prefix}_ci95_low_ms"] > values[f"{prefix}_ci95_high_ms"]:  # type: ignore[operator]
                fail(f"{label} has a reversed {prefix} interval")
    return indexed


def validate_construction(rows: list[dict[str, str]], configs: Sequence[Config]) -> dict[tuple[str, ...], dict[str, str]]:
    keys = ("campaign", "cell_label", "lane", "domain")
    indexed = index_unique(rows, keys, "construction")
    expected = {config_identity(config): config for config in configs}
    if set(indexed) != set(expected):
        missing = sorted(set(expected) - set(indexed))[:8]
        extra = sorted(set(indexed) - set(expected))[:8]
        fail(f"construction matrix incomplete: missing={missing}, extra={extra}")
    for key in sorted(indexed):
        row, config = indexed[key], expected[key]
        label = f"construction[{key}]"
        validate_config_columns(row, config, label)
        if csv_int(row, "process_blocks", label, 1) != 5:
            fail(f"{label} must span five process blocks")
        numeric: dict[str, float | None] = {}
        for metric in CONSTRUCTION_METRICS:
            numeric[metric] = csv_float(row, f"{metric}_median", label, allow_blank=True)
        required_positive = (
            "content_documents", "content_triples", "total_source_graphs",
            "corpus_generation_ns", "resident_bytes", "peak_resident_bytes",
        )
        for metric in required_positive:
            if (numeric[metric] or 0) <= 0:
                fail(f"{label}.{metric}_median must be positive")
        if (numeric["peak_resident_bytes"] or 0) < (numeric["resident_bytes"] or 0):
            fail(f"{label} peak RSS is below sampled RSS")
        target = numeric["target_readable_documents"]
        evaluation = numeric["evaluation_readable_documents"]
        total = numeric["content_documents"]
        if target is None or evaluation is None or total is None or not 0 <= target <= evaluation <= total:
            fail(f"{label} readable-document counts are invalid")
        materialized = config.lane == "materialized-routed"
        if materialized and (numeric["auth_triples_total"] or 0) <= 0:
            fail(f"{label}.auth_triples_total_median must be positive in the materialized lane")
        if not materialized and numeric["auth_triples_total"] is not None:
            fail(f"{label}.auth_triples_total_median must be blank in the native lane")
        lane_required = (
            ("graph_load_ns", "wac_materialization_ns", "route_index_ns")
            if materialized else ("lws_seed_ns", "http_store_max_total_bytes", "http_store_max_resource_count")
        )
        lane_forbidden = (
            ("lws_seed_ns", "http_store_max_total_bytes", "http_store_max_resource_count")
            if materialized else ("graph_load_ns", "wac_materialization_ns", "route_index_ns")
        )
        if any(numeric[field] is None for field in lane_required) or any(numeric[field] is not None for field in lane_forbidden):
            fail(f"{label} lane-specific construction fields are inconsistent")
        if not materialized and numeric["http_store_max_resource_count"] != numeric["total_source_graphs"] + 1:  # type: ignore[operator]
            fail(f"{label} native HTTP resource limit disagrees with source-graph count")
        allocations = ("construction_allocations", "construction_allocated_bytes")
        if config.profile == "instrumentation":
            if any(numeric[field] is None for field in allocations):
                fail(f"{label} lacks instrumented construction counters")
        elif any(numeric[field] is not None for field in allocations):
            fail(f"{label} reports counters in the timing profile")
    return indexed


def validate_h2(
    document: dict[str, Any], summary: Mapping[tuple[str, ...], Mapping[str, str]],
    manifest: Mapping[str, Any], draws: int, seed: int,
) -> dict[tuple[str, str, str], dict[str, Any]]:
    if set(document) != H2_TOP_FIELDS:
        fail(
            "h2.json fields changed: "
            f"missing={sorted(H2_TOP_FIELDS - set(document))}, "
            f"extra={sorted(set(document) - H2_TOP_FIELDS)}"
        )
    if document.get("schema_version") != ANALYZER_SCHEMA_VERSION:
        fail("h2.json schema version changed")
    if document.get("analysis") != "prospective H2 hierarchical cluster bootstrap":
        fail("h2.json analysis label changed")
    inputs = require_list(document.get("inputs"), "h2.inputs")
    manifest_inputs = [item["path"] for item in require_list(manifest.get("input_files"), "manifest.input_files")]
    if inputs != manifest_inputs:
        fail("h2 input paths disagree with analyzer manifest")
    indexed: dict[tuple[str, str, str], dict[str, Any]] = {}
    for index, item in enumerate(require_list(document.get("results"), "h2.results")):
        row = require_object(item, f"h2.results[{index}]")
        if set(row) != H2_RESULT_FIELDS:
            fail(
                f"h2.results[{index}] fields changed: "
                f"missing={sorted(H2_RESULT_FIELDS - set(row))}, "
                f"extra={sorted(set(row) - H2_RESULT_FIELDS)}"
            )
        key = (
            require_string(row.get("lane"), f"h2[{index}].lane"),
            require_string(row.get("domain"), f"h2[{index}].domain"),
            require_string(row.get("query_id"), f"h2[{index}].query_id"),
        )
        if key in indexed:
            fail(f"h2 duplicates {key}")
        indexed[key] = row
    expected = {(lane, domain, query) for lane in LANES for domain in DOMAINS for query in QUERIES}
    if set(indexed) != expected:
        fail(f"h2 matrix incomplete: missing={sorted(expected - set(indexed))}, extra={sorted(set(indexed) - expected)}")
    for key, row in sorted(indexed.items()):
        label = f"h2[{key}]"
        if require_list(row.get("pods"), f"{label}.pods") != list(POD_COUNTS):
            fail(f"{label} Pod vector changed")
        for field, expected_value in (
            ("p_min", 1), ("p_max", 2048), ("complete_process_blocks", 5),
            ("requests", 750), ("bootstrap_draws", draws), ("bootstrap_seed", seed),
        ):
            if require_int(row.get(field), f"{label}.{field}", 0) != expected_value:
                fail(f"{label}.{field} is incomplete or inconsistent")
        if require_list(row.get("corpus_seeds"), f"{label}.corpus_seeds") != list(BLOCK_SEEDS):
            fail(f"{label} corpus seeds changed")
        if require_number(row.get("ratio_margin"), f"{label}.ratio_margin") != 1.10 or require_number(row.get("elasticity_margin"), f"{label}.elasticity_margin") != 0.10:
            fail(f"{label} prospective margins changed")
        interval_fields = (
            "ratio_ci95", "elasticity_ci95", "process_cpu_ratio_ci95",
            "process_cpu_elasticity_ci95",
        )
        intervals: dict[str, tuple[float, float]] = {}
        for field in interval_fields:
            values = require_list(row.get(field), f"{label}.{field}")
            if len(values) != 2:
                fail(f"{label}.{field} must have two endpoints")
            low = float(require_number(values[0], f"{label}.{field}[0]"))
            high = float(require_number(values[1], f"{label}.{field}[1]"))
            if low > high:
                fail(f"{label}.{field} is reversed")
            intervals[field] = (low, high)
        for field in (
            "median_latency_ratio", "pod_elasticity", "median_process_cpu_ratio",
            "process_cpu_pod_elasticity",
        ):
            require_number(row.get(field), f"{label}.{field}")
        latency_minimal = intervals["ratio_ci95"][1] <= 1.10 and intervals["elasticity_ci95"][1] <= 0.10
        cpu_minimal = intervals["process_cpu_ratio_ci95"][1] <= 1.10 and intervals["process_cpu_elasticity_ci95"][1] <= 0.10
        if require_bool(row.get("minimal_latency_scaling"), f"{label}.minimal_latency_scaling") != latency_minimal:
            fail(f"{label} latency verdict does not follow the frozen rule")
        if require_bool(row.get("minimal_cpu_scaling"), f"{label}.minimal_cpu_scaling") != cpu_minimal:
            fail(f"{label} CPU verdict does not follow the frozen rule")
        if require_bool(row.get("minimal_overhead"), f"{label}.minimal_overhead") != (latency_minimal and cpu_minimal):
            fail(f"{label} joint verdict does not follow all four criteria")
        lane, domain, query = key
        wall_low = summary[("pod-scaling", "pods-1", lane, domain, query, GUARDED[lane])]
        wall_high = summary[("pod-scaling", "pods-2048", lane, domain, query, GUARDED[lane])]
        family = require_string(row.get("query_family"), f"{label}.query_family")
        if family != wall_low["query_family"] or family != wall_high["query_family"]:
            fail(f"{label} query-family label disagrees with summary.csv")
        expected_wall = float(wall_high["latency_median_ms"]) / float(wall_low["latency_median_ms"])
        expected_cpu = float(wall_high["process_cpu_median_ms"]) / float(wall_low["process_cpu_median_ms"])
        if not math.isclose(float(row["median_latency_ratio"]), expected_wall, rel_tol=1e-12, abs_tol=1e-12):
            fail(f"{label} wall endpoint ratio disagrees with summary.csv")
        if not math.isclose(float(row["median_process_cpu_ratio"]), expected_cpu, rel_tol=1e-12, abs_tol=1e-12):
            fail(f"{label} CPU endpoint ratio disagrees with summary.csv")
    return indexed


def verify_analyzer_manifest(derived: Path, metadata: Mapping[str, Any], repo_root: Path) -> tuple[dict[str, Any], dict[str, Artifact]]:
    manifest_path = derived / "manifest.json"
    manifest = read_json(manifest_path, "analyzer manifest")
    if manifest.get("schema_version") != ANALYZER_SCHEMA_VERSION or manifest.get("canonical_required") is not True:
        fail("analyzer manifest is not a schema-6 canonical analysis")
    source = require_object(metadata.get("source"), "metadata.source")
    expected_commit = require_string(source.get("git_commit"), "metadata.source.git_commit")
    if not HEX40.fullmatch(expected_commit) or manifest.get("expected_commit") != expected_commit:
        fail("source commit and canonical analyzer expectation disagree")
    if source.get("tree_clean") is not True:
        fail("benchmark source tree was not clean")
    draws = require_int(manifest.get("bootstrap_draws"), "manifest.bootstrap_draws", 10000)
    require_int(manifest.get("bootstrap_seed"), "manifest.bootstrap_seed", 0)
    expected_counts = {
        "observations": 114080,
        "construction_records": 1280,
        "correctness_records": 1280,
        # Every raw run emits one applicability record for each of the eight frozen
        # query templates, including templates not selected by sensitivity/scenario
        # campaigns.  3,680 is the selected-query count, not the emitted-record count.
        "applicability_records": 1280 * len(QUERIES),
    }
    for field, expected in expected_counts.items():
        if require_int(manifest.get(field), f"manifest.{field}", 1) != expected:
            fail(f"canonical manifest {field} is {manifest.get(field)!r}, expected {expected}")
    outputs = require_list(manifest.get("outputs"), "manifest.outputs")
    if set(outputs) != REQUIRED_ANALYZER_OUTPUTS or len(outputs) != len(set(outputs)):
        fail(f"analyzer output set changed: {outputs}")
    descriptors = require_list(manifest.get("output_artifacts"), "manifest.output_artifacts")
    artifacts: dict[str, Artifact] = {}
    for index, descriptor_value in enumerate(descriptors):
        descriptor = require_object(descriptor_value, f"manifest.output_artifacts[{index}]")
        name = require_string(descriptor.get("path"), f"manifest.output_artifacts[{index}].path")
        if Path(name).name != name or name not in REQUIRED_ANALYZER_OUTPUTS or name in artifacts:
            fail(f"invalid/duplicate analyzer output artifact {name!r}")
        digest = require_string(descriptor.get("sha256"), f"artifact {name}.sha256")
        if not HEX64.fullmatch(digest):
            fail(f"artifact {name} digest is malformed")
        size = require_int(descriptor.get("bytes"), f"artifact {name}.bytes", 1)
        path = derived / name
        if not path.is_file() or path.is_symlink():
            fail(f"analyzer artifact missing: {path}")
        actual = Artifact(name, path, sha256_file(path), path.stat().st_size)
        if actual.sha256 != digest or actual.bytes != size:
            fail(f"analyzer artifact drift: {name}")
        artifacts[name] = actual
    if set(artifacts) != REQUIRED_ANALYZER_OUTPUTS:
        fail("analyzer output_artifacts does not cover every required output")
    script = require_object(manifest.get("analysis_script"), "manifest.analysis_script")
    analyzer_rel = require_string(require_object(metadata.get("analysis"), "metadata.analysis").get("analyzer_path"), "metadata.analysis.analyzer_path")
    analyzer_path = (repo_root / analyzer_rel).resolve()
    ensure_inside(repo_root, analyzer_path, "analyzer path")
    analyzer_hash = sha256_file(analyzer_path)
    if script.get("sha256") != analyzer_hash:
        fail("derived analysis was not produced by the current committed analyzer bytes")
    manifest_artifact = Artifact("manifest.json", manifest_path, sha256_file(manifest_path), manifest_path.stat().st_size)
    artifacts["manifest.json"] = manifest_artifact
    return manifest, artifacts


def resolve_zstd(command: str) -> tuple[Path, str, str]:
    """Resolve and identify the exact external decompressor without invoking a shell."""

    candidate = shutil.which(command) if Path(command).name == command else command
    if candidate is None:
        fail(f"zstd executable not found: {command!r}")
    executable = Path(candidate).resolve()
    if not executable.is_file() or not os.access(executable, os.X_OK):
        fail(f"zstd executable is not an executable regular file: {executable}")
    try:
        completed = subprocess.run(
            [str(executable), "--version"],
            check=False,
            capture_output=True,
            text=True,
            timeout=10,
        )
    except (OSError, subprocess.SubprocessError) as error:
        raise PublicationError(f"cannot identify zstd executable: {error}") from error
    if completed.returncode != 0:
        fail(f"zstd --version failed with exit {completed.returncode}")
    version = (completed.stdout or completed.stderr).strip().splitlines()
    if not version:
        fail("zstd --version produced no version string")
    return executable, version[0], sha256_file(executable)


def safe_posix_member(value: str, label: str) -> PurePosixPath:
    path = PurePosixPath(value)
    if (
        path.is_absolute()
        or not path.parts
        or any(part in {"", ".", ".."} for part in path.parts)
        or str(path) != value
    ):
        fail(f"{label} must be a normalized safe relative POSIX path")
    return path


def scan_sensitive_name(name: str) -> None:
    path = safe_posix_member(name, f"archive member {name!r}")
    if any(SENSITIVE_MEMBER_COMPONENTS.search(part) for part in path.parts):
        fail(f"raw archive contains a credential-like member name: {name}")


def hash_and_scan_member(source: Any, name: str, *, capture: bool = False) -> tuple[str, bytes | None]:
    digest = hashlib.sha256()
    tail = b""
    captured = bytearray() if capture else None
    while chunk := source.read(1024 * 1024):
        digest.update(chunk)
        if captured is not None:
            if len(captured) + len(chunk) > 16 * 1024 * 1024:
                fail(f"captured archive member is unexpectedly large: {name}")
            captured.extend(chunk)
        searchable = tail + chunk
        for label, pattern in SENSITIVE_CONTENT_PATTERNS:
            if pattern.search(searchable):
                fail(f"raw archive sanitization failed: {label} detected in {name}")
        tail = searchable[-1024:]
    return digest.hexdigest(), bytes(captured) if captured is not None else None


def validate_primary_cross_p_invariant(
    run_root: Path, input_descriptors: Sequence[Any]
) -> dict[str, int | bool]:
    """Recheck the answer-equivalence premise across the primary P intervention.

    The analyzer checks repetitions and guarded/reference pairs within a process fixture.
    Publication additionally checks that each profile/lane/domain/query/block keeps the
    query, answer bag, row count, and target-readable construction count invariant as P
    changes.  Raw timings are deliberately neither retained nor inspected here.
    """

    constructions: dict[str, tuple[int, str, str, str, int, int]] = {}
    observations: list[dict[str, Any]] = []
    for index, value in enumerate(input_descriptors):
        descriptor = require_object(value, f"manifest.input_files[{index}]")
        name = Path(require_string(descriptor.get("path"), f"input_files[{index}].path")).name
        path = run_root / "raw" / name
        try:
            source = path.open(encoding="utf-8")
        except OSError as error:
            raise PublicationError(f"cannot read raw invariant input {path}") from error
        with source:
            for line_number, line in enumerate(source, 1):
                try:
                    record = json.loads(line)
                except json.JSONDecodeError as error:
                    raise PublicationError(f"invalid JSONL in {path}:{line_number}") from error
                if not isinstance(record, dict):
                    fail(f"raw record is not an object: {path}:{line_number}")
                campaign = record.get("campaign")
                if not isinstance(campaign, str) or campaign.removesuffix("-instrumentation") != "pod-scaling":
                    continue
                kind = record.get("record_type")
                if kind == "construction":
                    run_uuid = require_string(record.get("run_uuid"), f"{path}:{line_number}.run_uuid")
                    profile = require_string(record.get("measurement_profile"), f"{path}:{line_number}.measurement_profile")
                    lane = require_string(record.get("lane"), f"{path}:{line_number}.lane")
                    domain = require_string(record.get("domain"), f"{path}:{line_number}.domain")
                    block = require_int(record.get("process_block"), f"{path}:{line_number}.process_block", 0)
                    pods = require_int(record.get("pods"), f"{path}:{line_number}.pods", 1)
                    target = require_int(record.get("target_readable_documents"), f"{path}:{line_number}.target_readable_documents", 0)
                    identity = (target, profile, lane, domain, block, pods)
                    if run_uuid in constructions:
                        fail(f"primary raw run repeats its construction record: {run_uuid}")
                    constructions[run_uuid] = identity
                elif kind == "observation":
                    observations.append(record)

    groups: dict[tuple[str, str, str, str, int], dict[int, set[tuple[str, str, int, int]]]] = {}
    for record in observations:
        run_uuid = require_string(record.get("run_uuid"), "primary observation.run_uuid")
        construction = constructions.get(run_uuid)
        if construction is None:
            fail(f"primary observation cannot be joined to construction: {run_uuid}")
        target, c_profile, c_lane, c_domain, c_block, c_pods = construction
        profile = require_string(record.get("measurement_profile"), "primary observation.measurement_profile")
        lane = require_string(record.get("lane"), "primary observation.lane")
        domain = require_string(record.get("domain"), "primary observation.domain")
        block = require_int(record.get("process_block"), "primary observation.process_block", 0)
        pods = require_int(record.get("pods"), "primary observation.pods", 1)
        if (profile, lane, domain, block, pods) != (c_profile, c_lane, c_domain, c_block, c_pods):
            fail(f"primary observation/construction identity mismatch for run {run_uuid}")
        query = require_string(record.get("query_id"), "primary observation.query_id")
        query_hash = require_string(record.get("query_hash_sha256"), "primary observation.query_hash_sha256")
        result_hash = require_string(record.get("result_hash_sha256"), "primary observation.result_hash_sha256")
        if not HEX64.fullmatch(query_hash) or not HEX64.fullmatch(result_hash):
            fail("primary cross-P invariant contains a malformed query/result digest")
        result_rows = require_int(record.get("result_rows"), "primary observation.result_rows", 0)
        key = (profile, lane, domain, query, block)
        groups.setdefault(key, {}).setdefault(pods, set()).add(
            (query_hash, result_hash, result_rows, target)
        )

    expected_keys = {
        (profile, lane, domain, query, block)
        for profile in ("timing", "instrumentation")
        for lane in LANES
        for domain in DOMAINS
        for query in QUERIES
        for block in range(len(BLOCK_SEEDS))
    }
    if set(groups) != expected_keys:
        missing = sorted(expected_keys - set(groups))[:8]
        extra = sorted(set(groups) - expected_keys)[:8]
        fail(f"primary cross-P invariant matrix is incomplete: missing={missing}, extra={extra}")
    for key, by_pod in sorted(groups.items()):
        if set(by_pod) != set(POD_COUNTS):
            fail(f"primary cross-P invariant lacks a Pod level for {key}: {sorted(by_pod)}")
        identities: set[tuple[str, str, int, int]] = set()
        for pods, values in sorted(by_pod.items()):
            if len(values) != 1:
                fail(f"primary result identity changes within P={pods} for {key}")
            identities.update(values)
        if len(identities) != 1:
            fail(
                "primary cross-P invariant failed for "
                f"{key}: query hash, result hash/rows, or target-readable count changed"
            )
    return {
        "groups": len(groups),
        "pod_levels_per_group": len(POD_COUNTS),
        "query_hash_invariant": True,
        "result_hash_invariant": True,
        "result_rows_invariant": True,
        "target_readable_documents_invariant": True,
    }


def verify_zstd_tar(
    archive: Artifact,
    manifest_artifact: Artifact,
    entries: Mapping[str, str],
    archive_member: str,
    zstd_command: str,
) -> dict[str, Any]:
    executable, zstd_version, zstd_hash = resolve_zstd(zstd_command)
    with archive.path.open("rb") as source:
        if source.read(4) != ZSTD_MAGIC:
            fail("raw archive is not a standard zstd frame")
    manifest_member_path = safe_posix_member(archive_member, "raw archive manifest member")
    archive_prefix = manifest_member_path.parent
    expected = {
        archive_member: manifest_artifact.sha256,
        **{
            str(archive_prefix / safe_posix_member(name, f"raw manifest entry {name!r}")): digest
            for name, digest in entries.items()
        },
    }
    expected_names = sorted(expected)
    seen_names: list[str] = []
    manifest_payload: bytes | None = None
    process: subprocess.Popen[bytes] | None = None
    try:
        process = subprocess.Popen(
            [str(executable), "--decompress", "--stdout", "--quiet", str(archive.path)],
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
        )
        if process.stdout is None or process.stderr is None:  # pragma: no cover - Popen contract
            fail("cannot open zstd decompression streams")
        with tarfile.open(fileobj=process.stdout, mode="r|") as bundle:
            for member in bundle:
                scan_sensitive_name(member.name)
                if not member.isfile() or member.issym() or member.islnk():
                    fail(f"raw archive may contain only regular files: {member.name}")
                if member.name in seen_names:
                    fail(f"raw archive repeats member {member.name}")
                if (
                    member.uid != 0
                    or member.gid != 0
                    or member.uname != ""
                    or member.gname != ""
                    or member.mtime != 0
                    or member.mode & 0o777 != 0o644
                    or member.pax_headers
                ):
                    fail(f"raw archive member has non-deterministic metadata: {member.name}")
                seen_names.append(member.name)
                expected_digest = expected.get(member.name)
                if expected_digest is None:
                    fail(f"raw archive contains an unmanifested regular member: {member.name}")
                extracted = bundle.extractfile(member)
                if extracted is None:  # pragma: no cover - regular member contract
                    fail(f"cannot read raw archive member {member.name}")
                actual_digest, payload = hash_and_scan_member(
                    extracted, member.name, capture=member.name == archive_member
                )
                if actual_digest != expected_digest:
                    fail(f"raw archive member digest mismatch: {member.name}")
                if payload is not None:
                    manifest_payload = payload
        stderr = process.stderr.read().decode("utf-8", errors="replace").strip()
        returncode = process.wait(timeout=30)
        if returncode != 0:
            fail(f"zstd decompression failed with exit {returncode}: {stderr[:200]}")
    except (OSError, tarfile.TarError, subprocess.SubprocessError) as error:
        raise PublicationError(f"cannot verify deterministic raw-sanitized.tar.zst: {error}") from error
    finally:
        if process is not None and process.poll() is None:
            process.terminate()
            try:
                process.wait(timeout=5)
            except subprocess.TimeoutExpired:  # pragma: no cover - defensive process cleanup
                process.kill()
                process.wait()
        if process is not None:
            if process.stdout is not None:
                process.stdout.close()
            if process.stderr is not None:
                process.stderr.close()
    if seen_names != expected_names:
        if set(seen_names) == set(expected_names):
            fail("raw archive members are not bytewise POSIX-path sorted")
        missing = sorted(set(expected_names) - set(seen_names))[:8]
        extra = sorted(set(seen_names) - set(expected_names))[:8]
        fail(f"raw archive member set mismatch: missing={missing}, extra={extra}")
    if manifest_payload != manifest_artifact.path.read_bytes():
        fail("raw archive manifest member disagrees with the locally verified manifest")
    return {
        "manifest_member": archive_member,
        "manifest": {
            "sha256": manifest_artifact.sha256,
            "bytes": manifest_artifact.bytes,
            "entry_count": len(entries),
        },
        "regular_members": len(seen_names),
        "all_members_rehashed": True,
        "exact_member_set": True,
        "sanitization_scan_passed": True,
        "deterministic_tar_headers": True,
        "zstd": {"version": zstd_version, "executable_sha256": zstd_hash},
    }


def validate_raw_provenance(
    metadata: Mapping[str, Any], metadata_dir: Path, repo_root: Path,
    expected_archive_path: Path, manifest: Mapping[str, Any],
    artifacts: Mapping[str, Artifact], zstd_command: str,
) -> dict[str, Any]:
    raw = require_object(metadata.get("raw_run"), "metadata.raw_run")
    run_root = resolve_from(metadata_dir, raw.get("run_root"), "metadata.raw_run.run_root")
    if not run_root.is_dir():
        fail(f"raw run root is missing: {run_root}")
    manifest_artifact = verify_declared_file(metadata_dir, raw.get("manifest"), "metadata.raw_run.manifest")
    if manifest_artifact.path.parent.resolve() != run_root.resolve():
        fail("raw MANIFEST.sha256 must be at the run root")
    entries = parse_sha256_manifest(manifest_artifact.path, run_root)
    for mandatory in (
        "DONE", "started-at.txt", "finished-at.txt", "environment.txt",
        "provenance/ec2-original-MANIFEST.sha256",
    ):
        if mandatory not in entries:
            fail(f"raw run manifest lacks {mandatory}")
    for name, artifact in artifacts.items():
        if name == "manifest.json":
            member = "derived/manifest.json"
        elif name == "derived/manifest.json":
            member = name
        elif name in REQUIRED_ANALYZER_OUTPUTS:
            member = f"derived/{name}"
        elif name.startswith("reports/") or name.startswith("correctness/") or name.startswith("failures/"):
            member = name
        else:
            continue
        if entries.get(member) != artifact.sha256:
            fail(f"raw run manifest does not bind {member}")
    input_descriptors = require_list(manifest.get("input_files"), "manifest.input_files")
    if len(input_descriptors) != 1280:
        fail(f"canonical analyzer manifest has {len(input_descriptors)} raw inputs, expected 1280")
    seen_input_names: set[str] = set()
    for index, value in enumerate(input_descriptors):
        item = require_object(value, f"manifest.input_files[{index}]")
        name = Path(require_string(item.get("path"), f"input_files[{index}].path")).name
        digest = require_string(item.get("sha256"), f"input_files[{index}].sha256")
        if name in seen_input_names or entries.get(f"raw/{name}") != digest:
            fail(f"raw input {name!r} is duplicate or absent from the archived manifest")
        seen_input_names.add(name)
    cross_p = validate_primary_cross_p_invariant(run_root, input_descriptors)
    archive_obj = require_object(raw.get("archive"), "metadata.raw_run.archive")
    allowed_archive_fields = {"path", "sha256", "bytes", "public_url"}
    if set(archive_obj) - allowed_archive_fields:
        fail(f"metadata.raw_run.archive has unsupported fields: {sorted(set(archive_obj)-allowed_archive_fields)}")
    archive_rel = require_string(archive_obj.get("path"), "metadata.raw_run.archive.path")
    if Path(archive_rel).is_absolute():
        fail("committed raw archive path must be repository-relative")
    archive_path = (repo_root / archive_rel).resolve()
    ensure_inside(repo_root, archive_path, "committed raw archive")
    if archive_path != expected_archive_path.resolve() or archive_rel != ensure_inside(repo_root, expected_archive_path, "expected raw archive"):
        fail("raw archive must be the run-local committed raw-sanitized.tar.zst")
    archive = verify_declared_file(repo_root, archive_obj, "metadata.raw_run.archive")
    public_url = archive_obj.get("public_url")
    if public_url is not None:
        public_url = require_string(public_url, "metadata.raw_run.archive.public_url")
        if not public_url.startswith("https://"):
            fail("raw archive public_url must use HTTPS")
    archive_member = require_string(
        raw.get("manifest_archive_member"),
        "metadata.raw_run.manifest_archive_member",
    )
    expected_manifest_member = f"{expected_archive_path.parent.name}/MANIFEST.sha256"
    if archive_member != expected_manifest_member:
        fail(
            "raw archive manifest member must use the immutable run-ID prefix: "
            f"{expected_manifest_member}"
        )
    verification = verify_zstd_tar(
        archive, manifest_artifact, entries, archive_member, zstd_command
    )
    descriptor: dict[str, Any] = {
        "path": archive_rel,
        "sha256": archive.sha256,
        "bytes": archive.bytes,
    }
    if public_url is not None:
        descriptor["public_url"] = public_url
    archive_prefix = safe_posix_member(archive_member, "raw archive manifest member").parent
    archived_metadata: dict[str, Artifact] = {}
    for name in (
        "DONE", "started-at.txt", "finished-at.txt", "environment.txt",
        "provenance/ec2-original-MANIFEST.sha256", "derived/manifest.json",
    ):
        path = run_root / name
        archived_metadata[name] = Artifact(name, path, entries[name], path.stat().st_size)
    return {
        "descriptor": descriptor,
        "verification": verification,
        "cross_p": cross_p,
        "input_archive_members": {
            name: str(archive_prefix / "raw" / name) for name in sorted(seen_input_names)
        },
        "archived_metadata": archived_metadata,
    }


def validate_environment(metadata: Mapping[str, Any], metadata_dir: Path) -> tuple[dict[str, Any], int]:
    environment = require_object(metadata.get("environment"), "metadata.environment")
    raw = require_object(metadata.get("raw_run"), "metadata.raw_run")
    run_root = resolve_from(metadata_dir, raw.get("run_root"), "metadata.raw_run.run_root")
    env_values = parse_environment_file(run_root / "environment.txt")
    source_commit = require_string(require_object(metadata.get("source"), "metadata.source").get("git_commit"), "metadata.source.git_commit")
    comparisons = {
        "mode": "canonical",
        "source_commit": source_commit,
        "source_status": "",
        "instance_id": require_string(environment.get("instance_id"), "metadata.environment.instance_id"),
        "instance_type": require_string(environment.get("instance_type"), "metadata.environment.instance_type"),
        "region": require_string(environment.get("region"), "metadata.environment.region"),
        "cpuset": require_string(environment.get("cpu_affinity"), "metadata.environment.cpu_affinity"),
        "kernel": require_string(environment.get("kernel"), "metadata.environment.kernel"),
    }
    for key, expected in comparisons.items():
        if env_values.get(key) != expected:
            fail(f"environment metadata disagrees with environment.txt for {key}")
    if not re.fullmatch(r"[0-9]+", comparisons["cpuset"]):
        fail("CPU affinity must name exactly one logical CPU")
    architecture = require_string(environment.get("architecture"), "metadata.environment.architecture")
    lscpu: dict[str, str] = {}
    for field in env_values.get("lscpu", "").split(";"):
        if ":" in field:
            key, value = field.split(":", 1)
            lscpu[key.strip()] = value.strip()
    if lscpu.get("Architecture") != architecture:
        fail("architecture metadata disagrees with lscpu")
    cpu_model = require_string(environment.get("cpu_model"), "metadata.environment.cpu_model")
    if cpu_model not in lscpu.values() and cpu_model not in env_values.get("lscpu", ""):
        fail("CPU model metadata is not present in lscpu evidence")
    started_text = (run_root / "started-at.txt").read_text(encoding="utf-8").strip()
    finished_text = (run_root / "finished-at.txt").read_text(encoding="utf-8").strip()
    started = parse_utc(started_text, "started-at.txt")
    finished = parse_utc(finished_text, "finished-at.txt")
    duration = int((finished - started).total_seconds())
    if duration <= 0 or duration > 12 * 60 * 60 + 300:
        fail(f"canonical host duration {duration}s is outside the watchdog envelope")
    result = {
        "class": "controlled-single-process-ec2",
        "host_label": require_string(environment.get("host_label"), "metadata.environment.host_label"),
        "tenancy": "shared",
        "noise_limitation": require_string(environment.get("noise_limitation"), "metadata.environment.noise_limitation"),
        "provider": "aws",
        "region": comparisons["region"],
        "instance_id": comparisons["instance_id"],
        "instance_type": comparisons["instance_type"],
        "architecture": architecture,
        "os": "linux",
        "kernel": comparisons["kernel"],
        "cpu_model": cpu_model,
        "cpu_affinity": comparisons["cpuset"],
        "rayon_threads": 1,
        "allocator": require_string(environment.get("allocator"), "metadata.environment.allocator"),
        "started_at_utc": started_text,
        "finished_at_utc": finished_text,
        "host_duration_seconds": duration,
    }
    return result, duration


def validate_correctness_file_records(
    records: Sequence[dict[str, Any]], path: Path, source_commit: str
) -> dict[str, Any]:
    """Validate one frozen correctness run without reading performance outcomes.

    A correctness JSONL file is a complete one-repetition benchmark run, not a
    one-line gate file.  Timing, CPU, allocation, RSS, and backend-counter values are
    deliberately opaque here: their fields must be present, but this publisher-only
    check validates only the frozen record schema, run identity, applicability set,
    and paired-record composition before extracting the single gate.
    """

    expected_total = sum(CORRECTNESS_RECORD_COUNTS.values())
    if len(records) != expected_total:
        fail(f"correctness file must contain exactly {expected_total} records: {path}")
    by_kind: dict[str, list[dict[str, Any]]] = {
        kind: [] for kind in CORRECTNESS_RECORD_COUNTS
    }
    for index, record in enumerate(records, 1):
        kind = record.get("record_type")
        if kind not in CORRECTNESS_FIELDS_BY_KIND:
            fail(f"correctness file has unknown record_type {kind!r}: {path}:{index}")
        expected_fields = CORRECTNESS_COMMON_FIELDS | CORRECTNESS_FIELDS_BY_KIND[kind]
        fields = set(record)
        if fields != expected_fields:
            fail(
                f"correctness {kind} schema changed at {path}:{index}: "
                f"missing={sorted(expected_fields - fields)}, "
                f"extra={sorted(fields - expected_fields)}"
            )
        if record.get("schema_version") != ANALYZER_SCHEMA_VERSION:
            fail(
                f"correctness record lacks frozen schema version "
                f"{ANALYZER_SCHEMA_VERSION}: {path}:{index}"
            )
        if record.get("source_commit") != source_commit:
            fail(f"correctness record source commit mismatch: {path}:{index}")
        by_kind[kind].append(record)
    actual_counts = {kind: len(kind_records) for kind, kind_records in by_kind.items()}
    if actual_counts != CORRECTNESS_RECORD_COUNTS:
        fail(
            f"correctness file record composition changed: {path}: "
            f"expected={CORRECTNESS_RECORD_COUNTS}, actual={actual_counts}"
        )

    common_order = sorted(CORRECTNESS_COMMON_FIELDS)
    baseline = tuple(records[0][field] for field in common_order)
    if any(tuple(record[field] for field in common_order) != baseline for record in records[1:]):
        fail(f"correctness file changes common run metadata: {path}")
    common = records[0]
    lane = require_string(common.get("lane"), f"{path}.lane")
    domain = require_string(common.get("domain"), f"{path}.domain")
    principal = require_string(common.get("principal"), f"{path}.principal")
    coverage = require_int(
        common.get("own_acl_coverage_per_mille"), f"{path}.coverage", 0
    )
    seed = require_int(common.get("corpus_seed"), f"{path}.seed", 0)
    pods = require_int(common.get("pods"), f"{path}.pods", 1)
    if lane not in LANES or domain not in DOMAINS:
        fail(f"correctness file has an unknown lane/domain: {path}")
    if principal not in CORRECTNESS_PRINCIPALS:
        fail(f"correctness file has an unknown serialized principal {principal!r}: {path}")
    expected_cell_label = f"{CORRECTNESS_PRINCIPAL_CLI_LABEL[principal]}-{coverage}"
    expected_common = {
        "source_dirty": False,
        "profile": "release",
        "measurement_profile": "timing",
        "campaign": "correctness",
        "cell_label": expected_cell_label,
        "topology": TOPOLOGY[lane],
        "documents_per_pod": 8,
        "triples_per_document": 8,
        "container_depth": 3,
        "public_per_mille": 300,
        "private_per_mille": 400,
        "shared_per_mille": 300,
        "process_block": 0,
        "configuration_order": None,
        "configuration_order_seed": None,
        "warmups_configured": 0,
        "repetitions_configured": 1,
        "concurrency": 1,
        "rayon_threads": "1",
    }
    changed_common = [
        field for field, expected in expected_common.items() if common.get(field) != expected
    ]
    if changed_common:
        fail(f"correctness file changes frozen run metadata {changed_common}: {path}")
    if coverage not in (100, 1000) or seed not in (17, 42, 101) or pods not in (1, 8, 32):
        fail(f"correctness file is outside the frozen factor matrix: {path}")
    require_string(common.get("run_id"), f"{path}.run_id")
    require_string(common.get("run_uuid"), f"{path}.run_uuid")
    corpus_hash = require_string(common.get("corpus_hash_sha256"), f"{path}.corpus_hash_sha256")
    if not HEX64.fullmatch(corpus_hash):
        fail(f"correctness file corpus hash is not lowercase SHA-256: {path}")
    cpu_affinity = require_string(common.get("cpu_affinity"), f"{path}.cpu_affinity")
    if not re.fullmatch(r"[0-9]+", cpu_affinity):
        fail(f"correctness file is not pinned to one logical CPU: {path}")

    applicability_by_query: dict[str, dict[str, Any]] = {}
    for record in by_kind["applicability"]:
        query = require_string(record.get("query_id"), f"{path}.applicability.query_id")
        if query in applicability_by_query:
            fail(f"correctness file repeats applicability for {query}: {path}")
        expected_metadata = CORRECTNESS_QUERY_METADATA.get(query)
        if expected_metadata is None:
            fail(f"correctness file has unknown applicability query {query!r}: {path}")
        family, minimum_triples = expected_metadata
        if (
            record.get("query_family") != family
            or record.get("minimum_triples_per_document") != minimum_triples
            or record.get("applicable") is not True
            or record.get("selected") is not True
            or record.get("reason") is not None
        ):
            fail(f"correctness applicability metadata changed for {query}: {path}")
        query_hash = record.get("query_hash_sha256")
        if not isinstance(query_hash, str) or not HEX64.fullmatch(query_hash):
            fail(f"correctness applicability query hash is invalid for {query}: {path}")
        applicability_by_query[query] = record
    if set(applicability_by_query) != set(QUERIES):
        fail(f"correctness applicability query matrix is incomplete: {path}")

    observations: dict[tuple[str, str], dict[str, Any]] = {}
    observations_by_query: dict[str, list[dict[str, Any]]] = {query: [] for query in QUERIES}
    for record in by_kind["observation"]:
        query = require_string(record.get("query_id"), f"{path}.observation.query_id")
        operation = require_string(record.get("operation"), f"{path}.observation.operation")
        key = (query, operation)
        if query not in applicability_by_query or operation not in OPERATIONS[lane]:
            fail(f"correctness file has an off-matrix observation {key}: {path}")
        if key in observations:
            fail(f"correctness file repeats observation {key}: {path}")
        applicability = applicability_by_query[query]
        if (
            record.get("query_family") != applicability["query_family"]
            or record.get("query_hash_sha256") != applicability["query_hash_sha256"]
            or record.get("repetition") != 0
            or record.get("warmup") is not False
            or record.get("correctness") is not True
        ):
            fail(f"correctness observation metadata changed for {key}: {path}")
        expected_pair_id = f"{common['run_id']}:0:{query}:0"
        if record.get("pair_id") != expected_pair_id:
            fail(f"correctness observation pair identifier changed for {key}: {path}")
        order = record.get("order_in_pair")
        if isinstance(order, bool) or order not in (0, 1):
            fail(f"correctness observation pair order is invalid for {key}: {path}")
        observations[key] = record
        observations_by_query[query].append(record)
    expected_observations = {
        (query, operation) for query in QUERIES for operation in OPERATIONS[lane]
    }
    if set(observations) != expected_observations:
        fail(f"correctness observation matrix is incomplete: {path}")
    for query, pair in observations_by_query.items():
        if {record["order_in_pair"] for record in pair} != {0, 1}:
            fail(f"correctness observation pair order is incomplete for {query}: {path}")

    return by_kind["correctness-gate"][0]


def derive_correctness_gates(
    run_root: Path, source_commit: str
) -> tuple[dict[str, Any], dict[str, Artifact]]:
    correctness_root = run_root / "correctness"
    try:
        files = sorted(correctness_root.rglob("*.jsonl"))
    except OSError as error:
        raise PublicationError(f"cannot enumerate correctness evidence: {error}") from error
    if not files or any(not path.is_file() or path.is_symlink() for path in files):
        fail("checksummed correctness/*.jsonl evidence is missing or non-regular")
    gates: dict[tuple[str, str, str, int, int, int], dict[str, Any]] = {}
    run_uuids: set[str] = set()
    artifacts: dict[str, Artifact] = {}
    for path in files:
        rel = path.relative_to(run_root).as_posix()
        artifacts[rel] = Artifact(rel, path, sha256_file(path), path.stat().st_size)
        try:
            source = path.open(encoding="utf-8")
        except OSError as error:  # pragma: no cover - rglob race defence
            raise PublicationError(f"cannot read correctness evidence {path}") from error
        file_records: list[dict[str, Any]] = []
        with source:
            for line_number, line in enumerate(source, 1):
                try:
                    record = json.loads(line)
                except json.JSONDecodeError as error:
                    raise PublicationError(f"invalid correctness JSONL {path}:{line_number}") from error
                if not isinstance(record, dict):
                    fail(f"correctness JSONL record is not an object: {path}:{line_number}")
                file_records.append(record)
        record = validate_correctness_file_records(file_records, path, source_commit)
        lane = require_string(record.get("lane"), f"{path}.lane")
        domain = require_string(record.get("domain"), f"{path}.domain")
        principal = require_string(record.get("principal"), f"{path}.principal")
        coverage = require_int(record.get("own_acl_coverage_per_mille"), f"{path}.coverage", 0)
        seed = require_int(record.get("corpus_seed"), f"{path}.seed", 0)
        pods = require_int(record.get("pods"), f"{path}.pods", 1)
        key = (lane, domain, principal, coverage, seed, pods)
        if key in gates:
            fail(f"correctness matrix repeats {key}")
        run_uuid = require_string(record.get("run_uuid"), f"{path}.run_uuid")
        if run_uuid in run_uuids:
            fail(f"correctness matrix repeats run_uuid {run_uuid!r}")
        run_uuids.add(run_uuid)
        if record.get("exact_result_bags") is not True:
            fail(f"correctness gate failed exact result bags: {key}")
        if record.get("source_dirty") is not False:
            fail(f"correctness gate was not executed from a clean source tree: {key}")
        if record.get("documents_per_pod") != 8 or record.get("triples_per_document") != 8:
            fail(f"correctness gate corpus shape changed: {key}")
        if lane == "materialized-routed":
            expected_gate = "all four principal classes against physically filtered reference"
            expected_principals, expected_queries = 4, 32
        elif lane == "native-http-assembly":
            expected_gate = "primary principal through real HTTP route against physically filtered reference"
            expected_principals, expected_queries = 1, 8
        else:
            fail(f"correctness gate has unknown lane {lane!r}")
        if (
            record.get("gate") != expected_gate
            or record.get("principals_checked") != expected_principals
            or record.get("queries_checked") != expected_queries
        ):
            fail(f"correctness gate scope/count changed: {key}")
        gates[key] = record
    expected_keys = {
        (lane, domain, principal, coverage, seed, pods)
        for lane in LANES
        for domain in DOMAINS
        for principal in CORRECTNESS_PRINCIPALS
        for coverage in (100, 1000)
        for seed in (17, 42, 101)
        for pods in (1, 8, 32)
    }
    if set(gates) != expected_keys:
        missing = sorted(expected_keys-set(gates))[:8]
        extra = sorted(set(gates)-expected_keys)[:8]
        fail(f"correctness matrix is incomplete: missing={missing}, extra={extra}")
    comparisons = sum(int(record["queries_checked"]) for record in gates.values())
    materialized_comparisons = sum(
        int(record["queries_checked"]) for key, record in gates.items()
        if key[0] == "materialized-routed"
    )
    if comparisons != 5760 or materialized_comparisons != 4608:
        fail("derived correctness comparison totals disagree with the frozen lane scopes")
    return {
        "passed": True,
        "records": len(gates),
        "oracle_comparisons": comparisons,
        "materialized_oracle_comparisons": materialized_comparisons,
        "source_files": [
            {"path": name, "sha256": artifact.sha256, "bytes": artifact.bytes}
            for name, artifact in sorted(artifacts.items())
        ],
    }, artifacts


def validate_suite_attestations(
    annotation: Mapping[str, Any], run_root: Path, source_commit: str
) -> tuple[dict[str, bool], dict[str, Artifact]]:
    named_tests = {
        "replacing_a_generated_own_acl_revokes_the_recipient_immediately",
        "replacing_an_acl_revokes_the_native_query_route_immediately",
        "direct_no_leak_matches_ldp_get_authorization",
        "negation_cannot_distinguish_an_unreadable_resource_from_absence",
    }
    expected_commands = {
        ("cargo", "test", "--locked", "-p", "sparq-acbench", "--test", "deployment"),
        ("cargo", "test", "--locked", "-p", "sparq-solid", "--test", "acbench_deployment"),
        (
            "cargo", "test", "--locked", "-p", "sparq-lws-core", "--features",
            "sparql-endpoint", "--test", "sparql_endpoint",
        ),
    }
    attestations = require_list(annotation.get("suite_attestations"), "correctness annotation.suite_attestations")
    if len(attestations) != 3:
        fail("correctness annotation must bind exactly three frozen cargo test suites")
    found_tests: set[str] = set()
    artifacts: dict[str, Artifact] = {}
    commands: set[tuple[str, ...]] = set()
    for index, value in enumerate(attestations):
        item = require_object(value, f"correctness suite attestation {index}")
        if item.get("source_commit") != source_commit or item.get("exit_status") != 0:
            fail(f"correctness suite {index} lacks a clean source-commit/exit attestation")
        command_value = require_list(item.get("command"), f"correctness suite {index}.command")
        if not command_value or any(not isinstance(part, str) or not part for part in command_value):
            fail(f"correctness suite {index} command is invalid")
        command = tuple(command_value)
        if command in commands or command[:2] != ("cargo", "test"):
            fail(f"correctness suite {index} is duplicate or not a cargo test invocation")
        commands.add(command)
        log = verify_declared_file(run_root, item.get("log"), f"correctness suite {index}.log")
        rel = ensure_inside(run_root, log.path, f"correctness suite {index} log")
        text = log.path.read_text(encoding="utf-8", errors="strict")
        if f"source_commit={source_commit}" not in text or "test result: ok." not in text:
            fail(f"correctness suite {index} log lacks source commit or passing cargo summary")
        for test in named_tests:
            if re.search(rf"(?m)^test .*{re.escape(test)} .* ok$", text):
                found_tests.add(test)
        artifacts[rel] = Artifact(rel, log.path, log.sha256, log.bytes)
    if commands != expected_commands:
        fail("correctness annotations do not bind the three exact frozen Cargo commands")
    if found_tests != named_tests:
        fail(f"correctness suite logs omit named security probes: {sorted(named_tests-found_tests)}")
    return {
        "allow_all_owner": True,
        "allow_one_audience": True,
        "deny_all_stranger": True,
        "unreadable_absent": True,
        "explicit_and_variable_graph": True,
        "duplicate_multiset": True,
        "live_revocation": True,
    }, artifacts


def derive_exclusions(
    report: Mapping[str, Any], run_root: Path
) -> tuple[dict[str, Any], dict[str, Artifact]]:
    entries = require_list(report.get("entries"), "exclusions.entries")
    artifacts: dict[str, Artifact] = {}
    discovered: dict[str, tuple[str, Path, Artifact]] = {}
    failures_root = run_root / "failures"
    if not failures_root.is_dir() or failures_root.is_symlink():
        fail("retained failure-evidence directory is missing or unsafe")
    for sentinel_path in sorted(failures_root.rglob("FAILED.json")):
        if not sentinel_path.is_file() or sentinel_path.is_symlink():
            fail(f"retained failure sentinel is missing or unsafe: {sentinel_path}")
        sentinel_rel = ensure_inside(run_root, sentinel_path, "retained failure sentinel")
        sentinel_record = read_json(sentinel_path, f"retained failure sentinel {sentinel_rel}")
        attempt = require_string(sentinel_record.get("attempt_id"), f"{sentinel_rel}.attempt_id")
        reason = require_string(sentinel_record.get("reason_code"), f"{sentinel_rel}.reason_code")
        if (
            sentinel_record.get("schema_version") != 1
            or sentinel_record.get("status") != "FAILED"
            or require_int(sentinel_record.get("exit_status"), f"{sentinel_rel}.exit_status", 1) == 0
        ):
            fail(f"retained failure sentinel has invalid failure state: {sentinel_rel}")
        if attempt in discovered:
            fail(f"retained failure sentinels repeat attempt ID {attempt}")
        artifact = Artifact(
            sentinel_rel, sentinel_path, sha256_file(sentinel_path), sentinel_path.stat().st_size
        )
        discovered[attempt] = (reason, sentinel_path, artifact)

    annotated: set[str] = set()
    for index, value in enumerate(entries):
        entry = require_object(value, f"exclusions.entries[{index}]")
        allowed_entry_fields = {
            "attempt_id", "reason_code", "sentinel", "stderr", "partial_inputs", "note"
        }
        if set(entry) - allowed_entry_fields:
            fail(f"exclusion annotation {index} has unsupported claims")
        attempt = require_string(entry.get("attempt_id"), f"exclusion {index}.attempt_id")
        reason = require_string(entry.get("reason_code"), f"exclusion {index}.reason_code")
        if attempt in annotated:
            fail(f"exclusion report repeats attempt ID {attempt}")
        annotated.add(attempt)
        if attempt not in discovered:
            fail(f"exclusion annotation has no discovered FAILED.json sentinel: {attempt}")
        discovered_reason, discovered_path, discovered_artifact = discovered[attempt]
        sentinel = verify_declared_file(run_root, entry.get("sentinel"), f"exclusion {attempt}.sentinel")
        sentinel_rel = ensure_inside(run_root, sentinel.path, f"exclusion {attempt} sentinel")
        if sentinel.path.resolve() != discovered_path.resolve() or sentinel.sha256 != discovered_artifact.sha256 or reason != discovered_reason:
            fail(f"retained failure sentinel disagrees with exclusion annotation: {attempt}")
        artifacts[sentinel_rel] = discovered_artifact
        stderr = verify_declared_file(run_root, entry.get("stderr"), f"exclusion {attempt}.stderr")
        stderr_rel = ensure_inside(run_root, stderr.path, f"exclusion {attempt} stderr")
        artifacts[stderr_rel] = Artifact(stderr_rel, stderr.path, stderr.sha256, stderr.bytes)
        for evidence_index, evidence in enumerate(require_list(entry.get("partial_inputs", []), f"exclusion {attempt}.partial_inputs")):
            artifact = verify_declared_file(run_root, evidence, f"exclusion {attempt}.partial_inputs[{evidence_index}]")
            rel = ensure_inside(run_root, artifact.path, f"exclusion {attempt} partial input")
            if artifact.path.suffix == ".partial" or ".partial" in artifact.path.name:
                fail("raw-sanitized archive must not include .partial files")
            artifacts[rel] = Artifact(rel, artifact.path, artifact.sha256, artifact.bytes)
    if annotated != set(discovered):
        fail(
            "exclusion annotations do not bind every discovered FAILED.json sentinel: "
            f"missing={sorted(set(discovered)-annotated)}, extra={sorted(annotated-set(discovered))}"
        )
    return {
        "attempts": len(discovered),
        "all_discovered_sentinels_bound": True,
        "source_files": [
            {"path": name, "sha256": artifact.sha256, "bytes": artifact.bytes}
            for name, artifact in sorted(artifacts.items())
        ],
    }, artifacts


def validate_cost_and_support(
    metadata: Mapping[str, Any], metadata_dir: Path, source_commit: str,
    host_duration: int,
) -> tuple[dict[str, Any], dict[str, Any], dict[str, Any], dict[str, Artifact]]:
    cost_descriptor = require_object(metadata.get("cost_report"), "metadata.cost_report")
    cost_artifact = verify_declared_file(metadata_dir, cost_descriptor, "metadata.cost_report")
    cost = read_json(cost_artifact.path, "cost report")
    if cost.get("schema_version") != 1 or cost.get("currency") != "USD" or cost.get("status") != "final-accounted":
        fail("cost report is not a final accounted USD report")
    accounting_method = require_string(cost.get("accounting_method"), "cost.accounting_method")
    estimated = require_bool(cost.get("estimated"), "cost.estimated")
    permitted_methods = {
        "duration-times-official-rate-plus-ebs-ipv4": True,
        "aws-cost-explorer-tag-scoped-final": False,
    }
    if accounting_method not in permitted_methods or estimated is not permitted_methods[accounting_method]:
        fail("cost accounting method and estimated flag are inconsistent")
    if require_int(cost.get("canonical_host_duration_seconds"), "cost.canonical_host_duration_seconds", 1) != host_duration:
        fail("cost report host duration disagrees with run timestamps")
    components = require_object(cost.get("components_usd"), "cost.components_usd")
    expected_component_keys = {
        "canonical_compute", "canonical_storage", "canonical_public_ipv4",
        "prior_valid_and_invalid_attempts", "other",
    }
    if set(components) != expected_component_keys:
        fail(f"cost component set changed: {sorted(components)}")
    component_values = {name: decimal_value(value, f"cost.components_usd.{name}") for name, value in components.items()}
    if not component_values:
        fail("cost report has no components")
    study_total = decimal_value(cost.get("study_total_usd"), "cost.study_total_usd")
    canonical_total = decimal_value(cost.get("canonical_run_usd"), "cost.canonical_run_usd")
    if sum(component_values.values(), Decimal(0)) != study_total:
        fail("cost components do not sum exactly to study_total_usd")
    canonical_components = sum(
        (component_values[name] for name in ("canonical_compute", "canonical_storage", "canonical_public_ipv4")),
        Decimal(0),
    )
    if canonical_components != canonical_total:
        fail("canonical cost components do not sum exactly to canonical_run_usd")
    if canonical_total > study_total or study_total > Decimal("100"):
        fail("cost report violates canonical<=study<=USD100")
    cost_output = {
        "status": "final-accounted",
        "accounting_method": accounting_method,
        "estimated": estimated,
        "currency": "USD",
        "study_total_usd_decimal": str(study_total),
        "canonical_run_usd_decimal": str(canonical_total),
        "components_usd": {name: str(value) for name, value in sorted(component_values.items())},
        "basis": require_string(cost.get("basis"), "cost.basis"),
        "report": {"sha256": cost_artifact.sha256, "bytes": cost_artifact.bytes},
    }

    raw = require_object(metadata.get("raw_run"), "metadata.raw_run")
    run_root = resolve_from(metadata_dir, raw.get("run_root"), "metadata.raw_run.run_root")
    correctness_artifact = verify_declared_file(metadata_dir, metadata.get("correctness_report"), "metadata.correctness_report")
    correctness_annotation = read_json(correctness_artifact.path, "correctness annotation")
    if (
        correctness_annotation.get("schema_version") != 1
        or correctness_annotation.get("source_commit") != source_commit
        or set(correctness_annotation) - {"schema_version", "source_commit", "note", "suite_attestations"}
    ):
        fail("correctness annotation may identify scope but may not assert pass/count claims")
    require_string(correctness_annotation.get("note"), "correctness annotation.note")
    correctness_output, correctness_artifacts = derive_correctness_gates(run_root, source_commit)
    probes, suite_artifacts = validate_suite_attestations(correctness_annotation, run_root, source_commit)
    correctness_output["all_required_probes_passed"] = all(probes.values())
    correctness_output["probes"] = probes
    correctness_output["suite_files"] = [
        {"path": name, "sha256": artifact.sha256, "bytes": artifact.bytes}
        for name, artifact in sorted(suite_artifacts.items())
    ]
    correctness_output["report"] = {"sha256": correctness_artifact.sha256, "bytes": correctness_artifact.bytes}

    exclusions_artifact = verify_declared_file(metadata_dir, metadata.get("exclusion_report"), "metadata.exclusion_report")
    exclusions = read_json(exclusions_artifact.path, "exclusion report")
    if exclusions.get("schema_version") != 1 or set(exclusions) - {"schema_version", "note", "entries"}:
        fail("exclusion report may annotate discovered sentinels but may not assert pass/count claims")
    require_string(exclusions.get("note"), "exclusion report.note")
    exclusions_output, exclusion_artifacts = derive_exclusions(exclusions, run_root)
    exclusions_output["report"] = {"sha256": exclusions_artifact.sha256, "bytes": exclusions_artifact.bytes}
    supporting = {**correctness_artifacts, **suite_artifacts, **exclusion_artifacts}
    return cost_output, correctness_output, exclusions_output, supporting


def escape(value: Any) -> str:
    return html.escape(str(value), quote=True)


COLORS = ("#2166ac", "#67a9cf", "#f7a35c", "#ef8a62", "#b2182b")


def axis_ticks(low: float, high: float, count: int = 4) -> list[float]:
    if not math.isfinite(low) or not math.isfinite(high):
        fail("cannot plot a non-finite range")
    if low == high:
        pad = abs(low) * 0.1 or 1.0
        low, high = low - pad, high + pad
    return [low + (high - low) * index / count for index in range(count + 1)]


def render_guarded_stack_content_reference(rows: Mapping[tuple[str, ...], Mapping[str, str]], sources: Mapping[str, Artifact], publisher_hash: str) -> bytes:
    primary = [
        row for key, row in sorted(rows.items()) if key[0] == "pod-scaling"
    ]
    if len(primary) != len(LANES) * len(DOMAINS) * len(POD_COUNTS) * len(QUERIES):
        fail("guarded-stack/content-reference figure cannot see every primary cell")
    metrics = (
        ("latency_ratio", "wall guarded/reference ratio", 1.0),
        ("latency_difference", "wall guarded-reference (ms)", 0.0),
        ("cpu_ratio", "CPU guarded/reference ratio", 1.0),
        ("cpu_difference", "CPU guarded-reference (ms)", 0.0),
    )
    facets = [(lane, domain) for lane in LANES for domain in DOMAINS]
    panel_w, panel_h = 395, 235
    left, top, gap_x, gap_y = 78, 76, 38, 54
    width = left + len(facets) * panel_w + (len(facets) - 1) * gap_x + 30
    height = top + len(metrics) * panel_h + (len(metrics) - 1) * gap_y + 105
    parts = [
        f'<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{height}" viewBox="0 0 {width} {height}">',
        '<rect width="100%" height="100%" fill="white"/>',
        '<style>text{font-family:system-ui,sans-serif;fill:#202124}.axis{stroke:#555}.grid{stroke:#e1e1e1}.ci{stroke-width:1.2}.ref{stroke:#777;stroke-dasharray:4 4}</style>',
        '<title>Guarded-stack/content-reference contrast for every primary Pod-count cell</title>',
        f'<metadata>source paired-overhead.csv sha256={sources["paired-overhead.csv"].sha256}; publisher sha256={publisher_hash}</metadata>',
        f'<text x="{width/2:.1f}" y="26" text-anchor="middle" font-size="18" font-weight="600">Guarded-stack/content-reference contrast: complete primary matrix</text>',
        f'<text x="{width/2:.1f}" y="48" text-anchor="middle" font-size="12">Five Pod counts are shown separately; domains, lanes, and query families are not pooled.</text>',
    ]
    for col, (lane, domain) in enumerate(facets):
        x0 = left + col * (panel_w + gap_x)
        parts.append(f'<text x="{x0 + panel_w/2:.1f}" y="68" text-anchor="middle" font-size="13" font-weight="600">{escape(LANE_SHORT[lane])} / {escape(domain)}</text>')
    for row_index, (prefix, label, reference) in enumerate(metrics):
        y0 = top + row_index * (panel_h + gap_y)
        parts.append(f'<text x="18" y="{y0 + panel_h/2:.1f}" transform="rotate(-90 18 {y0 + panel_h/2:.1f})" text-anchor="middle" font-size="12">{escape(label)}</text>')
        for col, (lane, domain) in enumerate(facets):
            x0 = left + col * (panel_w + gap_x)
            facet = [r for r in primary if r["lane"] == lane and r["domain"] == domain]
            if prefix.endswith("ratio"):
                center_field = f"{prefix}_median"
                low_field, high_field = f"{prefix}_ci95_low", f"{prefix}_ci95_high"
            else:
                center_field = f"{prefix}_median_ms"
                low_field, high_field = f"{prefix}_ci95_low_ms", f"{prefix}_ci95_high_ms"
            values = [float(r[field]) for r in facet for field in (low_field, high_field)] + [reference]
            lo, hi = min(values), max(values)
            pad = (hi - lo) * 0.09 or (abs(hi) * 0.09 or 1.0)
            lo, hi = lo - pad, hi + pad
            def py(value: float) -> float:
                return y0 + panel_h - (value - lo) / (hi - lo) * panel_h
            parts.append(f'<rect x="{x0}" y="{y0}" width="{panel_w}" height="{panel_h}" fill="none" stroke="#bbb"/>')
            for tick in axis_ticks(lo, hi):
                yy = py(tick)
                parts.append(f'<line class="grid" x1="{x0}" y1="{yy:.2f}" x2="{x0+panel_w}" y2="{yy:.2f}"/><text x="{x0-5}" y="{yy+4:.2f}" text-anchor="end" font-size="9">{tick:.3g}</text>')
            parts.append(f'<line class="ref" x1="{x0}" y1="{py(reference):.2f}" x2="{x0+panel_w}" y2="{py(reference):.2f}"/>')
            for query_index, query in enumerate(QUERIES):
                base_x = x0 + (query_index + 0.5) * panel_w / len(QUERIES)
                parts.append(f'<text x="{base_x:.2f}" y="{y0+panel_h+14}" text-anchor="middle" font-size="8">{escape(QUERY_SHORT[query])}</text>')
                for pod_index, pod in enumerate(POD_COUNTS):
                    cell = next((r for r in facet if r["query_id"] == query and int(r["pods"]) == pod), None)
                    if cell is None:
                        fail(f"guarded-stack/content-reference figure is missing {lane}/{domain}/{query}/P={pod}")
                    xx = base_x + (pod_index - 2) * 6.2
                    center = float(cell[center_field])
                    low_value, high_value = float(cell[low_field]), float(cell[high_field])
                    color = COLORS[pod_index]
                    parts.append(f'<line class="ci" stroke="{color}" x1="{xx:.2f}" y1="{py(low_value):.2f}" x2="{xx:.2f}" y2="{py(high_value):.2f}"/><circle cx="{xx:.2f}" cy="{py(center):.2f}" r="2.4" fill="{color}"/>')
    legend_y = height - 50
    parts.append(f'<text x="{width/2-190}" y="{legend_y}" font-size="11">resident Pods:</text>')
    for index, pod in enumerate(POD_COUNTS):
        xx = width / 2 - 95 + index * 65
        parts.append(f'<circle cx="{xx}" cy="{legend_y-4}" r="4" fill="{COLORS[index]}"/><text x="{xx+8}" y="{legend_y}" font-size="10">{pod}</text>')
    parts.append(f'<text x="{width/2:.1f}" y="{height-20}" text-anchor="middle" font-size="11">Query-answer-equivalent content-only reference; not pure access-control overhead. Bars are marginal 95% hierarchical block-bootstrap intervals.</text>')
    parts.append('</svg>')
    return ("\n".join(parts) + "\n").encode("utf-8")


def factor_for_label(label: str) -> tuple[str, float, str]:
    if label.startswith("documents-"):
        return "documents", float(label.split("-")[1]), label.split("-")[1]
    if label.startswith("triples-"):
        return "triples", float(label.split("-")[1]), label.split("-")[1]
    if label.startswith("placement-"):
        _, coverage, depth = label.split("-")
        return "policy", float(int(coverage) * 10 + int(depth)), f"{int(coverage)/10:g}%/d{depth}"
    if label.startswith("visibility-"):
        public = int(label.split("-")[1])
        return "visibility", float(public), f"{public/10:g}%"
    fail(f"unknown sensitivity label {label}")


def render_factor_sensitivities(summary: Mapping[tuple[str, ...], Mapping[str, str]], sources: Mapping[str, Artifact], publisher_hash: str) -> bytes:
    factors = ("documents", "triples", "policy", "visibility")
    rowspec = (
        ("materialized-routed", "timing", "latency_median_ms", "routed latency (ms)"),
        ("native-http-assembly", "timing", "latency_median_ms", "HTTP latency (ms)"),
        ("materialized-routed", "instrumentation", "allocation_operations_median", "routed allocations"),
        ("native-http-assembly", "instrumentation", "allocation_operations_median", "HTTP allocations"),
        ("native-http-assembly", "instrumentation", "backend_operations_median", "HTTP backend ops"),
    )
    selected = []
    for key, row in sorted(summary.items()):
        campaign, _, lane, _, _, operation = key
        if campaign.removesuffix("-instrumentation") != "sensitivity" or operation != GUARDED[lane]:
            continue
        selected.append(row)
    expected_count = 2 * 25 * 2 * 2 * 2  # profiles, domain specs, lanes, queries
    if len(selected) != expected_count:
        fail(f"sensitivity figure selection has {len(selected)} rows, expected {expected_count}")
    panel_w, panel_h, gap_x, gap_y = 355, 210, 42, 46
    left, top = 82, 78
    width = left + len(factors) * panel_w + (len(factors)-1)*gap_x + 30
    height = top + len(rowspec)*panel_h + (len(rowspec)-1)*gap_y + 95
    parts = [
        f'<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{height}" viewBox="0 0 {width} {height}">',
        '<rect width="100%" height="100%" fill="white"/>',
        '<style>text{font-family:system-ui,sans-serif;fill:#202124}.grid{stroke:#e3e3e3}.line{fill:none;stroke-width:1.8}.axis{stroke:#666}</style>',
        '<title>Complete AC-SPARQL factor sensitivity matrix</title>',
        f'<metadata>source summary.csv sha256={sources["summary.csv"].sha256}; publisher sha256={publisher_hash}</metadata>',
        f'<text x="{width/2:.1f}" y="26" text-anchor="middle" font-size="18" font-weight="600">Independent factor sensitivities</text>',
        f'<text x="{width/2:.1f}" y="48" text-anchor="middle" font-size="12">Descriptive medians only; instrumentation counters explain work and instrumentation timings are excluded.</text>',
    ]
    titles = {"documents":"documents/Pod", "triples":"triples/document", "policy":"own-ACL coverage / depth", "visibility":"public readable fraction"}
    series_styles = {
        ("health", "q1-point"): ("#2166ac", ""),
        ("health", "q8-graph-scan"): ("#67a9cf", "4 3"),
        ("social", "q1-point"): ("#b2182b", ""),
        ("social", "q8-graph-scan"): ("#ef8a62", "4 3"),
    }
    for col, factor in enumerate(factors):
        x0 = left + col*(panel_w+gap_x)
        parts.append(f'<text x="{x0+panel_w/2:.1f}" y="70" text-anchor="middle" font-size="13" font-weight="600">{escape(titles[factor])}</text>')
    for row_index, (lane, profile, metric, row_label) in enumerate(rowspec):
        y0 = top + row_index*(panel_h+gap_y)
        parts.append(f'<text x="18" y="{y0+panel_h/2:.1f}" transform="rotate(-90 18 {y0+panel_h/2:.1f})" text-anchor="middle" font-size="11">{escape(row_label)}</text>')
        campaign = "sensitivity" if profile == "timing" else "sensitivity-instrumentation"
        for col, factor in enumerate(factors):
            x0 = left + col*(panel_w+gap_x)
            cells = []
            for item in selected:
                item_factor, xvalue, xlabel = factor_for_label(item["cell_label"])
                if item_factor == factor and item["campaign"] == campaign and item["lane"] == lane:
                    if metric == "backend_operations_median" and item[metric] == "":
                        fail("native guarded instrumentation lacks backend operations")
                    cells.append((item, xvalue, xlabel, float(item[metric])))
            if not cells:
                fail(f"sensitivity figure lacks {lane}/{profile}/{factor}")
            labels = sorted({(xvalue, xlabel) for _, xvalue, xlabel, _ in cells})
            x_positions = {pair: x0 + (index+0.5)*panel_w/len(labels) for index, pair in enumerate(labels)}
            positive = [value for *_, value in cells if value > 0]
            if len(positive) != len(cells):
                fail(f"sensitivity metric {metric} must be positive")
            log_values = [math.log10(value) for value in positive]
            lo, hi = min(log_values), max(log_values)
            pad = (hi-lo)*0.08 or 0.15
            lo, hi = lo-pad, hi+pad
            def py(value: float) -> float:
                return y0 + panel_h - (math.log10(value)-lo)/(hi-lo)*panel_h
            parts.append(f'<rect x="{x0}" y="{y0}" width="{panel_w}" height="{panel_h}" fill="none" stroke="#bbb"/>')
            for tick in axis_ticks(lo, hi, 3):
                yy = y0 + panel_h - (tick-lo)/(hi-lo)*panel_h
                parts.append(f'<line class="grid" x1="{x0}" y1="{yy:.2f}" x2="{x0+panel_w}" y2="{yy:.2f}"/><text x="{x0-5}" y="{yy+4:.2f}" text-anchor="end" font-size="8">10^{tick:.1f}</text>')
            for label_pair, xx in x_positions.items():
                parts.append(f'<text x="{xx:.2f}" y="{y0+panel_h+13}" text-anchor="middle" font-size="7">{escape(label_pair[1])}</text>')
            for series, (color, dash) in series_styles.items():
                points = sorted((x_positions[(xv, xl)], value) for item, xv, xl, value in cells if (item["domain"], item["query_id"]) == series)
                if len(points) != len(labels):
                    fail(f"sensitivity figure series {series} is incomplete for {factor}")
                poly = " ".join(f"{xx:.2f},{py(value):.2f}" for xx, value in points)
                dash_attr = f' stroke-dasharray="{dash}"' if dash else ""
                parts.append(f'<polyline class="line" stroke="{color}"{dash_attr} points="{poly}"/>')
                for xx, value in points:
                    parts.append(f'<circle cx="{xx:.2f}" cy="{py(value):.2f}" r="2.2" fill="{color}"/>')
    legend_y = height-45
    legends = (("health q1", "#2166ac", ""), ("health q8", "#67a9cf", "4 3"), ("social q1", "#b2182b", ""), ("social q8", "#ef8a62", "4 3"))
    for index, (label, color, dash) in enumerate(legends):
        xx = width/2-270+index*180
        dash_attr = f' stroke-dasharray="{dash}"' if dash else ""
        parts.append(f'<line x1="{xx}" y1="{legend_y}" x2="{xx+28}" y2="{legend_y}" stroke="{color}" stroke-width="2"{dash_attr}/><text x="{xx+34}" y="{legend_y+4}" font-size="10">{escape(label)}</text>')
    parts.append(f'<text x="{width/2:.1f}" y="{height-15}" text-anchor="middle" font-size="10">All q1/q8, social/health, and declared factor levels are shown. Axes are panel-local log scales; no trend fit is claimed.</text>')
    parts.append('</svg>')
    return ("\n".join(parts)+"\n").encode("utf-8")


def render_construction_resident(construction: Mapping[tuple[str, ...], Mapping[str, str]], sources: Mapping[str, Artifact], publisher_hash: str) -> bytes:
    primary = [
        row for key, row in sorted(construction.items())
        if key[0].removesuffix("-instrumentation") == "pod-scaling"
    ]
    if len(primary) != 2 * len(LANES) * len(DOMAINS) * len(POD_COUNTS):
        fail("construction figure cannot see the complete primary timing/instrumentation matrix")
    timing = [row for row in primary if row["measurement_profile"] == "timing"]
    panels = (
        ("materialized phases", "pods", "phase"),
        ("native service seeding", "pods", "seed"),
        ("resident memory", "pods", "rss"),
        ("resident memory vs content", "triples", "rss"),
    )
    width, height = 1320, 770
    panel_w, panel_h, gap = 560, 250, 90
    starts = ((100, 80), (100+panel_w+gap,80), (100, 405), (100+panel_w+gap,405))
    parts = [
        f'<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{height}" viewBox="0 0 {width} {height}">',
        '<rect width="100%" height="100%" fill="white"/>',
        '<style>text{font-family:system-ui,sans-serif;fill:#202124}.grid{stroke:#e3e3e3}.line{fill:none;stroke-width:2}.axis{stroke:#666}</style>',
        '<title>Cold construction phases and conservative whole-process RSS</title>',
        f'<metadata>source construction.csv sha256={sources["construction.csv"].sha256}; publisher sha256={publisher_hash}</metadata>',
        '<text x="660" y="27" text-anchor="middle" font-size="18" font-weight="600">Cold construction and resident-capacity indicators</text>',
        '<text x="660" y="48" text-anchor="middle" font-size="12">Timing-profile construction medians; RSS includes the oracle and peak RSS includes generation.</text>',
    ]
    palette = {"health":"#2166ac", "social":"#b2182b"}
    for panel_index, (title, xkind, kind) in enumerate(panels):
        x0, y0 = starts[panel_index]
        series: dict[str, list[tuple[float,float]]] = {}
        if kind == "phase":
            for domain in DOMAINS:
                for metric, suffix in (("graph_load_ns_median","load"),("wac_materialization_ns_median","WAC"),("route_index_ns_median","route")):
                    series[f"{domain} {suffix}"] = sorted((float(r["pods"]), float(r[metric])/1e6) for r in timing if r["lane"]=="materialized-routed" and r["domain"]==domain)
        elif kind == "seed":
            for domain in DOMAINS:
                series[domain] = sorted((float(r["pods"]), float(r["lws_seed_ns_median"])/1e6) for r in timing if r["lane"]=="native-http-assembly" and r["domain"]==domain)
        else:
            for lane in LANES:
                for domain in DOMAINS:
                    for metric, suffix in (("resident_bytes_median","current"),("peak_resident_bytes_median","peak")):
                        series[f"{LANE_SHORT[lane]} {domain} {suffix}"] = sorted(((float(r["pods"]) if xkind=="pods" else float(r["content_triples_median"])), float(r[metric])/(1024**3)) for r in timing if r["lane"]==lane and r["domain"]==domain)
        if any(len(values) != len(POD_COUNTS) for values in series.values()):
            fail(f"construction figure panel {title} has an incomplete series")
        all_x = [x for values in series.values() for x,_ in values]
        all_y = [y for values in series.values() for _,y in values]
        if any(value <= 0 for value in (*all_x,*all_y)):
            fail(f"construction figure panel {title} requires positive log inputs")
        lx0,lx1=min(map(math.log10,all_x)),max(map(math.log10,all_x))
        ly0,ly1=min(map(math.log10,all_y)),max(map(math.log10,all_y))
        if lx0==lx1: lx1+=1
        if ly0==ly1: ly1+=1
        ly0-=0.08*(ly1-ly0); ly1+=0.08*(ly1-ly0)
        def px(value: float)->float: return x0+(math.log10(value)-lx0)/(lx1-lx0)*panel_w
        def py(value: float)->float: return y0+panel_h-(math.log10(value)-ly0)/(ly1-ly0)*panel_h
        parts.append(f'<text x="{x0+panel_w/2}" y="{y0-15}" text-anchor="middle" font-size="14" font-weight="600">{escape(title)}</text><rect x="{x0}" y="{y0}" width="{panel_w}" height="{panel_h}" fill="none" stroke="#bbb"/>')
        for tick in axis_ticks(ly0,ly1,4):
            yy=y0+panel_h-(tick-ly0)/(ly1-ly0)*panel_h
            parts.append(f'<line class="grid" x1="{x0}" y1="{yy:.2f}" x2="{x0+panel_w}" y2="{yy:.2f}"/><text x="{x0-7}" y="{yy+4:.2f}" text-anchor="end" font-size="9">10^{tick:.1f}</text>')
        dash_patterns=("","5 3","2 3","7 3 2 3","10 3","3 2")
        for index,(name,values) in enumerate(sorted(series.items())):
            domain="health" if "health" in name else "social"
            color=palette[domain]
            dash=dash_patterns[index%len(dash_patterns)]
            dash_attr=f' stroke-dasharray="{dash}"' if dash else ""
            points=" ".join(f"{px(x):.2f},{py(y):.2f}" for x,y in values)
            parts.append(f'<polyline class="line" stroke="{color}"{dash_attr} points="{points}"/><text x="{px(values[-1][0])+4:.2f}" y="{py(values[-1][1])+3:.2f}" font-size="8">{escape(name)}</text>')
            for x,y in values: parts.append(f'<circle cx="{px(x):.2f}" cy="{py(y):.2f}" r="2.5" fill="{color}"/>')
        xlabel="resident Pods" if xkind=="pods" else "realized content triples"
        ylabel="milliseconds" if kind in {"phase","seed"} else "GiB"
        parts.append(f'<text x="{x0+panel_w/2}" y="{y0+panel_h+23}" text-anchor="middle" font-size="11">{xlabel} (log)</text><text x="{x0-58}" y="{y0+panel_h/2}" transform="rotate(-90 {x0-58} {y0+panel_h/2})" text-anchor="middle" font-size="11">{ylabel} (log)</text>')
    parts.append('<text x="660" y="750" text-anchor="middle" font-size="10">Lines connect descriptive medians for every P level; no fitted linear model or capacity limit is inferred.</text></svg>')
    return ("\n".join(parts)+"\n").encode("utf-8")


def pointer_escape(token: str) -> str:
    return token.replace("~", "~0").replace("/", "~1")


def set_pointer(document: dict[str, Any], tokens: Sequence[str], value: Any) -> str:
    current = document
    for token in tokens[:-1]:
        child = current.setdefault(token, {})
        if not isinstance(child, dict):
            fail(f"result path collision at {tokens}")
        current = child
    if tokens[-1] in current:
        fail(f"duplicate result path {tokens}")
    current[tokens[-1]] = value
    return "/results/" + "/".join(pointer_escape(token) for token in tokens)


class ResultBuilder:
    def __init__(self, artifacts: Mapping[str, Artifact]) -> None:
        self.results: dict[str, Any] = {}
        self.evidence: dict[str, Any] = {}
        self.bindings: dict[str, Any] = {}
        self.keys: set[str] = set()
        self.artifacts = artifacts

    def add(
        self, tokens: Sequence[str], value: int | float | bool, *, key: str, unit: str,
        estimator: str, note: str, sources: Sequence[tuple[str, str]],
        hypothesis: str | None = None, paper_evidence: bool = True,
    ) -> None:
        if not KEY_RE.fullmatch(key):
            fail(f"invalid paper evidence key {key!r}")
        if key in self.keys:
            fail(f"duplicate result/evidence key {key}")
        self.keys.add(key)
        if isinstance(value, float) and not math.isfinite(value):
            fail(f"non-finite result for {key}")
        if isinstance(value, bool):
            if unit != "boolean" or hypothesis not in {"H1", "H2"}:
                fail(f"Boolean result {key} must be an H1/H2 mechanical verdict")
        elif hypothesis is not None:
            fail(f"numeric/descriptive result {key} cannot declare a hypothesis verdict")
        pointer = set_pointer(self.results, tokens, value)
        bindings = []
        for artifact_name, locator in sources:
            artifact = self.artifacts[artifact_name]
            bindings.append({
                "artifact": artifact_name,
                "sha256": artifact.sha256,
                "locator": locator,
            })
        if not bindings:
            fail(f"result {key} has no source binding")
        self.bindings[pointer] = bindings
        if not paper_evidence:
            return
        declaration = {
            "pointer": pointer,
            "unit": unit,
            "estimator": estimator,
            "note": note,
            "papers": [PAPER_SLUG],
        }
        if hypothesis is not None:
            declaration["hypothesis"] = hypothesis
        self.evidence[key] = declaration


def h2_source_locator(row_index: int, field: str) -> str:
    return f"/results/{row_index}/{field}"


def result_scalar_pointers(value: Any, tokens: tuple[str, ...] = ()) -> set[str]:
    """Enumerate every admissible scalar leaf in the object-keyed result tree."""

    pointers: set[str] = set()
    if isinstance(value, dict):
        for key, child in value.items():
            if not isinstance(key, str) or not key:
                fail("result object keys must be non-empty strings")
            pointers.update(result_scalar_pointers(child, (*tokens, key)))
    elif isinstance(value, list):
        for index, child in enumerate(value):
            pointers.update(result_scalar_pointers(child, (*tokens, str(index))))
    elif isinstance(value, bool):
        pointers.add("/results/" + "/".join(pointer_escape(token) for token in tokens))
    elif isinstance(value, (int, float)) and math.isfinite(float(value)):
        pointers.add("/results/" + "/".join(pointer_escape(token) for token in tokens))
    else:
        fail(f"results contain a non-finite or non-scalar leaf at {tokens}")
    return pointers


def validate_result_binding_coverage(
    builder: ResultBuilder,
    input_files: Sequence[Mapping[str, Any]],
    output_files: Sequence[Mapping[str, Any]],
) -> None:
    pointers = result_scalar_pointers(builder.results)
    if pointers != set(builder.bindings):
        fail("result_bindings do not exactly cover every numeric/Boolean result leaf")
    available: dict[str, str] = {}
    for item in output_files:
        name, digest = str(item["name"]), str(item["sha256"])
        if name in available:
            fail(f"duplicate output/input artifact name in binding resolver: {name}")
        available[name] = digest
    for item in input_files:
        name, digest = str(item["path"]), str(item["sha256"])
        if name in available:
            fail(f"duplicate output/input artifact name in binding resolver: {name}")
        available[name] = digest
    for pointer, values in builder.bindings.items():
        if not isinstance(values, list) or not values:
            fail(f"result binding is empty: {pointer}")
        seen: set[tuple[str, str, str]] = set()
        for binding in values:
            if set(binding) != {"artifact", "sha256", "locator"}:
                fail(f"result binding has an unsupported field: {pointer}")
            artifact = require_string(binding.get("artifact"), f"binding {pointer}.artifact")
            digest = require_string(binding.get("sha256"), f"binding {pointer}.sha256")
            locator = require_string(binding.get("locator"), f"binding {pointer}.locator")
            triple = (artifact, digest, locator)
            if triple in seen:
                fail(f"result binding repeats a source triple: {pointer}")
            seen.add(triple)
            if available.get(artifact) != digest:
                fail(f"result binding source is absent or hash-mismatched: {pointer}: {artifact}")
    for key, declaration in builder.evidence.items():
        pointer = require_string(declaration.get("pointer"), f"paper evidence {key}.pointer")
        if pointer not in builder.bindings:
            fail(f"paper evidence lacks a complete result binding: {key}")


def build_results(
    h2_document: Mapping[str, Any], h2: Mapping[tuple[str,str,str], Mapping[str,Any]],
    summary: Mapping[tuple[str,...], Mapping[str,str]],
    overhead: Mapping[tuple[str,...], Mapping[str,str]],
    construction: Mapping[tuple[str,...], Mapping[str,str]], manifest: Mapping[str,Any],
    correctness: Mapping[str,Any], exclusions: Mapping[str,Any], cost: Mapping[str,Any],
    host_duration: int, artifacts: dict[str,Artifact],
) -> ResultBuilder:
    builder = ResultBuilder(artifacts)
    h2_rows = require_list(h2_document.get("results"), "h2.results")
    h2_indices = {(row["lane"],row["domain"],row["query_id"]): index for index,row in enumerate(h2_rows)}
    field_specs = (
        ("median_latency_ratio", ("wall","median_ratio"), "ratio"),
        ("ratio_ci95", ("wall","ratio_ci95"), "interval"),
        ("pod_elasticity", ("wall","pod_elasticity"), "elasticity"),
        ("elasticity_ci95", ("wall","elasticity_ci95"), "interval"),
        ("median_process_cpu_ratio", ("process_cpu","median_ratio"), "ratio"),
        ("process_cpu_ratio_ci95", ("process_cpu","ratio_ci95"), "interval"),
        ("process_cpu_pod_elasticity", ("process_cpu","pod_elasticity"), "elasticity"),
        ("process_cpu_elasticity_ci95", ("process_cpu","elasticity_ci95"), "interval"),
    )
    for (lane, domain, query), row in sorted(h2.items()):
        base = ("h2", lane, domain, query)
        keybase = f"ac_sparql.h2.{LANE_SHORT[lane]}.{domain}.{QUERY_SHORT[query]}"
        row_index = h2_indices[(lane,domain,query)]
        for source_field, target, kind in field_specs:
            value = row[source_field]
            if kind == "interval":
                for endpoint_index, endpoint in enumerate(("low","high")):
                    builder.add(
                        (*base,*target,endpoint), float(value[endpoint_index]),
                        key=f"{keybase}.{'.'.join(target)}.{endpoint}", unit="ratio" if any("ratio" in token for token in target) else "elasticity",
                        estimator="marginal 95% hierarchical corpus/process-block bootstrap endpoint",
                        note=f"Prospective {lane}, {domain}, {query} H2 {'.'.join(target)} {endpoint} endpoint.",
                        sources=(("h2.json",h2_source_locator(row_index,f"{source_field}/{endpoint_index}")),),
                    )
            else:
                builder.add(
                    (*base,*target), float(value), key=f"{keybase}.{'.'.join(target)}",
                    unit="ratio" if kind=="ratio" else "elasticity",
                    estimator="P=2048/P=1 median endpoint ratio" if kind=="ratio" else "equal-weight mean of complete-block log-log slopes",
                    note=f"Prospective {lane}, {domain}, {query} H2 {'.'.join(target)} estimate.",
                    sources=(("h2.json",h2_source_locator(row_index,source_field)),),
                )
        for field in ("minimal_latency_scaling","minimal_cpu_scaling","minimal_overhead"):
            builder.add(
                (*base,field), bool(row[field]), key=f"{keybase}.{field}", unit="boolean",
                estimator="frozen upper-confidence-bound decision rule" if field!="minimal_overhead" else "conjunction of all four frozen wall/CPU criteria",
                note=f"Derived, query-specific {field.replace('_',' ')} verdict; cells are never pooled.",
                sources=(("h2.json",h2_source_locator(row_index,field)),),
                hypothesis="H2",
            )
    for lane in LANES:
        values=[bool(row["minimal_overhead"]) for (cell_lane,_,_),row in sorted(h2.items()) if cell_lane==lane]
        if len(values)!=16: fail(f"H2 rollup for {lane} is incomplete")
        pass_count=sum(values); fail_count=len(values)-pass_count
        lane_key=LANE_SHORT[lane]
        rollup_sources=tuple(("h2.json",h2_source_locator(h2_indices[key],"minimal_overhead")) for key in sorted(h2) if key[0]==lane)
        for name,value,unit in (
            ("cells",len(values),"cells"),("pass_count",pass_count,"cells"),("fail_count",fail_count,"cells"),
            ("all_cells_meet",all(values),"boolean"),("no_cells_meet",not any(values),"boolean"),("mixed",0<pass_count<len(values),"boolean"),
        ):
            builder.add(("h2_rollup",lane,name),value,key=f"ac_sparql.h2.{lane_key}.rollup.{name}",unit=unit,
                estimator="mechanical count/Boolean over all sixteen predeclared domain-by-query verdicts",
                note=f"No query selection: {name.replace('_',' ')} across all sixteen {lane} H2 cells.",sources=rollup_sources,
                hypothesis="H2" if isinstance(value,bool) else None)

    # Stable object-keyed representation of every admitted summary outcome.  Timing
    # profiles expose wall/CPU summaries; instrumentation profiles expose work counters,
    # never their diagnostically perturbed timings.
    common_summary_fields = (
        "requests", "run_uuids", "process_blocks", "result_rows",
    )
    timing_summary_fields = (
        "latency_median_ms", "latency_p95_ms", "latency_p99_ms", "latency_mad_ms",
        "process_cpu_median_ms", "process_cpu_p95_ms", "process_cpu_p99_ms",
        "process_cpu_mad_ms",
    )
    instrumentation_summary_fields = (
        "allocation_operations_median", "allocated_bytes_median",
        "backend_operations_median",
    )
    for (campaign, cell, lane, domain, query, operation), row in sorted(summary.items()):
        profile = row["measurement_profile"]
        fields = common_summary_fields + (
            timing_summary_fields if profile == "timing" else instrumentation_summary_fields
        )
        locator_base = (
            f"campaign={campaign};cell={cell};lane={lane};domain={domain};"
            f"query={query};operation={operation}"
        )
        for field in fields:
            if row[field] == "":
                continue
            integer = field in {"requests", "run_uuids", "process_blocks", "result_rows"}
            value: int | float = int(row[field]) if integer else float(row[field])
            if field.endswith("_ms"):
                unit = "milliseconds"
            elif "bytes" in field:
                unit = "bytes/request"
            elif "operations" in field:
                unit = "operations/request"
            elif field == "requests":
                unit = "requests"
            elif field == "run_uuids":
                unit = "fixtures"
            elif field == "process_blocks":
                unit = "blocks"
            else:
                unit = "rows"
            builder.add(
                ("request_cells", campaign, lane, domain, query, operation, cell, field),
                value,
                key=f"ac_sparql.request.{campaign}.{LANE_SHORT[lane]}.{domain}.{QUERY_SHORT[query]}.{operation}.{cell}.{field}",
                unit=unit,
                estimator=(
                    "complete five-block timing-profile summary"
                    if profile == "timing"
                    else "complete deterministic instrumentation-profile summary"
                ),
                note=(
                    "Atomic complete-cell outcome; no query, domain, lane, or factor selection."
                    if profile == "timing"
                    else "Atomic work counter; instrumentation-profile timing is not admitted as latency evidence."
                ),
                sources=(("summary.csv", f"{locator_base};field={field}"),),
                paper_evidence=False,
            )

    # Every paired timing cell is retained.  This is a guarded-stack/content-reference
    # contrast against a query-answer-equivalent content-only physical reference, not a
    # pure authorization overhead or an otherwise-identical same-data counterfactual.
    overhead_fields = (
        "pairs", "process_blocks", "pairs_per_process_block",
        "latency_ratio_median", "latency_ratio_p95", "latency_ratio_ci95_low",
        "latency_ratio_ci95_high", "latency_difference_median_ms",
        "latency_difference_p95_ms", "latency_difference_ci95_low_ms",
        "latency_difference_ci95_high_ms", "cpu_ratio_median", "cpu_ratio_ci95_low",
        "cpu_ratio_ci95_high", "cpu_difference_median_ms",
        "cpu_difference_ci95_low_ms", "cpu_difference_ci95_high_ms",
    )
    for (campaign, cell, lane, domain, query), row in sorted(overhead.items()):
        locator_base = (
            f"campaign={campaign};cell={cell};lane={lane};domain={domain};query={query}"
        )
        for field in overhead_fields:
            value = int(row[field]) if field in {"pairs", "process_blocks", "pairs_per_process_block"} else float(row[field])
            if field.endswith("_ms"):
                unit = "milliseconds"
            elif "ratio" in field:
                unit = "ratio"
            elif field == "pairs":
                unit = "pairs"
            elif field == "process_blocks":
                unit = "blocks"
            else:
                unit = "pairs/block"
            builder.add(
                ("guarded_stack_content_reference", campaign, lane, domain, query, cell, field),
                value,
                key=f"ac_sparql.guarded_stack_content_reference.{campaign}.{LANE_SHORT[lane]}.{domain}.{QUERY_SHORT[query]}.{cell}.{field}",
                unit=unit,
                estimator=(
                    "marginal 95% hierarchical block-bootstrap endpoint"
                    if "ci95" in field
                    else "paired complete-cell descriptive estimate"
                ),
                note="Guarded-stack/content-reference contrast against a query-answer-equivalent content-only physical reference; no binary H4 verdict.",
                sources=(("paired-overhead.csv", f"{locator_base};field={field}"),),
                paper_evidence=False,
            )

    # Construction is emitted without collapsing the two architectural lanes.  Blank
    # lane-inapplicable fields are omitted; all present numeric fields are addressable.
    construction_design_fields = (
        "pods", "documents_per_pod", "triples_per_document", "container_depth",
        "own_acl_coverage_per_mille", "public_per_mille", "private_per_mille",
        "shared_per_mille", "process_blocks",
    )
    for (campaign, cell, lane, domain), row in sorted(construction.items()):
        locator_base = f"campaign={campaign};cell={cell};lane={lane};domain={domain}"
        for field in (*construction_design_fields, *(f"{metric}_median" for metric in CONSTRUCTION_METRICS)):
            if row[field] == "":
                continue
            value = float(row[field])
            if field.endswith("_ns_median"):
                unit = "nanoseconds"
            elif "bytes" in field:
                unit = "bytes"
            elif field.endswith("_per_mille"):
                unit = "per-mille"
            elif field == "process_blocks":
                unit = "blocks"
            elif field == "pods":
                unit = "Pods"
            else:
                unit = "count"
            builder.add(
                ("construction", campaign, lane, domain, cell, field),
                value,
                key=f"ac_sparql.construction.{campaign}.{LANE_SHORT[lane]}.{domain}.{cell}.{field}",
                unit=unit,
                estimator="median across five complete process fixtures" if field.endswith("_median") else "frozen cell design value",
                note="Lane-specific construction/capacity value; RSS is whole-process and completion is not a capacity maximum.",
                sources=(("construction.csv", f"{locator_base};field={field}"),),
                paper_evidence=False,
            )
    timing_total=sum(int(row["requests"]) for row in summary.values() if row["measurement_profile"]=="timing")
    instrumentation_total=sum(int(row["requests"]) for row in summary.values() if row["measurement_profile"]=="instrumentation")
    paired_cells=len(summary)//2
    fixtures=sum(int(row["process_blocks"]) for row in construction.values())
    campaign_values=(
        ("valid_observations",require_int(manifest.get("observations"),"manifest.observations",1)),
        ("valid_timing_observations",timing_total),("valid_instrumentation_observations",instrumentation_total),
        ("completed_paired_cells",paired_cells),("complete_process_fixtures",fixtures),
        ("raw_files",len(require_list(manifest.get("input_files"),"manifest.input_files"))),
        ("excluded_attempts",int(exclusions["attempts"])),
    )
    if timing_total+instrumentation_total != int(manifest["observations"]):
        fail("summary request totals disagree with analyzer manifest observations")
    campaign_sources=(("derived/manifest.json","/observations"),("summary.csv","all complete rows"),("construction.csv","all complete rows"))
    for name,value in campaign_values:
        sources = (
            tuple(
                (str(item["path"]), "verified retained failure evidence")
                for item in require_list(exclusions.get("source_files"), "exclusions.source_files")
            ) + (("reports/exclusion-report.json", "checked exclusion annotation index"),)
            if name == "excluded_attempts"
            else campaign_sources
        )
        builder.add(("campaign",name),value,key=f"ac_sparql.campaign.{name}",unit="count",estimator="mechanical validated-manifest/matrix count",note=f"Canonical admitted {name.replace('_',' ')}.",sources=sources)
    completeness = (
        ("primary_h4_cells", 160, "cells", (("paired-overhead.csv", "campaign=pod-scaling; all 160 rows"),)),
        ("all_h4_cells", 368, "cells", (("paired-overhead.csv", "all complete timing-profile rows"),)),
        ("h4_cells_with_complete_clustered_intervals", 368, "cells", (("paired-overhead.csv", "all CI endpoint fields in all 368 rows"),)),
        ("sensitivity_guarded_cells", 200, "cells", (("summary.csv", "campaign=sensitivity; guarded rows"),)),
        ("construction_cells", 256, "cells", (("construction.csv", "all complete timing/instrumentation rows"),)),
    )
    for name, value, unit, sources in completeness:
        builder.add(
            ("campaign", name), value, key=f"ac_sparql.campaign.{name}", unit=unit,
            estimator="exact frozen-matrix completeness check",
            note=f"Completeness indicator for {name.replace('_', ' ')}; not an effect verdict.",
            sources=sources,
        )
    correctness_gate_sources=tuple(
        (str(item["path"]), "complete checksummed correctness run file")
        for item in require_list(correctness.get("source_files"), "correctness.source_files")
    )
    correctness_suite_sources=tuple(
        (str(item["path"]), "source-commit-attested passing cargo test log")
        for item in require_list(correctness.get("suite_files"), "correctness.suite_files")
    )
    for name,value,unit in (
        ("records",int(correctness["records"]),"configurations"),
        ("oracle_comparisons",int(correctness["oracle_comparisons"]),"comparisons"),
        ("materialized_oracle_comparisons",int(correctness["materialized_oracle_comparisons"]),"comparisons"),
        ("passed",True,"boolean"),("all_required_probes_passed",True,"boolean"),
    ):
        sources = correctness_gate_sources + (
            correctness_suite_sources if isinstance(value, bool) else ()
        )
        builder.add(("correctness",name),value,key=f"ac_sparql.correctness.{name}",unit=unit,
            estimator="sum/validation over the complete raw correctness-gate matrix",note="Correctness-only evidence for the generated WAC subset; counts are derived from checksummed gate records.",sources=sources,hypothesis="H1" if isinstance(value,bool) else None)
    # Deterministic native mechanism endpoints: retain both domains, never choose one.
    for domain in DOMAINS:
        values=[]
        for pod in (1,2048):
            row=summary[("pod-scaling-instrumentation",f"pods-{pod}","native-http-assembly",domain,"q1-point",GUARDED["native-http-assembly"])]
            value=float(row["backend_operations_median"])
            values.append(value)
            locator=f"campaign=pod-scaling-instrumentation;cell=pods-{pod};lane=native-http-assembly;domain={domain};query=q1-point;operation=native-http-request;field=backend_operations_median"
            builder.add(("mechanism","native_backend_operations",domain,f"pods_{pod}"),value,
                key=f"ac_sparql.mechanism.http_backend_ops.{domain}.p{pod}",unit="operations/request",estimator="median deterministic instrumentation count across five process blocks",note=f"Native q1 backend work at P={pod}; instrumentation timing is not used.",sources=(("summary.csv",locator),))
        builder.add(("mechanism","native_backend_operations",domain,"growth_ratio"),values[1]/values[0],
            key=f"ac_sparql.mechanism.http_backend_ops.{domain}.growth_ratio",unit="ratio",estimator="P=2048 median operations divided by P=1 median operations",note="Mechanism count ratio; not a latency estimate.",sources=(("summary.csv",f"native {domain} q1 endpoint rows"),))
    # Scenario table: lane-specific values avoid collapsing unlike implementations.
    scenario_alias={"social-count-small":"social_small","social-count-anchor":"social_anchor","health-compact-small":"health_small","health-compact-anchor":"health_anchor"}
    for label,alias in sorted(scenario_alias.items()):
        for lane in LANES:
            domain="social" if label.startswith("social") else "health"
            row=construction[("scenarios",label,lane,domain)]
            lane_key=LANE_SHORT[lane]
            metrics=("content_documents","content_triples","control_documents","control_triples","resident_bytes","peak_resident_bytes")
            if lane=="materialized-routed": metrics=(*metrics,"graph_load_ns","wac_materialization_ns","route_index_ns")
            else: metrics=(*metrics,"lws_seed_ns")
            locator_base=f"campaign=scenarios;cell={label};lane={lane};domain={domain}"
            for metric in metrics:
                value=float(row[f"{metric}_median"])
                unit="bytes" if "bytes" in metric else ("nanoseconds" if metric.endswith("_ns") else "count")
                builder.add(("scenarios",alias,lane,metric),value,
                    key=f"ac_sparql.scenario.{alias}.{lane_key}.{metric}",unit=unit,estimator="median across five complete construction fixtures",note=f"Scenario {label}, {lane}, {metric}; a completed cell is not a capacity maximum.",sources=(("construction.csv",f"{locator_base};field={metric}_median"),))
            builder.add(("scenarios",alias,lane,"complete_process_blocks"),5,
                key=f"ac_sparql.scenario.{alias}.{lane_key}.complete_process_blocks",unit="blocks",estimator="five-block matrix-completeness validation",note="Completed block count only; this named synthetic configuration is not a capacity maximum.",sources=(("construction.csv",locator_base),))
    cost_estimator = ("accounted direct-resource estimate" if cost["estimated"] else "tag-scoped final AWS billing")
    builder.add(("artifact","study_cost_usd"),float(Decimal(cost["study_total_usd_decimal"])),key="ac_sparql.artifact.study_cost_usd",unit="USD",estimator=cost_estimator,note="Study-wide total includes valid and invalid attempts and all declared direct-resource components.",sources=(("reports/cost-report.json","/study_total_usd"),))
    builder.add(("artifact","canonical_run_cost_usd"),float(Decimal(cost["canonical_run_usd_decimal"])),key="ac_sparql.artifact.canonical_run_cost_usd",unit="USD",estimator=cost_estimator,note="Canonical-run direct-resource component, reported separately from the study-wide total.",sources=(("reports/cost-report.json","/canonical_run_usd"),))
    builder.add(("artifact","canonical_host_duration_seconds"),host_duration,key="ac_sparql.artifact.canonical_host_duration_seconds",unit="seconds",estimator="finished-at minus started-at",note="Disposable canonical host wall duration, bounded by the watchdog.",sources=(("started-at.txt","complete UTC timestamp"),("finished-at.txt","complete UTC timestamp")))
    return builder


def figure_descriptor(repo_root: Path, path: Path, payload: bytes) -> dict[str, Any]:
    rel=ensure_inside(repo_root,path,"publication figure")
    if not rel.startswith("site/papers/figures/canonical-timing/") or not rel.endswith(".svg"):
        fail(f"publication figure is outside the reserved tree: {rel}")
    return {
        "path": rel,
        "sha256": sha256_bytes(payload),
        "media_type": "image/svg+xml",
        "output_name": path.name,
    }


def validate_svg_payload(payload: bytes, label: str) -> None:
    try:
        text = payload.decode("utf-8")
    except UnicodeDecodeError as error:
        raise PublicationError(f"{label} is not UTF-8 SVG") from error
    lowered = text.lower()
    if "<svg" not in lowered or 'xmlns="http://www.w3.org/2000/svg"' not in lowered:
        fail(f"{label} lacks an SVG root/namespace")
    forbidden = ("<script", "<foreignobject", "<!doctype", "<!entity", "javascript:")
    if any(token in lowered for token in forbidden):
        fail(f"{label} contains forbidden active/external SVG content")
    for match in re.finditer(r"(?:href|src)\s*=\s*['\"]([^'\"]+)", text, re.IGNORECASE):
        if not match.group(1).startswith("#"):
            fail(f"{label} contains a non-local SVG reference")


def reconcile_immutable_outputs(outputs: Mapping[Path, bytes], mode: str) -> None:
    """Compare every target before creating any missing immutable output."""

    if mode not in {"write", "check"}:
        fail(f"unknown publication mode {mode!r}")
    missing: list[Path] = []
    for path, payload in sorted(outputs.items(), key=lambda item: str(item[0])):
        if path.exists():
            if not path.is_file() or path.is_symlink():
                fail(f"publication target is not a regular non-symlink file: {path}")
            if path.read_bytes() != payload:
                fail(f"immutable publication target differs; use a new run ID: {path}")
        else:
            missing.append(path)
    if mode == "check":
        if missing:
            fail(f"publication check found missing output: {missing[0]}")
        return
    for path in missing:
        path.parent.mkdir(parents=True, exist_ok=True)
        descriptor, temp_name = tempfile.mkstemp(prefix=f".{path.name}.", dir=path.parent)
        try:
            with os.fdopen(descriptor, "wb") as output:
                output.write(outputs[path])
                output.flush()
                os.fsync(output.fileno())
            try:
                os.link(temp_name, path)
            except FileExistsError:
                if not path.is_file() or path.is_symlink() or path.read_bytes() != outputs[path]:
                    fail(f"publication race produced a differing immutable target: {path}")
        finally:
            if os.path.exists(temp_name):
                os.unlink(temp_name)


def export(
    repo_root: Path, derived: Path, metadata_path: Path, out_envelope: Path,
    figure_dir: Path, *, zstd_command: str = "zstd", mode: str = "check",
) -> dict[str, Any]:
    repo_root,derived,metadata_path,out_envelope,figure_dir=(path.resolve() for path in (repo_root,derived,metadata_path,out_envelope,figure_dir))
    ensure_inside(repo_root,out_envelope,"envelope output")
    ensure_inside(repo_root,figure_dir,"figure directory")
    metadata=read_json(metadata_path,"publication metadata")
    if metadata.get("schema_version")!=1 or metadata.get("study_id")!=STUDY_ID:
        fail("publication metadata schema/study ID mismatch")
    run_id=require_string(metadata.get("run_id"),"metadata.run_id")
    if not RUN_ID_RE.fullmatch(run_id):
        fail("run_id must be a safe lowercase repository path component")
    expected_envelope=(repo_root/"bench/canonical-competitor-results/ac-sparql"/run_id/"paper-summary.json").resolve()
    expected_figures=(repo_root/"site/papers/figures/canonical-timing"/PAPER_SLUG/run_id).resolve()
    if out_envelope != expected_envelope:
        fail(f"envelope must use the immutable run-specific canonical path: {expected_envelope}")
    if figure_dir != expected_figures:
        fail(f"figure directory must use the immutable run-specific canonical path: {expected_figures}")
    manifest,artifacts=verify_analyzer_manifest(derived,metadata,repo_root)
    draws=require_int(manifest.get("bootstrap_draws"),"manifest.bootstrap_draws",10000)
    seed=require_int(manifest.get("bootstrap_seed"),"manifest.bootstrap_seed",0)
    configs=all_configs()
    summary_rows=read_csv(artifacts["summary.csv"].path,"summary.csv",SUMMARY_FIELDS)
    overhead_rows=read_csv(artifacts["paired-overhead.csv"].path,"paired-overhead.csv",OVERHEAD_FIELDS)
    construction_rows=read_csv(artifacts["construction.csv"].path,"construction.csv",CONSTRUCTION_FIELDS)
    summary=validate_summary(summary_rows,configs)
    overhead=validate_overhead(overhead_rows,configs,draws,seed)
    construction=validate_construction(construction_rows,configs)
    h2_document=read_json(artifacts["h2.json"].path,"h2.json")
    h2=validate_h2(h2_document,summary,manifest,draws,seed)
    environment,host_duration=validate_environment(metadata,metadata_path.parent)
    source=require_object(metadata.get("source"),"metadata.source")
    source_commit=require_string(source.get("git_commit"),"metadata.source.git_commit")
    cost,correctness,exclusions,supporting_artifacts=validate_cost_and_support(metadata,metadata_path.parent,source_commit,host_duration)
    artifacts["reports/cost-report.json"] = Artifact(
        "reports/cost-report.json", Path("reports/cost-report.json"),
        str(cost["report"]["sha256"]), int(cost["report"]["bytes"]),
    )
    artifacts["reports/correctness-report.json"] = Artifact(
        "reports/correctness-report.json", Path("reports/correctness-report.json"),
        str(correctness["report"]["sha256"]), int(correctness["report"]["bytes"]),
    )
    artifacts["reports/exclusion-report.json"] = Artifact(
        "reports/exclusion-report.json", Path("reports/exclusion-report.json"),
        str(exclusions["report"]["sha256"]), int(exclusions["report"]["bytes"]),
    )
    for name, artifact in supporting_artifacts.items():
        if name in artifacts:
            fail(f"supporting evidence artifact collides with {name}")
        artifacts[name] = artifact
    raw_provenance=validate_raw_provenance(
        metadata, metadata_path.parent, repo_root,
        out_envelope.parent/"raw-sanitized.tar.zst", manifest, artifacts,
        zstd_command,
    )
    artifacts["derived/manifest.json"] = raw_provenance["archived_metadata"]["derived/manifest.json"]
    for name in ("started-at.txt", "finished-at.txt"):
        artifacts[name] = raw_provenance["archived_metadata"][name]
    analysis_meta=require_object(metadata.get("analysis"),"metadata.analysis")
    analysis_commit=require_string(analysis_meta.get("git_commit"),"metadata.analysis.git_commit")
    if not HEX40.fullmatch(analysis_commit) or analysis_meta.get("tree_clean") is not True:
        fail("analysis commit must be a clean lowercase 40-hex object")
    if analysis_commit == source_commit:
        fail("benchmark source and post-run analysis commits must remain distinct")
    protocol_rel=require_string(analysis_meta.get("protocol_path"),"metadata.analysis.protocol_path")
    protocol_path=(repo_root/protocol_rel).resolve(); ensure_inside(repo_root,protocol_path,"protocol path")
    analyzer_rel=require_string(analysis_meta.get("analyzer_path"),"metadata.analysis.analyzer_path")
    analyzer_path=(repo_root/analyzer_rel).resolve()
    publisher_rel=require_string(analysis_meta.get("publisher_path"),"metadata.analysis.publisher_path")
    publisher_path=(repo_root/publisher_rel).resolve(); ensure_inside(repo_root,publisher_path,"publisher path")
    if publisher_path != Path(__file__).resolve():
        fail("metadata publisher_path does not identify the executing exporter")
    publisher_hash=sha256_file(publisher_path)
    invocation = require_list(analysis_meta.get("invocation"), "metadata.analysis.invocation")
    if not invocation or any(not isinstance(item, str) or not item for item in invocation):
        fail("analysis invocation must be a non-empty array of command arguments")
    required_invocation_tokens = {
        "--require-canonical", "--expected-commit", source_commit,
        "--bootstrap-draws", str(draws), "--bootstrap-seed", str(seed),
    }
    if not required_invocation_tokens.issubset(set(invocation)):
        fail("analysis invocation omits canonical commit or bootstrap controls")
    figure_payloads={
        "pod_scaling_latency":artifacts["pod-scaling-latency.svg"].path.read_bytes(),
        "http_backend_operations":artifacts["http-backend-operations.svg"].path.read_bytes(),
        "guarded_stack_content_reference":render_guarded_stack_content_reference(overhead,artifacts,publisher_hash),
        "factor_sensitivities":render_factor_sensitivities(summary,artifacts,publisher_hash),
        "construction_resident":render_construction_resident(construction,artifacts,publisher_hash),
    }
    for name, payload in figure_payloads.items():
        validate_svg_payload(payload, f"figure {name}")
    figure_names={
        "pod_scaling_latency":"pod-scaling-latency.svg",
        "http_backend_operations":"http-backend-operations.svg",
        "guarded_stack_content_reference":"guarded-stack-content-reference.svg",
        "factor_sensitivities":"factor-sensitivities.svg",
        "construction_resident":"construction-resident.svg",
    }
    figures={name:figure_descriptor(repo_root,figure_dir/figure_names[name],payload) for name,payload in figure_payloads.items()}
    results=build_results(h2_document,h2,summary,overhead,construction,manifest,correctness,exclusions,cost,host_duration,artifacts)
    query_families={
        query: next(row["query_family"] for key,row in sorted(summary.items()) if key[4]==query)
        for query in QUERIES
    }
    metadata_hash=sha256_file(metadata_path)
    protocol_hash=sha256_file(protocol_path); analyzer_hash=sha256_file(analyzer_path)
    if protocol_version(protocol_path) != "1.33":
        fail("publication requires protocol amendment 1.33")
    input_files=[]
    for item_value in require_list(manifest.get("input_files"),"manifest.input_files"):
        item=require_object(item_value,"input file")
        name=Path(str(item["path"])).name
        archive_member=raw_provenance["input_archive_members"].get(name)
        if archive_member is None:
            fail(f"raw input has no verified archive member: {name}")
        input_files.append({"path":f"raw/{name}","sha256":str(item["sha256"]),"archive_member":archive_member})
    archive_prefix=safe_posix_member(
        str(raw_provenance["verification"]["manifest_member"]),
        "verified archive manifest member",
    ).parent
    supporting_inputs={
        **raw_provenance["archived_metadata"],
        **{name: artifact for name, artifact in supporting_artifacts.items()},
        "reports/cost-report.json": artifacts["reports/cost-report.json"],
        "reports/correctness-report.json": artifacts["reports/correctness-report.json"],
        "reports/exclusion-report.json": artifacts["reports/exclusion-report.json"],
    }
    for name, artifact in sorted(supporting_inputs.items()):
        input_files.append({
            "path":name,
            "sha256":artifact.sha256,
            "archive_member":str(archive_prefix/safe_posix_member(name,f"supporting input {name!r}")),
        })
    input_files.sort(key=lambda item: item["path"])
    output_files=[{"name":artifact.name,"sha256":artifact.sha256,"bytes":artifact.bytes} for name,artifact in sorted(artifacts.items()) if name in REQUIRED_ANALYZER_OUTPUTS]
    output_files.extend({"name":figure_names[name],"sha256":descriptor["sha256"],"bytes":len(figure_payloads[name])} for name,descriptor in sorted(figures.items()) if name not in {"pod_scaling_latency","http_backend_operations"})
    validate_result_binding_coverage(results, input_files, output_files)
    release=require_object(metadata.get("release"),"metadata.release")
    baseline=require_string(release.get("baseline_commit"),"metadata.release.baseline_commit")
    if not HEX40.fullmatch(baseline): fail("release baseline commit must be 40-hex")
    collected=require_string(metadata.get("collected_at_utc"),"metadata.collected_at_utc")
    parse_utc(collected,"metadata.collected_at_utc")
    if collected != environment["finished_at_utc"]:
        fail("collected_at_utc must equal the completed run timestamp")
    envelope={
        "schema_version":SCHEMA_VERSION,"canonical":True,"study_id":STUDY_ID,
        "run_id":run_id,"collected_at_utc":collected,
        "source":{"git_commit":source_commit,"tree_clean":True},
        "environment":environment,
        "workload":{
            "label":"WAC-scoped SPARQL Pod-scaling, sensitivity, and scenario campaigns",
            "dataset":"Deterministic count-calibrated social and compact-health corpora; not a sample of deployed Pods",
            "query_scope":"All eight predeclared query families in the primary intervention and q1/q8 in every sensitivity cell",
            "comparison_boundary":{
                "estimand":"guarded-stack/content-reference contrast",
                "reference":"query-answer-equivalent content-only physical reference",
                "limitation":"The guarded datasets and request paths may include structural/control graphs, authentication, assembly, and API work; this is neither pure access-control overhead nor an otherwise-identical same-data counterfactual.",
            },
            "rq3_scope":"Observable work proxies plus implementation-audited theoretical decomposition; no empirical stage-dominance or time-attribution claim.",
            "protocol":{"path":protocol_rel,"version":protocol_version(protocol_path),"sha256":protocol_hash},
            "pod_counts":list(POD_COUNTS),"domains":list(DOMAINS),"lanes":list(LANES),"query_ids":list(QUERIES),
            "query_families":query_families,
            "h2_decision_rule":{
                "endpoint_ratio_upper_bound":1.10,
                "pod_elasticity_upper_bound":0.10,
                "joint":"A cell meets H2 only when both wall and process-CPU upper bounds satisfy both margins.",
            },
            "process_blocks":5,"timing_warmups":10,"timing_repetitions":30,"instrumentation_warmups":1,"instrumentation_repetitions":1,
        },
        "analysis":{
            "git_commit":analysis_commit,"tree_clean":True,"canonical_validation_passed":True,
            "script":{"path":analyzer_rel,"sha256":analyzer_hash},
            "publisher":{"path":publisher_rel,"sha256":publisher_hash},
            "publication_metadata":{"sha256":metadata_hash,"bytes":metadata_path.stat().st_size},
            "analysis_manifest":{"sha256":artifacts["manifest.json"].sha256,"bytes":artifacts["manifest.json"].bytes},
            "invocation":invocation,
            "bootstrap_draws":draws,"bootstrap_seed":seed,"input_files":input_files,"output_files":output_files,
            "raw_archive":raw_provenance["descriptor"],
            "raw_archive_verification":raw_provenance["verification"],
            "primary_cross_p_invariant":raw_provenance["cross_p"],
            "actual_cost":cost,"exclusions":exclusions,
            "release":{"baseline_commit":baseline,"doi":require_string(release.get("doi"),"metadata.release.doi"),"license":require_string(release.get("license"),"metadata.release.license")},
            "result_bindings":dict(sorted(results.bindings.items())),
            "machine_interpretation":{
                "h2_materialized":("all" if all(row["minimal_overhead"] for key,row in h2.items() if key[0]=="materialized-routed") else "none" if not any(row["minimal_overhead"] for key,row in h2.items() if key[0]=="materialized-routed") else "mixed"),
                "h2_http":("all" if all(row["minimal_overhead"] for key,row in h2.items() if key[0]=="native-http-assembly") else "none" if not any(row["minimal_overhead"] for key,row in h2.items() if key[0]=="native-http-assembly") else "mixed"),
                "h3_h4_h5":"exploratory-descriptive-hand-review-no-binary-verdict",
            },
        },
        "correctness":correctness,
        "results":results.results,
        "figures":figures,
        "paper_evidence":dict(sorted(results.evidence.items())),
        "paper_figures":{
            "ac_sparql.figure.pod_scaling_latency":{"figure":"pod_scaling_latency","alt":"Median warm guarded-query latency versus resident Pod count for q1 and q8, separated by lane and domain.","note":"Analyzer-generated primary scaling figure; descriptive medians are bound by its SHA-256.","papers":[PAPER_SLUG]},
            "ac_sparql.figure.http_backend_operations":{"figure":"http_backend_operations","alt":"Native HTTP backend operations per q1 request versus resident Pod count, separated by domain.","note":"Instrumentation count figure; instrumentation timings are not latency evidence.","papers":[PAPER_SLUG]},
            "ac_sparql.figure.guarded_stack_content_reference":{"figure":"guarded_stack_content_reference","alt":"Sixteen panels show wall and CPU guarded-stack/content-reference paired ratios and differences for every primary query, domain, lane, and Pod count with block-bootstrap intervals.","note":"Complete guarded-stack/content-reference contrast against a query-answer-equivalent content-only physical reference; not pure authorization overhead or an otherwise-identical same-data counterfactual. All cells are shown without pooling or favourable selection.","papers":[PAPER_SLUG]},
            "ac_sparql.figure.factor_sensitivities":{"figure":"factor_sensitivities","alt":"Twenty panels show descriptive timing medians and instrumentation counts for every documents, triples, ACL placement/depth, and visibility sensitivity level.","note":"Complete q1/q8 sensitivity matrix; panel-local log scales and no fitted trends.","papers":[PAPER_SLUG]},
            "ac_sparql.figure.construction_resident":{"figure":"construction_resident","alt":"Construction phases and current and peak whole-process resident memory plotted against Pod count and realized content triples for both lanes and domains.","note":"RSS is a conservative whole-process indicator, not isolated server heap.","papers":[PAPER_SLUG]},
        },
    }
    envelope_payload=(json.dumps(
        envelope, sort_keys=True, ensure_ascii=True, allow_nan=False,
        separators=(",", ":"),
    )+"\n").encode("utf-8")
    outputs={figure_dir/figure_names[name]:payload for name,payload in figure_payloads.items()}
    outputs[out_envelope]=envelope_payload
    reconcile_immutable_outputs(outputs,mode)
    return envelope


def arguments(argv: Sequence[str] | None = None) -> argparse.Namespace:
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repo-root",type=Path,required=True)
    parser.add_argument("--derived",type=Path,required=True)
    parser.add_argument("--metadata",type=Path,required=True)
    parser.add_argument("--out-envelope",type=Path,required=True)
    parser.add_argument("--figure-dir",type=Path,required=True)
    parser.add_argument("--zstd",default="zstd",help="zstd executable used to stream-verify raw-sanitized.tar.zst")
    modes=parser.add_mutually_exclusive_group(required=True)
    modes.add_argument("--write",dest="mode",action="store_const",const="write")
    modes.add_argument("--check",dest="mode",action="store_const",const="check")
    return parser.parse_args(argv)


def main(argv: Sequence[str] | None = None) -> int:
    args=arguments(argv)
    envelope=export(
        args.repo_root,args.derived,args.metadata,args.out_envelope,args.figure_dir,
        zstd_command=args.zstd,mode=args.mode,
    )
    verb="verified" if args.mode=="check" else "published"
    print(
        f"{verb} {len(envelope['paper_evidence'])} paper scalar declarations, "
        f"{len(envelope['analysis']['result_bindings'])} full result bindings, and "
        f"{len(envelope['figures'])} digest-bound figures"
    )
    return 0


if __name__=="__main__":
    try:
        raise SystemExit(main())
    except PublicationError as error:
        print(f"publication refused: {error}",file=sys.stderr)
        raise SystemExit(2) from error
