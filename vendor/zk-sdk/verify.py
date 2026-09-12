#!/usr/bin/env python3
"""[GPT-6] Verify SDK vendor provenance and optionally run synthetic API checks."""

from pathlib import Path
import argparse
import hashlib
import json
import os
import shutil
import subprocess
import tempfile
import tomllib

ROOT = Path(__file__).resolve().parent
REPO = ROOT.parents[1]


def check() -> None:
    metadata = json.loads((ROOT / "UPSTREAM.json").read_text())
    for package in metadata["packages"]:
        name = f"{package['name']}-{package['version']}"
        directory = ROOT / name
        expected = dict(package["upstream_files"])
        for patch in package["patch_files"]:
            assert expected[patch["path"]] == patch["upstream_sha256"]
            expected[patch["path"]] = patch["patched_sha256"]
        for addition in package.get("additional_files", []):
            assert addition["path"] not in expected
            expected[addition["path"]] = addition["sha256"]
        assert {str(p.relative_to(directory)) for p in directory.rglob("*") if p.is_file()} == set(expected)
        for path, sha in expected.items():
            assert hashlib.sha256((directory / path).read_bytes()).hexdigest() == sha, (name, path)
        assert hashlib.sha256((ROOT / f"{name}.patch").read_bytes()).hexdigest() == package["patch_sha256"]
    rzup = tomllib.loads((ROOT / "rzup-0.5.2/Cargo.toml").read_text())
    assert rzup["features"]["default"] == ["cli", "install", "publish"]
    assert all("signatures" in rzup["features"][f] for f in ["install", "publish"])
    assert rzup["dependencies"]["rsa"]["optional"] is True
    for relative in ["Cargo.lock", "methods/guest/Cargo.lock"]:
        lock = tomllib.loads((REPO / "zk/sparql-evaluator" / relative).read_text())
        assert not {p["name"] for p in lock["package"]} & {"rsa", "option-ext", "dirs", "dirs-sys"}
        assert {p["version"] for p in lock["package"] if p["name"] == "risc0-zkvm"} == {"3.0.6"}
        assert {p["version"] for p in lock["package"] if p["name"] == "tracing-subscriber"} == {"0.3.23"}
    print("SDK upstream/patched inventories, hashes, retained feature defaults and detached locks match")


def smoke(offline: bool) -> None:
    if not os.environ.get("CARGO_TARGET_DIR"):
        raise ValueError("Set CARGO_TARGET_DIR to an existing compatible cache for the smoke tests")
    with tempfile.TemporaryDirectory(prefix="sparq-sdk-smoke-") as temporary:
        directory = Path(temporary)
        (directory / "src").mkdir()
        shutil.copyfile(ROOT / "tests/smoke.rs", directory / "src/lib.rs")
        # Local publish cfg only activates the unchanged included sign method.
        # It never enables rzup publication or networking in this harness.
        manifest = f'''[package]
name = "sparq-sdk-patch-smoke"
version = "0.0.0"
edition = "2021"
publish = false
[workspace]
[features]
default = ["publish"]
publish = []
[dependencies]
rzup = {{ path = {json.dumps(str(ROOT / "rzup-0.5.2"))}, default-features = false }}
ark-relations = {{ path = {json.dumps(str(ROOT / "ark-relations-0.5.1"))}, features = ["std"] }}
ark-bn254 = "=0.5.0"
tracing = "=0.1.44"
tracing-subscriber = {{ version = "=0.3.23", default-features = false, features = ["registry"] }}
rsa = "=0.9.10"
rand = "=0.8.8"
sha2 = "=0.10.9"
hex = "=0.4.3"
[profile.dev]
debug = 0
'''
        (directory / "Cargo.toml").write_text(manifest)
        shutil.copyfile(REPO / "zk/sparql-evaluator/Cargo.lock", directory / "Cargo.lock")
        env = os.environ.copy()
        env.update(SPARQ_SDK_VENDOR_ROOT=str(ROOT), SPARQ_SDK_SMOKE_ROOT=temporary,
                   RISC0_HOME=str(directory / "risc0"), GITHUB_TOKEN="synthetic-sdk-smoke",
                   CARGO_INCREMENTAL="0")
        command = ["cargo", "test", "--manifest-path", str(directory / "Cargo.toml"), "--lib"]
        if offline:
            command.append("--offline")
        subprocess.run(command, env=env, check=True, timeout=600)
        # Also compile/run the additive key-taking API; installer/publication
        # integration remains outside this synthetic harness.
        env["RISC0_HOME"] = str(directory / "risc0-key-mode")
        subprocess.run(command + ["--features", "rzup/signatures"], env=env, check=True, timeout=600)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--smoke", action="store_true")
    parser.add_argument("--offline", action="store_true", help="use only cached smoke-test dependencies")
    args = parser.parse_args()
    check()
    if args.smoke:
        smoke(args.offline)
