#!/usr/bin/env python3
"""[GPT-6] Required original-fixture replay with actual typed model causes."""
import argparse
import json
import os
from pathlib import Path
import subprocess

from corpus import digest, import_regressions, load, plan
from run import file_hash, run, write

ROOT = Path(__file__).resolve().parents[2]
BACKENDS = ("exact_v1", "exact_v2", "exact_v3")


def original_plan(root=ROOT):
    inventory = load(root / "bench/zk-bindings/exact-originals.json")
    cases = []
    for source in [*inventory["inputs"], inventory["data"]]:
        if file_hash(root / source["path"]) != source["sha256"]:
            raise ValueError("original fixture hash changed; review the inventory explicitly")
    for source in inventory["inputs"]:
        imported = import_regressions(root / source["path"], source["variables"])
        if (len(imported) != source["cases"]
                or sum(c["rejection"] for c in imported) != source["original_negative_cases"]):
            raise ValueError("original fixture denominator changed")
        cases.extend(imported)
    if len(cases) != inventory["cases"]:
        raise ValueError("missing original cases")
    by_id = {case["id"]:case for case in cases}
    for promotion in inventory["historical_promotions"]:
        if by_id[promotion["current_id"]]["query"] != promotion["original_fixture"]["query"]:
            raise ValueError("historical promotion changed the original query")
    result = plan(cases, BACKENDS, "native", coverage="mandatory_exact_originals_v1")
    if result["classifications"]:
        raise ValueError("unclassified original case cannot skip required replay")
    for backend, totals in result["totals"].items():
        if (totals["configured_jobs"] != inventory["expected_jobs_per_backend"][backend]
                or totals["positive_bindings_or_results"] != inventory["expected_positives_per_backend"][backend]):
            raise ValueError("versioned expectation denominator changed")
    return result


def check_report(report, manifest):
    if (not report.get("passed") or not report.get("complete_declared_domain")
            or report.get("coverage_gaps") or report.get("plan_sha256") != digest(manifest)
            or set(report["totals"]) != set(BACKENDS)):
        raise ValueError("required original replay incomplete")
    records = report["records"]
    ids = [r["job_id"] for r in records]
    expected_ids = {j["id"] for j in manifest["jobs"]}
    if len(ids) != len(expected_ids) or set(ids) != expected_ids:
        raise ValueError("missing or duplicate original job")
    for backend, expected in manifest["totals"].items():
        got = report["totals"][backend]
        if any(got[key] != expected["configured_jobs"] for key in
               ("configured_jobs", "shard_jobs", "executed_jobs", "passed_jobs")):
            raise ValueError("original replay denominator mismatch")
        if got["genuine_proofs"] != 0 or got["verified_proofs"] != 0:
            raise ValueError("native replay cannot count cryptographic proofs")
        if got["negative_stages"] != {"native":expected["negative_bindings_or_admissions"]}:
            raise ValueError("wrong original negative inventory")
    if any(r["status"] != "passed" for r in records):
        raise ValueError("failed original job")


def compiled_adapter(output):
    command = ["cargo", "build", "--locked", "--manifest-path", "zk/sparql-evaluator/Cargo.toml",
               "-p", "sparq-proved-evaluator-model", "--features", "graph-results",
               "--example", "proof_bindings", "--message-format=json"]
    write(output / "build.json", {"argv":command, "cwd":str(ROOT),
          "environment":{key:os.environ.get(key) for key in
                         ("CARGO_TARGET_DIR", "CARGO_BUILD_JOBS", "CARGO_INCREMENTAL", "RUSTFLAGS")}})
    with (output / "cargo.jsonl").open("xb") as stdout, (output / "cargo.stderr").open("xb") as stderr:
        subprocess.run(command, cwd=ROOT, check=True, stdout=stdout, stderr=stderr, timeout=600)
    executables = []
    for line in (output / "cargo.jsonl").read_text().splitlines():
        event = json.loads(line)
        if (event.get("reason") == "compiler-artifact" and event.get("executable")
                and event["target"]["name"] == "proof_bindings"):
            executables.append(Path(event["executable"]).resolve())
    if len(executables) != 1:
        raise ValueError("expected one actual Cargo-produced adapter")
    return executables[0]


def campaign(output):
    output.mkdir(parents=True, exist_ok=False)
    if subprocess.check_output(["git", "status", "--porcelain"], cwd=ROOT):
        raise ValueError("adapter must be built from a clean committed checkout")
    head = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip()
    event_path = os.environ.get("GITHUB_EVENT_PATH")
    event = load(event_path) if event_path else {}
    context = {"checkout_sha":head, "github_sha":os.environ.get("GITHUB_SHA"),
               "pr_head_sha":event.get("pull_request", {}).get("head", {}).get("sha"),
               "pr_base_sha":event.get("pull_request", {}).get("base", {}).get("sha"),
               "merge_group_head_sha":event.get("merge_group", {}).get("head_sha"),
               "model_lock_sha256":file_hash(ROOT / "zk/sparql-evaluator/Cargo.lock")}
    write(output / "checkout.json", context)
    manifest = original_plan()
    write(output / "plan.json", manifest)
    executable = compiled_adapter(output)
    adapter = {"argv":[str(executable)], "executable_sha256":file_hash(executable),
               "checkout":str(ROOT), "source_commit":head}
    write(output / "adapters.json", {backend:adapter for backend in BACKENDS})
    if not run(output / "plan.json", output / "adapters.json", output / "actual", 30, 1024*1024):
        raise ValueError("required original replay failed")
    check_report(load(output / "actual/report.json"), manifest)
    write(output / "completion.json", {"schema":"sparq.exact-original-replay.v1", "passed":True,
          "checkout":context, "original_cases":manifest["case_count"], "native_jobs":len(manifest["jobs"]),
          "proofs":0, "claim":"Native typed evaluation only; genuine guest campaigns are a separate required step."})


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    campaign(parser.parse_args().output.resolve())
