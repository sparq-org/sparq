#!/usr/bin/env python3
# [SONNET-4.6] Feature-matrix tier detector + sq-vya1 guard (bead sq-6g9kr).
# Design: research/feature-matrix-pyramid.md §4.
#
# Classifies each opt-in feature-matrix leg as sensitive (tier: test) or
# non-sensitive (a check-tier demotion candidate), and enforces the sq-vya1
# GUARD that every sensitive feature appears in some test:true leg.
#
# CORE INVARIANT (fail-closed, §4 "any parse/IO/grep error"):
#   Any parse/IO/grep error classifies the leg SENSITIVE. A detector bug
#   degrades to "keep the full leg", never to a silent demotion.
#
# Sensitivity criteria for a feature F in crate C:
#   (a) cfg(feature = "F") / cfg_attr form appears in tests/ or benches/ files
#   (b) cfg expression combining `test` and feature = "F" in src/ (e.g.,
#       cfg(all(test, feature = "F")))
#   (c) cfg(not(feature = "F")) or F under any not() anywhere in the crate
#   Any of (a/b/c) => SENSITIVE. Nested any()/all() are recursively parsed;
#   an expression the parser cannot handle => SENSITIVE (fail-closed).
#
# Usage:
#   python3 scripts/feature-matrix-tiers.py --classify
#       Print a classification table for all legs.
#   python3 scripts/feature-matrix-tiers.py --enforce
#       Exit non-zero if:
#         (1) any leg carries tier: check AND detector says sensitive AND
#             the fragment has no tier-reason: override; OR
#         (2) any sensitive (crate, feature) has no test:true leg IN THAT
#             CRATE and no written test-reason: override (sq-vya1 guard).
#             Cargo feature names are crate-local, so both invariants key on
#             the (crate, feature) PAIR — a test:true leg for feature F in
#             crate A must never satisfy a sensitive F in crate B.
#   python3 scripts/feature-matrix-tiers.py --report-unlegged
#       Advisory: features with no leg, against the SCOPE allowlist.
#
# Stdlib only (no new Python deps beyond stdlib). Runs under Python 3.10+.

from __future__ import annotations

import argparse
import os
import re
import sys
from dataclasses import dataclass, field
from pathlib import Path
from typing import List, Optional, Tuple

REPO_ROOT = Path(__file__).resolve().parent.parent
FRAGMENT_DIR = REPO_ROOT / ".github" / "feature-matrix.d"
CRATES_DIR = REPO_ROOT / "crates"

# Allowlist: features explicitly excluded from the unlegged completeness check.
# These are known to have no per-PR leg by design (design record §4.3).
SCOPE_ALLOWLIST: frozenset = frozenset(
    {"zlib-ng", "hdt", "write", "live", "embeddings", "mimalloc"}
)

# Rust source file extensions we scan.
_RUST_EXTENSIONS = frozenset({".rs"})

# Source directories to scan (relative to crate root).
_TEST_DIRS = ("tests", "benches")
_SRC_DIRS = ("src", "bin", "examples")


# ---------------------------------------------------------------------------
# cfg expression AST
# ---------------------------------------------------------------------------

class ParseError(Exception):
    """Raised when a cfg expression cannot be parsed. Triggers fail-closed."""


class _CfgNode:
    """Base class for cfg expression AST nodes."""
    __slots__: tuple = ()


class CfgFeature(_CfgNode):
    """cfg(feature = "name")"""
    __slots__ = ("name",)

    def __init__(self, name: str) -> None:
        self.name = name


class CfgTest(_CfgNode):
    """cfg(test) — the bare `test` predicate."""
    __slots__ = ()


class CfgNot(_CfgNode):
    """cfg(not(...))"""
    __slots__ = ("inner",)

    def __init__(self, inner: _CfgNode) -> None:
        self.inner = inner


class CfgAny(_CfgNode):
    """cfg(any(...))"""
    __slots__ = ("exprs",)

    def __init__(self, exprs: List[_CfgNode]) -> None:
        self.exprs = exprs


class CfgAll(_CfgNode):
    """cfg(all(...))"""
    __slots__ = ("exprs",)

    def __init__(self, exprs: List[_CfgNode]) -> None:
        self.exprs = exprs


class CfgOther(_CfgNode):
    """Any other predicate (target_arch, etc.) that we do not need to interpret."""
    __slots__ = ()


