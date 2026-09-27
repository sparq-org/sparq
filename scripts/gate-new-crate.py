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
# [OPUS-5] (#5742) G1 also covers the publishable npm packages under `packages/*`
# — see the "RULE (G1-npm)" block below. The Rust and npm halves are one gate and
# one CI leg because they enforce the same thing: a brand-new publishable surface
# must not merge without its maintenance artifacts.
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
# RULE (G1-npm, #5742): when a PR ADDS a new `packages/<x>/package.json`, fail
# unless the SAME PR also provides, for that package:
#   (d) a README.md (packages/<x>/README.md) — required for EVERY new package,
#       public or private, exactly as (b) is required even for a `publish = false`
#       crate stub: a package nobody can describe in one paragraph is debt on
#       arrival. (The crate README *template* gate does not apply to packages;
#       G1 only requires PRESENCE.)
#   (e) at least one test file under packages/<x>/ touched in the same PR; and
#   (f) at least one `.github/workflows/*.yml` that MENTIONS `packages/<x>` —
#       the npm lanes are NOT workspace-wide: js.yml runs `npm test` per package
#       under a step-level `working-directory: packages/<name>`, and gui.yml does
#       the same via a job-level default, so a new package's tests run in CI only
#       if a workflow names it. Like (b), this is a PRESENCE check, not a proof
#       of execution — a mention could be only a `paths:` filter — but it is the
#       floor that stops a package landing with no CI relationship at all.
#       (e) and (f) apply ONLY if the package is a PUBLIC (publishable) surface.
#       "Public" means the package.json does NOT set `private: true` — npm's own
#       publishable predicate, exposed here as `package_is_private()` so that the
#       reactive flow-on rules for new packages can load THIS function (the way
#       flow-on.py already shares gate G2's `pub_api_diff`) instead of
#       re-deriving it, and the two halves can never disagree about which
#       packages are public.
#
# ESCAPE HATCH (G1-npm): `"private": true` in the new package.json marks it a
# non-published internal package (a fixture/harness/app) — exempt from (e) + (f).
# The README (d) is still required.
#
# BACK-CATALOGUE: verified against the four packages on main at the time of
# writing — eyereasoner-compat, solid-server, rdfjs-conformance and sparq-client
# each already ship a README, a `test/` directory and a workflow that names them,
# so G1-npm codifies the standard the repo already holds itself to.
#
# SCOPE: `packages/<x>/package.json` only. The other workspace members (`js/`,
# `site/`, `gui/*`) already exist on main, so no PR can ADD their package.json
# and naming them here would be dead code; a genuinely new npm package belongs
# under `packages/` per the root package.json `workspaces` globs.
#
# DIFF SOURCE: the PR's changed-file list. In CI this is
#   git diff --name-only origin/<base>...HEAD          (changed files)
#   git diff --name-status origin/<base>...HEAD        (to find ADDED files)
# In tests / --dry-run it is read from a fixture file (one path per line, each
# optionally prefixed with a git status letter + tab, e.g. "A\tcrates/x/...").
#
# EXIT: 0 when every newly-added crate satisfies G1 and every newly-added npm
# package satisfies G1-npm (or there are none); 1 with a clear per-crate /
# per-package message naming exactly what is missing otherwise.
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
import json
import os
import re
import subprocess
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
BENCH_REGISTRY = REPO_ROOT / "bench" / "benchmarks.toml"

# A line in a status-prefixed diff: "A\tpath", "M\tpath", "R100\told\tnew", etc.
_STATUS_RE = re.compile(r"^([A-Z])\d*\t(.*)$")


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


# --------------------------------------------------------------------------- #
# G1-npm — new-package-completeness (#5742)
# --------------------------------------------------------------------------- #

# A test file inside a package. The existing packages all use a `test/` directory
# of `*.test.mjs` files; the other common JS conventions are accepted too so the
# gate does not force a house style on a new package.
_PKG_TEST_RE = re.compile(
    r"(?:^|/)(?:tests?|__tests__|spec)/|\.(?:test|spec)\.[cm]?[jt]sx?$"
)


def added_packages(added: list[str]) -> list[str]:
    """Package directory names whose packages/<x>/package.json was ADDED."""
    packages: list[str] = []
    seen: set[str] = set()
    for p in added:
        m = re.match(r"^packages/([^/]+)/package\.json$", p)
        if m and m.group(1) not in seen:
            seen.add(m.group(1))
            packages.append(m.group(1))
    return packages


def package_is_private(pkg: str) -> bool:
    """True iff packages/<pkg>/package.json sets `private: true` — i.e. the
    package is NOT publishable to npm.

    [OPUS-5] (#5742) This is intended as the SINGLE definition of "public npm
    package" for the maintenance-flow-on system. The proactive gate below is its
    only caller today; the reactive flow-on rules for new packages should load it
    through `flow-on.py::_load_script_module` — exactly how that engine already
    shares gate G2's public-API diff via `pub_api_diff.py` — rather than
    re-deriving the predicate, so a package can never be public to one half of
    the system and private to the other.

    npm treats only a truthy `private` as suppressing publication; the string
    "true" is accepted here too because package.json is hand-edited and that
    typo would otherwise silently make an internal package look publishable.
    Unreadable or malformed JSON is treated as NOT private — the strict default,
    matching crate_is_stub()'s fail-strict behaviour."""
    manifest = REPO_ROOT / "packages" / pkg / "package.json"
    try:
        data = json.loads(manifest.read_text(encoding="utf-8"))
    except (OSError, ValueError):
        return False
    if not isinstance(data, dict):
        return False
    private = data.get("private")
    return private is True or (isinstance(private, str) and private.lower() == "true")


