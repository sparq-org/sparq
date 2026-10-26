#!/usr/bin/env python3
"""Fail-closed checks on the ZK live-toolchain gate, with a RUNTIME ledger as the
proof that every live test in the lane passed its gate AND reached its completion
call after its assertions.

WHY
    `crates/sparq-zk*/tests/`, and the crates' own `#[cfg(test)]` code under `src/`,
    hold two kinds of test: STRUCTURAL ones that need no external tooling, and LIVE
    ones that shell out to a real `nargo` / `bb`. A live test that quietly returns
    when its tool is absent reports `ok` for a body that never ran.
    `tests/support/mod.rs` is the single gate that ends that: `gate(test, tools)`
    returns `Some(token)` and RECORDS THE CALL SITE when every tool is on `PATH`;
    it prints a named skip and records nothing when the lane declares itself
    toolchain-free via `SPARQ_ZK_ALLOW_MISSING_TOOLCHAIN`; otherwise it panics.
    `live_tool(&token, tool)` is the only way to spawn one of those tools: it needs
    the token, which only `gate` mints, and it re-checks `PATH` and panics
    itself, so neither a stale token nor an `is_err()` test can become a soft pass.

    Two things have to hold, and neither is self-enforcing:

      1. No test may obtain `nargo` / `bb` by any route other than the gate module.
      2. The lane that HAS the toolchain must actually RUN every live test to the end
         of its assertions. A live test that no step runs, that returns before the
         gate, or that returns after the gate without asserting, checks nothing.

    (1) is a line lint. (2) used to be a static read of the workflow's `cargo test`
    command grammar. That cannot work: `-- --list`, `--skip`, `false &&`, `exit 0`,
    `set +e`, a step `env:`, a `GITHUB_ENV` write, a `working-directory`, a
    `--manifest-path`, and job-level `if:` / `continue-on-error` all leave such a
    check green while nothing runs. So (2) is a RUNTIME LEDGER, which is fail-closed
    by construction, and the static command-grammar, feature and `#[ignore]`
    analysis is deleted rather than patched.

THE LEDGER
    `zk-toolchain.yml` sets `SPARQ_ZK_LIVE_LEDGER` at JOB level (the name is a
    PRIVATE const inside the gate module, so no test can read or redirect it). Each
    gate call site writes TWO lines, each opened with `O_APPEND` and written in a
    single write:

      enter <file>:<line>     by the gate, when it lets the body run;
      complete <file>:<line>  by `support::complete(token)`, which each live test
                              calls as its LAST statement, after its assertions.

    The key is the GATE's call site, taken from `#[track_caller]` +
    `std::panic::Location::caller()` and carried by the token to the completion
    call, so the two lines of one test match by construction. The `test` argument is
    a caller-supplied string used only in skip and panic messages; keying on it would
    let one call site satisfy another. The inventory below recomputes the keys from
    source. A skip appends nothing; a panic appends nothing; a test that fails an
    assertion or returns early after the gate leaves `enter` without `complete`.
    Completion is deliberately NOT done in `Drop`: a token dropped uncompleted
    PANICS instead (unless the thread is already panicking), so an early return fails
    where it runs as well as in the ledger.

    The lane's final step runs `--verify-ledger` with a LITERAL path, which fails when
    any inventoried call site lacks its `enter` or its `complete` line, when a key has
    a `complete` and no `enter`, when a line is in neither form, when the ledger is
    missing or empty, when the inventory does not parse, or when a key matches NO
    inventoried call site (an unpredicted key means the inventory missed a site).

    The ledger proves each live body was let through the gate and reached the call
    placed after its assertions. It does not grade what those assertions check, and
    it is not a coverage metric.

CHECK A - acquisition lint (fail-closed)
    Over every `*.rs` under the scan trees, OUTSIDE the helper modules, code fails
    when it matches any of the rules below. A1, A2, A4, A5, A8 and C2/C3 read the
    whole file with comments and literal contents blanked, so a construct that wraps
    over several lines is ONE match (a `Command` whose `::new(` is on the next line,
    a multi-line `use`); the finding is reported at the line where it starts.

      A1  a `new(` on a path whose last segment is `Command`, whose argument is a
          string literal whose basename (directory stripped, `.exe` stripped) is
          `nargo` or `bb`, a shell (`sh`, `bash`, `dash`, `zsh`), or a LAUNCHER that
          runs the program named in its arguments (`env`, `xargs`, `nice`, `timeout`,
          `nohup`, `stdbuf`, `time`, `setsid`, `sudo`, `busybox`, ...); or a spawned
          command whose STATEMENT carries a `nargo` / `bb` string literal anywhere in
          its arguments (`Command::new("env").args(["nargo", "execute"])`).
      A2  the same `new(` with an argument that is not a string literal - the shape a
          const executable, an env-derived path or a resolver call takes.
      A3  a bare `NotFound` - the soft-pass arm.
      A4  a `which` probe (`which::`, `which(`, or a `use` statement naming it).
      A5  a `use` statement aliasing `Command` - an alias defeats A1/A2's spelling.
      A6  `cmd!(` - a macro process runner.
      A7  any mention of `SPARQ_ZK_LIVE_LEDGER` - only the gate may write the ledger,
          and a test must not be able to redirect its path. This is a check on the
          SPELLING of that name; the structural protection is that the gate module's
          const is private, so there is nothing to name.
      A8  the gate reached INDIRECTLY. ANY `use` STATEMENT that mentions `support`
          is an error - a brace list (`use ...support::{gate, live_tool};`), a glob,
          an item alias, or an alias on the MODULE (`use ...support as tc;`), also
          when it wraps over several lines or sits inside a function - as is
          `use ... gate as <alias>` and a gate function referenced without being
          called.
          Every ZK test declares `mod support;` and calls `support::gate(...)`
          qualified, so an import is never needed, and each of those shapes makes
          the call read as `gate(` or `tc::gate(` instead. The inventory also
          over-matches any path prefix (below), so the two rules cover the class
          from both ends.

    A line is exempt ONLY when a `// zk-toolchain-gate: allow <reason>` comment with
    a non-empty reason sits ON it, or in the comment block immediately above it (so a
    long reason may wrap over continuation comment lines). The scope is that ONE
    line: a blank line or any code between the comment and the line ends the block,
    and there is no file-level annotation. The marker must be in a real `//` comment
    rather than inside a string literal. Over-matching is intended.

    Spellings NOT closed, and not claimed to be: a tool reached through another
    process crate (`duct`, `xshell`, a raw `exec`), a path assembled at runtime from
    pieces, and a wrapper re-exported under another name from outside the scan trees.
    The ledger is the backstop for all of them - such a test either reached the gate
    (and was recorded) or did not (and is reported).

CHECK B - the helper modules are one module
    The per-crate copies of `tests/support/mod.rs` must be byte-identical.
    `sparq-zk` cannot depend on `sparq-zk-compose`'s test tree (it is the LOWER
    crate), so the gate is duplicated rather than shared.

CHECK F - the src tests use the SAME gate file
    A `src` test module cannot reach `tests/support`. Rather than a third copy, the
    crate root compiles that same file in:

        #[cfg(test)]
        #[path = "../tests/support/mod.rs"]
        mod support;

    A crate with a gate call in `src` must carry exactly that declaration, so the
    gate the `src` tests use cannot drift from the one the checker reads.

CHECK C - the driver route and the spawn result

      C1  a test declared `structural` (CHECK L) reaches the driver route without
          the gate, so it is safe only while every outcome it accepts names a
          specific error variant: a missing tool's spawn failure is then a different
          variant and the test fails. An arm matching `Err(_)`, `Err(..)`,
          `.is_err()`, a wildcard that accepts rather than panics, or an
          `Err(e) => ...` arm whose body neither panics nor asserts, in such a test,
          IS the soft pass and is reported. The scope is ONE `#[test]` function.
      C2  `DriverError::Spawn` used as a PATTERN - a match arm (its `=>` may be on a
          later line), an or-pattern, a guard, `if let` / `while let` / `let ...
          else`, or `if matches!(..)` - is an error unless annotated: branching on a
          spawn failure is a soft pass. `assert!(matches!(.., Spawn ..))` asserts
          the variant and is not reported.
      C3  the value a `live_tool(` call spawns must be bound to a name, followed
          by an assertion of its success (`out.status.success()`, `out.success()`,
          or an exit code equal to `Some(0)`) or a positive expected-diagnostic
          check on that value's `stdout` / `stderr`. A failed status alone does
          not identify a constraint. Negated diagnostics, empty diagnostic
          literals, disjunctions, unrelated output checks and inline spawns are
          rejected. An unwrap proves only that the process started.

CHECK G - completion placement
    Every inventoried gate call sits directly in a `#[test]` function, one per
    function, and that function's LAST statement (before a tail `Ok(())`) is the
    completion call `support::complete(<the token the gate returned>)`. Completion
    calls outside that final statement, including calls in helpers, are rejected.
    There is exactly one completion call, and no `return` or `?` between the gate
    statement and it. A completion moved before the assertions, deleted, duplicated,
    applied to another value, or skipped by an early exit is reported before any
    lane runs; the ledger reports the same at run time.

CHECK L - the declared inventory
    `crates/<crate>/tests/live-tests.txt` lists, independently of the scanner, every
    test that reaches nargo/bb, by stable identity `<crate-relative path>::<fn>`:

      live        it calls the gate (and so must complete it, CHECK G);
      structural  it reaches the driver route WITHOUT the gate because every
                  outcome it accepts is decided before any subprocess (C1).

    The scanner DISCOVERS, per `#[test]` function (following local helpers one
    fixpoint deep), the ones that call the gate and the ones that reach nargo/bb
    another way: `CircuitProver`, `driver::`, `DriverError::Spawn`, `live_tool(`, or
    a spawned tool, shell or launcher (A1, annotated or not). The two are compared in
    BOTH directions: a gated test not declared `live`, a `live` entry the scanner
    does not find gated, a routed ungated test not declared `structural`, and a
    `structural` entry that is gated or no longer routed are all errors. A driver-
    route test therefore has to be gated, or explicitly declared not to spawn.

CHECK D - the verify step is wired and cannot be skipped
    Exactly one step named VERIFY_STEP, in the job that carries `id: changes` and the
    two toolchain installs, as that job's LAST step; its `run` exactly the one verify
    command with the ledger path as a LITERAL (not read from the environment, which a
    reordering or an override could change); ONLY the keys `name` and `run` on it -
    an allow-list, because `shell: "true {0}"` would run the command through a
    program that ignores it and `timeout-minutes` could cut it short; no `defaults:`
    block at workflow or job level, which would set the step's shell and working
    directory from a distance; no `shell:` other than `bash` on any step of the job;
    the job's `if:` exactly ACCEPTED_JOB_IF and no
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
    Every gate call site in the scan trees, keyed `<file>:<line>` exactly as the gate
    records one:

      * the match is deliberately OVER-matching: any path prefix, or none, ending
        in `gate(`. `tc::gate(` and a bare `gate(` are call
        sites, so an import or a module alias cannot hide one. It is matched against
        text with comments AND string literals blanked, so a panic message that
        mentions "the holder gate (...)" is not a call site. A false site costs an
        INV error, or a key nothing records - loud, never silent;
      * two gate calls on ONE line share a key, so one could not be told from the
        other in the ledger; that is an error;
      * the first argument must still be a STRING LITERAL, on the call's line or in a
        joined window of the following lines - not for identity, but so a skip or
        panic line names something a reader can find in the source;
      * the file must belong to a cargo test target: `tests/<name>.rs`,
        `tests/<name>/main.rs`, a `[[test]]` path, or `src` test code (the lib
        target). A `[[test]]` path that does not exist is an error, and a gate call
        in a file whose target cannot be determined is an error.

    The site must sit directly in a `#[test]` function (CHECK G), so each key maps
    to exactly one test and its declared identity (CHECK L).

    `LEDGER_EXCLUSIONS` lists call sites a lane ledger will not contain. Each entry
    exempts exactly ONE site and carries its reason. A line move invalidates the key,
    which shows up as a missing entry - red, never a silent pass.

SCAN TREES
    `tests/` in both crates (every `*.rs`, whole file), each `[[test]] path` named by
    either Cargo.toml wherever it points, and both crates' `src/` (the lib test
    target runs their `#[cfg(test)]` code). The two scopes over `src/` differ on
    purpose: the acquisition LINT reads only the test-only regions, because the
    driver's production code spawns nargo/bb for real and linting that is
    meaningless; the INVENTORY reads every `src/**/*.rs` in FULL, because a gate call
    anywhere there is a call the lane must reach, and `crate::support` compiles only
    under `cfg(test)`. A file pulled in by `#[cfg(test)] mod NAME;` is test code in
    its entirety, and so is anything IT declares with a plain `mod NAME;`. The
    region reader models `#[cfg(test)]` and `#[cfg(all(test, ...))]` on an item
    (brace-matched, or to the terminating `;`), `#[cfg(test)] mod NAME;` (the named
    file, in its entirety), and a file-level `#![cfg(test)]`. Any other cfg attribute
    naming `test`, an item that never closes, and a `mod NAME;` resolving to no file
    are ERRORS, not omissions. A missing scan directory is an error.

LIMITATIONS, stated rather than implied
    * Discovery (CHECK L) and C1 follow only functions defined in the SAME file; a
      test that reaches nargo/bb through a helper in another module or crate, or a
      macro, is not discovered, and a `structural` declaration is an explicit claim
      that the test spawns nothing, not a proof of it. The ledger is the backstop
      for a live test that is gated; an ungated test that spawns through such a
      helper is not seen by either.
    * C1 reads arms by spelling. `.ok()`, `unwrap_or(..)` and `if let Ok(x) = ...`
      swallow an error without matching any of its patterns, so they are NOT
      flagged in a `structural` test.
    * The statement reader is a bracket counter over blanked text, not a Rust
      parser. A macro whose body is not ordinary statements, or a closure that
      returns early, can be read as a different statement shape; G reads a
      `return` / `?` inside a closure after the gate as an early exit (an
      over-report, never a silent pass).
    * C3 recognises the exit-status assertion only in the statements that follow
      the spawn in the SAME block; an assertion made by a caller on a returned
      value is reported.
    * The completion line proves the body reached the call after its assertions.
      It does not grade what those assertions check.

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
# `src` trees: live tests also live in the crates' own `#[cfg(test)]` code, which the
# lib test target runs. Only the test-only REGIONS of these files are linted and
# inventoried; production code above them spawns nargo/bb legitimately.
SRC_DIRS: list[tuple[str, str]] = [
    ("sparq-zk", "crates/sparq-zk/src"),
    ("sparq-zk-compose", "crates/sparq-zk-compose/src"),
]
# The ONE permitted way for a crate's `src` tests to reach the gate: the SAME FILE,
# compiled in with `#[path]`. A second copy could drift; this cannot.
SRC_GATE_DECL = (
    '#[cfg(test)]\n#[path = "../tests/support/mod.rs"]\nmod support;'
)
LIB_ROOTS: dict[str, str] = {
    "sparq-zk": "crates/sparq-zk/src/lib.rs",
    "sparq-zk-compose": "crates/sparq-zk-compose/src/lib.rs",
}
HELPER_MODULES: list[str] = [
    "crates/sparq-zk/tests/support/mod.rs",
    "crates/sparq-zk-compose/tests/support/mod.rs",
]
CARGO_TOMLS: dict[str, str] = {
    "sparq-zk": "crates/sparq-zk/Cargo.toml",
    "sparq-zk-compose": "crates/sparq-zk-compose/Cargo.toml",
}

LIVE_TOOLS = ("nargo", "bb")
SHELLS = ("sh", "bash", "dash", "zsh")
# Programs that run ANOTHER program named in their arguments. `Command::new("env")`
# with `nargo` in `.args([...])` reaches the tool without naming it as the
# executable, so a launcher is reported like a shell.
LAUNCHERS = ("env", "xargs", "nice", "nohup", "timeout", "stdbuf", "time", "setsid",
             "sudo", "busybox", "chrt", "ionice", "taskset", "unbuffer")

# The checked-in DECLARED inventory of live tests, one file per crate, independent
# of the scanner. Each non-comment line is `live <path>::<fn>` or
# `structural <path>::<fn>`, with `<path>` relative to the crate directory. The
# checker compares it against what the scanner DISCOVERS in both directions.
DECLARED_REL: dict[str, str] = {
    "sparq-zk": "crates/sparq-zk/tests/live-tests.txt",
    "sparq-zk-compose": "crates/sparq-zk-compose/tests/live-tests.txt",
}
DECLARED_KINDS = ("live", "structural")

# Call sites a lane ledger will NOT contain, keyed exactly as the gate records them
# (`<file>:<line>`), one entry per site with its reason. A line move invalidates the
# key, which turns into a missing entry — red, never a silent pass.
LEDGER_EXCLUSIONS: dict[str, str] = {
    "crates/sparq-zk/tests/toolchain_gate.rs:64": (
        "the gate's own probe (gate_child_probe): in the parent sweep it returns "
        "before reaching the gate, and the re-exec'd child reaches it with a "
        "deliberately absent tool name, so the gate correctly records nothing on "
        "either path"
    ),
    "crates/sparq-zk-compose/tests/toolchain_gate.rs:64": (
        "the gate's own probe in the byte-identical per-crate copy; same reason as "
        "the sparq-zk entry"
    ),
}

# Exact allow-annotation ratchet: file -> number of annotations. Each annotation is
# scoped to its own line (or the line below it), never to the file.
ALLOW_RATCHET: dict[str, int] = {
    "crates/sparq-zk-compose/src/driver.rs": 9,
    "crates/sparq-zk-compose/src/result.rs": 3,
    "crates/sparq-zk-compose/src/result/proof_bindings.rs": 1,
    "crates/sparq-zk-compose/src/result/signed.rs": 2,
    "crates/sparq-zk-compose/tests/e2e.rs": 0,
    "crates/sparq-zk-compose/tests/holder_pok_binding.rs": 0,
    "crates/sparq-zk-compose/tests/coverage_direct.rs": 2,
    "crates/sparq-zk-compose/tests/host_helpers.rs": 2,
    "crates/sparq-zk-compose/tests/toolchain_gate.rs": 1,
    "crates/sparq-zk/tests/toolchain_gate.rs": 1,
}

# Deliberately OVER-MATCHING: any path prefix (or none) ending in `gate(`.
# `support::gate(`, `crate::support::gate(`, `tc::gate(` and a bare `gate(`
# all count, so an import or a module alias cannot hide a call site.
# The cost is a false site, which is an INV error or a ledger key nothing records -
# loud, not silent. Matched against text with comments AND string literals blanked,
# so prose naming "the holder gate (...)" is not a call site.
GATE_SITE_RE = re.compile(
    r"(?:[A-Za-z_]\w*\s*::\s*)*\bgate\s*\("
)
NEW_RE = re.compile(
    r"\b([A-Za-z_][A-Za-z0-9_]*(?:\s*::\s*[A-Za-z_][A-Za-z0-9_]*)*)\s*::\s*new\s*\("
)
NOTFOUND_RE = re.compile(r"\bNotFound\b")
CMD_MACRO_RE = re.compile(r"\bcmd!\s*\(")
DRIVER_REF_RE = re.compile(r"CircuitProver|driver::|live_tool\s*\(|DriverError::Spawn")
# A result arm that ACCEPTS an unnamed error. In a file that reaches the driver
# without the gate, such an arm is what lets a missing-tool spawn failure be read as
# the rejection the test was looking for. An arm that names a specific variant
# (`Err(CheckError::HolderKeyMismatch) => {}`) is not one of these, and neither is a
# wildcard arm that panics.
PERMISSIVE_RES: tuple[re.Pattern[str], ...] = (
    re.compile(r"Err\s*\(\s*(?:_|\.\.)\s*\)"),
    re.compile(r"Err\s*\(\s*[a-z_]\w*\s*\)\s*=>\s*(?:\{\s*\}|\(\s*\))"),
    re.compile(r"\.is_err\s*\(\s*\)"),
    re.compile(r"(?:^|\W)(?:_|other|err|e)\s*=>\s*(?:\{\s*\}|\(\s*\))"),
)
# One ledger line: written by the gate (`enter`) or the completion call
# (`complete`), always followed by the gate's call-site key.
LEDGER_LINE_RE = re.compile(r"(enter|complete) (\S+:\d+)")
EXEMPT_RE = re.compile(r"//\s*zk-toolchain-gate:\s*allow\s+(\S.*)")
# The gate taken INDIRECTLY: renamed in a `use`, or referenced as a function value.
# Either would let a call site record a ledger entry from a line the inventory does
# not read, so both are rejected outright. The `use` side is read per STATEMENT in
# `lint_file` (a `use` may wrap over several lines); this is the value side.
GATE_VALUE_RE = re.compile(
    r"\bsupport\s*::\s*(?:gate|live_tool|complete)"
    r"\b(?!\s*\()"
)
# `#[cfg(test)]`, and the one narrowed form the trees use. Both are test-only.
CFG_TEST_OUTER_RE = re.compile(
    r"^\s*#\[cfg\((?:test|all\(\s*test\s*,[^)]*\))\)\]\s*$"
)
CFG_TEST_INNER_RE = re.compile(r"^\s*#!\[cfg\(test\)\]\s*$")
# Any OTHER cfg attribute naming `test` is a shape this reader does not model; it is
# an error rather than a silent omission.
CFG_TEST_ANY_RE = re.compile(r"#!?\[\s*cfg[^]]*\btest\b")
ATTR_OR_BLANK_RE = re.compile(r"^\s*(?:#|//|$)")
STRINGISH_RE = re.compile(
    r"r#*\"(?:[^\"]|\"(?!#))*\"#*|\"(?:[^\"\\]|\\.)*\"|'(?:[^'\\]|\\.)'"
)


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


def strip_noncode(text: str) -> str:
    """`text` with comments and literal CONTENTS blanked, newlines preserved.

    Line-by-line comment stripping cannot do this: the trees contain multi-line
    string literals (a `\\` continuation inside `format!`) that carry unbalanced
    braces, and a brace counter fed those lines walks off the end of the file. This
    is a small state machine over the whole text instead, handling `//`, nested
    `/* */`, `"..."`, `r#"..."#` and character literals. A `'` is read as a
    character literal only when it closes within a few characters, so a lifetime
    (`&'static`) stays code. Every blanked character becomes a space and every
    newline is kept, so line and column numbers do not move.
    """
    out: list[str] = []
    i, n = 0, len(text)
    while i < n:
        c = text[i]
        if c == "/" and i + 1 < n and text[i + 1] == "/":
            while i < n and text[i] != "\n":
                out.append(" ")
                i += 1
            continue
        if c == "/" and i + 1 < n and text[i + 1] == "*":
            depth = 0
            while i < n:
                if text.startswith("/*", i):
                    depth += 1
                    out.append("  ")
                    i += 2
                    continue
                if text.startswith("*/", i):
                    depth -= 1
                    out.append("  ")
                    i += 2
                    if depth == 0:
                        break
                    continue
                out.append("\n" if text[i] == "\n" else " ")
                i += 1
            continue
        if c == "r" and i + 1 < n:
            j = i + 1
            hashes = 0
            while j < n and text[j] == "#":
                hashes += 1
                j += 1
            if j < n and text[j] == '"':
                close = '"' + "#" * hashes
                end = text.find(close, j + 1)
                end = n if end < 0 else end + len(close)
                out.extend("\n" if ch == "\n" else " " for ch in text[i:end])
                i = end
                continue
        if c == '"':
            out.append(" ")
            i += 1
            while i < n:
                if text[i] == "\\":
                    # An escape, possibly `\` + NEWLINE (a continued string literal).
                    # The newline must survive or every later line number shifts.
                    for ch in text[i : i + 2]:
                        out.append("\n" if ch == "\n" else " ")
                    i += 2
                    continue
                if text[i] == '"':
                    out.append(" ")
                    i += 1
                    break
                out.append("\n" if text[i] == "\n" else " ")
                i += 1
            continue
        if c == "'":
            m = re.match(r"'(?:\\.|[^\\'])'", text[i:])
            if m:
                out.extend(" " for _ in range(m.end()))
                i += m.end()
                continue
        out.append(c)
        i += 1
    return "".join(out)


def _brace_span(code_lines: list[str], head: int) -> tuple[int, int] | str:
    """The span of the item starting at `head`, over ALREADY-stripped lines."""
    depth = 0
    opened = False
    for i in range(head, len(code_lines)):
        for ch in code_lines[i]:
            if ch == "{":
                depth += 1
                opened = True
            elif ch == "}":
                depth -= 1
                if opened and depth == 0:
                    return (head, i)
            elif ch == ";" and not opened and depth == 0:
                return (head, i)
    return f"the item at line {head + 1} never closes"


FN_HEAD_RE = re.compile(
    r"^\s*(?:#\[[^\]]*\]\s*)*(?:pub(?:\([^)]*\))?\s+)?"
    r"(?:const\s+|async\s+|unsafe\s+|extern\s+\"[^\"]*\"\s+)*fn\s+(\w+)"
)


def fn_spans(text: str) -> list[tuple[int, int, str]]:
    """(start, end, name) 0-based line spans of the top-level `fn` items in `text`.

    Used to scope CHECK C1 to ONE test function instead of a whole module: a test
    that reaches the prover and a sibling test with a permissive arm are different
    functions, and conflating them is how C1 used to fire on toolchain-free code.
    A `fn` nested inside another is absorbed by the outer span.
    """
    code = strip_noncode(text).splitlines()
    spans: list[tuple[int, int, str]] = []
    i = 0
    while i < len(code):
        m = FN_HEAD_RE.match(code[i])
        if m:
            span = _brace_span(code, i)
            if isinstance(span, str):
                return spans
            spans.append((span[0], span[1], m.group(1)))
            i = span[1] + 1
            continue
        i += 1
    return spans


def _reaching(bodies: dict[str, str], seed: re.Pattern[str]) -> set[str]:
    """Local function names whose body matches `seed`, or calls one that does.

    One fixpoint over the functions defined in this file, so a `#[test]` that builds
    its prover (or calls the gate) in a local helper is judged by what that helper
    does. Only names defined HERE are followed; a helper in another module is not,
    and that limit is stated in this file's header.
    """
    hits = {name for name, body in bodies.items() if seed.search(body)}
    changed = True
    while changed:
        changed = False
        for name, body in bodies.items():
            if name in hits:
                continue
            if any(re.search(rf"\b{re.escape(h)}\s*\(", body) for h in hits):
                hits.add(name)
                changed = True
    return hits


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
    """Is line `i` covered by an allow annotation?

    Scope is ONE LINE: the annotation must be on that line, or in the comment block
    immediately above it (so a long reason may wrap onto continuation comment lines).
    It never covers a file, and a blank line or any code between the comment and the
    line ends the block, so an annotation cannot drift onto later code.
    """
    if 0 <= i < len(lines) and annotation(lines[i]):
        return True
    j = i - 1
    while j >= 0 and lines[j].strip().startswith("//"):
        if annotation(lines[j]):
            return True
        j -= 1
    return False


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
class Unit:
    """One scanned source file, and the lines of it that are TEST code.

    `scope` is `None` for a whole test file under `tests/`; for a `src` file it is
    the set of 1-based line numbers inside `#[cfg(test)]` regions. `scoped_text`
    blanks every other line, so the line numbers a finding reports are the file's
    real ones and production code above a test module is never linted.
    """

    pkg: str
    rel: str
    text: str
    scope: set[int] | None = None

    def scoped_text(self) -> str:
        if self.scope is None:
            return self.text
        return "\n".join(
            line if (i + 1) in self.scope else ""
            for i, line in enumerate(self.text.splitlines())
        )


@dataclass
class Trees:
    files: list[Unit] = field(default_factory=list)
    binary_of: dict[str, str] = field(default_factory=dict)
    findings: list[Finding] = field(default_factory=list)


def _item_span(lines: list[str], code_lines: list[str],
               attr: int) -> tuple[int, int] | str:
    """The line span (0-based, inclusive) of the item annotated at line `attr`.

    Returns a `(start, end)` pair, or a string describing a shape this reader does
    not model — which the caller turns into a SCAN error rather than skipping.
    """
    i = attr + 1
    while i < len(lines) and ATTR_OR_BLANK_RE.match(lines[i]):
        i += 1
    if i >= len(lines):
        return "the annotated item is missing"
    span = _brace_span(code_lines, i)
    if isinstance(span, str):
        return span
    return (attr, span[1])


def _mod_file(root: Path, rel: str, name: str,
              explicit_path: str | None, inline: tuple[str, ...] = ()) -> Path | None:
    """The file a `mod NAME;` in `rel` resolves to, or None."""
    here = (root / rel).parent
    if explicit_path is not None:
        cand = here.joinpath(*inline, explicit_path)
        return cand if cand.is_file() else None
    stem = Path(rel).stem
    if stem not in ("lib", "mod", "main"):
        here = here / stem
    here = here.joinpath(*inline)
    for cand in (here / f"{name}.rs", here / name / "mod.rs"):
        if cand.is_file():
            return cand
    return None


def _rel_to(root: Path, path: Path) -> str | None:
    try:
        return str(path.resolve().relative_to(root.resolve()))
    except ValueError:
        return None


MOD_DECL_RE = re.compile(r"^\s*(?:pub(?:\([^)]*\))?\s+)?mod\s+(\w+)\s*;")
PATH_ATTR_RE = re.compile(r'^\s*#\[path\s*=\s*"([^"]+)"\]')


def _mod_targets(root: Path, rel: str, text: str, out: Trees) -> list[str]:
    """Every `mod NAME;` child file of `rel`, as repository-relative paths.

    Used on a file that is test code in FULL: its children are too, and nothing in
    them carries a `cfg(test)` attribute of its own to find them by.
    """
    found: list[str] = []
    code = strip_noncode(text)
    # Each inline module adds a directory to the ordinary Rust module path.
    # Read the whole blanked file so raw strings and block comments cannot invent
    # modules, and declarations on the same line retain their nesting.
    parents: list[tuple[str, int]] = []
    decl = re.compile(r"((?:\#\s*\[[^\]]*\]\s*)*)(?:pub(?:\([^)]*\))?\s+)?"
                      r"\bmod\s+(\w+)\s*([;{])")
    for m in decl.finditer(code):
        parents = [(name, end) for name, end in parents if end > m.start()]
        name = m.group(2)
        if m.group(3) == "{":
            _, end = enclosing_block(code, m.end())
            parents.append((name, end))
            continue
        attrs = text[m.start(1):m.end(1)]
        pm = re.search(r'\#\[path\s*=\s*"([^"]+)"\]', attrs)
        target = _mod_file(root, rel, name, pm.group(1) if pm else None,
                           tuple(n for n, _ in parents))
        if target is None:
            out.findings.append(
                Finding(f"{rel}:{line_of(code, m.start()) + 1}", "SCAN",
                        f"`mod {name};` in a test-only file resolves to no "
                        "file; the module's tests would not be scanned")
            )
            continue
        erel = _rel_to(root, target)
        if erel is not None:
            found.append(erel)
    return found


def _src_scope(root: Path, pkg: str, rel: str, text: str,
               out: Trees) -> tuple[set[int], list[str]]:
    """The test-only line numbers of a `src` file, plus the files it pulls in with
    `#[cfg(test)] mod NAME;` (which are test code in their entirety)."""
    lines = text.splitlines()
    scope: set[int] = set()
    extra: list[str] = []
    if not CFG_TEST_ANY_RE.search(text):
        # No cfg attribute names `test` anywhere, so there is no region to find and
        # no reason to pay for the whole-file literal scan.
        return scope, extra
    code_lines = strip_noncode(text).splitlines()
    for i, line in enumerate(lines):
        code = code_of(line)
        if not CFG_TEST_ANY_RE.search(code):
            continue
        if CFG_TEST_INNER_RE.match(code):
            return set(range(1, len(lines) + 1)), extra
        if not CFG_TEST_OUTER_RE.match(code):
            out.findings.append(
                Finding(f"{rel}:{i + 1}", "SCAN",
                        "a cfg attribute naming `test` in a shape this reader does "
                        f"not model ({code.strip()!r}); the region would be linted "
                        "wrongly, so model it here first")
            )
            continue
        span = _item_span(lines, code_lines, i)
        if isinstance(span, str):
            out.findings.append(Finding(f"{rel}:{i + 1}", "SCAN", span))
            continue
        start, end = span
        region = lines[start + 1 : end + 1]
        # An explicit `#[path = "..."]` wins over the usual module-file layout; that
        # is how a crate compiles in the shared gate file (CHECK F).
        explicit_path = None
        for attr in region:
            pm = PATH_ATTR_RE.match(attr)
            if pm:
                explicit_path = pm.group(1)
        body = "\n".join(
            line for line in region if not ATTR_OR_BLANK_RE.match(line)
        ).strip()
        m = re.match(r"(?:pub(?:\([^)]*\))?\s+)?mod\s+(\w+)\s*;", body)
        if m:
            target = _mod_file(root, rel, m.group(1), explicit_path)
            erel = None if target is None else _rel_to(root, target)
            if erel is None:
                out.findings.append(
                    Finding(f"{rel}:{i + 1}", "SCAN",
                            f"`#[cfg(test)] mod {m.group(1)};` resolves to no file "
                            f"at {explicit_path or 'the module path'}; the module's "
                            "tests would not be scanned")
                )
            else:
                extra.append(erel)
        else:
            # Inline test modules can themselves declare external children.
            # Keep original line offsets for diagnostics and nested module paths.
            region_text = "\n" * (start + 1) + "\n".join(region)
            extra.extend(_mod_targets(root, rel, region_text, out))
        scope.update(range(start + 1, end + 2))
    return scope, extra


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
            out.files.append(
                Unit(pkg, relp, path.read_text(encoding="utf-8", errors="replace"))
            )
            if r in helper_abs:
                continue
            if relp in explicit:
                out.binary_of[relp] = explicit[relp]
            elif path.parent == tests_root:
                out.binary_of[relp] = path.stem
            elif path.name == "main.rs" and path.parent.parent == tests_root:
                out.binary_of[relp] = path.parent.name
            # Otherwise the file is not a target; a gate call in it is an INV error.

    # `src` trees. The lib test target runs this crate's `#[cfg(test)]` code, so a
    # live test here needs the gate exactly as one under `tests/` does. Two different
    # scopes apply, on purpose:
    #   * the ACQUISITION LINT reads only the test-only regions, because the driver's
    #     production code spawns nargo/bb for real and linting that is meaningless;
    #   * the INVENTORY reads every file in FULL, because a gate call anywhere in
    #     `src` is a call the lane must reach. `crate::support` exists only under
    #     `cfg(test)`, so a gate call outside a region either is test code the region
    #     reader did not recognise, or does not compile.
    for pkg, rel in SRC_DIRS:
        d = root / rel
        if not d.is_dir():
            out.findings.append(
                Finding(rel, "SCAN",
                        "src scan directory is missing; src tests would be silently "
                        "unscanned")
            )
            continue
        texts: dict[str, str] = {}
        scopes: dict[str, set[int]] = {}
        pulled: list[str] = []
        for path in sorted(d.rglob("*.rs")):
            relp = str(path.relative_to(root))
            texts[relp] = path.read_text(encoding="utf-8", errors="replace")
            scope, extra = _src_scope(root, pkg, relp, texts[relp], out)
            scopes[relp] = scope
            pulled.extend(extra)
        # A file pulled in by `#[cfg(test)] mod NAME;` is test code in FULL, and so
        # is anything IT declares with a plain `mod NAME;` — the child module of a
        # test-only file is not reached by any `cfg(test)` attribute of its own.
        full: set[str] = set()
        while pulled:
            erel = pulled.pop()
            if erel in full:
                continue
            full.add(erel)
            if erel not in texts:
                p_abs = root / erel
                if not p_abs.is_file():
                    continue
                texts[erel] = p_abs.read_text(encoding="utf-8", errors="replace")
            pulled.extend(_mod_targets(root, erel, texts[erel], out))
        for relp, text in sorted(texts.items()):
            out.files.append(
                Unit(pkg, relp, text, None if relp in full else scopes.get(relp, set()))
            )
            out.binary_of[relp] = f"{pkg} lib"
    return out


def check_src_gate_decl(root: Path, trees: Trees) -> list[Finding]:
    """A crate whose `src` tests call the gate must declare the ONE shared module.

    The gate lives in `tests/support/mod.rs`; a `src` test module cannot reach it, so
    the crate compiles that same file in with `#[path]`. A second copy is not an
    option: it could drift from the one the checker reads.
    """
    findings: list[Finding] = []
    want = SRC_GATE_DECL
    for pkg, lib_rel in LIB_ROOTS.items():
        uses_gate = any(
            u.rel.startswith(f"crates/{pkg}/src/")
            and GATE_SITE_RE.search(strip_noncode(u.scoped_text()))
            for u in trees.files
        )
        p = root / lib_rel
        declared = p.is_file() and want in p.read_text(encoding="utf-8")
        if uses_gate and not declared:
            findings.append(
                Finding(lib_rel, "F",
                        "a src test in this crate calls the gate, so the crate root "
                        f"must declare it as the shared source:\n    "
                        + want.replace("\n", "\n    "))
            )
    return findings


# --------------------------------------------------------------------------- #
# CHECK A / B / C / E
# --------------------------------------------------------------------------- #


def line_of(text: str, pos: int) -> int:
    """0-based line number of character offset `pos` in `text`."""
    return text.count("\n", 0, pos)


_OPEN = "([{"
_CLOSE = ")]}"


def statements(code: str, start: int, end: int) -> list[tuple[int, int]]:
    """Top-level statement spans `(begin, end)` (end exclusive) of `code[start:end]`.

    `code` is STRIPPED text (comments and literal contents blanked), so every
    bracket here is real syntax. A statement ends at a `;` at depth zero, or at a
    `}` that returns to depth zero when what follows cannot continue the same
    expression (`.`, `?`, `;`, `,`, `)`, `else`, `as`). A trailing expression with
    no terminator is the last span. This is what lets a rule ask "which statement
    is this call in" and "what is the last statement of this function" across line
    breaks, rather than reading one line at a time.
    """
    spans: list[tuple[int, int]] = []
    depth = 0
    begin: int | None = None
    i = start
    while i < end:
        c = code[i]
        if begin is None:
            if c.isspace():
                i += 1
                continue
            begin = i
        if c in _OPEN:
            depth += 1
        elif c in _CLOSE:
            depth -= 1
            if depth == 0 and c == "}":
                j = i + 1
                while j < end and code[j].isspace():
                    j += 1
                rest = code[j:j + 5]
                if not (j < end and (code[j] in ".?;,)" or rest.startswith("else")
                                     or re.match(r"as\b", rest))):
                    spans.append((begin, i + 1))
                    begin = None
        elif c == ";" and depth == 0:
            spans.append((begin, i + 1))
            begin = None
        i += 1
    if begin is not None and code[begin:end].strip():
        spans.append((begin, end))
    return spans


def fn_body(code: str, head_line: int) -> tuple[int, int] | None:
    """Character offsets of the `{` and matching `}` of the fn starting at
    `head_line` (0-based) in stripped `code`, or None for a body-less fn."""
    offs = 0
    for _ in range(head_line):
        offs = code.index("\n", offs) + 1
    paren = 0
    open_at = None
    depth = 0
    for i in range(offs, len(code)):
        c = code[i]
        if open_at is None:
            if c in "([":
                paren += 1
            elif c in ")]":
                paren -= 1
            elif c == "{" and paren == 0:
                open_at, depth = i, 1
            elif c == ";" and paren == 0:
                return None
            continue
        if c == "{":
            depth += 1
        elif c == "}":
            depth -= 1
            if depth == 0:
                return (open_at, i)
    return None


TEST_ATTR_RE = re.compile(r"#\s*\[\s*test\s*\]")


def test_attrs(lines: list[str], head: int) -> str:
    """The attribute text directly above the fn at line `head` (0-based): every
    contiguous attribute, doc-comment, comment or blank line above it, plus the head
    line itself (a one-line `#[test] fn case()`)."""
    out = [code_of(lines[head])]
    j = head - 1
    depth = 0
    while j >= 0:
        raw = lines[j]
        st = raw.strip()
        if not st or st.startswith("//"):
            j -= 1
            continue
        code = code_of(raw)
        # An attribute may wrap over several lines (`#[ignore = "..."\n]`).
        if st.startswith("#") or depth > 0 or st.endswith("]") or st.endswith(")]"):
            out.append(code)
            depth += code.count("]") - code.count("[")
            depth = max(depth, 0)
            if st.startswith("#") and depth == 0:
                j -= 1
                continue
            j -= 1
            continue
        break
    return "\n".join(out)


def is_test_fn(lines: list[str], head: int) -> bool:
    return bool(TEST_ATTR_RE.search(test_attrs(lines, head)))


def literal_at(text: str, pos: int) -> str | None:
    """The string literal starting at `pos` of the ORIGINAL text (after spaces)."""
    m = re.match(r'\s*"((?:[^"\\]|\\.)*)"', text[pos:])
    return m.group(1) if m else None


def _stmt_containing(spans: list[tuple[int, int]], pos: int) -> int | None:
    for k, (a, b) in enumerate(spans):
        if a <= pos < b:
            return k
    return None


def enclosing_block(code: str, pos: int) -> tuple[int, int]:
    """Offsets of the `{` and `}` of the innermost BLOCK around `pos`. Unmatched
    `(` / `[` on the way out are passed through, so a call nested inside
    `assert!(...)` belongs to the block that holds the `assert!`."""
    depth = 0
    i = pos
    while i > 0:
        i -= 1
        c = code[i]
        if c in _CLOSE:
            depth += 1
        elif c in _OPEN:
            if depth:
                depth -= 1
            elif c == "{":
                break
    open_at = i
    depth = 0
    for j in range(open_at, len(code)):
        if code[j] in _OPEN:
            depth += 1
        elif code[j] in _CLOSE:
            depth -= 1
            if depth == 0:
                return (open_at, j)
    return (open_at, len(code))


def enclosing_stmt(code: str, pos: int) -> tuple[int, int, int]:
    """`(begin, end, block_end)`: the statement holding `pos` in its innermost
    block, and where that block closes."""
    bo, bc = enclosing_block(code, pos)
    spans = statements(code, bo + 1, bc)
    k = _stmt_containing(spans, pos)
    a, b = spans[k] if k is not None else (pos, pos)
    return a, b, bc


SPAWN_PATTERN_RES: tuple[re.Pattern[str], ...] = (
    # A match arm, or-pattern or guard on the variant; the `=>` may be on a later line.
    re.compile(r"DriverError\s*::\s*Spawn\s*(?:\{[^{}]*\}|\([^()]*\))?[\s)]*(?:=>|\||if\b)"),
    re.compile(r"\b(?:if|while)\s+let\b[^;{]*?DriverError\s*::\s*Spawn"),
    # `if matches!(..)` branches; a bare `assert!(matches!(..))` ASSERTS the variant.
    re.compile(r"\bif\s+!?\s*matches!\s*\([^;{]*?DriverError\s*::\s*Spawn"),
    re.compile(r"\blet\b[^;=]*?DriverError\s*::\s*Spawn"),
)
ASSERT_MACRO_RE = re.compile(r"\b(?:assert|assert_eq|assert_ne)!\s*\(")


def _paren_args(code: str, open_paren: int) -> str:
    depth = 0
    for j in range(open_paren, len(code)):
        if code[j] in _OPEN:
            depth += 1
        elif code[j] in _CLOSE:
            depth -= 1
            if depth == 0:
                return code[open_paren + 1:j]
    return code[open_paren + 1:]


def _asserts_status(code: str, name: str) -> bool:
    """Does an assertion inspect this spawn's exit status or failure diagnostic?"""
    source = code
    code = strip_noncode(code)
    diagnostic = rf"{re.escape(name)}\s*\.\s*(?:stdout|stderr)\b"
    diagnostic_check = re.compile(
        rf"(?:String\s*::\s*from_utf8_lossy\s*\(\s*&\s*{diagnostic}\s*\)"
        rf"|{diagnostic})\s*\.\s*(?:contains|starts_with|ends_with)\s*\(")
    for m in ASSERT_MACRO_RE.finditer(code):
        args = _paren_args(code, m.end() - 1)
        # Inspect the condition, excluding optional assertion-message arguments.
        depth = 0
        remaining = ""
        for i, char in enumerate(args):
            if char in _OPEN:
                depth += 1
            elif char in _CLOSE:
                depth -= 1
            elif char == "," and depth == 0:
                remaining = args[i + 1:].strip()
                args = args[:i]
                break
        arg_start = m.end() + len(args) - len(args.lstrip())
        args = args.strip()
        while args.startswith("(") and len(_paren_args(args, 0)) == len(args) - 2:
            inner = args[1:-1]
            arg_start += 1 + len(inner) - len(inner.lstrip())
            args = inner.strip()
        if (
                (m.group(0).startswith("assert!") and re.fullmatch(
                    rf"{re.escape(name)}\s*\.\s*(?:status\s*\.\s*)?success\s*\(\s*\)", args))
                or (m.group(0).startswith("assert_eq!") and re.fullmatch(
                    rf"{re.escape(name)}\s*\.\s*(?:status\s*\.\s*)?code\s*\(\s*\)", args)
                    and re.match(r"Some\s*\(\s*0\s*\)(?:\s*,|\s*$)", remaining))):
            return True
        match = diagnostic_check.match(args)
        if (m.group(0).startswith("assert!") and match
                and len(_paren_args(args, match.end() - 1)) == len(args) - match.end() - 1):
            pattern = _paren_args(args, match.end() - 1).strip()
            # An empty literal is contained in every output, including empty output.
            if pattern or literal_at(source, arg_start + match.end()):
                return True
    return False