# ---------------------------------------------------------------------------
# cfg expression parser
# ---------------------------------------------------------------------------

def _extract_balanced_parens(s: str, start: int) -> Tuple[str, int]:
    """Extract content of balanced parentheses at s[start] (which must be '(').

    Returns (inner_content, end_index_exclusive).
    Raises ParseError on unbalanced parentheses.
    """
    if start >= len(s) or s[start] != "(":
        raise ParseError(
            "expected '(' at pos {} in {!r}".format(start, s[:start + 20])
        )
    depth = 0
    for i in range(start, len(s)):
        c = s[i]
        if c == "(":
            depth += 1
        elif c == ")":
            depth -= 1
            if depth == 0:
                return s[start + 1 : i], i + 1
    raise ParseError(
        "unbalanced parentheses starting at pos {} in {!r}...".format(
            start, s[start : start + 40]
        )
    )


def _split_cfg_args(s: str) -> List[str]:
    """Split comma-separated cfg predicate list, respecting nested parentheses.

    Raises ParseError on unbalanced parentheses.
    """
    items: List[str] = []
    depth = 0
    current: List[str] = []
    for ch in s:
        if ch == "(":
            depth += 1
            current.append(ch)
        elif ch == ")":
            depth -= 1
            if depth < 0:
                raise ParseError(
                    "unbalanced ')' in cfg arg list: {!r}".format(s[:60])
                )
            current.append(ch)
        elif ch == "," and depth == 0:
            item = "".join(current).strip()
            if item:
                items.append(item)
            current = []
        else:
            current.append(ch)
    tail = "".join(current).strip()
    if tail:
        items.append(tail)
    return items


def parse_cfg(s: str) -> _CfgNode:
    """Parse a Rust cfg predicate string into an AST node.

    Raises ParseError if the expression cannot be understood.
    Callers treat ParseError as SENSITIVE (fail-closed).

    Handles: feature = "...", test, not(...), any(...), all(...), and
    any other bare or key = "value" predicate (CfgOther).
    """
    s = s.strip()
    if not s:
        raise ParseError("empty cfg expression")

    # bare `test` predicate
    if s == "test":
        return CfgTest()

    # feature = "name"
    m = re.fullmatch(r'feature\s*=\s*"([^"]*)"', s)
    if m:
        return CfgFeature(m.group(1))

    # function-form: not(...) / any(...) / all(...)
    fn_match = re.match(r"^(not|any|all)\s*\(", s)
    if fn_match:
        fn_name = fn_match.group(1)
        paren_pos = s.index("(", fn_match.start())
        inner, end = _extract_balanced_parens(s, paren_pos)
        remaining = s[end:].strip()
        if remaining:
            raise ParseError(
                "unexpected content after {}(...): {!r}".format(fn_name, remaining)
            )
        items = _split_cfg_args(inner)
        if fn_name == "not":
            if len(items) != 1:
                raise ParseError(
                    "not() requires exactly 1 argument, got {} in {!r}".format(
                        len(items), s
                    )
                )
            return CfgNot(parse_cfg(items[0]))
        # any / all
        children = [parse_cfg(item) for item in items]
        return CfgAny(children) if fn_name == "any" else CfgAll(children)

    # Other simple predicate: bare ident or ident = "value"
    if re.fullmatch(r'[a-zA-Z_][a-zA-Z0-9_\-]*(\s*=\s*"[^"]*")?', s):
        return CfgOther()

    raise ParseError("cannot parse cfg expression: {!r}".format(s[:80]))


# ---------------------------------------------------------------------------
# cfg tree queries
# ---------------------------------------------------------------------------

def _mentions_feature(node: _CfgNode, feature: str) -> bool:
    """Return True if feature F appears anywhere in the cfg tree."""
    if isinstance(node, CfgFeature):
        return node.name == feature
    if isinstance(node, CfgNot):
        return _mentions_feature(node.inner, feature)
    if isinstance(node, (CfgAny, CfgAll)):
        return any(_mentions_feature(e, feature) for e in node.exprs)
    return False


def _feature_under_not(node: _CfgNode, feature: str, _in_not: bool = False) -> bool:
    """Return True if feature F appears anywhere under a not() in the tree.

    Handles nested combinators: cfg(not(any(x, feature = "F"))) returns True.
    """
    if isinstance(node, CfgFeature):
        return _in_not and node.name == feature
    if isinstance(node, CfgNot):
        return _feature_under_not(node.inner, feature, _in_not=True)
    if isinstance(node, (CfgAny, CfgAll)):
        return any(_feature_under_not(e, feature, _in_not) for e in node.exprs)
    return False


