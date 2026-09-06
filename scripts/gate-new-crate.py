#!/usr/bin/env python3
# [OPUS-4.8] Gate G1 — new-crate-completeness (bead sq-ncvq.4, epic sq-ncvq).
# Authored by Opus 4.8 (Fable unavailable; flag for re-review when Fable returns).
#
# This is the PROACTIVE / merge-time half of the maintenance flow-on system
# (research/maintenance-flow-on-automation-design.md §2.1, gate G1). It is the
# COMPLEMENT of the reactive engine in scripts/flow-on.py (PR #220): that engine
# mints follow-on ISSUES after a PR merges; THIS script BLOCKS the PR before it
# merges when a new crate would land without its required maintenance artifacts.
#
# [OPUS-4.8] sq-ncvq.10 doc-sync: this gate is the "Enforced by: **G1**" cell of
# the "new crate" row in the AGENTS.md "Post-batch re-evaluation checklist" table.
# That table row and this docstring are the two halves of the same rule — change
# one and update the other; the divergence is what sq-ncvq.10 exists to prevent.
#
# RULE (G1): when a PR ADDS a new `crates/<x>/Cargo.toml`, fail unless the SAME
# PR also provides, for that crate:
#   (a) a registered benchmark — a `[[benchmark]]` entry in bench/benchmarks.toml
#       whose `source` references `crates/<x>` (matching the reactive rule's
#       `source = crates/<x>` expectation in flow-on-rules.toml); OR the crate is
#       an intentional stub marked `publish = false` in its Cargo.toml (the
#       design's documented stub-exemption escape hatch);
#   (b) a README.md (crates/<x>/README.md) — the template content itself is
#       separately enforced by check-readme-template.py; G1 only requires its
#       PRESENCE so a new crate is never undocumented;
#   (c) a SKILL.md — ONLY if the crate is a PUBLIC surface. A crate is treated as
#       public iff its Cargo.toml does NOT carry `publish = false`. A public crate
#       must have at least one skills/<surface>/SKILL.md touched in the same PR
#       (the AGENTS.md public-API → SKILL.md rule applied to a brand-new surface).
#       A `publish = false` stub is internal-only and exempt from (c) — and, via
#       the same marker, exempt from (a) as well.
#
# ESCAPE HATCH (design §2.1): `publish = false` in the new crate's Cargo.toml
# marks it an intentional stub — exempt from the bench (a) and SKILL (c)
# requirements. The README (b) is still required (a stub still needs a one-line
# "what/why" README).
#
# ESCAPE HATCH 2 (design §2.1 "Escape hatch" column / the AGENTS.md new-crate
# row): a `<!-- flow-on-exempt: reason -->` marker in the new crate's README
# waives G1 for that crate ENTIRELY — the crate carries its own README (that is
# where the marker lives), and the stated reason is the audit trail. The reason
# must be NON-EMPTY: a bare `<!-- flow-on-exempt -->` waives nothing, and main()
# says so loudly rather than silently ignoring it. This marker is read here and
# by the reactive engine (scripts/flow-on.py loads this module and calls
# exempt_crates()), so the two halves share ONE definition of "exempt". Until
# #5701 the marker was documented in AGENTS.md + the design record but read by
# NEITHER script.
#
# DIFF SOURCE: the PR's changed-file list. In CI this is
#   git diff --name-only origin/<base>...HEAD          (changed files)
#   git diff --name-status origin/<base>...HEAD        (to find ADDED files)
# In tests / --dry-run it is read from a fixture file (one path per line, each
# optionally prefixed with a git status letter + tab, e.g. "A\tcrates/x/...").
#
# EXIT: 0 when every newly-added crate satisfies G1 (or there are none); 1 with a
# clear per-crate message naming exactly what is missing otherwise.
#
# Usage:
#   gate-new-crate.py                       # CI: derive diff from git vs origin/<base>
#   gate-new-crate.py --base main           # override base ref
#   gate-new-crate.py --advisory            # warn-only soft-launch (never exit 1)
#   gate-new-crate.py --dry-run --changed-files files.txt   # hermetic (tests)
#
# stdlib-only.

