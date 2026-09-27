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
# ---------------------------------------------------------------------------
# RULE (G1-npm, issue #5843): the npm half of the same "a new unit must not land
# unexercised" invariant. When a PR ADDS a new `packages/<x>/package.json`, fail
# unless:
#   (n1) `packages/<x>` RESOLVES TO AN NPM WORKSPACE ENTRY — i.e. it is matched by
#        a glob in the repo-root package.json `workspaces` array. A directory the
#        root workspace does not claim is never installed by the root `npm ci`
#        every JS lane runs, so nothing in it can be built or tested in CI.
#   (n2) the package's TEST SCRIPT IS ACTUALLY INVOKED BY A WORKFLOW LEG. A
#        package that ships tests nobody runs is indistinguishable from a package
#        with no tests: `npm test` never executes, so a red suite reads green.
#        Coverage is established by scanning .github/workflows/*.yml for a step
#        that runs an npm/pnpm/yarn test command AND is bound to the package by
#        one of the three wiring forms the repo actually uses today:
#          - `working-directory: packages/<x>` (step-level OR job `defaults.run`),
#          - an `-w`/`--workspace=` flag naming `packages/<x>` or the manifest
#            `name` (also `--workspaces`, which runs every member),
#          - `cd packages/<x>` inside the run block.
#        (n2) applies when the package declares a `test` script OR ships test
#        files (test/, tests/, __tests__/, *.test.*, *.spec.*) — the "or ships
#        test files" limb is what stops the rename bypass of dropping the `test`
#        script while keeping the suite.
#
# WHY THIS IS A SEPARATE evaluate_packages(): it answers a question the crate
# rules cannot — CI WIRING, read from the workflow tree rather than from the
# diff. Issue #5742 deliberately left it out for exactly that reason; #5843 adds
# it as its own evaluator so the crate path stays untouched.
#
# HEURISTIC HONESTY: the workflow scan is a textual step-splitter, NOT a YAML
# parse (this gate is stdlib-only and the flow-on-gates job installs no PyYAML).
# What it checks is narrow and worth stating exactly. A step counts as coverage
# only when the SAME step both runs a test command and is bound to the package,
# so `npm test` in a step that does NOT inherit the package's directory is not
# credited. Known FALSE-PASS gaps, none of which this gate claims to close:
#   - it does not check that the covering workflow's `on.paths` filter actually
#     matches the new package, so a leg wired into a workflow that never
#     triggers for `packages/<x>` still counts;
#   - it does not check that the covering job is gating rather than advisory,
#     nor that it is reachable (`if:` conditions, matrix exclusions);
#   - `--workspaces` is credited without confirming the member's `test` script
#     would resolve;
#   - a step's whole run block is read as one text, so a test command anywhere
#     in it counts for the step's directory.
# Those gaps are acceptable because the case #5843 names — a new package wired
# NOWHERE AT ALL — is unambiguous and is exactly what this blocks; closing the
# rest needs the workflow evaluation semantics, not a diff-time gate.
# ---------------------------------------------------------------------------
#
# DIFF SOURCE: the PR's changed-file list. In CI this is
#   git diff --name-only origin/<base>...HEAD          (changed files)
#   git diff --name-status origin/<base>...HEAD        (to find ADDED files)
# In tests / --dry-run it is read from a fixture file (one path per line, each
# optionally prefixed with a git status letter + tab, e.g. "A\tcrates/x/...").
#
# EXIT: 0 when every newly-added crate satisfies G1 and every newly-added npm
# package satisfies G1-npm (or there are none); 1 with a clear per-unit message
# naming exactly what is missing otherwise.
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
import fnmatch
import json
import os
import re
import subprocess
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
BENCH_REGISTRY = REPO_ROOT / "bench" / "benchmarks.toml"
ROOT_MANIFEST = REPO_ROOT / "package.json"
WORKFLOW_DIR = REPO_ROOT / ".github" / "workflows"

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


