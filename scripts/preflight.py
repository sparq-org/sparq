#!/usr/bin/env python3
# [OPUS-5] Pre-submit runner — shift the mechanical merge-gates LEFT, into the
# worker's own worktree, BEFORE the PR exists (bead sq-presubmit, see the
# "first-pass review yield" analysis in the PR body).
#
# WHY THIS EXISTS — measured, not assumed.
# ----------------------------------------
# A census of the 831 review-verdict records on the registry `ledger` branch
# (orchestration/review-verdicts/<owner>--<repo>--pr<N>-round<K>.json) found 378
# round-1 reviews, of which 211 returned `request_changes`. Hand-classifying every
# one of the 317 blocking (major/blocker) findings those 211 reviews raised showed
# that roughly HALF are real defects a reviewer earned (algorithm/spec/crypto bugs,
# races, credential exposure) and roughly half are AUTHORING-TIME preventable:
#
#     CORRECTNESS       82 issues   (real bug — not preventable here)
#     CLAIM-vs-CODE     67 issues   (prose in the diff asserts what the diff's own
#                                    code does not do — partly preventable)
#     GUARD-NOT-PINNED  63 issues   (the guard/test the PR adds cannot detect the
#                                    case it exists for — preventable)
#     MALFORMED-INPUT   27 issues
#     REPO-CONTRACT     20 issues   (violates a STATED, ALREADY-SCRIPTED repo rule)
#     SECURITY          19 issues
#     ERROR-SWALLOW     11 issues
#     CI-NOT-EXERCISED  11 issues
#     SCOPE-VIOLATION    9 issues
#     RACE               8 issues
#
# The REPO-CONTRACT column is the scandal: every one of those 20 findings is a rule
# the repository ALREADY OWNS A SCRIPT FOR (G1 gate-new-crate.py, G2
# gate-api-skill.py, G6 check-config-documented.py, check-no-perf-numbers.py,
# check-readme-template.py, check-privacy-claims.sh). Two sampled examples — sparq
# #4262 ("G1 new-crate-completeness gate fails: all three new crates ship without a
# README.md") and #3822 ("Hard G2 public-api→skill gate fails") — had the
# `flow-on-gates quick-gates (G1 + G2 + G6)` check-run ALREADY RED at the exact head
# SHA the reviewer bound their verdict to. A whole review round was spent restating
# a machine result.
#
# The root cause is an information asymmetry, not a missing check: NONE of the
# worker briefs (.claude/agents/sparq-rust-impl.md, sparq-rust-feature.md,
# sparq-ci-infra.md, sparq-docs.md, sparq-site.md) name `gate-new-crate.py` or
# `gate-api-skill.py` at all. The author is graded on a checklist they were never
# handed. This script IS that checklist, executable.
#
# WHAT IT DOES AND DOES NOT CLAIM
# -------------------------------
# MECHANICAL checks (this script decides them; a failure exits non-zero):
#   G1/G2/G6, no-perf-numbers, readme-template, privacy-claims  — delegated to the
#   existing gate scripts, scoped to YOUR diff, so they fire in-worktree instead of
#   on CI (and instead of in a review round).
#   guard-untested — NEW here: a guard-shaped PUBLIC symbol added by the diff that
#   is named nowhere in the TEST TEXT of the tree. "Test text" is defined precisely
#   at `searchable_test_text` below, and it deliberately EXCLUDES the symbol's own
#   definition and every ordinary call site — including ones in the same file as a
#   test module. This is the STATICALLY decidable slice of GUARD-NOT-PINNED.
#   MEASURED on the current tree: 39 of 191 guard-shaped surface symbols are named
#   by no test text. The check is NOT wired into CI; it runs when an author runs it.
#
#   ITS PRECISION IS ~74%, NOT ~100% — stated because an earlier version of this
#   header claimed one false positive and that was wrong (see TEST_PATH_HINTS).
#   Re-measured by deleting each firing symbol and running its script's own gating
#   `--self-test`: 10 of the 31 Python firings red, i.e. they ARE pinned. All 10 are
#   INDIRECTLY reached — the self-test enters at the top and the guard is named only
#   OUTSIDE any test region. Enclosing scope of every reference, counted:
#     7  an intermediate top-level function (`verify_evidence`, `check_workflows`,
#        `check_tree`, `resolve_newest_workflow_runs`, `enrich_prs`, `apply_labels`,
#        `run_verify`);
#     3  a module-level dispatch table (`"doc-anchor": verify_doc_anchor`, …).
#   That residue is not fixed here on purpose: closing it needs a call-graph
#   over-approximation ("anything reachable from a test region counts"), which
#   would make a guard reached only via `main()` read as tested. That is the
#   fail-open direction, and this check exists to catch fail-open. A firing is
#   therefore an OBLIGATION TO LOOK, not a proof of absence. The 39 current
#   firings are filed as bead sq-sz0wh, which carries the per-symbol triage.
#
# NOT MECHANICAL, and this script does not pretend otherwise: the other slice of
# GUARD-NOT-PINNED (a test EXISTS but cannot discriminate — it asserts a bound, a
# type, or a marker string rather than the behaviour) is only decidable by MUTATING
# the guard and re-running. Likewise CLAIM-vs-CODE needs a human/model to read the
# prose against the code. For those two, this script prints the obligation and
# requires the author to have executed it; it cannot and must not "pass" them.
#
# This script LOWERS NO BAR. Every mechanical check it runs is a gate that already
# blocks the merge; running it earlier only changes WHERE the author learns.
#
# SUPPRESSION: an inline `preflight-allow: <check> — <why>` marker on (or directly
# above) the offending line, mirroring the repo's existing `privacy-claims-allow:`
# convention. A suppression without a `— why` clause does NOT suppress.
#
# DIFF SOURCE: `git diff --name-status origin/<base>...HEAD` plus `git diff -U0`
# for added lines; or a hermetic fixture (--changed-files / --added-lines) in tests.

from __future__ import annotations

