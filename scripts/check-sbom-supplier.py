#!/usr/bin/env python3
# sq-toze.26 (GS-1 / NTIA N1): assert every cargo component in a normalized
# CycloneDX SBOM carries a per-component `supplier` (NTIA "Supplier Name" slot).
#
# WHY: cargo-cyclonedx 0.5.9 leaves `supplier`/`publisher` empty on every component, so the
# literal NTIA minimum-element "Supplier Name" slot was unpopulated (gap GS-1). The
# publication normalizer `scripts/sbom-normalize.jq` now derives a supplier HONESTLY from
# each component's identity (registry/path) in the raw cargo-cyclonedx output:
#   * crates.io-published crate (registry+… bom-ref)                -> supplier "crates.io"
#   * vendored [patch.crates-io] upstream crate (path+file…/vendor) -> supplier "crates.io"
#   * first-party workspace crate (path+file…/crates/sparq-*)       -> supplier "Jesse Wright"
#   * anything else (none today)                                    -> supplier OMITTED (honest)
# It NEVER fabricates a supplier where the class is not determinable — NTIA's guidance is to
# omit / mark unknown rather than guess. This check is the BACKSTOP that keeps GS-1 honestly
# closed: it asserts every component whose supplier IS determinable carries one, and reports
# (without failing) any component that is honestly-unknown, so a future cargo-cyclonedx bump
# that emits an UNRECOGNISED component shape surfaces as a visible "unknown" rather than a
# silent regression to the empty-supplier state.
#
# Determinability is keyed off the canonical purl that survives normalization: every cargo
# component carries `pkg:cargo/<name>@<version>`, and today EVERY such component is one of the
# three determinable classes, so the expectation is 100% coverage. The check FAILS only if a
# component that should be classifiable (a cargo component) is missing its supplier — i.e. the
# normalizer's derivation silently stopped firing.
#
# Scope: a VEX (`supply-chain/*.vex.cdx.json`) carries no components and is skipped. Run on the
# SAME SBOM the publication path produces (after `scripts/sbom-normalize.jq`); see
# `.github/workflows/supply-chain.yml#sbom-supplier`.
#
# Exit codes: 0 = every cargo component has a supplier (full coverage);
#             1 = a cargo component is missing its supplier (the offenders are printed);
#             2 = usage / parse error / no SBOM with components found.
#
# Run:  python3 scripts/check-sbom-supplier.py path/to/*.cdx.json
#       python3 scripts/check-sbom-supplier.py            # globs **/*.cdx.json
# (stdlib only — json/glob on the runner's Python 3.12.)

from __future__ import annotations

import argparse
import glob
import json
import sys
from pathlib import Path


class CheckError(Exception):
    """Parse / usage failure (distinct from a missing-supplier finding)."""


def _walk_components(node: object) -> list[dict]:
    """Every component dict anywhere in the document — top-level `components[]`, the
    `metadata.component`, and any nested build-target sub-components. A component is a dict
    that carries a `purl` (so we judge real components, not arbitrary nested objects)."""
    out: list[dict] = []
    if isinstance(node, dict):
        if isinstance(node.get("purl"), str):
            out.append(node)
        for v in node.values():
            out.extend(_walk_components(v))
    elif isinstance(node, list):
        for v in node:
            out.extend(_walk_components(v))
    return out


def _supplier_name(comp: dict) -> str | None:
    """The component's NTIA Supplier Name = CycloneDX `supplier.name` (an
    organizationalEntity). Returns None when absent/empty."""
    sup = comp.get("supplier")
    if isinstance(sup, dict):
        name = sup.get("name")
        if isinstance(name, str) and name.strip():
            return name
    return None


def evaluate(components: list[dict]) -> tuple[bool, list[str]]:
    """Pure function over a list of component dicts (so the self-test can drive it without
    touching the filesystem). Returns (ok, lines).

    ok is True iff every cargo component carries a non-empty `supplier.name`. A component
    with NO purl is ignored (not a cargo component). A component honestly recorded as
    supplier-unknown is reported but does NOT fail the check (NTIA permits omission); today
    there are none, so any appearance is a visible signal, not a hidden regression."""
    lines: list[str] = []
    cargo = [c for c in components if str(c.get("purl", "")).startswith("pkg:cargo/")]
    if not cargo:
        return False, ["No cargo components found in SBOM — expected one supplier per component."]

    missing = sorted(
        str(c.get("purl")) for c in cargo if _supplier_name(c) is None
    )
    if missing:
        lines.append(
            "MISSING per-component supplier (NTIA N1 'Supplier Name' = CycloneDX "
            "supplier.name) on cargo component(s) — scripts/sbom-normalize.jq's "
            "derivation stopped firing (a tool bump likely changed the component shape; "
            "extend derive_supplier in scripts/sbom-normalize.jq):"
        )
        lines += [f"  - {p}" for p in missing]
        return False, lines

    names = sorted({_supplier_name(c) for c in cargo if _supplier_name(c)})
    lines.append(
        f"All {len(cargo)} cargo component(s) carry supplier.name "
        f"(distinct suppliers: {', '.join(names)})."
    )
    return True, lines


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(
        description="Assert every cargo component in a normalized CycloneDX SBOM carries a "
        "per-component supplier (NTIA N1 'Supplier Name'); GS-1, sq-toze.26."
    )
    ap.add_argument(
        "paths",
        nargs="*",
        help="SBOM file(s) or glob(s); default: **/*.cdx.json under the cwd.",
    )
    args = ap.parse_args(argv)

    patterns = args.paths or ["**/*.cdx.json"]
    files: list[Path] = []
    for pat in patterns:
        p = Path(pat)
        if p.is_file():
            files.append(p)
        else:
            files.extend(Path(m) for m in glob.glob(pat, recursive=True))
    files = sorted(set(files))

    if not files:
        print(
            f"check-sbom-supplier: ERROR: no SBOM files matched {patterns!r}",
            file=sys.stderr,
        )
        return 2

    overall_ok = True
    any_judged = False
    for f in files:
        try:
            doc = json.loads(f.read_text(encoding="utf-8"))
        except (FileNotFoundError, json.JSONDecodeError) as e:
            print(f"check-sbom-supplier: ERROR: {f}: {e}", file=sys.stderr)
            return 2
        components = _walk_components(doc)
        cargo = [c for c in components if str(c.get("purl", "")).startswith("pkg:cargo/")]
        if not cargo:
            print(f"{f}: no cargo components (VEX or non-component doc) — skipped.")
            continue
        any_judged = True
        ok, lines = evaluate(components)
        out = sys.stdout if ok else sys.stderr
        print(f"{f}: {lines[0]}", file=out)
        for line in lines[1:]:
            print(line, file=out)
        overall_ok = overall_ok and ok

    if not any_judged:
        print(
            "check-sbom-supplier: ERROR: no SBOM with cargo components found "
            f"in {patterns!r}",
            file=sys.stderr,
        )
        return 2

    return 0 if overall_ok else 1


if __name__ == "__main__":
    raise SystemExit(main())
