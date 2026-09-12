#!/usr/bin/env python3
# [OPUS-4.8] sq-toze.29 (GS-5): CI drift-check — VEX ignore-list vs deny.toml.
# Authored by Opus 4.8 (Fable unavailable; flag for re-review when Fable returns).
#
# WHY (the GS-5 concern): the supply-chain story has two places that name the
# advisories sparq deliberately tolerates:
#   • deny.toml  [advisories].ignore   — the ACTUAL CI GATE. cargo-deny fails the
#                                        PR on any advisory not on this list.
#   • supply-chain/vex.cdx.json        — the PUBLISHED VEX (Vulnerability
#                                        Exploitability eXchange) that tells a
#                                        downstream consumer the scope of each
#                                        tolerated advisory and any established
#                                        exploitability analysis.
# If an advisory is added to one but not the other, the public VEX and the real
# gate DISAGREE — either the SBOM/VEX claims an exploitability verdict for an
# advisory the gate no longer suppresses, or the gate silently suppresses an
# advisory the VEX never explains. Both are integrity problems. Until now the two
# were kept in step by a "KEEP THIS IN SYNC" comment + manual review; this script
# makes the invariant a CI gate.
#
# SOURCE OF TRUTH (direction): deny.toml [advisories].ignore is AUTHORITATIVE — it
# is the list cargo-deny actually enforces, so it defines which advisories the
# product tolerates. The VEX MUST carry exactly one `vulnerabilities[].id` entry
# per ignored advisory (a downstream-facing exploitability statement for each).
# The check is therefore a SET EQUALITY: the deny.toml ignore-id set must equal
# the VEX vulnerability-id set. The only tolerated difference is an entry listed
# in the JUSTIFIED-DRIFT allow-list below, with a written reason (there are none
# today, and there should normally be none — the allow-list exists so a genuinely
# one-sided, deliberately-justified entry can be recorded honestly rather than
# silencing the whole check).
#
# Exit codes:  0 = in sync (or every difference is justified);  1 = unjustified
# drift (the diff is printed);  2 = usage / parse error.
#
# Run:  python3 scripts/check-vex-deny-drift.py
#       python3 scripts/check-vex-deny-drift.py --deny deny.toml \
#               --vex supply-chain/vex.cdx.json
# (stdlib only — tomllib is 3.11+, present on the CI runner's Python 3.12.)

from __future__ import annotations

import argparse
import json
import re
import sys
import tomllib
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent

# Advisory-id shape we accept on either side: RUSTSEC-YYYY-NNNN or CVE-YYYY-NNNN.
# Used only to validate that what we extracted *looks* like an advisory id, so a
# malformed entry is reported rather than silently compared.
_ID_RE = re.compile(r"^(RUSTSEC|CVE)-\d{4}-\d{3,}$")

# JUSTIFIED one-sided entries. Maps an advisory id to a human reason. An id here is
# allowed to appear on ONLY ONE side without failing the gate. This MUST stay
# empty unless there is a real, documented reason an advisory belongs in deny.toml
# but not the VEX (or vice-versa); each entry is reviewed like a deny.toml ignore.
# [OPUS-4.8] Empty today: deny.toml and the VEX are 1:1 in sync.
JUSTIFIED_DRIFT: dict[str, str] = {}


class DriftError(Exception):
    """Parse/usage failure (distinct from a drift finding)."""


def deny_ignore_ids(deny_path: Path) -> set[str]:
    """Advisory ids in deny.toml [advisories].ignore.

    Robust to both cargo-deny ignore forms: a bare string id, or the structured
    ``{ id = "RUSTSEC-…", reason = "…" }`` table sparq uses. Parsed with tomllib
    so reason text / comments can never be mistaken for an id (a grep would).
    """
    try:
        doc = tomllib.loads(deny_path.read_text(encoding="utf-8"))
    except FileNotFoundError as e:
        raise DriftError(f"deny.toml not found: {deny_path}") from e
    except tomllib.TOMLDecodeError as e:
        raise DriftError(f"deny.toml is not valid TOML: {e}") from e

    ignore = doc.get("advisories", {}).get("ignore", [])
    ids: set[str] = set()
    for entry in ignore:
        if isinstance(entry, str):
            ids.add(entry)
        elif isinstance(entry, dict) and "id" in entry:
            ids.add(str(entry["id"]))
        else:
            raise DriftError(
                f"unrecognised deny.toml [advisories].ignore entry: {entry!r}"
            )
    return ids


