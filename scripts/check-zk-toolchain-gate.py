#!/usr/bin/env python3
"""Fail-closed checks on the ZK live-toolchain gate.

WHY
    `crates/sparq-zk*/tests/` hold two kinds of integration suite: STRUCTURAL ones
    that need no external tooling, and LIVE ones that shell out to a real `nargo` /
    `bb`. A live test that quietly returns when its tool is absent reports `ok` for a
    body that never ran. `tests/support/mod.rs::require_toolchain` is the single gate
    that ends that: present -> run, absent + `SPARQ_ZK_ALLOW_MISSING_TOOLCHAIN`
    declared -> a named printed skip, absent + undeclared -> a hard failure.

    Two things have to hold for that gate to be worth anything, and neither is
    self-enforcing:

      1. No test may obtain `nargo` / `bb` by any route OTHER than the gate's module.
         A private copy of the old pattern re-opens the soft pass, and every such
         copy so far was written by hand, one file at a time.
      2. The lane that HAS the toolchain must actually run the gated tests. A live
         test that no step runs executes its body nowhere: the toolchain-free lanes
         (`ci.yml`, `feature-matrix.yml`) set the opt-out at workflow level, so the
         gate skips there by declaration.

    This script enforces both by construction. It is a line lint plus a workflow
    reader; it does NOT parse Rust.

CHECK A - acquisition lint (fail-closed)
    Over every `*.rs` under the scan trees below, OUTSIDE the helper modules, a line
    FAILS when it matches any of:

      A1  `Command::new(` whose argument is the executable literal `"nargo"` or
          `"bb"` (the argument is read on the same line, or on the next non-blank
          line when the call's parenthesis ends the line).
      A2  `Command::new(` whose argument is not a string literal at all - the shape
          a const executable, an env-var-derived path or a resolver call takes.
      A3  `ErrorKind::NotFound` - the soft-pass arm.
      A4  a `which` probe (`which::`, `which(`, or a `use` of it).

    A line is exempt ONLY when it carries `// zk-toolchain-gate: allow <reason>`
    with a non-empty reason, on the line itself or on the line immediately above.
    Over-matching is intended: a declared, reasoned exemption is cheap, and a
    missing one is the failure this check exists to cause.

    A1's literal rule is scoped to EXECUTABLE position (the argument of
    `Command::new`) rather than any `"nargo"`/`"bb"` string anywhere. Every one of
    the ~90 `require_toolchain(test, &["nargo", "bb"])` call sites is itself a tool
    name in a string literal, and the error-message suites assert on the tool name
    in `DriverError`'s Display; an any-position rule would require rewriting all of
    them, which this change does not do. A2 is what closes the gap A1's scoping
    leaves: a tool obtained under any other name still has to reach `Command::new`,
    and reaching it with anything but a literal fails.

CHECK B - the helper modules are one module
    The per-crate copies of `tests/support/mod.rs` must be byte-identical.
    `sparq-zk` cannot depend on `sparq-zk-compose`'s test tree (it is the LOWER
    crate: compose depends on it), so the gate is duplicated rather than shared, and
    this check is what keeps the duplicate from drifting.

CHECK C - the lane runs the gated tests
    `zk-toolchain.yml` must carry one step named exactly COVERAGE_STEP, and that
    step must run every helper-using package PACKAGE-WIDE: a `cargo test -p <pkg>`
    invocation with no target selector (`--test`, `--tests`, `--lib`, `--bin(s)`,
    `--example(s)`, `--bench(es)`, `--doc`, `--all-targets`), no `--no-run`, no
    `|| true`, and no test-name filter or `--ignored` / `--exact` after `--`. The
    step must not carry `continue-on-error: true`.

    An `if:` on that step is accepted only when it is the one condition below AND
    that condition is provably always true for every event the workflow can
    receive - `on:` carries no `merge_group`, and the `changes` step's script
    defaults the output to `true`. Re-adding `merge_group` to `on:` turns this red,
    which is the point: the condition would then be able to skip the step.

CHECK D - feature-gated live tests
    A helper-using file with a crate-level `#![cfg(feature = "F")]` compiles to
    nothing unless `F` is on. Such a binary must be run by SOME named step whose
    invocation activates `F` for its package - the package-wide step above, or a
    step that names the binary with `--features`. Feature activation is resolved
    through the crates' own `[features]` tables, so a forwarding feature
    (`sparq-zk-compose/dual-leaf` -> `sparq-zk/dual-leaf`) counts, and a bare
    `--features F` counts only when exactly one package is selected.

CHECK E - ignored live tests
    A live test may be `#[ignore]`d only while a step runs the ignored set: either
    COVERAGE_STEP passes `--include-ignored`, or the separately named
    IGNORED_STEP runs that package's ignored tests package-wide. Detected with a
    line grep for `#[ignore` in helper-using files.

LIMITATIONS (deliberate, documented)
    * The lint is line-based. A tool acquired inside a `macro_rules!` body, or
      through a helper that itself takes the executable name as a parameter from
      somewhere the lint cannot see, is only caught at the point where it reaches
      `Command::new` - which A1/A2 do catch, but the reported line is that point,
      not the caller. A shell indirection (`Command::new("sh").arg("-c")
      .arg("nargo ...")`) is NOT caught: `"sh"` is a literal and the tool name is an
      argument. Closing that needs the any-position literal rule above.
    * CHECK C/D/E read the workflow, so they verify what the lane is WIRED to run,
      not that it ran. The lane's own run is the proof of execution.
    * Feature resolution covers `[features]` entries and `dep/feat` forwarding; it
      does not model optional-dependency activation (`dep:x`) or `?` weak features
      beyond ignoring them.

USAGE
    python3 scripts/check-zk-toolchain-gate.py --check
        Run every check against the repository; exit 0 when clean, 1 with findings.
    python3 scripts/check-zk-toolchain-gate.py --self-test
        Run every check against built-in negative fixtures, one per evasion class;
        exit 0 only when EACH fixture is reported and the clean fixture is not.
    --root PATH
        Check PATH instead of the repository root.

DEPENDENCIES: PyYAML (installed with `python3 -m pip` by the lanes that call this).
"""

