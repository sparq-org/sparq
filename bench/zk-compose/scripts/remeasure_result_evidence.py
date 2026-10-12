#!/usr/bin/env python3
"""Re-measure the result evidence members and rewrite the four evidence files.

`verify_result_evidence.py` binds the evidence to sha256 hashes of every zk/compose Noir
source, so a change to those sources needs a fresh measurement. Two steps:

    git archive <commit> | tar -x -C run1        # and again into run2
    remeasure_result_evidence.py measure run1 run1.json
    remeasure_result_evidence.py measure run2 run2.json
    remeasure_result_evidence.py apply run1.json run2.json --job job.json \\
        --commit <commit> --observed-at <utc> --platform <host> \\
        --expect-member <member> ... --expect-source <path> ...

`measure` compiles every member with the pinned nargo, hashes its ACIR bytecode, ABI and
artifact, runs `bb gates -s ultra_honk`, and for the v1 members writes the noir-recursive
verification key. `apply` refuses to write unless the two runs agree (the artifact hash
embeds the checkout path and is not compared), every v1 ACIR, ABI and key equals its
713025dc baseline record, every size equals gate_counts_latest.json, and exactly the expected
members and sources change. It validates all four files before replacing any of them. The
result is always labelled non-canonical; the job manifest and run outputs are not part of
the repository, so the two-run agreement it records is checked when `apply` runs, not later.
"""

from __future__ import annotations

import argparse
import base64
import hashlib
import json
import os
import subprocess
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
EVIDENCE = ("result_v1_compatibility", "result_capacity_gates", "result_signed_gates", "result_public_gates")
# Fields each file records per member; the compatibility file is checked separately.
FIELDS = {
    "result_capacity_gates": ("circuit_size", "acir_bytecode_sha256"),
    "result_signed_gates": ("circuit_size", "acir_bytecode_sha256", "abi_sha256"),
    "result_public_gates": ("circuit_size", "acir_bytecode_sha256", "abi_sha256", "gate_log_sha256"),
}