def lint_file(rel: str, text: str, honour_exempt: bool = True) -> list[Finding]:
    """The acquisition lint (A*) and the spawn-result rules (C2, C3) over one file.

    `honour_exempt=False` reports annotated lines too; discovery uses that, because
    an allow annotation excuses a line from the LINT, not a test from the inventory.
    """
    findings: list[Finding] = []
    lines = text.splitlines()
    code = strip_noncode(text)
    reported: set[tuple[int, str]] = set()

    def report(ln: int, rule: str, detail: str) -> None:
        if (ln, rule) in reported or (honour_exempt and exempt(lines, ln)):
            return
        reported.add((ln, rule))
        findings.append(Finding(f"{rel}:{ln + 1}", rule, detail))

    # A1/A2 over the WHOLE stripped text, so `Command` and `::new(` on different
    # lines are one match, and the arguments of the same STATEMENT are read too.
    for m in NEW_RE.finditer(code):
        if m.group(1).split("::")[-1].strip() != "Command":
            continue
        ln = line_of(code, m.start())
        lit = literal_at(text, m.end())
        if lit is None:
            arg = " ".join(text[m.end():m.end() + 60].split())
            report(ln, "A2", f"`Command::new(` with a non-literal argument ({arg[:48]!r})")
            continue
        base = basename_of(lit)
        if base in LIVE_TOOLS:
            report(ln, "A1", f'spawns the executable literal "{lit}" directly; use '
                   "`support::live_tool`")
        elif base in SHELLS:
            report(ln, "A1", f'spawns a shell ("{lit}"), which can launder a tool name '
                   "into an argument")
        elif base in LAUNCHERS:
            report(ln, "A1", f'spawns the launcher "{lit}", which runs the program named '
                   "in its arguments")
        a, b, _ = enclosing_stmt(code, m.start())
        for s in STRINGISH_RE.finditer(text[a:b]):
            lit_s = s.group(0)
            if lit_s.startswith("'"):
                continue
            inner = (re.sub(r'^r#*"|"#*$', "", lit_s) if lit_s.startswith("r")
                     else lit_s[1:-1])
            if basename_of(inner) in LIVE_TOOLS:
                report(ln, "A1", f"names the tool {inner!r} in the arguments of a "
                       "spawned command; use `support::live_tool`")

    # A4/A5/A8 on `use` STATEMENTS, which may wrap over several lines.
    for m in re.finditer(r"\buse\s+[^;]*;", code):
        ln = line_of(code, m.start())
        stmt = " ".join(m.group(0).split())
        if re.search(r"\bCommand\s+as\s+\w+", stmt):
            report(ln, "A5", "`Command` aliased under another name defeats the spelling "
                   "of A1/A2")
        if re.search(r"\bwhich\b", stmt):
            report(ln, "A4", "a `which` probe is a private tool gate")
        if re.search(r"\bsupport\b", stmt) or re.search(
                r"\b(?:gate|live_tool|complete)\s+as\s+\w+",
                stmt):
            report(ln, "A8",
                   "a `use` that reaches the gate module (a brace list, a glob, an "
                   "item alias or a module alias). The inventory reads gate CALL SITES "
                   "by spelling, so an import makes the call read as something it does "
                   "not look for. Declare `mod support;` and call `support::gate(...)` "
                   "qualified")
    for m in GATE_VALUE_RE.finditer(code):
        report(line_of(code, m.start()), "A8",
               "the gate taken as a value rather than called here; the call site the "
               "ledger records would then be somewhere the inventory does not read")

    for i, line in enumerate(lines):
        c = code_of(line)
        if NOTFOUND_RE.search(c):
            report(i, "A3", "`NotFound` is the soft-pass arm; let the gate decide")
        if re.search(r"\bwhich\s*(?:::|\()", c):
            report(i, "A4", "a `which` probe is a private tool gate")
        if CMD_MACRO_RE.search(c):
            report(i, "A6", "`cmd!(` is a process runner")
        if LEDGER_ENV in c:
            report(i, "A7", f"only the gate module may touch {LEDGER_ENV}; a test must "
                   "not be able to redirect the ledger")

    # C2: the spawn-failure variant used as a PATTERN, across line breaks.
    for rx in SPAWN_PATTERN_RES:
        for m in rx.finditer(code):
            at = m.start() + m.group(0).find("Spawn")
            report(line_of(code, at), "C2",
                   "branches on `DriverError::Spawn`, which is what a missing tool "
                   "produces; that is a soft pass")

    # C3: the value a `live_tool(` call spawns must have its exit status asserted.
    for m in re.finditer(r"\blive_tool\s*\(", code):
        ln = line_of(code, m.start())
        a, b, block_end = enclosing_stmt(code, m.start())
        stmt = " ".join(code[a:b].split())
        bind = re.match(r"let\s+(?:mut\s+)?([A-Za-z_]\w*)\s*(?::[^=]*)?=", stmt)
        if not bind or bind.group(1) == "_" :
            report(ln, "C3", "the result of this `live_tool(` spawn is discarded; bind "
                   "it and assert its exit status (or its expected failure diagnostic)")
            continue
        if not _asserts_status(text[b:block_end], bind.group(1)):
            report(ln, "C3", f"`{bind.group(1)}`, the result of this `live_tool(` spawn, "
                   "has no supported success or positive expected-diagnostic assertion in "
                   "the statements that follow; an unwrap proves only that the process started")
    return findings