def _has_test_predicate(node: _CfgNode) -> bool:
    """Return True if the `test` predicate appears anywhere in the tree."""
    if isinstance(node, CfgTest):
        return True
    if isinstance(node, CfgNot):
        return _has_test_predicate(node.inner)
    if isinstance(node, (CfgAny, CfgAll)):
        return any(_has_test_predicate(e) for e in node.exprs)
    return False


def _feature_in_test_cfg(node: _CfgNode, feature: str) -> bool:
    """Return True if the expression mentions both `test` and feature F.

    Catches cfg(all(test, feature = "F")) and similar patterns that gate
    test blocks in src/ files (class b-direct in the design record).
    """
    return _mentions_feature(node, feature) and _has_test_predicate(node)


def _feature_adjacent_to_test_attr(content: str, feature: str) -> bool:
    """Return True if #[cfg(feature = "F")] and #[test] appear adjacent in source.

    Detects the b-direct pattern: a feature-gated #[test] function in src/ where
    the two attributes are on consecutive (or nearly-consecutive) lines:

        #[cfg(feature = "F")]
        #[test]
        fn my_test() { ... }

    Uses a conservative window of 3 lines so back-to-back attributes on the same
    item are caught while avoiding false positives from unrelated code blocks.
    Fail-closed: any regex error -> returns True (but regexes here are static).
    """
    escaped = re.escape(feature)
    cfg_pat = re.compile(
        r'#\s*\[?\s*cfg\s*\(\s*feature\s*=\s*"' + escaped + r'"\s*\)'
    )
    test_pat = re.compile(r"#\s*\[?\s*test\s*\]")
    lines = content.splitlines()
    cfg_lines: List[int] = []
    test_lines: List[int] = []
    for i, line in enumerate(lines):
        if cfg_pat.search(line):
            cfg_lines.append(i)
        if test_pat.search(line):
            test_lines.append(i)
    # Adjacent within 3 lines (covers #[cfg(...)] then #[test] then fn body)
    for ci in cfg_lines:
        for ti in test_lines:
            if abs(ci - ti) <= 3:
                return True
    return False


# ---------------------------------------------------------------------------
# Source file scanner
# ---------------------------------------------------------------------------

# Pattern to find the start of cfg / cfg_attr attributes (inner and outer form).
_CFG_START_RE = re.compile(r"#!?\[cfg(?:_attr)?\s*\(")

def _sanitize_for_cfg_scan(content: str) -> str:
    """Prepare Rust source content for safe cfg-expression extraction.

    A left-to-right lexical scan masks comments, character literals, and
    ordinary/raw string literals while preserving length and CR/LF. Real cfg
    attributes remain unchanged; cfg-like text in non-code spans is blanked.
    """
    safe = list(content)

    def mask(start: int, end: int) -> None:
        for pos in range(start, end):
            if safe[pos] not in ("\n", "\r"):
                safe[pos] = "_"

    def quoted_end(start: int, quote: str) -> Optional[int]:
        pos = start + 1
        while pos < len(content):
            if content[pos] == "\\":
                pos += 2
            elif content[pos] == quote:
                return pos + 1
            else:
                pos += 1
        return None

    i = 0
    while i < len(content):
        if content.startswith("//", i):
            end = content.find("\n", i + 2)
            end = len(content) if end < 0 else end
            mask(i, end)
            i = end
            continue

        if content.startswith("/*", i):
            depth = 1
            end = i + 2
            while end < len(content) and depth:
                if content.startswith("/*", end):
                    depth += 1
                    end += 2
                elif content.startswith("*/", end):
                    depth -= 1
                    end += 2
                else:
                    end += 1
            mask(i, end)
            i = end
            continue

        raw_start = i
        if content.startswith(("br", "cr"), i):
            hash_start = i + 2
        elif content.startswith("r", i):
            hash_start = i + 1
        else:
            hash_start = -1
        if hash_start >= 0:
            quote = hash_start
            while quote < len(content) and content[quote] == "#":
                quote += 1
            hash_count = quote - hash_start
            if hash_count <= 255 and quote < len(content) and content[quote] == '"':
                terminator = '"' + ("#" * hash_count)
                end = content.find(terminator, quote + 1)
                end = len(content) if end < 0 else end + len(terminator)
                mask(raw_start, end)
                i = end
                continue

        quote = i + 1 if content[i:i + 1] in ("b", "c") else i
        if quote < len(content) and content[quote] == '"':
            end = quoted_end(quote, '"')
            end = len(content) if end is None else end
            mask(i, end)
            i = end
            continue

        if content[i] == "'":
            end = quoted_end(i, "'")
            body = content[i + 1 : end - 1] if end is not None else ""
            if end is not None and "\n" not in body and "\r" not in body and (
                len(body) == 1 or body.startswith("\\")
            ):
                mask(i, end)
                i = end
                continue

        i += 1

    return "".join(safe)