from __future__ import annotations

import argparse
import re
import shutil
import sys
import tempfile
import tomllib
from dataclasses import dataclass, field
from pathlib import Path

WORKFLOW_REL = ".github/workflows/zk-toolchain.yml"

# The step that must run every helper-using package package-wide.
COVERAGE_STEP = "Run the default-run live ZK tests (real nargo/bb)"
# The separately named step that runs the ignored set.
IGNORED_STEP = "Run the ignored ZK forge + anchor + e2e suite (real bb)"

# Scan trees. A missing directory is an ERROR: a renamed or moved test tree must not
# silently empty the lint.
SCAN_DIRS: list[tuple[str, str]] = [
    ("sparq-zk", "crates/sparq-zk/tests"),
    ("sparq-zk", "crates/sparq-zk/tests/support"),
    ("sparq-zk-compose", "crates/sparq-zk-compose/tests"),
    ("sparq-zk-compose", "crates/sparq-zk-compose/tests/support"),
]

# The gate itself. Exempt from CHECK A, and required to be byte-identical (CHECK B).
HELPER_MODULES: list[str] = [
    "crates/sparq-zk/tests/support/mod.rs",
    "crates/sparq-zk-compose/tests/support/mod.rs",
]

CARGO_TOMLS: dict[str, str] = {
    "sparq-zk": "crates/sparq-zk/Cargo.toml",
    "sparq-zk-compose": "crates/sparq-zk-compose/Cargo.toml",
}

# The `if:` on COVERAGE_STEP that may be accepted, and only while it is provably
# always true (see CHECK C).
ACCEPTED_STEP_IF = "steps.changes.outputs.zk == 'true'"

LIVE_TOOLS = ("nargo", "bb")
HELPER_CALLS = ("require_toolchain", "live_tool")

EXEMPT_RE = re.compile(r"//\s*zk-toolchain-gate:\s*allow\s+(\S.*)")
COMMAND_NEW_RE = re.compile(r"Command::new\(")
NOTFOUND_RE = re.compile(r"ErrorKind::NotFound")
WHICH_RE = re.compile(r"\bwhich\s*(?:::|\()|^\s*use\s+[^;]*\bwhich\b")
CRATE_CFG_RE = re.compile(r"^#!\[\s*cfg\s*\(")
FEATURE_RE = re.compile(r'feature\s*=\s*"([^"]+)"')
IGNORE_RE = re.compile(r"#\[\s*ignore")

TARGET_SELECTORS = (
    "--test",
    "--tests",
    "--lib",
    "--bin",
    "--bins",
    "--example",
    "--examples",
    "--bench",
    "--benches",
    "--doc",
    "--doctests",
    "--all-targets",
)
VALUE_FLAGS = (
    "-p",
    "--package",
    "--features",
    "--manifest-path",
    "--target",
    "--target-dir",
    "--profile",
    "--jobs",
    "-j",
    "--config",
    "--color",
    "--message-format",
    "--test",
    "--bin",
    "--example",
    "--bench",
    "--exclude",
    "--test-threads",
    "--skip",
    "--format",
    "--logfile",
)


@dataclass
class Finding:
    where: str
    rule: str
    detail: str

    def __str__(self) -> str:
        return f"{self.where}: [{self.rule}] {self.detail}"


# --------------------------------------------------------------------------- #
# CHECK A / B — the acquisition lint
# --------------------------------------------------------------------------- #


def _exempt(lines: list[str], i: int) -> bool:
    """`i` is exempt when it, or the line above it, declares a reason."""
    for j in (i, i - 1):
        if 0 <= j < len(lines) and EXEMPT_RE.search(lines[j]):
            return True
    return False