# --------------------------------------------------------------------------- #
# CHECK G - completion records
# --------------------------------------------------------------------------- #

COMPLETE_CALL_RE = re.compile(
    r"\b(?:crate\s*::\s*)?support\s*::\s*complete\s*\("
)
EARLY_EXIT_RE = re.compile(r"\breturn\b|\?\s*(?=[;.)\],{}])")


@dataclass
class TestFn:
    """One `#[test]` function, by stable identity `<crate-relative path>::<name>`."""

    ident: str
    rel: str
    line: int  # 1-based head line
    name: str


def _crate_of(rel: str) -> str | None:
    for pkg in CARGO_TOMLS:
        if rel.startswith(f"crates/{pkg}/"):
            return pkg
    return None


def _ident(rel: str, name: str) -> str:
    pkg = _crate_of(rel)
    short = rel[len(f"crates/{pkg}/"):] if pkg else rel
    return f"{short}::{name}"


def check_completion(trees: Trees, sites: list["Site"]) -> list[Finding]:
    """CHECK G: every inventoried gate call sits in a `#[test]` function whose LAST
    statement closes it — `support::complete(<token>)` — with no early exit
    between the gate and that call.

    Last means after every assertion, so the completion line in the ledger exists
    only for a body that ran its checks to the end. The ledger enforces the same at
    run time; this rule names the line before any lane runs.
    """
    findings: list[Finding] = []
    by_file: dict[str, list[Site]] = {}
    for site in sites:
        if site.key in LEDGER_EXCLUSIONS:
            continue
        by_file.setdefault(site.key.rsplit(":", 1)[0], []).append(site)
    units = {u.rel: u for u in trees.files}
    # Read every completion, even in files with no gate call. A helper, closure,
    # macro or initializer cannot close a test's obligation on its behalf.
    for unit in trees.files:
        if unit.rel in HELPER_MODULES:
            continue
        code = strip_noncode(unit.text)
        calls = list(COMPLETE_CALL_RE.finditer(code))
        if not calls:
            continue
        spans = fn_spans(unit.text)
        lines = unit.text.splitlines()
        call_lines = {line_of(code, call.start()) for call in calls}
        final_calls: set[int] = set()
        for head, end, _name in spans:
            if not is_test_fn(lines, head) or not any(head <= ln <= end for ln in call_lines):
                continue
            body = fn_body(code, head)
            if body is None:
                continue
            stmts = statements(code, body[0] + 1, body[1])
            if stmts and code[stmts[-1][0]:stmts[-1][1]].strip() == "Ok(())":
                stmts.pop()
            if stmts:
                a, b = stmts[-1]
                match = COMPLETE_CALL_RE.match(code, a, b)
                if match and code[a:b].rstrip().endswith(";"):
                    final_calls.add(match.start())
        for call in calls:
            if call.start() not in final_calls:
                findings.append(Finding(
                    f"{unit.rel}:{line_of(code, call.start()) + 1}", "G",
                    "`support::complete(token)` must be the LAST statement of a "
                    "`#[test]` function; helpers and other contexts cannot complete it"))
    for rel, file_sites in sorted(by_file.items()):
        text = units[rel].text
        lines = text.splitlines()
        code = strip_noncode(text)
        spans = fn_spans(text)
        per_fn: dict[int, list[Site]] = {}
        for site in file_sites:
            ln = int(site.key.rsplit(":", 1)[1]) - 1
            owner = [sp for sp in spans if sp[0] <= ln <= sp[1]]
            if not owner or not is_test_fn(lines, owner[0][0]):
                findings.append(
                    Finding(site.key, "G",
                            "a gate call outside a `#[test]` function: the ledger key "
                            "must belong to exactly one test, which completes it")
                )
                continue
            per_fn.setdefault(owner[0][0], []).append(site)
        for head, fsites in sorted(per_fn.items()):
            name = next(n for a, _b, n in spans if a == head)
            if re.search(r"#\[\s*should_panic\b", test_attrs(lines, head)):
                findings.append(Finding(fsites[0].key, "G",
                                        "a live test must assert the expected error, not use "
                                        "#[should_panic]: an uncompleted token also panics"))
            if len(fsites) > 1:
                findings.append(
                    Finding(fsites[1].key, "G",
                            f"`{name}` calls the gate more than once; one gate call per "
                            "live test, so each completion closes exactly one site")
                )
                continue
            site = fsites[0]
            body = fn_body(code, head)
            if body is None:
                continue
            stmts = statements(code, body[0] + 1, body[1])
            site_line = int(site.key.rsplit(":", 1)[1]) - 1
            site_pos = sum(len(x) + 1 for x in code.split("\n")[:site_line])
            gm = GATE_SITE_RE.search(code, site_pos)
            k = _stmt_containing(stmts, gm.start()) if gm else None
            if k is None:
                findings.append(Finding(site.key, "G", "the gate call is not a statement "
                                        "of the test body itself"))
                continue
            gate_stmt = " ".join(code[stmts[k][0]:stmts[k][1]].split())
            calls = list(COMPLETE_CALL_RE.finditer(code, body[0], body[1]))
            last = len(stmts) - 1
            if last > k and " ".join(code[stmts[last][0]:stmts[last][1]].split()) == "Ok(())":
                last -= 1
            last_stmt = " ".join(code[stmts[last][0]:stmts[last][1]].split())
            want = "support::complete(<token>)"
            if not calls:
                findings.append(
                    Finding(site.key, "G",
                            f"`{name}` passes the gate but never calls `{want}`; the "
                            "ledger would hold an entry and no completion")
                )
                continue
            if len(calls) > 1:
                findings.append(
                    Finding(f"{rel}:{line_of(code, calls[1].start()) + 1}", "G",
                            f"`{name}` closes its gate site more than once")
                )
                continue
            if last <= k or not COMPLETE_CALL_RE.match(last_stmt) or not last_stmt.endswith(";"):
                findings.append(
                    Finding(f"{rel}:{line_of(code, calls[0].start()) + 1}", "G",
                            f"`{want}` must be the LAST statement of `{name}`, after "
                            "every assertion; here it is followed by "
                            f"{last_stmt[:60]!r}")
                )
                continue
            tok = re.match(r"\S+\(\s*([A-Za-z_]\w*)\s*\)\s*;$", last_stmt)
            bound = set(re.findall(r"\b([a-z_]\w*)\b", gate_stmt.split("=", 1)[0]))
            for a, b in stmts[k + 1:last]:
                st = " ".join(code[a:b].split())
                rb = re.match(r"let\s+(?:mut\s+)?([a-z_]\w*)\s*=\s*([a-z_]\w*)\b", st)
                if rb and rb.group(2) in bound:
                    bound.add(rb.group(1))
            if not tok or tok.group(1) not in bound:
                findings.append(
                    Finding(f"{rel}:{line_of(code, calls[0].start()) + 1}", "G",
                            f"`{name}` completes a value that is not the token its "
                            "gate call returned")
                )
                continue
            for a, b in stmts[k + 1:last]:
                em = EARLY_EXIT_RE.search(code, a, b)
                if em:
                    findings.append(
                        Finding(f"{rel}:{line_of(code, em.start()) + 1}", "G",
                                f"an early exit in `{name}` after its gate: this path "
                                "leaves an entry and no completion. Assert instead")
                    )
                    break
    return findings


