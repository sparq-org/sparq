#!/usr/bin/env python3
"""Enumerate the TOOLCHAIN-BOUND test binaries of the ZK crates and check that
`.github/workflows/zk-toolchain.yml` actually runs each of them.

WHY
    `crates/sparq-zk*/tests/` hold two kinds of integration suite: STRUCTURAL ones
    that need no external tooling, and LIVE ones that shell out to a real `nargo` /
    `bb`. A live test whose tool is absent from `PATH` does NOT fail by default —
    `tests/support/mod.rs::require_toolchain` turns it into a printed skip whenever
    the lane declares itself toolchain-free via `SPARQ_ZK_ALLOW_MISSING_TOOLCHAIN`,
    which the toolchain-free lanes (`ci.yml`, `feature-matrix.yml`) set at workflow
    level. So a live, NON-`#[ignore]`d test reports `ok` in every lane except the
    `zk-toolchain` lane, and the `zk-toolchain` lane's own sweep of the ignored
    suite (`cargo test -- --ignored`) SKIPS every non-ignored test. A non-ignored
    live binary that no explicit `cargo test --test <bin>` invocation in that
    workflow names therefore executes its live body in NO CI lane at all.

    That set was maintained by hand, one `--test` line at a time, so a newly added
    binary was missed by default. This script derives the set MECHANICALLY from the
    test sources and fails when the workflow does not count one.

WHAT COUNTS AS TOOLCHAIN-BOUND (the marker)
    A test binary is toolchain-bound when it has at least one NON-`#[ignore]`d
    `#[test]` function matching either marker:

    M1  the DECLARED gate: the body calls `require_toolchain(...)` and the tool list
        passed to it names `"nargo"` and/or `"bb"`. This is the repo's single live
        gate (`crates/sparq-zk-compose/tests/support/mod.rs`); a tool list of
        anything else (the gate's own self-test passes `"cargo"` and a deliberately
        absent name) is NOT toolchain-bound.

    M2  a PRIVATE skip gate: the body spawns `Command::new("nargo")` or
        `Command::new("bb")` directly, does NOT route through `require_toolchain`,
        and contains a `return` — i.e. it is a surviving copy of the pre-gate
        pattern that quietly returns when the tool is missing. Counting M2 keeps the
        marker from being bypassable by writing a fresh private copy of the gate.

    An `#[ignore]`d live test is NOT counted: the workflow's
    `cargo test -p sparq-zk-compose -p sparq-zk -- --ignored` sweep already runs
    every ignored test in both packages with the toolchain on `PATH`.

WHAT COUNTS AS COVERAGE
    A `cargo test` invocation inside a `run:` block of `zk-toolchain.yml` that names
    the binary with `--test <bin>` (or `--test=<bin>`). When the binary's file
    carries a crate-level `#![cfg(feature = "F")]`, the naming invocation must also
    activate `F` (`--features F`, `--features <pkg>/F`, or `--all-features`) —
    otherwise the binary compiles to nothing there and the coverage is nominal only.

LIMITATIONS (deliberate, documented)
    * Marker detection is per-test-function and non-transitive: a live spawn reached
      only through a helper fn that the test calls is seen only if the test also
      calls `require_toolchain` (M1). Helpers that assert in BOTH the present and
      absent cases (e.g. `tests/coverage_direct.rs`) are correctly not counted —
      they have no skip path to hide.
    * Only `crates/sparq-zk/tests/*.rs` and `crates/sparq-zk-compose/tests/*.rs`
      top-level files are scanned (a test BINARY is one such file); `tests/support/`
      and other subdirectories are modules, not binaries. The binary NAME is taken
      to be the file stem (the cargo convention); neither crate declares a
      `[[test]]` entry that renames a target, and a renaming entry would make the
      derived name wrong.
    * `#[cfg(...)]` on an individual test fn is not interpreted; only the
      crate-level inner `#![cfg(feature = ...)]` attributes are.

USAGE
    python3 scripts/check-zk-toolchain-gate-coverage.py --check
        Scan the repository; exit 0 when every toolchain-bound binary is counted,
        exit 1 with findings otherwise.
    python3 scripts/check-zk-toolchain-gate-coverage.py --list
        Print the enumeration (binary, marker, counting step) and exit 0.
    python3 scripts/check-zk-toolchain-gate-coverage.py --self-test
        Run the detector against synthetic trees: one where a planted, unlisted
        toolchain-bound binary MUST be reported, and one where the same tree lists
        it and MUST come back clean. Exit 0 only if the detector behaves.
    --root PATH
        Scan PATH instead of the repository root (used by --self-test and the unit
        test in scripts/tests/test_zk_toolchain_gate_coverage.py).

DEPENDENCIES: PyYAML (the workflow step installs it hash-pinned from
.github/requirements/docs-quality.txt).
"""

