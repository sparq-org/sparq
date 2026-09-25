#!/usr/bin/env python3
"""[GPT-6] Mandatory bounded CI invocation; missing cells/tools fail closed."""
import argparse
import json
import os
from pathlib import Path
import shutil
import subprocess

from corpus import exhaustive, finite_proof_universe, load, plan
from run import file_hash, run, write

ROOT = Path(__file__).resolve().parents[2]


def build(output, label, command, target_name):
    """Discover the exact Cargo-produced executable, never a stale glob match."""
    command = command + ["--message-format=json"]
    write(output / f"{label}-build.json", {"argv":command, "cwd":str(ROOT),
          "environment":{key:os.environ.get(key) for key in (
              "CARGO_TARGET_DIR", "CARGO_BUILD_JOBS", "CARGO_INCREMENTAL", "RUSTFLAGS",
              "CARGO_PROFILE_DEV_DEBUG", "DEVELOPER_DIR", "SDKROOT")}})
    with (output / f"{label}-cargo.jsonl").open("xb") as stdout, (output / f"{label}-cargo.stderr").open("xb") as stderr:
        subprocess.run(command, cwd=ROOT, stdout=stdout, stderr=stderr,
                       check=True, timeout=600)
    executables = []
    for line in (output / f"{label}-cargo.jsonl").read_text().splitlines():
        event = json.loads(line)
        if (event.get("reason") == "compiler-artifact" and event.get("executable")
                and event["target"]["name"] == target_name):
            executables.append(event["executable"])
    if len(executables) != 1:
        raise ValueError(f"expected exactly one compiled {label} executable")
    return Path(executables[0]).resolve()


def exact_tools():
    expected = {"nargo":"1.0.0-beta.21", "bb":"5.0.0-nightly.20260324"}
    result = {}
    for name, version in expected.items():
        found = shutil.which(name)
        if not found:
            raise ValueError(f"required real tool {name} is missing")
        path = Path(found).resolve()
        output = subprocess.run([str(path), "--version"], capture_output=True, check=True, timeout=30)
        stdout, stderr = output.stdout.decode(), output.stderr.decode()
        # Check the full version token, then pin every byte of output and binary
        # for every subsequent job. A substring such as beta.210 cannot pass.
        first = stdout.splitlines()[0] if stdout.splitlines() else ""
        if first != (f"nargo version = {version}" if name == "nargo" else version):
            raise ValueError(f"unexpected exact {name} version")
        result[name] = {"path":str(path), "sha256":file_hash(path),
                        "version_stdout":stdout, "version_stderr":stderr}
    return result


def check_inventory(report, expected):
    if not report.get("passed") or not report.get("complete_declared_domain"):
        raise ValueError("incomplete or failing executed domain")
    if set(report["totals"]) != set(expected):
        raise ValueError("backend denominator mismatch")
    for backend, (jobs, proofs, constraint_negatives) in expected.items():
        got = report["totals"][backend]
        if any(got[field] != jobs for field in ("configured_jobs", "shard_jobs", "executed_jobs", "passed_jobs")):
            raise ValueError(f"missing or duplicate cells for {backend}")
        if got["genuine_proofs"] != proofs or got["verified_proofs"] != proofs:
            raise ValueError(f"wrong actual proof count for {backend}")
        if constraint_negatives is not None and got["negative_stages"] != {"constraint":constraint_negatives}:
            raise ValueError("honest API refusal cannot replace a direct malicious witness")


def campaign(output):
    output.mkdir(parents=True, exist_ok=False)
    head = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip()
    event_path = os.environ.get("GITHUB_EVENT_PATH")
    event = json.loads(Path(event_path).read_text()) if event_path else {}
    context = {"checkout_sha":head, "github_sha":os.environ.get("GITHUB_SHA"),
               "pr_head_sha":event.get("pull_request", {}).get("head", {}).get("sha"),
               "pr_base_sha":event.get("pull_request", {}).get("base", {}).get("sha"),
               "merge_group_head_sha":event.get("merge_group", {}).get("head_sha"),
               "root_lock_sha256":file_hash(ROOT / "Cargo.lock"),
               "model_lock_sha256":file_hash(ROOT / "zk/sparql-evaluator/Cargo.lock")}
    write(output / "checkout.json", context)
    tools = exact_tools()
    write(output / "tools.json", tools)
    print("Building exact committed adapters", flush=True)
    noir = build(output, "noir", ["cargo", "test", "--locked", "-p", "sparq-zk-compose",
                                  "--features", "successful-results", "--lib", "--no-run"], "sparq_zk_compose")
    model = build(output, "model", ["cargo", "build", "--locked", "--manifest-path", "zk/sparql-evaluator/Cargo.toml",
                                   "-p", "sparq-proved-evaluator-model", "--features", "evaluate",
                                   "--example", "proof_bindings"], "proof_bindings")
    common = {"checkout":str(ROOT), "source_commit":head}
    noir_adapter = common | {"kind":"rust_test", "argv":[str(noir), "--ignored", "--exact",
                          "result::proof_bindings::run_job", "--nocapture"],
                          "executable_sha256":file_hash(noir), "tools":tools}
    model_adapter = common | {"argv":[str(model)], "executable_sha256":file_hash(model)}
    adapters = {"noir_unsigned":noir_adapter, "noir_signed":noir_adapter,
                "exact_v1":model_adapter, "exact_v2":model_adapter}
    write(output / "adapters.json", adapters)
    for label, cases, backends, tier, expected in [
        ("native", exhaustive(), list(adapters), "native",
         {"noir_unsigned":(576,0,None), "noir_signed":(576,0,None), "exact_v1":(224,0,None), "exact_v2":(224,0,None)}),
        ("real", finite_proof_universe(), ["noir_unsigned","noir_signed"], "real",
         {"noir_unsigned":(46,4,42), "noir_signed":(46,4,42)}),
    ]:
        path = output / f"{label}-plan.json"
        write(path, plan(cases, backends, tier, coverage=f"mandatory_ci_{label}_v1"))
        if not run(path, output / "adapters.json", output / label, 600, 16*1024*1024):
            raise ValueError(f"{label} campaign failed")
        check_inventory(load(output / label / "report.json"), expected)
    write(output / "completion.json", {"schema":"sparq.proof-bindings.ci.v1", "passed":True,
                                        "checkout":context, "native_jobs":1600, "real_jobs":92,
                                        "proofs":8, "constraint_negatives":84})


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()
    campaign(args.output.resolve())


if __name__ == "__main__":
    main()