import argparse
import ast
import os
import re
import subprocess
import sys
from dataclasses import dataclass, field
from pathlib import Path, PurePosixPath

REPO_ROOT = Path(__file__).resolve().parent.parent

# ---------------------------------------------------------------------------
# guard-untested: the statically decidable slice of GUARD-NOT-PINNED.
# ---------------------------------------------------------------------------
# Narrow ON PURPOSE. Only a guard-shaped symbol on a real SURFACE is flagged:
#   Rust   — `pub fn` / `pub(crate) fn` in crates/*/src/**.rs
#   Python — a module-level `def` (column 0) in scripts/*.py
# A private Rust `fn` or a nested Python `def` is NOT flagged: those are reached
# through their caller and the coverage ratchet already speaks to them. The point
# of this check is the surface a reviewer will ask "what test reds if you delete
# this?" about.
GUARD_STEMS = (
    "validate",
    "verify",
    "check",
    "assert",
    "require",
    "reject",
    "ensure",
    "forbid",
    "refuse",
    "guard",
)
# `fn validate_x` / `fn check_x` / `fn x_guard` / `fn is_x_valid` / `fn x_is_allowed`
_STEM = "|".join(GUARD_STEMS)
RUST_GUARD_RE = re.compile(
    r"^\s*pub(?:\s*\([^)]*\))?\s+(?:async\s+)?(?:const\s+|unsafe\s+|extern\s+\"[^\"]*\"\s+)*"
    rf"fn\s+(?P<name>(?:{_STEM})\w*|\w*_(?:{_STEM})\w*|is_\w+_(?:valid|safe|allowed|trusted|pinned))\s*[(<]"
)
PY_GUARD_RE = re.compile(
    rf"^def\s+(?P<name>_?(?:{_STEM})\w*|_?\w*_(?:{_STEM})\w*|_?is_\w+_(?:valid|safe|allowed|trusted|pinned))\s*\("
)

RUST_SRC_RE = re.compile(r"^crates/[^/]+/src/.*\.rs$")
PY_SCRIPT_RE = re.compile(r"^scripts/[^/]*\.py$")

# Whole-file test hosts, decided from the PATH alone. Each is a target that a
# test runner BUILDS, so deleting an item it names breaks that build:
# `crates/*/tests` + `scripts/tests` (run outright), `benches` and `examples`
# (built by `cargo test`), `fuzz` (built by `cargo fuzz`).
#
# CORRECTION — the sentence that used to stand here was false, and it was
# load-bearing (it was the stated evidence that this tuple is COMPLETE), so it
# is quoted rather than quietly deleted:
#
#   "`/fuzz/` is here on MEASURED evidence: it was the ONE false positive among
#    the 39 distinct symbols this check reports on the current tree"
#
# "the ONE false positive" was not true, and the tuple was not complete.
# Re-measured by deleting each Python firing and running that script's own gating
# `--self-test`: 10 of 31 red. Two more were Rust: `sparq-mpc::check_bounded_path`
# (14 `#[test]` fns in `hidden_path/planner/tests.rs`) and
# `sparq-shacl::validate_with_model` (called by two `crates/sparq-shacl/examples/*.rs`).
#
# Only ONE of those three mechanisms was a missing path hint (`/examples/`, added
# here). The other two were holes in the resolver itself and are fixed as such —
# `#[cfg(test)] mod X;` resolution below, and `python_test_regions` reading the
# real parse tree. A fifth substring would not have reached either of them.
#
# AND THE `/fuzz/` HALF NAMED THE WRONG SYMBOL. `eval::validate_graph` is
# `pub(crate)`; `fuzz/` is a separate cargo workspace and cannot reference a
# crate-private item at all, so it CANNOT "stop compiling if the function is
# deleted". It appears in `validate_shacl.rs` only in a header COMMENT. The hint
# still earns its place — but through a different symbol. Counted over all 14
# `fuzz/**.rs` and all 148 guard-shaped names, with `mask_rust` separating code
# from prose:
#
#     validate        code=True    <- a real call; THIS is what /fuzz/ pins
#     validate_graph  code=False   } named only in comment/string text
#     guard           code=False   }
#     required        code=False   }
#
# KNOWN LIMITATION, measured and not fixed here. A whole-file test host is
# searched INCLUDING its comments and string literals, so a prose mention
# satisfies a guard — and 3 of those 4 are exactly that. Short, common names
# (`guard`, `required`) are the worst case: they occur in ordinary English and
# are therefore near-permanently silenced. The fix (mask comments in whole-file
# hosts) is strictly fail-CLOSED — it can only ADD findings — but it moves the
# census, and shipping a precision figure I had not re-derived is the mistake
# this round exists to correct. Filed on bead sq-sz0wh instead.
TEST_PATH_HINTS = (
    "/tests/",
    "scripts/tests/",
    "/benches/",
    "/fuzz/",
    "/examples/",
)
TEST_NAME_RE = re.compile(r"(^|/)(test_[^/]*\.py|[^/]*_test\.py|[^/]*\.rs)$")

# The separator MUST be surrounded by whitespace. Without that, `[a-z0-9-]+`
# backtracks and eats its own hyphen: `preflight-allow: guard-untested —` parsed as
# check=`guard`, why=`untested —`, i.e. a marker with NO reason silently produced a
# well-formed match for a check name nobody wrote. Found by mutation M3 (the
# `why.strip()` conjunct survived deletion because it was unreachable — the test
# that "proved" a missing reason does not suppress was actually passing on the
# accidental check-name mismatch). Requiring `\s+[-—]\s+` makes the reason
# genuinely mandatory and makes a hyphenated check name parse whole.
SUPPRESS_RE = re.compile(r"preflight-allow:\s*(?P<check>[a-z0-9][a-z0-9-]*)\s+[-—]\s+(?P<why>\S.*)")