from __future__ import annotations

import argparse
import os
import re
import subprocess
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
BENCH_REGISTRY = REPO_ROOT / "bench" / "benchmarks.toml"

# A line in a status-prefixed diff: "A\tpath", "M\tpath", "R100\told\tnew", etc.
_STATUS_RE = re.compile(r"^([A-Z])\d*\t(.*)$")

# The documented per-crate escape hatch: `<!-- flow-on-exempt: reason -->` in the
# crate README. Single-line only (`.` does not span newlines), reason captured so
# the gate can print WHY the crate was waived.
_EXEMPT_RE = re.compile(r"<!--[ \t]*flow-on-exempt:[ \t]*(.+?)[ \t]*-->")
# A marker written without a reason (or misspelt around the colon) — never a
# waiver, but worth reporting so the author doesn't think it took effect.
_EXEMPT_TOKEN = "flow-on-exempt"


def parse_status_lines(lines: list[str]) -> tuple[list[str], list[str]]:
    """Split a `git diff --name-status`-style list into (changed, added).

    Each line is either "<STATUS>\t<path>" (optionally "<STATUS>\t<old>\t<new>"
    for renames/copies) or a bare path (treated as a generic change, NOT added).
    Returns (all_changed_paths, added_paths)."""
    changed: list[str] = []
    added: list[str] = []
    for raw in lines:
        line = raw.rstrip("\n")
        if not line.strip():
            continue
        m = _STATUS_RE.match(line)
        if m:
            status, rest = m.group(1), m.group(2)
            # For renames/copies the destination path is the last tab field.
            path = rest.split("\t")[-1]
            changed.append(path)
            # [OPUS-4.8] `A` (added) and `C` (copied) both materialise a NEW
            # destination path. Git only emits `C` with copy-detection enabled
            # (-C/--find-copies), but if a crate directory is introduced via a
            # copy the gate must still treat it as added, or new-crate detection
            # could be evaded (accidentally or deliberately).
            if status in ("A", "C"):
                added.append(path)
        else:
            # Bare path (e.g. `git diff --name-only`): a change, not provably added.
            changed.append(line)
    return changed, added


def _normalize_base(base: str) -> str:
    """Accept a bare branch ('main') or a full ref ('refs/heads/main', as the
    merge_group event supplies in github.event.merge_group.base_ref) and return
    the remote-tracking ref to diff against ('origin/main')."""
    base = base.strip()
    if base.startswith("refs/heads/"):
        base = base[len("refs/heads/") :]
    # Already an origin/* ref — use as-is.
    if base.startswith("origin/") or base == "HEAD":
        return base
    return f"origin/{base}"


def git_diff(base: str) -> tuple[list[str], list[str]]:
    """Return (changed, added) from git vs the base ref using a 3-dot diff."""
    ref = _normalize_base(base)
    try:
        out = subprocess.run(
            ["git", "diff", "--name-status", f"{ref}...HEAD"],
            cwd=REPO_ROOT,
            capture_output=True,
            text=True,
            check=True,
        ).stdout
    except subprocess.CalledProcessError as e:  # pragma: no cover - CI-only path
        sys.stderr.write(f"error: git diff failed: {e.stderr}\n")
        sys.exit(2)
    return parse_status_lines(out.splitlines())


def added_crates(added: list[str]) -> list[str]:
    """Crate names whose Cargo.toml was ADDED (a brand-new crate)."""
    crates: list[str] = []
    for p in added:
        m = re.match(r"^crates/([^/]+)/Cargo\.toml$", p)
        if m:
            crates.append(m.group(1))
    # Stable, de-duplicated order.
    seen: set[str] = set()
    out: list[str] = []
    for c in crates:
        if c not in seen:
            seen.add(c)
            out.append(c)
    return out


