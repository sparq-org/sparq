#!/usr/bin/env python3
# Change-coupled formal-verification selector (bead sq-63ups). 🤖 SPARQ agent.
#
# WHAT: given a PR / merge-group diff, decide which Kani proof suites from
# ci/formal-verification.toml must run ON THIS PR as gating checks. A suite is
# selected iff it is `pr_gate = true` AND the diff touches one of its `proves`
# paths (the source files whose correctness its harnesses establish). Unrelated
# PRs select nothing and the heavy proof jobs are `if:`-skipped (the ci-summary
# aggregator treats `skipped` as satisfied once the selection pre-job succeeded).
#
# FAIL-CLOSED CONTRACT (mirrors scripts/ci_select.py, design §4.3 of
# research/change-based-test-selection.md):
#   * any FV full-run trigger changed (this manifest, this script, the
#     completeness gate, the FV workflow, kani.yml)  => ALL pr_gate suites run;
#   * any diff/infrastructure error with a READABLE manifest => ALL pr_gate
#     suites run, exit 0 (a selector bug degrades to running more, never less);
#   * an UNREADABLE/INVALID manifest => exit 1. Unlike ci_select.py we cannot
#     "run everything" without the manifest (it IS the suite list), so the only
#     sound degradation is a red select job — and because the job name carries
#     the phrase "change-based test selection", scripts/ci_summary_gate.py REDs
#     the whole gate when this job does not conclude success, so the skips on
#     the commit can never be trusted on the back of a broken selector.
#
# OUTPUTS ($GITHUB_OUTPUT):
#   any    — "true" | "false": whether any suite was selected (job-level guard;
#            an empty fromJSON matrix is a workflow error, so the proof job is
#            `if:`-guarded on this instead).
#   suites — JSON array of {name, crate, features, harnesses} objects (harnesses
#            space-joined) consumed directly as the proof job's matrix.
#
# Usage:
#   fv_select.py --event pull_request --base <sha> --head <sha>
#   fv_select.py --event merge_group --base <sha> --head <sha>
#   fv_select.py --changed-file paths.txt --manifest fv.toml   # hermetic (tests)
#   fv_select.py --self-test                                   # hermetic fixtures
#
# Stdlib only (tomllib >= 3.11). Runs under the runner's system python3 / the
# CI setup-python 3.12.

from __future__ import annotations

import argparse
import fnmatch
import json
import os
import subprocess
import sys
import tempfile

DEFAULT_MANIFEST = "ci/formal-verification.toml"

# A change to any of these re-obligates EVERY pr_gate suite (the selection
# policy / harness wiring itself moved — absence of proof means run, mirroring
# ci_select.py `_FULL_TRIGGERS`). Deliberately NOT keyed on Cargo.lock /
# toolchain pins: every proof cone in the manifest is intra-crate pure code and
# the nightly kani.yml lane re-proves the full matrix daily as the backstop.
FV_FULL_TRIGGERS = (
    "ci/formal-verification.toml",
    "scripts/fv_select.py",
    "scripts/check-fv-manifest.py",
    ".github/workflows/formal-verification.yml",
    ".github/workflows/kani.yml",
)


class ManifestError(Exception):
    """The manifest is unreadable/invalid — the ONE non-fail-open error (exit 1)."""


class SelectorError(Exception):
    """Any other internal failure. main() traps this => all pr_gate suites, exit 0."""


def load_manifest(path: str) -> list[dict]:
    """Return the [[suite]] list. Raises ManifestError on any shape problem."""
    import tomllib

    try:
        with open(path, "rb") as fh:
            data = tomllib.load(fh)
    except (OSError, tomllib.TOMLDecodeError) as exc:
        raise ManifestError(f"cannot read manifest {path}: {exc}") from exc
    suites = data.get("suite")
    if not isinstance(suites, list) or not suites:
        raise ManifestError(f"{path}: [[suite]] array missing or empty")
    for i, s in enumerate(suites):
        for key in ("id", "name", "crate", "harnesses", "proves"):
            if key not in s:
                raise ManifestError(f"{path}: suite #{i} missing key {key!r}")
        if not isinstance(s["harnesses"], list) or not s["harnesses"]:
            raise ManifestError(f"{path}: suite {s.get('id')!r}: empty harness list")
        if not isinstance(s["proves"], list) or not s["proves"]:
            raise ManifestError(f"{path}: suite {s.get('id')!r}: empty proves list")
        if s.get("pr_gate") is False and not s.get("pr_gate_blocked_by"):
            raise ManifestError(
                f"{path}: suite {s.get('id')!r}: pr_gate=false requires pr_gate_blocked_by"
            )
    return suites


