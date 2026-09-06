#!/usr/bin/env python3
# [SONNET-4.6] CI lint (bead sq-6vshe.11): every cargo-llvm-cov COVERAGE job must set a
# job-level `RUSTFLAGS`, and every such job must set the SAME value.
#
# WHY THIS GATE EXISTS — the silently-shadowed link flag (measured 2026-07-30):
# ci.yml sets, at WORKFLOW level, the sq-6vshe.1 build-profile trio, whose third leg is
#
#     CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUSTFLAGS: "-C link-arg=-fuse-ld=lld"
#
# scoped to the host triple so wasm32 links keep using wasm-ld. Cargo resolves rustflags
# from FOUR sources and uses only the FIRST one that is set — it never merges them:
#
#     CARGO_ENCODED_RUSTFLAGS  >  RUSTFLAGS  >  target.<triple>.rustflags  >  build.rustflags
#                                               (^ what the workflow-level var above is)
#
# cargo-llvm-cov injects `-C instrument-coverage` by setting the top TWO (that is exactly
# what `cargo llvm-cov show-env` exports, and what scripts/coverage-engine-shard.sh
# `build-objects` sources into its shell). So in every coverage job the workflow-level
# per-target var was DROPPED WHOLE, and the instrumented links — the heaviest links in CI —
# ran on the default linker while every sibling job used lld. Nothing failed; the flag just
# quietly did not apply, which is why it survived from sq-6vshe.1 until it was measured.
#
# The fix is to set `RUSTFLAGS` at JOB level in those jobs: cargo-llvm-cov extends an
# existing RUSTFLAGS rather than shadowing it, so the link flag rides through. This lint
# makes the OMISSION itself fail CI so the shadowing cannot silently re-appear.
#
# NOTE ON WHAT THIS LINT DOES AND DOES NOT ASSERT: it checks that a job-level RUSTFLAGS is
# PRESENT and CONSISTENT — a static, decidable property. It does NOT and cannot assert that
# the resulting flags actually reach rustc; that depends on cargo-llvm-cov's behaviour and
# is confirmed by reading a coverage job's compile line in CI.
#
# SECOND, INDEPENDENT REASON FOR THE SAMENESS RULE (the engine split): ci.yml's
# `coverage-engine-run` matrix compiles instrumented sparq-engine binaries on three runners
# and `coverage-engine-merge` recompiles them on a fourth, then merges the partitions'
# .profraw against that fourth build. That merge is only valid while all four compiles see
# IDENTICAL inputs (same toolchain, same features, same RUSTFLAGS) — a per-job RUSTFLAGS
# that drifted between run and merge would degrade the merge silently, i.e. as an
# undercount that reads as a coverage regression. Requiring one shared value across every
# cargo-llvm-cov job enforces that by construction instead of by comment.
#
# RULE: scan .github/workflows/*.yml (+ *.yaml). A job is a COVERAGE JOB if it either
#   (a) has a step installing cargo-llvm-cov — a `with:` mapping whose `tool:` value names
#       `cargo-llvm-cov` (the SHA-pin-safe selector check-install-action-tool.py enforces), or
#   (b) contains a `run:` line invoking `cargo llvm-cov`.
# Every coverage job MUST declare a job-level `env:` containing a `RUSTFLAGS:` key, and all
# coverage jobs across all workflows MUST agree on its value. Jobs that merely run a
# coverage SCRIPT with no compile (e.g. ci.yml's `coverage-floors`, which only runs the
# no-compile presence / monotonicity / shard-partition checks) match neither (a) nor (b) and
# are correctly out of scope — they build nothing, so no linker flag applies to them.
#
# WHY A HAND-ROLLED STDLIB PARSER (no PyYAML): this runs in the docs-quality `ci-scripts`
# job, which installs no Python deps — same constraint, and the same tightly-constrained
# block style, as scripts/check-install-action-tool.py. We only need to (a) split `jobs:`
# into per-job blocks, (b) read the job-level keys of each block, and (c) read the `with:`/
# `run:` lines inside it. It deliberately is not a general YAML parser.
#
# EXIT: 0 when every coverage job sets a job-level RUSTFLAGS and all values agree (or there
# are no coverage jobs); 1 with a per-offence message otherwise.
#
# Usage:
#   check-coverage-rustflags.py                  # scan .github/workflows
#   check-coverage-rustflags.py --root <dir>     # scan <dir>/.github/workflows
#   check-coverage-rustflags.py --self-test      # hermetic logic self-test
#
# stdlib-only.

from __future__ import annotations

import argparse
import re
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent

