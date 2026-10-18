#!/usr/bin/env python3
"""Fail-closed checks on the ZK live-toolchain gate, with a RUNTIME ledger as the
proof that the lane reached the gate in every live test.

WHY
    `crates/sparq-zk*/tests/` hold two kinds of integration suite: STRUCTURAL ones
    that need no external tooling, and LIVE ones that shell out to a real `nargo` /
    `bb`. A live test that quietly returns when its tool is absent reports `ok` for a
    body that never ran. `tests/support/mod.rs` is the single gate that ends that:
    `gate(test, tools)` returns `Some(token)` and RECORDS `test` when every tool is
    on `PATH`; it prints a named skip and records nothing when the lane declares
    itself toolchain-free via `SPARQ_ZK_ALLOW_MISSING_TOOLCHAIN`; otherwise it
    panics. `live_tool(&token, tool)` is the only way to spawn one of those tools: it
    needs the token, which only `gate` mints, and it re-checks `PATH` and panics
    itself, so neither a stale token nor an `is_err()` test can become a soft pass.

    Two things have to hold, and neither is self-enforcing:

      1. No test may obtain `nargo` / `bb` by any route other than the gate module.
      2. The lane that HAS the toolchain must actually REACH the gate in every live
         test. A live test that no step runs, or that returns before the gate,
         executes its body nowhere.

    (1) is a line lint. (2) used to be a static read of the workflow's `cargo test`
    command grammar. That cannot work: `-- --list`, `--skip`, `false &&`, `exit 0`,
    `set +e`, a step `env:`, a `GITHUB_ENV` write, a `working-directory`, a
    `--manifest-path`, and job-level `if:` / `continue-on-error` all leave such a
    check green while nothing runs. So (2) is a RUNTIME LEDGER, which is fail-closed
    by construction, and the static command-grammar, feature and `#[ignore]`
    analysis is deleted rather than patched.

THE LEDGER
    `zk-toolchain.yml` sets `SPARQ_ZK_LIVE_LEDGER` at JOB level. Every time the gate
    lets a live body run it appends one line, `<package>/<binary>::<test>`, opened
    with `O_APPEND` and written in a single write. A skip appends nothing; a panic
    appends nothing. The lane's final step runs `--verify-ledger <literal path>`,
    which fails when any inventoried call site left no entry, when the ledger is
    missing or empty, or when the inventory does not parse. It reports unknown
    entries without failing on them.

    The ledger proves the gate was REACHED for each inventoried call site. It does
    NOT prove the body went on to assert anything, and it is not a coverage metric.

CHECK A - acquisition lint (fail-closed)
    Over every `*.rs` under the scan trees, OUTSIDE the helper modules, a line FAILS
    when its code (comments stripped) matches any of:

      A1  a `new(` on a path whose last segment is `Command`, whose argument is a
          string literal whose basename (directory stripped, `.exe` stripped) is
          `nargo` or `bb`, or is `sh` or `bash` - a shell launders a tool name into
          an argument.
      A2  the same `new(` with an argument that is not a string literal - the shape a
          const executable, an env-derived path or a resolver call takes.
      A3  a bare `NotFound` - the soft-pass arm.
      A4  a `which` probe (`which::`, `which(`, or a `use` of it).
      A5  `use ... Command as <alias>` - an alias defeats A1/A2's spelling.
      A6  `cmd!(` - a macro process runner.
      A7  any mention of `SPARQ_ZK_LIVE_LEDGER` - only the gate may write the ledger,
          and a test must not be able to redirect its path.

    A line is exempt ONLY when a `// zk-toolchain-gate: allow <reason>` comment with
    a non-empty reason sits on it or the line above, and the marker must be in a real
    `//` comment rather than inside a string literal. Over-matching is intended.

    Spellings NOT closed, and not claimed to be: a tool reached through another
    process crate (`duct`, `xshell`, a raw `exec`), a path assembled at runtime from
    pieces, and a wrapper re-exported under another name from outside the scan trees.
    The ledger is the backstop for all of them - such a test either reached the gate
    (and was recorded) or did not (and is reported).

CHECK B - the helper modules are one module
    The per-crate copies of `tests/support/mod.rs` must be byte-identical.
    `sparq-zk` cannot depend on `sparq-zk-compose`'s test tree (it is the LOWER
    crate), so the gate is duplicated rather than shared.

CHECK C - the driver route
    A verification call through `CircuitProver` shells out to `bb` inside the driver,
    so a test can soft-pass without ever naming a tool:

      C1  a file referencing `CircuitProver`, `driver::`, `live_tool(` or
          `DriverError::Spawn` must call the gate, unless the FILE carries an allow
          annotation with a reason.
      C2  `DriverError::Spawn` in a `match` arm or an `if let` pattern is an error
          unless annotated: branching on a spawn failure is a soft pass.
      C3  a `live_tool(` call must have `.expect(` or `.unwrap(` within the following
          lines, so its spawn result cannot be discarded. A token check, best effort.

CHECK D - the verify step is wired and cannot be skipped
    Exactly one step named VERIFY_STEP, in the job that carries `id: changes` and the
    two toolchain installs, as that job's LAST step; its `run` exactly the one verify
    command with the ledger path as a LITERAL (not read from the environment, which a
    test could rewrite); no `if:`, `continue-on-error`, `env:` or
    `working-directory` on it; the job's `if:` exactly ACCEPTED_JOB_IF and no
    `continue-on-error` on the job; job-level `SPARQ_ZK_LIVE_LEDGER` equal to that
    literal; at least one runnable step in the job; and
    `SPARQ_ZK_ALLOW_MISSING_TOOLCHAIN` mentioned on no non-comment line of the
    workflow except inside the run block of the step that asserts it is unset (a YAML
    comment cannot set anything). Any shape this reader does not recognise is an
    error.

CHECK E - the allow-annotation ratchet
    The exact set of files carrying allow annotations, and the count in each, is
    recorded in ALLOW_RATCHET. A new annotation, or one silently removed, is an
    error: an annotation is a reviewed decision, not a local convenience.

INVENTORY
    Every gate call site in the scan trees, keyed `<package>/<binary>::<test>`:

      * the first argument must be a STRING LITERAL, on the call's line or in a
        joined window of the following lines; anything else is an error;
      * the key must be unique; a duplicate is an error;
      * the binary is the cargo test-target name: the stem of `tests/<name>.rs`, the
        directory of `tests/<name>/main.rs`, or the `name` of a `[[test]]` entry with
        a `path`. A `[[test]]` path that does not exist is an error, and a gate call
        in a file whose target cannot be determined is an error.

    `LEDGER_EXCLUSIONS` lists keys that a lane ledger will not contain. Each entry
    exempts exactly ONE named test and carries its reason.

    A limitation: a gate call reached through a shared helper that forwarded a name
    VARIABLE would not be a literal at the call site. No such helper exists in the
    trees today - the only calls not on a single line are the gate's own self-test,
    read through the joined window - and the literal rule is what keeps it that way.

USAGE
    python3 scripts/check-zk-toolchain-gate.py --check
    python3 scripts/check-zk-toolchain-gate.py --self-test
    python3 scripts/check-zk-toolchain-gate.py --verify-ledger <path>
    --root PATH   check PATH instead of the repository root.

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

VERIFY_STEP = "Verify every live ZK test ran"
ASSERT_STEP = "Assert the live-toolchain opt-out is NOT set in this lane"
LEDGER_ENV = "SPARQ_ZK_LIVE_LEDGER"
ALLOW_MISSING_ENV = "SPARQ_ZK_ALLOW_MISSING_TOOLCHAIN"
LEDGER_PATH_EXPR = "${{ github.workspace }}/.zk-live-ledger-${{ github.run_id }}-${{ github.run_attempt }}.txt"
VERIFY_CMD = (
    "python3 scripts/check-zk-toolchain-gate.py --verify-ledger "
    '"${{ github.workspace }}/.zk-live-ledger-${{ github.run_id }}-${{ github.run_attempt }}.txt"'
)
INSTALL_STEPS = ("Install nargo (Noir) — pinned", "Install bb (Barretenberg) — pinned")
# The job's own `if:`, accepted verbatim; any other value is an error, because the
# verify step must not become skippable.
ACCEPTED_JOB_IF = (
    "github.event_name != 'pull_request' || github.event.pull_request.draft != true"
)

SCAN_DIRS: list[tuple[str, str]] = [
    ("sparq-zk", "crates/sparq-zk/tests"),
    ("sparq-zk", "crates/sparq-zk/tests/support"),
    ("sparq-zk-compose", "crates/sparq-zk-compose/tests"),
    ("sparq-zk-compose", "crates/sparq-zk-compose/tests/support"),
]
HELPER_MODULES: list[str] = [
    "crates/sparq-zk/tests/support/mod.rs",
    "crates/sparq-zk-compose/tests/support/mod.rs",
]
CARGO_TOMLS: dict[str, str] = {
    "sparq-zk": "crates/sparq-zk/Cargo.toml",
    "sparq-zk-compose": "crates/sparq-zk-compose/Cargo.toml",
}

LIVE_TOOLS = ("nargo", "bb")
SHELLS = ("sh", "bash")

# Keys a lane ledger will NOT contain. One entry per named test, each with a reason.
LEDGER_EXCLUSIONS: dict[str, str] = {
    "sparq-zk/toolchain_gate::gate_child_probe": (
        "the gate's own probe: in the parent sweep it returns before reaching the "
        "gate, and the re-exec'd child reaches it with a deliberately absent tool "
        "name, so the gate correctly records nothing on either path"
    ),
    "sparq-zk-compose/toolchain_gate::gate_child_probe": (
        "the gate's own probe in the byte-identical per-crate copy; same reason as "
        "the sparq-zk entry"
    ),
}

# Exact allow-annotation ratchet: file -> number of annotations.
ALLOW_RATCHET: dict[str, int] = {
    "crates/sparq-zk-compose/tests/coverage_direct.rs": 2,
    "crates/sparq-zk-compose/tests/holder_pop_forge.rs": 1,
    "crates/sparq-zk-compose/tests/host_helpers.rs": 2,
    "crates/sparq-zk-compose/tests/toolchain_gate.rs": 1,
    "crates/sparq-zk/tests/toolchain_gate.rs": 1,
}

GATE_CALL_RE = re.compile(r"\b(?:gate|require_toolchain)\s*\(")
GATE_SITE_RE = re.compile(r"\b(?:support\s*::\s*gate|require_toolchain)\s*\(")
NEW_RE = re.compile(
    r"\b([A-Za-z_][A-Za-z0-9_]*(?:\s*::\s*[A-Za-z_][A-Za-z0-9_]*)*)\s*::\s*new\s*\("
)
NOTFOUND_RE = re.compile(r"\bNotFound\b")
WHICH_RE = re.compile(r"\bwhich\s*(?:::|\()|^\s*use\s+[^;]*\bwhich\b")
COMMAND_ALIAS_RE = re.compile(r"^\s*use\s+[^;]*\bCommand\s+as\s+\w+")
CMD_MACRO_RE = re.compile(r"\bcmd!\s*\(")
DRIVER_REF_RE = re.compile(r"CircuitProver|driver::|live_tool\s*\(|DriverError::Spawn")
SPAWN_PATTERN_RE = re.compile(r"DriverError::Spawn")
EXEMPT_RE = re.compile(r"//\s*zk-toolchain-gate:\s*allow\s+(\S.*)")


@dataclass
class Finding:
    where: str
    rule: str
    detail: str

    def __str__(self) -> str:
        return f"{self.where}: [{self.rule}] {self.detail}"


# --------------------------------------------------------------------------- #
# Small helpers
# --------------------------------------------------------------------------- #


def comment_start(line: str) -> int:
    """Index of the first `//` that begins a real comment, or -1.

    A `//` inside a string literal is skipped, so a marker quoted in a fixture
    string is not an exemption.
    """
    in_str = False
    i = 0
    while i < len(line) - 1:
        c = line[i]
        if in_str and c == "\\":
            i += 2
            continue
        if c == '"':
            in_str = not in_str
        elif not in_str and c == "/" and line[i + 1] == "/":
            return i
        i += 1
    return -1


def code_of(line: str) -> str:
    at = comment_start(line)
    return line if at < 0 else line[:at]


def annotation(line: str) -> str | None:
    at = comment_start(line)
    if at < 0:
        return None
    m = EXEMPT_RE.search(line[at:])
    return m.group(1).strip() if m else None


def exempt(lines: list[str], i: int) -> bool:
    return any(annotation(lines[j]) for j in (i, i - 1) if 0 <= j < len(lines))


def arg_text(lines: list[str], i: int, col: int, window: int = 4) -> str:
    """The argument text at line `i`, column `col`, continued over `window` lines
    when the call's parenthesis ends the line."""
    head = lines[i][col:].strip()
    if head:
        return head
    parts: list[str] = []
    for j in range(i + 1, min(i + window, len(lines))):
        if lines[j].strip():
            parts.append(lines[j].strip())
            break
    return " ".join(parts)