def _argument_after(lines: list[str], i: int, col: int) -> str:
    """The text following a `Command::new(` at line `i`, column `col`.

    Continues onto the next non-blank line when the call's parenthesis ends the
    line, so the multi-line `Command::new(\\n    "nargo",\\n)` form is read too.
    """
    rest = lines[i][col:].strip()
    if rest:
        return rest
    for j in range(i + 1, min(i + 3, len(lines))):
        if lines[j].strip():
            return lines[j].strip()
    return ""


def lint_file(rel: str, text: str) -> list[Finding]:
    findings: list[Finding] = []
    lines = text.splitlines()
    for i, line in enumerate(lines):
        if _exempt(lines, i):
            continue
        at = f"{rel}:{i + 1}"
        for m in COMMAND_NEW_RE.finditer(line):
            arg = _argument_after(lines, i, m.end())
            lit = re.match(r'"([^"]*)"', arg)
            if lit and lit.group(1) in LIVE_TOOLS:
                findings.append(
                    Finding(
                        at,
                        "A1",
                        f'spawns the executable literal "{lit.group(1)}" directly; '
                        "obtain it through the gate's `support::live_tool` instead",
                    )
                )
            elif not lit:
                findings.append(
                    Finding(
                        at,
                        "A2",
                        f"`Command::new(` with a non-literal argument ({arg[:48]!r}); "
                        "a const, env-derived or resolved executable bypasses the gate",
                    )
                )
        if NOTFOUND_RE.search(line):
            findings.append(
                Finding(
                    at,
                    "A3",
                    "`ErrorKind::NotFound` is the soft-pass arm a missing tool used to "
                    "take; let `support::require_toolchain` decide instead",
                )
            )
        if WHICH_RE.search(line):
            findings.append(
                Finding(at, "A4", "a `which` probe is a private tool gate")
            )
    return findings


def check_helper_modules_identical(root: Path) -> list[Finding]:
    findings: list[Finding] = []
    texts: dict[str, bytes] = {}
    for rel in HELPER_MODULES:
        p = root / rel
        if not p.is_file():
            findings.append(Finding(rel, "B", "helper module is missing"))
            continue
        texts[rel] = p.read_bytes()
    if len(texts) == len(HELPER_MODULES) and len(set(texts.values())) != 1:
        findings.append(
            Finding(
                " vs ".join(HELPER_MODULES),
                "B",
                "the per-crate copies of the gate have drifted; they must be "
                "byte-identical",
            )
        )
    return findings


# --------------------------------------------------------------------------- #
# Source inventory (line greps only)
# --------------------------------------------------------------------------- #


@dataclass
class TestFile:
    package: str
    rel: str
    name: str  # the cargo test-target name (the file stem)
    uses_helper: bool = False
    has_ignore: bool = False
    required_features: set[str] = field(default_factory=set)
    forbidden_features: set[str] = field(default_factory=set)


def collect_sources(root: Path) -> tuple[list[TestFile], list[Finding]]:
    findings: list[Finding] = []
    helper_abs = {(root / h).resolve() for h in HELPER_MODULES}
    seen: set[Path] = set()
    files: list[TestFile] = []
    for package, rel in SCAN_DIRS:
        d = root / rel
        if not d.is_dir():
            findings.append(
                Finding(rel, "SCAN", "scan directory is missing; the lint would be "
                                     "silently empty over it")
            )
            continue
        for path in sorted(d.rglob("*.rs")):
            if path.resolve() in seen:
                continue
            seen.add(path.resolve())
            relp = str(path.relative_to(root))
            text = path.read_text(encoding="utf-8", errors="replace")
            if path.resolve() not in helper_abs:
                findings.extend(lint_file(relp, text))
            # Only a top-level file of a `tests/` dir is a test BINARY.
            is_binary = path.parent.name == "tests"
            tf = TestFile(package=package, rel=relp, name=path.stem)
            tf.uses_helper = any(c in text for c in HELPER_CALLS)
            tf.has_ignore = bool(IGNORE_RE.search(text))
            for line in text.splitlines():
                if not CRATE_CFG_RE.match(line):
                    continue
                names = set(FEATURE_RE.findall(line))
                negated = {
                    f
                    for neg in re.findall(r"not\s*\(([^)]*)\)", line)
                    for f in FEATURE_RE.findall(neg)
                }
                tf.required_features |= names - negated
                tf.forbidden_features |= negated
            if is_binary and path.resolve() not in helper_abs:
                files.append(tf)
    return files, findings


# --------------------------------------------------------------------------- #
# Feature resolution
# --------------------------------------------------------------------------- #


def feature_tables(root: Path) -> dict[str, dict[str, list[str]]]:
    tables: dict[str, dict[str, list[str]]] = {}
    for pkg, rel in CARGO_TOMLS.items():
        p = root / rel
        if not p.is_file():
            tables[pkg] = {}
            continue
        with p.open("rb") as fh:
            tables[pkg] = tomllib.load(fh).get("features", {})
    return tables


