"""[GPT-6] Source-bound analysis of frozen HTTP campaigns, including failed runs.

A local passing cell is distinct from full-service capacity. Coarse rate brackets
bound an operational capacity only under the stated monotonicity assumption; an
equal highest tested passing rate is never silently treated as true equivalence.
"""
from collections import Counter, defaultdict
from contextlib import contextmanager
import hashlib
import io
import itertools
import json
import math
from pathlib import Path
import random
import re
import sqlite3
import subprocess
import tempfile

from indexed_analysis import indexed_component_summary


def canonical(value):
    return json.dumps(value, sort_keys=True, separators=(",", ":"), allow_nan=False)


def sha_file(path):
    h = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            h.update(block)
    return h.hexdigest()


def percentile(values, fraction):
    values = sorted(values)
    return values[math.ceil(len(values) * fraction) - 1] if values else None


def stats(values):
    values = sorted(values)
    if not values:
        return {"n": 0, "p50": None, "p95": None, "p99": None, "maximum": None}
    return {"n": len(values), "p50": values[math.ceil(len(values) * .5) - 1],
            "p95": values[math.ceil(len(values) * .95) - 1],
            "p99": values[math.ceil(len(values) * .99) - 1], "maximum": values[-1]}


def numeric(value):
    return isinstance(value, (int, float)) and not isinstance(value, bool) and math.isfinite(value) and value >= 0


class Evidence:
    """Stream compressed inputs, retaining hashes rather than whole inventories."""
    def __init__(self, root):
        self.root = Path(root).resolve()
        self.hashes, self.manifest_hashes, self.errors, self.parse_errors = {}, {}, [], []
        self.manifest_status = "missing-provisional"
        manifest = self.root / "MANIFEST.sha256"
        if not manifest.is_file():
            return
        self.hashes["MANIFEST.sha256"] = sha_file(manifest)
        for line in manifest.read_text().splitlines():
            match = re.fullmatch(r"([0-9a-f]{64}) [ *](.+)", line)
            if not match:
                self.errors.append({"file": "MANIFEST.sha256", "error": "invalid-line"}); continue
            expected, name = match.groups()
            try:
                path = self.safe(name)
            except ValueError:
                self.errors.append({"file": name, "error": "unsafe-path"}); continue
            if name in self.manifest_hashes:
                self.errors.append({"file": name, "error": "duplicate-manifest-member"}); continue
            self.manifest_hashes[name] = expected
            if not path.is_file() or sha_file(path) != expected:
                self.errors.append({"file": name, "error": "missing-or-checksum-mismatch"})
        self.manifest_status = "verified" if not self.errors else "invalid"

    def safe(self, name):
        path = (self.root / name).resolve()
        if self.root not in path.parents:
            raise ValueError("Artifact path escapes root")
        return path

    def locate(self, name):
        paths = [self.safe(name)]
        if name.endswith(".jsonl"):
            paths.append(self.safe(name + ".zst"))
        present = [p for p in paths if p.is_file()]
        if len(present) > 1:
            self.errors.append({"file": name, "error": "ambiguous-plain-and-compressed-input"})
            return None
        return present[0] if present else None

    def checked(self, path):
        name = str(path.relative_to(self.root))
        digest = sha_file(path)
        if name in self.hashes and self.hashes[name] != digest:
            self.errors.append({"file": name, "error": "changed-during-analysis"})
        self.hashes[name] = digest
        if self.manifest_status != "missing-provisional" and self.manifest_hashes.get(name) != digest:
            self.errors.append({"file": name, "error": "parsed-input-not-bound-by-manifest"})

    @contextmanager
    def stream(self, name):
        path = self.locate(name)
        if path is None:
            yield None; return
        self.checked(path)
        if path.suffix == ".zst":
            # CLI streaming avoids retaining decompressed multi-million-Pod data.
            with tempfile.TemporaryFile() as errors:
                process = subprocess.Popen(["zstd", "-d", "-q", "-c", str(path)], stdout=subprocess.PIPE, stderr=errors)
                try:
                    with io.TextIOWrapper(process.stdout, encoding="utf-8") as handle:
                        yield handle
                finally:
                    if process.poll() is None:
                        process.wait()
                    if process.returncode:
                        self.errors.append({"file": path.name, "error": "zstd-decompression-failed"})
        else:
            with path.open(encoding="utf-8") as handle:
                yield handle
        self.checked(path)

    def text(self, name):
        with self.stream(name) as stream:
            return stream.read() if stream is not None else None

    def document(self, name):
        text = self.text(name)
        if not text:
            return None
        try:
            result = json.loads(text)
            if not isinstance(result, dict): raise ValueError("expected object")
            return result
        except ValueError:
            self.parse_errors.append({"file": name, "error": "invalid-json"}); return None

    def rows(self, name, strict=True):
        with self.stream(name) as stream:
            if stream is None: return
            for number, line in enumerate(stream, 1):
                if not line.strip(): continue
                try:
                    row = json.loads(line)
                    if not isinstance(row, dict): raise ValueError("expected object")
                    yield row
                except ValueError:
                    if strict:
                        self.parse_errors.append({"file": name, "line": number, "error": "invalid-jsonl"})


def key_values(text):
    result = {}
    for line in (text or "").splitlines():
        parts = line.replace(":", " ").split()
        if len(parts) >= 2 and parts[1].isdigit():
            result[parts[0]] = int(parts[1]) * (1024 if len(parts) > 2 and parts[2] == "kB" else 1)
    return result


