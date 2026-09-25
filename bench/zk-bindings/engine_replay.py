#!/usr/bin/env python3
"""[GPT-6] Complete native seed/storage matrices; no builds, installs or proofs."""
import argparse
import hashlib
import itertools
import json
import os
from pathlib import Path
import subprocess

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
MODES = ("dense", "fork", "compressed", "mmap", "mmap-compressed", "dict-spill")
GAPS = {"no-agreement", "adjudicated-count-only", "unclassified-execution"}
MAX_RECORD_BYTES = 16 * 1024 * 1024


class RecordCapacity(ValueError):
    """The controller's own explicit output cap, not an evaluator capacity cause."""


def require(value, message):
    if not value:
        raise ValueError(message)


def digest(path):
    h = hashlib.sha256()
    with Path(path).open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            h.update(block)
    return h.hexdigest()


def jobs(matrix):
    require(matrix["schema"] == "sparq.engine-replay.matrix.v1", "matrix schema")
    axes = [matrix[k] for k in ("profiles", "categories", "seeds", "storage")]
    for axis in axes:
        require(axis and len(axis) == len(set(axis)), "empty or duplicate matrix axis")
    require(tuple(matrix["storage"]) == MODES, "missing storage variant")
    require(all(type(s) is int and 0 <= s < 2**64 for s in matrix["seeds"]), "u64 seed required")
    planned = [{"id": f"{p}-{c}-{s}-{m}", "profile": p, "category": c, "seed": s, "storage": m}
               for p, c, s, m in itertools.product(*axes)]
    require(len(planned) == matrix["configured_native_cells"], "configured matrix denominator differs")
    require(all(all(ch.isalnum() or ch in "-_" for ch in job["id"]) for job in planned), "unsafe cell identifier")
    return planned


def validate_record(job, record, registry_path):
    require(record["schema"] == "sparq.engine-seed-replay.v1", "record schema")
    for key in ("seed", "category", "storage"):
        require(record[key] == job[key], f"wrong {key}")
    require(record["proof_count"] == record["verified_proof_count"] == 0, "native counted as proof")
    require(record["proof_bridge"] == {
        "status": "not-prepared", "backend": None, "authority": None,
        "public_statement_sha256": None, "private_witness_sha256": None, "reuse_allowed": False,
    }, "native result cannot authorize proof reuse")
    require(Path(record["divergence_registry"]["source"]).resolve() == registry_path.resolve(), "wrong oracle registry")
    require(record["input"]["format"] == "turtle" and record["input"]["named_graph_catalog"] == [], "input dataset contract")
    require(isinstance(record["input"]["query"], str) and isinstance(record["input"]["dataset"], str), "missing original input")
    status = record["comparison"]["status"]
    require(status in GAPS | {"agreement", "mismatch"}, "unknown comparison status")
    if record["native_error"] is not None or record["reference_error"] is not None:
        return "unclassified-execution"
    require(record["native_result"] is not None and record["reference_result"] is not None, "missing answer")
    return status


def summarize(plan, outcomes):
    expected = {j["id"]: j for j in plan}
    require(len(expected) == len(plan), "duplicate planned cell")
    actual = {r["id"]: r for r in outcomes}
    require(len(actual) == len(outcomes) and actual.keys() == expected.keys(), "missing, extra or duplicate cell")
    counts = {}
    identities = {}
    for cell in outcomes:
        state = cell["status"]
        counts[state] = counts.get(state, 0) + 1
        if "record" not in cell:
            continue
        record = cell["record"]
        job = expected[cell["id"]]
        key = (job["category"], job["seed"])
        # Identical source/query and raw term observations are prerequisites only.
        # Unordered row permutations, blank labels and valid tie choices may differ;
        # do not coerce them into the same public statement or private witness.
        observed = (record["input"], record["observed_ntriples"], record["native_result"])
        identities.setdefault(key, []).append(observed)
    expected_per_seed = {}
    for job in plan:
        key = (job["category"], job["seed"])
        expected_per_seed[key] = expected_per_seed.get(key, 0) + 1
    identity_equal = sum(len(cells) == expected_per_seed[key] and all(v == cells[0] for v in cells)
                         for key, cells in identities.items())
    input_mismatches = sum(any(v[:2] != cells[0][:2] for v in cells) for cells in identities.values())
    return {
        "configured_native_cells": len(plan), "recorded_native_cells": len(outcomes),
        "comparisons": counts, "raw_identity_equal_seed_cases": identity_equal,
        "raw_identity_observed_seed_cases": len(identities),
        "configured_seed_cases": len(expected_per_seed), "input_identity_mismatches": input_mismatches,
        "complete_oracle_agreement": counts == {"agreement": len(plan)} and input_mismatches == 0,
        "complete_raw_identity": identity_equal == len(expected_per_seed),
        "proof_count": 0, "verified_proof_count": 0, "proof_reuse_authorized": False,
    }