# --------------------------------------------------------------------------- #
# CHECK L - the declared inventory of live tests
# --------------------------------------------------------------------------- #


def load_declared(root: Path) -> tuple[dict[str, str], list[Finding]]:
    """`{ident: kind}` from every crate's declared list."""
    declared: dict[str, str] = {}
    findings: list[Finding] = []
    for pkg, rel in DECLARED_REL.items():
        p = root / rel
        if not p.is_file():
            findings.append(Finding(rel, "L", "the declared live-test inventory is "
                                    "missing; the scanner would have nothing to agree with"))
            continue
        for n, raw in enumerate(p.read_text(encoding="utf-8").splitlines(), 1):
            line = raw.split("#", 1)[0].strip()
            if not line:
                continue
            m = re.fullmatch(r"(\w+)\s+(\S+\.rs::\w+)", line)
            if not m or m.group(1) not in DECLARED_KINDS:
                findings.append(Finding(f"{rel}:{n}", "L",
                                        f"expected `live|structural <path>::<fn>`, got {line!r}"))
                continue
            ident = f"{pkg}/{m.group(2)}"
            if ident in declared:
                findings.append(Finding(f"{rel}:{n}", "L",
                                        f"{m.group(2)} is declared twice"))
                continue
            declared[ident] = m.group(1)
    return declared, findings


