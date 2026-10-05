#!/usr/bin/env python3
# sq-tmyw: assert every package URL in a generated CycloneDX SBOM is the
# canonical, host-independent cargo form.
#
# WHY: cargo-cyclonedx 0.5.9 decorates workspace / path-dependency purls with the CI
# runner's filesystem layout — `?download_url=file://<abs-path>` and, on the root
# build-target sub-components, a trailing `#src/<file>` subpath. `scripts/sbom-normalize.jq`
# strips that decoration so the PUBLISHED SBOM carries only canonical
# `pkg:cargo/<name>@<version>` purls (gaps GS-6 / GS-7, sq-toze.30 + sq-uujh). That
# normalizer is the load-bearing control, and it is hand-written against the exact
# decoration shapes 0.5.9 emits. A FUTURE cargo-cyclonedx upgrade could re-introduce a
# DIFFERENT decoration (a new qualifier, a re-encoded subpath, a `?repository_url=…`,
# etc.) that the normalizer's pattern does not match — silently re-leaking a host path
# or a non-canonical purl into the released/uploaded SBOM with no one noticing.
#
# This check is the BACKSTOP: it parses the *normalized* SBOM(s) and asserts that EVERY
# `purl` carried by EVERY cargo component (the top-level `components[]`, the
# `metadata.component`, and any nested build-target sub-components) matches
#     ^pkg:cargo/[^?#]+@[^?#]+$
# i.e. a bare `pkg:cargo/<name>@<version>` with NO query (`?…`) and NO fragment (`#…`).
# If a future tool bump re-decorates a purl, this fails loudly and names the offending
# purls, so the normalizer must be extended (or the upstream fix adopted) before the
# decorated SBOM can ship. Run it on the SAME SBOM the publication path produces (after
# `scripts/sbom-normalize.jq`); see `.github/workflows/supply-chain.yml#sbom-purl-canonical`.
#
# Scope: ONLY `pkg:cargo/…` purls are asserted-canonical. A non-cargo purl (none today)
# is reported as UNEXPECTED rather than silently accepted — this is a Rust workspace SBOM
# and a stray non-cargo purl would itself be a finding. (The npm/JS SBOM has its own
# generator-side `--validate`; see compliance/sbom/.)
#
# Exit codes: 0 = every cargo purl canonical; 1 = a non-canonical / unexpected purl
# (the offenders are printed); 2 = usage / parse error / no SBOM found.
#
# Run:  python3 scripts/check-sbom-purl-canonical.py path/to/*.cdx.json
#       python3 scripts/check-sbom-purl-canonical.py            # globs **/*.cdx.json
# (stdlib only — json/re/glob on the runner's Python 3.12.)

from __future__ import annotations

import argparse
import glob
import json
import re
import sys
from pathlib import Path

# The canonical cargo purl: pkg:cargo/<name>@<version>, NO query, NO fragment.
# `[^?#]+` for both name and version forbids any `?…` qualifier or `#…` subpath —
# exactly the decoration `scripts/sbom-normalize.jq` is responsible for stripping.
CANONICAL_CARGO_PURL = re.compile(r"^pkg:cargo/[^?#]+@[^?#]+$")


class CheckError(Exception):
    """Parse / usage failure (distinct from a non-canonical-purl finding)."""


def _walk_purls(node: object) -> list[str]:
    """Every `purl` string anywhere in the document (components, metadata.component,
    nested build-target sub-components). Recursion is over dicts/lists so it is robust
    to where in the tree a component sits."""
    out: list[str] = []
    if isinstance(node, dict):
        purl = node.get("purl")
        if isinstance(purl, str):
            out.append(purl)
        for v in node.values():
            out.extend(_walk_purls(v))
    elif isinstance(node, list):
        for v in node:
            out.extend(_walk_purls(v))
    return out


