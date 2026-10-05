#!/usr/bin/env python3
# [OPUS-5] Readiness VISIBILITY suite — the "can the dispatcher see this work?" contract.
#
# The registry's dispatch.yml clones this repo and runs THIS repo's scripts/ready-issues.py +
# scripts/dispatch-plan.py, so a defect here is a fleet-wide dispatch defect. The suite pins the
# three things that made ready work invisible, each of which was measured live on sparq-org/sparq:
#
#   1. OCCUPANCY ATTRIBUTION — `packages_of` maps "no area: label" to the GLOBAL partition.
#      Applied to a CANDIDATE that is correct fail-closed behaviour (cross-cutting work must
#      serialize). Applied to an OCCUPANT it inverts: "cannot attribute" became "seizes every
#      crate". Nothing in the pipeline puts `area:` labels on PRs — all 60 open sparq PRs had
#      none — so ONE unlabelled PR held __global__ and drove the local frontier to zero.
#   2. status:in-progress-review — absent from BUSY_STATUS and from the reserve branch, so such
#      an issue was neither excluded nor reserving: a double-dispatch on both halves.
#   3. LOCAL/ORCHESTRATOR PARITY — dispatch.yml builds its readiness input from ISSUES ONLY and
#      suppresses issues covered by a linked open PR EXCEPT the in-flight ones, which it keeps
#      so they still reserve their package. The local CLI must preview THAT frontier; when it
#      did not, the two disagreed 0-vs-6 on the same live snapshot. Review round 2 found the
#      first fix still wrong for the dominant live shape (it dropped EVERY linked row, freeing
#      the crate an in-review issue is actively occupying) and the parity claim guarded only by
#      a source-substring assertion, so the regression survived mutation. The parity tests below
#      therefore run the REAL main()/--diagnose, stubbing only the two network calls.
#
# Plus the YAML seam: routing-self-tests.yml listed scripts/ready-issues.py in its `paths:` filter
# but never INVOKED its --self-test, so all of that script's assertions were dead in this repo's
# CI and only ran later inside the registry's dispatch tick (where a failure breaks EVERY target).
from __future__ import annotations

import contextlib
import importlib.util
import io
import json
import re
import subprocess
import sys
import tempfile
import tomllib
import unittest
from pathlib import Path
from unittest import mock

import yaml

REPO_ROOT = Path(__file__).resolve().parent.parent.parent
SCRIPTS = REPO_ROOT / "scripts"
WORKFLOW = REPO_ROOT / ".github" / "workflows" / "routing-self-tests.yml"


def _load(name: str, filename: str):
    path = SCRIPTS / filename
    spec = importlib.util.spec_from_file_location(name, path)
    assert spec is not None and spec.loader is not None, f"cannot load {path}"
    module = importlib.util.module_from_spec(spec)
    sys.modules[name] = module
    spec.loader.exec_module(module)
    return module


ready = _load("ready_issues_under_test", "ready-issues.py")
plan = _load("dispatch_plan_under_test", "dispatch-plan.py")
triage = _load("triage_under_test", "triage.py")
# retriage.py imports `triage` by name from its own directory, so SCRIPTS must be importable.
sys.path.insert(0, str(SCRIPTS))
retriage = _load("retriage_under_test", "retriage.py")

READY = ["status:ready", "role:impl"]


def iss(number, labels, blockers=0, state="OPEN"):
    return {"number": number, "state": state, "labels": list(labels),
            "open_blockers": blockers}


def pr(number, labels, draft=False):
    return {"number": number, "state": "OPEN", "labels": list(labels),
            "pull_request": {}, "draft": draft}


def numbers(rows):
    return [row["number"] for row in rows]


def quiet(_message):
    pass


class TestUnlabelledOccupantAttribution(unittest.TestCase):
    """Class 1 — an occupant we cannot attribute must reserve NOTHING, not EVERYTHING."""

    def test_unlabelled_pr_does_not_seize_the_global_partition(self):
        # THE live defect: one area-less PR made the entire frontier empty.
        waiting = iss(20, READY + ["priority:P1", "area:sparq-core"])
        unlabelled = pr(70, [])
        self.assertEqual(
            numbers(ready.compute_ready([unlabelled, waiting], conflict_log=quiet)), [20],
            "an area-less PR must not hold __global__ and stall the whole fleet")

    def test_unlabelled_pr_leaves_every_unrelated_crate_dispatchable(self):
        # The blast radius, not just one row: with the bug, ALL of these vanished at once.
        # [OPUS-5] sparq#4336: the keys were synthetic `area:crate-21/22/23`. Those share the head
        # segment `crate`, and containment-aware conflict() now (correctly) treats a shared head
        # as one partition, so the fixture no longer modelled "three UNRELATED crates" at all.
        # Switched to three real, disjoint workspace crates — the assertion is unchanged in force.
        board = [pr(70, [])] + [
            iss(n, READY + ["priority:P1", f"area:{a}"])
            for n, a in ((21, "sparq-core"), (22, "sparq-hdt"), (23, "sparq-geo"))]
        self.assertEqual(
            numbers(ready.compute_ready(board, conflict_log=quiet)), [21, 22, 23])

    def test_area_labelled_pr_still_reserves_exactly_its_crate(self):
        # The fix must not become "PRs never reserve": a DECLARED area is still occupancy.
        waiting = iss(20, READY + ["priority:P1", "area:sparq-core"])
        other = iss(21, READY + ["priority:P1", "area:sparq-hdt"])
        board = [pr(70, ["area:sparq-core"]), waiting, other]
        self.assertEqual(numbers(ready.compute_ready(board, conflict_log=quiet)), [21],
                         "a PR labelled area:sparq-core must still hold sparq-core (only)")

    def test_area_less_ISSUE_candidate_still_fails_closed_to_global(self):
        # The asymmetry is deliberate — the CANDIDATE-side global rule is untouched.
        self.assertEqual(ready.packages_of({"role:impl"}), {ready.GLOBAL})
        self.assertEqual(ready.declared_packages({"role:impl"}), set())
        board = [iss(30, READY + ["priority:P0"]),
                 iss(31, READY + ["priority:P1", "area:sparq-core"])]
        self.assertEqual(numbers(ready.compute_ready(board, conflict_log=quiet)), [30],
                         "an area-less READY ISSUE must still serialize against everything")

    def test_unlabelled_in_progress_issue_also_reserves_nothing(self):
        # Same attribution rule on the issue-occupancy path, not just the PR path.
        board = [iss(72, ["status:in-progress"]),
                 iss(20, READY + ["priority:P1", "area:sparq-core"])]
        self.assertEqual(numbers(ready.compute_ready(board, conflict_log=quiet)), [20])


class TestSubCrateContainment(unittest.TestCase):
    """sparq#4336 — the UNDER-serialisation defect: a region key never overlapped its crate.

    `conflict()` compared partition keys by exact-string set overlap, so `area:sparq-server-http`
    did not overlap `area:sparq-server`. Both entered the frontier in one tick, and a sub-crate
    issue entered despite an open PR holding the parent crate: two workers, one crate, no lock.
    57.1% of same-crate 24h PR pairs share a file (research/crate-region-parallelism.md §4), and a
    semantic collision (both compile, both pass, together broken) is invisible to git.

    Every test here goes END-TO-END through compute_ready() where it can, so deleting
    `keys_conflict`, flattening `partition_path` to the identity, or reverting `conflict()` to
    `pkgs & blockers.keys()` reds this class.
    """

    # The four live labels that are NOT workspace crates -> they are regions inside one.
    CONTAINED = {"sparq-server-http": "sparq-server",
                 "sparq-core-nt-dict": "sparq-core",
                 "sparq-core-store": "sparq-core",
                 "sparq-engine-exec": "sparq-engine"}
    # The fifth label from the issue's table. It IS a real workspace crate at origin/main, so the
    # table was already stale — see test_conformance_floors_is_a_real_crate_and_must_stay_split.
    NOT_CONTAINED = "sparq-conformance-floors"

    def test_every_live_sub_crate_label_conflicts_with_its_parent_crate(self):
        for child, parent in self.CONTAINED.items():
            with self.subTest(child=child):
                self.assertTrue(ready.keys_conflict(child, parent),
                                f"{child} names a region inside {parent}")
                self.assertTrue(ready.keys_conflict(parent, child), "conflict must be symmetric")
                self.assertEqual(ready.partition_path(child), (parent,))

    def test_parent_crate_issue_blocks_a_sub_crate_issue_in_the_same_tick(self):
        # Ordering A: the PARENT is selected first (lower number wins the priority tie).
        board = [iss(1, READY + ["priority:P1", "area:sparq-server"]),
                 iss(2, READY + ["priority:P1", "area:sparq-server-http"])]
        self.assertEqual(numbers(ready.compute_ready(board, conflict_log=quiet)), [1],
                         "area:sparq-server-http must not enter alongside area:sparq-server")

    def test_sub_crate_issue_blocks_a_parent_crate_issue_in_the_same_tick(self):
        # Ordering B: the CHILD is selected first. Both orderings, per the acceptance criteria.
        board = [iss(1, READY + ["priority:P1", "area:sparq-server-http"]),
                 iss(2, READY + ["priority:P1", "area:sparq-server"])]
        self.assertEqual(numbers(ready.compute_ready(board, conflict_log=quiet)), [1],
                         "area:sparq-server must not enter alongside area:sparq-server-http")

    def test_open_pr_holding_the_parent_crate_blocks_a_sub_crate_issue(self):
        # The OCCUPANCY half of the defect, demonstrated live in the issue.
        board = [pr(70, ["area:sparq-server"]),
                 iss(20, READY + ["priority:P1", "area:sparq-server-http"])]
        self.assertEqual(ready.compute_ready(board, conflict_log=quiet), [],
                         "an open PR on sparq-server must hold the whole crate")

    def test_open_pr_holding_a_sub_crate_blocks_a_parent_crate_issue(self):
        board = [pr(70, ["area:sparq-core-store"]),
                 iss(20, READY + ["priority:P1", "area:sparq-core"])]
        self.assertEqual(ready.compute_ready(board, conflict_log=quiet), [],
                         "an open PR on a region must hold its containing crate")

    def test_sibling_sub_crate_keys_of_one_parent_conflict_with_each_other(self):
        # A sibling hole is the SAME defect one level down: sparq-core-store and
        # sparq-core-nt-dict are both files in crates/sparq-core. Neither string is a prefix of
        # the other, so a naive prefix-on-the-raw-key rule would pass the parent tests and still
        # put two workers in sparq-core.
        self.assertTrue(ready.keys_conflict("sparq-core-store", "sparq-core-nt-dict"))
        board = [iss(1, READY + ["priority:P1", "area:sparq-core-store"]),
                 iss(2, READY + ["priority:P1", "area:sparq-core-nt-dict"])]
        self.assertEqual(numbers(ready.compute_ready(board, conflict_log=quiet)), [1])

    def test_unrelated_crates_still_do_not_conflict(self):
        # The fix must not collapse everything into one lock and destroy the frontier.
        for a, b in (("sparq-core", "sparq-engine"), ("sparq-server", "sparq-hdt"),
                     ("sparq-zk", "sparq-mpc"), ("site", "bench")):
            with self.subTest(pair=(a, b)):
                self.assertFalse(ready.keys_conflict(a, b))
        board = [iss(n, READY + ["priority:P1", f"area:{a}"]) for n, a in
                 ((1, "sparq-core"), (2, "sparq-engine"), (3, "sparq-hdt"), (4, "site"))]
        self.assertEqual(numbers(ready.compute_ready(board, conflict_log=quiet)), [1, 2, 3, 4])

    def test_real_sibling_crates_sharing_a_name_prefix_stay_independent(self):
        # `sparq-engine-serialize` is a REAL crate directory, so it is NOT a region inside
        # `sparq-engine` even though the strings nest. Only the workspace tree can tell these
        # apart from `sparq-engine-exec`; a hand-written containment table cannot.
        for crate in ("sparq-engine-serialize", "sparq-engine-service", "sparq-reason-dl",
                      "sparq-lws-core", "sparq-zk-compose"):
            with self.subTest(crate=crate):
                self.assertEqual(ready.partition_path(crate), (crate,))
        self.assertFalse(ready.keys_conflict("sparq-engine", "sparq-engine-serialize"))
        self.assertFalse(ready.keys_conflict("sparq-engine-exec", "sparq-engine-serialize"))
        self.assertTrue(ready.keys_conflict("sparq-engine", "sparq-engine-exec"),
                        "the non-crate sibling must still collapse into the crate")

    def test_conformance_floors_is_a_real_crate_and_must_stay_split(self):
        # Issue #4336's table lists area:sparq-conformance-floors as a region inside
        # crates/sparq-conformance. It is NOT: crates/sparq-conformance-floors is a workspace
        # member at origin/main. The table was stale before it was written down, which is the
        # whole argument for deriving containment from the tree instead of listing it.
        self.assertTrue((REPO_ROOT / "crates" / self.NOT_CONTAINED / "Cargo.toml").is_file())
        self.assertEqual(ready.partition_path(self.NOT_CONTAINED), (self.NOT_CONTAINED,))
        self.assertFalse(ready.keys_conflict("sparq-conformance", self.NOT_CONTAINED))