def accepted_binaries(matrix, manifest):
    """Require explicit caller-accepted binary/build records, never ambient PATH."""
    head = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip()
    require(not subprocess.check_output(["git", "status", "--porcelain", "--untracked-files=no"], cwd=ROOT), "tracked source is dirty")
    require(manifest["source_commit"] == head, "accepted binary source differs")
    require(set(manifest["profiles"]) == set(matrix["profiles"]), "missing or extra compiled variant")
    for profile, record in manifest["profiles"].items():
        require(Path(record["binary"]).is_absolute(), "absolute accepted binary required")
        require(digest(record["binary"]) == record["sha256"], "binary identity differs")
        require(digest(record["build_record"]) == record["build_record_sha256"], "build record differs")
        build = json.loads(Path(record["build_record"]).read_text())
        require(build["source_commit"] == head and build["binary_sha256"] == record["sha256"], "unbound build provenance")
        require(build["features"] == matrix["profile_features"][profile], "wrong compiled feature declaration")
    return manifest["profiles"]


def execute(matrix, manifest, output):
    plan = jobs(matrix)
    binaries = accepted_binaries(matrix, manifest)
    output.mkdir(parents=True, exist_ok=False)
    registry = ROOT / "bench/differential-divergences.json"
    registry_hash = digest(registry)
    (output / "plan.json").write_text(json.dumps({"matrix": matrix, "binaries": manifest,
        "registry_sha256": registry_hash, "jobs": plan}, indent=2) + "\n")
    outcomes = []
    # Explicit storage selection; never inherit an original fuzz-mode override.
    env = {k: v for k, v in os.environ.items() if not k.startswith("SPARQ_FUZZ_")}
    env["SPARQ_FUZZ_DIVERGENCES"] = str(registry)
    for job in plan:
        directory = output / job["id"]
        binary = binaries[job["profile"]]
        cell = {"id": job["id"], "status": "infrastructure", "artifacts": {}}
        command = [binary["binary"], "fuzz-replay", str(job["seed"]), job["category"], job["storage"], str(directory)]
        try:
            require(digest(binary["binary"]) == binary["sha256"], "binary changed during replay")
            with (output / (job["id"] + ".stdout")).open("xb") as stdout, (output / (job["id"] + ".stderr")).open("xb") as stderr:
                result = subprocess.run(command, cwd=ROOT, env=env, stdout=stdout, stderr=stderr, timeout=180)
            cell["exit_code"] = result.returncode
            record_path = directory / "record.json"
            if record_path.stat().st_size > MAX_RECORD_BYTES:
                raise RecordCapacity("controller record capacity exceeded")
            record = json.loads(record_path.read_text())
            cell["status"] = validate_record(job, record, registry)
            cell["record"] = record
            require((directory / "query.rq").read_text() == record["input"]["query"], "query artifact differs")
            require((directory / "data.ttl").read_text() == record["input"]["dataset"], "dataset artifact differs")
            cell["artifacts"] = {p.name: digest(p) for p in (record_path, directory / "query.rq", directory / "data.ttl")}
            if result.returncode != 0 and cell["status"] not in {"mismatch", "unclassified-execution"}:
                cell["status"] = "infrastructure"
        except subprocess.TimeoutExpired:
            cell["status"] = "timeout"
        except RecordCapacity as error:
            cell["status"] = "controller-capacity"
            cell["detail"] = str(error)
        except (OSError, ValueError, KeyError, TypeError) as error:
            cell["status"] = "infrastructure"
            cell["detail"] = str(error)
        for suffix in (".stdout", ".stderr"):
            retained = output / (job["id"] + suffix)
            if retained.exists():
                cell["artifacts"][retained.name] = digest(retained)
        outcomes.append(cell)
        with (output / "outcomes.jsonl").open("a") as stream:
            stream.write(json.dumps(cell, sort_keys=True) + "\n")
    require(digest(registry) == registry_hash, "oracle registry changed")
    report = summarize(plan, outcomes)
    (output / "report.json").write_text(json.dumps(report, indent=2) + "\n")
    return report


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--matrix", type=Path, default=HERE / "engine-matrix.json")
    parser.add_argument("--accepted-binaries", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    report = execute(json.loads(args.matrix.read_text()), json.loads(args.accepted_binaries.read_text()), args.output.resolve())
    raise SystemExit(0 if report["complete_oracle_agreement"] else 1)
