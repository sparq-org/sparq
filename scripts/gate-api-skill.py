#!/usr/bin/env python3
# Gate G2 — public-api→skill (bead sq-ncvq.5, epic sq-ncvq).
#
# PROACTIVE / merge-time half of the maintenance flow-on system
# (research/maintenance-flow-on-automation-design.md §2.1, gate G2). Complements
# the reactive engine (scripts/flow-on.py, PR #220): that engine mints a
# "sync SKILL.md" follow-on ISSUE after merge; THIS gate BLOCKS the PR before it
# merges so the docs never drift in the first place.
#
# sq-ncvq.10 doc-sync: this gate is the "Enforced by: **G2**" cell of
# the "public API" row in the AGENTS.md "Post-batch re-evaluation checklist" table
# (alongside the reactive `flow-on:changed-public-feature-docs` rule). That table
# row and this docstring are the two halves of the same rule — change one and
# update the other; the divergence is what sq-ncvq.10 exists to prevent.
#
# RULE (G2): when a PR changes a PUBLIC SURFACE without touching any
# skills/**/SKILL.md in the same diff, FAIL and point at the AGENTS.md
# public-API → SKILL.md maintenance rule.
#
# A "public surface" change is a changed file under a PUBLISHED Rust crate's
# `src/**` (crate NOT `publish = false`) whose unified diff actually changes a
# `pub `-exported item signature — see pub_api_changed(). Concretely, a hunk
# must ADD or REMOVE a line matching the public-item pattern
# (`pub fn`/`pub struct`/`pub enum`/`pub trait`/`pub const`/`pub type`/
# `pub mod`/`pub use`).
#
# WHY a `pub`-signature diff and not "any src change" — false-positive
# remediation (bead flow-on,ci; blocked #250, mis-fired #244):
#   * #250 added only `// SAFETY:` comments + `#![warn(...)]` lint attributes to
#     several crates' src, including the BINDING crates sparq-cli / sparq-bench.
#     The OLD rule blanket-tripped on ANY src change to a binding crate, so a
#     comment/attribute-only edit failed G2. Now NOTHING but a real `pub `-item
#     line trips it — comments (`//`,`///`,`/* */`), attributes (`#[...]`/
#     `#![...]`), and non-`pub` lines are all inert.
#   * #244 RELOCATED `pub fn n3_proof_tree` within explain.rs: the same signature
#     line appeared once added and once removed (net-zero), which the old
#     line-presence heuristic read as a public-API change. Now a `pub `-item line
#     that is BOTH added and removed unchanged (a pure move) cancels out — only a
#     NET added-or-removed signature trips the gate.
#   * A CI-only PR (no `crates/*/src/**` in the diff at all) can never reach the
#     `pub` check, so it always passes — confirmed by the path filter + a test.
#
# Restricted visibilities (`pub(crate)`/`pub(super)`/`pub(in …)`) are NOT the
# crate's public surface and never trip the gate (the pattern requires
# whitespace after `pub`).
#
# SUPPRESSION (mirrors flow-on.py's SKILL_PATHS exactly): the gate is satisfied
# the moment the SAME diff touches ANY skills/**/SKILL.md — that is the signal
# "the docs were synced in this change", identical to the reactive engine's
# suppression of its docs rule. This keeps the two halves consistent: a PR that
# the gate lets through is also a PR the reactive engine would NOT re-flag.
#
# ESCAPE HATCH (design §2.1): the per-PR label `skill-not-needed` (justified in
# the PR body) suppresses the gate. In CI the label list is passed via
# --labels-file; locally/in tests it is injected. (The reactive engine is
# orthogonal — it runs post-merge and is not a blocker.)
#
# DIFF SOURCE: git diff --name-only origin/<base>...HEAD (CI), or a fixture file
# (--changed-files, one path per line, optionally status-prefixed) for tests.
# To evaluate the `pub` heuristic for a changed crate src file we read the unified
# diff (git diff origin/<base>...HEAD -- <file>) for NET added/removed `pub `-item
# signatures; in hermetic mode the caller may inject this via pub_overrides.
#
# EXIT: 0 when no public surface changed, or a SKILL.md was touched, or the
# escape-hatch label is present; 1 (with the AGENTS.md pointer) otherwise.
#
# Usage:
#   gate-api-skill.py                        # CI: diff vs origin/<base>
#   gate-api-skill.py --base main
#   gate-api-skill.py --advisory             # warn-only soft-launch
#   gate-api-skill.py --dry-run --changed-files files.txt [--labels-file l.txt]
#
# stdlib-only.

from __future__ import annotations

import argparse
import importlib.util
import os
import re
import subprocess
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent


