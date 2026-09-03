#!/usr/bin/env python3
"""Synthetic-only tests for the AC-SPARQL publication exporter."""

from __future__ import annotations

import csv
import hashlib
import importlib.util
import io
import json
import shutil
import subprocess
import sys
import tarfile
import tempfile
import unittest
from pathlib import Path
from typing import Any


SOURCE = Path(__file__).with_name("export_publication.py")
ARCHIVE_SOURCE = Path(__file__).with_name("create_raw_archive.py")


def digest(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def write_json(path: Path, value: Any) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def write_csv(path: Path, fields: set[str], rows: list[dict[str, Any]]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    ordered = sorted(fields)
    with path.open("w", newline="", encoding="utf-8") as output:
        writer = csv.DictWriter(output, fieldnames=ordered)
        writer.writeheader()
        writer.writerows(rows)


def load_module(path: Path):
    module_name = f"fixture_{path.stem}"
    spec = importlib.util.spec_from_file_location(module_name, path)
    assert spec and spec.loader
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


def write_deterministic_tar_zst(
    archive: Path, members: list[tuple[str, Path]], zstd: str, *, mtime: int = 0
) -> None:
    archive.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.NamedTemporaryFile(suffix=".tar", delete=False) as temporary:
        tar_path = Path(temporary.name)
    try:
        with tarfile.open(tar_path, "w", format=tarfile.USTAR_FORMAT) as bundle:
            for name, path in sorted(members):
                payload = path.read_bytes()
                info = tarfile.TarInfo(name)
                info.size = len(payload)
                info.mode = 0o644
                info.uid = info.gid = 0
                info.uname = info.gname = ""
                info.mtime = mtime
                bundle.addfile(info, io.BytesIO(payload))
        subprocess.run(
            [zstd, "-q", "-f", "-T1", "-1", str(tar_path), "-o", str(archive)],
            check=True,
            capture_output=True,
        )
    finally:
        tar_path.unlink(missing_ok=True)


class Fixture:
    def __init__(self, base: Path) -> None:
        self.root = base / "repo"
        self.run = base / "run"
        self.derived = self.run / "derived"
        self.metadata = base / "publication-metadata.json"
        self.publisher = self.root / "bench/ac/scaling/export_publication.py"
        self.analyzer = self.root / "bench/ac/scaling/analyze.py"
        self.protocol = self.root / "bench/ac/scaling/ANALYSIS-PROTOCOL.md"
        self.envelope = self.root / "bench/canonical-competitor-results/ac-sparql/run-fixture/paper-summary.json"
        self.figures = self.root / "site/papers/figures/canonical-timing/access-controlled-sparql-pod-scale/run-fixture"
        self.archive = self.envelope.parent / "raw-sanitized.tar.zst"
        self.zstd = shutil.which("zstd") or ""
        if not self.zstd:
            raise unittest.SkipTest("zstd is required for raw-sanitized.tar.zst tests")
        self.source_commit = "a" * 40
        self.analysis_commit = "b" * 40
        self.publisher.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(SOURCE, self.publisher)
        self.analyzer.write_text("# fixture blinded analyzer\n", encoding="utf-8")
        self.protocol.write_text("**Protocol version:** 1.31\n\n# Fixture protocol\n", encoding="utf-8")
        self.mod = load_module(self.publisher)
        self.configs = self.mod.all_configs()
        self.input_files = [
            {"path": f"/var/tmp/sparq-ac-study/raw/cell-{index:04d}.jsonl", "sha256": ""}
            for index in range(1280)
        ]
        self.summary_rows = self.make_summary()
        self.construction_rows = self.make_construction()
        self.h2 = self.make_h2()
        self.overhead_rows = self.make_overhead()
        self.write_bundle()

    def config_fields(self, config) -> dict[str, Any]:
        return {
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

    def latency(self, config, query: str, operation: str) -> tuple[float, float]:
        q = self.mod.QUERIES.index(query) + 1
        base = 0.25 + q * 0.12
        log_position = self.mod.POD_COUNTS.index(config.pods) / 4 if config.campaign.startswith("pod-scaling") else 0.25
        if operation == self.mod.GUARDED[config.lane]:
            wall_multiplier = 1.0 + (0.02 if config.lane == "materialized-routed" else 1.0) * log_position
            cpu_multiplier = 1.0 + (0.01 if config.lane == "materialized-routed" else 0.8) * log_position
        else:
            wall_multiplier = 0.82
            cpu_multiplier = 0.80
        shape = 1 + config.documents / 5000 + config.triples / 4000
        return base * wall_multiplier * shape, base * cpu_multiplier * shape

    def make_summary(self) -> list[dict[str, Any]]:
        rows = []
        for config in self.configs:
            for query in config.query_ids:
                for operation in self.mod.OPERATIONS[config.lane]:
                    wall, cpu = self.latency(config, query, operation)
                    row = {
                        **self.config_fields(config),
                        "query_id": query,
                        "query_family": f"fixture family {query}",
                        "operation": operation,
                        "requests": config.requests_per_summary_row,
                        "run_uuids": 5,
                        "process_blocks": 5,
                        "latency_median_ms": wall,
                        "latency_p95_ms": wall * 1.1,
                        "latency_p99_ms": wall * 1.15,
                        "latency_mad_ms": wall * 0.03,
                        "process_cpu_median_ms": cpu,
                        "process_cpu_p95_ms": cpu * 1.1,
                        "process_cpu_p99_ms": cpu * 1.15,
                        "process_cpu_mad_ms": cpu * 0.03,
                        "allocation_operations_median": "",
                        "allocated_bytes_median": "",
                        "backend_operations_median": "",
                        "result_rows": 1,
                    }
                    if config.profile == "instrumentation":
                        row["allocation_operations_median"] = 100 + config.documents
                        row["allocated_bytes_median"] = 4096 + config.documents * config.triples
                        if config.lane == "native-http-assembly" and operation == self.mod.GUARDED[config.lane]:
                            row["backend_operations_median"] = config.pods * config.documents * 3 + 7
                    rows.append(row)
        return rows

    def make_construction(self) -> list[dict[str, Any]]:
        rows = []
        for config in self.configs:
            content_documents = config.pods * config.documents
            content_triples = content_documents * config.triples
            values: dict[str, Any] = {
                "content_documents": content_documents,
                "content_triples": content_triples,
                "container_graphs": config.pods * (config.depth + 1),
                "control_documents": config.pods * (config.depth + 2),
                "control_triples": config.pods * (config.depth + 2) * 5,
                "total_source_graphs": content_documents + config.pods * (2 * config.depth + 3),
                "target_readable_documents": config.documents,
                "evaluation_readable_documents": config.documents,
                "corpus_generation_ns": max(1, content_triples * 100),
                "graph_load_ns": "",
                "wac_materialization_ns": "",
                "route_index_ns": "",
                "lws_seed_ns": "",
                "auth_triples_total": config.pods * 20,
                "http_store_max_total_bytes": "",
                "http_store_max_resource_count": "",
                "resident_bytes": 50_000_000 + content_triples * 40,
                "peak_resident_bytes": 60_000_000 + content_triples * 50,
                "construction_allocations": "",
                "construction_allocated_bytes": "",
            }
            if config.lane == "materialized-routed":
                values.update({
                    "graph_load_ns": max(1, content_triples * 20),
                    "wac_materialization_ns": max(1, config.pods * config.coverage + 1),
                    "route_index_ns": max(1, config.pods * 100),
                })
            else:
                values.update({
                    "lws_seed_ns": max(1, content_triples * 30),
                    "http_store_max_total_bytes": content_triples * 100 + 1024,
                    "http_store_max_resource_count": values["total_source_graphs"] + 1,
                })
            if config.profile == "instrumentation":
                values["construction_allocations"] = content_documents * 10 + 1
                values["construction_allocated_bytes"] = content_triples * 64 + 1
            row = {**self.config_fields(config), "process_blocks": 5}
            row.update({f"{name}_median": value for name, value in values.items()})
            rows.append(row)
        return rows

    def primary_summary(self, lane: str, domain: str, query: str, pods: int) -> dict[str, Any]:
        return next(
            row for row in self.summary_rows
            if row["campaign"] == "pod-scaling" and row["lane"] == lane
            and row["domain"] == domain and row["query_id"] == query
            and row["pods"] == pods and row["operation"] == self.mod.GUARDED[lane]
        )

    def make_h2(self) -> dict[str, Any]:
        rows = []
        for lane in self.mod.LANES:
            for domain in self.mod.DOMAINS:
                for query in self.mod.QUERIES:
                    low, high = self.primary_summary(lane, domain, query, 1), self.primary_summary(lane, domain, query, 2048)
                    wall_ratio = high["latency_median_ms"] / low["latency_median_ms"]
                    cpu_ratio = high["process_cpu_median_ms"] / low["process_cpu_median_ms"]
                    if lane == "materialized-routed":
                        wall_ci, cpu_ci = [wall_ratio - 0.01, wall_ratio + 0.01], [cpu_ratio - 0.01, cpu_ratio + 0.01]
                        wall_beta, cpu_beta = 0.002, 0.001
                        wall_beta_ci, cpu_beta_ci = [-0.002, 0.006], [-0.002, 0.004]
                    else:
                        wall_ci, cpu_ci = [wall_ratio - 0.1, wall_ratio + 0.1], [cpu_ratio - 0.1, cpu_ratio + 0.1]
                        wall_beta, cpu_beta = 0.09, 0.08
                        wall_beta_ci, cpu_beta_ci = [0.07, 0.11], [0.06, 0.10]
                    latency_ok = wall_ci[1] <= 1.10 and wall_beta_ci[1] <= 0.10
                    cpu_ok = cpu_ci[1] <= 1.10 and cpu_beta_ci[1] <= 0.10
                    rows.append({
                        "lane": lane, "domain": domain, "query_id": query,
                        "query_family": f"fixture family {query}", "pods": list(self.mod.POD_COUNTS),
                        "p_min": 1, "p_max": 2048, "complete_process_blocks": 5,
                        "corpus_seeds": list(self.mod.BLOCK_SEEDS), "requests": 750,
                        "median_latency_ratio": wall_ratio, "ratio_ci95": wall_ci,
                        "pod_elasticity": wall_beta, "elasticity_ci95": wall_beta_ci,
                        "median_process_cpu_ratio": cpu_ratio, "process_cpu_ratio_ci95": cpu_ci,
                        "process_cpu_pod_elasticity": cpu_beta,
                        "process_cpu_elasticity_ci95": cpu_beta_ci,
                        "ratio_margin": 1.10, "elasticity_margin": 0.10,
                        "minimal_latency_scaling": latency_ok, "minimal_cpu_scaling": cpu_ok,
                        "minimal_overhead": latency_ok and cpu_ok,
                        "bootstrap_draws": 10000, "bootstrap_seed": 20260903,
                    })
        return {
            "schema_version": 6,
            "analysis": "prospective H2 hierarchical cluster bootstrap",
            "inputs": [],
            "results": rows,
        }

    def make_overhead(self) -> list[dict[str, Any]]:
        rows = []
        for config in self.configs:
            if config.profile != "timing":
                continue
            for query in config.query_ids:
                guarded = self.latency(config, query, self.mod.GUARDED[config.lane])
                plain = self.latency(config, query, self.mod.OPERATIONS[config.lane][1])
                wall_ratio, cpu_ratio = guarded[0] / plain[0], guarded[1] / plain[1]
                wall_diff, cpu_diff = guarded[0] - plain[0], guarded[1] - plain[1]
                rows.append({
                    **self.config_fields(config), "query_id": query,
                    "query_family": f"fixture family {query}", "pairs": 150,
                    "process_blocks": 5, "pairs_per_process_block": 30,
                    "bootstrap_draws": 10000, "bootstrap_seed": 20260903,
                    "bootstrap_method": self.mod.H4_METHOD,
                    "latency_ratio_median": wall_ratio, "latency_ratio_p95": wall_ratio * 1.04,
                    "latency_ratio_ci95_low": wall_ratio * 0.98,
                    "latency_ratio_ci95_high": wall_ratio * 1.02,
                    "latency_difference_median_ms": wall_diff,
                    "latency_difference_p95_ms": wall_diff + 0.02,
                    "latency_difference_ci95_low_ms": wall_diff - 0.01,
                    "latency_difference_ci95_high_ms": wall_diff + 0.01,
                    "cpu_ratio_median": cpu_ratio,
                    "cpu_ratio_ci95_low": cpu_ratio * 0.98,
                    "cpu_ratio_ci95_high": cpu_ratio * 1.02,
                    "cpu_difference_median_ms": cpu_diff,
                    "cpu_difference_ci95_low_ms": cpu_diff - 0.01,
                    "cpu_difference_ci95_high_ms": cpu_diff + 0.01,
                })
        return rows

    def write_bundle(self) -> None:
        self.derived.mkdir(parents=True, exist_ok=True)
        raw = self.run / "raw"
        raw.mkdir(parents=True, exist_ok=True)
        fixture_configs = [
            (config, block)
            for config in self.configs
            for block in range(len(self.mod.BLOCK_SEEDS))
        ]
        self.assert_fixture_count = len(fixture_configs)
        if len(fixture_configs) != len(self.input_files):
            raise AssertionError("fixture configuration count drift")
        for index, (descriptor, (config, block)) in enumerate(zip(self.input_files, fixture_configs)):
            path = raw / f"cell-{index:04d}.jsonl"
            run_uuid = f"fixture-{index:04d}"
            base = {
                "campaign": config.campaign,
                "measurement_profile": config.profile,
                "lane": config.lane,
                "domain": config.domain,
                "cell_label": config.cell_label,
                "pods": config.pods,
                "process_block": block,
                "run_uuid": run_uuid,
            }
            records: list[dict[str, Any]] = [{
                **base,
                "record_type": "construction",
                "target_readable_documents": config.documents,
            }]
            for query in config.query_ids:
                query_hash = hashlib.sha256(f"query:{config.domain}:{query}".encode()).hexdigest()
                if config.campaign.removesuffix("-instrumentation") == "pod-scaling":
                    result_key = f"primary:{config.profile}:{config.lane}:{config.domain}:{query}:{block}"
                else:
                    result_key = f"secondary:{config.campaign}:{config.lane}:{config.domain}:{config.cell_label}:{query}:{block}"
                records.append({
                    **base,
                    "record_type": "observation",
                    "query_id": query,
                    "query_hash_sha256": query_hash,
                    "result_hash_sha256": hashlib.sha256(result_key.encode()).hexdigest(),
                    "result_rows": 1,
                })
            path.write_text(
                "".join(json.dumps(record, sort_keys=True) + "\n" for record in records),
                encoding="utf-8",
            )
            descriptor["sha256"] = digest(path)
        self.h2["inputs"] = [item["path"] for item in self.input_files]
        write_csv(self.derived / "summary.csv", self.mod.SUMMARY_FIELDS, self.summary_rows)
        write_csv(self.derived / "paired-overhead.csv", self.mod.OVERHEAD_FIELDS, self.overhead_rows)
        write_csv(self.derived / "construction.csv", self.mod.CONSTRUCTION_FIELDS, self.construction_rows)
        write_json(self.derived / "h2.json", self.h2)
        (self.derived / "pod-scaling-latency.svg").write_text('<svg xmlns="http://www.w3.org/2000/svg"><text>fixture scaling</text></svg>\n', encoding="utf-8")
        (self.derived / "http-backend-operations.svg").write_text('<svg xmlns="http://www.w3.org/2000/svg"><text>fixture backend</text></svg>\n', encoding="utf-8")
        names = sorted(self.mod.REQUIRED_ANALYZER_OUTPUTS)
        manifest = {
            "schema_version": 6, "input_files": self.input_files,
            "analysis_script": {"path": "/var/tmp/source/bench/ac/scaling/analyze.py", "sha256": digest(self.analyzer)},
            "canonical_required": True, "expected_commit": self.source_commit,
            "observations": 114080, "construction_records": 1280,
            "correctness_records": 1280, "applicability_records": 3680,
            "bootstrap_draws": 10000, "bootstrap_seed": 20260903,
            "outputs": names,
            "output_artifacts": [
                {"path": name, "bytes": (self.derived / name).stat().st_size, "sha256": digest(self.derived / name)}
                for name in names
            ],
        }
        write_json(self.derived / "manifest.json", manifest)
        (self.run / "DONE").write_bytes(b"")
        (self.run / "started-at.txt").write_text("2026-09-03T10:00:00Z\n", encoding="utf-8")
        (self.run / "finished-at.txt").write_text("2026-09-03T11:00:00Z\n", encoding="utf-8")
        environment = "\n".join((
            "mode=canonical", f"source_commit={self.source_commit}", "source_status=",
            "instance_id=i-fixture", "instance_type=r7g.xlarge", "region=eu-west-2",
            "cpuset=1", "rustc=rustc fixture", "cargo=cargo fixture", "python=Python fixture",
            "kernel=Linux fixture 6.8.0 aarch64", "cgroup=0::/fixture",
            "memory_max_bytes=23000000000", "meminfo=MemTotal: 32000000 kB;",
            "lscpu=Architecture: aarch64;Model name: Fixture CPU;", "governor=performance",
        )) + "\n"
        (self.run / "environment.txt").write_text(environment, encoding="utf-8")
        cost = {
            "schema_version": 1, "currency": "USD", "status": "final-accounted",
            "accounting_method": "duration-times-official-rate-plus-ebs-ipv4",
            "estimated": True,
            "canonical_host_duration_seconds": 3600,
            "components_usd": {
                "canonical_compute": "2.00", "canonical_storage": "0.40",
                "canonical_public_ipv4": "0.10",
                "prior_valid_and_invalid_attempts": "0.20", "other": "0.00",
            },
            "study_total_usd": "2.70", "canonical_run_usd": "2.50",
            "basis": "Synthetic duration-times-rate fixture; no real charge.",
        }
        correctness_root = self.run / "correctness"
        correctness_root.mkdir(parents=True, exist_ok=True)
        gate_index = 0
        for lane in self.mod.LANES:
            for domain in self.mod.DOMAINS:
                for principal in self.mod.CORRECTNESS_PRINCIPALS:
                    for coverage in (100, 1000):
                        for seed in (17, 42, 101):
                            for pods in (1, 8, 32):
                                if lane == "materialized-routed":
                                    gate = "all four principal classes against physically filtered reference"
                                    principals_checked, queries_checked = 4, 32
                                else:
                                    gate = "primary principal through real HTTP route against physically filtered reference"
                                    principals_checked, queries_checked = 1, 8
                                cli_principal = self.mod.CORRECTNESS_PRINCIPAL_CLI_LABEL[principal]
                                run_id = (
                                    f"correctness-{self.mod.LANE_SHORT[lane]}-{domain}-"
                                    f"{cli_principal}-p{pods}-a{coverage}-s{seed}"
                                )
                                common = {
                                    "schema_version": self.mod.ANALYZER_SCHEMA_VERSION,
                                    "run_id": run_id,
                                    "run_uuid": f"correctness-fixture-{gate_index:03d}",
                                    "source_commit": self.source_commit,
                                    "source_dirty": False,
                                    "host": "fixture-host",
                                    "instance_id": "i-fixture",
                                    "instance_type": "r7g.xlarge",
                                    "cloud_region": "eu-west-2",
                                    "os": "linux",
                                    "architecture": "aarch64",
                                    "rustc": "rustc fixture",
                                    "profile": "release",
                                    "features": "sparq-lws-core/default; sparq-solid/default",
                                    "measurement_profile": "timing",
                                    "campaign": "correctness",
                                    "cell_label": f"{cli_principal}-{coverage}",
                                    "lane": lane,
                                    "domain": domain,
                                    "topology": self.mod.TOPOLOGY[lane],
                                    "principal": principal,
                                    "own_acl_coverage_per_mille": coverage,
                                    "corpus_seed": seed,
                                    "pods": pods,
                                    "documents_per_pod": 8,
                                    "triples_per_document": 8,
                                    "container_depth": 3,
                                    "public_per_mille": 300,
                                    "private_per_mille": 400,
                                    "shared_per_mille": 300,
                                    "corpus_hash_sha256": hashlib.sha256(
                                        f"corpus:{lane}:{domain}:{principal}:{coverage}:{seed}:{pods}".encode()
                                    ).hexdigest(),
                                    "process_block": 0,
                                    "configuration_order": None,
                                    "configuration_order_seed": None,
                                    "warmups_configured": 0,
                                    "repetitions_configured": 1,
                                    "concurrency": 1,
                                    "rayon_threads": "1",
                                    "cpu_affinity": "1",
                                }
                                query_hashes = {
                                    query: hashlib.sha256(
                                        f"query:{domain}:{query}".encode()
                                    ).hexdigest()
                                    for query in self.mod.QUERIES
                                }
                                records = []
                                for query in self.mod.QUERIES:
                                    family, minimum = self.mod.CORRECTNESS_QUERY_METADATA[query]
                                    records.append({
                                        **common,
                                        "record_type": "applicability",
                                        "utc_unix_ns": gate_index * 100 + len(records) + 1,
                                        "query_id": query,
                                        "query_family": family,
                                        "query_hash_sha256": query_hashes[query],
                                        "minimum_triples_per_document": minimum,
                                        "applicable": True,
                                        "selected": True,
                                        "reason": None,
                                    })
                                records.append({
                                    **common,
                                    "record_type": "correctness-gate",
                                    "utc_unix_ns": gate_index * 100 + len(records) + 1,
                                    "gate": gate,
                                    "principals_checked": principals_checked,
                                    "queries_checked": queries_checked,
                                    "exact_result_bags": True,
                                })
                                content_documents = pods * 8
                                container_graphs = pods * 4
                                control_documents = pods * 5
                                construction = {
                                    **common,
                                    "record_type": "construction",
                                    "utc_unix_ns": gate_index * 100 + len(records) + 1,
                                    "content_documents": content_documents,
                                    "content_triples": content_documents * 8,
                                    "container_graphs": container_graphs,
                                    "control_documents": control_documents,
                                    "control_triples": control_documents * 5,
                                    "total_source_graphs": (
                                        content_documents + container_graphs + control_documents
                                    ),
                                    "target_readable_documents": 8,
                                    "evaluation_readable_documents": 8,
                                    "corpus_generation_ns": 1,
                                    "graph_load_ns": 1 if lane == "materialized-routed" else None,
                                    "wac_materialization_ns": (
                                        1 if lane == "materialized-routed" else None
                                    ),
                                    "route_index_ns": 1 if lane == "materialized-routed" else None,
                                    "lws_seed_ns": 1 if lane == "native-http-assembly" else None,
                                    "auth_triples_total": (
                                        1 if lane == "materialized-routed" else None
                                    ),
                                    "http_store_max_total_bytes": (
                                        1024 if lane == "native-http-assembly" else None
                                    ),
                                    "http_store_max_resource_count": (
                                        content_documents + container_graphs + control_documents + 1
                                        if lane == "native-http-assembly" else None
                                    ),
                                    "construction_allocations": None,
                                    "construction_allocated_bytes": None,
                                    "resident_bytes": 1024,
                                    "peak_resident_bytes": 2048,
                                }
                                records.append(construction)
                                for query_index, query in enumerate(self.mod.QUERIES):
                                    family, _minimum = self.mod.CORRECTNESS_QUERY_METADATA[query]
                                    result_hash = hashlib.sha256(
                                        f"result:{lane}:{domain}:{principal}:{query}".encode()
                                    ).hexdigest()
                                    ordered_operations = list(self.mod.OPERATIONS[lane])
                                    if query_index % 2:
                                        ordered_operations.reverse()
                                    for order_in_pair, operation in enumerate(ordered_operations):
                                        records.append({
                                            **common,
                                            "record_type": "observation",
                                            "utc_unix_ns": gate_index * 100 + len(records) + 1,
                                            "query_id": query,
                                            "query_family": family,
                                            "query_hash_sha256": query_hashes[query],
                                            "operation": operation,
                                            "pair_id": f"{run_id}:0:{query}:0",
                                            "repetition": 0,
                                            "warmup": False,
                                            "order_in_pair": order_in_pair,
                                            "wall_ns": 1,
                                            "process_cpu_ns": 1,
                                            "allocation_operations": None,
                                            "allocated_bytes": None,
                                            "response_bytes": (
                                                64 if lane == "native-http-assembly" else None
                                            ),
                                            "result_rows": 1,
                                            "result_hash_sha256": result_hash,
                                            "correctness": True,
                                            "http_status": (
                                                200 if operation == "native-http-request" else None
                                            ),
                                            "backend_sparql_queries": None,
                                            "backend_sparql_updates": None,
                                            "backend_blob_gets": None,
                                            "backend_blob_puts": None,
                                            "backend_blob_other": None,
                                            "backend_total_operations": None,
                                            "backend_max_in_flight": None,
                                        })
                                if len(records) != 26:
                                    raise AssertionError("correctness fixture composition drift")
                                (correctness_root / f"gate-{gate_index:03d}.jsonl").write_text(
                                    "".join(
                                        json.dumps(record, sort_keys=True) + "\n"
                                        for record in records
                                    ),
                                    encoding="utf-8",
                                )
                                gate_index += 1
        suite_root = correctness_root / "suite-logs"
        suite_root.mkdir(parents=True, exist_ok=True)
        test_groups = (
            (
                "sparq-acbench-deployment",
                ["cargo", "test", "--locked", "-p", "sparq-acbench", "--test", "deployment"],
                (),
            ),
            (
                "sparq-solid-acbench-deployment",
                ["cargo", "test", "--locked", "-p", "sparq-solid", "--test", "acbench_deployment"],
                ("replacing_a_generated_own_acl_revokes_the_recipient_immediately",),
            ),
            (
                "sparq-lws-core-sparql-endpoint",
                ["cargo", "test", "--locked", "-p", "sparq-lws-core", "--features", "sparql-endpoint", "--test", "sparql_endpoint"],
                (
                    "replacing_an_acl_revokes_the_native_query_route_immediately",
                    "direct_no_leak_matches_ldp_get_authorization",
                    "negation_cannot_distinguish_an_unreadable_resource_from_absence",
                ),
            ),
        )
        suite_attestations = []
        for name, command, tests in test_groups:
            log = suite_root / f"{name}.log"
            log.write_text(
                f"source_commit={self.source_commit}\n"
                + "".join(f"test integration::{test} ... ok\n" for test in tests)
                + "test result: ok. synthetic fixture\n",
                encoding="utf-8",
            )
            suite_attestations.append({
                "command": command,
                "source_commit": self.source_commit,
                "exit_status": 0,
                "log": {
                    "path": log.relative_to(self.run).as_posix(),
                    "sha256": digest(log),
                    "bytes": log.stat().st_size,
                },
            })
        correctness = {
            "schema_version": 1,
            "source_commit": self.source_commit,
            "note": "Synthetic annotation; pass/count claims are derived from gate files and logs.",
            "suite_attestations": suite_attestations,
        }
        exclusion_entries = []
        for attempt, reason in (
            ("fixture-expired", "credential-expiry"),
            ("fixture-transport", "setup-transport"),
        ):
            failure_root = self.run / "failures" / attempt
            failure_root.mkdir(parents=True, exist_ok=True)
            sentinel = failure_root / "FAILED.json"
            write_json(sentinel, {
                "schema_version": 1, "status": "FAILED", "attempt_id": attempt,
                "reason_code": reason, "exit_status": 1,
            })
            stderr = failure_root / "stderr.txt"
            stderr.write_text("synthetic retained failure evidence\n", encoding="utf-8")
            exclusion_entries.append({
                "attempt_id": attempt,
                "reason_code": reason,
                "sentinel": {"path": sentinel.relative_to(self.run).as_posix(), "sha256": digest(sentinel), "bytes": sentinel.stat().st_size},
                "stderr": {"path": stderr.relative_to(self.run).as_posix(), "sha256": digest(stderr), "bytes": stderr.stat().st_size},
                "partial_inputs": [],
            })
        exclusions = {
            "schema_version": 1,
            "note": "Synthetic annotations; the attempt count is derived from discovered FAILED.json sentinels.",
            "entries": exclusion_entries,
        }
        cost_path, correctness_path, exclusions_path = (
            self.root.parent / "cost-report.json",
            self.root.parent / "correctness-report.json",
            self.root.parent / "exclusions.json",
        )
        write_json(cost_path, cost)
        write_json(correctness_path, correctness)
        write_json(exclusions_path, exclusions)
        reports = self.run / "reports"
        reports.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(cost_path, reports / "cost-report.json")
        shutil.copyfile(correctness_path, reports / "correctness-report.json")
        shutil.copyfile(exclusions_path, reports / "exclusion-report.json")
        provenance = self.run / "provenance"
        provenance.mkdir(parents=True, exist_ok=True)
        (provenance / "ec2-original-MANIFEST.sha256").write_text(
            "synthetic fixture standing in for the original EC2 manifest\n",
            encoding="utf-8",
        )
        manifest_lines = []
        for path in sorted(item for item in self.run.rglob("*") if item.is_file() and item.name != "MANIFEST.sha256"):
            rel = path.relative_to(self.run).as_posix()
            manifest_lines.append(f"{digest(path)}  ./{rel}\n")
        (self.run / "MANIFEST.sha256").write_text("".join(manifest_lines), encoding="utf-8")
        archive_members = [("run-fixture/MANIFEST.sha256", self.run / "MANIFEST.sha256")]
        archive_members.extend(
            (f"run-fixture/{path.relative_to(self.run).as_posix()}", path)
            for path in sorted(item for item in self.run.rglob("*") if item.is_file() and item.name != "MANIFEST.sha256")
        )
        write_deterministic_tar_zst(self.archive, archive_members, self.zstd)
        metadata = {
            "schema_version": 1, "study_id": self.mod.STUDY_ID,
            "run_id": "run-fixture", "collected_at_utc": "2026-09-03T11:00:00Z",
            "source": {"git_commit": self.source_commit, "tree_clean": True},
            "analysis": {
                "git_commit": self.analysis_commit, "tree_clean": True,
                "analyzer_path": "bench/ac/scaling/analyze.py",
                "protocol_path": "bench/ac/scaling/ANALYSIS-PROTOCOL.md",
                "publisher_path": "bench/ac/scaling/export_publication.py",
                "invocation": ["python3", "bench/ac/scaling/analyze.py", "raw", "--out", "derived", "--bootstrap-draws", "10000", "--bootstrap-seed", "20260903", "--require-canonical", "--expected-commit", self.source_commit],
            },
            "environment": {
                "instance_id": "i-fixture", "instance_type": "r7g.xlarge",
                "region": "eu-west-2", "architecture": "aarch64",
                "kernel": "Linux fixture 6.8.0 aarch64", "cpu_model": "Fixture CPU",
                "cpu_affinity": "1", "allocator": "mimalloc",
                "host_label": "Fixture shared EC2 Linux/aarch64 host pinned to one logical CPU",
                "noise_limitation": "Shared tenancy cannot exclude host-level interference.",
            },
            "raw_run": {
                "run_root": str(self.run),
                "manifest": {"path": str(self.run / "MANIFEST.sha256"), "sha256": digest(self.run / "MANIFEST.sha256"), "bytes": (self.run / "MANIFEST.sha256").stat().st_size},
                "manifest_archive_member": "run-fixture/MANIFEST.sha256",
                "archive": {"path": self.archive.relative_to(self.root).as_posix(), "sha256": digest(self.archive), "bytes": self.archive.stat().st_size},
            },
            "cost_report": {"path": str(cost_path), "sha256": digest(cost_path), "bytes": cost_path.stat().st_size},
            "correctness_report": {"path": str(correctness_path), "sha256": digest(correctness_path), "bytes": correctness_path.stat().st_size},
            "exclusion_report": {"path": str(exclusions_path), "sha256": digest(exclusions_path), "bytes": exclusions_path.stat().st_size},
            "release": {"baseline_commit": "c" * 40, "doi": "10.0000/fixture", "license": "Apache-2.0"},
        }
        write_json(self.metadata, metadata)

    def publish(self):
        return self.mod.export(
            self.root, self.derived, self.metadata, self.envelope, self.figures,
            zstd_command=self.zstd, mode="write",
        )


def scalar_pointers(value: Any, tokens: tuple[str, ...] = ()) -> set[str]:
    found: set[str] = set()
    if isinstance(value, dict):
        for key, child in value.items():
            found |= scalar_pointers(child, (*tokens, key))
    elif isinstance(value, (int, float, bool)):
        escaped = [token.replace("~", "~0").replace("/", "~1") for token in tokens]
        found.add("/results/" + "/".join(escaped))
    else:
        raise AssertionError(f"non-scalar result leaf: {value!r}")
    return found


class PublicationExporterTests(unittest.TestCase):
    def test_archive_creator_preserves_ec2_manifest_and_is_reproducible(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            fx = Fixture(Path(directory))
            metadata = json.loads(fx.metadata.read_text(encoding="utf-8"))
            reports = {
                "cost-report.json": Path(metadata["cost_report"]["path"]),
                "correctness-report.json": Path(metadata["correctness_report"]["path"]),
                "exclusion-report.json": Path(metadata["exclusion_report"]["path"]),
            }
            ec2_manifest_hash = digest(fx.run / "MANIFEST.sha256")
            (fx.run / "provenance/ec2-original-MANIFEST.sha256").unlink()
            fx.archive.unlink()
            creator = load_module(ARCHIVE_SOURCE)
            created = creator.create_archive(
                fx.root, fx.run, "run-fixture", reports, ec2_manifest_hash,
                fx.zstd, "write", compression_level="-1",
            )
            archive_hash = digest(fx.archive)
            self.assertEqual(created["archive"]["sha256"], archive_hash)
            self.assertEqual(
                digest(fx.run / "provenance/ec2-original-MANIFEST.sha256"),
                ec2_manifest_hash,
            )
            self.assertIn(
                "./provenance/ec2-original-MANIFEST.sha256",
                (fx.run / "MANIFEST.sha256").read_text(encoding="utf-8"),
            )
            checked = creator.create_archive(
                fx.root, fx.run, "run-fixture", reports, ec2_manifest_hash,
                fx.zstd, "check", compression_level="-1",
            )
            self.assertEqual(checked["archive"]["sha256"], archive_hash)

    def test_complete_fixture_is_deterministic_and_fully_bound(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            fx = Fixture(Path(directory))
            envelope = fx.publish()
            first = fx.envelope.read_bytes()
            second_envelope = fx.publish()
            self.assertEqual(first, fx.envelope.read_bytes())
            self.assertEqual(envelope, second_envelope)
            declared = {item["pointer"] for item in envelope["paper_evidence"].values()}
            all_results = scalar_pointers(envelope["results"])
            result_bindings = envelope["analysis"]["result_bindings"]
            self.assertEqual(list(result_bindings), sorted(result_bindings))
            self.assertEqual(all_results, set(result_bindings))
            self.assertLess(declared, all_results)
            self.assertLess(len(declared), 1000)
            available = {
                item["name"]: item["sha256"]
                for item in envelope["analysis"]["output_files"]
            }
            available.update({
                item["path"]: item["sha256"]
                for item in envelope["analysis"]["input_files"]
            })
            for bindings in result_bindings.values():
                triples = {
                    (binding["artifact"], binding["sha256"], binding["locator"])
                    for binding in bindings
                }
                self.assertEqual(len(triples), len(bindings))
                for artifact, sha256, locator in triples:
                    self.assertEqual(available[artifact], sha256)
                    self.assertIsInstance(locator, str)
                    self.assertTrue(locator)
            self.assertEqual(len(envelope["figures"]), 5)
            for descriptor in envelope["figures"].values():
                path = fx.root / descriptor["path"]
                self.assertEqual(digest(path), descriptor["sha256"])
                output = next(
                    item for item in envelope["analysis"]["output_files"]
                    if item["name"] == descriptor["output_name"]
                )
                self.assertEqual(output["sha256"], descriptor["sha256"])
                self.assertEqual(output["bytes"], path.stat().st_size)
            self.assertEqual(
                envelope["analysis"]["raw_archive"]["path"],
                "bench/canonical-competitor-results/ac-sparql/run-fixture/raw-sanitized.tar.zst",
            )
            self.assertTrue(envelope["analysis"]["raw_archive_verification"]["all_members_rehashed"])
            self.assertEqual(
                envelope["analysis"]["raw_archive_verification"]["regular_members"],
                envelope["analysis"]["raw_archive_verification"]["manifest"]["entry_count"] + 1,
            )
            self.assertTrue(all("archive_member" in item for item in envelope["analysis"]["input_files"]))
            self.assertIn("guarded_stack_content_reference", envelope["results"])
            self.assertNotIn("paired_guarded_interface_premium", envelope["results"])
            self.assertEqual(envelope["correctness"]["records"], 288)
            self.assertEqual(envelope["correctness"]["oracle_comparisons"], 5760)
            self.assertEqual(
                envelope["correctness"]["materialized_oracle_comparisons"], 4608
            )
            for declaration in envelope["paper_evidence"].values():
                pointer = declaration["pointer"]
                if pointer in declared and declaration["unit"] == "boolean":
                    self.assertIn(declaration.get("hypothesis"), {"H1", "H2"})
            self.assertEqual(envelope["analysis"]["machine_interpretation"]["h2_materialized"], "all")
            self.assertEqual(envelope["analysis"]["machine_interpretation"]["h2_http"], "none")

    def test_no_primary_query_domain_or_pod_is_selected_out_of_overhead_figure(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            fx = Fixture(Path(directory))
            fx.publish()
            svg = (fx.figures / "guarded-stack-content-reference.svg").read_text(encoding="utf-8")
            # Four metrics x four lane/domain facets x eight queries x five Pod counts.
            self.assertEqual(svg.count('r="2.4"'), 4 * 4 * 8 * 5)
            self.assertIn("not pooled", svg)

    def test_realistic_correctness_file_has_frozen_26_record_composition(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            fx = Fixture(Path(directory))
            path = sorted((fx.run / "correctness").glob("*.jsonl"))[0]
            records = [json.loads(line) for line in path.read_text(encoding="utf-8").splitlines()]
            self.assertEqual(len(records), 26)
            self.assertEqual(
                {
                    kind: sum(record["record_type"] == kind for record in records)
                    for kind in fx.mod.CORRECTNESS_RECORD_COUNTS
                },
                fx.mod.CORRECTNESS_RECORD_COUNTS,
            )
            gate = fx.mod.validate_correctness_file_records(
                records, path, fx.source_commit
            )
            self.assertEqual(gate["record_type"], "correctness-gate")

    def test_correctness_file_rejects_unknown_duplicate_and_cli_principal_records(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            fx = Fixture(Path(directory))
            path = sorted((fx.run / "correctness").glob("*.jsonl"))[0]
            original = [json.loads(line) for line in path.read_text(encoding="utf-8").splitlines()]

            unknown = json.loads(json.dumps(original))
            unknown[-1]["record_type"] = "unexpected-record"
            with self.assertRaisesRegex(fx.mod.PublicationError, "unknown record_type"):
                fx.mod.validate_correctness_file_records(unknown, path, fx.source_commit)

            duplicate_gate = json.loads(json.dumps(original))
            gate = next(record for record in duplicate_gate if record["record_type"] == "correctness-gate")
            construction_index = next(
                index for index, record in enumerate(duplicate_gate)
                if record["record_type"] == "construction"
            )
            duplicate_gate[construction_index] = dict(gate)
            with self.assertRaisesRegex(fx.mod.PublicationError, "record composition changed"):
                fx.mod.validate_correctness_file_records(
                    duplicate_gate, path, fx.source_commit
                )

            duplicate_observation = json.loads(json.dumps(original))
            observation_indices = [
                index for index, record in enumerate(duplicate_observation)
                if record["record_type"] == "observation"
            ]
            duplicate_observation[observation_indices[-1]] = dict(
                duplicate_observation[observation_indices[-2]]
            )
            with self.assertRaisesRegex(fx.mod.PublicationError, "repeats observation"):
                fx.mod.validate_correctness_file_records(
                    duplicate_observation, path, fx.source_commit
                )

            cli_principal = json.loads(json.dumps(original))
            for record in cli_principal:
                record["principal"] = "recipient"
                record["cell_label"] = "recipient-100"
            with self.assertRaisesRegex(fx.mod.PublicationError, "unknown serialized principal"):
                fx.mod.validate_correctness_file_records(
                    cli_principal, path, fx.source_commit
                )

    def test_missing_summary_cell_fails_before_publication(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            fx = Fixture(Path(directory))
            with self.assertRaisesRegex(fx.mod.PublicationError, "summary matrix incomplete"):
                fx.mod.validate_summary(fx.summary_rows[:-1], fx.configs)
            self.assertFalse(fx.envelope.exists())

    def test_derived_byte_drift_fails_closed(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            fx = Fixture(Path(directory))
            with (fx.derived / "summary.csv").open("a", encoding="utf-8") as output:
                output.write("tamper\n")
            with self.assertRaisesRegex(fx.mod.PublicationError, "artifact drift"):
                fx.publish()
            self.assertFalse(fx.envelope.exists())

    def test_h2_verdict_is_recomputed_not_trusted(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            fx = Fixture(Path(directory))
            document = json.loads(json.dumps(fx.h2))
            document["results"][0]["minimal_overhead"] = not document["results"][0]["minimal_overhead"]
            summary_rows = fx.mod.read_csv(
                fx.derived / "summary.csv", "summary.csv", fx.mod.SUMMARY_FIELDS
            )
            summary = fx.mod.validate_summary(summary_rows, fx.configs)
            manifest = json.loads((fx.derived / "manifest.json").read_text(encoding="utf-8"))
            with self.assertRaisesRegex(fx.mod.PublicationError, "joint verdict"):
                fx.mod.validate_h2(document, summary, manifest, 10000, 20260903)

    def test_cost_over_budget_is_rejected_even_when_hash_is_updated(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            fx = Fixture(Path(directory))
            metadata = json.loads(fx.metadata.read_text(encoding="utf-8"))
            cost_path = Path(metadata["cost_report"]["path"])
            cost = json.loads(cost_path.read_text(encoding="utf-8"))
            cost["components_usd"] = {
                "canonical_compute": "100.01", "canonical_storage": "0.00",
                "canonical_public_ipv4": "0.00",
                "prior_valid_and_invalid_attempts": "0.00", "other": "0.00",
            }
            cost["study_total_usd"] = "100.01"
            cost["canonical_run_usd"] = "100.01"
            write_json(cost_path, cost)
            metadata["cost_report"].update({"sha256": digest(cost_path), "bytes": cost_path.stat().st_size})
            write_json(fx.metadata, metadata)
            with self.assertRaisesRegex(fx.mod.PublicationError, "USD100"):
                fx.publish()

    def test_raw_archive_must_contain_the_verified_manifest(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            fx = Fixture(Path(directory))
            metadata = json.loads(fx.metadata.read_text(encoding="utf-8"))
            archive = fx.root / metadata["raw_run"]["archive"]["path"]
            wrong = fx.root.parent / "wrong.txt"
            wrong.write_text("wrong\n", encoding="utf-8")
            write_deterministic_tar_zst(
                archive, [("run-fixture/MANIFEST.sha256", wrong)], fx.zstd
            )
            metadata["raw_run"]["archive"].update({"sha256": digest(archive), "bytes": archive.stat().st_size})
            write_json(fx.metadata, metadata)
            with self.assertRaisesRegex(fx.mod.PublicationError, "digest mismatch|member set mismatch"):
                fx.publish()

    def test_check_mode_and_immutable_outputs_fail_closed(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            fx = Fixture(Path(directory))
            fx.publish()
            checked = fx.mod.export(
                fx.root, fx.derived, fx.metadata, fx.envelope, fx.figures,
                zstd_command=fx.zstd, mode="check",
            )
            self.assertEqual(checked, json.loads(fx.envelope.read_text(encoding="utf-8")))
            target = fx.figures / "factor-sensitivities.svg"
            target.write_text("<svg/>\n", encoding="utf-8")
            with self.assertRaisesRegex(fx.mod.PublicationError, "immutable publication target differs"):
                fx.publish()

    def test_primary_cross_p_result_identity_change_is_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            fx = Fixture(Path(directory))
            target = fx.run / "raw/cell-0000.jsonl"
            records = [json.loads(line) for line in target.read_text(encoding="utf-8").splitlines()]
            observation = next(record for record in records if record["record_type"] == "observation")
            observation["result_hash_sha256"] = "f" * 64
            target.write_text(
                "".join(json.dumps(record, sort_keys=True) + "\n" for record in records),
                encoding="utf-8",
            )
            with self.assertRaisesRegex(fx.mod.PublicationError, "cross-P invariant failed"):
                fx.mod.validate_primary_cross_p_invariant(fx.run, fx.input_files)

    def test_secret_scanner_rejects_credentials_without_echoing_them(self) -> None:
        with self.assertRaisesRegex(Exception, "AWS access-key identifier"):
            module = load_module(SOURCE)
            module.hash_and_scan_member(
                io.BytesIO(b"access=AKIAABCDEFGHIJKLMNOP\n"), "run-fixture/raw/example.jsonl"
            )

    def test_exclusion_count_is_derived_from_every_discovered_sentinel(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            fx = Fixture(Path(directory))
            unannotated = fx.run / "failures/unannotated/FAILED.json"
            write_json(unannotated, {
                "schema_version": 1,
                "status": "FAILED",
                "attempt_id": "unannotated",
                "reason_code": "synthetic",
                "exit_status": 2,
            })
            report_path = Path(
                json.loads(fx.metadata.read_text(encoding="utf-8"))["exclusion_report"]["path"]
            )
            report = json.loads(report_path.read_text(encoding="utf-8"))
            with self.assertRaisesRegex(
                fx.mod.PublicationError, "do not bind every discovered"
            ):
                fx.mod.derive_exclusions(report, fx.run)


if __name__ == "__main__":
    unittest.main()