def suppressed(lines: list[str], idx: int, check: str) -> bool:
    """A `preflight-allow: <check> — <why>` marker on the line or the one above it.

    A marker with no `— why` clause does NOT suppress: the whole value of the
    escape hatch is the recorded reason. That is enforced by SUPPRESS_RE itself
    (the `\\S.*` reason group is not optional), so there is deliberately NO
    second `why` test here — a redundant conjunct would be unreachable code
    masquerading as a guard.
    """
    for probe in (idx, idx - 1):
        if probe < 0 or probe >= len(lines):
            continue
        m = SUPPRESS_RE.search(lines[probe])
        if m and m.group("check") == check:
            return True
    return False


@dataclass
class Finding:
    check: str
    path: str
    detail: str
    fix: str


@dataclass
class Result:
    findings: list[Finding] = field(default_factory=list)
    ran: list[str] = field(default_factory=list)
    skipped: list[str] = field(default_factory=list)

    @property
    def ok(self) -> bool:
        return not self.findings


def added_symbols(added_lines: dict[str, list[str]]) -> list[tuple[str, str]]:
    """(path, symbol) for every guard-shaped surface symbol ADDED by the diff."""
    out: list[tuple[str, str]] = []
    for path, lines in sorted(added_lines.items()):
        if RUST_SRC_RE.match(path):
            rx = RUST_GUARD_RE
        elif PY_SCRIPT_RE.match(path):
            rx = PY_GUARD_RE
        else:
            continue
        for i, line in enumerate(lines):
            m = rx.match(line)
            if not m:
                continue
            if suppressed(lines, i, "guard-untested"):
                continue
            out.append((path, m.group("name")))
    return out


# ---------------------------------------------------------------------------
# What text counts as A TEST.  [OPUS-5]
# ---------------------------------------------------------------------------
# BUG THIS REPLACES (found in review; the checker committed the very defect it
# exists to prevent). The first version admitted a whole file on a FILE-LEVEL
# marker — `#[cfg(test)]` anywhere in a Rust src file, `--self-test` anywhere in
# a `scripts/*.py` — and then word-searched that file's ENTIRE text. A file
# contains its own `pub fn` / `def` definition line, so **a symbol's own defining
# file was accepted as that symbol's test**: an EMPTY `#[cfg(test)] mod tests {}`
# made every guard in the file permanently invisible. MEASURED on this tree:
# 48 of 191 guard-shaped surface symbols could never be reported.
#
# The fix is to scope the search to the file's TEST REGIONS, so the definition
# span — and every ordinary call site in the same file — is excluded BY
# CONSTRUCTION rather than by a subtraction that could be got wrong:
#
#   Rust src : the body of each `#[cfg(<pred implying test>)]` item, plus the
#              contents of ``` fenced blocks in `///` / `//!` doc comments
#              (a doctest is run by `cargo test` and reds if the fn is deleted).
#   scripts/ : the body of each top-level `def *self_test*` / `def test_*` and of
#              each top-level `class *Test*` — the repo's dominant convention
#              (check-terminology.py, release-interval-guard.py::self_test,
#              check-vectorized-feature-off.py::run_self_test, …).
#   tests/ , benches/ : the whole file. It is a test file end to end.
#
# `#[cfg(not(test))]` and `#[cfg(any(not(feature = "x"), test))]` are NOT test
# regions: they are compiled in ordinary builds. The predicate must IMPLY test.

_RUST_RAW_STR_RE = re.compile(r"r(#*)\"")
_RUST_CHAR_RE = re.compile(r"'(?:\\u\{[0-9a-fA-F_]+\}|\\.|[^\\'\n])'")
_IDENT_CH = re.compile(r"\w")


def mask_rust(text: str) -> str:
    """Same-length copy of `text` with comment / string / char CONTENT blanked.

    Offsets are preserved so a span found in the mask slices the ORIGINAL text.
    Blanking first is what makes brace matching safe: a `"{"` in a string literal
    or a `// {` in a comment must not open a block.
    """
    out = list(text)
    i, n = 0, len(text)

    def blank(a: int, b: int) -> None:
        for k in range(a, min(b, n)):
            if out[k] != "\n":
                out[k] = " "

    while i < n:
        c = text[i]
        if c == "/" and text.startswith("//", i):
            j = text.find("\n", i)
            j = n if j < 0 else j
            blank(i, j)
            i = j
        elif c == "/" and text.startswith("/*", i):
            depth, j = 1, i + 2
            while j < n and depth:
                if text.startswith("/*", j):
                    depth += 1
                    j += 2
                elif text.startswith("*/", j):
                    depth -= 1
                    j += 2
                else:
                    j += 1
            blank(i, j)
            i = j
        elif c == "r" and (i == 0 or not _IDENT_CH.match(text[i - 1]) or text[i - 1] == "b"):
            m = _RUST_RAW_STR_RE.match(text, i)
            if not m:
                i += 1
                continue
            close = '"' + m.group(1)
            j = text.find(close, m.end())
            j = n if j < 0 else j + len(close)
            blank(i, j)
            i = j
        elif c == '"':
            j = i + 1
            while j < n:
                if text[j] == "\\":
                    j += 2
                    continue
                if text[j] == '"':
                    j += 1
                    break
                j += 1
            blank(i, j)
            i = j
        elif c == "'":
            m = _RUST_CHAR_RE.match(text, i)
            if m:  # a char literal; a lifetime `'a` is left alone
                blank(i, m.end())
                i = m.end()
            else:
                i += 1
        else:
            i += 1
    return "".join(out)


def _cfg_implies_test(pred: str) -> bool:
    """Does this `cfg(...)` predicate hold ONLY when `test` is set?

    `test` -> yes. `all(A, B)` -> yes if ANY conjunct implies test.
    `any(A, B)` -> yes only if EVERY disjunct implies test. `not(..)` -> no.
    Anything else (a plain `feature = "x"`) -> no.
    """
    pred = pred.strip()
    if pred == "test":
        return True
    m = re.match(r"(all|any|not)\s*\((.*)\)\s*$", pred, re.S)
    if not m:
        return False
    op, inner = m.group(1), m.group(2)
    if op == "not":
        return False
    parts, depth, cur = [], 0, []
    for ch in inner:
        if ch == "," and depth == 0:
            parts.append("".join(cur))
            cur = []
            continue
        depth += (ch == "(") - (ch == ")")
        cur.append(ch)
    parts.append("".join(cur))
    parts = [p for p in parts if p.strip()]
    if not parts:
        return False
    return (any if op == "all" else all)(_cfg_implies_test(p) for p in parts)