class TestTotalPartitionMapping(unittest.TestCase):
    """The durable half: an UNKNOWN key must fail SAFE with no code change.

    Under-serialisation corrupts; over-serialisation only delays. So the map is total and every
    unrecognised key resolves UPWARD to the coarsest thing that could contain it.
    """

    def test_newly_invented_sub_crate_key_resolves_to_its_crate(self):
        # Nobody registers this label anywhere. It must still be caught by the lock.
        for invented in ("sparq-server-zzz", "sparq-core-brand-new-region",
                         "sparq-engine-exec-v2", "sparq-hdt-writer"):
            with self.subTest(key=invented):
                parent = ready.partition_path(invented)[0]
                self.assertIn(parent, ready.workspace_roots())
                self.assertTrue(invented.startswith(parent + "-"))
        board = [pr(70, ["area:sparq-server"]),
                 iss(20, READY + ["priority:P1", "area:sparq-server-zzz"])]
        self.assertEqual(ready.compute_ready(board, conflict_log=quiet), [],
                         "an invented region key must fail SAFE into its crate's lock")

    def test_unknown_key_under_an_unknown_parent_still_resolves_upward(self):
        # `upstream` is not a directory, so the workspace cannot confirm it — the key's own head
        # segment is then the coarsest partition it names, and honouring it OVER-reserves.
        self.assertEqual(ready.partition_path("upstream-noir"), ("upstream",))
        self.assertTrue(ready.keys_conflict("upstream", "upstream-noir"))
        self.assertTrue(ready.keys_conflict("upstream-noir", "upstream-oxigraph"))

    def test_single_segment_unknown_key_keeps_its_own_partition(self):
        # It names nothing narrower than itself, so it cannot be under-serialising, and routing it
        # to __global__ would be a fleet stall, not a fix: MEASURED on the live 2026-07-26
        # snapshot, a __global__ terminal fallback took the frontier from 6 to 0 because open PR
        # #4238 carries `area:deps` and `deps` is not a directory.
        for key in ("upstream", "deps", "workspace", "release", "accuracy", "frobnicate"):
            with self.subTest(key=key):
                self.assertEqual(ready.partition_path(key), (key,))
        board = [iss(1, READY + ["priority:P1", "area:deps"]),
                 iss(2, READY + ["priority:P1", "area:sparq-core"])]
        self.assertEqual(numbers(ready.compute_ready(board, conflict_log=quiet)), [1, 2])

    def test_degenerate_key_fails_all_the_way_closed_to_global(self):
        # The terminal of the resolution chain. A key with no head segment to latch onto reserves
        # EVERYTHING rather than nothing.
        for degenerate in ("", "-", "---"):
            with self.subTest(key=degenerate):
                self.assertEqual(ready.partition_path(degenerate), ())
                self.assertTrue(ready.keys_conflict(degenerate, "sparq-core"))
                self.assertTrue(ready.keys_conflict(degenerate, ready.GLOBAL))

    def test_global_conflicts_with_every_key_and_the_mapping_is_total(self):
        self.assertEqual(ready.partition_path(ready.GLOBAL), ())
        for key in ("sparq-core", "sparq-server-http", "site-papers", "deps", "zz-top"):
            with self.subTest(key=key):
                self.assertTrue(ready.keys_conflict(ready.GLOBAL, key))
                self.assertTrue(ready.keys_conflict(key, ready.GLOBAL))
                self.assertIsInstance(ready.partition_path(key), tuple)

    def test_containment_is_reflexive_and_symmetric(self):
        for key in ("sparq-core", "sparq-core-store", ready.GLOBAL, "deps"):
            with self.subTest(key=key):
                self.assertTrue(ready.keys_conflict(key, key))
        self.assertEqual(ready.keys_conflict("sparq-core", "sparq-core-store"),
                         ready.keys_conflict("sparq-core-store", "sparq-core"))


class TestWorkspaceDerivedRoots(unittest.TestCase):
    """Roots are READ FROM THE TREE. A table would already be stale (see conformance-floors)."""

    def test_roots_come_from_the_real_repository_tree(self):
        roots = ready.workspace_roots()
        for crate in (p.name for p in (REPO_ROOT / "crates").iterdir() if p.is_dir()):
            self.assertIn(crate, roots, f"crate {crate} must be a recognised partition root")
        for top in ("site", "bench", "scripts", "research", "crates"):
            self.assertIn(top, roots)
        self.assertNotIn(".github", roots, "dot-directories are not area: key names")

    def test_a_crate_added_with_no_code_change_is_recognised_immediately(self):
        # The anti-staleness property, exercised against a synthetic tree rather than asserted.
        with tempfile.TemporaryDirectory() as tmp:
            base = Path(tmp)
            (base / "crates" / "sparq-brandnew").mkdir(parents=True)
            (base / "crates" / "sparq-brandnew-region").mkdir(parents=True)
            (base / "site").mkdir()
            # The fixture must now declare its members: `workspace_roots` asserts the scanned tree
            # against the manifest (`assert_workspace_tree`). This is a FIXTURE change only — the
            # property under test is unchanged, and the two crates below are still recognised with
            # no code change, which is the whole point of the test.
            (base / "Cargo.toml").write_text(
                '[workspace]\nresolver = "2"\n'
                'members = ["crates/sparq-brandnew", "crates/sparq-brandnew-region"]\n',
                encoding="utf-8")
            roots = ready.workspace_roots(str(base))
            self.assertEqual(roots, {"crates", "site", "sparq-brandnew", "sparq-brandnew-region"})
            # present in the tree -> its own partition; absent -> collapses into its parent
            self.assertEqual(ready.partition_path("sparq-brandnew-region", roots),
                             ("sparq-brandnew-region",))
            self.assertEqual(ready.partition_path("sparq-brandnew-unlisted", roots),
                             ("sparq-brandnew",))
            self.assertFalse(ready.keys_conflict("sparq-brandnew", "sparq-brandnew-region", roots))
            self.assertTrue(ready.keys_conflict("sparq-brandnew", "sparq-brandnew-unlisted", roots))

    def test_an_empty_root_set_over_reserves_rather_than_under_reserves(self):
        # UNCHANGED PROPERTY, kept because it is the reason the collapse is not a CORRECTNESS bug:
        # given an empty root set, every key falls back to its head segment, so all `sparq-*` keys
        # land on `sparq` and unrelated crates OVER-reserve. Over-serialisation costs delay;
        # under-serialisation double-dispatches. That direction still holds and is still tested —
        # here against an EXPLICIT root set, which is the caller-supplies-the-roots path and is
        # deliberately not guarded.
        roots = set()
        self.assertEqual(ready.partition_path("sparq-core", roots), ("sparq",))
        self.assertTrue(ready.keys_conflict("sparq-core", "sparq-engine", roots),
                        "with no roots, unrelated crates must OVER-reserve")

    def test_an_unreadable_tree_now_refuses_instead_of_collapsing_silently(self):
        """[OPUS-5] A DOCUMENTED TRADE, DELIBERATELY REVISITED — see the PR body.

        This test used to assert that an unreadable tree returns `set()` and lets every key
        collapse onto `sparq`, on the stated grounds that this is "a fleet-wide slowdown, never a
        double dispatch". The SAFETY half of that claim is correct and is still tested directly
        above; it is not what changed.

        What changed is the measured cost of the silence. MEASURED 2026-07-28 against a live sparq
        snapshot with sparq's own engine: the same board, same labels and same code planned a ready
        frontier of 4 on the real tree and 2 on a scripts-only tree, with 185 of 377 refusals
        attributed to a single phantom `sparq-algos` partition — and NOTHING distinguished the two
        runs. `candidates` and `top-contended` are computed from label sets, so the census line
        reads exactly the same either way. "A fleet-wide slowdown" that no instrument can see is
        indistinguishable from a busy board, which is how it survived a whole investigation before
        being caught by accident.

        So the unattributable case is now LOUD rather than silent. The engine refuses to partition
        a tree it cannot verify instead of emitting a frontier whose meaning it cannot vouch for.
        """
        with self.assertRaises(ready.DegeneratePartitionRoots) as caught:
            ready.workspace_roots("/nonexistent/sparq-checkout")
        self.assertIn("refusing to partition", str(caught.exception))
        # ...and it names the tree it scanned, so the operator can see WHICH checkout was wrong.
        self.assertIn("/nonexistent/sparq-checkout", str(caught.exception))

    def test_dump_partitions_exports_the_mapping_for_the_registry_parity_fixture(self):
        # The registry's dispatch.yml mirrors this key space in `busy_packages_of_pulls`; the two
        # must agree or the fleet double-dispatches. This is the machine-readable contract.
        out = subprocess.run(
            [sys.executable, str(SCRIPTS / "ready-issues.py"), "--dump-partitions",
             "sparq-server-http", "sparq-engine-serialize", "deps"],
            capture_output=True, text=True, check=True).stdout
        dumped = json.loads(out)
        self.assertIn("sparq-core", dumped["roots"])
        self.assertEqual(dumped["resolved"]["sparq-server-http"], ["sparq-server"])
        self.assertEqual(dumped["resolved"]["sparq-engine-serialize"], ["sparq-engine-serialize"])
        self.assertEqual(dumped["resolved"]["deps"], ["deps"])

    def test_dump_partitions_reproduces_the_declared_registry_parity_fixture(self):
        """sparq#4365 — the CLI artifact and the declared fixture are ONE expectation, not two.

        `dispatch-plan.py --self-test` asserts the loaded planner MODULE against
        `orchestration/registry-contract.toml`'s `[partition_resolver.parity_fixture]`. A registry
        leg that would rather diff a file than call the module reads it from
        `--dump-partitions`, which is a different code path (argv parsing, JSON encoding, a fresh
        process with its own tree scan). If those two ever disagree the contract has two answers
        and the second repo can pick the wrong one, so both are pinned to the same table here.
        """
        fixture = tomllib.loads(
            (REPO_ROOT / "orchestration" / "registry-contract.toml").read_text(encoding="utf-8")
        )["partition_resolver"]["parity_fixture"]
        # Every key #4365 enumerates must be declared — a fixture is only as good as its coverage,
        # and silently dropping a row would leave this test green over a shrinking contract.
        self.assertEqual(set(fixture), {
            "sparq-server-http", "sparq-core-store", "sparq-core-nt-dict", "sparq-engine-exec",
            "sparq-engine-serialize", "sparq-conformance-floors", "deps", "upstream-noir", ""})
        keys = sorted(fixture)
        out = subprocess.run(
            [sys.executable, str(SCRIPTS / "ready-issues.py"), "--dump-partitions", *keys],
            capture_output=True, text=True, check=True).stdout
        self.assertEqual(json.loads(out)["resolved"], dict(fixture))


class TestConflictAttribution(unittest.TestCase):
    """A conflict line must name the RAW held label and the COARSEST holder, deterministically."""

    def test_conflict_line_names_the_raw_parent_label_not_the_resolved_path(self):
        lines = []
        board = [pr(70, ["area:sparq-server"]),
                 iss(20, READY + ["priority:P1", "area:sparq-server-http"])]
        ready.compute_ready(board, conflict_log=lines.append)
        self.assertEqual(lines, ["conflict #20: area sparq-server held by pr#70"])

    def test_conflict_line_names_the_raw_child_label_when_the_child_holds(self):
        lines = []
        board = [pr(70, ["area:sparq-core-store"]),
                 iss(20, READY + ["priority:P1", "area:sparq-core"])]
        ready.compute_ready(board, conflict_log=lines.append)
        self.assertEqual(lines, ["conflict #20: area sparq-core-store held by pr#70"])

    def test_the_coarsest_holder_is_reported_when_several_conflict(self):
        # __global__ (path length 0) outranks any named area; named areas tie-break alphabetically.
        lines = []
        board = [iss(70, ["status:in-progress"] + ["area:sparq-server"]),
                 iss(71, ["status:in-progress"] + ["area:sparq-core"]),
                 iss(30, READY + ["priority:P0"])]
        ready.compute_ready(board, conflict_log=lines.append)
        self.assertEqual(lines, ["conflict #30: area sparq-core held by issue#71"])


class TestInProgressReviewStatus(unittest.TestCase):
    """Class 2 — status:in-progress-review failed OPEN on BOTH halves."""

    def test_in_progress_review_is_busy(self):
        self.assertIn("status:in-progress-review", ready.BUSY_STATUS)
        self.assertTrue(ready.is_busy({"status:in-progress-review"}))

    def test_in_progress_review_issue_is_never_selected(self):
        board = [iss(10, READY + ["priority:P0", "area:sparq-zk",
                                  "status:in-progress-review"])]
        self.assertEqual(ready.compute_ready(board, conflict_log=quiet), [],
                         "an issue already in review must not be dispatched again")

    def test_in_progress_review_issue_reserves_its_area(self):
        # The other half: dispatch.yml KEEPS these rows in its input precisely because it
        # believes they reserve. If they do not, a second worker takes the same crate.
        board = [iss(10, ["status:in-progress-review", "area:sparq-zk"]),
                 iss(20, READY + ["priority:P1", "area:sparq-zk"])]
        self.assertEqual(ready.compute_ready(board, conflict_log=quiet), [],
                         "an in-review issue must still hold its crate")

    def test_exclusion_reason_names_the_review_status(self):
        self.assertEqual(
            ready.exclusion_reason({"status:ready", "status:in-progress-review"}),
            "busy: status:in-progress-review")