def digest(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def run(command: list[str], cwd: Path) -> str:
    result = subprocess.run(command, cwd=cwd, capture_output=True, text=True, timeout=3600)
    if result.returncode:
        raise SystemExit(f"{command} failed:\n{result.stdout}\n{result.stderr}")
    return result.stdout


def load(root: Path) -> dict[str, dict]:
    return {name: json.loads((root / "bench/zk-compose" / f"{name}.json").read_text()) for name in EVIDENCE}


def measure(source: Path, output: Path) -> None:
    source = source.resolve()
    nargo, bb = os.environ.get("NARGO", "nargo"), os.environ.get("BB", "bb")
    evidence = load(source)
    members = sorted(set().union(*(data["members"] for data in evidence.values())))
    v1 = evidence["result_v1_compatibility"]["members"]
    tool_paths = {tool: Path(run(["which", path], source).strip()).resolve()
                  for tool, path in (("nargo", nargo), ("bb", bb))}
    result = {
        "tools": {"nargo": run([nargo, "--version"], source).strip(), "bb": run([bb, "--version"], source).strip(),
                  "nargo_sha256": digest(tool_paths["nargo"].read_bytes()),
                  "bb_sha256": digest(tool_paths["bb"].read_bytes())},
        "source_files": {
            str(path.relative_to(source)): digest(path.read_bytes())
            for path in sorted((source / "zk/compose").rglob("*.nr"))
            if "target" not in path.relative_to(source).parts
        },
        "members": {},
    }
    compose = source / "zk/compose"
    for member in members:
        run([nargo, "compile", "--package", member, "--force"], compose)
        artifact = compose / "target" / f"{member}.json"
        raw = artifact.read_bytes()
        compiled = json.loads(raw)
        gates = run([bb, "gates", "-s", "ultra_honk", "-b", str(artifact)], compose)
        record = {
            "circuit_size": json.loads(gates)["functions"][0]["circuit_size"],
            "acir_bytecode_sha256": digest(base64.b64decode(compiled["bytecode"])),
            "abi_sha256": digest(json.dumps(compiled["abi"], sort_keys=True, separators=(",", ":")).encode()),
            "artifact_sha256": digest(raw),
            "gate_log_sha256": digest(gates.encode()),
        }
        if member in v1:
            with tempfile.TemporaryDirectory() as key_dir:
                run([bb, "write_vk", "-b", str(artifact), "-o", key_dir, "-t", "noir-recursive"], compose)
                key = (Path(key_dir) / "vk").read_bytes()
            record["vk_sha256"], record["vk_bytes"] = digest(key), len(key)
        result["members"][member] = record
        print(member, record["circuit_size"], flush=True)
    output.write_text(json.dumps(result, indent=1, sort_keys=True) + "\n")


def without_artifacts(run_result: dict) -> dict:
    copy = json.loads(json.dumps(run_result))
    for record in copy["members"].values():
        record.pop("artifact_sha256")
    return copy


def apply(args: argparse.Namespace) -> None:
    first, second = (json.loads(path.read_text()) for path in (args.first, args.second))
    if without_artifacts(first) != without_artifacts(second):
        raise SystemExit("the two runs disagree")
    measured = first["members"]
    evidence = load(ROOT)
    snapshot = json.loads((ROOT / "bench/zk-compose/gate_counts_latest.json").read_text())["benchmarks"]
    for member, record in evidence["result_v1_compatibility"]["members"].items():
        base, got = record["base"], measured[member]
        if any(got[field] != base[field] for field in ("acir_bytecode_sha256", "abi_sha256", "vk_sha256", "vk_bytes")):
            raise SystemExit(f"{member}: v1 differs from its baseline record")
    changed = set()
    for name, fields in FIELDS.items():
        for member, record in evidence[name]["members"].items():
            new = {field: measured[member][field] for field in fields}
            if new["circuit_size"] != snapshot[member]["circuit_size"]:
                raise SystemExit(f"{member}: measured size differs from gate_counts_latest.json")
            if any(record[field] != new[field] for field in fields):
                changed.add(member)
            record.update(new)
    old = evidence["result_v1_compatibility"]["source_files"]
    new = first["source_files"]
    moved = {path for path in set(old) | set(new) if old.get(path) != new.get(path)}
    if changed != set(args.expect_member) or moved != set(args.expect_source):
        raise SystemExit(f"unexpected change: members {sorted(changed)}, sources {sorted(moved)}")
    tree = run(["git", "rev-parse", f"{args.commit}^{{tree}}"], ROOT).strip()
    provenance = {
        "source_commit": args.commit,
        "source_tree": tree,
        "observed_at_utc": args.observed_at,
        "platform": args.platform,
        "canonical": False,
        "tool_executable_sha256": {"nargo": first["tools"]["nargo_sha256"], "bb": first["tools"]["bb_sha256"]},
        "job_sha256": digest(args.job.read_bytes()),
        "independent_handoff_sha256": digest(args.second.read_bytes()),
    }
    method = ("At measured_source, two independent runs from clean git archives of source_commit compiled every "
              "listed member with the pinned nargo, hashed the ACIR bytecode, ABI and artifact, measured "
              "`bb gates -s ultra_honk` and, for the v1 members, regenerated the noir-recursive verification key "
              "(bench/zk-compose/scripts/remeasure_result_evidence.py). The two runs agreed on every circuit "
              "size, ACIR, ABI, key and gate log hash; every v1 ACIR, ABI and key equalled its baseline record. "
              "The compiled artifact embeds its checkout path, so artifact_sha256 is kept as recorded. The "
              "two-run agreement was checked outside the repository and cannot be reproduced from it alone: "
              "job_sha256 and independent_handoff_sha256 identify the job manifest and the second run's output, "
              "which are retained outside the repository, and that output includes path-dependent artifact "
              "hashes, so a rerun will not reproduce its digest.")
    limitations = ("Non-canonical: measured on a shared work box, not a dedicated machine; future CI replays are "
                   "still required. Static compile, circuit size and key identity only; no timing, soundness or "
                   "external-audit claim. source_files, not source_commit, binds the Noir bytes.")
    tools = first["tools"]
    for name, data in evidence.items():
        recorded = data.get("tool_versions") or {"nargo": data["nargo_version"], "bb": data["bb_version"]}
        if recorded["nargo"] != tools["nargo"] or recorded["bb"] != tools["bb"]:
            raise SystemExit(f"{name}: toolchain differs from the recorded one")
    # Every check has passed: stage all four files, then replace them, so a failure never
    # leaves a partly rewritten evidence set.
    staged = []
    for name, data in evidence.items():
        data["source_files"] = new
        data["measured_source"] = provenance
        data["measurement_method"] = method
        data["limitations"] = limitations
        target = ROOT / "bench/zk-compose" / f"{name}.json"
        handle, temporary = tempfile.mkstemp(dir=target.parent, prefix=f".{name}.", suffix=".json")
        with os.fdopen(handle, "w") as output:
            output.write(json.dumps(data, indent=2) + "\n")
        staged.append((temporary, target))
    for temporary, target in staged:
        os.replace(temporary, target)
    print("changed members:", ", ".join(sorted(changed)))


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    commands = parser.add_subparsers(dest="command", required=True)
    step = commands.add_parser("measure")
    step.add_argument("source", type=Path, help="clean archive of the commit to measure")
    step.add_argument("output", type=Path)
    step = commands.add_parser("apply")
    step.add_argument("first", type=Path)
    step.add_argument("second", type=Path, help="independent run; its hash is the handoff")
    step.add_argument("--job", type=Path, required=True, help="job manifest; its hash is job_sha256")
    step.add_argument("--commit", required=True)
    step.add_argument("--observed-at", required=True)
    step.add_argument("--platform", required=True)
    step.add_argument("--expect-member", action="append", default=[])
    step.add_argument("--expect-source", action="append", default=[])
    args = parser.parse_args()
    if args.command == "measure":
        measure(args.source, args.output)
    else:
        apply(args)


if __name__ == "__main__":
    main()