def evaluate(purls: list[str]) -> tuple[bool, list[str]]:
    """Pure function over a list of purl strings (so the self-test can drive it without
    touching the filesystem). Returns (ok, lines).

    ok is True iff every `pkg:cargo/…` purl is canonical AND no non-cargo purl appears.
    """
    lines: list[str] = []
    if not purls:
        # An SBOM with zero purls is itself suspect (every component should carry one).
        return False, ["No purls found in SBOM — expected one per component."]

    non_canonical = sorted(
        p for p in set(purls)
        if p.startswith("pkg:cargo/") and not CANONICAL_CARGO_PURL.match(p)
    )
    non_cargo = sorted(p for p in set(purls) if not p.startswith("pkg:cargo/"))

    if non_canonical:
        lines.append(
            "NON-CANONICAL cargo purl(s) (expected ^pkg:cargo/<name>@<version>$ with "
            "no ?query and no #fragment — a tool upgrade likely re-introduced purl "
            "decoration; extend scripts/sbom-normalize.jq or adopt the upstream fix):"
        )
        lines += [f"  - {p}" for p in non_canonical]
    if non_cargo:
        lines.append(
            "UNEXPECTED non-cargo purl(s) in a Rust-workspace SBOM (investigate — this "
            "check asserts only cargo purls; a stray purl is itself a finding):"
        )
        lines += [f"  - {p}" for p in non_cargo]

    if non_canonical or non_cargo:
        return False, lines

    lines.append(
        f"All {len(purls)} purl(s) ({len(set(purls))} unique) are canonical "
        "pkg:cargo/<name>@<version>."
    )
    return True, lines


def check_file(path: Path) -> tuple[bool, list[str]]:
    try:
        doc = json.loads(path.read_text(encoding="utf-8"))
    except FileNotFoundError as e:
        raise CheckError(f"SBOM not found: {path}") from e
    except json.JSONDecodeError as e:
        raise CheckError(f"SBOM is not valid JSON ({path}): {e}") from e
    ok, lines = evaluate(_walk_purls(doc))
    return ok, [f"{path}: {lines[0]}"] + lines[1:] if lines else (ok, [])


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(
        description="Assert every cargo purl in a normalized CycloneDX SBOM is the "
        "canonical pkg:cargo/<name>@<version> form (sq-tmyw)."
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
        # A literal existing path is used as-is; otherwise treat it as a glob.
        p = Path(pat)
        if p.is_file():
            files.append(p)
        else:
            files.extend(Path(m) for m in glob.glob(pat, recursive=True))
    files = sorted(set(files))

    # A VEX (supply-chain/*.vex.cdx.json) carries no component purls; skip pure-VEX docs
    # so a glob that catches one doesn't false-fail. We only judge files that have purls.
    if not files:
        print(
            "check-sbom-purl-canonical: ERROR: no SBOM files matched "
            f"{patterns!r}",
            file=sys.stderr,
        )
        return 2

    overall_ok = True
    any_judged = False
    for f in files:
        try:
            doc = json.loads(f.read_text(encoding="utf-8"))
        except (FileNotFoundError, json.JSONDecodeError) as e:
            print(f"check-sbom-purl-canonical: ERROR: {f}: {e}", file=sys.stderr)
            return 2
        purls = _walk_purls(doc)
        if not purls:
            print(f"{f}: no purls (VEX or non-component doc) — skipped.")
            continue
        any_judged = True
        ok, lines = evaluate(purls)
        out = sys.stdout if ok else sys.stderr
        print(f"{f}: {lines[0]}", file=out)
        for line in lines[1:]:
            print(line, file=out)
        overall_ok = overall_ok and ok

    if not any_judged:
        print(
            "check-sbom-purl-canonical: ERROR: no SBOM with component purls found "
            f"in {patterns!r}",
            file=sys.stderr,
        )
        return 2

    return 0 if overall_ok else 1


if __name__ == "__main__":
    raise SystemExit(main())