def joined_arg(lines: list[str], i: int, col: int, window: int = 4) -> str:
    parts = [lines[i][col:].strip()]
    for j in range(i + 1, min(i + window, len(lines))):
        parts.append(lines[j].strip())
    return " ".join(p for p in parts if p)


def basename_of(literal: str) -> str:
    name = literal.replace("\\\\", "/").rsplit("/", 1)[-1]
    return name[:-4] if name.lower().endswith(".exe") else name


# --------------------------------------------------------------------------- #
# Scan trees and cargo test targets
# --------------------------------------------------------------------------- #


@dataclass
class Trees:
    files: list[tuple[str, str, str]] = field(default_factory=list)
    binary_of: dict[str, str] = field(default_factory=dict)
    findings: list[Finding] = field(default_factory=list)


def collect(root: Path) -> Trees:
    out = Trees()
    helper_abs = {(root / h).resolve() for h in HELPER_MODULES}
    explicit: dict[str, str] = {}

    for pkg, rel in CARGO_TOMLS.items():
        p = root / rel
        if not p.is_file():
            out.findings.append(Finding(rel, "SCAN", "Cargo.toml is missing"))
            continue
        with p.open("rb") as fh:
            manifest = tomllib.load(fh)
        for entry in manifest.get("test") or []:
            path, name = entry.get("path"), entry.get("name")
            if not path or not name:
                out.findings.append(
                    Finding(rel, "SCAN", f"[[test]] entry {entry!r} has no name/path")
                )
                continue
            target = (root / Path(rel).parent / path).resolve()
            if not target.is_file():
                out.findings.append(
                    Finding(f"{rel} [[test]] {name}", "SCAN",
                            f"path {path!r} does not exist")
                )
                continue
            try:
                explicit[str(target.relative_to(root))] = name
            except ValueError:
                out.findings.append(
                    Finding(f"{rel} [[test]] {name}", "SCAN",
                            f"path {path!r} points outside the repository")
                )

    seen: set[Path] = set()
    for pkg, rel in SCAN_DIRS:
        d = root / rel
        if not d.is_dir():
            out.findings.append(
                Finding(rel, "SCAN",
                        "scan directory is missing; the lint would be silently empty "
                        "over it")
            )
            continue
        tests_root = d.resolve()
        while tests_root.name != "tests" and tests_root != tests_root.parent:
            tests_root = tests_root.parent
        for path in sorted(d.rglob("*.rs")):
            r = path.resolve()
            if r in seen:
                continue
            seen.add(r)
            relp = str(path.relative_to(root))
            out.files.append((pkg, relp, path.read_text(encoding="utf-8", errors="replace")))
            if r in helper_abs:
                continue
            if relp in explicit:
                out.binary_of[relp] = explicit[relp]
            elif path.parent == tests_root:
                out.binary_of[relp] = path.stem
            elif path.name == "main.rs" and path.parent.parent == tests_root:
                out.binary_of[relp] = path.parent.name
            # Otherwise the file is not a target; a gate call in it is an INV error.
    return out