def _extract_cfg_expressions(content: str) -> List[str]:
    """Extract all cfg predicate strings from Rust source file content.

    For ``#[cfg(...)]`` / ``#![cfg(...)]``: returns the inner content.
    For ``#[cfg_attr(cond, ...)]``: returns the condition (first arg only).

    Uses a two-phase approach:
    1. ``_sanitize_for_cfg_scan`` masks string literals + blanks line comments.
       This prevents false matches from ``#[cfg(...)]`` embedded in string
       literals or doc comments.  The sanitised string has the SAME LENGTH
       as the original, so positions are identical.
    2. We use the sanitised string to FIND cfg attribute positions (balanced
       parens are resolved on the sanitised content, where fake #[cfg( inside
       strings no longer appear), then slice the ORIGINAL content at those
       positions to recover the real feature names.

    Raises ParseError if a balanced-paren extraction or arg split fails.
    """
    # _sanitize_for_cfg_scan preserves string length (no chars removed).
    safe = _sanitize_for_cfg_scan(content)
    assert len(safe) == len(content), "sanitize must preserve string length"

    results: List[str] = []
    for m in _CFG_START_RE.finditer(safe):
        paren_start = m.end() - 1  # the '(' that closes the attribute opener
        # Resolve balanced parens on the SAFE content (which has no stray
        # chars inside string literals that could confuse the paren counter).
        _inner_safe, end_pos = _extract_balanced_parens(safe, paren_start)
        # Slice the ORIGINAL content at the exact same positions to recover
        # the real feature names (which were masked in 'safe').
        outer_inner = content[paren_start + 1 : end_pos - 1]

        if "cfg_attr" in m.group():
            # cfg_attr(condition, attr, ...): condition is the first argument.
            # Use _split_cfg_args on the SAFE inner slice so that stray commas
            # inside string literals don't fragment the condition, then map
            # back to original positions via the preserved length invariant.
            inner_safe = safe[paren_start + 1 : end_pos - 1]
            args_safe = _split_cfg_args(inner_safe)
            if not args_safe:
                raise ParseError(
                    "empty cfg_attr at offset {}".format(m.start())
                )
            # The first arg in safe has the same span as in original.
            # Find its start/end by scanning from paren_start + 1.
            cond_safe = args_safe[0]
            cond_start_in_inner = inner_safe.index(cond_safe)
            cond_orig = outer_inner[
                cond_start_in_inner : cond_start_in_inner + len(cond_safe)
            ]
            results.append(_normalize_cfg_expr(cond_orig.strip()))
        else:
            results.append(_normalize_cfg_expr(outer_inner.strip()))
    return results


def _normalize_cfg_expr(s: str) -> str:
    r"""Normalize backslash-escaped quotes in a cfg expression string.

    Rust multi-line string literals (line continuation with ``\``) can prevent
    the string-literal masking regex from matching across line boundaries.
    When a cfg expression is extracted from within such an unmasked string, the
    feature name may appear as ``feature = \"name\"`` (with ``\"`` escapes)
    rather than ``feature = "name"``.  Normalising ``\"`` → ``"`` before
    parsing ensures these expressions parse correctly and produce the right
    feature name rather than triggering the fail-closed path.

    This is safe because cfg feature names never contain backslashes or bare
    quotes, so the substitution is unambiguous in this context.
    """
    return s.replace('\\"', '"')