def expand(
    seeds: set[tuple[str, str]], tables: dict[str, dict[str, list[str]]]
) -> set[tuple[str, str]]:
    """Transitive closure of (package, feature) activations."""
    out = set(seeds)
    queue = list(seeds)
    while queue:
        pkg, feat = queue.pop()
        for item in tables.get(pkg, {}).get(feat, []):
            if item.startswith("dep:"):
                continue
            if "/" in item:
                dep, _, sub = item.partition("/")
                dep = dep.rstrip("?")
                nxt = (dep, sub)
            else:
                nxt = (pkg, item)
            if nxt not in out:
                out.add(nxt)
                queue.append(nxt)
    return out


# --------------------------------------------------------------------------- #
# Workflow reading
# --------------------------------------------------------------------------- #


@dataclass
class Invocation:
    step: str
    raw: str
    packages: set[str]
    binaries: set[str]
    package_wide: bool
    activated: set[tuple[str, str]]
    rejected: list[str]


@dataclass
class Step:
    name: str
    run: str
    if_: str | None
    continue_on_error: bool
    invocations: list[Invocation]


def _tokens(chunk: str) -> list[str]:
    return [t for t in re.split(r"\s+", chunk.strip()) if t]


def _split_flags(tokens: list[str]) -> tuple[list[str], list[str]]:
    """(tokens before `--`, tokens after `--`)."""
    if "--" in tokens:
        i = tokens.index("--")
        return tokens[:i], tokens[i + 1 :]
    return tokens, []


def _positionals(tokens: list[str]) -> list[str]:
    out: list[str] = []
    i = 0
    while i < len(tokens):
        t = tokens[i]
        if t.startswith("-"):
            bare = t.split("=", 1)[0]
            if "=" not in t and bare in VALUE_FLAGS and i + 1 < len(tokens):
                i += 2
                continue
            i += 1
            continue
        out.append(t)
        i += 1
    return out


def _values(tokens: list[str], flags: tuple[str, ...]) -> list[str]:
    out: list[str] = []
    i = 0
    while i < len(tokens):
        t = tokens[i]
        if "=" in t and t.split("=", 1)[0] in flags:
            out.append(t.split("=", 1)[1])
        elif t in flags and i + 1 < len(tokens):
            out.append(tokens[i + 1])
            i += 1
        i += 1
    return out


def parse_invocation(
    step_name: str, chunk: str, tables: dict[str, dict[str, list[str]]]
) -> Invocation:
    pre, post = _split_flags(_tokens(chunk))
    packages = set(_values(pre, ("-p", "--package")))
    binaries = set(_values(pre, ("--test",)))
    features: set[str] = set()
    for group in _values(pre, ("--features",)):
        features.update(f for f in re.split(r"[,\s]+", group) if f)

    rejected: list[str] = []
    if any(t.split("=", 1)[0] in TARGET_SELECTORS for t in pre):
        rejected.append("has a target selector")
    if "--no-run" in pre:
        rejected.append("has --no-run")
    if "|| true" in chunk or re.search(r";\s*true\b", chunk):
        rejected.append("swallows its exit code")
    for bad in ("--ignored", "--exact"):
        if bad in post:
            rejected.append(f"passes {bad}")
    extra = _positionals(post) + [p for p in _positionals(pre) if p not in ("cargo", "test")]
    if extra:
        rejected.append(f"has a test-name filter ({', '.join(extra)})")

    seeds: set[tuple[str, str]] = set()
    if "--all-features" in pre:
        for pkg in packages or set(tables):
            seeds |= {(pkg, f) for f in tables.get(pkg, {})}
    for tok in features:
        if "/" in tok:
            pkg, _, feat = tok.partition("/")
            seeds.add((pkg.rstrip("?"), feat))
        elif len(packages) == 1:
            # A bare `--features F` is unambiguous only for a single selected
            # package; with several, cargo wants `<pkg>/F` and so does this check.
            seeds.add((next(iter(packages)), tok))
    if "--no-default-features" not in pre:
        for pkg in packages:
            seeds.add((pkg, "default"))

    return Invocation(
        step=step_name,
        raw=chunk.strip(),
        packages=packages,
        binaries=binaries,
        package_wide=not rejected,
        activated=expand(seeds, tables),
        rejected=rejected,
    )


_SPLIT = re.compile(r"(?:^|[;&|]|\n)\s*(?=cargo\s+test\b)")