# The `pub `-item multiset-diff logic now lives in ONE shared module
# (scripts/pub_api_diff.py) so the reactive flow-on engine
# (scripts/flow-on.py, rule `changed-public-feature-docs`) reuses the EXACT same
# net-public-API-change test this gate uses — the two can no longer drift
# (bead sq-l0a0). Loaded by path (the scripts dir is not an importable package).
def _load_pub_api_diff():
    spec = importlib.util.spec_from_file_location(
        "pub_api_diff", REPO_ROOT / "scripts" / "pub_api_diff.py"
    )
    assert spec and spec.loader
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


_pad = _load_pub_api_diff()

# Binding / entry-point crates (sparq-cli/server/py/wasm). HISTORICALLY
# any src/** change here was a public-surface change; that blanket rule
# false-positived on comment/lint-attribute-only edits (blocked #250). The gate now
# applies the SAME `pub `-signature heuristic to EVERY published crate's src/**,
# binding crates included, so only a real public-item change trips it — there is no
# longer a special-cased prefix list.

# Suppression signal — identical to flow-on.py's SKILL_PATHS: any SKILL.md under
# skills/ means the docs were synced in this diff.
SKILL_SUFFIX = "SKILL.md"
SKILL_PREFIX = "skills/"

# Escape-hatch label (design §2.1).
SKILL_NOT_NEEDED_LABEL = "skill-not-needed"

_STATUS_RE = re.compile(r"^([A-Z])\d*\t(.*)$")
_CRATE_SRC_RE = re.compile(r"^crates/([^/]+)/src/.+")


def parse_paths(lines: list[str]) -> list[str]:
    """Extract paths from a `git diff --name-only` / `--name-status` listing."""
    paths: list[str] = []
    for raw in lines:
        line = raw.rstrip("\n")
        if not line.strip():
            continue
        m = _STATUS_RE.match(line)
        if m:
            paths.append(m.group(2).split("\t")[-1])
        else:
            paths.append(line)
    return paths


def _normalize_base(base: str) -> str:
    """Accept a bare branch ('main') or a full ref ('refs/heads/main', as the
    merge_group event supplies in github.event.merge_group.base_ref) and return
    the remote-tracking ref to diff against ('origin/main')."""
    base = base.strip()
    if base.startswith("refs/heads/"):
        base = base[len("refs/heads/") :]
    if base.startswith("origin/") or base == "HEAD":
        return base
    return f"origin/{base}"


def git_changed(base: str) -> list[str]:
    ref = _normalize_base(base)
    try:
        out = subprocess.run(
            ["git", "diff", "--name-only", f"{ref}...HEAD"],
            cwd=REPO_ROOT,
            capture_output=True,
            text=True,
            check=True,
        ).stdout
    except subprocess.CalledProcessError as e:  # pragma: no cover - CI-only
        sys.stderr.write(f"error: git diff failed: {e.stderr}\n")
        sys.exit(2)
    return parse_paths(out.splitlines())


def crate_is_published(crate: str) -> bool:
    """True iff crates/<crate>/Cargo.toml does NOT declare `publish = false`."""
    cargo = REPO_ROOT / "crates" / crate / "Cargo.toml"
    try:
        text = cargo.read_text(encoding="utf-8")
    except OSError:
        # Unknown crate manifest — treat as published (strict default).
        return True
    return re.search(r"^\s*publish\s*=\s*false\b", text, re.MULTILINE) is None


# The PUBLIC-ITEM signature pattern + the diff scanner now live in the
# shared scripts/pub_api_diff.py (so flow-on.py reuses the identical test). These
# module-level aliases preserve the historical `_PUB_ITEM_RE` / `_scan_pub_diff`
# names this gate (and test_gates.py) reference.
_PUB_ITEM_RE = _pad.PUB_ITEM_RE
_scan_pub_diff = _pad.scan_pub_diff


def pub_diff_lines(path: str, base: str) -> tuple[list[str], list[str]]:
    """Return (added, removed) public-item signature lines from `path`'s diff.

    Each returned string is the diff line with its leading +/- marker stripped and
    surrounding whitespace normalised, so an identical signature that is both added
    and removed (a pure relocation) compares equal across the two lists. Reads the
    per-file unified diff from git; on any error returns ([], []) — conservative,
    never a CI-only crash."""
    ref = _normalize_base(base)
    try:
        out = subprocess.run(
            ["git", "diff", f"{ref}...HEAD", "--", path],
            cwd=REPO_ROOT,
            capture_output=True,
            text=True,
            check=True,
        ).stdout
    except subprocess.CalledProcessError:  # pragma: no cover - CI-only
        return [], []
    return _scan_pub_diff(out.splitlines())


