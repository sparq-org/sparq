#!/usr/bin/env python3
"""[GPT-6] Generate and execute bounded synthetic campaigns without pooling contracts."""
import argparse
import hashlib
import json
import math
import os
from pathlib import Path
import platform
import secrets
import shutil
import signal
import subprocess
import sys
import time

SCHEMA = "sparq.workload-campaign.v1"
ROOT = Path(__file__).resolve().parents[3]
QUERY = "SELECT DISTINCT ?name WHERE { ?s <urn:name> ?name . ?s <urn:age> ?age . FILTER(?age >= 18) }"
CONTRACT = {
    "meaning": "selected_successful_support_unsigned_v1",
    "exact_dataset_scope": "not_applicable_no_completeness_claim",
    "query": QUERY,
    "released_terms": [{"name": '"Alice"'}],
    "authority": "synthetic_babyjubjub_issuer_seed_1",
    "signature_suite": "sparq_schnorr_babyjubjub_poseidon2",
    "status_regime": "verifier_owned_snapshot_epoch_7_hidden_reference",
    "capacity_policy": "k1_or_k2_smallest_p3_r4_n16_tiny_integer_status_d10",
    "disclosure_regime": "public_query_result_issuer_capacity_private_roots_and_operands",
}


class StartedProcessError(OSError):
    """A host I/O failure after the sample process was actually created."""


def encoded(value):
    return (json.dumps(value, sort_keys=True, indent=2, allow_nan=False) + "\n").encode()


def digest(data):
    return hashlib.sha256(data).hexdigest()