# --------------------------------------------------------------------------- #
# CHECK A / B / C / E
# --------------------------------------------------------------------------- #


def lint_file(rel: str, text: str) -> list[Finding]:
    findings: list[Finding] = []
    lines = text.splitlines()
    file_annotated = any(annotation(line) for line in lines)

    for i, line in enumerate(lines):
        if exempt(lines, i):
            continue
        code = code_of(line)
        at = f"{rel}:{i + 1}"
        for m in NEW_RE.finditer(code):
            if m.group(1).split("::")[-1].strip() != "Command":
                continue
            arg = arg_text(lines, i, m.end())
            lit = re.match(r'"((?:[^"\\]|\\.)*)"', arg)
            if lit:
                base = basename_of(lit.group(1))
                if base in LIVE_TOOLS:
                    findings.append(
                        Finding(at, "A1",
                                f'spawns the executable literal "{lit.group(1)}" '
                                "directly; use `support::live_tool`")
                    )
                elif base in SHELLS:
                    findings.append(
                        Finding(at, "A1",
                                f'spawns a shell ("{lit.group(1)}"), which can launder '
                                "a tool name into an argument")
                    )
            else:
                findings.append(
                    Finding(at, "A2",
                            f"`Command::new(` with a non-literal argument "
                            f"({arg[:48]!r})")
                )
        if NOTFOUND_RE.search(code):
            findings.append(
                Finding(at, "A3", "`NotFound` is the soft-pass arm; let the gate decide")
            )
        if WHICH_RE.search(code):
            findings.append(Finding(at, "A4", "a `which` probe is a private tool gate"))
        if COMMAND_ALIAS_RE.search(code):
            findings.append(
                Finding(at, "A5",
                        "`Command` aliased under another name defeats the spelling of "
                        "A1/A2")
            )
        if CMD_MACRO_RE.search(code):
            findings.append(Finding(at, "A6", "`cmd!(` is a process runner"))
        if LEDGER_ENV in code:
            findings.append(
                Finding(at, "A7",
                        f"only the gate module may touch {LEDGER_ENV}; a test must not "
                        "be able to redirect the ledger")
            )
        if SPAWN_PATTERN_RE.search(code) and ("=>" in code or "if let" in code):
            findings.append(
                Finding(at, "C2",
                        "branches on `DriverError::Spawn`, which is what a missing tool "
                        "produces; that is a soft pass")
            )
        if re.search(r"live_tool\s*\(", code):
            tail = "\n".join(lines[i : i + 12])
            if ".expect(" not in tail and ".unwrap(" not in tail:
                findings.append(
                    Finding(at, "C3",
                            "`live_tool(` with no nearby `.expect(`/`.unwrap(`: the "
                            "spawn result must not be discarded")
                )

    if DRIVER_REF_RE.search(text) and not GATE_CALL_RE.search(text) and not file_annotated:
        findings.append(
            Finding(rel, "C1",
                    "reaches the prover/driver route (CircuitProver, driver::, "
                    "live_tool or DriverError::Spawn) but never calls the gate, so a "
                    "missing tool becomes a verification error the test may read as a "
                    "rejection")
        )
    return findings


