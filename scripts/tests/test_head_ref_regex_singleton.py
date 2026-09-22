#!/usr/bin/env python3
# [OPUS-5] The registry review-lane HEAD-REF gate is a SINGLETON BY VALUE, not by import.
# 🤖 SPARQ agent. Target issue #5462.
#
# WHY THE REPLICATION EXISTS AND CANNOT BE REFACTORED AWAY. The registry's
# `enumerate_review_items` admits a PR into the review lane only if its head ref matches
# one regex. Several sparq scripts must reproduce that same admission test to reason about
# which PRs a reviewer can ever see. They cannot share a constant: each workflow
# sparse-checks-out only its own script, so review_lane_alarm.py, ready-issues.py and
# batch-merge.py have no importable common module at runtime. The copies are deliberate.
#
# WHAT IS NOT DELIBERATE is drift between them — two components disagreeing about which
# PRs the review lane can see. MEASURED ON THIS TREE before this file existed: three
# scripts held a compiled copy, and the only pin of any kind was
# test_review_lane_alarm.py::test_the_regex_is_byte_identical_to_the_registry_gate, which
# asserts ONE script against a literal. Nothing bound the copies to EACH OTHER, so
# ready-issues.py and batch-merge.py could each drift on their own with a green CI.
# (#5462 describes a fourth copy in verdict-bridge.py and a verdict-bridge/alarm pin added
# by #4677. Neither is on main — #4677 is unmerged as of this commit — so neither is
# claimed here. When #4677 lands it adds a copy, TestReplicationCensus goes red on
# arrival, and that PR declares the new site. That is the intended behaviour, not a
# conflict.)
#
# This file binds every copy to one string along three independent axes:
#
# 1. RUNTIME (TestCompiledPatternsAgree) — each script is imported and its COMPILED
#    pattern is compared to the canonical string, and to the other copies. This is the
#    axis that catches a real behavioural divergence, because it reads the object the
#    script actually matches with, not a comment about it. Change any one copy's regex
#    => red.
#
# 2. SOURCE (TestEveryInRepoCopyIsCanonical) — the whole tree is scanned for anchored
#    `sparq-agent/issue-` occurrences, INCLUDING the ones in prose: workflow headers
#    and docstrings that quote the gate are load-bearing documentation, and a stale quote
#    misleads the next reader as effectively as a stale regex. Any occurrence that is
#    neither the canonical regex nor the canonical `<n>` schematic => red.
#
# 3. CENSUS (TestReplicationCensus) — the discovered site set is pinned to a declared
#    inventory, so a copy added anywhere in the tree goes red on arrival and has to be
#    declared here. That is the property #5462 asks for: the check must not silently
#    widen to cover a new copy without a human noticing the replication grew.
#
# ANTI-VACUITY. The scan asserts it found every declared file (a broken walk root or a
# broken extraction regex would otherwise make every assertion pass over an empty set),
# and the suite pins its own docs-quality.yml call site so it cannot leave CI unnoticed.
#
# Stdlib only. Run:
#   python3 scripts/tests/test_head_ref_regex_singleton.py

from __future__ import annotations

import importlib.util
import re
import sys
import unittest
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent.parent
SCRIPTS = REPO_ROOT / "scripts"
DOCS_QUALITY = REPO_ROOT / ".github" / "workflows" / "docs-quality.yml"

# The registry's `enumerate_review_items` head-ref admission regex, verbatim. If the
# registry ever loosens or tightens this, EVERY site below must move in the same commit —
# that is the entire point of this file.
CANONICAL = r"^sparq-agent/issue-([1-9][0-9]*)-"
# The human-readable schematic used in prose that explains the gate without quoting the
# metacharacters. Pinned too, so the docs cannot drift into describing a different shape.
CANONICAL_SCHEMATIC = "^sparq-agent/issue-<n>-"

# The scripts that hold a COMPILED copy, and the attribute each one calls it. Deliberately
# spelled out rather than discovered: a script that drops its copy entirely is also a
# change worth reviewing, and this list goes red on that too.
COMPILED_COPIES = (
    ("review_lane_alarm.py", "REGISTRY_HEAD_REF_RE"),
    ("ready-issues.py", "_LINK_HEAD"),
    ("batch-merge.py", "WORKER_ISSUE_RE"),
)

# The declared replication census: repo-relative path -> number of anchored occurrences.
# Counts include prose quotes, not just compiled regexes. Adding a copy is allowed; adding
# it SILENTLY is not — bump the count here in the same commit and a reviewer sees the
# replication grow.
DECLARED_SITES = {
    ".github/workflows/batch-merge.yml": 1,
    ".github/workflows/review-alarm.yml": 1,
    "scripts/batch-merge.py": 2,
    "scripts/ready-issues.py": 1,
    "scripts/review_lane_alarm.py": 3,
    "scripts/tests/test_head_ref_regex_singleton.py": 2,
    "scripts/tests/test_review_lane_alarm.py": 1,
}

# An anchored occurrence runs from the `^` to the first quote/backtick/whitespace. The `^`
# is what separates a PATTERN from a real branch name: test fixtures carry unanchored refs
# like sparq-agent/issue-2908-30221671021-1, which must not be mistaken for copies.
# The anchor is spelled in two pieces so the scanner does not match its own source line —
# a self-match would be reported as a non-canonical copy on every run.
_ANCHOR = "^" + "sparq-agent/issue-"
_OCCURRENCE_RE = re.compile(re.escape(_ANCHOR) + r"[^\"'`\s]*")

