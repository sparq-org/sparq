#!/usr/bin/env python3
"""Generate or check method-bound result key pins using the canonical driver commands."""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import tempfile
import tomllib

ROOT = Path(__file__).resolve().parents[3]
COMPOSE = ROOT / "zk/compose"
PINS = ROOT / "crates/sparq-zk-compose/tests/result_method_keys.json"


def run(args, cwd=COMPOSE):
    return subprocess.check_output(args, cwd=cwd, stderr=subprocess.PIPE)


def generate():
    if run(["nargo", "--version"]).decode().splitlines()[0].strip() != "nargo version = 1.0.0-beta.21":
        raise SystemExit("nargo version mismatch")
    if run(["bb", "--version"]).decode().strip() != "5.0.0-nightly.20260324":
        raise SystemExit("bb version mismatch")
    members = sorted(m for m in tomllib.loads((COMPOSE / "Nargo.toml").read_text())["workspace"]["members"] if m.startswith("result_v5_"))
    pins = {}
    for member in members:
        run(["nargo", "compile", "--package", member])
        artifact = json.loads((COMPOSE / "target" / f"{member}.json").read_bytes())
        # Same target and invocation as CircuitProver::canonical_package_vk.
        with tempfile.TemporaryDirectory(prefix="sparq-result-key-") as scratch:
            run(["bb", "write_vk", "-b", str(COMPOSE / "target" / f"{member}.json"), "-o", scratch, "-t", "noir-recursive"])
            key = (Path(scratch) / "vk").read_bytes()
        pins[member] = {
            "abi_sha256": hashlib.sha256(json.dumps(artifact["abi"], sort_keys=True, separators=(",", ":")).encode()).hexdigest(),
            "vk_bytes": len(key),
            "vk_sha256": hashlib.sha256(key).hexdigest(),
        }
    return {"target": "noir-recursive", "members": pins}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    modes = parser.add_mutually_exclusive_group(required=True)
    modes.add_argument("--write", action="store_true")
    modes.add_argument("--check", action="store_true")
    args = parser.parse_args()
    result = generate()
    if args.write:
        PINS.write_text(json.dumps(result, indent=2) + "\n")
    elif json.loads(PINS.read_text()) != result:
        raise SystemExit("method-bound result key pins differ")


if __name__ == "__main__":
    main()