def read_workflow(root: Path) -> tuple[list[Step], dict, list[Finding]]:
    import yaml  # noqa: PLC0415

    path = root / WORKFLOW_REL
    if not path.is_file():
        return [], {}, [Finding(WORKFLOW_REL, "C", "workflow is missing")]
    doc = yaml.safe_load(path.read_text(encoding="utf-8")) or {}
    tables = feature_tables(root)
    steps: list[Step] = []
    for job in (doc.get("jobs") or {}).values():
        for raw in job.get("steps") or []:
            run = raw.get("run")
            if not run:
                continue
            name = raw.get("name") or "<unnamed step>"
            flat = run.replace("\\\n", " ")
            invs: list[Invocation] = []
            for chunk in _SPLIT.split(flat):
                if not chunk.strip().startswith("cargo test"):
                    continue
                invs.append(parse_invocation(name, chunk.split("\n", 1)[0], tables))
            steps.append(
                Step(
                    name=name,
                    run=run,
                    if_=raw.get("if"),
                    continue_on_error=bool(raw.get("continue-on-error")),
                    invocations=invs,
                )
            )
    return steps, doc, []


def step_if_is_always_true(doc: dict, cond: str) -> bool:
    """Whether COVERAGE_STEP's `if:` cannot skip the step on any event `on:` allows.

    The `changes` step defaults its output to `true` and only the `merge_group`
    branch can set it to `false`, so the condition holds for every event — as long as
    `merge_group` is not a trigger. Re-adding it makes this false.
    """
    if cond.strip() != ACCEPTED_STEP_IF:
        return False
    triggers = doc.get("on", doc.get(True)) or {}
    if isinstance(triggers, dict) and "merge_group" in triggers:
        return False
    if isinstance(triggers, list) and "merge_group" in triggers:
        return False
    for job in (doc.get("jobs") or {}).values():
        for raw in job.get("steps") or []:
            if raw.get("id") == "changes":
                return bool(re.search(r"^\s*zk=true\s*$", raw.get("run", ""), re.M))
    return False


# --------------------------------------------------------------------------- #
# The checks
# --------------------------------------------------------------------------- #


def run_checks(root: Path) -> tuple[list[Finding], list[str]]:
    findings: list[Finding] = []
    notes: list[str] = []

    files, src_findings = collect_sources(root)
    findings.extend(src_findings)
    findings.extend(check_helper_modules_identical(root))

    steps, doc, wf_findings = read_workflow(root)
    findings.extend(wf_findings)
    if not steps:
        return findings, notes

    live = [f for f in files if f.uses_helper]
    packages = sorted({f.package for f in live})
    notes.append(
        "helper-using test binaries: "
        + ", ".join(f"{f.package}::{f.name}" for f in live)
    )

    named = [s for s in steps if s.name == COVERAGE_STEP]
    if len(named) != 1:
        findings.append(
            Finding(
                WORKFLOW_REL,
                "C",
                f"expected exactly one step named {COVERAGE_STEP!r}, found {len(named)}",
            )
        )
        return findings, notes
    step = named[0]

    if step.continue_on_error:
        findings.append(Finding(WORKFLOW_REL, "C", f"{step.name!r} has continue-on-error"))
    if step.if_ is not None and not step_if_is_always_true(doc, str(step.if_)):
        findings.append(
            Finding(
                WORKFLOW_REL,
                "C",
                f"{step.name!r} carries an `if:` that is not provably always true "
                f"({str(step.if_).strip()!r}); it could skip the only step that runs "
                "the gated tests",
            )
        )
    for inv in step.invocations:
        if inv.rejected:
            notes.append(f"not package-wide in {step.name!r}: {'; '.join(inv.rejected)}")

    wide = [i for i in step.invocations if i.package_wide]
    for pkg in packages:
        if not any(pkg in i.packages for i in wide):
            findings.append(
                Finding(
                    WORKFLOW_REL,
                    "C",
                    f"{step.name!r} has no package-wide `cargo test -p {pkg}` "
                    "invocation, so that package's live tests run in no lane",
                )
            )

    # CHECK D — feature-gated live binaries.
    all_invs = [i for s in steps for i in s.invocations]
    for f in live:
        for feature in sorted(f.required_features):
            def covers(inv: Invocation, f=f, feature=feature) -> bool:
                if f.package not in inv.packages:
                    return False
                if not (inv.package_wide or f.name in inv.binaries):
                    return False
                if (f.package, feature) not in inv.activated:
                    return False
                return not any(
                    (f.package, g) in inv.activated for g in f.forbidden_features
                )

            if not any(covers(i) for i in all_invs):
                findings.append(
                    Finding(
                        f.rel,
                        "D",
                        f'is live and crate-level gated on feature "{feature}", but no '
                        "step runs it with that feature activated, so it compiles to "
                        "nothing wherever it does run",
                    )
                )

    # CHECK E — ignored live tests.
    include_ignored = "--include-ignored" in step.run
    sweep = [s for s in steps if s.name == IGNORED_STEP]
    swept: set[str] = set()
    for s in sweep:
        for inv in s.invocations:
            if "--ignored" in _split_flags(_tokens(inv.raw))[1] and "--no-run" not in inv.raw:
                swept |= inv.packages
    for f in live:
        if not f.has_ignore:
            continue
        if include_ignored or f.package in swept:
            continue
        findings.append(
            Finding(
                f.rel,
                "E",
                "holds `#[ignore]`d live tests, but neither "
                f"{COVERAGE_STEP!r} (--include-ignored) nor {IGNORED_STEP!r} runs "
                f"the ignored set for {f.package}",
            )
        )
    return findings, notes