def check_helpers(root: Path) -> list[Finding]:
    findings: list[Finding] = []
    blobs: dict[str, bytes] = {}
    for rel in HELPER_MODULES:
        p = root / rel
        if not p.is_file():
            findings.append(Finding(rel, "B", "helper module is missing"))
            continue
        blobs[rel] = p.read_bytes()
    if len(blobs) == len(HELPER_MODULES) and len(set(blobs.values())) != 1:
        findings.append(
            Finding(" vs ".join(HELPER_MODULES), "B",
                    "the per-crate copies of the gate have drifted; they must be "
                    "byte-identical")
        )
    return findings


def check_ratchet(trees: Trees) -> list[Finding]:
    counts: dict[str, int] = {}
    for _pkg, rel, text in trees.files:
        n = sum(1 for line in text.splitlines() if annotation(line))
        if n:
            counts[rel] = n
    findings: list[Finding] = []
    for rel in sorted(set(counts) | set(ALLOW_RATCHET)):
        want, got = ALLOW_RATCHET.get(rel, 0), counts.get(rel, 0)
        if want != got:
            findings.append(
                Finding(rel, "E",
                        f"carries {got} allow annotation(s); the recorded ratchet says "
                        f"{want}. An annotation is a reviewed decision — update "
                        "ALLOW_RATCHET in this script in the same change")
            )
    return findings


# --------------------------------------------------------------------------- #
# Inventory
# --------------------------------------------------------------------------- #


@dataclass
class Site:
    key: str
    where: str


def inventory(trees: Trees) -> tuple[list[Site], list[Finding]]:
    sites: list[Site] = []
    findings: list[Finding] = []
    seen: dict[str, str] = {}
    helper_rels = set(HELPER_MODULES)
    for pkg, rel, text in trees.files:
        if rel in helper_rels:
            continue
        lines = text.splitlines()
        for i, line in enumerate(lines):
            for m in GATE_SITE_RE.finditer(code_of(line)):
                at = f"{rel}:{i + 1}"
                arg = joined_arg(lines, i, m.end())
                lit = re.match(r'"((?:[^"\\]|\\.)*)"', arg)
                if not lit:
                    findings.append(
                        Finding(at, "INV",
                                "the gate's first argument is not a string literal "
                                f"({arg[:48]!r}); the expected ledger entries are built "
                                "from those literals")
                    )
                    continue
                binary = trees.binary_of.get(rel)
                if binary is None:
                    findings.append(
                        Finding(at, "INV",
                                "the cargo test target for this file cannot be "
                                "determined (not tests/<name>.rs, tests/<name>/main.rs "
                                "or a [[test]] path)")
                    )
                    continue
                key = f"{pkg}/{binary}::{lit.group(1)}"
                if key in seen:
                    findings.append(
                        Finding(at, "INV",
                                f"duplicate ledger key {key!r}, already at {seen[key]}")
                    )
                    continue
                seen[key] = at
                sites.append(Site(key=key, where=at))
    return sites, findings


# --------------------------------------------------------------------------- #
# CHECK D
# --------------------------------------------------------------------------- #


