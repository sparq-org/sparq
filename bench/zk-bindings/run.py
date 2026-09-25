#!/usr/bin/env python3
"""[GPT-6] Fail-closed proof corpus planning, execution, retention and replay."""
import argparse
from collections import Counter
import hashlib
import itertools
import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import time

from corpus import BACKENDS, digest, encoded, exhaustive, finite_proof_universe, import_regressions, load, plan, sampled


def write(path, value):
    with Path(path).open("xb") as stream:
        stream.write(encoded(value))


def file_hash(path):
    with Path(path).open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def equal_result(expected, actual, permutation_budget=40_320):
    """Exact terms and one global blank-node bijection; bag multiplicity survives."""
    if expected == actual:
        return True
    if not isinstance(expected, dict) or not isinstance(actual, dict) or set(expected) != set(actual):
        return False
    if "Select" not in expected:
        # Graph fixtures require independent RDF parsing/isomorphism in the
        # existing conformance comparator, not line splitting or per-row labels.
        if "Graph" in expected:
            raise ValueError("graph comparison requires the existing RDF comparator adapter")
        return False
    exp, act = expected["Select"], actual["Select"]
    if (set(exp["variables"]) != set(act["variables"]) or exp["order"] != act["order"]
            or len(exp["rows"]) != len(act["rows"])):
        return False
    width = len(exp["variables"])
    if width != len(set(exp["variables"])) or len(act["variables"]) != width:
        raise ValueError("duplicate result variables")
    if any(len(r) != width for r in exp["rows"] + act["rows"]):
        raise ValueError("result row width differs from projection")
    def blanks(rows):
        return sorted({t for r in rows for t in r if isinstance(t, str) and t.startswith("_:")})
    a, b = blanks(exp["rows"]), blanks(act["rows"])
    if len(a) != len(b):
        return False
    work = 1
    for n in range(1, len(a) + 1):
        work *= n
        if work > permutation_budget:
            raise ValueError("independent result isomorphism capacity exceeded")
    positions = [act["variables"].index(v) for v in exp["variables"]]
    target = [tuple(r[i] for i in positions) for r in act["rows"]]
    for permutation in itertools.permutations(b):
        mapping = dict(zip(a, permutation))
        rows = [tuple(mapping.get(t, t) for t in r) for r in exp["rows"]]
        if (rows == target if exp["order"] == "Sequence" else Counter(rows) == Counter(target)):
            return True
    return False


def check_outcome(job, outcome, output):
    for field, expected in [("schema", "sparq.proof-binding-outcome.v1"), ("job_id", job["id"]),
                            ("case_sha256", job["case_sha256"]), ("backend", job["backend"]), ("tier", job["tier"])]:
        if outcome.get(field) != expected:
            raise ValueError(f"outcome {field} not bound to job")
    if outcome.get("observed") != ("accepted" if job["expected_accept"] else "rejected"):
        raise ValueError("wrong acceptance, or unsupported/missing adapter masquerading as success")
    stage = outcome.get("stage")
    if stage not in ("native", "support", "admission", "constraint", "proof_verifier"):
        raise ValueError("unclassified rejection stage")
    proofs, verified = outcome.get("proof_count"), outcome.get("verified_count")
    if type(proofs) is not int or type(verified) is not int or not 0 <= verified <= proofs:
        raise ValueError("invalid actual proof inventory")
    if job["tier"] != "real" and (proofs or verified):
        raise ValueError("non-cryptographic lane counted proofs")
    if job["tier"] == "real" and job["expected_accept"] and (stage != "proof_verifier" or not verified):
        raise ValueError("real positive lacks genuine proving and independent verification")
    if not job["expected_accept"] and not outcome.get("error_class"):
        raise ValueError("untyped rejection")
    if job["operation"] == "result" and job["expected_accept"]:
        if not equal_result(job["expected_result"], outcome.get("result")):
            raise ValueError("independent full-result oracle mismatch")
    artifacts = outcome.get("artifacts")
    if not isinstance(artifacts, list) or (proofs and not artifacts):
        raise ValueError("missing retained proof artifacts")
    for artifact in artifacts:
        path = (output / artifact["path"]).resolve()
        if not path.is_relative_to(output.resolve()) or not path.is_file():
            raise ValueError("artifact escaped output or is missing")
        if file_hash(path) != artifact["sha256"]:
            raise ValueError("artifact hash differs from recorded proof")
    if not isinstance(outcome.get("controls"), list):
        raise ValueError("missing control inventory")
    if job["operation"] == "attack" and stage != "constraint":
        raise ValueError("malicious-witness job stopped at an honest API wrapper")
    return {"proofs": proofs, "verified": verified, "stage": stage}


