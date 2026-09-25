#!/usr/bin/env python3
"""[GPT-6] Check native-only patch provenance; optionally execute Ark API controls."""

import argparse
import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import tomllib


SUPPORT = Path(__file__).resolve().parent
NATIVE = SUPPORT.parent
PACKAGE = NATIVE / "vendor/ark-relations-0.4.0"


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def inventory(directory):
    return {
        path.relative_to(directory).as_posix(): digest(path)
        for path in directory.rglob("*")
        if path.is_file()
    }


def require(condition, message):
    if not condition:
        raise ValueError(message)


def check_metadata(metadata):
    selected = [
        p for p in metadata["packages"]
        if p["name"] == "ark-relations" and p["version"] == "0.4.0"
    ]
    require(len(selected) == 1, "one native Ark package must resolve")
    require(selected[0]["source"] is None, "native Ark patch must be local")
    require(
        Path(selected[0]["manifest_path"]).resolve() == (PACKAGE / "Cargo.toml").resolve(),
        "native Ark metadata must select the exact vendor manifest",
    )
    nodes = {node["id"] for node in metadata["resolve"]["nodes"]}
    require(selected[0]["id"] in nodes, "native Ark patch must be in the active graph")
    active = [p for p in metadata["packages"] if p["id"] in nodes]
    subscribers = [p for p in active if p["name"] == "tracing-subscriber"]
    require(
        len(subscribers) == 1 and subscribers[0]["version"] == "0.3.23",
        "active graph must select the pinned maintained subscriber",
    )


def native_metadata():
    """Resolve the already fetched locked graph; preserve Cargo failure details."""
    result = subprocess.run(
        ["cargo", "metadata", "--offline", "--locked", "--all-features", "--format-version", "1", "--manifest-path", str(NATIVE / "Cargo.toml")],
        capture_output=True, text=True, timeout=120,
    )
    if result.returncode:
        # Metadata stdout is machine data. Only Cargo's diagnostic stream belongs
        # in a failed check; there is no retry with different resolution flags.
        if result.stderr:
            print(result.stderr, file=sys.stderr, end="" if result.stderr.endswith("\n") else "\n")
        result.check_returncode()
    return json.loads(result.stdout)


def verify():
    provenance = json.loads((SUPPORT / "UPSTREAM.json").read_text())
    require(provenance["package_path"] == "vendor/ark-relations-0.4.0", "package path")
    require(provenance["patch_path"] == "ark-relations.patch", "patch path")
    require(provenance["package"] == "ark-relations" and provenance["version"] == "0.4.0", "package identity")
    expected = {n: r["patched_sha256"] for n, r in provenance["files"].items()}
    original = {n: r["upstream_sha256"] for n, r in provenance["files"].items()}
    require(inventory(PACKAGE) == expected, "complete vendor file inventory/hash mismatch")
    patch = SUPPORT / "ark-relations.patch"
    require(digest(patch) == provenance["patch_sha256"], "patch hash mismatch")
    with tempfile.TemporaryDirectory(prefix="sparq-native-ark-reconstruct-") as temporary:
        copied = Path(temporary) / "package"
        shutil.copytree(PACKAGE, copied)
        for reverse, hashes in [(True, original), (False, expected)]:
            arguments = ["git", "apply", "--unidiff-zero", "-p1"]
            if reverse:
                arguments.append("--reverse")
            subprocess.run(arguments + [str(patch)], cwd=copied, check=True, timeout=30)
            require(inventory(copied) == hashes, "patch reconstruction mismatch")
    for filename in ["Cargo.toml", "Cargo.toml.orig"]:
        manifest = tomllib.loads((PACKAGE / filename).read_text())
        dependency = manifest["dependencies"]["tracing-subscriber"]
        require(dependency == {"version": "0.3.23", "optional": True, "default-features": False}, "subscriber feature contract")
        require(manifest["features"]["default"] == [], "Ark default features changed")
    manifest = tomllib.loads((NATIVE / "Cargo.toml").read_text())
    require(
        manifest["patch"]["crates-io"]["ark-relations"] == {"path": "vendor/ark-relations-0.4.0"},
        "native manifest must select the exact local patch path",
    )
    lock = tomllib.loads((NATIVE / "Cargo.lock").read_text())
    ark = [p for p in lock["package"] if p["name"] == "ark-relations"]
    require(len(ark) == 1 and ark[0]["version"] == "0.4.0" and "source" not in ark[0], "local Ark lock selection")
    check_metadata(native_metadata())
    return provenance


def smoke(offline):
    # An isolated test-only graph enables Registry; the real native graph does not.
    # Its resolved temporary lock is hashed in the result before cleanup.
    with tempfile.TemporaryDirectory(prefix="sparq-native-ark-smoke-") as temporary:
        root = Path(temporary)
        package = json.dumps(str(PACKAGE))
        (root / "Cargo.toml").write_text(f'''[package]
name = "sparq-native-ark-compatibility"
version = "0.0.0"
edition = "2021"
[workspace]
[lib]
path = "lib.rs"
[features]
default = []
std = ["ark-relations/std", "dep:tracing", "dep:tracing-subscriber", "dep:ark-bls12-381"]
[dependencies]
ark-relations = {{ path = {package}, default-features = false }}
tracing = {{ version = "=0.1.44", default-features = false, features = ["std"], optional = true }}
tracing-subscriber = {{ version = "=0.3.23", default-features = false, features = ["registry", "std"], optional = true }}
ark-bls12-381 = {{ version = "=0.4.0", default-features = false, features = ["scalar_field"], optional = true }}
''')
        shutil.copyfile(SUPPORT / "constraint_smoke.rs", root / "lib.rs")
        shutil.copyfile(NATIVE / "Cargo.lock", root / "Cargo.lock")
        flags = ["--offline"] if offline else []
        subprocess.run(["cargo", "metadata", *flags, "--format-version", "1"], cwd=root, check=True, stdout=subprocess.DEVNULL, timeout=120)
        subprocess.run(["cargo", "check", *flags, "--locked", "--no-default-features"], cwd=root, check=True, timeout=300)
        subprocess.run(["cargo", "test", *flags, "--locked", "--features", "std", "--lib"], cwd=root, check=True, timeout=300)
        return {"temporary_smoke_lock_sha256": digest(root / "Cargo.lock"), "scope": "no-default compile plus two actual std Registry/constraint tests; test-only Registry dependencies are not the native runtime graph"}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--smoke", action="store_true")
    parser.add_argument("--offline", action="store_true")
    args = parser.parse_args()
    provenance = verify()
    result = {"provenance": "pass", "archive_sha256": provenance["archive_sha256"], "upstream_audit": False}
    if args.smoke:
        result["smoke"] = smoke(args.offline)
    print(json.dumps(result, indent=2))


if __name__ == "__main__":
    main()