class TestDrainableBacklogVsConcurrencyFrontier(unittest.TestCase):
    """The two must stay distinct — conflating them hides a healthy backlog."""

    BOARD = [
        iss(1, READY + ["priority:P1", "area:sparq-core"]),
        iss(2, READY + ["priority:P2", "area:sparq-core"]),
        iss(3, READY + ["priority:P2", "area:sparq-core"]),
        iss(4, READY + ["priority:P1", "area:sparq-hdt"]),
    ]

    def test_ready_candidates_counts_all_drainable_work(self):
        self.assertEqual(
            sorted(c[1] for c in ready.ready_candidates(self.BOARD)), [1, 2, 3, 4])

    def test_compute_ready_serialises_one_per_package(self):
        self.assertEqual(numbers(ready.compute_ready(self.BOARD, conflict_log=quiet)), [1, 4])

    def test_frontier_is_always_a_subset_of_the_drainable_backlog(self):
        frontier = numbers(ready.compute_ready(self.BOARD, conflict_log=quiet))
        drainable = {c[1] for c in ready.ready_candidates(self.BOARD)}
        self.assertTrue(set(frontier) <= drainable)
        self.assertLessEqual(len(frontier), len(drainable))

    def test_every_dropped_attested_candidate_is_attributable(self):
        # A silent `continue` is how a label-regressed issue leaves the frontier forever.
        board = [
            iss(40, READY + ["priority:P1", "needs:design", "area:a"]),
            iss(41, READY + ["priority:P1", "area:b"], blockers=2),
            iss(42, READY + ["area:c"]),
            iss(43, ["status:ready", "priority:P1", "area:d"]),
            iss(44, ["priority:P1", "role:impl", "area:e"]),  # NOT attested -> must stay quiet
        ]
        lines = []
        ready.ready_candidates(board, log=lines.append)
        got = {int(re.search(r"#(\d+)", line).group(1)): line for line in lines}
        self.assertEqual(sorted(got), [40, 41, 42, 43])
        self.assertIn("gated by needs:design", got[40])
        self.assertIn("2 open blocker(s)", got[41])
        self.assertIn("no single valid priority", got[42])
        self.assertIn("no role:* label", got[43])
        self.assertNotIn(44, got, "a non-attested issue is not a candidate and must not log")


class TestRolelessInvisibility(unittest.TestCase):
    """The class that appears in no plan and no diagnostic — it must be REPORTED."""

    def test_roleless_ready_reports_the_invisible_issues(self):
        board = [
            iss(50, ["status:ready", "priority:P1", "area:a"]),          # roleless
            iss(51, ["status:ready", "area:b"]),                          # roleless, no priority
            iss(52, READY + ["priority:P1", "area:c"]),                   # fine
            iss(53, ["status:ready", "needs:user", "area:d"]),            # gated, not this class
        ]
        self.assertEqual(ready.roleless_ready(board), [50, 51])

    def test_roleless_issues_are_genuinely_absent_from_the_frontier(self):
        board = [iss(50, ["status:ready", "priority:P1", "area:a"])]
        self.assertEqual(ready.compute_ready(board, conflict_log=quiet), [])
        self.assertEqual(ready.roleless_ready(board), [50],
                         "invisible to dispatch, but NOT invisible to the report")

    def test_target_planner_exposes_roleless_ready_to_dispatch_yml(self):
        # dispatch.yml does getattr(dispatch, "roleless_ready", None) and degrades to a
        # "planner has no roleless_ready()" warning when the target predates it.
        self.assertTrue(callable(getattr(plan, "roleless_ready", None)),
                        "dispatch-plan.py must export roleless_ready for the registry planner")
        self.assertEqual(plan.roleless_ready(
            [iss(50, ["status:ready", "priority:P1", "area:a"])]), [50])


def worker_pr(number, issue_number):
    """A pipeline-owned worker PR whose head branch links `issue_number` (dispatch.yml's rule)."""
    return {"number": number, "state": "OPEN", "labels": [], "pull_request": {}, "draft": False,
            "head": {"ref": f"sparq-agent/issue-{issue_number}-fix",
                     "repo": {"full_name": "sparq-org/sparq"}},
            "body": "", "author_association": "NONE"}


class TestLocalOrchestratorParity(unittest.TestCase):
    """The divergence that mattered: the local CLI must preview the dispatched frontier.

    [OPUS-5] The parity claim used to rest on a SOURCE-SUBSTRING assertion plus two test-local
    reimplementations of both sides — so `compute_ready(visible)` -> `compute_ready(issues)` in
    the real main() restored the whole bug with the suite green. Every assertion below now runs
    the REAL main()/--diagnose over a stubbed snapshot, stubbing ONLY the two network calls, and
    compares its printed frontier against a mirror of dispatch.yml's comprehension.
    """

    @staticmethod
    def _orchestrator(issues_and_prs, pulls=()):
        """Mirror of dispatch.yml `ready_input`: ISSUES ONLY; a linked row survives iff in-flight.

        Copied shape (agent-account-registry .github/workflows/dispatch.yml):
            ready_input = [row for row in readiness_input
                           if "status:in-progress" in row["labels"]
                           or "status:in-progress-review" in row["labels"]
                           or (row["number"] not in linked and trusted(...))]
        The `trusted(...)` conjunct is registry-side (needs the issue author + the per-repo
        trusted-bot list) and is out of scope for the local preview; see dispatchable_view's
        docstring for that documented residual divergence.
        """
        linked = ready.linked_issue_numbers(list(pulls), "sparq-org/sparq")
        rows = [it for it in issues_and_prs if "pull_request" not in it]
        rows = [it for it in rows
                if it["number"] not in linked
                or {"status:in-progress", "status:in-progress-review"} & set(it["labels"])]
        return numbers(ready.compute_ready(rows, conflict_log=quiet))

    @staticmethod
    def _run_cli_streams(issues_and_prs, pulls=(), argv=()):
        """Execute the REAL main() end-to-end; only `_fetch`/`_fetch_pulls` are stubbed.

        Returns (stdout, stderr). STDERR is not incidental: `compute_ready`'s default
        `conflict_log` writes the per-conflict ATTRIBUTION line there, and attribution is the one
        observable the PR/issue unit fold changes on main()'s default path — see
        `TestUnitReservation.test_the_default_CLI_path_attributes_a_conflict_to_the_PR`.
        """
        real_fetch, real_pulls, real_argv = ready._fetch, ready._fetch_pulls, sys.argv
        ready._fetch = lambda repo, ceiling=10000: [dict(it) for it in issues_and_prs]
        ready._fetch_pulls = lambda repo: [dict(p) for p in pulls]
        sys.argv = ["ready-issues.py", *argv]
        out, err = io.StringIO(), io.StringIO()
        try:
            with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
                rc = ready.main()
        finally:
            ready._fetch, ready._fetch_pulls, sys.argv = real_fetch, real_pulls, real_argv
        assert rc == 0, f"main() exited {rc}"
        return out.getvalue(), err.getvalue()

    @classmethod
    def _run_cli(cls, issues_and_prs, pulls=(), argv=()):
        return cls._run_cli_streams(issues_and_prs, pulls, argv)[0]

    @classmethod
    def _local(cls, issues_and_prs, pulls=()):
        """The frontier the REAL CLI prints, parsed from its `P<n>  #<num>  [...]` rows."""
        return [int(n) for n in re.findall(
            r"^P\d+\s+#\s*(\d+)\s", cls._run_cli(issues_and_prs, pulls), re.M)]

    # -- the executed counterexample -------------------------------------------------------
    # in-review #100 on sparq-core, covered by its OWN worker PR, + attested #200 on sparq-core.
    COUNTEREXAMPLE_PULLS = (worker_pr(101, 100),)
    COUNTEREXAMPLE_BOARD = (
        iss(100, READY + ["priority:P1", "area:sparq-core", "status:in-progress-review"]),
        iss(200, READY + ["priority:P1", "area:sparq-core"]),
    )

    def test_in_flight_issue_covered_by_its_own_pr_still_reserves_its_crate(self):
        # THE regression. Dropping every linked row frees sparq-core while #100 is actively
        # being worked, and the next tick dispatches a SECOND worker onto the same crate.
        board, pulls = list(self.COUNTEREXAMPLE_BOARD), list(self.COUNTEREXAMPLE_PULLS)
        self.assertEqual(self._orchestrator(board, pulls), [],
                         "sanity: the orchestrator keeps #100 as an occupant, so #200 conflicts")
        self.assertEqual(self._local(board, pulls), [],
                         "local CLI dispatched #200 onto a crate the orchestrator sees as busy")

    def test_cli_frontier_equals_orchestrator_frontier_on_the_counterexample(self):
        board, pulls = list(self.COUNTEREXAMPLE_BOARD), list(self.COUNTEREXAMPLE_PULLS)
        self.assertEqual(self._local(board, pulls), self._orchestrator(board, pulls))

    def test_diagnose_frontier_equals_orchestrator_frontier_on_the_counterexample(self):
        # --diagnose reads its own `visible`; it regressed independently of the default path.
        out = self._run_cli(list(self.COUNTEREXAMPLE_BOARD), list(self.COUNTEREXAMPLE_PULLS),
                            argv=("--diagnose",))
        self.assertIn("concurrency frontier (compute_ready): 0", out)
        self.assertIn("drainable backlog (ready_candidates): 1", out)

    def test_diagnose_does_not_call_an_in_flight_row_merely_pr_covered(self):
        # The bucket must agree with the view: a row dispatchable_view KEEPS is reported as
        # busy, not as suppressed. Otherwise the taxonomy explains a drop that never happened.
        counts, _roleless, _cands, frontier, _units = ready.diagnose(
            list(self.COUNTEREXAMPLE_BOARD), linked={100})
        self.assertEqual(counts.get("busy: status:in-progress-review"), 1)
        self.assertIsNone(counts.get("covered by an open linked PR"))
        self.assertEqual(numbers(frontier), [])

    # -- the previously-covered shapes, now through the real CLI ---------------------------
    def test_unlabelled_prs_do_not_make_the_two_views_disagree(self):
        # The exact live shape: many area-less open PRs + attested issues on distinct crates.
        # [OPUS-5] sparq#4336: real disjoint crates, for the reason recorded on
        # test_unlabelled_pr_leaves_every_unrelated_crate_dispatchable.
        board = [pr(3803, []), pr(3799, []), pr(3798, [])] + [
            iss(n, READY + ["priority:P3", f"area:{a}"])
            for n, a in ((3694, "sparq-core"), (3756, "sparq-hdt"), (3757, "sparq-geo"))]
        self.assertEqual(self._local(board), self._orchestrator(board))
        self.assertEqual(self._local(board), [3694, 3756, 3757])

    def test_linked_pr_suppression_matches_on_both_sides(self):
        # A linked row with NO in-flight status is still suppressed on both sides.
        board = [iss(60, READY + ["priority:P1", "area:sparq-core"]),
                 iss(61, READY + ["priority:P1", "area:sparq-hdt"])]
        pulls = [worker_pr(62, 60)]
        self.assertEqual(self._local(board, pulls), self._orchestrator(board, pulls))
        self.assertEqual(self._local(board, pulls), [61])

    def test_local_cli_applies_linked_pr_suppression_at_all(self):
        # Behavioural, not a substring: deleting the linked_issue_numbers call in main() must
        # let #60 back onto the frontier ahead of the crate it is already covered on.
        board = [iss(60, READY + ["priority:P0", "area:sparq-core"]),
                 iss(61, READY + ["priority:P1", "area:sparq-hdt"])]
        self.assertEqual(self._local(board, [worker_pr(62, 60)]), [61],
                         "main() must suppress issues covered by an open linked PR, as dispatch does")


