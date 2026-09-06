#!/usr/bin/env python3
"""[GPT-6] Analyze exploratory HTTP pilots; never admit a capacity/equivalence claim.

Only declared measurement inputs are parsed. Other manifest members are streamed
for checksum verification without exposing their contents. All percentiles use
nearest rank; failed/missing requests remain in the offered denominator.
"""
import argparse
from collections import Counter
import hashlib
import json
import math
from pathlib import Path
import re


def sha256(data):
    return hashlib.sha256(data).hexdigest()


def quantiles(values):
    values = sorted(x for x in values if isinstance(x, (int, float)) and x >= 0)
    if not values:
        return {"n": 0, "p50": None, "p95": None, "p99": None, "maximum": None}
    return {"n": len(values), **{f"p{p}": values[math.ceil(len(values) * p / 100) - 1]
                              for p in (50, 95, 99)}, "maximum": values[-1]}


class Inputs:
    def __init__(self, root):
        self.root = root.resolve()
        self.contents = {}
        self.errors = []

    def read(self, name):
        if name not in self.contents:
            path = (self.root / name).resolve()
            if self.root not in path.parents:
                raise ValueError(f"Input escapes artifact directory: {name}")
            self.contents[name] = path.read_bytes() if path.is_file() else None
        return self.contents[name]

    def text(self, name):
        data = self.read(name)
        return None if data is None else data.decode("utf-8")

    def json(self, name):
        text = self.text(name)
        if text is None or not text.strip():
            return None
        try:
            return json.loads(text)
        except ValueError:
            self.errors.append({"file": name, "error": "invalid-json"})
            return None

    def rows(self, name):
        text = self.text(name)
        rows = []
        for i, line in enumerate((text or "").splitlines(), 1):
            if not line.strip():
                continue
            try:
                row = json.loads(line)
                if not isinstance(row, dict):
                    raise ValueError("not an object")
                rows.append(row)
            except ValueError:
                self.errors.append({"file": name, "line": i, "error": "invalid-jsonl"})
        return rows

    def verify_manifest(self):
        manifest = self.text("MANIFEST.sha256")
        if manifest is None:
            return {"status": "missing-provisional", "verified_files": 0, "errors": []}
        errors, count, seen = [], 0, set()
        for line in manifest.splitlines():
            match = re.fullmatch(r"([0-9a-f]{64}) [ *](.+)", line)
            if not match:
                errors.append({"error": "invalid-manifest-line"})
                continue
            expected, name = match.groups()
            path = (self.root / name).resolve()
            if self.root not in path.parents or name in seen:
                errors.append({"file": name, "error": "unsafe-or-duplicate-manifest-path"})
                continue
            seen.add(name)
            if not path.is_file():
                errors.append({"file": name, "error": "missing"})
                continue
            # Already parsed inputs are checked from the same byte snapshot.
            if name in self.contents:
                actual = sha256(self.contents[name]) if self.contents[name] is not None else None
            else:
                h = hashlib.sha256()
                with path.open("rb") as handle:
                    for block in iter(lambda: handle.read(1024 * 1024), b""):
                        h.update(block)
                actual = h.hexdigest()
            if actual != expected:
                errors.append({"file": name, "error": "checksum-mismatch"})
            else:
                count += 1
        measurement_suffixes = ("-requests.jsonl", "-manifest.json", "-verify.jsonl", "-outcome.json",
                                "-status-after.txt", "-smaps.txt", "-disk.txt", "-io-after.txt",
                                "-server.jsonl", "-pod-summaries.jsonl")
        for name, data in self.contents.items():
            if data is not None and name.endswith(measurement_suffixes) and name not in seen:
                errors.append({"file": name, "error": "measurement-not-in-final-manifest"})
        return {"status": "verified" if not errors else "invalid", "verified_files": count,
                "errors": errors, "manifest_sha256": sha256(manifest.encode())}


def proc_numbers(text):
    result = {}
    for line in (text or "").splitlines():
        match = re.fullmatch(r"([A-Za-z_]+):\s+(\d+)(?:\s+(kB))?", line)
        if match:
            name, number, unit = match.groups()
            result[name] = int(number) * (1024 if unit == "kB" else 1)
    return result