_SKIP_DIRS = frozenset({
    ".git", "target", "node_modules", "__pycache__", ".venv", "venv", ".next",
    "out", "dist", "vendor", ".mypy_cache", ".pytest_cache", ".ruff_cache",
})
_MAX_BYTES = 4 * 1024 * 1024


def _load(name: str, filename: str):
    """Import a hyphen-named script by path (they are not importable as modules)."""
    path = SCRIPTS / filename
    spec = importlib.util.spec_from_file_location(name, path)
    assert spec is not None and spec.loader is not None, f"cannot load {path}"
    module = importlib.util.module_from_spec(spec)
    sys.modules[name] = module
    spec.loader.exec_module(module)
    return module


def _scan_tree():
    """repo-relative path -> list of anchored occurrence strings, over the whole tree."""
    found: dict[str, list[str]] = {}
    for path in sorted(REPO_ROOT.rglob("*")):
        if not path.is_file() or path.is_symlink():
            continue
        if any(part in _SKIP_DIRS for part in path.relative_to(REPO_ROOT).parts):
            continue
        try:
            if path.stat().st_size > _MAX_BYTES:
                continue
            raw = path.read_bytes()
        except OSError:
            continue
        if b"\x00" in raw[:8192]:  # binary
            continue
        hits = _OCCURRENCE_RE.findall(raw.decode("utf-8", errors="ignore"))
        if hits:
            found[path.relative_to(REPO_ROOT).as_posix()] = hits
    return found


class TestCompiledPatternsAgree(unittest.TestCase):
    """The axis that catches real behavioural divergence: the objects that do the matching.

    #4677 bound one pair this way and left the rest unbound. All of them are bound now.
    """

    def test_every_compiled_copy_is_byte_identical_to_the_canonical(self):
        for filename, attr in COMPILED_COPIES:
            with self.subTest(script=filename):
                module = _load(f"{filename}__head_ref_pin", filename)
                self.assertTrue(
                    hasattr(module, attr),
                    f"scripts/{filename} no longer defines {attr}. If the copy moved or was "
                    f"renamed, update COMPILED_COPIES here in the same commit — an unbound "
                    f"copy is how the review lane and its consumers drift apart.",
                )
                self.assertEqual(
                    getattr(module, attr).pattern, CANONICAL,
                    f"scripts/{filename}:{attr} has drifted from the registry's "
                    f"`enumerate_review_items` head-ref gate. Two components disagreeing "
                    f"about which PRs are visible to the review lane is the #4677 defect.",
                )

    def test_the_copies_agree_with_each_other_and_not_merely_with_this_file(self):
        """Guards the case where CANONICAL itself is edited to match one drifted copy."""
        patterns = {
            filename: getattr(_load(f"{filename}__head_ref_agree", filename), attr).pattern
            for filename, attr in COMPILED_COPIES
        }
        self.assertEqual(
            len(set(patterns.values())), 1,
            f"the compiled head-ref copies disagree with each other: {patterns}",
        )


class TestEveryInRepoCopyIsCanonical(unittest.TestCase):
    """The source axis — covers prose quotes the runtime axis cannot see."""

    @classmethod
    def setUpClass(cls):
        cls.found = _scan_tree()

    def test_the_scan_is_not_vacuous(self):
        """A broken walk root or extraction regex must fail loudly, not pass over nothing."""
        missing = sorted(set(DECLARED_SITES) - set(self.found))
        self.assertEqual(
            missing, [],
            f"the scan found no anchored occurrence in {missing}. Either the copy was "
            f"removed (drop it from DECLARED_SITES) or the scanner is broken — in which "
            f"case every other assertion in this file is silently vacuous.",
        )

    def test_every_occurrence_is_the_canonical_regex_or_the_canonical_schematic(self):
        allowed = {CANONICAL, CANONICAL_SCHEMATIC}
        bad = sorted(
            (path, hit)
            for path, hits in self.found.items()
            for hit in hits
            if hit not in allowed
        )
        self.assertEqual(
            bad, [],
            f"these in-repo copies of the review-lane head-ref gate are not byte-identical "
            f"to `{CANONICAL}` (or its `<n>` schematic): {bad}. Every copy — comment, "
            f"docstring, workflow header or compiled regex — must move together.",
        )


class TestReplicationCensus(unittest.TestCase):
    """A new copy must be DECLARED. Silent widening is how the census stopped being true."""

    def test_the_discovered_sites_match_the_declared_inventory(self):
        actual = {path: len(hits) for path, hits in _scan_tree().items()}
        self.assertEqual(
            actual, DECLARED_SITES,
            "the set of in-repo review-lane head-ref copies changed. Adding a copy is fine "
            "(the scripts genuinely cannot import a shared constant — each workflow "
            "sparse-checks-out only its own script), but declare it in DECLARED_SITES in "
            "the same commit so the replication stays visible and stays pinned.",
        )


class TestThisSuiteStaysWiredIntoCI(unittest.TestCase):
    """Anti-vacuity anchor: an unrun gate is not a gate."""

    def test_docs_quality_invokes_this_file(self):
        self.assertIn(
            "python3 scripts/tests/test_head_ref_regex_singleton.py",
            DOCS_QUALITY.read_text(encoding="utf-8"),
            "docs-quality.yml must invoke test_head_ref_regex_singleton.py",
        )


if __name__ == "__main__":
    unittest.main(verbosity=2)