# The tool name install-action is asked for, and the cargo subcommand it provides.
LLVM_COV_TOOL = "cargo-llvm-cov"
_RUN_LLVM_COV_RE = re.compile(r"\bcargo\s+llvm-cov\b")
# `tool: cargo-llvm-cov` / `tool: cargo-llvm-cov,nextest` (install-action takes a CSV list).
_TOOL_RE = re.compile(r"^tool:\s*(.+?)\s*$")
# A job header: `  <id>:` with nothing but an optional comment after the colon.
_JOB_HEADER_RE = re.compile(r"^(\s*)([A-Za-z_][A-Za-z0-9_.-]*):\s*(?:#.*)?$")


def _indent(line: str) -> int:
    """Number of leading spaces (tabs are invalid YAML indent in these files)."""
    return len(line) - len(line.lstrip(" "))


def _is_ignorable(line: str) -> bool:
    return (not line.strip()) or line.lstrip(" ").startswith("#")


def _scalar(raw_value: str) -> str:
    """Normalise a YAML scalar written on one line: strip a wrapping quote pair, else strip
    a trailing ` #` comment. Our values carry no `#` inside quotes, so this is sufficient."""
    v = raw_value.strip()
    if len(v) >= 2 and v[0] in "\"'" and v[-1] == v[0]:
        return v[1:-1]
    # Unquoted: a ` #` begins a comment.
    cut = v.find(" #")
    if cut != -1:
        v = v[:cut]
    return v.strip()


def split_jobs(text: str) -> list[tuple[str, list[str]]]:
    """Split a workflow file into (job_id, body_lines) pairs.

    Finds the top-level `jobs:` key, then treats every key one indent level deeper as a
    job header; the job's body is every following line until the next sibling job header
    or a dedent back to (or past) the `jobs:` indent. Blank/comment lines are retained so
    body lines keep their original indentation semantics.
    """
    lines = text.splitlines()
    n = len(lines)
    i = 0
    jobs_indent: int | None = None
    while i < n:
        m = _JOB_HEADER_RE.match(lines[i])
        if m and m.group(2) == "jobs" and _indent(lines[i]) == 0:
            jobs_indent = 0
            i += 1
            break
        i += 1
    if jobs_indent is None:
        return []

    out: list[tuple[str, list[str]]] = []
    job_indent: int | None = None
    while i < n:
        line = lines[i]
        if _is_ignorable(line):
            i += 1
            continue
        ind = _indent(line)
        if ind <= jobs_indent:
            break  # dedented out of `jobs:`
        m = _JOB_HEADER_RE.match(line)
        if m and (job_indent is None or ind == job_indent):
            job_indent = ind
            job_id = m.group(2)
            body: list[str] = []
            i += 1
            while i < n:
                nxt = lines[i]
                if _is_ignorable(nxt):
                    body.append(nxt)
                    i += 1
                    continue
                nind = _indent(nxt)
                if nind <= job_indent:
                    break  # next job header, or out of `jobs:`
                body.append(nxt)
                i += 1
            out.append((job_id, body))
        else:
            i += 1
    return out


def _body_key_indent(body: list[str]) -> int | None:
    """The indent of the job's own keys (`runs-on:`, `env:`, `steps:` …) — the smallest
    indent among the body's non-ignorable lines."""
    indents = [_indent(ln) for ln in body if not _is_ignorable(ln)]
    return min(indents) if indents else None


def job_level_env(body: list[str]) -> dict[str, str]:
    """Return the job-level `env:` mapping (keys at one level below `env:`).

    Only an `env:` at the JOB-key indent counts — a step's `env:` sits several levels
    deeper and is not what cargo reads for the whole job's builds.
    """
    key_indent = _body_key_indent(body)
    if key_indent is None:
        return {}
    env: dict[str, str] = {}
    inside = False
    for raw in body:
        if _is_ignorable(raw):
            continue
        ind = _indent(raw)
        stripped = raw.lstrip(" ")
        if inside:
            if ind <= key_indent:
                inside = False  # dedented out of the env mapping; fall through
            else:
                k, sep, v = stripped.partition(":")
                if sep:
                    env[k.strip()] = _scalar(v)
                continue
        if ind == key_indent and stripped.startswith("env:"):
            inside = True
    return env


def uses_llvm_cov(body: list[str]) -> bool:
    """True if this job installs cargo-llvm-cov or invokes `cargo llvm-cov`."""
    for raw in body:
        if _is_ignorable(raw):
            continue
        stripped = raw.lstrip(" ")
        if stripped.startswith("- "):
            stripped = stripped[2:]
        m = _TOOL_RE.match(stripped)
        if m and LLVM_COV_TOOL in [t.strip() for t in m.group(1).split(",")]:
            return True
        # A `run:` invoking the subcommand directly (block scalars included: the regex is
        # applied to every body line, so a `cargo llvm-cov …` inside a `run: |` matches).
        if _RUN_LLVM_COV_RE.search(raw):
            return True
    return False