def summarize_requests(rows, deadline_us=200000):
    starts = [x for x in rows if x.get("record_type") == "load-start"]
    ends = [x for x in rows if x.get("record_type") == "load-complete"]
    requests = [x for x in rows if x.get("record_type") == "request"]
    start = starts[0] if starts else {}
    end = ends[-1] if ends else {}
    settings = start.get("settings", {})
    declared = int(settings["requests"]) if "requests" in settings else None
    offered = end.get("offered", declared)
    seqs = [x.get("sequence") for x in requests]
    issues = []
    if len(starts) != 1 or len(ends) != 1:
        issues.append("missing-or-duplicate-start-completion")
    if declared is not None and offered != declared:
        issues.append("start-completion-offered-disagree")
    if offered is None or set(seqs) != set(range(offered)) or len(seqs) != offered:
        issues.append("missing-duplicate-or-invalid-request-sequences")
    dropped = sum(x.get("outcome") == "client-admission-drop" for x in requests)
    if end and end.get("client_dropped") != dropped:
        issues.append("client-drop-total-disagrees")
    successful = [x for x in requests if x.get("outcome") == "ok" and x.get("status") == 200]
    within = [x for x in successful if isinstance(x.get("scheduled_latency_us"), (int, float))
              and 0 <= x["scheduled_latency_us"] <= deadline_us]
    server_within = [x for x in successful if isinstance(x.get("server_us"), (int, float))
                     and 0 <= x["server_us"] <= deadline_us]
    success_fraction = len(successful) / offered if offered else None
    elapsed = end.get("elapsed_us")
    groups = {}
    for group_field in ("operation", "query_id"):
        counts = {}
        for key in sorted({x.get(group_field, "unclassified") for x in requests}):
            subset = [x for x in requests if x.get(group_field, "unclassified") == key]
            counts[key] = {"recorded": len(subset),
                           "successful": sum(x.get("outcome") == "ok" and x.get("status") == 200 for x in subset),
                           "outcomes": dict(sorted(Counter(x.get("outcome", "missing") for x in subset).items()))}
        groups[group_field] = counts
    policy = [x for x in successful if x.get("operation") == "policy-write"]
    cache = [x for x in successful if x.get("cache_hit") in (0, 1)]
    timeout_ms = int(settings["timeout-ms"]) if "timeout-ms" in settings else None
    failures = [x for x in requests if x not in successful]
    mutations = [x for x in requests if x.get("operation") in ("content-write", "policy-write")]
    return {
        "complete_record_sequence": not issues, "data_issues": issues,
        "settings": {k: settings[k] for k in ("arrival", "mix", "pods", "query-set", "rate", "requests",
                                              "seed", "selection", "timeout-ms") if k in settings},
        "offered": offered, "recorded": len(requests), "successful": len(successful),
        "unsuccessful_or_unrecorded": offered - len(successful) if offered is not None else None,
        "success_fraction": success_fraction, "outcomes": dict(sorted(Counter(x.get("outcome", "missing") for x in requests).items())),
        "diagnostic_deadline_us": deadline_us, "successful_within_scheduled_deadline": len(within),
        "successful_within_server_production_deadline": len(server_within),
        "within_scheduled_deadline_fraction_of_offered": len(within) / offered if offered else None,
        "elapsed_including_drain_us": elapsed,
        "observed_successful_completions_per_second": len(successful) * 1e6 / elapsed if elapsed else None,
        "observed_deadline_completions_per_second": len(within) * 1e6 / elapsed if elapsed else None,
        "throughput_scope": "observed completions over elapsed time including drain, not sustainable goodput or a saturation limit",
        "latency_us": {
            "successful_complete_body_from_scheduled_arrival": quantiles(x.get("scheduled_latency_us") for x in successful),
            "successful_complete_body_from_http_dispatch": quantiles(x.get("http_latency_us") for x in successful),
            "all_recorded_outcomes_from_scheduled_arrival": quantiles(x.get("scheduled_latency_us") for x in requests),
            "successful_server_response_production": quantiles(x.get("server_us") for x in successful),
        },
        "successful_phase_us": {key: quantiles(x.get(key) for x in successful) for key in
                                 ("dispatch_lag_us", "credential_preparation_us", "queue_us", "auth_us",
                                  "load_us", "materialize_us", "operation_us")},
        "cache": {"successful_rows_with_indicator": len(cache),
                  "hit_fraction": sum(x["cache_hit"] for x in cache) / len(cache) if cache else None,
                  "miss_scheduled_us": quantiles(x.get("scheduled_latency_us") for x in cache if x["cache_hit"] == 0),
                  "hit_scheduled_us": quantiles(x.get("scheduled_latency_us") for x in cache if x["cache_hit"] == 1),
                  "maximum_reported_source_bytes": max((x.get("cache_source_bytes", 0) for x in requests), default=None)},
        "policy_write_coverage": {"successful": len(policy),
                                  "zero_triple_delta": sum(x.get("policy_triple_delta") == 0 for x in policy),
                                  "nonzero_triple_delta": sum(isinstance(x.get("policy_triple_delta"), int) and x["policy_triple_delta"] != 0 for x in policy),
                                  "unknown_delta": sum("policy_triple_delta" not in x for x in policy),
                                  "effective_rights_change_verified_by_timing_rows": False},
        "content_write_scope": "existing-value replacement; neither net-new ingestion nor expiry was exercised",
        "mutation_commit_evidence": {
            "acknowledged_success": sum(x.get("outcome") == "ok" and x.get("status") == 200 for x in mutations),
            "transport_or_body_failure_commit_unknown": sum(x.get("outcome") in ("transport-error", "body-error") for x in mutations),
            "reconciled_against_durable_journal": False,
            "scope": "a failed client receipt does not prove the server rolled back or never committed the write",
        },
        "explicit_timeout_count": sum(x.get("timeout") is True or x.get("outcome") == "timeout" for x in failures),
        "failure_durations_at_least_configured_timeout": sum(isinstance(x.get("http_latency_us"), (int, float))
                and x["http_latency_us"] >= timeout_ms * 1000 for x in failures) if timeout_ms is not None else None,
        "timeout_classification": "generic transport-error rows do not identify the underlying cause; duration is not a timeout classification",
        "groups": groups,
    }