from __future__ import annotations

import argparse
import os
import re
import sys
import tempfile
from dataclasses import dataclass, field
from pathlib import Path

WORKFLOW_REL = ".github/workflows/zk-toolchain.yml"

# (cargo package, tests dir) — one test BINARY per top-level .rs file.
TEST_DIRS: list[tuple[str, str]] = [
    ("sparq-zk", "crates/sparq-zk/tests"),
    ("sparq-zk-compose", "crates/sparq-zk-compose/tests"),
]

# The tools whose absence the live gate hides.
LIVE_TOOLS = ("nargo", "bb")


# --------------------------------------------------------------------------- #
# Rust source scanning
# --------------------------------------------------------------------------- #


def strip_comments(src: str) -> str:
    """Blank out `//`-to-EOL and `/* */` comments, preserving string literals and
    offsets/line structure (newlines inside a block comment are kept)."""
    out: list[str] = []
    i, n = 0, len(src)
    while i < n:
        c = src[i]
        if c == '"':
            out.append(c)
            i += 1
            while i < n:
                if src[i] == "\\":
                    out.append(src[i : i + 2])
                    i += 2
                    continue
                out.append(src[i])
                if src[i] == '"':
                    i += 1
                    break
                i += 1
            continue
        if c == "r" and i + 1 < n and src[i + 1] in '#"':
            # Raw string: r"..."  /  r#"..."#  /  r##"..."##
            j = i + 1
            hashes = 0
            while j < n and src[j] == "#":
                hashes += 1
                j += 1
            if j < n and src[j] == '"':
                close = '"' + "#" * hashes
                end = src.find(close, j + 1)
                end = n if end == -1 else end + len(close)
                out.append(src[i:end])
                i = end
                continue
        if c == "/" and i + 1 < n and src[i + 1] == "/":
            while i < n and src[i] != "\n":
                i += 1
            continue
        if c == "/" and i + 1 < n and src[i + 1] == "*":
            depth, i = 1, i + 2
            while i < n and depth:
                if src.startswith("/*", i):
                    depth += 1
                    i += 2
                elif src.startswith("*/", i):
                    depth -= 1
                    i += 2
                else:
                    if src[i] == "\n":
                        out.append("\n")
                    i += 1
            continue
        out.append(c)
        i += 1
    return "".join(out)


_ATTR_RE = re.compile(r"^#!?\[")
_FN_RE = re.compile(r"^(?:pub(?:\([^)]*\))?\s+)?(?:async\s+)?(?:unsafe\s+)?fn\s+(\w+)")
_CRATE_FEATURE_RE = re.compile(r'#!\[\s*cfg\s*\(.*?feature\s*=\s*"([^"]+)"', re.DOTALL)
_ALL_CRATE_FEATURES_RE = re.compile(r'feature\s*=\s*"([^"]+)"')


def _match_braces(src: str, start: int) -> str:
    """Return the `{...}` block that begins at the first `{` at/after `start`."""
    open_at = src.find("{", start)
    if open_at == -1:
        return ""
    depth, i, n = 0, open_at, len(src)
    while i < n:
        ch = src[i]
        if ch == '"':
            i += 1
            while i < n:
                if src[i] == "\\":
                    i += 2
                    continue
                if src[i] == '"':
                    break
                i += 1
        elif ch == "{":
            depth += 1
        elif ch == "}":
            depth -= 1
            if depth == 0:
                return src[open_at : i + 1]
        i += 1
    return src[open_at:]