def report(root: Path, quiet: bool = False) -> int:
    findings, notes = run_checks(root)
    if not quiet:
        for n in notes:
            print(f"note: {n}")
    if not findings:
        if not quiet:
            print("\nOK: the ZK live-toolchain gate is the only acquisition path and "
                  "the lane runs every gated package.")
        return 0
    print("\nFAIL:", file=sys.stderr)
    for f in findings:
        print(f"  {f}", file=sys.stderr)
    print(
        "\nA1/A2/A3/A4 are the acquisition lint: route the tool through "
        "`support::require_toolchain` + `support::live_tool`, or declare the line "
        "with `// zk-toolchain-gate: allow <reason>`. B is helper-module drift. "
        "C/D/E are the lane wiring in " + WORKFLOW_REL + ".",
        file=sys.stderr,
    )
    return 1


# --------------------------------------------------------------------------- #
# Self-test — one negative fixture per evasion class
# --------------------------------------------------------------------------- #

_WF = """\
name: zk-toolchain
on:
  schedule:
    - cron: "23 4 * * *"
  workflow_dispatch:
  pull_request:
  push:
    branches: [main]
jobs:
  forge-suite:
    runs-on: ubuntu-latest
    steps:
      - name: Detect zk changes (merge_group changed-files gate)
        id: changes
        run: |
          zk=true
          echo "zk=${zk}" >> "$GITHUB_OUTPUT"
      - name: __COVERAGE__
        if: steps.changes.outputs.zk == 'true'
        run: |
          cargo test -p sparq-zk-compose -p sparq-zk -- --test-threads=1
      - name: __IGNORED__
        if: steps.changes.outputs.zk == 'true'
        run: |
          cargo test -p sparq-zk-compose -p sparq-zk -- --ignored --test-threads=1
      - name: Run the dual-leaf lane suites (default-off feature, toolchain on PATH)
        if: steps.changes.outputs.zk == 'true'
        run: |
          cargo test -p sparq-zk -p sparq-zk-compose --features sparq-zk-compose/dual-leaf \\
            --test dual_leaf_lane \\
            -- --test-threads=1
""".replace("__COVERAGE__", COVERAGE_STEP).replace("__IGNORED__", IGNORED_STEP)

_SUPPORT = '''\
pub const ALLOW_MISSING_ENV: &str = "SPARQ_ZK_ALLOW_MISSING_TOOLCHAIN";

pub fn live_tool(tool: &str) -> std::process::Command {
    std::process::Command::new(tool)
}

pub fn require_toolchain(_test: &str, _tools: &[&str]) -> bool {
    true
}
'''

_CARGO = """\
[package]
name = "%s"
version = "0.0.0"

[features]
default = []
dual-leaf = [%s]
"""

_GOOD_LIVE = '''\
mod support;

#[test]
fn live_case() {
    if !support::require_toolchain("live_case", &["nargo", "bb"]) {
        return;
    }
    let _ = support::live_tool("nargo").arg("execute").output();
}
'''

_GOOD_IGNORED = '''\
mod support;

#[test]
#[ignore = "needs the live toolchain"]
fn forge_case() {
    assert!(support::require_toolchain("forge_case", &["nargo", "bb"]));
}
'''

_GOOD_FEATURE_LIVE = '''\
#![cfg(feature = "dual-leaf")]

mod support;

#[test]
fn in_circuit_case() {
    if !support::require_toolchain("in_circuit_case", &["nargo"]) {
        return;
    }
    let _ = support::live_tool("nargo").output();
}
'''

# ---- the evasion classes, one fixture each ----

EVASIONS: dict[str, tuple[str, str]] = {
    # name: (relative file under crates/sparq-zk-compose/tests, contents)
    "helper wrapper outside the module": (
        "evade_wrapper.rs",
        'fn gate() -> bool {\n'
        '    std::process::Command::new("nargo").output().is_ok()\n'
        "}\n"
        "\n#[test]\nfn case() {\n    assert!(gate());\n}\n",
    ),
    "const executable": (
        "evade_const.rs",
        'const TOOL: &str = "nargo";\n'
        "\n#[test]\nfn case() {\n"
        "    let _ = std::process::Command::new(TOOL).output();\n"
        "}\n",
    ),
    "which probe": (
        "evade_which.rs",
        "#[test]\nfn case() {\n"
        '    let found = which::which("nargo").is_ok();\n'
        "    assert!(found);\n"
        "}\n",
    ),
    "env-gated driver": (
        "evade_env.rs",
        "#[test]\nfn case() {\n"
        '    let bin = std::env::var("NARGO_BIN").unwrap_or_default();\n'
        "    let _ = std::process::Command::new(bin).output();\n"
        "}\n",
    ),
    "one-line test fn": (
        "evade_oneline.rs",
        '#[test] fn case() { let _ = std::process::Command::new("bb").output(); }\n',
    ),
    "NotFound soft-pass": (
        "evade_notfound.rs",
        "mod support;\n"
        "\n#[test]\nfn case() {\n"
        '    let r = support::live_tool("nargo").output();\n'
        "    if matches!(&r, Err(e) if e.kind() == std::io::ErrorKind::NotFound) {\n"
        "        return;\n"
        "    }\n"
        "    assert!(r.is_ok());\n"
        "}\n",
    ),
}