def analyze(root):
    inputs = Inputs(root)
    labels = sorted({p.name.removesuffix("-manifest.json") for p in root.glob("pilot-*-manifest.json")}
                    | {p.name.removesuffix("-outcome.json") for p in root.glob("pilot-*-outcome.json")}
                    | {p.name.removesuffix("-requests.jsonl") for p in root.glob("pilot-*-requests.jsonl")})
    configurations = []
    for label in labels:
        manifest = inputs.json(label + "-manifest.json") or {}
        verification = [x for x in inputs.rows(label + "-verify.jsonl") if x.get("record_type") == "verification-complete"]
        request_rows = inputs.rows(label + "-requests.jsonl")
        records = summarize_requests(request_rows)
        # Server stdout may also contain non-JSON diagnostics; parse only its
        # structured readiness record, without treating log text as instructions.
        server_records = []
        for line in (inputs.text(label + "-server.jsonl") or "").splitlines():
            try:
                row = json.loads(line)
            except ValueError:
                continue
            if isinstance(row, dict) and row.get("record_type") == "server-ready":
                server_records.append(row)
        server = server_records[0] if server_records else {}
        server_settings = server.get("settings", {})
        pod_summaries = inputs.rows(label + "-pod-summaries.jsonl")
        maximum_pod_bytes = int(server_settings["max-pod-bytes"]) if "max-pod-bytes" in server_settings else None
        oversized = {x["pod_id"] for x in pod_summaries
                     if maximum_pod_bytes is not None and x.get("bytes", 0) > maximum_pod_bytes}
        status = proc_numbers(inputs.text(label + "-status-after.txt"))
        smaps = proc_numbers(inputs.text(label + "-smaps.txt"))
        io_before = proc_numbers(inputs.text(label + "-io-before.txt"))
        io_after = proc_numbers(inputs.text(label + "-io-after.txt"))
        disk = inputs.text(label + "-disk.txt")
        disk_lines = (disk or "").splitlines()
        config = manifest.get("config", {})
        configurations.append({
            "label": label, "model": manifest.get("model"), "pods": manifest.get("pods"),
            "corpus_profile": config.get("profile"), "literal_profile": config.get("literal_profile"),
            "configuration_outcome": inputs.json(label + "-outcome.json"),
            "server": {"workers": server.get("workers"), "authentication": server.get("authentication"),
                       "settings": {k: server_settings[k] for k in ("cache-bytes", "cache-pods", "max-pod-bytes", "queue-capacity")
                                    if k in server_settings}},
            "pod_admission": {"source_byte_limit": maximum_pod_bytes,
                              "stored_pods_above_limit": len(oversized) if maximum_pod_bytes is not None else None,
                              "scheduled_requests_targeting_above_limit_pods": sum(x.get("pod") in oversized for x in request_rows
                                                                                        if x.get("record_type") == "request"),
                              "scope": "size comparison only; does not classify a timed-out or generic transport failure as a server rejection"},
            "verification": verification[-1] if verification else None,
            "verification_scope": "pack/index checksum validation and sampled policy-neutral reference queries; not all canonical admission gates",
            "requests": records,
            "footprint": {
                "manifest": {k: manifest.get(k) for k in ("source_bytes", "packed_bytes", "index_bytes", "quads", "records",
                    "maximum_pod_source_bytes", "maximum_pod_compressed_bytes", "generation_us", "packed_sha256",
                    "index_sha256", "populated", "binary_payloads_included")},
                "allocated_corpus_bytes_before_load": int(disk_lines[-1].split()[0]) if disk_lines else None,
                "postload_journal_allocated_bytes": None,
                "per_pod_source_bytes": quantiles(x.get("bytes") for x in pod_summaries),
                "per_pod_compressed_bytes": quantiles(x.get("compressed_bytes") for x in pod_summaries),
                "per_pod_records": quantiles(x.get("records") for x in pod_summaries),
                "process_after_bytes": {k: status.get(k) for k in ("VmRSS", "VmHWM")},
                "smaps_after_bytes": {k: smaps.get(k) for k in ("Rss", "Pss", "Private_Clean", "Private_Dirty")},
                "process_io_delta_bytes": {k: io_after[k] - io_before[k] for k in ("read_bytes", "write_bytes", "rchar", "wchar")
                                           if k in io_before and k in io_after},
                "cgroup_memory_peak_bytes": None,
                "scope": "remote persisted pack manifest; pre-load du excludes later journal state; process snapshots do not establish the server cgroup RAM minimum",
            },
            "protocol_capacity_admitted": False, "equivalence_admitted": False,
        })
    scope = inputs.json("runner-scope.json")
    source_commit = (inputs.text("source-commit.txt") or "").strip()
    declared_hashes = {}
    for line in (inputs.text("input-hashes.txt") or "").splitlines():
        match = re.fullmatch(r"([0-9a-f]{64}) [ *](.+)", line)
        if match:
            declared_hashes[match.group(2)] = match.group(1)
    environment = {}
    for line in (inputs.text("environment.txt") or "").splitlines():
        if line.startswith("rustc "):
            environment["compiler"] = line
        elif ":" in line:
            key, value = line.split(":", 1)
            if key.strip() in ("Architecture", "CPU(s)", "Model name", "Thread(s) per core", "Core(s) per socket"):
                environment[key.strip()] = value.strip()
    done = inputs.read("DONE") is not None
    failed = inputs.read("FAILED") is not None
    storage_stops = [{"file": p.name, "outcome": inputs.json(p.name)} for p in sorted(root.glob("*-storage-stop.json"))]
    integrity = inputs.verify_manifest()
    complete = done and not failed and integrity["status"] == "verified" and not inputs.errors
    return {
        "schema_version": 1, "analysis": "exploratory-pilot", "canonical": False,
        "artifact_directory": str(root.resolve()), "run_id": root.name, "source_commit": source_commit,
        "analysis_script_sha256": sha256(Path(__file__).read_bytes()),
        "declared_source_input_sha256": declared_hashes, "host_environment": environment,
        "runner_scope": scope, "artifact_integrity": integrity,
        "artifact_status": "complete-checksummed" if complete else "provisional-or-invalid",
        "data_errors": inputs.errors, "storage_stops": storage_stops,
        "protocol_capacity_admitted": False, "equivalence_admitted": False,
        "claim_limitations": ["Exploratory owner-only, uniformly selected query templates with replacement writes",
                              "One schedule per configuration; fixed WAC then ACP order; no independent paired inference",
                              "Offered pilot rate is not population-derived service demand or independently established saturation",
                              "Latency percentiles of successful replies exclude failures; all-offered deadline fractions retain them",
                              "No original media binaries; corpus is partially calibrated, not all service data",
                              "No policy-update or minimum-cgroup-memory claim from unexercised operations or absent counters"],
        "configurations": configurations,
        "input_sha256": {name: sha256(data) for name, data in sorted(inputs.contents.items()) if data is not None},
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("artifact_directory", type=Path)
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()
    result = analyze(args.artifact_directory)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(result, indent=2, sort_keys=True, allow_nan=False) + "\n")
    print(json.dumps({"output": str(args.output), "status": result["artifact_status"],
                      "configurations": len(result["configurations"]), "canonical": False}))


if __name__ == "__main__":
    main()