_CFG_ATTR_RE = re.compile(r"#\s*\[\s*cfg\s*\(")


def _cfg_test_attr_ends(masked: str) -> list[int]:
    """Offset just past the closing `]` of every `#[cfg(<implies test>)]`.

    One place decides "is this attribute a test gate", so the two consumers below
    — the braced-body scan and the `mod X;` declaration scan — can never drift
    apart on the predicate.
    """
    ends: list[int] = []
    n = len(masked)
    for m in _CFG_ATTR_RE.finditer(masked):
        depth, j = 1, m.end()
        while j < n and depth:
            depth += (masked[j] == "(") - (masked[j] == ")")
            j += 1
        if depth:
            continue
        if not _cfg_implies_test(masked[m.end(): j - 1]):
            continue
        k = j
        while k < n and masked[k].isspace():
            k += 1
        if k >= n or masked[k] != "]":
            continue
        ends.append(k + 1)
    return ends


def rust_cfg_test_spans(masked: str) -> list[tuple[int, int]]:
    """(start, end) of every `#[cfg(<implies test>)] <item> { .. }` BODY."""
    spans: list[tuple[int, int]] = []
    n = len(masked)
    for k in _cfg_test_attr_ends(masked):
        while k < n:                       # walk to the item's body, or give up
            if masked[k] == ";":           # `#[cfg(test)] use foo;` — no body
                break
            if masked[k] == "{":
                depth, e = 1, k + 1
                while e < n and depth:
                    depth += (masked[e] == "{") - (masked[e] == "}")
                    e += 1
                # [OPUS-5] An UNBALANCED walk is not a test region. The only way to
                # reach EOF with the block still open is malformed input — an
                # unterminated string literal blanks the rest of the file, so the
                # closing `}` disappears and the span would run to EOF, dragging
                # every later definition into the "test text" and silencing every
                # guard below it. Fail CLOSED: no span. Verified a no-op on the real
                # tree (630 Rust src files, 0 unbalanced spans) — it can only fire on
                # Rust that does not compile.
                if depth == 0:
                    spans.append((k, e))
                break
            k += 1
    return spans


# ---------------------------------------------------------------------------
# `#[cfg(test)] mod X;` — the test body lives in ANOTHER FILE.  [OPUS-5]
# ---------------------------------------------------------------------------
# Round-3 blocking finding. A bodyless `mod X;` under a test cfg is a MINORITY
# form here — counted: 611 `crates/*/tests/*.rs` integration files, 421 src files
# with an in-file `#[cfg(test)]`, and 14 of these — but the resolver could not see
# it at all, so all 14 contributed nothing while the other two forms worked. The
# declaring attribute is in the PARENT file, so `X.rs` itself contains no
# `#[cfg(test)]`. Measured: 14 such files, 14 of 14 invisible, which is what made
# `sparq-mpc::check_bounded_path` — called by 14 `#[test]` fns in
# `hidden_path/planner/tests.rs` — read as "named by no test".
#
# `TEST_PATH_HINTS` could not fix this: it matches the DIRECTORY `/tests/`, and
# these files are `planner/tests.rs`, `adversarial_tests.rs`, `ttl/tests.rs` …
# (The review counted 13; the 14th is `sparq-engine/src/cs_gate.rs`, declared under
# `all(test, feature = "cs-planner")` rather than a bare `#[cfg(test)]` — which is
# why the seed shares `_cfg_test_attr_ends` with the span scan instead of matching
# the literal attribute text.)
# Adding a fifth path substring would have been the third patch to the same
# defect CLASS (test text that exists but is not attributed to the guard). So
# this resolves the declaration the way rustc does instead: `mod X;` in `p.rs`
# means `p/X.rs` or `p/X/mod.rs`, and in `lib.rs`/`main.rs`/`mod.rs` it means the
# sibling `X.rs` or `X/mod.rs`.
_MOD_DECL_RE = re.compile(r"\s*(?:pub(?:\s*\([^)]*\))?\s+)?mod\s+(?P<name>\w+)\s*;")
_ANY_MOD_DECL_RE = re.compile(
    r"^[^\S\n]*(?:pub(?:\s*\([^)]*\))?\s+)?mod\s+(\w+)\s*;", re.M
)


def rust_cfg_test_mod_decls(masked: str) -> list[str]:
    """Names declared by `#[cfg(<implies test>)] (pub )?mod NAME;` — bodyless."""
    names: list[str] = []
    for k in _cfg_test_attr_ends(masked):
        m = _MOD_DECL_RE.match(masked, k)
        if m:
            names.append(m.group("name"))
    return names


def rust_mod_decls(masked: str) -> list[str]:
    """Every bodyless `(pub )?mod NAME;`, cfg-gated or not."""
    return _ANY_MOD_DECL_RE.findall(masked)


def rust_module_files(rel: str, name: str) -> list[str]:
    """The paths rustc would look in for the body of `mod <name>;` declared in `rel`."""
    p = PurePosixPath(rel)
    base = p.parent if p.stem in ("lib", "main", "mod") else p.parent / p.stem
    return [str(base / f"{name}.rs"), str(base / name / "mod.rs")]


