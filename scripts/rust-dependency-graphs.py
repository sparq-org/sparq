#!/usr/bin/env python3
"""[GPT-6] Run dependency gates over each independently locked Rust workspace."""
from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import subprocess
import sys
import tomllib

ROOT = Path(__file__).resolve().parent.parent
MANIFESTS = (
    Path("Cargo.toml"),
    Path("zk/sparql-evaluator/Cargo.toml"),
    Path("zk/sparql-evaluator/methods/guest/Cargo.toml"),
)
ACTIONS = ("deny-integrity", "deny-advisories", "fetch", "vet", "sbom", "sbom-paths", "patch-policy")


def command(action: str, manifest: Path, root: Path = ROOT) -> list[str]:
    cargo = os.environ.get("CARGO", "cargo")
    path = str(manifest)
    if action.startswith("deny-"):
        checks = ["advisories"] if action == "deny-advisories" else ["bans", "sources", "licenses"]
        return [cargo, "deny", "--manifest-path", path, "--config", str(root / "deny.toml"), "--locked", "check", *checks]
    if action == "fetch":
        return [cargo, "fetch", "--manifest-path", path, "--locked"]
    if action == "vet":
        # Vet's --locked pins imports, not Cargo.lock. Pin both independently.
        return [cargo, "vet", "--manifest-path", path, "--store-path", "supply-chain",
                "--locked", "--frozen", "--cargo-arg=--locked", "--no-minimize-exemptions"]
    if action == "sbom":
        return [cargo, "cyclonedx", "--manifest-path", path, "--all", "--all-features",
                "--target", "all", "--format", "json", "--spec-version", "1.5"]
    raise ValueError(action)


def patch_policy(root: Path) -> None:
    """Upstream audits are necessary, but do not attest the modified local bytes."""
    metadata = json.loads((root / "vendor/zk-sdk/UPSTREAM.json").read_text())
    policies = tomllib.loads((root / "supply-chain/config.toml").read_text()).get("policy", {})
    for package in metadata["packages"]:
        if policies.get(package["name"], {}).get("audit-as-crates-io") is not True:
            raise ValueError(f"vendored {package['name']} lacks explicit upstream audit policy; "
                             "path dependencies must not silently count as audited first-party source")
    # This requires an independent source review of the pinned patch delta too;
    # it is not a cryptographic attestation that such a review occurred.
    subprocess.run([sys.executable, "vendor/zk-sdk/verify.py"], cwd=root, check=True)


def sbom_paths(root: Path) -> list[Path]:
    paths = []
    for manifest in MANIFESTS:
        metadata = json.loads(subprocess.check_output([
            os.environ.get("CARGO", "cargo"), "metadata", "--manifest-path", str(manifest),
            "--locked", "--offline", "--no-deps", "--format-version", "1",
        ], cwd=root))
        members = set(metadata["workspace_members"])
        if not members:
            raise ValueError(f"empty workspace member inventory: {manifest}")
        found = set()
        for package in metadata["packages"]:
            if package["id"] not in members:
                continue
            found.add(package["id"])
            path = Path(package["manifest_path"]).parent / f"{package['name']}.cdx.json"
            relative = path.relative_to(root)
            content = json.loads(path.read_text())  # Missing/empty output fails closed.
            if content.get("bomFormat") != "CycloneDX" or content.get("metadata", {}).get("component", {}).get("name") != package["name"]:
                raise ValueError(f"wrong SBOM component identity: {relative}")
            paths.append(relative)
        if found != members:
            raise ValueError(f"incomplete workspace member metadata: {manifest}")
    return sorted(set(paths))


def run_graphs(action: str, root: Path = ROOT) -> int:
    failed = False
    for manifest in MANIFESTS:
        lock = root / manifest.with_name("Cargo.lock")
        try:
            (root / manifest).read_bytes()
            before = lock.read_bytes()
            print(f"Dependency graph: {manifest} ({action})", flush=True)
            env = os.environ.copy()
            if action == "sbom":
                # cyclonedx has no --locked switch. Fetch first, disable its network,
                # and reject any lockfile change rather than publishing a new graph.
                env["CARGO_NET_OFFLINE"] = "true"
            result = subprocess.run(command(action, manifest, root), cwd=root, env=env, check=False)
            if result.returncode != 0:
                failed = True
            if lock.read_bytes() != before:
                raise ValueError(f"dependency operation changed {lock.relative_to(root)}")
        except (OSError, ValueError) as error:
            print(f"ERROR: {error}", file=sys.stderr)
            failed = True
    return int(failed)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=ACTIONS)
    args = parser.parse_args()
    try:
        if args.action == "patch-policy":
            patch_policy(ROOT)
            return 0
        if args.action == "sbom-paths":
            paths = sbom_paths(ROOT)
            sys.stdout.buffer.write(b"\0".join(os.fsencode(p) for p in paths) + b"\0")
            return 0
        return run_graphs(args.action)
    except (OSError, ValueError, subprocess.CalledProcessError) as error:
        print(f"ERROR: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