def discover(trees: Trees) -> tuple[dict[str, TestFn], set[str], set[str], dict[str, list[int]]]:
    """Every `#[test]` function, the ones that call the gate, and the ones that
    reach nargo/bb some other way: through the driver route (`CircuitProver`,
    `driver::`, `DriverError::Spawn`, `live_tool`) or a spawn of a tool, shell or
    launcher — directly or through a local helper (one fixpoint over this file)."""
    tests: dict[str, TestFn] = {}
    gated: set[str] = set()
    routed: set[str] = set()
    lines_of: dict[str, list[int]] = {}
    helper_rels = set(HELPER_MODULES)
    for unit in trees.files:
        if unit.rel in helper_rels:
            continue
        text = unit.scoped_text()
        if "#[test]" not in text.replace(" ", ""):
            continue
        lines = text.splitlines()
        code = strip_noncode(text)
        clines = code.splitlines()
        spans = fn_spans(text)
        bodies = {n: "\n".join(clines[a:b + 1]) for a, b, n in spans}
        # One lint pass per file; an A1 hit (a spawned tool, shell or launcher)
        # marks the function that holds it, and callers follow by the fixpoint.
        a1_lines = {int(f.where.rsplit(":", 1)[1]) - 1
                    for f in lint_file(unit.rel, text, honour_exempt=False)
                    if f.rule == "A1"}
        spawning = {n for a, b, n in spans if any(a <= ln <= b for ln in a1_lines)}
        reach = _reaching(bodies, DRIVER_REF_RE)
        if spawning:
            reach |= _reaching(bodies, re.compile(
                r"\bfn\s+(?:" + "|".join(map(re.escape, sorted(spawning))) + r")\b"))
        pkg = _crate_of(unit.rel)
        for a, b, n in spans:
            if not is_test_fn(lines, a):
                continue
            ident = f"{pkg}/{_ident(unit.rel, n)}"
            lines_of.setdefault(ident, []).append(a + 1)
            tests[ident] = TestFn(ident, unit.rel, a + 1, n)
            if GATE_SITE_RE.search(bodies[n]):
                gated.add(ident)
            if n in reach:
                routed.add(ident)
    return tests, gated, routed, lines_of


def check_declared(root: Path, trees: Trees) -> list[Finding]:
    declared, findings = load_declared(root)
    tests, gated, routed, lines_of = discover(trees)
    live = {i for i, k in declared.items() if k == "live"}
    structural = {i for i, k in declared.items() if k == "structural"}

    def where(ident: str) -> str:
        t = tests.get(ident)
        return f"{t.rel}:{t.line}" if t else DECLARED_REL[ident.split("/", 1)[0]]

    def short(ident: str) -> str:
        return ident.split("/", 1)[1]

    for ident, heads in sorted(lines_of.items()):
        if len(heads) > 1 and (ident in declared or ident in gated or ident in routed):
            findings.append(Finding(where(ident), "L",
                                    f"two test functions share the identity {short(ident)}; "
                                    "the declared inventory cannot tell them apart"))
    for ident in sorted(gated - live):
        findings.append(Finding(where(ident), "L",
                                f"`{short(ident)}` calls the gate but is not declared "
                                f"`live` in {DECLARED_REL[ident.split('/', 1)[0]]}"))
    for ident in sorted(live - gated):
        if ident in routed:
            why = "it reaches nargo/bb without calling the gate"
        elif ident in tests:
            why = "the scanner finds no gate call in it"
        else:
            why = "the scanner finds no such test (renamed, moved or deleted?)"
        findings.append(Finding(where(ident), "L",
                                f"`{short(ident)}` is declared `live` but {why}"))
    for ident in sorted((routed - gated) - structural):
        findings.append(Finding(where(ident), "L",
                                f"`{short(ident)}` reaches nargo/bb (the driver route, "
                                "`live_tool`, or a spawned tool/launcher) without the "
                                "gate. Gate it, complete it and declare it `live`; or, if "
                                "it provably spawns nothing (every outcome it accepts "
                                "is decided before any subprocess), declare it "
                                "`structural`"))
    for ident in sorted(structural):
        if ident in gated:
            findings.append(Finding(where(ident), "L",
                                    f"`{short(ident)}` is declared `structural` but calls "
                                    "the gate; declare it `live`"))
        elif ident not in routed:
            findings.append(Finding(where(ident), "L",
                                    f"`{short(ident)}` is declared `structural` but the "
                                    "scanner does not find it reaching the driver route; "
                                    "drop the entry"))
    findings.extend(_permissive_arms(trees, structural & routed - gated))
    return findings


ARM_RE = re.compile(r"\bErr\s*\(\s*([a-z_]\w*)\s*\)\s*=>\s*")
FAILS_RE = re.compile(r"panic!|assert|unreachable!|todo!|unimplemented!|\breturn\s+Err\b|"
                      r"\.unwrap\s*\(|\.expect\s*\(")


def _arm_accepts(code: str, start: int) -> tuple[bool, int]:
    """Does the match arm whose body begins at `start` ACCEPT (neither panic nor
    assert)? Returns that and where the body ends."""
    if code[start] == "{":
        depth = 0
        for j in range(start, len(code)):
            if code[j] == "{":
                depth += 1
            elif code[j] == "}":
                depth -= 1
                if depth == 0:
                    return (not FAILS_RE.search(code[start:j + 1]), j)
        return (False, len(code))
    depth = 0
    for j in range(start, len(code)):
        c = code[j]
        if c in _OPEN:
            depth += 1
        elif c in _CLOSE:
            if depth == 0:
                return (not FAILS_RE.search(code[start:j]), j)
            depth -= 1
        elif c == "," and depth == 0:
            return (not FAILS_RE.search(code[start:j]), j)
    return (False, len(code))


def _permissive_arms(trees: Trees, idents: set[str]) -> list[Finding]:
    """CHECK C1 over the tests declared `structural`: such a test is safe only while
    every outcome it accepts names a SPECIFIC error variant. An arm accepting an
    unnamed error (`Err(_)`, `Err(..)`, `.is_err()`, an accepting wildcard, or an
    `Err(e) => { ... }` arm whose body neither panics nor asserts) would read a
    missing tool's spawn failure as the rejection the test wanted."""
    findings: list[Finding] = []
    want: dict[str, set[str]] = {}
    for ident in idents:
        pkg, rest = ident.split("/", 1)
        path, name = rest.rsplit("::", 1)
        want.setdefault(f"crates/{pkg}/{path}", set()).add(name)
    for unit in trees.files:
        names = want.get(unit.rel)
        if not names:
            continue
        text = unit.scoped_text()
        lines = text.splitlines()
        code = strip_noncode(text)
        clines = code.splitlines()
        for a, b, n in fn_spans(text):
            if n not in names:
                continue
            for i in range(a, b + 1):
                if exempt(lines, i):
                    continue
                if any(rx.search(clines[i]) for rx in PERMISSIVE_RES):
                    findings.append(Finding(f"{unit.rel}:{i + 1}", "C1", _C1_MSG))
            lo = sum(len(x) + 1 for x in clines[:a])
            hi = sum(len(x) + 1 for x in clines[:b + 1])
            for m in ARM_RE.finditer(code, lo, hi):
                ok, _end = _arm_accepts(code, m.end())
                ln = line_of(code, m.start())
                if ok and not exempt(lines, ln):
                    findings.append(Finding(f"{unit.rel}:{ln + 1}", "C1", _C1_MSG))
    return findings