def _call_args(body: str, callee: str) -> list[str]:
    """Parenthesised argument text of every `callee(...)` call in `body`."""
    args: list[str] = []
    for m in re.finditer(re.escape(callee) + r"\s*\(", body):
        depth, i, n = 0, m.end() - 1, len(body)
        while i < n:
            ch = body[i]
            if ch == '"':
                i += 1
                while i < n:
                    if body[i] == "\\":
                        i += 2
                        continue
                    if body[i] == '"':
                        break
                    i += 1
            elif ch == "(":
                depth += 1
            elif ch == ")":
                depth -= 1
                if depth == 0:
                    args.append(body[m.end() : i])
                    break
            i += 1
    return args


@dataclass
class Binary:
    """One test binary (one top-level file under a scanned `tests/` dir)."""

    package: str
    name: str
    path: str
    required_features: set[str] = field(default_factory=set)
    # test fn name -> marker ("M1 declared gate" / "M2 private skip gate")
    bound_tests: dict[str, str] = field(default_factory=dict)
    ignored_bound_tests: list[str] = field(default_factory=list)

    @property
    def toolchain_bound(self) -> bool:
        return bool(self.bound_tests)


def scan_binary(package: str, path: Path) -> Binary:
    raw = path.read_text(encoding="utf-8", errors="replace")
    src = strip_comments(raw)
    binary = Binary(package=package, name=path.stem, path=str(path))

    for attr in _CRATE_FEATURE_RE.finditer(src):
        # Union of every `feature = "F"` in the inner cfg attribute that matched
        # (conservative, as scripts/check-feature-test-execution.py does).
        span = src[attr.start() : src.find("]", attr.start()) + 1]
        binary.required_features.update(_ALL_CRATE_FEATURES_RE.findall(span))

    lines = src.splitlines(keepends=True)
    attrs: list[str] = []
    pending = ""
    offset = 0
    for line in lines:
        here = offset
        offset += len(line)
        stripped = line.strip()
        if pending:
            pending += " " + stripped
            if pending.count("[") <= pending.count("]"):
                attrs.append(pending)
                pending = ""
            continue
        if _ATTR_RE.match(stripped):
            if stripped.count("[") > stripped.count("]"):
                pending = stripped
            else:
                attrs.append(stripped)
            continue
        if not stripped:
            continue
        fn = _FN_RE.match(stripped)
        if fn:
            is_test = any(a.startswith("#[test") for a in attrs)
            if is_test:
                ignored = any(a.startswith("#[ignore") for a in attrs)
                body = _match_braces(src, here)
                marker = classify(body)
                if marker:
                    if ignored:
                        binary.ignored_bound_tests.append(fn.group(1))
                    else:
                        binary.bound_tests[fn.group(1)] = marker
            attrs = []
            continue
        attrs = []
    return binary


def classify(body: str) -> str | None:
    """The marker this test body matches, or None."""
    gate_args = _call_args(body, "require_toolchain")
    if gate_args:
        for arg in gate_args:
            names = set(re.findall(r'"([^"]+)"', arg))
            if names & set(LIVE_TOOLS):
                return "M1 declared gate"
        # Calls the gate, but for tools whose absence is not the live-toolchain
        # trap (the gate's own self-test). Not toolchain-bound, and not an
        # undeclared private gate either.
        return None
    spawns = _call_args(body, "Command::new")
    for arg in spawns:
        if re.fullmatch(r'\s*"(%s)"\s*' % "|".join(LIVE_TOOLS), arg):
            if re.search(r"\breturn\b", body):
                return "M2 private skip gate"
    return None


def enumerate_binaries(root: Path) -> list[Binary]:
    found: list[Binary] = []
    for package, rel in TEST_DIRS:
        d = root / rel
        if not d.is_dir():
            continue
        for path in sorted(d.glob("*.rs")):
            found.append(scan_binary(package, path))
    return found


# --------------------------------------------------------------------------- #
# Workflow scanning
# --------------------------------------------------------------------------- #