def execute_child(argv, directory, timeout, output_limit, env=None):
    """Retain logs; resource failures never become successful negatives."""
    started = time.monotonic()
    with (directory / "stdout.txt").open("xb") as stdout, (directory / "stderr.txt").open("xb") as stderr:
        child = subprocess.Popen(argv, stdout=stdout, stderr=stderr, start_new_session=True, env=env)
        reason = None
        try:
            while child.poll() is None:
                elapsed = time.monotonic() - started
                if elapsed > timeout:
                    reason = "timeout"
                if sum(p.stat().st_size for p in (directory / "stdout.txt", directory / "stderr.txt")) > output_limit:
                    reason = "output_capacity"
                if reason:
                    os.killpg(child.pid, signal.SIGKILL)
                    break
                time.sleep(0.05)
            code = child.wait()
        finally:
            if child.poll() is None:
                os.killpg(child.pid, signal.SIGKILL)
                child.wait()
    if sum(p.stat().st_size for p in (directory / "stdout.txt", directory / "stderr.txt")) > output_limit:
        reason = "output_capacity"
    if reason or code != 0:
        raise RuntimeError(f"adapter infrastructure/command failure: {reason or code}")


def adapter_identity(adapter):
    argv = adapter.get("argv")
    if not isinstance(argv, list) or not argv or any(not isinstance(a, str) or not a for a in argv):
        raise ValueError("adapter argv required")
    executable = Path(argv[0]).resolve()
    if not executable.is_file() or file_hash(executable) != adapter["executable_sha256"]:
        raise ValueError("adapter executable identity mismatch")
    checkout = Path(adapter["checkout"]).resolve()
    current = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=checkout, text=True).strip()
    if current != adapter["source_commit"]:
        raise ValueError("adapter source commit mismatch")
    if subprocess.check_output(["git", "status", "--porcelain", "--untracked-files=no"], cwd=checkout):
        raise ValueError("adapter source checkout is dirty")
    for name, pin in adapter.get("tools", {}).items():
        path = Path(pin["path"]).resolve()
        if path.name != name or file_hash(path) != pin["sha256"]:
            raise ValueError("proof tool binary identity mismatch")
        version = subprocess.run([str(path), "--version"], capture_output=True, timeout=30, check=True)
        if version.stdout.decode() != pin["version_stdout"] or version.stderr.decode() != pin.get("version_stderr", ""):
            raise ValueError("proof tool version output mismatch")
    return adapter | {"argv": [str(executable), *argv[1:]]}


def validate_plan(manifest):
    if manifest.get("schema") != "sparq.proof-bindings.plan.v1" or not manifest.get("jobs"):
        raise ValueError("empty or unknown replay plan")
    rebuilt = plan(manifest["cases"], list(manifest["totals"]), manifest["tier"],
                   manifest["shard"], manifest["shards"], manifest["coverage"])
    if rebuilt != manifest:
        raise ValueError("plan differs from its retained corpus, oracle jobs or complete denominators")