def crate_is_stub(crate: str, changed: list[str]) -> bool:
    """True iff the new crate's Cargo.toml declares `publish = false` (an
    intentional stub, exempt from bench + SKILL requirements).

    Reads the on-disk Cargo.toml in the worktree (the PR's checkout already has
    the added file). In hermetic tests the file may not exist on disk; callers
    pass the stub status explicitly via the fixture, so a missing file is simply
    treated as NOT a stub (the strict default)."""
    cargo = REPO_ROOT / "crates" / crate / "Cargo.toml"
    try:
        text = cargo.read_text(encoding="utf-8")
    except OSError:
        return False
    return re.search(r"^\s*publish\s*=\s*false\b", text, re.MULTILINE) is not None


def crate_readme_text(crate: str, root: Path | None = None) -> str | None:
    """The crate README's text, or None when it is absent/unreadable."""
    readme = (root or REPO_ROOT) / "crates" / crate / "README.md"
    try:
        return readme.read_text(encoding="utf-8")
    except OSError:
        return None


def flow_on_exempt_reason(readme_text: str | None) -> str | None:
    """The REASON from a `<!-- flow-on-exempt: reason -->` marker, else None.

    A marker with an empty reason is NOT an exemption: the design's escape hatch
    is "`<!-- flow-on-exempt: reason -->` in the crate README, recorded as a
    bead" — the reason IS the record, so waiving the gate without one would make
    the hatch unauditable."""
    if not readme_text:
        return None
    m = _EXEMPT_RE.search(readme_text)
    if not m:
        return None
    return m.group(1).strip() or None


def flow_on_exempt_marker_is_malformed(readme_text: str | None) -> bool:
    """True when the README mentions the marker but no valid one parses — a
    silently-inert escape hatch, which is exactly the #5701 failure mode."""
    if not readme_text:
        return False
    return _EXEMPT_TOKEN in readme_text and flow_on_exempt_reason(readme_text) is None


def crate_flow_on_exempt(crate: str, root: Path | None = None) -> str | None:
    """The exemption reason declared in `crates/<crate>/README.md`, else None."""
    return flow_on_exempt_reason(crate_readme_text(crate, root))


def exempt_crates(added: list[str], root: Path | None = None) -> dict[str, str]:
    """{crate: reason} for every newly-added crate whose README waives follow-on
    machinery. Shared with scripts/flow-on.py so the proactive gate and the
    reactive engine honour the same marker."""
    out: dict[str, str] = {}
    for crate in added_crates(added):
        reason = crate_flow_on_exempt(crate, root)
        if reason:
            out[crate] = reason
    return out


def crate_has_registered_bench(crate: str) -> bool:
    """True iff bench/benchmarks.toml has a `source` field referencing the crate.

    Mirrors the reactive rule's expectation (`source = crates/<x>`): we match any
    benchmark whose `source` line contains `crates/<crate>/` or `crates/<crate>"`
    (end of the path component), so a registered bench for the crate counts."""
    try:
        text = BENCH_REGISTRY.read_text(encoding="utf-8")
    except OSError:
        return False
    needle = re.compile(
        rf"^\s*source\s*=.*crates/{re.escape(crate)}(?:/|\b)", re.MULTILINE
    )
    return needle.search(text) is not None