def file_digest(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def load(path, maximum=2 * 1024 * 1024):
    def unique(pairs):
        result = {}
        for key, value in pairs:
            if key in result:
                raise ValueError(f"duplicate JSON key: {key}")
            result[key] = value
        return result
    with path.open("rb") as stream:
        data = stream.read(maximum + 1)
    if len(data) > maximum:
        raise ValueError("JSON byte capacity exceeded")
    def finite(number):
        value = float(number)
        if not math.isfinite(value):
            raise ValueError("non-finite JSON number")
        return value
    return json.loads(data, object_pairs_hook=unique, parse_float=finite,
                      parse_constant=lambda value: (_ for _ in ()).throw(ValueError(value)))


def save(path, value):
    with path.open("xb") as stream:
        stream.write(encoded(value))


def seed_ok(seed, length=16):
    return isinstance(seed, str) and len(seed) == length and all(c in "0123456789abcdef" for c in seed)


def validate_profile(profile):
    keys = {"schema", "seed", "wallet_scales", "organization_scales", "warmups", "repetitions", "timeout_seconds", "include_unsupported"}
    if not isinstance(profile, dict) or set(profile) != keys or profile["schema"] != SCHEMA or not seed_ok(profile["seed"]):
        raise ValueError("profile schema or seed rejected")
    for field, lower in [("wallet_scales", 3), ("organization_scales", 2)]:
        values = profile[field]
        if not isinstance(values, list) or len(values) > 4 or len(set(values)) != len(values) or any(type(n) is not int or not lower <= n <= 128 for n in values):
            raise ValueError(f"{field} exceeds admitted synthetic capacity; no truncation")
    if not profile["wallet_scales"] and not profile["organization_scales"]:
        raise ValueError("empty campaign")
    for field, low, high in [("warmups", 0, 2), ("repetitions", 1, 5), ("timeout_seconds", 1, 7200)]:
        if type(profile[field]) is not int or not low <= profile[field] <= high:
            raise ValueError(f"invalid {field}")
    if type(profile["include_unsupported"]) is not bool:
        raise ValueError("include_unsupported must be boolean")


def wallet(seed, scale):
    names, ages = [], []
    for subject, name, age in [("alice", "Alice", 42), ("bob", "Bob", 12), ("carla", "Carla", 50)]:
        subject = f"urn:sparq:bench:{seed}:{subject}"
        names.append(dict(subject=subject, predicate="urn:name", text=name, integer=None))
        ages.append(dict(subject=subject, predicate="urn:age", text=None, integer=age))
    credentials = [names, ages]
    for index in range(2, scale):
        credentials.append(names + ages + [dict(subject=f"urn:sparq:bench:{seed}:credential:{index}", predicate="urn:padding", text="synthetic", integer=None)])
    return dict(seed=seed, scale=scale, credentials=credentials)


def organization(seed, scale):
    namespace = f"urn:sparq:bench:{seed}:"
    graphs = [namespace + name for name in ["department-a", "department-b", "empty"]]
    nquads = "".join(f"<{namespace}member-{department}-{index}> <urn:member> <urn:Employee> <{graphs[department]}> .\n"
                     for department in range(2) for index in range(scale))
    clauses = "".join(f"FROM NAMED <{graph}> " for graph in graphs)
    query = f"SELECT ?g (COUNT(?s) AS ?n) {clauses}WHERE {{ GRAPH ?g {{ OPTIONAL {{ ?s <urn:member> ?kind }} }} }} GROUP BY ?g ORDER BY ?g"
    # Closed-form oracle: each occupied graph has exactly `scale` distinct subjects;
    # OPTIONAL keeps the empty graph's unbound row and COUNT(?s) is zero there.
    expected = {"Select": {"variables": ["g", "n"], "order": "Sequence", "rows": [
        [f"<{graph}>", f'"{0 if index == 2 else scale}"^^<http://www.w3.org/2001/XMLSchema#integer>']
        for index, graph in enumerate(graphs)]}}
    return dict(seed=seed, scale=scale, nquads=nquads, named_graphs=graphs, query=query, expected=expected)


def build_plan(profile):
    validate_profile(profile)
    inputs, cells = {}, []
    for family, generate in [("wallet", wallet), ("organization", organization)]:
        for scale in profile[family + "_scales"]:
            name = f"{family}-{scale:04}"
            fixture = generate(profile["seed"], scale)
            inputs[name] = fixture
            cells.append((name, family, scale, digest(encoded(fixture))))
    jobs = []
    for round_index in range(profile["warmups"] + profile["repetitions"]):
        role = "warmup" if round_index < profile["warmups"] else "measurement"
        rotated = cells[round_index % len(cells):] + cells[:round_index % len(cells)]
        for name, family, scale, sha in rotated:
            variants = ["selected:first_success", "selected:optimize"] if family == "wallet" else ["exact:verifier_agreed", "exact:holder_declared"]
            if round_index % 2:
                variants.reverse()
            if profile["include_unsupported"]:
                variants += ["exact:wallet"] if family == "wallet" else ["selected:organization"]
            for variant in variants:
                backend, mode = variant.split(":")
                unsupported = mode in {"wallet", "organization"}
                jobs.append({"ordinal": len(jobs), "round": round_index, "role": role,
                             "family": family, "scale": scale, "fixture": name,
                             "fixture_sha256": sha, "backend": backend, "mode": mode,
                             "unsupported": unsupported,
                             "unsupported_reason": ("profile has no matched adapter statement; credential authentication and exact-dataset authority are different contracts" if unsupported else None),
                             "comparison_pair": f"{name}:round:{round_index}" if family == "wallet" and not unsupported else None})
    return {"schema": SCHEMA, "profile": profile, "jobs": jobs}, inputs


def generate(profile_path, output):
    plan, inputs = build_plan(load(profile_path))
    output.mkdir()
    (output / "inputs").mkdir()
    for name, fixture in inputs.items():
        save(output / "inputs" / f"{name}.json", fixture)
    save(output / "plan.json", plan)
    return plan


def read_generated(directory):
    plan = load(directory / "plan.json")
    expected, inputs = build_plan(plan["profile"])
    if plan != expected:
        raise ValueError("generated schedule changed; regenerate from the reviewed profile")
    for name, fixture in inputs.items():
        path = directory / "inputs" / f"{name}.json"
        expected_bytes = encoded(fixture)
        if path.is_symlink():
            raise ValueError("materialized fixture changed")
        with path.open("rb") as stream:
            if stream.read(len(expected_bytes) + 1) != expected_bytes:
                raise ValueError("materialized fixture changed")
    return plan, inputs


def manifest(job, fixture, run_id):
    challenge = hashlib.sha256(b"sparq-workload-campaign-nonce-v1\0" + bytes.fromhex(run_id) + job["ordinal"].to_bytes(8, "little")).digest()
    if job["backend"] == "selected":
        # High-bit-set values cannot coincide with the adapter's fixed tamper nonce.
        nonce_id = int.from_bytes(challenge[:8], "little") | (1 << 63)
        return dict(schema_version=2, fixture="wallet_candidate_sweep_v1", contract=CONTRACT,
                    planners=[job["mode"]], warmup_runs_per_planner=0, measured_runs_per_planner=1,
                    wallet=fixture, nonce_id=nonce_id)
    return dict(schema_version=2, run_id=challenge[:16].hex(), repetitions=1, warmups=0,
                cases=[dict(fixture="organization_named_count", authority=job["mode"], organization=fixture)])


def inspect_report(report, job, expected_manifest):
    """Require measured success and typed controls; exit zero alone is insufficient."""
    if job["backend"] == "selected":
        if report.get("schema_version") != 2 or report.get("status") != "success" or report.get("experiment") != expected_manifest:
            raise ValueError("selected adapter report/manifest not successful or bound")
        runs = report.get("runs", [])
        if len(runs) != 1 or runs[0].get("status") != "success" or runs[0].get("planner") != job["mode"]:
            raise ValueError("selected sample inventory rejected")
        sample = runs[0]
        controls = sample.get("tamper_controls", {})
        if set(controls) != {"proof_bytes", "released_term", "challenge_both_sides", "requested_query", "accepted_status_snapshot"} or any(value.get("status") != "rejected_as_expected" for value in controls.values()) or sample.get("replay_control", {}).get("status") != "rejected_as_expected":
            raise ValueError("selected tamper/replay evidence incomplete")
        return report["acceptance_contract_blake3"], sample
    if report.get("schema") != "sparq.exact-evaluator-experiment.v1" or report.get("complete") is not True or report.get("run_id") != expected_manifest["run_id"] or report.get("manifest_sha256") != digest(encoded(expected_manifest)):
        raise ValueError("exact adapter report not complete or bound")
    samples = report.get("samples", [])
    if len(samples) != 1 or samples[0].get("status") != "proved_and_independently_verified" or samples[0].get("receipt_kind") != "Succinct":
        raise ValueError("exact receipt sample inventory rejected")
    sample = samples[0]
    if sample.get("result") != expected_manifest["cases"][0]["organization"]["expected"] or sample.get("semantic_contract", {}).get("authority_mode") != job["mode"]:
        raise ValueError("exact golden or authority changed")
    expected_controls = {"expected_query_bytes", "expected_nonce", "expected_authority", "expected_dataset_root", "consumed_nonce_replay", "authenticated_returned_result", "authenticated_dataset_root", "succinct_seal_bytes"}
    if set(sample.get("typed_acceptance_controls_passed", [])) != expected_controls or len(sample["typed_acceptance_controls_passed"]) != 8 or set(report.get("artifact_acceptance_controls_passed", [])) != {"artifact_bytes_digest", "independent_pin_digest", "independent_pin_image_id"}:
        raise ValueError("exact typed controls incomplete")
    return sample["semantic_contract_sha256"], sample


def invoke(argv, cwd, stdout_path, stderr_path, timeout, env):
    start = time.monotonic_ns()
    with stdout_path.open("xb") as stdout, stderr_path.open("xb") as stderr:
        child = subprocess.Popen(argv, cwd=cwd, stdout=stdout, stderr=stderr, env=env, start_new_session=True)
        code = None
        try:
            while True:
                remaining = timeout - (time.monotonic_ns() - start) / 1e9
                if remaining <= 0:
                    status = "timeout"
                    break
                if stdout_path.stat().st_size + stderr_path.stat().st_size > 16 * 1024 * 1024:
                    status = "output_capacity"
                    break
                try:
                    code = child.wait(timeout=min(remaining, 30))
                    # A successful exit can race the last polling observation.
                    status = "output_capacity" if stdout_path.stat().st_size + stderr_path.stat().st_size > 16 * 1024 * 1024 else "exited"
                    break
                except subprocess.TimeoutExpired:
                    print("local adapter sample remains active; no result inferred", flush=True)
            if status != "exited" and code is None:
                # Only the new child process group we own, including prover children.
                try:
                    os.killpg(child.pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass
                child.wait()
                code = child.returncode
        except BaseException as error:
            try:
                os.killpg(child.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            child.wait()
            if isinstance(error, OSError):
                raise StartedProcessError(str(error)) from error
            raise
    return dict(status=status, exit_code=code, inclusive_process_ns=time.monotonic_ns() - start)


def run(directory, output, tools, run_id):
    if os.name != "posix":
        raise ValueError("current local backend adapters require POSIX")
    if not seed_ok(run_id, 32):
        raise ValueError("run_id must be 32 lowercase hexadecimal characters")
    plan, inputs = read_generated(directory)
    output = output.absolute()
    if output.resolve().is_relative_to(ROOT):
        raise ValueError("campaign outputs must be outside the source checkout")
    identities = {}
    for name, path in tools.items():
        if path is not None:
            path = path.resolve(strict=True)
            if not path.is_file():
                raise ValueError(f"{name} must be a regular file")
            tools[name] = path
            identities[name] = {"path": str(path), "sha256": file_digest(path), "is_build_attestation": False}
    for name in ["nargo", "bb"]:
        if tools.get(name) is not None and tools[name].name != name:
            raise ValueError(f"{name} path must have that basename for the existing driver lookup")
    output.mkdir()
    save(output / "plan.json", plan)
    save(output / "invocation.json", {"run_id": run_id, "tools": identities, "python": sys.version,
                                     "platform": platform.platform(), "runner_sha256": file_digest(Path(__file__)),
                                     "build_attestation": None, "peak_process_tree_rss_bytes": None})
    env = os.environ.copy()
    if tools.get("nargo") is not None and tools.get("bb") is not None:
        env["PATH"] = os.pathsep.join([str(tools["nargo"].parent), str(tools["bb"].parent), env.get("PATH", "")])
    for name in ["nargo", "bb"]:
        if tools.get(name) is not None and Path(shutil.which(name, path=env["PATH"]) or "").resolve() != tools[name]:
            raise ValueError(f"PATH would shadow the explicitly supplied {name} executable")
    records, warmups, paired, challenges = [], {}, {}, set()
    for job in plan["jobs"]:
        sample_path = output / f"sample-{job['ordinal']:04}"
        sample_path.mkdir()
        record = dict(job, run_id=run_id, status="failure", subprocess_started=False,
                      completed_disk_backend_warmups=warmups.get((job["fixture"], job["mode"]), 0),
                      in_process_warmup=False, process_tree_peak_rss_bytes=None)
        try:
            if job["unsupported"]:
                record["status"] = "unsupported"
            else:
                required = ["selected", "nargo", "bb"] if job["backend"] == "selected" else ["exact", "artifact", "pin", "r0vm"]
                if any(tools.get(name) is None for name in required):
                    raise ValueError("required supplied tool/artifact missing: " + ",".join(name for name in required if tools.get(name) is None))
                generated = manifest(job, inputs[job["fixture"]], run_id)
                request = sample_path / "manifest.json"
                challenge = generated.get("nonce_id", generated.get("run_id"))
                if (job["backend"], challenge) in challenges:
                    raise ValueError("duplicate synthetic challenge in schedule")
                challenges.add((job["backend"], challenge))
                save(request, generated)
                record["manifest_sha256"] = file_digest(request)
                adapter_output = sample_path / "adapter"
                argv = [str(tools[job["backend"]]), str(request)]
                if job["backend"] == "exact":
                    argv += [str(tools[name]) for name in ["artifact", "pin", "r0vm"]]
                argv.append(str(adapter_output))
                record["argv"] = argv
                record["process"] = invoke(argv, ROOT, sample_path / "stdout.txt", sample_path / "stderr.txt", plan["profile"]["timeout_seconds"], env)
                record["subprocess_started"] = True
                raw = adapter_output / "report.json"
                if raw.exists():
                    record["adapter_report_sha256"] = file_digest(raw)
                record["raw_process_outputs"] = [{"path": name, "sha256": file_digest(sample_path / name), "bytes": (sample_path / name).stat().st_size} for name in ["stdout.txt", "stderr.txt"]]
                if record["process"]["status"] in {"timeout", "output_capacity"}:
                    record["status"] = record["process"]["status"]
                elif record["process"]["exit_code"] != 0:
                    record["status"] = "adapter_failure"
                else:
                    report = load(raw, 8 * 1024 * 1024)
                    contract, sample = inspect_report(report, job, generated)
                    if job["comparison_pair"]:
                        previous = paired.setdefault(job["comparison_pair"], contract)
                        if previous != contract:
                            raise ValueError("paired planners changed acceptance contract")
                    if any(file_digest(path) != identities[name]["sha256"] for name, path in tools.items() if path is not None):
                        raise ValueError("supplied tool or artifact changed during campaign")
                    record.update(status="success", adapter_contract_digest=contract,
                                  sample=sample, artifacts=[{"path": str(p.relative_to(sample_path)), "sha256": file_digest(p), "bytes": p.stat().st_size}
                                                           for p in sorted(adapter_output.rglob("*")) if p.is_file()])
                    if job["role"] == "warmup":
                        key = job["fixture"], job["mode"]
                        warmups[key] = warmups.get(key, 0) + 1
        except (OSError, ValueError, KeyError, TypeError, AttributeError, IndexError) as error:
            if isinstance(error, StartedProcessError):
                record["subprocess_started"] = True
            record["error"] = str(error)
        save(sample_path / "record.json", record)
        records.append(record)
        print(f"sample {job['ordinal']} {job['family']} {job['scale']} {job['mode']}: {record['status']}", flush=True)
    counts = {status: sum(r["status"] == status for r in records) for status in sorted({r["status"] for r in records})}
    success = all(r["status"] in {"success", "unsupported"} for r in records)
    save(output / "report.json", {"schema": SCHEMA, "schedule_complete": len(records) == len(plan["jobs"]),
                                  "all_supported_samples_succeeded": success, "run_id": run_id, "counts": counts,
                                  "canonical_performance": False, "cross_backend_speedup": None,
                                  "timing_contract": "per-sample child process is inclusive and not additive with adapter stages; warmups affect only retained disk/backend/OS caches, never a new process's in-memory state",
                                  "comparison_contract": "paired wallet planners share input/query/answer/issuer/status/capacity policy; realized K1/K2 public capacity differs. Exact organization authority modes are separate contracts. No cross-backend aggregate.",
                                  "records": records})
    return success


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    generator = commands.add_parser("generate")
    generator.add_argument("profile", type=Path)
    generator.add_argument("output", type=Path)
    executor = commands.add_parser("run")
    executor.add_argument("generated", type=Path)
    executor.add_argument("output", type=Path)
    executor.add_argument("--run-id", default=None)
    for name in ["selected", "exact", "nargo", "bb", "artifact", "pin", "r0vm"]:
        executor.add_argument("--" + name, type=Path)
    args = parser.parse_args()
    try:
        if args.command == "generate":
            plan = generate(args.profile, args.output)
            print(f"generated {len(plan['jobs'])} scheduled records; no measurements executed")
            return 0
        return 0 if run(args.generated, args.output, {name: getattr(args, name) for name in ["selected", "exact", "nargo", "bb", "artifact", "pin", "r0vm"]}, args.run_id or secrets.token_hex(16)) else 1
    except (OSError, ValueError, KeyError, TypeError) as error:
        parser.exit(2, f"campaign rejected: {error}\n")


if __name__ == "__main__":
    raise SystemExit(main())