def _collect_rust_files(directory: Path) -> List[Path]:
    """Recursively collect .rs files under `directory`.

    Raises OSError if the directory or any subdirectory cannot be listed.
    Uses ``onerror`` so ``os.walk`` re-raises permission errors instead of
    silently swallowing them (the default ``onerror=None`` behaviour).
    """
    result: List[Path] = []

    def _reraise(exc: OSError) -> None:  # pragma: no cover
        raise exc

    for root, _dirs, files in os.walk(str(directory), onerror=_reraise):
        for fname in files:
            if Path(fname).suffix in _RUST_EXTENSIONS:
                result.append(Path(root) / fname)
    return result


# ---------------------------------------------------------------------------
# Leg classifier
# ---------------------------------------------------------------------------

@dataclass
class LegClassification:
    """Result of classifying a single feature-matrix leg."""

    sensitive: bool
    reasons: List[str] = field(default_factory=list)
    error_msg: Optional[str] = None


def _classify_feature_in_test_files(
    crate_dir: Path,
    feature: str,
    fail_open: bool,
) -> Optional[LegClassification]:
    """Check tests/ and benches/ directories for cfg(feature = F).

    Returns a SENSITIVE LegClassification if found, else None.
    Returns a SENSITIVE result on any IO/parse error unless fail_open=True.
    """
    for test_dir_name in _TEST_DIRS:
        test_dir = crate_dir / test_dir_name
        if not test_dir.is_dir():
            continue
        try:
            rust_files = _collect_rust_files(test_dir)
        except OSError as exc:
            msg = "IO error listing {}: {}".format(test_dir, exc)
            if fail_open:
                return None
            return LegClassification(
                sensitive=True, reasons=["error: " + msg], error_msg=msg
            )
        for fpath in rust_files:
            try:
                content = fpath.read_text(encoding="utf-8", errors="replace")
                exprs = _extract_cfg_expressions(content)
            except (OSError, ParseError) as exc:
                msg = "error reading/scanning {}: {}".format(fpath, exc)
                if fail_open:
                    continue
                return LegClassification(
                    sensitive=True, reasons=["error: " + msg], error_msg=msg
                )
            for expr_str in exprs:
                try:
                    node = parse_cfg(expr_str)
                except ParseError as exc:
                    msg = "parse error in {}: {!r} -> {}".format(fpath, expr_str[:60], exc)
                    if fail_open:
                        continue
                    return LegClassification(
                        sensitive=True, reasons=["error: " + msg], error_msg=msg
                    )
                if _mentions_feature(node, feature):
                    return LegClassification(
                        sensitive=True,
                        reasons=[
                            "cfg(feature={!r}) in test file {}".format(
                                feature, fpath
                            )
                        ],
                    )
    return None


def _classify_feature_in_src_files(
    crate_dir: Path,
    feature: str,
    fail_open: bool,
) -> Optional[LegClassification]:
    """Check src/, bin/, examples/ for not-feature or test+feature cfg patterns.

    Sensitive if:
    - feature F appears under not() at any nesting level (anywhere)
    - cfg expression contains both `test` and feature F (b-direct pattern)

    Returns a SENSITIVE LegClassification if found, else None.
    Returns a SENSITIVE result on any IO/parse error unless fail_open=True.
    """
    for src_dir_name in _SRC_DIRS:
        src_dir = crate_dir / src_dir_name
        if not src_dir.is_dir():
            continue
        try:
            rust_files = _collect_rust_files(src_dir)
        except OSError as exc:
            msg = "IO error listing {}: {}".format(src_dir, exc)
            if fail_open:
                return None
            return LegClassification(
                sensitive=True, reasons=["error: " + msg], error_msg=msg
            )
        for fpath in rust_files:
            try:
                content = fpath.read_text(encoding="utf-8", errors="replace")
                exprs = _extract_cfg_expressions(content)
            except (OSError, ParseError) as exc:
                msg = "error reading/scanning {}: {}".format(fpath, exc)
                if fail_open:
                    continue
                return LegClassification(
                    sensitive=True, reasons=["error: " + msg], error_msg=msg
                )
            for expr_str in exprs:
                try:
                    node = parse_cfg(expr_str)
                except ParseError as exc:
                    msg = "parse error in {}: {!r} -> {}".format(fpath, expr_str[:60], exc)
                    if fail_open:
                        continue
                    return LegClassification(
                        sensitive=True, reasons=["error: " + msg], error_msg=msg
                    )
                # (c) feature under not() anywhere
                if _feature_under_not(node, feature):
                    return LegClassification(
                        sensitive=True,
                        reasons=[
                            "cfg(not(... feature={!r} ...)) in {}".format(
                                feature, fpath
                            )
                        ],
                    )
                # (b) test+feature cfg expression in src/ (b-direct pattern, form 1:
                # cfg(all(test, feature = "F")))
                if _feature_in_test_cfg(node, feature):
                    return LegClassification(
                        sensitive=True,
                        reasons=[
                            "cfg(test + feature={!r}) in {}".format(feature, fpath)
                        ],
                    )
            # (b) b-direct form 2: #[cfg(feature = "F")] adjacent to #[test]
            # (checked per file after scanning all cfg expressions)
            if _feature_adjacent_to_test_attr(content, feature):
                return LegClassification(
                    sensitive=True,
                    reasons=[
                        "cfg(feature={!r}) adjacent to #[test] in {}".format(
                            feature, fpath
                        )
                    ],
                )
    return None