def package_has_tests(pkg: str, changed: list[str]) -> bool:
    """True iff the PR touches at least one test file inside packages/<pkg>/."""
    prefix = f"packages/{pkg}/"
    return any(
        p.startswith(prefix) and _PKG_TEST_RE.search(p[len(prefix) :])
        for p in changed
    )


def package_has_ci_leg(pkg: str) -> bool:
    """True iff some .github/workflows/*.yml mentions `packages/<pkg>`.

    Reads the workflow tree in the worktree (the PR's checkout), so a leg ADDED
    by the same PR counts. Presence, not proof of execution — see (f)."""
    needle = f"packages/{pkg}"
    workflows = REPO_ROOT / ".github" / "workflows"
    try:
        entries = sorted(workflows.glob("*.y*ml"))
    except OSError:  # pragma: no cover - defensive
        return False
    for wf in entries:
        try:
            if needle in wf.read_text(encoding="utf-8"):
                return True
        except OSError:  # pragma: no cover - defensive
            continue
    return False


def evaluate_packages(
    changed: list[str],
    added: list[str],
    *,
    private_overrides: dict[str, bool] | None = None,
    ci_overrides: dict[str, bool] | None = None,
) -> list[tuple[str, list[str]]]:
    """Return [(package, [missing-reasons])] for every newly-added npm package
    that violates G1-npm. An empty list means the gate PASSES.

    private_overrides / ci_overrides let hermetic tests inject the `private: true`
    and workflow-mention facts without touching disk."""
    private_overrides = private_overrides or {}
    ci_overrides = ci_overrides or {}
    violations: list[tuple[str, list[str]]] = []

    for pkg in added_packages(added):
        is_private = private_overrides.get(pkg)
        if is_private is None:
            is_private = package_is_private(pkg)

        missing: list[str] = []

        # (d) README.md — always required (even for a private package).
        readme = f"packages/{pkg}/README.md"
        if readme not in changed:
            missing.append(f"a README at {readme} (added in the same PR)")

        if not is_private:
            # (e) tests — required because the package is a publishable surface.
            if not package_has_tests(pkg, changed):
                missing.append(
                    f"at least one test file under packages/{pkg}/ "
                    "(a tests/ directory, or a *.test.* / *.spec.* file) "
                    '— or mark the package `"private": true` if it is never published'
                )
            # (f) a CI leg that names the package — the npm lanes are per-package
            # (`working-directory: packages/<name>`), never workspace-wide.
            has_ci = ci_overrides.get(pkg)
            if has_ci is None:
                has_ci = package_has_ci_leg(pkg)
            if not has_ci:
                missing.append(
                    "a CI leg naming the package — no .github/workflows/*.yml "
                    f"mentions packages/{pkg}, so its tests would never run "
                    "(the npm lanes are per-package, not workspace-wide)"
                )

        if missing:
            violations.append((pkg, missing))

    return violations


def evaluate(
    changed: list[str],
    added: list[str],
    *,
    stub_overrides: dict[str, bool] | None = None,
    bench_overrides: dict[str, bool] | None = None,
) -> list[tuple[str, list[str]]]:
    """Return [(crate, [missing-reasons])] for every newly-added crate that
    violates G1. An empty list means the gate PASSES.

    stub_overrides / bench_overrides let hermetic tests inject the
    publish-status and bench-registration facts without touching disk."""
    stub_overrides = stub_overrides or {}
    bench_overrides = bench_overrides or {}
    violations: list[tuple[str, list[str]]] = []

    for crate in added_crates(added):
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
        description="G1 new-crate/new-package completeness gate (sq-ncvq.4, #5742)."
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

    violations = evaluate(changed, added)
    pkg_violations = evaluate_packages(changed, added)

    if not violations and not pkg_violations:
        print(
            "G1 new-crate/new-package completeness: PASS — no new crate or npm "
            "package missing artifacts."
        )
        return 0

    print("G1 new-crate/new-package completeness: FAIL")
    for crate, missing in violations:
        print(f"\n  crates/{crate} is a new crate but is missing:")
        for m in missing:
            print(f"    - {m}")
    for pkg, missing in pkg_violations:
        print(f"\n  packages/{pkg} is a new npm package but is missing:")
        for m in missing:
            print(f"    - {m}")
    print(
        "\nA new crate or npm package must ship its maintenance artifacts in the "
        "SAME PR — the crate half from "
        "research/maintenance-flow-on-automation-design.md §2.1 (gate G1), the npm "
        "half from issue #5742. Stub crates may set `publish = false` in Cargo.toml "
        "to opt out of the bench + SKILL requirements, and an internal npm package "
        'may set `"private": true` to opt out of the test + CI-leg requirements '
        "(the README is required either way)."
    )

    if args.advisory or args.dry_run:
        print("\n(advisory/dry-run: not failing the build)")
        return 0
    return 1


if __name__ == "__main__":
    raise SystemExit(main())
