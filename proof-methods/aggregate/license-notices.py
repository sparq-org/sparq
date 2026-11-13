#!/usr/bin/env python3
"""Collect dependency licence notices for the generated Node distribution."""
import json
from pathlib import Path
import subprocess
import sys

manifest, output = map(Path, sys.argv[1:])
metadata = json.loads(subprocess.check_output([
    "cargo", "metadata", "--manifest-path", str(manifest), "--locked", "--offline",
    "--format-version", "1", "--features", "wasm", "--filter-platform", "wasm32-unknown-unknown",
]))
packages = {p["id"]: p for p in metadata["packages"]}
nodes = {n["id"]: n for n in metadata["resolve"]["nodes"]}
root = metadata["resolve"]["root"]
if root is None:
    raise SystemExit("dependency metadata has no selected package")
pending, seen = [root], set()
while pending:
    package = pending.pop()
    if package not in seen:
        seen.add(package)
        pending.extend(d["pkg"] for d in nodes[package]["deps"])
sections = ["Third-party licence notices\n"]
for package in sorted((packages[p] for p in seen if p != root), key=lambda p: (p["name"], p["version"])):
    directory = Path(package["manifest_path"]).parent
    files = {p for p in directory.iterdir() if p.is_file() and p.name.lower().startswith(("license", "licence", "copying", "copyright", "notice"))}
    if package.get("license_file"):
        files.add(directory / package["license_file"])
    if not files:
        raise SystemExit(f"missing licence notices: {package['name']}")
    if any(not path.resolve().is_relative_to(directory.resolve()) for path in files):
        raise SystemExit(f"licence path outside package: {package['name']}")
    spdx = package.get("license") or "see supplied licence file"
    sections.append(f"\n{package['name']} {package['version']}\nSPDX: {spdx}\n")
    for path in sorted(files):
        sections.append(f"\n--- {path.name} ---\n{path.read_text()}\n")
output.write_text("\n".join(sections))
