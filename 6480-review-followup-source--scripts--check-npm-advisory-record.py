#!/usr/bin/env python3
# [OPUS-5] #3767: CI drift-check — supply-chain/npm-advisories.md vs package-lock.json.
#
# WHY. Dependabot repeatedly concludes `security_update_not_possible` on this repo's
# npm graph: a patched release exists for `brace-expansion` / `postcss`, but the
# reachable update path cannot land it, so no PR is opened and the alert stays open.
# The honest response is NOT a `dependabot.yml` ignore (that would suppress the future
# patch notification too) — it is a written disposition plus a TRIPWIRE, so the moment
# the graph moves off the recorded state a human revisits the disposition.
#
# This is that tripwire. `supply-chain/npm-advisories.md` carries a machine-readable
# block naming, for each tolerated npm advisory, every instance in the lock and the
# thing that pins it. This script asserts that record against the LIVE lock:
#
#   • path-set equality per package — a new (or vanished) nested copy REDs, so a fifth
#     `brace-expansion` cannot appear unrecorded;
#   • each recorded instance still resolves at the recorded version;
#   • each recorded pin still holds — for `kind: package`, the pinning lock entry
#     exists at the recorded version and still declares the recorded range; for
#     `kind: root_override`, the root package.json `overrides` entry still carries the
#     recorded value (that is the real pin for `postcss`).
#
# DIRECTION OF THE GATE. A RED here is normally the GOOD news: the graph moved, most
# likely because a patch finally became reachable. The fix is to update or delete the
# record and close #3767 — never to relax the check. It fires on the remediation PR by
# design, exactly as the VEX/deny drift gate does for the cargo side.
#
# It deliberately does NOT parse advisory feeds or reach the network: it can only tell
# you the recorded picture is stale, not whether a package is vulnerable. Advisory
# detection stays with Dependabot (see the scope caveat in the record).
#
# Exit codes:  0 = record matches the lock;  1 = drift (every finding is printed);
#              2 = usage / parse error.
#
# Run:  python3 scripts/check-npm-advisory-record.py
#       python3 scripts/check-npm-advisory-record.py --record supply-chain/npm-advisories.md \
#               --lock package-lock.json --manifest package.json
# (stdlib only — json/re/pathlib on the CI runner's Python 3.12.)

from __future__ import annotations

import argparse
import json
import re
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent

RECORD_PATH = Path("supply-chain/npm-advisories.md")
LOCK_PATH = Path("package-lock.json")
MANIFEST_PATH = Path("package.json")

# The record block is delimited by HTML-comment sentinels so the surrounding prose can
# be rewritten freely without breaking the parse. A ```json fence inside them carries
# the payload.
_BLOCK_RE = re.compile(
    r"<!--\s*npm-advisory-record:begin\s*-->(?P<body>.*?)<!--\s*npm-advisory-record:end\s*-->",
    re.DOTALL,
)
_FENCE_RE = re.compile(r"```(?:json)?\s*(?P<json>\{.*\})\s*```", re.DOTALL)

_PIN_KINDS = ("package", "root_override")
# npm manifest fields a pinning entry may declare the dependency in. `sharp` is an
# OPTIONAL dependency of `next`, so a `dependencies`-only lookup would silently find
# nothing and compare None == None if the record omitted the range.
_DEP_FIELDS = (
    "dependencies",
    "optionalDependencies",
    "devDependencies",
    "peerDependencies",
)


class RecordError(Exception):
    """Parse/usage failure (distinct from a drift finding)."""


def parse_record(record_path: Path) -> dict:
    """Extract and validate the machine-readable block from the markdown record."""
    try:
        text = record_path.read_text(encoding="utf-8")
    except FileNotFoundError as e:
        raise RecordError(f"record not found: {record_path}") from e

    block = _BLOCK_RE.search(text)
    if block is None:
        raise RecordError(
            f"{record_path}: no <!-- npm-advisory-record:begin/end --> block found"
        )
    fence = _FENCE_RE.search(block.group("body"))
    if fence is None:
        raise RecordError(f"{record_path}: record block contains no ```json fence")
    try:
        doc = json.loads(fence.group("json"))
    except json.JSONDecodeError as e:
        raise RecordError(f"{record_path}: record block is not valid JSON: {e}") from e

    packages = doc.get("packages")
    if not isinstance(packages, list) or not packages:
        raise RecordError(f"{record_path}: record has no non-empty `packages` list")
    for pkg in packages:
        name = pkg.get("name")
        if not isinstance(name, str) or not name:
            raise RecordError(f"{record_path}: a package entry has no `name`")
        instances = pkg.get("instances")
        if not isinstance(instances, list) or not instances:
            raise RecordError(f"{record_path}: `{name}` has no non-empty `instances`")
        for inst in instances:
            for key in ("path", "version", "pinned_by"):
                if key not in inst:
                    raise RecordError(f"{record_path}: `{name}` instance missing `{key}`")
            pin = inst["pinned_by"]
            kind = pin.get("kind")
            if kind not in _PIN_KINDS:
                raise RecordError(
                    f"{record_path}: `{name}` instance at {inst['path']} has unknown "
                    f"pin kind {kind!r} (expected one of {_PIN_KINDS})"
                )
            if kind == "package" and pin.get("field", "dependencies") not in _DEP_FIELDS:
                raise RecordError(
                    f"{record_path}: `{name}` instance at {inst['path']} names unknown "
                    f"pin field {pin.get('field')!r} (expected one of {_DEP_FIELDS})"
                )
    return doc