def check_workflow(root: Path) -> list[Finding]:
    import yaml  # noqa: PLC0415

    path = root / WORKFLOW_REL
    if not path.is_file():
        return [Finding(WORKFLOW_REL, "D", "workflow is missing")]
    raw = path.read_text(encoding="utf-8")
    doc = yaml.safe_load(raw) or {}
    jobs = doc.get("jobs") or {}
    if not jobs:
        return [Finding(WORKFLOW_REL, "D", "workflow declares no jobs")]

    hosts = [
        (jid, job)
        for jid, job in jobs.items()
        if any((s or {}).get("name") == VERIFY_STEP for s in (job.get("steps") or []))
    ]
    total = sum(
        1
        for job in jobs.values()
        for s in (job.get("steps") or [])
        if (s or {}).get("name") == VERIFY_STEP
    )
    if total != 1 or len(hosts) != 1:
        return [
            Finding(WORKFLOW_REL, "D",
                    f"expected exactly one step named {VERIFY_STEP!r}, found {total}")
        ]

    findings: list[Finding] = []
    jid, job = hosts[0]
    steps = [s or {} for s in (job.get("steps") or [])]
    if not [s for s in steps if s.get("run")]:
        findings.append(Finding(jid, "D", "the job has no runnable step"))

    names = [s.get("name") for s in steps]
    if "changes" not in [s.get("id") for s in steps]:
        findings.append(
            Finding(jid, "D",
                    "the verify step is not in the job that carries `id: changes`")
        )
    for needed in INSTALL_STEPS:
        if needed not in names:
            findings.append(
                Finding(jid, "D",
                        f"the verify step is not in the job that runs {needed!r}")
            )
    if names and names[-1] != VERIFY_STEP:
        findings.append(
            Finding(VERIFY_STEP, "D",
                    "must be the job's LAST step, after every live step; last is "
                    f"{names[-1]!r}")
        )

    verify = next(s for s in steps if s.get("name") == VERIFY_STEP)
    if str(verify.get("run", "")).strip() != VERIFY_CMD:
        findings.append(
            Finding(VERIFY_STEP, "D",
                    "the run block must be exactly the one verify command with the "
                    "ledger path as a literal; got "
                    f"{str(verify.get('run', '')).strip()!r}")
        )
    for key in ("if", "continue-on-error", "env", "working-directory"):
        if key in verify:
            findings.append(
                Finding(VERIFY_STEP, "D", f"must not carry `{key}:` — it must always run")
            )

    if str(job.get("if", ACCEPTED_JOB_IF)).strip() != ACCEPTED_JOB_IF:
        findings.append(
            Finding(jid, "D",
                    f"the job `if:` is not the accepted value; got {job.get('if')!r}")
        )
    if job.get("continue-on-error"):
        findings.append(Finding(jid, "D", "the job must not carry continue-on-error"))

    env = job.get("env") or {}
    if str(env.get(LEDGER_ENV, "")).strip() != LEDGER_PATH_EXPR:
        findings.append(
            Finding(jid, "D",
                    f"job-level env must set {LEDGER_ENV} to {LEDGER_PATH_EXPR!r}; got "
                    f"{env.get(LEDGER_ENV)!r}")
        )

    allowed = "".join(
        str(s.get("run", "")) for s in steps if s.get("name") == ASSERT_STEP
    )
    # A YAML comment cannot set anything, so only non-comment lines count; everything
    # else must belong to the assert step's own run block.
    stray = [
        str(n)
        for n, line in enumerate(raw.splitlines(), 1)
        if ALLOW_MISSING_ENV in line
        and not line.lstrip().startswith("#")
        and line.strip() not in allowed
    ]
    if stray:
        findings.append(
            Finding(WORKFLOW_REL, "D",
                    f"{ALLOW_MISSING_ENV} is mentioned outside {ASSERT_STEP!r} (line(s) "
                    f"{', '.join(stray)}); setting it here would turn every live test "
                    "into a skip")
        )
    return findings


# --------------------------------------------------------------------------- #
# Drivers
# --------------------------------------------------------------------------- #


def static_findings(root: Path) -> tuple[list[Finding], list[Site]]:
    trees = collect(root)
    findings = list(trees.findings)
    helper_rels = set(HELPER_MODULES)
    for _pkg, rel, text in trees.files:
        if rel not in helper_rels:
            findings.extend(lint_file(rel, text))
    findings.extend(check_helpers(root))
    findings.extend(check_ratchet(trees))
    sites, inv = inventory(trees)
    findings.extend(inv)
    findings.extend(check_workflow(root))
    for key in sorted(set(LEDGER_EXCLUSIONS) - {s.key for s in sites}):
        findings.append(
            Finding("LEDGER_EXCLUSIONS", "INV",
                    f"{key!r} is excluded from the ledger but is no longer a gate call "
                    "site; drop the entry")
        )
    return findings, sites


def report_check(root: Path) -> int:
    findings, sites = static_findings(root)
    print(f"note: {len(sites)} gate call site(s) inventoried; "
          f"{len(LEDGER_EXCLUSIONS)} excluded from the ledger")
    for key, reason in sorted(LEDGER_EXCLUSIONS.items()):
        print(f"note: excluded {key} — {reason}")
    if not findings:
        print("\nOK: the gate is the only acquisition path this lint can see, the "
              "inventory parses, and the verify step is wired so it cannot be skipped.")
        return 0
    print("\nFAIL:", file=sys.stderr)
    for f in findings:
        print(f"  {f}", file=sys.stderr)
    return 1


def verify_ledger(root: Path, ledger: Path) -> int:
    findings, sites = static_findings(root)
    blocking = [f for f in findings if f.rule in ("INV", "SCAN")]
    if blocking:
        print("FAIL: the inventory does not parse, so the ledger cannot be verified:",
              file=sys.stderr)
        for f in blocking:
            print(f"  {f}", file=sys.stderr)
        return 1
    if not ledger.is_file():
        print(f"FAIL: the ledger {ledger} does not exist. The gate appends one line per "
              "live body it lets run, so no file means no live test reached the gate.",
              file=sys.stderr)
        return 1
    entries = [
        line.strip()
        for line in ledger.read_text(encoding="utf-8", errors="replace").splitlines()
        if line.strip()
    ]
    if not entries:
        print(f"FAIL: the ledger {ledger} is empty — no live test reached the gate.",
              file=sys.stderr)
        return 1
    recorded = set(entries)
    keys = {s.key for s in sites}
    expected = keys - set(LEDGER_EXCLUSIONS)
    missing = sorted(expected - recorded)
    unknown = sorted(recorded - keys)
    print(f"note: {len(recorded)} distinct ledger entries, {len(expected)} expected")
    for key in unknown:
        print(f"note: ledger entry with no matching call site: {key}")
    if missing:
        where = {s.key: s.where for s in sites}
        print("\nFAIL: these gate call sites left no ledger entry, so their live bodies "
              "did not run in this lane:", file=sys.stderr)
        for key in missing:
            print(f"  {key}  ({where.get(key, '?')})", file=sys.stderr)
        print("\nEither no step runs them, or they returned before reaching the gate. "
              "If one is legitimately not runnable in this lane, add it to "
              "LEDGER_EXCLUSIONS in this script with a reason — one entry per named "
              "test.", file=sys.stderr)
        return 1
    print("\nOK: every inventoried gate call site left a ledger entry. That proves the "
          "gate was REACHED for each one; it says nothing about what the bodies "
          "asserted.")
    return 0


# --------------------------------------------------------------------------- #
# Self-test
# --------------------------------------------------------------------------- #

