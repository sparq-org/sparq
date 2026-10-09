#!/usr/bin/env python3
"""[GPT-6] Prepare and explicitly fetch only the pinned diagnostic baseline graph."""
import argparse
import json
import os
from pathlib import Path
import subprocess
import time
import tomllib

import verify


def baseline_packages(native=verify.NATIVE):
    """Select the transitive closure from reviewed lock objects, without resolving."""
    current = tomllib.loads((native / "Cargo.lock").read_text())["package"]
    removed = json.loads((native / "vendor-support/wasmer/lock-delta.json").read_text())["removed_registry_packages"]
    packages = current + removed
    selected = {}

    def visit(name, version=None):
        matches = [p for p in packages if p["name"] == name and (version is None or p["version"] == version)]
        verify.require(len(matches) == 1, "ambiguous baseline dependency")
        package = matches[0]
        key = (package["name"], package["version"])
        if key in selected:
            return
        verify.require(package.get("source") == "registry+https://github.com/rust-lang/crates.io-index"
                       and len(package.get("checksum", "")) == 64, "baseline must retain registry identity")
        selected[key] = package
        for dependency in package.get("dependencies", []):
            parts = dependency.split()
            verify.require(len(parts) in (1, 2), "unexpected baseline dependency source")
            visit(parts[0], parts[1] if len(parts) == 2 else None)

    visit("proc-macro-error2", "2.0.1")
    verify.require(len(selected) == 6, "unexpected baseline closure")
    return [selected[key] for key in sorted(selected)]


def prepare(output):
    verify.verify()
    output.mkdir(parents=True, exist_ok=False)
    packages = baseline_packages()
    # This ephemeral package is used only by cargo fetch/metadata, never built.
    # Explicit syn retains the native lock's printing/quote dependency edge;
    # proc-macro-error2 alone enables a smaller feature subset of that release.
    manifest = ('[package]\nname = "sparq-wasmer-baseline-fetch"\nversion = "0.0.0"\nedition = "2021"\n'
                '[workspace]\n[lib]\npath = ' + json.dumps(str(verify.SUPPORT / "layout.rs")) + '\n'
                '[dependencies]\nproc-macro-error2 = "=2.0.1"\nsyn = "=2.0.119"\n')
    (output / "Cargo.toml").write_text(manifest)
    lock = '# [GPT-6] Exact test-only registry closure from the native lock and reviewed removed nodes.\nversion = 4\n'
    for package in packages + [{"name": "sparq-wasmer-baseline-fetch", "version": "0.0.0", "dependencies": ["proc-macro-error2", "syn"]}]:
        lock += '\n[[package]]\n'
        for key in ("name", "version", "source", "checksum"):
            if key in package:
                lock += key + ' = ' + json.dumps(package[key]) + '\n'
        if package.get("dependencies"):
            # The selected closure has one version per package. Cargo's canonical
            # lock spelling therefore omits the version disambiguator.
            dependencies = [d.split()[0] for d in package["dependencies"]]
            lock += 'dependencies = ' + json.dumps(dependencies) + '\n'
    (output / "Cargo.lock").write_text(lock)
    record = {"scope": "Ephemeral fetch-only baseline, never part of the native runtime graph",
              "registry_packages": packages, "native_lock_sha256": verify.sha((verify.NATIVE / "Cargo.lock").read_bytes()),
              "manifest_sha256": verify.sha(manifest.encode()), "lock_sha256": verify.sha(lock.encode())}
    (output / "inputs.json").write_text(json.dumps(record, indent=2) + "\n")
    return record


def fetch(output):
    record = prepare(output)
    command = ["cargo", "fetch", "--locked", "--manifest-path", str(output / "Cargo.toml")]
    start = time.monotonic()
    with (output / "fetch.stdout").open("xb") as stdout, (output / "fetch.stderr").open("xb") as stderr:
        child = subprocess.run(command, env=os.environ, stdout=stdout, stderr=stderr, timeout=300)
    receipt = {"command": command, "exit_code": child.returncode, "elapsed_seconds": time.monotonic() - start,
               "manifest_sha256_after": verify.sha((output / "Cargo.toml").read_bytes()),
               "lock_sha256_after": verify.sha((output / "Cargo.lock").read_bytes()), "compiled": False}
    (output / "fetch.json").write_text(json.dumps(receipt, indent=2) + "\n")
    verify.require(child.returncode == 0, "pinned baseline fetch failed")
    verify.require(receipt["manifest_sha256_after"] == record["manifest_sha256"]
                   and receipt["lock_sha256_after"] == record["lock_sha256"], "baseline inputs changed")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--fetch", action="store_true", help="Explicit hosted network fetch; no compilation")
    args = parser.parse_args()
    (fetch if args.fetch else prepare)(args.output.resolve())