class TestUnitReservation(unittest.TestCase):
    """A PR and the issues it closes are ONE unit: they reserve the UNION, exactly ONCE.

    [OPUS-5] MEASURED on the live sparq snapshot (2026-07-27, 1473 open issues / 123 open PRs):
    65 occupying PRs + 46 in-flight issues produced 158 reservations over 49 distinct partition
    keys — 20 of them a duplicate of a key the unit's other half already held.

    Two facts pin the shape of the rule, and both are measurements rather than opinions:

    * DEDUP IS NOT A FRONTIER LEVER. `conflict()` tests membership in the SET of held keys, so a
      second occupant on an already-held key changes nothing. 158 -> 138 reservations left the held
      set at 49 keys and the live frontier at 3 -> 3. Every test below therefore asserts on the
      RESERVATION STRUCTURE and on end-to-end dispatch decisions, not on a frontier count that the
      dedup cannot move.
    * DROPPING THE ISSUE HALF UNDER-SERIALISES. Over the 94 open PRs with an open linked source
      issue: 31 pairs PR ⊋ issue, 18 identical, 6 source-with-no-`area:`, but 13 PR ⊊ issue and 26
      INCOMPARABLE. So in 39/94 = 41% of pairs the PR's key set is NOT a superset and dropping the
      issue's reservation frees a key the unit really occupies — two workers in one crate, the
      corrupting direction. Hence union, never drop.
    """

    @staticmethod
    def keys(units):
        return set().union(set(), *[areas for areas, _artifact in units])

    @staticmethod
    def by_pr(units):
        return {artifact["number"]: sorted(areas) for areas, artifact in units}

    # -- the headline guard, asserted through compute_ready AND through the real CLI ---------
    def test_pr_and_source_issue_reserve_the_union_exactly_once(self):
        # THE titled guard. The pair declares {sparq-core} (PR) and {sparq-hdt} (issue): the unit
        # must hold BOTH, under ONE artifact, and neither key may be reserved twice.
        source = iss(100, ["status:in-progress-review", "area:sparq-hdt"])
        worker = pr(101, ["area:sparq-core"])
        links = {101: {100}}
        units = ready.unit_reservations([worker, source], links)
        self.assertEqual(self.by_pr(units), {101: ["sparq-core", "sparq-hdt"]},
                         "the pair must reserve the union under the PR, as ONE unit")
        self.assertEqual(len(units), 1, "the source issue must not reserve a second time")
        # ...and the union is actually enforced: BOTH crates are held against fresh candidates.
        board = [worker, source,
                 iss(200, READY + ["priority:P1", "area:sparq-core"]),
                 iss(201, READY + ["priority:P1", "area:sparq-hdt"]),
                 iss(202, READY + ["priority:P1", "area:sparq-geo"])]
        self.assertEqual(
            numbers(ready.compute_ready(board, conflict_log=quiet, source_links=links)), [202],
            "both halves of the unit's union must block, and unrelated crates must not")

    def test_the_pair_is_ONE_occupant_not_two(self):
        # The dedup itself, stated so that removing `consumed` (letting the source issue reserve
        # again on its own) reds even though the HELD KEY SET is unchanged by that mutation.
        source = iss(100, ["status:in-progress-review", "area:sparq-core"])
        units = ready.unit_reservations([pr(101, ["area:sparq-core"]), source], {101: {100}})
        self.assertEqual([artifact["number"] for _areas, artifact in units], [101])
        self.assertEqual(sum(len(areas) for areas, _ in units), 1,
                         "one unit of work must produce exactly one reservation of sparq-core")

    def test_conflict_attribution_names_the_PR_of_a_paired_unit(self):
        # Attribution is the one thing dedup DOES change, and it changes it for the better: the
        # PR is the advanceable artifact (it can merge or close), the source issue is not.
        board = [iss(100, ["status:in-progress-review", "area:sparq-core"]),
                 pr(101, ["area:sparq-core"]),
                 iss(200, READY + ["priority:P1", "area:sparq-core"])]
        logs = []
        ready.compute_ready(board, conflict_log=logs.append, source_links={101: {100}})
        self.assertEqual(logs, ["conflict #200: area sparq-core held by pr#101"])

    # -- the four obligations that make the rule impossible to weaken into a DROP ------------
    def test_source_issue_broader_than_its_pr_does_not_lose_the_extra_areas(self):
        # 39/94 live pairs are non-superset. Narrowing the unit to the PR's own set here would
        # free sparq-hdt and sparq-geo while a worker is mid-flight on them.
        source = iss(100, ["status:in-progress-review", "area:sparq-hdt", "area:sparq-geo"])
        links = {101: {100}}
        self.assertEqual(self.by_pr(ready.unit_reservations([pr(101, ["area:sparq-hdt"]), source],
                                                            links)),
                         {101: ["sparq-geo", "sparq-hdt"]})
        board = [pr(101, ["area:sparq-hdt"]), source,
                 iss(200, READY + ["priority:P1", "area:sparq-geo"])]
        self.assertEqual(numbers(ready.compute_ready(board, conflict_log=quiet,
                                                     source_links=links)), [],
                         "an area the ISSUE half alone declares must still be held by the unit")

    def test_source_issue_with_no_area_keeps_the_units_reservation_intact(self):
        # sparq#4336 / PR #4360: the source issue carried NO `area:` at all. Folding it into the
        # unit must not shrink what the unit holds — the PR's own key must survive untouched.
        source = iss(100, ["status:in-progress-review"])
        links = {101: {100}}
        self.assertEqual(self.by_pr(ready.unit_reservations([pr(101, ["area:sparq-core"]), source],
                                                            links)),
                         {101: ["sparq-core"]})
        board = [pr(101, ["area:sparq-core"]), source,
                 iss(200, READY + ["priority:P1", "area:sparq-core"]),
                 iss(201, READY + ["priority:P1", "area:sparq-hdt"])]
        self.assertEqual(numbers(ready.compute_ready(board, conflict_log=quiet,
                                                     source_links=links)), [201])

    def test_a_units_reservation_is_a_SUPERSET_of_every_members_own(self):
        # MONOTONICITY — the property that makes under-serialisation structurally impossible,
        # whatever the two halves declare. Exhaustive over every containment direction, plus the
        # `{GLOBAL}` member: a half whose own rule yields the serializing partition must keep it.
        cases = [
            (["area:sparq-core"], ["area:sparq-core"]),               # identical
            (["area:sparq-core", "area:sparq-hdt"], ["area:sparq-core"]),   # PR superset
            (["area:sparq-core"], ["area:sparq-core", "area:sparq-hdt"]),   # issue superset
            (["area:sparq-core"], ["area:sparq-hdt"]),                # incomparable
            ([], ["area:sparq-hdt"]),                                 # PR declares nothing
            (["area:sparq-core"], []),                                # issue declares nothing
            ([f"area:{ready.GLOBAL}"], ["area:sparq-core"]),          # a GLOBAL-holding half
            (["area:sparq-core"], [f"area:{ready.GLOBAL}"]),
        ]
        for pr_labels, issue_labels in cases:
            with self.subTest(pr=pr_labels, issue=issue_labels):
                worker = pr(101, pr_labels)
                source = iss(100, ["status:in-progress-review"] + issue_labels)
                units = ready.unit_reservations([worker, source], {101: {100}})
                held = self.keys(units)
                for member in (worker, source):
                    self.assertLessEqual(
                        ready._own_reservation(member), held,
                        f"unit dropped a key member #{member['number']} reserves on its own")

    def test_a_GLOBAL_holding_half_still_serializes_the_whole_board(self):
        # The fail-closed global must survive the fold END-TO-END, not merely in the key set.
        board = [pr(101, [f"area:{ready.GLOBAL}"]),
                 iss(100, ["status:in-progress-review", "area:sparq-core"]),
                 iss(200, READY + ["priority:P1", "area:sparq-geo"])]
        self.assertEqual(numbers(ready.compute_ready(board, conflict_log=quiet,
                                                     source_links={101: {100}})), [],
                         "a unit holding __global__ must still block every unrelated crate")

    # -- the units that must NOT be folded together -------------------------------------------
    def test_two_unrelated_units_still_reserve_separately(self):
        board = [pr(101, ["area:sparq-core"]), iss(100, ["status:in-progress-review",
                                                         "area:sparq-core"]),
                 pr(301, ["area:sparq-hdt"]), iss(300, ["status:in-progress-review",
                                                        "area:sparq-hdt"])]
        links = {101: {100}, 301: {300}}
        units = ready.unit_reservations(board, links)
        self.assertEqual(self.by_pr(units), {101: ["sparq-core"], 301: ["sparq-hdt"]},
                         "two genuinely distinct units must remain two occupants")
        # ...and EVERY unit must reach `reserve()`, not just the first. Truncating the occupancy
        # loop (`unit_reservations(...)[:1]`) is a six-character edit that frees every unit after
        # the first, so assert the SECOND unit's crate is held too.
        contenders = [iss(200, READY + ["priority:P1", "area:sparq-core"]),
                      iss(201, READY + ["priority:P2", "area:sparq-hdt"]),
                      iss(202, READY + ["priority:P3", "area:sparq-geo"])]
        self.assertEqual(
            numbers(ready.compute_ready(board + contenders, conflict_log=quiet,
                                        source_links=links)), [202],
            "both units must hold their crate — a truncated occupancy loop frees the second")

    def test_an_issue_with_no_linked_pr_is_unaffected(self):
        lone = iss(100, ["status:in-progress-review", "area:sparq-core"])
        other = pr(301, ["area:sparq-hdt"])
        units = ready.unit_reservations([lone, other], {301: set()})
        self.assertEqual(sorted(a["number"] for _k, a in units), [100, 301])
        self.assertEqual(self.keys(units), {"sparq-core", "sparq-hdt"})
        # and it still blocks its own crate on the frontier
        board = [lone, iss(200, READY + ["priority:P1", "area:sparq-core"])]
        self.assertEqual(numbers(ready.compute_ready(board, conflict_log=quiet,
                                                     source_links={301: set()})), [])

    def test_a_fork_PR_never_folds_an_issue_into_its_unit(self):
        # source_issue_links is the ONLY linkage rule; a fork head is attacker-controlled text.
        fork = {"number": 101, "state": "OPEN", "labels": [], "pull_request": {}, "draft": False,
                "head": {"ref": "sparq-agent/issue-100-fix", "repo": {"full_name": "attacker/x"}},
                "body": "Closes #100", "author_association": "NONE"}
        self.assertEqual(ready.source_issue_links([fork], "sparq-org/sparq"), {})

    # -- the no-op contract the REGISTRY depends on -------------------------------------------
    def test_unit_reservations_without_links_is_identical_to_the_legacy_loop(self):
        # dispatch.yml calls `compute_ready(ready_input)` with no source_links. If that call is not
        # byte-identical to the pre-refactor loop, the two repositories cannot merge in either
        # order. Reproduce the LEGACY loop here and compare pairs AND their order (attribution is
        # order-sensitive: `blockers[area][0]` names the first reserver).
        board = [pr(101, ["area:sparq-core"]),
                 iss(100, ["status:in-progress-review", "area:sparq-core"]),
                 iss(102, ["status:in-progress", "area:sparq-hdt"]),
                 pr(103, ["area:sparq-geo", "needs:user"]),      # parked -> reserves nothing
                 pr(104, []),                                    # unattributable -> nothing
                 iss(105, READY + ["priority:P1", "area:sparq-zk"]),
                 iss(106, ["status:in-progress-review", "review:needs-user", "area:sparq-mpc"])]
        legacy = []
        for row in board:
            if str(row.get("state", "OPEN")).upper() != "OPEN" or not ready.occupies_area(row):
                continue
            labels = ready.labels_of(row)
            if "pull_request" in row or labels & ready.IN_FLIGHT_STATUS:
                areas = ready._reserving_packages(labels)
                if areas:
                    legacy.append((areas, row["number"]))
        self.assertEqual([(areas, artifact["number"])
                          for areas, artifact in ready.unit_reservations(board)], legacy)
        self.assertEqual(legacy, [({"sparq-core"}, 101), ({"sparq-core"}, 100),
                                  ({"sparq-hdt"}, 102)],
                         "sanity: the legacy expectation itself must be the live rule, not a stub")

    def test_compute_ready_without_source_links_is_unchanged(self):
        board = [pr(101, ["area:sparq-core"]),
                 iss(100, ["status:in-progress-review", "area:sparq-core"]),
                 iss(200, READY + ["priority:P1", "area:sparq-core"]),
                 iss(201, READY + ["priority:P2", "area:sparq-hdt"])]
        logs = []
        self.assertEqual(numbers(ready.compute_ready(board, conflict_log=logs.append)), [201])
        self.assertEqual(logs, ["conflict #200: area sparq-core held by pr#101"],
                         "omitting source_links must leave attribution exactly as it was")

    # -- the CALL SITES, not just the helper ---------------------------------------------------
    # [OPUS-5] MEASURED, and the reason these three tests are shaped the way they are: the held
    # KEY SET is invariant under folding, because a unit reserves the union of exactly its members'
    # own reservations. So NO frontier assertion anywhere can witness `source_links` being dropped
    # at a call site — a first cut of this suite asserted frontiers and left both CLI legs' mutants
    # ALIVE. What the fold does change is (a) how many occupants there are and (b) which artifact a
    # conflict is attributed to. Each leg is therefore pinned on the observable it actually has.
    @staticmethod
    def _pair_board():
        """Live shape: in-review issue #100 on sparq-hdt + its worker PR #101 on sparq-core."""
        pulls = [dict(worker_pr(101, 100), labels=[{"name": "area:sparq-core"}])]
        rows = [iss(100, ["status:in-progress-review", "area:sparq-hdt"]),
                iss(200, READY + ["priority:P1", "area:sparq-core"])]
        rows += [dict(p, labels=[lb["name"] for lb in p["labels"]]) for p in pulls]
        return rows, pulls

    def test_the_held_key_set_is_invariant_under_folding(self):
        # WHY no frontier assertion can guard the fold, asserted rather than asserted-about. A unit
        # reserves the union of exactly its members' own reservations, so the union OVER UNITS
        # equals the union OVER MEMBERS: `conflict()` reads only `blockers.keys()`, therefore the
        # frontier is identical with and without `source_links`. This is what licenses
        # `diagnose()` not passing it, and it is why the CLI guards pin attribution + unit count.
        board = [pr(101, ["area:sparq-core"]),
                 iss(100, ["status:in-progress-review", "area:sparq-hdt"]),
                 pr(301, []), iss(300, ["status:in-progress-review", "area:sparq-geo"]),
                 pr(401, ["area:sparq-zk"]), iss(400, ["status:in-progress-review"]),
                 iss(200, READY + ["priority:P1", "area:sparq-core"]),
                 iss(201, READY + ["priority:P2", "area:sparq-hdt"]),
                 iss(202, READY + ["priority:P3", "area:sparq-mpc"])]
        links = {101: {100}, 301: {300}, 401: {400}}
        self.assertEqual(self.keys(ready.unit_reservations(board)),
                         self.keys(ready.unit_reservations(board, links)),
                         "folding must not add or remove a single held partition key")
        self.assertEqual(numbers(ready.compute_ready(board, conflict_log=quiet)),
                         numbers(ready.compute_ready(board, conflict_log=quiet,
                                                     source_links=links)))
        self.assertEqual(numbers(ready.compute_ready(board, conflict_log=quiet,
                                                     source_links=links)), [202],
                         "sanity: the board must actually exercise conflicts, not be all-free")

    def test_diagnose_reports_the_pair_as_ONE_unit(self):
        # Kills: `diagnose()` dropping `source_links=` from its compute_ready/unit_reservations.
        board, pulls = self._pair_board()
        _c, _r, _cands, _frontier, units = ready.diagnose(
            board, linked={100}, source_links={101: {100}})
        self.assertEqual([artifact["number"] for _areas, artifact in units], [101])
        self.assertEqual(sorted(next(iter(a for a, _ in units))), ["sparq-core", "sparq-hdt"])

    def test_the_diagnose_CLI_leg_prints_the_folded_unit_count(self):
        # Kills: main() --diagnose losing the fold anywhere between diagnose() and the print.
        board, pulls = self._pair_board()
        out = TestLocalOrchestratorParity._run_cli(board, pulls, argv=("--diagnose",))
        self.assertIn("unit occupancy: 1 unit(s), 2 reservation(s) over 2 partition key(s)", out,
                      "the PR+issue pair must be reported as ONE unit holding TWO keys")

    def test_the_default_CLI_path_attributes_a_conflict_to_the_PR(self):
        # Kills: main()'s DEFAULT (non---diagnose) leg dropping `source_links=`. That leg's only
        # observable is the attribution line compute_ready writes to stderr; without the fold the
        # blocker is reported as `issue#100`, which no one can merge or close.
        board, pulls = self._pair_board()
        board = board + [iss(201, READY + ["priority:P1", "area:sparq-hdt"])]
        _out, err = TestLocalOrchestratorParity._run_cli_streams(board, pulls)
        self.assertIn("conflict #201: area sparq-hdt held by pr#101", err,
                      "the default CLI path must fold the unit and attribute to the PR")
        self.assertNotIn("held by issue#100", err)