def _write_fixture(root: Path) -> None:
    (root / WORKFLOW_REL).parent.mkdir(parents=True, exist_ok=True)
    (root / WORKFLOW_REL).write_text(_WF, encoding="utf-8")
    for pkg, rel in CARGO_TOMLS.items():
        p = root / rel
        p.parent.mkdir(parents=True, exist_ok=True)
        fwd = '"sparq-zk/dual-leaf"' if pkg == "sparq-zk-compose" else ""
        p.write_text(_CARGO % (pkg, fwd), encoding="utf-8")
    for _pkg, rel in SCAN_DIRS:
        (root / rel).mkdir(parents=True, exist_ok=True)
    for helper in HELPER_MODULES:
        (root / helper).write_text(_SUPPORT, encoding="utf-8")
    compose = root / "crates/sparq-zk-compose/tests"
    (compose / "live.rs").write_text(_GOOD_LIVE, encoding="utf-8")
    (compose / "forge.rs").write_text(_GOOD_IGNORED, encoding="utf-8")
    (compose / "dual_leaf_lane.rs").write_text(_GOOD_FEATURE_LIVE, encoding="utf-8")
    (root / "crates/sparq-zk/tests/live_zk.rs").write_text(_GOOD_LIVE, encoding="utf-8")


def _fresh(stack: list[tempfile.TemporaryDirectory]) -> Path:
    td = tempfile.TemporaryDirectory()
    stack.append(td)
    root = Path(td.name)
    _write_fixture(root)
    return root


