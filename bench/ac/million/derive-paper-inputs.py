#!/usr/bin/env python3
"""[GPT-6] Derive manuscript inputs without turning assumptions into measurements."""

import argparse
import hashlib
import json
from decimal import Decimal, ROUND_HALF_EVEN, localcontext
from pathlib import Path


ROOT = Path(__file__).resolve().parents[3]
HERE = Path(__file__).resolve().parent


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def pointer(document, path):
    value = document
    for segment in path.strip("/").split("/"):
        value = value[segment.replace("~1", "/").replace("~0", "~")]
    return value


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--calibration", type=Path, default=HERE / "corpus-calibration.json")
    args = parser.parse_args()
    workload_path = HERE / "workload.json"
    workload = json.loads(workload_path.read_text(), parse_float=Decimal)
    per_active = sum(
        item["actions_per_active_person_day"]
        * item["logical_reads_per_action"]
        * (1 - item["client_cache_hit_fraction"])
        * item["pod_fanout"]
        / item["batch_factor"]
        for item in workload["journeys"]
    )
    active = workload["population"]["daily_active_fraction"]
    background = workload["background"]
    per_person = {
        "query": active * per_active,
        "background-read": background["read_requests_per_hosted_person_day"],
        "content-write": active * background["content_write_requests_per_active_person_day"],
        "policy-write": background["policy_changes_per_hosted_person_day"],
    }
    rates = []
    for pods in (1000, 10000, 100000, 1000000, 2000000):
        for scenario, multiplier in workload["offered_multiplier_scenarios"].items():
            components = {
                key: Decimal(pods) * count / 86400 * multiplier
                for key, count in per_person.items()
            }
            rates.append({"pods": pods, "scenario": scenario,
                          "components_rps": components, "total_rps": sum(components.values())})
    derived = {
        "schema_version": 1,
        "source": "workload.json",
        "source_sha256": digest(workload_path),
        "source_role": "declared scenario, not measured demand",
        "numeric_encoding": "Decimal arithmetic; JSON numbers rounded half-even to nine decimal places",
        "journeys_per_active_person_day": sum(x["actions_per_active_person_day"] for x in workload["journeys"]),
        "per_active_person_daily_foreground_queries": per_active,
        "per_hosted_person_daily_requests": per_person,
        "population_rates": rates,
    }
    def encode_decimal(value):
        if not isinstance(value, Decimal):
            raise TypeError(f"Unsupported JSON value: {type(value)}")
        return float(value.quantize(Decimal("0.000000001"), rounding=ROUND_HALF_EVEN))

    (HERE / "workload-derived.json").write_text(
        json.dumps(derived, indent=2, default=encode_decimal) + "\n"
    )

    source = ROOT / "bench/canonical-competitor-results/ac-sparql/ec2-canonical-20260903t085125z/paper-summary.json"
    baseline = json.loads(source.read_text())
    bindings = {
        "correctness_passed": "/correctness/passed",
        "oracle_comparisons": "/correctness/oracle_comparisons",
        "pod_counts": "/workload/pod_counts",
        "process_blocks": "/workload/process_blocks",
        "timing_repetitions": "/workload/timing_repetitions",
        "routed_pass_count": "/results/h2_rollup/materialized-routed/pass_count",
        "native_pass_count": "/results/h2_rollup/native-http-assembly/pass_count",
        "cells": "/results/h2_rollup/materialized-routed/cells",
    }
    evidence = {
        "schema_version": 1,
        "role": "reused smaller-scale canonical WAC reference, not measurements of the new server",
        "source": str(source.relative_to(ROOT)),
        "source_sha256": digest(source),
        "run_id": baseline["run_id"],
        "source_commit": baseline["source"]["git_commit"],
        "values": {key: {"value": pointer(baseline, path), "pointer": path}
                   for key, path in bindings.items()},
    }
    (ROOT / "research/solid-pod-scale-baseline.json").write_text(json.dumps(evidence, indent=2) + "\n")
    if args.calibration.is_file():
        corpus = {
            "schema_version": 1,
            "source": "bench/ac/million/corpus-calibration.json",
            "source_sha256": digest(args.calibration),
            "role": "source-backed observations and declared corpus assumptions; not measured server performance",
            "calibration": json.loads(args.calibration.read_text()),
        }
        (ROOT / "research/solid-pod-scale-corpus.json").write_text(json.dumps(corpus, indent=2) + "\n")


if __name__ == "__main__":
    with localcontext() as context:
        context.prec = 50
        main()
