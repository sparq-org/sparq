"""[GPT-6] Bounded indexed-storage diagnostics; never HTTP/population capacity."""
from collections import Counter, defaultdict
import math
import re


def number(value):
    return type(value) in (int, float) and math.isfinite(value) and value >= 0


def indexed_component_summary(evidence, model, events, stats):
    stem = "indexed-history8-" + model
    rows = list(evidence.rows(stem + ".jsonl", strict=False))
    selected_events = [e for e in events if e.get("record_type") == "indexed-component-diagnostic" and e.get("model") == model]
    issues, phases, references, comparisons = [], defaultdict(dict), {}, {}
    configurations, completions, pod_completions = [], [], {}
    wrong = any(e.get("classification") == "wrong-result" for e in selected_events)
    for row in rows:
        kind, pod, storage = row.get("record_type"), row.get("pod"), row.get("storage")
        if kind in ("pod-complete", "prepare", "save", "open", "materialize", "query") and (type(pod) is not int or pod < 0):
            issues.append("invalid-pod-identity"); continue
        if kind == "configuration": configurations.append(row)
        elif kind == "diagnostic-complete": completions.append(row)
        elif kind == "pod-complete":
            if pod in pod_completions: issues.append("duplicate-pod-completion")
            pod_completions[pod] = row
        elif kind in ("prepare", "save", "open", "materialize"):
            key = (kind, storage)
            if pod in phases[key]: issues.append("duplicate-phase-record")
            phases[key][pod] = row
        elif kind == "query":
            if storage not in ("memory", "raw", "compressed") or not isinstance(row.get("role"), str) or not isinstance(row.get("query"), str) or type(row.get("rows")) is not int or row["rows"] < 0:
                issues.append("invalid-query-comparison-record"); continue
            key = (pod, row.get("role"), row.get("query"))
            target = references if storage == "memory" else comparisons
            item = key if storage == "memory" else (storage, *key)
            if item in target: issues.append("duplicate-query-record")
            target[item] = row
            if storage == "memory" and row.get("reference") is not True: issues.append("memory-reference-unmarked")
            if storage != "memory" and row.get("exact_result_match") is not True:
                issues.append("indexed-result-match-not-confirmed")
                wrong |= row.get("exact_result_match") is False
    config = configurations[0] if len(configurations) == 1 else {}
    if len(configurations) != 1: issues.append("configuration-missing-or-duplicate")
    if config.get("schema_version") != 1 or config.get("model") != model: issues.append("configuration-schema-or-model-disagrees")
    expected = set(range(config["pods"])) if type(config.get("pods")) is int and 0 < config["pods"] <= 16 else set()
    if not expected: issues.append("invalid-bounded-pod-count")
    for phase in (("prepare", None), ("save", "raw"), ("save", "compressed"), ("open", "raw"), ("open", "compressed"), ("materialize", "memory")):
        if set(phases[phase]) != expected: issues.append("phase-pod-coverage-incomplete:" + str(phase))
    if set(pod_completions) != expected: issues.append("pod-completion-coverage-incomplete")
    if len(selected_events) != 1 or selected_events[0].get("classification") != "complete" or selected_events[0].get("exit_code") != 0:
        issues.append("runner-diagnostic-completion-not-successful")
    counts = Counter()
    for (storage, pod, role, query), row in comparisons.items():
        counts[storage] += row.get("exact_result_match") is True
        reference = references.get((pod, role, query))
        if reference is None: issues.append("comparison-reference-missing")
        elif row.get("rows") != reference.get("rows"):
            issues.append("indexed-reference-row-count-disagrees"); wrong = True
    for storage in ("raw", "compressed"):
        keys = {k[1:] for k in comparisons if k[0] == storage}
        if keys != set(references) or not references: issues.append("comparison-query-coverage-incomplete:" + storage)
    per_pod = Counter(key[1] for key in comparisons)
    for pod, completion in pod_completions.items():
        if completion.get("exact_result_comparisons") != per_pod[pod]: issues.append("pod-comparison-total-disagrees")
    if len(completions) != 1 or completions[0].get("pods") != config.get("pods") or completions[0].get("exact_result_comparisons") != len(comparisons):
        issues.append("diagnostic-comparison-total-disagrees")

    # The file-list hashes are remote observations, not locally archived index files.
    file_groups, seen = defaultdict(lambda: {"files": 0, "logical_bytes": 0}), set()
    for row in evidence.rows(stem + "-files.jsonl"):
        path, size, digest = row.get("path"), row.get("bytes"), row.get("sha256")
        match = re.fullmatch(r"(pod-([0-9]+)-(raw|compressed))/(.+)", path) if isinstance(path, str) else None
        if not match or ".." in path.split("/") or path in seen or type(size) is not int or size < 0 or not isinstance(digest, str) or not re.fullmatch(r"[0-9a-f]{64}", digest):
            issues.append("invalid-or-duplicate-indexed-file-record"); continue
        seen.add(path)
        target = file_groups[(int(match[2]), match[3])]
        target["files"] += 1; target["logical_bytes"] += size
    expected_groups = {(pod, storage) for pod in expected for storage in ("raw", "compressed")}
    if set(file_groups) != expected_groups: issues.append("indexed-file-list-coverage-incomplete")
    for (pod, storage), totals in file_groups.items():
        opened = phases[("open", storage)].get(pod, {}).get("files", {})
        if any(totals[k] != opened.get(k) for k in totals): issues.append("indexed-file-list-disagrees-with-open-snapshot")
    if any(e["file"].startswith(stem) for e in evidence.parse_errors): issues.append("indexed-record-parsing-failed")

    timing_fields = {"prepare": ("generate_ns", "parse_and_index_ns"), "save": ("save_ns",),
                     "open": ("open_and_validation_ns", "constructor_ns", "materialize_ns", "open_to_authorized_ready_ns"),
                     "materialize": ("constructor_ns", "materialize_ns", "constructor_to_authorized_ready_ns")}
    timings = {}
    for (kind, storage), per_pod_rows in phases.items():
        for field in timing_fields[kind]:
            values = [r.get(field) for r in per_pod_rows.values()]
            if not all(number(v) for v in values): issues.append("phase-timer-missing:" + field)
            timings[":".join((kind, storage or "source", field))] = stats(v for v in values if number(v))
    prepare_to_ready = {}
    for pod in expected:
        parse = phases[("prepare", None)].get(pod, {}).get("parse_and_index_ns")
        ready = phases[("materialize", "memory")].get(pod, {}).get("constructor_to_authorized_ready_ns")
        if number(parse) and number(ready): prepare_to_ready[pod] = parse + ready
    timings["parse_plus_memory_authorized_ready_ns"] = stats(prepare_to_ready.values())
    footprint = {}
    for storage in ("raw", "compressed"):
        snapshots = {}
        for stage in ("save", "open"):
            data = list(phases[(stage, storage)].values())
            snapshots[stage] = {}
            for field in ("files", "directories", "logical_bytes", "allocated_bytes"):
                values = [r.get("files", {}).get(field) for r in data]
                snapshots[stage][field] = {"total": sum(values) if len(values) == len(expected) and all(number(v) for v in values) else None,
                                          "per_pod": stats(v for v in values if number(v))}
        footprint[storage] = {"snapshots": snapshots,
                              "final_list_files": sum(v["files"] for (pod, variant), v in file_groups.items() if variant == storage),
                              "final_list_logical_bytes": sum(v["logical_bytes"] for (pod, variant), v in file_groups.items() if variant == storage)}
    return {"model": model, "input_stem": stem, "complete": bool(rows and not issues and not wrong),
            "correctness_failure": wrong, "issues": sorted(set(issues)), "valid_for_component_inference": False,
            "configuration": {**{k: config.get(k) for k in ("schema_version", "model", "pods", "max_pod_bytes", "scope", "order", "results")},
                              "corpus": {k: config.get("config", {}).get(k) for k in ("profile", "history_months", "literal_profile")}},
            "runner_events": selected_events, "completed_pods": sorted(pod_completions),
            "exact_comparisons": {"observed": len(comparisons), "confirmed_matches": sum(counts.values()),
                                  "by_storage": dict(counts), "reference_queries": len(references),
                                  "declared_completion": completions[-1] if completions else None},
            "timing_ns": timings, "footprint": footprint,
            "per_pod": [{"pod": pod, "prepare": phases[("prepare", None)].get(pod),
                         "memory": phases[("materialize", "memory")].get(pod),
                         "open": {s: phases[("open", s)].get(pod) for s in ("raw", "compressed")}} for pod in sorted(expected)],
            "scope": "bounded sequential component experiment, memory then raw then compressed; OS cache uncontrolled after files were generated; no HTTP, million-indexed-capacity, or crash-durability claim",
            "timing_scope": "parse-plus-memory-ready excludes generation/save; open-to-authorized-ready includes existing Graph::open validation and authorization materialization; distributions span different Pods, not independent run repetitions",
            "file_scope": "save/open snapshots measure allocated bytes; final checksummed file list verifies file counts/logical bytes against open snapshots; listed per-file hashes are remote observations, index file contents are not independently rehashed locally"}