def scan_text(text: str) -> tuple[list[str], dict[str, str]]:
    """Scan one workflow file.

    Returns (offences, values) where `offences` describes coverage jobs with NO job-level
    RUSTFLAGS, and `values` maps each compliant coverage job id to its RUSTFLAGS value (the
    caller cross-checks sameness across files).
    """
    offences: list[str] = []
    values: dict[str, str] = {}
    for job_id, body in split_jobs(text):
        if not uses_llvm_cov(body):
            continue
        env = job_level_env(body)
        if "RUSTFLAGS" not in env:
            offences.append(job_id)
        else:
            values[job_id] = env["RUSTFLAGS"]
    return offences, values


def scan_workflows(root: Path) -> tuple[list[tuple[Path, str]], dict[str, str]]:
    """Scan every workflow under `root`/.github/workflows.

    Returns (missing, values): `missing` is a list of (file, job_id) with no job-level
    RUSTFLAGS; `values` maps "<file>:<job_id>" to the RUSTFLAGS value found.
    """
    wf_dir = root / ".github" / "workflows"
    missing: list[tuple[Path, str]] = []
    values: dict[str, str] = {}
    if not wf_dir.is_dir():
        return missing, values
    for path in sorted(wf_dir.iterdir()):
        if path.suffix not in (".yml", ".yaml"):
            continue
        try:
            text = path.read_text(encoding="utf-8", errors="ignore")
        except OSError:
            continue
        offences, found = scan_text(text)
        for job_id in offences:
            missing.append((path, job_id))
        for job_id, val in found.items():
            values[f"{path.name}:{job_id}"] = val
    return missing, values


# --------------------------------------------------------------------------- tests
_GOOD = """\
jobs:
  coverage-measure:
    runs-on: ubuntu-latest
    env:
      RUSTFLAGS: "-C link-arg=-fuse-ld=lld"
    steps:
      - uses: taiki-e/install-action@deadbeefdeadbeefdeadbeefdeadbeefdeadbeef # cargo-llvm-cov
        with:
          tool: cargo-llvm-cov
      - run: ./scripts/coverage.sh
"""

# The classic regression this gate exists for: the coverage job installs cargo-llvm-cov but
# sets no job-level RUSTFLAGS, so the workflow-level per-target var is silently shadowed.
_BAD_NO_RUSTFLAGS = """\
jobs:
  coverage-measure:
    runs-on: ubuntu-latest
    steps:
      - uses: taiki-e/install-action@deadbeefdeadbeefdeadbeefdeadbeefdeadbeef # cargo-llvm-cov
        with:
          tool: cargo-llvm-cov
      - run: ./scripts/coverage.sh
"""

# A STEP-level env is NOT enough: cargo-llvm-cov's own compiles happen in other steps too
# (and in scripts they shell out to), so only a job-level env covers the whole job.
_BAD_STEP_LEVEL_ENV = """\
jobs:
  coverage-measure:
    runs-on: ubuntu-latest
    steps:
      - uses: taiki-e/install-action@deadbeefdeadbeefdeadbeefdeadbeefdeadbeef # cargo-llvm-cov
        with:
          tool: cargo-llvm-cov
      - name: measure
        env:
          RUSTFLAGS: "-C link-arg=-fuse-ld=lld"
        run: ./scripts/coverage.sh
"""

# A CSV tool list (`cargo-llvm-cov,nextest`) still identifies a coverage job.
_GOOD_CSV_TOOL = """\
jobs:
  coverage-engine-run:
    runs-on: ubuntu-latest
    env:
      RUSTFLAGS: "-C link-arg=-fuse-ld=lld"
    steps:
      - uses: taiki-e/install-action@deadbeefdeadbeefdeadbeefdeadbeefdeadbeef # cargo-llvm-cov
        with:
          tool: cargo-llvm-cov,nextest
"""

# Detected via the `run:` invocation even with no install-action step.
_BAD_RUN_INVOCATION = """\
jobs:
  adhoc-coverage:
    runs-on: ubuntu-latest
    steps:
      - run: cargo llvm-cov report --summary-only
"""

# A no-compile coverage job (presence / monotonicity checks only) is OUT of scope: it never
# links anything, so it needs no linker flag.
_OK_NO_COMPILE_JOB = """\
jobs:
  coverage-floors:
    runs-on: ubuntu-latest
    steps:
      - run: python3 scripts/coverage-presence.py --check
      - run: ./scripts/coverage.sh --check-shards
"""

# A non-coverage job without RUSTFLAGS must never be flagged.
_OK_OTHER_JOB = """\
jobs:
  lint:
    runs-on: ubuntu-latest
    steps:
      - run: cargo clippy --all-targets -- -D warnings
"""

# Two coverage jobs, only the second missing the job-level env.
_MIXED = """\
jobs:
  a:
    runs-on: ubuntu-latest
    env:
      RUSTFLAGS: "-C link-arg=-fuse-ld=lld"
    steps:
      - run: cargo llvm-cov nextest
  b:
    runs-on: ubuntu-latest
    steps:
      - run: cargo llvm-cov report
"""