def classify_leg(
    crate_dir: Path,
    features: List[str],
    *,
    fail_open: bool = False,
) -> LegClassification:
    """Classify a feature-matrix leg as sensitive or non-sensitive.

    ``crate_dir`` is the root of the crate (contains Cargo.toml, src/, tests/...).
    ``features`` is the list of feature names for this leg.

    INVARIANT (fail_open=False, the only production path):
        Any IO/parse error -> sensitive=True (fail-closed). A detector bug can
        only keep a full leg; it can never silently demote a sensitive leg.

    ``fail_open=True`` is exposed ONLY so the test suite can verify the
    invariant by mutation: the fail-open copy must disagree with the
    fail-closed copy on error cases, proving the guard is load-bearing.
    Do NOT use fail_open=True in production (--classify / --enforce).
    """
    if not crate_dir.is_dir():
        msg = "crate directory not found or not readable: {}".format(crate_dir)
        if fail_open:
            return LegClassification(sensitive=False, reasons=[], error_msg=msg)
        return LegClassification(
            sensitive=True, reasons=["error: " + msg], error_msg=msg
        )

    for feature in features:
        # (a) cfg(feature = F) in test files
        result = _classify_feature_in_test_files(crate_dir, feature, fail_open)
        if result is not None:
            return result
        # (b) + (c) test+feature or not-feature in src files
        result = _classify_feature_in_src_files(crate_dir, feature, fail_open)
        if result is not None:
            return result

    return LegClassification(sensitive=False, reasons=[])


# ---------------------------------------------------------------------------
# Fragment loader
# ---------------------------------------------------------------------------

def _crate_dir(crate_name: str) -> Path:
    """Resolve a crate name to its directory under crates/."""
    return CRATES_DIR / crate_name


def load_fragments() -> List[dict]:
    """Load all fragment legs from .github/feature-matrix.d/*.yml.

    Returns a list of leg dicts (as assembled by assemble-feature-matrix.py).
    Exits with a clear error message if PyYAML is unavailable or fragments
    cannot be loaded.
    """
    try:
        import yaml  # PyYAML — already a dep (assemble-feature-matrix.py)
    except ImportError:
        sys.stderr.write(
            "error: PyYAML is required for loading fragment files "
            "(pip install pyyaml)\n"
        )
        sys.exit(2)

    import glob

    fragments = sorted(glob.glob(str(FRAGMENT_DIR / "*.yml")))
    if not fragments:
        sys.stderr.write(
            "error: no fragment files found under {}\n".format(FRAGMENT_DIR)
        )
        sys.exit(1)

    legs: List[dict] = []
    for fpath in fragments:
        with open(fpath, "r", encoding="utf-8") as fh:
            data = yaml.safe_load(fh)
        if data is None:
            continue
        if not isinstance(data, list):
            sys.stderr.write(
                "error: {}: expected a YAML list, got {}\n".format(
                    fpath, type(data).__name__
                )
            )
            sys.exit(1)
        for leg in data:
            if not isinstance(leg, dict):
                sys.stderr.write(
                    "error: {}: leg is not a dict: {!r}\n".format(fpath, leg)
                )
                sys.exit(1)
            legs.append(leg)
    return legs


# ---------------------------------------------------------------------------
# Classify mode
# ---------------------------------------------------------------------------

def _features_from_leg(leg: dict) -> List[str]:
    """Split the comma-separated features string from a leg dict."""
    raw = leg.get("features", "") or ""
    return [f.strip() for f in str(raw).split(",") if f.strip()]