def run(plan_path, adapters_path, output, timeout, output_limit):
    manifest, adapters = load(plan_path), load(adapters_path)
    validate_plan(manifest)
    identities = {b: adapter_identity(adapters[b]) for b in manifest["totals"]}
    if manifest["tier"] != "native":
        for backend, adapter in identities.items():
            if backend.startswith("noir_") and set(adapter.get("tools", {})) != {"nargo", "bb"}:
                raise ValueError("Noir execution requires exact binary hashes and full version outputs")
    if len({job["id"] for job in manifest["jobs"]}) != len(manifest["jobs"]):
        raise ValueError("duplicate planned job")
    output.mkdir(parents=True, exist_ok=False)
    write(output / "plan.json", manifest)
    write(output / "adapters.json", identities)
    records, failure = [], False
    for ordinal, job in enumerate(manifest["jobs"]):
        directory = output / f"{ordinal:06}-{job['id'][:12]}"
        directory.mkdir()
        input_path = directory / "job.json"
        write(input_path, job)
        record = {"job_id": job["id"], "backend": job["backend"], "input_sha256": file_hash(input_path),
                  "expected_accept": job["expected_accept"], "status": "error"}
        try:
            result_dir = directory / "adapter"
            adapter = identities[job["backend"]]
            argv = adapter["argv"]
            env = None
            if adapter.get("kind") == "rust_test":
                env = os.environ | {"SPARQ_PROOF_BINDING_JOB": str(input_path),
                                    "SPARQ_PROOF_BINDING_OUTPUT": str(result_dir)}
            else:
                argv = [*argv, str(input_path), str(result_dir)]
            if adapter.get("tools"):
                tool_path = os.pathsep.join(str(Path(t["path"]).resolve().parent) for t in adapter["tools"].values())
                env = (env or dict(os.environ)) | {"PATH": tool_path + os.pathsep + os.environ.get("PATH", "")}
            execute_child(argv, directory, timeout, output_limit, env)
            outcome = load(result_dir / "outcome.json")
            inventory = check_outcome(job, outcome, result_dir)
            record.update(status="passed", inventory=inventory, outcome=outcome)
        except (OSError, ValueError, KeyError, TypeError, RuntimeError) as error:
            failure = True
            record["error"] = str(error)
        records.append(record)
        write(directory / "record.json", record)
        print(f"{ordinal + 1}/{len(manifest['jobs'])} {job['backend']} {record['status']}", flush=True)
    totals = {}
    for backend, configured in manifest["totals"].items():
        subset = [r for r in records if r["backend"] == backend]
        totals[backend] = configured | {"executed_jobs": len(subset), "passed_jobs": sum(r["status"] == "passed" for r in subset),
                                       "genuine_proofs": sum(r.get("inventory", {}).get("proofs", 0) for r in subset),
                                       "verified_proofs": sum(r.get("inventory", {}).get("verified", 0) for r in subset),
                                       "negative_stages": dict(Counter(r["inventory"]["stage"] for r in subset
                                                                       if not r["expected_accept"] and "inventory" in r))}
    gaps = [c for c in manifest["classifications"] if c["status"] != "excluded"]
    write(output / "report.json", {"schema": "sparq.proof-bindings.report.v1", "plan_sha256": digest(manifest),
                                  "coverage": manifest["coverage"], "shard": manifest["shard"], "shards": manifest["shards"],
                                  "totals": totals, "coverage_gaps": gaps, "records": records,
                                  "passed": not failure and not gaps,
                                  "complete_declared_domain":not failure and not gaps and manifest["shards"] == 1,
                                  "all_backend_profiles_complete":False,
                                  "required_suite_inventory":"bench/zk-bindings/inventory.json",
                                  "claim": "These executed cells only; selected support does not establish completeness. A shard is not the complete declared domain."})
    return not failure and not gaps


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="command", required=True)
    make = sub.add_parser("plan")
    make.add_argument("--output", type=Path, required=True)
    make.add_argument("--backends", nargs="+", choices=BACKENDS, default=list(BACKENDS))
    make.add_argument("--tier", choices=("native", "constraint", "real"), required=True)
    make.add_argument("--shard", type=int, default=0)
    make.add_argument("--shards", type=int, default=1)
    source = make.add_mutually_exclusive_group(required=True)
    source.add_argument("--exhaustive-tiny", action="store_true")
    source.add_argument("--finite-noir", action="store_true")
    source.add_argument("--regressions", type=Path)
    source.add_argument("--corpus", type=Path)
    source.add_argument("--seed", type=int)
    make.add_argument("--variables", nargs="+", help="Existing projection for row-only numeric goldens; never inferred from evaluated output")
    make.add_argument("--count", type=int, default=100)
    execute = sub.add_parser("run")
    execute.add_argument("--plan", type=Path, required=True)
    execute.add_argument("--adapters", type=Path, required=True)
    execute.add_argument("--output", type=Path, required=True)
    execute.add_argument("--timeout", type=int, default=1800)
    execute.add_argument("--output-limit", type=int, default=16 * 1024 * 1024)
    args = parser.parse_args()
    if args.command == "plan":
        if args.exhaustive_tiny:
            cases, coverage = exhaustive(), "exhaustive_tiny_v1"
        elif args.finite_noir:
            cases, coverage = finite_proof_universe(), "complete_two_edge_cycle_v1_with_noir_attacks"
        elif args.regressions:
            cases, coverage = import_regressions(args.regressions, args.variables), "retained_existing_regressions"
        elif args.corpus:
            cases, coverage = load(args.corpus), "retained_imported_corpus"
            if isinstance(cases, dict):
                cases = cases["cases"]
        else:
            cases, coverage = sampled(args.seed, args.count), "sampled_not_exhaustive"
        write(args.output, plan(cases, args.backends, args.tier, args.shard, args.shards, coverage))
        return True
    if not 1 <= args.timeout <= 7200 or not 1024 <= args.output_limit <= 64 * 1024 * 1024:
        raise ValueError("execution budget outside bounded profile")
    return run(args.plan, args.adapters, args.output, args.timeout, args.output_limit)


if __name__ == "__main__":
    try:
        sys.exit(0 if main() else 1)
    except (OSError, ValueError, KeyError, TypeError) as error:
        print(f"proof corpus rejected: {error}", file=sys.stderr)
        sys.exit(2)