def rust_test_module_files(masks: dict[str, str]) -> set[str]:
    """Rust src files whose ENTIRE text is test-only.

    Seeded from `#[cfg(test)] mod X;` declarations and then closed transitively:
    a module declared BY a test-only module is itself compiled only under test,
    cfg attribute or not, so `X/mod.rs` declaring `mod helpers;` makes
    `X/helpers.rs` test text too. Only paths that actually exist in `masks` (i.e.
    tracked Rust src files) are admitted, so a `#[path = "…"]` override or a
    module living outside `crates/*/src` simply does not resolve.
    """
    out: set[str] = set()
    work: list[str] = []
    for rel in sorted(masks):
        for name in rust_cfg_test_mod_decls(masks[rel]):
            for cand in rust_module_files(rel, name):
                if cand in masks and cand not in out:
                    out.add(cand)
                    work.append(cand)
    while work:
        rel = work.pop()
        for name in rust_mod_decls(masks[rel]):
            for cand in rust_module_files(rel, name):
                if cand in masks and cand not in out:
                    out.add(cand)
                    work.append(cand)
    return out


_DOC_LINE_RE = re.compile(r"^\s*//[/!]")


def rust_doctest_text(text: str) -> str:
    """Contents of ``` fenced blocks inside `///` / `//!` doc comments.

    A doctest is compiled and run by `cargo test`, and stops compiling when the
    item it calls is deleted — so it genuinely pins the symbol. Prose in the doc
    comment does NOT count; only the fenced code.
    """
    out, in_fence = [], False
    for line in text.splitlines():
        if not _DOC_LINE_RE.match(line):
            in_fence = False
            continue
        body = re.sub(r"^\s*//[/!]\s?", "", line)
        if body.lstrip().startswith("```"):
            in_fence = not in_fence
            continue
        if in_fence:
            out.append(body)
    return "\n".join(out)


PY_TEST_DEF_NAME_RE = re.compile(r"^(?:\w*self_test\w*|test_\w*)$")
PY_TEST_CLASS_NAME_RE = re.compile(r"^\w*[Tt]est\w*$")


def python_test_regions(text: str) -> str:
    """Bodies of top-level `def *self_test*` / `def test_*` / `class *Test*`.

    Read out of Python's OWN parse tree, not off an indentation heuristic.  [OPUS-5]

    ROUND-3 CORRECTION. The previous version walked lines: the region was the
    header plus every following line that was blank or began with a space, a tab
    or `)`. That is not what a Python block is, and it silently TRUNCATED two
    real self-tests in this tree at the first column-0 line inside the body:

      * `scripts/export-kb-dump.py` — a `# …` comment at column 0 inside
        `run_dry_run_self_test`, which is legal Python and does not end the
        block. Region captured: 628 chars of 11955, and `run_leak_check` was
        therefore reported as untested by a self-test that names it 4 lines past
        the cut. (Only that one: `leak_check_file` and
        `_rdflib_check_restricted_projection` are named nowhere in the recovered
        region and still fire — they are part of the indirect-reachability
        residue, not this bug.)
      * `scripts/check-spec-normative-status.py` — a triple-quoted fixture whose
        markdown content starts at column 0. Region captured: 54 chars of 656.
        No symbol in that file fires either way; it is the same hole, found by
        differencing the heuristic against `ast` across every `scripts/*.py`
        rather than by a firing.

    Two different holes in one heuristic is the design being wrong rather than
    incomplete, so the heuristic is gone. `ast` gives the exact extent of every
    top-level statement, so there is no third TRUNCATION case to find. What
    remains approximate is which top-level names count as tests (below), not
    where their bodies end.

    A file that does not parse contributes NOTHING (fail-closed): its guards get
    reported rather than silently satisfied. Every `scripts/*.py` in the tree
    parses today — they all run in CI — so this is a guard, not a code path the
    tree depends on.
    """
    try:
        tree = ast.parse(text)
    except SyntaxError:
        return ""
    lines = text.splitlines()
    out: list[str] = []
    for node in tree.body:
        if isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef)):
            keep = PY_TEST_DEF_NAME_RE.match(node.name) is not None
        elif isinstance(node, ast.ClassDef):
            keep = PY_TEST_CLASS_NAME_RE.match(node.name) is not None
        else:
            continue
        if keep and node.end_lineno is not None:
            out.extend(lines[node.lineno - 1: node.end_lineno])
    return "\n".join(out)


def searchable_test_text(
    rel: str,
    text: str,
    test_module_files: frozenset[str] | set[str] = frozenset(),
    masked: str | None = None,
) -> str:
    """The slice of `rel` in which naming a guard counts as testing it.

    `test_module_files` is the set of Rust src files that some parent declares as
    `#[cfg(test)] mod X;` — those are compiled ONLY under test, so like a
    `tests/` file they count end to end. `masked` is an optional pre-computed
    `mask_rust(text)` (the corpus builder already has one).
    """
    if any(h in "/" + rel for h in TEST_PATH_HINTS):
        return text                                    # a test file, end to end
    if rel in test_module_files:
        return text                                    # ditto: `#[cfg(test)] mod X;`
    if RUST_SRC_RE.match(rel):
        masked = mask_rust(text) if masked is None else masked
        parts = [text[a:b] for a, b in rust_cfg_test_spans(masked)]
        doc = rust_doctest_text(text)
        if doc:
            parts.append(doc)
        return "\n".join(parts)
    if PY_SCRIPT_RE.match(rel):
        return python_test_regions(text)
    return ""