@dataclass
class Invocation:
    step: str
    binaries: set[str]
    features: set[str]
    all_features: bool


_CARGO_TEST_SPLIT = re.compile(r"(?:^|[;&|]|\n)\s*(?=cargo\s+test\b)")


def parse_workflow(path: Path) -> list[Invocation]:
    import yaml  # noqa: PLC0415  (installed hash-pinned by the workflow step)

    doc = yaml.safe_load(path.read_text(encoding="utf-8"))
    invocations: list[Invocation] = []
    for job in (doc.get("jobs") or {}).values():
        for step in job.get("steps") or []:
            run = step.get("run")
            if not run:
                continue
            name = step.get("name") or "<unnamed step>"
            # Join shell line-continuations so a multi-line invocation is one string.
            flat = run.replace("\\\n", " ")
            for chunk in _CARGO_TEST_SPLIT.split(flat):
                if not chunk.strip().startswith("cargo test"):
                    continue
                chunk = chunk.split("\n", 1)[0]
                bins = set(re.findall(r"--test[ =]+([A-Za-z0-9_]+)", chunk))
                feats: set[str] = set()
                for group in re.findall(r"--features[ =]+([^\s]+)", chunk):
                    feats.update(f for f in re.split(r"[,\s]+", group) if f)
                if bins:
                    invocations.append(
                        Invocation(
                            step=name,
                            binaries=bins,
                            features=feats,
                            all_features="--all-features" in chunk,
                        )
                    )
    return invocations


def activates(inv: Invocation, feature: str) -> bool:
    if inv.all_features:
        return True
    return any(f == feature or f.endswith("/" + feature) for f in inv.features)


# --------------------------------------------------------------------------- #
# The check
# --------------------------------------------------------------------------- #


@dataclass
class Finding:
    binary: Binary
    reason: str


def check(root: Path) -> tuple[list[Binary], list[Invocation], list[Finding]]:
    binaries = enumerate_binaries(root)
    invocations = parse_workflow(root / WORKFLOW_REL)
    findings: list[Finding] = []
    for binary in binaries:
        if not binary.toolchain_bound:
            continue
        naming = [inv for inv in invocations if binary.name in inv.binaries]
        if not naming:
            findings.append(
                Finding(
                    binary,
                    f"no `cargo test --test {binary.name}` invocation in {WORKFLOW_REL}",
                )
            )
            continue
        for feature in sorted(binary.required_features):
            if not any(activates(inv, feature) for inv in naming):
                findings.append(
                    Finding(
                        binary,
                        f"named by {', '.join(repr(i.step) for i in naming)} but none of "
                        f'those invocations activates the crate-level feature "{feature}", '
                        "so the binary compiles to nothing there",
                    )
                )
    return binaries, invocations, findings


def counting_steps(binary: Binary, invocations: list[Invocation]) -> str:
    steps = [inv.step for inv in invocations if binary.name in inv.binaries]
    return "; ".join(dict.fromkeys(steps)) if steps else "(none)"


def render(binaries: list[Binary], invocations: list[Invocation]) -> str:
    out = ["Toolchain-bound test binaries (non-ignored live tests):"]
    for b in binaries:
        if not b.toolchain_bound:
            continue
        markers = sorted({m for m in b.bound_tests.values()})
        feats = f" [features: {', '.join(sorted(b.required_features))}]" if b.required_features else ""
        out.append(
            f"  {b.package}::{b.name}{feats} — {len(b.bound_tests)} live test(s), "
            f"{'/'.join(markers)} → counted by: {counting_steps(b, invocations)}"
        )
    ignored_only = [
        b for b in binaries if not b.toolchain_bound and b.ignored_bound_tests
    ]
    if ignored_only:
        out.append(
            "Ignored-only live binaries (covered by the `-- --ignored` sweep): "
            + ", ".join(f"{b.name}" for b in ignored_only)
        )
    return "\n".join(out)