_WF = """\
name: zk-toolchain
on:
  schedule:
    - cron: "23 4 * * *"
  workflow_dispatch:
jobs:
  forge-suite:
    if: __JOBIF__
    runs-on: ubuntu-latest
    env:
      __LEDGER__: __LEDGERPATH__
    steps:
      - name: Detect zk changes
        id: changes
        run: |
          zk=true
          echo "zk=${zk}" >> "$GITHUB_OUTPUT"
      - name: __INSTALL0__
        run: noirup --version 1.0.0
      - name: __INSTALL1__
        run: bbup --version 1.0.0
      - name: __ASSERT__
        run: |
          if [ -n "${__OPTOUT__:-}" ]; then exit 1; fi
      - name: Run the default-run live ZK tests (real nargo/bb)
        run: |
          cargo test -p sparq-zk-compose -p sparq-zk -- --test-threads=1
      - name: __VERIFY__
        run: __VERIFYCMD__
"""
_WF = (
    _WF.replace("__JOBIF__", ACCEPTED_JOB_IF)
    .replace("__LEDGERPATH__", LEDGER_PATH_EXPR)
    .replace("__LEDGER__", LEDGER_ENV)
    .replace("__OPTOUT__", ALLOW_MISSING_ENV)
    .replace("__ASSERT__", ASSERT_STEP)
    .replace("__INSTALL0__", INSTALL_STEPS[0])
    .replace("__INSTALL1__", INSTALL_STEPS[1])
    .replace("__VERIFYCMD__", VERIFY_CMD)
    .replace("__VERIFY__", VERIFY_STEP)
)

_SUPPORT = '''\
pub const LEDGER_ENV: &str = "SPARQ_ZK_LIVE_LEDGER";

pub struct LiveToolchain {
    _seal: (),
}

pub fn live_tool(_gate: &LiveToolchain, tool: &str) -> std::process::Command {
    assert!(!tool.is_empty());
    std::process::Command::new(tool)
}

pub fn gate(_test: &str, _tools: &[&str]) -> Option<LiveToolchain> {
    Some(LiveToolchain { _seal: () })
}

pub fn require_toolchain(test: &str, tools: &[&str]) -> bool {
    gate(test, tools).is_some()
}
'''

_CARGO = '[package]\nname = "%s"\nversion = "0.0.0"\n'

_GOOD_LIVE = '''\
mod support;

#[test]
fn live_case() {
    let gate = match support::gate("live_case", &["nargo", "bb"]) {
        Some(g) => g,
        None => return,
    };
    let out = support::live_tool(&gate, "nargo").output().expect("nargo runs");
    assert!(out.status.success());
}
'''

_GOOD_PLAIN = '''\
mod support;

#[test]
fn plain_case() {
    if !support::require_toolchain("plain_case", &["nargo"]) {
        return;
    }
    assert_eq!(2 + 2, 4);
}
'''

EVASIONS: dict[str, tuple[str, str]] = {
    "a helper wrapper outside the gate module": (
        "evade_wrapper.rs",
        'fn g() -> bool { std::process::Command::new("nargo").output().is_ok() }\n'
        "\n#[test]\nfn case() { assert!(g()); }\n",
    ),
    "a const executable": (
        "evade_const.rs",
        'const T: &str = "nargo";\n#[test]\nfn case() {\n'
        "    let _ = std::process::Command::new(T).output();\n}\n",
    ),
    "a which probe": (
        "evade_which.rs",
        '#[test]\nfn case() { assert!(which::which("nargo").is_ok()); }\n',
    ),
    "an env-gated driver": (
        "evade_env.rs",
        "#[test]\nfn case() {\n"
        '    let b = std::env::var("NARGO_BIN").unwrap_or_default();\n'
        "    let _ = std::process::Command::new(b).output();\n}\n",
    ),
    "a one-line test fn": (
        "evade_oneline.rs",
        '#[test] fn case() { let _ = std::process::Command::new("bb").output(); }\n',
    ),
    "a not-found soft pass": (
        "evade_notfound.rs",
        "#[test]\nfn case() {\n"
        "    if matches!(std::io::ErrorKind::NotFound, std::io::ErrorKind::Other) "
        "{ return; }\n}\n",
    ),
    "a path-literal executable": (
        "evade_path.rs",
        "#[test]\nfn case() {\n"
        '    let _ = std::process::Command::new("/root/.nargo/bin/nargo").output();\n}\n',
    ),
    "a windows-suffixed executable": (
        "evade_exe.rs",
        '#[test]\nfn case() {\n'
        '    let _ = std::process::Command::new("bb.exe").output();\n}\n',
    ),
    "a whitespace-spelled new": (
        "evade_space.rs",
        "#[test]\nfn case() {\n"
        '    let _ = std::process::Command :: new ("nargo").output();\n}\n',
    ),
    "a shell laundering the tool name": (
        "evade_shell.rs",
        "#[test]\nfn case() {\n"
        '    let _ = std::process::Command::new("sh").arg("-c").arg("nargo x").output();\n'
        "}\n",
    ),
    "a Command alias": (
        "evade_alias.rs",
        "use std::process::Command as Runner;\n#[test]\nfn case() {\n"
        '    let _ = Runner::new("nargo").output();\n}\n',
    ),
    "a cmd! macro": (
        "evade_cmd_macro.rs",
        '#[test]\nfn case() { let _ = cmd!("nargo", "execute"); }\n',
    ),
    "a test redirecting the ledger": (
        "evade_ledger.rs",
        "#[test]\nfn case() {\n"
        '    std::env::set_var("SPARQ_ZK_LIVE_LEDGER", "/dev/null");\n}\n',
    ),
    "an ungated driver route": (
        "evade_driver.rs",
        "use sparq_zk_compose::driver::CircuitProver;\n#[test]\nfn case() {\n"
        "    let p = CircuitProver::from_crate_root();\n    let _ = p;\n}\n",
    ),
    "a branch on a spawn failure": (
        "evade_spawn_arm.rs",
        "mod support;\n#[test]\nfn case() {\n"
        '    if !support::require_toolchain("spawn_arm_case", &["nargo"]) { return; }\n'
        "    match run() {\n        Err(DriverError::Spawn { .. }) => return,\n"
        "        _ => {}\n    }\n}\n",
    ),
    "a discarded live_tool spawn result": (
        "evade_discard.rs",
        "mod support;\n#[test]\nfn case() {\n"
        '    let g = match support::gate("discard_case", &["nargo"]) {\n'
        "        Some(g) => g,\n        None => return,\n    };\n"
        '    let _ = support::live_tool(&g, "nargo").output().ok();\n}\n',
    ),
    "a non-literal gate name": (
        "evade_dyn_name.rs",
        'mod support;\nconst N: &str = "dyn_case";\n#[test]\nfn case() {\n'
        '    if !support::require_toolchain(N, &["nargo"]) { return; }\n}\n',
    ),
    "a duplicate inventory name": (
        "evade_dup.rs",
        "mod support;\n#[test]\nfn a() {\n"
        '    if !support::require_toolchain("dup_name", &["nargo"]) { return; }\n}\n'
        "\n#[test]\nfn b() {\n"
        '    if !support::require_toolchain("dup_name", &["nargo"]) { return; }\n}\n',
    ),
    "a quoted exemption marker": (
        "evade_quoted.rs",
        "#[test]\nfn case() {\n"
        '    let s = "// zk-toolchain-gate: allow not a real comment";\n'
        '    let _ = std::process::Command::new("nargo").output();\n'
        "    let _ = s;\n}\n",
    ),
}