def test_corpus(root: Path) -> list[tuple[str, str]]:
    """(relpath, TEST TEXT) for every file that can host a test naming a guard.

    The second element is deliberately NOT the file's whole text — see the block
    comment above — EXCEPT for a file that is a test target in its entirety: a
    `tests/`/`benches/`/`fuzz/` file, or one declared by a parent as
    `#[cfg(test)] mod X;`. A file with no test region contributes nothing, so an
    ordinary call site never satisfies a guard, and neither does the guard's own
    definition line.
    """
    corpus: list[tuple[str, str]] = []
    try:
        tracked = subprocess.run(
            ["git", "ls-files"], cwd=root, capture_output=True, text=True, check=True
        ).stdout.splitlines()
    except (subprocess.CalledProcessError, FileNotFoundError):
        return corpus
    # Read + mask every Rust src file ONCE: the mask is needed both to resolve
    # `#[cfg(test)] mod X;` declarations (which live in the PARENT file, so this
    # cannot be decided per-file) and to scope each file's own test regions.
    texts: dict[str, str] = {}
    masks: dict[str, str] = {}
    for rel in tracked:
        if not RUST_SRC_RE.match(rel):
            continue
        try:
            texts[rel] = (root / rel).read_text(encoding="utf-8", errors="replace")
        except OSError:
            continue
        masks[rel] = mask_rust(texts[rel])
    mod_files = rust_test_module_files(masks)
    for rel in tracked:
        is_test_dir = any(h in "/" + rel for h in TEST_PATH_HINTS)
        is_py_script = PY_SCRIPT_RE.match(rel) is not None
        is_rust_src = RUST_SRC_RE.match(rel) is not None
        if not (is_test_dir or is_rust_src or is_py_script):
            continue
        if not (is_py_script or TEST_NAME_RE.search(rel)):
            continue
        if rel in texts:
            text = texts[rel]
        else:
            try:
                text = (root / rel).read_text(encoding="utf-8", errors="replace")
            except OSError:
                continue
        scoped = searchable_test_text(rel, text, mod_files, masks.get(rel))
        if not scoped.strip():
            continue
        corpus.append((rel, scoped))
    return corpus


def check_guard_untested(added_lines: dict[str, list[str]], root: Path) -> list[Finding]:
    symbols = added_symbols(added_lines)
    if not symbols:
        return []
    corpus = test_corpus(root)
    findings: list[Finding] = []
    for path, name in symbols:
        word = re.compile(r"\b" + re.escape(name) + r"\b")
        if any(word.search(text) for _rel, text in corpus):
            continue
        findings.append(
            Finding(
                check="guard-untested",
                path=path,
                detail=f"new guard-shaped surface `{name}` is named by no test in the tree",
                fix=(
                    f"add a test that FAILS when `{name}` is deleted or inverted "
                    f"(not one that merely calls it), or annotate the definition with "
                    f"`preflight-allow: guard-untested — <why>`"
                ),
            )
        )
    return findings


# ---------------------------------------------------------------------------
# Delegated gates: the scripts the repo already owns and the briefs never name.
# ---------------------------------------------------------------------------
@dataclass
class Delegate:
    name: str
    argv: list[str]
    #  None  -> always run; otherwise run only if some changed path matches.
    path_filter: re.Pattern[str] | None = None
    pass_changed_files: bool = False
    pass_changed_paths: bool = False

    @property
    def script(self) -> str:
        """The gate script itself — `argv[0]` is the interpreter (python3/bash)."""
        return self.argv[1]


DELEGATES = [
    Delegate("G1 new-crate/new-package completeness", ["python3", "scripts/gate-new-crate.py"],
             pass_changed_files=True),
    Delegate("G2 public-api-to-skill", ["python3", "scripts/gate-api-skill.py"],
             pass_changed_files=True),
    Delegate("G6 new-config-to-docs", ["python3", "scripts/check-config-documented.py"],
             pass_changed_files=True),
    Delegate("no-perf-numbers", ["python3", "scripts/check-no-perf-numbers.py", "--enforce"],
             path_filter=re.compile(r"\.(md|typ)$"), pass_changed_paths=True),
    Delegate("readme-template", ["python3", "scripts/check-readme-template.py", "--enforce"],
             path_filter=re.compile(r"^crates/[^/]+/README\.md$"), pass_changed_paths=True),
    Delegate("privacy-claims", ["bash", "scripts/check-privacy-claims.sh"]),
]


def run_delegates(changed: list[str], root: Path, changed_files_path: Path | None) -> Result:
    res = Result()
    for d in DELEGATES:
        matched = [p for p in changed if d.path_filter.search(p)] if d.path_filter else changed
        if d.path_filter and not matched:
            res.skipped.append(f"{d.name} (no matching path in the diff)")
            continue
        # [OPUS-5] FAIL CLOSED on a missing gate script. This was a `skipped`
        # entry, so renaming or deleting any gate script silently degraded that
        # gate to a skip and preflight still exited 0 — a gate that does not run
        # reported as a gate that passed. "The script is gone" is a finding
        # about the diff, never a reason to pass it.
        if not (root / d.script).exists():
            res.findings.append(
                Finding(
                    check=d.name,
                    path=d.script,
                    detail=f"the gate script `{d.script}` does not exist under {root} — "
                           f"this gate DID NOT RUN",
                    fix=f"restore `{d.script}`, or if it was renamed, update DELEGATES in "
                        f"scripts/preflight.py to match",
                )
            )
            continue
        argv = list(d.argv)
        if d.pass_changed_files and changed_files_path is not None:
            argv += ["--changed-files", str(changed_files_path)]
        if d.pass_changed_paths:
            existing = [p for p in matched if (root / p).exists()]
            if not existing:
                res.skipped.append(f"{d.name} (all matching paths deleted)")
                continue
            argv += existing
        proc = subprocess.run(argv, cwd=root, capture_output=True, text=True)
        res.ran.append(d.name)
        if proc.returncode != 0:
            tail = (proc.stdout + proc.stderr).strip().splitlines()
            res.findings.append(
                Finding(
                    check=d.name,
                    path="(diff)",
                    detail="\n      ".join(tail[-12:]) or f"exit {proc.returncode}",
                    fix=f"reproduce with: {' '.join(argv)}",
                )
            )
    return res