# --------------------------------------------------------------------------- #
# G1-npm — a new packages/<x> must be a workspace member whose tests a CI leg runs
# (issue #5843; the follow-up #5742 named but deliberately did not implement).
# --------------------------------------------------------------------------- #

# A run command that executes a package's tests: `npm test`, `npm run test`,
# `npm run test:unit`, `pnpm test`, `yarn test`, and the same with intervening
# flags (`npm -w packages/x test`, `npm run test --workspaces`). [ \t] rather
# than \s throughout so a match can never straddle two lines of a `run: |` block
# (e.g. "npm run typecheck\nnpm test" must match on the SECOND line only).
_TEST_CMD_RE = re.compile(
    r"\b(?:npm|pnpm|yarn|bun)(?:[ \t]+-{1,2}[\w=@./:-]+)*"
    r"[ \t]+(?:test\b|run[ \t]+test(?::[\w.:-]+)?\b)"
)

# `working-directory: packages/foo` / `working-directory: "packages/foo"`.
_WD_RE = re.compile(r"^\s*working-directory:[ \t]*['\"]?([^'\"#\s]+)")

# A `run:` key; group 2 is the inline scalar (empty for a `|` / `>` block).
_RUN_RE = re.compile(r"^(\s*)-?\s*run:[ \t]*(?:[|>][-+0-9]*)?[ \t]*(.*?)\s*$")

# Test files a package can ship: test/ tests/ __tests__/ *.test.* *.spec.*
_TEST_FILE_RE = re.compile(
    r"(?:^|/)(?:tests?|__tests__)/|(?:^|/)[^/]+\.(?:test|spec)\.[cm]?[jt]sx?$"
)


def added_packages(added: list[str]) -> list[str]:
    """Package directory names whose packages/<x>/package.json was ADDED."""
    names: list[str] = []
    seen: set[str] = set()
    for p in added:
        m = re.match(r"^packages/([^/]+)/package\.json$", p)
        if m and m.group(1) not in seen:
            seen.add(m.group(1))
            names.append(m.group(1))
    return names


def workspace_globs(root_manifest: Path | None = None) -> list[str]:
    """The repo-root package.json `workspaces` globs (npm accepts either a bare
    array or the yarn-style {"packages": [...]} object)."""
    try:
        data = json.loads((root_manifest or ROOT_MANIFEST).read_text(encoding="utf-8"))
    except (OSError, ValueError):
        return []
    ws = data.get("workspaces")
    if isinstance(ws, dict):
        ws = ws.get("packages")
    if not isinstance(ws, list):
        return []
    return [g for g in ws if isinstance(g, str)]


def package_is_workspace_member(package: str, globs: list[str]) -> bool:
    """True iff `packages/<package>` is matched by one of the workspace globs.

    fnmatch's `*` also crosses `/`, which is MORE permissive than npm's glob —
    harmless here because the path being matched has exactly one segment after
    `packages/`, so `packages/*` and `packages/**` behave identically on it."""
    path = f"packages/{package}"
    return any(fnmatch.fnmatchcase(path, g.rstrip("/")) for g in globs)


def package_manifest(package: str) -> dict:
    """The new package's own package.json (already on disk in the PR checkout)."""
    try:
        data = json.loads(
            (REPO_ROOT / "packages" / package / "package.json").read_text(
                encoding="utf-8"
            )
        )
    except (OSError, ValueError):
        return {}
    return data if isinstance(data, dict) else {}


def package_ships_tests(package: str, changed: list[str]) -> bool:
    """True iff the package declares a `test` script OR the PR adds a test file
    under it. The second limb is what makes (n2) bypass-resistant: dropping the
    `test` script while keeping the suite must not silently exempt the package."""
    scripts = package_manifest(package).get("scripts")
    if isinstance(scripts, dict) and isinstance(scripts.get("test"), str):
        return True
    prefix = f"packages/{package}/"
    return any(
        p.startswith(prefix) and _TEST_FILE_RE.search(p[len(prefix) :]) for p in changed
    )