def resource_summary(before, after, memory_gib):
    issues = []
    samples = {}
    for label, document in (("before", before), ("after", after)):
        if not document or "error" in document:
            issues.append(label + "-resource-capture-missing-or-failed")
        document = document or {}
        samples[label] = {"memory_peak_bytes": int(document["memory.peak"].strip()) if document.get("memory.peak", "").strip().isdigit() else None,
                          "memory_current_bytes": int(document["memory.current"].strip()) if document.get("memory.current", "").strip().isdigit() else None,
                          "memory_events": key_values(document.get("memory.events")),
                          "memory_stat_bytes": key_values(document.get("memory.stat")),
                          "cpu_stat": key_values(document.get("cpu.stat")),
                          "process_status_bytes": {k: v for k, v in key_values(document.get("proc/status")).items() if k.startswith("Vm")},
                          "process_smaps_bytes": key_values(document.get("proc/smaps_rollup")),
                          "process_io_bytes": key_values(document.get("proc/io")),
                          "io_stat_raw": document.get("io.stat")}
        if samples[label]["memory_peak_bytes"] is None or samples[label]["memory_current_bytes"] is None:
            issues.append(label + "-memory-counter-missing")
        events = samples[label]["memory_events"]
        if not {"oom", "oom_kill"}.issubset(events):
            issues.append(label + "-oom-counters-missing")
        elif events["oom"] or events["oom_kill"]:
            issues.append(label + "-oom-observed")
    ceiling = memory_gib * 1024**3
    peak = samples["after"]["memory_peak_bytes"]
    if peak is not None and peak > ceiling: issues.append("memory-ceiling-exceeded")
    deltas = {}
    for section in ("cpu_stat", "process_io_bytes", "memory_events"):
        b, a = samples["before"][section], samples["after"][section]
        deltas[section] = {k: a[k] - b[k] for k in a.keys() & b.keys()}
        if any(value < 0 for value in deltas[section].values()):
            issues.append("resource-counter-regression:" + section)
    return {"passed": not issues, "issues": issues, "ceiling_bytes": ceiling,
            "samples": samples, "deltas": deltas,
            "scope": "fresh per-server cgroup peak includes warmup and charged page cache; not a whole-host memory minimum"}


def inventory_summary(evidence, dataset, model):
    label = dataset["id"] + "-" + model
    manifest = evidence.document(label + "-manifest.json") or {}
    count, records, quads, source_bytes, packed_bytes, largest, services = 0, 0, 0, 0, 0, 0, Counter()
    issues = []
    for row in evidence.rows(label + "-pod-summaries.jsonl"):
        # Writer emits Pod IDs in order; check exhaustively without a huge set.
        if row.get("pod_id") != count: issues.append("inventory-id-sequence-mismatch")
        count += 1
        records += row.get("records", 0); quads += row.get("quads", 0)
        source_bytes += row.get("bytes", 0); packed_bytes += row.get("compressed_bytes", 0)
        largest = max(largest, row.get("bytes", 0)); services.update(row.get("records_by_service", {}))
    expected = {"pods": count, "records": records, "quads": quads, "source_bytes": source_bytes,
                "packed_bytes": packed_bytes, "maximum_pod_source_bytes": largest}
    for key, value in expected.items():
        if manifest.get(key) != value: issues.append("manifest-inventory-mismatch:" + key)
    if count != dataset["pods"] or not manifest.get("populated"):
        issues.append("population-not-fully-persisted")
    if sum(services.values()) != records: issues.append("domain-record-totals-disagree")
    # Verification subprocess output may end with plain-text admission errors.
    # Absence of its completion invalidates this dataset, not another dataset.
    verification = [r for r in evidence.rows(label + "-verify.jsonl", strict=False) if r.get("record_type") == "verification-complete"]
    if not verification: issues.append("verification-completion-missing")
    disk = evidence.text(label + "-disk.txt")
    allocated = None
    if disk:
        try: allocated = int(disk.splitlines()[-1].split()[0])
        except (ValueError, IndexError): issues.append("invalid-disk-allocation-record")
    if any(x["file"].startswith(label + "-") for x in evidence.parse_errors):
        issues.append("dataset-record-parsing-failed")
    return {"dataset": dataset["id"], "model": model, "role": dataset["role"],
            "inventory_consistent": not issues, "issues": sorted(set(issues)),
            "manifest": {k: manifest.get(k) for k in ("format", "pods", "records", "quads", "source_bytes", "packed_bytes", "index_bytes",
                "maximum_pod_source_bytes", "maximum_pod_compressed_bytes", "packed_sha256", "index_sha256", "binary_payloads_included", "populated")},
            "config": {k: manifest.get("config", {}).get(k) for k in ("profile", "history_months", "literal_profile")},
            "records_by_service": dict(sorted(services.items())), "allocated_bytes_before_load": allocated,
            "verification": verification[-1] if verification else None,
            "scope": "complete inventory totals checked locally; remote pack/index checksums asserted by verification, payload packs may not accompany review bundle"}