# ---------------------------------------------------------------------------
# The obligations no script can decide. Printed, never auto-passed.
# ---------------------------------------------------------------------------
NON_MECHANICAL = """\
NOT CHECKED BY THIS SCRIPT — you must execute these yourself before opening the PR.
They are the two largest preventable review-failure classes in the verdict corpus
(GUARD-NOT-PINNED 63 findings, CLAIM-vs-CODE 67 findings) and neither is decidable
by static analysis:

  1. MUTATE YOUR HEADLINE GUARD. Take the feature named in your PR title. DELETE or
     INVERT it in the worktree and RUN the suite. If nothing goes red, your test is
     vacuous — that is a blocking defect, and it is the single most common one the
     reviewers find. Do not reason about it; execute it. Report which test died.
     (`guard-untested` above only catches a guard with NO test at all. A test that
     exists but asserts a bound, a type, or a marker string instead of the behaviour
     passes this script and fails review.)

  2. READ YOUR OWN PROSE AGAINST YOUR OWN DIFF. For every line of documentation,
     rustdoc, README, SKILL.md, comment, research record or PR-body claim you added:
     point at the code in THIS diff that makes it true. If you cannot, delete the
     sentence or fix the code. Overclaiming is blocking. The corpus is full of
     diffs whose docs describe a module, flag, constant or test file that the diff
     does not contain.
"""


def collect_diff(base: str, root: Path) -> tuple[list[str], dict[str, list[str]]]:
    merge_base = subprocess.run(
        ["git", "merge-base", base, "HEAD"], cwd=root, capture_output=True, text=True
    ).stdout.strip() or base
    names = subprocess.run(
        ["git", "diff", "--name-only", "--diff-filter=d", f"{merge_base}...HEAD"],
        cwd=root, capture_output=True, text=True, check=True,
    ).stdout.split()
    raw = subprocess.run(
        ["git", "diff", "-U0", f"{merge_base}...HEAD"],
        cwd=root, capture_output=True, text=True, check=True,
    ).stdout
    return names, parse_added(raw)


def parse_added(unified: str) -> dict[str, list[str]]:
    """path -> added lines (no leading '+'). `-U0` so no context leaks in."""
    added: dict[str, list[str]] = {}
    path: str | None = None
    for line in unified.splitlines():
        if line.startswith("+++ b/"):
            path = line[6:]
            added.setdefault(path, [])
        elif line.startswith("+++ "):
            path = None
        elif path is not None and line.startswith("+") and not line.startswith("+++"):
            added[path].append(line[1:])
    return {k: v for k, v in added.items() if v}


def report(res: Result, quiet: bool) -> int:
    if res.ran and not quiet:
        print("preflight: ran  " + ", ".join(res.ran))
    if res.skipped and not quiet:
        print("preflight: skip " + "; ".join(res.skipped))
    if res.ok:
        print("\npreflight: PASS — every mechanical pre-submit gate is green on this diff.")
    else:
        print(f"\npreflight: FAIL — {len(res.findings)} mechanical finding(s):\n")
        for f in res.findings:
            print(f"  [{f.check}] {f.path}")
            print(f"      {f.detail}")
            print(f"      fix: {f.fix}\n")
    print()
    print(NON_MECHANICAL)
    return 0 if res.ok else 1


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(
        description="Pre-submit gate runner — run every mechanical merge-gate against "
                    "YOUR diff, in YOUR worktree, before the PR exists."
    )
    ap.add_argument("--base", default="origin/main", help="base ref (default origin/main)")
    ap.add_argument("--changed-files", help="hermetic input: file of changed paths")
    ap.add_argument("--added-lines", help="hermetic input: a unified diff to read added lines from")
    ap.add_argument("--root", default=str(REPO_ROOT), help="repo root (tests inject a fixture tree)")
    ap.add_argument("--only", choices=["guard-untested", "delegates"],
                    help="run a single check group (tests + fast local loops)")
    ap.add_argument("--quiet", action="store_true")
    ap.add_argument("--self-test", action="store_true", help="run the hermetic self-test and exit")
    args = ap.parse_args(argv)

    if args.self_test:
        return self_test()

    root = Path(args.root).resolve()
    changed_files_path: Path | None = None
    if args.changed_files:
        changed_files_path = Path(args.changed_files).resolve()
        changed = [
            ln.split("\t")[-1].strip()
            for ln in changed_files_path.read_text().splitlines()
            if ln.strip()
        ]
        added = parse_added(Path(args.added_lines).read_text()) if args.added_lines else {}
    else:
        changed, added = collect_diff(args.base, root)

    res = Result()
    if args.only != "delegates":
        res.findings += check_guard_untested(added, root)
    if args.only != "guard-untested":
        sub = run_delegates(changed, root, changed_files_path)
        res.findings += sub.findings
        res.ran += sub.ran
        res.skipped += sub.skipped
    if args.only != "delegates":
        res.ran.append("guard-untested")
    return report(res, args.quiet)


