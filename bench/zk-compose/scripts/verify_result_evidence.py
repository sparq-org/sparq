#!/usr/bin/env python3
"""[GPT-6] Check result compatibility evidence; optionally rebuild the fixed foundation."""

from __future__ import annotations

import argparse
import base64
import copy
import hashlib
import json
from pathlib import Path
import subprocess
import tempfile

BASE = "713025dc0fcdcbdcd070ba4dcbde524be50ba055"
ROOT = Path(__file__).resolve().parents[3]
EVIDENCE = ROOT / "bench/zk-compose/result_v1_compatibility.json"
CAPACITY = ROOT / "bench/zk-compose/result_capacity_gates.json"
SNAPSHOT = ROOT / "bench/zk-compose/gate_counts_latest.json"
MEMBERS = {f"result_v1_k{k}_n16_p3_r4_f{f}" for k in (1, 2) for f in (0, 2)}


def digest(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def check(data: dict, root: Path) -> None:
    if data.get("comparison_base") != BASE or set(data.get("members", {})) != MEMBERS:
        raise ValueError("compatibility baseline or member inventory mismatch")
    for member, record in data["members"].items():
        base, candidate = record["base"], record["candidate"]
        key = bytes.fromhex(base["verification_key_hex"])
        if len(key) != base["vk_bytes"] or len(key) != candidate["vk_bytes"]:
            raise ValueError(f"{member}: key length mismatch")
        if digest(key) != base["vk_sha256"]:
            raise ValueError(f"{member}: baseline key hash mismatch")
        for flag, field in (("acir_identical", "acir_bytecode_sha256"),
                            ("abi_identical", "abi_sha256"), ("vk_identical", "vk_sha256")):
            if record[flag] is not True or base[field] != candidate[field]:
                raise ValueError(f"{member}: {flag} does not establish identity")
            if len(base[field]) != 64 or any(c not in "0123456789abcdef" for c in base[field]):
                raise ValueError(f"{member}: invalid {field}")
    expected = {
        str(p.relative_to(root)) for p in (root / "zk/compose").rglob("*.nr")
        if "target" not in p.relative_to(root).parts
    }
    if set(data["source_files"]) != expected:
        raise ValueError("measured Noir source inventory mismatch")
    for name, expected_hash in data["source_files"].items():
        path = root / name
        if not path.resolve().is_relative_to(root.resolve()):
            raise ValueError("source hash path escapes the repository")
        if digest(path.read_bytes()) != expected_hash:
            raise ValueError(f"{name}: measured source hash mismatch")


def check_capacity(capacity: dict, data: dict, snapshot: dict) -> None:
    if capacity["source_files"] != data["source_files"] or capacity["comparison_base"] != BASE:
        raise ValueError("capacity evidence does not identify the same measured source")
    for tool in ("nargo", "bb"):
        if capacity[f"{tool}_version"] != data["tool_versions"][tool]:
            raise ValueError(f"{tool}: capacity and compatibility toolchains differ")
    expected = {
        name for name in snapshot["benchmarks"]
        if name.startswith(("result_v1_", "result_v2_"))
        or name in {f"filter_int_d{digits}" for digits in range(1, 5)}
    }
    if set(capacity["members"]) != expected:
        raise ValueError("capacity evidence member inventory differs from the gate snapshot")
    for name, record in capacity["members"].items():
        if record["circuit_size"] != snapshot["benchmarks"][name]["circuit_size"]:
            raise ValueError(f"{name}: capacity evidence gate count differs from the snapshot")
        acir_hash = record["acir_bytecode_sha256"]
        if len(acir_hash) != 64 or any(c not in "0123456789abcdef" for c in acir_hash):
            raise ValueError(f"{name}: invalid measured ACIR hash")
        if name in MEMBERS and acir_hash != data["members"][name]["candidate"]["acir_bytecode_sha256"]:
            raise ValueError(f"{name}: measured capacity and compatibility ACIR hashes differ")


def self_test(data: dict, capacity: dict, snapshot: dict) -> None:
    member = sorted(MEMBERS)[0]
    variants = []
    changed = copy.deepcopy(data)
    changed["members"][member]["base"]["vk_bytes"] += 1
    variants.append(changed)
    changed = copy.deepcopy(data)
    changed["members"][member]["abi_identical"] = False
    variants.append(changed)
    changed = copy.deepcopy(data)
    changed["members"][member]["candidate"]["vk_sha256"] = "0" * 64
    variants.append(changed)
    changed = copy.deepcopy(data)
    del changed["members"][member]
    variants.append(changed)
    changed = copy.deepcopy(data)
    changed["source_files"][next(iter(changed["source_files"]))] = "0" * 64
    variants.append(changed)
    changed = copy.deepcopy(data)
    changed["comparison_base"] = changed["candidate_parent"]
    variants.append(changed)
    for index, changed in enumerate(variants):
        try:
            check(changed, ROOT)
        except ValueError:
            continue
        raise AssertionError(f"corrupted evidence variant {index} was accepted")
    capacity_variants = []
    changed = copy.deepcopy(capacity)
    changed["source_files"] = {}
    capacity_variants.append(changed)
    changed = copy.deepcopy(capacity)
    changed["members"][member]["circuit_size"] += 1
    capacity_variants.append(changed)
    changed = copy.deepcopy(capacity)
    changed["members"][member]["acir_bytecode_sha256"] = "0" * 64
    capacity_variants.append(changed)
    for index, changed in enumerate(capacity_variants):
        try:
            check_capacity(changed, data, snapshot)
        except ValueError:
            continue
        raise AssertionError(f"corrupted capacity evidence variant {index} was accepted")
    print(f"Result evidence: {len(variants) + len(capacity_variants)} corruption controls rejected")


def run(command: list[str], cwd: Path, timeout: int = 600) -> str:
    result = subprocess.run(command, cwd=cwd, capture_output=True, text=True, timeout=timeout)
    if result.returncode:
        raise RuntimeError(f"{command[0]} failed:\n{result.stdout}\n{result.stderr}")
    return result.stdout.strip()


def rebuild_baseline(data: dict, baseline_root: Path) -> None:
    """The CI step supplies a separate archive fetched from the literal BASE commit."""
    source = baseline_root.resolve() / "zk/compose"
    if source == (ROOT / "zk/compose").resolve():
        raise ValueError("baseline compilation must not use the candidate working tree")
    for tool in ("nargo", "bb"):
        if run([tool, "--version"], source) != data["tool_versions"][tool]:
            raise ValueError(f"{tool}: exact recorded toolchain version mismatch")
    for member in sorted(MEMBERS):
        run(["nargo", "compile", "--package", member], source)
        artifact = source / "target" / f"{member}.json"
        compiled = json.loads(artifact.read_text())
        expected = data["members"][member]["base"]
        if digest(base64.b64decode(compiled["bytecode"])) != expected["acir_bytecode_sha256"]:
            raise ValueError(f"{member}: independently rebuilt baseline ACIR mismatch")
        abi = json.dumps(compiled["abi"], sort_keys=True, separators=(",", ":")).encode()
        if digest(abi) != expected["abi_sha256"]:
            raise ValueError(f"{member}: independently rebuilt baseline ABI mismatch")
        with tempfile.TemporaryDirectory(prefix="sparq-baseline-key-") as output:
            run(["bb", "write_vk", "-b", str(artifact), "-o", output, "-t", "noir-recursive"], source)
            if (Path(output) / "vk").read_bytes() != bytes.fromhex(expected["verification_key_hex"]):
                raise ValueError(f"{member}: independently rebuilt baseline key mismatch")
        print(f"{BASE} {member}: independently rebuilt ACIR, ABI and key match", flush=True)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--self-test", action="store_true")
    parser.add_argument("--baseline-root", type=Path)
    args = parser.parse_args()
    data = json.loads(EVIDENCE.read_text())
    capacity = json.loads(CAPACITY.read_text())
    snapshot = json.loads(SNAPSHOT.read_text())
    check(data, ROOT)
    check_capacity(capacity, data, snapshot)
    print("Result evidence: inventory, lengths, identities, gate counts and source hashes match")
    if args.self_test:
        self_test(data, capacity, snapshot)
    if args.baseline_root:
        rebuild_baseline(data, args.baseline_root)


if __name__ == "__main__":
    main()