def pub_api_changed(path: str, base: str) -> bool:
    """True iff `path`'s diff NET-changes a `pub `-exported item signature.

    A pure RELOCATION — the same signature added once and removed once
    (net-zero) — is NOT a public-API change (mis-fired G2 on #244, which moved
    `pub fn n3_proof_tree` within a file). We compare added vs removed signatures
    as MULTISETS and only report a change when they differ: a genuinely NEW or
    DELETED export, or a signature whose text actually changed. Comment-only /
    attribute-only / non-`pub` edits yield two empty lists → no change."""
    added, removed = pub_diff_lines(path, base)
    return sorted(added) != sorted(removed)


def public_surface_changes(
    changed: list[str],
    base: str,
    *,
    pub_overrides: dict[str, bool] | None = None,
) -> list[str]:
    """Return the changed paths that constitute a public-surface change.

    A path is in scope iff it is under a PUBLISHED crate's `src/**`
    (the `_CRATE_SRC_RE` filter — a non-`crates/*/src/**` path, e.g. a CI workflow
    or a test, never reaches the `pub` check, so a CI-only PR always passes) AND
    its diff NET-changes a `pub `-exported item (pub_api_changed). The binding
    crates get the SAME `pub`-signature test as every other crate now — no blanket
    trip on comment/attribute-only edits."""
    pub_overrides = pub_overrides or {}
    hits: list[str] = []
    for p in changed:
        m = _CRATE_SRC_RE.match(p)
        if not m:
            continue
        crate = m.group(1)
        if not crate_is_published(crate):
            continue
        is_pub = pub_overrides.get(p)
        if is_pub is None:
            is_pub = pub_api_changed(p, base)
        if is_pub:
            hits.append(p)
    return hits


def skill_touched(changed: list[str]) -> bool:
    return any(p.startswith(SKILL_PREFIX) and p.endswith(SKILL_SUFFIX) for p in changed)


def evaluate(
    changed: list[str],
    labels: list[str],
    base: str,
    *,
    pub_overrides: dict[str, bool] | None = None,
) -> tuple[bool, list[str]]:
    """Return (ok, surface_hits). ok=True means the gate PASSES.

    PASS when: no public surface changed; OR a SKILL.md was touched; OR the
    `skill-not-needed` escape-hatch label is present."""
    hits = public_surface_changes(changed, base, pub_overrides=pub_overrides)
    if not hits:
        return True, []
    if skill_touched(changed):
        return True, hits
    if SKILL_NOT_NEEDED_LABEL in labels:
        return True, hits
    return False, hits


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(
        description="G2 public-api→skill gate (sq-ncvq.5)."
    )
    ap.add_argument(
        "--base",
        default=os.environ.get("GATE_BASE_REF", "main"),
        help="base ref to diff against (origin/<base>); default 'main'.",
    )
    ap.add_argument("--changed-files", help="hermetic input: file of diff lines.")
    ap.add_argument(
        "--labels-file",
        help="hermetic input: file of PR labels, one per line (for the escape hatch).",
    )
    ap.add_argument("--dry-run", action="store_true", help="never exit non-zero.")
    ap.add_argument(
        "--advisory", action="store_true", help="soft-launch: report but exit 0."
    )
    args = ap.parse_args(argv)

    if args.changed_files:
        lines = Path(args.changed_files).read_text(encoding="utf-8").splitlines()
        changed = parse_paths(lines)
    else:
        changed = git_changed(args.base)

    labels: list[str] = []
    if args.labels_file:
        labels = [
            ln.strip()
            for ln in Path(args.labels_file).read_text(encoding="utf-8").splitlines()
            if ln.strip()
        ]

    ok, hits = evaluate(changed, labels, args.base)

    if ok:
        if hits:
            print(
                "G2 public-api→skill: PASS — public surface changed and a "
                "SKILL.md (or the skill-not-needed label) is present."
            )
        else:
            print("G2 public-api→skill: PASS — no public surface changed.")
        return 0

    print("G2 public-api→skill: FAIL")
    print("\n  These changed files are a public surface, but NO skills/**/SKILL.md")
    print("  was edited in the same PR:")
    for p in hits:
        print(f"    - {p}")
    print(
        "\nThe AGENTS.md public-API → SKILL.md maintenance rule (AGENTS.md "
        '"MAINTENANCE RULE (REQUIRED)" / "Public-API → SKILL.md maintenance rule") '
        "requires updating the matching skills/<surface>/SKILL.md in the SAME "
        "change. Edit the relevant SKILL.md (the surface map is in "
        "skills/SKILL.md), or — if no doc change is warranted — add the "
        f"`{SKILL_NOT_NEEDED_LABEL}` label with a justification in the PR body."
    )

    if args.advisory or args.dry_run:
        print("\n(advisory/dry-run: not failing the build)")
        return 0
    return 1


if __name__ == "__main__":
    raise SystemExit(main())