def self_test() -> int:
    stack: list[tempfile.TemporaryDirectory] = []
    failures: list[str] = []

    def expect_clean(root: Path, label: str) -> None:
        findings, _ = run_checks(root)
        if findings:
            failures.append(f"{label} must be clean, got: " + "; ".join(map(str, findings)))

    def expect_red(root: Path, label: str, rule: str | None = None) -> None:
        findings, _ = run_checks(root)
        if not findings:
            failures.append(f"{label} must be reported, but the check was clean")
        elif rule and not any(f.rule == rule for f in findings):
            failures.append(
                f"{label} must be reported as {rule}, got "
                + "; ".join(f.rule for f in findings)
            )

    expect_clean(_fresh(stack), "the clean fixture")

    # (A) acquisition evasions.
    for label, (name, body) in EVASIONS.items():
        root = _fresh(stack)
        (root / "crates/sparq-zk-compose/tests" / name).write_text(body, encoding="utf-8")
        expect_red(root, f"evasion: {label}")

    # An exemption with a reason clears it; one without a reason does not.
    root = _fresh(stack)
    (root / "crates/sparq-zk-compose/tests/evade_const.rs").write_text(
        'const TOOL: &str = "nargo";\n'
        "\n#[test]\nfn case() {\n"
        "    // zk-toolchain-gate: allow a declared reason\n"
        "    let _ = std::process::Command::new(TOOL).output();\n"
        "}\n",
        encoding="utf-8",
    )
    expect_clean(root, "a declared exemption")
    root = _fresh(stack)
    (root / "crates/sparq-zk-compose/tests/evade_const.rs").write_text(
        'const TOOL: &str = "nargo";\n'
        "\n#[test]\nfn case() {\n"
        "    // zk-toolchain-gate: allow\n"
        "    let _ = std::process::Command::new(TOOL).output();\n"
        "}\n",
        encoding="utf-8",
    )
    expect_red(root, "an exemption with no reason", "A2")

    # (B) helper-module drift.
    root = _fresh(stack)
    (root / HELPER_MODULES[0]).write_text(_SUPPORT + "// drift\n", encoding="utf-8")
    expect_red(root, "helper-module drift", "B")

    # (SCAN) a missing scan directory.
    root = _fresh(stack)
    shutil.rmtree(root / "crates/sparq-zk/tests/support")
    expect_red(root, "a missing scan directory", "SCAN")

    # (C) the coverage step's shape.
    for label, repl in (
        ("the step gone", ("      - name: " + COVERAGE_STEP, "      - name: Something else")),
        (
            "a missing package",
            (
                "cargo test -p sparq-zk-compose -p sparq-zk -- --test-threads=1\n      - name: "
                + IGNORED_STEP,
                "cargo test -p sparq-zk-compose -- --test-threads=1\n      - name: "
                + IGNORED_STEP,
            ),
        ),
        (
            "--no-run on the step",
            (
                "cargo test -p sparq-zk-compose -p sparq-zk -- --test-threads=1\n      - name: "
                + IGNORED_STEP,
                "cargo test --no-run -p sparq-zk-compose -p sparq-zk -- --test-threads=1\n"
                "      - name: " + IGNORED_STEP,
            ),
        ),
        (
            "a test-name filter on the step",
            (
                "cargo test -p sparq-zk-compose -p sparq-zk -- --test-threads=1\n      - name: "
                + IGNORED_STEP,
                "cargo test -p sparq-zk-compose -p sparq-zk -- live_case --test-threads=1\n"
                "      - name: " + IGNORED_STEP,
            ),
        ),
        (
            "-- --ignored on the step",
            (
                "cargo test -p sparq-zk-compose -p sparq-zk -- --test-threads=1\n      - name: "
                + IGNORED_STEP,
                "cargo test -p sparq-zk-compose -p sparq-zk -- --ignored --test-threads=1\n"
                "      - name: " + IGNORED_STEP,
            ),
        ),
        (
            "|| true on the step",
            (
                "cargo test -p sparq-zk-compose -p sparq-zk -- --test-threads=1\n      - name: "
                + IGNORED_STEP,
                "cargo test -p sparq-zk-compose -p sparq-zk -- --test-threads=1 || true\n"
                "      - name: " + IGNORED_STEP,
            ),
        ),
        (
            "a target selector on the step",
            (
                "cargo test -p sparq-zk-compose -p sparq-zk -- --test-threads=1\n      - name: "
                + IGNORED_STEP,
                "cargo test -p sparq-zk-compose -p sparq-zk --test live -- --test-threads=1\n"
                "      - name: " + IGNORED_STEP,
            ),
        ),
    ):
        root = _fresh(stack)
        wf = root / WORKFLOW_REL
        text = wf.read_text(encoding="utf-8")
        old, new = repl
        if old not in text:
            failures.append(f"fixture {label!r} could not be planted")
            continue
        wf.write_text(text.replace(old, new, 1), encoding="utf-8")
        expect_red(root, f"coverage step: {label}", "C")

    # (C) an `if:` that can actually skip the step, and a merge_group trigger that
    # makes the accepted one skippable again.
    root = _fresh(stack)
    wf = root / WORKFLOW_REL
    wf.write_text(
        wf.read_text(encoding="utf-8").replace(
            f"      - name: {COVERAGE_STEP}\n        if: {ACCEPTED_STEP_IF}",
            f"      - name: {COVERAGE_STEP}\n        if: github.event_name == 'schedule'",
            1,
        ),
        encoding="utf-8",
    )
    expect_red(root, "coverage step: a skippable `if:`", "C")

    root = _fresh(stack)
    wf = root / WORKFLOW_REL
    wf.write_text(
        wf.read_text(encoding="utf-8").replace(
            "  workflow_dispatch:", "  workflow_dispatch:\n  merge_group:", 1
        ),
        encoding="utf-8",
    )
    expect_red(root, "coverage step: merge_group re-added", "C")

    # (D) a feature-gated live test whose step no longer activates the feature.
    root = _fresh(stack)
    wf = root / WORKFLOW_REL
    wf.write_text(
        wf.read_text(encoding="utf-8").replace(
            "--features sparq-zk-compose/dual-leaf \\\n", "", 1
        ),
        encoding="utf-8",
    )
    expect_red(root, "a feature-gated live test run without its feature", "D")

    # (E) an ignored live test with no sweep.
    root = _fresh(stack)
    wf = root / WORKFLOW_REL
    wf.write_text(
        wf.read_text(encoding="utf-8").replace(
            f"      - name: {IGNORED_STEP}", "      - name: Something else", 1
        ),
        encoding="utf-8",
    )
    expect_red(root, "an ignored live test with no sweep", "E")

    for td in stack:
        td.cleanup()
    if failures:
        print("SELF-TEST FAILED (the checks are broken):", file=sys.stderr)
        for f in failures:
            print(f"  - {f}", file=sys.stderr)
        return 1
    print(
        f"SELF-TEST OK: the clean fixture passes and all {len(EVASIONS) + 13} planted "
        "evasions (acquisition, exemption-without-reason, helper drift, missing scan "
        "dir, coverage-step shape, feature gap, ignored-without-sweep) are reported."
    )
    return 0


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description="Fail-closed checks on the ZK toolchain gate.")
    ap.add_argument("--check", action="store_true", help="check the tree (default)")
    ap.add_argument("--self-test", action="store_true", help="run the negative fixtures")
    ap.add_argument("--root", default=None, help="root to check (default: this repo)")
    args = ap.parse_args(argv)
    if args.self_test:
        return self_test()
    root = Path(args.root) if args.root else Path(__file__).resolve().parent.parent
    return report(root)


if __name__ == "__main__":
    sys.exit(main())