class TestOrchestratorOccupancyGap(unittest.TestCase):
    """The consequence of PLAN seeing the ISSUE half of every unit only — and its live precondition.

    [OPUS-5] HISTORY. `dispatch.yml` used to build `readiness_input` from
    `[issue for issue in snapshot("issues", index) if "pull_request" not in issue]`, so PLAN
    reserved the ISSUE half of every unit and never the PR half. Registry CLAIM re-derived busy
    areas from the pulls snapshot and dropped the doomed rows, so they were never double-dispatched
    — but it dropped them AFTER compute_ready committed the frontier, so each burned a partition
    with no backfill (the registry's own issue #113 shape).

    [OPUS-5 2026-07-27] THAT IS FIXED, and the fix moved the risk to this side of the seam.
    Registry #773 (merged 15:54Z) folds every open PR row into PLAN's occupancy — but CONDITIONALLY,
    on an executable capability probe run against the target repo's own planner, and a planner that
    fails the probe silently keeps the old issue-only behaviour. VERIFIED against the live PLAN job
    of registry dispatch run 30283405388 (16:24Z, master 84c1b687, which contains #773 at 58dc61bd),
    whose runtime output reads:

        sparq-org/sparq: PLAN occupancy carries 118 open pull-request row(s)
        — reserves PR areas and never dispatches a PR row

    So the gap is no longer a property of dispatch.yml; it is a property of `compute_ready` in THIS
    repo. `ready.pr_row_occupancy_probe` mirrors the registry's probe so a sparq commit that turns
    PR-half occupancy back off is caught by sparq's own CI instead of by a warning printed in a
    repository nobody working here reads.
    """

    BOARD = (
        pr(101, ["area:sparq-core"]),                                   # PR half only
        iss(100, ["status:in-progress-review", "area:sparq-hdt"]),      # issue half only
    )

    def test_stripping_pr_rows_loses_the_pr_half_of_every_unit(self):
        pr_aware, issue_only, unheld = ready.occupancy_parity(list(self.BOARD), {101: {100}})
        self.assertEqual(pr_aware, {"sparq-core", "sparq-hdt"})
        self.assertEqual(issue_only, {"sparq-hdt"})
        self.assertEqual(unheld, {"sparq-core"})

    def test_the_gap_is_exactly_what_lets_a_second_worker_into_the_crate(self):
        # Behavioural, not a set difference: the issue-only view DISPATCHES onto the PR's crate.
        board = list(self.BOARD) + [iss(200, READY + ["priority:P1", "area:sparq-core"])]
        issue_only = [row for row in board if "pull_request" not in row]
        self.assertEqual(numbers(ready.compute_ready(issue_only, conflict_log=quiet)), [200],
                         "sanity: this is what the orchestrator does today")
        self.assertEqual(numbers(ready.compute_ready(board, conflict_log=quiet,
                                                     source_links={101: {100}})), [],
                         "the PR-aware unit view must hold #200 back")

    def test_parity_measurement_is_empty_when_nothing_is_stripped(self):
        # No false alarm: an issue-only snapshot with no PR-held keys reports no gap.
        board = [iss(100, ["status:in-progress-review", "area:sparq-hdt"])]
        _pr_aware, _issue_only, unheld = ready.occupancy_parity(board, {})
        self.assertEqual(unheld, set())

    # -- the probe: the live precondition for PLAN reserving our PR rows ---------------------

    @staticmethod
    def _engine(kind):
        """A planner broken in exactly ONE of the ways the probe must catch."""
        def compute(rows, *_args, **_kwargs):
            if kind == "raises":
                raise RuntimeError("hostile planner")
            if kind == "ignores_pr_areas":
                # Never selects a PR row (obligation 1 holds) but reserves nothing for it, so the
                # rival sails through and the whole fold is an expensive no-op.
                survivors = [row for row in rows if "pull_request" not in row]
                return ready.compute_ready(survivors, conflict_log=quiet)
            if kind == "dispatches_pr":
                return list(rows)                    # treats a PR row as a dispatch candidate
            raise AssertionError(f"unknown engine kind {kind!r}")
        return compute

    def test_the_live_engine_satisfies_the_registrys_pr_row_probe(self):
        # THE contract. If this goes red, dispatch.yml stops folding sparq's PR rows into PLAN's
        # occupancy and every open PR's crate reads FREE to the orchestrator again.
        ok, why = ready.pr_row_occupancy_probe()
        self.assertTrue(ok, f"registry would strip our PR rows: {why}")
        self.assertEqual(why, "reserves PR areas and never dispatches a PR row")

    def test_the_probe_rejects_an_engine_that_dispatches_a_pull_request_row(self):
        ok, why = ready.pr_row_occupancy_probe(self._engine("dispatches_pr"))
        self.assertFalse(ok)
        self.assertIn("DISPATCHES", why)

    def test_the_probe_rejects_an_engine_that_ignores_a_pull_requests_area(self):
        # Passes the SAFETY obligation (never selects the PR) while making the fold a no-op.
        ok, why = ready.pr_row_occupancy_probe(self._engine("ignores_pr_areas"))
        self.assertFalse(ok)
        self.assertIn("does not RESERVE", why)

    def test_the_probe_is_fail_closed_on_an_engine_that_raises(self):
        ok, why = ready.pr_row_occupancy_probe(self._engine("raises"))
        self.assertFalse(ok)
        self.assertIn("probe raised", why)

    def test_the_probe_calls_the_engine_exactly_as_the_orchestrator_does(self):
        # dispatch.yml calls `planner.compute_ready([...])` positionally with NO keywords. A probe
        # that passed `conflict_log=` would stay green through a signature change that breaks the
        # orchestrator's real call.
        seen = []

        def recorder(rows, *args, **kwargs):
            seen.append((args, dict(kwargs)))
            return ready.compute_ready(rows, conflict_log=quiet)

        ready.pr_row_occupancy_probe(recorder)
        self.assertEqual(seen, [((), {}), ((), {})], "probe must not pass extra arguments")

    def test_the_registry_probes_the_engine_this_module_defines(self):
        # dispatch.yml loads scripts/dispatch-plan.py and probes ITS `compute_ready`. The local
        # mirror only characterises the orchestrator while dispatch-plan RE-EXPORTS this engine
        # rather than wrapping it — a wrapper could satisfy the probe here and fail it there.
        self.assertIs(plan.compute_ready, plan._ready.compute_ready,
                      "dispatch-plan must re-export the readiness engine, not redefine it")
        self.assertTrue(ready.pr_row_occupancy_probe(plan.compute_ready)[0],
                        "the object the registry actually probes must pass the probe")

    # -- the warning is gated on the probe, not on `unheld` -----------------------------------

    GAP_BOARD = [iss(100, ["status:in-progress-review", "area:sparq-hdt"]),
                 iss(200, READY + ["priority:P1", "area:sparq-geo"])]
    GAP_PULLS = [dict(worker_pr(101, 100), labels=[{"name": "area:sparq-core"}])]

    def _diagnose(self):
        prs = [dict(p, labels=[lb["name"] for lb in p["labels"]]) for p in self.GAP_PULLS]
        return TestLocalOrchestratorParity._run_cli(
            self.GAP_BOARD + prs, self.GAP_PULLS, argv=("--diagnose",))

    def test_diagnose_reports_the_gap_loudly_when_the_probe_FAILS(self):
        with mock.patch.object(ready, "pr_row_occupancy_probe",
                               lambda engine=None: (False, "planner DISPATCHES a pull-request "
                                                           "row as if it were an issue")):
            out = self._diagnose()
        self.assertIn("ORCHESTRATOR OCCUPANCY GAP", out)
        self.assertIn("sparq-core", out.split("ORCHESTRATOR OCCUPANCY GAP")[1])

    def test_diagnose_does_NOT_warn_while_the_probe_PASSES(self):
        # The stale-alarm fix. `unheld` is non-empty on this board (the PR holds sparq-core and its
        # source issue does not), so an `if unheld:` warning fires here — forever, on every healthy
        # board, since registry #773. Deleting the probe gate re-reds this test.
        self.assertTrue(ready.pr_row_occupancy_probe()[0], "precondition: probe passes")
        _pr_aware, _issue_only, unheld = ready.occupancy_parity(
            [dict(p, labels=[lb["name"] for lb in p["labels"]], state="OPEN",
                  pull_request={}, open_blockers=0) for p in self.GAP_PULLS] + self.GAP_BOARD,
            {101: {100}})
        self.assertIn("sparq-core", unheld, "precondition: the old trigger is armed on this board")
        out = self._diagnose()
        self.assertNotIn("ORCHESTRATOR OCCUPANCY GAP", out)

    def test_diagnose_states_the_healthy_verdict_rather_than_going_silent(self):
        out = self._diagnose()
        self.assertIn("orchestrator occupancy: dispatch.yml RESERVES", out)
        self.assertIn("reserves PR areas and never dispatches a PR row", out)


class TestLinkedIssueDetection(unittest.TestCase):
    """Fork PRs must never suppress an issue (the head branch text is attacker-controlled)."""

    REPO = "sparq-org/sparq"

    def _pull(self, ref, body="", full_name="sparq-org/sparq", association="NONE"):
        return {"head": {"ref": ref, "repo": {"full_name": full_name}},
                "body": body, "author_association": association}

    def test_pipeline_owned_head_links_its_issue(self):
        self.assertEqual(
            ready.linked_issue_numbers([self._pull("sparq-agent/issue-42-fix")], self.REPO), {42})

    def test_fork_head_does_not_link(self):
        self.assertEqual(ready.linked_issue_numbers(
            [self._pull("sparq-agent/issue-42-fix", full_name="attacker/sparq")], self.REPO),
            set(), "a fork PR must never suppress an issue")

    def test_closing_keyword_needs_a_trusted_association(self):
        untrusted = self._pull("patch-1", body="Closes #42", association="NONE")
        trusted = self._pull("patch-1", body="Closes #42", association="MEMBER")
        self.assertEqual(ready.linked_issue_numbers([untrusted], self.REPO), set())
        self.assertEqual(ready.linked_issue_numbers([trusted], self.REPO), {42})