# Both set RUSTFLAGS but DISAGREE — the engine split's cross-runner .profraw merge requires
# identical compile inputs, so this must be caught (by the caller's sameness check).
_DIVERGENT_VALUES = """\
jobs:
  coverage-engine-run:
    runs-on: ubuntu-latest
    env:
      RUSTFLAGS: "-C link-arg=-fuse-ld=lld"
    steps:
      - run: cargo llvm-cov nextest --no-report
  coverage-engine-merge:
    runs-on: ubuntu-latest
    env:
      RUSTFLAGS: "-C target-cpu=native"
    steps:
      - run: cargo llvm-cov report
"""


def self_test() -> int:
    # (label, text, expected offence count, expected distinct RUSTFLAGS values)
    cases = [
        ("good (job-level RUSTFLAGS)", _GOOD, 0, 1),
        ("bad (coverage job, no RUSTFLAGS)", _BAD_NO_RUSTFLAGS, 1, 0),
        ("bad (step-level env does not count)", _BAD_STEP_LEVEL_ENV, 1, 0),
        ("good (CSV tool list)", _GOOD_CSV_TOOL, 0, 1),
        ("bad (detected via `run: cargo llvm-cov`)", _BAD_RUN_INVOCATION, 1, 0),
        ("ok (no-compile coverage job, out of scope)", _OK_NO_COMPILE_JOB, 0, 0),
        ("ok (non-coverage job)", _OK_OTHER_JOB, 0, 0),
        ("mixed (1 good, 1 bad)", _MIXED, 1, 1),
        ("divergent values (2 good jobs, 2 values)", _DIVERGENT_VALUES, 0, 2),
    ]
    failures = 0
    for label, text, want_offences, want_values in cases:
        offences, values = scan_text(text)
        distinct = len(set(values.values()))
        ok = len(offences) == want_offences and distinct == want_values
        print(
            f"  [{'PASS' if ok else 'FAIL'}] {label}: "
            f"{len(offences)} offence(s) (want {want_offences}), "
            f"{distinct} distinct value(s) (want {want_values})"
        )
        if not ok:
            failures += 1
    if failures:
        print(f"\nself-test: {failures} case(s) FAILED")
        return 1
    print("\nself-test: all cases PASS")
    return 0


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(
        description="Fail if a cargo-llvm-cov coverage job has no job-level RUSTFLAGS, "
        "or if coverage jobs disagree on its value (bead sq-6vshe.11)."
    )
    ap.add_argument(
        "--root",
        default=str(REPO_ROOT),
        help="repo root whose .github/workflows is scanned (default: this repo).",
    )
    ap.add_argument(
        "--self-test",
        action="store_true",
        help="run the hermetic logic self-test and exit.",
    )
    args = ap.parse_args(argv)

    if args.self_test:
        return self_test()

    missing, values = scan_workflows(Path(args.root))
    distinct = sorted(set(values.values()))

    if not missing and len(distinct) <= 1:
        n = len(values)
        print(
            "coverage RUSTFLAGS gate: PASS — "
            f"{n} cargo-llvm-cov job(s), each with a job-level RUSTFLAGS"
            + (f" = {distinct[0]!r}." if distinct else ".")
        )
        return 0

    print("coverage RUSTFLAGS gate: FAIL\n")
    root_path = Path(args.root)
    if missing:
        print(
            "These cargo-llvm-cov jobs declare NO job-level `env: RUSTFLAGS:`.\n"
            "cargo-llvm-cov sets RUSTFLAGS/CARGO_ENCODED_RUSTFLAGS to inject\n"
            "`-C instrument-coverage`, and cargo uses only the FIRST rustflags source it\n"
            "finds — so the workflow-level CARGO_TARGET_<triple>_RUSTFLAGS is silently\n"
            "DROPPED in these jobs and the instrumented links lose the lld flag\n"
            "(bead sq-6vshe.11). Add a job-level `env:` with the shared RUSTFLAGS value:\n"
        )
        for path, job_id in missing:
            rel = path.relative_to(root_path) if path.is_relative_to(root_path) else path
            print(f"    - {rel}: job `{job_id}`")
        print()
    if len(distinct) > 1:
        print(
            "Coverage jobs DISAGREE on the RUSTFLAGS value. The engine coverage split\n"
            "merges .profraw produced by separately-compiled instrumented binaries, which\n"
            "is only valid while every compile sees identical inputs — so all\n"
            "cargo-llvm-cov jobs must use one shared value. Found:\n"
        )
        for key, val in sorted(values.items()):
            print(f"    - {key}: {val!r}")
    return 1


if __name__ == "__main__":
    raise SystemExit(main())