def _write_fixture(root: Path) -> None:
    (root / WORKFLOW_REL).parent.mkdir(parents=True, exist_ok=True)
    (root / WORKFLOW_REL).write_text(_WF, encoding="utf-8")
    for pkg, rel in CARGO_TOMLS.items():
        p = root / rel
        p.parent.mkdir(parents=True, exist_ok=True)
        p.write_text(_CARGO % pkg, encoding="utf-8")
    for _pkg, rel in SCAN_DIRS:
        (root / rel).mkdir(parents=True, exist_ok=True)
    for helper in HELPER_MODULES:
        (root / helper).write_text(_SUPPORT, encoding="utf-8")
    (root / "crates/sparq-zk-compose/tests/live.rs").write_text(_GOOD_LIVE, encoding="utf-8")
    (root / "crates/sparq-zk/tests/plain.rs").write_text(_GOOD_PLAIN, encoding="utf-8")


def self_test() -> int:
    global LEDGER_EXCLUSIONS, ALLOW_RATCHET
    real_excl, real_ratchet = LEDGER_EXCLUSIONS, ALLOW_RATCHET
    LEDGER_EXCLUSIONS, ALLOW_RATCHET = {}, {}
    stack: list[tempfile.TemporaryDirectory] = []
    failures: list[str] = []
    reds = 0

    def fresh() -> Path:
        td = tempfile.TemporaryDirectory()
        stack.append(td)
        root = Path(td.name)
        _write_fixture(root)
        return root

    def expect_clean(root: Path, label: str) -> None:
        findings, _ = static_findings(root)
        if findings:
            failures.append(f"{label} must be clean, got: " + "; ".join(map(str, findings)))

    def expect_red(root: Path, label: str, rule: str | None = None) -> None:
        nonlocal reds
        reds += 1
        findings, _ = static_findings(root)
        if not findings:
            failures.append(f"{label} must be reported, but the check was clean")
        elif rule and not any(f.rule == rule for f in findings):
            failures.append(
                f"{label} must be reported as {rule}, got "
                + "; ".join(sorted({f.rule for f in findings}))
            )

    def expect_ledger(root: Path, lines: list[str] | None, label: str, want: int) -> None:
        nonlocal reds
        if want:
            reds += 1
        if lines is None:
            path = root / "no-such-ledger.txt"
        else:
            path = root / "ledger.txt"
            path.write_text("".join(f"{line}\n" for line in lines), encoding="utf-8")
        got = verify_ledger(root, path)
        if got != want:
            failures.append(f"{label}: --verify-ledger returned {got}, expected {want}")

    expect_clean(fresh(), "the clean fixture")

    for label, (name, body) in EVASIONS.items():
        root = fresh()
        (root / "crates/sparq-zk-compose/tests" / name).write_text(body, encoding="utf-8")
        expect_red(root, f"evasion: {label}")

    # An annotation with a reason clears the finding, and is itself ratcheted.
    root = fresh()
    ALLOW_RATCHET = {"crates/sparq-zk-compose/tests/ok_exempt.rs": 1}
    (root / "crates/sparq-zk-compose/tests/ok_exempt.rs").write_text(
        'const T: &str = "nargo";\n#[test]\nfn case() {\n'
        "    // zk-toolchain-gate: allow a stated reason\n"
        "    let _ = std::process::Command::new(T).output();\n}\n",
        encoding="utf-8",
    )
    expect_clean(root, "a declared exemption inside the ratchet")
    ALLOW_RATCHET = {}
    root = fresh()
    (root / "crates/sparq-zk-compose/tests/bare_exempt.rs").write_text(
        'const T: &str = "nargo";\n#[test]\nfn case() {\n'
        "    // zk-toolchain-gate: allow\n"
        "    let _ = std::process::Command::new(T).output();\n}\n",
        encoding="utf-8",
    )
    expect_red(root, "an annotation with no reason", "A2")
    root = fresh()
    (root / "crates/sparq-zk-compose/tests/unratcheted.rs").write_text(
        "#[test]\nfn case() {\n"
        "    // zk-toolchain-gate: allow a stated reason\n"
        '    let _ = std::process::Command::new("nargo").output();\n}\n',
        encoding="utf-8",
    )
    expect_red(root, "an annotation outside the ratchet", "E")

    root = fresh()
    (root / HELPER_MODULES[0]).write_text(_SUPPORT + "// drift\n", encoding="utf-8")
    expect_red(root, "helper-module drift", "B")
    root = fresh()
    shutil.rmtree(root / "crates/sparq-zk/tests/support")
    expect_red(root, "a missing scan directory", "SCAN")

    root = fresh()
    p = root / CARGO_TOMLS["sparq-zk-compose"]
    p.write_text(
        p.read_text(encoding="utf-8")
        + '\n[[test]]\nname = "ghost"\npath = "tests/ghost/main.rs"\n',
        encoding="utf-8",
    )
    expect_red(root, "a [[test]] path that does not exist", "SCAN")

    root = fresh()
    sub = root / "crates/sparq-zk-compose/tests/helpers_dir"
    sub.mkdir()
    (sub / "shared.rs").write_text(
        'fn f() { let _ = require_toolchain("orphan", &["nargo"]); }\n', encoding="utf-8"
    )
    expect_red(root, "a gate call in a file that is not a target", "INV")

    root = fresh()
    sub = root / "crates/sparq-zk-compose/tests/subdir_target"
    sub.mkdir()
    (sub / "main.rs").write_text(
        'mod nothing;\nfn main() {}\n#[test]\nfn sub_case() {\n'
        '    let _ = require_toolchain("sub_case", &["nargo"]);\n}\n',
        encoding="utf-8",
    )
    (sub / "nothing.rs").write_text("// module of the subdir target\n", encoding="utf-8")
    expect_ledger(
        root,
        [
            "sparq-zk-compose/live::live_case",
            "sparq-zk/plain::plain_case",
            "sparq-zk-compose/subdir_target::sub_case",
        ],
        "a tests/<name>/main.rs target is inventoried",
        0,
    )

    for label, repl in (
        ("the step gone", (f"      - name: {VERIFY_STEP}", "      - name: Other")),
        ("an `if:` on it",
         (f"      - name: {VERIFY_STEP}\n        run:",
          f"      - name: {VERIFY_STEP}\n        if: always()\n        run:")),
        ("continue-on-error on it",
         (f"      - name: {VERIFY_STEP}\n        run:",
          f"      - name: {VERIFY_STEP}\n        continue-on-error: true\n        run:")),
        ("an env on it",
         (f"      - name: {VERIFY_STEP}\n        run:",
          f"      - name: {VERIFY_STEP}\n        env:\n          X: '1'\n        run:")),
        ("a working-directory on it",
         (f"      - name: {VERIFY_STEP}\n        run:",
          f"      - name: {VERIFY_STEP}\n        working-directory: crates\n        run:")),
        ("an extra command in its run",
         (f"        run: {VERIFY_CMD}",
          f"        run: |\n          set +e\n          {VERIFY_CMD}")),
        ("the path taken from the environment",
         (f'--verify-ledger "{LEDGER_PATH_EXPR}"', '--verify-ledger "$SPARQ_ZK_LIVE_LEDGER"')),
        ("the job-level ledger env gone",
         (f"    env:\n      {LEDGER_ENV}: {LEDGER_PATH_EXPR}\n", "")),
        ("a job-level if that can skip it",
         (f"    if: {ACCEPTED_JOB_IF}", "    if: github.event_name == 'schedule'")),
        ("job continue-on-error",
         ("    runs-on: ubuntu-latest", "    continue-on-error: true\n    runs-on: ubuntu-latest")),
        ("the opt-out set in the lane",
         ("    env:\n", f"    env:\n      {ALLOW_MISSING_ENV}: '1'\n")),
        ("a live step after it",
         (f"      - name: {VERIFY_STEP}\n        run: {VERIFY_CMD}\n",
          f"      - name: {VERIFY_STEP}\n        run: {VERIFY_CMD}\n"
          "      - name: One more live step\n        run: cargo test -p sparq-zk\n")),
        ("the toolchain installs in another job",
         (f"      - name: {INSTALL_STEPS[0]}\n        run: noirup --version 1.0.0\n", "")),
    ):
        root = fresh()
        wf = root / WORKFLOW_REL
        text = wf.read_text(encoding="utf-8")
        old, new = repl
        if old not in text:
            failures.append(f"fixture {label!r} could not be planted")
            continue
        wf.write_text(text.replace(old, new, 1), encoding="utf-8")
        expect_red(root, f"verify step: {label}", "D")

    root = fresh()
    expect_ledger(
        root,
        ["sparq-zk-compose/live::live_case", "sparq-zk/plain::plain_case"],
        "a complete ledger",
        0,
    )
    expect_ledger(root, ["sparq-zk-compose/live::live_case"], "a missing name", 1)
    expect_ledger(root, [], "an empty ledger", 1)
    expect_ledger(root, None, "a missing ledger file", 1)
    expect_ledger(
        root,
        [
            "sparq-zk-compose/live::live_case",
            "sparq-zk/plain::plain_case",
            "sparq-zk/plain::who_is_this",
        ],
        "an unknown entry (reported, not fatal)",
        0,
    )
    root = fresh()
    (root / "crates/sparq-zk-compose/tests/evade_dup.rs").write_text(
        EVASIONS["a duplicate inventory name"][1], encoding="utf-8"
    )
    expect_ledger(root, ["sparq-zk-compose/live::live_case"], "a duplicate inventory name", 1)

    for td in stack:
        td.cleanup()
    LEDGER_EXCLUSIONS, ALLOW_RATCHET = real_excl, real_ratchet
    if failures:
        print("SELF-TEST FAILED (the checks are broken):", file=sys.stderr)
        for f in failures:
            print(f"  - {f}", file=sys.stderr)
        return 1
    print(f"SELF-TEST OK: the clean fixture passes and all {reds} planted fixtures are "
          "reported (acquisition and spelling evasions, the driver route, the "
          "inventory, the annotation ratchet, helper drift, the scan trees, the "
          "verify-step shape, and ledger verification).")
    return 0


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description="Fail-closed checks on the ZK toolchain gate.")
    ap.add_argument("--check", action="store_true", help="check the tree (default)")
    ap.add_argument("--self-test", action="store_true", help="run the negative fixtures")
    ap.add_argument("--verify-ledger", metavar="PATH", help="verify a runtime ledger")
    ap.add_argument("--root", default=None, help="root to check (default: this repo)")
    args = ap.parse_args(argv)
    if args.self_test:
        return self_test()
    root = Path(args.root) if args.root else Path(__file__).resolve().parent.parent
    if args.verify_ledger:
        return verify_ledger(root, Path(args.verify_ledger))
    return report_check(root)


if __name__ == "__main__":
    sys.exit(main())