class TestDiagnoseTaxonomy(unittest.TestCase):
    """--diagnose must account for EVERY open issue, so no class can hide."""

    def test_buckets_partition_the_open_backlog(self):
        board = [
            iss(1, READY + ["priority:P1", "area:a"]),
            iss(2, ["status:untriaged"]),
            iss(3, []),
            iss(4, READY + ["priority:P1", "needs:ec2", "area:b"]),
            iss(5, READY + ["priority:P1", "area:c"]),
            pr(70, []),
            iss(6, READY + ["priority:P1", "area:d"], state="CLOSED"),
        ]
        counts, roleless, cands, frontier, _units = ready.diagnose(board, linked={5})
        self.assertEqual(sum(counts.values()), 5, "one bucket per OPEN issue, PRs/closed excluded")
        self.assertEqual(counts["ENUMERABLE"], 1)
        self.assertEqual(counts["covered by an open linked PR"], 1)
        self.assertEqual(counts["gated by needs:ec2"], 1)
        self.assertEqual(counts["no status:ready attestation"], 2)
        self.assertEqual(roleless, [])
        self.assertEqual([c[1] for c in cands], [1])
        self.assertEqual(numbers(frontier), [1])


class TestRetriageCronSeam(unittest.TestCase):
    """The YAML seam that makes the retriage sweep a WRITE and not a report.

    [OPUS-5] Until the fetch-reach fix, retriage promoted 0 issues live, so every property of
    this workflow was unobservable — nothing downstream changed whether it ran, ran dry, or did
    not run at all. Now that it promotes (74 on the live snapshot), each of these is load-bearing
    and none of them was pinned by any test:

      * dropping `--apply` turns the cron into a permanent silent dry-run;
      * `if: false` (or a deleted step/job) stops the sweep with the schedule still green;
      * removing `issues: write` makes every label write fail one-by-one at runtime.

    Parsed structurally rather than by substring precisely because `if: false` on the job or the
    step is invisible to a substring search — the measured shape of every uncaught mutant in this
    repo's workflow-mutation runs.
    """

    RETRIAGE = REPO_ROOT / ".github" / "workflows" / "retriage.yml"
    DOCS_QUALITY = REPO_ROOT / ".github" / "workflows" / "docs-quality.yml"

    @classmethod
    def setUpClass(cls):
        cls.doc = yaml.safe_load(cls.RETRIAGE.read_text(encoding="utf-8"))

    def _steps(self):
        jobs = self.doc["jobs"]
        self.assertIn("retriage", jobs, "the retriage job must exist")
        job = jobs["retriage"]
        self.assertNotIn("if", job, "the retriage job must not be conditionally disabled")
        return job["steps"]

    def _run_blocks(self):
        blocks = []
        for step in self._steps():
            self.assertNotIn("if", step, f"retriage step {step.get('name')!r} is conditional")
            if "run" in step:
                blocks.append(step["run"])
        return "\n".join(blocks)

    def test_the_cron_actually_applies_its_plan(self):
        # MUTANT: drop `--apply` => the sweep prints a plan forever and promotes nothing.
        self.assertRegex(self._run_blocks(), r"retriage\.py\b[^\n]*--apply",
                         "retriage.yml must run retriage.py with --apply, or the cron is a "
                         "permanent dry-run and no issue is ever promoted")

    def test_the_cron_self_tests_before_it_writes(self):
        run = self._run_blocks()
        # Assert PRESENCE before ORDER: `str.index` on a missing needle raises, which would make
        # a deleted --apply an ERROR (a crash-kill) instead of a clean assertion failure.
        self.assertIn("retriage.py --self-test", run)
        self.assertIn("--apply", run)
        self.assertLess(run.index("retriage.py --self-test"), run.index("--apply"),
                        "the fixtures must run BEFORE the sweep writes labels")

    def test_the_sweep_is_scheduled(self):
        on = self.doc.get("on") or self.doc.get(True)      # YAML 1.1 parses bare `on:` as True
        self.assertTrue((on or {}).get("schedule"), "retriage must stay on a schedule")

    def test_the_job_can_write_labels(self):
        # MUTANT: drop `issues: write` => every promotion fails at runtime, one 403 per issue.
        perms = self.doc.get("permissions") or {}
        self.assertEqual(perms.get("issues"), "write",
                         "retriage writes labels; without issues:write every promotion 403s")

    def test_the_workflows_this_class_guards_are_path_triggers(self):
        # [OPUS-5] Otherwise this whole class is VACUOUS on the one edit it exists to catch:
        # routing-self-tests.yml is `paths:`-filtered, so a PR touching ONLY retriage.yml would
        # never run it and deleting --apply would sail through green. Exactly 2 — the
        # pull_request filter AND the push filter.
        source = (REPO_ROOT / ".github" / "workflows"
                  / "routing-self-tests.yml").read_text(encoding="utf-8")
        paths_section = source[:source.index("permissions:")]
        for workflow in (".github/workflows/retriage.yml",
                         ".github/workflows/docs-quality.yml"):
            self.assertEqual(paths_section.count(f'"{workflow}"'), 2,
                             f"{workflow} must re-run this gate on BOTH pull_request and push, "
                             "or the seam assertions above never execute on the PR that breaks "
                             "them")

    def test_triage_and_retriage_fixtures_run_on_every_pr(self):
        # The pair is self-tested in docs-quality.yml so a PR that changes promotion behaviour
        # reds THERE rather than silently at the next cron fire (#3419). Deleting either
        # invocation must red this.
        source = self.DOCS_QUALITY.read_text(encoding="utf-8")
        for script in ("scripts/triage.py", "scripts/retriage.py"):
            self.assertIn(f"python3 {script} --self-test", source,
                          f"{script} --self-test is never RUN on a PR — its assertions are dead")


class TestAreaClassifierCronSeam(unittest.TestCase):
    """The YAML seam that makes the `needs:area` park's ONLY EXIT actually fire.

    [OPUS-5] #3816. `area:` is the partition key every dispatch stage keys off, and
    `triage.py` parks a no-area issue `needs:area` rather than promoting it. Only
    `scripts/triage-area.py` can lift that park — `retriage.py` emits PROMOTING deltas and
    structurally cannot — and until triage-area.yml existed nothing RAN it, so the park was
    terminal in automation: the parked backlog was re-skipped every tick with no exit.

    Each property below is one edit away from restoring that state, and none of them is
    observable anywhere else — a lane that has stopped writing looks exactly like a lane
    with nothing to write:

      * dropping `--apply` turns the sweep into a permanent silent dry-run;
      * `if: false` (or a deleted step/job) stops it with the schedule still green;
      * dropping the schedule leaves a lane only a human can fire, i.e. the hand-run state
        this workflow was added to replace;
      * removing `issues: write` makes every label write 403 one issue at a time;
      * running the sweep before its fixtures lets a rule-table regression mislabel live
        issues, and a wrong `area:` routes a worker at the wrong crate.

    Parsed structurally rather than by substring for the same reason as
    TestRetriageCronSeam: `if: false` on the job or a step is invisible to a substring
    search, and that is the measured shape of every uncaught workflow mutant in this repo.
    """

    TRIAGE_AREA = REPO_ROOT / ".github" / "workflows" / "triage-area.yml"

    @classmethod
    def setUpClass(cls):
        cls.doc = yaml.safe_load(cls.TRIAGE_AREA.read_text(encoding="utf-8"))

    def _steps(self):
        jobs = self.doc["jobs"]
        self.assertIn("classify", jobs, "the classify job must exist")
        job = jobs["classify"]
        self.assertNotIn("if", job, "the classify job must not be conditionally disabled")
        return job["steps"]

    def _run_blocks(self):
        blocks = []
        for step in self._steps():
            self.assertNotIn("if", step, f"triage-area step {step.get('name')!r} is conditional")
            if "run" in step:
                blocks.append(step["run"])
        return "\n".join(blocks)

    def test_the_cron_actually_applies_its_plan(self):
        # MUTANT: drop `--apply` => the sweep prints a plan forever and unparks nothing, so
        # every parked issue stays undispatchable exactly as it was before this lane existed.
        self.assertRegex(self._run_blocks(), r"triage-area\.py\b[^\n]*--apply",
                         "triage-area.yml must run triage-area.py with --apply, or the cron "
                         "is a permanent dry-run and the needs:area park never lifts")

    def test_the_cron_self_tests_before_it_writes(self):
        run = self._run_blocks()
        # Presence before ORDER: `str.index` on a missing needle raises, which would make a
        # deleted --apply an ERROR (a crash-kill) instead of a clean assertion failure.
        self.assertIn("triage-area.py --self-test", run)
        self.assertIn("scripts/tests/test_triage_area.py", run)
        self.assertIn("--apply", run)
        self.assertLess(run.index("triage-area.py --self-test"), run.index("--apply"),
                        "the rule fixtures must run BEFORE the sweep writes area: labels")
        self.assertLess(run.index("scripts/tests/test_triage_area.py"), run.index("--apply"),
                        "the fail-closed safety suite must run BEFORE the sweep writes labels")

    def test_the_sweep_is_scheduled(self):
        on = self.doc.get("on") or self.doc.get(True)      # YAML 1.1 parses bare `on:` as True
        self.assertTrue((on or {}).get("schedule"),
                        "without a schedule this is the hand-run state #3816 measured: the "
                        "park has no automated exit")

    def test_the_job_can_write_labels(self):
        # MUTANT: drop `issues: write` => every unpark fails at runtime, one 403 per issue.
        perms = self.doc.get("permissions") or {}
        self.assertEqual(perms.get("issues"), "write",
                         "triage-area writes area: labels; without issues:write every "
                         "unpark 403s")

    def test_the_unattributable_residue_is_always_reported(self):
        # #3816's third finding: the issues no rule can attribute get no label, no report and
        # no route back to a human, so they are re-skipped forever. The report step is what
        # turns that residue into a number a maintainer sees; deleting it must red here.
        run = self._run_blocks()
        self.assertIn("GITHUB_STEP_SUMMARY", run,
                      "the left-parked residue must reach the run summary, or the "
                      "unattributable class is silent again")
        self.assertIn("LEFT", run,
                      "the residue report must extract the classifier's LEFT lines")

    def test_the_budget_deferred_writes_reach_the_run_summary_too(self):
        # [OPUS-5] #5448. The sweep bounds its `gh issue edit` volume per run to stay
        # under GitHub's secondary limit on content-mutating requests. That cap is only
        # acceptable because it is REPORTED: a tick that deferred 300 issues and a tick
        # with nothing to do look identical otherwise, which is exactly the silence
        # #3816 added this report to end. The classifier prints `DEFERRED` lines
        # (scripts/tests/test_triage_area.py::TestWriteBudget pins that half); this step
        # is what carries them to a human.
        run = self._run_blocks()
        self.assertIn("DEFERRED", run,
                      "the per-run write budget's deferrals never reach the run "
                      "summary — the cap is silent, and a deferred backlog is "
                      "indistinguishable from an empty one")

    def test_this_lane_never_runs_on_a_pull_request(self):
        # It holds `issues: write` and executes the checked-out tree. A `pull_request`
        # trigger would run PR-authored code with a write token; `pull_request_target`
        # would be worse. schedule + workflow_dispatch only.
        on = self.doc.get("on") or self.doc.get(True) or {}
        for trigger in ("pull_request", "pull_request_target", "merge_group", "issues",
                        "issue_comment"):
            self.assertNotIn(trigger, on,
                             f"triage-area holds issues:write — it must not trigger on "
                             f"{trigger}")

    def test_the_workflow_this_class_guards_is_a_path_trigger(self):
        # [OPUS-5] Otherwise this whole class is VACUOUS on the one edit it exists to catch:
        # routing-self-tests.yml is `paths:`-filtered, so a PR touching ONLY triage-area.yml
        # would never run it and deleting --apply would sail through green. Exactly 2 — the
        # pull_request filter AND the push filter.
        source = (REPO_ROOT / ".github" / "workflows"
                  / "routing-self-tests.yml").read_text(encoding="utf-8")
        paths_section = source[:source.index("permissions:")]
        self.assertEqual(paths_section.count('".github/workflows/triage-area.yml"'), 2,
                         ".github/workflows/triage-area.yml must re-run this gate on BOTH "
                         "pull_request and push, or the seam assertions above never execute "
                         "on the PR that breaks them")

    def test_the_classifier_fixtures_run_on_every_pr(self):
        # The classifier is self-tested in docs-quality.yml (no paths filter) so a rule-table
        # change reds THERE rather than silently at the next cron fire — the same #3419
        # argument that covers triage.py/retriage.py.
        source = TestRetriageCronSeam.DOCS_QUALITY.read_text(encoding="utf-8")
        self.assertIn("python3 scripts/triage-area.py --self-test", source,
                      "scripts/triage-area.py --self-test is never RUN on a PR — its "
                      "assertions are dead")
        self.assertIn("python3 scripts/tests/test_triage_area.py", source,
                      "the fail-closed safety suite is never RUN on a PR")