def path_matches(path: str, pattern: str) -> bool:
    """`dir/**` matches the dir or anything beneath it; otherwise fnmatch
    (an exact path is its own fnmatch). Same semantics as ci_select.py's map."""
    if pattern.endswith("/**"):
        root = pattern[:-3]
        return path == root or path.startswith(root + "/")
    return fnmatch.fnmatch(path, pattern)


def select_suites(changed_paths: list[str], suites: list[dict]) -> tuple[list[dict], str]:
    """Pure core: (changed paths, manifest suites) -> (selected pr_gate suites, reason)."""
    gate_suites = [s for s in suites if s.get("pr_gate") is True]
    for path in changed_paths:
        path = path.strip()
        if any(path_matches(path, trig) for trig in FV_FULL_TRIGGERS):
            return gate_suites, f"FV full trigger changed ({path}): all pr_gate suites run"
    selected: list[dict] = []
    hits: list[str] = []
    for s in gate_suites:
        matched = [
            p for p in changed_paths if any(path_matches(p.strip(), pat) for pat in s["proves"])
        ]
        if matched:
            selected.append(s)
            hits.append(f"{s['id']} <= {', '.join(sorted(set(matched)))}")
    if selected:
        return selected, "; ".join(hits)
    return [], "no verified path touched; every proof suite provably unaffected"


def to_matrix(selected: list[dict]) -> list[dict]:
    return [
        {
            "name": s["name"],
            "crate": s["crate"],
            "features": s.get("features", ""),
            "harnesses": " ".join(s["harnesses"]),
        }
        for s in selected
    ]


def git_changed_paths(base: str, head: str) -> list[str]:
    # Three-dot = diff against the merge-base; --no-renames reports a move as
    # delete+add so BOTH paths are attributed (same as ci_select.py).
    try:
        out = subprocess.run(
            ["git", "diff", "--name-only", "--no-renames", f"{base}...{head}"],
            check=True,
            capture_output=True,
            text=True,
            timeout=180,
        )
    except (subprocess.CalledProcessError, subprocess.TimeoutExpired, OSError) as exc:
        raise SelectorError(f"git diff failed: {exc}") from exc
    return [ln for ln in out.stdout.splitlines() if ln.strip()]


def write_outputs(selected: list[dict], reason: str, output_file: str | None,
                  summary_file: str | None) -> None:
    matrix = to_matrix(selected)
    if output_file:
        with open(output_file, "a", encoding="utf-8") as fh:
            fh.write(f"any={'true' if matrix else 'false'}\n")
            fh.write("suites=" + json.dumps(matrix) + "\n")
    if summary_file:
        with open(summary_file, "a", encoding="utf-8") as fh:
            fh.write("### 🤖 Formal-verification selection (fv-select)\n\n")
            fh.write(f"**Reason:** {reason}\n\n")
            if matrix:
                fh.write("| Suite | Crate | Harnesses |\n| --- | --- | --- |\n")
                for m in matrix:
                    fh.write(
                        f"| {m['name']} | `{m['crate']}` | {len(m['harnesses'].split())} |\n"
                    )
            else:
                fh.write("No proof suite selected — the change-coupled Kani job is skipped.\n")


def main(argv: list[str] | None = None) -> int:
    p = argparse.ArgumentParser(description="Change-coupled formal-verification selector.")
    p.add_argument("--event", default="pull_request")
    p.add_argument("--base")
    p.add_argument("--head", default="HEAD")
    p.add_argument("--changed-file", help="hermetic: newline-delimited changed paths")
    p.add_argument("--manifest", default=DEFAULT_MANIFEST)
    p.add_argument("--output-file")
    p.add_argument("--summary-file")
    p.add_argument("--self-test", action="store_true")
    args = p.parse_args(argv)

    if args.self_test:
        return self_test()

    output_file = args.output_file or os.environ.get("GITHUB_OUTPUT")
    summary_file = args.summary_file or os.environ.get("GITHUB_STEP_SUMMARY")

    # An unreadable manifest is NOT trapped: red select job => red gate (header).
    suites = load_manifest(args.manifest)

    try:
        if args.event not in ("pull_request", "merge_group"):
            raise SelectorError(f"event {args.event!r} carries no sound diff pair")
        if args.changed_file:
            with open(args.changed_file, encoding="utf-8") as fh:
                changed = [ln for ln in fh.read().splitlines() if ln.strip()]
        else:
            if not args.base:
                raise SelectorError("--base is required for a diff-based run")
            changed = git_changed_paths(args.base, args.head)
        selected, reason = select_suites(changed, suites)
    except SelectorError as exc:
        # Fail-closed: run every pr_gate suite (more, never less), exit 0.
        selected = [s for s in suites if s.get("pr_gate") is True]
        reason = f"selector error, failing closed to all pr_gate suites: {exc}"

    print(json.dumps({"reason": reason, "suites": to_matrix(selected)}, indent=2))
    try:
        write_outputs(selected, reason, output_file, summary_file)
    except OSError as exc:
        print(f"::error::fv-select could not write outputs: {exc}")
        return 1
    return 0