def vex_ids(vex_path: Path) -> set[str]:
    """Advisory ids in the VEX vulnerabilities[].id set."""
    try:
        doc = json.loads(vex_path.read_text(encoding="utf-8"))
    except FileNotFoundError as e:
        raise DriftError(f"VEX not found: {vex_path}") from e
    except json.JSONDecodeError as e:
        raise DriftError(f"VEX is not valid JSON: {e}") from e

    vulns = doc.get("vulnerabilities", [])
    ids: set[str] = set()
    for v in vulns:
        if not isinstance(v, dict) or "id" not in v:
            raise DriftError(f"VEX vulnerability entry missing `id`: {v!r}")
        ids.add(str(v["id"]))
    return ids


def evaluate(deny_ids: set[str], vexed_ids: set[str]) -> tuple[bool, list[str]]:
    """Return (ok, lines). `ok` is True iff there is no UNJUSTIFIED drift.

    Pure function over the two id sets + the JUSTIFIED_DRIFT allow-list, so the
    self-test can drive it without touching the filesystem.
    """
    lines: list[str] = []

    # Sanity: every extracted id must look like an advisory id, on both sides.
    malformed = sorted(
        i for i in (deny_ids | vexed_ids) if not _ID_RE.match(i)
    )
    if malformed:
        lines.append("Malformed advisory id(s) (expected RUSTSEC-/CVE-YYYY-NNNN):")
        lines += [f"  - {i}" for i in malformed]
        return False, lines

    only_deny = deny_ids - vexed_ids   # gated but not explained by the VEX
    only_vex = vexed_ids - deny_ids    # explained by the VEX but not gated

    # An id is tolerated one-sided ONLY if it is in the justified allow-list.
    unjustified_deny = sorted(only_deny - JUSTIFIED_DRIFT.keys())
    unjustified_vex = sorted(only_vex - JUSTIFIED_DRIFT.keys())

    if not unjustified_deny and not unjustified_vex:
        n = len(deny_ids & vexed_ids)
        lines.append(
            f"VEX ↔ deny.toml in sync: {n} advisory id(s) match 1:1."
        )
        justified = sorted((only_deny | only_vex) & JUSTIFIED_DRIFT.keys())
        for i in justified:
            lines.append(f"  (justified one-sided: {i} — {JUSTIFIED_DRIFT[i]})")
        return True, lines

    lines.append("DRIFT: deny.toml [advisories].ignore and the VEX disagree.")
    lines.append(
        "  Source of truth is deny.toml (the cargo-deny gate); the VEX "
        "(supply-chain/vex.cdx.json) MUST carry one vulnerabilities[].id per "
        "ignored advisory."
    )
    if unjustified_deny:
        lines.append(
            "  In deny.toml [advisories].ignore but MISSING from the VEX "
            "(add a vulnerabilities[] entry explaining exploitability, or remove "
            "the deny.toml ignore):"
        )
        lines += [f"    - {i}" for i in unjustified_deny]
    if unjustified_vex:
        lines.append(
            "  In the VEX but NOT in deny.toml [advisories].ignore (the VEX "
            "explains an advisory the gate no longer suppresses — add it back to "
            "deny.toml, or remove the VEX vulnerabilities[] entry):"
        )
        lines += [f"    - {i}" for i in unjustified_vex]
    lines.append(
        "  If a one-sided entry is genuinely intended, record it with a reason "
        "in JUSTIFIED_DRIFT in scripts/check-vex-deny-drift.py."
    )
    return False, lines


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(
        description="Fail if deny.toml [advisories].ignore and the VEX "
        "vulnerabilities[].id sets drift apart (GS-5 / sq-toze.29)."
    )
    ap.add_argument(
        "--deny", default=str(REPO_ROOT / "deny.toml"), help="path to deny.toml"
    )
    ap.add_argument(
        "--vex",
        default=str(REPO_ROOT / "supply-chain" / "vex.cdx.json"),
        help="path to the CycloneDX VEX document",
    )
    args = ap.parse_args(argv)

    try:
        d = deny_ignore_ids(Path(args.deny))
        v = vex_ids(Path(args.vex))
    except DriftError as e:
        print(f"check-vex-deny-drift: ERROR: {e}", file=sys.stderr)
        return 2

    ok, lines = evaluate(d, v)
    out = sys.stdout if ok else sys.stderr
    for line in lines:
        print(line, file=out)
    return 0 if ok else 1


if __name__ == "__main__":
    raise SystemExit(main())