def workflow_steps(text: str) -> list[tuple[str, str]]:
    """Split one workflow file into (effective_working_directory, run_text) pairs.

    Textual, not a YAML parse — see the HEURISTIC HONESTY note in the header. A
    step's effective working-directory is its own `working-directory:` if it has
    one, else the enclosing job's `defaults.run.working-directory`. Only the
    region after a top-level `jobs:` line is scanned, so the `on.paths:` filters
    (which name `packages/**` in js.yml) can never be mistaken for wiring."""
    lines = text.splitlines()
    try:
        start = next(i for i, ln in enumerate(lines) if re.match(r"^jobs:\s*$", ln)) + 1
    except StopIteration:
        return []

    steps: list[tuple[str, str]] = []
    job_default_wd = ""
    step_wd: str | None = None
    step_runs: list[str] = []
    in_step = False

    def flush() -> None:
        if step_runs:
            wd = step_wd if step_wd is not None else job_default_wd
            steps.append((wd, "\n".join(step_runs)))

    i = start
    while i < len(lines):
        line = lines[i]
        stripped = line.strip()
        indent = len(line) - len(line.lstrip())

        # A new job (a key at indent 2) resets the job-level default.
        if re.match(r"^  [A-Za-z_][\w.-]*:\s*$", line):
            flush()
            job_default_wd, step_wd, step_runs, in_step = "", None, [], False
            i += 1
            continue

        # A new step (a list item under `steps:`).
        if stripped.startswith("- ") or stripped == "-":
            flush()
            step_wd, step_runs, in_step = None, [], True
            # fall through: `- run: ...` / `- working-directory: ...` on this line

        m = _WD_RE.match(line)
        if m:
            if in_step:
                step_wd = m.group(1)
            else:
                # Before the first step of the job => the `defaults.run` block.
                job_default_wd = m.group(1)
            i += 1
            continue

        # `run:` is only a COMMAND inside a step. Outside one it is the
        # `defaults:` -> `run:` MAPPING whose body holds working-directory —
        # consuming that as a block scalar would swallow the job default.
        m = _RUN_RE.match(line) if in_step else None
        if m:
            inline = m.group(2)
            if inline:
                step_runs.append(inline)
                i += 1
                continue
            # Block scalar: consume the more-indented body.
            run_indent = indent
            i += 1
            while i < len(lines):
                body = lines[i]
                if body.strip() and (len(body) - len(body.lstrip())) <= run_indent:
                    break
                step_runs.append(body.strip())
                i += 1
            continue

        i += 1

    flush()
    return steps


def _step_covers_package(
    wd: str, run_text: str, package: str, pkg_name: str | None
) -> bool:
    """True iff this step both runs a test command and is bound to the package."""
    if not _TEST_CMD_RE.search(run_text):
        return False

    path = f"packages/{package}"
    wd = wd.strip().rstrip("/").lstrip("./")
    if wd == path or wd.startswith(path + "/"):
        return True
    if re.search(rf"\bcd[ \t]+\.?/?{re.escape(path)}(?:/|\b)", run_text):
        return True
    # `--workspaces` runs every member, so it covers the new one too.
    if re.search(r"--workspaces\b", run_text):
        return True
    targets = [path, package] + ([pkg_name] if pkg_name else [])
    return any(
        re.search(rf"(?:-w|--workspace)[= \t]+?{re.escape(t)}(?:/|\b)", run_text)
        for t in targets
    )