def run_check(root: Path, quiet: bool = False) -> int:
    binaries, invocations, findings = check(root)
    if not quiet:
        print(render(binaries, invocations))
    if not findings:
        if not quiet:
            print(
                f"\nOK: every toolchain-bound binary is counted by {WORKFLOW_REL}."
            )
        return 0
    print("\nFAIL: toolchain-bound test binaries the lane does not count:", file=sys.stderr)
    for f in findings:
        print(f"  {f.binary.path}: {f.reason}", file=sys.stderr)
        for name, marker in sorted(f.binary.bound_tests.items()):
            print(f"      non-ignored live test `{name}` ({marker})", file=sys.stderr)
    print(
        "\nWithout a `--test <bin>` invocation in that workflow, these live bodies run in "
        "NO CI lane: the toolchain-free lanes declare "
        "SPARQ_ZK_ALLOW_MISSING_TOOLCHAIN and skip them, and this lane's "
        "`-- --ignored` sweep skips every non-ignored test. Add the binary to a step "
        f"in {WORKFLOW_REL} (or `#[ignore]` the live test so the ignored sweep runs it).",
        file=sys.stderr,
    )
    return 1


# --------------------------------------------------------------------------- #
# Self-test
# --------------------------------------------------------------------------- #

_FIXTURE_WORKFLOW = """\
name: zk-toolchain
on: [workflow_dispatch]
jobs:
  forge-suite:
    runs-on: ubuntu-latest
    steps:
      - name: Run the counted gates
        run: |
          cargo test -p sparq-zk-compose --test counted_live -- --test-threads=1
      - name: Run the dual-leaf lane suites
        run: |
          cargo test -p sparq-zk-compose --features sparq-zk-compose/dual-leaf \\
            --test counted_feature_live \\
            -- --test-threads=1
      - name: Run the ignored suite
        run: cargo test -p sparq-zk-compose -p sparq-zk -- --ignored --test-threads=1
"""

_FIXTURE_COUNTED = """\
mod support;

#[test]
fn live_case() {
    if !support::require_toolchain("live_case", &["nargo", "bb"]) {
        return;
    }
    assert!(true);
}
"""

_FIXTURE_COUNTED_FEATURE = """\
#![cfg(feature = "dual-leaf")]

mod support;

#[test]
fn in_circuit_case() {
    if !support::require_toolchain("in_circuit_case", &["nargo"]) {
        return;
    }
    assert!(true);
}
"""

_FIXTURE_IGNORED_ONLY = """\
mod support;

#[test]
#[ignore = "needs the live toolchain"]
fn forge_case() {
    assert!(support::require_toolchain("forge_case", &["nargo", "bb"]));
}
"""

_FIXTURE_NOT_BOUND = """\
#[test]
fn structural_case() {
    // Mentions nargo only in prose, and the gate self-test's tool list is not live.
    assert_eq!(2 + 2, 4);
}
"""

_FIXTURE_GATE_SELFTEST = """\
mod support;

#[test]
fn a_tool_on_path_passes_the_gate() {
    assert!(support::require_toolchain("a_tool_on_path_passes_the_gate", &["cargo"]));
}
"""

# The planted violations. One per marker.
_FIXTURE_PLANTED_M1 = """\
mod support;

#[test]
fn planted_live_case() {
    if !support::require_toolchain("planted_live_case", &["nargo", "bb"]) {
        return;
    }
    assert!(true);
}
"""

_FIXTURE_PLANTED_M2 = """\
#[test]
fn planted_private_gate_case() {
    let out = match std::process::Command::new("nargo").arg("--version").output() {
        Ok(o) => o,
        Err(_) => {
            eprintln!("nargo not on PATH");
            return;
        }
    };
    assert!(out.status.success());
}
"""