def evaluate(
    changed: list[str],
    added: list[str],
    *,
    stub_overrides: dict[str, bool] | None = None,
    bench_overrides: dict[str, bool] | None = None,
    exempt_overrides: dict[str, str | None] | None = None,
) -> list[tuple[str, list[str]]]:
    """Return [(crate, [missing-reasons])] for every newly-added crate that
    violates G1. An empty list means the gate PASSES.

    stub_overrides / bench_overrides / exempt_overrides let hermetic tests inject
    the publish-status, bench-registration and README-exemption facts without
    touching disk."""
    stub_overrides = stub_overrides or {}
    bench_overrides = bench_overrides or {}
    exempt_overrides = exempt_overrides or {}
    violations: list[tuple[str, list[str]]] = []

    for crate in added_crates(added):
        # The README escape hatch waives the whole crate (see ESCAPE HATCH 2).
        exempt = exempt_overrides.get(crate)
        if exempt is None:
            exempt = crate_flow_on_exempt(crate)
        if exempt:
            continue

        is_stub = stub_overrides.get(crate)
        if is_stub is None:
            is_stub = crate_is_stub(crate, changed)

        missing: list[str] = []

        # (b) README.md — always required (even for stubs).
        readme = f"crates/{crate}/README.md"
        if readme not in changed:
            missing.append(
                f"a README at {readme} (added in the same PR)"
            )

        if not is_stub:
            # (a) registered benchmark.
            has_bench = bench_overrides.get(crate)
            if has_bench is None:
                has_bench = crate_has_registered_bench(crate)
            if not has_bench:
                missing.append(
                    "a registered benchmark in bench/benchmarks.toml "
                    f"(a [[benchmark]] whose `source` references crates/{crate}) "
                    "— or mark the crate `publish = false` if it is an intentional stub"
                )
            # (c) SKILL.md — required because the crate is a public surface.
            if not any(
                p.startswith("skills/") and p.endswith("SKILL.md") for p in changed
            ):
                missing.append(
                    "a skills/<surface>/SKILL.md for the new public surface "
                    "(AGENTS.md public-API → SKILL.md rule) — or mark the crate "
                    "`publish = false` if it is internal-only"
                )

        if missing:
            violations.append((crate, missing))

    return violations


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(
        description="G1 new-crate-completeness gate (sq-ncvq.4)."
    )
    ap.add_argument(
        "--base",
        default=os.environ.get("GATE_BASE_REF", "main"),
        help="base ref to diff against (origin/<base>); default 'main'.",
    )
    ap.add_argument(
        "--changed-files",
        help="hermetic input: a file of diff lines (status-prefixed or bare paths).",
    )
    ap.add_argument(
        "--dry-run",
        action="store_true",
        help="never exit non-zero; print the verdict only (for local/testing).",
    )
    ap.add_argument(
        "--advisory",
        action="store_true",
        help="soft-launch: report violations but always exit 0.",
    )
    args = ap.parse_args(argv)

    if args.changed_files:
        lines = Path(args.changed_files).read_text(encoding="utf-8").splitlines()
        changed, added = parse_status_lines(lines)
    else:
        changed, added = git_diff(args.base)

    # Report both halves of the escape hatch BEFORE the verdict: a waiver must be
    # visible in the log (it is meant to be reconciled into a bead), and a marker
    # that does not parse must not look like it worked.
    for crate in added_crates(added):
        readme = crate_readme_text(crate)
        reason = flow_on_exempt_reason(readme)
        if reason:
            print(
                f"G1: crates/{crate} is EXEMPT via `<!-- flow-on-exempt: {reason} -->` "
                "in its README — record the decision as a bead."
            )
        elif flow_on_exempt_marker_is_malformed(readme):
            print(
                f"G1: crates/{crate}'s README mentions `{_EXEMPT_TOKEN}` but no valid "
                "marker parsed — the exact form is `<!-- flow-on-exempt: reason -->` "
                "with a non-empty reason. NOT exempt."
            )

    violations = evaluate(changed, added)

    if not violations:
        print("G1 new-crate-completeness: PASS — no new crate missing artifacts.")
        return 0

    print("G1 new-crate-completeness: FAIL")
    for crate, missing in violations:
        print(f"\n  crates/{crate} is a new crate but is missing:")
        for m in missing:
            print(f"    - {m}")
    print(
        "\nA new crate must ship its maintenance artifacts in the SAME PR "
        "(research/maintenance-flow-on-automation-design.md §2.1, gate G1). "
        "Stub crates may set `publish = false` in Cargo.toml to opt out of the "
        "bench + SKILL requirements (README still required); a crate README "
        "carrying `<!-- flow-on-exempt: reason -->` is waived entirely, with the "
        "reason recorded as a bead."
    )

    if args.advisory or args.dry_run:
        print("\n(advisory/dry-run: not failing the build)")
        return 0
    return 1


if __name__ == "__main__":
    raise SystemExit(main())