def cmd_classify(legs: List[dict]) -> None:
    """Print a classification table for all legs to stdout."""
    print("{:<60} {:<12} {}".format("LEG", "TIER", "REASON"))
    print("-" * 100)
    for leg in legs:
        name = leg.get("name", "<unnamed>")
        crate = leg.get("crate", "")
        features = _features_from_leg(leg)
        declared_tier = leg.get("tier", "test")
        tier_reason = leg.get("tier-reason", "")

        clf = classify_leg(_crate_dir(crate), features)

        detector_tier = "test" if clf.sensitive else "check"
        reason_str = "; ".join(clf.reasons) if clf.reasons else ""
        if clf.error_msg:
            reason_str = "ERROR: " + clf.error_msg

        tier_label = declared_tier if declared_tier else "test"
        conflict = ""
        if tier_label == "check" and clf.sensitive and not tier_reason:
            conflict = " [CONFLICT: declared check but detector=sensitive]"

        print("{:<60} {:<12} {}{}".format(
            name[:60],
            "{}->{}{}".format(
                tier_label, detector_tier, " (override)" if tier_reason else ""
            ),
            reason_str[:60] if reason_str else "(none)",
            conflict,
        ))


# ---------------------------------------------------------------------------
# Enforce mode
# ---------------------------------------------------------------------------

def cmd_enforce(legs: List[dict]) -> int:
    """Enforce tier consistency and the sq-vya1 guard.

    Returns an exit code: 0 = all clean, non-zero = violations found.

    Enforcement invariants (design record §4):
    (1) A leg may carry tier: check only if the detector says non-sensitive
        OR the fragment carries an explicit tier-reason: override.
        tier: check + sensitive + no tier-reason => VIOLATION.
    (2) Every (crate, feature) whose leg is sensitive must appear in some
        test:true leg FOR THAT CRATE. A sensitive (crate, feature) with no
        test:true leg => VIOLATION (sq-vya1 guard), unless a leg for that
        (crate, feature) carries a written `test-reason:` override.

    Both invariants key on the (crate, feature) PAIR, never the bare feature
    name: cargo feature names are crate-LOCAL, so a `test: true` leg for
    feature F in crate A says nothing about a sensitive F in crate B. Keying
    on the bare name let exactly that cross-crate collision silently satisfy
    the guard (15 feature names are shared across crates in the live
    fragments), which is the coverage gap this guard exists to catch.
    """
    violations: List[str] = []

    # Map (crate, feature) -> has_test_true_leg (for the sq-vya1 guard)
    feature_has_test_leg: dict = {}
    sensitive_features: set = set()
    # (crate, feature) pairs exempted by a written `test-reason:` on a leg —
    # the coverage lives outside `cargo test` (see the fragment's reason).
    test_reason_exempt: dict = {}

    for leg in legs:
        name = leg.get("name", "<unnamed>")
        crate = leg.get("crate", "")
        features = _features_from_leg(leg)
        declared_tier = str(leg.get("tier", "test") or "test").strip()
        tier_reason = leg.get("tier-reason", "") or ""
        test_reason = str(leg.get("test-reason", "") or "").strip()
        test_flag = bool(leg.get("test", True))

        clf = classify_leg(_crate_dir(crate), features)

        if clf.sensitive:
            for f in features:
                sensitive_features.add((crate, f))

        # Track test:true coverage for all features in this leg
        if test_flag:
            for f in features:
                feature_has_test_leg[(crate, f)] = True
        else:
            for f in features:
                if (crate, f) not in feature_has_test_leg:
                    feature_has_test_leg[(crate, f)] = False
                if test_reason:
                    test_reason_exempt[(crate, f)] = test_reason

        # Invariant (1): tier: check + sensitive + no override => VIOLATION
        if declared_tier == "check" and clf.sensitive and not tier_reason.strip():
            violations.append(
                "VIOLATION(1) leg {!r}: declared tier:check but detector "
                "says sensitive (reasons: {}). Add a tier-reason: override "
                "or promote the leg back to tier:test.".format(
                    name,
                    "; ".join(clf.reasons) or clf.error_msg or "unknown",
                )
            )

    # Invariant (2): every sensitive (crate, feature) must have a test:true leg
    # in ITS OWN crate — a same-named feature elsewhere does not count.
    for crate, f in sorted(sensitive_features):
        if feature_has_test_leg.get((crate, f), False):
            continue
        if (crate, f) in test_reason_exempt:
            continue
        violations.append(
            "VIOLATION(2) feature {!r} of crate {!r} is sensitive (has "
            "feature-gated tests) but appears in no test:true leg for that "
            "crate — the sq-vya1 guard. Add or restore a test:true leg for "
            "this crate's feature, or declare a written `test-reason:` on "
            "the leg if the coverage genuinely lives outside `cargo "
            "test`.".format(f, crate)
        )

    if violations:
        for v in violations:
            sys.stderr.write(v + "\n")
        sys.stderr.write(
            "\n{} enforcement violation(s) found.\n".format(len(violations))
        )
        return 1

    print("OK: {} legs checked, 0 enforcement violations.".format(len(legs)))
    return 0