class RequestAnalysis:
    def __init__(self, database, measurement):
        self.db = sqlite3.connect(database)
        self.db.executescript("CREATE TABLE durable(id TEXT PRIMARY KEY,pod INTEGER,digest TEXT);"
                              "CREATE TABLE offered(id TEXT PRIMARY KEY,pod INTEGER,digest TEXT,outcome TEXT);"
                              "CREATE TABLE schedule(sequence INTEGER PRIMARY KEY,digest TEXT);")
        self.measurement = measurement
        self.issues, self.starts, self.ends, self.audit_ends = [], [], [], []
        self.values = defaultdict(list); self.outcomes = Counter(); self.operations = defaultdict(Counter)
        self.query_families = defaultdict(Counter); self.principals = defaultdict(Counter)
        self.scheduled, self.queues = [], []
        self.recorded = self.successful = self.timely = self.server_timely = self.lagged = self.timeouts = 0
        self.explicit_timeout_fields = self.unknown_mutations = self.acknowledged_mutations = 0
        self.receipt_counts, self.policy, self.mutation_errors = Counter(), Counter(), []
        self.cache = Counter(); self.cache_by_outcome = defaultdict(Counter); self.cache_successful = Counter()
        self.http_status = Counter({"missing": 0, "invalid": 0}); self.http_status_by_outcome = defaultdict(Counter)

    def audit(self, rows):
        for row in rows:
            if row.get("record_type") == "mutation-audit-complete": self.audit_ends.append(row)
            if row.get("record_type") != "committed-mutation": continue
            receipt = row.get("receipt")
            if not isinstance(receipt, dict) or not isinstance(receipt.get("id"), str):
                self.mutation_errors.append("invalid-durable-receipt"); continue
            digest = hashlib.sha256(canonical(receipt).encode()).hexdigest()
            try: self.db.execute("INSERT INTO durable VALUES(?,?,?)", (receipt["id"], row.get("pod"), digest))
            except sqlite3.IntegrityError: self.mutation_errors.append("duplicate-durable-receipt")
        self.db.commit()

    def requests(self, rows):
        for row in rows:
            kind = row.get("record_type")
            if kind == "load-start": self.starts.append(row); continue
            if kind == "load-complete": self.ends.append(row); continue
            if kind != "request": continue
            self.recorded += 1
            outcome, operation = row.get("outcome", "missing"), row.get("operation", "unclassified")
            self.outcomes[outcome] += 1; self.operations[operation][outcome] += 1
            # [GPT-6] Count recorded statuses independently of outcomes; never impute a response.
            status = row.get("status")
            status_key = str(status) if type(status) is int and 100 <= status <= 599 else ("missing" if status is None else "invalid")
            self.http_status[status_key] += 1
            self.http_status_by_outcome[outcome][status_key] += 1
            self.query_families[row.get("query_id", "unclassified")][outcome] += 1
            self.principals[row.get("principal", "unclassified")][outcome] += 1
            intended = {k: row.get(k) for k in ("sequence", "pod", "principal", "channel", "scheduled_us", "operation", "service", "query_id", "desired_grant", "planned_records")}
            digest = hashlib.sha256(canonical(intended).encode()).hexdigest()
            sequence = row.get("sequence")
            if not isinstance(sequence, int) or isinstance(sequence, bool) or sequence < 0:
                self.issues.append("invalid-sequence")
            else:
                try: self.db.execute("INSERT INTO schedule VALUES(?,?)", (sequence, digest))
                except sqlite3.IntegrityError: self.issues.append("duplicate-sequence")
            scheduled = row.get("scheduled_us")
            if numeric(scheduled):
                self.scheduled.append(scheduled)
                if numeric(row.get("queue_us")): self.queues.append((scheduled, row["queue_us"]))
            else: self.issues.append("missing-scheduled-arrival")
            if "is_timeout" in row: self.explicit_timeout_fields += 1
            self.timeouts += row.get("is_timeout") is True
            self.lagged += numeric(row.get("dispatch_lag_us")) and row["dispatch_lag_us"] > self.measurement["server_deadline_ms"] * 1000
            successful = outcome == "ok" and row.get("status") == 200
            if outcome == "ok" and not successful: self.issues.append("success-status-disagreement")
            self.successful += successful
            cache_value = row.get("cache_hit")
            cache_state = ("hit" if cache_value == 1 else "miss") if type(cache_value) is int and cache_value in (0, 1) else ("missing" if cache_value is None else "invalid")
            self.cache[cache_state] += 1
            self.cache_by_outcome[outcome][cache_state] += 1
            if successful: self.cache_successful[cache_state] += 1
            deadline = self.measurement["server_deadline_ms"] * 1000
            self.timely += successful and numeric(row.get("scheduled_latency_us")) and row["scheduled_latency_us"] <= deadline
            self.server_timely += successful and numeric(row.get("server_us")) and row["server_us"] <= deadline
            for field in ("scheduled_latency_us", "http_latency_us", "server_us", "queue_us", "auth_us", "load_us", "materialize_us", "operation_us", "dispatch_lag_us", "credential_preparation_us"):
                if numeric(row.get(field)):
                    self.values["all:" + field].append(row[field])
                    if successful: self.values["successful:" + field].append(row[field])
            if successful and not all(numeric(row.get(k)) for k in ("scheduled_latency_us", "http_latency_us", "server_us")):
                self.issues.append("successful-response-timer-missing")
            if "mutation_id" in row:
                receipt = row.get("mutation_receipt")
                digest = hashlib.sha256(canonical(receipt).encode()).hexdigest() if isinstance(receipt, dict) else None
                try: self.db.execute("INSERT INTO offered VALUES(?,?,?,?)", (row["mutation_id"], row.get("pod"), digest, outcome))
                except sqlite3.IntegrityError: self.mutation_errors.append("duplicate-offered-mutation-id")
                if successful:
                    self.acknowledged_mutations += 1
                    if not isinstance(receipt, dict) or receipt.get("id") != row["mutation_id"]:
                        self.mutation_errors.append("acknowledged-receipt-missing-or-wrong-id")
                    else:
                        for field in ("inserted_records", "deleted_records", "inserted_triples", "deleted_triples"):
                            if isinstance(receipt.get(field), int): self.receipt_counts[field] += receipt[field]
                        if "planned_records" in row:
                            exact = receipt.get("inserted_triples") == row.get("expected_inserted_triples") and receipt.get("deleted_triples") == row.get("expected_deleted_triples")
                            absent_noop = operation == "modify" and receipt.get("inserted_triples") == receipt.get("deleted_triples") == 0 and isinstance(receipt.get("poststate_triples"), list) and bool(receipt["poststate_triples"]) and all(x == 0 for x in receipt["poststate_triples"])
                            if not exact and not absent_noop: self.mutation_errors.append("mutation-count-mismatch")
                            if absent_noop: self.receipt_counts["absent-record-modification-noops"] += 1
                elif outcome in ("transport-error", "body-error"):
                    self.unknown_mutations += 1
            elif operation in ("ingest", "expire", "modify", "policy-attempt"):
                self.mutation_errors.append("mutation-id-missing")
            if operation == "policy-attempt":
                self.policy["offered"] += 1
                if successful:
                    self.policy["acknowledged"] += 1
                    delta = row.get("policy_triple_delta")
                    self.policy["zero_delta" if delta == 0 else "nonzero_delta" if isinstance(delta, int) else "unknown_delta"] += 1
        self.db.commit()

    def result(self):
        if len(self.starts) != 1 or len(self.ends) != 1: self.issues.append("missing-or-duplicate-load-boundaries")
        start, end = (self.starts[0] if self.starts else {}), (self.ends[-1] if self.ends else {})
        offered = end.get("offered")
        sequence = self.db.execute("SELECT COUNT(*),MIN(sequence),MAX(sequence) FROM schedule").fetchone()
        if not isinstance(offered, int) or offered <= 0 or sequence != (offered, 0, offered - 1) or self.recorded != offered:
            self.issues.append("offered-sequence-coverage-incomplete")
        if end.get("client_dropped") != self.outcomes["client-admission-drop"]: self.issues.append("drop-total-disagrees")
        if end.get("plan_exhausted") != self.outcomes["client-plan-exhausted"]: self.issues.append("plan-exhaustion-total-disagrees")
        durable = self.db.execute("SELECT COUNT(*) FROM durable").fetchone()[0]
        if len(self.audit_ends) != 1 or self.audit_ends[0].get("committed_receipts") != durable:
            self.mutation_errors.append("audit-completion-missing-or-count-disagrees")
        missing = self.db.execute("SELECT COUNT(*) FROM offered o LEFT JOIN durable d ON o.id=d.id WHERE o.outcome='ok' AND (d.id IS NULL OR o.pod!=d.pod OR o.digest IS NULL OR o.digest!=d.digest)").fetchone()[0]
        if missing: self.mutation_errors.append("acknowledgement-not-identically-durable")
        unexpected = self.db.execute("SELECT COUNT(*) FROM durable d LEFT JOIN offered o ON o.id=d.id WHERE o.id IS NULL OR o.outcome NOT IN ('ok','transport-error','body-error')").fetchone()[0]
        if unexpected: self.mutation_errors.append("durable-receipt-without-permitted-offer")
        wrong_pod = self.db.execute("SELECT COUNT(*) FROM offered o JOIN durable d ON o.id=d.id WHERE o.pod!=d.pod").fetchone()[0]
        if wrong_pod: self.mutation_errors.append("durable-receipt-target-pod-disagrees")
        resolved = self.db.execute("SELECT COUNT(*) FROM offered o JOIN durable d ON o.id=d.id WHERE o.outcome IN ('transport-error','body-error')").fetchone()[0]
        digest = hashlib.sha256()
        for (item,) in self.db.execute("SELECT digest FROM schedule ORDER BY sequence"): digest.update(item.encode())
        queue_rule = self.measurement.get("queue_stability", {})
        sorted_arrivals = sorted(self.scheduled)
        quarter = max(1, len(sorted_arrivals) // 4)
        first_end = sorted_arrivals[quarter - 1] if sorted_arrivals else None
        last_start = sorted_arrivals[-quarter] if sorted_arrivals else None
        first = stats(v for t, v in self.queues if first_end is not None and t <= first_end)["p95"]
        last = stats(v for t, v in self.queues if last_start is not None and t >= last_start)["p95"]
        tolerance = max(queue_rule.get("allowed_growth_us_floor", 1000), (first or 0) * queue_rule.get("allowed_growth_fraction_of_first_quarter", .1))
        coverage = len(self.queues) / offered if offered else None
        queue_pass = first is not None and last is not None and last <= first + tolerance and coverage is not None and coverage >= queue_rule.get("required_timer_coverage_fraction_of_offered", .99)
        elapsed = end.get("elapsed_us")
        rate = start.get("offered_rate")
        duration_ok = numeric(elapsed) and elapsed >= self.measurement["measurement_seconds"] * 1000000 and offered is not None and offered >= self.measurement["minimum_offered"]
        # Scheduled horizon, not drain time, establishes measurement duration.
        if not sorted_arrivals or max(sorted_arrivals) < self.measurement["measurement_seconds"] * 1000000:
            duration_ok = False
        if not duration_ok: self.issues.append("measurement-duration-or-minimum-count-unmet")
        return {"offered": offered, "recorded": self.recorded, "successful": self.successful,
                "within_scheduled_deadline": self.timely, "within_server_production_deadline": self.server_timely,
                "success_fraction_of_offered": self.successful / offered if offered else None,
                "deadline_fraction_of_offered": self.timely / offered if offered else None,
                "outcomes": dict(self.outcomes), "operations": {k: dict(v) for k, v in sorted(self.operations.items())},
                "http_status": {"counts_of_recorded": dict(sorted(self.http_status.items())),
                                "by_outcome": {k: dict(sorted(v.items())) for k, v in sorted(self.http_status_by_outcome.items())},
                                "scope": "recorded request rows only; integer statuses 100–599 retain their code, absent/null is missing, other values are invalid; unrecorded requests receive no status and offered denominators are unchanged"},
                "query_templates": {k: dict(v) for k, v in sorted(self.query_families.items())},
                "principals": {k: dict(v) for k, v in sorted(self.principals.items())},
                "offered_rate": rate, "elapsed_including_drain_us": elapsed,
                "observed_successful_rps": self.successful * 1e6 / elapsed if numeric(elapsed) and elapsed else None,
                "observed_deadline_rps": self.timely * 1e6 / elapsed if numeric(elapsed) and elapsed else None,
                "rps_scope": "successful completions over elapsed including drain; not sustainable capacity",
                "latency_us": {k: stats(v) for k, v in sorted(self.values.items())},
                "cache": {"counts_of_recorded": {k: self.cache[k] for k in ("hit", "miss", "missing", "invalid")},
                          "successful_response_counts": {k: self.cache_successful[k] for k in ("hit", "miss", "missing", "invalid")},
                          "by_outcome": {o: dict(v) for o, v in sorted(self.cache_by_outcome.items())},
                          "classification_coverage_of_offered": (self.cache["hit"] + self.cache["miss"]) / offered if offered else None,
                          "hit_fraction_of_classified": self.cache["hit"] / (self.cache["hit"] + self.cache["miss"]) if self.cache["hit"] + self.cache["miss"] else None,
                          "scope": "observed server cache header only; missing responses or headers are not inferred misses; counts retain failed outcomes and do not alter offered denominators"},
                "issues": sorted(set(self.issues)), "complete_valid_schedule": not self.issues,
                "schedule_sha256": digest.hexdigest(),
                "queue": {"passed": queue_pass, "timer_coverage_of_offered": coverage, "first_quarter_p95_us": first,
                          "last_quarter_p95_us": last, "allowed_growth_us": tolerance,
                          "quarters": "first/last quarters of scheduled arrivals, including arrivals whose requests failed"},
                "client_limited": bool(self.outcomes["client-admission-drop"] or self.lagged),
                "dispatch_lag_over_deadline": self.lagged,
                "explicit_timeouts": self.timeouts, "rows_with_explicit_timeout_classification": self.explicit_timeout_fields,
                "mutation": {"reconciled": not self.mutation_errors, "issues": sorted(set(self.mutation_errors)),
                             "durable_receipts": durable, "acknowledged": self.acknowledged_mutations,
                             "unknown_at_client": self.unknown_mutations, "unknown_resolved_committed": resolved,
                             "unknown_resolved_not_committed": self.unknown_mutations - resolved if not self.mutation_errors else None,
                             "acknowledged_counts": dict(self.receipt_counts), "policy": dict(self.policy),
                             "policy_effective_rights_verified": False,
                             "audit_completion": self.audit_ends[-1] if self.audit_ends else None},
                "load_metadata": {**{k: start.get(k) for k in ("workload_sha256", "derived_rate", "offered_rate", "mutation_epoch", "arrival", "read_target_selection", "write_target_selection")},
                                  "settings": {k: start.get("settings", {}).get(k) for k in ("seed", "scenario", "selection", "mix", "timeout-ms", "max-inflight")},
                                  "corpus": {k: start.get("corpus_manifest", {}).get(k) for k in ("pods", "packed_sha256", "index_sha256")}}}

    def close(self):
        self.db.close()


def paired_ratio_ci(pairs, resamples=10000, seed=2026090699):
    """Geometric mean ACP/WAC ratio; bootstrap independent paired runs."""
    if len(pairs) < 2 or any(not numeric(w) or not numeric(a) or w <= 0 or a <= 0 for w, a in pairs):
        return {"available": False, "reason": "need at least two positive paired independent-run estimates"}
    logs = [math.log(a / w) for w, a in pairs]
    randomizer = random.Random(seed)
    estimates = sorted(math.exp(math.fsum(logs[randomizer.randrange(len(logs))] for _ in logs) / len(logs)) for _ in range(resamples))
    return {"available": True, "paired_runs": len(pairs), "estimate": math.exp(math.fsum(logs) / len(logs)),
            "ci95": [percentile(estimates, .025), percentile(estimates, .975)], "resamples": resamples, "seed": seed,
            "method": "percentile bootstrap of paired runs, geometric mean ACP/WAC ratio; requests are not independent replicates"}


def capacity_bounds(observations, margin=1.10):
    """observations: model -> [{rate, state}], state pass/fail/inconclusive.

    Fail means all independent repetitions fail valid local service checks without
    client limitation, missing evidence or correctness/resource measurement gaps.
    """
    bounds = {}
    for model in ("wac", "acp"):
        rows = observations.get(model, [])
        passing = [r["rate"] for r in rows if r["state"] == "pass"]
        failing = [r["rate"] for r in rows if r["state"] == "fail"]
        lower, upper = max(passing, default=None), min(failing, default=None)
        monotone = lower is None or upper is None or lower < upper
        bounds[model] = {"highest_tested_passing_rate": lower, "lowest_consistently_failing_rate": upper,
                         "monotone_bracket": monotone}
    w, a = bounds["wac"], bounds["acp"]
    ratio = None
    if all(x["monotone_bracket"] and x["highest_tested_passing_rate"] is not None and x["lowest_consistently_failing_rate"] is not None for x in (w, a)):
        ratio = [a["highest_tested_passing_rate"] / w["lowest_consistently_failing_rate"],
                 a["lowest_consistently_failing_rate"] / w["highest_tested_passing_rate"]]
    tested_ratio = a["highest_tested_passing_rate"] / w["highest_tested_passing_rate"] if w["highest_tested_passing_rate"] and a["highest_tested_passing_rate"] else None
    equivalent = ratio is not None and ratio[0] >= 1 / margin and ratio[1] <= margin
    return {"models": bounds, "highest_tested_passing_rate_ratio_acp_over_wac": tested_ratio,
            "conservative_capacity_ratio_bounds": ratio,
            "capacity_equivalence": "bounded-within-margin" if equivalent else "unestablished",
            "margin_interval": [1 / margin, margin],
            "scope": "conditional operational capacity bounds under monotone rate response and frozen repeated-run criteria; not a confidence interval or an interpolated true maximum",
            "interpretation": "equal highest passing grid points do not establish true capacity equivalence; missing/inconclusive upper brackets remain unbounded"}


def dedicated_churn(evidence, dataset, model):
    label = dataset["id"] + "-" + model + "-dedicated-churn"
    rows = list(evidence.rows(label + "-requests.jsonl"))
    audit = list(evidence.rows(label + "-audit.jsonl"))
    issues, durable = [], {}
    for row in audit:
        if row.get("record_type") != "committed-mutation": continue
        receipt = row.get("receipt", {})
        if not isinstance(receipt.get("id"), str) or receipt["id"] in durable:
            issues.append("invalid-or-duplicate-churn-receipt"); continue
        durable[receipt["id"]] = (row.get("pod"), receipt)
    changes = [r for r in rows if r.get("record_type") == "request"]
    expected_states = ["grant", "revoke", "probe-revoked", "probe-revoked", "grant", "revoke", "grant"]
    completes = [r.get("state") for r in rows if r.get("record_type") == "policy-churn-check-complete"]
    if completes != expected_states: issues.append("dedicated-churn-stages-incomplete")
    if [r.get("policy_triple_delta") for r in changes] != [0, -1, 1, -1, 1]:
        issues.append("expected-grant-revoke-changes-not-observed")
    offered_ids = set()
    for row in changes:
        identity = row.get("mutation_id")
        if identity in offered_ids: issues.append("duplicate-churn-mutation-id")
        offered_ids.add(identity)
        if row.get("outcome") != "ok" or row.get("status") != 200 or durable.get(identity) != (row.get("pod"), row.get("mutation_receipt")):
            issues.append("churn-acknowledgement-not-identically-durable")
    if set(durable) != offered_ids: issues.append("unexpected-or-missing-churn-durable-receipt")
    audit_ends = [r for r in audit if r.get("record_type") == "mutation-audit-complete"]
    if len(audit_ends) != 1 or audit_ends[0].get("committed_receipts") != len(durable):
        issues.append("churn-audit-completion-missing")
    negative = [r for r in rows if r.get("record_type") == "negative-policy-probe"]
    if len(negative) != 15 or any(r.get("status") != 403 for r in negative):
        issues.append("negative-policy-administration-probes-failed-or-missing")
    authorization = [r for r in rows if r.get("record_type") == "authorization-probe"]
    if len(authorization) != 21: issues.append("authorization-probe-count-incomplete")
    for row in authorization:
        try: count = int(row.get("count"))
        except (TypeError, ValueError): count = None
        if row.get("outcome") != "ok" or row.get("status") != 200 or count != row.get("expected_calendar_records"):
            issues.append("post-change-authorization-probe-failed")
    eviction = [r for r in rows if r.get("record_type") == "eviction-probe"]
    if dataset["pods"] > 1 and (len(eviction) != 1 or eviction[0].get("outcome") != "ok"):
        issues.append("eviction-probe-failed-or-missing")
    reported = evidence.document(label + "-reconciliation.json")
    if not reported or reported.get("passed") is not True: issues.append("reported-churn-reconciliation-not-passed")
    return {"dataset": dataset["id"], "model": model, "passed": not issues, "issues": sorted(set(issues)),
            "completed_states": completes, "durable_receipts": len(durable),
            "authorization_probes": len(authorization), "negative_administration_probes": len(negative),
            "scope": "generated calendar owner administration, recipient grant/revoke, eviction and runner-declared restart sequence; not a security audit or general ACP ACR conformance proof"}


def expected_cells(campaign):
    datasets = {d["id"]: d for d in campaign["corpora"]}
    for group in campaign["groups"]:
        for dataset_id, memory, cpus, rate, replicate, model in itertools.product(
                group["datasets"], group["memory_gib"], group["cpus"], group.get("rates", ["derived"]), range(group["repeat"]), ("wac", "acp")):
            yield {"label": f'{dataset_id}-{group["id"]}-ram{memory}-cpu{cpus}-r{rate}-{replicate}-{model}',
                   "dataset": dataset_id, "group": group["id"], "memory_gib": memory, "cpus": cpus,
                   "rate_override": rate, "replicate": replicate, "model": model, "seed": campaign["seeds"][replicate],
                   "required_replicates": group["repeat"], "corpus_role": datasets[dataset_id]["role"]}


def pair_tables(cells, margin):
    grouped = defaultdict(list)
    for cell in cells:
        key = (cell["dataset"], cell["group"], cell["memory_gib"], cell["cpus"], str(cell["rate_override"]))
        grouped[key].append(cell)
    result = []
    for key, group in sorted(grouped.items()):
        by_model = {m: sorted((c for c in group if c["model"] == m), key=lambda c: c["replicate"]) for m in ("wac", "acp")}
        expected = group[0]["required_replicates"]
        reasons = []
        if any(len(v) != expected for v in by_model.values()): reasons.append("paired-repetitions-missing")
        pairs, deadline_pairs = [], []
        for w, a in zip(by_model["wac"], by_model["acp"]):
            wr, ar = w.get("requests", {}), a.get("requests", {})
            if w["replicate"] != a["replicate"] or w["seed"] != a["seed"]: reasons.append("paired-seed-or-replicate-mismatch")
            if not w.get("valid_for_inference") or not a.get("valid_for_inference"): reasons.append("invalid-or-unreviewed-pair")
            if wr.get("schedule_sha256") != ar.get("schedule_sha256"): reasons.append("paired-intended-schedules-differ")
            if not numeric(wr.get("offered_rate")) or not numeric(ar.get("offered_rate")) or not math.isclose(wr["offered_rate"], ar["offered_rate"], rel_tol=1e-12): reasons.append("paired-offered-rates-differ")
            pairs.append((wr.get("latency_us", {}).get("successful:scheduled_latency_us", {}).get("p95"),
                          ar.get("latency_us", {}).get("successful:scheduled_latency_us", {}).get("p95")))
            deadline_pairs.append((wr.get("observed_deadline_rps"), ar.get("observed_deadline_rps")))
        latency = paired_ratio_ci(pairs) if not reasons else {"available": False, "reason": "; ".join(sorted(set(reasons)))}
        completed = paired_ratio_ci(deadline_pairs) if not reasons else {"available": False, "reason": "invalid or incomplete paired evidence"}
        all_pass = all(c.get("local_guard") == "pass" for c in group) and len(group) == expected * 2
        interval = latency.get("ci95")
        latency_equivalent = all_pass and interval is not None and interval[0] >= 1 / margin and interval[1] <= margin
        result.append({"dataset": key[0], "group": key[1], "memory_gib": key[2], "cpus": key[3], "rate_override": key[4],
                       "required_pairs": expected, "issues": sorted(set(reasons)),
                       "all_models_and_replicates_pass_local_guard": all_pass,
                       "paired_p95_scheduled_response_ratio": latency,
                       "latency_equivalence": "within-margin" if latency_equivalent else "unestablished",
                       "same_load_observed_deadline_completion_ratio": completed,
                       "same_load_throughput_interpretation": "offered-load-capped completion rate; never a capacity-equivalence result"})
    return result


def bracket_tables(cells, margin):
    grouped = defaultdict(list)
    for cell in cells:
        if cell["rate_override"] == "derived": continue
        key = (cell["dataset"], cell["group"], cell["memory_gib"], cell["cpus"])
        grouped[key].append(cell)
    output = []
    for key, group in sorted(grouped.items()):
        if len({c["rate_override"] for c in group}) < 2: continue
        observations = {}
        for model in ("wac", "acp"):
            model_rows = [c for c in group if c["model"] == model]
            observations[model] = []
            for rate in sorted({c["rate_override"] for c in model_rows}):
                rows = [c for c in model_rows if c["rate_override"] == rate]
                complete = len(rows) == rows[0]["required_replicates"]
                state = "pass" if complete and all(r.get("local_guard") == "pass" for r in rows) else (
                    "fail" if complete and all(r.get("local_guard") == "fail" and r.get("valid_for_inference") for r in rows) else "inconclusive")
                observations[model].append({"rate": rate, "state": state})
        bounded = capacity_bounds(observations, margin)
        independent_pairs = []
        for replicate in range(group[0]["required_replicates"]):
            highest = {}
            for model in ("wac", "acp"):
                passing = [c["rate_override"] for c in group if c["model"] == model and c["replicate"] == replicate and c.get("local_guard") == "pass"]
                highest[model] = max(passing, default=None)
            if all(highest.values()): independent_pairs.append((highest["wac"], highest["acp"]))
        tested_ci = paired_ratio_ci(independent_pairs) if len(independent_pairs) == group[0]["required_replicates"] else {"available": False, "reason": "passing rate missing in a required paired repetition"}
        output.append({"dataset": key[0], "group": key[1], "memory_gib": key[2], "cpus": key[3],
                       "rate_observations": observations, **bounded,
                       "paired_highest_tested_passing_rate_ratio": tested_ci,
                       "paired_grid_ratio_scope": "uncertainty in the highest passing tested-grid statistic; does not resolve untested rates between brackets"})
    return output


def analyze_campaign(root, review_path=None):
    evidence = Evidence(root)
    campaign = evidence.document("campaign.json")
    if not campaign: raise ValueError("A campaign.json object is required")
    source_commit = (evidence.text("source-commit.txt") or "").strip()
    runtime = evidence.document("campaign-runtime.json")
    input_hashes = {}
    for line in (evidence.text("input-hashes.txt") or "").splitlines():
        match = re.fullmatch(r"([0-9a-f]{64}) [ *](.+)", line)
        if match: input_hashes[match.group(2)] = match.group(1)
    events = list(evidence.rows("campaign-events.jsonl"))
    review = json.loads(Path(review_path).read_text()) if review_path else {}
    review_matches = review.get("source_commit") == source_commit and bool(source_commit)
    review_status = review.get("status", "unreviewed") if review_matches else "unreviewed"
    indexed = [indexed_component_summary(evidence, model, events, stats) for model in ("wac", "acp")]
    global_quarantines = [e for e in events if e.get("record_type") == "correctness-quarantine"]
    global_quarantines += [{"record_type": "correctness-quarantine", "origin": "independent-indexed-analysis", "model": d["model"],
                            "reason": "explicit indexed result mismatch"} for d in indexed if d["correctness_failure"]]
    if review_status == "quarantined" or global_quarantines: review_status = "quarantined"
    # A finalized, checksummed failed campaign can still contain valid earlier
    # cells. Execution failure is distinct from source-correctness quarantine.
    finalized = (Path(root) / "DONE").exists() or (Path(root) / "FAILED").exists()
    verified_integrity = evidence.manifest_status == "verified" and finalized
    inventories, churn = {}, {}
    for dataset in campaign["corpora"]:
        for model in dataset["models"]:
            inventories[(dataset["id"], model)] = inventory_summary(evidence, dataset, model)
            churn[(dataset["id"], model)] = dedicated_churn(evidence, dataset, model)
    dataset_quarantines = {(e.get("dataset"), e.get("model")) for e in events if e.get("record_type") in (
        "correctness-or-admission-quarantine", "admission-quarantine", "dataset-correctness-quarantine", "dataset-not-capacity-eligible")}
    cells = []
    with tempfile.TemporaryDirectory(prefix="sparq-campaign-analysis-") as scratch:
        for index, metadata in enumerate(expected_cells(campaign)):
            label = metadata["label"]
            reported = evidence.document(label + "-summary.json")
            if evidence.locate(label + "-requests.jsonl") is None:
                cells.append({**metadata, "local_guard": "unmeasured", "valid_for_inference": False,
                              "issues": ["raw-request-records-absent"], "reported_summary": reported,
                              "unmeasured_context": [event for event in events if event.get("record_type") in (
                                  "larger-rate-grid-stop", "larger-memory-grid-stop", "dataset-not-capacity-eligible", "campaign-stopped")
                                  and event.get("dataset", metadata["dataset"]) == metadata["dataset"]
                                  and event.get("group", metadata["group"]) == metadata["group"]]})
                continue
            analyzer = RequestAnalysis(Path(scratch) / f"cell-{index}.sqlite", campaign["measurement"])
            try:
                analyzer.audit(evidence.rows(label + "-audit.jsonl"))
                analyzer.requests(evidence.rows(label + "-requests.jsonl"))
                requests = analyzer.result()
            finally:
                analyzer.close()
                (Path(scratch) / f"cell-{index}.sqlite").unlink(missing_ok=True)
            resources = resource_summary(evidence.document(label + "-before-resources.json"),
                                         evidence.document(label + "-after-resources.json"), metadata["memory_gib"])
            reconciliation = evidence.document(label + "-reconciliation.json")
            issues = list(requests["issues"])
            if not reported: issues.append("runner-summary-missing")
            else:
                for name in ("dataset", "group", "memory_gib", "cpus", "rate_override", "replicate", "model", "seed"):
                    if reported.get(name) != metadata[name]: issues.append("runner-metadata-disagrees:" + name)
                for name in ("load_exit_code", "warmup_exit_code", "audit_exit_code"):
                    if reported.get(name) != 0: issues.append(name + "-not-successful")
            if not reconciliation or reconciliation.get("passed") is not True: issues.append("reported-reconciliation-not-passed")
            if not reported or reported.get("audit_exit_code") != 0 or not reconciliation or reconciliation.get("passed") is not True:
                requests["mutation"]["reconciled"] = False
                requests["mutation"]["unknown_resolved_committed"] = None
                requests["mutation"]["unknown_resolved_not_committed"] = None
                requests["mutation"]["issues"] = sorted(set(requests["mutation"]["issues"] + ["audit-or-reconciliation-not-admitted"]))
            if not requests["mutation"]["reconciled"]: issues.extend(requests["mutation"]["issues"])
            inv = inventories[(metadata["dataset"], metadata["model"])]
            if not inv["inventory_consistent"]: issues.extend(inv["issues"])
            actual_rate = requests["offered_rate"]
            expected_rate = metadata["rate_override"] if metadata["rate_override"] != "derived" else requests["load_metadata"]["derived_rate"]
            if not numeric(actual_rate) or not numeric(expected_rate) or actual_rate <= 0 or not math.isclose(actual_rate, expected_rate, rel_tol=1e-12):
                issues.append("raw-offered-rate-disagrees-with-cell")
            if reported and reported.get("offered_rate") != actual_rate:
                issues.append("runner-offered-rate-disagrees-with-raw")
            settings = requests["load_metadata"]["settings"]
            if str(settings.get("seed")) != str(metadata["seed"]): issues.append("raw-schedule-seed-disagrees")
            expected_hash = input_hashes.get("bench/ac/million/workload.json")
            if not expected_hash or requests["load_metadata"]["workload_sha256"] != expected_hash:
                issues.append("raw-workload-hash-unbound-or-disagrees")
            for key in ("pods", "packed_sha256", "index_sha256"):
                if requests["load_metadata"]["corpus"][key] != inv["manifest"].get(key): issues.append("raw-corpus-binding-disagrees:" + key)
            if not churn[(metadata["dataset"], metadata["model"])]["passed"]:
                issues.append("dedicated-policy-checks-not-passed")
            if (metadata["dataset"], metadata["model"]) in dataset_quarantines or (metadata["dataset"], None) in dataset_quarantines:
                issues.append("dataset-quarantined")
            if review_status != "passed": issues.append("source-" + review_status)
            if not verified_integrity: issues.append("artifact-not-complete-checksummed")
            if any(x["file"].startswith(label + "-") for x in evidence.parse_errors):
                issues.append("cell-record-parsing-failed")
            missing_resources = [x for x in resources["issues"] if "missing" in x or "failed" in x or "regression" in x]
            issues.extend(missing_resources)
            valid = not issues and not requests["client_limited"] and not requests["outcomes"].get("client-plan-exhausted", 0)
            if requests["client_limited"]: issues.append("client-limited")
            if requests["outcomes"].get("client-plan-exhausted", 0): issues.append("workload-plan-exhausted")
            m = campaign["measurement"]
            passes = valid and resources["passed"] and requests["queue"]["passed"] and requests["success_fraction_of_offered"] >= m["success_fraction"] and requests["deadline_fraction_of_offered"] >= m["deadline_fraction_of_all_offered"] and requests["within_server_production_deadline"] / requests["offered"] >= m["deadline_fraction_of_all_offered"]
            cells.append({**metadata, "valid_for_inference": valid, "local_guard": "pass" if passes else "fail" if valid else "inconclusive",
                          "issues": sorted(set(issues)), "requests": requests, "resources": resources,
                          "reported_summary": reported, "reported_reconciliation": reconciliation,
                          "scope": "local frozen-workload service guard; not full-service million-history admission"})
    # Errors found while decoding any selected member invalidate the campaign.
    global_parsing = [x for x in evidence.parse_errors if x["file"] in ("campaign.json", "campaign-events.jsonl", "campaign-runtime.json")]
    if evidence.errors or global_parsing:
        for cell in cells:
            if cell["local_guard"] != "unmeasured": cell["local_guard"] = "inconclusive"
            cell["valid_for_inference"] = False
            cell["issues"] = sorted(set(cell["issues"] + ["artifact-integrity-or-decoding-error"]))
    for diagnostic in indexed:
        diagnostic["valid_for_component_inference"] = bool(diagnostic["complete"] and verified_integrity and not evidence.errors and not global_parsing and review_status == "passed")
        diagnostic["source_status"] = review_status
    margin = 1.10
    pairs = pair_tables(cells, margin)
    brackets = bracket_tables(cells, margin)
    return {"schema_version": 1, "analysis_kind": "frozen-campaign-independent-accounting",
            "campaign_id": campaign["campaign_id"], "artifact_directory": str(Path(root).resolve()),
            "source_commit": source_commit, "runtime": runtime, "campaign": campaign, "declared_source_input_sha256": input_hashes,
            "analysis_script_sha256": sha_file(Path(__file__)),
            "analysis_sources_sha256": {p.name: sha_file(p) for p in (Path(__file__), Path(__file__).with_name("indexed_analysis.py"))},
            "source_review": {"scope": "benchmark-method and request-accounting review", "status": review_status,
                              "source_binding_matches": review_matches, "review_sha256": sha_file(Path(review_path)) if review_path else None,
                              "record": review, "quarantine_events": global_quarantines},
            "artifact_integrity": {"manifest_status": evidence.manifest_status, "complete": verified_integrity and not evidence.errors,
                                   "execution_failed_marker": (Path(root) / "FAILED").exists(),
                                   "errors": evidence.errors, "record_parsing_errors": evidence.parse_errors,
                                   "parsed_input_sha256": dict(sorted(evidence.hashes.items()))},
            "corpora": list(inventories.values()), "indexed_component_diagnostics": indexed, "dedicated_policy_checks": list(churn.values()), "events": events,
            "cells": cells, "paired_comparisons": pairs, "capacity_brackets": brackets,
            "full_service_million_history_admitted": False,
            "full_service_scope": "Local cell passes alone do not admit the requested retained-history million-Pod service. Network journeys, complete required operation/resource/scale coverage and a separate final evidence assessment remain necessary.",
            "statistical_plan": {"equivalence_margin_ratio": margin, "ci_level": .95, "bootstrap_resamples": 10000, "bootstrap_seed": 2026090699,
                                 "capacity_bounds": "preserve unsampled rate intervals; bootstrap of equal coarse-grid pass rates cannot remove bracket uncertainty"}}