def package_ci_test_legs(
    package: str, pkg_name: str | None = None, workflow_dir: Path | None = None
) -> list[str]:
    """Workflow filenames containing a leg that runs `packages/<package>`'s tests."""
    directory = workflow_dir or WORKFLOW_DIR
    legs: list[str] = []
    try:
        candidates = sorted(
            p for p in directory.iterdir() if p.suffix in (".yml", ".yaml")
        )
    except OSError:
        return []
    for wf in candidates:
        try:
            text = wf.read_text(encoding="utf-8")
        except OSError:
            continue
        # Cheap pre-filter: a covering leg either names the package somewhere or
        # uses `--workspaces` (which runs every member without naming any).
        if (
            package not in text
            and not (pkg_name and pkg_name in text)
            and "--workspaces" not in text
        ):
            continue
        if any(
            _step_covers_package(wd, run, package, pkg_name)
            for wd, run in workflow_steps(text)
        ):
            legs.append(wf.name)
    return legs


def evaluate_packages(
    changed: list[str],
    added: list[str],
    *,
    member_overrides: dict[str, bool] | None = None,
    ships_tests_overrides: dict[str, bool] | None = None,
    legs_overrides: dict[str, list[str]] | None = None,
) -> list[tuple[str, list[str]]]:
    """Return [(package, [missing-reasons])] for every newly-added npm package
    that violates G1-npm. An empty list means the leg PASSES.

    The *_overrides let hermetic tests inject the workspace-membership,
    ships-tests and CI-wiring facts without touching disk."""
    member_overrides = member_overrides or {}
    ships_tests_overrides = ships_tests_overrides or {}
    legs_overrides = legs_overrides or {}
    globs = workspace_globs()
    violations: list[tuple[str, list[str]]] = []

    for package in added_packages(added):
        missing: list[str] = []

        # (n1) resolves to an npm workspace entry.
        is_member = member_overrides.get(package)
        if is_member is None:
            is_member = package_is_workspace_member(package, globs)
        if not is_member:
            missing.append(
                f"an npm workspace entry matching packages/{package} in the "
                "repo-root package.json `workspaces` globs — without one the root "
                "`npm ci` never installs it, so no CI leg can build or test it"
            )

        # (n2) a workflow leg actually runs its tests.
        ships = ships_tests_overrides.get(package)
        if ships is None:
            ships = package_ships_tests(package, changed)
        if ships:
            legs = legs_overrides.get(package)
            if legs is None:
                legs = package_ci_test_legs(
                    package, package_manifest(package).get("name")
                )
            if not legs:
                missing.append(
                    "a CI leg that runs its tests — no step in .github/workflows/ "
                    f"invokes `npm test` for packages/{package}. Add a step with "
                    f"`working-directory: packages/{package}` (see the per-package "
                    "steps in .github/workflows/js.yml) or `npm test -w "
                    f"packages/{package}`, so the suite cannot ship unexecuted"
                )

        if missing:
            violations.append((package, missing))

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

    violations = evaluate(changed, added)
    pkg_violations = evaluate_packages(changed, added)

    if not violations and not pkg_violations:
        print(
            "G1 new-crate-completeness: PASS — no new crate or npm package "
            "missing artifacts."
        )
        return 0

    print("G1 new-crate-completeness: FAIL")
    for crate, missing in violations:
        print(f"\n  crates/{crate} is a new crate but is missing:")
        for m in missing:
            print(f"    - {m}")
    for package, missing in pkg_violations:
        print(f"\n  packages/{package} is a new npm package but is missing:")
        for m in missing:
            print(f"    - {m}")
    if violations:
        print(
            "\nA new crate must ship its maintenance artifacts in the SAME PR "
            "(research/maintenance-flow-on-automation-design.md §2.1, gate G1). "
            "Stub crates may set `publish = false` in Cargo.toml to opt out of the "
            "bench + SKILL requirements (README still required)."
        )
    if pkg_violations:
        print(
            "\nA new npm package must be a workspace member whose tests a CI leg "
            "actually runs (G1-npm, issue #5843) — otherwise its suite ships "
            "unexecuted and a red test reads green."
        )

    if args.advisory or args.dry_run:
        print("\n(advisory/dry-run: not failing the build)")
        return 0
    return 1


if __name__ == "__main__":
    raise SystemExit(main())
