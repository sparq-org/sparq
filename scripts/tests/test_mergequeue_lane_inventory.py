#!/usr/bin/env python3
# [OPUS-5] issue #5165 — PINS the merge-queue LANE INVENTORY in
# `docs/branch-protection.md` §*Merge-queue subset* against the `on:` blocks of
# `.github/workflows/`.
#
# WHY a test at all: this is the exact defect #5165 reported, and it is a SILENT
# one. Six workflows have left the queue since the 2026-07-10 profile — five under the
# 2026-07-18 maintainer directive (formal-verification, fuzz, zk-toolchain,
# container-scan, supply-chain) and bench.yml earlier under sq-6vshe.6 — but the
# doc-of-record's prose named only three, the measured lane table in
# research/ci-mergequeue-speedup-2026-07.md §2.1 still listed all six as gate legs, and
# three lanes that gate on the queue today appeared in neither. Nothing went red,
# because a workflow's trigger set and the prose that describes it are two unrelated
# files. The consequence is not cosmetic: the next
# lever in the sq-6vshe program is sized off that lane list, so a stale list silently
# mis-sizes the work (the #5165 report itself under-counted the removals by two).
#
# WHAT IS PINNED, and how strongly. The two lists are NOT equally checkable, so this
# is stated exactly rather than as "both lists are pinned":
#   1. THE TRIGGERING LIST IS PINNED BOTH WAYS (set equality). The live set is fully
#      derivable from the `on:` blocks, so adding or removing a `merge_group` trigger
#      without updating the doc goes red, in either direction. Verified by mutation:
#      dropping `routing-self-tests.yml` from the doc list, and adding a `merge_group:`
#      trigger back to `fuzz.yml`, each turn a named test red.
#   2. THE DROPPED LIST IS PINNED ONE WAY ONLY (soundness, not completeness). Every
#      workflow it names must really not trigger — so a lane that silently regains the
#      trigger is caught. It CANNOT be checked for completeness: "workflows that once
#      had a merge_group trigger" is repo history, not present structure, and most
#      workflows in the tree never had one. Deleting an entry from that list is
#      therefore NOT caught here — verified by mutation, it stays green — and stays a
#      review-time responsibility. Git history is the backstop.
#
# What is deliberately NOT pinned: durations (they belong to a run-history pass, not
# to a hermetic test), whether a lane GATES vs is registered advisory (that is
# `.github/advisory-registry.json`'s job, enforced by check-advisory-registry.py),
# and whether a triggering workflow is operationally ENABLED — `codeql.yml` lists
# `merge_group` but is `disabled_manually`, which is Actions state this test cannot
# see and which the doc records in prose alongside the list.
#
# The trigger parser is imported from test_mergequeue_cache_posture.py rather than
# re-implemented, so the two suites can never disagree about what "triggers on
# merge_group" means.
#
# Hermetic: stdlib only (no PyYAML, no network, no gh).
# Run:  python3 scripts/tests/test_mergequeue_lane_inventory.py

from __future__ import annotations

import re
import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

from test_mergequeue_cache_posture import (  # noqa: E402
    WORKFLOWS,
    triggers_on_merge_group,
)

REPO_ROOT = Path(__file__).resolve().parent.parent.parent
BRANCH_PROTECTION = REPO_ROOT / "docs" / "branch-protection.md"

# The anchor sentences that open each list. They are prose, so they are pinned here
# explicitly: if an edit rewords one, this file fails LOUDLY on the missing anchor
# rather than silently checking an empty set.
TRIGGERS_ANCHOR = "The lanes that DO trigger on `merge_group` today are:"
DROPPED_ANCHOR = "Heavy or independent lanes that already ran and gated on the PR head dropped their"

_WORKFLOW_FILE = re.compile(r"`([A-Za-z0-9_.-]+\.ya?ml)`")


def live_merge_group_workflows() -> set[str]:
    """Filenames of every workflow whose top-level `on:` declares `merge_group`."""
    return {
        p.name
        for p in sorted(WORKFLOWS.glob("*.yml")) + sorted(WORKFLOWS.glob("*.yaml"))
        if triggers_on_merge_group(p)
    }


def paragraph_after(anchor: str) -> str:
    """The blank-line-delimited paragraph that the `anchor` sentence opens."""
    lines = BRANCH_PROTECTION.read_text().split("\n")
    start = next((i for i, l in enumerate(lines) if anchor in l), None)
    if start is None:
        raise AssertionError(
            f"anchor not found in {BRANCH_PROTECTION.name}: {anchor!r}. The prose was "
            "reworded — re-point this test at the renamed sentence, do not delete the "
            "assertion."
        )
    end = next(
        (i for i in range(start + 1, len(lines)) if not lines[i].strip()), len(lines)
    )
    return "\n".join(lines[start:end])


def workflows_named_in(anchor: str) -> set[str]:
    return set(_WORKFLOW_FILE.findall(paragraph_after(anchor)))


class TestNonVacuity(unittest.TestCase):
    """Every assertion below compares discovered sets. If discovery silently returns
    nothing, the comparisons pass while checking NOTHING — so discovery is asserted
    first, against floors low enough to survive a real lane change but high enough to
    catch a parser that has stopped seeing anything."""

    def test_workflow_dir_is_populated(self) -> None:
        self.assertGreater(len(list(WORKFLOWS.glob("*.yml"))), 50)

    def test_some_workflow_triggers_on_merge_group(self) -> None:
        live = live_merge_group_workflows()
        self.assertIn(
            "ci-summary.yml",
            live,
            "ci-summary.yml IS the required gate and must run on the queue ref; the "
            f"on:-block parser found {sorted(live)}",
        )
        self.assertGreaterEqual(len(live), 4)

    def test_both_doc_lists_are_non_empty(self) -> None:
        self.assertGreaterEqual(len(workflows_named_in(TRIGGERS_ANCHOR)), 4)
        self.assertGreaterEqual(len(workflows_named_in(DROPPED_ANCHOR)), 3)


class TestLaneInventoryMatchesWorkflows(unittest.TestCase):
    def test_doc_names_exactly_the_merge_group_triggered_workflows(self) -> None:
        live = live_merge_group_workflows()
        documented = workflows_named_in(TRIGGERS_ANCHOR)
        self.assertEqual(
            documented,
            live,
            "docs/branch-protection.md §Merge-queue subset lists the wrong lane set. "
            f"Triggers but undocumented: {sorted(live - documented)}; documented but "
            f"does not trigger: {sorted(documented - live)}. This list is the "
            "doc-of-record the sq-6vshe speedup levers are sized off "
            "(research/ci-mergequeue-speedup-2026-07.md §2.1) — update the prose in "
            "the SAME change that adds or removes a merge_group trigger.",
        )

    def test_lanes_documented_as_dropped_really_dropped_the_trigger(self) -> None:
        live = live_merge_group_workflows()
        for name in sorted(workflows_named_in(DROPPED_ANCHOR)):
            with self.subTest(workflow=name):
                self.assertTrue(
                    (WORKFLOWS / name).exists(),
                    f"{name} is named as a dropped merge-queue lane but no such "
                    "workflow exists",
                )
                self.assertNotIn(
                    name,
                    live,
                    f"{name} is documented as having DROPPED its merge_group trigger, "
                    "but its on: block still declares one. Either the trigger came "
                    "back (move it to the other list) or the prose is wrong.",
                )


if __name__ == "__main__":
    unittest.main()