# --------------------------------------------------------------------------- #
# Hermetic self-test (run as a gating step before the real selection)
# --------------------------------------------------------------------------- #
_FIXTURE_MANIFEST = """\
schema = 1
[[suite]]
id = "a"
name = "crate-a (proofs)"
crate = "crate-a"
features = "--features f"
harnesses = ["h1", "h2"]
proves = ["crates/crate-a/src/x.rs", "crates/shared/src/dep.rs"]
nightly = true
pr_gate = true
[[suite]]
id = "b"
name = "crate-b (slow proofs)"
crate = "crate-b"
features = ""
harnesses = ["h3"]
proves = ["crates/crate-b/src/**"]
nightly = true
pr_gate = false
pr_gate_blocked_by = "sq-xxxx"
"""


def self_test() -> int:
    import tomllib  # noqa: F401  (assert availability early)

    failures: list[str] = []

    def check(cond: bool, label: str) -> None:
        if not cond:
            failures.append(label)

    with tempfile.TemporaryDirectory() as td:
        man = os.path.join(td, "fv.toml")
        with open(man, "w", encoding="utf-8") as fh:
            fh.write(_FIXTURE_MANIFEST)
        suites = load_manifest(man)

        # 1. touching a proved path selects exactly its suite.
        sel, _ = select_suites(["crates/crate-a/src/x.rs", "README.md"], suites)
        check([s["id"] for s in sel] == ["a"], "proved-path selects its suite")
        # 2. a shared proves path selects every suite listing it.
        sel, _ = select_suites(["crates/shared/src/dep.rs"], suites)
        check([s["id"] for s in sel] == ["a"], "shared cone path selects dependents")
        # 3. an unrelated diff selects nothing.
        sel, _ = select_suites(["site/app/page.tsx", "docs/x.md"], suites)
        check(sel == [], "unrelated diff selects nothing")
        # 4. a pr_gate=false suite is NEVER selected, even on a direct hit.
        sel, _ = select_suites(["crates/crate-b/src/lib.rs"], suites)
        check(sel == [], "pr_gate=false suite is never PR-selected")
        # 5. an FV full trigger selects ALL pr_gate suites.
        sel, reason = select_suites(["ci/formal-verification.toml"], suites)
        check([s["id"] for s in sel] == ["a"] and "full trigger" in reason,
              "full trigger selects all pr_gate suites")
        # 6. glob proves patterns match beneath the root.
        check(path_matches("crates/crate-b/src/deep/mod.rs", "crates/crate-b/src/**"),
              "dir/** glob matches nested path")
        # 7. malformed manifest => ManifestError (the exit-1 path).
        bad = os.path.join(td, "bad.toml")
        with open(bad, "w", encoding="utf-8") as fh:
            fh.write("[[suite]]\nid = 'x'\n")  # missing required keys
        try:
            load_manifest(bad)
            failures.append("malformed manifest must raise")
        except ManifestError:
            pass
        # 8. pr_gate=false without an owner bead => ManifestError.
        bad2 = os.path.join(td, "bad2.toml")
        with open(bad2, "w", encoding="utf-8") as fh:
            fh.write(_FIXTURE_MANIFEST.replace('pr_gate_blocked_by = "sq-xxxx"\n', ""))
        try:
            load_manifest(bad2)
            failures.append("pr_gate=false without blocked_by must raise")
        except ManifestError:
            pass

    if failures:
        for f in failures:
            print(f"::error::fv_select self-test FAILED: {f}")
        return 1
    print("fv_select self-test: all fixtures behaved (selection + fail modes).")
    return 0


if __name__ == "__main__":
    sys.exit(main())
