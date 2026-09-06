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


def population_rates(per_person, workload):
    rates = []
    for pods in (1000, 10000, 100000, 1000000, 2000000):
        for scenario, multiplier in workload["offered_multiplier_scenarios"].items():
            components = {key: Decimal(pods) * count / 86400 * multiplier
                          for key, count in per_person.items()}
            rates.append({"pods": pods, "scenario": scenario,
                          "components_rps": components, "total_rps": sum(components.values())})
    return rates


def stock_based_writes(workload, calibration_path, inventory_path=None, inventory_manifest_path=None):
    calibration = json.loads(calibration_path.read_text(), parse_float=Decimal)
    execution = workload["execution_v2"]
    horizon = calibration["history_months"]["value"] * execution["retention_days_per_month"]
    sources = {"calibration_sha256": digest(calibration_path)}
    if inventory_path is not None:
        if inventory_manifest_path is None:
            raise ValueError("An inventory manifest is required to establish the retention horizon and Pod count")
        manifest = json.loads(inventory_manifest_path.read_text())
        horizon = manifest["config"]["history_months"] * execution["retention_days_per_month"]
        if horizon <= 0:
            raise ValueError("Inventory retention horizon must be positive")
        rows = [json.loads(line) for line in inventory_path.read_text().splitlines() if line.strip()]
        if not rows or len(rows) != manifest["pods"] or {x["pod_id"] for x in rows} != set(range(manifest["pods"])):
            raise ValueError("Inventory must have exactly one summary for every manifest Pod")
        services = set().union(*(x["records_by_service"] for x in rows))
        stock = {s: sum(Decimal(x["records_by_service"].get(s, 0)) for x in rows) / len(rows)
                 for s in services}
        basis = "observed retained inventory per Pod; rates remain stationary-retention scenario assumptions"
        sources.update(inventory_sha256=digest(inventory_path), inventory_pods=len(rows),
                       inventory_manifest_sha256=digest(inventory_manifest_path))
    else:
        classes = calibration["population_intensity"]["classes"]
        total_weight = sum(x["weight"] for x in classes)
        def expected_scaled(count):
            return sum(Decimal(x["weight"]) * max(1, count * x["numerator"] // x["denominator"])
                       for x in classes) / total_weight if count else Decimal(0)
        stock = {}
        for service, domain in calibration["domains"].items():
            if service == "ratings":
                continue
            count = domain.get("retained_snapshot_records") if service == "contacts" else (
                domain.get("normalized_unit_intensity_rate", domain.get("monthly_records", 0))
                * calibration["history_months"]["value"])
            stock[service] = expected_scaled(count)
        cdf_path = (calibration_path.parent / calibration["domains"]["ratings"]["cdf"]).resolve()
        cdf = json.loads(cdf_path.read_text())
        previous, weighted = 0, 0
        for count, cumulative in cdf:
            if cumulative <= previous:
                raise ValueError("Rating CDF must have strictly increasing cumulative counts")
            weighted += count * (cumulative - previous)
            previous = cumulative
        stock["ratings"] = Decimal(weighted) / previous
        sources["ratings_cdf_sha256"] = digest(cdf_path)
        basis = "theoretical retained-stock mean under declared intensity mixture and empirical rating-count CDF; not a generated population measurement"
    services = []
    for batch in execution["mutation_batches"]:
        service = batch["service"]
        arrivals = stock[service] / horizon
        services.append({"service": service, "retained_records_per_hosted_person": stock[service],
                         "new_records_per_hosted_day": arrivals,
                         "ingest_requests_per_hosted_day": arrivals / batch["ingest_records"],
                         "expire_requests_per_hosted_day": arrivals / batch["expire_records"],
                         "modify_requests_per_hosted_day": arrivals * batch["modifications_per_ingested_record"] / batch["modify_records"]})
    contacts = stock["contacts"] * execution["contacts_modifications_per_retained_record_year"] / execution["days_per_year"]
    totals = {kind: sum(x[kind + "_requests_per_hosted_day"] for x in services)
              for kind in ("ingest", "expire", "modify")}
    totals["modify"] += contacts
    return {"basis": basis, "source_hashes": sources, "retention_days": horizon,
            "services": services, "contacts_modify_requests_per_hosted_day": contacts,
            "per_hosted_person_daily_content_requests": totals,
            "content_write_requests_per_hosted_day": sum(totals.values()),
            "scope": "fixed resource topology; balanced ingestion/expiry in expectation; actual finite-run changes require successful mutation receipts"}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--calibration", type=Path, default=HERE / "corpus-calibration.json")
    parser.add_argument("--inventory", type=Path, help="Optional per-Pod summary JSONL for stock-based rates")
    parser.add_argument("--inventory-manifest", type=Path, help="Matching manifest supplies actual retention horizon and population")
    args = parser.parse_args()
    if (args.inventory is None) != (args.inventory_manifest is None):
        parser.error("--inventory and --inventory-manifest must be provided together")
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
    legacy_per_person = dict(per_person)
    writes = None
    if "execution_v2" in workload:
        writes = stock_based_writes(workload, args.calibration, args.inventory, args.inventory_manifest)
        per_person.update({"background-read": workload["execution_v2"]["background_read_requests_per_hosted_day"],
                           "content-write": writes["content_write_requests_per_hosted_day"],
                           "policy-write": workload["execution_v2"]["policy_attempts_per_hosted_day"]})
    derived = {
        "schema_version": 2,
        "source": "workload.json",
        "source_sha256": digest(workload_path),
        "source_role": "declared scenario, not measured demand",
        "numeric_encoding": "Decimal arithmetic; JSON numbers rounded half-even to nine decimal places",
        "journeys_per_active_person_day": sum(x["actions_per_active_person_day"] for x in workload["journeys"]),
        "per_active_person_daily_foreground_queries": per_active,
        "per_hosted_person_daily_requests": per_person,
        "population_rates": population_rates(per_person, workload),
    }
    if writes is not None:
        derived["stock_based_writes"] = writes
        derived["legacy_pilot_v1"] = {
            "scope": "legacy four-weight approximation and existing-value replacement writes; not the main journey mix",
            "per_hosted_person_daily_requests": legacy_per_person,
            "population_rates": population_rates(legacy_per_person, workload),
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
        "endpoint_ratio_threshold": "/workload/h2_decision_rule/endpoint_ratio_upper_bound",
        "elasticity_threshold": "/workload/h2_decision_rule/pod_elasticity_upper_bound",
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
    evidence["effect_size_summary"] = []
    for lane in ("materialized-routed", "native-http-assembly"):
        for metric in ("wall", "process_cpu"):
            cells = [cell[metric] for domain in baseline["results"]["h2"][lane].values()
                     for cell in domain.values()]
            evidence["effect_size_summary"].append({
                "lane": lane, "metric": metric, "cells": len(cells),
                "source_pointer_pattern": f"/results/h2/{lane}/{{domain}}/{{query}}/{metric}",
                "endpoint_ratio_range": [min(x["median_ratio"] for x in cells),
                                         max(x["median_ratio"] for x in cells)],
                "maximum_endpoint_ratio_ci95_high": max(x["ratio_ci95"]["high"] for x in cells),
                "elasticity_range": [min(x["pod_elasticity"] for x in cells),
                                     max(x["pod_elasticity"] for x in cells)],
                "maximum_elasticity_ci95_high": max(x["elasticity_ci95"]["high"] for x in cells),
            })
    evidence["accompanying_artifacts"] = {
        "directory": str(source.parent.relative_to(ROOT)),
        "raw_archive": baseline["analysis"]["raw_archive"],
        "analysis_commit": baseline["analysis"]["git_commit"],
        "public_archival_availability": "unresolved; accompanying local review bundle only",
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
