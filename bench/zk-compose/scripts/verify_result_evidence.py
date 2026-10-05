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
SIGNED = ROOT / "bench/zk-compose/result_signed_gates.json"
SNAPSHOT = ROOT / "bench/zk-compose/gate_counts_latest.json"
PUBLIC = ROOT / "bench/zk-compose/result_public_gates.json"
REGRESSION = ROOT / "crates/sparq-zk-compose/tests/gate_count_snapshot.json"
MEMBERS = {f"result_v1_k{k}_n16_p3_r4_f{f}" for k in (1, 2) for f in (0, 2)}
PUBLIC_MEMBERS = {f"result_v4_k{k}_n16_p3_r4_f0_d10" for k in (1, 2)}


def digest(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def is_hex(value: object, length: int = 64) -> bool:
    return isinstance(value, str) and len(value) == length and all(c in "0123456789abcdef" for c in value)


def check_measured_source(source: object) -> None:
    """[OPUS-5.5] Require exact, explicitly non-canonical candidate provenance."""
    if not isinstance(source, dict):
        raise ValueError("measured source provenance is missing")
    if not all(is_hex(source.get(field), 40) for field in ("source_commit", "source_tree")):
        raise ValueError("measured source commit or tree is not a full object id")
    hashes = source.get("tool_executable_sha256")
    if not isinstance(hashes, dict) or set(hashes) != {"nargo", "bb"} or not all(
        is_hex(value) for value in hashes.values()
    ):
        raise ValueError("measured tool executable hashes are incomplete or invalid")
    if not all(is_hex(source.get(field)) for field in ("job_sha256", "independent_handoff_sha256")):
        raise ValueError("measured job or handoff hash is invalid")
    if source.get("canonical") is not False:
        raise ValueError("work-box measurements must stay labelled non-canonical")


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
    check_measured_source(data.get("measured_source"))
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
    if (capacity["source_files"] != data["source_files"] or capacity["comparison_base"] != BASE
            or capacity.get("measured_source") != data["measured_source"]):
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


def check_signed(signed: dict, data: dict, snapshot: dict) -> None:
    """[GPT-6] Bind the fixed signed-capacity measurement inventory to its source."""
    if (signed["source_files"] != data["source_files"] or signed["tool_versions"] != data["tool_versions"]
            or signed.get("measured_source") != data["measured_source"]):
        raise ValueError("signed evidence source or toolchain differs from compatibility evidence")
    expected = {
        f"result_v3_k{k}_n16_p3_r4_f{f}_s{64 if f else 0}_d{depth}"
        for k in (1, 2) for f in (0, 2) for depth in (10, 17, 20)
    }
    if set(signed["members"]) != expected or {
        name for name in snapshot["benchmarks"] if name.startswith("result_v3_")
    } != expected:
        raise ValueError("signed measurement or snapshot member inventory mismatch")
    for name, record in signed["members"].items():
        if record["circuit_size"] != snapshot["benchmarks"][name]["circuit_size"]:
            raise ValueError(f"{name}: signed gate count differs from the snapshot")
        for field in ("acir_bytecode_sha256", "abi_sha256"):
            value = record[field]
            if len(value) != 64 or any(c not in "0123456789abcdef" for c in value):
                raise ValueError(f"{name}: invalid signed {field}")


def legacy_acir(capacity: dict, signed: dict) -> set[str]:
    return {r["acir_bytecode_sha256"] for r in (*capacity["members"].values(), *signed["members"].values())}


def check_public(public: dict, data: dict, snapshot: dict, regression: dict, legacy: set[str]) -> None:
    """[OPUS-5.5] Bind the version-4 static measurements to the same source and tools."""
    if (public["source_files"] != data["source_files"] or public["tool_versions"] != data["tool_versions"]
            or public.get("measured_source") != data["measured_source"]):
        raise ValueError("public-pattern evidence source, toolchain or provenance differs")
    if set(public["members"]) != PUBLIC_MEMBERS or {
        name for name in snapshot["benchmarks"] if name.startswith("result_v4_")
    } != PUBLIC_MEMBERS or {
        name for name in regression["members"] if name.startswith("result_v4_")
    } != PUBLIC_MEMBERS:
        raise ValueError("public-pattern measurement or snapshot member inventory mismatch")
    required = {"zk/compose/compose_core/src/result_public.nr"} | {
        f"zk/compose/{name}/src/main.nr" for name in PUBLIC_MEMBERS
    }
    if not required <= set(public["source_files"]):
        raise ValueError("public-pattern sources are absent from the measured inventory")
    seen: set[str] = set()
    for name, record in public["members"].items():
        size = record["circuit_size"]
        if (type(size) is not int or size <= 0 or size != snapshot["benchmarks"][name]["circuit_size"]
                or size != regression["members"][name]):
            raise ValueError(f"{name}: public-pattern gate count differs from the snapshots")
        for field in ("acir_bytecode_sha256", "abi_sha256", "artifact_sha256", "gate_log_sha256"):
            if not is_hex(record[field]):
                raise ValueError(f"{name}: invalid public-pattern {field}")
        # A version-4 circuit has its own public ABI, so its ACIR cannot repeat another member's.
        if record["acir_bytecode_sha256"] in legacy | seen:
            raise ValueError(f"{name}: public-pattern ACIR hash duplicates another member")
        seen.add(record["acir_bytecode_sha256"])


def self_test(data: dict, capacity: dict, signed: dict, snapshot: dict,
              public: dict, regression: dict) -> None:
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
    changed["comparison_base"] = "0" * 40
    variants.append(changed)
    # [OPUS-5.5] Candidate provenance: relabelled canonical, truncated commit, bad tool hash.
    for path, value in ((("canonical",), True), (("source_commit",), "14d426bd"),
                        (("tool_executable_sha256", "bb"), "invalid")):
        changed = copy.deepcopy(data)
        target = changed["measured_source"]
        for key in path[:-1]:
            target = target[key]
        target[path[-1]] = value
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
    changed = copy.deepcopy(capacity)
    changed["measured_source"]["observed_at_utc"] = "2026-09-12T20:44:49.380619+00:00"
    capacity_variants.append(changed)
    for index, changed in enumerate(capacity_variants):
        try:
            check_capacity(changed, data, snapshot)
        except ValueError:
            continue
        raise AssertionError(f"corrupted capacity evidence variant {index} was accepted")
    signed_variants = []
    member = sorted(signed["members"])[0]
    for field, value in (("source_files", {}), ("tool_versions", {})):
        changed = copy.deepcopy(signed)
        changed[field] = value
        signed_variants.append(changed)
    changed = copy.deepcopy(signed)
    del changed["members"][member]
    signed_variants.append(changed)
    changed = copy.deepcopy(signed)
    changed["members"][member]["circuit_size"] += 1
    signed_variants.append(changed)
    changed = copy.deepcopy(signed)
    changed["members"][member]["abi_sha256"] = "invalid"
    signed_variants.append(changed)
    changed = copy.deepcopy(signed)
    changed["measured_source"]["platform"] = "canonical CI"
    signed_variants.append(changed)
    for index, changed in enumerate(signed_variants):
        try:
            check_signed(changed, data, snapshot)
        except ValueError:
            continue
        raise AssertionError(f"corrupted signed evidence variant {index} was accepted")
    # [OPUS-5.5] Version-4 record corruptions; each tuple is (public, gate JSON, regression snapshot).
    legacy = legacy_acir(capacity, signed)
    member = sorted(PUBLIC_MEMBERS)[0]
    public_variants = []
    for field, value in (("source_files", {}), ("tool_versions", {})):
        changed = copy.deepcopy(public)
        changed[field] = value
        public_variants.append((changed, snapshot, regression))
    changed = copy.deepcopy(public)
    changed["measured_source"]["tool_executable_sha256"]["nargo"] = "0" * 64
    public_variants.append((changed, snapshot, regression))
    changed = copy.deepcopy(public)
    del changed["members"][member]
    public_variants.append((changed, snapshot, regression))
    changed = copy.deepcopy(public)
    changed["members"]["result_v4_k3_n16_p3_r4_f0_d10"] = changed["members"][member]
    public_variants.append((changed, snapshot, regression))
    changed = copy.deepcopy(public)
    changed["members"][member]["circuit_size"] += 1
    public_variants.append((changed, snapshot, regression))
    for field, value in (("acir_bytecode_sha256", "invalid"), ("abi_sha256", "0" * 63),
                         ("artifact_sha256", "A" * 64), ("gate_log_sha256", None),
                         ("acir_bytecode_sha256", sorted(legacy)[0])):
        changed = copy.deepcopy(public)
        changed["members"][member][field] = value
        public_variants.append((changed, snapshot, regression))
    changed = copy.deepcopy(public)
    other = sorted(PUBLIC_MEMBERS)[1]
    changed["members"][other]["acir_bytecode_sha256"] = changed["members"][member]["acir_bytecode_sha256"]
    public_variants.append((changed, snapshot, regression))
    changed = copy.deepcopy(snapshot)
    del changed["benchmarks"][member]
    public_variants.append((public, changed, regression))
    changed = copy.deepcopy(regression)
    changed["members"][member] += 1
    public_variants.append((public, snapshot, changed))
    for index, (changed, gates, baseline) in enumerate(public_variants):
        try:
            check_public(changed, data, gates, baseline, legacy)
        except ValueError:
            continue
        raise AssertionError(f"corrupted public-pattern evidence variant {index} was accepted")
    total = len(variants) + len(capacity_variants) + len(signed_variants) + len(public_variants)
    print(f"Result evidence: {total} corruption controls rejected")


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
    signed = json.loads(SIGNED.read_text())
    snapshot = json.loads(SNAPSHOT.read_text())
    public = json.loads(PUBLIC.read_text())
    regression = json.loads(REGRESSION.read_text())
    check(data, ROOT)
    check_capacity(capacity, data, snapshot)
    check_signed(signed, data, snapshot)
    check_public(public, data, snapshot, regression, legacy_acir(capacity, signed))
    print("Result evidence: inventory, lengths, identities, gate counts and source hashes match")
    if args.self_test:
        self_test(data, capacity, signed, snapshot, public, regression)
    if args.baseline_root:
        rebuild_baseline(data, args.baseline_root)


if __name__ == "__main__":
    main()