_C1_MSG = ("this `structural` test reaches the driver route without the gate, and this "
           "arm ACCEPTS an unnamed error — a missing tool's spawn failure would be read "
           "as the rejection the test wanted. Assert the specific rejection variant, or "
           "gate the test and declare it `live`")


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
    for unit in trees.files:
        n = sum(1 for line in unit.scoped_text().splitlines() if annotation(line))
        if n:
            counts[unit.rel] = n
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
    for unit in trees.files:
        if unit.rel in helper_rels:
            continue
        # The INVENTORY reads the whole file, never the lint scope: a gate call the
        # region reader did not see is still a call the lane must reach.
        scoped = unit.text
        lines = scoped.splitlines()
        # Match on text with comments AND string literals blanked: the over-matching
        # regex would otherwise read a panic message that says "the holder gate (..."
        # as a call site. Blanking preserves every offset, so the argument below is
        # still read from the real line.
        for i, code in enumerate(strip_noncode(scoped).splitlines()):
            for m in GATE_SITE_RE.finditer(code):
                # The ledger key is the CALL SITE, exactly as `#[track_caller]` and
                # `std::panic::Location` record it: the workspace-relative path of
                # this file and the line the call starts on.
                key = f"{unit.rel}:{i + 1}"
                at = key
                arg = joined_arg(lines, i, m.end())
                lit = re.match(r'"((?:[^"\\]|\\.)*)"', arg)
                if not lit:
                    findings.append(
                        Finding(at, "INV",
                                "the gate's first argument is not a string literal "
                                f"({arg[:48]!r}); a skip or panic line must name "
                                "something a reader can find in the source")
                    )
                    continue
                if trees.binary_of.get(unit.rel) is None:
                    findings.append(
                        Finding(at, "INV",
                                "the cargo test target for this file cannot be "
                                "determined (not tests/<name>.rs, tests/<name>/main.rs, "
                                "a [[test]] path, or src test code)")
                    )
                    continue
                if key in seen:
                    findings.append(
                        Finding(at, "INV",
                                f"two gate calls share the call site {key!r} (already "
                                f"at {seen[key]}), so one cannot be told from the "
                                "other in the ledger; put them on separate lines")
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
    # An allow-list, not a deny-list: `shell: "true {0}"` would run the command
    # through a program that ignores it, `timeout-minutes` could cut it short, and a
    # key nobody thought of is exactly how this step gets neutralised next time.
    extra_keys = sorted(set(verify) - {"name", "run"})
    if extra_keys:
        findings.append(
            Finding(VERIFY_STEP, "D",
                    f"may carry only `name:` and `run:`; it also carries "
                    f"{', '.join(extra_keys)}")
        )

    # `defaults` sets `shell:` and `working-directory:` for every step that does not
    # override them, so it reaches the verify step without appearing on it.
    for where, holder in ((WORKFLOW_REL, doc), (jid, job)):
        if holder.get("defaults") is not None:
            findings.append(
                Finding(where, "D",
                        "`defaults:` would set the verify step's shell and working "
                        "directory from a distance; declare neither")
            )
    for i, step in enumerate(steps):
        shell = step.get("shell")
        if shell is not None and str(shell).strip() != "bash":
            findings.append(
                Finding(f"{jid} step {i + 1} ({step.get('name')!r})", "D",
                        f"`shell: {shell!r}` — a step in this job may only use "
                        "`bash`; any other shell can swallow a non-zero exit")
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
    for unit in trees.files:
        if unit.rel not in helper_rels:
            findings.extend(lint_file(unit.rel, unit.scoped_text()))
    findings.extend(check_helpers(root))
    findings.extend(check_src_gate_decl(root, trees))
    findings.extend(check_ratchet(trees))
    sites, inv = inventory(trees)
    findings.extend(inv)
    findings.extend(check_completion(trees, sites))
    findings.extend(check_declared(root, trees))
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
        print("\nOK: the gate is the only acquisition path this lint can see, every "
              "gated test completes its gate as its last statement, the declared "
              "inventory matches what the scanner discovers in both directions, and "
              "the verify step is wired so it cannot be skipped.")
        return 0
    print("\nFAIL:", file=sys.stderr)
    for f in findings:
        print(f"  {f}", file=sys.stderr)
    return 1


def verify_ledger(root: Path, ledger: Path) -> int:
    findings, sites = static_findings(root)
    blocking = findings
    if blocking:
        print("FAIL: the static inventory does not parse or its checks fail, so the ledger cannot be verified:",
              file=sys.stderr)
        for f in blocking:
            print(f"  {f}", file=sys.stderr)
        return 1
    if not ledger.is_file():
        print(f"FAIL: the ledger {ledger} does not exist. The gate appends one line per "
              "live body it lets run, so no file means no live test reached the gate.",
              file=sys.stderr)
        return 1
    raw_lines = [
        line.strip()
        for line in ledger.read_text(encoding="utf-8", errors="replace").splitlines()
        if line.strip()
    ]
    if not raw_lines:
        print(f"FAIL: the ledger {ledger} is empty — no live test reached the gate.",
              file=sys.stderr)
        return 1
    entered: set[str] = set()
    completed: set[str] = set()
    malformed: list[str] = []
    for line in raw_lines:
        m = LEDGER_LINE_RE.fullmatch(line)
        if not m:
            malformed.append(line)
        elif m.group(1) == "enter":
            entered.add(m.group(2))
        else:
            completed.add(m.group(2))
    if malformed:
        print("FAIL: these ledger lines are neither `enter <file>:<line>` nor "
              "`complete <file>:<line>`:", file=sys.stderr)
        for line in malformed[:20]:
            print(f"  {line!r}", file=sys.stderr)
        return 1
    keys = {s.key for s in sites}
    expected = keys - set(LEDGER_EXCLUSIONS)
    unknown = sorted((entered | completed) - keys)
    orphan = sorted(completed - entered)
    no_entry = sorted(expected - entered)
    no_completion = sorted((expected & entered) - completed)
    print(f"note: {len(entered)} distinct entry and {len(completed)} distinct completion "
          f"key(s); {len(expected)} expected")
    if unknown:
        # A key the inventory did not predict means the inventory MISSED a call
        # site - an import or an alias the lint does not read, or a file outside the
        # scan trees. Reporting that without failing would leave exactly the hole
        # this check exists to close.
        print("\nFAIL: these ledger keys match no inventoried gate call site, so the "
              "inventory is not reading every site:", file=sys.stderr)
        for key in unknown:
            print(f"  {key}", file=sys.stderr)
        print("\nEither the call site is spelled in a way CHECK A8 and the inventory "
              "do not read (an import, a module alias, a macro), or its file is "
              "outside the scan trees. Fix the spelling or widen the scan - do not "
              "exclude it.", file=sys.stderr)
        return 1
    if orphan:
        print("\nFAIL: these gate call sites have a completion line but no entry line; "
              "only the gate writes an entry, so the ledger was not written by it:",
              file=sys.stderr)
        for key in orphan:
            print(f"  {key}", file=sys.stderr)
        return 1
    where = {s.key: s.where for s in sites}
    if no_entry:
        print("\nFAIL: these gate call sites left no ledger entry, so their live bodies "
              "did not run in this lane:", file=sys.stderr)
        for key in no_entry:
            print(f"  {key}  ({where.get(key, '?')})", file=sys.stderr)
        print("\nEither no step runs them, or they returned before reaching the gate. "
              "If one is legitimately not runnable in this lane, add it to "
              "LEDGER_EXCLUSIONS in this script with a reason — one entry per named "
              "test.", file=sys.stderr)
    if no_completion:
        print("\nFAIL: these gate call sites were ENTERED but never COMPLETED, so their "
              "bodies did not reach the completion call after their assertions:",
              file=sys.stderr)
        for key in no_completion:
            print(f"  {key}  ({where.get(key, '?')})", file=sys.stderr)
        print("\nThe test failed after its gate, returned early, or its completion call "
              "is missing; the test's own output in this lane says which.",
              file=sys.stderr)
    if no_entry or no_completion:
        return 1
    print("\nOK: every inventoried gate call site left an entry line AND a completion "
          "line, and the ledger holds nothing else. That proves each live body was let "
          "through the gate and reached the completion call placed after its "
          "assertions; it is not a coverage metric.")
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
const LEDGER_ENV: &str = "SPARQ_ZK_LIVE_LEDGER";

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

pub fn complete(_token: LiveToolchain) {}
'''

# A crate root that declares the shared gate module the way CHECK F requires, and a
# `src` module whose `#[cfg(test)]` region holds a gated live test.
_SRC_LIB = """\
pub mod thing;

""" + SRC_GATE_DECL + "\n"

_SRC_THING = '''\
pub fn production() -> std::process::Command {
    // Production code, OUTSIDE any test region: spawning the tool here is the
    // driver's job and must never be linted.
    std::process::Command::new("nargo")
}

#[cfg(test)]
mod tests {
    #[test]
    fn src_live_case() {
        let gate = match crate::support::gate("src_live_case", &["nargo"]) {
            Some(g) => g,
            None => return,
        };
        let out = crate::support::live_tool(&gate, "nargo").output().expect("runs");
        assert!(out.status.success());
        crate::support::complete(gate);
    }
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
    support::complete(gate);
}
'''

_GOOD_PLAIN = '''\
mod support;

#[test]
fn plain_case() {
    let Some(token) = support::gate("plain_case", &["nargo"]) else { return; };
    assert_eq!(2 + 2, 4);
    support::complete(token);
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
    "an ungated driver route that accepts an unnamed error": (
        "evade_driver.rs",
        "use sparq_zk_compose::driver::CircuitProver;\n#[test]\nfn case() {\n"
        "    let p = CircuitProver::from_crate_root();\n"
        "    match verify(&p) {\n        Err(_) => {}\n"
        '        Ok(()) => panic!("must be rejected"),\n    }\n}\n',
    ),
    "an ungated driver route that discards the error": (
        "evade_driver_iserr.rs",
        "use sparq_zk_compose::driver::CircuitProver;\n#[test]\nfn case() {\n"
        "    let p = CircuitProver::from_crate_root();\n"
        "    assert!(verify(&p).is_err());\n}\n",
    ),
    "an ungated driver route with an accepting wildcard arm": (
        "evade_driver_wild.rs",
        "use sparq_zk_compose::driver::CircuitProver;\n#[test]\nfn case() {\n"
        "    let p = CircuitProver::from_crate_root();\n"
        "    match verify(&p) {\n        Ok(()) => panic!(\"no\"),\n"
        "        _ => {}\n    }\n}\n",
    ),
    "a branch on a spawn failure": (
        "evade_spawn_arm.rs",
        "mod support;\n#[test]\nfn case() {\n"
        '    let Some(gate) = support::gate("spawn_arm_case", &["nargo"]) else { return; };\n'
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
        '    let Some(gate) = support::gate(N, &["nargo"]) else { return; };\n}\n',
    ),
    # Two gate calls on ONE line share a call site, so the ledger cannot tell which
    # of them ran. (Two calls with the same NAME on different lines are fine now:
    # the name is a message, the location is the identity.)
    "two gate calls on one line": (
        "evade_dup.rs",
        "mod support;\n#[test]\nfn a() {\n"
        '    let _ = support::gate("x", &["nargo"]); '
        'let _ = support::gate("y", &["nargo"]);\n}\n',
    ),
    # The gate reached indirectly: renamed in a `use`, or taken as a function value.
    "the gate in a brace import": (
        "evade_brace.rs",
        "mod support;\nuse support::{gate, live_tool};\n#[test]\nfn case() {\n"
        '    let g = gate("case", &["nargo"]).unwrap();\n'
        '    let _ = live_tool(&g, "nargo").output().expect("runs");\n}\n',
    ),
    "the gate in a glob import": (
        "evade_glob.rs",
        "mod support;\nuse support::*;\n#[test]\nfn case() {\n"
        '    let _ = gate("case", &["nargo"]);\n}\n',
    ),
    "the gate module aliased": (
        "evade_modalias.rs",
        "mod support;\nuse support as tc;\n#[test]\nfn case() {\n"
        '    let _ = tc::gate("case", &["nargo"]);\n}\n',
    ),
    "the gate renamed in a use": (
        "evade_alias.rs",
        "mod support;\nuse support::gate as g;\n#[test]\nfn case() {\n"
        '    let _ = g("case", &["nargo"]);\n}\n',
    ),
    "the gate taken as a function value": (
        "evade_fnvalue.rs",
        "mod support;\n#[test]\nfn case() {\n"
        "    let f = support::gate;\n"
        '    let _ = f("case", &["nargo"]);\n}\n',
    ),
    "live_tool renamed in a use": (
        "evade_tool_alias.rs",
        "mod support;\nuse support::live_tool as lt;\n#[test]\nfn case() {\n"
        '    let g = support::gate("case", &["nargo"]).unwrap();\n'
        '    let _ = lt(&g, "nargo").output().expect("runs");\n}\n',
    ),
    # ---- statement-based shapes a line-at-a-time reader missed ----
    "an `env nargo execute` launcher": (
        "evade_env_launcher.rs",
        "#[test]\nfn case() {\n"
        '    let out = std::process::Command::new("env")\n'
        '        .args(["nargo", "execute"])\n'
        '        .output()\n        .expect("runs");\n'
        "    assert!(out.status.success());\n}\n",
    ),
    "a Command whose `::new` is on the next line": (
        "evade_split_new.rs",
        "#[test]\nfn case() {\n"
        "    let _ = std::process::Command\n"
        '        ::new("nargo")\n        .output();\n}\n',
    ),
    "a multi-line renamed gate import in a feature-gated test": (
        "evade_ml_import.rs",
        "mod support;\n"
        '#[cfg(feature = "dual-leaf")]\n#[test]\nfn case() {\n'
        "    use support::{\n        gate as g,\n    };\n"
        '    let t = g("case", &["nargo"]).unwrap();\n'
        "    support::complete(t);\n}\n",
    ),
    "a Spawn arm whose `=>` is on the next line": (
        "evade_spawn_nextline.rs",
        "mod support;\n#[test]\nfn case() {\n"
        '    let Some(gate) = support::gate("case", &["nargo"]) else { return; };\n'
        "    match run() {\n"
        "        Err(DriverError::Spawn { .. })\n            => {}\n"
        '        other => panic!("{other:?}"),\n    }\n'
        "    support::complete(gate);\n}\n",
    ),
    "a live_tool spawn whose result is unwrapped but never asserted": (
        "evade_unasserted.rs",
        "mod support;\n#[test]\nfn case() {\n"
        '    let g = match support::gate("case", &["nargo"]) {\n'
        "        Some(g) => g,\n        None => return,\n    };\n"
        '    let out = support::live_tool(&g, "nargo").output().expect("runs");\n'
        '    let other = std::fs::read("x").unwrap();\n'
        "    assert!(other.is_empty());\n"
        "    support::complete(g);\n}\n",
    ),
    "a live_tool spawn with only an absent diagnostic assertion": (
        "absent_diagnostic.rs",
        "mod support;\n#[test]\nfn case() {\n"
        '    let Some(gate) = support::gate("case", &["nargo"]) else { return; };\n'
        '    let out = support::live_tool(&gate, "nargo").output().expect("runs");\n'
        '    assert!(!String::from_utf8_lossy(&out.stderr).contains(EXPECTED));\n'
        "    support::complete(gate);\n}\n",
    ),
    "a live_tool diagnostic mentioned beside an unrelated contains call": (
        "unrelated_diagnostic.rs",
        'mod support;\n#[test]\nfn case() {\n'
        '    let Some(g) = support::gate("case", &["nargo"]) else { return; };\n'
        '    let out = support::live_tool(&g, "nargo").output().unwrap();\n'
        '    assert!(out.stderr.is_empty() && other.contains(EXPECTED));\n'
        '    support::complete(g);\n}\n',
    ),
    "a live_tool diagnostic condition made true by another clause": (
        "optional_diagnostic.rs",
        'mod support;\n#[test]\nfn case() {\n'
        '    let Some(g) = support::gate("case", &["nargo"]) else { return; };\n'
        '    let out = support::live_tool(&g, "nargo").output().unwrap();\n'
        '    assert!(out.stderr.contains(EXPECTED) || true);\n'
        '    support::complete(g);\n}\n',
    ),
    "a live_tool failure status without a diagnostic": (
        "failure_status.rs",
        'mod support;\n#[test]\nfn case() {\n'
        '    let Some(g) = support::gate("case", &["nargo"]) else { return; };\n'
        '    let out = support::live_tool(&g, "nargo").output().unwrap();\n'
        '    assert!(!out.status.success());\n'
        '    support::complete(g);\n}\n',
    ),
    "an inline spawn assertion that does not require success": (
        "inline_status.rs",
        'mod support;\n#[test]\nfn case() {\n'
        '    let Some(g) = support::gate("case", &["nargo"]) else { return; };\n'
        '    assert!(true || support::live_tool(&g, "nargo").status().unwrap().success());\n'
        '    support::complete(g);\n}\n',
    ),
    "a live_tool diagnostic assertion with an empty pattern": (
        "empty_diagnostic.rs",
        'mod support;\n#[test]\nfn case() {\n'
        '    let Some(g) = support::gate("case", &["nargo"]) else { return; };\n'
        '    let out = support::live_tool(&g, "nargo").output().unwrap();\n'
        '    assert!(String::from_utf8_lossy(&out.stderr).contains(""));\n'
        '    support::complete(g);\n}\n',
    ),
    "an ungated driver route whose error arm only prints": (
        "evade_driver_eprintln.rs",
        "use sparq_zk_compose::driver::CircuitProver;\n#[test]\nfn case() {\n"
        "    let p = CircuitProver::from_crate_root();\n"
        "    match verify(&p) {\n"
        '        Err(e) => {\n            eprintln!("rejected: {e}");\n        }\n'
        '        Ok(()) => panic!("must be rejected"),\n    }\n}\n',
    ),
    "a quoted exemption marker": (
        "evade_quoted.rs",
        "#[test]\nfn case() {\n"
        '    let s = "// zk-toolchain-gate: allow not a real comment";\n'
        '    let _ = std::process::Command::new("nargo").output();\n'
        "    let _ = s;\n}\n",
    ),
}


# The rule each evasion must be reported under (it may ALSO trip others: an
# undeclared live test is an L finding as well). A fixture that went red only for an
# incidental reason would otherwise stop testing its own rule.
EVASION_RULES: dict[str, str] = {
    "a helper wrapper outside the gate module": "A1",
    "a const executable": "A2",
    "a which probe": "A4",
    "an env-gated driver": "A2",
    "a one-line test fn": "A1",
    "a not-found soft pass": "A3",
    "a path-literal executable": "A1",
    "a windows-suffixed executable": "A1",
    "a whitespace-spelled new": "A1",
    "a shell laundering the tool name": "A1",
    "a Command alias": "A5",
    "a cmd! macro": "A6",
    "a test redirecting the ledger": "A7",
    "an ungated driver route that accepts an unnamed error": "L",
    "an ungated driver route that discards the error": "L",
    "an ungated driver route with an accepting wildcard arm": "L",
    "a branch on a spawn failure": "C2",
    "a discarded live_tool spawn result": "C3",
    "a non-literal gate name": "INV",
    "two gate calls on one line": "INV",
    "the gate in a brace import": "A8",
    "the gate in a glob import": "A8",
    "the gate module aliased": "A8",
    "the gate renamed in a use": "A8",
    "the gate taken as a function value": "A8",
    "live_tool renamed in a use": "A8",
    "an `env nargo execute` launcher": "A1",
    "a Command whose `::new` is on the next line": "A1",
    "a multi-line renamed gate import in a feature-gated test": "A8",
    "a Spawn arm whose `=>` is on the next line": "C2",
    "a live_tool spawn whose result is unwrapped but never asserted": "C3",
    "a live_tool spawn with only an absent diagnostic assertion": "C3",
    "a live_tool diagnostic mentioned beside an unrelated contains call": "C3",
    "a live_tool diagnostic condition made true by another clause": "C3",
    "a live_tool failure status without a diagnostic": "C3",
    "an inline spawn assertion that does not require success": "C3",
    "a live_tool diagnostic assertion with an empty pattern": "C3",
    "an ungated driver route whose error arm only prints": "L",
    "a quoted exemption marker": "A1",
}

# Every gate call site the clean fixture contains, keyed exactly as the gate records
# one: `<file>:<line>` of the line the call starts on. Written down rather than read
# back from the inventory, so a change to either side has to be noticed.
_FIXTURE_KEYS = [
    "crates/sparq-zk-compose/src/thing.rs:11",
    "crates/sparq-zk-compose/tests/live.rs:5",
    "crates/sparq-zk/src/thing.rs:11",
    "crates/sparq-zk/tests/plain.rs:5",
]


_DECLARED = {
    "sparq-zk": "live tests/plain.rs::plain_case\nlive src/thing.rs::src_live_case\n",
    "sparq-zk-compose": (
        "# the clean fixture's live and structural tests\n"
        "live tests/live.rs::live_case\n"
        "live src/thing.rs::src_live_case\n"
        "structural tests/structural_driver.rs::case\n"
    ),
}


def _ledger_lines(keys: list[str]) -> list[str]:
    """A complete ledger for `keys`: an entry and a completion line for each."""
    return [f"enter {k}" for k in keys] + [f"complete {k}" for k in keys]


def _write_fixture(root: Path) -> None:
    (root / WORKFLOW_REL).parent.mkdir(parents=True, exist_ok=True)
    (root / WORKFLOW_REL).write_text(_WF, encoding="utf-8")
    for pkg, rel in CARGO_TOMLS.items():
        p = root / rel
        p.parent.mkdir(parents=True, exist_ok=True)
        p.write_text(_CARGO % pkg, encoding="utf-8")
    for _pkg, rel in SCAN_DIRS:
        (root / rel).mkdir(parents=True, exist_ok=True)
    for pkg, lib_rel in LIB_ROOTS.items():
        (root / lib_rel).parent.mkdir(parents=True, exist_ok=True)
        (root / lib_rel).write_text(_SRC_LIB, encoding="utf-8")
        (root / lib_rel).parent.joinpath("thing.rs").write_text(
            _SRC_THING, encoding="utf-8")
    for helper in HELPER_MODULES:
        (root / helper).write_text(_SUPPORT, encoding="utf-8")
    for pkg, rel in DECLARED_REL.items():
        (root / rel).write_text(_DECLARED[pkg], encoding="utf-8")
    (root / "crates/sparq-zk-compose/tests/live.rs").write_text(_GOOD_LIVE, encoding="utf-8")
    (root / "crates/sparq-zk/tests/plain.rs").write_text(_GOOD_PLAIN, encoding="utf-8")
    # A STRUCTURAL driver-route suite: it builds a CircuitProver but every arm it
    # accepts names a specific variant and its wildcard arm panics, so a missing
    # tool's spawn failure fails the test. This must stay CLEAN without an
    # annotation — the shape crates/sparq-zk-compose/tests/holder_pop_forge.rs has.
    (root / "crates/sparq-zk-compose/tests/structural_driver.rs").write_text(
        "use sparq_zk_compose::driver::CircuitProver;\n#[test]\nfn case() {\n"
        "    let p = CircuitProver::from_crate_root();\n"
        "    match verify(&p) {\n"
        "        Err(CheckError::HolderKeyMismatch) => {}\n"
        '        other => panic!("wrong rejection: {other:?}"),\n    }\n}\n',
        encoding="utf-8",
    )


def _completion_and_inventory_fixtures(fresh, expect_red, expect_clean) -> None:
    """CHECK G (completion placement) and CHECK L (the declared inventory)."""
    compose = "crates/sparq-zk-compose"
    live = f"{compose}/tests/live.rs"

    def plant(root: Path, rel: str, body: str) -> None:
        (root / rel).write_text(body, encoding="utf-8")

    for label, body in (
        ("the completion call deleted",
         _GOOD_LIVE.replace("    support::complete(gate);\n", "")),
        ("the completion call moved before the assertions",
         _GOOD_LIVE.replace("    assert!(out.status.success());\n    support::complete(gate);\n",
                            "    support::complete(gate);\n    assert!(out.status.success());\n")),
        ("an early return after the gate",
         _GOOD_LIVE.replace("    assert!(out.status.success());\n",
                            "    if out.stdout.is_empty() { return; }\n"
                            "    assert!(out.status.success());\n")),
        ("a `?` exit after the gate",
         _GOOD_LIVE.replace("    let out = ", "    std::fs::metadata(\"x\")?;\n    let out = ")),
        ("a `?` exit before a closing brace",
         _GOOD_LIVE.replace("    let out = ", "    let x = { f()? };\n    let out = ")),
        ("a `?` exit before an opening brace",
         _GOOD_LIVE.replace("    let out = ", "    if f()? {}\n    let out = ")),
        ("a completion of a value that is not the gate token",
         _GOOD_LIVE.replace("    support::complete(gate);\n",
                            "    let other = make();\n    support::complete(other);\n")),
        ("a completion without a token",
         _GOOD_LIVE.replace("    support::complete(gate);\n",
                            "    support::complete();\n")),
        ("two completion calls",
         _GOOD_LIVE.replace("    support::complete(gate);\n",
                            "    support::complete(gate);\n    support::complete(gate);\n")),
    ):
        root = fresh()
        plant(root, live, body)
        expect_red(root, f"completion: {label}", "G")

    root = fresh()
    plant(root, f"{compose}/tests/live.rs", _GOOD_LIVE.replace("#[test]\n", ""))
    expect_red(root, "completion: a gate call outside a #[test] function", "G")

    root = fresh()
    plant(root, "crates/sparq-zk/tests/plain.rs",
          _GOOD_PLAIN.replace("    support::complete(token);\n", ""))
    expect_red(root, "completion: a token gate with no completion call", "G")

    for label, extra in (
        ("a helper in a file without a gate", "fn helper(token: Token) {\n    support::complete(token);\n}\n"),
        ("a completion in an initializer", "const VALUE: () = { support::complete(token); };\n"),
        ("a completion in a nested helper", "#[test] fn plain() {\n    fn helper(token: Token) {\n        support::complete(token);\n    }\n}\n"),
    ):
        root = fresh()
        plant(root, f"{compose}/tests/extra.rs", extra)
        expect_red(root, f"completion: {label}", "G")

    root = fresh()
    plant(root, live, _GOOD_LIVE.replace("#[test]", "#[test]\n#[should_panic]"))
    expect_red(root, "completion: should_panic can accept the token's Drop panic", "G")

    for label, body, rule in (
        ("an env nargo execute launcher",
         '#[test] fn launched() { let _ = Command::new("env")\n'
         '    .args(["nargo", "execute"]).output().unwrap(); }', "A1"),
        ("a split renamed gate import in a feature-gated test",
         '#[cfg(feature = "optional")]\n#[test]\nfn renamed() {\n'
         '    use crate::support::{\n        gate as g\n    };\n'
         '    let _ = g("renamed", &["nargo"]);\n}', "A8"),
        ("a pub(crate) gate import", 'pub(crate) use support::{gate as g};', "A8"),
        ("an attributed pub(in) gate import",
         '#[cfg(test)] pub(in crate::tests) use crate::support::{gate as g};', "A8"),
        ("an assertion on a different spawn value",
         _GOOD_LIVE.replace('assert!(out.status.success())',
                            'assert!(other.status.success())'), "C3"),
    ):
        root = fresh()
        plant(root, live, body)
        expect_red(root, label, rule)

    root = fresh()
    plant(root, live, _GOOD_LIVE.replace("assert!(out.status.success())",
          'assert!(String::from_utf8_lossy(&out.stderr).contains("expected constraint"))'))
    expect_clean(root, "a spawn's own failure diagnostic is asserted")

    # CHECK L, both directions.
    root = fresh()
    plant(root, f"{compose}/tests/extra.rs",
          _GOOD_LIVE.replace("live_case", "extra_case"))
    expect_red(root, "inventory: an undeclared live test", "L")
    root = fresh()
    decl = root / DECLARED_REL["sparq-zk-compose"]
    decl.write_text(_DECLARED["sparq-zk-compose"].replace(
        "live tests/live.rs::live_case\n", ""), encoding="utf-8")
    expect_red(root, "inventory: a declared-list entry removed", "L")
    root = fresh()
    decl = root / DECLARED_REL["sparq-zk-compose"]
    decl.write_text(_DECLARED["sparq-zk-compose"] + "live tests/gone.rs::ghost\n",
                    encoding="utf-8")
    expect_red(root, "inventory: a declared test the scanner does not find", "L")
    root = fresh()
    decl = root / DECLARED_REL["sparq-zk-compose"]
    decl.write_text(_DECLARED["sparq-zk-compose"].replace(
        "structural tests/structural_driver.rs::case\n", ""), encoding="utf-8")
    expect_red(root, "inventory: an undeclared ungated driver-route test", "L")
    root = fresh()
    decl = root / DECLARED_REL["sparq-zk-compose"]
    decl.write_text(_DECLARED["sparq-zk-compose"].replace(
        "live tests/live.rs::live_case", "structural tests/live.rs::live_case"),
        encoding="utf-8")
    expect_red(root, "inventory: a gated test declared structural", "L")
    root = fresh()
    decl = root / DECLARED_REL["sparq-zk-compose"]
    decl.write_text(_DECLARED["sparq-zk-compose"] + "maybe tests/live.rs::live_case\n",
                    encoding="utf-8")
    expect_red(root, "inventory: a malformed declaration", "L")
    root = fresh()
    (root / DECLARED_REL["sparq-zk"]).unlink()
    expect_red(root, "inventory: a missing declared list", "L")

    # A driver-route test DECLARED structural still may not accept an unnamed error.
    root = fresh()
    plant(root, f"{compose}/tests/structural_driver.rs",
          EVASIONS["an ungated driver route whose error arm only prints"][1])
    expect_red(root, "structural: an `Err(e) => {{ eprintln!(..) }}` arm", "C1")
    root = fresh()
    plant(root, f"{compose}/tests/structural_driver.rs",
          EVASIONS["an ungated driver route that accepts an unnamed error"][1])
    expect_red(root, "structural: an `Err(_) => {{}}` arm", "C1")
    # A driver route through a launcher helper is discovered too.
    root = fresh()
    plant(root, f"{compose}/tests/launcher_helper.rs",
          "fn run() -> std::process::Output {\n"
          '    // zk-toolchain-gate: allow fixture: the launcher is the subject\n'
          '    std::process::Command::new("env").arg("nargo").output().unwrap()\n}\n'
          "#[test]\nfn case() {\n    assert!(run().status.success());\n}\n")
    expect_red(root, "inventory: a test reaching nargo through a launcher helper", "L")


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
        expect_red(root, f"evasion: {label}", EVASION_RULES[label])

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

    # An annotation is scoped to ONE line (plus the comment block above it): a code
    # line between the annotation and the finding ends its reach.
    root = fresh()
    ALLOW_RATCHET = {"crates/sparq-zk-compose/tests/far_exempt.rs": 1}
    (root / "crates/sparq-zk-compose/tests/far_exempt.rs").write_text(
        "#[test]\nfn case() {\n"
        "    // zk-toolchain-gate: allow a stated reason\n"
        "    let n = 1;\n"
        '    let _ = std::process::Command::new("nargo").arg(n.to_string()).output();\n}\n',
        encoding="utf-8",
    )
    expect_red(root, "an annotation one code line away from the finding", "A1")
    # ... and a WRAPPED reason still covers the line right below it.
    root = fresh()
    ALLOW_RATCHET = {"crates/sparq-zk-compose/tests/wrapped_exempt.rs": 1}
    (root / "crates/sparq-zk-compose/tests/wrapped_exempt.rs").write_text(
        'const T: &str = "cargo";\n#[test]\nfn case() {\n'
        "    // zk-toolchain-gate: allow a reason long enough to need\n"
        "    //   a second comment line to finish the sentence\n"
        "    let _ = std::process::Command::new(T).output();\n}\n",
        encoding="utf-8",
    )
    expect_clean(root, "an annotation whose reason wraps")
    ALLOW_RATCHET = {}

    # `src` test regions: linted like a test file, while production code in the same
    # file is not. The clean fixture's `thing.rs` is the negative control.
    root = fresh()
    (root / "crates/sparq-zk-compose/src/thing.rs").write_text(
        _SRC_THING.replace(
            'crate::support::live_tool(&gate, "nargo")',
            'std::process::Command::new("nargo")'),
        encoding="utf-8",
    )
    expect_red(root, "a src test region spawning a tool directly", "A1")
    root = fresh()
    (root / "crates/sparq-zk-compose/src/lib.rs").write_text(
        "pub mod thing;\n", encoding="utf-8")
    expect_red(root, "a src gate call with no shared-gate declaration", "F")
    root = fresh()
    (root / "crates/sparq-zk/src/thing.rs").write_text(
        '#[cfg(any(test, feature = "x"))]\nmod tests {\n    #[test]\n'
        "    fn case() {}\n}\n", encoding="utf-8")
    expect_red(root, "a cfg shape the region reader does not model", "SCAN")
    root = fresh()
    (root / "crates/sparq-zk/src/thing.rs").write_text(
        "#[cfg(test)]\nmod ghost;\n", encoding="utf-8")
    expect_red(root, "a src test module naming no file", "SCAN")
    root = fresh()
    (root / "crates/sparq-zk/src/thing.rs").write_text(
        '#[cfg(test)]\n#[path = "nowhere/support.rs"]\nmod support;\n',
        encoding="utf-8")
    expect_red(root, "a src test module whose #[path] resolves to nothing", "SCAN")
    root = fresh()
    (root / "crates/sparq-zk/src/thing.rs").write_text(
        "#[cfg(test)]\nmod tests {\n    #[test]\n    fn case() {\n"
        '        let _ = std::env::var("SPARQ_ZK_LIVE_LEDGER");\n    }\n}\n',
        encoding="utf-8")
    expect_red(root, "a src test reading the ledger variable", "A7")

    # A gate call at FILE level, outside any `#[cfg(test)]` region: the lint scope
    # would not see it, the inventory reads the whole file and does.
    root = fresh()
    (root / "crates/sparq-zk-compose/src/stray.rs").write_text(
        "fn helper() {\n"
        '    let _ = crate::support::gate("stray_site", &["nargo"]);\n}\n',
        encoding="utf-8")
    sites, inv = inventory(collect(root))
    if not any(s.key == "crates/sparq-zk-compose/src/stray.rs:2" for s in sites):
        failures.append(
            "a gate call outside a cfg(test) region must still be inventoried, got: "
            + "; ".join(s.key for s in sites)
        )
    reds += 1
    del inv

    # A call through a path that is neither `support::` nor bare. A8 catches the
    # `use` that would create such an alias, but only if the `use` is in a file the
    # lint reads; the inventory over-matches any path prefix so the SITE is seen
    # either way, and a key nothing records is then reported by --verify-ledger.
    root = fresh()
    (root / "crates/sparq-zk-compose/tests/aliased_call.rs").write_text(
        "#[test]\nfn case() {\n"
        '    let _ = tc::gate("aliased_call", &["nargo"]);\n}\n',
        encoding="utf-8",
    )
    sites, _ = inventory(collect(root))
    if not any(
        s.key == "crates/sparq-zk-compose/tests/aliased_call.rs:3" for s in sites
    ):
        failures.append(
            "a gate call through an aliased path must still be inventoried, got: "
            + "; ".join(s.key for s in sites)
        )
    reds += 1

    # A child `mod` of a `#[cfg(test)] mod NAME;` FILE is test code too, with no
    # cfg attribute of its own to find it by.
    root = fresh()
    (root / "crates/sparq-zk-compose/src/thing.rs").write_text(
        "#[cfg(test)]\nmod inner;\n", encoding="utf-8")
    inner = root / "crates/sparq-zk-compose/src/thing"
    (inner / "inner").mkdir(parents=True, exist_ok=True)
    (inner / "inner.rs").write_text("mod deeper;\n", encoding="utf-8")
    (inner / "inner" / "deeper.rs").write_text(
        "#[test]\nfn case() {\n"
        '    let _ = std::process::Command::new("nargo").output();\n}\n',
        encoding="utf-8")
    expect_red(root, "a grandchild module of a src test module", "A1")

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
        'fn f() { let _ = support::gate("orphan", &["nargo"]); }\n', encoding="utf-8"
    )
    expect_red(root, "a gate call in a file that is not a target", "INV")

    root = fresh()
    sub = root / "crates/sparq-zk-compose/tests/subdir_target"
    sub.mkdir()
    (sub / "main.rs").write_text(
        'mod nothing;\nfn main() {}\n#[test]\nfn sub_case() {\n'
        '    let Some(token) = support::gate("sub_case", &["nargo"]) else { return; };\n'
        '    assert_eq!(2 + 2, 4);\n    support::complete(token);\n}\n',
        encoding="utf-8",
    )
    (sub / "nothing.rs").write_text("// module of the subdir target\n", encoding="utf-8")
    declared = root / DECLARED_REL["sparq-zk-compose"]
    declared.write_text(declared.read_text() + "live tests/subdir_target/main.rs::sub_case\n")
    expect_ledger(
        root,
        _ledger_lines(
            _FIXTURE_KEYS + ["crates/sparq-zk-compose/tests/subdir_target/main.rs:5"]),
        "a tests/<name>/main.rs target is inventoried",
        0,
    )

    _completion_and_inventory_fixtures(fresh, expect_red, expect_clean)

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
        ("a shell that swallows the exit code",
         (f"      - name: {VERIFY_STEP}\n        run:",
          f"      - name: {VERIFY_STEP}\n        shell: \"true {{0}}\"\n        run:")),
        ("a timeout on it",
         (f"      - name: {VERIFY_STEP}\n        run:",
          f"      - name: {VERIFY_STEP}\n        timeout-minutes: 1\n        run:")),
        ("job-level defaults",
         ("    runs-on: ubuntu-latest",
          "    defaults:\n      run:\n        shell: \"true {0}\"\n"
          "    runs-on: ubuntu-latest")),
        ("workflow-level defaults",
         ("jobs:\n", "defaults:\n  run:\n    working-directory: crates\njobs:\n")),
        ("a non-bash shell on another step of the job",
         ("      - name: Detect zk changes\n        id: changes\n",
          "      - name: Detect zk changes\n        id: changes\n        shell: pwsh\n")),
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
    expect_ledger(root, _ledger_lines(_FIXTURE_KEYS), "a complete ledger", 0)
    expect_ledger(root, _ledger_lines(_FIXTURE_KEYS[:-1]), "a missing call site", 1)
    expect_ledger(root, [], "an empty ledger", 1)
    expect_ledger(root, None, "a missing ledger file", 1)
    expect_ledger(
        root,
        _ledger_lines(_FIXTURE_KEYS + ["crates/sparq-zk/tests/plain.rs:999"]),
        "an entry matching no inventoried call site",
        1,
    )
    # The completion half: a test that returned early after its gate (or failed an
    # assertion) leaves its `enter` line and no `complete` line.
    expect_ledger(
        root,
        _ledger_lines(_FIXTURE_KEYS)[:-1],
        "an entry with no completion (an early return after the gate)",
        1,
    )
    expect_ledger(
        root,
        [f"enter {k}" for k in _FIXTURE_KEYS],
        "entries only, as the entry-only ledger used to be",
        1,
    )
    expect_ledger(
        root,
        _ledger_lines(_FIXTURE_KEYS)[1:],
        "a completion with no entry",
        1,
    )
    expect_ledger(
        root,
        _ledger_lines(_FIXTURE_KEYS) + [_FIXTURE_KEYS[0]],
        "a line in the old bare-key format",
        1,
    )
    root = fresh()
    (root / "crates/sparq-zk-compose/tests/evade_dup.rs").write_text(
        EVASIONS["two gate calls on one line"][1], encoding="utf-8"
    )
    expect_ledger(root, _ledger_lines(_FIXTURE_KEYS), "two gate calls on one line", 1)

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
          "inventory, completion placement, the declared inventory, the annotation "
          "ratchet, helper drift, the scan trees, the verify-step shape, and ledger "
          "verification of entries and completions).")
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
    root = Path(args.root).resolve() if args.root else Path(__file__).resolve().parent.parent
    if args.verify_ledger:
        return verify_ledger(root, Path(args.verify_ledger))
    return report_check(root)


if __name__ == "__main__":
    sys.exit(main())