class TestTriageGateAgreesWithReadinessEngine(unittest.TestCase):
    """`triage.py` and `ready-issues.py` must mean the same thing by "gated".

    [OPUS-5] triage() gated `status:ready` on the single literal `needs:user`, while
    `ready.is_gated` treats the whole `needs:*` namespace as a hard dispatch gate. So triage
    attested `status:ready` on `needs:ec2` / `needs:docker` / `needs:zk` issues and the readiness
    engine was the ONLY thing keeping them off the frontier — a single-point defence for work
    gated on real external preconditions. Measured live 2026-07-26: 21 open issues carried both
    a `status:ready` attestation and a real gate.
    """

    def test_every_gate_the_engine_refuses_also_blocks_attestation(self):
        for gate in ("needs:ec2", "needs:docker", "needs:zk", "needs:upstream",
                     "needs:maintainer", "needs:external-subject", "needs:user"):
            labels = {"priority:P1", "role:impl", "area:sparq-core", gate}
            self.assertTrue(ready.is_gated(labels),
                            f"sanity: the readiness engine must treat {gate} as a gate")
            self.assertFalse(triage.triage(labels, "feature")["ready"],
                             f"triage attested status:ready on a {gate}-gated issue; the "
                             "readiness engine is then the only thing keeping it off the "
                             "frontier")

    def test_an_unknown_future_gate_blocks_by_default(self):
        # Namespace rule, not an allow-list: a gate invented tomorrow must not need a code change.
        labels = {"priority:P1", "role:impl", "area:sparq-core", "needs:something-new"}
        self.assertTrue(ready.is_gated(labels))
        self.assertFalse(triage.triage(labels, "feature")["ready"])

    def test_the_self_clearing_area_park_is_not_a_permanent_block(self):
        # needs:area is triage's OWN park. If it counted as blocking, `ready` would be False
        # forever and the remove that lifts it — which only runs in the ready branch — could
        # never fire, so an issue that later gains an area would be stuck for good.
        result = triage.triage(
            {"priority:P1", "role:impl", "area:sparq-core", "needs:area", "status:untriaged"},
            "feature")
        self.assertTrue(result["ready"], "an area landed; the park must lift")
        self.assertIn("needs:area", result["remove"])

    def test_a_gated_issue_is_not_double_parked_with_needs_area(self):
        result = triage.triage({"priority:P1", "role:impl", "needs:ec2"}, "task")
        self.assertNotIn("needs:area", result["add"])


class TestRetriageReachesNeverTriagedIssues(unittest.TestCase):
    """An issue the event triager never ran on carries NO `status:*` label — so a LABEL query
    cannot find it, and that is exactly how retriage built its queue.

    [OPUS-5] Measured live on sparq-org/sparq 2026-07-26: 242 open statusless issues, 238 of them
    without the `flow-on` label that #2474's workaround keyed on. `retriage --repo sparq-org/sparq`
    printed "0 issue(s) promotable"; with the snapshot fetch it plans 74.
    """

    def test_the_statusless_class_needs_triage_regardless_of_flow_on(self):
        self.assertTrue(retriage._needs_triage({"priority:P2", "role:impl", "area:sparq-core"}))
        self.assertTrue(retriage._needs_triage(set()), "a zero-label issue was never triaged")
        self.assertTrue(retriage._needs_triage({"status:untriaged"}))

    def test_already_routed_and_parked_work_stays_out_of_the_queue(self):
        # Widening to "no status label" must not drag in-flight or human-parked work back in.
        for status in ("status:ready", "status:deferred", "status:blocked", "status:parked",
                       "status:in-progress", "status:in-progress-review"):
            self.assertFalse(retriage._needs_triage({status}),
                             f"{status} is already routed; retriage must not churn it")

    def test_reaching_an_issue_is_not_promoting_it(self):
        # Every gate that refuses a status:untriaged issue must equally refuse a statusless one.
        trusted = {"jeswr"}
        refused = [
            {"number": 1, "author": "jeswr",
             "labels": ["priority:P1", "role:impl", "area:sparq-zk", "needs:ec2"]},
            {"number": 2, "author": "jeswr",
             "labels": ["kind:epic", "priority:P1", "role:impl", "area:sparq-core"]},
            {"number": 3, "author": "jeswr",
             "labels": ["trust:untrusted", "priority:P1", "role:impl", "area:sparq-core"]},
            {"number": 4, "author": "jeswr", "labels": ["priority:P1", "role:impl"]},
            {"number": 5, "author": "rando",
             "labels": ["priority:P1", "role:impl", "area:sparq-core"]},
            {"number": 6, "author": "github-actions[bot]",
             "labels": ["priority:P1", "role:impl", "area:sparq-core"]},
            {"number": 7, "author": "jeswr", "labels": []},
        ]
        self.assertEqual(retriage.plan_retriage(refused, lambda who: who in trusted), [],
                         "widening the fetch must not widen what is promoted")

    def test_a_complete_statusless_issue_does_promote(self):
        rows = [{"number": 9, "author": "jeswr",
                 "labels": ["priority:P2", "role:impl", "area:sparq-core"]}]
        self.assertEqual(retriage.plan_retriage(rows, lambda who: who == "jeswr"),
                         [(9, ["status:ready"], [])])

    def test_the_plan_is_idempotent(self):
        rows = [{"number": 9, "author": "jeswr",
                 "labels": ["priority:P2", "role:impl", "area:sparq-core"]}]
        def trusted(who):
            return who == "jeswr"

        first = retriage.plan_retriage(rows, trusted)
        applied = [{**rows[0],
                    "labels": sorted((set(rows[0]["labels"]) | set(first[0][1])) - set(first[0][2]))}]
        self.assertEqual(retriage.plan_retriage(applied, trusted), [],
                         "a second cron fire must not re-edit an already-promoted issue")


class TestRoutingSelfTestWorkflowWiring(unittest.TestCase):
    """The YAML seam — a self-test that no workflow INVOKES is not a gate.

    routing-self-tests.yml listed scripts/ready-issues.py under `paths:` (so it looked covered)
    while its `run:` block never executed that script's --self-test. Deleting either the paths
    entry or the invocation must red THIS test.
    """

    SOURCE = WORKFLOW.read_text(encoding="utf-8")
    INVOKED = ("scripts/routing-validate.py", "scripts/route-resolve.py",
               "scripts/dispatch-plan.py", "scripts/ready-issues.py")

    def test_every_self_tested_script_is_actually_invoked(self):
        run_block = self.SOURCE[self.SOURCE.index("Validate routing schema"):]
        for script in self.INVOKED:
            self.assertIn(f"python3 {script} --self-test", run_block,
                          f"{script} --self-test is never RUN — its assertions are dead in CI")

    def test_ready_issues_self_test_is_invoked_not_merely_path_filtered(self):
        # The precise regression: present in `paths:`, absent from `run:`.
        run_block = self.SOURCE[self.SOURCE.index("Validate routing schema"):]
        self.assertIn("python3 scripts/ready-issues.py --self-test", run_block)

    def test_this_test_file_is_itself_a_path_trigger(self):
        # Scoped to the `paths:` section ON PURPOSE: a whole-file substring search passes for
        # the WRONG reason, because this filename also appears in the run: block, so deleting
        # both paths entries left the old assertion green. Both trigger blocks
        # (pull_request + push) must list it, hence the exact count of 2.
        self.assertEqual(
            self._paths_section().count('"scripts/tests/test_readiness_visibility.py"'), 2,
            "edits to this suite must re-run the gate that executes it, on PR *and* push")

    def test_this_suite_is_invoked_by_the_workflow(self):
        run_block = self.SOURCE[self.SOURCE.index("Validate routing schema"):]
        self.assertIn("python3 scripts/tests/test_readiness_visibility.py", run_block)

    def _paths_section(self):
        return self.SOURCE[:self.SOURCE.index("permissions:")]

    def test_every_invoked_script_is_a_path_trigger(self):
        paths_section = self._paths_section()
        for script in self.INVOKED:
            # Exactly 2: the pull_request filter AND the push filter. Dropping either one
            # lets the script change without re-running the gate on that trigger.
            self.assertEqual(paths_section.count(f'"{script}"'), 2,
                             f"{script} must be a path trigger on BOTH pull_request and push")

    # [OPUS-5] sparq#4819 review, S8. The rule above covers the SCRIPTS the gate runs. It says
    # nothing about the DATA those scripts assert against, and `dispatch-plan.py --self-test` now
    # asserts the shipped `orchestration/registry-contract.toml` (the names the registry reads off
    # that module). Dropping that file from both `paths:` filters survived the entire suite —
    # including every test in this class — because a data file is not an invoked script. That is
    # the same silent-invisibility class the contract file exists to close, one layer out, in the
    # half whose whole thesis is "pin the wiring".
    #
    # DERIVED, not listed. A hard-coded tuple here would be one more written statement that has to
    # be remembered; scanning the invoked scripts for the orchestration data they actually open
    # means a NEW data input demands its own trigger with no edit to this file. The derivation is
    # asserted non-empty and asserted to have found the known inputs, because a regex that silently
    # matches nothing is the way this kind of check fails — quietly, toward "nothing to report".
    _DATA_REF = re.compile(r'orchestration/([A-Za-z0-9._-]+\.(?:toml|json))'
                           r'|"orchestration",\s*"([A-Za-z0-9._-]+\.(?:toml|json))"')

    def _declared_data_inputs(self):
        found = {}
        for script in self.INVOKED:
            text = (REPO_ROOT / script).read_text(encoding="utf-8")
            for match in self._DATA_REF.finditer(text):
                name = match.group(1) or match.group(2)
                found.setdefault(f"orchestration/{name}", set()).add(script)
        return found

    def test_the_data_input_derivation_is_not_vacuous(self):
        found = self._declared_data_inputs()
        # A KNOWN-POSITIVE control: these two are read by the gate's own scripts today, so a
        # derivation that cannot see them cannot see a third one either.
        self.assertLessEqual({"orchestration/routing.toml",
                              "orchestration/registry-contract.toml"}, set(found), found)
        for path in found:
            self.assertTrue((REPO_ROOT / path).is_file(),
                            f"{path} is referenced by a gated script but does not exist")

    def test_every_orchestration_data_input_is_a_path_trigger(self):
        paths_section = self._paths_section()
        for path, readers in sorted(self._declared_data_inputs().items()):
            self.assertEqual(
                paths_section.count(f'"{path}"'), 2,
                f"{path} is asserted against by {sorted(readers)} but is not a path trigger on "
                "BOTH pull_request and push — it could change without re-running the gate that "
                "reads it")

    def test_the_partition_guards_tree_inputs_are_path_triggers(self):
        """[OPUS-5] PR #4925 review — the inputs `workspace_roots()` READS FROM THE TREE.

        The `_declared_data_inputs` derivation above is scoped to `orchestration/*.toml|json`, so
        it cannot see these: the partition guard's inputs are the root workspace manifest and the
        crate directory listing. Neither was a path trigger, and root `Cargo.toml` is the one file
        that can turn the guard into a hard PLAN stop — a routine edit to
        `members = ["crates/*"]` would have merged green with the guard's own tests never running,
        then stopped dispatch for BOTH target repositories on the next tick.

        `crates/*/Cargo.toml` rather than `crates/**`: what this gate needs to re-run for is a
        change to the SET of crate partition roots, which happens exactly when a crate manifest is
        added, removed or renamed. `crates/**` would fire the lane on essentially every Rust PR in
        a 67-crate workspace and cover no additional change to that set.
        """
        paths_section = self._paths_section()
        for path in ("Cargo.toml", "crates/*/Cargo.toml"):
            self.assertEqual(
                paths_section.count(f'"{path}"'), 2,
                f"{path} is read by ready-issues.py's partition guard but is not a path trigger "
                "on BOTH pull_request and push — it could change without re-running the gate "
                "that reads it, and this one can hard-stop PLAN")

    def test_the_partition_guard_reads_the_manifest_it_claims_to(self):
        # Non-vacuity pairing for the row above: assert the guard really does open root
        # Cargo.toml, so the trigger cannot be pinned for a file nothing reads.
        source = (SCRIPTS / "ready-issues.py").read_text(encoding="utf-8")
        self.assertIn('WORKSPACE_MANIFEST = "Cargo.toml"', source)
        self.assertIn('CRATES_DIR = "crates"', source)
        self.assertIn("os.path.join(repo_root, WORKSPACE_MANIFEST)", source)

    def test_gate_is_not_declared_advisory(self):
        # [OPUS-5] Guards the rule that is LIVE. ci-summary's discovery changed on
        # 2026-07-25 (#3773): a check is non-gating iff it is EXPLICITLY DECLARED in
        # .github/advisory-registry.json, keyed on workflow file + job id. The old
        # `\b(advisory|informational)\b` NAME rule is gone — it had silently neutralised
        # four real gates, two of them documented in-repo as HARD — so a rename can no
        # longer demote anything (declarations bind to job identity and
        # check-advisory-registry.py C4 reds on a rename). Asserting on the job NAME would
        # therefore guard a rule that no longer exists; the real demotion path is a
        # registry entry, so that is what is asserted.
        registry = json.loads(
            (REPO_ROOT / ".github" / "advisory-registry.json").read_text(encoding="utf-8"))
        declared = {(e.get("workflow"), e.get("job_id"))
                    for e in registry.get("jobs", {}).values()}
        self.assertNotIn(("routing-self-tests.yml", "validate"), declared,
                         "declaring this job advisory stops ci-summary gating on it")

    def test_merge_group_trigger_present(self):
        # merge_group cannot use a paths filter; without the trigger the queue ref
        # never exposes this gating check.
        self.assertRegex(self.SOURCE, r"(?m)^  merge_group:")