def load_lock(lock_path: Path) -> dict[str, dict]:
    """The lockfile's `packages` map (path -> entry). lockfileVersion 2/3."""
    try:
        doc = json.loads(lock_path.read_text(encoding="utf-8"))
    except FileNotFoundError as e:
        raise RecordError(f"lockfile not found: {lock_path}") from e
    except json.JSONDecodeError as e:
        raise RecordError(f"{lock_path}: not valid JSON: {e}") from e
    packages = doc.get("packages")
    if not isinstance(packages, dict):
        raise RecordError(
            f"{lock_path}: no `packages` map (lockfileVersion 2+ required, got "
            f"{doc.get('lockfileVersion')!r})"
        )
    return packages


def load_root_overrides(manifest_path: Path) -> dict:
    try:
        doc = json.loads(manifest_path.read_text(encoding="utf-8"))
    except FileNotFoundError as e:
        raise RecordError(f"manifest not found: {manifest_path}") from e
    except json.JSONDecodeError as e:
        raise RecordError(f"{manifest_path}: not valid JSON: {e}") from e
    overrides = doc.get("overrides", {})
    return overrides if isinstance(overrides, dict) else {}


def lock_instances(packages: dict[str, dict], name: str) -> set[str]:
    """Every install path of `name` in the lock.

    A path names the package iff its final `node_modules/<name>` segment matches — so
    `node_modules/@tailwindcss/postcss` is NOT an instance of `postcss`, and a nested
    `.../node_modules/postcss` is.
    """
    top = f"node_modules/{name}"
    nested = f"/node_modules/{name}"
    return {p for p in packages if p == top or p.endswith(nested)}


def evaluate(record: dict, packages: dict[str, dict], overrides: dict) -> tuple[bool, list[str]]:
    """Pure comparison. Returns (ok, report lines)."""
    findings: list[str] = []
    lines: list[str] = []

    for pkg in record["packages"]:
        name = pkg["name"]
        recorded = {inst["path"]: inst for inst in pkg["instances"]}
        actual = lock_instances(packages, name)

        for path in sorted(actual - recorded.keys()):
            findings.append(
                f"{name}: UNRECORDED instance in the lock at `{path}` "
                f"(version {packages[path].get('version')!r}) — the graph gained a copy"
            )
        for path in sorted(recorded.keys() - actual):
            findings.append(
                f"{name}: recorded instance `{path}` is GONE from the lock — the graph moved"
            )

        for path in sorted(recorded.keys() & actual):
            inst = recorded[path]
            got = packages[path].get("version")
            if got != inst["version"]:
                findings.append(
                    f"{name}: `{path}` is version {got!r}, record says {inst['version']!r}"
                )
            findings.extend(_check_pin(name, path, inst["pinned_by"], packages, overrides))

        lines.append(f"  {name}: {len(actual)} instance(s), all recorded")

    if findings:
        return False, [
            "npm advisory record is STALE — the lock no longer matches "
            f"{RECORD_PATH}:",
            *(f"  ✗ {f}" for f in findings),
            "",
            "This is usually the GOOD signal: the npm graph moved, most likely because a",
            "patched version became reachable. Re-check the open Dependabot alerts, update",
            "or delete the affected entry in the record (and close its tracking issue).",
            "Do NOT relax this check to make it pass.",
        ]
    return True, ["npm advisory record matches the lock:", *lines]


def _check_pin(
    name: str, path: str, pin: dict, packages: dict[str, dict], overrides: dict
) -> list[str]:
    if pin["kind"] == "root_override":
        key = pin.get("key", name)
        got = overrides.get(key)
        if got != pin.get("value"):
            return [
                f"{name}: `{path}` is recorded as pinned by root override "
                f"`overrides.{key}` = {pin.get('value')!r}, but package.json has {got!r}"
            ]
        return []

    # kind == "package": the pinning lock entry must still exist, at the recorded
    # version, still declaring the recorded range on `name`.
    pin_path = pin["path"]
    entry = packages.get(pin_path)
    if entry is None:
        return [f"{name}: `{path}` is recorded as pinned by `{pin_path}`, which is GONE"]
    findings = []
    if entry.get("version") != pin["version"]:
        findings.append(
            f"{name}: pinning package `{pin_path}` is version "
            f"{entry.get('version')!r}, record says {pin['version']!r}"
        )
    field = pin.get("field", "dependencies")
    got_range = (entry.get(field) or {}).get(name)
    if got_range != pin["range"]:
        findings.append(
            f"{name}: pinning package `{pin_path}` declares {field}.{name} = "
            f"{got_range!r}, record says {pin['range']!r}"
        )
    return findings


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--record", type=Path, default=REPO_ROOT / RECORD_PATH)
    ap.add_argument("--lock", type=Path, default=REPO_ROOT / LOCK_PATH)
    ap.add_argument("--manifest", type=Path, default=REPO_ROOT / MANIFEST_PATH)
    args = ap.parse_args(argv)

    try:
        record = parse_record(args.record)
        packages = load_lock(args.lock)
        overrides = load_root_overrides(args.manifest)
    except RecordError as e:
        print(f"error: {e}", file=sys.stderr)
        return 2

    ok, lines = evaluate(record, packages, overrides)
    print("\n".join(lines))
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