# ---------------------------------------------------------------------------
# Report-unlegged mode
# ---------------------------------------------------------------------------

def cmd_report_unlegged(legs: List[dict]) -> None:
    """Advisory: report Cargo.toml opt-in features with no leg.

    Compares the set of [features] in each crate's Cargo.toml against the
    legs declared in the fragment files, excluding the SCOPE_ALLOWLIST.
    Output is purely informational (exit 0 always).
    """
    # Collect all features covered by any leg
    legged: set = set()
    for leg in legs:
        for f in _features_from_leg(leg):
            legged.add(f)

    # Scan Cargo.tomls for declared features
    if not CRATES_DIR.is_dir():
        sys.stderr.write(
            "warning: crates directory not found: {}\n".format(CRATES_DIR)
        )
        return

    unlegged: List[str] = []
    for cargo_toml in CRATES_DIR.glob("*/Cargo.toml"):
        try:
            text = cargo_toml.read_text(encoding="utf-8", errors="replace")
        except OSError:
            continue
        in_features = False
        for line in text.splitlines():
            stripped = line.strip()
            if stripped == "[features]":
                in_features = True
                continue
            if stripped.startswith("[") and stripped != "[features]":
                in_features = False
            if not in_features:
                continue
            # Feature declaration: name = [...]  or  name = []
            m = re.match(r"^([a-zA-Z][a-zA-Z0-9_\-]*)\s*=", stripped)
            if m:
                fname = m.group(1)
                if fname == "default":
                    continue
                if fname in legged:
                    continue
                if fname in SCOPE_ALLOWLIST:
                    continue
                unlegged.append("{}/{}".format(cargo_toml.parent.name, fname))

    if unlegged:
        print(
            "Advisory: {} feature(s) declared in Cargo.toml but have no "
            "feature-matrix leg (and are not in the SCOPE allowlist):".format(
                len(unlegged)
            )
        )
        for item in sorted(unlegged):
            print("  {}".format(item))
    else:
        print("Advisory: all declared features are legged or in the SCOPE allowlist.")


# ---------------------------------------------------------------------------
# Entry point
# ---------------------------------------------------------------------------

def main() -> None:
    parser = argparse.ArgumentParser(
        description=(
            "Feature-matrix tier detector. Classifies feature-matrix legs as "
            "sensitive (tier:test) or non-sensitive (check-tier candidate), and "
            "enforces the sq-vya1 GUARD. Design: research/feature-matrix-pyramid.md §4."
        )
    )
    group = parser.add_mutually_exclusive_group(required=True)
    group.add_argument(
        "--classify",
        action="store_true",
        help="Print a classification table for all legs.",
    )
    group.add_argument(
        "--enforce",
        action="store_true",
        help=(
            "Exit non-zero if any tier:check leg is sensitive (no tier-reason "
            "override), or if any sensitive (crate, feature) has no test:true "
            "leg in that crate and no test-reason override (sq-vya1 guard)."
        ),
    )
    group.add_argument(
        "--report-unlegged",
        action="store_true",
        help=(
            "Advisory: report features declared in Cargo.tomls but with no leg "
            "(excluding SCOPE allowlist). Always exits 0."
        ),
    )
    args = parser.parse_args()

    legs = load_fragments()

    if args.classify:
        cmd_classify(legs)
        sys.exit(0)
    elif args.enforce:
        sys.exit(cmd_enforce(legs))
    else:
        cmd_report_unlegged(legs)
        sys.exit(0)


if __name__ == "__main__":
    main()