class TestNonReservingCrossCuttingPartitions(unittest.TestCase):
    """`ci` / `docs` ROUTE work but do not OCCUPY it — and everything else still does.

    [OPUS-5 2026-07-28] The dispatch frontier is the binding throughput constraint: three
    consecutive production PLAN runs read `candidates=379 frontier=1 partition-deferred=378`, i.e.
    99.7% of attested candidates refused for partition contention. Driving the REAL production
    predicate (the registry's dispatch.yml `readiness` step, this repo's engine, a live snapshot)
    reproduced that at `candidates=376 frontier=1`, and made `ci`+`docs` non-reserving takes it to
    `candidates=376 frontier=3` — the two added rows declaring exactly `area:ci` and `area:docs`.

    The justification is a MEASURED conflict rate, not an argument (see NON_RESERVING_PARTITIONS
    for the table): 5% of open `area:ci` holder pairs and 3% of `area:docs` pairs share a changed
    file, against 100% for `area:deps` (all `Cargo.lock`) and 57.1% for crate areas. So the safety
    half of this contract is as load-bearing as the throughput half, and both are pinned here.

    Every assertion runs END-TO-END through `compute_ready`. An assertion that only inspected
    `reserves_partition`'s shape would stay green with its call site in `_reserving_packages`
    deleted, which is the surviving-mutant class this estate keeps measuring.
    """

    def test_a_ci_only_issue_is_offered_while_an_occupant_holds_ci(self):
        # THE headline behaviour. Before: the PR held `ci` and #20 was partition-deferred.
        waiting = iss(20, READY + ["priority:P1", "area:ci"])
        occupant = pr(70, ["area:ci"])
        self.assertEqual(
            numbers(ready.compute_ready([occupant, waiting], conflict_log=quiet)), [20],
            "a PR holding area:ci must no longer refuse a ci-only candidate")

    def test_a_docs_only_issue_is_offered_while_an_occupant_holds_docs(self):
        waiting = iss(20, READY + ["priority:P1", "area:docs"])
        self.assertEqual(
            numbers(ready.compute_ready([pr(70, ["area:docs"]), waiting], conflict_log=quiet)),
            [20])

    def test_an_in_progress_issue_holding_ci_also_stops_reserving_it(self):
        # The issue-occupancy path, not just the PR path — both go through _reserving_packages.
        board = [iss(72, ["status:in-progress", "area:ci"]),
                 iss(20, READY + ["priority:P1", "area:ci"])]
        self.assertEqual(numbers(ready.compute_ready(board, conflict_log=quiet)), [20])

    def test_the_exempt_partition_is_released_and_nothing_else_is(self):
        # The measured shape of the change on the live board: reserved keys lost EXACTLY
        # {ci, docs}. A mutant that widens the set shows up here as an extra released key.
        rows = [pr(70, ["area:ci", "area:docs", "area:deps", "area:sparq-core"])]
        held = {key for keys, _ in ready.unit_reservations(rows) for key in keys}
        self.assertEqual(held, {"deps", "sparq-core"})

    # -- the SAFETY half: what must still reserve ------------------------------------------
    def test_deps_still_reserves_because_every_deps_pair_collides_on_the_lockfile(self):
        # MEASURED: 3 of 3 open `area:deps` holder pairs share a changed file, all Cargo.lock.
        # Serialising deps is CORRECT and this exemption must never grow to include it.
        waiting = iss(20, READY + ["priority:P1", "area:deps"])
        self.assertEqual(
            numbers(ready.compute_ready([pr(70, ["area:deps"]), waiting], conflict_log=quiet)),
            [], "area:deps must still be occupied by an open PR that declares it")

    def test_crate_areas_still_reserve(self):
        # 57.1% pairwise file collision (research/crate-region-parallelism.md §4).
        waiting = iss(20, READY + ["priority:P1", "area:sparq-core"])
        self.assertEqual(
            numbers(ready.compute_ready([pr(70, ["area:sparq-core"]), waiting],
                                        conflict_log=quiet)), [])

    def test_sub_crate_containment_still_reserves_through_the_exemption(self):
        # The exemption is applied on the PARTITION PATH, so it must not have flattened the
        # containment algebra it shares that path with.
        waiting = iss(20, READY + ["priority:P1", "area:sparq-core-store"])
        self.assertEqual(
            numbers(ready.compute_ready([pr(70, ["area:sparq-core"]), waiting],
                                        conflict_log=quiet)), [])

    def test_the_global_partition_can_never_be_exempted(self):
        # `partition_path` maps GLOBAL and every degenerate key to `()`, the root that CONTAINS
        # every partition. Exempting it would be "fail toward exempt everything" exactly.
        self.assertTrue(ready.reserves_partition(ready.GLOBAL))
        self.assertTrue(ready.reserves_partition(""))
        waiting = iss(20, READY + ["priority:P1", "area:sparq-core"])
        board = [pr(70, [f"area:{ready.GLOBAL}"]), waiting]
        self.assertEqual(numbers(ready.compute_ready(board, conflict_log=quiet)), [],
                         "an occupant on the global partition must still serialize everything")
        for bad in ({ready.GLOBAL}, {"ci", ready.GLOBAL}, {"-"}, {""}):
            self.assertEqual(ready.non_reserving_partitions(bad), frozenset(), bad)

    # -- the FAIL-SAFE: malformed declaration degrades to TODAY's behaviour -----------------
    def test_a_malformed_declaration_falls_back_to_reserving(self):
        waiting = iss(20, READY + ["priority:P1", "area:ci"])
        board = [pr(70, ["area:ci"]), waiting]
        for broken in (None, "ci,docs", 7, {"ci": True}, {"ci", 7}, ["ci", ""], ("ci", None),
                       {"ci", "  "}):
            with mock.patch.object(ready, "NON_RESERVING_PARTITIONS", broken):
                self.assertEqual(ready.non_reserving_partitions(), frozenset(), broken)
                self.assertEqual(
                    numbers(ready.compute_ready(board, conflict_log=quiet)), [],
                    f"a {broken!r} declaration must degrade to RESERVING, never to exempt-all")

    def test_an_absent_declaration_falls_back_to_reserving(self):
        # Deleting the constant entirely is the "unreadable" case: NameError would abort the
        # whole dispatch tick for every target, so the engine must simply reserve.
        waiting = iss(20, READY + ["priority:P1", "area:ci"])
        board = [pr(70, ["area:ci"]), waiting]
        with mock.patch.object(ready, "NON_RESERVING_PARTITIONS", frozenset()):
            self.assertEqual(numbers(ready.compute_ready(board, conflict_log=quiet)), [])

    def test_a_wellformed_declaration_is_honoured_so_the_failsafe_is_not_vacuous(self):
        # KNOWN-POSITIVE control for the loop above: the same board, a VALID declaration, the
        # opposite outcome. Without this a fail-safe that rejected everything would look perfect.
        waiting = iss(20, READY + ["priority:P1", "area:ci"])
        board = [pr(70, ["area:ci"]), waiting]
        with mock.patch.object(ready, "NON_RESERVING_PARTITIONS", frozenset({"ci"})):
            self.assertEqual(numbers(ready.compute_ready(board, conflict_log=quiet)), [20])

    # -- SCOPE: what this change deliberately does NOT touch --------------------------------
    def test_candidacy_is_unchanged_a_ci_issue_is_still_counted_and_routed(self):
        # Measured obligation 5: `candidates=` must not move. The candidate-side key algebra is
        # untouched, so a ci issue is still a candidate, still keyed on `ci`, still dispatchable.
        self.assertEqual(ready.packages_of({"area:ci"}), {"ci"})
        self.assertEqual(ready.packages_of({"area:docs"}), {"docs"})
        self.assertEqual(ready.declared_packages({"area:ci", "area:docs"}), {"ci", "docs"})
        board = [pr(70, ["area:ci"]), iss(20, READY + ["priority:P1", "area:ci"]),
                 iss(21, READY + ["priority:P1", "area:docs"]),
                 iss(22, READY + ["priority:P1", "area:deps"])]
        cands = ready.ready_candidates(board, log=quiet)
        self.assertEqual([(number, sorted(keys)) for _p, number, _row, keys in cands],
                         [(20, ["ci"]), (21, ["docs"]), (22, ["deps"])],
                         "candidacy and candidate keying must be byte-identical to before")

    def test_a_selected_ci_candidate_still_reserves_ci_for_the_tick(self):
        # The exemption is the OCCUPANCY half ONLY. Per-tick dispatch width stays one worker per
        # partition; this is why the live frontier moved 1 -> 3 and not 1 -> ~50.
        board = [iss(20, READY + ["priority:P0", "area:ci"]),
                 iss(21, READY + ["priority:P1", "area:ci"])]
        self.assertEqual(numbers(ready.compute_ready(board, conflict_log=quiet)), [20])

    def test_ci_fragments_travels_with_the_ci_partition(self):
        # `ci-fragments` is a LIVE label that resolves into the `ci` partition. Exempting the raw
        # string only would leave it reserving a partition `ci` itself does not — incoherent with
        # `keys_conflict`, and a trap for the next reader.
        self.assertEqual(ready.partition_path("ci-fragments"), ("ci",))
        self.assertFalse(ready.reserves_partition("ci-fragments"))
        waiting = iss(20, READY + ["priority:P1", "area:ci"])
        self.assertEqual(
            numbers(ready.compute_ready([pr(70, ["area:ci-fragments"]), waiting],
                                        conflict_log=quiet)), [20])

    def test_the_declaration_records_its_measured_basis(self):
        # The brief's one durable requirement: the next reader must be able to RE-DERIVE the
        # decision rather than guess at it. Asserted on the numbers, not on prose.
        source = (SCRIPTS / "ready-issues.py").read_text(encoding="utf-8")
        head, _, rest = source.partition("NON_RESERVING_PARTITIONS = ")
        self.assertTrue(rest, "NON_RESERVING_PARTITIONS is no longer declared")
        basis = head[head.rindex("# ------"):]
        for token in ("area:ci", "area:docs", "area:deps", "Cargo.lock",
                      "5%", "3%", "100%", "57.1%", "crate-region-parallelism.md"):
            self.assertIn(token, basis,
                          f"the measured basis for the exemption no longer states {token}")

    # -- sparq#4929: THE SECOND OCCUPANCY LEG ----------------------------------------------
    def test_the_exemption_is_offered_to_the_registry_on_both_channels(self):
        """sparq#4929 — the CLAIM leg must be able to ASK, not to re-type `{"ci", "docs"}`.

        #4928 landed the exemption on the PLAN leg (everything above). #4929 reports that the
        registry has a SECOND occupancy leg — `dispatch-claim.py::busy_packages_of_pulls`, applied
        at `filter_busy_area_items` on assemble and at `revalidate_items_against_live_pulls` at
        claim time — which unions a busy-area set from open worker PRs with no exemption, so the
        rows this engine now offers are re-deferred one layer down.

        The registry reads sparq on two channels and they must not disagree, exactly as #4365's
        fixture pins for the resolver: `dispatch.yml`'s readiness step `load_dispatch`es
        `scripts/dispatch-plan.py` and calls the module, while a leg holding no planner can only
        read `--dump-partitions` — a different code path (argv parsing, JSON encoding, a fresh
        process with its own tree scan). Both are pinned to the ONE table in
        `orchestration/registry-contract.toml`.

        NOTHING HERE OBSERVES THE REGISTRY. This is the sparq half; green does not mean
        `busy_packages_of_pulls` has changed.
        """
        contract = tomllib.loads(
            (REPO_ROOT / "orchestration" / "registry-contract.toml").read_text(encoding="utf-8")
        )["non_reserving_partitions"]
        fixture = contract["parity_fixture"]
        # Coverage, not just agreement: a silently dropped row would leave this green over a
        # shrinking contract. `ci-fragments` is the single row an exact-string `key in {"ci",
        # "docs"}` mirror gets WRONG (it resolves into `ci`), and `deps`/the crate areas are the
        # safety half #4929 asks to keep reserving on both legs.
        self.assertEqual(set(fixture), {"ci", "docs", "ci-fragments", "deps", "sparq-core",
                                        "sparq-core-store", "__global__", ""})
        self.assertEqual(sorted(contract["declared"]["partitions"]),
                         sorted(ready.non_reserving_partitions()))
        self.assertEqual(sorted(k for k in fixture
                                if fixture[k] != (k not in set(contract["declared"]["partitions"]))),
                         ["ci-fragments"],
                         "the fixture must contain a row exact-string membership gets wrong")

        # CHANNEL 1 — the in-process predicate. (The `load_dispatch`-shaped probe, which loads
        # `dispatch-plan.py` exactly as the registry does, is `dispatch-plan.py --self-test`; this
        # asserts the engine those exports are bound to.)
        self.assertEqual({k: ready.reserves_partition(k) for k in fixture}, dict(fixture))

        # CHANNEL 2 — the offline CLI, in a fresh process.
        keys = sorted(k for k in fixture if k)          # argv cannot carry the degenerate key
        out = subprocess.run(
            [sys.executable, str(SCRIPTS / "ready-issues.py"), "--dump-partitions", *keys],
            capture_output=True, text=True, check=True).stdout
        dumped = json.loads(out)
        self.assertEqual(dumped["reserves"], {k: fixture[k] for k in keys})
        self.assertEqual(dumped["non_reserving"], sorted(contract["declared"]["partitions"]))
        # ...and the #4365 keys the registry already reads are still there beside them.
        self.assertEqual(dumped["resolved"]["ci-fragments"], ["ci"])


if __name__ == "__main__":
    unittest.main(verbosity=2)