def _write_fixture(root: Path, planted: dict[str, str]) -> None:
    wf = root / WORKFLOW_REL
    wf.parent.mkdir(parents=True, exist_ok=True)
    wf.write_text(_FIXTURE_WORKFLOW, encoding="utf-8")
    tests = root / "crates/sparq-zk-compose/tests"
    tests.mkdir(parents=True, exist_ok=True)
    (tests / "support").mkdir(exist_ok=True)
    (tests / "support/mod.rs").write_text("// gate\n", encoding="utf-8")
    (tests / "counted_live.rs").write_text(_FIXTURE_COUNTED, encoding="utf-8")
    (tests / "counted_feature_live.rs").write_text(
        _FIXTURE_COUNTED_FEATURE, encoding="utf-8"
    )
    (tests / "ignored_only.rs").write_text(_FIXTURE_IGNORED_ONLY, encoding="utf-8")
    (tests / "structural.rs").write_text(_FIXTURE_NOT_BOUND, encoding="utf-8")
    (tests / "toolchain_gate.rs").write_text(_FIXTURE_GATE_SELFTEST, encoding="utf-8")
    for name, body in planted.items():
        (tests / f"{name}.rs").write_text(body, encoding="utf-8")


def self_test() -> int:
    failures: list[str] = []

    # (1) Clean baseline: no planted binary, everything counted.
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        _write_fixture(root, {})
        _, invocations, findings = check(root)
        if findings:
            failures.append(
                "the clean fixture must come back clean, got: "
                + "; ".join(f"{f.binary.name}: {f.reason}" for f in findings)
            )
        counted = {b for inv in invocations for b in inv.binaries}
        if counted != {"counted_live", "counted_feature_live"}:
            failures.append(f"workflow parse found {sorted(counted)}")

    # (2) A planted, UNLISTED toolchain-bound binary per marker must be reported.
    for label, body in (
        ("planted_m1", _FIXTURE_PLANTED_M1),
        ("planted_m2", _FIXTURE_PLANTED_M2),
    ):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            _write_fixture(root, {label: body})
            _, _, findings = check(root)
            names = {f.binary.name for f in findings}
            if names != {label}:
                failures.append(
                    f"a planted unlisted toolchain-bound binary ({label}) must be the "
                    f"ONLY finding, got {sorted(names)}"
                )

    # (3) Listing the planted binary in the workflow clears the finding.
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        _write_fixture(root, {"planted_m1": _FIXTURE_PLANTED_M1})
        wf = root / WORKFLOW_REL
        wf.write_text(
            _FIXTURE_WORKFLOW.replace(
                "--test counted_live", "--test counted_live --test planted_m1"
            ),
            encoding="utf-8",
        )
        _, _, findings = check(root)
        if findings:
            failures.append(
                "listing the planted binary must clear the finding, got: "
                + "; ".join(f"{f.binary.name}: {f.reason}" for f in findings)
            )

    # (4) A feature-gated binary named WITHOUT its feature is nominal coverage only.
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        _write_fixture(root, {})
        wf = root / WORKFLOW_REL
        wf.write_text(
            _FIXTURE_WORKFLOW.replace("--features sparq-zk-compose/dual-leaf \\\n", ""),
            encoding="utf-8",
        )
        _, _, findings = check(root)
        if {f.binary.name for f in findings} != {"counted_feature_live"}:
            failures.append(
                "dropping the feature from the naming invocation must be reported, got "
                + str(sorted(f.binary.name for f in findings))
            )

    if failures:
        print("SELF-TEST FAILED (the detector is broken):", file=sys.stderr)
        for f in failures:
            print(f"  - {f}", file=sys.stderr)
        return 1
    print("SELF-TEST OK: the detector reports a planted unlisted toolchain-bound "
          "binary (both markers), accepts it once listed, keeps an ignored-only "
          "binary out, and rejects nominal feature coverage.")
    return 0


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--check", action="store_true", help="scan and gate (default)")
    ap.add_argument("--list", action="store_true", help="print the enumeration only")
    ap.add_argument("--self-test", action="store_true", help="run the built-in fixtures")
    ap.add_argument(
        "--root",
        default=None,
        help="repository root to scan (default: the repo this script lives in)",
    )
    args = ap.parse_args(argv)

    if args.self_test:
        return self_test()

    root = Path(args.root) if args.root else Path(__file__).resolve().parent.parent
    if args.list:
        binaries, invocations, _ = check(root)
        print(render(binaries, invocations))
        return 0
    return run_check(root)


if __name__ == "__main__":
    sys.exit(main())