# ---------------------------------------------------------------------------
# Hermetic self-test (repo convention: a --self-test step wired HARD in
# docs-quality.yml, so the checker cannot silently rot).
# ---------------------------------------------------------------------------
def self_test() -> int:
    import tempfile

    failures: list[str] = []

    def chk(name: str, got: object, want: object) -> None:
        if got != want:
            failures.append(f"{name}: got {got!r}, want {want!r}")
        print(f"  {'ok  ' if got == want else 'FAIL'} {name}")

    # --- added_symbols: what is and is not a guard-shaped SURFACE ---
    rust = "crates/sparq-x/src/lib.rs"
    pyf = "scripts/thing.py"
    chk("rust pub fn validate_* is a guard",
        added_symbols({rust: ["pub fn validate_envelope(e: &E) -> bool {"]}),
        [(rust, "validate_envelope")])
    chk("rust pub fn *_guard is a guard",
        added_symbols({rust: ["pub fn admission_guard(x: u8) {"]}),
        [(rust, "admission_guard")])
    chk("rust pub fn is_*_valid is a guard",
        added_symbols({rust: ["pub fn is_lease_valid(l: &L) -> bool {"]}),
        [(rust, "is_lease_valid")])
    chk("rust pub(crate) fn check_* is a guard",
        added_symbols({rust: ["pub(crate) fn check_bounds(n: usize) {"]}),
        [(rust, "check_bounds")])
    chk("rust PRIVATE fn check_* is NOT a surface",
        added_symbols({rust: ["fn check_bounds(n: usize) {"]}), [])
    chk("rust pub fn with no guard stem is NOT a guard",
        added_symbols({rust: ["pub fn render_row(r: &R) {"]}), [])
    chk("python module-level def validate_* is a guard",
        added_symbols({pyf: ["def validate_lease(x):"]}), [(pyf, "validate_lease")])
    chk("python NESTED def is NOT a surface",
        added_symbols({pyf: ["    def validate_lease(x):"]}), [])
    chk("a non-src, non-scripts path is out of scope",
        added_symbols({"docs/x.rs": ["pub fn validate_envelope() {"]}), [])

    # --- suppression semantics: a reason is MANDATORY ---
    chk("suppression with a reason suppresses",
        added_symbols({rust: [
            "// preflight-allow: guard-untested — proven by the kani harness",
            "pub fn validate_envelope() {",
        ]}), [])
    chk("suppression WITHOUT a reason does NOT suppress",
        added_symbols({rust: [
            "// preflight-allow: guard-untested —",
            "pub fn validate_envelope() {",
        ]}), [(rust, "validate_envelope")])
    chk("suppression for a DIFFERENT check does not suppress",
        added_symbols({rust: [
            "// preflight-allow: no-perf-numbers — unrelated",
            "pub fn validate_envelope() {",
        ]}), [(rust, "validate_envelope")])

    # --- parse_added: -U0 unified diff, added lines only ---
    diff = (
        "diff --git a/scripts/a.py b/scripts/a.py\n"
        "--- a/scripts/a.py\n"
        "+++ b/scripts/a.py\n"
        "@@ -1,0 +2 @@\n"
        "+def validate_x(v):\n"
        "-def gone(v):\n"
    )
    chk("parse_added keeps only + lines", parse_added(diff), {"scripts/a.py": ["def validate_x(v):"]})
    chk("parse_added ignores the +++ header",
        "+++ b/scripts/a.py" in parse_added(diff)["scripts/a.py"], False)

    # --- END-TO-END on a real throwaway git tree: the check must RED on a guard
    #     with no test, and GREEN once a test names it. This is the behaviour the
    #     mutation test in scripts/tests/test_preflight.py deletes the detector on.
    with tempfile.TemporaryDirectory() as td:
        root = Path(td)
        (root / "crates/sparq-x/src").mkdir(parents=True)
        (root / "crates/sparq-x/src/lib.rs").write_text("pub fn validate_envelope() {}\n")
        subprocess.run(["git", "init", "-q"], cwd=root, check=True)
        subprocess.run(["git", "add", "-A"], cwd=root, check=True)
        added = {"crates/sparq-x/src/lib.rs": ["pub fn validate_envelope() {}"]}
        chk("e2e: guard with NO test -> 1 finding",
            len(check_guard_untested(added, root)), 1)

        (root / "crates/sparq-x/tests").mkdir(parents=True)
        (root / "crates/sparq-x/tests/t.rs").write_text(
            "#[test]\nfn t() { sparq_x::validate_envelope(); }\n"
        )
        subprocess.run(["git", "add", "-A"], cwd=root, check=True)
        chk("e2e: guard named by a test -> 0 findings",
            len(check_guard_untested(added, root)), 0)

        # An unrelated test naming a DIFFERENT symbol must not satisfy the guard —
        # otherwise the check is vacuous (any test file in the repo would pass it).
        (root / "crates/sparq-x/tests/t.rs").write_text(
            "#[test]\nfn t() { sparq_x::something_else(); }\n"
        )
        subprocess.run(["git", "add", "-A"], cwd=root, check=True)
        chk("e2e: an unrelated test does NOT satisfy the guard",
            len(check_guard_untested(added, root)), 1)

    # --- `#[cfg(test)] mod X;` — the test body lives in ANOTHER FILE (round 3) ---
    with tempfile.TemporaryDirectory() as td:
        root = Path(td)
        (root / "crates/sparq-x/src").mkdir(parents=True)
        added = {"crates/sparq-x/src/lib.rs": ["pub fn validate_envelope() {}"]}
        subprocess.run(["git", "init", "-q"], cwd=root, check=True)

        (root / "crates/sparq-x/src/lib.rs").write_text(
            "pub fn validate_envelope() {}\n#[cfg(test)]\nmod unit;\n"
        )
        (root / "crates/sparq-x/src/unit.rs").write_text(
            "#[test]\nfn t() { crate::validate_envelope(); }\n"
        )
        subprocess.run(["git", "add", "-A"], cwd=root, check=True)
        chk("e2e: a separate-file `#[cfg(test)] mod X;` satisfies the guard",
            len(check_guard_untested(added, root)), 0)

        # …and the other direction: a PLAIN `mod X;` is production code.
        (root / "crates/sparq-x/src/lib.rs").write_text(
            "pub fn validate_envelope() {}\nmod unit;\n"
        )
        (root / "crates/sparq-x/src/unit.rs").write_text(
            "pub fn go() { crate::validate_envelope(); }\n"
        )
        subprocess.run(["git", "add", "-A"], cwd=root, check=True)
        chk("e2e: a plain `mod X;` does NOT satisfy the guard",
            len(check_guard_untested(added, root)), 1)

    chk("`#[cfg(test)] mod X;` resolves, a braced `mod X { .. }` does not",
        (rust_cfg_test_mod_decls(mask_rust("#[cfg(test)]\nmod unit;\n")),
         rust_cfg_test_mod_decls(mask_rust("#[cfg(test)]\nmod unit { }\n"))),
        (["unit"], []))
    chk("a column-0 comment does not end a python test region",
        "validate_lease" in python_test_regions(
            "def self_test():\n    ok = True\n# col-0\n    validate_lease(ok)\n"),
        True)
    chk("an unparseable python script contributes no test text",
        python_test_regions("def self_test():\n    validate_lease(1)\ndef (\n"), "")

    print()
    if failures:
        print(f"preflight --self-test: FAIL ({len(failures)})")
        for f in failures:
            print("  " + f)
        return 1
    print("preflight --self-test: PASS")
    return 0


if __name__ == "__main__":
    sys.exit(main())
